#!/usr/bin/env python3
"""Synthetic host regressions for the no-QMP Task24 interactive runner."""

from __future__ import annotations

import hashlib
import io
import json
import os
import signal
import socket
import stat
import struct
import sys
import tempfile
import textwrap
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing, redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import Mock, patch


sys.path.insert(0, str(Path(__file__).resolve().parent))
import task24_interactive_acceptance as runner
import task24_recovery_serials as recovery
from task24_rfb_input import RfbKeyboard
from task24_run_qemu_child import SerialMarkerWatcher


def _receipt(marker: str, launch: int) -> str:
    return recovery._receipt_line(
        marker, generation=launch, launch=launch, pid=10,
        c1=20, revision=30, terminal=40,
    )


def _newc_archive(entries: list[tuple[str, bytes, int, int]]) -> bytes:
    result = bytearray()
    for inode, (name, payload, mode, links) in enumerate(
        entries + [("TRAILER!!!", b"", 0, 1)], 1,
    ):
        encoded_name = name.encode("ascii") + b"\0"
        fields = [inode, mode, 0, 0, links, 0, len(payload), 0, 0, 0, 0, len(encoded_name), 0]
        result.extend(b"070701" + b"".join(f"{field:08x}".encode("ascii") for field in fields))
        result.extend(encoded_name)
        result.extend(bytes(-len(result) % 4))
        result.extend(payload)
        result.extend(bytes(-len(result) % 4))
    return bytes(result)



def _phase_serial(phase: str) -> str:
    launch = 1 if phase == "save" else 2
    lines = [recovery.MOUNT]
    if phase == "read":
        lines += [_receipt(recovery.PREVIOUS_TERMINAL, 1), recovery.LOGIN_WAIT]
    if phase == "save":
        lines.append(runner.REGISTRATION_WAIT)
    lines += [recovery.SESSION, recovery.HANDSHAKE, recovery.READY]
    if phase == "save":
        lines += [recovery.INPUT, f"{recovery.SAVED} object=1234567890abcdef revision=2"]
    else:
        lines += [f"{recovery.REOPENED} object=1234567890abcdef revision=2", recovery.INPUT]
    lines += [
        recovery.CLOSED, recovery.REAPED_NORMAL, recovery.IDENTITY_EXIT_REQUESTED,
        _receipt(recovery.C3_IDENTITY_NORMAL_EXIT_REAPED, launch),
        _receipt(recovery.CURRENT_TERMINAL, launch),
        "[    3.100000] reboot: Power down",
    ]
    return "\n".join(lines) + "\n"


def _reject_serial() -> str:
    return "\n".join((
        recovery.MOUNT, _receipt(recovery.PREVIOUS_TERMINAL, 1), recovery.LOGIN_WAIT,
        recovery.LOGIN_CONNECTED, recovery.LOGIN_REJECTED, recovery.LOGIN_WAIT,
        recovery.LOGIN_CANCELLED, recovery.IDENTITY_EXIT_REQUESTED,
        _receipt(recovery.C3_IDENTITY_NORMAL_EXIT_REAPED, 3),
        _receipt(recovery.CURRENT_TERMINAL, 3),
    )) + "\n"


