"""Read-only evidence adapter; this is not a Nagi SDK or support contract."""

import hashlib
import json
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def inspect_contract(repo_root=REPO):
    """Invalidate candidates on missing/changed evidence, including version drift.

    Only bundled, repository-relative paths are read. No input UI value controls
    evidence I/O. A UI owner can replace this adapter and its reviewed snapshot.
    """
    snapshot = json.loads((HERE / "contract_snapshot.json").read_text(encoding="utf-8"))
    evidence = []
    for item in snapshot["files"]:
        try:
            actual = hashlib.sha256((Path(repo_root) / item["path"]).read_bytes()).hexdigest()
            state = "matched" if actual == item["sha256"] else "changed"
        except OSError:
            actual, state = None, "missing"
        evidence.append(dict(item, actual_sha256=actual, state=state))
    return {
        "adapter_version": 1,
        "package": "nagi-ui",
        "expected_version": snapshot["package_version"],
        "state": "matched" if all(e["state"] == "matched" for e in evidence) else "needs_review",
        "evidence": evidence,
        "approval": "PROVISIONAL",
    }
