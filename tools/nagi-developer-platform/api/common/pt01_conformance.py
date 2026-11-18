"""Static conformance check of a report against the PR #91 schema facts.

The facts live in pr91_schema.json (data); replacing that file (or passing
`schema=`) adapts to the canonical PT-01 contract. Deviation codes are fixed
strings and never reflect report content. This never opens paths from a report.
"""

import json
from pathlib import Path

from .pt01_report import load_report_document


def load_schema():
    return json.loads(Path(__file__).with_name("pr91_schema.json").read_text(encoding="utf-8"))


def _is_int(value):
    return type(value) is int and value >= 0


def _string_list(value, allowed=None):
    return (isinstance(value, list) and all(isinstance(v, str) for v in value)
            and (allowed is None or all(v in allowed for v in value)))


def check_document(document, *, schema=None):
    """Return a sorted tuple of deviation codes; empty means conformant."""
    schema = load_schema() if schema is None else schema
    out = set()
    if not isinstance(document, dict):
        return ("root_not_object",)
    if type(document.get("schema_version")) is not int or document["schema_version"] != schema["schema_version"]:
        out.add("schema_version")
    top = schema["top_level"]
    expected = {"schema_version", *top}
    for name in sorted(expected):
        if name not in document:
            out.add("missing_field:" + name)
    if set(document) - expected:
        out.add("unexpected_field:root")
    for name, spec in top.items():
        if name not in document:
            continue
        value, kind = document[name], spec["type"]
        if kind == "string_list" and not _string_list(value, spec.get("enum")):
            out.add("invalid_type_or_enum:" + name)
        elif kind == "dependency_list":
            _check_dependencies(value, schema, out)
        elif kind == "analysis":
            _check_analysis(value, schema, out)
        elif kind == "finding_list":
            _check_findings(value, schema, out)
    _check_cross(document, schema, out)
    return tuple(sorted(out))


def _check_dependencies(value, schema, out):
    fields = set(schema["dependency_fields"])
    if not isinstance(value, list):
        out.add("invalid_type_or_enum:dependencies")
        return
    for row in value:
        if not isinstance(row, dict):
            out.add("invalid_type_or_enum:dependencies")
        elif set(row) != fields:
            out.add("dependency_fields")
        elif (not isinstance(row["name"], str) or not isinstance(row["source"], str)
              or row["review_status"] not in schema["review_status_enum"]):
            out.add("invalid_type_or_enum:dependencies")


def _check_analysis(value, schema, out):
    ints, bools = schema["analysis_integer_fields"], schema["analysis_boolean_fields"]
    names = {*ints, *bools, "conclusion"}
    if not isinstance(value, dict):
        out.add("invalid_type_or_enum:analysis")
        return
    if names - set(value):
        out.add("analysis_missing_field")
    if set(value) - names:
        out.add("unexpected_field:analysis")
    if any(n in value and not _is_int(value[n]) for n in ints):
        out.add("invalid_type_or_enum:analysis")
    if any(n in value and type(value[n]) is not bool for n in bools):
        out.add("invalid_type_or_enum:analysis")
    if "conclusion" in value and value["conclusion"] not in schema["conclusion_enum"]:
        out.add("invalid_type_or_enum:analysis")


def _member(value, allowed):
    """Membership that is False (not an exception) for unhashable JSON values."""
    return isinstance(value, str) and value in allowed


def _check_findings(value, schema, out):
    if not isinstance(value, list):
        out.add("invalid_type_or_enum:findings")
        return
    fields = set(schema["finding_fields"])
    prefix, sep = schema["evidence_prefix"], schema["evidence_separator"]
    keys = []
    for row in value:
        if not isinstance(row, dict):
            out.add("invalid_type_or_enum:findings")
            continue
        if fields - set(row):
            out.add("finding_missing_field")
        if set(row) - fields:
            out.add("unexpected_field:finding")
        category, path, line = row.get("category"), row.get("path"), row.get("line")
        evidence = row.get("evidence")
        if "category" in row and not _member(category, schema["categories"]):
            out.add("unknown_category")
        if "path" in row and (not isinstance(path, str) or not path):
            out.add("invalid_path_type")
        if "line" in row and (type(line) is not int or line < 1):
            out.add("invalid_line")  # null, 0, bool, float and strings included
        if "review_status" in row and not _member(row["review_status"], schema["review_status_enum"]):
            out.add("unknown_review_status")
        if "evidence" in row:
            if not isinstance(evidence, str) or not evidence.startswith(prefix):
                out.add("evidence_format")
            elif _member(category, schema["categories"]):
                tokens = evidence[len(prefix):].split(sep)
                if not all(t in schema["categories"][category] for t in tokens):
                    out.add("evidence_token_not_in_category")
        if (isinstance(path, str) and type(line) is int and isinstance(category, str)
                and isinstance(evidence, str)):
            keys.append((path, line, category, evidence))
    if keys != sorted(keys) or len(keys) != len(set(keys)):
        out.add("findings_not_sorted_unique")


def _check_cross(document, schema, out):
    analysis, findings = document.get("analysis"), document.get("findings")
    deps = document.get("dependencies")
    if (isinstance(analysis, dict) and isinstance(findings, list) and isinstance(deps, list)
            and type(analysis.get("truncated")) is bool
            and _member(analysis.get("conclusion"), schema["conclusion_enum"])):
        needs = analysis["truncated"] or bool(findings) or bool(deps)
        want = "requires-review" if needs else "no-known-boundary-detected"
        if analysis["conclusion"] != want:
            out.add("conclusion_inconsistent")
    limits = document.get("limitations")
    if isinstance(limits, list) and not set(schema["required_limitations"]) <= {
            v for v in limits if isinstance(v, str)}:
        out.add("required_limitation_missing")


def check_report(data, *, schema=None):
    """Bytes form of check_document; parse errors raise ReportError."""
    return check_document(load_report_document(data), schema=schema)


def completeness(data):
    """(truncated, conclusion) from a report, or (None, None) if the shape is missing.

    `truncated=True` means findings may be incomplete: absence of a finding is then
    not evidence that an API is unused.
    """
    document = load_report_document(data)
    analysis = document.get("analysis") if isinstance(document, dict) else None
    if not isinstance(analysis, dict) or type(analysis.get("truncated")) is not bool:
        return None, None
    conclusion = analysis.get("conclusion")
    return analysis["truncated"], conclusion if isinstance(conclusion, str) else None
