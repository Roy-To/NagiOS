#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from attest import (
    AttestationError, archive_spike_source, build_environment, forbidden_environment,
    materialize_spike_source, reject_ambient_cargo_config, validate_output,
)
from verify_attestation import VerificationError, check_manifest, verify_attestation


SCRATCH = Path(os.environ.get("NAGI_TEST_SCRATCH", tempfile.gettempdir()))
SPIKE_REL = "experiments/nagi-aarch64-spike"


def sha(data):
    return hashlib.sha256(data).hexdigest()


class AttestationFixture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        cls.temp = tempfile.TemporaryDirectory(prefix="nagi-attest-test-", dir=SCRATCH)
        cls.base = Path(cls.temp.name)
        cls.repo = cls.base / "repo"
        cls.output = cls.base / "receipt"
        cls.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(cls.repo)], check=True)
        subprocess.run(["git", "-C", str(cls.repo), "config", "user.email", "fixture@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(cls.repo), "config", "user.name", "Fixture"], check=True)
        cls.main = cls.commit({"main.txt": b"main"})
        cls.source = cls.commit({
            "main.txt": b"main",
            f"{SPIKE_REL}/run.py": b"runner fixture",
            f"{SPIKE_REL}/attest.py": b"generator fixture",
            f"{SPIKE_REL}/rust-toolchain.toml": b'[toolchain]\nchannel = "nightly-2025-08-01"\n',
        })
        cls.tree = cls.git("rev-parse", f"{cls.source}^{{tree}}")
        cls.write_receipt()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def setUp(self):
        self.write_receipt()

    @classmethod
    def git(cls, *args):
        return subprocess.run(["git", *args], cwd=cls.repo, text=True, capture_output=True, check=True).stdout.strip()

    @classmethod
    def commit(cls, files):
        for name, data in files.items():
            path = cls.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        subprocess.run(["git", "-C", str(cls.repo), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(cls.repo), "commit", "-qm", "fixture"], check=True)
        return cls.git("rev-parse", "HEAD")

    @classmethod
    def clear_output(cls):
        if cls.output.exists():
            for path in sorted(cls.output.rglob("*"), key=lambda item: len(item.parts), reverse=True):
                if path.is_symlink():
                    path.unlink()
                elif path.is_file():
                    path.chmod(0o600)
                elif path.is_dir():
                    path.chmod(0o700)
            cls.output.chmod(0o700)
            shutil.rmtree(cls.output)
        cls.output.mkdir()

    @classmethod
    def write_receipt(cls):
        cls.clear_output()
        archive, paths = archive_spike_source(cls.repo, cls.source)
        archive_path = cls.output / "source/spike.tar"
        archive_path.parent.mkdir(parents=True)
        archive_path.write_bytes(archive)
        snapshot_root = cls.output / "source-snapshot"
        snapshot_spike = materialize_spike_source(archive, paths, snapshot_root)
        snapshot_path = "source-snapshot/" + SPIKE_REL
        archive_sha = sha(archive)

        elf = bytearray(64)
        elf[:6] = b"\x7fELF\x02\x01"
        elf[18:20] = (183).to_bytes(2, "little")
        data = bytes(elf)
        for label in ("a", "b"):
            build_dir = cls.output / f"build-{label}"
            build_dir.mkdir()
            (build_dir / "stdout.log").write_bytes(b"")
            (build_dir / "stderr.log").write_bytes(b"")
            (build_dir / "spike.elf").write_bytes(data)

        toolchain = {
            "rust_toolchain_toml": "nightly-2025-08-01",
            "rust_toolchain_toml_sha256": sha((cls.repo / f"{SPIKE_REL}/rust-toolchain.toml").read_bytes()),
            "rustup_active": "nightly-2025-08-01",
            "rustup_path": "/tools/rustup", "rustup_sha256": "1" * 64,
            "cargo_path": "/tools/cargo", "cargo_sha256": "c" * 64,
            "rustc_path": "/tools/rustc", "rustc_sha256": "d" * 64,
            "rust_lld_path": "/tools/rust-lld", "rust_lld_sha256": "e" * 64,
            "python_path": "/tools/python", "python_sha256": "f" * 64,
            "python_version": "3.14.0", "cargo_v": "cargo 1.90", "rustc_vV": "rustc 1.90",
        }
        qemu_path = "/tools/qemu-system-aarch64"
        qemu_sha = "a" * 64
        result_paths = {}
        for label, rc, outcome, virt, serial in (
            ("el1", 0, "PASS", "off", b"SPIKE ARM64 EL1 PL011 PASS\nSPIKE SVC VECTOR ERET PASS\nSPIKE GENERIC COUNTER PASS\nSPIKE COMPLETE PASS\n"),
            ("el2", 1, "FAIL", "on", b"SPIKE FAIL exception-level expected EL1 got EL2\n"),
        ):
            run_dir = cls.output / "runs" / label
            run_dir.mkdir(parents=True)
            (run_dir / "serial.log").write_bytes(serial)
            (run_dir / "stderr.log").write_bytes(b"")
            (run_dir / "runner-stdout.log").write_bytes(b"")
            (run_dir / "runner-stderr.log").write_bytes(b"")
            result = {
                "source_attestation": "NOT_CLAIMED_BY_RUNNER",
                "binary_path": str(cls.output / "build-a/spike.elf"),
                "binary_sha256": sha(data), "binary_sha256_after": sha(data),
                "qemu_path": qemu_path, "qemu_sha256": qemu_sha, "qemu_sha256_after": qemu_sha,
                "returncode": rc, "timed_out": False, "outcome": outcome, "virtualization": virt,
                "serial_sha256": sha(serial), "stderr_sha256": sha(b""),
            }
            result_path = run_dir / "result.json"
            result_path.write_text(json.dumps(result) + "\n")
            result_paths[label] = result_path

        builds = []
        for label in ("a", "b"):
            build_dir = cls.output / f"build-{label}"
            builds.append({
                "id": label, "source_commit": cls.source, "source_tree": cls.tree,
                "source_archive_sha256": archive_sha,
                "argv": [toolchain["cargo_path"], "build", "--release", "--locked", "--offline"],
                "cwd": snapshot_path, "cargo_home_seed": "empty",
                "environment_policy": "minimal-allowlist; inherited build variables discarded",
                "cargo_config_policy": "reject external .cargo/config and .cargo/config.toml in cwd ancestors; empty isolated CARGO_HOME",
                "exit_code": 0,
                "stdout_path": f"build-{label}/stdout.log", "stdout_sha256": sha(b""),
                "stderr_path": f"build-{label}/stderr.log", "stderr_sha256": sha(b""),
                "elf_path": f"build-{label}/spike.elf", "elf_sha256": sha(data), "elf_machine": 183,
            })

        runs = []
        for label, virt, rc, verdict in (("el1", "off", 0, "PASS"), ("el2", "on", 1, "MET")):
            run_dir = cls.output / "runs" / label
            prefix = f"runs/{label}/"
            runs.append({
                "id": label, "virtualization": virt, "runner_exit_code": rc, "derived_verdict": verdict,
                "argv": [toolchain["python_path"], "run.py", "--qemu", qemu_path,
                         "--binary", str(cls.output / "build-a/spike.elf")],
                "cwd": snapshot_path, "runner_source_path": snapshot_path + "/run.py",
                "result_path": prefix + "result.json", "result_json_sha256": sha(result_paths[label].read_bytes()),
                "serial_path": prefix + "serial.log", "serial_sha256": sha((run_dir / "serial.log").read_bytes()),
                "stderr_path": prefix + "stderr.log", "stderr_sha256": sha(b""),
                "runner_stdout_path": prefix + "runner-stdout.log", "runner_stdout_sha256": sha(b""),
                "runner_stderr_path": prefix + "runner-stderr.log", "runner_stderr_sha256": sha(b""),
                "binary_sha256": sha(data), "binary_sha256_after": sha(data),
                "qemu_sha256": qemu_sha, "qemu_sha256_after": qemu_sha,
            })

        manifest = {
            path.relative_to(cls.output).as_posix(): sha(path.read_bytes())
            for path in cls.output.rglob("*") if path.is_file() and path.name != "attestation.json"
        }
        record = {
            "schema": "nagi-spike-attestation/2", "mode": "build-and-run",
            "source": {
                "main": cls.main, "commit": cls.source, "tree": cls.tree,
                "head_after_build": cls.source, "tree_after_build": cls.tree,
                "tracked_clean": True, "source_clean": True,
            },
            "source_snapshot": {
                "path": snapshot_path, "archive_path": "source/spike.tar", "archive_sha256": archive_sha,
                "file_count": len(paths), "selection": "git archive of explicit source commit",
                "snapshot_permissions": "read-only during build and run",
            },
            "generator": {
                "path": f"{SPIKE_REL}/attest.py",
                "git_blob": cls.git("rev-parse", f"{cls.source}:{SPIKE_REL}/attest.py"),
                "sha256": sha(b"generator fixture"),
            },
            "runner": {
                "path": f"{SPIKE_REL}/run.py",
                "git_blob": cls.git("rev-parse", f"{cls.source}:{SPIKE_REL}/run.py"),
                "sha256": sha(b"runner fixture"),
            },
            "toolchain": toolchain,
            "builds": builds,
            "build_reproducibility": {"two_builds_match": True, "status": "MATCHED"},
            "qemu": {"path_realpath": qemu_path, "sha256": qemu_sha, "version": "QEMU fixture"},
            "runs": runs,
            "acceptance": {
                "el1": "PASS", "el2_negative": "MET", "source_bound_runs": "PASS",
                "two_build_comparison": "MATCHED", "independent_rebuild": "NOT_RUN",
                "phase_c": "PENDING", "independent_rebuild_note": "Fixture data; JSON is not a trusted signature.",
            },
            "environment": {
                "policy": "clean allowlist; inherited variables are not passed to Cargo or runner; external Cargo config files are rejected",
                "discarded_override_names": [], "inherited_environment_values_recorded": False,
                "cargo_home_isolated": True, "cargo_home_seed": "empty", "cargo_offline": True,
            },
            "files": manifest,
        }
        (cls.output / "attestation.json").write_text(json.dumps(record, indent=2) + "\n")

    def verify(self, **kwargs):
        args = {"expected_main": self.main, "expected_source_commit": self.source, "expected_source_tree": self.tree}
        args.update(kwargs)
        return verify_attestation(self.repo, self.output / "attestation.json", **args)

    def reload_receipt(self):
        return json.loads((self.output / "attestation.json").read_text())

    def save(self, record):
        (self.output / "attestation.json").write_text(json.dumps(record, indent=2) + "\n")

    def update_result(self, index, mutate):
        record = self.reload_receipt()
        item = record["runs"][index]
        path = self.output / item["result_path"]
        result = json.loads(path.read_text())
        mutate(result)
        path.write_text(json.dumps(result) + "\n")
        digest = sha(path.read_bytes())
        record["files"][item["result_path"]] = digest
        item["result_json_sha256"] = digest
        self.save(record)

    def test_valid_receipt_binds_source_snapshot_and_artifacts(self):
        result = self.verify()
        self.assertEqual(result["source"], self.source)
        self.assertEqual(result["tree"], self.tree)
        self.assertEqual(result["two_build_comparison"], "MATCHED")
        self.assertEqual(result["phase_c"], "PENDING")

    def test_source_substitution_and_tree_object_substitution_are_rejected(self):
        with self.assertRaisesRegex(VerificationError, "explicit --source-commit"):
            self.verify(expected_source_commit=self.main, expected_source_tree=self.git("rev-parse", f"{self.main}^{{tree}}"))
        with self.assertRaisesRegex(VerificationError, "tree object"):
            self.verify(expected_source_tree="0" * 40)
        record = self.reload_receipt()
        record["source"]["commit"] = self.main
        record["source"]["tree"] = self.git("rev-parse", f"{self.main}^{{tree}}")
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "explicit --source-commit"):
            self.verify()

    def test_missing_short_and_nonexistent_source_commits_are_rejected(self):
        for value in ("", self.source[:12], "f" * 40):
            with self.subTest(value=value):
                record = self.reload_receipt()
                record["source"]["commit"] = value
                self.save(record)
                with self.assertRaises(VerificationError):
                    self.verify()
                self.write_receipt()

    def test_nonancestor_source_is_rejected(self):
        subprocess.run(["git", "-C", str(self.repo), "checkout", "--quiet", "--orphan", "other-branch"], check=True)
        other = self.repo / "other.txt"
        other.write_text("other\n")
        subprocess.run(["git", "-C", str(self.repo), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "orphan"], check=True)
        try:
            with self.assertRaisesRegex(VerificationError, "not an ancestor"):
                self.verify()
        finally:
            subprocess.run(["git", "-C", str(self.repo), "checkout", "--quiet", self.source], check=True)

    def test_tree_and_dirty_claim_mismatches_are_rejected(self):
        record = self.reload_receipt()
        record["source"]["tree"] = "0" * 40
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "tree"):
            self.verify()
        self.write_receipt()
        record = self.reload_receipt()
        record["source"]["tracked_clean"] = False
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "clean"):
            self.verify()

    def test_manifest_rejects_missing_extra_symlink_and_hash_mismatch(self):
        record = self.reload_receipt()
        record["files"]["attestation.json"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "exclude"):
            check_manifest(self.output, record)
        self.write_receipt()
        (self.output / "build-a/stdout.log").unlink()
        with self.assertRaisesRegex(VerificationError, "missing"):
            self.verify()
        self.write_receipt()
        (self.output / "extra.log").write_text("extra")
        with self.assertRaisesRegex(VerificationError, "set mismatch"):
            self.verify()
        (self.output / "extra.log").unlink()
        (self.output / "linked.log").symlink_to(self.output / "build-a/spike.elf")
        with self.assertRaisesRegex(VerificationError, "symlink"):
            self.verify()

    def test_external_elf_path_and_missing_build_block_are_rejected(self):
        record = self.reload_receipt()
        record["builds"][0]["elf_path"] = "/tmp/external.elf"
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "relative"):
            self.verify()
        self.write_receipt()
        record = self.reload_receipt()
        record["builds"] = []
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "exactly build"):
            self.verify()

    def test_source_snapshot_tamper_is_rejected_even_with_updated_manifest(self):
        record = self.reload_receipt()
        file_path = self.output / "source-snapshot" / f"{SPIKE_REL}/run.py"
        file_path.chmod(0o644)
        file_path.write_bytes(b"tampered runner fixture")
        file_path.chmod(0o444)
        record["files"][file_path.relative_to(self.output).as_posix()] = sha(file_path.read_bytes())
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "differs from commit"):
            self.verify()

    def test_build_source_binding_and_elf_hash_are_verified(self):
        record = self.reload_receipt()
        record["builds"][0]["source_commit"] = self.main
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "not bound to source"):
            self.verify()
        self.write_receipt()
        record = self.reload_receipt()
        record["builds"][0]["elf_sha256"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "digest"):
            self.verify()

    def test_el2_timeout_wrong_return_and_complete_marker_are_rejected(self):
        self.update_result(1, lambda result: result.update(returncode=1, timed_out=True))
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            self.verify()
        self.write_receipt()
        self.update_result(1, lambda result: result.update(returncode=2))
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            self.verify()
        self.write_receipt()
        record = self.reload_receipt()
        item = record["runs"][1]
        serial_path = self.output / item["serial_path"]
        serial_path.write_bytes(b"SPIKE FAIL exception-level expected EL1 got EL2\nSPIKE COMPLETE PASS\n")
        item["serial_sha256"] = sha(serial_path.read_bytes())
        record["files"][item["serial_path"]] = item["serial_sha256"]
        result_path = self.output / item["result_path"]
        result = json.loads(result_path.read_text())
        result["serial_sha256"] = item["serial_sha256"]
        result_path.write_text(json.dumps(result) + "\n")
        digest = sha(result_path.read_bytes())
        item["result_json_sha256"] = digest
        record["files"][item["result_path"]] = digest
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            self.verify()

    def test_qemu_and_serial_hash_mismatches_are_rejected(self):
        self.update_result(0, lambda result: result.update(qemu_sha256="b" * 64, qemu_sha256_after="b" * 64))
        with self.assertRaisesRegex(VerificationError, "QEMU identity"):
            self.verify()
        self.write_receipt()
        record = self.reload_receipt()
        record["runs"][0]["serial_sha256"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "hash mismatch"):
            self.verify()

    def test_cli_reports_integrity_only_and_differing_builds_fail(self):
        script = Path(__file__).with_name("verify_attestation.py")

        def invoke():
            return subprocess.run([
                sys.executable, str(script), "--repo", str(self.repo),
                "--attestation", str(self.output / "attestation.json"),
                "--main", self.main, "--source-commit", self.source, "--source-tree", self.tree,
            ], capture_output=True, text=True, check=False)

        matched = invoke()
        self.assertEqual(matched.returncode, 0, matched.stderr)
        self.assertIn("RECEIPT INTEGRITY VERIFIED", matched.stdout)
        self.assertNotIn("PASS", matched.stdout)

        record = self.reload_receipt()
        build_b = self.output / record["builds"][1]["elf_path"]
        changed = bytearray(build_b.read_bytes())
        changed[30] = 1
        build_b.write_bytes(changed)
        build_b_hash = sha(bytes(changed))
        record["builds"][1]["elf_sha256"] = build_b_hash
        record["files"][record["builds"][1]["elf_path"]] = build_b_hash
        record["build_reproducibility"] = {"two_builds_match": False, "status": "DIFFERENT"}
        record["acceptance"]["two_build_comparison"] = "DIFFERENT"
        self.save(record)
        verified = self.verify()
        self.assertEqual(verified["receipt_status"], "REPRODUCIBILITY_FAILED")
        result = invoke()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("REPRODUCIBILITY FAILED", result.stdout)
        self.assertNotIn("PASS", result.stdout)

    def test_existing_output_is_refused_without_overwrite(self):
        with self.assertRaisesRegex(AttestationError, "must not already exist"):
            validate_output(self.repo, self.output, self.base)

    def test_build_environment_discards_overrides_and_reports_injection_names(self):
        temp = self.base / "clean-env"
        env = build_environment(temp, temp / "target", temp / "cargo-home", {
            "cargo_path": "/toolchain/bin/cargo", "rust_lld_path": "/toolchain/lib/rust-lld",
            "rustc_path": "/toolchain/bin/rustc",
        })
        self.assertNotIn("RUSTFLAGS", env)
        self.assertNotIn("CARGO_TARGET_AARCH64_UNKNOWN_NONE_RUSTFLAGS", env)
        self.assertNotIn("CARGO_TARGET_AARCH64_UNKNOWN_NONE_LINKER", env)
        self.assertNotIn("CARGO_UNSTABLE_BUILD_STD", env)
        self.assertEqual(env["CARGO_NET_OFFLINE"], "true")
        self.assertEqual(env["CARGO_TARGET_DIR"], str(temp / "target"))
        inherited = {
            "RUSTFLAGS": "x", "CARGO_TARGET_AARCH64_UNKNOWN_NONE_RUSTFLAGS": "x",
            "CARGO_TARGET_AARCH64_UNKNOWN_NONE_LINKER": "x", "CARGO_UNSTABLE_BUILD_STD": "x",
        }
        self.assertEqual(forbidden_environment(inherited), sorted(inherited))

    def test_parent_cargo_config_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix="cargo-config-fixture-", dir=SCRATCH) as temp:
            base = Path(temp)
            cwd = base / "build/source"
            cwd.mkdir(parents=True)
            cargo_home = base / "isolated-cargo-home"
            cargo_home.mkdir()
            config = base / ".cargo/config.toml"
            config.parent.mkdir()
            config.write_text("[build]\nrustflags = ['-Dwarnings']\n")
            with self.assertRaisesRegex(AttestationError, "ambient Cargo config"):
                reject_ambient_cargo_config(cwd, cargo_home)

    def test_generator_has_no_arbitrary_binary_input_and_requires_source_sha(self):
        source = Path(__file__).with_name("attest.py").read_text()
        self.assertNotIn('add_argument("--binary"', source)
        self.assertIn('add_argument("--source-commit", required=True)', source)
        self.assertIn('add_argument("--source-tree", required=True)', source)


if __name__ == "__main__":
    unittest.main()
