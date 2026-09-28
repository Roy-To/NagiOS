# Nagi OS 0.2 — Diagnostics / Observability Foundation Workstream

## 1. Workstream

- Proposed ID: `DIAG-01`
- Name: `Nagi 0.2 Diagnostics / Observability Foundation`
- Proposed branch: `codex/0.2-diagnostics`
- Target: Nagi OS 0.2
- Type: Independent foundation
- Priority: Medium/High
- Final target state: `PASS`

同等workstreamがあれば重複作成せず継続する。

---

## 2. Purpose

Nagi OSおよびfirst-party software共通のdiagnostics / observability基盤を構築する。

以下を技術的に追跡可能にする。

- subsystem failure
- error timing
- service health
- repeated failures
- operation context
- safe developer diagnostics
- application/runtime/capability/model等のfailure分類

ただしdiagnosticsをユーザー監視システムにしない。

---

## 3. Diagnostics と AI Activity Ledger の分離

### Diagnostics

目的:

- troubleshooting
- health
- errors
- developer support
- crash evidence

### AI Activity Ledger

目的:

- meaningful AI actions
- user-visible AI history
- provenance
- undo/Wayback integration

raw diagnosticsを自動的にActivity Ledgerへ流さない。

semanticな一部eventだけをbridge経由でpromotionできる設計にする。

---

## 4. Relationship to Nagi 0.1

Nagi 0.1 active milestoneを無断変更しない。

0.1 loggingを全面rewriteしない。

0.2 diagnosticsはhost-testable foundationとして独立実装する。

---

## 5. Structured Event Model

最低限表現可能にする。

- timestamp
- severity
- subsystem
- stable event code
- message/template ID
- correlation ID
- operation/session ID
- component/source
- structured fields
- error category
- privacy classification

free-form messageだけをmachine identityにしない。

---

## 6. Severity

例:

- Trace
- Debug
- Info
- Warning
- Error
- Critical

既存conventionがあればそちらを優先。

---

## 7. Subsystem Identification

例:

- kernel
- runtime
- UI
- filesystem
- Albert
- model runtime
- capability/security
- networking
- app
- Wayback
- update/install

extensibleにする。

---

## 8. Correlation

関連eventsをcorrelation可能にする。

例:

- app launch
- model request
- file operation
- AI action
- service startup
- recovery operation

correlation ID自体にprivate dataを入れない。

---

## 9. Redaction / Sensitive Data

default-safeにする。

絶対に安易に記録しない。

- password
- token
- authentication secret
- capability secret
- credential
- cryptographic private material

不要なら避ける。

- full document content
- complete prompts
- raw file content
- sensitive path fragments

typed safe fields / redaction wrapper等を検討する。

redaction testを必須とする。

---

## 10. Sinks

最低限:

- memory/test sink
- development/console sink

将来拡張:

- local persistent log
- diagnostic bundle
- diagnostic viewer

final filesystem/UIがなくてもfoundationを進める。

---

## 11. Buffering / Failure

diagnostics自身のfailureでsystemをcrashさせない。

考慮:

- bounded buffer
- full buffer
- sink failure
- malformed optional data
- unavailable storage

critical pathでdiagnostics出力待ちにより無期限blockしない。

---

## 12. Health Model

例:

- Healthy
- Degraded
- Unavailable
- Unknown

表現:

- current state
- stable reason code
- last meaningful transition
- optional safe detail

localized proseそのものをstate identityにしない。

---

## 13. Health Registry

可能ならsubsystem health registryを実装。

test:

- registration
- transition
- multiple subsystem
- unavailable provider
- duplicate registration
- removal/stale behavior

---

## 14. Crash / Error Reporting Contract

表現可能にする。

- process/component
- failure category
- stable error code
- safe context
- correlation
- version/build metadata
- optional backtrace reference

full kernel crash dumpは本foundationの必須条件ではない。

Internet uploadをdefaultにしない。

---

## 15. Developer Diagnostics Snapshot

sanitized diagnostic snapshot/bundle APIを準備する。

将来含められる情報:

- build/version
- health summary
- safe recent events
- enabled components
- sanitized environment

secret除外可能であること。

---

## 16. Stable Event IDs

English message textとは別にstable codeを持つ。

概念例:

`MODEL.RUNTIME.LOAD_FAILED`

既存conventionがあれば従う。

---

## 17. Localization

machine event identityとuser-facing localizationを分離する。

英語internal baseline / 日本語official equal-class supportを両立する。

---

## 18. Activity Ledger Boundary

explicit semantic bridgeを定義する。

promotion候補例:

- AI changed a file
- AI launched application
- AI restored Wayback state

diagnosticsのみの例:

- retry count
- parser warning
- buffer occupancy
- internal timeout
- renderer debug

Activity Ledger本体は所有範囲でなければ実装しない。

