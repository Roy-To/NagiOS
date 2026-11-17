"""Tests for the NDP-03 generator. They call the real generator/writer code.

Everything is written under temporary directories; the repository is never a
destination. Host-only: nothing here builds the generated crate or claims Nagi
target acceptance.
"""
import copy
import errno
import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import generate as gen

REPO_ROOT = gen.REPO_ROOT


def _load_validator():
    spec = importlib.util.spec_from_file_location(
        "_ndp03_validate", REPO_ROOT / "tools/nagi-developer-platform/validation/validate.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def valid_inputs(**changes):
    inputs = {
        "app_id": "com.example.level-zero",
        "package_name": "level-zero",
        "display_name": {"en-US": "Level Zero", "ja-JP": "レベルゼロ"},
        "output_dir": "samples/level-zero",
    }
    inputs.update(changes)
    return inputs


def tree(root):
    """{relative posix path: bytes or None for directories} below root, no symlink following."""
    result = {}
    for current, dirs, files in os.walk(root):
        for name in dirs + files:
            path = Path(current, name)
            relative = path.relative_to(root).as_posix()
            result[relative] = None if path.is_dir() and not path.is_symlink() else (
                os.readlink(path).encode() if path.is_symlink() else path.read_bytes())
    return result


def make_checkout(root):
    # Minimal stand-in for the selected checkout: generation only checks that
    # the SDK Cargo.toml exists; contents are not read.
    (root / "sdk/rust").mkdir(parents=True)
    (root / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
    (root / "samples").mkdir()
    return root


_DELETING_CALLS = ("unlink", "remove", "rmdir", "removedirs", "rename", "replace")


def _forbid_from_generator(name):
    real = getattr(os, name)

    def guard(*args, **kwargs):
        if sys._getframe(1).f_globals.get("__name__") == gen.__name__:
            raise AssertionError("generator must never call os." + name)
        return real(*args, **kwargs)
    return guard


class GeneratorTestCase(unittest.TestCase):
    def setUp(self):
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        self.root = make_checkout(Path(holder.name) / "checkout")
        self.outside = Path(holder.name) / "outside"
        self.outside.mkdir()
        # Policy: the generator never deletes, renames or replaces anything.
        # Calls made by the tests themselves pass through.
        for name in _DELETING_CALLS:
            patcher = mock.patch.object(gen.os, name, _forbid_from_generator(name))
            patcher.start()
            self.addCleanup(patcher.stop)

    def reason(self, inputs=None, root=None):
        with self.assertRaises(gen.GenerationError) as caught:
            gen.generate(valid_inputs() if inputs is None else inputs, root or self.root)
        return caught.exception


class GenerationTests(GeneratorTestCase):
    def test_creates_exactly_the_contract_outputs(self):
        result = gen.generate(valid_inputs(), self.root)
        contract = gen.contract_module.load_contract()
        expected = {entry["path"] for entry in contract["outputs"]}
        out = self.root / "samples/level-zero"
        files = {path for path, data in tree(out).items() if data is not None}
        self.assertEqual(files, expected)
        self.assertEqual(result["status"], "created")
        self.assertFalse(result["built"])
        self.assertFalse(result["validated"])
        self.assertEqual(sorted(result["created"]),
                         sorted("samples/level-zero/" + path for path in expected))

    def test_content_matches_the_contract_preview_and_refers_to_real_sdk_path(self):
        gen.generate(valid_inputs(), self.root)
        out = self.root / "samples/level-zero"
        preview = gen.contract_module.preview(valid_inputs())
        for relative, text in preview.items():
            self.assertEqual((out / relative).read_bytes(), text.encode("utf-8"))
        self.assertIn('nagi-sdk = { path = "../../sdk/rust" }',
                      (out / "Cargo.toml").read_text(encoding="utf-8"))

    def test_generated_manifest_satisfies_schema_root_requirements(self):
        gen.generate(valid_inputs(), self.root)
        manifest = json.loads((self.root / "samples/level-zero/manifest.json")
                              .read_text(encoding="utf-8"))
        schema = json.loads((REPO_ROOT / "sdk/rust/schemas/app-manifest.schema.json")
                            .read_text(encoding="utf-8"))
        self.assertLessEqual(set(schema["required"]), set(manifest))
        if schema.get("additionalProperties") is False:
            self.assertLessEqual(set(manifest), set(schema["properties"]))
        self.assertEqual(manifest["origin"], "third-party")
        self.assertEqual(manifest["requestedCapabilities"], [])
        self.assertEqual(manifest["displayName"], valid_inputs()["display_name"])

    def test_generated_manifest_passes_real_validator_when_available(self):
        validator = _load_validator()
        gen.generate(valid_inputs(), self.root)
        result = validator.validate(self.root, "samples/level-zero/manifest.json")
        if result["status"] == "unavailable":
            self.skipTest("trusted nagi-pkg validator not prepared ({}); manifest was NOT "
                          "validated by the real validator".format(result["reason"]))
        self.assertEqual(result["status"], "passed", result)
        self.assertEqual(result["reason"], "metadata_valid")

    def test_nested_output_directory_below_existing_parent(self):
        (self.root / "samples/group").mkdir()
        gen.generate(valid_inputs(output_dir="samples/group/app"), self.root)
        self.assertIn('path = "../../../sdk/rust"',
                      (self.root / "samples/group/app/Cargo.toml").read_text(encoding="utf-8"))

    def test_japanese_and_quoted_display_name_round_trip(self):
        name = {"en-US": 'Say "hi"\\ now', "ja-JP": "「日本語」アプリ"}
        gen.generate(valid_inputs(display_name=name), self.root)
        manifest = json.loads((self.root / "samples/level-zero/manifest.json")
                              .read_text(encoding="utf-8"))
        self.assertEqual(manifest["displayName"], name)

    def test_directories_are_widened_to_the_usual_mode_only_after_success(self):
        mask = os.umask(0o022)
        os.umask(mask)
        gen.generate(valid_inputs(), self.root)
        for relative in ("samples/level-zero", "samples/level-zero/src"):
            mode = stat.S_IMODE(os.lstat(self.root / relative).st_mode)
            self.assertEqual(mode, 0o755 & ~mask, relative)

    def test_plan_reports_files_and_writes_nothing(self):
        before = tree(self.root)
        plan = gen.plan(valid_inputs(), self.root)
        self.assertEqual(plan["status"], "planned")
        self.assertEqual([item["path"] for item in plan["files"]],
                         [e["path"] for e in gen.contract_module.load_contract()["outputs"]])
        self.assertTrue(all(item["bytes"] > 0 for item in plan["files"]))
        self.assertEqual(tree(self.root), before)

    def test_plan_applies_destination_checks(self):
        (self.root / "samples/level-zero").mkdir()
        with self.assertRaises(gen.GenerationError) as caught:
            gen.plan(valid_inputs(), self.root)
        self.assertEqual(caught.exception.code, "output_exists")


class DeterminismAndImmutabilityTests(GeneratorTestCase):
    def test_same_inputs_give_identical_bytes_in_separate_checkouts(self):
        other = self.root.parent / "checkout2"
        (other / "sdk/rust").mkdir(parents=True)
        (other / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
        (other / "samples").mkdir()
        gen.generate(valid_inputs(), self.root)
        gen.generate(valid_inputs(), other)
        self.assertEqual(tree(self.root / "samples"), tree(other / "samples"))

    def test_inputs_and_contract_are_not_mutated(self):
        inputs = valid_inputs()
        snapshot = copy.deepcopy(inputs)
        contract = gen.contract_module.load_contract()
        contract_snapshot = copy.deepcopy(contract)
        gen.plan(inputs, self.root, contract=contract)
        gen.generate(inputs, self.root, contract=contract)
        self.assertEqual(inputs, snapshot)
        self.assertEqual(contract, contract_snapshot)

    def test_rerun_is_rejected_and_leaves_first_result_untouched(self):
        gen.generate(valid_inputs(), self.root)
        before = tree(self.root)
        error = self.reason()
        self.assertEqual(error.code, "output_exists")
        self.assertEqual(error.removed, [])
        self.assertEqual(tree(self.root), before)


class RejectionTests(GeneratorTestCase):
    def assertRejected(self, code, inputs=None, root=None):
        before = tree(self.root)
        error = self.reason(inputs, root)
        self.assertEqual(error.code, code)
        self.assertEqual(tree(self.root), before, "rejection must not change the checkout")

    def test_invalid_app_id(self):
        for bad in ("", "no-dots", "Com.Example.App", "com.example.", "../x", 5):
            with self.subTest(bad=bad):
                self.assertRejected("invalid_template_input:app_id", valid_inputs(app_id=bad))

    def test_invalid_package_name_and_reserved_word(self):
        for bad in ("Level", "-x", "x--y", "match", ""):
            with self.subTest(bad=bad):
                self.assertRejected("invalid_template_input:package_name",
                                    valid_inputs(package_name=bad))

    def test_unsupported_sdk_version_unknown_input_and_profile(self):
        self.assertRejected("unsupported_sdk_package_version", valid_inputs(sdk_version="9.9.9"))
        self.assertRejected("unknown_template_input", valid_inputs(capabilities=["net"]))
        self.assertRejected("invalid_template_input:profile", valid_inputs(profile="gui"))

    def test_missing_or_incomplete_display_name(self):
        self.assertRejected("invalid_display_name",
                            valid_inputs(display_name={"en-US": "Only English"}))
        self.assertRejected("invalid_display_name",
                            valid_inputs(display_name={"en-US": "A", "ja-JP": "bad\nline"}))

    def test_output_dir_escapes(self):
        for bad in ("../escape", "samples/../../escape", "/abs/path", "samples//x", "./x",
                    "C:evil", "a\\b", ""):
            with self.subTest(bad=bad):
                error = self.reason(valid_inputs(output_dir=bad))
                self.assertEqual(error.code, "invalid_repo_relative_directory")
        self.assertEqual(os.listdir(self.outside), [])

    def test_existing_directory_even_if_empty_and_existing_file(self):
        (self.root / "samples/level-zero").mkdir()
        self.assertRejected("output_exists")
        (self.root / "samples/level-zero").rmdir()
        (self.root / "samples/level-zero").write_text("user data", encoding="utf-8")
        self.assertRejected("output_exists")
        self.assertEqual((self.root / "samples/level-zero").read_text(encoding="utf-8"),
                         "user data")

    def test_non_empty_existing_directory_is_not_merged(self):
        existing = self.root / "samples/level-zero"
        existing.mkdir()
        (existing / "manifest.json").write_text("mine", encoding="utf-8")
        self.assertRejected("output_exists")
        self.assertEqual((existing / "manifest.json").read_text(encoding="utf-8"), "mine")

    def test_symlink_as_destination_is_rejected_and_target_untouched(self):
        os.symlink(self.outside, self.root / "samples/level-zero")
        self.assertRejected("output_symlink")
        self.assertEqual(os.listdir(self.outside), [])

    def test_dangling_symlink_as_destination_is_rejected(self):
        os.symlink(self.outside / "nonexistent", self.root / "samples/level-zero")
        self.assertRejected("output_symlink")
        self.assertFalse((self.outside / "nonexistent").exists())

    def test_symlinked_ancestor_is_rejected(self):
        os.symlink(self.outside, self.root / "linked")
        self.assertRejected("symlink_in_path", valid_inputs(output_dir="linked/app"))
        self.assertEqual(os.listdir(self.outside), [])

    def test_symlinked_sdk_directory_is_not_accepted_as_sdk(self):
        os.rename(self.root / "sdk/rust", self.outside / "rust")
        os.symlink(self.outside / "rust", self.root / "sdk/rust")
        self.assertRejected("symlink_in_path")

    def test_missing_parent_is_not_created(self):
        self.assertRejected("output_parent_missing", valid_inputs(output_dir="new-parent/app"))
        self.assertFalse((self.root / "new-parent").exists())

    def test_missing_sdk_checkout(self):
        os.remove(self.root / "sdk/rust/Cargo.toml")
        self.assertRejected("sdk_checkout_not_found")

    def test_protected_trees(self):
        for bad in ("sdk/rust/app", "sdk", ".git/app", "tools/nagi-pkg/app"):
            with self.subTest(bad=bad):
                self.assertRejected("protected_output_directory", valid_inputs(output_dir=bad))

    def test_unusable_destination_root(self):
        with self.assertRaises(gen.GenerationError) as caught:
            gen.generate(valid_inputs(), self.root / "does-not-exist")
        self.assertEqual(caught.exception.code, "destination_root_unavailable")


OUT = "samples/level-zero"
CREATED_BEFORE_LIB_RS = [OUT + "/", OUT + "/Cargo.toml", OUT + "/manifest.json", OUT + "/src/"]


class FailureTests(GeneratorTestCase):
    """Failures never delete anything (enforced by the setUp guard): objects this
    run created stay in place and are reported; foreign objects are untouched."""

    def hook_open(self, name, action):
        """Call `action()` when the generator creates file `name`; it may raise."""
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == name and flags & os.O_CREAT:
                action()
            return real_open(path, flags, *args, **kwargs)
        return mock.patch.object(gen.os, "open", wrapper)

    @staticmethod
    def fail(code=errno.ENOSPC):
        def action():
            raise OSError(code, "injected")
        return action

    def assertOnlyAdded(self, before, added):
        after = tree(self.root)
        for path, data in before.items():
            self.assertEqual(after.get(path, "missing"), data, path)
        self.assertEqual(set(after) - set(before), set(added))

    def test_write_failure_deletes_nothing_and_reports_what_was_left(self):
        keep = self.root / "samples/precious.txt"
        keep.write_text("keep", encoding="utf-8")
        before = tree(self.root)
        with self.hook_open("lib.rs", self.fail()):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:ENOSPC")
        self.assertEqual(error.created, CREATED_BEFORE_LIB_RS)
        self.assertEqual(error.left_behind, CREATED_BEFORE_LIB_RS)
        self.assertEqual(error.removed, [])
        self.assertEqual(error.report()["status"], "failed")
        self.assertOnlyAdded(before, [OUT, OUT + "/Cargo.toml", OUT + "/manifest.json",
                                      OUT + "/src"])
        preview = gen.contract_module.preview(valid_inputs())
        for relative in ("Cargo.toml", "manifest.json"):
            self.assertEqual((self.root / OUT / relative).read_bytes(),
                             preview[relative].encode("utf-8"))

    def test_failure_on_first_file_leaves_only_the_new_directory(self):
        before = tree(self.root)
        with self.hook_open("Cargo.toml", self.fail()):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:ENOSPC")
        self.assertEqual(error.left_behind, [OUT + "/"])
        self.assertOnlyAdded(before, [OUT])

    def test_foreign_file_dropped_mid_run_is_untouched_and_run_fails(self):
        def drop():
            (self.root / OUT / "foreign.txt").write_text("x", encoding="utf-8")
        with self.hook_open("lib.rs", drop):
            error = self.reason()
        # Final verification sees an object it did not create and refuses success.
        self.assertEqual(error.code, "output_raced")
        self.assertEqual((self.root / OUT / "foreign.txt").read_text(encoding="utf-8"), "x")
        self.assertIn(OUT + "/", error.left_behind)
        self.assertNotIn(OUT + "/foreign.txt", error.created)

    def test_replaced_created_file_is_left_untouched_on_failure(self):
        out = self.root / OUT

        def replace_then_fail():
            (out / "Cargo.toml").unlink()
            (out / "Cargo.toml").write_text("theirs", encoding="utf-8")
            raise OSError(errno.EIO, "injected")
        with self.hook_open("lib.rs", replace_then_fail):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:EIO")
        self.assertEqual((out / "Cargo.toml").read_text(encoding="utf-8"), "theirs")
        self.assertEqual(error.removed, [])

    def test_replacement_of_a_written_file_is_detected_before_success(self):
        # Review P1 (inode reuse): our Cargo.toml is unlinked and a new file takes
        # its name. Because the creating fd is still held, the new file cannot
        # reuse our inode, and the final check refuses to report success.
        out = self.root / OUT

        def replace():
            (out / "Cargo.toml").unlink()
            (out / "Cargo.toml").write_text("theirs", encoding="utf-8")
        with self.hook_open("UNSUPPORTED_APIS.md", replace):
            error = self.reason()
        self.assertEqual(error.code, "output_raced")
        self.assertEqual((out / "Cargo.toml").read_text(encoding="utf-8"), "theirs")
        self.assertEqual(error.removed, [])

    def test_replaced_created_directory_is_detected_and_left(self):
        out = self.root / OUT

        def replace():
            (out / "src/lib.rs").unlink()
            (out / "src").rmdir()
            (out / "src").mkdir()
        with self.hook_open("README.md", replace):
            error = self.reason()
        self.assertEqual(error.code, "output_raced")
        self.assertTrue((out / "src").is_dir())
        self.assertEqual(os.listdir(out / "src"), [])
        self.assertEqual(error.removed, [])

    def test_created_handles_stay_open_until_the_run_has_finished_checking(self):
        # Review P1 (inode reuse): no created file/directory fd may be closed
        # before the last identity check and the final fchmod.
        events, created_fds = [], set()
        real_open, real_close, real_stat, real_fchmod = os.open, os.close, os.stat, os.fchmod
        real_mkdir = os.mkdir
        pending_dirs = set()

        def open_(path, flags, *args, **kwargs):
            fd = real_open(path, flags, *args, **kwargs)
            if flags & os.O_CREAT or path in pending_dirs:
                pending_dirs.discard(path)
                created_fds.add(fd)
            return fd

        def mkdir(path, *args, **kwargs):
            real_mkdir(path, *args, **kwargs)
            pending_dirs.add(path)

        def close(fd):
            events.append(("close", fd))
            return real_close(fd)

        def stat_(*args, **kwargs):
            events.append(("check",))
            return real_stat(*args, **kwargs)

        def fchmod(fd, mode):
            events.append(("check",))
            return real_fchmod(fd, mode)
        with mock.patch.object(gen.os, "open", open_), mock.patch.object(gen.os, "mkdir", mkdir), \
                mock.patch.object(gen.os, "close", close), \
                mock.patch.object(gen.os, "stat", stat_), \
                mock.patch.object(gen.os, "fchmod", fchmod):
            gen.generate(valid_inputs(), self.root)
        self.assertEqual(len(created_fds), 7)       # 2 directories + 5 files
        last_check = max(i for i, event in enumerate(events) if event[0] == "check")
        closes = {event[1]: i for i, event in enumerate(events) if event[0] == "close"}
        for fd in created_fds:
            self.assertIn(fd, closes, "created fd leaked")
            self.assertGreater(closes[fd], last_check)

    def test_exclusive_create_refuses_a_file_that_appears_concurrently(self):
        def appear():
            (self.root / OUT / "README.md").write_text("theirs", encoding="utf-8")
        with self.hook_open("README.md", appear):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:EEXIST")
        self.assertEqual((self.root / OUT / "README.md").read_text(encoding="utf-8"), "theirs")
        self.assertNotIn(OUT + "/README.md", error.created)

    def test_keyboard_interrupt_propagates_and_deletes_nothing(self):
        def interrupt():
            raise KeyboardInterrupt
        before = tree(self.root)
        with self.hook_open("lib.rs", interrupt):
            with self.assertRaises(KeyboardInterrupt):
                gen.generate(valid_inputs(), self.root)
        self.assertOnlyAdded(before, [OUT, OUT + "/Cargo.toml", OUT + "/manifest.json",
                                      OUT + "/src"])


class DirectorySwapTests(GeneratorTestCase):
    """Review P1: a directory replaced between our mkdir() and open()."""

    def swap_after_mkdir(self, target, make_replacement):
        """After the generator's mkdir() of `target`, move it away and install a replacement."""
        real_mkdir = os.mkdir
        path = self.root / target
        moved = path.parent / (path.name + ".moved-ours")

        def mkdir(name, *args, **kwargs):
            real_mkdir(name, *args, **kwargs)
            if name == path.name and not moved.exists():
                os.rename(path, moved)
                make_replacement(path)
        return mock.patch.object(gen.os, "mkdir", mkdir), moved

    def assertRefused(self, target, make_replacement, created_before):
        patcher, moved = self.swap_after_mkdir(target, make_replacement)
        replacement = self.root / target
        with patcher:
            error = self.reason()
        self.assertEqual(error.code, "output_raced")
        self.assertEqual(error.created, created_before)
        self.assertEqual(error.left_behind, created_before + [target + "/"])
        self.assertEqual(error.removed, [])
        # Our original directory and the replacement both remain; nothing was
        # written into the replacement or into what it points to.
        self.assertTrue(moved.is_dir())
        self.assertEqual(os.listdir(moved), [])
        return replacement

    def check_cases(self, target, created_before):
        def empty_default_mode(path):
            path.mkdir()                                   # mode 0o777 & ~umask

        def private_but_not_empty(path):
            path.mkdir(mode=0o700)
            (path / "theirs.txt").write_text("theirs", encoding="utf-8")

        def group_writable(path):
            path.mkdir()
            os.chmod(path, 0o770)

        def symlink(path):
            os.symlink(self.outside, path)
        for make in (empty_default_mode, private_but_not_empty, group_writable, symlink):
            with self.subTest(target=target, replacement=make.__name__):
                self.fresh_checkout()
                replacement = self.assertRefused(target, make, created_before)
                if make is private_but_not_empty:
                    self.assertEqual(os.listdir(replacement), ["theirs.txt"])
                elif make is symlink:
                    self.assertTrue(replacement.is_symlink())
                    self.assertEqual(os.listdir(self.outside), [])
                else:
                    self.assertEqual(os.listdir(replacement), [])

    def fresh_checkout(self):
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        self.root = make_checkout(Path(holder.name) / "checkout")

    def test_swap_of_the_output_directory_is_refused_and_left(self):
        self.check_cases(OUT, [])

    def test_swap_of_a_nested_directory_is_refused_and_left(self):
        self.check_cases(OUT + "/src", [OUT + "/", OUT + "/Cargo.toml", OUT + "/manifest.json"])

    def test_residual_mode_matching_swap_is_not_detectable_but_deletes_nothing(self):
        # Outside the threat model (README "脅威モデルと限界"): an adversarial
        # concurrent process is not defended against. This only documents the
        # observed behavior, not a guarantee: a same-user process that installs
        # an empty 0o700 directory of its own cannot be told apart from ours
        # (mkdir() returns no handle). Nothing is deleted or overwritten, and
        # our original directory is left where it was moved.
        patcher, moved = self.swap_after_mkdir(OUT, lambda path: path.mkdir(mode=0o700))
        with patcher:
            gen.generate(valid_inputs(), self.root)
        self.assertTrue(moved.is_dir())
        self.assertEqual(os.listdir(moved), [])


class CliTests(GeneratorTestCase):
    ARGS = ["--app-id", "com.example.level-zero", "--package-name", "level-zero",
            "--name-en", "Level Zero", "--name-ja", "レベルゼロ",
            "--output-dir", "samples/level-zero"]

    def run_cli(self, action, extra=()):
        from io import StringIO
        buffer = StringIO()
        with mock.patch("sys.stdout", buffer):
            code = gen.main([action, "--root", str(self.root)] + self.ARGS + list(extra))
        return code, json.loads(buffer.getvalue())

    def test_plan_then_create_then_rejected_rerun(self):
        code, result = self.run_cli("plan")
        self.assertEqual((code, result["status"]), (0, "planned"))
        self.assertFalse((self.root / "samples/level-zero").exists())
        code, result = self.run_cli("create")
        self.assertEqual((code, result["status"]), (0, "created"))
        code, result = self.run_cli("create")
        self.assertEqual((code, result["reason"]), (1, "output_exists"))

    def test_cli_output_has_no_host_absolute_path(self):
        code, result = self.run_cli("create")
        self.assertNotIn(str(self.root), json.dumps(result))
        code, result = self.run_cli("create")
        self.assertNotIn(str(self.root), json.dumps(result))


if __name__ == "__main__":
    unittest.main()
