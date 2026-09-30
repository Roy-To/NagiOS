#!/usr/bin/env python3
"""Check that every locked external Cargo package declares a license expression."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    repository = Path(__file__).resolve().parents[1]
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            cwd=repository,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        print(f"FAIL cannot run cargo metadata: {error}", file=sys.stderr)
        return 2

    if result.returncode != 0:
        print(result.stderr, file=sys.stderr, end="")
        return result.returncode

    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        print(f"FAIL cargo metadata returned invalid JSON: {error}", file=sys.stderr)
        return 2

    external = [package for package in metadata.get("packages", []) if package.get("source")]
    missing = [package for package in external if not package.get("license")]
    if missing:
        for package in missing:
            print(
                f"MISSING license expression: {package['name']} {package['version']} ({package['source']})",
                file=sys.stderr,
            )
        print(f"FAIL {len(missing)} of {len(external)} external packages lack metadata", file=sys.stderr)
        return 1

    expressions = {package["license"] for package in external}
    print(
        f"PASS Cargo metadata: {len(external)} external locked packages declare "
        f"license expressions ({len(expressions)} distinct expressions)"
    )
    print("The graph includes dev and target-specific packages; it is not an image bill of materials.")
    print("This checks package metadata only; it does not review license texts or redistribution obligations.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
