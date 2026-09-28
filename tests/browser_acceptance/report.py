#!/usr/bin/env python3
"""Create a truthful, case-level M18-C browser acceptance report."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = Path(__file__).resolve().with_name("cases.json")
WORKSTREAM_ID = "m18c-browser-acceptance-reliability"
BASE_SHA = "94e9a027618182b10c0ac2315e94673543f22423"
CASE_STATUSES = ("PASS", "FAIL", "SKIP-BLOCKED", "NOT-IMPLEMENTED")


class ReportError(ValueError):
    """Raised when a manifest or evidence bundle is ambiguous or malformed."""


def load_manifest(path: Path = MANIFEST_PATH) -> dict:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReportError(f"could not read case manifest {path}: {error}") from error
    validate_manifest(manifest)
    return manifest


def validate_manifest(manifest: dict) -> None:
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ReportError("case manifest schema_version must be 1")
    if manifest.get("workstream_id") != WORKSTREAM_ID:
        raise ReportError(f"case manifest workstream_id must be {WORKSTREAM_ID}")
    cases = manifest.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ReportError("case manifest must contain a non-empty cases array")
    identifiers = set()
    for index, case in enumerate(cases):
        if not isinstance(case, dict):
            raise ReportError(f"case manifest cases[{index}] must be an object")
        for name in ("id", "area", "requirement"):
            if not isinstance(case.get(name), str) or not case[name].strip():
                raise ReportError(f"case manifest cases[{index}].{name} must be non-empty")
        identifier = case["id"]
        if identifier in identifiers:
            raise ReportError(f"duplicate case id {identifier!r}")
        identifiers.add(identifier)
        owners = case.get("owner_workstreams")
        if not isinstance(owners, list) or not owners or any(
            not isinstance(owner, str) or not owner.strip() for owner in owners
        ):
            raise ReportError(f"case {identifier!r} must name an owning workstream")
        blocker = case.get("blocker")
        if blocker is not None and (not isinstance(blocker, str) or not blocker.strip()):
            raise ReportError(f"case {identifier!r} blocker must be non-empty when present")


def validate_evidence(manifest: dict, evidence: dict | None) -> dict:
    if evidence is None:
        return {}
    if not isinstance(evidence, dict) or evidence.get("schema_version") != 1:
        raise ReportError("evidence schema_version must be 1")
    evidence_cases = evidence.get("cases")
    if not isinstance(evidence_cases, list):
        raise ReportError("evidence must contain a cases array")
    known_ids = {case["id"] for case in manifest["cases"]}
    by_id = {}
    for index, item in enumerate(evidence_cases):
        if not isinstance(item, dict):
            raise ReportError(f"evidence cases[{index}] must be an object")
        identifier = item.get("id")
        if identifier not in known_ids:
            raise ReportError(f"evidence references unknown case {identifier!r}")
        if identifier in by_id:
            raise ReportError(f"duplicate evidence for case {identifier!r}")
        status = item.get("status")
        if status not in ("PASS", "FAIL", "BLOCKED"):
            raise ReportError(f"evidence for {identifier!r} must be PASS, FAIL, or BLOCKED")
        proof = item.get("evidence")
        if not isinstance(proof, str) or not proof.strip():
            raise ReportError(f"evidence for {identifier!r} must include non-empty proof")
        by_id[identifier] = {"status": status, "evidence": proof.strip()}
    return by_id


def build_report(
    manifest: dict,
    evidence: dict | None = None,
    *,
    branch: str = "",
    head_sha: str = "",
    generated_at: str | None = None,
) -> dict:
    validate_manifest(manifest)
    evidence_by_id = validate_evidence(manifest, evidence)
    cases = []
    for item in manifest["cases"]:
        observation = evidence_by_id.get(item["id"])
        if observation is None:
            blocker = item.get("blocker")
            status = "SKIP-BLOCKED" if blocker else "NOT-IMPLEMENTED"
            proof = ""
            reason = blocker or "No browser acceptance evidence was supplied."
        elif observation["status"] == "BLOCKED":
            status = "SKIP-BLOCKED"
            proof = ""
            reason = observation["evidence"]
        else:
            status = observation["status"]
            proof = observation["evidence"]
            reason = ""
        cases.append({
            "id": item["id"],
            "area": item["area"],
            "requirement": item["requirement"],
            "status": status,
            "owner_workstreams": item["owner_workstreams"],
            "reason": reason,
            "evidence": proof,
        })

    summary = {status: 0 for status in CASE_STATUSES}
    for case in cases:
        summary[case["status"]] += 1
    summary["total"] = len(cases)
    if summary["FAIL"]:
        status = "FAIL"
    elif summary["PASS"] == summary["total"]:
        status = "PASS"
    else:
        status = "PARTIAL"
    if not generated_at:
        generated_at = datetime.now(timezone.utc).isoformat(timespec="seconds").replace(
            "+00:00", "Z"
        )
    return {
        "schema_version": 1,
        "workstream_id": manifest["workstream_id"],
        "branch": branch,
        "base_sha": manifest.get("base_sha", BASE_SHA),
        "head_sha": head_sha,
        "generated_at": generated_at,
        "status": status,
        "summary": summary,
        "cases": cases,
    }


def _git_value(*arguments: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, default=MANIFEST_PATH)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--branch", default="")
    parser.add_argument("--head-sha", default="")
    parser.add_argument("--github-summary", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        manifest = load_manifest(arguments.cases)
        evidence = None
        if arguments.evidence is not None:
            try:
                evidence = json.loads(arguments.evidence.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise ReportError(f"could not read evidence {arguments.evidence}: {error}") from error
        report = build_report(
            manifest,
            evidence,
            branch=arguments.branch or _git_value("branch", "--show-current"),
            head_sha=arguments.head_sha or _git_value("rev-parse", "HEAD"),
        )
        if arguments.github_summary:
            _append_github_summary(report)
    except ReportError as error:
        print(f"M18-C acceptance report FAIL: {error}", file=sys.stderr)
        return 2

    encoded = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if arguments.output is not None:
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(encoded, encoding="utf-8")
    for case in report["cases"]:
        owner = ", ".join(case["owner_workstreams"])
        detail = case["evidence"] or case["reason"]
        print(f"[{case['status']}] {case['id']} owner={owner}: {detail}")
    print(
        "M18-C browser acceptance "
        f"{report['status']}: {report['summary']['PASS']} PASS, "
        f"{report['summary']['FAIL']} FAIL, "
        f"{report['summary']['SKIP-BLOCKED']} SKIP-BLOCKED, "
        f"{report['summary']['NOT-IMPLEMENTED']} NOT-IMPLEMENTED"
    )
    if arguments.output is not None:
        print(f"M18-C JSON report: {arguments.output}")
    return 1 if report["status"] == "FAIL" else 0


def _append_github_summary(report: dict) -> None:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        raise ReportError("--github-summary requires GITHUB_STEP_SUMMARY")
    lines = [
        "## M18-C Browser Acceptance",
        "",
        f"**Status: {report['status']}** — {report['summary']['PASS']} PASS, "
        f"{report['summary']['FAIL']} FAIL, "
        f"{report['summary']['SKIP-BLOCKED']} SKIP-BLOCKED, "
        f"{report['summary']['NOT-IMPLEMENTED']} NOT-IMPLEMENTED.",
        "",
        "| Case | Status | Owner | Evidence / blocker |",
        "| --- | --- | --- | --- |",
    ]
    for case in report["cases"]:
        detail = case["evidence"] or case["reason"]
        fields = (case["id"], case["status"], ", ".join(case["owner_workstreams"]), detail)
        safe = [value.replace("|", "\\|").replace("\n", " ") for value in fields]
        lines.append("| " + " | ".join(safe) + " |")
    with Path(summary_path).open("a", encoding="utf-8") as output:
        output.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
