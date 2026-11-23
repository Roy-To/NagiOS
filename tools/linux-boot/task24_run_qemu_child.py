#!/usr/bin/env python3
"""Run one foreground QEMU child with QMP or an exact serial-marker stop."""

from __future__ import annotations

import argparse
import json
import os
import re
import selectors
import signal
import socket
import stat
import subprocess
import sys
import time
from pathlib import Path

from task24_recovery_serials import CURRENT_TERMINAL, RECEIPT_VALUES

MAX_SERIAL_LOG_BYTES = 16 * 1024 * 1024
MAX_SERIAL_LINE_BYTES = 16 * 1024


class QmpConnection:
    """Small deadline-aware line reader for one QMP connection."""

    def __init__(self, connection: socket.socket) -> None:
        self.connection = connection
        self.buffer = bytearray()

    def readline(self, deadline: float) -> bytes:
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                if newline > 1_048_576:
                    raise RuntimeError("QMP line exceeds 1 MiB")
                line = bytes(self.buffer[: newline + 1])
                del self.buffer[: newline + 1]
                return line
            if len(self.buffer) > 1_048_576:
                raise RuntimeError("QMP line exceeds 1 MiB")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("QMP response deadline exceeded")
            self.connection.settimeout(remaining)
            try:
                chunk = self.connection.recv(4096)
            except socket.timeout as error:
                raise TimeoutError("QMP response deadline exceeded") from error
            if not chunk:
                raise RuntimeError("QMP closed before completing a response")
            self.buffer.extend(chunk)

    def write_json(self, payload: dict[str, object], deadline: float) -> None:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("QMP response deadline exceeded")
        self.connection.settimeout(remaining)
        encoded = json.dumps(payload, separators=(",", ":")).encode() + b"\r\n"
        try:
            self.connection.sendall(encoded)
        except socket.timeout as error:
            raise TimeoutError("QMP response deadline exceeded") from error


def qmp_command(
    socket_path: Path,
    command: str,
    timeout: float = 2.0,
    *,
    arguments: dict[str, object] | None = None,
) -> None:
    deadline = time.monotonic() + timeout
    error: OSError | None = None
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        while True:
            try:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("QMP connect deadline exceeded")
                connection.settimeout(remaining)
                connection.connect(str(socket_path))
                break
            except OSError as caught:
                error = caught
                if time.monotonic() >= deadline:
                    raise error
                time.sleep(0.05)
        stream = QmpConnection(connection)
        greeting = json.loads(stream.readline(deadline))
        if "QMP" not in greeting:
            raise RuntimeError("QMP greeting missing")
        command_payload: dict[str, object] = {"execute": command}
        if arguments is not None:
            command_payload["arguments"] = arguments
        for payload in ({"execute": "qmp_capabilities"}, command_payload):
            stream.write_json(payload, deadline)
            while True:
                response = json.loads(stream.readline(deadline))
                if "event" in response:
                    continue
                if "error" in response:
                    raise RuntimeError(f"QMP command failed: {response['error'].get('class', 'unknown')}")
                if "return" in response:
                    break
    finally:
        connection.close()


_ALLOWED_SENDKEYS = frozenset({"tab", "ret", "esc", "a", "s", "spc"})


def qmp_sendkey(socket_path: Path, key: str, timeout: float = 2.0) -> None:
    """Send one allowlisted UI key through QMP's human-monitor command."""
    normalized = key.lower()
    if normalized not in _ALLOWED_SENDKEYS:
        raise ValueError("key is outside the fixed desktop-smoke input set")
    qmp_command(
        socket_path,
        "human-monitor-command",
        timeout,
        arguments={"command-line": f"sendkey {normalized}"},
    )


def wait_child(process: subprocess.Popen, timeout: float) -> int | None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = process.poll()
        if result is not None:
            return result
        time.sleep(0.05)
    return process.poll()


def terminate_owned_child(process: subprocess.Popen, timeout: float = 5.0) -> int:
    """Terminate and reap only the foreground child started by this runner."""
    if process.poll() is None:
        try:
            process.terminate()
        except ProcessLookupError:
            pass
        if wait_child(process, timeout) is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
    return process.wait()


