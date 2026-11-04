#!/usr/bin/python3
"""Test-only QEMU orchestration peer; no guest acceptance is simulated."""
import json
import os
from pathlib import Path
import socket
import sys
import time

root = Path(__file__).parent
(root / "pid").write_text(str(os.getpid()))
args = sys.argv[1:]
qmp = args[args.index("-qmp") + 1]
serial = args[args.index("-serial") + 1]

def listen(device):
    port = int(device.split(":")[2].split(",")[0])
    sock = socket.socket()
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("127.0.0.1", port))
    sock.listen()
    return sock

qmp_listener = listen(qmp)
serial_listener = listen(serial) if serial.startswith("tcp:") else None
if serial_listener:
    serial_stream, _ = serial_listener.accept()
else:
    Path(serial.removeprefix("file:")).write_text("")
stream, _ = qmp_listener.accept()
with stream, stream.makefile("rwb", buffering=0) as peer:
    peer.write(b'{"QMP":{"version":{"qemu":{"major":9,"minor":2,"micro":0},"package":""},"capabilities":[]}}\r\n')
    for line in peer:
        request = json.loads(line)
        command = request["execute"]
        with (root / "requests").open("a") as log:
            log.write(line.decode())
        if command == "query-status" and (root / "stall").exists():
            time.sleep(30)
        value = {"status": "running", "running": True} if command == "query-status" else {}
        if command == "human-monitor-command":
            value = "RIP=000000007eb84171 firmware instruction"
        peer.write((json.dumps({"return": value}) + "\r\n").encode())
