"""Host-only regression tests for the Task24 release PID 1 gate."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from task24_release_gate import (
    EXPECTED_MARKERS,
    EXPECTED_MARKER_COUNT,
    GateError,
    check_rustflags,
    cleanup_isolated_target,
    prepare_build_paths,
    validate_artifacts,
    validate_marker_counts,
    validate_output_files,
    validate_profile,
)


class Task24ReleaseGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="task24-release-gate-test-")
        self.root = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def cargo_env(self) -> dict[str, str]:
        cargo_home = self.root / "cargo-home"
        cargo_home.mkdir(exist_ok=True)
        return {"CARGO_HOME": str(cargo_home)}

    def artifact(
        self,
        name: str,
        kind: str,
        *,
        test: object = False,
        opt_level: object = "3",
        debug_assertions: object = False,
        fresh: object = False,
        executable: str | None = None,
        include_test: bool = True,
        include_fresh: bool = True,
        filenames: list[str] | None = None,
    ) -> dict[str, object]:
        profile: dict[str, object] = {
            "opt_level": opt_level,
            "debug_assertions": debug_assertions,
        }
        if include_test:
            profile["test"] = test
        report: dict[str, object] = {
            "reason": "compiler-artifact",
            "target": {"name": name, "kind": [kind]},
            "profile": profile,
        }
        if include_fresh:
            report["fresh"] = fresh
        if executable is not None:
            report["executable"] = executable
        if filenames is not None:
            report["filenames"] = filenames
        return report

    def valid_reports(self, target: Path) -> list[dict[str, object]]:
        target_release = target / "x86_64-unknown-linux-musl" / "release"
        executable = target_release / "nagi-init"
        target_library = target_release / "deps" / "libserde.rlib"
        host_library = target / "release" / "deps" / "libunicode_ident.rlib"
        proc_macro = target / "release" / "deps" / "libexample_macro.dylib"
        build_script = target_release / "build" / "build-script-build"
        for artifact_path in (executable, target_library, host_library, proc_macro, build_script):
            artifact_path.parent.mkdir(parents=True, exist_ok=True)
            artifact_path.write_bytes(b"test artifact")
        return [
            self.artifact("nagi-init", "bin", executable=str(executable), filenames=[str(executable)]),
            self.artifact("serde", "lib", filenames=[str(target_library)]),
            self.artifact(
                "unicode_ident",
                "lib",
                opt_level="0",
                filenames=[str(host_library)],
            ),
            self.artifact(
                "example-macro",
                "proc-macro",
                opt_level="0",
                debug_assertions=True,
                filenames=[str(proc_macro)],
            ),
            self.artifact(
                "build-script-build",
                "custom-build",
                opt_level="0",
                debug_assertions=True,
                filenames=[str(build_script)],
            ),
        ]

    def test_only_debug_and_release_profiles_are_accepted(self) -> None:
        self.assertEqual(validate_profile("release"), "release")
        self.assertEqual(validate_profile("debug"), "debug")
        for profile in ("release-lto", "bench", "", None, 1):
            with self.subTest(profile=profile), self.assertRaises(GateError):
                validate_profile(profile)

    def test_release_profile_checks_every_linked_artifact(self) -> None:
        target = self.root / "isolated"
        target.mkdir()
        reports = self.valid_reports(target)
        _, summary, original_path = validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)
        self.assertEqual(summary["linked_artifact_count"], 2)
        self.assertEqual(summary["excluded_build_script_or_proc_macro_count"], 2)
        self.assertEqual(summary["host_only_artifact_count"], 1)
        self.assertEqual(original_path, "x86_64-unknown-linux-musl/release/nagi-init")

        reports[1]["profile"]["opt_level"] = "2"  # type: ignore[index]
        with self.assertRaisesRegex(GateError, "linked release artifact"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

        reports[1]["profile"]["opt_level"] = "3"  # type: ignore[index]
        reports[1]["profile"]["debug_assertions"] = True  # type: ignore[index]
        with self.assertRaisesRegex(GateError, "linked release artifact"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

        reports = self.valid_reports(target)
        reports[0]["profile"]["opt_level"] = "2"  # type: ignore[index]
        with self.assertRaisesRegex(GateError, "linked release artifact"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

    def test_artifact_metadata_fails_closed_for_test_profile_and_fresh(self) -> None:
        target = self.root / "isolated"
        target.mkdir()
        reports = self.valid_reports(target)
        reports[1]["profile"].pop("test")  # type: ignore[union-attr]
        with self.assertRaisesRegex(GateError, "boolean profile.test"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

        reports = self.valid_reports(target)
        reports[1]["profile"]["test"] = "false"  # type: ignore[index]
        with self.assertRaisesRegex(GateError, "boolean profile.test"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

        reports = self.valid_reports(target)
        reports[1]["fresh"] = True
        with self.assertRaisesRegex(GateError, "reused"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

        reports = self.valid_reports(target)
        reports[1].pop("fresh")
        with self.assertRaisesRegex(GateError, "boolean fresh"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

    def test_pid1_executable_must_reside_under_isolated_target_dir(self) -> None:
        target = self.root / "isolated"
        target.mkdir()
        external = self.root / "outside" / "nagi-init"
        external.parent.mkdir()
        external.write_bytes(b"test")
        reports = self.valid_reports(target)
        reports[0]["executable"] = str(external)
        with self.assertRaisesRegex(GateError, "outside the isolated"):
            validate_artifacts(reports, "release", "x86_64-unknown-linux-musl", target)

    def test_marker_set_is_exactly_the_five_canonical_names(self) -> None:
        self.assertEqual(len(EXPECTED_MARKERS), EXPECTED_MARKER_COUNT)
        self.assertEqual(
            set(EXPECTED_MARKERS),
            {
                "nagi.dev_ws10",
                "NAGI_DEV_WS10",
                "NAGI_A_INPUT_TO_PRESENT",
                "NAGI_A_GUEST_MEM",
                "SYSTEM_DATA_MOUNT",
            },
        )
        zero_counts = {marker: 0 for marker in EXPECTED_MARKERS}
        self.assertEqual(validate_marker_counts(zero_counts, "release"), 0)
        self.assertEqual(validate_marker_counts({marker: 1 for marker in EXPECTED_MARKERS}, "debug"), 5)
        with self.assertRaisesRegex(GateError, "exactly the five"):
            validate_marker_counts({"NAGI_DEV_WS10": 0}, "release")
        with self.assertRaisesRegex(GateError, "exactly the five"):
            validate_marker_counts({**zero_counts, "renamed": zero_counts[EXPECTED_MARKERS[0]]}, "release")

    def test_rustflags_environment_overrides_are_rejected(self) -> None:
        for name in (
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
        ):
            with self.subTest(name=name), self.assertRaisesRegex(GateError, name):
                check_rustflags(self.root, {**self.cargo_env(), name: ""})

    def test_rustflags_in_cargo_config_are_rejected(self) -> None:
        config_dir = self.root / ".cargo"
        config_dir.mkdir()
        config = config_dir / "config.toml"
        env = self.cargo_env()

        for contents in (
            '[build]\nrustflags = ["-C", "debug-assertions=yes"]\n',
            "[target.'x86_64-unknown-linux-musl']\nrustflags = []\n",
            '[env]\nRUSTFLAGS = "-C opt-level=0"\n',
            '[env]\nCARGO_ENCODED_RUSTFLAGS = ""\n',
        ):
            with self.subTest(contents=contents):
                config.write_text(contents, encoding="utf-8")
                with self.assertRaises(GateError):
                    check_rustflags(self.root, env)

        config.write_text("[build]\ntarget = 'x86_64-unknown-linux-musl'\n", encoding="utf-8")
        self.assertEqual(check_rustflags(self.root, env)["cargo_config_override_count"], 0)

    def test_unparseable_cargo_config_is_rejected(self) -> None:
        config_dir = self.root / ".cargo"
        config_dir.mkdir()
        (config_dir / "config.toml").write_text("[build\nrustflags = []\n", encoding="utf-8")
        with self.assertRaisesRegex(GateError, "inspect a Cargo config"):
            check_rustflags(self.root, self.cargo_env())

    def test_cargo_home_rustflags_config_is_rejected(self) -> None:
        cargo_home = self.root / "custom-cargo-home"
        cargo_home.mkdir()
        (cargo_home / "config").write_text('[build]\nrustflags = "-C opt-level=1"\n', encoding="utf-8")
        with self.assertRaisesRegex(GateError, "Cargo rustflags configuration"):
            check_rustflags(self.root, {"CARGO_HOME": str(cargo_home)})

    def test_build_paths_are_fresh_isolated_and_owned_cleanup_is_bounded(self) -> None:
        target_root = self.root / "cargo-target"
        output_dir = self.root / "out" / "run-1"
        output_dir.parent.mkdir()
        isolated = prepare_build_paths(output_dir, target_root)
        self.assertEqual(isolated.parent, target_root.resolve())
        self.assertTrue(isolated.name.startswith("task24-release-pid1-"))
        self.assertEqual(list(isolated.iterdir()), [])
        self.assertEqual(list(output_dir.iterdir()), [])
        cleanup_isolated_target(target_root, isolated)
        self.assertFalse(isolated.exists())

    def test_stale_output_and_unexpected_output_files_are_rejected_without_deletion(self) -> None:
        target_root = self.root / "cargo-target"
        output_dir = self.root / "out"
        output_dir.mkdir()
        stale_file = output_dir / "old-receipt.json"
        stale_file.write_text("preserve", encoding="utf-8")
        with self.assertRaisesRegex(GateError, "already exists"):
            prepare_build_paths(output_dir, target_root)
        self.assertEqual(stale_file.read_text(encoding="utf-8"), "preserve")

        with self.assertRaisesRegex(GateError, "unexpected files"):
            validate_output_files(output_dir, ["build-receipt.json"])


if __name__ == "__main__":
    unittest.main()
