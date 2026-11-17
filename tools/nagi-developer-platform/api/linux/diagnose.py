"""Apply trusted Linux rules to report evidence, without scanning source."""

import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import sys

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from common.pt01_report import ReportError, adapt_report_v1, read_report


# Mask only evidence supplied by PT-01. This is a C/C++ token projection, not a
# parser: incomplete fragments and symbol-only evidence cannot establish the
# surrounding lexical state. Comments are dropped; literals become barriers.
_LEXEMES = re.compile(
    r'(?P<comment>//[^\r\n]*|/\*.*?(?:\*/|\Z))'
    r'|(?P<include>\#[ \t]*include[ \t]*(?:<(?P<angle>[^>\r\n]*)>|"(?P<quoted>[^"\r\n]*)"))'
    r'|(?P<raw>(?:u8|u|U|L)?R"(?P<delimiter>[^ ()\\\t\r\n]{0,16})\(.*?\)(?P=delimiter)")'
    r'|(?P<string>(?:u8|u|U|L)?"(?:\\.|[^"\\])*(?:"|\Z))'
    r'|(?P<number>\d(?:[\w.]|\'(?=\w))*)'
    r"|(?P<char>(?:u8|u|U|L)?'(?:\\.|[^'\\])*(?:'|\Z))"
    r'|(?P<identifier>[^\W\d]\w*)|(?P<arrow>->)|(?P<scope>::)|(?P<space>\s+)|(?P<other>.)',
    re.DOTALL,
)

# A preceding identifier of these kinds does not make `name(` a declaration.
_EXPRESSION_KEYWORDS = frozenset((
    "return", "else", "do", "case", "goto", "throw", "co_return", "co_await", "co_yield",
    "sizeof", "alignof", "decltype", "typeof", "new", "delete", "and", "or", "not"))

_SHAPES = ("call", "declaration_or_definition", "global_scope_call", "identifier")
_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_HEADER = re.compile(r"[A-Za-z0-9_./+-]+")
_AVAILABILITY = ("unsupported", "contract_only", "unknown")


def lex(evidence):
    """Return (kind, text) tokens; kinds: id, header, op, lit."""
    # C/C++ preprocessing joins continued lines, including inside // comments.
    evidence = re.sub(r"\\\r?\n", "", evidence)
    tokens = []
    offset = 0
    while offset < len(evidence):
        match = _LEXEMES.match(evidence, offset)
        offset = match.end()
        kind = match.lastgroup
        if kind == "include" or kind in ("angle", "quoted"):
            name = match.group("angle") if match.group("angle") is not None else match.group("quoted")
            tokens.append(("header", name.strip()))
        elif kind == "identifier":
            tokens.append(("id", match.group()))
        elif kind in ("arrow", "scope"):
            tokens.append(("op", match.group()))
        elif kind in ("raw", "string", "char", "number"):
            tokens.append(("lit", None))
        elif kind == "other":
            tokens.append(("op", match.group()))
        # comments and whitespace separate tokens but are not tokens.
    return tokens


def occurrences(tokens):
    """Project tokens to header names and identifier uses with their syntactic shape.

    Member access (`x.name`, `p->name`) and qualified names (`ns::name`) are not
    treated as the unqualified C name. A leading `::name` is global scope.
    """
    headers = []
    names = []
    for i, (kind, text) in enumerate(tokens):
        if kind == "header":
            headers.append(text)
            continue
        if kind != "id":
            continue
        prev = tokens[i - 1] if i else None
        qualifier = None
        if prev == ("op", ".") or prev == ("op", "->"):
            continue
        if prev == ("op", "::"):
            before = tokens[i - 2] if i >= 2 else None
            if before is not None and (before[0] == "id" or before in (("op", ">"), ("op", ")"), ("op", "]"))):
                continue
            qualifier = "global"
        is_call = i + 1 < len(tokens) and tokens[i + 1] == ("op", "(")
        declaration = (qualifier is None and prev is not None and prev[0] == "id"
                       and prev[1] not in _EXPRESSION_KEYWORDS)
        names.append((text, is_call, qualifier == "global", declaration))
    return headers, names


def load_rules():
    """Only the bundled, trusted data is loaded; reports cannot select rules."""
    return json.loads(Path(__file__).with_name("rules.json").read_text(encoding="utf-8"))


