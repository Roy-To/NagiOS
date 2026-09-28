# Nagi OS — M18-C Browser Acceptance / Reliability Workstream

## 0. Purpose

This workstream exists to shorten the final integration and stabilization phase of M18 by building the acceptance, reliability, regression, and recovery test surface in parallel with the ongoing M18 implementation.

This is **not** a third browser implementation stream.

M18-C must not duplicate or take ownership of the production implementation already being developed by:

- M18 main integration
- M18-A: remote web / HTTP(S) / network path
- M18-B: browser UI / state / persistence path
- M19-PREP or later semantic/search work

The goal is to make M18 objectively testable and to expose integration failures early.

Primary deliverable:

> A repeatable acceptance and reliability harness that can prove the browser works across navigation, network, session/state persistence, recovery, and user-facing browser operations, with clear PASS/FAIL output suitable for CI and final M18 acceptance.

---

## 1. Repository

Primary repository:

`/Users/tozawa/Developer/NagiOS`

If Codex is operating in an existing dedicated worktree, use that worktree instead of creating unrelated duplicate clones.

Before implementation, inspect:

- `AGENTS.md`
- current M18 specification
- M17 acceptance/result
- M18 main integration state
- M18-A state
- M18-B state
- M19-PREP state if present
- DF-01 / `.dev` workstream conventions
- current CI workflows
- current browser tests
- current browser storage/session/history/bookmark APIs
- any existing network fixtures / mock servers / test assets

Repository state and actual Git history take precedence over old chat summaries.

---

## 2. Branch / Worktree

Preferred dedicated branch:

`codex/m18c-browser-acceptance-reliability`

Preferred worktree name:

`m18c-browser-acceptance-reliability`

### Base selection

Do **not** guess a stale SHA.

At workstream start:

1. Identify the accepted M17 baseline.
2. Identify the current M18 integration base actually being used by the active M18 workstreams.
3. Select the safest shared base that avoids pulling unfinished M18-A/M18-B production edits into this branch unnecessarily.
4. Record the exact base branch and base SHA in the workstream state/report.

If M18-specific public interfaces already exist on the current M18 integration branch and are needed for compiling the harness, use that integration base only after confirming it is the intended shared integration point.

Do not rebase/cherry-pick other active workstreams merely to obtain unfinished functionality.

---

## 3. Ownership Boundary

M18-C owns acceptance and reliability infrastructure.

### Allowed primary edit areas

Prefer edits in areas equivalent to:

- browser acceptance tests
- browser integration tests
- end-to-end test harnesses
- deterministic test fixtures
- local HTTP/HTTPS test server fixtures
- redirect/error fixtures
- session/history/bookmark persistence test fixtures
- recovery/crash simulation harnesses
- browser test utilities
- CI jobs dedicated to M18 acceptance
- test-only feature gates
- test documentation
- workstream state/status files
- acceptance evidence/log parsers if needed

Examples may include paths such as:

- `tests/`
- `integration-tests/`
- `user/nagi-albert/tests/`
- `user/nagi-posix/tests/`
- `tools/`
- `.github/workflows/`
- `.dev/workstreams/...`
- `docs/...`

Use the repository's actual layout.

### Conditionally allowed edits

Minimal production edits are allowed **only** when required to expose a stable test seam, such as:

- dependency injection boundary
- test-only constructor
- feature-gated mock provider
- deterministic clock/storage/network adapter
- log/diagnostic hook needed for acceptance verification

Such edits must be:

- small
- generic
- non-invasive
- backward compatible
- clearly justified
- not an alternate implementation of M18-A or M18-B

### Forbidden scope

Do not independently implement or redesign:

- Servo/browser engine internals
- full HTTP(S) transport
- TLS stack
- DNS stack
- browser chrome/UI product behavior
- tab implementation
- session/history/bookmark production ownership
- compositor/rendering architecture
- kernel ABI
- capability system
- model runtime
- 0.2 product architecture
- M19 semantic/search production functionality

Do not modify another active workstream's state as if you own it.

Do not merge active branches into this branch unless the integration owner explicitly directs it.

---

## 4. Core Acceptance Matrix

Build the harness so each row can independently report:

- PASS
- FAIL
- SKIP-BLOCKED
- NOT-IMPLEMENTED

A missing implementation is not a reason to stop building the test.

### A. Navigation

Cover at minimum:

