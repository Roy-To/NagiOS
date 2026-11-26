#!/usr/bin/env python3
"""Fail-closed validation helpers for the Task24 release PID 1 gate."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any, Mapping, Sequence


EXPECTED_MARKERS = (
    "nagi.dev_ws10",
    "NAGI_DEV_WS10",
    "NAGI_A_INPUT_TO_PRESENT",
    "NAGI_A_GUEST_MEM",
    "SYSTEM_DATA_MOUNT",
)
EXPECTED_MARKER_COUNT = 5
VALID_PROFILES = frozenset({"debug", "release"})
NON_LINKED_TARGET_KINDS = frozenset({"custom-build", "proc-macro"})
ISOLATED_TARGET_PREFIX = "task24-release-pid1-"
RUSTFLAGS_ENV_NAME = re.compile(
    r"(?:^RUSTFLAGS$|^CARGO_(?:BUILD|ENCODED)_RUSTFLAGS$|^CARGO_TARGET_.+_RUSTFLAGS$)"
)


class GateError(ValueError):
    """A release gate input failed validation."""


def validate_profile(profile: Any) -> str:
    if not isinstance(profile, str) or profile not in VALID_PROFILES:
        raise GateError("profile must be exactly 'debug' or 'release'")
    return profile


def _is_rustflags_env_name(name: str) -> bool:
    return RUSTFLAGS_ENV_NAME.fullmatch(name) is not None


def _cargo_config_paths(repo_root: Path, env: Mapping[str, str]) -> list[Path]:
    root = repo_root.resolve()
    candidates: list[Path] = []
    for directory in (root, *root.parents):
        cargo_dir = directory / ".cargo"
        legacy_config = cargo_dir / "config"
        toml_config = cargo_dir / "config.toml"
        selected_config = (
            legacy_config if legacy_config.exists() or legacy_config.is_symlink() else toml_config
        )
        candidates.append(selected_config)

    cargo_home_text = env.get("CARGO_HOME")
    cargo_home = Path(cargo_home_text).expanduser() if cargo_home_text else Path.home() / ".cargo"
    if not cargo_home.is_absolute():
        cargo_home = root / cargo_home
    legacy_home_config = cargo_home / "config"
    toml_home_config = cargo_home / "config.toml"
    selected_home_config = (
        legacy_home_config
        if legacy_home_config.exists() or legacy_home_config.is_symlink()
        else toml_home_config
    )
    candidates.append(selected_home_config)

    unique: list[Path] = []
    seen: set[Path] = set()
    for candidate in candidates:
        resolved = candidate.resolve()
        if resolved not in seen:
            seen.add(resolved)
            unique.append(resolved)
    return unique


def _config_has_rustflags(document: Mapping[str, Any]) -> bool:
    if "include" in document:
        raise GateError("Cargo config includes are forbidden")

    build = document.get("build", {})
    if not isinstance(build, dict):
        raise GateError("Cargo config has a malformed [build] table")
    if "rustflags" in build:
        return True

    targets = document.get("target", {})
    if not isinstance(targets, dict):
        raise GateError("Cargo config has a malformed [target] table")
    for target_config in targets.values():
        if not isinstance(target_config, dict):
            raise GateError("Cargo config has a malformed target table")
        if "rustflags" in target_config:
            return True

    configured_env = document.get("env", {})
    if not isinstance(configured_env, dict):
        raise GateError("Cargo config has a malformed [env] table")
    return any(_is_rustflags_env_name(name) for name in configured_env)


def check_rustflags(repo_root: Path, env: Mapping[str, str] | None = None) -> dict[str, Any]:
    """Reject inherited/configured compiler flag overrides before invoking Cargo."""
    actual_env = os.environ if env is None else env
    environment_overrides = sorted(name for name in actual_env if _is_rustflags_env_name(name))
    if environment_overrides:
        raise GateError("Cargo/Rust flag override environment variables are forbidden: " + ", ".join(environment_overrides))

    checked_configs: list[str] = []
    for config_path in _cargo_config_paths(repo_root, actual_env):
        if not config_path.is_file():
            continue
        try:
            document = tomllib.loads(config_path.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise GateError("could not safely inspect a Cargo config file") from error
        if _config_has_rustflags(document):
            raise GateError("Cargo rustflags configuration is forbidden")
        checked_configs.append(str(config_path))

    return {
        "environment_override_count": 0,
        "cargo_config_override_count": 0,
        "cargo_config_files_checked": len(checked_configs),
    }


def _paths_overlap(first: Path, second: Path) -> bool:
    return first == second or first in second.parents or second in first.parents


def prepare_build_paths(output_dir: Path, target_root: Path) -> Path:
    """Create a brand-new output directory and a unique empty Cargo target child."""
    output = output_dir.expanduser().resolve()
    raw_target_root = target_root.expanduser()
    if not raw_target_root.is_absolute():
        raw_target_root = Path.cwd() / raw_target_root
    if raw_target_root.is_symlink():
        raise GateError("CARGO_TARGET_DIR must not be a symlink")
    target_base = raw_target_root.resolve()

    if os.path.lexists(output):
        raise GateError("output directory already exists; a fresh output path is required")
    if not output.parent.is_dir():
        raise GateError("output parent directory does not exist")
    if _paths_overlap(output, target_base):
        raise GateError("output directory and isolated Cargo target root must be separate")

    target_base.mkdir(parents=True, exist_ok=True)
    isolated_target = Path(tempfile.mkdtemp(prefix=ISOLATED_TARGET_PREFIX, dir=target_base))
    try:
        output.mkdir(mode=0o755)
    except BaseException:
        shutil.rmtree(isolated_target)
        raise
    if any(isolated_target.iterdir()) or any(output.iterdir()):
        shutil.rmtree(isolated_target)
        output.rmdir()
        raise GateError("new build directories were not empty")
    return isolated_target


def cleanup_isolated_target(target_root: Path, isolated_target: Path) -> None:
    """Remove only the exact uniquely named child created by prepare_build_paths."""
    root = target_root.expanduser()
    if not root.is_absolute():
        root = Path.cwd() / root
    if root.is_symlink():
        raise GateError("CARGO_TARGET_DIR must not be a symlink during cleanup")
    root = root.resolve()
    target = isolated_target.expanduser()
    if not target.is_absolute():
        target = Path.cwd() / target
    if target.is_symlink():
        raise GateError("refusing to remove a symlink as an isolated target")
    target = target.resolve()
    if target.parent != root or not target.name.startswith(ISOLATED_TARGET_PREFIX):
        raise GateError("refusing to remove a path not owned by this Task24 build")
    if target.exists():
        shutil.rmtree(target)


def validate_output_files(output_dir: Path, expected_names: Sequence[str]) -> None:
    output = output_dir.resolve(strict=True)
    if not output.is_dir():
        raise GateError("build output path is not a directory")
    actual = {path.name for path in output.iterdir()}
    expected = set(expected_names)
    if len(expected) != len(expected_names) or actual != expected:
        raise GateError("build output directory contains missing or unexpected files")
    if any(not (output / name).is_file() for name in expected):
        raise GateError("build output directory contains a non-file entry")


def _compiler_artifacts(reports: Sequence[Mapping[str, Any]]) -> list[Mapping[str, Any]]:
    return [report for report in reports if report.get("reason") == "compiler-artifact"]


def validate_artifacts(
    reports: Sequence[Mapping[str, Any]], profile: str, target_triple: str, isolated_target_dir: Path
) -> tuple[Mapping[str, Any], dict[str, Any], str]:
    """Validate all Cargo artifact metadata and return PID 1 plus an aggregate receipt."""
    selected_profile = validate_profile(profile)
    if not target_triple or Path(target_triple).name != target_triple:
        raise GateError("Cargo target triple must be a single non-empty path component")
    artifacts = _compiler_artifacts(reports)
    if not artifacts:
        raise GateError("Cargo emitted no compiler-artifact reports")

    binary_artifacts: list[Mapping[str, Any]] = []
    checked_linked_profiles: list[dict[str, Any]] = []
    excluded_artifact_count = 0
    host_only_artifact_count = 0
    isolated_target = isolated_target_dir.resolve(strict=True)
    for artifact in artifacts:
        target = artifact.get("target")
        profile_data = artifact.get("profile")
        if not isinstance(target, dict) or not isinstance(profile_data, dict):
            raise GateError("Cargo compiler-artifact report has malformed target/profile data")
        test_profile = profile_data.get("test")
        if type(test_profile) is not bool:
            raise GateError("every Cargo compiler-artifact must report boolean profile.test")
        if test_profile:
            raise GateError("Cargo emitted an artifact with the test profile enabled")
        fresh = artifact.get("fresh")
        if type(fresh) is not bool:
            raise GateError("every Cargo compiler-artifact must report boolean fresh metadata")
        if fresh:
            raise GateError("Cargo reused a fresh/stale compiler artifact in an isolated build")

        kinds = target.get("kind")
        target_name = target.get("name")
        if not isinstance(kinds, list) or not kinds or not all(isinstance(kind, str) for kind in kinds):
            raise GateError("Cargo compiler-artifact report has malformed target kinds")
        if not isinstance(target_name, str) or not target_name:
            raise GateError("Cargo compiler-artifact report has no target name")
        if target_name == "nagi-init" and kinds == ["bin"] and artifact.get("executable"):
            binary_artifacts.append(artifact)

        if NON_LINKED_TARGET_KINDS.intersection(kinds):
            excluded_artifact_count += 1
            continue

        filenames = artifact.get("filenames")
        if not isinstance(filenames, list) or not filenames or not all(isinstance(name, str) for name in filenames):
            raise GateError("linked Cargo compiler-artifact must report its output filenames")
        relative_filenames: list[Path] = []
        for filename in filenames:
            filename_path = Path(filename)
            if not filename_path.is_absolute():
                filename_path = Path.cwd() / filename_path
            try:
                relative_filenames.append(filename_path.resolve(strict=True).relative_to(isolated_target))
            except (OSError, ValueError) as error:
                raise GateError("Cargo compiler-artifact output is missing or outside the isolated target directory") from error
        target_specific = [relative.parts[0] == target_triple for relative in relative_filenames if relative.parts]
        if not target_specific or any(target_specific) and not all(target_specific):
            raise GateError("Cargo compiler-artifact mixes target-specific and host output paths")
        if not all(target_specific):
            host_only_artifact_count += 1
            continue

        opt_level = profile_data.get("opt_level")
        debug_assertions = profile_data.get("debug_assertions")
        if selected_profile == "release" and (opt_level != "3" or debug_assertions is not False):
            raise GateError(
                f"linked release artifact {target_name!r} must have opt_level='3' and debug_assertions=false"
            )
        checked_linked_profiles.append(
            {
                "target": target_name,
                "kinds": kinds,
                "opt_level": opt_level,
                "debug_assertions": debug_assertions,
            }
        )

    if len(binary_artifacts) != 1:
        raise GateError("expected exactly one linked Task24 PID 1 ELF")
    binary = binary_artifacts[0]
    executable_text = binary.get("executable")
    if not isinstance(executable_text, str) or not executable_text:
        raise GateError("Cargo did not provide the PID 1 executable path")
    executable_path = Path(executable_text)
    if not executable_path.is_absolute():
        executable_path = Path.cwd() / executable_path
    try:
        original_executable = executable_path.resolve(strict=True)
        isolated_target = isolated_target_dir.resolve(strict=True)
        relative_executable = original_executable.relative_to(isolated_target)
    except (OSError, ValueError) as error:
        raise GateError("PID 1 executable is missing or outside the isolated Cargo target directory") from error

    return (
        binary,
        {
            "compiler_artifact_count": len(artifacts),
            "linked_artifact_count": len(checked_linked_profiles),
            "excluded_build_script_or_proc_macro_count": excluded_artifact_count,
            "host_only_artifact_count": host_only_artifact_count,
            "all_compiler_artifacts_test_false": True,
            "test_profile_artifact_count": 0,
            "all_compiler_artifacts_fresh_false": True,
            "fresh_artifact_count": 0,
            "linked_artifact_profiles": checked_linked_profiles,
        },
        relative_executable.as_posix(),
    )


def validate_marker_counts(counts: Mapping[str, Any], profile: str) -> int:
    selected_profile = validate_profile(profile)
    if not isinstance(counts, Mapping) or set(counts) != set(EXPECTED_MARKERS) or len(counts) != EXPECTED_MARKER_COUNT:
        raise GateError("development marker counts must cover exactly the five expected markers")
    if any(type(count) is not int or count < 0 for count in counts.values()):
        raise GateError("development marker counts must be non-negative integers")
    if selected_profile == "release" and any(counts.values()):
        raise GateError("release ELF retained a Task24 development marker")
    if selected_profile == "debug" and not all(counts[marker] > 0 for marker in EXPECTED_MARKERS):
        raise GateError("acceptance ELF is missing a Task24 development marker")
    return sum(counts.values())


def _main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    rustflags_parser = subparsers.add_parser("check-rustflags")
    rustflags_parser.add_argument("--repo-root", type=Path, required=True)

    prepare_parser = subparsers.add_parser("prepare-build-paths")
    prepare_parser.add_argument("--output-dir", type=Path, required=True)
    prepare_parser.add_argument("--target-root", type=Path, required=True)

    cleanup_parser = subparsers.add_parser("cleanup-target")
    cleanup_parser.add_argument("--target-root", type=Path, required=True)
    cleanup_parser.add_argument("--isolated-target-dir", type=Path, required=True)

    output_parser = subparsers.add_parser("check-output")
    output_parser.add_argument("--output-dir", type=Path, required=True)
    output_parser.add_argument("--expected", nargs="*", default=[])

    args = parser.parse_args(argv)
    try:
        if args.command == "check-rustflags":
            result = check_rustflags(args.repo_root)
            print(json.dumps(result, sort_keys=True))
        elif args.command == "prepare-build-paths":
            isolated = prepare_build_paths(args.output_dir, args.target_root)
            print(isolated)
        elif args.command == "cleanup-target":
            cleanup_isolated_target(args.target_root, args.isolated_target_dir)
        elif args.command == "check-output":
            validate_output_files(args.output_dir, args.expected)
        else:
            parser.error("unknown command")
    except (GateError, OSError) as error:
        print(f"Task24 release gate: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
