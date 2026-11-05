#!/usr/bin/env python3
"""Restore exact reviewed Git objects offline. Never create/update public refs."""
import hashlib
import json
from pathlib import Path
import subprocess

DOC = "docs/porting/multiarch/"
BUNDLE_SHA256 = "8de10b5e5ba10efb3c3347ec3bb81762d190d8578713cb06309b19b14b2ce52a"
BUNDLE_BYTES = 34088
ANCHORS = {
    "current_records": ("2b394ca5e0795ae1b5a355f5e48fd9d02791d5cc", "1a5b3cab6cc477edc54115a01cdfd10a3596c8df"),
    "prior_composition": ("482cac3f13ca7ad78f37bf98c67a4a16ef004351", "e881ca57cea0865dca24af0a5629d92c48c4550c"),
    "historical_records": ("dc11f55d934ae73f508858458a09a3994889520c", "26beb4905c44132d63a6db377e23da0521448ab3"),
}
REFS = {
    "refs/heads/codex/arm64-integration-gates-57": ANCHORS["prior_composition"][0],
    "refs/heads/codex/arm64-records-restored-57": ANCHORS["historical_records"][0],
}


def git(root, *args):
    result = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
    if result.returncode:
        raise ValueError(result.stderr.strip() or "Git command failed")
    return result.stdout.strip()


def validate_manifest(value):
    if set(value) != {"schema", "bundle", "bytes", "sha256", *ANCHORS, "prerequisites", "scope"}:
        raise ValueError("dependency manifest fields differ")
    if (value["schema"] != "nagi-pr57-offline-review-objects/1"
            or value["bundle"] != "review-dependencies.bundle"
            or value["bytes"] != BUNDLE_BYTES or value["sha256"] != BUNDLE_SHA256):
        raise ValueError("dependency identity differs from reviewed bundle")
    for key, (commit, tree) in ANCHORS.items():
        if value[key] != {"commit": commit, "tree": tree}:
            raise ValueError("dependency anchor changed: " + key)


def validate_bundle(data):
    if len(data) != BUNDLE_BYTES or hashlib.sha256(data).hexdigest() != BUNDLE_SHA256:
        raise ValueError("dependency bundle size/hash mismatch")


def restore(root):
    root = Path(root).resolve()
    directory = root / DOC
    manifest = directory / "review-dependencies.json"
    bundle = directory / "review-dependencies.bundle"
    if any(path.is_symlink() or not path.is_file() for path in (manifest, bundle)):
        raise ValueError("dependency inputs must be regular files")
    validate_manifest(json.loads(manifest.read_text()))
    validate_bundle(bundle.read_bytes())
    refs = dict(line.split(" ", 1)[::-1] for line in git(root, "bundle", "list-heads", str(bundle)).splitlines())
    if refs != REFS:
        raise ValueError("bundle advertised refs differ")
    git(root, "bundle", "verify", str(bundle))
    # unbundle imports objects only. It neither fetches a network URL nor writes
    # local/remote branch refs, and does not create a merge commit.
    before_head = git(root, "rev-parse", "HEAD")
    before_refs = git(root, "for-each-ref", "--format=%(refname) %(objectname)")
    git(root, "bundle", "unbundle", str(bundle))
    for key, (commit, tree) in ANCHORS.items():
        if git(root, "rev-parse", commit + "^{commit}") != commit or git(root, "rev-parse", commit + "^{tree}") != tree:
            raise ValueError("restored commit/tree mismatch: " + key)
    if git(root, "rev-parse", "HEAD") != before_head or git(root, "for-each-ref", "--format=%(refname) %(objectname)") != before_refs:
        raise ValueError("object restore unexpectedly changed HEAD or refs")
    return {key: commit for key, (commit, _) in ANCHORS.items()}


if __name__ == "__main__":
    print(json.dumps(restore(Path(__file__).resolve().parents[3]), indent=2))
