#!/usr/bin/env python3
"""Run the Linux development Notes UI save, stop, and same-disk readback flow."""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
LINUX_BOOT = REPO_ROOT / "tools" / "linux-boot"
CPIO_BUILDER = LINUX_BOOT / "build_dev_service_cpio.py"
QEMU_RUNNER = LINUX_BOOT / "run_qemu_child.py"
sys.path.insert(0, str(LINUX_BOOT))
from prepare import InputError as PrepareInputError, _validate_kernel
from run_qemu_child import qmp_command, qmp_sendkey

RUN_TIMEOUT_SECONDS = 600
MARKER_WAIT_SECONDS = 570
DISK_BYTES = 8 * 1024 * 1024
NOTES_ID = "5638866587302756353"
NOTES_BODY = 'body="a s"'
SERVICE_HANDSHAKE = (
    "nagi-init: development-only NIPC handshake and correlated "
    "file.search reply verified"
)
KEYS = {
    "save": ("tab", "tab", "tab", "ret", "a", "spc", "s", "ret"),
    "read": ("tab", "tab", "tab", "ret"),
}
RECOVERY_UI_KEYS = ("tab", "tab", "tab", "ret", "a", "spc", "s")


class AcceptanceError(RuntimeError):
    pass


def require_regular_file(path: Path, label: str, *, executable: bool = False) -> Path:
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.stat()
    except OSError as error:
        raise AcceptanceError(f"{label} input unavailable ({error.__class__.__name__})") from None
    if not resolved.is_file():
        raise AcceptanceError(f"{label} input must be a regular file")
    if executable and not os.access(resolved, os.X_OK):
        raise AcceptanceError(f"{label} input is not executable")
    if metadata.st_size == 0:
        raise AcceptanceError(f"{label} input is empty")
    return resolved


def validate_x86_64_kernel(path: Path) -> dict[str, int]:
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise AcceptanceError(f"kernel input could not be opened ({error.__class__.__name__})") from None
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise AcceptanceError("kernel input must be a regular file")
        try:
            return _validate_kernel(descriptor, metadata.st_size, "x86_64")
        except PrepareInputError as error:
            raise AcceptanceError(f"kernel input is not a bootable x86_64 Linux bzImage: {error}") from None
    except OSError as error:
        raise AcceptanceError(f"kernel input could not be read ({error.__class__.__name__})") from None
    finally:
        os.close(descriptor)


def create_blank_disk(path: Path) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags, 0o600)
    except FileExistsError:
        raise AcceptanceError("refusing to overwrite an existing Notes disk") from None
    try:
        os.ftruncate(descriptor, DISK_BYTES)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def build_phase_cpio(args: argparse.Namespace, output: Path, phase: str) -> None:
    command = [
        sys.executable,
        str(CPIO_BUILDER),
        "--init",
        str(args.init),
        "--package",
        str(args.package),
        "--service",
        str(args.service),
        "--client",
        str(args.client),
        "--notes-desktop",
        str(args.notes_elf),
        "--notes-phase",
        phase,
        "--output",
        str(output),
    ]
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        raise AcceptanceError(
            f"{phase} initramfs build failed (exit={result.returncode}): "
            f"{result.stderr.strip() or result.stdout.strip()}"
        )
    try:
        summary = json.loads(result.stdout)
    except json.JSONDecodeError:
        raise AcceptanceError(f"{phase} initramfs builder returned invalid summary") from None
    if summary.get("cpio_structure") != "PASS" or summary.get("development_input_bytes_match") != "PASS":
        raise AcceptanceError(f"{phase} initramfs did not pass its structural input checks")
    print(f"cpio phase={phase} structure=PASS fixed_inputs=PASS")


def unique_qmp_socket(output: Path, phase: str) -> Path:
    filename = {"save": "s", "read": "r", "recovery-ui": "u"}.get(phase)
    if filename is None:
        raise AcceptanceError("unknown QMP phase")
    output = output.resolve(strict=True)
    if not output.is_dir():
        raise AcceptanceError("QMP output path must be a directory")
    path = output / filename
    if len(os.fsencode(path)) >= 100:
        raise AcceptanceError("QMP socket path exceeds the supported Unix-domain path length")
    if path.exists():
        raise AcceptanceError("refusing to reuse an existing QMP socket path")
    return path


