#!/usr/bin/env python3
"""Validate integrated guest evidence, never distribution/legal acceptance.

Proposed producer API (no runtime implementation here): one UTF-8 JSON object
per serial-log line prefixed by EVENT_PREFIX. Required keys are event, session_id,
phase, disk_id, image_sha256, data. Every event binds the original image, including
restart/offline events. EVENT_SEQUENCES documents the required ordered constants.
Data contracts are enforced in _operations and _stress below; producers must emit
records only after successful real guest operations, not predicted/canned results.
Stress records include positive duration_ms/desktop_samples/audio_frames,
memory_bytes_start/end and handles_start/end; growth must match these readings.
STRESS_LIMITS fixes the ceilings; the submitted limits object must equal it.
Hashes bind artifacts, not the honesty of the collector: retain trusted QEMU logs.

Manifest v1 keys: schema_version, source_revision, image_sha256, profile,
session_id, phases. phases maps boot/restart/offline to path, sha256, disk_id,
initial_image_sha256, disk_start_sha256, disk_end_sha256, machine, network.
Paths are relative to --root. machine is exactly MACHINE. boot's starting disk
hash equals the original image hash; later starts equal the preceding end hash.
Logs are at most 16 MiB; manifest at most 1 MiB. Unknown manifest keys, duplicate
JSON keys, nonfinite numbers, unsafe paths, and missing events fail closed.

Public API: validate_session(root, image, evidence, revision) -> None. Caller
supplies a verified revision; CLI obtains it from git_source_revision (clean tree).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

from release import (GRANITE_SHA256, ReleaseError, git_source_revision,
                     image_build_provenance, require_production_image)

EVENT_PREFIX = "NAGI_SESSION_EVENT "
PROFILE = "production-session-v1"
MACHINE = {"type": "q35", "arch": "x86_64", "vcpus": 4, "memory_bytes": 8 * 1024**3}
MAX_LOG_BYTES = 16 * 1024**2
MAX_MANIFEST_BYTES = 1024**2
SIGN_IN = ("session.profile", "session.start", "login.unlocked", "login.ready")
OPERATIONS = ("files.created", "files.renamed", "files.searched", "files.deleted",
              "ai.user_request", "ai.summary", "ai.plan", "plan.validated",
              "plan.policy_allowed", "plan.user_confirmed", "plan.executed",
              "plan.undone", "servo.user_navigation", "servo.history_recorded")
EVENT_SEQUENCES = {
    "boot": SIGN_IN + OPERATIONS + ("stress.measured",),
    "restart": SIGN_IN + ("files.restored", "servo.history_restored"),
    "offline": SIGN_IN + OPERATIONS[:-2] + ("servo.history_restored", "offline.completed", "stress.measured"),
}
# Explicit session acceptance ceilings, not configurable by submitted evidence.
STRESS_LIMITS = {"desktop_latency_ms": 250, "audio_underruns": 0,
                 "memory_growth_bytes": 64 * 1024**2, "handle_growth": 32,
                 "granite_cpu_percent": 300, "oom_count": 0}
# Existing init reports low-level bootstrap gates before entering the ordinary
# session. Those are diagnostics, not substitutes for session operation evidence.
BOOTSTRAP_MARKERS = {f"Nagi M{milestone} acceptance PASS" for milestone in range(1, 8)}
BAD_LOG = re.compile(r"acceptance|fixture|\bFAIL(?:ED|URE)?\b|\bpanic\b|"
                     r"\bOOM\b|out[- ]of[- ]memory|\bM\d+\s+PASS\b", re.I)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseError(message)


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _json(text):
    def nonfinite(value):
        raise ReleaseError(f"nonfinite JSON number: {value}")
    try:
        return json.loads(text, object_pairs_hook=_pairs, parse_constant=nonfinite)
    except (ValueError, RecursionError) as error:
        raise ReleaseError(f"invalid JSON: {error}") from error


def _keys(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys.split()),
            f"invalid {label} fields")


def _id(value):
    require(isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}", value)
            is not None, "missing or invalid identity")
    return value


def _filename(value):
    require(isinstance(value, str) and value not in ("", ".", "..")
            and len(value.encode("utf-8")) <= 32 and "/" not in value and "\\" not in value
            and all(ord(char) >= 32 and ord(char) != 127 for char in value),
            "missing or invalid UTF-8 filename")
    return value


def _hash(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-f0-9]{64}", value) is not None,
            "invalid SHA-256")
    return value


def _path(root, value):
    require(isinstance(value, str) and value and "\\" not in value and "\x00" not in value
            and not value.startswith("/") and all(p not in ("", ".", "..")
            for p in value.split("/")), "path must be relative without traversal")
    path = root
    for part in value.split("/"):
        path = path / part
        require(not path.is_symlink(), "symlink evidence path")
    require(path.is_file(), f"missing regular file: {value}")
    return path


def _read(path, limit):
    with path.open("rb") as stream:
        content = stream.read(limit + 1)
    require(0 < len(content) <= limit, "empty or oversized evidence")
    return content


def _positive(value):
    return type(value) is int and value > 0


def _expect(data, **fields):
    for key, value in fields.items():
        require(key in data and type(data[key]) is type(value) and data[key] == value,
                f"event data mismatch: {key}")


def _stress(data):
    _expect(data, limits=STRESS_LIMITS)
    _keys(data["limits"], " ".join(STRESS_LIMITS), "stress limits")
    _expect(data["limits"], **STRESS_LIMITS)
    require(_positive(data.get("duration_ms")) and data["duration_ms"] >= 60000,
            "stress requires at least 60 seconds")
    _expect(data, desktop=True, files=True, notes=True, granite_loaded=True,
            audio_playing=True, semantic_search=True)
    require(type(data.get("servo_tabs")) is int and 3 <= data["servo_tabs"] <= 5,
            "stress requires 3-5 Servo tabs")
    require(_positive(data.get("desktop_samples")) and _positive(data.get("audio_frames")),
            "stress lacks positive samples")
    for key, limit in STRESS_LIMITS.items():
        value = data.get(key)
        require(type(value) is int and 0 <= value <= limit, f"invalid stress measurement: {key}")
    require(data["desktop_latency_ms"] > 0 and data["granite_cpu_percent"] > 0,
            "stress lacks positive activity measurements")
    for resource, growth in (("memory_bytes", "memory_growth_bytes"), ("handles", "handle_growth")):
        start, end = data.get(resource + "_start"), data.get(resource + "_end")
        require(_positive(start) and _positive(end), "stress lacks positive resource measurements")
        require(max(0, end - start) == data[growth], "stress resource growth disagrees with measurements")


def _operations(events, browser=True):
    created = events["files.created"]
    object_id = _id(created.get("object_id"))
    _hash(created.get("content_sha256"))
    for name in ("files.renamed", "files.searched"):
        _expect(events[name], object_id=object_id, content_sha256=created["content_sha256"])
    rename = events["files.renamed"]
    require(_filename(rename.get("old_name")) != _filename(rename.get("new_name")), "rename did not change name")
    _expect(events["files.searched"], query=rename["new_name"], found=True)
    require(_id(events["files.deleted"].get("object_id")) != object_id,
            "delete must use a separate object so persisted identity remains testable")
    _expect(events["files.deleted"], absent=True)
    request = _id(events["ai.user_request"].get("request_id"))
    _expect(events["ai.user_request"], user_initiated=True, local=True)
    for name in ("ai.summary", "ai.plan"):
        data = events[name]
        _expect(data, request_id=request, provider="llama.cpp", model="IBM Granite 4.2 3B",
                model_sha256=GRANITE_SHA256, local=True, guest=True)
        require(_positive(data.get("generated_tokens")) and _positive(data.get("inference_ms")),
                "AI lacks real inference measurements")
        _hash(data.get("output_sha256"))
    plan = _id(events["ai.plan"].get("plan_id"))
    _expect(events["ai.plan"], structured=True, object_id=object_id)
    for name in ("plan.validated", "plan.policy_allowed", "plan.user_confirmed",
                 "plan.executed", "plan.undone"):
        _expect(events[name], request_id=request, plan_id=plan, object_id=object_id)
    _expect(events["plan.validated"], valid=True)
    _expect(events["plan.policy_allowed"], allowed=True)
    _expect(events["plan.user_confirmed"], user_initiated=True, confirmed=True)
    transaction = _id(events["plan.executed"].get("transaction_id"))
    _expect(events["plan.executed"], changed=True)
    _expect(events["plan.undone"], transaction_id=transaction, restored=True)
    _hash(events["plan.executed"].get("before_sha256"))
    _hash(events["plan.executed"].get("after_sha256"))
    require(events["plan.executed"]["before_sha256"] != events["plan.executed"]["after_sha256"],
            "plan did not change state")
    _expect(events["plan.undone"], state_sha256=events["plan.executed"]["before_sha256"])
    if not browser:
        return object_id, created["content_sha256"], None, None
    navigation = events["servo.user_navigation"]
    _expect(navigation, engine="Servo", user_initiated=True, rendered=True)
    require(isinstance(navigation.get("url"), str) and navigation["url"].startswith("https://")
            and len(navigation["url"]) > 8, "missing HTTPS navigation")
    history_id = _id(events["servo.history_recorded"].get("history_id"))
    _expect(events["servo.history_recorded"], url=navigation["url"], persisted=True)
    return object_id, created["content_sha256"], history_id, navigation["url"]


def validate_session(root: Path, image: Path, evidence: Path, revision: str) -> None:
    """Raise ReleaseError for missing, inconsistent, or inadmissible evidence."""
    root = Path(root).absolute()
    require(not any(p.is_symlink() for p in (root, *root.parents)), "symlink root")
    # CLI paths may be absolute but must remain within root and have safe components.
    def input_path(path):
        path = Path(path)
        if path.is_absolute():
            try:
                path = path.relative_to(root)
            except ValueError as error:
                raise ReleaseError("input outside root") from error
        return _path(root, str(path))
    image = input_path(image)
    _path(root, str(image.relative_to(root)) + ".build-info")
    provenance = image_build_provenance(root, image, revision)
    require_production_image(provenance)
    manifest = _json(_read(input_path(evidence), MAX_MANIFEST_BYTES).decode("utf-8"))
    _keys(manifest, "schema_version source_revision image_sha256 profile session_id phases", "manifest")
    require(type(manifest["schema_version"]) is int and manifest["schema_version"] == 1,
            "unsupported schema version")
    _expect(manifest, source_revision=revision, image_sha256=provenance["image_sha256"], profile=PROFILE)
    session = _id(manifest["session_id"])
    _keys(manifest["phases"], "boot restart offline", "phases")
    previous = manifest["image_sha256"]
    disk_id = None
    boot_identity = None
    paths = set()
    for phase, sequence in EVENT_SEQUENCES.items():
        record = manifest["phases"][phase]
        _keys(record, "path sha256 disk_id initial_image_sha256 disk_start_sha256 disk_end_sha256 machine network", phase)
        _expect(record, initial_image_sha256=manifest["image_sha256"], machine=MACHINE,
                network=phase != "offline")
        # dict equality alone treats bool as int; enforce exact reference types.
        _keys(record["machine"], "type arch vcpus memory_bytes", "machine")
        _expect(record["machine"], **MACHINE)
        current_disk = _id(record["disk_id"])
        if disk_id is None:
            disk_id = current_disk
        require(current_disk == disk_id, "mixed disk identity")
        require(_hash(record["disk_start_sha256"]) == previous, "broken disk hash chain")
        previous = _hash(record["disk_end_sha256"])
        path = _path(root, record["path"])
        require(path not in paths, "phase log reused")
        paths.add(path)
        content = _read(path, MAX_LOG_BYTES)
        require(hashlib.sha256(content).hexdigest() == _hash(record["sha256"]), "log SHA-256 mismatch")
        log = content.decode("utf-8")
        entered_session = False
        for line in log.splitlines():
            if line.startswith(EVENT_PREFIX):
                entered_session = True
            if not entered_session and line in BOOTSTRAP_MARKERS:
                continue
            if not line.startswith(EVENT_PREFIX):
                require(not BAD_LOG.search(line), "prohibited fixture, milestone, or guest failure marker")
        records = []
        for line in log.splitlines():
            if EVENT_PREFIX in line:
                require(line.startswith(EVENT_PREFIX), "malformed event prefix")
                event = _json(line[len(EVENT_PREFIX):])
                _keys(event, "event session_id phase disk_id image_sha256 data", "event")
                _expect(event, session_id=session, phase=phase, disk_id=disk_id,
                        image_sha256=manifest["image_sha256"])
                require(isinstance(event["data"], dict), "invalid event data")
                records.append(event)
        require([e["event"] for e in records] == list(sequence), f"missing or out-of-order {phase} events")
        events = {e["event"]: e["data"] for e in records}
        _expect(events["session.profile"], profile=PROFILE)
        _expect(events["session.start"], ordinary=True, network=phase != "offline")
        _expect(events["login.unlocked"], user="owner", unlocked=True)
        _expect(events["login.ready"], files=True, search=True, local_ai=True, servo=True)
        if phase in ("boot", "offline"):
            identity = _operations(events, browser=phase == "boot")
            _stress(events["stress.measured"])
            if phase == "boot":
                boot_identity = identity
            else:
                # Offline use restores local browser history; it must not
                # require a fresh external HTTPS connection.
                _, _, history, url = boot_identity
                _expect(events["servo.history_restored"], history_id=history, url=url, found=True)
                _expect(events["offline.completed"], network=False, files=True, search=True,
                        local_ai=True, servo_history=True, voice=True, wayback=True)
        else:
            obj, digest, history, url = boot_identity
            _expect(events["files.restored"], object_id=obj, content_sha256=digest, found=True)
            _expect(events["servo.history_restored"], history_id=history, url=url, found=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        revision = git_source_revision(args.root)
        validate_session(args.root, args.image, args.evidence, revision)
    except (ReleaseError, OSError, UnicodeError) as error:
        print(f"session evidence rejected: {error}", file=sys.stderr)
        return 1
    print("integrated session evidence validated; M30 distribution/legal acceptance NOT_EVALUATED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
