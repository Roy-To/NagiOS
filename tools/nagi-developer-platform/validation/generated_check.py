#!/usr/bin/env python3
"""NDP-09c: connect the NDP-03 generator output to the Stage 1 Manifest check.

Thin host-only glue. It asks the existing generator (`generator/generate.py`,
read-only here) to write a Level 0 scaffold into a private temporary checkout
stand-in, then hands only the generated `manifest.json` to `validate.validate`
(Stage 1, the real `nagi-pkg manifest validate`). It never builds the generated
crate, runs Cargo, fetches anything, or feeds the scaffold to Stage 2.

The temporary checkout is removed by this module (the generator itself never
deletes). Results carry no absolute paths. Host results are not Nagi target
acceptance.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import tempfile

import validate

GENERATOR = validate.REPO / "tools/nagi-developer-platform/generator/generate.py"

# PROVISIONAL (see README): default inputs for the connected scaffold and the
# Manifest location the generator's contract produces. Change here to adapt.
DEFAULT_INPUTS = {
    "app_id": "com.example.level-zero",
    "package_name": "level-zero",
    "display_name": {"en-US": "Level Zero", "ja-JP": "レベルゼロ"},
    "output_dir": "samples/level-zero",
}
MANIFEST_NAME = "manifest.json"


def load_generator():
    """Load generate.py by path under a private name (it is not a package)."""
    spec = importlib.util.spec_from_file_location("_ndp09c_generate", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check_generated(inputs=None, timeout=10, cancel=None, generator=None):
    """Generate a scaffold in a temp checkout stand-in and run Stage 1 on its Manifest.

    Returns the Stage 1 result extended with `generator` (`created`/`failed`)
    and `generator_reason`. When generation fails, Stage 1 is not run and the
    status is `failed`; `not_run` is never reported as success.
    """
    inputs = dict(DEFAULT_INPUTS if inputs is None else inputs)
    base = {"stage": 1, "check": "generated_manifest", "scope": "host_metadata_only",
            "status": "failed", "exit_code": None, "built": False}
    try:
        generator = load_generator() if generator is None else generator
    except (OSError, SyntaxError, ImportError):
        return {**base, "generator": "unavailable", "reason": "generator_unavailable"}
    output_dir = inputs.get("output_dir")
    with tempfile.TemporaryDirectory(prefix="nagi-ndp-gen-stage1-") as directory:
        root = Path(directory)
        # Stand-in checkout: the generator only checks that the SDK Cargo.toml exists.
        (root / "sdk/rust").mkdir(parents=True)
        (root / "sdk/rust/Cargo.toml").write_text("# stand-in\n", encoding="utf-8")
        (root / "samples").mkdir()
        try:
            generator.generate(inputs, root)
        except generator.GenerationError as error:
            return {**base, "generator": "failed", "reason": "generation_failed",
                    "generator_reason": error.code}
        except OSError:
            return {**base, "generator": "failed", "reason": "generation_failed",
                    "generator_reason": "io_error"}
        result = validate.validate(root, "{}/{}".format(output_dir, MANIFEST_NAME),
                                   stage=1, timeout=timeout, cancel=cancel)
    return {**result, "check": "generated_manifest", "built": False,
            "generator": "created", "generator_reason": None}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=10)
    args = parser.parse_args(argv)
    try:
        result = check_generated(timeout=args.timeout)
    except KeyboardInterrupt:
        result = {"status": "cancelled", "reason": "cancelled"}
    print(json.dumps(result, sort_keys=True, ensure_ascii=False))
    return {"passed": 0, "unavailable": 2, "not_run": 3, "timeout": 124,
            "cancelled": 130}.get(result["status"], 1)


if __name__ == "__main__":
    sys.exit(main())