def qemu_argv(
    args: argparse.Namespace,
    initramfs: Path,
    disk: Path,
    serial: Path,
    qmp: Path,
    *,
    recovery_ui_smoke: bool = False,
) -> list[str]:
    argv = [
        str(args.qemu),
        "-machine",
        "q35,i8042=off" if recovery_ui_smoke else "q35",
        "-accel",
        "tcg,thread=multi",
        "-cpu",
        "max",
        "-smp",
        "4",
        "-m",
        "8192",
        "-kernel",
        str(args.kernel),
        "-initrd",
        str(initramfs),
        "-append",
        "console=ttyS0 rdinit=/init",
        "-drive",
        f"if=none,file={disk},format=raw,id=notesdisk",
        "-device",
        "virtio-blk-pci,drive=notesdisk",
        "-device",
        "VGA,xres=320,yres=200,edid=on",
        "-display",
        "none",
        "-nic",
        "none",
        "-serial",
        f"file:{serial}",
        "-monitor",
        "none",
        "-qmp",
        f"unix:{qmp},server=on,wait=off",
        "-no-reboot",
    ]
    if recovery_ui_smoke:
        argv.extend(
            [
                "-device",
                "virtio-keyboard-pci,id=notes-recovery-keyboard,serial=nagi-notes-recovery-v1",
            ]
        )
    return argv


def read_new_serial(path: Path, offset: int) -> tuple[str, int]:
    try:
        with path.open("rb") as serial:
            serial.seek(offset)
            payload = serial.read()
    except FileNotFoundError:
        return "", offset
    return payload.decode("utf-8", errors="replace"), offset + len(payload)


def runner_summary(path: Path) -> dict[str, object]:
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    for line in reversed(lines):
        try:
            summary = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(summary, dict) and "qemu_exit" in summary:
            return summary
    raise AcceptanceError("QEMU supervisor did not record its child exit result")


def phase_completion_ready(serial_text: str, phase: str, keys_sent: bool) -> bool:
    if phase not in KEYS:
        raise AcceptanceError(f"unsupported Notes phase {phase!r}")

    phase_name = phase.upper()
    other_phase = "read" if phase == "save" else "save"
    other_name = other_phase.upper()
    if (
        f"NAGI_NOTES_DESKTOP_INPUT_READY phase={other_phase} " in serial_text
        or f"NAGI_NOTES_DESKTOP_{other_name}_PASS" in serial_text
        or f"NAGI_NOTES_DESKTOP_CHILD phase={other_phase} " in serial_text
    ):
        raise AcceptanceError(f"{phase} QEMU emitted a marker for the other Notes phase")

    service_index = serial_text.find(SERVICE_HANDSHAKE)
    ready_index = serial_text.find("nagi-init: state=ready")
    if service_index >= 0 and ready_index >= 0 and service_index >= ready_index:
        raise AcceptanceError("service handshake did not precede service READY")
    input_marker = f"NAGI_NOTES_DESKTOP_INPUT_READY phase={phase} "
    input_index = serial_text.find(input_marker)
    if input_index >= 0 and (
        service_index < 0
        or ready_index < 0
        or service_index >= input_index
        or ready_index >= input_index
    ):
        raise AcceptanceError(f"{phase} Notes input became ready before service READY")

    pass_prefix = f"NAGI_NOTES_DESKTOP_{phase_name}_PASS"
    pass_marker = (
        f'NAGI_NOTES_DESKTOP_{phase_name}_PASS id={NOTES_ID} revision=2 '
        f'{NOTES_BODY} normal_input=PASS fbdev=PASS nagi_vfs=PASS'
    )
    pass_prefix_index = serial_text.find(pass_prefix)
    pass_index = serial_text.find(pass_marker)
    if pass_prefix_index >= 0 and (pass_index < 0 or pass_prefix_index != pass_index):
        raise AcceptanceError(f"{phase} Notes result marker does not match the required result")
    if pass_index >= 0:
        if input_index < 0 or input_index >= pass_index:
            raise AcceptanceError(f"{phase} Notes result preceded its input-ready marker")
        if not keys_sent:
            raise AcceptanceError(f"{phase} Notes result preceded successful QMP key delivery")

    child_prefix = f"NAGI_NOTES_DESKTOP_CHILD phase={phase} "
    child_marker = f"{child_prefix}exit=Some(0) signal=None"
    child_prefix_index = serial_text.find(child_prefix)
    child_index = serial_text.find(child_marker)
    if child_prefix_index >= 0 and (child_index < 0 or child_prefix_index != child_index):
        raise AcceptanceError(f"{phase} Notes child did not report a successful exit")
    if child_index >= 0 and (pass_index < 0 or child_index <= pass_index):
        raise AcceptanceError(f"{phase} Notes child completion did not follow the phase result")
    return pass_index >= 0 and child_index >= 0


