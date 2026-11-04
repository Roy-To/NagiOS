#!/usr/bin/env python3
"""Check a bounded ordinary Files probe's artifact chain, never formal M30.

This checks trusted local QEMU collector artifacts, not arbitrary submitted
evidence. Screenshots still require review for the Search result and Bar state.
It neither invokes session_acceptance.py nor produces NAGI_SESSION_EVENT records.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def regular(root, path):
    path = Path(path)
    require(".." not in path.parts, "artifact path traversal")
    if not path.is_absolute():
        path = root / path
    require(not any(p.is_symlink() for p in (path, *path.parents)), "symlink artifact")
    require(path.is_relative_to(root) and path.is_file(), f"missing/outside artifact: {path}")
    return path


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def pairs(items):
    value = {}
    for key, item in items:
        require(key not in value, f"duplicate JSON key: {key}")
        value[key] = item
    return value


def read_json(path):
    require(0 < path.stat().st_size <= 1024 * 1024, "empty/oversized JSON")
    def nonfinite(value):
        raise ValueError(f"nonfinite JSON number: {value}")
    return json.loads(path.read_text(), object_pairs_hook=pairs, parse_constant=nonfinite)


def check(root, phase_paths):
    root = root.absolute()
    require(".." not in root.parts, "root path traversal")
    require(not any(p.is_symlink() for p in (root, *root.parents)), "symlink root")
    image = revision = previous = None
    report = []
    phase_dirs = set()
    for phase, directory in zip(("format", "session", "restart", "offline"), phase_paths):
        directory = root / directory
        require(directory not in phase_dirs, "phase reused")
        phase_dirs.add(directory)
        result = read_json(regular(root, directory / "result.json"))
        current_image = regular(root, result["image"])
        if image is None:
            image = current_image
            revision = result["source_revision"]
            require(re.fullmatch(r"[0-9a-f]{40}", revision) is not None, "invalid revision")
            previous = sha(image)
            provenance = dict(line.split("=", 1) for line in
                              regular(root, str(image) + ".build-info").read_text().splitlines())
            require(provenance.get("source_revision") == revision
                    and provenance.get("image_sha256") == previous,
                    "image provenance mismatch")
            require(provenance.get("profile") == "native-provider-session-v1"
                    and provenance.get("init_features") == "production-session,m20-model-service",
                    "this probe requires the ordinary native partial profile")
        require(current_image == image and result["source_revision"] == revision,
                "mixed image/source scope")
        require(result["image_sha256"] == sha(image) and result["source_image_unchanged"] is True,
                "pristine image changed")
        require(result["disk_start_sha256"] == previous, "broken stopped-disk hash chain")
        disk = regular(root, directory / "disk.qcow2")
        require(result["disk_end_sha256"] == sha(disk), "disk snapshot hash mismatch")
        previous = result["disk_end_sha256"]
        require(type(result["qemu_exit"]) is int and result["qemu_exit"] == 0
                and result["M30"] == "BLOCKED", "invalid probe result")
        expected = "first-boot-formatted" if phase == "format" else "ordinary-desktop-signed-in"
        require(result["reason"] == expected, f"incomplete {phase} probe")
        log_path = regular(root, directory / "serial.log")
        require(0 < log_path.stat().st_size <= 16 * 1024**2, "empty/oversized serial log")
        log = log_path.read_text()
        require(not re.search(r"\bFAIL(?:ED|URE)?\b|\bpanic\b|\bOOM\b", log), "guest failure")
        command = read_json(regular(root, directory / "command.json"))
        require(isinstance(command, list) and all(isinstance(v, str) for v in command),
                "invalid QEMU command")
        for option, value in (("-machine", "q35"), ("-smp", "4"), ("-m", "8G")):
            require(option in command and command[command.index(option) + 1] == value,
                    "reference machine mismatch")
        if phase == "format":
            require("Nagi M7 persistent write PASS" in log
                    and "Nagi M7 reboot required PASS" in log, "format/write not observed")
            screenshots = ["screen.ppm"]
        else:
            for marker in ("Nagi M7 persistent read PASS", "Nagi production profile signed-in-files-v1",
                           "Nagi login readiness reported PASS",
                           "Nagi M19 signed-in desktop SearchService ready PASS",
                           "Nagi ADR0043 isolated process spawned pid=",
                           "Nagi M19 Files Search child reaped PASS"):
                require(marker in log, f"missing {phase} observation: {marker}")
            regular(root, directory / "input.jsonl")
            if phase == "session":
                operations = re.findall(r"Nagi Files UI operation PASS operation=(\w+)", log)
                require(operations == ["create", "rename", "create", "trash", "restore"],
                        "Files lifecycle differs from the collected probe")
                require("capability=search.query" in log and "capability=files.search" in log,
                        "initial signed Search consent not observed")
                require(log.count("Nagi login readiness reported PASS") == 2,
                        "lock/relogin not observed")
                screenshots = ["renamed.ppm", "trash-view.ppm", "search-finished.ppm",
                               "worker-reply.ppm", "relogged-bar.ppm"]
            elif phase == "restart":
                require("Nagi login READY mode=unlock" in log, "Owner not restored")
                screenshots = ["search-after-restart.ppm", "files-after-restart.ppm"]
            else:
                offline = read_json(regular(root, directory / "offline.json"))
                require(offline == {"network_link_up": False, "applied_before_guest_execution": True}
                        and "-S" in command, "offline boot not recorded")
                require("Nagi Files UI operation PASS operation=create" in log,
                        "offline create not observed")
                screenshots = ["offline-search.ppm", "offline-created.ppm"]
        frames = {}
        for name in screenshots:
            frame = regular(root, directory / name)
            require(frame.stat().st_size > 1000 and frame.read_bytes()[:2] == b"P6", "invalid frame")
            frames[name] = sha(frame)
        report.append({"phase": phase, "directory": str(directory.relative_to(root)),
                       "result_sha256": sha(directory / "result.json"),
                       "command_sha256": sha(directory / "command.json"),
                       "input_sha256": None if phase == "format" else sha(directory / "input.jsonl"),
                       "offline_sha256": sha(directory / "offline.json") if phase == "offline" else None,
                       "serial_sha256": sha(log_path), "disk_end_sha256": previous,
                       "frame_sha256": frames})
    return {"schema_version": 1, "scope": "ordinary-files-probe-artifact-chain",
            "source_revision": revision, "image": str(image.relative_to(root)),
            "image_sha256": sha(image), "phases": report,
            "build_info_sha256": sha(Path(str(image) + ".build-info")),
            "manual_review_required": "Search names/count, Bar failure and cleared relogin frame",
            "inference": "NOT_EVALUATED", "ObjectId_content_hash_acceptance": "NOT_EVALUATED",
            "M30": "BLOCKED", "collector_sha256": sha(Path(__file__))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("phases", nargs=4, type=Path, metavar="PHASE_DIRECTORY")
    args = parser.parse_args()
    try:
        result = check(args.root, args.phases)
    except (ValueError, OSError, KeyError, TypeError, IndexError) as error:
        print(f"ordinary Files probe rejected: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
