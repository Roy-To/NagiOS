#!/usr/bin/env python3
"""Summarize every rust-lld undefined symbol and find possible archive providers."""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
from pathlib import Path


UNDEFINED_SYMBOL = re.compile(r"rust-lld: error: undefined symbol:\s*(.*?)\s*$")
NM_DEFINED_SYMBOL = re.compile(r"^(?:[0-9A-Fa-f]+\s+)?([A-Za-z?])\s+(.*)$")
LINK_INPUT_SUFFIXES = {".a", ".rlib", ".o"}


def read_symbols(log_path: Path) -> list[str]:
    seen: set[str] = set()
    symbols: list[str] = []
    for line in log_path.read_text(errors="replace").splitlines():
        # `gh run view --log` and the GitHub runner repeat a short annotation
        # payload in their own output. It is not another linker diagnostic.
        if "##[error]" in line or "::error title=" in line:
            continue
        match = UNDEFINED_SYMBOL.search(line)
        if not match:
            continue
        symbol = match.group(1).strip()
        if symbol and symbol not in seen:
            seen.add(symbol)
            symbols.append(symbol)
    return symbols


def find_link_inputs(
    roots: list[Path], top_level_roots: list[Path]
) -> tuple[list[Path], list[str]]:
    candidates: set[Path] = set()
    scanned_roots: list[str] = []
    for root in roots:
        if not root.exists():
            continue
        scanned_roots.append(f"{root} (recursive)")
        for path in root.rglob("*"):
            if path.is_file() and path.suffix in LINK_INPUT_SUFFIXES:
                candidates.add(path)
    for root in top_level_roots:
        if not root.exists():
            continue
        scanned_roots.append(f"{root} (top-level only)")
        for path in root.iterdir():
            if path.is_file() and path.suffix in LINK_INPUT_SUFFIXES:
                candidates.add(path)
    return sorted(candidates), scanned_roots


def find_providers(
    symbols: list[str], roots: list[Path], top_level_roots: list[Path], nm: str
) -> tuple[dict[str, list[str]], list[str], list[Path], list[str]]:
    candidates, scanned_roots = find_link_inputs(roots, top_level_roots)
    wanted = set(symbols)
    providers: dict[str, list[str]] = {symbol: [] for symbol in symbols}
    notes: list[str] = []

    if not symbols:
        return providers, notes, candidates, scanned_roots
    if not candidates:
        notes.append("No .a, .rlib, or .o link inputs were found under the scan roots.")
        return providers, notes, candidates, scanned_roots
    if not shutil.which(nm):
        notes.append(f"{nm} is unavailable; archive/object provider scan was skipped.")
        return providers, notes, candidates, scanned_roots

    for offset in range(0, len(candidates), 32):
        batch = candidates[offset : offset + 32]
        try:
            result = subprocess.run(
                [nm, "--defined-only", "--demangle", "--print-file-name"]
                + [str(path) for path in batch],
                check=False,
                capture_output=True,
                text=True,
                errors="replace",
            )
        except OSError as error:
            notes.append(f"Could not run {nm}: {error}")
            break
        if result.returncode:
            detail = result.stderr.strip().splitlines()
            if detail:
                notes.append(f"{nm} batch starting at {batch[0]}: {detail[0]}")

        for line in result.stdout.splitlines():
            if ": " not in line:
                continue
            input_name, record = line.rsplit(": ", 1)
            match = NM_DEFINED_SYMBOL.match(record)
            if not match:
                continue
            symbol = match.group(2).strip()
            if symbol in wanted and input_name not in providers[symbol]:
                providers[symbol].append(input_name)

    return providers, notes, candidates, scanned_roots


def markdown_code(value: str) -> str:
    return "`" + value.replace("\\", "\\\\").replace("`", "\\`") + "`"


def render_report(
    symbols: list[str],
    providers: dict[str, list[str]],
    notes: list[str],
    input_count: int,
    roots: list[str],
) -> str:
    lines = [
        "## M17 target linker inventory",
        "",
        f"Found **{len(symbols)} unique undefined symbol(s)** in the real Nagi target link log.",
        "",
        "Provider matches are candidates found in the scanned target archives/objects; a match can still indicate an archive-order or extraction issue.",
        "",
        "### Undefined symbols",
        "",
    ]
    if symbols:
        lines.extend(f"- {markdown_code(symbol)}" for symbol in symbols)
    else:
        lines.append("No rust-lld undefined-symbol diagnostics were parsed from the log.")

    lines.extend(["", "### Potential definitions from `llvm-nm`", ""])
    matched = [(symbol, paths) for symbol, paths in providers.items() if paths]
    if matched:
        for symbol, paths in matched:
            preview = ", ".join(markdown_code(path) for path in paths[:6])
            if len(paths) > 6:
                preview += f", and {len(paths) - 6} more"
            lines.append(f"- {markdown_code(symbol)}: {preview}")
    else:
        lines.append("No exact matching definitions were found in the scanned inputs.")

    lines.extend(["", f"Scanned **{input_count}** `.a`, `.rlib`, and `.o` input(s) under:", ""])
    lines.extend(f"- `{root}`" for root in roots)
    if notes:
        lines.extend(["", "### Provider scan notes", ""])
        lines.extend(f"- {note}" for note in notes)
    lines.append("")

    return "\n".join(lines)


def publish_report(report_path: Path, summary_path: Path | None, report: str) -> None:
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(report, encoding="utf-8")
    if summary_path is not None:
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        with summary_path.open("a", encoding="utf-8") as summary:
            summary.write(report)

    # Keep provider evidence in the failed step log as well as the job summary.
    # This makes it retrievable from CI log APIs when a downstream consumer
    # cannot access GitHub's step-summary rendering.
    print(report, end="")


def emit_annotations(symbols: list[str]) -> None:
    if not symbols:
        return

    # Keep each workflow-command message comfortably below annotation limits
    # while preserving the complete inventory across multiple annotations.
    chunks: list[str] = []
    current: list[str] = []
    current_size = 0
    for symbol in symbols:
        extra_size = len(symbol) + (3 if current else 0)
        if current and current_size + extra_size > 12000:
            chunks.append(" | ".join(current))
            current = []
            current_size = 0
            extra_size = len(symbol)
        current.append(symbol)
        current_size += extra_size
    if current:
        chunks.append(" | ".join(current))

    total = len(chunks)
    for index, chunk in enumerate(chunks, start=1):
        escaped = chunk.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=M17 target undefined symbols ({index}/{total})::{escaped}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True, type=Path)
    parser.add_argument("--inventory", required=True, type=Path)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--nm", default="llvm-nm")
    parser.add_argument("--search-root", action="append", default=[], type=Path)
    parser.add_argument("--search-top-level", action="append", default=[], type=Path)
    args = parser.parse_args()

    symbols = read_symbols(args.log)
    args.inventory.parent.mkdir(parents=True, exist_ok=True)
    args.inventory.write_text("".join(f"{symbol}\n" for symbol in symbols))

    providers, notes, candidates, scanned_roots = find_providers(
        symbols, args.search_root, args.search_top_level, args.nm
    )
    report = render_report(
        symbols,
        providers,
        notes,
        len(candidates),
        scanned_roots,
    )
    publish_report(args.inventory.with_suffix(".report.md"), args.summary, report)

    print(f"M17 target linker inventory: {len(symbols)} unique undefined symbol(s)")
    for symbol in symbols:
        print(f"  {symbol}")
    emit_annotations(symbols)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
