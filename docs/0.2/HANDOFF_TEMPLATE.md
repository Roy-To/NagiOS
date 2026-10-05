# Workstream Handoff

Copy this into the owning workstream state update or link the state file. Do
not maintain a second hand-written status ledger.

```text
Workstream:
Branch:
HEAD:
Base:
Status:

Completed:
Last verified commit:
Tests (command, PASS/FAIL, exit code):
CI (run ID, URL, head SHA, job/stage, conclusion):

Current blocker (failure class):
Evidence (log paths and SHA-256):
Attempted fixes and how the hypothesis changed:
Do not repeat:

Acceptance criteria:
Deferred:
Cross-workstream dependencies/contracts:
Next exact action:
```

At handoff, refresh the state from actual Git/CI evidence, inspect status and
diff, and leave the worktree resumable. Never copy credentials, secrets, or
unredacted sensitive user data into tracked state or logs.
