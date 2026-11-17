"""Deterministic, non-executing NDP-06 -> provisional Nagi UI gap report."""

import argparse
import copy
import json
from pathlib import Path
import sys

from contract import inspect_contract


HERE = Path(__file__).resolve().parent
STATUSES = ("supported", "partial", "unsupported", "needs_decision")
AXES = ("appearance_layout", "pointer_keyboard", "focus", "accessibility",
        "localization", "backend")
COLLECTIONS = ("widgets", "layouts", "connections", "bindings", "resources",
               "unknown", "custom_widgets", "unresolved_references")
MAX_INPUT_BYTES = 4 * 1024 * 1024


class MappingError(ValueError):
    """Stable diagnostic code, with no source content or input path."""

    def __init__(self, code):
        self.code = code
        super().__init__(code)


def _validate(analysis):
    # Bound recursive data before copying it. The CLI also bounds serialized input.
    stack, count = [(analysis, 0)], 0
    while stack:
        value, depth = stack.pop()
        count += 1
        if count > 100000 or depth > 64:
            raise MappingError("analysis_limit")
        if isinstance(value, dict):
            if not all(isinstance(k, str) for k in value):
                raise MappingError("invalid_analysis")
            if "attributes" in value and (not isinstance(value["attributes"], dict) or
                    not all(isinstance(v, str) for v in value["attributes"].values())):
                raise MappingError("invalid_analysis")
            if "text" in value and not isinstance(value["text"], str):
                raise MappingError("invalid_analysis")
            stack.extend((v, depth + 1) for v in value.values())
        elif isinstance(value, list):
            stack.extend((v, depth + 1) for v in value)
        elif value is not None and type(value) not in (str, bool, int):
            raise MappingError("invalid_analysis")
    if not isinstance(analysis, dict):
        raise MappingError("invalid_analysis")
    if type(analysis.get("analysis_version")) is not int or analysis["analysis_version"] != 1:
        raise MappingError("unsupported_analysis_version")
    if analysis.get("format") not in ("qt-designer-ui", "gtk-builder-ui"):
        raise MappingError("unsupported_analysis_format")
    for key in COLLECTIONS:
        if not isinstance(analysis.get(key), list) or not all(
                isinstance(item, dict) for item in analysis[key]):
            raise MappingError("invalid_analysis")
    for key in ("tab_order", "declared_slots", "limitations"):
        if not isinstance(analysis.get(key), list) or not all(
                isinstance(item, str) for item in analysis[key]):
            raise MappingError("invalid_analysis")
    for key in ("widgets", "layouts"):
        names = set()
        for item in analysis[key]:
            if (not isinstance(item.get("name"), str) or not item["name"]
                    or item["name"] in names or not isinstance(item.get("class"), str)
                    or type(item.get("recognized")) is not bool
                    or not isinstance(item.get("properties"), list)):
                raise MappingError("invalid_analysis")
            names.add(item["name"])
            for prop in item["properties"]:
                if (not isinstance(prop, dict) or not isinstance(prop.get("name"), str)
                        or not isinstance(prop.get("values"), list)
                        or not all(isinstance(v, dict) for v in prop["values"])):
                    raise MappingError("invalid_analysis")
            children = item.get("children", [])
            if not isinstance(children, list):
                raise MappingError("invalid_analysis")
            for child in children:
                if not isinstance(child, dict) or not isinstance(child.get("objects"), list) or not all(
                        isinstance(o, dict) and isinstance(o.get("name"), str) for o in child["objects"]):
                    raise MappingError("invalid_analysis")


def _axis(status, *reasons, refs=()):
    return {"status": status, "reasons": list(reasons), "contract_refs": list(refs),
            "verification": "not_run"}


def _property_text(fact, name):
    for prop in fact["properties"]:
        if prop["name"] == name:
            return "".join(v.get("text", "") for v in prop["values"]
                           if isinstance(v.get("text", ""), str))
    return ""


def _metadata(fact, analysis):
    """Keep the source metadata associated with its widget; infer no semantics."""
    props = fact["properties"]
    accessible = [p for p in props if p["name"] in
                  ("accessibleName", "accessibleDescription", "accessible-name",
                   "accessible-description", "accessibility")]
    # GTK3's AtkObject lives in an internal-child rather than a direct property.
    accessible_ids = set()
    for child in fact.get("children", []):
        if child.get("attributes", {}).get("internal-child") == "accessible":
            accessible_ids.update(o.get("name") for o in child.get("objects", []))
    accessible += [w for w in analysis["widgets"] if w["name"] in accessible_ids]
    text_names = {"text", "label", "title", "windowTitle", "placeholderText",
                  "placeholder-text", "toolTip", "tooltip-text", "accessibleName",
                  "accessibleDescription", "accessible-name", "accessible-description",
                  "accessibility"}
    text = [p for p in props if p["name"] in text_names]
    return accessible, text


