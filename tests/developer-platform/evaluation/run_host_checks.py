#!/usr/bin/env python3
"""Repeat the fixed NDP host fixtures; never run the platform inputs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path("tests/developer-platform/evaluation")
ANALYZER = Path("tools/nagi-developer-platform/ui/analysis/analyze.py")
INPUTS = tuple(FIXTURES / name for name in (
    "portable/summary.c", "portable/summary.h", "portable/summary_test.c",
    "windows/read_prefix.c", "linux/watch_changes.c", "controls/text_only.c",
    "ui/task_panel.ui",
)) + (ANALYZER,)


class EvaluationError(Exception):
    def __init__(self, phase, code):
        self.phase = phase
        self.code = code
        super().__init__(phase + ": " + code)


def fingerprint(root):
    return {path.as_posix(): hashlib.sha256((root / path).read_bytes()).hexdigest()
            for path in INPUTS}


def run_command(command, root, phase, timeout=10):
    start = time.perf_counter()
    try:
        result = subprocess.run(
            command, cwd=root, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
            capture_output=True, timeout=timeout, check=False,
        )
    except subprocess.TimeoutExpired:
        raise EvaluationError(phase, "timeout") from None
    except OSError:
        raise EvaluationError(phase, "command_unavailable") from None
    elapsed_ms = (time.perf_counter() - start) * 1000
    if result.returncode != 0:
        # Forward actual diagnostics, but keep the machine-readable error stable.
        if result.stderr:
            print(result.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        raise EvaluationError(phase, "exit_" + str(result.returncode))
    return result.stdout, elapsed_ms


def repeat_command(command, root, phase, repeats):
    first = None
    samples = []
    for _ in range(repeats):
        output, elapsed_ms = run_command(command, root, phase)
        if first is not None and output != first:
            raise EvaluationError(phase, "nondeterministic_stdout")
        first = output
        samples.append(elapsed_ms)
    return first, {
        "samples_ms": samples,
        "median_ms": statistics.median(samples),
        "stdout_identical": True,
    }


def evaluate(root, repeats):
    if not 2 <= repeats <= 20:
        raise EvaluationError("arguments", "repeats_must_be_2_to_20")
    compiler = shutil.which("clang")
    if compiler is None:
        raise EvaluationError("portable_build", "clang_unavailable")
    before = fingerprint(root)
    try:
        version, _ = run_command([compiler, "--version"], root, "compiler_version")
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-host-") as directory:
            binary = str(Path(directory) / "summary-test")
            flags = ["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic"]
            _, build_ms = run_command(
                [compiler] + flags + [str(FIXTURES / "portable/summary.c"),
                                      str(FIXTURES / "portable/summary_test.c"),
                                      "-o", binary], root, "portable_build", timeout=30,
            )
            _, control_ms = run_command(
                [compiler] + flags + ["-fsyntax-only", str(FIXTURES / "controls/text_only.c")],
                root, "control_syntax", timeout=30,
            )
            _, portable = repeat_command([binary], root, "portable_run", repeats)
            output, ui = repeat_command(
                [sys.executable, "-B", str(ANALYZER), "--root",
                 str(FIXTURES / "ui"), "task_panel.ui"], root, "ui_analysis", repeats,
            )
            try:
                facts = json.loads(output)
                counts = {key: len(facts[key]) for key in (
                    "widgets", "layouts", "tab_order", "connections", "unknown",
                    "unresolved_references",
                )}
                if any(item["execution"] != "not_run" for item in facts["connections"]):
                    raise EvaluationError("ui_analysis", "unexpected_execution_claim")
            except (ValueError, KeyError, TypeError):
                raise EvaluationError("ui_analysis", "invalid_report") from None
            ui["facts"] = counts
    finally:
        # Check even when a build, run or comparison fails. Never restore a changed input.
        if fingerprint(root) != before:
            raise EvaluationError("input_integrity", "source_changed")
    return {
        "status": "PASS", "scope": "synthetic_host_fixtures_only", "repeats": repeats,
        "environment": {"os": platform.system(), "machine": platform.machine(),
                        "python": platform.python_version(),
                        "compiler": version.decode("utf-8").splitlines()[0]},
        "portable_build_ms": build_ms, "control_syntax_ms": control_ms,
        "portable_run": portable, "ui_analysis": ui,
        "timing_scope": "subprocess wall time including startup; not human work or speedup",
        "inputs_unchanged": True,
        "not_run": ["windows_input", "linux_input", "manual_comparison", "llm_comparison",
                    "sdk_validator", "ui_rendering", "nagi_target"],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=3, help="2 to 20 runs (default: 3)")
    args = parser.parse_args(argv)
    try:
        result = evaluate(ROOT, args.repeats)
    except EvaluationError as error:
        print(json.dumps({"status": "FAIL", "phase": error.phase, "error": error.code}),
              file=sys.stderr)
        return 1
    except OSError:
        print(json.dumps({"status": "FAIL", "error": "source_unavailable"}), file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
