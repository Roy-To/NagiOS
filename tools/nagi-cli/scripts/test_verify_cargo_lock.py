#!/usr/bin/env python3
"""Exercise the read-only Cargo lock preflight on a tiny local workspace."""

from __future__ import annotations

import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("verify-cargo-lock.py")
SPEC = importlib.util.spec_from_file_location("verify_cargo_lock", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import lock preflight at {SCRIPT}")
VERIFY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VERIFY
SPEC.loader.exec_module(VERIFY)

PACKER_SCRIPT = Path(__file__).with_name("pack-linux-musl-guest.py")
PACKER_SPEC = importlib.util.spec_from_file_location("lock_test_packer", PACKER_SCRIPT)
if PACKER_SPEC is None or PACKER_SPEC.loader is None:
    raise RuntimeError(f"cannot import guest packer at {PACKER_SCRIPT}")
PACKER = importlib.util.module_from_spec(PACKER_SPEC)
sys.modules[PACKER_SPEC.name] = PACKER
PACKER_SPEC.loader.exec_module(PACKER)

REPO_ROOT = SCRIPT.parents[3]


def _write_package(root: Path, relative: str, name: str, dependencies: str = "") -> None:
    package = root / relative
    (package / "src").mkdir(parents=True, exist_ok=True)
    (package / "Cargo.toml").write_text(
        "[package]\n"
        f'name = "{name}"\n'
        'version = "0.1.0"\n'
        'edition = "2021"\n'
        "\n[dependencies]\n"
        f"{dependencies}",
        encoding="utf-8",
    )
    (package / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")


class CargoLockPreflightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(
            prefix="nagi-cargo-lock-test-", dir=REPO_ROOT / "target"
        )
        self.root = Path(self.temp.name)
        self.cargo = os.environ.get("CARGO") or shutil.which("cargo")
        if not self.cargo:
            self.skipTest("Cargo is unavailable on PATH")

        (self.root / "Cargo.toml").write_text(
            '[workspace]\n'
            'members = ["tools/nagi-cli", "user/nagi-model-manager"]\n'
            'resolver = "2"\n',
            encoding="utf-8",
        )
        _write_package(
            self.root,
            "tools/nagi-cli",
            "nagi-cli",
            'nagi-model-manager = { path = "../../user/nagi-model-manager" }\n',
        )
        _write_package(self.root, "user/nagi-model-manager", "nagi-model-manager")
        lock = subprocess.run(
            [
                self.cargo,
                "generate-lockfile",
                "--offline",
                "--manifest-path",
                str(self.root / "Cargo.toml"),
            ],
            cwd=self.root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        self.assertEqual(lock.returncode, 0, lock.stderr)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_current_workspace_lock_passes_without_mutation(self) -> None:
        lock = self.root / "Cargo.lock"
        before = lock.read_bytes()
        result = VERIFY.validate(self.cargo, self.root / "Cargo.toml")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(lock.read_bytes(), before)

    def test_preflight_is_fingerprinted_as_build_support(self) -> None:
        fingerprints = PACKER._fingerprint_build_support(REPO_ROOT)
        self.assertIn("tools/nagi-cli/scripts/verify-cargo-lock.py", fingerprints)

    def test_new_workspace_member_fails_closed_until_lock_is_refreshed(self) -> None:
        cli_manifest = self.root / "tools/nagi-cli/Cargo.toml"
        cli_before = cli_manifest.read_bytes()
        scheduler_path = "crates/nagi-job-scheduler"
        _write_package(
            self.root,
            scheduler_path,
            "nagi-job-scheduler",
            'nagi-model-manager = { path = "../../user/nagi-model-manager" }\n',
        )
        root_manifest = self.root / "Cargo.toml"
        root_manifest.write_text(
            '[workspace]\n'
            'members = ["tools/nagi-cli", "user/nagi-model-manager", '
            '"crates/nagi-job-scheduler"]\n'
            'resolver = "2"\n',
            encoding="utf-8",
        )

        lock = self.root / "Cargo.lock"
        stale_lock = lock.read_bytes()
        result = VERIFY.validate(self.cargo, root_manifest)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            VERIFY.classify_result(result.returncode, result.stderr, lock),
            "lock-update-required",
        )
        self.assertEqual(lock.read_bytes(), stale_lock)
        self.assertEqual(cli_manifest.read_bytes(), cli_before)

        helper = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--cargo",
                self.cargo,
                "--manifest-path",
                str(root_manifest),
            ],
            cwd=self.root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(helper.returncode, result.returncode)
        self.assertEqual(helper.stderr, result.stderr)
        self.assertEqual(
            helper.stdout.strip(), b"METADATA_CLASSIFICATION=lock-update-required"
        )

        refreshed = subprocess.run(
            [
                self.cargo,
                "generate-lockfile",
                "--offline",
                "--manifest-path",
                str(root_manifest),
            ],
            cwd=self.root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        self.assertEqual(refreshed.returncode, 0, refreshed.stderr)
        accepted = VERIFY.validate(self.cargo, root_manifest)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)

    def test_name_only_offline_index_miss_remains_unknown(self) -> None:
        root = self.root / "offline-registry"
        (root / "src").mkdir(parents=True)
        (root / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        manifest = root / "Cargo.toml"
        manifest.write_text(
            "[workspace]\n"
            "[package]\n"
            'name = "offline-probe"\n'
            'version = "0.1.0"\n'
            'edition = "2021"\n'
            "[dependencies]\n"
            'serde = "=1.0.229"\n',
            encoding="utf-8",
        )
        generated = subprocess.run(
            [
                self.cargo,
                "generate-lockfile",
                "--offline",
                "--manifest-path",
                str(manifest),
            ],
            cwd=root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if generated.returncode:
            self.skipTest(
                "host Cargo index cannot generate the already-locked serde fixture"
            )

        lock = root / "Cargo.lock"
        before = lock.read_bytes()
        empty_cargo_home = root / "empty-cargo-home"
        empty_cargo_home.mkdir()
        env = dict(os.environ, CARGO_HOME=str(empty_cargo_home))
        result = VERIFY.validate(self.cargo, manifest, env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"location searched: crates.io index", result.stderr)
        self.assertIn(b"using offline mode (--offline)", result.stderr)
        self.assertEqual(
            VERIFY.classify_result(result.returncode, result.stderr, lock),
            "unknown",
        )
        self.assertEqual(lock.read_bytes(), before)

    def test_offline_cache_classification_requires_exact_lock_identity(self) -> None:
        root = self.root / "offline-download"
        (root / "src").mkdir(parents=True)
        (root / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        manifest = root / "Cargo.toml"
        manifest.write_text(
            "[workspace]\n"
            "[package]\n"
            'name = "offline-download-probe"\n'
            'version = "0.1.0"\n'
            'edition = "2021"\n'
            "[dependencies]\n"
            'serde = "=1.0.229"\n',
            encoding="utf-8",
        )
        generated = subprocess.run(
            [
                self.cargo,
                "generate-lockfile",
                "--offline",
                "--manifest-path",
                str(manifest),
            ],
            cwd=root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if generated.returncode:
            self.skipTest(
                "host Cargo index cannot generate the already-locked serde fixture"
            )

        cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
        registry_index = cargo_home / "registry" / "index"
        if not registry_index.is_dir():
            self.skipTest("host Cargo registry index is unavailable for offline setup")
        isolated_home = root / "isolated-cargo-home"
        isolated_home.mkdir()
        (isolated_home / "registry").mkdir()
        shutil.copytree(registry_index, isolated_home / "registry" / "index")
        env = dict(os.environ, CARGO_HOME=str(isolated_home))
        fetched = subprocess.run(
            [
                self.cargo,
                "fetch",
                "--locked",
                "--offline",
                "--manifest-path",
                str(manifest),
            ],
            cwd=root,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertNotEqual(fetched.returncode, 0)
        self.assertIn(
            b"attempting to make an HTTP request, but --offline was specified",
            fetched.stderr,
        )
        identity = re.search(
            rb"(?m)^error: failed to download `([^` ]+) v([^`]+)`$", fetched.stderr
        )
        self.assertIsNotNone(identity, fetched.stderr)
        name, version = identity.groups()
        lock = root / "Cargo.lock"
        self.assertEqual(
            VERIFY.classify_result(fetched.returncode, fetched.stderr, lock),
            "offline-cache-insufficient",
        )

        mismatched_version = root / "mismatched-version.lock"
        mismatched_version.write_bytes(
            b'version = 4\n\n[[package]]\nname = "'
            + name
            + b'"\nversion = "0.0.0"\nsource = "'
            + VERIFY.CRATES_IO_SOURCE
            + b'"\n'
        )
        self.assertEqual(
            VERIFY.classify_result(
                fetched.returncode, fetched.stderr, mismatched_version
            ),
            "unknown",
        )

        mismatched_source = root / "mismatched-source.lock"
        mismatched_source.write_bytes(
            b'version = 4\n\n[[package]]\nname = "'
            + name
            + b'"\nversion = "'
            + version
            + b'"\nsource = "registry+https://mirror.invalid/index"\n'
        )
        self.assertEqual(
            VERIFY.classify_result(
                fetched.returncode, fetched.stderr, mismatched_source
            ),
            "unknown",
        )

    def test_offline_unknown_package_remains_unknown(self) -> None:
        root = self.root / "unknown-registry"
        (root / "src").mkdir(parents=True)
        (root / "src/lib.rs").write_text("pub fn fixture() {}\n", encoding="utf-8")
        manifest = root / "Cargo.toml"
        manifest.write_text(
            "[workspace]\n"
            "[package]\n"
            'name = "unknown-probe"\n'
            'version = "0.1.0"\n'
            'edition = "2021"\n'
            "[dependencies]\n"
            'nagi-unresolved-registry-probe = "0.1"\n',
            encoding="utf-8",
        )
        empty_cargo_home = root / "empty-cargo-home"
        empty_cargo_home.mkdir()
        env = dict(os.environ, CARGO_HOME=str(empty_cargo_home))
        result = VERIFY.validate(self.cargo, manifest, env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"no matching package named", result.stderr)
        self.assertIn(b"using offline mode (--offline)", result.stderr)
        self.assertEqual(
            VERIFY.classify_result(
                result.returncode, result.stderr, root / "Cargo.lock"
            ),
            "unknown",
        )

    def test_other_cargo_metadata_error_has_generic_classification(self) -> None:
        manifest = self.root / "tools/nagi-cli/Cargo.toml"
        manifest.write_text("[package\nthis is invalid TOML\n", encoding="utf-8")
        result = VERIFY.validate(self.cargo, self.root / "Cargo.toml")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(result.stderr.startswith(b"error:"))
        self.assertEqual(
            VERIFY.classify_result(
                result.returncode, result.stderr, self.root / "Cargo.lock"
            ),
            "metadata-failure",
        )


if __name__ == "__main__":
    unittest.main()
