"""Apply trusted Windows rules to report evidence, without scanning source."""

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


# Mask only evidence supplied by PT-01. This is not a source parser: incomplete
# fragments and symbol-only evidence cannot establish surrounding lexical state.
# Raw/interpolated strings are conservatively opaque, including interpolations.
_LEXEMES = re.compile(
    r'(?P<comment>//[^\r\n]*|/\*.*?(?:\*/|\Z))'
    r'|(?P<cppraw>(?:u8|u|U|L)?R"(?P<delimiter>[^ ()\\\t\r\n]{0,16})\(.*?\)(?P=delimiter)")'
    r'|(?P<csraw>\$*(?P<quotes>"{3,}).*?(?P=quotes))'
    # `#include "name.h"`: the quoted header name is visible to alias matching
    # (the angle form is already plain tokens); other string contents stay opaque.
    r'|(?P<quotedinclude>\#[ \t]*include[ \t]*"[^"\r\n]*")'
    r'|(?P<verbatim>(?:\$?@|@\$)"(?:""|[^"])*(?:"|\Z))'
    r'|(?P<string>(?:u8|u|U|L|\$)?"(?:\\.|[^"\\])*(?:"|\Z))'
    r"|(?P<char>'(?:\\.|[^'\\])*(?:'|\Z))"
    r'|(?P<identifier>[^\W\d]\w*)|(?P<dot>\.)|(?P<space>\s+)|(?P<other>.)',
    re.DOTALL,
)


def _skip_literal(evidence, match, nesting=0):
    """Keep C# interpolation expressions opaque, including nested literals."""
    text = match.group()
    interpolated = (match.lastgroup == "string" and text.startswith('$"')) or (
        match.lastgroup == "verbatim" and text.startswith(('$@"', '@$"')))
    if not interpolated:
        return match.end()
    if nesting >= 32:
        return len(evidence)  # Unresolved fragment: conservatively mask the rest.
    verbatim = text.startswith(('$@"', '@$"'))
    i = match.start() + (3 if verbatim else 2)
    depth = 0
    while i < len(evidence):
        char = evidence[i]
        if depth:
            part = _LEXEMES.match(evidence, i)
            if part.lastgroup in ("comment", "cppraw", "csraw", "verbatim", "string", "char"):
                i = _skip_literal(evidence, part, nesting + 1)
                continue
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
        else:
            if not verbatim and char == "\\":
                i += 2
                continue
            if char == '"':
                if verbatim and evidence[i:i + 2] == '""':
                    i += 2
                    continue
                return i + 1
            if evidence[i:i + 2] in ("{{", "}}"):
                i += 2
                continue
            if char == "{":
                depth = 1
        i += 1
    return len(evidence)


def evidence_tokens(evidence):
    # Conservatively honor C/C++ preprocessing continuations, including within
    # line comments. The original fragment (not this projection) is hashed.
    evidence = re.sub(r"\\\r?\n", "", evidence)
    tokens = []
    offset = 0
    while offset < len(evidence):
        match = _LEXEMES.match(evidence, offset)
        offset = _skip_literal(evidence, match)
        kind = match.lastgroup
        if kind in ("identifier", "dot"):
            tokens.append(match.group())
        elif kind == "quotedinclude":
            name = match.group().split('"', 1)[1]
            tokens.append(None)
            tokens.extend(re.findall(r"[^\W\d]\w*|\.", name))
            tokens.append(None)
        elif kind != "space":
            # A barrier prevents a dotted alias spanning a comment or literal.
            tokens.append(None)
    return tokens


def load_rules():
    """Only the bundled, trusted data is loaded; reports cannot select rules."""
    return json.loads(Path(__file__).with_name("rules.json").read_text(encoding="utf-8"))


def _matches(tokens, alias, fold=False):
    """`fold` (rule data `case_insensitive`) is for names Windows resolves case-insensitively."""
    expected = re.findall(r"[^\W\d]\w*|\.", alias)
    if fold:
        expected = [t.lower() for t in expected]
        tokens = [t if t is None else t.lower() for t in tokens]
    width = len(expected)
    return any(tokens[i:i + width] == expected for i in range(len(tokens) - width + 1))


def diagnose_report(data, *, allowed_paths, adapter=adapt_report_v1, rules=None):
    """Return an internal result. Adapter/rules injection is for trusted callers."""
    findings = adapter(data, allowed_paths=allowed_paths)
    rules = load_rules() if rules is None else copy.deepcopy(rules)
    rows = []
    for finding in findings:
        tokens = evidence_tokens(finding.evidence)
        source = {
            "category": finding.category, "path": finding.path, "line": finding.line,
            "review_status": finding.review_status,
            "evidence_sha256": hashlib.sha256(finding.evidence.encode("utf-8")).hexdigest(),
            "evidence_scope": "report_fragment_only",
        }
        matched = False
        for rule in rules["rules"]:
            aliases = sorted(a for a in rule["aliases"] if _matches(tokens, a, rule.get("case_insensitive") is True))
            if not aliases:
                continue
            matched = True
            rows.append({
                "source": copy.deepcopy(source), "rule_id": rule["id"], "original_apis": aliases,
                "family": rule["family"], "classification": rule["classification"],
                "detection": "lexical_candidate", "reason": rule["reason"],
                "alternatives": copy.deepcopy(rule["alternatives"]),
                "gaps": list(rule["gaps"]), "references": list(rule["references"]),
                "availability": rule["availability"], "approval": "PROVISIONAL",
            })
        if not matched:
            has_code = any(t is not None and t != "." for t in tokens)
            state = ("unknown_api" if finding.category in rules["windows_categories"]
                     else "not_classified") if has_code else "no_code_evidence"
            rows.append({
                "source": source, "rule_id": None, "original_apis": [],
                "family": "unknown", "classification": "unresolved",
                "detection": state, "reason": "No bundled API rule matches this report fragment.",
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
