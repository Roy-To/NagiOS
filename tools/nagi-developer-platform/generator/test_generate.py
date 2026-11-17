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


class GeneratorTestCase(unittest.TestCase):
    def setUp(self):
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        self.root = Path(holder.name) / "checkout"
        # Minimal stand-in for the selected checkout: generation only checks that
        # the SDK Cargo.toml exists; contents are not read.
        (self.root / "sdk/rust").mkdir(parents=True)
        (self.root / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
        (self.root / "samples").mkdir()
        self.outside = Path(holder.name) / "outside"
        self.outside.mkdir()

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


class FailureCleanupTests(GeneratorTestCase):
    def failing_open(self, fail_name):
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == fail_name and flags & os.O_CREAT:
                raise OSError(errno.ENOSPC, "injected")
            return real_open(path, flags, *args, **kwargs)
        return mock.patch.object(gen.os, "open", wrapper)

    def test_write_failure_removes_only_created_files_and_reports_them(self):
        keep = self.root / "samples/precious.txt"
        keep.write_text("keep", encoding="utf-8")
        before = tree(self.root)
        with self.failing_open("lib.rs"):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:ENOSPC")
        self.assertIn("samples/level-zero/Cargo.toml", error.created)
        self.assertIn("samples/level-zero/manifest.json", error.removed)
        self.assertIn("samples/level-zero/src/", error.removed)
        self.assertIn("samples/level-zero/", error.removed)
        self.assertEqual(error.left_behind, [])
        self.assertEqual(tree(self.root), before)
        self.assertEqual(keep.read_text(encoding="utf-8"), "keep")

    def test_failure_on_first_file_removes_new_directory(self):
        before = tree(self.root)
        with self.failing_open("Cargo.toml"):
            error = self.reason()
        self.assertEqual(error.code, "write_failed:ENOSPC")
        self.assertEqual(tree(self.root), before)
        self.assertEqual(error.report()["status"], "failed")

    def test_failure_after_data_written_does_not_delete_foreign_files(self):
        # Something else drops a file into our new directory mid-run; cleanup
        # must not delete it, and the leftover directory must be reported.
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == "lib.rs" and flags & os.O_CREAT:
                (self.root / "samples/level-zero/foreign.txt").write_text("x", encoding="utf-8")
                raise OSError(errno.EIO, "injected")
            return real_open(path, flags, *args, **kwargs)
        with mock.patch.object(gen.os, "open", wrapper):
            error = self.reason()
        self.assertTrue((self.root / "samples/level-zero/foreign.txt").exists())
        self.assertIn("samples/level-zero/", error.left_behind)
        self.assertNotIn("samples/level-zero/foreign.txt", error.removed)

    def replace_then_fail(self, replace):
        """Run the generator; just before lib.rs is created, `replace()` then fail."""
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == "lib.rs" and flags & os.O_CREAT:
                replace()
                raise OSError(errno.EIO, "injected")
            return real_open(path, flags, *args, **kwargs)
        with mock.patch.object(gen.os, "open", wrapper):
            return self.reason()

    def assertNoQuarantineLeft(self):
        self.assertEqual([n for n in os.listdir(self.root / "samples") if "cleanup" in n], [])

    def test_replaced_created_file_is_not_deleted_by_cleanup(self):
        # Review P1: a file we created is replaced by another process (new inode)
        # before a later write fails. Cleanup must keep the replacement.
        out = self.root / "samples/level-zero"

        def replace():
            (out / "Cargo.toml").unlink()
            (out / "Cargo.toml").write_text("theirs", encoding="utf-8")
        error = self.replace_then_fail(replace)
        self.assertEqual(error.code, "write_failed:EIO")
        self.assertEqual((out / "Cargo.toml").read_text(encoding="utf-8"), "theirs")
        self.assertIn("samples/level-zero/Cargo.toml", error.left_behind)
        self.assertNotIn("samples/level-zero/Cargo.toml", error.removed)
        self.assertIn("samples/level-zero/manifest.json", error.removed)
        self.assertIn("samples/level-zero/", error.left_behind)
        self.assertEqual(sorted(os.listdir(out)), ["Cargo.toml"])
        self.assertNoQuarantineLeft()

    def test_replacement_racing_the_identity_check_is_restored_not_deleted(self):
        # The swap happens after cleanup's identity check, right before the
        # move: the moved object is foreign, so it is put back, not unlinked.
        out = self.root / "samples/level-zero"
        real_rename = os.rename
        swapped = []

        def rename(src, dst, *args, **kwargs):
            if src == "Cargo.toml" and not swapped:
                swapped.append(True)
                (out / "Cargo.toml").unlink()
                (out / "Cargo.toml").write_text("theirs", encoding="utf-8")
            return real_rename(src, dst, *args, **kwargs)
        with mock.patch.object(gen.os, "rename", rename):
            error = self.replace_then_fail(lambda: None)
        self.assertEqual(swapped, [True])
        self.assertEqual((out / "Cargo.toml").read_text(encoding="utf-8"), "theirs")
        self.assertIn("samples/level-zero/Cargo.toml", error.left_behind)
        self.assertNotIn("samples/level-zero/Cargo.toml", error.removed)
        self.assertNoQuarantineLeft()

    def test_replaced_created_directory_is_not_removed(self):
        out = self.root / "samples/level-zero"

        def replace():
            (out / "src").rmdir()
            (out / "src").mkdir()
        error = self.replace_then_fail(replace)
        self.assertTrue((out / "src").is_dir())
        self.assertIn("samples/level-zero/src/", error.left_behind)
        self.assertNotIn("samples/level-zero/src/", error.removed)
        self.assertNoQuarantineLeft()

    def test_exclusive_create_refuses_a_file_that_appears_concurrently(self):
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == "README.md" and flags & os.O_CREAT:
                (self.root / "samples/level-zero/README.md").write_text("theirs", encoding="utf-8")
            return real_open(path, flags, *args, **kwargs)
        with mock.patch.object(gen.os, "open", wrapper):
            error = self.reason()
        self.assertTrue(error.code.startswith("write_failed:"))
        self.assertEqual((self.root / "samples/level-zero/README.md").read_text(encoding="utf-8"),
                         "theirs")

    def test_keyboard_interrupt_cleans_up_and_propagates(self):
        real_open = os.open

        def wrapper(path, flags, *args, **kwargs):
            if path == "lib.rs" and flags & os.O_CREAT:
                raise KeyboardInterrupt
            return real_open(path, flags, *args, **kwargs)
        before = tree(self.root)
        with mock.patch.object(gen.os, "open", wrapper):
            with self.assertRaises(KeyboardInterrupt):
                gen.generate(valid_inputs(), self.root)
        self.assertEqual(tree(self.root), before)


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
