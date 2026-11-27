import json
import os
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from generate_release_key import generate
from release_signing import DEV_PUBLIC_KEY, SigningError, open_private, private_location, sha256
from sign_release_materials import sign


class PrivatePathTests(unittest.TestCase):
    def test_rejects_checkout_linked_worktree_bare_repo_and_codex(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch).resolve()
            repo = root / "repo"
            linked = root / "linked"
            bare = root / "bare"
            codex = root / ".CoDeX"
            admin = root / "admin"
            for path in (repo, linked, bare, codex, admin):
                path.mkdir()
            (repo / ".git").mkdir()
            (linked / ".git").write_text("gitdir: /unused\n")
            (bare / "HEAD").touch()
            (bare / "objects").mkdir()
            (admin / "HEAD").touch()
            (admin / "commondir").write_text("..\n")
            outside = root / "private"
            outside.mkdir()
            (root / "alias").symlink_to(linked, target_is_directory=True)
            (linked / "alias").symlink_to(outside, target_is_directory=True)
            for path in (repo, linked, bare, codex, admin, root / "alias", linked / "alias"):
                with self.subTest(path=path.name), self.assertRaises(SigningError):
                    with private_location(path / "key.pem"):
                        self.fail("protected path was accepted")

    def test_rejects_loose_permissions_and_hardlinks(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch).resolve()
            dummy = root / "plain"
            dummy.touch(mode=0o644)
            with private_location(dummy) as (directory, name):
                with self.assertRaises(SigningError):
                    open_private(directory, name)
                dummy.chmod(0o600)
                (root / "alias").hardlink_to(dummy)
                with self.assertRaises(SigningError):
                    open_private(directory, name)


@unittest.skipUnless(shutil.which("openssl"), "OpenSSL 3 required")
class SyntheticSigningTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name).resolve()
        self.secret = b"temporary test passphrase"
        self.private = self.root / "key.pem"
        self.public = self.root / "public.raw"
        self.raw = generate(self.private, self.public, self.secret)
        self.material = self.root / "catalog.preimage"
        self.material.write_bytes(b"Nagi Linux boot system service catalog v1\0reviewed bytes")
        self.manifest = self.root / "manifest.json"
        self.write_manifest(self.material.name)
        self.args = SimpleNamespace(
            private_key=self.private,
            public_key=self.public,
            public_key_sha256=sha256(self.raw),
            manifest=self.manifest,
            manifest_sha256=sha256(self.manifest.read_bytes()),
            output_directory=self.root / "signatures",
        )

    def write_manifest(self, name):
        self.manifest.write_text(json.dumps({
            "schema_version": 1,
            "materials": [{"file": name, "sha256": sha256(self.material.read_bytes())}],
        }))

    def test_generates_private_encrypted_file_and_signs_reviewed_material(self):
        self.assertEqual(self.private.stat().st_mode & 0o777, 0o600)
        with self.private.open("rb") as stream:
            self.assertEqual(stream.readline(), b"-----BEGIN ENCRYPTED PRIVATE KEY-----\n")
        self.assertEqual(self.public.read_bytes(), self.raw)
        with patch("sign_release_materials.passphrase", return_value=self.secret) as prompt:
            sign(self.args)
        prompt.assert_called_once_with()
        signature = (self.args.output_directory / (self.material.name + ".sig")).read_bytes()
        self.assertEqual(len(signature), 64)

    def test_rejects_public_key_mismatch_and_test_vector(self):
        for raw in (DEV_PUBLIC_KEY, bytes.fromhex("01" * 32)):
            self.public.write_bytes(raw)
            self.args.public_key_sha256 = sha256(raw)
            with patch("sign_release_materials.passphrase", return_value=self.secret):
                with self.assertRaises(SigningError):
                    sign(self.args)
            self.assertFalse(self.args.output_directory.exists())

    def test_rejects_material_drift_and_private_key_as_material(self):
        self.material.write_bytes(b"changed")
        with patch("sign_release_materials.passphrase") as prompt:
            with self.assertRaises(SigningError):
                sign(self.args)
            self.write_manifest(self.private.name)
            self.args.manifest_sha256 = sha256(self.manifest.read_bytes())
            with self.assertRaisesRegex(SigningError, "separate from the private key"):
                sign(self.args)
        prompt.assert_not_called()
        self.assertFalse(self.args.output_directory.exists())

    def test_does_not_overwrite_existing_key(self):
        with self.assertRaises(FileExistsError):
            generate(self.private, self.root / "other.raw", self.secret)
        self.assertTrue(self.private.is_file())


if __name__ == "__main__":
    unittest.main()
