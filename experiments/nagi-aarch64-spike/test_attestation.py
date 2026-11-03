#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from verify_attestation import VerificationError, check_manifest, verify_attestation
from attest import AttestationError, validate_output


SCRATCH = Path(os.environ.get("NAGI_TEST_SCRATCH", tempfile.gettempdir()))


def sha(data):
    return hashlib.sha256(data).hexdigest()


class AttestationFixture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="nagi-attest-test-", dir=SCRATCH)
        cls.base = Path(cls.temp.name)
        cls.repo = cls.base / "repo"
        cls.output = cls.base / "receipt"
        cls.repo.mkdir()
        cls.output.mkdir()
        subprocess.run(["git", "init", "-q", str(cls.repo)], check=True)
        subprocess.run(["git", "-C", str(cls.repo), "config", "user.email", "fixture@example.invalid"], check=True)
        subprocess.run(["git", "-C", str(cls.repo), "config", "user.name", "Fixture"], check=True)
        cls.main = cls.commit({"main.txt": b"main"})
        cls.source = cls.commit({
            "main.txt": b"main",
            "experiments/nagi-aarch64-spike/run.py": b"runner fixture",
            "experiments/nagi-aarch64-spike/attest.py": b"generator fixture",
            "experiments/nagi-aarch64-spike/rust-toolchain.toml": b'[toolchain]\nchannel = "nightly-2025-08-01"\n',
        })
        cls.tree = cls.git("rev-parse", f"{cls.source}^{{tree}}")
        cls.write_receipt()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

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
    def write_receipt(cls):
        elf = bytearray(64)
        elf[:6] = b"\x7fELF\x02\x01"
        elf[18:20] = (183).to_bytes(2, "little")
        data = bytes(elf)
        cls.output.mkdir(exist_ok=True)
        for name, content in {
            "build-a.elf": data,
            "build-b.elf": data,
            "a.out": b"",
            "a.err": b"",
            "b.out": b"",
            "b.err": b"",
            "serial1.log": b"SPIKE ARM64 EL1 PL011 PASS\nSPIKE SVC VECTOR ERET PASS\nSPIKE GENERIC COUNTER PASS\nSPIKE COMPLETE PASS\n",
            "serial2.log": b"SPIKE FAIL exception-level expected EL1 got EL2\n",
            "stderr1.log": b"",
            "stderr2.log": b"",
            "runner1.out": b"",
            "runner1.err": b"",
            "runner2.out": b"",
            "runner2.err": b"",
        }.items():
            (cls.output / name).write_bytes(content)

        def result(label, rc, outcome, serial_name, virt, elf_sha):
            return {
                "source_attestation": "NOT_CLAIMED_BY_RUNNER",
                "binary_path": str(cls.output / ("build-a.elf")),
                "binary_sha256": elf_sha,
                "binary_sha256_after": elf_sha,
                "qemu_path": "/tools/qemu-system-aarch64",
                "qemu_sha256": "a" * 64,
                "qemu_sha256_after": "a" * 64,
                "returncode": rc,
                "timed_out": False,
                "outcome": outcome,
                "virtualization": virt,
                "serial_sha256": sha((cls.output / serial_name).read_bytes()),
                "stderr_sha256": sha(b""),
            }

        results = {
            "runs/el1/result.json": result("el1", 0, "PASS", "serial1.log", "off", sha(data)),
            "runs/el2/result.json": result("el2", 1, "FAIL", "serial2.log", "on", sha(data)),
        }
        for path, value in results.items():
            target = cls.output / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps(value) + "\n")
        cls.manifest = {
            path.relative_to(cls.output).as_posix(): sha(path.read_bytes())
            for path in cls.output.rglob("*") if path.is_file() and path.name != "attestation.json"
        }
        record = {
            "schema": "nagi-spike-attestation/1",
            "mode": "build-and-run",
            "source": {
                "main": cls.main, "commit": cls.source, "tree": cls.tree,
                "head_after_build": cls.source, "tree_after_build": cls.tree,
                "tracked_clean": True, "source_clean": True,
            },
            "generator": {
                "path": "experiments/nagi-aarch64-spike/attest.py",
                "git_blob": cls.git("rev-parse", f"{cls.source}:experiments/nagi-aarch64-spike/attest.py"),
                "sha256": sha(b"generator fixture"),
            },
            "runner": {
                "path": "experiments/nagi-aarch64-spike/run.py",
                "git_blob": cls.git("rev-parse", f"{cls.source}:experiments/nagi-aarch64-spike/run.py"),
                "sha256": sha(b"runner fixture"),
            },
            "toolchain": {
                "rust_toolchain_toml": "nightly-2025-08-01",
                "rust_toolchain_toml_sha256": sha((cls.repo / "experiments/nagi-aarch64-spike/rust-toolchain.toml").read_bytes()),
                "rustup_active": "nightly-2025-08-01",
                "cargo_path": "/tools/cargo",
                "cargo_sha256": "c" * 64,
                "rustc_path": "/tools/rustc",
                "rustc_sha256": "d" * 64,
                "rust_lld_sha256": "e" * 64,
            },
            "builds": [
                {"id": label, "source_commit": cls.source, "source_tree": cls.tree,
                 "argv": ["cargo", "build", "--release", "--locked", "--offline"],
                 "exit_code": 0, "stdout_path": f"{label}.out", "stdout_sha256": sha(b""),
                 "stderr_path": f"{label}.err", "stderr_sha256": sha(b""),
                 "elf_path": f"build-{label}.elf", "elf_sha256": sha(data), "elf_machine": 183}
                for label in ("a", "b")
            ],
            "build_reproducibility": {"two_builds_match": True},
            "qemu": {"path_realpath": "/tools/qemu-system-aarch64", "sha256": "a" * 64},
            "runs": [
                {"id": "el1", "virtualization": "off", "runner_exit_code": 0, "derived_verdict": "PASS",
                 "argv": ["python3", "run.py", "--qemu", "/tools/qemu-system-aarch64", "--binary", str(cls.output / "build-a.elf")],
                 "result_path": "runs/el1/result.json", "result_json_sha256": sha((cls.output / "runs/el1/result.json").read_bytes()),
                 "serial_path": "serial1.log", "serial_sha256": sha((cls.output / "serial1.log").read_bytes()),
                 "stderr_path": "stderr1.log", "stderr_sha256": sha(b""),
                 "runner_stdout_path": "runner1.out", "runner_stdout_sha256": sha(b""),
                 "runner_stderr_path": "runner1.err", "runner_stderr_sha256": sha(b""),
                 "binary_sha256": sha(data), "binary_sha256_after": sha(data),
                 "qemu_sha256": "a"*64, "qemu_sha256_after": "a"*64},
                {"id": "el2", "virtualization": "on", "runner_exit_code": 1, "derived_verdict": "MET",
                 "argv": ["python3", "run.py", "--qemu", "/tools/qemu-system-aarch64", "--binary", str(cls.output / "build-a.elf")],
                 "result_path": "runs/el2/result.json", "result_json_sha256": sha((cls.output / "runs/el2/result.json").read_bytes()),
                 "serial_path": "serial2.log", "serial_sha256": sha((cls.output / "serial2.log").read_bytes()),
                 "stderr_path": "stderr2.log", "stderr_sha256": sha(b""),
                 "runner_stdout_path": "runner2.out", "runner_stdout_sha256": sha(b""),
                 "runner_stderr_path": "runner2.err", "runner_stderr_sha256": sha(b""),
                 "binary_sha256": sha(data), "binary_sha256_after": sha(data),
                 "qemu_sha256": "a"*64, "qemu_sha256_after": "a"*64},
            ],
            "acceptance": {"el1": "PASS", "el2_negative": "MET", "independent_rebuild": "NOT_RUN",
                           "two_build_comparison": "MATCHED"},
        }
        record["files"] = cls.manifest
        (cls.output / "attestation.json").write_text(json.dumps(record, indent=2) + "\n")

    def reload_receipt(self):
        return json.loads((self.output / "attestation.json").read_text())

    def save(self, record):
        (self.output / "attestation.json").write_text(json.dumps(record, indent=2) + "\n")

    def test_valid_receipt_binds_source_and_manifests_exact_artifacts(self):
        result = verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.assertEqual(result["source"], self.source)

    def test_attestation_self_hash_is_rejected(self):
        record = self.reload_receipt()
        record["files"]["attestation.json"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "exclude"):
            check_manifest(self.output, record)
        self.write_receipt()

    def test_missing_manifest_file_is_rejected(self):
        (self.output / "runner1.out").unlink()
        with self.assertRaisesRegex(VerificationError, "missing"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_extra_manifest_file_is_rejected(self):
        (self.output / "unexpected").write_bytes(b"extra")
        with self.assertRaisesRegex(VerificationError, "set mismatch"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        (self.output / "unexpected").unlink()

    def test_external_elf_path_is_rejected(self):
        record = self.reload_receipt()
        record["builds"][0]["elf_path"] = "/tmp/external.elf"
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "relative"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_elf_hash_mismatch_is_rejected(self):
        record = self.reload_receipt()
        record["builds"][0]["elf_sha256"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "digest"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_el2_timeout_wrong_rc_and_complete_marker_are_rejected(self):
        record = self.reload_receipt()
        path = self.output / "runs/el2/result.json"
        result = json.loads(path.read_text())
        result["returncode"] = 1
        result["timed_out"] = True
        path.write_text(json.dumps(result) + "\n")
        record["files"]["runs/el2/result.json"] = sha(path.read_bytes())
        record["runs"][1]["result_json_sha256"] = sha(path.read_bytes())
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

        serial = self.output / "serial2.log"
        serial.write_bytes(b"SPIKE FAIL exception-level expected EL1 got EL2\nSPIKE COMPLETE PASS\n")
        record = self.reload_receipt()
        record["files"]["serial2.log"] = sha(serial.read_bytes())
        record["runs"][1]["serial_sha256"] = sha(serial.read_bytes())
        result_path = self.output / record["runs"][1]["result_path"]
        result = json.loads(result_path.read_text())
        result["serial_sha256"] = sha(serial.read_bytes())
        result_path.write_text(json.dumps(result) + "\n")
        result_digest = sha(result_path.read_bytes())
        record["files"][record["runs"][1]["result_path"]] = result_digest
        record["runs"][1]["result_json_sha256"] = result_digest
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_nonancestor_source_is_rejected(self):
        other = self.repo / "other"
        other.write_text("no")
        subprocess.run(["git", "-C", str(self.repo), "checkout", "--quiet", "--orphan", "other-branch"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "rm", "-rf", "--quiet", "."], check=False, capture_output=True)
        other.write_text("other")
        subprocess.run(["git", "-C", str(self.repo), "add", "-A"], check=True)
        subprocess.run(["git", "-C", str(self.repo), "commit", "-qm", "orphan"], check=True)
        try:
            with self.assertRaisesRegex(VerificationError, "not an ancestor"):
                verify_attestation(self.repo, self.output / "attestation.json", self.main)
        finally:
            subprocess.run(["git", "-C", str(self.repo), "checkout", "--quiet", self.source], check=True)

    def test_missing_short_and_nonexistent_source_commits_are_rejected(self):
        for value in ("", self.source[:12], "f" * 40):
            with self.subTest(value=value):
                record = self.reload_receipt()
                record["source"]["commit"] = value
                self.save(record)
                with self.assertRaises(VerificationError):
                    verify_attestation(self.repo, self.output / "attestation.json", self.main)
                self.write_receipt()

    def test_source_tree_mismatch_and_dirty_claim_are_rejected(self):
        record = self.reload_receipt()
        record["source"]["tree"] = "0" * 40
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "tree"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

        record = self.reload_receipt()
        record["source"]["tracked_clean"] = False
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "clean"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_missing_build_block_is_rejected(self):
        record = self.reload_receipt()
        record["builds"] = []
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "exactly build"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_qemu_and_serial_hash_mismatches_are_rejected(self):
        record = self.reload_receipt()
        result_path = self.output / record["runs"][0]["result_path"]
        result = json.loads(result_path.read_text())
        result["qemu_sha256"] = "b" * 64
        result["qemu_sha256_after"] = "b" * 64
        result_path.write_text(json.dumps(result) + "\n")
        digest = sha(result_path.read_bytes())
        record["files"][record["runs"][0]["result_path"]] = digest
        record["runs"][0]["result_json_sha256"] = digest
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "QEMU identity"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

        record = self.reload_receipt()
        record["runs"][0]["serial_sha256"] = "0" * 64
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "hash mismatch"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_el2_wrong_return_code_is_rejected(self):
        record = self.reload_receipt()
        result_path = self.output / record["runs"][1]["result_path"]
        result = json.loads(result_path.read_text())
        result["returncode"] = 2
        result_path.write_text(json.dumps(result) + "\n")
        digest = sha(result_path.read_bytes())
        record["files"][record["runs"][1]["result_path"]] = digest
        record["runs"][1]["result_json_sha256"] = digest
        self.save(record)
        with self.assertRaisesRegex(VerificationError, "expected-negative"):
            verify_attestation(self.repo, self.output / "attestation.json", self.main)
        self.write_receipt()

    def test_existing_output_is_refused_without_overwrite(self):
        with self.assertRaisesRegex(AttestationError, "must not already exist"):
            validate_output(self.repo, self.output, self.base)

    def test_generator_cli_has_no_arbitrary_binary_input(self):
        source = (Path(__file__).parent / "attest.py").read_text()
        self.assertNotIn('add_argument("--binary"', source)
        self.assertIn('add_argument("--source-commit", required=True)', source)


if __name__ == "__main__":
    unittest.main()