class SerialMarkerWatcher:
    """Match one exact CRLF-tolerant line while reading only appended bytes."""

    def __init__(self, path: Path, marker: str, *, task24_receipt: bool = False) -> None:
        self.path = path
        self.expected = marker.encode("utf-8")
        self.receipt = re.compile((re.escape(marker) + RECEIPT_VALUES).encode()) if task24_receipt else None
        self.descriptor: int | None = None
        self.pending = bytearray()
        self.bytes_read = 0

    def close(self) -> None:
        if self.descriptor is not None:
            os.close(self.descriptor)
            self.descriptor = None

    def _matches(self, line: bytes) -> bool:
        if self.receipt is not None:
            return self.receipt.fullmatch(line) is not None
        return line == self.expected

    def seen(self, *, final: bool = False) -> bool:
        if self.descriptor is None:
            try:
                self.descriptor = os.open(
                    self.path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
                )
            except FileNotFoundError:
                return False

        metadata = os.fstat(self.descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_SERIAL_LOG_BYTES:
            raise ValueError("serial marker log is not a regular file or exceeds 16 MiB")
        if metadata.st_size < self.bytes_read:
            raise ValueError("serial marker log was truncated while being observed")

        while True:
            block = os.read(self.descriptor, 64 * 1024)
            if not block:
                break
            self.bytes_read += len(block)
            self.pending.extend(block)
            while True:
                newline = self.pending.find(b"\n")
                if newline < 0:
                    if len(self.pending) > MAX_SERIAL_LINE_BYTES:
                        raise ValueError("serial marker log contains an overlong line")
                    break
                line = bytes(self.pending[:newline])
                del self.pending[: newline + 1]
                if line.endswith(b"\r"):
                    line = line[:-1]
                if self._matches(line):
                    return True

        if final:
            line = bytes(self.pending)
            if line.endswith(b"\r"):
                line = line[:-1]
            return self._matches(line)
        return False


def validate_qemu_argv(
    argv: list[str], qmp_argument: str | None, serial_argument: str | None = None
) -> None:
    if not argv or not argv[0]:
        raise ValueError("QEMU argv is required after --")
    if any(argument == "-daemonize" for argument in argv[1:]):
        raise ValueError("-daemonize is incompatible with foreground child supervision")
    if qmp_argument is not None:
        if qmp_argument not in argv:
            raise ValueError("QEMU argv must include the requested QMP socket")
        return
    if "-qmp" in argv:
        raise ValueError("serial-marker shutdown must not configure QMP")
    if serial_argument is None or not any(
        argv[index] == "-serial" and index + 1 < len(argv)
        and argv[index + 1] == serial_argument
        for index in range(len(argv))
    ):
        raise ValueError("serial-marker shutdown must include the requested file serial log")
    if not any(
        argv[index] == "-monitor" and index + 1 < len(argv) and argv[index + 1] == "none"
        for index in range(len(argv))
    ):
        raise ValueError("serial-marker shutdown requires -monitor none")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--shutdown-mode", choices=("qmp", "serial-marker"), default="qmp")
    parser.add_argument("--qmp-socket", type=Path)
    parser.add_argument("--serial-log", type=Path)
    parser.add_argument("--success-marker")
    parser.add_argument("--task24-terminal-receipt", action="store_true")
    parser.add_argument(
        "--wait-for-natural-exit",
        action="store_true",
        help="after the serial marker, wait for the owned child to exit on its own",
    )
    parser.add_argument("--qemu-log", type=Path, required=True)
    parser.add_argument("qemu_argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not 1 <= args.timeout_seconds <= 600:
        parser.error("timeout must be between 1 and 600 seconds")
    if args.qemu_argv[:1] == ["--"]:
        args.qemu_argv = args.qemu_argv[1:]
    if not args.qemu_log.parent.is_dir():
        parser.error("QEMU log parent directory must already exist")
    if args.task24_terminal_receipt and not (
        args.shutdown_mode == "serial-marker" and args.wait_for_natural_exit
        and args.success_marker == CURRENT_TERMINAL
    ):
        parser.error("Task24 receipt watching requires its terminal marker and natural-exit mode")

    qmp_argument: str | None = None
    serial_argument: str | None = None
    if args.shutdown_mode == "qmp":
        if args.wait_for_natural_exit:
            parser.error("--wait-for-natural-exit requires serial-marker shutdown")
        if args.qmp_socket is None:
            parser.error("--shutdown-mode qmp requires --qmp-socket")
        if args.serial_log is not None or args.success_marker is not None:
            parser.error("--serial-log and --success-marker are only valid in serial-marker mode")
        if args.qmp_socket.exists() or args.qmp_socket.is_symlink():
            parser.error("QMP socket path already exists")
        qmp_argument = f"unix:{args.qmp_socket},server=on,wait=off"
    else:
        if args.qmp_socket is not None:
            parser.error("serial-marker shutdown does not accept --qmp-socket")
        if args.serial_log is None or args.success_marker is None:
            parser.error("--shutdown-mode serial-marker requires --serial-log and --success-marker")
        if not args.success_marker or "\r" in args.success_marker or "\n" in args.success_marker:
            parser.error("--success-marker must be one nonempty serial line")
        if len(args.success_marker.encode("utf-8")) > 2048:
            parser.error("--success-marker exceeds 2048 bytes")
        if not args.serial_log.parent.is_dir():
            parser.error("serial log parent directory must already exist")
        if args.serial_log.exists() or args.serial_log.is_symlink():
            parser.error("refusing to reuse a serial log")
        serial_argument = f"file:{args.serial_log}"

    try:
        validate_qemu_argv(args.qemu_argv, qmp_argument, serial_argument)
    except ValueError as error:
        parser.error(str(error))

    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        log_fd = os.open(args.qemu_log, flags, 0o600)
    except FileExistsError:
        parser.error("refusing to overwrite a QEMU log")

    process: subprocess.Popen | None = None
    selector = selectors.DefaultSelector()
    stop_requested = False
    stop_signal: int | None = None
    timed_out = False
    marker_seen = False
    stopped_on_marker = False
    control_error: str | None = None
    started = 0.0
    marker_watcher = (
        SerialMarkerWatcher(
            args.serial_log, args.success_marker, task24_receipt=args.task24_terminal_receipt,
        )
        if args.shutdown_mode == "serial-marker"
        else None
    )

    def request_stop(signum: int, _frame: object) -> None:
        nonlocal stop_requested, stop_signal
        stop_requested = True
        stop_signal = signum

    previous_handlers = {
        signum: signal.signal(signum, request_stop)
        for signum in (signal.SIGINT, signal.SIGTERM)
    }
    try:
        with os.fdopen(log_fd, "wb", closefd=True) as qemu_log:
            try:
                process = subprocess.Popen(
                    args.qemu_argv,
                    stdin=subprocess.DEVNULL,
                    stdout=qemu_log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
            except OSError as error:
                print(f"qemu child could not start ({error.__class__.__name__})", file=sys.stderr)
                return 127
            started = time.monotonic()
            print(f"qemu_child_started pid={process.pid}", flush=True)
            if args.shutdown_mode == "qmp":
                try:
                    selector.register(sys.stdin, selectors.EVENT_READ)
                except (OSError, ValueError):
                    pass

            while process.poll() is None:
                if stop_requested:
                    if args.shutdown_mode == "qmp":
                        try:
                            qmp_command(args.qmp_socket, "quit", timeout=1.0)
                        except (OSError, RuntimeError, ValueError):
                            terminate_owned_child(process)
                        else:
                            if wait_child(process, 5.0) is None:
                                terminate_owned_child(process)
                    else:
                        terminate_owned_child(process)
                    break

                elapsed = time.monotonic() - started
                if elapsed >= args.timeout_seconds:
                    timed_out = True
                    if args.shutdown_mode == "qmp":
                        requested_powerdown = False
                        try:
                            qmp_command(args.qmp_socket, "system_powerdown", timeout=1.0)
                            requested_powerdown = True
                        except (OSError, RuntimeError, ValueError):
                            pass
                        if requested_powerdown:
                            wait_child(process, 15.0)
                        terminate_owned_child(process)
                    else:
                        terminate_owned_child(process)
                    break

                if args.shutdown_mode == "serial-marker":
                    try:
                        if not marker_seen:
                            marker_seen = marker_watcher.seen()
                    except (OSError, ValueError) as error:
                        control_error = f"serial marker check failed ({error.__class__.__name__})"
                        terminate_owned_child(process)
                        break
                    if marker_seen:
                        if args.wait_for_natural_exit:
                            time.sleep(min(0.05, args.timeout_seconds - elapsed))
                            continue
                        if process.poll() is None:
                            stopped_on_marker = True
                            terminate_owned_child(process)
                        break
                    time.sleep(min(0.05, args.timeout_seconds - elapsed))
                    continue

                for key, _ in selector.select(timeout=min(0.25, args.timeout_seconds - elapsed)):
                    line = key.fileobj.readline()
                    if not line:
                        selector.unregister(key.fileobj)
                        continue
                    if isinstance(line, bytes):
                        line = line.decode("utf-8", errors="replace")
                    raw_command = line.strip()
                    command = raw_command.upper()
                    if command in ("QUIT", "POWERDOWN"):
                        try:
                            qmp_command(
                                args.qmp_socket,
                                "quit" if command == "QUIT" else "system_powerdown",
                                timeout=1.0,
                            )
                        except (OSError, RuntimeError, ValueError):
                            if command == "QUIT":
                                terminate_owned_child(process)
                        if command == "QUIT":
                            if process.poll() is None and wait_child(process, 5.0) is None:
                                terminate_owned_child(process)
                            break
                    elif command.startswith("KEY "):
                        key_name = raw_command.split(maxsplit=1)[1]
                        qmp_sendkey(args.qmp_socket, key_name, timeout=1.0)
                    elif command:
                        print(
                            "accepted_control=QUIT|POWERDOWN|KEY tab|KEY ret|KEY a|KEY s|KEY spc",
                            flush=True,
                        )

            if (
                args.shutdown_mode == "serial-marker"
                and args.wait_for_natural_exit
                and not timed_out
                and stop_signal is None
                and control_error is None
                and time.monotonic() - started >= args.timeout_seconds
            ):
                # A delayed poll can observe natural exit after the loop's deadline.
                timed_out = True
            return_code = process.wait()
            if (
                args.shutdown_mode == "serial-marker"
                and not marker_seen
                and not timed_out
                and control_error is None
            ):
                try:
                    marker_seen = marker_watcher.seen(final=True)
                except (OSError, ValueError) as error:
                    control_error = f"serial marker check failed ({error.__class__.__name__})"
            elapsed = time.monotonic() - started
            result = {
                "qemu_exit": return_code,
                "timed_out": timed_out,
                "cancelled": stop_signal is not None,
                "cancel_signal": stop_signal,
                "elapsed_seconds": round(elapsed, 3),
            }
            if args.shutdown_mode == "serial-marker":
                result["success_marker_seen"] = marker_seen
                result["stopped_on_marker"] = stopped_on_marker
                result["wait_for_natural_exit"] = args.wait_for_natural_exit
            print(json.dumps(result, sort_keys=True), flush=True)
            if control_error is not None:
                print(control_error, file=sys.stderr)
                return 2
            if stop_signal is not None:
                return 128 + stop_signal
            if timed_out:
                return 124
            if args.shutdown_mode == "serial-marker":
                if args.wait_for_natural_exit:
                    if marker_seen and return_code == 0:
                        return 0
                    return return_code if return_code != 0 else 1
                if marker_seen and (stopped_on_marker or return_code == 0):
                    return 0
                return return_code if return_code != 0 else 1
            return return_code
    finally:
        selector.close()
        if marker_watcher is not None:
            marker_watcher.close()
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)
        if process is not None and process.poll() is None:
            terminate_owned_child(process)
        if args.qmp_socket is not None:
            try:
                if stat.S_ISSOCK(args.qmp_socket.lstat().st_mode):
                    args.qmp_socket.unlink()
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
