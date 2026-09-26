#!/usr/bin/env python3
"""Validate SDK-owned manifest examples and representative rejected documents."""

import copy
import json
from pathlib import Path

from jsonschema import Draft202012Validator


ROOT = Path(__file__).resolve().parents[3]
SCHEMA_PATH = ROOT / "sdk/rust/schemas/app-manifest.schema.json"
FIXTURES = ROOT / "sdk/rust/fixtures/manifests"


def main() -> None:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    documents = [json.loads(path.read_text(encoding="utf-8")) for path in sorted(FIXTURES.glob("*.json"))]
    if len(documents) < 3:
        raise SystemExit("FAIL expected at least three manifest fixtures")
    for path, document in zip(sorted(FIXTURES.glob("*.json")), documents):
        errors = list(validator.iter_errors(document))
        if errors:
            details = "; ".join(error.message for error in errors)
            raise SystemExit(f"FAIL {path.name}: {details}")
    optional_services = copy.deepcopy(documents[-1])
    optional_services.pop("backgroundServices", None)
    if list(validator.iter_errors(optional_services)):
        raise SystemExit("FAIL schema requires optional background service metadata")

    invalid_documents = []
    unsafe_entrypoint = copy.deepcopy(documents[0])
    unsafe_entrypoint["entrypoint"]["target"] = "../outside.napp"
    invalid_documents.append(unsafe_entrypoint)
    out_of_range_sdk = copy.deepcopy(documents[0])
    out_of_range_sdk["sdkContractVersion"]["major"] = 65536
    invalid_documents.append(out_of_range_sdk)
    unnamespaced_extension = copy.deepcopy(documents[0])
    unnamespaced_extension["extensions"] = {"vendor": True}
    invalid_documents.append(unnamespaced_extension)
    malformed_locale = copy.deepcopy(documents[0])
    malformed_locale["supportedLocales"] = ["en-US", "JA-jp"]
    invalid_documents.append(malformed_locale)
    malformed_version = copy.deepcopy(documents[0])
    malformed_version["version"] = "1.0.0-01"
    invalid_documents.append(malformed_version)
    missing_japanese_name = copy.deepcopy(documents[0])
    del missing_japanese_name["displayName"]["ja-JP"]
    invalid_documents.append(missing_japanese_name)
    unsafe_resource = copy.deepcopy(documents[0])
    unsafe_resource["icon"] = "appres://icons/../private.svg"
    invalid_documents.append(unsafe_resource)
    for uri in (
        "appres://icons/a b.svg",
        "appres://icons/a\u0000b.svg",
        "appres://icons/日本語.svg",
        "appres://icons//notes.svg",
    ):
        invalid_resource = copy.deepcopy(documents[0])
        invalid_resource["icon"] = uri
        invalid_documents.append(invalid_resource)
    for target in ("bin/a b.napp", "bin/日本語.napp", "bin//notes.napp"):
        invalid_entrypoint = copy.deepcopy(documents[0])
        invalid_entrypoint["entrypoint"]["target"] = target
        invalid_documents.append(invalid_entrypoint)
    if any(not list(validator.iter_errors(document)) for document in invalid_documents):
        raise SystemExit("FAIL schema accepted a malformed manifest")
    semantic_state = copy.deepcopy(documents[0])
    semantic_state["stateCompatibility"]["minimumReadableVersion"] = (
        semantic_state["stateCompatibility"]["currentVersion"] + 1
    )
    if list(validator.iter_errors(semantic_state)):
        raise SystemExit("FAIL structural schema unexpectedly handles a relational state rule")
    semantic_duplicate = copy.deepcopy(documents[0])
    semantic_duplicate["intents"].append(copy.deepcopy(semantic_duplicate["intents"][0]))
    if list(validator.iter_errors(semantic_duplicate)):
        raise SystemExit("FAIL structural schema unexpectedly handles semantic ID uniqueness")
    print(
        f"PASS Draft 2020-12 schema: {len(documents)} valid fixture(s), "
        "optional services accepted, "
        f"{len(invalid_documents)} malformed document(s) rejected; "
        "duplicate-ID and state-version rules delegated to semantic validation"
    )


if __name__ == "__main__":
    main()
