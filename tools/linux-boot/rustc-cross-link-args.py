#!/usr/bin/env python3
"""Keep x86_64-musl linker switches off Cargo's macOS host build scripts."""

from __future__ import annotations

import os
import sys


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("rustc executable argument is missing")
    compiler, *arguments = sys.argv[1:]
    crate_name = None
    crate_type = None
    for index, argument in enumerate(arguments):
        if argument == "--crate-name" and index + 1 < len(arguments):
            crate_name = arguments[index + 1]
        elif argument == "--crate-type" and index + 1 < len(arguments):
            crate_type = arguments[index + 1]
        elif argument.startswith("--crate-type="):
            crate_type = argument.partition("=")[2]
    host_build_tool = crate_name == "build_script_build" or crate_type == "proc-macro"
    target_build = not host_build_tool and any(
        argument == "--target=x86_64-unknown-linux-musl"
        or argument == "x86_64-unknown-linux-musl"
        for argument in arguments
    )
    if not target_build:
        filtered: list[str] = []
        index = 0
        while index < len(arguments):
            argument = arguments[index]
            if (
                argument == "-C"
                and index + 1 < len(arguments)
                and arguments[index + 1]
                in (
                    "link-arg=--target=x86_64-unknown-linux-musl",
                    "link-arg=-fuse-ld=lld",
                )
            ):
                index += 2
                continue
            if argument in (
                "-Clink-arg=--target=x86_64-unknown-linux-musl",
                "-Clink-arg=-fuse-ld=lld",
            ):
                index += 1
                continue
            filtered.append(argument)
            index += 1
        arguments = filtered
    os.execv(compiler, [compiler, *arguments])


if __name__ == "__main__":
    main()
