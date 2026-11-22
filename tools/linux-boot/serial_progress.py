#!/usr/bin/env python3
"""Summarize how far a Task24 QEMU serial log got along the goal-1 chain.

Reads one or more serial logs and prints a short report: which stage markers
appeared (in order), the first missing stage, known failure markers, and the
last few lines before the log ended. The point is to avoid pasting a whole
serial log into an AI chat: paste this report instead.

This is a triage aid, not acceptance. Formal checks stay in
tools/linux-boot/task24_recovery_serials.py and the runner receipts.

Usage:
  serial_progress.py save.log [read.log ...] [--tail N] [--json]
Exit status: 0 = READY and save (or reopen) reached, 1 = stopped earlier, 2 = usage/IO error.
"""
import argparse
import json
import re
import sys

# Marker strings mirror tools/linux-boot/task24_recovery_serials.py (2026-11-22).
STAGES = [
    ("data_mounted", "NAGI_DEV_IDENTITY_DATA_MOUNTED"),
    ("login_waiting", "NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT"),
    ("session_attached", "NAGI_DEV_TASK24_SESSION_ATTACHED"),
    ("identity_handshake", "NAGI_DEV_TASK24_IDENTITY_HANDSHAKE_COMPLETED"),
    ("ready", "nagi-init: state=ready"),
    ("notes_input_ready", "NAGI_DEV_NOTES_INPUT_READY"),
    ("notes_saved", "NAGI_DEV_NOTES_SAVED"),
    ("notes_reopened", "NAGI_DEV_NOTES_REOPENED"),
    ("notes_closed", "NAGI_DEV_NOTES_CLOSED"),
    ("child_reaped", "NAGI_DEV_TASK24_CHILD_REAPED_AND_RETIRED"),
]
FAILURES = [
    "NAGI_C1_BOOT_RECOVERY_UNAVAILABLE",
    "C1_BOOT_RECOVERY_UNAVAILABLE",
    "NAGI_DEV_TASK24_LOGIN_UNAVAILABLE",
    "NAGI_DEV_TASK24_LOGIN_STOPPED",
    "NAGI_DEV_TASK24_IDENTITY_TERMINAL_UNAVAILABLE",
    "NAGI_DEV_TASK24_REQUIRED_IDENTITY_MONITOR_UNAVAILABLE",
    "NAGI_DEV_TASK24_CLEANUP_PENDING",
]
# Generic trouble patterns worth surfacing (kernel panics, Rust panics, typed errors).
TROUBLE = re.compile(
    r"(Kernel panic|panicked at|thread '.*' panicked|\bFAIL\b|StoreUnavailable|"
    r"Unavailable\b|Quarantin|timed? ?out|Segmentation fault|Oops:|BUG:)",
    re.IGNORECASE,
)
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def analyze(text, tail):
    lines = [ANSI.sub("", l).rstrip("\r") for l in text.splitlines()]
    seen = {}
    for i, line in enumerate(lines):
        for key, marker in STAGES:
            if key not in seen and marker in line:
                seen[key] = (i + 1, line.strip()[:200])
    order_ok = True
    last_line = -1
    for key, _ in STAGES:
        if key in seen:
            if seen[key][0] < last_line:
                order_ok = False
            last_line = seen[key][0]
    first_missing = next((k for k, _ in STAGES if k not in seen), None)
    failures = []
    for i, line in enumerate(lines):
        if any(f in line for f in FAILURES) or TROUBLE.search(line):
            failures.append((i + 1, line.strip()[:240]))
    # Collapse repeated identical trouble lines.
    uniq, seen_txt = [], set()
    for n, t in failures:
        if t not in seen_txt:
            seen_txt.add(t)
            uniq.append((n, t))
    return {
        "lines": len(lines),
        "stages": {k: seen.get(k) for k, _ in STAGES},
        "order_ok": order_ok,
        "first_missing": first_missing,
        "trouble": uniq[:15],
        "trouble_total": len(failures),
        "tail": [l[:240] for l in lines[-tail:]] if tail else [],
    }


def render(path, r):
    out = [f"== {path} ({r['lines']} lines)"]
    for key, _ in STAGES:
        hit = r["stages"][key]
        out.append(f"  [{'x' if hit else ' '}] {key:20s} " + (f"L{hit[0]}" if hit else ""))
    out.append(f"  order: {'OK' if r['order_ok'] else 'OUT OF ORDER'}")
    out.append(f"  stopped before: {r['first_missing'] or '(all stages seen)'}")
    if r["trouble"]:
        out.append(f"  trouble lines ({r['trouble_total']} total, first unique shown):")
        out += [f"    L{n}: {t}" for n, t in r["trouble"]]
    if r["tail"]:
        out.append("  last lines:")
        out += [f"    | {t}" for t in r["tail"]]
    return "\n".join(out)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("logs", nargs="+")
    p.add_argument("--tail", type=int, default=8)
    p.add_argument("--json", action="store_true")
    a = p.parse_args(argv)
    results, reached = {}, False
    for path in a.logs:
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                results[path] = analyze(f.read(), a.tail)
        except OSError as e:
            print(f"cannot read {path}: {e}", file=sys.stderr)
            return 2
        st = results[path]["stages"]
        if st["ready"] and (st["notes_saved"] or st["notes_reopened"]):
            reached = True
    if a.json:
        print(json.dumps(results, ensure_ascii=False, indent=1))
    else:
        print("\n".join(render(p_, r) for p_, r in results.items()))
    return 0 if reached else 1


if __name__ == "__main__":
    sys.exit(main())