- direct URL navigation
- repeated navigation
- navigation cancellation/replacement
- same-document or equivalent lightweight navigation if supported
- invalid URL handling
- unsupported scheme handling
- back
- forward
- reload
- redirect chain
- redirect loop / excessive redirects

### B. HTTP / HTTPS

Cover at minimum:

- HTTP success
- HTTPS success
- non-2xx response handling
- connection refused
- timeout
- malformed response where practical
- DNS/host-resolution failure path
- TLS/certificate failure path
- redirect HTTP -> HTTPS if supported
- deterministic local fixtures whenever possible

Avoid internet-dependent CI tests unless there is no practical alternative.

### C. Tabs / Browser State

Cover where supported:

- create tab
- close tab
- switch active tab
- multiple independent tab histories
- reopen/restore behavior if in M18 scope
- state isolation between tabs

### D. History

Cover:

- history entry creation
- ordering
- duplicate/reload behavior according to specification
- persistence
- reload after process restart
- corrupt or invalid persisted state handling

### E. Bookmarks

Cover:

- create
- read/list
- update if supported
- delete if supported
- duplicate behavior
- persistence
- restart restoration
- malformed persisted state handling

### F. Session Persistence / Restore

Cover:

- clean shutdown persistence
- restart restore
- multi-tab restore if supported
- active tab restore
- navigation state restore where specified
- missing state file
- empty state
- incompatible/old state version if versioning exists
- partially corrupt state

### G. Recovery / Reliability

Create deterministic tests or harnesses for:

- interrupted persistence/write
- process termination between state changes
- invalid saved data
- unavailable storage
- read/write failure propagation
- one failing tab/request not corrupting unrelated persisted state
- repeated open/close cycles
- repeated restore cycles

Do not claim true crash-safety unless the test actually proves the required guarantees.

### H. Download / Upload

Where M18 scope supports them, cover:

- download initiation
- destination/result signaling
- failure path
- upload/file-selection handoff
- cancellation
- unavailable path
- filename/path edge cases

Use test fixtures rather than real external services.

### I. Clipboard / IME

Where current M18 scope supports them, cover:

- copy
- paste
- empty clipboard
- text input
- IME composition boundaries
- confirmed/committed text
- cancellation where exposed

If host automation cannot faithfully validate a user-input path, provide the strongest automated contract test possible and document the remaining manual acceptance step.

---

## 5. Deterministic Test Infrastructure

Prefer deterministic local infrastructure.

Implement or reuse:

- local HTTP fixture server
- local HTTPS fixture if feasible
- known redirect endpoints
- delayed/timeout endpoint
- error/status endpoints
- deterministic HTML fixtures
- upload/download fixture endpoints
- isolated temporary storage directories
- seeded state files
- corrupt-state fixtures

Tests must avoid depending on public websites for routine CI.

All temp files, sockets, and servers must be cleaned up reliably.

---

## 6. M18 Feature-Gate Verification

Where the repository uses feature flags or target-specific gates, verify both sides when practical.

Examples:

- M17 baseline does not accidentally enable M18 persistence/browser behavior.
- M18 build enables the intended browser/storage interfaces.
- host-only fixtures do not leak into the Nagi target.
- test-only features do not become production defaults.

If current CI already checks a portion of this, extend rather than duplicate it.

---

## 7. CI Integration

Add focused CI coverage without making every unrelated job unnecessarily expensive.

Preferred structure:

1. fast host/unit tests
2. deterministic browser acceptance harness
3. target compile/check gates
4. optional heavier integration stage

The CI must make it easy to identify:

- which acceptance case failed
- whether the failure is implementation missing vs regression
- whether failure belongs to M18-A, M18-B, main integration, or M18-C infrastructure

Do not mask failures simply to obtain a green check.

Temporary expected-failure/skip handling is acceptable only when:

- the production capability is genuinely not merged yet
- the reason is machine-readable or clearly logged
- the test remains present
- the expected blocker is named

As M18 functionality lands, convert blocked/skipped cases to required PASS.

---

## 8. Compatibility With Active M18 Workstreams

The harness should be written against stable contracts, not internal implementation details.

When M18-A or M18-B changes:

- adapt tests to the approved public contract
- do not duplicate their implementation
- do not silently rewrite product semantics in the test to make it pass

When a real product bug is discovered:

1. reduce it to a minimal failing test
2. commit the test/harness work in M18-C
3. identify the likely owning workstream
4. if the fix is trivial and inside the minimal test-seam allowance, fix it here
5. otherwise record the blocker precisely for the integration owner / owning branch
6. continue implementing other independent acceptance coverage instead of stopping

---

## 9. State Tracking

If DF-01 workstream state conventions are available, create/update the dedicated M18-C state in the proper location.

State should record at minimum:

- workstream id
- branch
- base SHA
- current HEAD
- status
- dependencies
- ownership
- completed acceptance areas
- blocked acceptance areas
- test commands
- last verified result
- known cross-workstream blockers

Preferred statuses:

- `ACTIVE`
- `PARTIAL`
- `BLOCKED`
- `PASS`

Do not mark `PASS` while required acceptance rows remain unverified.

If the shared registry is owned by an integration owner and editing it would violate ownership rules, do not change it. Instead prepare the exact registration proposal in the workstream state/report.

---

## 10. Required Verification

Use the repository's actual commands, but the final verification should include the strongest applicable combination of:

- formatting
- lint / Clippy with warnings treated as errors where applicable
- unit tests
- browser acceptance tests
- persistence tests
- host integration tests
- relevant target `check`
- feature-enabled and feature-disabled checks where appropriate
- CI workflow validation
- clean Git status

If a full target/QEMU run is feasible and relevant to this harness, run it.

If it is not feasible from the current environment, do not fake completion; record the exact unverified gate and continue all other possible verification.

---

## 11. Acceptance Criteria

M18-C reaches `PASS` only when all of the following are true:

1. A reproducible M18 browser acceptance harness exists.
2. Navigation acceptance coverage exists.
3. HTTP/HTTPS success and failure-path coverage exists to the extent supported by M18 contracts.
4. History persistence coverage exists.
5. Bookmark persistence coverage exists.
6. Session save/restore coverage exists.
7. Reliability/recovery coverage exists for invalid/interrupted persisted state.
8. Tab/state-isolation coverage exists where tabs are in M18 scope.
9. Download/upload acceptance exists where those functions are in M18 scope.
10. Clipboard/IME acceptance exists at the strongest automatable level available.
11. Tests use deterministic local fixtures wherever practical.
12. CI exposes clear case-level failures.
13. M17-vs-M18 feature-gate behavior is protected where applicable.
14. No parallel production implementation of M18-A/M18-B was introduced.
15. Required tests pass against the current integrated M18 implementation once dependencies are available.
16. Workstream state is updated.
17. Changes are committed.
18. Branch is pushed.
19. Remote branch HEAD matches local committed HEAD.
20. Working tree is clean.

If dependencies are still unfinished, remain `PARTIAL` or `BLOCKED`, but complete every independent acceptance item possible before stopping.

---

## 12. Continuous Execution Rule

This workstream must not stop after:

- repository inspection
- architecture review
- test plan creation
- TODO generation
- identifying missing production functionality
- a single failing test
- one CI failure

Instead, continue:

**inspect -> implement harness -> run tests -> diagnose -> fix harness/test seams -> rerun -> expand coverage -> update state -> commit -> push**

When one acceptance area is blocked by M18-A/M18-B, move immediately to another independent area.

Stop only for a genuine external hard blocker that prevents all meaningful progress in this workstream.

---

## 13. Final Report Format

At completion or genuine hard block, report:

### Status
`PASS`, `PARTIAL`, or `BLOCKED`

### Branch
- branch
- base SHA
- HEAD
- remote HEAD
- clean/dirty

### Implemented
Concise list of acceptance infrastructure and tests created.

### Verification
Exact commands and outcomes.

### Acceptance matrix
For each major area:

- Navigation
- HTTP/HTTPS
- Tabs
- History
- Bookmarks
- Session restore
- Reliability/recovery
- Download/upload
- Clipboard/IME
- CI/feature gates

show `PASS`, `BLOCKED`, `SKIPPED`, or `NOT IMPLEMENTED`.

### Remaining blockers
Name the owning dependency/workstream and the exact missing contract.

### Commits
List commit hashes.

### Push
Confirm remote branch is updated.

---

## 14. Non-Goals

M18-C is not responsible for declaring the whole Nagi OS complete.

M18-C is not an excuse to merge or rewrite other active workstreams.

Its job is narrower:

> Make M18 measurable, reproducible, regression-resistant, and ready for fast final integration.
