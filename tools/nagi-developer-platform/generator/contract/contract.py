"""Executable, read-only preview of the NDP-02 data contract (Python 3.9).

No output writer, shell execution, dependency fetch or SDK validator lives here.
NDP-03 must implement level0.json's writePolicy when persisting this preview.
"""

import json
import posixpath
import re
from pathlib import Path, PurePosixPath
from string import Template


CONTRACT_DIR = Path(__file__).resolve().parent
REPO_ROOT = CONTRACT_DIR.parents[3]


def load_contract():
    return json.loads((CONTRACT_DIR / "level0.json").read_text(encoding="utf-8"))


def _relative_directory(value):
    if (not isinstance(value, str) or not value or "\\" in value
            or any(ord(char) < 32 or ord(char) == 127 for char in value)
            or ":" in value or value.startswith("/")):
        raise ValueError("invalid_repo_relative_directory")
    if any(part in ("", ".", "..") for part in value.split("/")):
        raise ValueError("invalid_repo_relative_directory")
    return PurePosixPath(value)


def sdk_dependency(reference, output_dir):
    """Single adapter for SDK delivery. Paths are lexical repo-relative paths.

    Replacing the delivery scheme changes this function/reference data only;
    templates and the manifest do not embed an SDK location.
    """
    if reference["adapter"] != "repo-path":
        raise ValueError("unsupported_sdk_reference_adapter")
    sdk_path = _relative_directory(reference["repoPath"])
    destination = _relative_directory(output_dir)
    relative = posixpath.relpath(str(sdk_path), str(destination))
    # JSON quoting is valid TOML basic-string quoting for these bounded paths.
    return '{} = {{ path = {} }}'.format(reference["package"], json.dumps(relative))


def _pointer(document, pointer):
    for key in pointer.lstrip("/").split("/"):
        document = document[key.replace("~1", "/").replace("~0", "~")]
    return document


def normalize_inputs(inputs, contract=None, repo_root=REPO_ROOT):
    """Validate only the narrow template input contract, not app manifests."""
    contract = load_contract() if contract is None else contract
    if not isinstance(inputs, dict) or set(inputs) - set(contract["inputs"]):
        raise ValueError("unknown_template_input")
    schema = json.loads((Path(repo_root) / contract["sources"]["manifestSchema"])
                        .read_text(encoding="utf-8"))
    result = {}
    for key, rule in contract["inputs"].items():
        if key not in inputs and rule.get("required"):
            raise ValueError("missing_template_input:" + key)
        value = inputs.get(key, rule.get("default"))
        if rule["type"] == "repo-relative-directory":
            _relative_directory(value)
        elif rule["type"] == "localized-name":
            if (not isinstance(value, dict)
                    or set(value) - set(rule["locales"])
                    or not set(rule["requiredLocales"]).issubset(value)
                    or any(not isinstance(name, str) or not name.strip()
                           or len(name) > rule["maxLength"]
                           or any(ord(char) < 32 or ord(char) == 127 for char in name)
                           or any(0xD800 <= ord(char) <= 0xDFFF for char in name)
                           for name in value.values())):
                raise ValueError("invalid_display_name")
            value = dict(value)
        else:
            if not isinstance(value, str):
                raise ValueError("invalid_template_input:" + key)
            pattern = rule.get("pattern")
            if "patternSource" in rule:
                pattern = _pointer(schema, rule["patternSource"])
            if ((pattern is not None and re.fullmatch(pattern, value) is None)
                    or ("maxLength" in rule and len(value) > rule["maxLength"])
                    or value in rule.get("reserved", [])
                    or ("enum" in rule and value not in rule["enum"])):
                raise ValueError("invalid_template_input:" + key)
            if (rule.get("equalsSdkPackageVersion")
                    and value != contract["sdkReference"]["packageVersion"]):
                raise ValueError("unsupported_sdk_package_version")
        result[key] = value
    return result


def _substitute(value, inputs):
    if isinstance(value, dict):
        if set(value) == {"$input"}:
            return inputs[value["$input"]]
        return {key: _substitute(item, inputs) for key, item in value.items()}
    if isinstance(value, list):
        return [_substitute(item, inputs) for item in value]
    if isinstance(value, str):
        return Template(value).substitute(inputs)
    return value


def preview(inputs, contract=None, repo_root=REPO_ROOT):
    """Return {relative output path: UTF-8 text}; write nothing anywhere.

    Destination existence, symlinks, races and exclusive creation belong to
    the NDP-03 writer, not this lexical/in-memory contract preview.
    """
    contract = load_contract() if contract is None else contract
    values = normalize_inputs(inputs, contract, repo_root)
    values["sdk_dependency"] = sdk_dependency(contract["sdkReference"], values["output_dir"])
    result = {}
    for output in contract["outputs"]:
        if "data" in output:
            text = json.dumps(_substitute(contract[output["data"]], values),
                              ensure_ascii=False, indent=2) + "\n"
        else:
            template = (CONTRACT_DIR / output["template"]).read_text(encoding="utf-8")
            text = Template(template).substitute(values)
        result[output["path"]] = text
    return result
