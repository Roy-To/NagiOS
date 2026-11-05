#!/usr/bin/env python3
"""Read-only M30 evidence inventory; never a formal acceptance result.

Uses the existing session and release contracts. Hashes establish consistency
of supplied bytes, not their origin, signatures, or execution in a guest.
No receipt, event, timestamp, or acceptance artifact is produced by this tool.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

import release
import session_acceptance as session


class MissingInput(release.ReleaseError):
    """An explicitly selected input does not exist."""


INPUT_ERRORS = (release.ReleaseError, OSError, UnicodeError, ValueError,
                TypeError, KeyError, IndexError, RecursionError)


def selected_path(root, value, *, directory=False):
    """Do not follow symlinks, including directory components or dangling links."""
    path = Path(value)
    if path.is_absolute():
        try:
            path = path.relative_to(root)
        except ValueError as error:
            raise release.ReleaseError("input outside root") from error
    parts = path.parts
    if not parts or any(part in (".", "..") for part in parts):
        raise release.ReleaseError("input must be a rooted relative path without traversal")
    current = root
    for part in parts:
        current /= part
        if current.is_symlink():
            raise release.ReleaseError(f"symlink input: {current.relative_to(root)}")
        if not current.exists():
            raise MissingInput(f"missing input: {current.relative_to(root)}")
    if directory:
        if not current.is_dir():
            raise release.ReleaseError(f"not a directory: {path}")
    elif not current.is_file():
        raise release.ReleaseError(f"not a regular file: {path}")
    return current


def collect(root, revision, image, evidence, bundle):
    """Return observations only; explicit paths avoid mixing unrelated runs.

    revision is the caller's full expected source revision. The clean Git check
    is reported independently so other missing inputs remain visible on failure.
    """
    root = Path(root).absolute()
    report = {
        "schema_version": 1,
        "report_kind": "m30-evidence-preflight",
        "m30_acceptance": "NOT_EVALUATED",
        "formal_gate": "BLOCKED",
        "expected_source_revision": revision,
        "checks": [],
        "artifacts": [],
    }

    def check(name, operation, integrity="VALIDATED_CONTRACT"):
        result = {"id": name, "status": "NOT_VERIFIED", "content_integrity": "NOT_CHECKED"}
        report["checks"].append(result)
        try:
            operation()
        except MissingInput as error:
            result.update(status="MISSING", reason=str(error))
        except INPUT_ERRORS as error:
            result.update(status="INVALID", reason=str(error))
        else:
            result.update(content_integrity=integrity,
                          reason=("Nonempty regular file hashed; bundle validation is reported separately."
                                  if integrity == "FILE_HASHED" else
                                  "Existing content contract validated; origin and guest execution are not authenticated."))

    def artifact(path):
        relative = path.relative_to(root).as_posix()
        if not any(item["path"] == relative for item in report["artifacts"]):
            report["artifacts"].append({"path": relative, "size_bytes": path.stat().st_size,
                                        "sha256": release.sha256_file(path)})

    def source():
        session.require(not any(p.is_symlink() for p in (root, *root.parents)), "symlink root")
        session.require(isinstance(revision, str) and re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", revision),
                        "expected source must be a full commit ID")
        session.require(release.git_source_revision(root) == revision, "source revision differs from expected commit")

    # Reject an unsafe root before reading any selected artifacts.
    if any(p.is_symlink() for p in (root, *root.parents)):
        check("source", source)
        return report
    check("source", source)

    def image_content():
        path = selected_path(root, image)
        info = selected_path(root, str(path) + release.IMAGE_BUILD_INFO_SUFFIX)
        provenance = release.image_build_provenance(root, path, revision)
        release.require_production_image(provenance)
        artifact(path)
        artifact(info)

    check("production_image", image_content)

    def session_content():
        path = selected_path(root, evidence)
        # Validate before recording any manifest claims in the inventory.
        manifest = session._json(session._read(path, session.MAX_MANIFEST_BYTES).decode("utf-8"))
        if isinstance(manifest, dict) and isinstance(manifest.get("phases"), dict):
            for phase in session.EVENT_SEQUENCES:
                record = manifest["phases"].get(phase)
                if isinstance(record, dict) and isinstance(record.get("path"), str):
                    selected_path(root, record["path"])
        selected_path(root, image)
        selected_path(root, str(image) + release.IMAGE_BUILD_INFO_SUFFIX)
        session.validate_session(root, image, path, revision)
        artifact(path)
        for phase in session.EVENT_SEQUENCES:
            artifact(selected_path(root, manifest["phases"][phase]["path"]))

    check("ordinary_session", session_content)

    # Inventory every required distribution input independently, so a missing
    # first document does not hide all remaining absent inputs.
    required = (release.MANIFEST_NAME, release.BUILD_MANIFEST_NAME,
                release.SOURCE_REVISION_NAME, release.SUMS_NAME, release.IMAGE_NAME,
                *(name for _, name in release.DOC_INPUTS))
    for name in required:
        def distribution_input(name=name):
            path = selected_path(root, Path(bundle) / name)
            release._regular_file(path, "distribution input")
            artifact(path)
        check("distribution_input:" + name, distribution_input, "FILE_HASHED")

    def distribution_content():
        directory = selected_path(root, bundle, directory=True)
        # The existing release reader predates strict JSON parsing. Reject
        # duplicate keys and nonfinite values before using its real contract.
        for name in (release.MANIFEST_NAME, release.BUILD_MANIFEST_NAME):
            path = selected_path(root, directory / name)
            session._json(session._read(path, session.MAX_MANIFEST_BYTES).decode("utf-8"))
        release.verify_release(directory)
        build = session._json((directory / release.BUILD_MANIFEST_NAME).read_text(encoding="utf-8"))
        session.require(build["source_revision"] == revision, "distribution source differs from expected commit")
        pristine = selected_path(root, image)
        session.require(build["reference_image_sha256"] == release.sha256_file(pristine),
                        "distribution image differs from selected session image")

    check("distribution_bundle", distribution_content)
    for name, reason in (
        ("execution_origin", "No authenticated producer/origin contract is available. Self-consistent hashes, events and CLI run receipts do not prove guest execution or signed-in identity."),
        ("voice_execution", "The session contract checks an offline voice flag and audio stress measurements, but has no independent voice operation evidence contract."),
        ("distribution_review", "Artifact integrity is separate from complete linked binary/model/font redistribution review; no authoritative approval contract is supplied."),
    ):
        report["checks"].append({"id": name, "status": "NOT_VERIFIED",
                                  "content_integrity": "NOT_CHECKED", "reason": reason})
    report["artifacts"].sort(key=lambda item: item["path"])
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--revision", required=True, help="full expected source commit ID")
    parser.add_argument("--image", type=Path, default=Path("out/artifacts") / release.IMAGE_NAME)
    parser.add_argument("--evidence", type=Path, default=Path("evidence/session.json"))
    parser.add_argument("--bundle", type=Path, default=Path("out/release/Nagi-OS-0.1-devpreview"))
    args = parser.parse_args(argv)
    report = collect(args.root, args.revision, args.image, args.evidence, args.bundle)
    sys.stdout.buffer.write(release.canonical_json_bytes(report))
    # This is deliberately not a success command, even for consistent fixtures.
    return 1


if __name__ == "__main__":
    sys.exit(main())
