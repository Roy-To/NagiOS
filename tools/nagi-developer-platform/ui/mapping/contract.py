"""Read-only evidence adapter; this is not a Nagi SDK or support contract."""

import hashlib
import json
import os
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def _sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _has_symlink(repo, relative):
    """True when any component below the repo root is a symlink (never followed)."""
    current = Path(repo)
    for part in Path(relative).parts:
        current = current / part
        if current.is_symlink():
            return True
    return False


def _unlisted(repo, snapshot, listed):
    """Files under the snapshot's enumerated directories that the snapshot lacks.

    A new source file can change the contract without touching any listed file, so
    it must send the report back to review. Directories and suffixes come from the
    reviewed snapshot, never from an input UI value.

    Returns (unlisted regular files, rejected symlink paths). Symlinks, whether to a
    file or a directory, are never followed or read: they are returned for fail-closed
    rejection, so a link cannot make the adapter hash a file outside the repository.
    """
    repo, found, rejected = Path(repo), set(), set()
    for rule in snapshot.get("enumerate", []):
        if _has_symlink(repo, rule["dir"]):
            rejected.add(rule["dir"])
            continue
        for current, dirs, files in os.walk(repo / rule["dir"], followlinks=False):
            for name in list(dirs):
                if (Path(current) / name).is_symlink():
                    dirs.remove(name)
                    rejected.add((Path(current) / name).relative_to(repo).as_posix())
            for name in files:
                if not name.endswith(rule["suffix"]):
                    continue
                path = Path(current) / name
                relative = path.relative_to(repo).as_posix()
                if path.is_symlink():
                    rejected.add(relative)
                elif path.is_file():
                    found.add(relative)
    return sorted(found - listed), sorted(rejected)


def inspect_contract(repo_root=REPO):
    """Invalidate candidates on missing/changed/added evidence, including version drift.

    Only bundled, repository-relative paths are read. No input UI value controls
    evidence I/O. A UI owner can replace this adapter and its reviewed snapshot.
    """
    snapshot = json.loads((HERE / "contract_snapshot.json").read_text(encoding="utf-8"))
    evidence = []
    for item in snapshot["files"]:
        if _has_symlink(repo_root, item["path"]):
            # Fixed state code only; the link target is never read or reported.
            evidence.append(dict(item, actual_sha256=None, state="symlink_rejected"))
            continue
        try:
            actual = _sha256(Path(repo_root) / item["path"])
            state = "matched" if actual == item["sha256"] else "changed"
        except OSError:
            actual, state = None, "missing"
        evidence.append(dict(item, actual_sha256=actual, state=state))
    listed = {item["path"] for item in snapshot["files"]}
    unlisted, rejected = _unlisted(repo_root, snapshot, listed)
    for path in rejected:
        if path not in listed:
            evidence.append({"path": path, "sha256": None, "actual_sha256": None,
                             "state": "symlink_rejected"})
    for path in unlisted:
        try:
            actual = _sha256(Path(repo_root) / path)
        except OSError:
            actual = None
        evidence.append({"path": path, "sha256": None, "actual_sha256": actual,
                         "state": "unlisted"})
    return {
        "adapter_version": 1,
        "package": "nagi-ui",
        "expected_version": snapshot["package_version"],
        "state": "matched" if all(e["state"] == "matched" for e in evidence) else "needs_review",
        "evidence": evidence,
        "approval": "PROVISIONAL",
    }