def recovery_ui_completion_ready(
    serial_text: str,
    *,
    keys_sent: bool,
    retry_enter_sent: bool,
    discard_start_sent: bool,
    cancel_discard_sent: bool,
    retry_after_cancel_sent: bool,
    final_discard_start_sent: bool,
    confirm_discard_sent: bool,
) -> bool:
    pass_marker = (
        "NAGI_NOTES_RECOVERY_UI_PASS phase=recovery-ui-smoke explicit_discard=1 "
        "dirty_owner=discarded discard_notice=presented persisted_revision=1 persisted_body=empty"
    )
    if pass_marker not in serial_text:
        if "NAGI_NOTES_RECOVERY_UI_PASS" in serial_text:
            raise AcceptanceError("recovery UI completion marker did not match the required state")
        return False

    child_prefix = "NAGI_NOTES_DESKTOP_CHILD phase=save "
    child_marker = f"{child_prefix}exit=Some(0) signal=None"
    if child_prefix not in serial_text:
        # The UI has reported its result, but PID 1 still needs to observe and
        # reap the optional Notes child before the supervisor may stop QEMU.
        return False
    if child_marker not in serial_text:
        raise AcceptanceError("recovery UI Notes child did not report a successful exit")

    ordered_markers = (
        SERVICE_HANDSHAKE,
        "nagi-init: state=ready",
        "NAGI_NOTES_RECOVERY_VFS_FULL phase=recovery-ui-smoke ",
        "NAGI_NOTES_DESKTOP_INPUT_READY phase=save ",
        "NAGI_NOTES_RECOVERY_INPUT_FAILURE_INJECTED phase=recovery-ui-smoke after=KEY_S",
        "NAGI_NOTES_RECOVERY NAGI_NOTES_INPUT_SAVE_FAILED ",
        "NAGI_NOTES_RECOVERY_NOTICE_FAILURE_INJECTED phase=recovery-ui-smoke source=framebuffer_status",
        "NAGI_NOTES_RECOVERY_REOPEN_UNAVAILABLE_INJECTED phase=recovery-ui-smoke remaining=6",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY attempt=1 ",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt=1 pending_payload=retained owner=held user_action_prompt=none result=failed",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY attempt=2 ",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt=2 pending_payload=retained owner=held user_action_prompt=none result=failed",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY attempt=3 ",
        "NAGI_NOTES_RECOVERY_HELD phase=recovery-ui-smoke ",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt=3 pending_payload=cleared owner=held user_action_prompt=none result=delivered",
        "NAGI_NOTES_RECOVERY_RECONNECTED phase=recovery-ui-smoke ",
        "NAGI_NOTES_RECOVERY_STEP step=RetryFailed",
        "NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationStarted",
        "NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationCancelled",
        "NAGI_NOTES_RECOVERY_STEP step=RetryFailed",
        "NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationStarted",
        "NAGI_NOTES_RECOVERY_STEP step=Discarded ",
        pass_marker,
        child_marker,
    )
    cursor = 0
    for marker in ordered_markers:
        position = serial_text.find(marker, cursor)
        if position < 0:
            raise AcceptanceError(f"recovery UI marker missing or out of order: {marker}")
        cursor = position + len(marker)
    if (
        serial_text.count("NAGI_NOTES_RECOVERY_STEP step=RetryFailed") != 2
        or serial_text.count("NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationStarted") != 2
        or serial_text.count("NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationCancelled") != 1
        or serial_text.count("NAGI_NOTES_RECOVERY_STEP step=Discarded ") != 1
    ):
        raise AcceptanceError("recovery UI command count did not match retry/cancel/explicit-discard flow")
    if serial_text.count("NAGI_NOTES_RECOVERY_NOTICE_RETRY attempt=") != 3:
        raise AcceptanceError("recovery UI periodic notice retry exceeded or missed its three-attempt owner budget")
    if serial_text.count("NAGI_NOTES_RECOVERY_NOTICE_FAILURE_INJECTED ") != 3:
        raise AcceptanceError("recovery UI notice retry did not exercise the expected transient display failures")
    if serial_text.count("NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT ") != 3:
        raise AcceptanceError("recovery UI notice retry did not retain owner state during each unavailable-input attempt")
    if serial_text.count("NAGI_NOTES_RECOVERY_REOPEN_UNAVAILABLE_INJECTED ") != 7:
        raise AcceptanceError("recovery UI did not hold the input-unavailable loop through the scheduled retries")
    if not all(
        (
            keys_sent,
            retry_enter_sent,
            discard_start_sent,
            cancel_discard_sent,
            retry_after_cancel_sent,
            final_discard_start_sent,
            confirm_discard_sent,
        )
    ):
        raise AcceptanceError("recovery UI result preceded successful QMP input delivery")

    capacity_match = re.search(
        r"NAGI_NOTES_RECOVERY_VFS_FULL phase=recovery-ui-smoke "
        r"successful_files=(\d+) successful_bytes=(\d+) failed_write=Capacity flushed=1",
        serial_text,
    )
    if capacity_match is None:
        raise AcceptanceError("recovery UI did not prove real VFS capacity exhaustion")
    if int(capacity_match.group(1)) <= 0 or int(capacity_match.group(2)) < 6 * 1024 * 1024:
        raise AcceptanceError("recovery UI VFS filler writes were below the minimum gate")
    for marker in (
        "dirty_owner=1 dirty_revision=1 dirty_body=a_s persisted_revision=1 "
        "persisted_body=empty fbdev_notice=save_failed_draft_held waiting_hint=draft_remains_in_memory",
        "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt=3 pending_payload=cleared owner=held user_action_prompt=none result=delivered",
    ):
        if marker not in serial_text:
            raise AcceptanceError("recovery UI did not retain and present the failed draft")
    if "NAGI_NOTES_DESKTOP_SAVE_PASS" in serial_text:
        raise AcceptanceError("recovery UI incorrectly reported a successful save")
    return True