def _select_rule(fact, kind, rules, source_format):
    rule = rules[source_format][kind].get(fact["class"])
    if not fact["recognized"]:
        return None
    if rule is not None:
        rule = copy.deepcopy(rule)
        if fact["class"] == "GtkBox":
            orientation = _property_text(fact, "orientation").strip().lower()
            if orientation not in ("horizontal", "vertical"):
                return None  # Never invent the toolkit's implicit default.
            rule["targets"] = ["LayoutKind::" + ("Row" if orientation == "horizontal" else "Column")]
    return rule


def _control_row(fact, kind, index, analysis, rules, contract):
    rule = _select_rule(fact, kind, rules, analysis["format"])
    valid = contract["state"] == "matched"
    if rule is None:
        relation, mapping_status, targets, refs = "unknown", "needs_decision", [], []
        reason = "source_semantics_need_review"
    elif rule["relation"] == "unavailable":
        relation, mapping_status, targets, refs = "unavailable", "unsupported", [], []
        reason = rule["reason"]
        if not valid:
            mapping_status, reason = "needs_decision", "contract_evidence_changed_or_missing"
    else:
        relation, targets, refs = rule["relation"], rule["targets"], rule["refs"]
        mapping_status = "supported" if relation == "direct" else "partial"
        reason = rule["reason"]
        if not valid:
            mapping_status, reason = "needs_decision", "contract_evidence_changed_or_missing"

    accessible, text = _metadata(fact, analysis)
    focus_props = [p for p in fact["properties"] if p["name"] in
                   ("focusPolicy", "focusable", "can-focus", "has-focus", "focus-on-click")]
    focus_order = [name for name in analysis["tab_order"] if name == fact["name"]]
    models = rule.get("models", []) if rule and valid else []
    mapped = bool(targets) and valid
    appearance = _axis("partial", reason, "geometry_style_and_rendering_unverified", refs=refs)
    if not mapped:
        appearance = _axis(mapping_status, reason, refs=refs)
    axes = {
        "appearance_layout": appearance,
        "pointer_keyboard": _axis("partial" if models else "needs_decision",
                                  "model_only_input_adapter_missing" if models else
                                  "source_interaction_semantics_need_review", refs=models),
        "focus": _axis("partial" if mapped and (focus_props or focus_order) else "needs_decision",
                       "declared_focus_not_validated" if focus_props or focus_order else
                       "focus_information_missing", refs=["FocusManager", "FocusScope"] if mapped else []),
        "accessibility": _axis("partial" if mapped and accessible else "needs_decision",
                               "metadata_needs_stable_keys_roles_and_relations" if accessible else
                               "accessible_metadata_missing", refs=["AccessibleNode"] if mapped else []),
        "localization": _axis("partial" if mapped and text else "needs_decision",
                              "source_text_requires_message_keys_catalogs_and_measurement" if text else
                              "text_and_catalog_evidence_missing",
                              refs=["MessageKey", "ResolvedText", "TextConstraints"] if mapped else []),
        "backend": _axis("unsupported", "renderer_and_service_adapters_not_provided"),
    }
    if not valid:
        for axis in AXES[:-1]:
            axes[axis] = _axis("needs_decision", "contract_evidence_changed_or_missing")
    return {
        "id": "{}:{}".format(kind, index), "source_kind": kind, "source": copy.deepcopy(fact),
        "mapping": {"status": mapping_status, "relation": relation,
                    "candidates": targets, "contract_refs": refs, "reason": reason,
                    "approval": "PROVISIONAL"},
        "status": mapping_status if mapping_status in ("unsupported", "needs_decision") else "partial",
        "axes": axes,
        "source_metadata": {"focus_properties": copy.deepcopy(focus_props),
                            "in_declared_tab_order": bool(focus_order),
                            "accessibility": copy.deepcopy(accessible), "text": copy.deepcopy(text)},
    }


def _gap_row(kind, index, fact):
    reasons = {
        "connections": "declared_event_requires_typed_app_adapter",
        "bindings": "binding_requires_explicit_state_adapter",
        "resources": "resource_or_library_not_loaded",
        "unknown": "source_semantics_need_review",
        "custom_widgets": "custom_widget_requires_owner_review",
        "unresolved_references": "source_reference_unresolved",
        "declared_slots": "declared_slot_has_no_implementation_evidence",
    }
    unsupported = kind in ("connections", "bindings", "resources", "declared_slots")
    status = "unsupported" if unsupported else "needs_decision"
    axes = {a: _axis("needs_decision", "source_semantics_need_review") for a in AXES}
    axes["backend"] = _axis(status, reasons[kind])
    return {"id": "{}:{}".format(kind, index), "source_kind": kind,
            "source": copy.deepcopy(fact), "status": status,
            "mapping": {"status": status, "relation": "unavailable" if unsupported else "unknown",
                        "candidates": [], "contract_refs": [], "reason": reasons[kind],
                        "approval": "PROVISIONAL"}, "axes": axes}


