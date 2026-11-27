import hashlib
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from system_verity_build_inputs import (
    BuildInputError,
    executable_identity,
    resolve_rustc_path,
    resume_tool_path,
    verify_executable_identity,
    verify_manifest_tool_identity,
    validate_resume_manifest,
    write_resume_manifest,
)


class SystemVerityBuildInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.paths = {}
        for name, content in (
            ("base_config", b"base config"),
            ("final_config", b"final config"),
            ("merge_fragment", b"DM_VERITY=y\n"),
            ("builder_script", b"build script"),
            ("source_verifier", b"source verifier"),
            ("build_inputs_helper", b"build input helper"),
            ("config_validator", b"config validator"),
            ("make", b"GNU Make"),
            ("rustc", b"pinned rustc executable"),
        ):
            path = self.root / name
            path.write_bytes(content)
            if name == "rustc":
                path.chmod(0o755)
            self.paths[name] = path
        self.source_dir = self.root / "linux-6.18.63"
        self.source_dir.mkdir()
        (self.source_dir / "Makefile").write_bytes(b"VERSION = 6\n")
        self.archive = self.root / "linux-6.18.63.tar.xz"
        self.archive.write_bytes(b"pinned archive bytes")
        self.host_support = self.root / "host-support-before.json"
        self.host_support.write_bytes(b"original host support")
        self.source_identity = {
            "archive_sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "tree_sha256": "a" * 64,
        }
        self.manifest = self.root / "resume-inputs.json"
        self.tool_versions = {"rustc": "rustc 1.86.0 (05f9846f8 2025-03-31)\nLLVM version: 19.1.7"}

    def tearDown(self):
        self.temp.cleanup()

    def test_resume_manifest_accepts_identical_build_inputs(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        digest = validate_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.assertEqual(len(digest), 64)

    def test_resume_manifest_rejects_modified_final_config(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.paths["final_config"].write_bytes(b"changed config")
        with self.assertRaisesRegex(BuildInputError, "final_config"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support, self.tool_versions,
            )

    def test_resume_manifest_rejects_modified_original_host_support(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.host_support.write_bytes(b"changed host support")
        with self.assertRaisesRegex(BuildInputError, "host-support"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support, self.tool_versions,
            )

    def test_resume_manifest_rejects_changed_rustc_version(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        changed_versions = {"rustc": "rustc 1.87.0 (new build)"}
        with self.assertRaisesRegex(BuildInputError, "tool version changed: rustc"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support, changed_versions,
            )

    def test_resume_manifest_must_match_receipt_rustc_identity(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        digest = hashlib.sha256(self.paths["rustc"].read_bytes()).hexdigest()
        verify_manifest_tool_identity(
            self.manifest, "rustc", self.paths["rustc"], digest,
            self.tool_versions["rustc"],
        )
        with self.assertRaisesRegex(BuildInputError, "identity differs from the build receipt"):
            verify_manifest_tool_identity(
                self.manifest, "rustc", self.paths["rustc"], "f" * 64,
                self.tool_versions["rustc"],
            )
        with self.assertRaisesRegex(BuildInputError, "version differs from the build receipt"):
            verify_manifest_tool_identity(
                self.manifest, "rustc", self.paths["rustc"], digest,
                "rustc 1.87.0 (new build)",
            )

    def test_resume_manifest_rejects_changed_rustc_binary(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.paths["rustc"].write_bytes(b"different rustc executable")
        with self.assertRaisesRegex(BuildInputError, "resume build input changed: rustc"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support, self.tool_versions,
            )

    def test_resume_uses_the_original_canonical_rustc_path(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.assertEqual(
            resume_tool_path(self.manifest, "rustc"),
            self.paths["rustc"].resolve(strict=True),
        )

    def test_resume_refuses_changed_rustc_before_running_it(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support, self.tool_versions,
        )
        self.paths["rustc"].write_bytes(b"changed executable")
        with self.assertRaisesRegex(BuildInputError, "pinned rustc executable changed"):
            resume_tool_path(self.manifest, "rustc")

    def test_resolve_rustc_path_uses_rustup_which_for_proxy_symlink(self):
        rustup = self.root / "rustup"
        compiler = self.root / "toolchain" / "bin" / "rustc"
        compiler.parent.mkdir(parents=True)
        compiler.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        compiler.chmod(0o755)
        rustup.write_text(
            f"#!/bin/sh\n[ \"$1\" = which ] && [ \"$2\" = rustc ] || exit 41\nprintf '%s\\n' '{compiler}'\n",
            encoding="utf-8",
        )
        rustup.chmod(0o755)
        proxy_dir = self.root / "cargo-bin"
        proxy_dir.mkdir()
        proxy = proxy_dir / "rustc"
        proxy.symlink_to(rustup)

        self.assertEqual(resolve_rustc_path(str(proxy)), compiler.resolve())

    def test_resolve_rustc_path_keeps_direct_compiler(self):
        compiler = self.root / "direct-rustc"
        compiler.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        compiler.chmod(0o755)

        self.assertEqual(resolve_rustc_path(str(compiler)), compiler.resolve())

    def test_executable_identity_rejects_changed_binary(self):
        compiler = self.root / "test-rustc"
        compiler.write_text("#!/bin/sh\nprintf 'rustc fixture 1\\n'\n", encoding="utf-8")
        compiler.chmod(0o755)
        expected = executable_identity(compiler)
        compiler.write_text("#!/bin/sh\nprintf 'rustc fixture 2\\n'\n", encoding="utf-8")
        compiler.chmod(0o755)

        with self.assertRaisesRegex(BuildInputError, "pinned rustc executable changed"):
            verify_executable_identity(compiler, expected["sha256"], expected["version"], "rustc")

    def test_executable_identity_rejects_changed_version(self):
        compiler = self.root / "test-rustc-versioned"
        compiler.write_text(
            "#!/bin/sh\nprintf '%s\\n' \"$RUSTC_FIXTURE_VERSION\"\n",
            encoding="utf-8",
        )
        compiler.chmod(0o755)
        with patch.dict(os.environ, {"RUSTC_FIXTURE_VERSION": "rustc fixture 1"}):
            expected = executable_identity(compiler)

        with patch.dict(os.environ, {"RUSTC_FIXTURE_VERSION": "rustc fixture 2"}):
            with self.assertRaisesRegex(BuildInputError, "pinned rustc version changed"):
                verify_executable_identity(compiler, expected["sha256"], expected["version"], "rustc")

    def test_resume_manifest_is_required(self):
        with self.assertRaisesRegex(BuildInputError, "manifest is missing"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support, self.tool_versions,
            )


if __name__ == "__main__":
    unittest.main()