def require_successful_supervisor(
    phase: str, process_returncode: int | None, summary: dict[str, object]
) -> None:
    if process_returncode != 0 or summary.get("qemu_exit") != 0:
        raise AcceptanceError(f"{phase} QEMU did not exit cleanly: {summary}")
    if summary.get("timed_out") is not False or summary.get("cancelled") is not False:
        raise AcceptanceError(f"{phase} QEMU was timed out or cancelled: {summary}")


def run_phase(
    args: argparse.Namespace,
    output: Path,
    disk: Path,
    phase: str,
    *,
    recovery_ui_smoke: bool = False,
) -> None:
    run_label = "recovery-ui" if recovery_ui_smoke else phase
    initramfs = output / f"notes-{phase}.cpio"
    serial_path = output / f"notes-{run_label}.serial.log"
    qemu_log = output / f"notes-{run_label}.qemu.log"
    supervisor_log = output / f"notes-{run_label}.supervisor.log"
    controls_log = output / f"notes-{run_label}.controls.log"
    controls_log.touch(exist_ok=False)
    qmp = unique_qmp_socket(output, run_label)
    qemu_log.parent.mkdir(parents=True, exist_ok=True)
    supervisor_command = [
        sys.executable,
        str(QEMU_RUNNER),
        "--timeout-seconds",
        str(RUN_TIMEOUT_SECONDS),
        "--qmp-socket",
        str(qmp),
        "--qemu-log",
        str(qemu_log),
        "--",
        *qemu_argv(
            args,
            initramfs,
            disk,
            serial_path,
            qmp,
            recovery_ui_smoke=recovery_ui_smoke,
        ),
    ]

    started = time.monotonic()
    with supervisor_log.open("xb") as log:
        process = subprocess.Popen(
            supervisor_command,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
    serial_offset = 0
    serial_text = ""
    sent_keys = False
    retry_enter_sent = False
    discard_start_sent = False
    cancel_discard_sent = False
    retry_after_cancel_sent = False
    final_discard_start_sent = False
    confirm_discard_sent = False
    ready_marker = f"NAGI_NOTES_DESKTOP_INPUT_READY phase={phase} "
    stopping = False

    def stop_supervised_child() -> None:
        nonlocal stopping
        if process.poll() is not None:
            return
        if not stopping:
            stopping = True
            try:
                qmp_command(qmp, "quit", timeout=1.0)
                with controls_log.open("a", encoding="utf-8") as log:
                    log.write("command=quit qmp_reply=return\n")
            except (OSError, RuntimeError, TimeoutError, ValueError):
                process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                log = supervisor_log.read_text(encoding="utf-8", errors="replace")
                match = re.search(r"qemu_child_started pid=(\d+)", log)
                if match is not None:
                    qemu_pid = int(match.group(1))
                    try:
                        os.killpg(qemu_pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        try:
                            os.killpg(qemu_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            raise AcceptanceError("QEMU supervisor failed to reap its owned child") from None
                else:
                    raise AcceptanceError("QEMU supervisor failed to stop before reporting its child PID") from None

    try:
        deadline = started + MARKER_WAIT_SECONDS
        while time.monotonic() < deadline:
            new_text, serial_offset = read_new_serial(serial_path, serial_offset)
            if new_text:
                serial_text += new_text
            if (
                "Kernel panic - not syncing:" in serial_text
                or "NAGI_NOTES_DESKTOP_FAIL " in serial_text
                or "nagi-init: startup failed" in serial_text
                or "nagi-init: failed to mount devtmpfs" in serial_text
            ):
                raise AcceptanceError(f"{phase} guest reported a kernel or Notes failure")
            if recovery_ui_smoke and recovery_ui_completion_ready(
                serial_text,
                keys_sent=sent_keys,
                retry_enter_sent=retry_enter_sent,
                discard_start_sent=discard_start_sent,
                cancel_discard_sent=cancel_discard_sent,
                retry_after_cancel_sent=retry_after_cancel_sent,
                final_discard_start_sent=final_discard_start_sent,
                confirm_discard_sent=confirm_discard_sent,
            ):
                stop_supervised_child()
                break
            if not recovery_ui_smoke and phase_completion_ready(serial_text, phase, sent_keys):
                stop_supervised_child()
                break
            if process.poll() is not None:
                raise AcceptanceError(
                    f"{phase} QEMU supervisor exited before the desktop acceptance result"
                )
            if ready_marker in serial_text and not sent_keys:
                if SERVICE_HANDSHAKE not in serial_text or "nagi-init: state=ready" not in serial_text:
                    raise AcceptanceError(f"{phase} Notes input became ready before service READY")
                for key in (RECOVERY_UI_KEYS if recovery_ui_smoke else KEYS[phase]):
                    qmp_sendkey(qmp, key, timeout=2.0)
                    with controls_log.open("a", encoding="utf-8") as log:
                        log.write(f"command=human-monitor-command key={key} qmp_reply=return\n")
                    time.sleep(0.1 if recovery_ui_smoke else 0.25)
                sent_keys = True
            if recovery_ui_smoke and sent_keys:
                retry_failures = serial_text.count("NAGI_NOTES_RECOVERY_STEP step=RetryFailed")
                discard_starts = serial_text.count(
                    "NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationStarted"
                )
                discard_cancellations = serial_text.count(
                    "NAGI_NOTES_RECOVERY_STEP step=DiscardConfirmationCancelled"
                )

                def send_recovery_key(key: str) -> None:
                    qmp_sendkey(qmp, key, timeout=2.0)
                    with controls_log.open("a", encoding="utf-8") as log:
                        log.write(f"command=human-monitor-command key={key} qmp_reply=return\n")

                if (
                    "NAGI_NOTES_RECOVERY_RECONNECTED phase=recovery-ui-smoke " in serial_text
                    and not retry_enter_sent
                ):
                    send_recovery_key("ret")
                    retry_enter_sent = True
                elif retry_enter_sent and retry_failures >= 1 and not discard_start_sent:
                    send_recovery_key("esc")
                    discard_start_sent = True
                elif discard_start_sent and discard_starts >= 1 and not cancel_discard_sent:
                    send_recovery_key("esc")
                    cancel_discard_sent = True
                elif cancel_discard_sent and discard_cancellations >= 1 and not retry_after_cancel_sent:
                    send_recovery_key("ret")
                    retry_after_cancel_sent = True
                elif retry_after_cancel_sent and retry_failures >= 2 and not final_discard_start_sent:
                    send_recovery_key("esc")
                    final_discard_start_sent = True
                elif final_discard_start_sent and discard_starts >= 2 and not confirm_discard_sent:
                    send_recovery_key("ret")
                    confirm_discard_sent = True
            time.sleep(0.1)
        else:
            raise AcceptanceError(f"{phase} guest did not complete before the marker deadline")

        if process.poll() is None:
            stop_supervised_child()
        summary = runner_summary(supervisor_log)
        require_successful_supervisor(run_label, process.returncode, summary)
        if recovery_ui_smoke:
            print("recovery_ui=PASS save_failed=true dirty_owner_held=true explicit_discard=true child_exit=0 qemu_exit=0 timed_out=false")
        else:
            print(f"phase={phase} UI=PASS child_exit=0 qemu_exit=0 timed_out=false")
    finally:
        if process.poll() is None:
            stop_supervised_child()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--init", type=Path, required=True, help="the already-built Linux nagi-init ELF")
    parser.add_argument("--notes-elf", type=Path, required=True, help="the already-built Notes desktop ELF reused in both phases")
    parser.add_argument("--package", type=Path, required=True, help="fixed signed development package")
    parser.add_argument("--service", type=Path, required=True, help="fixed development service ELF")
    parser.add_argument("--client", type=Path, required=True, help="fixed development client ELF")
    parser.add_argument("--kernel", type=Path, required=True, help="the accepted x86_64 Linux kernel image")
    parser.add_argument("--qemu", type=Path, required=True, help="qemu-system-x86_64 executable")
    parser.add_argument("--output-dir", type=Path, required=True, help="new, unused directory for CPIOs, logs, and the disk")
    parser.add_argument(
        "--recovery-ui-smoke",
        action="store_true",
        help="run the Linux development VFS-full, held-draft, explicit-discard UI scenario",
    )
    args = parser.parse_args()

    try:
        args.init = require_regular_file(args.init, "init")
        args.notes_elf = require_regular_file(args.notes_elf, "Notes desktop")
        args.package = require_regular_file(args.package, "signed development package")
        args.service = require_regular_file(args.service, "development service")
        args.client = require_regular_file(args.client, "development client")
        args.kernel = require_regular_file(args.kernel, "Linux kernel")
        args.qemu = require_regular_file(args.qemu, "QEMU", executable=True)
        kernel_header = validate_x86_64_kernel(args.kernel)
        print(
            "kernel=x86_64-bzImage "
            f"boot_protocol=0x{kernel_header['boot_protocol_version']:04x} "
            f"xloadflags=0x{kernel_header['xloadflags']:04x}"
        )
        output = args.output_dir.expanduser().resolve()
        output.mkdir(parents=False, exist_ok=False)
        phases = ("save",) if args.recovery_ui_smoke else ("save", "read")
        for phase in phases:
            build_phase_cpio(args, output / f"notes-{phase}.cpio", phase)
        disk = output / "notes-persistence.raw"
        create_blank_disk(disk)
        if args.recovery_ui_smoke:
            print(f"disk=created bytes={DISK_BYTES} scenario=recovery_ui_smoke")
            run_phase(args, output, disk, "save", recovery_ui_smoke=True)
            print("normal_ui_save_failure_recovery_and_explicit_discard=PASS")
        else:
            print(f"disk=created bytes={DISK_BYTES} same_disk_for_both_phases=true")
            for phase in phases:
                run_phase(args, output, disk, phase)
            print("normal_ui_save_and_same_disk_restart_readback=PASS")
        return 0
    except (AcceptanceError, OSError, subprocess.SubprocessError) as error:
        print(f"acceptance=FAIL reason={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