def map_analysis(analysis, contract=None, rules=None):
    """Accept NDP-06 v1 facts, return an independent JSON-compatible gap report.

    Optional contract/rules are trusted local adapter data, never UI expressions.
    The complete analysis is retained so unsupported properties cannot disappear.
    """
    _validate(analysis)
    contract = copy.deepcopy(inspect_contract() if contract is None else contract)
    rules = json.loads((HERE / "rules.json").read_text(encoding="utf-8")) if rules is None else rules
    rows = []
    for kind in ("widgets", "layouts"):
        rows.extend(_control_row(f, kind, i, analysis, rules, contract)
                    for i, f in enumerate(analysis[kind]))
    for kind in COLLECTIONS[2:] + ("declared_slots",):
        rows.extend(_gap_row(kind, i, f) for i, f in enumerate(analysis[kind]))
    counts = {s: sum(r["status"] == s for r in rows) for s in STATUSES}
    axis_counts = {a: {s: sum(r["axes"][a]["status"] == s for r in rows) for s in STATUSES}
                   for a in AXES}
    return {
        "mapping_version": 1, "analysis_version": analysis["analysis_version"],
        "source_format": analysis["format"], "approval": "PROVISIONAL",
        "contract": contract, "source_analysis": copy.deepcopy(analysis), "rows": rows,
        "summary": {"status_counts": counts, "axis_status_counts": axis_counts,
                    "port_complete": False, "target_acceptance": "not_run",
                    "renderer": "not_provided_by_mapping_or_nagi_ui_contract"},
        "limitations": [
            "Internal provisional mapping, not a public SDK support matrix or UI-owner approval.",
            "Supported describes contract vocabulary only, never rendered or executable compatibility.",
            "Unlisted properties, styles, placement and source logic require manual review.",
            "No renderer, device input, focus service, accessibility service, catalogs or app adapters run.",
        ],
    }


def json_report(report):
    return json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n"


def markdown_report(report):
    # JSON-escape control characters and HTML-escape source values in table cells.
    def cell(value):
        import html
        return html.escape(json.dumps(value, ensure_ascii=False), quote=True).replace("|", "&#124;").replace("`", "&#96;")

    lines = ["# Provisional Nagi UI mapping", "", "Approval: PROVISIONAL. Target acceptance: not_run.",
             "Supported = contract vocabulary only. Port complete: false.",
             "Contract evidence: " + report["contract"]["state"], "",
             "| Source | Candidates | Mapping | " + " | ".join(AXES) + " |",
             "|---|---|---|" + "---|" * len(AXES)]
    for row in report["rows"]:
        fact = row["source"]
        identity = fact.get("name", row["id"]) if isinstance(fact, dict) else fact
        label = cell([row["source_kind"], fact.get("class", "") if isinstance(fact, dict) else "", identity])
        cells = [label, cell(row["mapping"]["candidates"]),
                 row["mapping"]["status"] + " / " + row["mapping"]["relation"] + ": " + row["mapping"]["reason"]]
        cells += [row["axes"][a]["status"] + ": " + ", ".join(row["axes"][a]["reasons"]) for a in AXES]
        lines.append("| " + " | ".join(cells) + " |")
    lines += ["", "## Counts", "", cell(report["summary"]["status_counts"]), "",
              "## Evidence", ""]
    lines += ["- " + cell(e["path"]) + ": " + e["state"] + " (SHA-256 " + e["sha256"] + ")"
              for e in report["contract"]["evidence"]]
    lines += ["", "## Limits", ""] + ["- " + item for item in report["limitations"]]
    lines += ["", "Full source facts (including text, placement, events, bindings and unknowns) are retained in the JSON report."]
    return "\n".join(lines) + "\n"


def _unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise MappingError("duplicate_json_key")
        value[key] = item
    return value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    args = parser.parse_args(argv)
    try:
        data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
        if len(data) > MAX_INPUT_BYTES:
            raise MappingError("input_too_large")
        analysis = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object)
        report = map_analysis(analysis)
    except MappingError as error:
        print(json.dumps({"error": error.code}), file=sys.stderr)
        return 2
    except (UnicodeError, ValueError, RecursionError):
        print(json.dumps({"error": "invalid_analysis_json"}), file=sys.stderr)
        return 2
    sys.stdout.write(markdown_report(report) if args.format == "markdown" else json_report(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
