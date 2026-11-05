"""Synthetic content tests only; none of these fixtures proves guest execution."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

import m30_preflight as preflight
import release
import test_session_acceptance as fixtures


class M30PreflightTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SessionAcceptanceTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.bundle = self.root / "bundle"

    def collect(self):
        with mock.patch.object(release, "git_source_revision", return_value=fixtures.REVISION):
            return preflight.collect(self.root, fixtures.REVISION, self.fixture.image,
                                     self.fixture.evidence, self.bundle)

    def check(self, name):
        return next(check for check in self.collect()["checks"] if check["id"] == name)

    def bundle_fixture(self):
        """Build a structurally consistent bundle from synthetic bytes, not an OS."""
        self.bundle.mkdir()
        (self.bundle / release.IMAGE_NAME).write_bytes(self.fixture.image.read_bytes())
        provenance = release.image_build_provenance(self.root, self.fixture.image, fixtures.REVISION)
        build = {key: "test-only" for key in (
            "nagi_version", "kernel_build_id", "servo_revision", "mesa_revision", "llama_cpp_revision")}
        build.update(source_revision=fixtures.REVISION, reference_image_sha256=self.fixture.image_hash,
                     granite_sha256=release.GRANITE_SHA256, toolchain_versions={},
                     image_build_provenance=provenance, granite_model_bytes_bundled=True)
        (self.bundle / release.BUILD_MANIFEST_NAME).write_text(json.dumps(build))
        (self.bundle / release.SOURCE_REVISION_NAME).write_text(fixtures.REVISION + "\n")
        for _, name in release.DOC_INPUTS:
            path = self.bundle / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic documentation\n")
        self.reindex()

    def reindex(self):
        copied = [(path.relative_to(self.bundle), "synthetic fixture")
                  for path in sorted(self.bundle.rglob("*"))
                  if path.is_file() and path.name not in (release.MANIFEST_NAME, release.SUMS_NAME)]
        (self.bundle / release.MANIFEST_NAME).write_bytes(
            release.canonical_json_bytes(release.release_manifest_for(self.bundle, copied)))
        (self.bundle / release.SUMS_NAME).write_bytes(release.checksum_lines(self.bundle))

    def cli_report(self):
        result = subprocess.run([sys.executable, str(Path(preflight.__file__)),
                                 "--root", str(self.root), "--revision", fixtures.REVISION,
                                 "--image", str(self.fixture.image), "--evidence", str(self.fixture.evidence),
                                 "--bundle", str(self.bundle)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(result.stdout, "preflight must preserve diagnostic JSON")
        self.assertEqual(result.stderr, "")
        report = json.loads(result.stdout)
        self.assertEqual(report["formal_gate"], "BLOCKED")
        self.assertEqual(report["m30_acceptance"], "NOT_EVALUATED")
        return report

    def invalid_bundle_cli(self, reason):
        check = next(c for c in self.cli_report()["checks"] if c["id"] == "distribution_bundle")
        self.assertEqual(check["status"], "INVALID")
        self.assertEqual(check["content_integrity"], "NOT_CHECKED")
        self.assertIn(reason, check["reason"])
        # The direct release CLI is another consumer of these same validators.
        direct = subprocess.run([sys.executable, release.__file__, "verify", "--directory", str(self.bundle)],
                                capture_output=True, text=True)
        self.assertEqual(direct.returncode, 2, direct.stderr)
        self.assertIn(reason, direct.stderr)
        self.assertNotIn("Traceback", direct.stderr)

    def test_review_components_array_emits_invalid_json_diagnostic(self):
        self.bundle_fixture()
        path = self.bundle / release.BUILD_MANIFEST_NAME
        build = json.loads(path.read_text())
        build["image_build_provenance"]["components"] = []
        path.write_text(json.dumps(build))
        self.reindex()
        self.invalid_bundle_cli("components")

    def test_review_overflow_number_emits_invalid_json_diagnostic(self):
        self.bundle_fixture()
        path = self.bundle / release.BUILD_MANIFEST_NAME
        build = json.loads(path.read_text())
        build["toolchain_versions"] = {"rustc": "OVERFLOW_NUMBER"}
        path.write_text(json.dumps(build).replace('"OVERFLOW_NUMBER"', '1e400'))
        self.reindex()
        self.invalid_bundle_cli("nonfinite")

    def test_review_boolean_schema_emits_invalid_json_diagnostic(self):
        self.bundle_fixture()
        path = self.bundle / release.MANIFEST_NAME
        manifest = json.loads(path.read_text())
        manifest["schema_version"] = True
        path.write_text(json.dumps(manifest))
        (self.bundle / release.SUMS_NAME).write_bytes(release.checksum_lines(self.bundle))
        self.invalid_bundle_cli("schema")

    def test_schema_type_boundaries_in_release_and_build_manifests(self):
        self.bundle_fixture()
        for name in (release.MANIFEST_NAME, release.BUILD_MANIFEST_NAME):
            path = self.bundle / name
            original = path.read_text()
            for value in (True, False, 1.0, "1", None, [], {}, 0, 2):
                with self.subTest(name=name, value=value):
                    parsed = json.loads(original)
                    parsed["schema_version"] = value
                    path.write_text(json.dumps(parsed))
                    if name == release.BUILD_MANIFEST_NAME:
                        self.reindex()
                    else:
                        (self.bundle / release.SUMS_NAME).write_bytes(release.checksum_lines(self.bundle))
                    self.invalid_bundle_cli("schema")
            path.write_text(original)
            self.reindex()
        # Optional legacy build schema remains optional; actual schema integer 1 works.
        self.assertEqual(self.check("distribution_bundle")["content_integrity"], "VALIDATED_CONTRACT")
        path = self.bundle / release.BUILD_MANIFEST_NAME
        parsed = json.loads(path.read_text())
        parsed["schema_version"] = 1
        parsed["toolchain_versions"] = {"rustc": "rustc 1.97.1", "cargo": "cargo 1.97.1"}
        path.write_text(json.dumps(parsed))
        self.reindex()
        self.assertEqual(self.check("distribution_bundle")["content_integrity"], "VALIDATED_CONTRACT")

    def test_toolchain_version_types_preserve_string_contract(self):
        self.bundle_fixture()
        path = self.bundle / release.BUILD_MANIFEST_NAME
        original = json.loads(path.read_text())
        for versions in ([], None, True, {"rustc": 1e308}, {"rustc": True},
                         {"rustc": None}, {"rustc": []}, {"rustc": " "}, {"": "rustc 1"}):
            with self.subTest(versions=versions):
                parsed = copy.deepcopy(original)
                parsed["toolchain_versions"] = versions
                path.write_text(json.dumps(parsed))
                self.reindex()
                self.invalid_bundle_cli("toolchain_versions")
        # Text containing exponent syntax is text, not a numeric overflow.
        original["toolchain_versions"] = {"rustc": "synthetic version 1e400"}
        path.write_text(json.dumps(original))
        self.reindex()
        self.assertEqual(self.check("distribution_bundle")["content_integrity"], "VALIDATED_CONTRACT")

    def test_delegated_attribute_error_preserves_all_diagnostics(self):
        self.bundle_fixture()
        with mock.patch.object(release, "verify_release", side_effect=AttributeError("malformed delegated input")):
            check = self.check("distribution_bundle")
        self.assertEqual(check["status"], "INVALID")
        self.assertIn("AttributeError: malformed delegated input", check["reason"])
        self.assertEqual(self.check("execution_origin")["status"], "NOT_VERIFIED")

    def test_consistent_synthetic_content_never_proves_acceptance_or_origin(self):
        self.bundle_fixture()
        report = self.collect()
        self.assertEqual(report["m30_acceptance"], "NOT_EVALUATED")
        self.assertEqual(report["formal_gate"], "BLOCKED")
        self.assertTrue(all(check["status"] == "NOT_VERIFIED" for check in report["checks"]))
        for name in ("ordinary_session", "production_image", "distribution_bundle"):
            self.assertEqual(self.check(name)["content_integrity"], "VALIDATED_CONTRACT")
        for name in ("voice_execution", "execution_origin", "distribution_review"):
            self.assertEqual(self.check(name)["content_integrity"], "NOT_CHECKED")
        log = next(item for item in report["artifacts"] if item["path"] == "boot.log")
        self.assertEqual(log["sha256"], fixtures.digest((self.root / "boot.log").read_bytes()))

    def test_missing_inputs_are_reported_independently(self):
        self.fixture.image.unlink()
        self.fixture.evidence.unlink()
        checks = self.collect()["checks"]
        self.assertEqual(next(c for c in checks if c["id"] == "production_image")["status"], "MISSING")
        self.assertEqual(next(c for c in checks if c["id"] == "ordinary_session")["status"], "MISSING")
        distribution = [c for c in checks if c["id"].startswith("distribution_input:")]
        self.assertEqual(len(distribution), 5 + len(release.DOC_INPUTS))
        self.assertTrue(all(c["status"] == "MISSING" for c in distribution))

    def test_missing_phase_log_has_precise_missing_reason(self):
        (self.root / "restart.log").unlink()
        check = self.check("ordinary_session")
        self.assertEqual(check["status"], "MISSING")
        self.assertIn("restart.log", check["reason"])

    def test_bad_json_duplicate_and_nonfinite_are_invalid(self):
        for content in ('{', '{"phases": {}, "phases": {}}', '{"value": NaN}'):
            with self.subTest(content=content):
                self.fixture.evidence.write_text(content)
                self.assertEqual(self.check("ordinary_session")["status"], "INVALID")

    def test_tampered_content_is_invalid_even_with_success_marker(self):
        with (self.root / "boot.log").open("a") as stream:
            stream.write("M30 PASS\n")
        self.assertIn("SHA-256 mismatch", self.check("ordinary_session")["reason"])

    def test_forged_success_marker_with_recomputed_hash_is_invalid(self):
        self.fixture.events["boot"] = []
        self.fixture.write()
        path = self.root / "boot.log"
        path.write_text("M30 PASS\n")
        self.fixture.manifest["phases"]["boot"]["sha256"] = fixtures.digest(path.read_bytes())
        self.fixture.evidence.write_text(json.dumps(self.fixture.manifest))
        self.assertIn("prohibited", self.check("ordinary_session")["reason"])

    def test_mixed_source_image_session_and_disk_are_invalid(self):
        original = copy.deepcopy(self.fixture.manifest)
        for field in ("source_revision", "image_sha256", "session_id", "disk_id"):
            with self.subTest(field=field):
                self.fixture.manifest = copy.deepcopy(original)
                if field == "disk_id":
                    self.fixture.manifest["phases"]["restart"][field] = "disk:other"
                else:
                    self.fixture.manifest[field] = "f" * 40 if field == "source_revision" else "f" * 64
                self.fixture.write()
                self.assertEqual(self.check("ordinary_session")["status"], "INVALID")

    def test_partial_image_profile_is_invalid(self):
        self.fixture.info.write_text(self.fixture.info.read_text().replace(
            "production-session-v1", "signed-in-files-v1"))
        self.assertIn("partial or fixture", self.check("production_image")["reason"])
        self.assertEqual(self.check("ordinary_session")["status"], "INVALID")

    def test_symlink_and_outside_paths_are_invalid(self):
        (self.root / "link").symlink_to(self.root, target_is_directory=True)
        for value in (self.root / "link/evidence.json", self.root / "../outside", Path("../outside")):
            with self.subTest(value=value):
                with self.assertRaises(release.ReleaseError):
                    preflight.selected_path(self.root, value)
        self.fixture.evidence.unlink()
        self.fixture.evidence.symlink_to(self.root / "absent")
        self.assertEqual(self.check("ordinary_session")["status"], "INVALID")

    def test_dirty_or_wrong_git_source_does_not_hide_other_checks(self):
        with mock.patch.object(release, "git_source_revision", side_effect=release.ReleaseError("dirty source")):
            report = preflight.collect(self.root, fixtures.REVISION, self.fixture.image,
                                       self.fixture.evidence, self.bundle)
        self.assertEqual(report["checks"][0]["status"], "INVALID")
        self.assertGreater(len(report["checks"]), 10)
        with mock.patch.object(release, "git_source_revision", return_value="b" * 40):
            report = preflight.collect(self.root, fixtures.REVISION, self.fixture.image,
                                       self.fixture.evidence, self.bundle)
        self.assertIn("differs", report["checks"][0]["reason"])

    def test_bundle_bound_to_other_source_or_image_is_invalid(self):
        self.bundle_fixture()
        path = self.bundle / release.BUILD_MANIFEST_NAME
        original = json.loads(path.read_text())
        changed = copy.deepcopy(original)
        changed["source_revision"] = "b" * 40
        changed["image_build_provenance"]["source_revision"] = "b" * 40
        path.write_text(json.dumps(changed))
        (self.bundle / release.SOURCE_REVISION_NAME).write_text("b" * 40)
        self.reindex()
        self.assertIn("source differs", self.check("distribution_bundle")["reason"])
        path.write_text(json.dumps(original))
        (self.bundle / release.SOURCE_REVISION_NAME).write_text(fixtures.REVISION)
        self.reindex()
        self.fixture.image.write_bytes(b"other selected image")
        self.assertIn("image differs", self.check("distribution_bundle")["reason"])

    def test_bundle_tamper_and_duplicate_keys_fail_closed(self):
        self.bundle_fixture()
        (self.bundle / release.DOC_INPUTS[0][1]).write_text("tampered")
        self.assertEqual(self.check("distribution_bundle")["status"], "INVALID")
        (self.bundle / release.MANIFEST_NAME).write_text('{"schema_version": 1, "schema_version": 1}')
        self.assertIn("duplicate JSON key", self.check("distribution_bundle")["reason"])

    def test_cli_always_nonzero_and_emits_json_without_writing_inputs(self):
        self.bundle_fixture()
        before = {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        result = subprocess.run([sys.executable, str(Path(preflight.__file__)),
                                 "--root", str(self.root), "--revision", fixtures.REVISION,
                                 "--image", str(self.fixture.image), "--evidence", str(self.fixture.evidence),
                                 "--bundle", str(self.bundle)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(json.loads(result.stdout)["formal_gate"], "BLOCKED")
        self.assertEqual(result.stderr, "")
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.root.rglob("*") if p.is_file()})
