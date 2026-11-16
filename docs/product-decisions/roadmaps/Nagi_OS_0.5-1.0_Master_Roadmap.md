# Nagi OS 0.5–1.0 Master Roadmap

> **2026-11-16 近距離ファイル共有:** [DEC-064](../DEC-064-nagi-share-nearby-file-transfer.md)で、Nagi Share（仮称）の純正OS機能としての採用を確定した。Local／Offline-firstの単発ファイル直接転送はInternet・Cloud account・LLMを必須とせず、DEC-014のWorkspace移行・同期、Team／Organization／Remote Agentとは別範囲である。技術・UI詳細・対応版／端末・性能は未確定。既存の0.8 Multi-device段階案への割当てや、モバイル対応・速度を保証せず、認可・情報区分・所有／Profile境界とGateを維持する。本機能のHandoff拡張はアイデア段階であり、既存Continuity構想は維持する。詳細は[ロードマップ索引](README.md#近距離ファイル共有の製品方針2026-11-16)を参照。

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

## Linux基盤への適用注記（2026-11-13）

[DEC-042](../DEC-042-linux-kernel-transition.md)のLinux kernel選択、[DEC-043](../DEC-043-nagi-userland-independent-os.md)のNagi専用userland／既存distribution基盤非採用／個別OSSの選択的再利用はCONFIRMED。[DEC-044](../DEC-044-linux-migration-development-authorization.md)と[移行ADR](../../decisions/2026-11-12-linux-migration-development-scope.md)はPrivateでの対象限定の移行設計・実装・検証を承認済みである。これは製品の恒久構成・正式切替やrelease activationと別の承認であり、具体的な実開始・統合成功は対象refと証拠で確認する。

割当済みの移行開発では、本文に残る旧独自kernel完成・旧M30順序を一律の着手前提としない。既存release Gate・Workstream owner/path・安全境界・SSD/Git等の個別制限は維持する。具体的Linux構成・互換方式・package・適用版・切替日を本改訂で確定しない。開発用の可逆的選択はDEC-044の委任内で記録・検証でき、全体設計の完成を新たな一律開始Gateにしない。

[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)と[Private 0.2補足](../../Nagi_OS_0.2_Linux_Migration_Addendum.md)を併読する。Private内容の公開・配布は別承認を要する。

1.0の一般公開可能な品質・安定性・security・開発者体験／配布目標を維持する。§4の版別配置はPROVISIONALのまま。Linux移行を特定releaseへ割り当てず、公開日や互換保証を加えない。0.3 Memory Engineと0.4 Role/Agent Gateを従来どおり区別する。


**Project:** NAGI — Native Agentic General Interface  
**Document Type:** Long-range product and architecture roadmap proposal  
**Target:** 0.5–1.0, after the initial 0.4 bounded Nagi Role workflow  
**Status:** CONFIRMED direction and high-level 1.0 target inherited from approved product decisions / revised 0.3 roadmap; detailed version scope and ordering are PROVISIONAL  
**As of:** 2026-11-16  
**Companion:** [0.3 Revised Architecture Roadmap](Nagi_OS_0.3_Architecture_Roadmap_Revised.md) / [0.4+ Multi-Agent Organization Roadmap](Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md)  
**Decision record:** [DEC-021 — 0.5–1.0 roadmap boundaries and sequencing](../DEC-021-roadmap-0.5-1.0.md)

> This document proposes a path beyond 0.4. It does not replace an implementation specification, approve a release, activate a workstream, or change an existing acceptance gate. Treat the version-by-version allocation below as provisional until a later decision is recorded.

## 1. Purpose and status labels

> **2026-11-16 connectivity update:** [DEC-066](../DEC-066-device-connectivity-lan-first.md) confirms same-Wi-Fi/LAN Nagi devices as the initial connectivity target and a shared Device Connectivity/Trust/Transport abstraction without hardcoded transports. Nearby direct Wi-Fi and remote internet connectivity remain extensible options; initial remote support, always-on Cloud, online Nagi accounts and operator-run servers are not required. Preserve offline-first, authentication/trust, encryption, explicit permission, current User/Profile/Resource Permission and Privacy, revocation, external communication scope, and DEC-014's separate migration/sync/cloud grants. No implicit transfer, synchronization or Task replay; Clipboard remains unadopted. Protocols, UI, devices, operations/costs, release timing and performance remain provisional. This does not assign the feature to 0.8 or any other release or change existing gates.

This roadmap carries the user-approved direction for Nagi beyond 0.4 into a planning view through 1.0. It combines the staged Agent Team proposal with confirmed product boundaries in DEC-009 through DEC-020, while keeping version-by-version assignments, sequencing, and implementation details open.

Use these labels throughout:

- **CONFIRMED** — a product boundary already agreed in a linked decision record or the approved direction summarized here.
- **PROVISIONAL** — a proposed milestone, version assignment, or sequence that still needs review against implementation evidence.
- **OPEN** — a product or technical choice that must be decided before it becomes an implementation contract.

A CONFIRMED boundary does not make every roadmap item CONFIRMED. Version mapping details, sequencing, acceptance criteria, and implementation outcomes in Section 4 remain PROVISIONAL. The revised 0.3 Roadmap already establishes the high-level 1.0 target of a general-public-ready Nagi OS; DEC-017 also confirms a public developer environment and app distribution foundation as a 1.0 goal.

## 2. Confirmed direction carried forward

### Near-term product priority (2026-11-16, DEC-074 follow-up)

Prioritize establishing Nagi as an OS where AI works naturally as a standard OS capability. Hierarchical Agent Organization remains a non-priority long-term vision. The proposed organization phases below do not make hierarchy a near-term priority or a prerequisite for 1.0. Delivery versions, hierarchy depth and implementation sequencing remain PROVISIONAL. Preserve the existing 0.3/0.4 gates and the initial single-Nagi `builder` Role workflow. See [DEC-074](../DEC-074-human-led-agent-organization.md) for the confirmed priority and illustrative experience-validation candidates.

### Release boundaries

- **0.3** is the single-Personal-Agent generation: Nagi uses the user's Profile and Memory, performs approved cross-application work through typed Public Actions, and preserves offline and permission boundaries. The **Hardware-Adaptive Memory Engine** is a major 0.3 technical topic under **DEC-013**: host-only benchmark preparation may precede it, but formal runtime implementation stays behind the existing 0.3 specification and activation gates and is not shifted to 0.5. Canonical Memory remains user-owned and model-independent; hardware-dependent indexes, caches, and Context adapters are derived and rebuildable. Hardware transitions must not require relearning canonical Memory. The revised 0.3 roadmap's Implementation Specification gate remains in force.
- **0.4** follows 0.3 acceptance and a separate 0.4 activation decision. **DEC-015 now confirms mode C:** initially, the single Personal Agent Nagi switches to the programming Role `builder` for scoped coding Tasks. Task state, workspace, permission scope and evidence remain separately tracked even though a separate Agent identity and separate LLM are not required. The earlier two-Agent Nagi + Builder proposal is superseded for the initial workflow. A specialist Agent may later execute the same `builder` Role after explicit architecture, permission/delegation, runtime and acceptance gates. **DEC-016** retains bounded autonomy for edits/tests but excludes Git commit/push by default; Nagi applies evidence-based risk-aware completion. The 0.4+ roadmap governs staged future Team expansion without treating it as initial deployment.
- **Long-term AI organization direction (DEC-074):** The product aims for one human manager overseeing one or more AI supervisors, with specialized worker Agents under them; the organization may grow in size and depth as device resources allow. This is a confirmed high-level direction, not a release commitment. Nagi remains the consistent user-facing gateway under DEC-021, while its exact relationship to multiple supervisors is OPEN.
- **0.5–1.0** may extend this foundation only after the applicable prior release has passed its Target Runtime acceptance and the next release has been explicitly activated.

### Product behavior and safety

- Nagi remains the user's primary and consistent interface. Existing GUI operation remains complete and available when AI is unavailable; Agent Roles and Models do not grant authority.
- Side effects use typed Public Actions and current OS Policy. Delegation narrows scope, is revocable, has bounded resources and time, and propagates Pause / Cancel / Stop. Results include provenance and honest partial-failure reporting.
- Offline-first use, user-owned data, and strict separation between private Profile Memory, Task data, Agent data, Team data, and Organization data continue.
- **DEC-009:** Nagi Home is the default, dialogue-centered workspace; Nagi Desktop retains full conventional GUI use. Users can switch between them without losing conversation, Task, Workspace, or app state; launching an app from Home opens Desktop. “Continue where you left off” restores state but does not resume a Task without explicit user action.
- **DEC-010:** Intent-first UI remains the direction. A new Workspace may receive a purpose-fit layout across available displays; major changes to an existing Workspace are proposed for user choice. Manual placement, pinned windows, and existing Workspaces are respected. On display disconnect, windows remain visible and may return to their prior layout after reconnect unless the user changed the layout meanwhile.
- **DEC-011:** Task approval uses a hybrid conversation-card and expanded confirmation experience. Voice can help revise or approve only within current authentication and Policy; critical operations still require explicit on-screen approval. Detailed progress UI and risk thresholds remain open.
- **DEC-012:** Workspace names and ordering remain user-controlled; smart grouping and natural-language recall may help users return to prior work. Nagi may recommend a likely match with reasons and show alternatives, but the user chooses what to restore. “Past” classification never deletes a Workspace, and restoration never restarts an interrupted AI Task.
- **DEC-014:** Workspace restoration state is saved on important state changes with periodic supplemental saves. Named checkpoints, ordinary autosaves and pre-restore protection states are distinguishable and searchable across Workspaces using only permitted metadata; previews show what can be restored, and the user chooses the checkpoint and destination. Restore defaults to a new Workspace; restoring into the original first protects its prior state, keeping its latest protected version plus multiple generations for an unspecified period. Old automatic history and expired protected generations may be pruned under the retention rules; Workspaces, named checkpoints, and the latest protected state are not automatically deleted. Restoration follows current Permission / Privacy and manual placement, does not scan file contents as a search shortcut, rewind files, or resume Tasks. Exact timing, retention, and version assignment remain open.
- **DEC-015 / DEC-016:** `builder` is the confirmed programming Role. Initially Nagi executes it by Role switching (mode C), within a separate scoped Task/workspace/authorization boundary; an independently registered specialist Agent is optional later, not a release prerequisite. Evidence-based work, risk review, finite retries and separate authorization for Git commit/push remain.
- **DEC-018:** Builder edits in an isolated area. Before apply, Nagi checks the approved scope, current Permission, target Resource revision / conflict state, verification evidence, and recoverable protection of the original file contents. Only ordinary, in-scope, verified, non-conflicting changes with successful protection may be conditionally applied through a re-authorized Public Action; high-risk, failed, uncertain, conflicting, permission-denied, or unprotected changes go to the user. Git commit / push and production changes remain separately authorized.
- **DEC-019:** Task-specific work data may be cleaned up only after safe application and verification, and after required diffs, evidence, Activity records, and recovery data are retained. Failed, partial, pending, stopped, interrupted, or unknown work and protected data stay available. Cleanup first checks ownership, active use, references, permissions, and retention policy; uncertain targets are deferred. Retention limits remain open.
- **DEC-036:** Builder-created applications default to personal use. If the user asks to share or publish, Nagi reviews source/package contents, dependencies, personal or secret data, settings/learning Memory, licenses, and required Permissions, then may export source or a package after needed separation/verification and explicit user approval. Limited sharing with named recipients and general publication through GitHub/Store are distinct scopes. Creating a share does not itself authorize external transmission or publication; recipients independently verify and reapprove Permissions. The source user's Memory, Permissions, and credentials never transfer. Store implementation, complete secret removal, and automatic license classification are not guaranteed; version assignment and implementation details remain provisional and existing gates stay in force.
- **DEC-041:** Normal uninstall of a builder-created app removes the installed application while retaining user data, settings, protected source, and DEC-040 development projects/history by default. Complete deletion requires the user to inspect and select each disclosed target after Nagi explains known shared references, impact, and recoverability; current OS Permission / Policy and the canonical Public Action path still govern. Do not delete shared data, in-progress work, recovery information, another user's data, or uncertain targets. Reinstall/rebuild and restoration of old permissions depend on artifacts, manifests, compatibility, and current authorization and are not guaranteed. Retention, UI, implementation, and version assignment remain provisional; DEC-014, DEC-019, DEC-040 and existing gates remain in force.
- **DEC-017:** The developer ecosystem is a staged goal: host-side preparation in 0.1, the existing 0.2 app-platform foundation, deeper Public Action integration in 0.3, and a public developer environment plus app distribution foundation by 1.0. Improving the portability and migration path for existing Rust, C, and C++ apps is also a product goal; it does not guarantee that every application or framework will run unchanged. AI-coding-optimized developer experience is a confirmed product priority; its specific catalogs, CLI, sandbox and verification mechanisms remain PROVISIONAL. The Store is not the only possible distribution path. Supported environments, packaging, signing, distribution details, and acceptance remain open; existing 0.2/0.3 specifications and gates remain authoritative.
- **DEC-020:** AI starts with a small, structured Context relevant to the current intent / Task and may request more detail through typed Resource references. Every retrieval rechecks current Permission / Policy, classification, provenance, owner, scope, target Resource, and revision; a reference or request does not expand authority. Do not inject all files, logs, raw screen dumps, or Memory by default. Preserve DEC-002's existing authorized Cloud sharing rules, including its allowed non-sensitive Memory Context, and permit broader initial Context when an explicitly approved Task needs it within current Policy. Use contractually exposed Events and typed Public Action / Service APIs through existing IPC / Provider boundaries. Event-driven is the default; deterministic OS / app services handle deterministic work before LLM inference. Events alone do not authorize inference, notifications, screen interruptions, or Actions. Human GUI remains available; screenshot recognition with coordinate clicking is not the normal native path. Efficiency is a measured design goal, not a demonstrated saving or guarantee; schemas, budgets, retrieval triggers, event subscriptions, scheduler, fallback, metrics, and release assignment remain open.

These behaviors do not authorize implementation in 0.1, 0.2, 0.3, or 0.4, and they do not modify any existing release gate.

## 3. Planning principles

1. Deliver the smallest useful local workflow before expanding users, devices, or organizational scope.
2. Keep sequential execution as the supported baseline. Enable parallel work only on hardware profiles that pass measured resource and isolation checks; profile selection and switching must use measured capability and load, not device name alone.
3. Keep AI operation event-driven by default and deterministic-first. Measure CPU, RAM, latency, power, token use, and outcome quality before making efficiency claims. Preserve normal GUI use when AI is unavailable.
4. Add team and organization data only with explicit ownership, membership, sharing, retention, export, deletion, and offboarding rules.
5. Require reproducible Target Runtime evidence for authority boundaries, cancellation, recovery, offline behavior, and artifact verification.
6. Keep DEC-013's Hardware-Adaptive Memory Engine work within 0.3's gated scope. Benchmark preparation is not runtime acceptance and does not justify moving formal 0.3 implementation into 0.5.
7. Prefer deferring a capability over weakening a permission boundary or treating a proposal as a committed release requirement.
8. Carry forward the revised 0.3 Roadmap's high-level 1.0 target: a Nagi OS with quality, stability, security, compatibility, and developer experience suitable for general release. DEC-017 also sets a public developer-environment and app-distribution foundation goal for 1.0. Improving portability and migration for existing Rust, C, and C++ apps is a product goal, not a promise of unchanged compatibility. Measurable thresholds, detailed scope, release decision/date, support matrix, and Store policy remain open. Organization-scale features, Cloud services, and multi-device operation are not prerequisites unless separately approved.

## 4. Provisional version map

This is a planning hypothesis, not a delivery promise. Version mappings, sequencing, and detailed outcomes are PROVISIONAL, except DEC-017's confirmed 1.0 developer-environment and app-distribution foundation goal. Phase references point to the existing 0.4+ roadmap's Phase A–F and may be re-sliced when implementation evidence is available.

| Version | Proposed focus | Candidate outcomes | Status and dependency |
|---|---|---|---|
| **0.5** | Candidate Phase D: extend dependable Nagi + `builder` Role workflow and evaluate independent specialist Agents | If this mapping is adopted, add Research and Review / Verify roles around Builder; return structured reports that compare worker output to deterministic evidence. Agent or Nagi success claims alone are not acceptance evidence, and conflicts must not overwrite newer user data. Preserve DEC-018: edits occur in an isolated area; before apply, Nagi checks current Permission, approved scope, verification evidence, target Resource revision / conflict state, and recoverable protection of the original file contents. Only ordinary, in-scope, verified, non-conflicting changes with successful protection may be applied through a re-authorized Public Action; uncertain, high-risk, failed, conflicting, or unprotected changes go to the user. Git commit / push and production changes remain separately authorized. Preserve DEC-019: cleanup follows successful safe application and verification only after required diffs, evidence, Activity, and recovery data are retained; failed, partial, pending, stopped, interrupted, or unknown work stays available. Also preserve DEC-040: formally retained builder app development projects and records needed for future changes, verification, or recovery remain local-first long-term protected data and are excluded from Task-temporary/cache cleanup. App binaries, source projects, user data, and Canonical Memory remain separate; saving does not authorize Git commit/push, cloud backup, external transfer, or sharing. Exact format, retention, capacity, Git use, migration format, and release assignment remain provisional. Validate offline completion, finite retry, Stop, provider restart, and supported Wayback behavior. Keep one user, one device, and sequential Role-Task execution as the initial baseline; independent specialist Agents and their delegation require separate validation. | **PROVISIONAL.** Requires accepted 0.4 Target Runtime evidence and separate 0.5 activation. If evidence or governance is not ready, defer or split Phase D rather than implying the initial 0.4 Role workflow includes these roles. |
| **0.6** | Resource-aware scheduling and AI event path | Measure device capability and current load to choose processing paths; keep sequential work as baseline and introduce bounded concurrency only where budget, isolation, cancellation, and foreground responsiveness pass. Use deterministic services for deterministic work and LLM inference only where needed. Evaluate the event-driven default, staged Context acquisition, event subscription and aggregation, and supported legacy-app fallback. Measure CPU/GPU, RAM, storage, latency, power, token use, and quality/outcomes; do not assume or claim savings before evidence. Rebuild hardware-dependent indexes / Context adapters when needed while preserving canonical Memory. | **PROVISIONAL.** Follows 0.5 workflow evidence; profile definitions, scheduler, event contracts, fallback, metrics, and thresholds are OPEN. |
| **0.7** | Candidate Phase F governance and Workspace continuity | Define Team / Organization artifact ownership, membership and revocation, scoped sharing, promotion, retention, export, deletion, and audit, initially within the local single-user model. Consider DEC-014 restoration UX: event-based plus periodic autosave; distinguish and search Workspace and checkpoint history from permitted metadata; preview restore availability; default to a new Workspace; protect the latest pre-restore state and multiple generations for an unspecified period; prune only eligible old automatic/protection history; never auto-delete Workspaces or named checkpoints. Respect current permissions, manual layout, and fixed windows; do not scan file contents for history search, rewind files, or resume Tasks. Consider multi-user / multi-device only after local single-user invariants pass and a separate gate is approved. | **PROVISIONAL.** Version assignment is a proposal. Exact storage, retention, search, and UI details remain OPEN; multi-user activation is not implied. |
| **0.8** | Optional multi-user or multi-device expansion gate | If separately approved, validate identity isolation, membership changes, sync consent, conflict handling, backup/restore, and offline behavior before enabling a limited expansion. If not approved, continue hardening the local single-user Team without blocking 0.9 work. | **PROVISIONAL / optional.** Scope, data transfer, and whether this checkpoint exists are OPEN. |
| **0.9** | Stabilization and release-candidate evidence | Integrate the approved 0.5–0.8 scope; close security and reliability findings; verify migration and compatibility; run reproducible Target Runtime acceptance for offline operation, authority, cancellation, recovery, and user-facing Task state. Avoid adding major new capabilities by default. | **PROVISIONAL.** Exit evidence and support matrix are OPEN until an implementation specification defines them. |
| **1.0** | General-public-quality Nagi OS and developer/distribution foundation | Carry forward the revised 0.3 Roadmap's high-level target for general-public-ready quality, stability, security, compatibility, and developer experience. Also pursue DEC-017's public developer environment and app distribution foundation goal, with AI-coding-optimized development and migration support for portable Rust / C / C++ applications as product goals. Exact quality thresholds, technical mechanisms, supported applications, toolchains, installation/package validation, signing, onboarding, migration guidance, channels, and acceptance evidence remain to be specified. The Store is not the only possible distribution route; this does not guarantee existing apps run unchanged. | **The high-level 1.0 target is retained from the revised 0.3 Roadmap; DEC-017's developer/distribution goal is CONFIRMED.** Measurable criteria, detailed scope, release decision/date, support matrix, and acceptance remain PROVISIONAL / OPEN. |

### Dependency outline

```text
0.3 Personal Agent accepted
        |
        v
0.4 bounded one-user / one-device Nagi builder Role workflow accepted and activated
        |
        v
0.5 candidate Phase D workflow with Research / Review / Verify roles
        |
        v
0.6 measured resource scheduling and optional bounded concurrency
        |
        v
0.7 candidate Phase F governance + Workspace continuity; multi-user only after local invariants pass
        |
        +---- 0.8 optional multi-user / multi-device decision gate
        |
        v
0.9 release-candidate integration and evidence
        |
        v
1.0 general-release-quality bar + developer / distribution foundation
```

The arrows describe a proposed dependency order, not fixed release dates. If evidence or governance is not ready, a capability moves to a later version without weakening its gate.

## 5. Cross-release acceptance themes

Later implementation specifications should turn these themes into measurable criteria without inventing thresholds in this roadmap.

- **Authority:** Every Action is checked by the OS at execution time; delegation cannot exceed the current grant; revocation and expiry fail closed.
- **Task lifecycle:** Root and child Task state, retry, cancellation, partial completion, and result-unknown outcomes remain visible and auditable.
- **Evidence:** Worker output is a provenance-bearing proposal or artifact. Review requires reproducible checks, not an Agent's claim of success.
- **Memory and ownership:** Private Profile Memory is not copied into Agent, Team, or Organization context without an explicit applicable Policy and grant.
- **Offline and accessibility:** Local operation and ordinary GUI use remain viable when a Model, voice provider, network, or Agent is unavailable.
- **Workspace continuity:** Naming, grouping, layout, restoration, and Home/Desktop switching preserve user state and do not trigger Task execution.
- **Recovery and compatibility:** Backup, restore, migration, and rollback claims match what the providers can actually recover.
- **Responsiveness and resource use:** Foreground UI and voice interaction take priority over background Agent workloads; resource exhaustion is reported rather than counted as success. Evaluate measured hardware/load paths and track latency, CPU, RAM, power, token use, and task quality before claiming efficiency.
- **AI interaction path:** Begin with small structured Context and retrieve additional detail only through authorized references; recheck Permission, classification, provenance, ownership, scope, Resource, and revision each time. Preserve DEC-002's authorized sharing rules and approved-Task exceptions. Contract Events, deterministic-first processing, typed Public Action execution, and normal GUI availability remain consistent with DEC-020. Event detection alone does not authorize model inference, a notification, or an Action.
- **Hardware migration:** Hardware-dependent Memory indexes and Context adapters can be rebuilt from canonical model-independent Memory without changing its ownership, provenance, permissions, or forgetting state.

## 6. Candidate 1.0 readiness evidence

The 1.0 implementation specification should define supported configurations and objective evidence for:

1. Security and privacy review of all activated Agent, memory, and sharing scopes.
2. Target Runtime acceptance of the supported workflows, including offline fallback, cancellation, and recoverable failure.
3. Stable versioned interfaces and a migration path for persisted Profiles, Tasks, Workspaces, and artifacts.
4. User-visible controls to inspect, approve, pause, stop, recover, export, and delete data within the supported scope.
5. Reproducible installation, update, backup/restore, diagnostics, and developer onboarding for supported targets.
6. A usable public developer environment and validated app distribution flow, with supported channels and Store role explicitly documented; the Store is not the only required route.
7. An AI-coding-optimized developer experience whose chosen catalogs, CLI, sandbox, and verification mechanisms are explicitly specified and accepted, without promoting current proposals into fixed contracts.
8. A documented migration path that helps developers assess and port suitable existing Rust / C / C++ applications and reuse portable code, without implying unchanged compatibility for every application.
9. Performance and resource limits measured on named reference profiles, with task quality/outcomes evaluated alongside latency, CPU, RAM, storage, power, and token use. No efficiency benefit is presumed before measurement.

These are candidate evidence categories. Exact thresholds, supported models, hardware, locales, and distribution channels remain OPEN.

## 7. Open decisions

The following items remain unresolved and must not be presented as settled requirements:

- Exact release numbering, boundaries, and whether the candidate stages span more than one version.
- The implementation version and timing of the confirmed DEC-009–012 user experience; the proposed milestone mapping does not make those assignments confirmed.
- Whether 0.8 exists as a separate milestone, and whether multi-user, multi-device, remote Agent, or synchronization features are included before 1.0.
- Team / Organization owners, administrators, members, guests, offboarding, recovery, governance, and dispute resolution.
- DEC-074's Agent hierarchy: number and roles of supervisors and workers, depth, Nagi's place in the hierarchy, resource-based expansion rules, and version assignment.
- Which data may be shared, for which purpose, with which retention, export, deletion, backup, and audit rules.
- Third-party Agent / Role packages, distribution, signing, review, and revocation.
- Supported local Models, voice providers, Hardware profiles, event-driven scheduler, deterministic/inference boundary details, parallelism, fairness, and numerical budgets.
- DEC-020's Context / Event schemas, event types and subscription rules, event aggregation and priority, Context acquisition limits, compatibility fallback for older apps, scheduler design, performance baseline, and model/hardware routing.
- DEC-013's Memory profile names/count, hardware signals and switching thresholds, index rebuild strategy, and the measurable quality/resource targets.
- Risk taxonomy, actions requiring screen-based final approval, reusable approvals, and human-only actions.
- Detailed Task / Activity retention, redaction, artifact ownership, and Memory promotion policy.
- API and schema versioning, migration guarantees, compatibility support period, and supported application integrations.
- Target Runtime acceptance methods, security review standards, performance thresholds, release dates, public-launch criteria, and support policy.
- DEC-017's public developer-environment and distribution foundation details: supported host environments, toolchains, package and signing contracts, channel scope, and acceptance evidence.
- Which AI-coding mechanisms from DEC-017's provisional catalog, CLI, sandbox, and host-to-QEMU verification candidates to adopt.
- Existing Rust / C / C++ application migration priorities, diagnostic support, toolchain matrix, and compatibility expectations.

An item may move from OPEN or PROVISIONAL to CONFIRMED only after the user approves the specific scope and the decision record is updated. Evidence-dependent implementation choices belong in the relevant implementation specification.

## 8. Relationship to existing documents

- [DEC-007 — Roadmap alignment review](../DEC-007-roadmap-alignment.md) remains a review note and does not change user decisions.
- [DEC-009 — Nagi-first, Home and Desktop](../DEC-009-nagi-first-home-desktop.md), [DEC-010 — Intent-first UI](../DEC-010-intent-first-adaptive-ui.md), [DEC-011 — Task approval and progress](../DEC-011-task-approval-progress-ui.md), and [DEC-012 — Workspace organization](../DEC-012-workspace-naming-organization.md) carry the confirmed product behavior referenced here.
- [DEC-013 — Hardware-Adaptive Memory Engine](../DEC-013-lightweight-memory-engine-benchmark.md) keeps formal implementation in the gated 0.3 scope, preserves model-independent canonical Memory across hardware transitions, and treats indexes / Context adapters as rebuildable derived data; profile names, algorithms, thresholds, and measured results remain provisional.
- [DEC-014 — Workspace state checkpoints](../DEC-014-workspace-state-checkpoints.md) confirms automatic restoration-state saving and named checkpoints, while leaving implementation timing and retention details open.
- [DEC-015 — Confirmed builder role and initial mode C](../DEC-015-builder-role-review.md) and [DEC-016 — Builder autonomy and risk-based completion](../DEC-016-builder-autonomy-risk-based-completion.md) record the initial Nagi `builder` Role-switch workflow (mode C) and its user-selected autonomy/completion rules; independent specialist Agents are future options, not initial 0.4 requirements.
- [DEC-074 — Human-managed hierarchical AI organization](../DEC-074-human-led-agent-organization.md) records the confirmed long-term hierarchy direction while leaving Agent counts, roles, implementation, resource rules, and release mapping provisional.
- [DEC-017 — Third-party developer ecosystem](../DEC-017-third-party-developer-ecosystem.md) records staged developer goals, AI-coding priority, and the goal of making portable Rust / C / C++ apps easier to migrate; exact mechanisms and compatibility scope remain provisional.
- [DEC-018 — Builder isolated editing and conditional apply](../DEC-018-builder-isolated-edit-conditional-apply.md) and [DEC-019 — Builder work-data cleanup](../DEC-019-builder-work-data-cleanup.md) preserve the coding Role workflow's edit, approval, recovery, and cleanup boundaries; initially Nagi executes the builder Role, with a separate Agent remaining optional for later phases.
- [DEC-020 — AI operation path and resource use](../DEC-020-ai-os-interaction-path.md) records the confirmed structured Context / Event, deterministic-first, typed-action and event-driven principles; implementation, performance claims, and release assignment remain provisional.
- [DEC-021 — 0.5–1.0 roadmap boundaries and sequencing](../DEC-021-roadmap-0.5-1.0.md) records inherited confirmed boundaries and which version assignments remain provisional.
- The [revised 0.3 roadmap](Nagi_OS_0.3_Architecture_Roadmap_Revised.md) and [0.4+ roadmap](Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md) remain authoritative for their respective architecture intents, gates, and acceptance proposals.
- No private decision or roadmap is copied to the Public repository by this document.

---

**End of Proposal**

## PDF Reader / Editorの製品方針（2026-11-16）

[DEC-061](../DEC-061-first-party-pdf-reader-editor.md)で、Nagi純正PDF Reader兼Editorの採用と段階的拡張をCONFIRMEDとして記録した。高速表示・検索・ページ操作・注釈・印刷・フォーム入力などを先に整え、その後、既存本文／画像の高度編集・OCR・元内容の削除と検証を伴う墨消し・署名関連機能を段階的に検討する。通常閲覧・編集はAI非必須とし、Nagi Project / Smart Paste / Writer / Sheets / Slides等と共通Public APIで連携する。AIによる表抽出・要約・比較・引用は利用者の承認と元資料の権限・機密分類の範囲に限る。

具体的なリリース版・高度機能の順序・技術・合格値はPROVISIONAL。本ロードマップの対象版への割当て、Acrobat完全互換、全機能初期実装、実装済み・Acceptance通過を意味せず、既存Gateは変更しない。
