"""Bounded PR #91 report-v1 projection. Never opens paths from a report."""

from dataclasses import dataclass
import json
import math
import re


# Resource bounds (PROVISIONAL, not a product requirement). MAX_FINDINGS follows the
# finding cap observed in the PR #91 scanner (10,000), so a report the scanner can
# legitimately emit is not rejected; byte/value bounds are sized for that count.
MAX_REPORT_BYTES = 8 * 1024 * 1024
MAX_FINDINGS = 10000
MAX_EVIDENCE_CHARS = 4096
MAX_DEPTH = 32
MAX_VALUES = 200000


class ReportError(ValueError):
    """A stable code, with no reflected report content."""


@dataclass(frozen=True)
class Finding:
    category: str
    path: str
    line: int
    evidence: str
    review_status: str


def relative_path(value):
    if (not isinstance(value, str) or not value or len(value) > 1024
            or any(ord(c) < 32 or ord(c) == 127 or 0xD800 <= ord(c) <= 0xDFFF for c in value)
            or any(c in value for c in ("\\", ":", "%"))
            or value.startswith("~")
            or any(p in ("", ".", "..") for p in value.split("/"))):
        raise ReportError("invalid_relative_path")
    return value


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ReportError("duplicate_json_key")
        result[key] = value
    return result


def _constant(_):
    raise ReportError("invalid_json_constant")


def _integer(value):
    if len(value) > 20:
        raise ReportError("json_integer_limit")
    return int(value)


def _float(value):
    number = float(value)
    if not math.isfinite(number):
        raise ReportError("invalid_json_number")
    return number


def _check_tree(document):
    stack = [(document, 0)]
    count = 0
    while stack:
        value, depth = stack.pop()
        count += 1
        if depth > MAX_DEPTH or count > MAX_VALUES:
            raise ReportError("json_structure_limit")
        if isinstance(value, dict):
            stack.extend((v, depth + 1) for v in value.values())
        elif isinstance(value, list):
            stack.extend((v, depth + 1) for v in value)


def load_report_document(data):
    """Bounded JSON parse of report bytes. Returns the document, validates nothing else."""
    if not isinstance(data, bytes):
        raise ReportError("report_requires_bytes")
    if len(data) > MAX_REPORT_BYTES:
        raise ReportError("report_byte_limit")
    try:
        document = json.loads(data.decode("utf-8"), object_pairs_hook=_pairs,
                              parse_constant=_constant, parse_int=_integer, parse_float=_float)
    except (UnicodeError, ValueError, RecursionError) as error:
        if isinstance(error, ReportError):
            raise
        raise ReportError("invalid_report_json") from None
    _check_tree(document)
    return document


def adapt_report_v1(data, *, allowed_paths, strict=False):
    """Accept UTF-8 bytes and an explicit allowlist, return immutable findings.

    Extra report/row fields are ignored after bounded JSON validation. Only the
    five documented fields are projected; schema evolution belongs here.
    `strict=True` additionally requires conformance with the PR #91 schema facts
    (see pt01_conformance.py); the default stays the lenient provisional projection.
    """
    document = load_report_document(data)
    if (not isinstance(document, dict)
            or type(document.get("schema_version")) is not int
            or document["schema_version"] != 1):
        raise ReportError("unsupported_report_version")
    rows = document.get("findings")
    if not isinstance(rows, list):
        raise ReportError("invalid_findings")
    if len(rows) > MAX_FINDINGS:
        raise ReportError("finding_count_limit")
    if not isinstance(allowed_paths, (list, tuple, set, frozenset)):
        raise ReportError("explicit_path_allowlist_required")
    paths = {relative_path(p) for p in allowed_paths}
    findings = []
    for row in rows:
        if not isinstance(row, dict):
            raise ReportError("invalid_finding")
        for name in ("category", "review_status"):
            value = row.get(name)
            if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", value):
                raise ReportError("invalid_finding_" + name)
        path = relative_path(row.get("path"))
        if path not in paths:
            raise ReportError("finding_outside_selected_paths")
        line = row.get("line")
        if type(line) is not int or not 1 <= line <= 2147483647:
            raise ReportError("invalid_finding_line")
        evidence = row.get("evidence")
        if (not isinstance(evidence, str) or not evidence.strip()
                or len(evidence) > MAX_EVIDENCE_CHARS
                or any(ord(c) < 32 and c not in "\t\n\r" for c in evidence)
                or any(0xD800 <= ord(c) <= 0xDFFF for c in evidence)):
            raise ReportError("invalid_finding_evidence")
        findings.append(Finding(row["category"], path, line, evidence, row["review_status"]))
    if strict:
        from .pt01_conformance import check_document
        if check_document(document):
            raise ReportError("pr91_conformance_violation")
    return tuple(findings)


def read_report(stream):
    """Read at most the byte budget plus one; do not drain oversized input."""
    data = stream.read(MAX_REPORT_BYTES + 1)
    if len(data) > MAX_REPORT_BYTES:
        raise ReportError("report_byte_limit")
    return data
