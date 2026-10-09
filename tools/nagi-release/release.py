#!/usr/bin/env python3
"""Deterministic, fail-closed assembly and integrity checks for Nagi M30.

This tool assembles already-built release inputs. It does not build Nagi or
turn artifact integrity into a claim that the reference guest acceptance has
passed. See README.md for the input contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any, Iterable

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 is still supported by nagi.toml.
    tomllib = None  # type: ignore[assignment]


IMAGE_NAME = "Nagi-OS-0.1-devpreview.qcow2"
IMAGE_BUILD_INFO_SUFFIX = ".build-info"
MANIFEST_NAME = "release-manifest.json"
BUILD_MANIFEST_NAME = "build-manifest.json"
SOURCE_REVISION_NAME = "source-revision.txt"
SUMS_NAME = "SHA256SUMS"
SCHEMA_VERSION = 1
TRACKED_LICENSE_ROOT = Path("licenses/source-tree")
LICENSE_FILENAME_PATTERN = re.compile(
    r"^(?:LICENSE|LICENCE|COPYING|NOTICE)(?:$|[._-].*)",
    re.IGNORECASE,
)

DOC_INPUTS = (
    ("RELEASE_NOTES.md", "RELEASE_NOTES.md"),
    ("THIRD_PARTY_NOTICES.md", "licenses/THIRD_PARTY_NOTICES.md"),
    ("docs/architecture/README.md", "docs/architecture/README.md"),
    (
        "docs/architecture/decision-and-generative-ai-architecture.md",
        "docs/architecture/decision-and-generative-ai-architecture.md",
    ),
    ("docs/architecture/language-architecture.md", "docs/architecture/language-architecture.md"),
    (
        "docs/architecture/model-runtime-and-store-contract.md",
        "docs/architecture/model-runtime-and-store-contract.md",
    ),
    ("docs/architecture/search-and-workspace.md", "docs/architecture/search-and-workspace.md"),
    (
        "docs/architecture/unified-device-application-model.md",
        "docs/architecture/unified-device-application-model.md",
    ),
    ("sdk/README.md", "docs/sdk/README.md"),
    ("CONTRIBUTING.md", "CONTRIBUTING.md"),
    ("ROADMAP.md", "ROADMAP.md"),
)

# This digest is the upstream Granite Q4_K_M pin recorded by the M20
# workstream. It identifies the pinned model bytes; M20 states that those bytes
# are not bundled in the repository or guest image.
GRANITE_SHA256 = "e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5"
GRANITE_PIN_SOURCE = "docs/workstreams/NagiOS_M20_AI_Runtime_Granite_Workstream.md"
GRANITE_PIN_REPOSITORY_REVISION = "c40945d71cd90f249a56985e8155551a9188dc30"


class ReleaseError(Exception):
    """A missing, inconsistent, or unverifiable release input."""


def canonical_json_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n").encode(
        "utf-8"
    )


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _regular_file(path: Path, label: str) -> Path:
    if path.is_symlink():
        raise ReleaseError(f"{label} must not be a symlink: {path}")
    if not path.exists():
        raise ReleaseError(f"missing {label}: {path}")
    if not path.is_file():
        raise ReleaseError(f"{label} is not a regular file: {path}")
    if path.stat().st_size == 0:
        raise ReleaseError(f"{label} is empty: {path}")
    return path


def _within(root: Path, candidate: Path, label: str) -> Path:
    resolved = candidate.resolve(strict=True)
    try:
        resolved.relative_to(root.resolve(strict=True))
    except ValueError as error:
        raise ReleaseError(f"{label} must be inside the repository: {candidate}") from error
    return resolved


def image_build_provenance(root: Path, image_path: Path, expected_revision: str) -> dict[str, Any]:
    root = root.resolve(strict=True)
    image = _regular_file(image_path, "release qcow2 image")
    image = _within(root, image, "release qcow2 image")
    info_path = image.with_name(image.name + IMAGE_BUILD_INFO_SUFFIX)
    info_path = _regular_file(info_path, "release image build provenance")
    info_path = _within(root, info_path, "release image build provenance")
    try:
        lines = info_path.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeError) as error:
        raise ReleaseError(f"cannot read release image build provenance: {error}") from error

    fields: dict[str, str] = {}
    for line in lines:
        if line.count("=") != 1:
            raise ReleaseError("release image build provenance has a malformed field")
        key, value = line.split("=", 1)
        if key in fields:
            raise ReleaseError(f"release image build provenance repeats {key}")
        fields[key] = value
    required = {"format_version", "source_revision", "image_sha256"}
    version = fields.get("format_version")
    if version == "2":
        required |= {"profile", "init_features", "components", "model_store_sha256"}
    if set(fields) != required:
        raise ReleaseError("release image build provenance has unsupported fields")
    revision = fields["source_revision"]
    image_sha256 = fields["image_sha256"]
    if version not in {"1", "2"}:
        raise ReleaseError("unsupported release image build provenance version")
    if len(revision) not in {40, 64} or re.fullmatch(r"[0-9a-f]+", revision) is None:
        raise ReleaseError("release image provenance lacks a full source revision")
    if revision != expected_revision:
        raise ReleaseError("release image was built from a different source revision")
    if re.fullmatch(r"[0-9a-f]{64}", image_sha256) is None:
        raise ReleaseError("release image provenance lacks a valid image SHA-256")
    if sha256_file(image) != image_sha256:
        raise ReleaseError("release image SHA-256 disagrees with its build provenance")
    result = {
        "format_version": int(version),
        "source_revision": revision,
        "image_sha256": image_sha256,
    }
    if version == "2":
        result.update({key: fields[key] for key in required - set(result)})
    return result


def require_production_image(provenance: dict[str, Any]) -> None:
    """Configuration gate only; this cannot establish guest acceptance."""
    if provenance.get("format_version") != 2:
        raise ReleaseError("legacy layout/fixture provenance cannot be assembled as Nagi 0.1")
    if provenance.get("profile") != "production-session-v1":
        raise ReleaseError("partial or fixture session profile cannot be assembled as Nagi 0.1")
    components = provenance.get("components", "").split(",")
    expected = {"login", "files", "search", "servo", "local-ai", "history", "voice"}
    if len(components) != len(expected) or set(components) != expected:
        raise ReleaseError("production session lacks required integrated components")
    features = provenance.get("init_features", "").split(",")
    if "production-session" not in features or len(features) != len(set(features)):
        raise ReleaseError("production session lacks its ordinary init feature")
    if any(not re.fullmatch(r"[a-z0-9-]+", feature)
           or "acceptance" in feature or "fixture" in feature
           or re.match(r"m[0-9]+-", feature) for feature in features):
        raise ReleaseError("production session contains a milestone/fixture feature")
    if provenance.get("model_store_sha256") != GRANITE_SHA256:
        raise ReleaseError("production session lacks the pinned Granite Model Store bytes")


def _reject_symlink_components(root: Path, relative: PurePosixPath, label: str) -> Path:
    candidate = root
    if candidate.is_symlink():
        raise ReleaseError(f"{label} must not use a symlink root: {candidate}")
    for part in relative.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise ReleaseError(f"{label} must not pass through a symlink: {candidate}")
    return candidate


def _reject_bundle_symlinks(directory: Path) -> None:
    if directory.is_symlink():
        raise ReleaseError(f"release bundle root must not be a symlink: {directory}")

    def fail_walk(error: OSError) -> None:
        raise ReleaseError(f"cannot inspect release bundle: {error}") from error

    for root, directories, files in os.walk(directory, followlinks=False, onerror=fail_walk):
        for name in (*directories, *files):
            path = Path(root) / name
            if path.is_symlink():
                raise ReleaseError(f"release bundle contains a symlink: {path}")


def _rooted(root: Path, path: Path) -> Path:
    return path if path.is_absolute() else root / path


def required_document_inputs(root: Path) -> list[tuple[Path, str]]:
    missing: list[str] = []
    found: list[tuple[Path, str]] = []
    for source_name, package_name in DOC_INPUTS:
        source = root / source_name
        if source.is_symlink() or not source.is_file() or source.stat().st_size == 0:
            missing.append(source_name)
        else:
            found.append((source, package_name))
    if missing:
        raise ReleaseError("missing required release documentation: " + ", ".join(missing))
    return found


def tracked_license_inputs(root: Path) -> list[tuple[Path, str]]:
    root = root.resolve(strict=True)
    try:
        result = subprocess.run(
            ["git", "ls-files", "-z", "--", "third_party"],
            cwd=root,
            check=True,
            capture_output=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", b"") or str(error)
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", errors="replace")
        raise ReleaseError(f"cannot enumerate tracked third-party files: {detail.strip()}") from error

    found: list[tuple[Path, str]] = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        try:
            relative_text = raw_path.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ReleaseError("tracked third-party path is not valid UTF-8") from error
        relative = PurePosixPath(relative_text)
        if (
            relative.is_absolute()
            or not relative.parts
            or relative.parts[0] != "third_party"
            or ".." in relative.parts
        ):
            raise ReleaseError(f"unsafe tracked third-party path: {relative_text}")
        if LICENSE_FILENAME_PATTERN.fullmatch(relative.name) is None:
            continue
        if (
            "\\" in relative_text
            or ":" in relative_text
            or any(ord(character) < 32 or ord(character) == 127 for character in relative_text)
        ):
            raise ReleaseError(f"unsafe tracked third-party license path: {relative_text}")
        source = _reject_symlink_components(root, relative, "tracked third-party license text")
        if source.is_symlink() or not source.is_file() or source.stat().st_size == 0:
            raise ReleaseError(
                "tracked third-party license text is not a non-empty regular file: "
                f"{relative_text}"
            )
        package_path = (TRACKED_LICENSE_ROOT / Path(*relative.parts)).as_posix()
        found.append((source, package_path))

    if not found:
        raise ReleaseError("no tracked third-party license or notice texts were found")
    return sorted(found, key=lambda item: item[0].relative_to(root).as_posix())


def git_source_revision(root: Path) -> str:
    try:
        revision = subprocess.run(
            ["git", "rev-parse", "--verify", "HEAD"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        dirty = subprocess.run(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "") or str(error)
        raise ReleaseError(f"cannot establish Git source provenance: {detail.strip()}") from error
    if len(revision) not in {40, 64} or re.fullmatch(r"[0-9a-f]+", revision) is None:
        raise ReleaseError("Git returned an invalid source revision")
    if dirty:
        raise ReleaseError(
            "release source tree is not clean; commit or remove all tracked and untracked changes"
        )
    return revision


def kernel_build_id(root: Path, kernel_path: Path) -> str:
    root = root.resolve(strict=True)
    kernel = _regular_file(kernel_path, "kernel ELF")
    kernel = _within(root, kernel, "kernel ELF")
    kernel_relative = kernel.relative_to(root)
    if not kernel_relative.parts or kernel_relative.parts[0] not in {"target", "out"}:
        raise ReleaseError("kernel ELF must come from a repository build output under target/ or out/")
    relative_parts = [part.casefold() for part in kernel_relative.parts]
    if any("fixture" in part or part in {"test", "tests"} for part in relative_parts):
        raise ReleaseError("kernel provenance points into a test or fixture path")
    with kernel.open("rb") as stream:
        header = stream.read(20)
    if len(header) < 20 or header[:4] != b"\x7fELF":
        raise ReleaseError(f"kernel artifact is not an ELF file: {kernel}")
    if header[4] != 2 or header[5] != 1:
        raise ReleaseError("kernel artifact must be little-endian ELF64")
    if struct.unpack_from("<H", header, 18)[0] != 62:
        raise ReleaseError("kernel artifact must target x86-64")
    elf_type = struct.unpack_from("<H", header, 16)[0]
    if elf_type not in {2, 3}:
        raise ReleaseError("kernel artifact must be an executable ELF image")
    return "sha256:" + sha256_file(kernel)


def _pinned_revision(source_lock: dict[str, Any], source_name: str) -> str:
    try:
        value = source_lock["sources"][source_name]["revision"]
    except (KeyError, TypeError) as error:
        raise ReleaseError(f"third_party/sources.lock lacks sources.{source_name}.revision") from error
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40,64}", value):
        raise ReleaseError(f"sources.{source_name}.revision is not a full pinned revision")
    return value


def _read_repository_toml(path: Path) -> dict[str, Any]:
    if tomllib is not None:
        with path.open("rb") as stream:
            return tomllib.load(stream)

    # The release tool only consumes scalar values from these checked-in
    # configuration files. Keep Python 3.10 support without adding a runtime
    # TOML dependency by parsing the narrowly-scoped table/key subset here.
    result: dict[str, Any] = {}
    current: dict[str, Any] | None = None
    section_pattern = re.compile(r"^\[([^\[\]]+)\]\s*(?:#.*)?$")
    key_pattern = re.compile(r"^([A-Za-z0-9_-]+)\s*=\s*(.*?)\s*(?:#.*)?$")
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        section = section_pattern.fullmatch(line)
        if section:
            name = section.group(1)
            current = result
            for part in name.split("."):
                current = current.setdefault(part, {})
            continue
        if current is None:
            continue
        assignment = key_pattern.fullmatch(line)
        if assignment is None:
            continue
        key, encoded = assignment.groups()
        if encoded.startswith('"') and encoded.endswith('"'):
            current[key] = encoded[1:-1]
        elif re.fullmatch(r"[0-9]+", encoded):
            current[key] = int(encoded)
        elif encoded in {"true", "false"}:
            current[key] = encoded == "true"
        else:
            raise ReleaseError(f"unsupported TOML value in {path}:{line_number}")
    return result


def repository_metadata(root: Path, kernel_path: Path) -> dict[str, Any]:
    try:
        project_config = _read_repository_toml(root / "nagi.toml")
        source_lock = _read_repository_toml(root / "third_party/sources.lock")
    except (OSError, ValueError) as error:
        raise ReleaseError(f"cannot read version or source pins: {error}") from error

    version = project_config.get("project", {}).get("version")
    disk_gib = project_config.get("reference_machine", {}).get("disk_gib")
    if not isinstance(version, str) or not version:
        raise ReleaseError("nagi.toml lacks project.version")
    if not isinstance(disk_gib, int) or disk_gib <= 0:
        raise ReleaseError("nagi.toml lacks a valid reference_machine.disk_gib")

    m20_pin_doc = root / GRANITE_PIN_SOURCE
    _regular_file(m20_pin_doc, "M20 Granite pin provenance")
    pin_text = m20_pin_doc.read_text(encoding="utf-8")
    if GRANITE_SHA256 not in pin_text or GRANITE_PIN_REPOSITORY_REVISION not in pin_text:
        raise ReleaseError("M20 Granite pin no longer matches the release tool's recorded pin")

    return {
        "nagi_version": version,
        "reference_disk_gib": disk_gib,
        "servo_revision": _pinned_revision(source_lock, "servo"),
        "mesa_revision": _pinned_revision(source_lock, "mesa"),
        "llama_cpp_revision": _pinned_revision(source_lock, "llama_cpp"),
        "granite_sha256": GRANITE_SHA256,
        "granite_pin_source": GRANITE_PIN_SOURCE,
        "granite_model_bytes_bundled": False,
        "kernel_build_id": kernel_build_id(root, kernel_path),
    }


def _tool_version(name: str, command: list[str], root: Path) -> str:
    executable = shutil.which(command[0])
    if executable is None:
        raise ReleaseError(f"required release tool is unavailable: {command[0]}")
    try:
        result = subprocess.run(
            [executable, *command[1:]],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "") or str(error)
        raise ReleaseError(f"cannot record {name} version: {detail.strip()}") from error
    value = (result.stdout or result.stderr).strip().splitlines()
    if not value or not value[0].strip():
        raise ReleaseError(f"{name} did not report a version")
    return value[0].strip()


def toolchain_versions(root: Path) -> dict[str, str]:
    commands = {
        "rustc": ["rustc", "--version"],
        "cargo": ["cargo", "--version"],
        "clang": ["clang", "--version"],
        "lld": ["ld.lld", "--version"],
        "qemu_system_x86_64": ["qemu-system-x86_64", "--version"],
        "qemu_img": ["qemu-img", "--version"],
    }
    return {name: _tool_version(name, command, root) for name, command in commands.items()}


def validate_qcow2(image_path: Path, root: Path, disk_gib: int) -> None:
    root = root.resolve(strict=True)
    image = _regular_file(image_path, "release qcow2 image")
    image = _within(root, image, "release qcow2 image")
    image_relative = image.relative_to(root)
    if not image_relative.parts or image_relative.parts[0] != "out":
        raise ReleaseError("release qcow2 image must come from a repository build output under out/")
    qemu_img = shutil.which("qemu-img")
    if qemu_img is None:
        raise ReleaseError("required release tool is unavailable: qemu-img")
    try:
        info_result = subprocess.run(
            [qemu_img, "info", "--output=json", str(image)],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
        info = json.loads(info_result.stdout)
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        detail = getattr(error, "stderr", "") or str(error)
        raise ReleaseError(f"cannot inspect release image with qemu-img: {detail.strip()}") from error
    if info.get("format") != "qcow2":
        raise ReleaseError("release image must be a qcow2 image")
    if info.get("backing-filename"):
        raise ReleaseError("release image must be self-contained and have no backing file")
    expected_size = disk_gib * 1024 * 1024 * 1024
    if info.get("virtual-size") != expected_size:
        raise ReleaseError(
            f"qcow2 virtual size must match nagi.toml ({disk_gib} GiB, {expected_size} bytes)"
        )
    try:
        subprocess.run(
            [qemu_img, "check", "-f", "qcow2", str(image)],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "") or getattr(error, "stdout", "") or str(error)
        raise ReleaseError(f"qemu-img check failed for release image: {detail.strip()}") from error


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_json_bytes(value))


def checksum_lines(directory: Path, exclude: Iterable[str] = (SUMS_NAME,)) -> bytes:
    excluded = set(exclude)
    files = sorted(
        (entry for entry in directory.rglob("*") if entry.is_file()),
        key=lambda entry: entry.relative_to(directory).as_posix(),
    )
    lines = []
    for path in files:
        relative = path.relative_to(directory).as_posix()
        if relative in excluded:
            continue
        lines.append(f"{sha256_file(path)}  {relative}\n")
    return "".join(lines).encode("ascii")


def verify_checksum_index(directory: Path) -> None:
    _reject_bundle_symlinks(directory)
    sums_path = directory / SUMS_NAME
    _regular_file(sums_path, "SHA256SUMS")
    expected: dict[str, str] = {}
    try:
        lines = sums_path.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        raise ReleaseError(f"cannot read SHA256SUMS: {error}") from error
    for number, line in enumerate(lines, start=1):
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if match is None:
            raise ReleaseError(f"malformed SHA256SUMS line {number}")
        digest, relative_text = match.groups()
        relative = PurePosixPath(relative_text)
        if (
            relative.is_absolute()
            or ".." in relative.parts
            or "\\" in relative_text
            or ":" in relative_text
            or relative_text in expected
        ):
            raise ReleaseError(f"unsafe or duplicate checksum path on line {number}")
        expected[relative_text] = digest
    actual_files = {
        path.relative_to(directory).as_posix()
        for path in directory.rglob("*")
        if path.is_file() and path != sums_path
    }
    if actual_files != set(expected):
        missing = sorted(set(expected) - actual_files)
        extra = sorted(actual_files - set(expected))
        raise ReleaseError(f"SHA256SUMS file set mismatch; missing={missing}, extra={extra}")
    for relative, expected_digest in expected.items():
        path = directory.joinpath(*PurePosixPath(relative).parts)
        if path.is_symlink() or not path.is_file():
            raise ReleaseError(f"checksummed artifact is not a regular file: {relative}")
        actual_digest = sha256_file(path)
        if actual_digest != expected_digest:
            raise ReleaseError(f"SHA-256 mismatch: {relative}")


def verify_tracked_license_inventory(directory: Path, build_manifest: dict[str, Any]) -> None:
    inventory = build_manifest.get("tracked_license_texts")
    directory = directory.resolve(strict=True)
    source_tree_relative = PurePosixPath(TRACKED_LICENSE_ROOT.as_posix())
    source_tree = _reject_symlink_components(
        directory, source_tree_relative, "tracked license inventory"
    )
    if inventory is None:
        if source_tree.exists():
            raise ReleaseError("release build manifest lacks its tracked license text inventory")
        return
    if not isinstance(inventory, list) or not inventory:
        raise ReleaseError("release build manifest has an invalid tracked license text inventory")

    expected_paths: set[str] = set()
    for record in inventory:
        if not isinstance(record, dict):
            raise ReleaseError("release build manifest contains an invalid license record")
        source_text = record.get("source_path")
        package_text = record.get("package_path")
        digest = record.get("sha256")
        if not isinstance(source_text, str) or not isinstance(package_text, str):
            raise ReleaseError("release build manifest contains an invalid license path")
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise ReleaseError("release build manifest contains an invalid license digest")

        source = PurePosixPath(source_text)
        package = PurePosixPath(package_text)
        if (
            source.is_absolute()
            or not source.parts
            or source.parts[0] != "third_party"
            or ".." in source.parts
            or "\\" in source_text
            or ":" in source_text
            or any(ord(character) < 32 or ord(character) == 127 for character in source_text)
            or LICENSE_FILENAME_PATTERN.fullmatch(source.name) is None
        ):
            raise ReleaseError(f"unsafe or unsupported tracked license source path: {source_text}")
        expected_package = TRACKED_LICENSE_ROOT / Path(*source.parts)
        if (
            package.is_absolute()
            or ".." in package.parts
            or "\\" in package_text
            or ":" in package_text
            or any(ord(character) < 32 or ord(character) == 127 for character in package_text)
            or package != PurePosixPath(expected_package.as_posix())
            or package_text in expected_paths
        ):
            raise ReleaseError(f"unsafe or duplicate tracked license package path: {package_text}")
        expected_paths.add(package_text)

        packaged_file = _reject_symlink_components(
            directory, package, "tracked license package path"
        )
        if not packaged_file.is_file() or packaged_file.stat().st_size == 0:
            raise ReleaseError(f"tracked license text is missing or unsafe: {package_text}")
        if sha256_file(packaged_file) != digest:
            raise ReleaseError(f"tracked license text SHA-256 mismatch: {package_text}")

    actual_paths: set[str] = set()
    if source_tree.exists():
        for path in source_tree.rglob("*"):
            if path.is_symlink():
                raise ReleaseError(f"tracked license inventory contains a symlink: {path}")
            if path.is_file():
                actual_paths.add(path.relative_to(directory).as_posix())
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        raise ReleaseError(
            f"tracked license text file set mismatch; missing={missing}, extra={extra}"
        )


def verify_release(directory: Path) -> None:
    _reject_bundle_symlinks(directory)
    _regular_file(directory / MANIFEST_NAME, "release manifest")
    _regular_file(directory / BUILD_MANIFEST_NAME, "build manifest")
    _regular_file(directory / SOURCE_REVISION_NAME, "source revision record")
    _regular_file(directory / IMAGE_NAME, "release qcow2 image")
    for _, package_name in DOC_INPUTS:
        _regular_file(directory / package_name, "required release documentation")

    try:
        manifest = json.loads((directory / MANIFEST_NAME).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseError(f"cannot read release manifest: {error}") from error
    if not isinstance(manifest, dict):
        raise ReleaseError("release manifest root must be a JSON object")
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ReleaseError("unsupported release manifest schema")
    if manifest.get("assembly_status") != "ASSEMBLED":
        raise ReleaseError("release manifest has an unsupported assembly status")
    if manifest.get("m30_acceptance") != "NOT_EVALUATED":
        raise ReleaseError("artifact verification cannot establish M30 guest acceptance")
    if "release_ready" in manifest:
        raise ReleaseError("release manifest must not make a release-readiness claim")
    verify_checksum_index(directory)

    try:
        build_manifest = json.loads(
            (directory / BUILD_MANIFEST_NAME).read_text(encoding="utf-8")
        )
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseError(f"cannot read build manifest: {error}") from error
    if not isinstance(build_manifest, dict):
        raise ReleaseError("build manifest root must be a JSON object")
    verify_tracked_license_inventory(directory, build_manifest)
    required_build_fields = {
        "nagi_version",
        "source_revision",
        "kernel_build_id",
        "reference_image_sha256",
        "servo_revision",
        "mesa_revision",
        "llama_cpp_revision",
        "granite_sha256",
        "toolchain_versions",
    }
    if not required_build_fields.issubset(build_manifest):
        absent = sorted(required_build_fields - set(build_manifest))
        raise ReleaseError(f"build manifest lacks provenance fields: {absent}")
    revision_text = (directory / SOURCE_REVISION_NAME).read_text(encoding="ascii").strip()
    if revision_text != build_manifest["source_revision"]:
        raise ReleaseError("source revision record disagrees with the build manifest")
    if build_manifest["reference_image_sha256"] != sha256_file(directory / IMAGE_NAME):
        raise ReleaseError("build manifest image hash disagrees with the release qcow2")
    if "image_build_provenance" in build_manifest:
        image_provenance = build_manifest["image_build_provenance"]
        if (
            not isinstance(image_provenance, dict)
            or image_provenance.get("format_version") != 2
            or image_provenance.get("source_revision") != build_manifest["source_revision"]
            or image_provenance.get("image_sha256") != build_manifest["reference_image_sha256"]
        ):
            raise ReleaseError("build manifest image provenance disagrees with source or qcow2")
        require_production_image(image_provenance)
        if build_manifest.get("granite_model_bytes_bundled") is not True:
            raise ReleaseError("production build manifest must record bundled Granite bytes")
    else:
        raise ReleaseError("build manifest lacks production image provenance")

    records = manifest.get("artifacts")
    if not isinstance(records, list):
        raise ReleaseError("release manifest has no artifact index")
    recorded: dict[str, dict[str, Any]] = {}
    for record in records:
        if not isinstance(record, dict) or not isinstance(record.get("path"), str):
            raise ReleaseError("release manifest contains an invalid artifact record")
        path = record["path"]
        if path in recorded:
            raise ReleaseError(f"duplicate artifact record: {path}")
        recorded[path] = record

    expected_payloads = {
        path.relative_to(directory).as_posix()
        for path in directory.rglob("*")
        if path.is_file() and path.name not in {SUMS_NAME, MANIFEST_NAME}
    }
    if set(recorded) != expected_payloads:
        missing = sorted(expected_payloads - set(recorded))
        extra = sorted(set(recorded) - expected_payloads)
        raise ReleaseError(f"release manifest file set mismatch; missing={missing}, extra={extra}")
    for relative, record in recorded.items():
        path = directory.joinpath(*PurePosixPath(relative).parts)
        if record.get("size_bytes") != path.stat().st_size:
            raise ReleaseError(f"release manifest size mismatch: {relative}")
        if record.get("sha256") != sha256_file(path):
            raise ReleaseError(f"release manifest SHA-256 mismatch: {relative}")
        if not isinstance(record.get("source"), str) or not record["source"]:
            raise ReleaseError(f"release manifest lacks source provenance: {relative}")


def release_manifest_for(stage: Path, copied: list[tuple[Path, str]]) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "assembly_status": "ASSEMBLED",
        "m30_acceptance": "NOT_EVALUATED",
        "artifacts": [
            _artifact_record(stage / relative, relative, source)
            for relative, source in sorted(copied, key=lambda item: item[0].as_posix())
        ],
    }


def _artifact_record(path: Path, relative_name: Path, source: str) -> dict[str, Any]:
    return {
        "path": relative_name.as_posix(),
        "size_bytes": path.stat().st_size,
        "sha256": sha256_file(path),
        "source": source,
    }


def assemble(root: Path, image_path: Path, kernel_path: Path, output: Path) -> None:
    root = root.resolve(strict=True)
    if output.exists():
        raise ReleaseError(f"output directory already exists; refusing to overwrite: {output}")

    documents = required_document_inputs(root)
    license_inputs = tracked_license_inputs(root)
    revision = git_source_revision(root)
    metadata = repository_metadata(root, kernel_path)
    validate_qcow2(image_path, root, metadata["reference_disk_gib"])
    image_provenance = image_build_provenance(root, image_path, revision)
    require_production_image(image_provenance)
    versions = toolchain_versions(root)

    image_path = _regular_file(image_path, "release qcow2 image").resolve(strict=True)
    build_manifest = {
        "schema_version": SCHEMA_VERSION,
        "nagi_version": metadata["nagi_version"],
        "source_revision": revision,
        "kernel_build_id": metadata["kernel_build_id"],
        "reference_image_sha256": sha256_file(image_path),
        "image_build_provenance": image_provenance,
        "servo_revision": metadata["servo_revision"],
        "mesa_revision": metadata["mesa_revision"],
        "llama_cpp_revision": metadata["llama_cpp_revision"],
        "granite_sha256": metadata["granite_sha256"],
        "granite_pin_source": metadata["granite_pin_source"],
        "granite_model_bytes_bundled": image_provenance["model_store_sha256"] == GRANITE_SHA256,
        "distribution_review": "NOT_EVALUATED",
        "reference_disk_gib": metadata["reference_disk_gib"],
        "toolchain_versions": versions,
        "tracked_license_texts": [
            {
                "source_path": source.relative_to(root).as_posix(),
                "package_path": package_name,
                "sha256": sha256_file(source),
            }
            for source, package_name in license_inputs
        ],
    }

    output = output.absolute()
    parent = output.parent
    parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".nagi-release-", dir=parent) as temporary:
        stage = Path(temporary) / "release"
        stage.mkdir()
        copied: list[tuple[Path, str]] = []
        shutil.copyfile(image_path, stage / IMAGE_NAME)
        copied.append((Path(IMAGE_NAME), "built reference-machine qcow2"))
        for source, package_name in documents:
            destination = stage / package_name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            copied.append((Path(package_name), source.relative_to(root).as_posix()))
        for source, package_name in license_inputs:
            destination = stage / package_name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            copied.append((Path(package_name), source.relative_to(root).as_posix()))

        (stage / SOURCE_REVISION_NAME).write_text(revision + "\n", encoding="ascii", newline="\n")
        _write_json(stage / BUILD_MANIFEST_NAME, build_manifest)
        copied.append((Path(SOURCE_REVISION_NAME), "git rev-parse HEAD"))
        copied.append((Path(BUILD_MANIFEST_NAME), "repository and build provenance"))

        _write_json(stage / MANIFEST_NAME, release_manifest_for(stage, copied))
        (stage / SUMS_NAME).write_bytes(checksum_lines(stage))
        for path in stage.rglob("*"):
            if path.is_file():
                path.chmod(0o644)
                os.utime(path, (0, 0))
        os.replace(stage, output)

    verify_release(output)


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    for command in ("preflight", "assemble"):
        subparser = subparsers.add_parser(command)
        subparser.add_argument("--root", type=Path, default=Path.cwd())
        subparser.add_argument("--image", type=Path, required=True, help="built qcow2 reference image")
        subparser.add_argument("--kernel", type=Path, required=True, help="built x86-64 kernel ELF")
        if command == "assemble":
            subparser.add_argument("--output", type=Path, required=True)

    verify_parser = subparsers.add_parser("verify", help="verify assembled files and SHA256SUMS")
    verify_parser.add_argument("--directory", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    try:
        if args.command == "verify":
            verify_release(args.directory.resolve(strict=True))
            print("Artifact integrity verified. M30 guest acceptance remains not evaluated.")
            return 0

        root = args.root.resolve(strict=True)
        if args.command == "preflight":
            required_document_inputs(root)
            tracked_license_inputs(root)
            revision = git_source_revision(root)
            kernel = _rooted(root, args.kernel)
            image = _rooted(root, args.image)
            metadata = repository_metadata(root, kernel)
            validate_qcow2(image, root, metadata["reference_disk_gib"])
            require_production_image(image_build_provenance(root, image, revision))
            toolchain_versions(root)
            print(
                "Release inputs and provenance verified; M30 guest acceptance is a separate, "
                "unverified gate."
            )
            return 0
        assemble(
            root,
            _rooted(root, args.image),
            _rooted(root, args.kernel),
            _rooted(root, args.output),
        )
        print("Artifacts assembled and checksummed. M30 guest acceptance remains not evaluated.")
        return 0
    except (ReleaseError, OSError) as error:
        print(f"release preflight failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
