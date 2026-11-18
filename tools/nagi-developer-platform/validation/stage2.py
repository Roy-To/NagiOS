#!/usr/bin/env python3
"""NDP Stage 2: host build/test of approved self-made C fixtures with a fixed clang argument vector.

Only targets listed in stages.json with approved source and header bytes run. Nothing in
the input (fixture directory contents, CLI arguments, Manifests) selects a command, flag,
source or output name. Stage 3 is not connected and never returns PASS.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile

import validate

REPO = validate.REPO
CONTRACT = Path(__file__).with_name("stages.json")
FIXTURE_PREFIX = ("tests", "developer-platform", "evaluation")
# Exact blob bytes from the approved NDP-10 fixture commit 715d3089bb1b268d0bab32b9e55128a283144ba0.
# Approval lives in the trusted adapter, outside the evaluated inputs and stages.json.
_APPROVED_FIXTURE_SHA256 = {
    ("portable-summary", (*FIXTURE_PREFIX, "portable")): {
        "summary.c": "1cd1d6c82eb7fa6a2ff1ad393341ce06ebf39655b0bfc4010959444a59829e8e",
        "summary_test.c": "e69ecfa868a601be98242e3fc24f2e4e9c57509fa59a254e91ec21d44211c996",
        "summary.h": "01033e490e36c46fed0985494372c24208390851cd09b59280c7227b51319f33",
    },
}
# Closed set of clang flags a contract entry may select. No -I/-include/-Xclang/@file/plugins.
ALLOWED_FLAGS = frozenset({"-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic", "-O0", "-O2",
                           "-fno-color-diagnostics"})
ENV_ALLOWLIST = ("PATH", "HOME", "DEVELOPER_DIR", "SDKROOT")
NAME = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]*\Z")
DEFAULT_COMPILE_TIMEOUT = 30
DEFAULT_TEST_TIMEOUT = 10
# Redacted, bounded diagnostics summary (PROVISIONAL shape; see README). Limits are data.
MAX_DIAGNOSTIC_ITEMS = 16
MAX_DIAGNOSTIC_CHARS = 1024
CATEGORY = re.compile(r"-W[a-z0-9+-]{1,40}\Z")
BASE_RESULT = {"stage": 2, "check": "portable_build_test", "scope": "host_build_test_fixture_only",
               "status": "not_run", "reason": "not_run", "steps": []}


class ContractError(Exception):
    pass


class Target:
    """One approved fixture build. Replaceable data; see stages.json."""

    def __init__(self, entry):
        try:
            self.id = entry["id"]
            self.directory = tuple(entry["directory"].split("/"))
            self.sources = tuple(entry["sources"])
            self.headers = tuple(entry.get("headers", ()))
            self.flags = tuple(entry["flags"])
            self.output = entry["output"]
            self.expect_prefix = entry["expect_stdout_prefix"].encode()
        except (KeyError, AttributeError, TypeError):
            raise ContractError("malformed_target") from None
        names = (self.id, self.output, *self.sources, *self.headers)
        if not all(isinstance(n, str) and NAME.match(n) for n in names) or not self.sources:
            raise ContractError("unsafe_name")
        if len(set((*self.sources, *self.headers, self.output))) != len(self.sources) + len(self.headers) + 1:
            raise ContractError("duplicate_name")
        if not set(self.flags) <= ALLOWED_FLAGS:
            raise ContractError("flag_not_allowed")
        if self.directory[:3] != FIXTURE_PREFIX or len(self.directory) < 4 or \
                not all(NAME.match(part) for part in self.directory):
            raise ContractError("directory_not_allowed")
        if not self.expect_prefix:
            raise ContractError("missing_expectation")

    def compile_command(self, clang):
        # The only place an argument vector is built.
        return [clang, *self.flags, "-o", self.output, *self.sources]


def load_targets(path=CONTRACT):
    try:
        document = json.loads(Path(path).read_bytes())
        targets = [Target(entry) for entry in document["stage2_targets"]]
    except (OSError, ValueError, KeyError, TypeError):
        raise ContractError("contract_unreadable") from None
    return {target.id: target for target in targets}


def _result(status, reason, **extra):
    return {**BASE_RESULT, "steps": [], "status": status, "reason": reason, **extra}


def _step(name, execution, ok_reason, fail_reason):
    status = execution["status"]
    exit_code = execution["exit_code"]
    if status == "exited":
        status, reason = ("passed", ok_reason) if exit_code == 0 else ("failed", fail_reason)
    else:
        reason = status
    return {"name": name, "status": status, "reason": reason, "exit_code": exit_code,
            "captured_output_bytes": len(execution["stdout"]) + len(execution["stderr"])}


def _category(rest):
    """The warning-flag name from a trailing `[-Wflag]` / `[-Werror,-Wflag]`, or None."""
    bracket = re.search(rb"\[([-A-Za-z0-9+,]{1,80})\]\Z", rest)
    for flag in (bracket[1].decode().split(",") if bracket else ()):
        if flag != "-Werror" and CATEGORY.match(flag):
            return flag
    return None


def summarize_diagnostics(output, names, scratch_prefixes=(), max_items=MAX_DIAGNOSTIC_ITEMS,
                          max_chars=MAX_DIAGNOSTIC_CHARS):
    """Reduce raw compiler output to counts plus (file, line, severity, category) items.

    Only `file:line:col: error|warning:` lines naming one of `names` (fixture-relative; a
    leading scratch directory is stripped) are recognised. Message text, source snippets,
    other paths and everything else are dropped, never copied. `items` is bounded by item
    count and by the serialised size; `truncated` says whether anything was left out.
    """
    alternation = "|".join(re.escape(name) for name in sorted(names, key=len, reverse=True))
    line_pattern = re.compile(rb"(?P<file>" + alternation.encode() + rb"):(?P<line>[0-9]{1,9}):[0-9]{1,9}: "
                              rb"(?P<kind>fatal error|error|warning):(?P<rest>.*)\Z")
    prefixes = tuple(sorted({p.rstrip("/") + "/" for p in scratch_prefixes if p}, key=len, reverse=True))
    summary = {"errors": 0, "warnings": 0, "items": [], "truncated": False}
    used = 0
    for raw in output.split(b"\n"):
        for prefix in prefixes:
            if raw.startswith(prefix.encode()):
                raw = raw[len(prefix):]
                break
        while raw.startswith(b"./"):
            raw = raw[2:]
        match = line_pattern.match(raw.rstrip(b"\r"))
        if match is None:
            continue
        severity = "warning" if match["kind"] == b"warning" else "error"
        summary["errors" if severity == "error" else "warnings"] += 1
        category = _category(match["rest"])
        item = {"file": match["file"].decode(), "line": int(match["line"]), "severity": severity,
                "category": category}
        size = len(json.dumps(item, sort_keys=True))
        if len(summary["items"]) >= max_items or used + size > max_chars:
            summary["truncated"] = True
            continue
        summary["items"].append(item)
        used += size
    return summary


def _gate(prior):
    if prior is None:
        return True
    items = [prior] if isinstance(prior, dict) else prior
    return isinstance(items, (list, tuple)) and bool(items) and all(
        isinstance(item, dict) and item.get("stage") == 1 and item.get("status") == "passed"
        for item in items)


def _real(path):
    return Path(os.path.realpath(path))


def run_stage2(target_id, fixture_dir=None, cwd=None, prior=None, compile_timeout=DEFAULT_COMPILE_TIMEOUT,
               test_timeout=DEFAULT_TEST_TIMEOUT, cancel=None, targets=None, base=REPO, clang=None):
    """Build and run one approved target. Every rejection happens before any process starts.

    `prior` is a Stage 1 result (or list of them) the caller already obtained; anything
    other than passed stops Stage 2. `base` is the checkout root the target directory is
    resolved against (tests use a temporary copy). `clang` overrides PATH lookup.
    """
    for timeout in (compile_timeout, test_timeout):
        if not isinstance(timeout, (int, float)) or not 0 < timeout <= 60:
            return _result("not_run", "invalid_timeout")
    if cancel is not None and cancel.is_set():
        return _result("cancelled", "cancelled")
    if not _gate(prior):
        return _result("not_run", "prior_stage_not_passed")
    try:
        table = load_targets() if targets is None else targets
    except ContractError as error:
        return _result("not_run", str(error))
    target = table.get(target_id) if isinstance(target_id, str) else None
    if target is None:
        return _result("not_run", "unknown_target")
    known = {"target": target.id}
    base_real = _real(base)
    try:
        cwd_real = _real(Path.cwd() if cwd is None else cwd)
    except OSError:
        return _result("not_run", "cwd_not_allowed", **known)
    if cwd_real != base_real and base_real not in cwd_real.parents:
        return _result("not_run", "cwd_not_allowed", **known)
    allowed = base_real.joinpath(*target.directory)
    # The approved directory must be a real directory, with no symlink on its path.
    if _real(allowed) != allowed or not allowed.is_dir():
        return _result("not_run", "input_not_allowed", **known)
    if fixture_dir is not None:
        try:
            requested = _real(cwd_real / fixture_dir)
        except (OSError, ValueError, TypeError):
            return _result("not_run", "input_not_allowed", **known)
        if requested != allowed:
            return _result("not_run", "input_not_allowed", **known)
    names = (*target.sources, *target.headers)
    approved = _APPROVED_FIXTURE_SHA256.get((target.id, target.directory))
    if approved is None or approved.keys() != set(names):
        return _result("not_run", "fixture_digest_mismatch", **known)
    try:
        # Only the listed files are read; other files in the directory are never touched.
        directory = Path(*target.directory)
        contents = {name: validate.read_manifest(base_real, directory / name) for name in names}
    except validate.InputError as error:
        return _result("not_run", str(error), **known)
    # Hash the same bytes that are staged below, without reopening any input file.
    digests = {name: hashlib.sha256(data).hexdigest() for name, data in contents.items()}
    if digests != approved:
        return _result("not_run", "fixture_digest_mismatch", **known)
    clang_path = clang if clang is not None else shutil.which("clang")
    if clang_path is None or not os.access(clang_path, os.X_OK):
        return _result("unavailable", "clang_unavailable", **known)
    steps = []
    diagnostics = None
    try:
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-stage2-") as scratch:
            for name, data in contents.items():
                (Path(scratch) / name).write_bytes(data)
            env = {key: os.environ[key] for key in ENV_ALLOWLIST if key in os.environ}
            env.update(LC_ALL="C", TMPDIR=scratch)
            built = validate._run(target.compile_command(str(clang_path)), scratch, compile_timeout,
                                  cancel, env=env)
            if built["status"] == "unavailable":
                return _result("unavailable", "clang_unavailable", **known)
            steps.append(_step("compile", built, "compiled", "compile_failed"))
            diagnostics = summarize_diagnostics(
                built["stdout"] + b"\n" + built["stderr"], names,
                (scratch, str(_real(scratch))))
            binary = Path(scratch) / target.output
            if steps[-1]["status"] == "passed" and (
                    binary.is_symlink() or not binary.is_file() or not os.access(binary, os.X_OK)):
                steps[-1].update(status="failed", reason="binary_missing")
            if steps[-1]["status"] != "passed":
                steps.append({"name": "run_test", "status": "not_run", "reason": "compile_did_not_succeed",
                              "exit_code": None, "captured_output_bytes": 0})
            else:
                ran = validate._run([str(binary)], scratch, test_timeout, cancel,
                                    env={"LC_ALL": "C", "TMPDIR": scratch})
                step = _step("run_test", ran, "test_passed", "test_failed")
                if step["status"] == "passed" and not ran["stdout"].startswith(target.expect_prefix):
                    step.update(status="failed", reason="invalid_tool_output")
                steps.append(step)
    except OSError:
        return _result("unavailable", "scratch_unavailable", **known)
    failing = next((step for step in steps if step["status"] != "passed"), None)
    # Additive field: present whenever the compiler ran, never on rejection results.
    known = {**known, "diagnostics": diagnostics}
    if failing is None:
        return _result("passed", "build_and_test_passed", steps=steps, **known)
    return _result(failing["status"], failing["reason"], steps=steps, **known)


def run_stage3(*_args, **_kwargs):
    """Stage 3 (Nagi target acceptance) is not connected: nothing is read or run, never PASS."""
    return {"stage": 3, "check": "nagi_target", "scope": "nagi_target", "status": "not_run",
            "reason": "stage_not_connected", "steps": []}


EXIT_CODES = {"passed": 0, "unavailable": 2, "not_run": 3, "timeout": 124, "cancelled": 130}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", type=int, choices=(2, 3), default=2)
    parser.add_argument("--target", default="portable-summary", help="Approved target id")
    parser.add_argument("--fixture-dir", help="Must equal the approved directory of the target")
    parser.add_argument("--stage1-root", help="Run Stage 1 first with this directory")
    parser.add_argument("--stage1-manifest", help="Relative Manifest path for the Stage 1 gate")
    parser.add_argument("--compile-timeout", type=float, default=DEFAULT_COMPILE_TIMEOUT)
    parser.add_argument("--test-timeout", type=float, default=DEFAULT_TEST_TIMEOUT)
    args = parser.parse_args(argv)
    try:
        if args.stage == 3:
            result = run_stage3()
        else:
            prior = None
            if args.stage1_manifest is not None:
                first = validate.validate(args.stage1_root or ".", args.stage1_manifest)
                prior = {"stage": 1, "status": first["status"], "reason": first["reason"]}
            result = run_stage2(args.target, args.fixture_dir, prior=prior,
                                compile_timeout=args.compile_timeout, test_timeout=args.test_timeout)
            if prior is not None:
                result["prior"] = [prior]
    except KeyboardInterrupt:
        result = _result("cancelled", "cancelled")
    print(json.dumps(result, sort_keys=True))
    return EXIT_CODES.get(result["status"], 1)


if __name__ == "__main__":
    sys.exit(main())
