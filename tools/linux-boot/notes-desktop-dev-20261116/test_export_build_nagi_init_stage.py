from __future__ import annotations

import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT = Path(__file__).with_name("export_build_nagi_init_stage.py")
SPEC = importlib.util.spec_from_file_location("export_build_nagi_init_stage", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
stage_tool = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage_tool)


def git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {result.stderr}")
    return result.stdout.strip()


class SourceStageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="nagi-source-stage-test-")
        # macOS may expose its temporary root through /var -> /private/var;
        # start ordinary fixtures from the physical path so only the explicit
        # symlink-negative tests exercise a symlinked ancestor.
        self.root = Path(self.temp.name).resolve()
        self.repo = self.root / "fixture-repo"
        self.stage_parent = self.root / "stage-parent"
        self.wrong_cwd = self.root / "unrelated-cwd"
        self.repo.mkdir()
        self.wrong_cwd.mkdir()
        git(self.repo, "init", "--quiet")
        git(self.repo, "config", "user.name", "stage fixture")
        git(self.repo, "config", "user.email", "stage-fixture@example.invalid")
        (self.repo / ".gitignore").write_text("ignored-stages/\n", encoding="utf-8")
        (self.repo / "Cargo.lock").write_text("pinned lock fixture\n", encoding="utf-8")
        (self.repo / "user").mkdir()
        (self.repo / "user/nagi-init").mkdir()
        (self.repo / "user/nagi-init/Cargo.toml").write_text("[package]\nname='nagi-init'\n", encoding="utf-8")
        vendor_sentinel = self.repo / "third_party/pinned/sentinel.txt"
        vendor_sentinel.parent.mkdir(parents=True)
        vendor_sentinel.write_text("committed vendor sentinel\n", encoding="utf-8")
        (self.repo / "tools/linux-boot/notes-desktop-dev-20261116").mkdir(parents=True)
        helper = self.repo / "tools/linux-boot/notes-desktop-dev-20261116/build_nagi_init_elf.sh"
        helper.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        helper.chmod(0o755)
        nagi = self.repo / "nagi"
        nagi.write_text(
            "#!/bin/sh\n"
            "if git rev-parse --show-toplevel >/dev/null 2>&1; then exit 19; fi\n"
            "printf '%s' \"$PWD\" >\"$STAGE_CWD_RECORD\"\n"
            "exit \"${STAGE_FETCH_EXIT:-0}\"\n",
            encoding="utf-8",
        )
        nagi.chmod(0o755)
        git(self.repo, "add", ".gitignore", "Cargo.lock", "user", "tools", "nagi", "third_party")
        git(self.repo, "commit", "--quiet", "-m", "fixture")
        self.commit = git(self.repo, "rev-parse", "HEAD")

    def tearDown(self) -> None:
        os.chdir(Path(__file__).resolve().parents[4])
        self.temp.cleanup()

    def test_archive_and_stage_command_ignore_callers_cwd(self) -> None:
        stage = stage_tool.create_stage_directory(self.stage_parent, "wrong-cwd")
        original_cwd = Path.cwd()
        try:
            os.chdir(self.wrong_cwd)
            stage_tool.export_commit(self.repo, self.commit, stage)
            self.assertEqual((stage / "Cargo.lock").read_text(encoding="utf-8"), "pinned lock fixture\n")
            self.assertFalse((stage / ".git").exists())

            record = stage / "fetch-cwd.txt"
            env = stage_tool._stage_git_environment(
                stage,
                {
                    "STAGE_CWD_RECORD": str(record),
                    "GIT_DIR": str(self.repo / ".git"),
                    "GIT_WORK_TREE": str(self.repo),
                },
            )
            with contextlib.redirect_stdout(io.StringIO()):
                stage_tool.run_in_stage(["./nagi", "fetch"], stage, env)
            self.assertEqual(record.read_text(encoding="utf-8"), str(stage))
            self.assertEqual(git(self.repo, "status", "--short"), "")
        finally:
            os.chdir(original_cwd)

    def test_existing_stage_is_rejected_without_touching_sentinel(self) -> None:
        destination = self.stage_parent / "source-existing"
        destination.mkdir(parents=True)
        sentinel = destination / "keep.bin"
        sentinel.write_bytes(b"preserve-existing-stage")
        with self.assertRaisesRegex(stage_tool.StageError, "refusing existing stage"):
            stage_tool.create_stage_directory(self.stage_parent, "existing")
        self.assertEqual(sentinel.read_bytes(), b"preserve-existing-stage")
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_nested_export_is_not_discovered_as_parent_worktree(self) -> None:
        nested_parent = self.repo / "ignored-stages"
        stage = stage_tool.create_stage_directory(nested_parent, "nested")
        stage_tool.export_commit(self.repo, self.commit, stage)
        self.assertFalse((stage / ".git").exists())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_export_commit_rejects_symlinked_stage_without_touching_target(self) -> None:
        target = self.stage_parent / "real-target"
        target.mkdir(parents=True)
        symlink_stage = self.stage_parent / "symlink-stage"
        symlink_stage.symlink_to(target, target_is_directory=True)

        with self.assertRaisesRegex(stage_tool.StageError, "refusing symlinked stage"):
            stage_tool.export_commit(self.repo, self.commit, symlink_stage)

        self.assertEqual(list(target.iterdir()), [])
        self.assertTrue(symlink_stage.is_symlink())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_export_commit_rejects_symlinked_ancestor_without_touching_target(self) -> None:
        real_parent = self.stage_parent / "real-parent"
        real_parent.mkdir(parents=True)
        target_stage = real_parent / "stage"
        target_stage.mkdir()
        symlink_parent = self.stage_parent / "linked-parent"
        symlink_parent.symlink_to(real_parent, target_is_directory=True)

        with self.assertRaisesRegex(stage_tool.StageError, "refusing symlinked stage path component"):
            stage_tool.export_commit(self.repo, self.commit, symlink_parent / "stage")

        self.assertEqual(list(target_stage.iterdir()), [])
        self.assertTrue(symlink_parent.is_symlink())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_pipeline_rejects_symlinked_stage_ancestor_before_build_tools(self) -> None:
        real_parent = self.stage_parent / "pipeline-parent"
        real_parent.mkdir(parents=True)
        symlink_parent = self.stage_parent / "pipeline-link"
        symlink_parent.symlink_to(real_parent, target_is_directory=True)
        requested_parent = symlink_parent / "new-stage-parent"

        with (
            mock.patch.object(stage_tool, "pinned_toolchain") as pinned_toolchain,
            mock.patch.object(stage_tool, "verify_source_commit") as verify_source_commit,
        ):
            with self.assertRaisesRegex(stage_tool.StageError, "refusing symlinked stage parent path component"):
                stage_tool.execute_pipeline(self.repo, requested_parent, "ancestor")

        pinned_toolchain.assert_not_called()
        verify_source_commit.assert_not_called()
        self.assertFalse((real_parent / "new-stage-parent").exists())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_cargo_writable_paths_and_git_boundary_are_stage_local(self) -> None:
        stage = stage_tool.create_stage_directory(self.stage_parent, "env-boundary")
        cargo_home = stage / "tools/linux-boot/notes-desktop-dev-20261116/cargo-home-rust198"
        target = stage / "tools/linux-boot/notes-desktop-dev-20261116/target-build"
        inherited = {
            "CARGO_HOME": "/outside/cargo-home",
            "CARGO_TARGET_DIR": "/outside/target",
            "CARGO_BUILD_BUILD_DIR": "/outside/intermediates",
            "CARGO_ENCODED_RUSTFLAGS": "--cfg wrong",
            "GIT_DIR": str(self.repo / ".git"),
        }
        with mock.patch.dict(os.environ, inherited, clear=False):
            env = stage_tool.stage_environment(stage, cargo_home, target)
        self.assertEqual(env["CARGO_HOME"], str(cargo_home))
        self.assertEqual(env["CARGO_TARGET_DIR"], str(target))
        self.assertEqual(env["CARGO_BUILD_TARGET_DIR"], str(target))
        self.assertEqual(env["CARGO_BUILD_BUILD_DIR"], str(target))
        self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", env)
        self.assertNotIn("GIT_DIR", env)
        self.assertEqual(env["GIT_CEILING_DIRECTORIES"], str(stage.parent))

    def test_pipeline_assembles_all_mutations_with_stage_cwd(self) -> None:
        calls: list[tuple[list[str], Path, dict[str, str]]] = []

        def fake_run(command: list[str], cwd: Path, env: dict[str, str]) -> None:
            calls.append((command, cwd, env))
            self.assertEqual(env["GIT_CEILING_DIRECTORIES"], str(cwd.parent))
            self.assertFalse((cwd / ".git").exists())
            self.assertEqual(env["CARGO_HOME"], str(cwd / stage_tool.LANE_RELATIVE / "cargo-home-rust198"))
            fetch_target = cwd / stage_tool.LANE_RELATIVE / "target-source-stage-fetch-pipeline"
            if "build_nagi_init_elf.sh" in command[0]:
                target = cwd / stage_tool.LANE_RELATIVE / f"target-nagi-init-rust198-source-stage-pipeline" / stage_tool.TARGET / "debug"
                self.assertEqual(env["CARGO_BUILD_BUILD_DIR"], str(target.parent.parent))
                target.mkdir(parents=True)
                (target / "nagi-init").write_bytes(b"fixture ELF")
            else:
                self.assertEqual(env["CARGO_TARGET_DIR"], str(fetch_target))
                self.assertEqual(env["CARGO_BUILD_BUILD_DIR"], str(fetch_target))

        with (
            mock.patch.object(stage_tool, "SOURCE_COMMIT", self.commit),
            mock.patch.object(stage_tool, "pinned_toolchain", return_value=(Path("/fixture/bin"), Path("/fixture/bin/cargo"))),
            mock.patch.object(stage_tool, "run_in_stage", side_effect=fake_run),
        ):
            with contextlib.redirect_stdout(io.StringIO()):
                output = stage_tool.execute_pipeline(self.repo, self.stage_parent, "pipeline")

        self.assertEqual(output.read_bytes(), b"fixture ELF")
        self.assertEqual(len(calls), 4)
        self.assertEqual([cwd for _, cwd, _ in calls], [calls[0][1]] * 4)
        self.assertEqual(calls[0][0], ["./nagi", "fetch"])
        self.assertIn("--locked", calls[1][0])
        self.assertIn("--target", calls[1][0])
        self.assertIn("--locked", calls[2][0])
        self.assertIn("build_nagi_init_elf.sh", calls[3][0][0])
        self.assertEqual(calls[3][0][1:], ["source-stage-pipeline"])

    def test_explicit_commit_exports_committed_tree_and_preserves_dirty_vendor(self) -> None:
        committed_lock = (self.repo / "Cargo.lock").read_text(encoding="utf-8")
        vendor_sentinel = self.repo / "third_party/pinned/sentinel.txt"
        committed_vendor = vendor_sentinel.read_text(encoding="utf-8")
        (self.repo / "Cargo.lock").write_text("uncommitted lock replacement\n", encoding="utf-8")
        vendor_sentinel.write_text("preserve live vendor sentinel\n", encoding="utf-8")
        dirty_only = self.repo / "third_party/untracked/dirty.txt"
        dirty_only.parent.mkdir(parents=True)
        dirty_only.write_text("must not enter export\n", encoding="utf-8")
        observed_stage: Path | None = None

        def fake_run(command: list[str], cwd: Path, env: dict[str, str]) -> None:
            nonlocal observed_stage
            self.assertEqual(env["GIT_CEILING_DIRECTORIES"], str(cwd.parent))
            observed_stage = cwd
            self.assertEqual((cwd / "Cargo.lock").read_text(encoding="utf-8"), committed_lock)
            self.assertEqual(
                (cwd / "third_party/pinned/sentinel.txt").read_text(encoding="utf-8"),
                committed_vendor,
            )
            self.assertFalse((cwd / "third_party/untracked/dirty.txt").exists())
            if "build_nagi_init_elf.sh" in command[0]:
                target = (
                    cwd
                    / stage_tool.LANE_RELATIVE
                    / "target-nagi-init-rust198-source-stage-dirty-source"
                    / stage_tool.TARGET
                    / "debug"
                )
                target.mkdir(parents=True)
                (target / "nagi-init").write_bytes(b"fixture ELF")

        with (
            mock.patch.object(stage_tool, "pinned_toolchain", return_value=(Path("/fixture/bin"), Path("/fixture/bin/cargo"))),
            mock.patch.object(stage_tool, "run_in_stage", side_effect=fake_run),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            output = stage_tool.execute_pipeline(self.repo, self.stage_parent, "dirty-source", self.commit)

        self.assertEqual(observed_stage, self.stage_parent / "source-dirty-source")
        self.assertEqual(output.read_bytes(), b"fixture ELF")
        self.assertEqual((self.repo / "Cargo.lock").read_text(encoding="utf-8"), "uncommitted lock replacement\n")
        self.assertEqual(vendor_sentinel.read_text(encoding="utf-8"), "preserve live vendor sentinel\n")
        self.assertEqual(dirty_only.read_text(encoding="utf-8"), "must not enter export\n")

    def test_plan_reports_commit_and_fresh_stage_without_mutation(self) -> None:
        label = f"plan-{self.commit[:8]}"
        expected_stage = self.stage_parent / f"source-{label}"
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            planned = stage_tool.plan_pipeline(self.repo, self.stage_parent, label, self.commit)

        self.assertEqual(planned, expected_stage)
        self.assertIn(f"source_commit={self.commit}", output.getvalue())
        self.assertIn(f"stage_root={expected_stage}", output.getvalue())
        self.assertIn("plan_only=true", output.getvalue())
        self.assertFalse(self.stage_parent.exists())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_invalid_or_missing_commit_is_rejected_before_tools_or_stage_creation(self) -> None:
        for label, commit in (("short-id", "deadbeef"), ("missing-id", "f" * 40)):
            with self.subTest(commit=commit):
                with mock.patch.object(stage_tool, "pinned_toolchain") as pinned_toolchain:
                    with self.assertRaises(stage_tool.StageError):
                        stage_tool.execute_pipeline(self.repo, self.stage_parent, label, commit)
                pinned_toolchain.assert_not_called()
                self.assertFalse(self.stage_parent.exists())
                self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_mid_export_failure_retains_incomplete_stage_and_blocks_reuse(self) -> None:
        stage = stage_tool.create_stage_directory(self.stage_parent, "mid-failure")
        with self.assertRaisesRegex(stage_tool.StageError, "partial stage retained"):
            stage_tool.export_commit(self.repo, "deadbeef", stage)
        self.assertTrue(stage.is_dir())
        with self.assertRaisesRegex(stage_tool.StageError, "refusing existing stage"):
            stage_tool.create_stage_directory(self.stage_parent, "mid-failure")
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_fetch_failure_preserves_export_for_diagnosis(self) -> None:
        stage = stage_tool.create_stage_directory(self.stage_parent, "fetch-failure")
        stage_tool.export_commit(self.repo, self.commit, stage)
        record = stage / "fetch-cwd.txt"
        env = stage_tool._stage_git_environment(
            stage,
            {
                "STAGE_CWD_RECORD": str(record),
                "STAGE_FETCH_EXIT": "17",
                "GIT_DIR": str(self.repo / ".git"),
                "GIT_WORK_TREE": str(self.repo),
            },
        )
        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(stage_tool.StageError, "exited 17"):
                stage_tool.run_in_stage(["./nagi", "fetch"], stage, env)
        self.assertEqual(record.read_text(encoding="utf-8"), str(stage))
        self.assertTrue((stage / "Cargo.lock").is_file())
        self.assertEqual(git(self.repo, "status", "--short"), "")

    def test_stage_label_cannot_escape_parent(self) -> None:
        with self.assertRaisesRegex(stage_tool.StageError, "stage label"):
            stage_tool.create_stage_directory(self.stage_parent, "../escape")
        self.assertFalse((self.root / "escape").exists())


if __name__ == "__main__":
    unittest.main()