class Task24InteractiveAcceptanceTests(unittest.TestCase):
    def test_l9_validates_the_previous_logout_and_new_credential_attachment(self):
        phase = "password-login"
        first_session = "session_epoch.3_" + "1" * 32
        second_session = "session_epoch.4_" + "2" * 32
        previous_logout = runner.L9_PREVIOUS_LOGOUT + first_session
        lines = [recovery.MOUNT, _receipt(recovery.PREVIOUS_TERMINAL, 1), previous_logout]
        flow = list(runner.L9_RFB_FLOW[phase])
        desktop = flow.index(recovery.DESKTOP_READY)
        lines += flow[:desktop] + [recovery.SESSION, recovery.HANDSHAKE, recovery.READY] + flow[desktop:]
        lines += [
            recovery.SESSION_LOGOUT, runner.L9_LOGGED_OUT + second_session,
            recovery.REAPED_NORMAL, recovery.IDENTITY_EXIT_REQUESTED,
            _receipt(recovery.C3_IDENTITY_NORMAL_EXIT_REAPED, 2),
            _receipt(recovery.CURRENT_TERMINAL, 2),
        ]
        serial = "\n".join(lines) + "\n"
        previous = "\n".join((
            runner.L9_LOGGED_OUT + first_session,
            _receipt(recovery.CURRENT_TERMINAL, 1),
        )) + "\n"
        self.assertEqual(runner.validate_l9_phase(serial, phase, previous)["confirmed_rejection_count"], 1)
        for invalid, reason in (
            (serial.replace(recovery.LOGIN_REJECTED + "\n", ""), "ordered L9"),
            (serial.replace(previous_logout + "\n", ""), "L9_PREVIOUS_LOGOUT"),
            (serial.replace(previous_logout, runner.L9_PREVIOUS_LOGOUT + second_session),
             "preceding L9 logout"),
            (serial.replace(previous_logout, recovery.PREVIOUS_USER_LOGOUT), "L9_PREVIOUS_LOGOUT"),
            (serial.replace(_receipt(recovery.PREVIOUS_TERMINAL, 1), _receipt(recovery.PREVIOUS_TERMINAL, 3)),
             "preceding L9 logout"),
        ):
            with self.assertRaisesRegex(ValueError, reason):
                runner.validate_l9_phase(invalid, phase, previous)
        with self.assertRaisesRegex(ValueError, "expected preceding L9 boot"):
            runner.validate_l9_phase(serial, phase, None)

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="task24-interactive-runner-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.run_id = "task24-host-fixture"
        self.run_directory = self.root / self.run_id
        (self.run_directory / "save").mkdir(parents=True)
        (self.run_directory / "read").mkdir()
        fixed = self.root / "fixed"
        fixed.mkdir()
        self.artifacts = {}
        for name, payload in (
            ("kernel", b"synthetic x86_64 kernel"),
            ("initramfs", b"synthetic Task24 initramfs"),
            ("system_image", b"synthetic read-only System"),
        ):
            path = fixed / f"{name}.bin"
            path.write_bytes(payload)
            self.artifacts[name] = {
                "path": path.as_posix(),
                "sha256": hashlib.sha256(payload).hexdigest(),
            }
        self.data_path = self.run_directory / "data.raw"
        self.data_uuid = "12345678-1234-4234-8234-123456789abc"
        superblock = bytearray(1024)
        superblock[0x38:0x3A] = b"\x53\xef"
        superblock[0x68:0x78] = uuid.UUID(self.data_uuid).bytes
        self.data_path.write_bytes(bytes(1024) + superblock)
        self.manifest = {
            "schema": runner.TASK24_SCHEMA,
            "run_id": self.run_id,
            "run_directory": self.run_directory.as_posix(),
            "candidate_source_commit": runner.TASK24_HEAD,
            "artifacts": self.artifacts,
            "data_work_copy": {
                "path": self.data_path.as_posix(),
                "created_for_run_id": self.run_id,
                "provisioning": "fresh-ext4",
                "filesystem_uuid": self.data_uuid,
            },
            "phases": {},
            "interactive_start_authorized": False,
        }
        for phase in runner.PHASES:
            self.manifest["phases"][phase] = {
                **{name: dict(value) for name, value in self.artifacts.items()},
                "data_image": self.data_path.as_posix(),
                "filesystem_uuid": self.data_uuid,
                "serial_path": (self.run_directory / phase / "serial.log").as_posix(),
                "timeout_seconds": 10,
                "monitor": "none",
                "qmp_enabled": False,
                "serial_transport": "file",
                "interactive_input": "manual",
                "start_authorized": False,
                "system_read_only": True,
                "data_read_only": False,
            }
        self.manifest_path = self.run_directory / "preflight.json"
        self.write_manifest()
        self.preflight_script = self.root / "task24_run_preflight.py"
        self.preflight_script.write_text(
            textwrap.dedent(
                """
                import json, sys
                from pathlib import Path
                manifest_path = Path(sys.argv[sys.argv.index('--manifest') + 1])
                manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
                artifacts = manifest['artifacts']
                phases = manifest['phases']
                result = {
                    'preflight': 'PASS',
                    'run_id': manifest['run_id'],
                    'candidate_source_commit': manifest['candidate_source_commit'],
                    'kernel_sha256': artifacts['kernel']['sha256'],
                    'initramfs_sha256': artifacts['initramfs']['sha256'],
                    'system_sha256': artifacts['system_image']['sha256'],
                    'same_system_candidate': True,
                    'same_data_work_copy': True,
                    'data_freshness': 'manifest-declared fresh-ext4',
                    'data_filesystem_uuid': manifest['data_work_copy']['filesystem_uuid'],
                    'serial_files_separate': True,
                    'phase_timeouts_seconds': {
                        phase: phases[phase]['timeout_seconds'] for phase in ('save', 'read')
                    },
                    'qmp_enabled': False,
                    'interactive_start_authorized': False,
                    'qemu_started': False,
                }
                print(json.dumps(result, sort_keys=True))
                """
            ),
            encoding="utf-8",
        )
        self.preflight_digest = hashlib.sha256(self.preflight_script.read_bytes()).hexdigest()

    def write_manifest(self) -> None:
        self.manifest_path.write_text(json.dumps(self.manifest, sort_keys=True), encoding="utf-8")

    def test_l9_non_start_requires_the_pinned_candidate_and_four_private_destinations(self):
        head = "d" * 40
        initramfs = self.artifacts["initramfs"]["sha256"]
        self.manifest["candidate_source_commit"] = head
        self.write_manifest()
        for phase in runner.L9_RFB_FLOW:
            (self.run_directory / phase).mkdir()
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", hashlib.sha256(self.preflight_script.read_bytes()).hexdigest()):
            with self.assertRaisesRegex(runner.RunnerError, "does not match the fixed L9"):
                runner.prepare_run(self.manifest_path, self.preflight_script, l9_pin=(head, "b" * 64))
            prepared = runner.prepare_run(self.manifest_path, self.preflight_script, l9_pin=(head, initramfs))
        fixture_path = self.run_directory / "development-fixture.json"
        fixture_path.write_text(json.dumps({
            "schema": "nagi-task24-rfb-auto-fixture/v1",
            "run_id": self.run_id,
            "preflight_manifest_sha256": prepared.manifest_sha256,
            "rfb_port": 5968,
            "owner_name": "taskfixture",
            "owner_password": "taskfixturealpha24001",
            "wrong_password": "taskfixtureomega24002",
            "note_text": "task fixture note",
        }), encoding="utf-8")
        fixture_path.chmod(0o600)
        prepared_l9 = runner.prepare_automatic_trial(prepared, fixture_path, l9_trial=True)
        self.assertEqual(prepared_l9["status"], "L9_LAYOUT_VERIFIED")
        self.assertEqual(prepared_l9["phases"], list(runner.L9_RFB_FLOW))
        self.assertFalse(prepared_l9["launch_authorized"])
        (self.run_directory / "locale-read" / "japanese-unlock.ppm").touch()
        with self.assertRaisesRegex(runner.RunnerError, "already has a run output"):
            runner.prepare_automatic_trial(prepared, fixture_path, l9_trial=True)

    def prepare(self) -> runner.PreparedRun:
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest):
            return runner.prepare_run(self.manifest_path, self.preflight_script)

    def _f9_profile_inputs(self) -> dict[str, Path | str]:
        elf_payload = b"synthetic pinned PID 1 ELF"
        elf_path = self.root / "fixed" / "nagi-init"
        elf_path.write_bytes(elf_payload)
        cpio_payload = _newc_archive([
            ("init", b"synthetic init", stat.S_IFREG | 0o755, 1),
            ("system", b"", stat.S_IFDIR | 0o755, 2),
            ("system/nagi-init", elf_payload, stat.S_IFREG | 0o755, 1),
        ])
        cpio_path = self.root / "fixed" / "task24.cpio"
        cpio_path.write_bytes(cpio_payload)
        elf_sha256 = hashlib.sha256(elf_payload).hexdigest()
        cpio_sha256 = hashlib.sha256(cpio_payload).hexdigest()
        self.manifest["candidate_source_commit"] = runner.TASK24_F9_8E2A_HEAD
        self.artifacts["initramfs"] = {"path": cpio_path.as_posix(), "sha256": cpio_sha256}
        self.manifest["artifacts"] = self.artifacts
        for phase in self.manifest["phases"].values():
            phase["initramfs"] = dict(self.artifacts["initramfs"])
        self.write_manifest()

        fixed_receipt_path = self.root / "fixed" / "fixed-input-receipt.json"
        fixed_receipt_path.write_text('{"fixture":"fixed candidate binding"}\n', encoding="utf-8")
        validation_receipt_path = self.root / "fixed" / "development-cpio-validation.json"
        validation_receipt_path.write_text(json.dumps({
            "development_inputs": [{
                "path": "nagi-init",
                "sha256": elf_sha256,
                "size_bytes": len(elf_payload),
            }],
        }, sort_keys=True), encoding="utf-8")
        return {
            "elf_path": elf_path,
            "elf_sha256": elf_sha256,
            "cpio_path": cpio_path,
            "cpio_sha256": cpio_sha256,
            "fixed_receipt_path": fixed_receipt_path,
            "fixed_receipt_sha256": hashlib.sha256(fixed_receipt_path.read_bytes()).hexdigest(),
            "validation_receipt_path": validation_receipt_path,
            "validation_receipt_sha256": hashlib.sha256(validation_receipt_path.read_bytes()).hexdigest(),
        }

    def _f9_profile_pins(self, inputs: dict[str, Path | str]):
        return patch.multiple(
            runner,
            TASK24_PREFLIGHT_SHA256=self.preflight_digest,
            TASK24_F9_8E2A_PID1_ELF_SHA256=inputs["elf_sha256"],
            TASK24_F9_8E2A_CPIO_SHA256=inputs["cpio_sha256"],
            TASK24_F9_FIXED_INPUT_RECEIPT_SHA256=inputs["fixed_receipt_sha256"],
            TASK24_F9_CPIO_VALIDATION_RECEIPT_SHA256=inputs["validation_receipt_sha256"],
            TASK24_F9_CPIO_VALIDATION_ELF_SIZE=Path(inputs["elf_path"]).stat().st_size,
        )

    def test_l9_logout_id_uses_the_same_canonical_format_in_serial_and_boot_args(self):
        prepared = self.prepare()
        prepared = runner.replace(prepared, phases={"locale-read": prepared.phases["read"]})
        qemu = self.root / "qemu-stub"
        nonce = "a" * 32
        for session in ("session_" + nonce, "session_epoch.18446744073709551615_" + nonce):
            self.assertEqual(runner.l9_logout_session([runner.L9_LOGGED_OUT + session],
                                                      runner.L9_LOGGED_OUT, "locale-save"), (session, 0))
            argv = runner.qemu_argv(prepared, "locale-read", qemu, previous_l9_session=session)
            self.assertIn(f"nagi.dev_l9_previous_session={session}", argv[argv.index("-append") + 1])
        for session in (
            "session_epoch.03_" + nonce,
            "session_epoch.18446744073709551616_" + nonce,
            "session_epoch.3_" + nonce + " extra",
        ):
            with self.assertRaisesRegex(ValueError, "malformed"):
                runner.l9_logout_session([runner.L9_LOGGED_OUT + session], runner.L9_LOGGED_OUT, "locale-save")
            with self.assertRaisesRegex(runner.RunnerError, "invalid L9 predecessor"):
                runner.qemu_argv(prepared, "locale-read", qemu, previous_l9_session=session)

    def test_automatic_display_is_loopback_only_and_manual_display_is_unchanged(self) -> None:
        prepared = self.prepare()
        qemu = self.root / "qemu-stub"
        manual = runner.qemu_argv(prepared, "save", qemu)
        automatic = runner.qemu_argv(prepared, "save", qemu, rfb_port=5968)
        self.assertEqual(manual[manual.index("-display") + 1], "cocoa")
        self.assertEqual(automatic[automatic.index("-display") + 1], "vnc=127.0.0.1:68")
        for args in (manual, automatic):
            self.assertEqual(args[args.index("-monitor") + 1], "none")
            self.assertNotIn("-qmp", args)
            self.assertNotIn("-vnc", args)
        with self.assertRaisesRegex(runner.RunnerError, "RFB port"):
            runner.qemu_argv(prepared, "save", qemu, rfb_port=5900)

    def test_automatic_failure_check_uses_only_complete_failure_lines(self) -> None:
        self.assertTrue(runner._automatic_failure_marker(
            recovery.FAILURES[2] + " status=unavailable\n"
        ))
        self.assertFalse(runner._automatic_failure_marker(
            recovery.FAILURES[2] + " status=unavailable"
        ))
        self.assertFalse(runner._automatic_failure_marker(recovery.LOGIN_REJECTED + "\n"))

    def _fake_qemu(self) -> tuple[Path, Path]:
        qemu = self.root / "qemu-stub"
        qemu.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env python3
                import json, os, sys, time
                from pathlib import Path
                args = sys.argv[1:]
                record = Path(os.environ['TASK24_FAKE_QEMU_RECORD'])
                release_dir = Path(os.environ['TASK24_FAKE_QEMU_RELEASE_DIR'])
                history = json.loads(record.read_text(encoding='utf-8')) if record.exists() else []
                history.append(args)
                record.write_text(json.dumps(history), encoding='utf-8')
                serial_arg = args[args.index('-serial') + 1]
                serial = Path(serial_arg.removeprefix('file:'))
                phase = serial.parent.name
                lines = json.loads(os.environ['TASK24_FAKE_SERIALS'])[phase].splitlines(keepends=True)
                waiting = next(i for i, line in enumerate(lines) if 'WAITING_FOR_INPUT' in line)
                serial.write_text(''.join(lines[:waiting + 1]), encoding='utf-8')
                release = release_dir / f'{phase}.release'
                deadline = time.monotonic() + 10
                while not release.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                if not release.exists():
                    raise SystemExit('synthetic visible-frame confirmation was not received')
                with serial.open('a', encoding='utf-8') as output:
                    output.write('NAGI_A_GUEST_MEM phase=write t_ms=1 mem_total_kib=8192 mem_available_kib=6000\\n')
                    output.write('NAGI_A_GUEST_MEM phase=write t_ms=2 mem_total_kib=8192 mem_available_kib=5000\\n')
                    output.write(''.join(lines[waiting + 1:]))
                if os.environ.get('TASK24_FAKE_AWAIT_HOST_STOP'):
                    time.sleep(30)
                """
            ),
            encoding="utf-8",
        )
        qemu.chmod(0o700)
        return qemu, self.root / "qemu-argv.json"

    def test_synthetic_same_data_pair_uses_separate_ready_gated_rfb_connections(self) -> None:
        prepared = self.prepare()
        qemu, record = self._fake_qemu()
        release_dir = self.root / "synthetic-key-release"
        release_dir.mkdir()
        gates = {"save": runner.REGISTRATION_WAIT, "read": recovery.LOGIN_WAIT}
        keys = {
            "save": bytes.fromhex("04 01 00 00 00 00 00 61 04 00 00 00 00 00 00 61"),
            "read": bytes.fromhex("04 01 00 00 00 00 00 62 04 00 00 00 00 00 00 62"),
        }
        confirmations: list[tuple[str, str]] = []

        def serve_one_key(listener: socket.socket, phase: str) -> None:
            peer, address = listener.accept()
            with peer, peer.makefile("rb") as stream:
                peer.settimeout(3)
                self.assertEqual(address[0], "127.0.0.1")
                peer.sendall(b"RFB 003.008\n")
                self.assertEqual(stream.read(12), b"RFB 003.008\n")
                peer.sendall(b"\x01\x01")
                self.assertEqual(stream.read(1), b"\x01")
                peer.sendall(bytes(4))
                self.assertEqual(stream.read(1), b"\x01")
                peer.sendall(struct.pack(">HH16xI", 640, 480, 4) + b"test")
                self.assertEqual(stream.read(16), keys[phase])
                self.assertEqual(stream.read(1), b"")
                # The fake child cannot emit its remaining serial until the
                # peer receives the complete press/release and the client closes.
                (release_dir / f"{phase}.release").touch(exist_ok=False)

        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(len(runner.PHASES))
            listener.settimeout(5)
            port = listener.getsockname()[1]

            def on_input_ready(run_id: str, phase: str, _deadline: float) -> dict[str, object]:
                self.assertEqual(run_id, prepared.run_id)
                self.assertFalse((release_dir / f"{phase}.release").exists())
                lines = prepared.phases[phase]["serial_path"].read_text().splitlines()
                self.assertIn(gates[phase], lines)
                self.assertNotIn(recovery.READY, lines)
                if phase == "read":
                    self.assertTrue((release_dir / "save.release").is_file())
                confirmations.append((run_id, phase))
                with closing(RfbKeyboard.connect_local(port, timeout=2)) as keyboard:
                    keyboard.tap(ord("a" if phase == "save" else "b"))
                return {"synthetic_loopback_keyboard": True}

            def serve_pair() -> None:
                for phase in runner.PHASES:
                    serve_one_key(listener, phase)

            with ThreadPoolExecutor(max_workers=1) as executor:
                peer_result = executor.submit(serve_pair)
                with patch.dict(os.environ, {
                    "TASK24_FAKE_QEMU_RECORD": record.as_posix(),
                    "TASK24_FAKE_SERIALS": json.dumps(
                        {phase: _phase_serial(phase) for phase in runner.PHASES}
                    ),
                    "TASK24_FAKE_QEMU_RELEASE_DIR": release_dir.as_posix(),
                }):
                    receipt = runner.run_authorized_pair(
                        prepared, slot=2, board_readback_sha256="a" * 64, qemu=qemu,
                        confirm_visible=on_input_ready,
                    )
                peer_result.result(timeout=5)

        self.assertEqual(confirmations, [(prepared.run_id, phase) for phase in runner.PHASES])
        self.assertEqual(receipt["status"], "RUNNER_COMPLETE")
        self.assertEqual(receipt["acceptance_status"], "NOT_ESTABLISHED")
        self.assertTrue(receipt["same_data_backing_file_across_boots"])
        for result in receipt["phases"]:
            self.assertEqual(result["marker_counts"]["pid1_ready_count"], 1)
            self.assertEqual(
                result["pre_action_frame_visibility_confirmation"],
                {"synthetic_loopback_keyboard": True},
            )
            self.assertEqual(result["post_action_display_content"], "NOT_VERIFIED")
        invocations = json.loads(record.read_text())
        self.assertEqual(len(invocations), 2)
        for args, phase in zip(invocations, runner.PHASES):
            self.assertEqual(args[args.index("-monitor") + 1], "none")
            self.assertNotIn("-qmp", args)
            self.assertEqual(
                args[args.index("-serial") + 1], f"file:{prepared.phases[phase]['serial_path']}",
            )

    def test_fixed_manifest_and_fake_qemu_complete_two_manual_natural_boots(self) -> None:
        prepared = self.prepare()
        qemu, record = self._fake_qemu()
        release_dir = self.root / "visible-frame-release"
        release_dir.mkdir()
        confirmations: list[tuple[str, str]] = []

        def confirm(run_id: str, phase: str, _deadline: float) -> dict[str, object]:
            confirmations.append((run_id, phase))
            (release_dir / f"{phase}.release").write_text("confirmed\n", encoding="utf-8")
            return {"operator_confirmed_visible_frame": True, "confirmed_at_utc": "synthetic"}

        with patch.dict(
            os.environ,
            {
                "TASK24_FAKE_QEMU_RECORD": record.as_posix(),
                "TASK24_FAKE_SERIALS": json.dumps({phase: _phase_serial(phase) for phase in runner.PHASES}),
                "TASK24_FAKE_QEMU_RELEASE_DIR": release_dir.as_posix(),
            },
        ):
            receipt = runner.run_authorized_pair(
                prepared,
                slot=2,
                board_readback_sha256="a" * 64,
                qemu=qemu,
                confirm_visible=confirm,
            )

        self.assertEqual(receipt["status"], "RUNNER_COMPLETE")
        self.assertEqual(receipt["acceptance_status"], "NOT_ESTABLISHED")
        self.assertEqual(
            receipt["manifest_candidate_source_commit"], runner.TASK24_HEAD
        )
        self.assertEqual(receipt["artifact_source_provenance"], "UNVERIFIED_BY_PREFLIGHT")
        self.assertEqual(receipt["data_freshness_evidence"], "MANIFEST_DECLARED_ONLY")
        self.assertEqual(
            receipt["pre_action_frame_visibility"], "operator confirmation only; no pixel capture"
        )
        self.assertEqual(
            receipt["acceptance_gates"]["post_action_display_content"], "NOT_VERIFIED"
        )
        self.assertEqual(
            receipt["acceptance_gates"]["task24_authentication_rejection_case"], "NOT_RUN"
        )
        self.assertEqual([phase["phase"] for phase in receipt["phases"]], ["save", "read"])
        self.assertTrue(all(phase["status"] == "RUNNER_PHASE_COMPLETE" for phase in receipt["phases"]))
        self.assertTrue(
            all(
                phase["marker_counts"]["identity_ready_count"] == 1
                and phase["marker_counts"]["pid1_ready_count"] == 1
                for phase in receipt["phases"]
            )
        )
        self.assertTrue(
            all(phase["post_action_display_content"] == "NOT_VERIFIED" for phase in receipt["phases"])
        )
        self.assertTrue(receipt["same_data_backing_file_across_boots"])
        self.assertTrue(receipt["qemu_started"])
        self.assertEqual(confirmations, [(self.run_id, "save"), (self.run_id, "read")])
        self.assertFalse(receipt["host_rss_used_as_guest_ram"])
        self.assertEqual(
            [phase["guest_memory"]["max_sampled_mem_total_minus_available_kib"] for phase in receipt["phases"]],
            [3192, 3192],
        )
        self.assertEqual(receipt["phases"][0]["guest_memory"]["clock_domain"], "guest Instant; no host alignment claimed")

        invocations = json.loads(record.read_text(encoding="utf-8"))
        self.assertEqual(len(invocations), 2)
        data_arguments = []
        serial_arguments = []
        for args in invocations:
            self.assertEqual(args[args.index("-machine") + 1], "q35,i8042=off")
            self.assertEqual(args[args.index("-accel") + 1], "tcg,thread=multi")
            self.assertEqual(args[args.index("-smp") + 1], "4")
            self.assertEqual(args[args.index("-m") + 1], "8192")
            self.assertEqual(args[args.index("-display") + 1], "cocoa")
            self.assertEqual(args[args.index("-monitor") + 1], "none")
            self.assertEqual(args[args.index("-nic") + 1], "none")
            self.assertIn("virtio-vga,xres=640,yres=480,edid=on", args)
            self.assertIn("virtio-keyboard-pci", args)
            serial = f"NAGI-DATA-{self.data_uuid[:8]}"
            self.assertIn(f"virtio-blk-pci,drive=datadisk,serial={serial}", args)
            command_line = args[args.index("-append") + 1].split()
            self.assertIn("nagi.dev_data.boot=identity", command_line)
            self.assertIn(f"nagi.dev_data.serial={serial}", command_line)
            self.assertIn(f"nagi.dev_data.fs_uuid={self.data_uuid}", command_line)
            self.assertIn("nagi.dev_data.accounts=accounts", command_line)
            self.assertIn("nagi.dev_data.snapshots=snapshots", command_line)
            self.assertNotIn("-qmp", args)
            drives = [arg for arg in args if arg.startswith("if=none,file=")]
            self.assertEqual(len(drives), 2)
            self.assertIn("readonly=on", drives[0])
            self.assertIn("virtio-blk-pci,drive=systemdisk", args)
            self.assertIn(self.data_path.as_posix(), drives[1])
            data_arguments.append(drives[1])
            serial_arguments.append(args[args.index("-serial") + 1])
        self.assertEqual(data_arguments[0], data_arguments[1])
        self.assertNotEqual(serial_arguments[0], serial_arguments[1])
        saved = json.loads((self.run_directory / runner.RECEIPT_NAME).read_text(encoding="utf-8"))
        self.assertEqual(saved["status"], "RUNNER_COMPLETE")
        self.assertEqual(saved["acceptance_status"], "NOT_ESTABLISHED")

    def test_automatic_trial_reuses_three_phase_validator_without_manual_tty_callback(self) -> None:
        (self.run_directory / "reject-close").mkdir()
        self.manifest["candidate_source_commit"] = runner.TASK24_C3_RESTART_RETEST_HEAD
        self.write_manifest()
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest), patch.object(
            runner,
            "TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256",
            self.artifacts["initramfs"]["sha256"],
        ):
            prepared = runner.prepare_run(
                self.manifest_path,
                self.preflight_script,
                c3_restart_retest=True,
            )
        fixture_path = self.run_directory / "development-fixture.json"
        fixture_path.write_text(json.dumps({
            "schema": "nagi-task24-rfb-auto-fixture/v1",
            "run_id": self.run_id,
            "preflight_manifest_sha256": prepared.manifest_sha256,
            "rfb_port": 5968,
            "owner_name": "taskfixture",
            "owner_password": "taskfixturealpha24001",
            "wrong_password": "taskfixtureomega24002",
            "note_text": "task fixture note",
        }), encoding="utf-8")
        fixture_path.chmod(0o600)
        with patch.object(runner, "TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256", "0" * 64):
            with self.assertRaisesRegex(runner.RunnerError, "pinned C3 restart retest initramfs"):
                runner.prepare_automatic_trial(prepared, fixture_path)
        qemu = self.root / "qemu-stub"
        qemu.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        qemu.chmod(0o700)
        input_modes: list[tuple[str, object, object]] = []

        def fake_run_phase(
            phase_prepared, phase, _qemu, confirm_visible=None, *, trial_validation=None,
            automatic_fixture=None,
        ):
            self.assertIsNone(confirm_visible)
            self.assertIsNotNone(automatic_fixture)
            self.assertEqual(automatic_fixture.rfb_port, 5968)
            self.assertIsNotNone(trial_validation)
            input_modes.append((phase, confirm_visible, automatic_fixture))
            serial = {
                "save": _phase_serial("save"),
                "reject-close": _reject_serial(),
                "read": _phase_serial("read").replace(
                    _receipt(recovery.PREVIOUS_TERMINAL, 1),
                    _receipt(recovery.PREVIOUS_TERMINAL, 3),
                ),
            }[phase]
            marker_counts = trial_validation(serial)
            serial_path = phase_prepared.phases[phase]["serial_path"]
            serial_path.write_text(serial, encoding="utf-8")
            summary = {
                "qemu_exit": -signal.SIGTERM if phase == "reject-close" else 0,
                "timed_out": False,
                "cancelled": phase == "reject-close",
                "cancel_signal": signal.SIGTERM if phase == "reject-close" else None,
                "stopped_on_marker": False,
                "wait_for_natural_exit": True,
            }
            (phase_prepared.run_directory / phase / "supervisor.log").write_text(
                "qemu_child_started pid=321\n" + json.dumps(summary) + "\n",
                encoding="utf-8",
            )
            return {
                "phase": phase,
                "status": "RUNNER_PHASE_COMPLETE",
                "data_identity": {
                    "device": runner._data_identity(phase_prepared.data_path)[0],
                    "inode": runner._data_identity(phase_prepared.data_path)[1],
                },
                "marker_counts": marker_counts,
                "post_action_display_content": "NOT_VERIFIED",
            }

        with patch.object(
            runner,
            "TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256",
            self.artifacts["initramfs"]["sha256"],
        ), patch.object(runner, "run_phase", side_effect=fake_run_phase):
            receipt = runner.run_authorized_pair(
                prepared,
                slot=2,
                board_readback_sha256="d" * 64,
                qemu=qemu,
                confirm_visible=None,
                automatic_trial=True,
                fixture_manifest_path=fixture_path,
            )
        self.assertEqual([phase for phase, _, _ in input_modes], list(runner.MANUAL_TRIAL_PHASES))
        self.assertTrue(all(callback is None for _, callback, _ in input_modes))
        self.assertEqual(receipt["status"], "RUNNER_COMPLETE")
        self.assertEqual(receipt["acceptance_status"], "NOT_ESTABLISHED")
        self.assertEqual(receipt["acceptance_gates"]["task24_authentication_rejection_case"], "SERIAL_VALIDATED")
        self.assertTrue(receipt["same_data_backing_file_across_boots"])
        self.assertEqual(receipt["display_backend"], "vnc-loopback-rfb")
        self.assertEqual(receipt["post_action_display_content"], "NOT_VERIFIED")
        serialized_receipt = (self.run_directory / runner.RECEIPT_NAME).read_text(encoding="utf-8")
        self.assertNotIn("taskfixturealpha24001", serialized_receipt)
        self.assertNotIn("taskfixtureomega24002", serialized_receipt)

    def test_missing_or_wrong_board_slot_fails_before_any_qemu_start(self) -> None:
        with self.assertRaisesRegex(runner.RunnerError, "slot 2"):
            runner.validate_start_gate(1, "a" * 64)
        with self.assertRaisesRegex(runner.RunnerError, "board readback"):
            runner.validate_start_gate(2, None)
        with self.assertRaisesRegex(runner.RunnerError, "board readback"):
            runner.validate_start_gate(2, "not-a-hash")

    def test_default_cli_mode_runs_preflight_without_starting_qemu(self) -> None:
        output = io.StringIO()
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest):
            with redirect_stdout(output):
                status = runner.main(
                    [
                        "--manifest",
                        self.manifest_path.as_posix(),
                        "--preflight-script",
                        self.preflight_script.as_posix(),
                    ]
                )
        self.assertEqual(status, 0)
        result = json.loads(output.getvalue())
        self.assertEqual(result["runner"], "PREPARED_NO_START")
        self.assertFalse(result["qemu_started"])
        self.assertFalse((self.run_directory / runner.RECEIPT_NAME).exists())

    def test_notes_save_retest_requires_its_own_source_and_initramfs_pins(self) -> None:
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest):
            with self.assertRaisesRegex(runner.RunnerError, "candidate source does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, notes_save_retest=True)
            self.manifest["candidate_source_commit"] = runner.TASK24_NOTES_SAVE_RETEST_HEAD
            self.write_manifest()
            with self.assertRaisesRegex(runner.RunnerError, "fixed public Task24 HEAD"):
                runner.prepare_run(self.manifest_path, self.preflight_script)
            with self.assertRaisesRegex(runner.RunnerError, "initramfs does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, notes_save_retest=True)
            with patch.object(
                runner, "TASK24_NOTES_SAVE_RETEST_INITRAMFS_SHA256",
                self.artifacts["initramfs"]["sha256"],
            ):
                prepared = runner.prepare_run(
                    self.manifest_path, self.preflight_script, notes_save_retest=True,
                )
            self.assertEqual(prepared.candidate_source_commit, runner.TASK24_NOTES_SAVE_RETEST_HEAD)

    def test_c3_restart_retest_keeps_public_and_notes_save_pins_separate(self) -> None:
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest):
            self.manifest["candidate_source_commit"] = runner.TASK24_NOTES_SAVE_RETEST_HEAD
            self.write_manifest()
            with self.assertRaisesRegex(runner.RunnerError, "candidate source does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, c3_restart_retest=True)

            self.manifest["candidate_source_commit"] = runner.TASK24_C3_RESTART_RETEST_HEAD
            self.write_manifest()
            with self.assertRaisesRegex(runner.RunnerError, "fixed public Task24 HEAD"):
                runner.prepare_run(self.manifest_path, self.preflight_script)
            with self.assertRaisesRegex(runner.RunnerError, "pinned Notes save retest"):
                runner.prepare_run(self.manifest_path, self.preflight_script, notes_save_retest=True)
            with self.assertRaisesRegex(runner.RunnerError, "initramfs does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, c3_restart_retest=True)

            with patch.object(
                runner, "TASK24_C3_RESTART_RETEST_INITRAMFS_SHA256",
                self.artifacts["initramfs"]["sha256"],
            ):
                prepared = runner.prepare_run(
                    self.manifest_path, self.preflight_script, c3_restart_retest=True,
                )
        self.assertEqual(prepared.candidate_source_commit, runner.TASK24_C3_RESTART_RETEST_HEAD)

    def test_f9_8e2a_profile_rejects_candidate_source_elf_and_cpio_mismatches(self) -> None:
        inputs = self._f9_profile_inputs()
        self.assertEqual(
            runner.TASK24_F9_8E2A_HEAD,
            "8e2a5ff11034e0fdbd47cf81b3cdbfa8c281efa9",
        )
        self.assertEqual(
            runner.TASK24_F9_8E2A_PID1_ELF_SHA256,
            "4493bfec50a272f858a72261308eaddfbba5d7cbeaabee1da22abc7616aa0148",
        )
        self.assertEqual(
            runner.TASK24_F9_8E2A_CPIO_SHA256,
            "e0f8f0b960290ac064420ff81318077cacb45617c877c2047de5124afd0cedcb",
        )
        common = {
            "f9_rfb_no_start": True,
            "f9_pid1_elf_path": inputs["elf_path"],
            "f9_fixed_input_receipt_path": inputs["fixed_receipt_path"],
            "f9_cpio_validation_receipt_path": inputs["validation_receipt_path"],
        }
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", self.preflight_digest):
            self.manifest["candidate_source_commit"] = "a" * 40
            self.write_manifest()
            with self.assertRaisesRegex(runner.RunnerError, "candidate source does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, **common)

            self.manifest["candidate_source_commit"] = runner.TASK24_F9_8E2A_HEAD
            self.write_manifest()
            with self.assertRaisesRegex(runner.RunnerError, "CPIO does not match"):
                runner.prepare_run(self.manifest_path, self.preflight_script, **common)

            with patch.object(runner, "TASK24_F9_8E2A_CPIO_SHA256", inputs["cpio_sha256"]):
                with self.assertRaisesRegex(runner.RunnerError, "PID 1 ELF does not match"):
                    runner.prepare_run(self.manifest_path, self.preflight_script, **common)

    def test_f9_8e2a_profile_emits_no_start_candidate_receipt_and_three_phase_layout(self) -> None:
        inputs = self._f9_profile_inputs()
        reject_close = self.run_directory / "reject-close"
        reject_close.mkdir()
        fixture_path = self.run_directory / "development-fixture.json"
        fixture_path.write_text(json.dumps({
            "schema": "nagi-task24-rfb-auto-fixture/v1",
            "run_id": self.run_id,
            "preflight_manifest_sha256": hashlib.sha256(self.manifest_path.read_bytes()).hexdigest(),
            "rfb_port": 5968,
            "owner_name": "taskfixture",
            "owner_password": "taskfixturealpha24001",
            "wrong_password": "taskfixtureomega24002",
            "note_text": "task fixture note",
        }), encoding="utf-8")
        fixture_path.chmod(0o600)
        args = [
            "--manifest", str(self.manifest_path),
            "--preflight-script", str(self.preflight_script),
            "--prepare-automatic-trial",
            "--fixture-manifest", str(fixture_path),
            "--f9-8e2a-rfb-no-start",
            "--pid1-elf", str(inputs["elf_path"]),
            "--f9-fixed-input-receipt", str(inputs["fixed_receipt_path"]),
            "--f9-cpio-validation-receipt", str(inputs["validation_receipt_path"]),
        ]
        with self._f9_profile_pins(inputs):
            prepared = runner.prepare_run(
                self.manifest_path,
                self.preflight_script,
                f9_rfb_no_start=True,
                f9_pid1_elf_path=inputs["elf_path"],
                f9_fixed_input_receipt_path=inputs["fixed_receipt_path"],
                f9_cpio_validation_receipt_path=inputs["validation_receipt_path"],
            )
            with self.assertRaisesRegex(runner.RunnerError, "no-start only"):
                runner.run_authorized_pair(
                    prepared, slot=2, board_readback_sha256="d" * 64,
                    qemu=self.root / "missing-qemu",
                )
            output = io.StringIO()
            with redirect_stdout(output), patch.object(runner, "run_authorized_pair") as launch:
                self.assertEqual(runner.main(args), 0)
            launch.assert_not_called()
            result = json.loads(output.getvalue())
            self.assertEqual(result["runner"], "PREPARED_NO_START")
            self.assertFalse(result["qemu_started"])
            candidate = result["candidate_check_receipt"]
            self.assertEqual(candidate["schema"], runner.TASK24_F9_CANDIDATE_CHECK_SCHEMA)
            self.assertEqual(candidate["profile"], "F9_8E2A_NO_START_RFB")
            self.assertEqual(candidate["status"], "PASS")
            self.assertEqual(candidate["source"]["candidate_source_commit"], runner.TASK24_F9_8E2A_HEAD)
            self.assertEqual(candidate["pid1_elf"]["sha256"], inputs["elf_sha256"])
            self.assertEqual(candidate["initramfs_cpio"]["manifest_sha256"], inputs["cpio_sha256"])
            self.assertEqual(
                candidate["cpio_validation_receipt"]["sha256"],
                inputs["validation_receipt_sha256"],
            )
            self.assertEqual(
                candidate["fixed_input_receipt"]["sha256"],
                inputs["fixed_receipt_sha256"],
            )
            embedded = candidate["cpio_embedded_pid1"]
            self.assertEqual(embedded["path"], "system/nagi-init")
            self.assertEqual(embedded["sha256"], inputs["elf_sha256"])
            self.assertTrue(embedded["matches_fixed_pid1_elf"])
            automatic = result["automatic_trial"]
            self.assertEqual(automatic["phases"], list(runner.MANUAL_TRIAL_PHASES))
            self.assertEqual(automatic["mode"], "AUTOMATIC_F9_8E2A_NO_START_FIXTURE")
            self.assertIn("no WS-10 RFB evidence", automatic["coverage"])
            self.assertFalse(automatic["launch_authorized"])
            self.assertFalse((self.run_directory / runner.RECEIPT_NAME).exists())
            self.assertEqual(list(reject_close.iterdir()), [])

            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                runner.main(args + ["--authorize-start"])

    def test_different_public_source_head_is_rejected_before_preflight(self) -> None:
        self.manifest["candidate_source_commit"] = "a" * 40
        self.write_manifest()
        with self.assertRaisesRegex(runner.RunnerError, "fixed public Task24 HEAD"):
            self.prepare()

    def test_unpinned_preflight_script_is_rejected(self) -> None:
        with patch.object(runner, "TASK24_PREFLIGHT_SHA256", "0" * 64):
            with self.assertRaisesRegex(runner.RunnerError, "fixed public HEAD pin"):
                runner.prepare_run(self.manifest_path, self.preflight_script)

    def test_visible_frame_confirmation_requires_the_exact_host_token(self) -> None:
        class ReadySelector:
            def register(self, *_args) -> None:
                return None

            def select(self, **_kwargs):
                return [(None, None)]

            def close(self) -> None:
                return None

        with patch.object(runner.selectors, "DefaultSelector", return_value=ReadySelector()):
            with patch.object(runner.sys, "stdin", io.StringIO(f"VISIBLE {self.run_id} save\n")):
                with redirect_stdout(io.StringIO()):
                    result = runner.request_visible_confirmation(
                        self.run_id, "save", runner.time.monotonic() + 2
                    )
        self.assertTrue(result["operator_confirmed_visible_frame"])
        with patch.object(runner.selectors, "DefaultSelector", return_value=ReadySelector()):
            with patch.object(runner.sys, "stdin", io.StringIO("run qemu\n")):
                with self.assertRaisesRegex(runner.RunnerError, "did not match"):
                    with redirect_stdout(io.StringIO()):
                        runner.request_visible_confirmation(
                            self.run_id, "save", runner.time.monotonic() + 2
                        )

    def test_changed_data_backing_file_stops_before_the_read_boot(self) -> None:
        prepared = self.prepare()
        qemu, _ = self._fake_qemu()
        initial = runner._data_identity(self.data_path)
        calls: list[str] = []

        def fake_phase(_prepared, phase, _qemu, _confirm):
            calls.append(phase)
            return {"data_identity": {"device": initial[0], "inode": initial[1]}}

        replacement = self.run_directory / "replacement.raw"
        replacement.write_bytes(b"different synthetic backing file")
        with patch.object(runner, "run_phase", side_effect=fake_phase):
            with patch.object(
                runner,
                "_data_identity",
                side_effect=[initial, initial, (initial[0], initial[1] + 1)],
            ):
                with self.assertRaisesRegex(runner.RunnerError, "changed between"):
                    runner.run_authorized_pair(
                        prepared,
                        slot=2,
                        board_readback_sha256="b" * 64,
                        qemu=qemu,
                        confirm_visible=lambda *_args: {},
                    )
        self.assertEqual(calls, ["save"])

    def test_timeout_is_recorded_as_failure_after_reaping_only_the_fake_child(self) -> None:
        for phase in runner.PHASES:
            self.manifest["phases"][phase]["timeout_seconds"] = 2
        self.write_manifest()
        prepared = self.prepare()
        qemu = self.root / "qemu-stall"
        qemu.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env python3
                import os, sys, time
                from pathlib import Path
                args = sys.argv[1:]
                release_dir = Path(os.environ['TASK24_FAKE_QEMU_RELEASE_DIR'])
                serial = Path(args[args.index('-serial') + 1].removeprefix('file:'))
                phase = serial.parent.name
                serial.write_text('NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT\\n', encoding='utf-8')
                release = release_dir / f'{phase}.release'
                deadline = time.monotonic() + 10
                while not release.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                if not release.exists():
                    raise SystemExit('synthetic visible-frame confirmation was not received')
                time.sleep(5)
                """
            ),
            encoding="utf-8",
        )
        qemu.chmod(0o700)
        release_dir = self.root / "timeout-visible-frame-release"
        release_dir.mkdir()

        def confirm_visible(_run_id: str, phase: str, _deadline: float) -> dict[str, object]:
            (release_dir / f"{phase}.release").write_text("confirmed\n", encoding="utf-8")
            return {"operator_confirmed_visible_frame": True}

        with patch.dict(os.environ, {"TASK24_FAKE_QEMU_RELEASE_DIR": release_dir.as_posix()}):
            with self.assertRaisesRegex(runner.RunnerError, "natural QEMU exit: .*timed_out.*True"):
                runner.run_authorized_pair(
                    prepared,
                    slot=2,
                    board_readback_sha256="c" * 64,
                    qemu=qemu,
                    confirm_visible=confirm_visible,
                )
        receipt = json.loads((self.run_directory / runner.RECEIPT_NAME).read_text(encoding="utf-8"))
        self.assertEqual(receipt["status"], "RUNNER_FAILED")
        self.assertEqual(receipt["acceptance_status"], "NOT_ESTABLISHED")
        self.assertTrue(receipt["qemu_started"])
        self.assertIn("timed_out", receipt["failure"])

    def test_guest_memory_summary_keeps_kib_and_rejects_incomplete_lines(self) -> None:
        summary = runner.guest_memory_summary(
            "NAGI_A_GUEST_MEM phase=write t_ms=3 mem_total_kib=10000 mem_available_kib=7000\n"
            "NAGI_A_GUEST_MEM phase=write t_ms=8 mem_total_kib=10000 mem_available_kib=4000\n"
        )
        self.assertEqual(summary["samples"], 2)
        self.assertEqual(summary["unit"], "KiB")
        self.assertEqual(summary["max_sampled_mem_total_minus_available_kib"], 6000)
        self.assertFalse(summary["absolute_peak_claimed"])
        self.assertFalse(summary["terminal_sample_guaranteed"])
        self.assertEqual(summary["clock_domain"], "guest Instant; no host alignment claimed")
        with self.assertRaisesRegex(runner.RunnerError, "missing its KiB"):
            runner.guest_memory_summary("NAGI_A_GUEST_MEM t_ms=8 mem_total_kib=10000")

    def test_observe_completed_pair_keeps_reconciliation_and_shutdown_separate(self) -> None:
        for phase in ("save", "read"):
            phase_dir = self.run_directory / phase
            (phase_dir / "serial.log").write_text(_phase_serial(phase), encoding="utf-8")
            status = {
                "qemu_exit": 0, "timed_out": phase == "read", "cancelled": False,
                "stopped_on_marker": False, "wait_for_natural_exit": True,
            }
            (phase_dir / "supervisor.log").write_text(
                "qemu_child_started pid=123\n" + json.dumps(status) + "\n", encoding="utf-8",
            )

        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(runner.main(["--observe-completed-pair", str(self.run_directory)]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["ready_count_per_boot"], 1)
        self.assertEqual(report["shutdown"], {
            "save": "GUEST_POWERDOWN_AND_NATURAL_EXIT", "read": "HOST_RECOVERY",
        })
        self.assertEqual(report["acceptance_status"], "NOT_ESTABLISHED")

        read = self.run_directory / "read" / "serial.log"
        read.write_text(
            read.read_text().replace(_receipt(recovery.PREVIOUS_TERMINAL, 1),
                                     _receipt(recovery.PREVIOUS_TERMINAL, 2)),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(runner.RunnerError, "different C3 terminal receipt"):
            runner.observe_completed_pair(self.run_directory)

    def test_duplicate_pid1_ready_is_rejected(self) -> None:
        serial = _phase_serial("save").replace(recovery.READY, f"{recovery.READY}\n{recovery.READY}")
        with self.assertRaisesRegex(runner.RunnerError, "requires exactly one .*state=ready"):
            runner._phase_success(serial, "save")

    def test_three_boot_observer_links_the_rejection_between_save_and_read(self) -> None:
        reject = "\n".join((
            recovery.MOUNT, _receipt(recovery.PREVIOUS_TERMINAL, 1), recovery.LOGIN_WAIT,
            recovery.LOGIN_CONNECTED, recovery.LOGIN_REJECTED, recovery.LOGIN_WAIT,
            recovery.LOGIN_CANCELLED, recovery.IDENTITY_EXIT_REQUESTED,
            _receipt(recovery.C3_IDENTITY_NORMAL_EXIT_REAPED, 3),
            _receipt(recovery.CURRENT_TERMINAL, 3),
            "[    3.100000] reboot: Power down",
        )) + "\n"
        save = _phase_serial("save")
        read = _phase_serial("read").replace(
            _receipt(recovery.PREVIOUS_TERMINAL, 1), _receipt(recovery.PREVIOUS_TERMINAL, 3)
        )
        for phase, serial in (("save", save), ("reject-close", reject), ("read", read)):
            phase_dir = self.run_directory / phase
            phase_dir.mkdir(exist_ok=True)
            (phase_dir / "serial.log").write_text(serial, encoding="utf-8")
            status = {
                "qemu_exit": 0, "timed_out": False, "cancelled": phase == "reject-close",
                "stopped_on_marker": False, "wait_for_natural_exit": True,
            }
            (phase_dir / "supervisor.log").write_text(
                "qemu_child_started pid=123\n" + json.dumps(status) + "\n", encoding="utf-8",
            )
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(runner.main([
                "--observe-completed-pair", str(self.run_directory), "--include-reject-close",
            ]), 0)
        report = json.loads(output.getvalue())
        self.assertEqual(report["reject_ready_count"], 0)
        self.assertEqual(report["shutdown"]["reject-close"], "HOST_RECOVERY")
        self.assertEqual(report["acceptance_status"], "NOT_ESTABLISHED")
        for malformed in (
            reject.replace(recovery.LOGIN_CANCELLED, recovery.READY),
            reject.replace(recovery.LOGIN_REJECTED, recovery.LOGIN_DENIED + " (Unavailable)"),
            reject.replace(recovery.LOGIN_WAIT + "\n" + recovery.LOGIN_CANCELLED,
                           recovery.LOGIN_CANCELLED),
            reject.replace(_receipt(recovery.CURRENT_TERMINAL, 3),
                           _receipt(recovery.CURRENT_TERMINAL, 1)),
        ):
            with self.subTest(malformed=malformed):
                with self.assertRaises(ValueError):
                    recovery.validate_save_reject_read(save, malformed, read)
        with self.assertRaisesRegex(ValueError, "different C3 terminal receipt"):
            recovery.validate_save_reject_read(save, reject, _phase_serial("read"))

    def test_manual_trial_preflight_keeps_public_check_and_never_starts(self) -> None:
        rejected = self.run_directory / "reject-close"
        rejected.mkdir()
        preflight = Path(runner.__file__).parent / "task24_run_preflight.py"
        output = io.StringIO()
        with redirect_stdout(output), patch.object(runner, "run_authorized_pair") as launch:
            code = runner.main([
                "--manifest", str(self.manifest_path), "--preflight-script", str(preflight),
                "--prepare-manual-trial",
            ])
        launch.assert_not_called()
        self.assertEqual(code, 0)
        result = json.loads(output.getvalue())
        self.assertEqual(result["preflight"]["preflight"], "PASS")
        self.assertFalse(result["qemu_started"])
        trial = result["manual_trial"]
        self.assertEqual(trial["phases"], ["save", "reject-close", "read"])
        self.assertEqual(trial["data_filesystem_uuid"], self.data_uuid)
        self.assertTrue(trial["data_uuid_verified_in_superblock"])
        self.assertEqual(trial["data_identity"]["inode"], self.data_path.stat().st_ino)
        self.assertFalse(trial["launch_authorized"])
        self.assertEqual(list(rejected.iterdir()), [])

    def test_manual_trial_preflight_rejects_stale_output_and_symlink(self) -> None:
        prepared = self.prepare()
        rejected = self.run_directory / "reject-close"
        rejected.mkdir()
        output = rejected / "serial.log"
        output.touch()
        with self.assertRaisesRegex(runner.RunnerError, "already has a run output"):
            runner.prepare_manual_trial(prepared)
        output.unlink()
        rejected.rmdir()
        rejected.symlink_to(self.run_directory / "read", target_is_directory=True)
        with self.assertRaisesRegex(runner.RunnerError, "own real run directory"):
            runner.prepare_manual_trial(prepared)

    def test_manual_trial_rejects_data_without_matching_superblock(self) -> None:
        (self.run_directory / "reject-close").mkdir()
        prepared = self.prepare()
        with self.data_path.open("r+b") as image:
            image.seek(1024 + 0x68)
            image.write(uuid.UUID(int=1).bytes)
        with self.assertRaisesRegex(runner.RunnerError, "UUID differs"):
            runner.prepare_manual_trial(prepared)
        self.data_path.write_bytes(b"synthetic invalid disk")
        with self.assertRaisesRegex(runner.RunnerError, "no ext filesystem superblock"):
            runner.prepare_manual_trial(prepared)

    def test_manual_trial_reaps_only_after_validated_receipts(self) -> None:
        (self.run_directory / "reject-close").mkdir()
        qemu, record = self._fake_qemu()
        release = self.root / "trial-release"
        release.mkdir()
        serials = {
            "save": _phase_serial("save").replace("[    3.100000] reboot: Power down\n", ""),
            "reject-close": _reject_serial(),
            "read": _phase_serial("read").replace(
                _receipt(recovery.PREVIOUS_TERMINAL, 1), _receipt(recovery.PREVIOUS_TERMINAL, 3)
            ).replace("[    3.100000] reboot: Power down\n", ""),
        }

        def confirm(_run_id, phase, _deadline):
            (release / f"{phase}.release").touch()
            return {"operator_confirmed_visible_frame": True}

        with patch.dict(os.environ, {
            "TASK24_FAKE_SERIALS": json.dumps(serials),
            "TASK24_FAKE_QEMU_RECORD": str(record),
            "TASK24_FAKE_QEMU_RELEASE_DIR": str(release),
            "TASK24_FAKE_AWAIT_HOST_STOP": "1",
        }), redirect_stdout(io.StringIO()):
            result = runner.run_authorized_pair(
                self.prepare(), slot=2, board_readback_sha256="a" * 64, qemu=qemu,
                confirm_visible=confirm, manual_trial=True,
            )
        self.assertEqual([item["phase"] for item in result["phases"]], list(runner.MANUAL_TRIAL_PHASES))
        self.assertTrue(all(item["owned_host_stop_after_terminal"] for item in result["phases"]))
        self.assertTrue(all(not item["natural_exit"] for item in result["phases"]))
        self.assertEqual(result["recovery_observation"]["reject_ready_count"], 0)
        self.assertEqual(result["acceptance_status"], "NOT_ESTABLISHED")
        summary = dict(result["phases"][0]["supervisor_result"], qemu_exit=-signal.SIGKILL)
        self.assertFalse(runner.validated_host_stop_result(True, 128 + signal.SIGTERM, summary))

    def test_manual_trial_does_not_request_stop_after_deadline_or_invalid_proof(self) -> None:
        process = Mock()
        process.poll.return_value = None
        validate = Mock(side_effect=runner.RunnerError("incomplete guest proof"))
        self.assertFalse(runner.stop_owned_after_terminal(process, "", validate, 0))
        validate.assert_not_called()
        with self.assertRaisesRegex(runner.RunnerError, "incomplete guest proof"):
            runner.stop_owned_after_terminal(process, "", validate, float("inf"))
        process.send_signal.assert_not_called()

    def _assert_manual_trial_phase_failure(
        self, phase: str, broken_serial: str, expected_reason: str,
    ) -> None:
        (self.run_directory / "reject-close").mkdir()
        qemu, record = self._fake_qemu()
        release = self.root / "release"
        release.mkdir()
        serials = {
            "save": _phase_serial("save"),
            "reject-close": _reject_serial(),
            "read": _phase_serial("read").replace(
                _receipt(recovery.PREVIOUS_TERMINAL, 1), _receipt(recovery.PREVIOUS_TERMINAL, 3),
            ),
        }
        serials[phase] = broken_serial
        phase_index = runner.MANUAL_TRIAL_PHASES.index(phase)

        def confirm(_run_id, active_phase, _deadline):
            (release / f"{active_phase}.release").touch()
            return {"operator_confirmed_visible_frame": True}

        with patch.dict(os.environ, {
            "TASK24_FAKE_SERIALS": json.dumps(serials),
            "TASK24_FAKE_QEMU_RECORD": str(record),
            "TASK24_FAKE_QEMU_RELEASE_DIR": str(release),
            "TASK24_FAKE_AWAIT_HOST_STOP": "1",
        }), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(runner.RunnerError, expected_reason):
                runner.run_authorized_pair(
                    self.prepare(), slot=2, board_readback_sha256="a" * 64, qemu=qemu,
                    confirm_visible=confirm, manual_trial=True,
                )
        receipt = json.loads((self.run_directory / runner.RECEIPT_NAME).read_text())
        self.assertEqual(receipt["status"], "RUNNER_FAILED")
        self.assertIsNone(receipt["active_phase"])
        self.assertEqual(receipt["failed_phase"], phase)
        self.assertEqual(receipt["failure_stage"], "phase")
        self.assertIn(expected_reason, receipt["failure"])
        self.assertTrue(receipt["qemu_started"])
        self.assertEqual(
            [item["phase"] for item in receipt["phases"]],
            list(runner.MANUAL_TRIAL_PHASES[:phase_index]),
        )
        self.assertEqual(receipt["acceptance_status"], "NOT_ESTABLISHED")
        self.assertEqual(len(json.loads(record.read_text())), phase_index + 1)
        for later_phase in runner.MANUAL_TRIAL_PHASES[phase_index + 1:]:
            self.assertEqual(list((self.run_directory / later_phase).iterdir()), [])

    def test_manual_trial_missing_saved_marker_records_save_failure_and_stops(self) -> None:
        incomplete = "".join(
            line for line in _phase_serial("save").splitlines(keepends=True)
            if not line.startswith(recovery.SAVED)
        )
        self._assert_manual_trial_phase_failure(
            "save", incomplete, "exactly one well-formed NAGI_DEV_NOTES_SAVED",
        )

    def test_manual_trial_ready_during_rejection_records_failure_and_skips_read(self) -> None:
        serial = _reject_serial().replace(recovery.LOGIN_CANCELLED,
                                          recovery.READY + "\n" + recovery.LOGIN_CANCELLED)
        self._assert_manual_trial_phase_failure(
            "reject-close", serial, "reject-close boot entered a forbidden state",
        )

    def test_manual_trial_reopened_revision_mismatch_records_read_failure(self) -> None:
        serial = _phase_serial("read").replace(
            _receipt(recovery.PREVIOUS_TERMINAL, 1), _receipt(recovery.PREVIOUS_TERMINAL, 3),
        ).replace(
            f"{recovery.REOPENED} object=1234567890abcdef revision=2",
            f"{recovery.REOPENED} object=1234567890abcdef revision=3",
        )
        self._assert_manual_trial_phase_failure(
            "read", serial, "saved and reopened Notes ObjectId/revision differ",
        )

    def test_summary_watcher_requires_complete_task24_receipt(self) -> None:
        marker = recovery.CURRENT_TERMINAL
        serial = self.root / "terminal.log"
        serial.write_text(marker + "\n" + marker + " service=org.nagi.identity\n")
        watcher = SerialMarkerWatcher(serial, marker, task24_receipt=True)
        try:
            self.assertFalse(watcher.seen())
            with serial.open("a", encoding="utf-8") as output:
                output.write(_receipt(marker, 1) + "\n")
            self.assertTrue(watcher.seen())
        finally:
            watcher.close()


if __name__ == "__main__":
    unittest.main()
