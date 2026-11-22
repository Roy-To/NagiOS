"""Host-only regressions for the non-launching Task24 paired-run preflight."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest.mock import patch


TOOL = Path(__file__).resolve().parents[1] / "task24_run_preflight.py"
SPEC = importlib.util.spec_from_file_location("task24_run_preflight", TOOL)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {TOOL}")
preflight_tool = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(preflight_tool)


class Task24RunPreflightTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="task24-paired-preflight-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.run_id = "task24-host-regression"
        self.run_directory = self.root / self.run_id
        self.run_directory.mkdir()
        (self.run_directory / "save").mkdir()
        (self.run_directory / "read").mkdir()

        artifacts_dir = self.root / "fixed-candidate"
        artifacts_dir.mkdir()
        artifacts = {}
        for name, content in (
            ("kernel", b"synthetic bzImage candidate"),
            ("initramfs", b"synthetic normal initramfs"),
            ("system_image", b"synthetic fixed System image"),
        ):
            path = artifacts_dir / f"{name}.bin"
            path.write_bytes(content)
            artifacts[name] = {
                "path": path.resolve().as_posix(),
                "sha256": hashlib.sha256(content).hexdigest(),
            }

        data_path = self.run_directory / "data.raw"
        data_path.write_bytes(b"synthetic fresh Data image")
        data_uuid = "12345678-1234-4234-8234-123456789abc"
        self.manifest: dict[str, Any] = {
            "schema": preflight_tool.SCHEMA,
            "run_id": self.run_id,
            "run_directory": self.run_directory.as_posix(),
            "candidate_source_commit": "a" * 40,
            "artifacts": artifacts,
            "data_work_copy": {
                "path": data_path.resolve().as_posix(),
                "created_for_run_id": self.run_id,
                "provisioning": "fresh-ext4",
                "filesystem_uuid": data_uuid,
            },
            "phases": {},
            "interactive_start_authorized": False,
        }
        for phase, timeout in (("save", 420), ("read", 420)):
            self.manifest["phases"][phase] = {
                **{
                    name: dict(artifact)
                    for name, artifact in artifacts.items()
                },
                "data_image": data_path.resolve().as_posix(),
                "filesystem_uuid": data_uuid,
                "serial_path": (self.run_directory / phase / "serial.log").as_posix(),
                "timeout_seconds": timeout,
                "monitor": "none",
                "qmp_enabled": False,
                "serial_transport": "file",
                "interactive_input": "manual",
                "start_authorized": False,
                "system_read_only": True,
                "data_read_only": False,
            }
        self.manifest_path = self.run_directory / "preflight.json"
        self._write_manifest()

    def _write_manifest(self) -> None:
        self.manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")

    def test_accepts_same_candidate_and_same_new_data_with_manual_bounded_phases(self) -> None:
        before = {
            path.relative_to(self.root).as_posix(): path.read_bytes()
            for path in self.root.rglob("*")
            if path.is_file()
        }

        result = preflight_tool.preflight(self.manifest_path)

        self.assertEqual(result["preflight"], "PASS")
        self.assertTrue(result["same_system_candidate"])
        self.assertTrue(result["same_data_work_copy"])
        self.assertEqual(result["data_freshness"], "manifest-declared fresh-ext4")
        self.assertEqual(result["serial_files_separate"], True)
        self.assertEqual(result["phase_timeouts_seconds"], {"save": 420, "read": 420})
        self.assertFalse(result["qmp_enabled"])
        self.assertFalse(result["interactive_start_authorized"])
        self.assertFalse(result["qemu_started"])
        after = {
            path.relative_to(self.root).as_posix(): path.read_bytes()
            for path in self.root.rglob("*")
            if path.is_file()
        }
        self.assertEqual(after, before, "preflight must not write or launch")

    def test_preflight_only_cli_reports_without_launching(self) -> None:
        output = io.StringIO()
        with patch.object(
            sys,
            "argv",
            [
                TOOL.as_posix(),
                "--manifest",
                self.manifest_path.as_posix(),
                "--preflight-only",
            ],
        ), redirect_stdout(output):
            self.assertEqual(preflight_tool.main(), 0)

        result = json.loads(output.getvalue())
        self.assertEqual(result["preflight"], "PASS")
        self.assertFalse(result["qemu_started"])

    def test_rejects_a_read_phase_using_another_data_copy(self) -> None:
        other_data = self.run_directory / "other-data.raw"
        other_data.write_bytes(b"another synthetic Data image")
        self.manifest["phases"]["read"]["data_image"] = other_data.as_posix()
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "same new Data work copy"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_a_different_system_candidate_in_read_phase(self) -> None:
        other = self.root / "fixed-candidate" / "other-initramfs.bin"
        other.write_bytes(b"different initramfs")
        self.manifest["phases"]["read"]["initramfs"] = {
            "path": other.as_posix(),
            "sha256": hashlib.sha256(other.read_bytes()).hexdigest(),
        }
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "differs from the fixed candidate"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_a_data_hard_link_to_the_system_candidate(self) -> None:
        data_path = Path(self.manifest["data_work_copy"]["path"])
        data_path.unlink()
        system_path = Path(self.manifest["artifacts"]["system_image"]["path"])
        os.link(system_path, data_path)
        with self.assertRaisesRegex(preflight_tool.PreflightError, "hard link"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_duplicate_serial_or_stale_serial_file(self) -> None:
        self.manifest["phases"]["read"]["serial_path"] = self.manifest["phases"]["save"]["serial_path"]
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "phase-local"):
            preflight_tool.preflight(self.manifest_path)

        self.manifest["phases"]["read"]["serial_path"] = (
            self.run_directory / "read" / "serial.log"
        ).as_posix()
        Path(self.manifest["phases"]["read"]["serial_path"]).write_text("old serial")
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "already exists"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_non_directory_serial_parent(self) -> None:
        read_directory = self.run_directory / "read"
        read_directory.rmdir()
        read_directory.write_text("not a directory", encoding="utf-8")
        with self.assertRaises(preflight_tool.PreflightError):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_serial_parents_with_the_same_device_and_inode(self) -> None:
        save_directory = self.run_directory / "save"
        read_directory = self.run_directory / "read"
        save_stat = save_directory.lstat()
        original_stat = Path.stat
        original_lstat = Path.lstat

        def alias_identity(path: Path, *args: Any, **kwargs: Any) -> Any:
            metadata = original_stat(path, *args, **kwargs)
            if path == read_directory:
                return SimpleNamespace(
                    st_dev=save_stat.st_dev,
                    st_ino=save_stat.st_ino,
                    st_mode=metadata.st_mode,
                    st_nlink=metadata.st_nlink,
                )
            return metadata

        def alias_lstat(path: Path, *args: Any, **kwargs: Any) -> Any:
            metadata = original_lstat(path, *args, **kwargs)
            if path == read_directory:
                return SimpleNamespace(
                    st_dev=save_stat.st_dev,
                    st_ino=save_stat.st_ino,
                    st_mode=metadata.st_mode,
                    st_nlink=metadata.st_nlink,
                )
            return metadata

        with patch.object(Path, "stat", alias_identity), patch.object(
            Path, "lstat", alias_lstat
        ):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError, "serial parents must be distinct"
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rechecks_serial_parent_identity_after_artifact_hashes(self) -> None:
        original_recheck = preflight_tool._recheck_serial_destinations
        replaced = False

        def replace_save_parent(
            run_directory: Path,
            serial_paths: list[Path],
            expected_identities: list[tuple[int, int]],
        ) -> None:
            nonlocal replaced
            if not replaced:
                replaced = True
                save_directory = run_directory / "save"
                save_directory.rename(run_directory / "save-before-recheck")
                save_directory.mkdir()
            original_recheck(run_directory, serial_paths, expected_identities)

        with patch.object(
            preflight_tool, "_recheck_serial_destinations", replace_save_parent
        ):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError, "serial parent identity changed before preflight PASS"
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rechecks_that_both_serial_logs_are_unused_after_artifact_hashes(self) -> None:
        original_recheck = preflight_tool._recheck_serial_destinations
        created = False

        def create_late_log(
            run_directory: Path,
            serial_paths: list[Path],
            expected_identities: list[tuple[int, int]],
        ) -> None:
            nonlocal created
            if not created:
                created = True
                serial_paths[1].write_text("late serial output", encoding="utf-8")
            original_recheck(run_directory, serial_paths, expected_identities)

        with patch.object(
            preflight_tool, "_recheck_serial_destinations", create_late_log
        ):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError, "serial path already exists"
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rejects_serial_parent_replaced_during_unused_path_check(self) -> None:
        read_directory = self.run_directory / "read"
        read_serial = read_directory / "serial.log"
        original_lstat = Path.lstat
        replaced = False

        def replace_before_missing_path_result(path: Path, *args: Any, **kwargs: Any) -> Any:
            nonlocal replaced
            if path == read_serial and not replaced:
                replaced = True
                read_directory.rename(self.run_directory / "read-before-recheck")
                read_directory.mkdir()
                raise FileNotFoundError(path.as_posix())
            return original_lstat(path, *args, **kwargs)

        with patch.object(Path, "lstat", replace_before_missing_path_result):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError,
                "serial parent identity changed during validation",
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rejects_unbounded_or_boolean_timeout(self) -> None:
        for timeout in (0, 601, True):
            with self.subTest(timeout=timeout):
                self.manifest["phases"]["save"]["timeout_seconds"] = timeout
                self._write_manifest()
                with self.assertRaises(preflight_tool.PreflightError):
                    preflight_tool.preflight(self.manifest_path)

    def test_rejects_qmp_or_start_authorization(self) -> None:
        self.manifest["phases"]["save"]["qmp_enabled"] = True
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "disable QMP"):
            preflight_tool.preflight(self.manifest_path)

        self.manifest["phases"]["save"]["qmp_enabled"] = False
        self.manifest["phases"]["read"]["start_authorized"] = True
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "explicit start signal"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_system_write_or_automatic_interaction(self) -> None:
        self.manifest["phases"]["save"]["system_read_only"] = False
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "System/Data access mode"):
            preflight_tool.preflight(self.manifest_path)

        self.manifest["phases"]["save"]["system_read_only"] = True
        self.manifest["phases"]["read"]["interactive_input"] = "automatic"
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "explicit start signal"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_artifact_digest_drift(self) -> None:
        self.manifest["artifacts"]["kernel"]["sha256"] = "0" * 64
        self.manifest["phases"]["save"]["kernel"]["sha256"] = "0" * 64
        self.manifest["phases"]["read"]["kernel"]["sha256"] = "0" * 64
        self._write_manifest()
        with self.assertRaisesRegex(preflight_tool.PreflightError, "digest does not match"):
            preflight_tool.preflight(self.manifest_path)

    def test_rejects_artifact_mutated_during_initial_hash(self) -> None:
        original_hash = preflight_tool._hash_open_file
        mutated = False

        def mutate_after_hash(source: Any, name: str) -> str:
            nonlocal mutated
            digest = original_hash(source, name)
            if name == "kernel" and not mutated:
                mutated = True
                Path(self.manifest["artifacts"]["kernel"]["path"]).write_bytes(
                    b"mutated while hash descriptor remained open"
                )
            return digest

        with patch.object(preflight_tool, "_hash_open_file", mutate_after_hash):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError, "changed while being hashed"
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rechecks_opened_artifact_after_hash_and_rejects_path_replacement(self) -> None:
        original_verify = preflight_tool._verify_pinned_artifact_contents
        replaced = False

        def replace_before_final_hash(artifact: Any, name: str) -> None:
            nonlocal replaced
            if name == "initramfs" and not replaced:
                replaced = True
                replacement = artifact.path.with_name("replacement-initramfs.bin")
                replacement.write_bytes(b"replacement candidate")
                os.replace(replacement, artifact.path)
            original_verify(artifact, name)

        with patch.object(
            preflight_tool,
            "_verify_pinned_artifact_contents",
            replace_before_final_hash,
        ):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError,
                "changed after its initial hash|path no longer matches the opened file descriptor",
            ):
                preflight_tool.preflight(self.manifest_path)

    def test_rejects_artifact_changed_at_the_preflight_pass_boundary(self) -> None:
        original_verify = preflight_tool._verify_pinned_artifact_identity
        changed = False

        def change_before_final_check(artifact: Any, name: str) -> None:
            nonlocal changed
            if name == "kernel" and not changed:
                changed = True
                artifact.path.write_bytes(b"changed after final digest check")
            original_verify(artifact, name)

        with patch.object(
            preflight_tool,
            "_verify_pinned_artifact_identity",
            change_before_final_check,
        ):
            with self.assertRaisesRegex(
                preflight_tool.PreflightError, "changed before preflight PASS"
            ):
                preflight_tool.preflight(self.manifest_path)


if __name__ == "__main__":
    unittest.main()
