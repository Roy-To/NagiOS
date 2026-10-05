#!/usr/bin/env python3
"""Validate serialized Wayback contracts and DF-01 state fixtures."""

from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

import jsonschema


def load_json(path: Path):
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: validate_schemas.py REPO_ROOT CONTRACT_FIXTURE_JSON")
    root = Path(sys.argv[1])
    fixtures = load_json(Path(sys.argv[2]))
    wayback_schema = load_json(root / "schemas/wayback/contracts-v1.schema.json")
    wayback_validator = jsonschema.Draft202012Validator(wayback_schema)
    jsonschema.Draft202012Validator.check_schema(wayback_schema)

    for contract_name in (
        "ledger",
        "activity_entry",
        "transaction",
        "snapshot_manifest",
        "restore_point",
        "restore_plan",
    ):
        value = fixtures[contract_name]
        wayback_validator.validate(value)
        unsupported = copy.deepcopy(value)
        unsupported["schema_version"] = 99
        try:
            wayback_validator.validate(unsupported)
        except jsonschema.ValidationError:
            pass
        else:
            raise AssertionError(f"{contract_name}: unsupported schema version was accepted")

    malformed_activity = copy.deepcopy(fixtures["activity_entry"])
    malformed_activity["id"] = ""
    try:
        wayback_validator.validate(malformed_activity)
    except jsonschema.ValidationError:
        pass
    else:
        raise AssertionError("activity_entry: empty stable ID was accepted")

    sensitive_payload = copy.deepcopy(fixtures["activity_entry"])
    sensitive_payload["metadata"]["clipboard.text"] = {
        "sensitivity": "secret",
        "disposition": "recorded",
        "value": {"type": "text", "value": "must not be stored"},
    }
    try:
        wayback_validator.validate(sensitive_payload)
    except jsonschema.ValidationError:
        pass
    else:
        raise AssertionError("activity_entry: recorded secret metadata was accepted")

    registry_schema = load_json(root / ".dev/schemas/workstreams.schema.json")
    state_schema = load_json(root / ".dev/schemas/workstream-state.schema.json")
    jsonschema.Draft202012Validator.check_schema(registry_schema)
    jsonschema.Draft202012Validator.check_schema(state_schema)
    checker = jsonschema.FormatChecker()
    jsonschema.validate(
        load_json(root / ".dev/workstreams.json"),
        registry_schema,
        format_checker=checker,
    )
    jsonschema.validate(
        load_json(root / ".dev/workstreams/wayback-activity-ledger/state.json"),
        state_schema,
        format_checker=checker,
    )
    print("PASS Draft 2020-12 Wayback contracts (6 instances, unknown-version negatives)")
    print("PASS Wayback schema negatives (empty ID and unredacted secret metadata)")
    print("PASS Draft 2020-12 DF-01 workstream registry and state")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
