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


def build_report(fixture):
    if fixture not in FIXTURES:
        raise ValueError("unknown_fixture")
    recorded_at = datetime.now(timezone.utc).isoformat(timespec="seconds")
    windows = diagnose(fixture, "windows")
    linux = diagnose(fixture, "linux")
    analysis, mapping = inspect_ui(fixture)
    generation, stage1 = generate_and_validate()
    return {
        "version": 1, "approval": "PROVISIONAL", "recorded_at": recorded_at,
        "fixture": fixture, "source_kind": "self_made_repository_fixture",
        "mode": "toolkit_non_ai", "scope": "host_fixture_evaluation",
        "target_acceptance": "not_run",
        "checks": {
            "windows_diagnosis": windows, "linux_diagnosis": linux,
            "ui_analysis": analysis, "ui_mapping": mapping,
            "generation": generation, "stage_1": stage1, "stage_2": host_build(fixture),
            "stage_2_package": unavailable("package_and_signature_checks_not_connected"),
            "stage_3": unavailable("stage_not_connected"),
        },
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
                problems.append("- {} `{}` · `{}` · `{}`".format(
                    name, label, item["status"], item["reason"]))
    if problems:
        lines.extend(("", *problems))
    lines.extend(("", "Stage 1 checks metadata of the scratch scaffold. Stage 2 covers the "
                  "approved host fixture only. Manual and AI comparisons, effort and reuse "
                  "measurements are pending.", ""))
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, choices=FIXTURES)
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    args = parser.parse_args(argv)
    report = build_report(args.fixture)
    if args.format == "markdown":
        output = markdown_report(report)
    else:
        output = json.dumps(report, indent=2) + "\n"
    if len(output.encode("utf-8")) > MAX_REPORT_BYTES:
        print("report_output_limit", file=sys.stderr)
        return 1
    sys.stdout.write(output)
    return int(any(item["status"] == "failed" for item in report["checks"].values()))


if __name__ == "__main__":
    sys.exit(main())
