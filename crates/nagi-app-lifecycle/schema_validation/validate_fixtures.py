#!/usr/bin/env python3
"""Validate the base manifest and APP-LC extension schemas and fixtures."""

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


ROOT = Path(__file__).resolve().parents[3]
BASE_SCHEMA_PATH = ROOT / "sdk/rust/schemas/app-manifest.schema.json"
EXTENSION_SCHEMA_PATH = ROOT / "crates/nagi-app-lifecycle/schemas/app-lifecycle-extension-v1.schema.json"
FIXTURES = ROOT / "crates/nagi-app-lifecycle/fixtures"
EXTENSION_ID = "org.nagi.app-lifecycle"


def main() -> None:
    base_schema = json.loads(BASE_SCHEMA_PATH.read_text(encoding="utf-8"))
    extension_schema = json.loads(EXTENSION_SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(base_schema)
    Draft202012Validator.check_schema(extension_schema)
    base_validator = Draft202012Validator(base_schema)
    extension_validator = Draft202012Validator(extension_schema)

    paths = sorted(FIXTURES.glob("*.json"))
    if len(paths) < 2:
        raise SystemExit("FAIL expected minimal and richer manifest fixtures")
    documents = [json.loads(path.read_text(encoding="utf-8")) for path in paths]
    for path, document in zip(paths, documents):
        errors = list(base_validator.iter_errors(document))
        if errors:
            raise SystemExit(f"FAIL base {path.name}: " + "; ".join(error.message for error in errors))
        errors = list(extension_validator.iter_errors(document["extensions"][EXTENSION_ID]))
        if errors:
            raise SystemExit(f"FAIL extension {path.name}: " + "; ".join(error.message for error in errors))

    rich_extension = copy.deepcopy(documents[-1]["extensions"][EXTENSION_ID])
    invalid_extensions = []
    unsupported_version = copy.deepcopy(rich_extension)
    unsupported_version["schemaVersion"] = 2
    invalid_extensions.append(unsupported_version)
    unknown_field = copy.deepcopy(rich_extension)
    unknown_field["silentlyIgnored"] = True
    invalid_extensions.append(unknown_field)
    malformed_capability = copy.deepcopy(rich_extension)
    malformed_capability["optionalCapabilities"] = ["../files"]
    invalid_extensions.append(malformed_capability)
    duplicate_capability = copy.deepcopy(rich_extension)
    duplicate_capability["optionalCapabilities"].append(
        duplicate_capability["optionalCapabilities"][0]
    )
    invalid_extensions.append(duplicate_capability)
    malformed_service_version = copy.deepcopy(rich_extension)
    malformed_service_version["requiredServices"][0]["contractVersion"]["major"] = 65536
    invalid_extensions.append(malformed_service_version)
    for index, extension in enumerate(invalid_extensions):
        if not list(extension_validator.iter_errors(extension)):
            raise SystemExit(f"FAIL extension schema accepted malformed case {index}")

    print(
        f"PASS Draft 2020-12 schemas: {len(documents)} manifest fixture(s) accepted; "
        f"{len(invalid_extensions)} malformed extension(s) rejected; "
        "cross-list contradictions and duplicate service IDs covered by Rust semantic tests"
    )


if __name__ == "__main__":
    main()
