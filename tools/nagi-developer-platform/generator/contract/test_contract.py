"""Source-backed contract checks, with no installs, builds or output writes."""

import copy
import json
import re
import unittest
from pathlib import PurePosixPath
from unittest.mock import patch

from contract import CONTRACT_DIR, REPO_ROOT, load_contract, preview, sdk_dependency


def canonical_errors(value, rule, schema):
    """Test-only structural walker over the live schema's exercised subset.

    This is not the typed semantic validator. Unknown exercised keywords fail
    so schema evolution cannot silently weaken these assertions.
    """
    known = {"$ref", "description", "type", "properties", "required",
             "additionalProperties", "const", "enum", "pattern", "not",
             "minimum", "maximum", "minLength", "maxLength", "minItems",
             "uniqueItems", "items", "contains", "allOf", "if", "then"}
    errors = ["unknown keyword: " + key for key in set(rule) - known]
    if "$ref" in rule:
        target = schema
        for key in rule["$ref"].removeprefix("#/").split("/"):
            target = target[key]
        return errors + canonical_errors(value, target, schema)
    types = {"object": dict, "array": list, "string": str, "integer": int}
    if "type" in rule and type(value) is not types[rule["type"]]:
        return errors + ["type"]
    if "const" in rule and value != rule["const"]:
        errors.append("const")
    if "enum" in rule and value not in rule["enum"]:
        errors.append("enum")
    if isinstance(value, dict):
        errors += ["required:" + key for key in rule.get("required", []) if key not in value]
        props = rule.get("properties", {})
        if rule.get("additionalProperties") is False:
            errors += ["extra:" + key for key in set(value) - set(props)]
        for key in set(value) & set(props):
            errors += canonical_errors(value[key], props[key], schema)
    if isinstance(value, list):
        if len(value) < rule.get("minItems", 0):
            errors.append("minItems")
        if rule.get("uniqueItems") and len({json.dumps(item, sort_keys=True) for item in value}) != len(value):
            errors.append("uniqueItems")
        if "contains" in rule and not any(not canonical_errors(item, rule["contains"], schema) for item in value):
            errors.append("contains")
        for item in value:
            if "items" in rule:
                errors += canonical_errors(item, rule["items"], schema)
    if isinstance(value, str):
        if "pattern" in rule and re.search(rule["pattern"], value) is None:
            errors.append("pattern")
        if len(value) < rule.get("minLength", 0) or len(value) > rule.get("maxLength", float("inf")):
            errors.append("length")
    if type(value) is int and (value < rule.get("minimum", -float("inf")) or value > rule.get("maximum", float("inf"))):
        errors.append("range")
    if "not" in rule and not canonical_errors(value, rule["not"], schema):
        errors.append("not")
    for child in rule.get("allOf", []):
        errors += canonical_errors(value, child, schema)
    if "if" in rule and not canonical_errors(value, rule["if"], schema):
        errors += canonical_errors(value, rule.get("then", {}), schema)
    return errors


