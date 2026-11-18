#!/usr/bin/env python3
"""Summarize the local migration toolkit run for a bundled evaluation fixture."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import generated_check
import stage2
import validate


REPO = validate.REPO
TOOLKIT = Path(__file__).resolve().parents[1]
EVALUATION = Path("tests/developer-platform/evaluation")
API_EVIDENCE = Path("tests/developer-platform/api")
API_CASES = {"windows": "read_prefix", "linux": "watch_changes"}
UI_PATHS = tuple(EVALUATION / "ui" / name for name in (
    "task_panel.ui", "gtk_minimal.ui", "gtk3_task_panel.ui", "gtk4_task_panel.ui",
    "xaml_minimal.xaml", "wpf_task_panel.xaml", "winui_task_panel.xaml",
))
FIXTURES = ("portable", "windows", "linux", "ui")
PORTABLE_TARGET = "portable-summary"
MAX_TOOL_OUTPUT = 512 * 1024
MAX_REPORT_BYTES = 128 * 1024
MAX_API_DETAILS = 32
TOOL_TIMEOUT = 10
MAPPING_STATUSES = ("supported", "partial", "unsupported", "needs_decision")
UI_FACTS = ("widgets", "layouts", "connections", "tab_order", "bindings",
            "resources", "unknown", "custom_widgets", "unresolved_references")


CHECK_STATUSES = ("passed", "failed", "not_run", "unsupported")
# PROVISIONAL plain-language notes for reason codes. A code missing from this
# table is shown as the code alone; no explanation is guessed for it.
REASON_DESCRIPTIONS = {
    "evidence_diagnosed": "API evidence was diagnosed.",
    "structure_analyzed": "The UI file structure was analyzed.",
    "mapping_summarized": "The UI mapping was summarized.",
    "ui_analyzed": "All UI files were analyzed.",
    "ui_mapped": "All UI files were mapped.",
    "scaffold_created": "The scratch scaffold was created.",
    "metadata_valid": "The validator accepted the scaffold Manifest.",
    "compiled": "The portable fixture compiled on the host.",
    "test_passed": "The compiled test program ran and produced the expected output.",
    "build_and_test_passed": "The portable fixture compiled and its test passed on the host.",
    "no_platform_evidence_for_fixture": "This fixture has no API evidence for the platform.",
    "no_ui_inputs_for_fixture": "This fixture has no UI inputs.",
    "no_approved_stage2_target_for_fixture": "No approved Stage 2 build target exists for this fixture.",
    "package_and_signature_checks_not_connected": "Package and signature checks are not connected yet.",
    "stage_not_connected": "This stage is not connected yet.",
    "generation_did_not_pass": "Scaffold generation did not pass, so the check was skipped.",
    "ui_analysis_did_not_pass": "UI analysis did not pass, so mapping was skipped.",
    "file_failed": "At least one file failed.",
    "file_not_run": "At least one file was not run.",
    "file_unsupported": "At least one file is unsupported.",
    "validator_unavailable": "The validator binary is not available.",
    "clang_unavailable": "The clang compiler is not available.",
    "scratch_unavailable": "A scratch directory could not be used.",
    "scratch_cleanup_failed": "The scratch directory could not be removed.",
    "approved_target_unavailable": "The approved Stage 2 target is not available.",
    "tool_unavailable": "A required tool is not available.",
    "tool_exit_nonzero": "A tool exited with an error.",
    "invalid_tool_output": "A tool produced output that was not in the expected form.",
    "unsupported_format": "The input format is not supported.",
    "input_unavailable": "An input file could not be read.",
    "input_too_large": "An input file exceeds the size limit.",
    "fixture_digest_mismatch": "The fixture differs from its approved content digest.",
    "output_limit": "Tool output exceeded the size limit.",
    "timeout": "A tool exceeded its time limit.",
    "io_error": "A file system error occurred.",
    "invalid_xml": "The file is not valid XML.",
}
# PROVISIONAL allowlist of reason codes that are intentionally shown as the code
# alone. Empty today: every reason in the four fixture reports has a description.
# Add a code here only on purpose; test_report.py fails for a reason that is in
# neither this set nor REASON_DESCRIPTIONS.
CODE_ONLY_REASONS = frozenset()


class CheckError(Exception):
    def __init__(self, reason, status="failed"):
        super().__init__(reason)
        self.status = status


def check(status, reason, paths=(), counts=None, **details):
    return {"status": status, "reason": reason,
            "input_paths": [path.as_posix() for path in paths],
            "counts": {**(counts or {}), "inputs": len(paths)}, **details}


def unavailable(reason, paths=()):
    return check("not_run", reason, paths)


def attempt(paths, operation):
    """Keep component failures local and omit arbitrary exception text."""
    try:
        return operation()
    except CheckError as error:
        return check(error.status, str(error), paths)
    except validate.InputError as error:
        return check("failed", str(error), paths)
    except OSError:
        return check("failed", "io_error", paths)
    except (KeyError, TypeError, ValueError, RecursionError):
        return check("failed", "invalid_tool_output", paths)


def reason_code(value, default):
    if isinstance(value, str) and re.fullmatch(r"[a-zA-Z][a-zA-Z0-9_:-]{0,79}", value):
        return value
    return default


def tool_error(stderr):
    message = stderr.decode("utf-8", errors="replace").strip()
    try:
        result = json.loads(message)
    except ValueError:
        return reason_code(message, "tool_exit_nonzero")
    return reason_code(result.get("error"), "tool_exit_nonzero") if isinstance(
        result, dict) else "tool_exit_nonzero"


def run_json_script(script, args=(), data=None):
    """Use the existing bounded transport and a file descriptor for CLI stdin."""
    command = [sys.executable, "-E", "-s", "-B", str(TOOLKIT / script), *args]
    if data is None:
        execution = validate._run(command, REPO, TOOL_TIMEOUT, output_limit=MAX_TOOL_OUTPUT)
    else:
        with tempfile.TemporaryFile() as source:
            source.write(data)
            source.seek(0)
            execution = validate._run(command, REPO, TOOL_TIMEOUT, output_limit=MAX_TOOL_OUTPUT,
                                      stdin=source)
    status = execution["status"]
    if status == "unavailable":
        raise CheckError("tool_unavailable", "not_run")
    if status != "exited":
        raise CheckError(status)
    if execution["exit_code"] != 0:
        reason = tool_error(execution["stderr"])
        raise CheckError(reason, "unsupported" if reason == "unsupported_format" else "failed")
    output = json.loads(execution["stdout"])
    if not isinstance(output, dict) or execution["stderr"]:
        raise CheckError("invalid_tool_output")
    return output, execution["stdout"]


def diagnose(fixture, platform):
    if fixture != platform:
        return check("unsupported", "no_platform_evidence_for_fixture")
    basename = API_CASES[platform]
    source = EVALUATION / platform / (basename + ".c")
    evidence = API_EVIDENCE / platform / (basename + ".report-v1.json")
    paths = (source, evidence)

    def run():
        # Check that both inputs are inside the trusted root. The diagnosis
        # consumes the handwritten evidence, using the source basename as its allowlist.
        validate.read_manifest(REPO, source)
        data = validate.read_manifest(REPO, evidence)
        result, _ = run_json_script("api/{}/diagnose.py".format(platform),
                                    ("--path", source.name), data)
        rows = result["rows"]
        if result["diagnostic_version"] != 1 or not isinstance(rows, list):
            raise CheckError("invalid_tool_output")
        counts = {"sources": 1, "evidence_reports": 1, "rows": len(rows),
                  "matched_rules": 0, "unresolved": 0, "unsupported": 0}
        details = []
        for row in rows:
            origin = row["source"]
            if origin["path"] != source.name:
                raise CheckError("finding_outside_selected_paths")
            counts["unresolved" if row["rule_id"] is None else "matched_rules"] += 1
            counts["unsupported"] += row["availability"] == "unsupported"
            if len(details) < MAX_API_DETAILS:
                details.append({"line": origin["line"], "rule_id": row["rule_id"],
                                "apis": row["original_apis"], "availability": row["availability"]})
        return check("passed", "evidence_diagnosed", paths, counts,
                     rows=details, truncated=len(details) < len(rows))

    return attempt(paths, run)


def summarize_ui(paths, files, success_reason):
    counts = Counter()
    states = Counter(item["status"] for item in files)
    for item in files:
        counts.update(item["counts"])
    counts.update(files=len(files), files_passed=states["passed"], files_failed=states["failed"],
                  files_not_run=states["not_run"], files_unsupported=states["unsupported"])
    for status, reason in (("failed", "file_failed"), ("not_run", "file_not_run"),
                           ("unsupported", "file_unsupported")):
        if states[status]:
            return check(status, reason, paths, counts, files=files)
    return check("passed", success_reason, paths, counts, files=files)


def inspect_ui(fixture):
    if fixture != "ui":
        return (check("unsupported", "no_ui_inputs_for_fixture"),
                check("unsupported", "no_ui_inputs_for_fixture"))
    analyzed, mapped = [], []
    for path in UI_PATHS:
        payload = None

        def analyze():
            nonlocal payload
            facts, encoded = run_json_script("ui/analysis/analyze.py",
                                             ("--root", str(REPO), path.as_posix()))
            if facts["analysis_version"] != 1 or facts["source"] != path.as_posix() or any(
                    not isinstance(facts[key], list) for key in UI_FACTS):
                raise CheckError("invalid_tool_output")
            result = check("passed", "structure_analyzed", (path,),
                           {key: len(facts[key]) for key in UI_FACTS}, format=facts["format"])
            payload = encoded
            return result

        def map_ui():
            mapping, _ = run_json_script("ui/mapping/map_ui.py", data=payload)
            status_counts = mapping["summary"]["status_counts"]
            if (mapping["mapping_version"] != 1 or mapping["source_format"] != analysis["format"]
                    or set(status_counts) != set(MAPPING_STATUSES)
                    or any(type(n) is not int or n < 0 for n in status_counts.values())
                    or not isinstance(mapping["rows"], list)
                    or sum(status_counts.values()) != len(mapping["rows"])):
                raise CheckError("invalid_tool_output")
            return check("passed", "mapping_summarized", (path,),
                         {"rows": len(mapping["rows"]), **status_counts},
                         format=mapping["source_format"], contract_evidence=mapping["contract"]["state"])

        analysis = attempt((path,), analyze)
        analyzed.append(analysis)
        mapped.append(attempt((path,), map_ui) if analysis["status"] == "passed" else
                      unavailable("ui_analysis_did_not_pass", (path,)))
    return (summarize_ui(UI_PATHS, analyzed, "ui_analyzed"),
            summarize_ui(UI_PATHS, mapped, "ui_mapped"))


def adapter_result(result, paths, counts, **details):
    status = result["status"]
    if status == "unavailable":
        status = "not_run"
    elif status not in ("passed", "not_run"):
        status = "failed"
    return check(status, reason_code(result["reason"], "adapter_error"), paths, counts, **details)


def generate_and_validate():
    manifest = Path(generated_check.DEFAULT_INPUTS["output_dir"]) / generated_check.MANIFEST_NAME
    provenance = {"input_root": "scratch_scaffold"}
    stage1 = check("not_run", "generation_did_not_pass", (manifest,), {"manifests": 0},
                   **provenance)
    generation = unavailable("scratch_unavailable")
    try:
        with tempfile.TemporaryDirectory(prefix="nagi-ndp-report-scaffold-") as directory:
            root = Path(directory)

            def generate():
                result = generated_check.generate_scaffold(root)
                if result["status"] == "created":
                    return check("passed", "scaffold_created", counts={"files": len(result["created"])})
                return adapter_result(result, (), {"files": 0})

            generation = attempt((), generate)
            if generation["status"] == "passed":
                def validate_manifest():
                    result = validate.validate(root, manifest)
                    return adapter_result(result, (manifest,),
                                          {"manifests": int(result["exit_code"] is not None)},
                                          exit_code=result["exit_code"], **provenance)

                stage1 = {**provenance, **attempt((manifest,), validate_manifest)}
    except OSError:
        if generation["status"] == "passed":
            generation.update(status="failed", reason="scratch_cleanup_failed")
    return generation, stage1


def host_build(fixture):
    if fixture != "portable":
        return check("unsupported", "no_approved_stage2_target_for_fixture")
    try:
        targets = stage2.load_targets()
    except stage2.ContractError as error:
        return unavailable(str(error))
    target = targets.get(PORTABLE_TARGET)
    if target is None or Path(*target.directory) != EVALUATION / fixture:
        return unavailable("approved_target_unavailable")
    directory = Path(*target.directory)
    paths = tuple(directory / name for name in (*target.sources, *target.headers))

    def run():
        # Stage 1 validates the generated scaffold. This independent host check
        # builds only the portable fixture described by the Stage 2 contract.
        result = stage2.run_stage2(target.id, fixture_dir=REPO / directory, cwd=REPO, targets=targets)
        # With the target selected and no prior-stage gate, Stage 2's not_run
        # denotes a rejected input. Report it as a failure and preserve the reason.
        if result["status"] == "not_run":
            result = {**result, "status": "failed"}
        steps = [{"name": item["name"], **adapter_result(
            item, paths, {}, exit_code=item["exit_code"])} for item in result["steps"]]
        counts = {"sources": len(target.sources), "headers": len(target.headers),
                  "steps_run": sum(step["exit_code"] is not None for step in steps),
                  "steps_passed": sum(step["status"] == "passed" for step in steps)}
        if result.get("diagnostics") is not None:
            counts.update({key: result["diagnostics"][key] for key in ("errors", "warnings")})
        return adapter_result(result, paths, counts, steps=steps)

    return attempt(paths, run)


def build_overview(checks):
    """Summarize check statuses; the non-passed list keeps check order."""
    counts = dict.fromkeys(CHECK_STATUSES, 0)
    for result in checks.values():
        counts[result["status"]] = counts.get(result["status"], 0) + 1
    return {"total": len(checks), "counts": counts,
            "not_passed": [{"name": name, "status": result["status"], "reason": result["reason"]}
                           for name, result in checks.items() if result["status"] != "passed"]}


def describe_reason(reason):
    text = REASON_DESCRIPTIONS.get(reason)
    return "`{}`".format(reason) if text is None else "`{}` — {}".format(reason, text)


def collect_reasons(report):
    """Every reason code in one report: checks, per-file results and steps."""
    reasons = set()
    for result in report["checks"].values():
        reasons.add(result["reason"])
        reasons.update(item["reason"] for item in (*result.get("files", ()), *result.get("steps", ())))
    return reasons


def undescribed_reasons(reports):
    """Sorted reason codes in `reports` that have neither a description nor an allowlist entry."""
    found = set().union(*(collect_reasons(item) for item in reports)) if reports else set()
    return sorted(found - REASON_DESCRIPTIONS.keys() - CODE_ONLY_REASONS)


def overview_lines(report):
    overview = report["overview"]
    passed = [name for name, result in report["checks"].items() if result["status"] == "passed"]
    lines = ["## Overview", "",
             "{} checks: {}.".format(overview["total"], ", ".join(
                 "{} `{}`".format(count, status) for status, count in overview["counts"].items())),
             "", "### Passed", ""]
    lines.extend("- {}".format(name) for name in passed) if passed else lines.append("- none")
    lines.extend(("", "### Unsupported or not run (reason)", ""))
    if overview["not_passed"]:
        lines.extend("- {} · `{}` · {}".format(item["name"], item["status"],
                                               describe_reason(item["reason"]))
                     for item in overview["not_passed"])
    else:
        lines.append("- none")
    lines.append("")
    return lines


def build_report(fixture):
    if fixture not in FIXTURES:
        raise ValueError("unknown_fixture")
    recorded_at = datetime.now(timezone.utc).isoformat(timespec="seconds")
    windows = diagnose(fixture, "windows")
    linux = diagnose(fixture, "linux")
    analysis, mapping = inspect_ui(fixture)
    generation, stage1 = generate_and_validate()
    checks = {
        "windows_diagnosis": windows, "linux_diagnosis": linux,
        "ui_analysis": analysis, "ui_mapping": mapping,
        "generation": generation, "stage_1": stage1, "stage_2": host_build(fixture),
        "stage_2_package": unavailable("package_and_signature_checks_not_connected"),
        "stage_3": unavailable("stage_not_connected"),
    }
    return {
        "version": 1, "approval": "PROVISIONAL", "recorded_at": recorded_at,
        "fixture": fixture, "source_kind": "self_made_repository_fixture",
        "mode": "toolkit_non_ai", "scope": "host_fixture_evaluation",
        "target_acceptance": "not_run",
        "checks": checks,
        "overview": build_overview(checks),
        "evaluation": {
            "manual_comparison": "not_run", "local_llm": "not_run", "external_ai": "not_run",
            "human_effort": "not_measured", "reuse_and_speedup": "not_measured",
            "resource_usage": "not_measured",
        },
    }


def markdown_report(report):
    lines = [
        "# Migration evaluation",
        "",
        "**PROVISIONAL** · Fixture `{}` · Toolkit (non-AI)".format(report["fixture"]),
        "",
        "Recorded at {}. Nagi target acceptance `not_run`.".format(report["recorded_at"]),
        "",
        *overview_lines(report),
        "## Checks",
        "",
        "| Check | Status | Reason | Counts | Input paths |",
        "|---|---|---|---|---|",
    ]
    for name, result in report["checks"].items():
        counts = ", ".join("{}={}".format(key, value) for key, value in result["counts"].items())
        paths = "<br>".join("`{}`".format(path) for path in result["input_paths"])
        lines.append("| {} | `{}` | `{}` | {} | {} |".format(
            name, result["status"], result["reason"], counts, paths or "—"))
    problems = []
    for name, result in report["checks"].items():
        for item in (*result.get("files", ()), *result.get("steps", ())):
            if item["status"] != "passed":
                label = item["name"] if "name" in item else item["input_paths"][0]
                problems.append("- {} `{}` · `{}` · {}".format(
                    name, label, item["status"], describe_reason(item["reason"])))
    if problems:
        lines.extend(("", *problems))
    lines.extend(("", "Stage 1 checks metadata of the scratch scaffold. Stage 2 covers the "
                  "approved host fixture only. Manual and AI comparisons, effort and reuse "
                  "measurements are pending.", ""))
    return "\n".join(lines)


def build_summary(reports):
    """PROVISIONAL fixture x check matrix over per-fixture reports.

    Status counts are summed from each report's own overview, so the
    classification stays in build_overview and is not implemented twice.
    """
    names = list(next(iter(reports.values()))["checks"]) if reports else []
    counts = dict.fromkeys(CHECK_STATUSES, 0)
    not_passed = []
    for fixture, result in reports.items():
        for status, count in result["overview"]["counts"].items():
            counts[status] = counts.get(status, 0) + count
        not_passed.extend({"fixture": fixture, **item} for item in result["overview"]["not_passed"])
    reasons = {}
    for item in not_passed:
        reasons.setdefault(item["reason"], REASON_DESCRIPTIONS.get(item["reason"]))
    return {
        "fixtures": list(reports), "checks": names,
        "matrix": {fixture: {name: {"status": result["checks"][name]["status"],
                                    "reason": result["checks"][name]["reason"]}
                             for name in names}
                   for fixture, result in reports.items()},
        "total": sum(result["overview"]["total"] for result in reports.values()),
        "counts": counts,
        "by_fixture": {fixture: result["overview"]["counts"] for fixture, result in reports.items()},
        "not_passed": not_passed,
        # null means the code is not in REASON_DESCRIPTIONS; nothing is guessed.
        "reasons": [{"reason": reason, "description": text} for reason, text in reasons.items()],
    }


def build_all_report():
    reports = {fixture: build_report(fixture) for fixture in FIXTURES}
    return {
        "version": 1, "approval": "PROVISIONAL",
        "recorded_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "fixture": "all", "source_kind": "self_made_repository_fixture",
        "mode": "toolkit_non_ai", "scope": "host_fixture_evaluation",
        "target_acceptance": "not_run",
        "summary": build_summary(reports),
        "reports": reports,
    }


def summary_lines(summary):
    names = summary["checks"]
    counts = summary["counts"]
    lines = ["## Fixture x check matrix", "",
             "{} results: {}.".format(summary["total"], ", ".join(
                 "{} `{}`".format(count, status) for status, count in counts.items())),
             "", "| Fixture | " + " | ".join(names) + " |",
             "|---|" + "---|" * len(names)]
    for fixture, row in summary["matrix"].items():
        lines.append("| {} | ".format(fixture) + " | ".join(
            "`{}`<br>`{}`".format(row[name]["status"], row[name]["reason"]) for name in names) + " |")
    lines.extend(("", "### Unsupported or not run (reason)", ""))
    if summary["not_passed"]:
        lines.extend("- {} / {} · `{}` · {}".format(item["fixture"], item["name"], item["status"],
                                                   describe_reason(item["reason"]))
                     for item in summary["not_passed"])
    else:
        lines.append("- none")
    lines.append("")
    return lines


def markdown_all_report(result):
    lines = ["# Migration evaluation: all fixtures", "",
             "**PROVISIONAL** · Toolkit (non-AI) · host fixtures only. "
             "Nagi target acceptance `not_run`.", "",
             *summary_lines(result["summary"])]
    parts = ["\n".join(lines)]
    parts.extend(markdown_report(item) for item in result["reports"].values())
    return "\n---\n\n".join(parts)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, choices=(*FIXTURES, "all"))
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    args = parser.parse_args(argv)
    if args.fixture == "all":
        report = build_all_report()
        reports = list(report["reports"].values())
        limit = MAX_REPORT_BYTES * len(FIXTURES)
        render = markdown_all_report
    else:
        report = build_report(args.fixture)
        reports = [report]
        limit = MAX_REPORT_BYTES
        render = markdown_report
    if args.format == "markdown":
        output = render(report)
    else:
        output = json.dumps(report, indent=2) + "\n"
    if len(output.encode("utf-8")) > limit:
        print("report_output_limit", file=sys.stderr)
        return 1
    sys.stdout.write(output)
    return int(any(item["status"] == "failed" for result in reports
                   for item in result["checks"].values()))


if __name__ == "__main__":
    sys.exit(main())
