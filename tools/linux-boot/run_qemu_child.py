#!/usr/bin/env python3
"""Run one foreground QEMU child with a hard timeout and explicit QMP control."""

from __future__ import annotations

import argparse
import json
import os
import selectors
import signal
import socket
import stat
import subprocess
import sys
import time
from pathlib import Path


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


def validate_qemu_argv(argv: list[str], qmp_argument: str) -> None:
    if not argv or not argv[0]:
        raise ValueError("QEMU argv is required after --")
    if any(argument == "-daemonize" for argument in argv[1:]):
        raise ValueError("-daemonize is incompatible with foreground child supervision")
    if qmp_argument not in argv:
        raise ValueError("QEMU argv must include the requested QMP socket")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--qmp-socket", type=Path, required=True)
    parser.add_argument("--qemu-log", type=Path, required=True)
    parser.add_argument("qemu_argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not 1 <= args.timeout_seconds <= 600:
        parser.error("timeout must be between 1 and 600 seconds")
    if args.qemu_argv[:1] == ["--"]:
        args.qemu_argv = args.qemu_argv[1:]
    if args.qmp_socket.exists():
        parser.error("QMP socket path already exists")
    if not args.qemu_log.parent.is_dir():
        parser.error("QEMU log parent directory must already exist")
    qmp_argument = f"unix:{args.qmp_socket},server=on,wait=off"
    try:
        validate_qemu_argv(args.qemu_argv, qmp_argument)
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
            process = subprocess.Popen(
                args.qemu_argv,
                stdin=subprocess.DEVNULL,
                stdout=qemu_log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            started = time.monotonic()
            print(f"qemu_child_started pid={process.pid}", flush=True)
            try:
                selector.register(sys.stdin, selectors.EVENT_READ)
            except (OSError, ValueError):
                pass
            timed_out = False
            while process.poll() is None:
                if stop_requested:
                    try:
                        qmp_command(args.qmp_socket, "quit", timeout=1.0)
                    except (OSError, RuntimeError, ValueError):
                        process.terminate()
                    if wait_child(process, 5.0) is None:
                        process.terminate()
                    if wait_child(process, 5.0) is None:
                        process.kill()
                    break
                elapsed = time.monotonic() - started
                if elapsed >= args.timeout_seconds:
                    timed_out = True
                    requested_powerdown = False
                    try:
                        qmp_command(args.qmp_socket, "system_powerdown", timeout=1.0)
                        requested_powerdown = True
                    except (OSError, RuntimeError, ValueError):
                        process.terminate()
                    if requested_powerdown and wait_child(process, 15.0) is None:
                        process.terminate()
                    if wait_child(process, 5.0) is None:
                        process.kill()
                    break
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
                                process.terminate()
                        if command == "QUIT":
                            if wait_child(process, 5.0) is None:
                                process.terminate()
                            if wait_child(process, 5.0) is None:
                                process.kill()
                            break
                    elif command.startswith("KEY "):
                        key = raw_command.split(maxsplit=1)[1]
                        qmp_sendkey(args.qmp_socket, key, timeout=1.0)
                    elif command:
                        print(
                            "accepted_control=QUIT|POWERDOWN|KEY tab|KEY ret|KEY a|KEY s|KEY spc",
                            flush=True,
                        )
            return_code = process.wait()
            elapsed = time.monotonic() - started
            print(
                json.dumps(
                    {
                        "qemu_exit": return_code,
                        "timed_out": timed_out,
                        "cancelled": stop_signal is not None,
                        "cancel_signal": stop_signal,
                        "elapsed_seconds": round(elapsed, 3),
                    },
                    sort_keys=True,
                ),
                flush=True,
            )
            if stop_signal is not None:
                return 128 + stop_signal
            return 124 if timed_out else return_code
    finally:
        selector.close()
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)
        if process is not None and process.poll() is None:
            process.terminate()
            if wait_child(process, 5.0) is None:
                process.kill()
            process.wait()
        try:
            if stat.S_ISSOCK(args.qmp_socket.lstat().st_mode):
                args.qmp_socket.unlink()
        except FileNotFoundError:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