class Level0ContractTests(unittest.TestCase):
    def setUp(self):
        self.contract = load_contract()
        self.inputs = {
            "app_id": "com.example.level-zero",
            "package_name": "level-zero",
            "display_name": {"en-US": "Level Zero", "ja-JP": "レベルゼロ"},
            "output_dir": "samples/level-zero",
        }
        self.schema = json.loads((REPO_ROOT / self.contract["sources"]["manifestSchema"])
                                 .read_text(encoding="utf-8"))

    def structural_errors(self, manifest):
        # Top-level schema annotations do not constrain an instance.
        root = {key: value for key, value in self.schema.items()
                if key not in ("$schema", "$id", "title", "description", "$defs")}
        return canonical_errors(manifest, root, self.schema)

    def test_manifest_uses_exactly_live_required_fields_recursively(self):
        manifest = json.loads(preview(self.inputs)["manifest.json"])
        self.assertEqual(set(manifest), set(self.schema["required"]))
        for field, definition in (("sdkContractVersion", "contractVersion"),
                                  ("entrypoint", "entrypoint"),
                                  ("stateCompatibility", "stateCompatibility")):
            self.assertEqual(set(manifest[field]), set(self.schema["$defs"][definition]["required"]))
        self.assertEqual(self.structural_errors(manifest), [])
        self.assertEqual(manifest["origin"], "third-party")
        self.assertEqual(manifest["resources"], [])
        self.assertEqual(manifest["intents"], [])
        self.assertEqual(manifest["requestedCapabilities"], [])
        self.assertEqual(manifest["displayName"], self.inputs["display_name"])
        self.assertEqual(manifest["stateCompatibility"]["currentVersion"],
                         manifest["stateCompatibility"]["minimumReadableVersion"])

    def test_every_required_field_is_actually_checked(self):
        manifest = json.loads(preview(self.inputs)["manifest.json"])
        for field in self.schema["required"]:
            with self.subTest(field=field):
                broken = dict(manifest)
                del broken[field]
                self.assertTrue(self.structural_errors(broken))

    def test_live_schema_rejects_unsafe_or_fabricated_metadata(self):
        manifest = json.loads(preview(self.inputs)["manifest.json"])
        cases = [("schemaVersion", 99), ("id", "../fake"),
                 ("origin", "owner"), ("entrypoint", {"kind": "native", "target": "../escape"}),
                 ("displayName", {"en-US": "Name"}), ("inventedPermission", True)]
        for key, value in cases:
            with self.subTest(key=key):
                self.assertTrue(self.structural_errors(dict(manifest, **{key: value})))

    def test_manifest_versions_match_sdk_definitions_and_loader(self):
        reference = self.contract["sdkReference"]
        module = (REPO_ROOT / reference["repoPath"] / "src/app_contract/mod.rs").read_text()
        schema_version = re.search(r"pub const APP_MANIFEST_SCHEMA_VERSION: u16 = (\d+);", module)
        sdk_version = re.search(r"pub const SDK_CONTRACT_VERSION: ContractVersion = ContractVersion \{ major: (\d+), minor: (\d+) \};", module)
        self.assertIsNotNone(schema_version)
        self.assertIsNotNone(sdk_version)
        self.assertEqual(self.contract["manifest"]["schemaVersion"], int(schema_version[1]))
        self.assertEqual(self.contract["manifest"]["sdkContractVersion"],
                         {"major": int(sdk_version[1]), "minor": int(sdk_version[2])})
        loader = (REPO_ROOT / self.contract["sources"]["manifestValidator"]).read_text()
        root_fields = re.search(r'ensure_fields\(\s*root,\s*"manifest",\s*&\[(.*?)\],\s*&\[(.*?)\]', loader, re.S)
        self.assertIsNotNone(root_fields)
        self.assertEqual(set(re.findall(r'"(\w+)"', root_fields[2])), set(self.schema["required"]))
        self.assertIn("AppIdentity::new(identifier, app_version, origin, publisher_id)", loader)
        self.assertIn("is_compatible_with(SDK_CONTRACT_VERSION)", loader)

    def test_source_imports_and_call_exist_in_public_sdk(self):
        source = preview(self.inputs)["src/lib.rs"]
        imports = re.search(r"use nagi_sdk::app_contract::\{([^}]+)\};", source)
        self.assertIsNotNone(imports)
        symbols = self.contract["sdkSymbols"]
        self.assertEqual({name.strip() for name in imports[1].split(",")},
                         {item["name"] for item in symbols})
        sdk = REPO_ROOT / self.contract["sdkReference"]["repoPath"] / "src"
        self.assertIn("pub mod app_contract;", (sdk / "lib.rs").read_text())
        module = (sdk / "app_contract/mod.rs").read_text()
        for symbol in symbols:
            with self.subTest(symbol=symbol["name"]):
                declaration = (sdk / "app_contract" / (symbol["module"] + ".rs")).read_text()
                self.assertIn(symbol["declaration"], declaration)
                export = re.search(r"pub use " + symbol["module"] + r"::\{([^}]+)\};", module, re.S)
                self.assertIsNotNone(export)
                self.assertIn(symbol["name"], [name.strip() for name in export[1].split(",")])
        identity = (sdk / "app_contract/identity.rs").read_text()
        self.assertRegex(identity, r"pub enum AppOrigin\s*\{\s*FirstParty,\s*ThirdParty,")
        self.assertRegex(identity, r"pub fn new\(\s*identifier: &'a str,\s*version: &'a str,\s*origin: AppOrigin,\s*publisher_id: Option<&'a str>,\s*\) -> Result<Self, AppError>")
        self.assertIn('AppIdentity::new("com.example.level-zero", "0.1.0", AppOrigin::ThirdParty, None)', source)
        self.assertNotIn("HELLO_", source)

    def test_dependency_matches_real_sample_and_sdk_package(self):
        reference = self.contract["sdkReference"]
        cargo = (REPO_ROOT / reference["repoPath"] / "Cargo.toml").read_text()
        self.assertRegex(cargo, r'(?m)^name = "' + re.escape(reference["package"]) + '"$')
        self.assertRegex(cargo, r'(?m)^version = "' + re.escape(reference["packageVersion"]) + '"$')
        self.assertRegex(cargo, r"(?m)^publish = false$")
        sample_path = REPO_ROOT / self.contract["sources"]["sampleCargo"]
        sample = sample_path.read_text()
        dependency = sdk_dependency(reference, self.inputs["output_dir"])
        self.assertIn(dependency, sample)
        relative = re.search(r'path = "([^"]+)"', dependency)[1]
        self.assertEqual((sample_path.parent / relative).resolve(),
                         (REPO_ROOT / reference["repoPath"]).resolve())
        self.assertIn('nagi-model = { path = "../../crates/nagi-model" }', cargo)
        self.assertTrue((REPO_ROOT / reference["repoPath"] / "../../crates/nagi-model/Cargo.toml").is_file())

    def test_dependency_adapter_handles_depth_and_single_location_replacement(self):
        ref = self.contract["sdkReference"]
        for destination in ("app", "samples/app", "scratch/nested/app"):
            with self.subTest(destination=destination):
                dependency = sdk_dependency(ref, destination)
                relative = json.loads(re.search(r"path = (.+) \}", dependency)[1])
                self.assertEqual((REPO_ROOT / destination / relative).resolve(),
                                 (REPO_ROOT / ref["repoPath"]).resolve())
        replacement = copy.deepcopy(self.contract)
        replacement["sdkReference"]["repoPath"] = "future/local-sdk"
        before = preview(self.inputs)
        after = preview(self.inputs, replacement)
        self.assertNotEqual(before["Cargo.toml"], after["Cargo.toml"])
        self.assertEqual({key: value for key, value in before.items() if key != "Cargo.toml"},
                         {key: value for key, value in after.items() if key != "Cargo.toml"})
        with self.assertRaises(ValueError):
            sdk_dependency(dict(ref, adapter="registry"), "app")

    def test_output_inventory_is_explicit_deterministic_and_has_no_side_effects(self):
        before = copy.deepcopy(self.inputs)
        # Read-only preview must not acquire writer responsibilities.
        with patch("pathlib.Path.write_text", side_effect=AssertionError("write")), \
                patch("pathlib.Path.write_bytes", side_effect=AssertionError("write")), \
                patch("pathlib.Path.mkdir", side_effect=AssertionError("mkdir")):
            first = preview(self.inputs)
        self.assertEqual(first, preview(self.inputs))
        self.assertEqual(self.inputs, before)
        self.assertEqual(set(first), {"Cargo.toml", "manifest.json", "src/lib.rs", "README.md", "UNSUPPORTED_APIS.md"})
        for path, content in first.items():
            self.assertFalse(PurePosixPath(path).is_absolute())
            self.assertNotIn("..", PurePosixPath(path).parts)
            self.assertNotIn("${", content)
            content.encode("utf-8")
            if path != "manifest.json":
                self.assertIn("Generated", content)
        cargo = first["Cargo.toml"]
        self.assertIn("[workspace]", cargo)
        self.assertIn("publish = false", cargo)
        self.assertNotIn("git =", cargo)
        self.assertNotIn("registry", cargo)
        self.assertIn("not Nagi target acceptance", first["README.md"])
        self.assertIn("No executable", first["README.md"])
        self.assertIn("never grants authority", first["UNSUPPORTED_APIS.md"])

    def test_json_encodes_display_names_as_data(self):
        inputs = copy.deepcopy(self.inputs)
        inputs["display_name"] = {"en-US": 'Name "quotes" ${app_id} \\', "ja-JP": "日本語😀"}
        manifest = json.loads(preview(inputs)["manifest.json"])
        self.assertEqual(manifest["displayName"], inputs["display_name"])
        self.assertEqual(self.structural_errors(manifest), [])

    def test_bounded_inputs_stay_below_live_loader_byte_limit(self):
        inputs = dict(self.inputs)
        inputs["display_name"] = {locale: "😀" * 256 for locale in ("en-US", "ja-JP")}
        inputs["app_id"] = "a." + "b" * 253
        inputs["app_version"] = "0.1.0+" + "b" * 250
        rendered = preview(inputs)["manifest.json"]
        loader = (REPO_ROOT / self.contract["sources"]["manifestValidator"]).read_text()
        limit = re.search(r"MAX_APP_MANIFEST_BYTES: u64 = ([\d_]+);", loader)
        self.assertIsNotNone(limit)
        self.assertLess(len(rendered.encode("utf-8")), int(limit[1].replace("_", "")))
        self.assertEqual(self.structural_errors(json.loads(rendered)), [])

    def test_invalid_inputs_are_rejected_before_preview(self):
        cases = [("app_id", "com.Example.app"), ("app_id", "a"),
                 ("app_id", 'com.test.bad"'), ("package_name", "../escape"),
                 ("package_name", "bad_name"), ("package_name", "x" * 65),
                 ("package_name", "self"), ("package_name", "crate"),
                 ("package_name", "async"), ("app_id", "a." + "b" * 254),
                 ("app_version", "01.0.0"), ("app_version", '0.1.0"'),
                 ("sdk_version", "99.0.0"), ("profile", "runtime"),
                 ("display_name", {"en-US": "Name"}),
                 ("display_name", {"en-US": "Name", "ja-JP": " "}),
                 ("display_name", {"en-US": "Name", "ja-JP": "\ud800"}),
                 ("display_name", {"en-US": "Name", "ja-JP": "あ" * 257}),
                 ("output_dir", "/absolute"), ("output_dir", "../escape"),
                 ("output_dir", "a/../b"), ("output_dir", "a//b"),
                 ("output_dir", "a\\b"), ("output_dir", "C:/app"),
                 ("output_dir", "."), ("output_dir", "a\ncommand"),
                 ("permissions", ["root"]), ("force", True)]
        for key, value in cases:
            with self.subTest(key=key, value=repr(value)):
                with self.assertRaises(ValueError):
                    preview(dict(self.inputs, **{key: value}))
        for key in self.inputs:
            with self.subTest(missing=key):
                inputs = dict(self.inputs)
                del inputs[key]
                with self.assertRaises(ValueError):
                    preview(inputs)

    def test_write_policy_and_provisional_boundaries_are_explicit(self):
        policy = self.contract["writePolicy"]
        self.assertFalse(policy["overwrite"])
        self.assertTrue(policy["exclusiveCreate"])
        self.assertEqual(policy["existingDestination"], "reject-even-if-empty")
        self.assertEqual(policy["symlinks"], "reject-in-destination-and-ancestors")
        self.assertEqual(self.contract["status"], "PROVISIONAL")
        self.assertEqual(self.contract["sdkReference"]["status"], "PROVISIONAL")
        self.assertFalse(self.contract["validation"]["automaticExecution"])
        self.assertEqual(self.contract["validation"]["entrypointArtifact"], "not-generated-or-verified")
        self.assertEqual(self.contract["validation"]["targetAcceptance"], "not-run")
        readme = (CONTRACT_DIR / "README.md").read_text(encoding="utf-8")
        self.assertIn("仮定（PROVISIONAL）", readme)
        self.assertIn("NDP-03", readme)


if __name__ == "__main__":
    unittest.main()
