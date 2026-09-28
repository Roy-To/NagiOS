#!/usr/bin/env python3
"""Verify the M17 target feature graph excludes M18-only browser features."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
M18_ONLY_FEATURES = (
    re.compile(r'^nagi-init feature "m18-[^"]+"$'),
    re.compile(r'^nagi-albert feature "m18-[^"]+"$'),
    re.compile(r'^nagi-posix feature "browser-storage"$'),
)


def evaluate_m17_feature_graph(graph: str) -> dict:
    """Return evidence only when the requested M17 feature graph is present."""
    if not isinstance(graph, str) or not graph.strip():
        status = "FAIL"
        proof = "the M17 Cargo feature graph was empty"
    else:
        lines = {line.strip() for line in graph.splitlines()}
        missing = []
        if not any(line.startswith("nagi-init v") for line in lines):
            missing.append("nagi-init package")
        if not any(line.startswith("nagi-albert v") for line in lines):
            missing.append("nagi-albert package")
        enabled = sorted(
            line
            for line in lines
            if any(pattern.fullmatch(line) for pattern in M18_ONLY_FEATURES)
        )
        if missing:
            status = "FAIL"
            proof = "M17 graph did not include expected nodes: " + ", ".join(missing)
        elif enabled:
            status = "FAIL"
            proof = "M17 graph enabled M18-only features: " + ", ".join(enabled)
        else:
            status = "PASS"
            proof = (
                "Cargo resolved nagi-init with --features m17-servo for the x86-64 Nagi target; "
                "the graph includes nagi-albert and no m18-* or nagi-posix/browser-storage feature."
            )
    return {
        "schema_version": 1,
        "cases": [{
            "id": "features.m17-excludes-m18",
            "status": status,
            "evidence": proof,
        }],
    }


def blocked_feature_graph_evidence(reason: str) -> dict:
    """Represent an unavailable graph as blocked evidence, never as a pass."""
    return {
        "schema_version": 1,
        "cases": [{
            "id": "features.m17-excludes-m18",
            "status": "BLOCKED",
            "evidence": f"The M17 target feature graph could not be inspected: {reason}",
        }],
    }


def _cargo_feature_graph() -> str:
    command = [
        "rustup", "run", "nightly-2025-08-01", "cargo", "tree",
        "--locked", "--package", "nagi-init",
        "--features", "m17-servo",
        "--target", "targets/x86_64-unknown-nagi-user.json",
        "--edges", "features", "--prefix", "none",
    ]
    result = subprocess.run(command, cwd=ROOT, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        detail = result.stderr.strip() or f"cargo tree exited with {result.returncode}"
        raise RuntimeError(detail)
    return result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, help="use a saved Cargo feature graph")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    try:
        graph = arguments.graph.read_text(encoding="utf-8") if arguments.graph else _cargo_feature_graph()
        evidence = evaluate_m17_feature_graph(graph)
    except (OSError, RuntimeError) as error:
        evidence = blocked_feature_graph_evidence(str(error))
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(
            json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(f"[BLOCKED] features.m17-excludes-m18: {evidence['cases'][0]['evidence']}")
        return 2
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(
        json.dumps(evidence, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    result = evidence["cases"][0]
    print(f"[{result['status']}] {result['id']}: {result['evidence']}")
    return 1 if result["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
