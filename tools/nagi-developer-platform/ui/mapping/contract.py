"""Read-only evidence adapter; this is not a Nagi SDK or support contract."""

import hashlib
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def _sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _unlisted(repo, snapshot, listed):
    """Files under the snapshot's enumerated directories that the snapshot lacks.

    A new source file can change the contract without touching any listed file, so
    it must send the report back to review. Directories and suffixes come from the
    reviewed snapshot, never from an input UI value.
    """
    repo, found = Path(repo), set()
    for rule in snapshot.get("enumerate", []):
        base = repo / rule["dir"]
        try:
            candidates = sorted(base.rglob("*" + rule["suffix"]))
        except OSError:
            continue
        for path in candidates:
            if path.is_file():
                found.add(path.relative_to(repo).as_posix())
    return sorted(found - listed)


def inspect_contract(repo_root=REPO):
    """Invalidate candidates on missing/changed/added evidence, including version drift.

    Only bundled, repository-relative paths are read. No input UI value controls
    evidence I/O. A UI owner can replace this adapter and its reviewed snapshot.
    """
    snapshot = json.loads((HERE / "contract_snapshot.json").read_text(encoding="utf-8"))
    evidence = []
    for item in snapshot["files"]:
        try:
            actual = _sha256(Path(repo_root) / item["path"])
            state = "matched" if actual == item["sha256"] else "changed"
        except OSError:
            actual, state = None, "missing"
        evidence.append(dict(item, actual_sha256=actual, state=state))
    listed = {item["path"] for item in snapshot["files"]}
    for path in _unlisted(repo_root, snapshot, listed):
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
