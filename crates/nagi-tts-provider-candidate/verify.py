#!/usr/bin/env python3
"""Offline, model-free host checks with the repository-pinned installed tools."""
import argparse
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("step", nargs="?", choices=("all", "fmt", "test", "clippy"), default="all")
    args = parser.parse_args()
    crate = Path(__file__).resolve().parent
    root = crate.parents[1]
    # rustup which only resolves an installed toolchain; it does not install one.
    pinned = "nightly-2025-08-01"
    tools = {}
    for name in ("cargo", "rustc", "rustdoc", "cargo-clippy", "clippy-driver"):
        result = subprocess.run(["rustup", "which", "--toolchain", pinned, name], check=True, capture_output=True, text=True)
        tools[name] = Path(result.stdout.strip())
    if len({path.parent for path in tools.values()}) != 1:
        raise RuntimeError("Pinned tools must resolve to one installed toolchain")
    env = os.environ.copy()
    env.update(PATH=str(tools["cargo"].parent) + os.pathsep + env.get("PATH", ""), RUSTC=str(tools["rustc"]), RUSTDOC=str(tools["rustdoc"]), CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="1", CARGO_TARGET_DIR=str(crate / "target"))
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
        env.pop(key, None)
    manifest = str(crate / "Cargo.toml")
    base = [str(tools["cargo"])]
    commands = {
        "fmt": base + ["fmt", "--manifest-path", manifest, "--", "--check"],
        "test": base + ["test", "--manifest-path", manifest, "--offline", "--locked", "--jobs", "1", "--", "--test-threads=1"],
        "clippy": base + ["clippy", "--manifest-path", manifest, "--all-targets", "--offline", "--locked", "--jobs", "1", "--", "-D", "warnings"],
    }
    selected = commands if args.step == "all" else {args.step: commands[args.step]}
    for step, argv in selected.items():
        print("Running " + step, flush=True)
        result = subprocess.run(argv, cwd=root, env=env, check=False)
        print(step + " RC " + str(result.returncode), flush=True)
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