def validate_rules(rules):
    """Reject malformed trusted rule data with a stable code (caller-supplied rules too)."""
    def fail():
        raise ValueError("invalid_rules")
    if not isinstance(rules, dict) or type(rules.get("rules_version")) is not int:
        fail()
    for key in ("linux_categories", "limitations", "rules"):
        if not isinstance(rules.get(key), list):
            fail()
    seen = set()
    for rule in rules["rules"]:
        if not isinstance(rule, dict) or not isinstance(rule.get("id"), str) or rule["id"] in seen:
            fail()
        seen.add(rule["id"])
        for key in ("family", "classification", "reason"):
            if not isinstance(rule.get(key), str) or not rule[key]:
                fail()
        if rule.get("availability") not in _AVAILABILITY:
            fail()
        for key, pattern in (("headers", _HEADER), ("header_prefixes", _HEADER),
                             ("calls", _NAME), ("identifiers", _NAME)):
            values = rule.get(key)
            if not isinstance(values, list) or len(set(values)) != len(values) or not all(
                    isinstance(v, str) and pattern.fullmatch(v) for v in values):
                fail()
        if not any(rule[k] for k in ("headers", "header_prefixes", "calls", "identifiers")):
            fail()
        for key in ("alternatives", "gaps", "references"):
            if not isinstance(rule.get(key), list):
                fail()
    return rules


def _match(rule, headers, names):
    """Return (sorted rule-owned API names, sorted shapes). Never reflects input text."""
    apis = set()
    shapes = set()
    for header in headers:
        if header in rule["headers"]:
            apis.add(header)
            shapes.add("include")
        for prefix in rule["header_prefixes"]:
            if header.startswith(prefix):
                apis.add(prefix + "*")
                shapes.add("include")
    for name, is_call, is_global, declaration in names:
        if name in rule["calls"] and is_call:
            apis.add(name)
            shapes.add("declaration_or_definition" if declaration
                       else "global_scope_call" if is_global else "call")
        elif name in rule["identifiers"]:
            apis.add(name)
            shapes.add("identifier")
    return sorted(apis), sorted(shapes)


def diagnose_report(data, *, allowed_paths, adapter=adapt_report_v1, rules=None):
    """Return an internal result. Adapter/rules injection is for trusted callers."""
    findings = adapter(data, allowed_paths=allowed_paths)
    rules = validate_rules(load_rules() if rules is None else copy.deepcopy(rules))
    rows = []
    for finding in findings:
        tokens = lex(finding.evidence)
        headers, names = occurrences(tokens)
        source = {
            "category": finding.category, "path": finding.path, "line": finding.line,
            "review_status": finding.review_status,
            "evidence_sha256": hashlib.sha256(finding.evidence.encode("utf-8")).hexdigest(),
            "evidence_scope": "report_fragment_only",
        }
        matched = False
        for rule in rules["rules"]:
            apis, shapes = _match(rule, headers, names)
            if not apis:
                continue
            matched = True
            # A declaration-shaped name only is not evidence of use; keep it visible.
            only_declaration = shapes == ["declaration_or_definition"]
            rows.append({
                "source": copy.deepcopy(source), "rule_id": rule["id"], "original_apis": apis,
                "family": rule["family"], "classification": rule["classification"],
                "detection": "declaration_candidate" if only_declaration else "lexical_candidate",
                "syntax_shapes": shapes, "reason": rule["reason"],
                "alternatives": copy.deepcopy(rule["alternatives"]),
                "gaps": list(rule["gaps"]), "references": list(rule["references"]),
                "availability": rule["availability"], "approval": "PROVISIONAL",
            })
        if not matched:
            has_code = bool(headers) or bool(names) or any(k == "id" for k, _ in tokens)
            state = ("unknown_api" if finding.category in rules["linux_categories"]
                     else "not_classified") if has_code else "no_code_evidence"
            rows.append({
                "source": source, "rule_id": None, "original_apis": [],
                "family": "unknown", "classification": "unresolved",
                "detection": state, "syntax_shapes": [],
                "reason": "No bundled API rule matches this report fragment.",
                "alternatives": [], "gaps": ["Manual source and SDK review is required."],
                "references": [], "availability": "unknown", "approval": "PROVISIONAL",
            })
    # Input order and rule order do not affect output; repeated evidence remains
    # repeated rather than erasing PT-01 facts. JSON serialization is the tie-break.
    rows.sort(key=lambda r: (r["source"]["path"], r["source"]["line"],
                             r["rule_id"] or "", json_report(r)))
    return {
        "diagnostic_version": 1,
        "input_adapter": ("pr91-report-v1-provisional" if adapter is adapt_report_v1
                          else "caller_supplied_adapter"),
        "rules_version": rules["rules_version"], "scope": "host_report_analysis",
        "compatibility": "not_assessed", "target_acceptance": "not_run",
        "limitations": list(rules["limitations"]), "rows": rows,
    }


def json_report(result):
    return json.dumps(result, ensure_ascii=True, sort_keys=True, indent=2) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--path", action="append", required=True,
                        help="Explicit allowed project-relative finding path; repeatable")
    args = parser.parse_args(argv)
    try:
        result = diagnose_report(read_report(sys.stdin.buffer), allowed_paths=args.path)
    except ReportError as error:
        print(str(error), file=sys.stderr)
        return 2
    sys.stdout.write(json_report(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