---

## 19. Non-scope

- cloud telemetry platform
- automatic log upload
- advertising analytics
- user tracking
- Wayback replacement
- Activity Ledger replacement
- M18 rewrite
- complete kernel crash dump
- secret storage
- unbounded logs

---

## 20. Implementation Sequence

### Phase A — Audit

確認:

- existing logs
- error types
- tracing
- health
- runtime conventions
- Wayback/Activity contracts
- DF-01 ownership

その後そのまま実装へ進む。

### Phase B — Event Schema

実装:

- severity
- event code
- subsystem
- structured fields
- correlation
- privacy classification

### Phase C — Sinks

実装:

- memory/test sink
- development sink

### Phase D — Redaction

安全field/redactionを実装しtests追加。

### Phase E — Health

health types / registryを実装。

### Phase F — Error / Crash Contract

portable error reportを実装。

### Phase G — Developer Snapshot

sanitized snapshot/bundle abstractionを実装。

### Phase H — Activity Bridge

explicit semantic adapterを実装。

### Phase I — Verification

- tests
- fix
- retest
- lint
- format
- state
- commit
- push

---

## 21. Required Tests

- structured event
- stable event ID
- severity
- subsystem
- correlation
- redaction
- secret fields
- sink failure
- bounded buffer
- health registration
- health transition
- duplicate registration
- snapshot sanitization
- Activity bridge filtering
- malformed data
- diagnostics failure safety

---

## 22. Acceptance Criteria

- [x] structured event model
- [x] stable event codes
- [x] severity
- [x] subsystem
- [x] correlation
- [x] redaction/privacy mechanism
- [x] redaction tests
- [x] memory/test sink
- [x] development sink
- [x] safe sink failure
- [x] health model
- [x] health registry
- [x] crash/error report contract
- [x] sanitized diagnostic snapshot
- [x] localization separated from event identity
- [x] explicit Activity Ledger boundary
- [x] raw diagnostics not automatically promoted
- [x] host tests PASS
- [x] format/lint PASS (owned diagnostics/CLI/bootstrap packages)
- [x] docs updated
- [x] state updated
- [x] committed
- [x] pushed
- [x] working tree clean

---

## 23. Completion Report

- Status: `PASS` for DIAG-01 host-testable foundation
- Workstream ID: `DIAG-01` (`diagnostics` state registry entry)
- Branch: `codex/ws-diagnostics`
- Base SHA: `0bc2ab2f19a915def9bc4af3ce7b42b81be05684`
- HEAD SHA: `2e748634272ab27a64fb6b55e02ca458c5e0a00e` (verified implementation and cross-platform redaction fix)
- Diagnostics architecture: shared `nagi-diagnostics` crate provides bounded structured events, sanitized memory/development sinks, buffered crash capture, service health registry, portable `ErrorReport`, and `DiagnosticSnapshot` contracts.
- Redaction: sensitive/credential fields, inline credential forms, path-like values, and absolute source paths are sanitized; serialized sink paths revalidate safe event data.
- Health: scoped health states and summaries support transitions, duplicate rejection, removal, and stale handles.
- Activity Ledger boundary: only explicit typed semantic candidates can cross `ActivityBridge`; raw diagnostics are not automatically promoted and no Activity Ledger implementation was added.
- Tests: 72 CLI unit tests, 22 CLI integration tests, and 27 diagnostics tests pass. Diagnostics/verify/host-smoke commands and pinned dependency fetch pass.
- CI: Ubuntu host passed in [run 36395201986](https://github.com/RT-NISH/NagiOS/actions/runs/36395201986). Its Windows workspace test found that host-native path parsing leaked POSIX absolute source paths; this was fixed by commit `2e748634272ab27a64fb6b55e02ca458c5e0a00e` and the full focused suite passed locally. The first run's independent M17 target job was cancelled by the next push. Retest run [36396121951](https://github.com/RT-NISH/NagiOS/actions/runs/36396121951) has Ubuntu and Windows host jobs passed; its target job is building Nagi user init. Neither result changes M17 status.
- Deferred integration: register the authoritative DF-01 validator when its registry enters this source line; keep guest persistence/collection behind M30 PASS and an explicit integration release gate.
- Blockers: none for the DIAG-01 acceptance. Repository-wide `./nagi fmt` reports existing fetched Servo formatting differences; full host-workspace test/Clippy commands hit x86-64 `libnagi` assembly on this aarch64 development host. Scoped affected-package format/lint and tests pass.
- Push status: implementation commit `6705de462704a03e0c059ac92eb942eacd7a39ea` and cross-platform redaction fix `2e748634272ab27a64fb6b55e02ca458c5e0a00e` are pushed to `origin/codex/ws-diagnostics`; final docs/state checkpoint commit will follow.
