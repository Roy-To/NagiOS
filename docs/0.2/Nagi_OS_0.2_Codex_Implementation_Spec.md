# Nagi OS 0.2 Platform Foundation
## Codex Implementation Specification

**Document ID:** `NAGI-OS-0.2-CODEX-IMPLEMENTATION-SPEC`  
**Status:** Implementation baseline / master roadmap  
**Target:** Nagi OS 0.2 Platform Foundation  
**Primary execution target:** QEMU x86-64 Nagi Virtual Reference Machine  
**Release-line prerequisite:** Nagi OS 0.1 M30 PASS plus explicit 0.2 activation checkpoint  
**Kernel:** Nagi Kernel, not Linux-based  
**Browser engine:** Servo only  
**Default Standard LLM:** IBM Granite 4.2 3B unless superseded by an accepted model contract  
**Primary languages:** English (`en-US`) and Japanese (`ja-JP`) as equal first-class UI languages  
**Development state authority:** `.dev/workstreams.json` plus each `.dev/workstreams/<id>/state.json`  
**0.1 state authority:** `docs/implementation_status.md`  
**Next product target:** Nagi OS 0.3 — Agentic System  
**Long-term public target:** Nagi OS 1.0 — Public OSS Release

---

# 0. Purpose of this document

This document is the master implementation specification for **Nagi OS 0.2 Platform Foundation**.

It converts the existing 0.2 development foundation, workstream registry, isolated
foundation branches, and first-party software work into one ordered release plan.
It is intentionally written at the same execution granularity as the Nagi OS 0.1
Codex implementation specification:

- product principles;
- architectural boundaries;
- release-line gates;
- platform contracts;
- named milestones;
- concrete deliverables;
- explicit acceptance criteria;
- dependency ordering;
- workstream ownership mapping;
- release Definition of Done;
- Codex implementation rules.

This document does **not** replace the Nagi 0.1 specification. Nagi 0.1 remains
the authority for the 0.1 release line until M30 passes.

This document does **not** authorize a workstream to violate its existing branch,
path ownership, activation gate, or merge boundary.

The purpose of Nagi 0.2 is not to add arbitrary features after 0.1. The purpose
is to turn the working 0.1 operating system into a coherent, versioned and
extensible **Nagi Platform** on which first-party and future third-party software
can rely.

The primary 0.2 success condition is:

> **Nagi becomes a stable application and system-service platform rather than a
> collection of individually wired 0.1 components.**

---

# 1. Specification precedence and source of truth

Before implementing a 0.2 milestone, inspect in this order:

1. repository-root and path-local `AGENTS.md`;
2. `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`;
3. `docs/implementation_status.md`;
4. this document;
5. `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`;
6. `docs/0.2/WORKSTREAMS.md`;
7. `.dev/workstreams.json`;
8. the active `.dev/workstreams/<id>/state.json`;
9. the relevant architecture/workstream specification;
10. `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` for first-party work;
11. accepted ADRs and versioned interface/schema contracts;
12. current source code and tests.

If documents disagree:

- a newer accepted versioned contract beats an older design sketch;
- 0.1 milestone acceptance must not be weakened to make 0.2 work easier;
- current repository evidence beats stale chat summaries;
- a workstream state file owns its status;
- `.dev/workstreams.json` owns branch/path/dependency registration;
- this document owns the **0.2 milestone ordering and release meaning**.

Do not silently resolve a normative conflict. Record the resolution in the
owning architecture/decision document and update compatibility tests.

---

# 2. Release-line boundary

## 2.1 Nagi 0.1 remains authoritative until M30

Nagi 0.2 runtime/product activation requires:

1. Nagi 0.1 M30 PASS;
2. an explicit 0.2 release boundary;
3. an explicit Integration Owner checkpoint for the workstream being activated.

Before that gate, only work explicitly authorized as host-side foundation,
contract design, fixtures, tools, schemas, isolated reference implementations,
or non-invasive first-party preparation may proceed.

Existing host-side foundations completed before the release boundary are valid
0.2 inputs. They are **not** evidence that the corresponding Nagi target runtime
feature has already passed.

## 2.2 No retroactive fake PASS

A host-only PASS remains a host-only PASS.

Examples:

- a host notification store does not prove target notification delivery;
- a host identity model does not prove target authenticated sessions;
- an in-memory Wayback backend does not prove target restore;
- a host Files provider does not prove native Nagi Files integration;
- a QEMU mock service does not prove the production service registry contract;
- a compile-only Windows check does not prove Windows runtime behavior.

When a host foundation is promoted into a runtime milestone, its tests are kept
and the target acceptance is added. Do not rewrite history and relabel the
foundation as if target acceptance had already occurred.

---

# 3. Nagi 0.2 product objective

Nagi 0.2 is the release where the operating system gains a coherent common
platform.

The 0.2 platform must provide:

- durable development and integration state;
- reproducible build provenance;
- legal/SBOM inventory foundations;
- structured diagnostics and observability;
- deterministic system integration testing;
- stable local identity and session concepts;
- capability and permission contracts;
- versioned system-service IPC;
- settings/configuration contracts;
- application manifest and lifecycle services;
- background jobs;
- notifications;
- safe installation/update primitives;
- model runtime/store contracts;
- shared UI design system and localization architecture;
- public platform object/action/context/workspace/activity contracts;
- Wayback/Activity and Search foundations;
- public SDK/App contract foundations;
- first-party software using the same public platform boundaries that future
  third-party software will use.

Nagi 0.2 must remain usable when AI is disabled or unavailable.

Nagi 0.2 must **prepare for** Nagi 0.3 Agentic System, but it must not turn every
0.2 service into an AI-specific subsystem.

---

# 4. Non-negotiable product principles

Nagi 0.2 inherits the 0.1 principles:

> **Safe by default. Powerful by choice.**

> **Restrict software, not the owner.**

> **The human says what they want; Nagi understands intent.**

> **AI is powerful but untrusted.**

The following are release rules.

## 4.1 Offline-first

Core platform operation must not require:

- cloud identity;
- cloud storage;
- cloud telemetry;
- cloud LLM;
- remote license server;
- permanent internet connectivity.

Network-dependent software such as Albert may naturally require networking for
remote content.

## 4.2 Capability before convenience

An application, automation, model, agent, background task, or service does not
gain authority merely because it is:

- first-party;
- signed by Nagi;
- started by the shell;
- invoked by an AI;
- part of the same Workspace;
- on the same user account.

Authority remains explicit and attenuated.

## 4.3 Public contract before private shortcut

First-party software should use the same public Nagi Platform contracts intended
for third-party software.

System-only operations may require System Capabilities, but “first-party” is not
itself a secret permission tier.

## 4.4 Deterministic policy boundary

Probabilistic systems may propose actions.

They may not decide kernel authority, silently grant capabilities, bypass
permissions, or reinterpret a failed action as successful.

## 4.5 Truthful UX

Never report:

- saved when persistence failed;
- sent when delivery failed;
- restored when no restore occurred;
- undoable when no valid inverse/checkpoint exists;
- synchronized when only a local fixture changed;
- target PASS from host-only evidence.

## 4.6 Reuse-first, resource-constrained AI

Nagi is intended to bring useful local AI to hardware people already own, not to
require a new high-end “AI PC” merely to participate.

Release rules:

- the 8 GB reference configuration is a deliberate product constraint, not merely
  a CI convenience;
- core OS usability takes precedence over model residency, model size, or peak
  inference throughput;
- local AI must degrade gracefully through smaller models, tighter context
  budgets, lazy load/unload, reduced concurrency, paused background work, or
  non-AI fallback paths rather than making the shell, Files, or foreground work
  unusable;
- the standard local-AI path must not assume a discrete GPU/NPU or 32 GB+ RAM;
- larger-memory systems may unlock larger models and richer multimodal workloads,
  but they are an enhancement tier rather than the baseline architecture;
- future ARM64, SBC, repurposed-PC, and mobile-class enablement must remain
  possible without high-level services depending on x86-only or accelerator-only
  behavior;
- cloud inference may extend capability, but it must not be required for
  local/offline platform behavior that Nagi defines as core;
- “legacy-environment friendly” means local/offline deployment and usefulness on
  constrained or repurposed hardware. It does not imply binary compatibility
  with legacy Windows, macOS, Linux, or other operating-system applications.

The product direction is:

> **Use the computer you already have. Make small local models useful by making
> the operating system context-aware and resource-aware.**

---

# 5. Official 0.2 execution target

Nagi 0.2 officially remains centered on the **Nagi Virtual Reference Machine**:

- QEMU;
- x86-64;
- UEFI / OVMF;
- q35;
- 4 vCPU reference configuration;
- 8 GB RAM reference configuration;
- VirtIO Block;
- VirtIO Network;
- VirtIO GPU;
- VirtIO Sound;
- VirtIO RNG;
- defined keyboard/mouse input path.

The 8 GB RAM target is intentional. A change that makes the normal 0.2 core plus
representative local-AI use require more than the reference memory budget must
be treated as an explicit product/release decision, not as an incidental
regression accepted only because developer machines are larger.

Lower-memory modes may be explored and are desirable for future repurposed,
SBC, and mobile-class devices, but 0.2 does not claim a 4 GB AI acceptance
baseline until measured target evidence exists.

0.2 may prepare physical-hardware abstractions, but broad PC compatibility is
not a release blocker.

## 5.1 Physical hardware policy

Do not make “runs on arbitrary PCs” a 0.2 release requirement.

Physical enablement follows a narrow reference-hardware strategy:

1. QEMU x86-64 remains the primary deterministic development target.
2. One selected x86-64 UEFI mini-PC becomes the first physical reference device.
3. Hardware abstractions must keep higher services independent of specific
   VirtIO or physical-device implementations.
4. ARM64 is introduced as a separate architecture target, initially through
   QEMU `aarch64`/`virt`.
5. Raspberry Pi support is a hardware-port project after generic ARM64 works.
6. Raspberry Pi 5 is a candidate ARM64 reference device, not an assumption
   embedded into the core architecture.

## 5.2 0.2 hardware non-goal

Nagi 0.2 does not require full Raspberry Pi, arbitrary laptop, arbitrary GPU,
Wi-Fi, Bluetooth, suspend/resume, or broad hardware certification.

---

# 6. High-level 0.2 architecture

```text
UEFI / Nagi Loader
        |
   Nagi Kernel
        |
   nagi-init
        |
+---------------- System Platform ----------------+
| Identity | Capability | Permission | Settings   |
| Service IPC | Supervisor | Jobs | Notifications |
| Update/Install | Diagnostics | Search | Wayback |
| Model Runtime | Localization | UI System        |
+-------------------------------------------------+
        |
+---------------- Public Nagi Platform ----------------+
| Resource | Document | Object | Action | Context       |
| Workspace | Activity | Revision | Checkpoint | Device |
| App Contract | SDK | Search Provider | Diff Provider  |
+------------------------------------------------------+
        |
+---------------- First-Party Software ----------------+
| Home | Albert | Files | Notes | Terminal | Activity   |
| Wayback | Search                                    |
+------------------------------------------------------+
        |
+------- Parallel / non-release-blocking 0.2 work ------+
| Writer | Sheets | Slides | Mail | Calendar           |
| Automations | Studio | People | Devices | Store      |
| Models                                              |
+------------------------------------------------------+
```

The architecture direction is:

> **Kernel manages execution, memory, communication and authority primitives.
> Versioned user-space services implement policy and product behavior. Apps use
> public platform contracts instead of depending on service internals.**

---

# 7. Nagi Core and Nagi Platform

## 7.1 Nagi Core

Nagi Core means the OS-internal services and infrastructure, including:

- service registry;
- identity/session;
- capability/permission;
- settings;
- lifecycle/supervisor;
- jobs;
- notification;
- package/update;
- diagnostics;
- model management/runtime;
- Resource/Workspace/Search/Activity/Wayback services.

## 7.2 Nagi Platform

Nagi Platform means the public contracts by which applications participate:

- ABI/IDL;
- Rust/C SDK;
- manifest;
- AppId and AppSessionId;
- Resource/Object/Document identities;
- Action definitions;
- Context publication;
- Workspace participation;
- Search providers;
- Activity recording;
- Wayback/checkpoint hooks;
- localization resources;
- UI design-system primitives;
- lifecycle and background execution declarations;
- capability declarations.

Nagi Platform is not synonymous with “first-party API”.

---

# 8. Stable identity model

Do not collapse logical identity into process IDs, paths, inodes, window IDs, or
temporary database rows.

The platform must distinguish at minimum:

- `UserId`
- `ProfileId`
- `SessionId`
- `NodeId`
- `AppId`
- `AppSessionId`
- `ExecutionInstanceId`
- `SurfaceId`
- `ResourceId`
- `ObjectId`
- `DocumentId` where required
- `WorkspaceId`
- `TransactionId`
- `ActivityId`
- `CheckpointId`
- `RevisionId`
- `JobId`
- `NotificationId`

A process restart must not silently change logical app/document identity.

A rename or move should not silently change Resource identity when the storage
provider can preserve it.

---

# 9. Local identity and session

Nagi remains local-first.

0.2 identity must support:

- Owner;
- Standard User;
- Guest;
- local profiles;
- ephemeral/guest sessions;
- explicit session ownership;
- session lease/recovery where appropriate;
- separation of authentication and authorization;
- profile-scoped storage/service access.

A valid session does not imply unrestricted authority.

AI and background jobs cannot convert a session into owner authority.

Cloud account integration is optional and outside the 0.2 core acceptance.

---

# 10. Capability and permission system

The platform distinguishes:

## 10.1 Permission

User/data/device access, for example:

- `files.read`
- `files.write`
- `microphone.capture`
- `clipboard.read`
- `network.connect`

## 10.2 Platform capability

Participation in a Nagi platform contract, for example:

- `context.publish`
- `actions.register`
- `search.provider`
- `workspace.participate`
- `wayback.checkpoint`
- `notifications.publish`
- `background.execute`

## 10.3 System capability

Strong OS management authority, for example:

- package installation;
- system settings modification;
- device management;
- privileged diagnostics;
- credential access.

Rights must be attenuable and fail closed.

The public API must support explicit denial paths and negative tests.

---

# 11. System-service IPC

Kernel Channels remain local low-level IPC primitives.

Nagi 0.2 adds a stable service-facing contract over them.

System-service IPC must define:

- service identity;
- service version;
- operation/method identity;
- request ID;
- correlation ID;
- caller/session context binding;
- typed request/response;
- bounded message size;
- explicit error classes;
- cancellation where meaningful;
- compatibility/version policy;
- capability enforcement boundary;
- transport-independent service contract.

Do not turn Kernel Channel into transparent network IPC.

Do not use JSON as the core trusted system wire format merely for convenience.

Large data should continue to use shared memory/VMO or bounded resource handles
instead of repeatedly copying through ordinary messages.

---

# 12. Settings and configuration

Settings must be a platform service, not scattered app-local flags.

The 0.2 settings contract must define:

- typed keys;
- namespaces;
- system/user/profile/app scope;
- defaults;
- validation;
- migration/versioning;
- read/write authorization;
- watch/subscribe semantics;
- atomic update boundary;
- corrupted-state recovery;
- localization-safe presentation metadata;
- explicit distinction between policy and user preference.

Examples of consumers include:

- localization;
- notifications/quiet mode;
- model defaults;
- permissions;
- shell preferences;
- accessibility;
- app preferences.

A Settings UI is not itself the Settings service.

---

# 13. Application manifest and lifecycle

0.2 separates two concerns that earlier isolated branches both called
`APP-LC-01`.

For this master specification:

- **0.2-M12** owns the **Application Manifest / Package Lifecycle Contract**.
- **0.2-M13** owns the **Application Supervisor / Process Lifecycle Runtime**.

Existing workstream IDs remain unchanged until the Integration Owner explicitly
renames or consolidates them.

## 13.1 Manifest contract

The application manifest defines at minimum:

- AppId;
- version;
- executable/entry point;
- supported platform/API versions;
- declared capabilities/permissions;
- services provided/consumed;
- actions;
- background behavior;
- localization resources;
- package metadata;
- upgrade/migration metadata.

## 13.2 Supervisor

The supervisor manages:

- start;
- ready;
- stop;
- crash;
- restart policy;
- dependency ordering;
- service registration;
- health;
- bounded restart loops;
- user/session ownership;
- graceful shutdown;
- failure diagnostics.

A valid package manifest does not itself mean an application is safely running.

---

# 14. Background jobs and task scheduling

0.2 must provide a bounded job abstraction for non-interactive work.

Required concepts:

- JobId;
- owner;
- profile/session scope;
- state machine;
- deduplication/idempotency;
- retry/backoff;
- deadlines;
- cancellation;
- cooperative shutdown;
- bounded persistence;
- deterministic clock/test injection;
- capability checks;
- diagnostic visibility.

Background execution may not be used to bypass foreground permission policy.

---

# 15. Notifications

Notifications are typed records, not arbitrary privileged callbacks.

Required concepts include:

- NotificationId;
- source attribution;
- localized/user text;
- severity/priority;
- lifecycle/read state;
- grouping;
- expiry;
- quiet/focus policy;
- bounded persistence;
- profile isolation;
- redaction/sensitivity handling;
- action descriptors that contain no authority.

Notification actions must be revalidated at activation time.

The notification core does not own:

- email;
- SMS;
- cloud push;
- arbitrary remote delivery.

Those are providers or later features.

---

# 16. Installation, update and package change

0.2 extends the 0.1 package/A-B foundations into a coherent installation/update
path.

Requirements:

- validated package paths;
- manifest compatibility checks;
- inventory;
- install/update/uninstall planning;
- atomic commit;
- rollback/recovery;
- interrupted-operation recovery;
- policy hooks;
- user-data preservation;
- version comparison;
- source mutation/stale-plan rejection;
- package provenance linkage.

A package manager must not claim success until the durable active inventory and
installed state agree.

System update and app package update may share primitives while remaining
different policy domains.

---

# 17. Diagnostics and observability

Diagnostics are a platform foundation.

Required behavior:

- structured events;
- severity/category;
- bounded metadata;
- redaction;
- crash capture;
- service/app health;
- diagnostic snapshot/report;
- correlation/request IDs;
- explicit Activity bridge where user-meaningful;
- failure-tolerant sinks;
- source/build fingerprint association.

Debug logs are not automatically Activity.

Secrets and credential payloads must never be copied into diagnostics by
default.

---

# 18. Build provenance and reproducibility

A release or reusable artifact must be attributable to its exact inputs.

The provenance/fingerprint contract should capture or explicitly mark absent:

- source commit;
- dirty state policy;
- compiler/toolchain;
- custom target;
- build features;
- flags;
- allowlisted environment;
- generated inputs;
- third-party source pins;
- patch digests;
- QEMU version/config where acceptance depends on it;
- firmware/OVMF input;
- artifact SHA-256 inventory.

Comparison must be deterministic and explain mismatches.

Artifact reuse must fail closed when compatibility inputs do not match.

---

# 19. License and SBOM foundation

0.2 release engineering must be able to produce a machine-readable dependency
inventory and release legal material.

Required foundations:

- component inventory;
- model artifact inventory;
- explicit unknown-license state;
- SPDX-compatible SBOM output;
- NOTICE candidate generation;
- deterministic ordering;
- path/credential privacy checks;
- source/license evidence;
- manual review queue.

The tool must not infer a legal conclusion from ambiguous prose.

Automated inventory is evidence for review, not legal advice.

---

# 20. Model Runtime and Model Store

0.2 stabilizes the local-model platform boundary.

Required concepts:

- model manifest;
- model identity/version;
- artifact digest/source;
- model store;
- validation;
- provider abstraction;
- lifecycle/load/unload;
- memory/resource limits;
- inference request/result contract;
- cancellation/timeouts;
- offline behavior;
- lazy load;
- diagnostics;
- permission/policy boundary for actions.

Granite 4.2 3B remains the preferred standard local model until an accepted
decision changes it.

Qwen/Gemma or future models are providers/packages, not separate privilege
domains.

0.2 does **not** make the model runtime a privileged system-policy engine.

---

# 21. UI design system

0.2 must establish shared UI primitives rather than letting each first-party app
invent its own widget behavior.

At minimum:

- layout primitives;
- typography;
- spacing;
- focus/keyboard semantics;
- interaction states;
- dialogs;
- menus;
- list/table patterns;
- inspector/sidebar patterns;
- accessibility metadata;
- text scaling;
- high-contrast compatibility;
- localization-aware layout;
- desktop surface conventions.

The design system must not depend on a particular app.

---

# 22. Localization and language architecture

English is the canonical internal language for:

- identifiers;
- APIs;
- schemas;
- message keys;
- diagnostics identifiers;
- configuration keys.

`en-US` and `ja-JP` are equal first-class user languages.

System language, region/locale, input language/IME, and AI conversation language
are distinct concepts.

Rules:

- no required UI text hard-coded in business logic;
- UTF-8 by default;
- selected-locale fallback to English;
- raw localization keys must not appear in normal UI;
- date/number/currency formatting is locale-aware;
- internal command/API IDs remain locale-neutral.

---

# 23. Platform object model

The following contracts must become shared platform concepts rather than
app-specific reinventions.

## 23.1 Resource

Stable user-visible data/resource identity.

A path is a locator, not necessarily identity.

## 23.2 Document

A user-owned persistent work product that may be backed by a file, database, or
future provider.

## 23.3 Object

A meaningful addressable item inside a Resource/Document:

- paragraph;
- note block;
- table/range;
- slide;
- browser selection;
- file resource;
- chart.

## 23.4 Revision

Immutable or versioned change identity where supported.

## 23.5 Provenance

Source, actor, transformation, revision, and time metadata for content or
actions when meaningful.

## 23.6 Action

Typed operation with:

- ActionId;
- input schema;
- output schema;
- side-effect classification;
- permissions/capabilities;
- reversibility;
- risk metadata.

GUI pixel clicking is not the canonical Action API.

## 23.7 Context

Current/recent work state intentionally published by applications.

Core must not scrape arbitrary application memory to manufacture Context.

## 23.8 Workspace

Semantic grouping across apps/resources/documents/sessions.

Workspace is not a folder.

---

# 24. Activity, Transaction and Wayback

0.2 turns 0.1 history into a shared semantic platform.

## 24.1 Activity

Human-understandable meaningful actions.

Actors include:

- USER;
- AGENT;
- APP;
- SYSTEM;
- AUTOMATION;
- future REMOTE_DEVICE.

## 24.2 Transaction

Groups related actions across one or multiple apps.

Do not require fake global ACID semantics for irreversible external systems.

## 24.3 Checkpoint

Restorable point for:

- Action;
- Document;
- Workspace;
- later System state.

## 24.4 Restore

Restore must be truthful.

Requirements:

- preview where meaningful;
- permission validation;
- stale revision validation;
- current-state preservation before destructive in-place restore;
- restore-as-copy;
- Activity record for the restore itself;
- explicit handling of partial failure;
- no Undo claim for irreversible external effects.

---

# 25. Search

Search is a service plus provider model, not merely a Search application.

Required layers:

1. exact/lexical;
2. metadata;
3. temporal/activity;
4. semantic/hybrid where available.

Basic search must work when AI/embedding is disabled.

Search must be permission-filtered before result disclosure.

The existence of an inaccessible secret resource must not leak through search.

Incremental indexing is preferred over repeated full scans.

---

# 26. SDK and app contract

The SDK must allow progressive adoption.

## Level 0 — Portable

Normal app launch/window/input/file selection.

## Level 1 — Platform Aware

Manifest, Resource identity, Context, Search participation.

## Level 2 — Agent Ready

Structured Actions, Activity, permission-aware automation/agent operations.

## Level 3 — Deep Native Integration

Workspace, Wayback/checkpoint, provenance, cross-app references, continuity-ready
semantic state.

First-party apps should converge toward Level 3 using the same public contracts.

---

# 27. First-party software target for 0.2

The 0.2 release-blocking first-party set is the Phase 1 Core Experience:

1. Home
2. Albert
3. Files
4. Notes
5. Terminal
6. Activity
7. Wayback
8. Search

Albert continues from the 0.1 implementation. Do not replace Servo.

Existing host/reference work for Files, Notes, Activity/Wayback, and Home/Search
must be adapted rather than rewritten without evidence.

## 27.1 Phase 2 software

The following may develop in parallel during 0.2:

- Writer;
- Sheets;
- Slides;
- Mail;
- Calendar;
- Automations.

They are **not required to block Nagi 0.2 release** unless an explicit later
release decision promotes them.

## 27.2 Phase 3 software

Studio, People, Devices, Store and Models remain later product tracks unless
explicitly promoted.

---

# 28. 0.3 boundary

Nagi 0.3 is the next product target:

# **Nagi OS 0.3 — Agentic System**

0.3 is where Nagi intentionally makes the operating environment agentic.

Planned 0.3 themes:

- Nagi Agent Runtime;
- System Context Graph;
- Personal System Memory;
- cross-app actions;
- capability-aware agent execution;
- AI Activity Ledger integration;
- Wayback-backed agent undo;
- natural-language Automations;
- unified Nagi command interface.

0.2 must expose stable contracts so 0.3 can call the same actions humans and apps
use.

0.2 must **not** prematurely implement a privileged AI bypass around those
contracts.

The desired transition is:

```text
Nagi 0.2
Human / App
   |
Public Action / Service API
   |
Capability + Permission
   |
System Service / App Provider

Nagi 0.3
Human
   |
Agent Runtime
   |
Planner / Validator
   |
the same Public Action / Service API
   |
Capability + Permission
   |
System Service / App Provider
```

---

# 29. Workstream registry and ownership policy

`.dev/workstreams.json` remains integration-owned.

A workstream owns exactly its registered paths and its own state file.

Shared files such as these remain Integration Owner controlled unless explicitly
assigned:

- root `Cargo.toml`;
- root `Cargo.lock`;
- shared ABI/IDL;
- `.github/workflows/**`;
- `.dev/workstreams.json`;
- `.dev/schemas/**`;
- release-level status documents.

A feature branch does not edit a global file merely to make its own CI green.

Instead, it supplies a registration/integration proposal when required.

---

# 30. Existing workstream mapping

This section maps the work that already exists in the repository to the 0.2
master milestones.

The mapping is normative for roadmap placement. Existing branch/path ownership
remains governed by the registry/state until an Integration Owner checkpoint
changes it.

## 30.1 Registered integration-branch workstreams

| Existing workstream ID | Primary 0.2 milestone(s) | Role |
|---|---|---|
| `development-foundation` | M01 | Durable workstream state, resume/verify tooling |
| `diagnostics` | M04, M28 | Diagnostics/observability and integration evidence |
| `integration-next-phase` | M00, M28-M30 | Release-line integration owner |
| `capability-permissions` | M08 | Capability/permission platform |
| `os-core` | M00, M09-M18, M28 | Kernel/ABI changes needed by 0.2 platform |
| `platform-apis` | M17-M21 | Resource/Action/Context/Workspace/Search platform contracts |
| `first-party-apps` | M22-M27 | Product app implementations |
| `first-party-integration` | M22-M27, M28 | Cross-app integration |
| `ai-runtime` | M16, M21 | Provider-neutral AI/model execution boundary; no privileged agent path |
| `sdk` | M21 | Public SDK/tooling |
| `developer-tooling` | M01-M03, M29 | Developer CLI and clean-checkout workflows |
| `ci-acceptance` | M05, M28-M30 | CI/acceptance orchestration |
| `documentation` | all, M29-M30 | Accepted contract and release documentation |
| `app-sdk-contract` | M12, M21 | Manifest/AppId/capability declarations and SDK contract |
| `wayback-activity-ledger` | M18 | Activity/Transaction/Checkpoint/Wayback contracts |
| `localization-i18n` | M06 | Common language and localization platform |
| `model-runtime` | M16 | Model Store/runtime foundation |
| `ui-design-system` | M07 | Shared UI system |
| `test-plat-01` | M05 | Deterministic system integration harness |

## 30.2 Existing isolated 0.2 foundation branches pending full integration

| Existing workstream / branch | Observed foundation state | Primary 0.2 milestone | Integration meaning |
|---|---:|---:|---|
| `notify-01` / `codex/0.2-notify-01` | PASS host foundation | M14 | Promote to runtime after release gate |
| `update-installation` / `codex/0.2-update-installation` | PASS host foundation | M15 | Register/integrate then target acceptance |
| `job-01` / `codex/0.2-job-01` | PARTIAL | M13 | Host scheduler complete enough to preserve; runtime gated |
| `ident-01` / `codex/0.2-ident-01` | PARTIAL | M08 | Identity reference foundation; storage/capability bindings remain |
| `settings-configuration` / `codex/0.2-settings-configuration` | NOT_STARTED runtime | M11 | Proposal/state exists; activation gated |
| `system-service-ipc` / `codex/0.2-system-service-ipc` | PARTIAL host contract | M10 | Register then bind to real service/channel semantics |
| `app-lifecycle-manifest` / `codex/0.2-app-lifecycle-manifest` | PASS host foundation | M12 | Manifest lifecycle contract |
| `app-lifecycle-supervisor` / `codex/0.2-app-lifecycle-supervisor` | NOT_STARTED runtime | M13 | Supervisor lifecycle runtime; duplicate local APP-LC label must be resolved |
| `build-provenance` / `codex/0.2-build-provenance` | BLOCKED on shared registration | M02 | Implementation exists; integration registration blocks shared CI |
| `license-sbom` / `codex/0.2-license-sbom` | PASS host foundation | M03 | Register and integrate legal/release tooling |

These observed states are a planning snapshot only. The branch-local state file
remains authoritative.

## 30.3 Existing first-party implementation tracks

| Existing track | Current role | Primary 0.2 milestone(s) |
|---|---|---|
| `codex/app-activity-wayback` | Host/reference Activity + Wayback contracts/UI model | M18, M25 |
| `codex/app-home-search` | Home registry + Search coordination/reference UI | M19, M26 |
| `codex/app-files` and integrated Files code | Files domain/provider/action foundation | M23 |
| `codex/app-notes` | Notes document/revision/action foundation | M24 |
| `codex/first-party-integration` | Host cross-app integration of Files/Notes/Activity/Wayback/Home/Search | M22-M26, M28 |
| 0.1 Albert/M18 branches | Browser foundation to be retained/adapted | M27 |
| 0.1 Search/M19 work | Semantic/search predecessor | M19 |
| 0.1 History/M15 work | History/transaction predecessor | M18 |
| 0.1 Package/M16 work | Package/SDK predecessor | M12, M15, M21 |

Do not duplicate these implementations merely because 0.2 assigns them new
release milestones.

---

# 31. Milestone activation model

Milestone statuses use:

- `NOT_STARTED`
- `IN_PROGRESS`
- `PARTIAL`
- `PASS`
- `BLOCKED`
- `DEFERRED`

## 31.1 Host foundation vs runtime PASS

A milestone may have a completed pre-release host foundation while the milestone
itself remains `PARTIAL` until target runtime acceptance passes.

## 31.2 Dependency rule

Do not proceed merely because another milestone has “most of the code”.

The required contract/acceptance named by the dependency must be PASS.

Independent host-side workstreams may run in parallel when their path ownership
and activation rules allow it.

---

# 32. Nagi 0.2 milestone roadmap

The milestones below are the authoritative 0.2 order.

The numbering is distinct from Nagi 0.1 milestones.

---

## 0.2-M00 — Release-Line Activation and 0.1 Baseline Freeze

### Deliver

- Nagi 0.1 M30 PASS recorded;
- immutable 0.1 release commit/reference;
- 0.2 integration branch selected;
- 0.2 release boundary recorded;
- current registry/state verification clean;
- explicit migration inventory from 0.1 to 0.2;
- preserved 0.1 acceptance suite.

### Acceptance

- `./nagi dev verify` passes;
- 0.1 release artifacts and source revision are known;
- no uncommitted 0.1 acceptance weakening;
- 0.2 Integration Owner checkpoint explicitly authorizes runtime activation.

### Gate

This is the mandatory runtime gate for M08 onward and any other target/product
workstream whose existing state requires M30 PASS.

### Primary workstreams

- `integration-next-phase`
- `development-foundation`
- `documentation`
- `ci-acceptance`

---

## 0.2-M01 — Development Foundation

### Deliver

- durable workstream registry;
- workstream state schema;
- `nagi dev status`;
- `nagi dev resume`;
- `nagi dev verify`;
- failure classes;
- resume protocol;
- branch/worktree/path ownership;
- CI evidence model.

### Acceptance

- registry parses;
- all registered IDs/branches/dependencies are unique and valid;
- state files validate;
- status/resume works from repository state without chat history;
- malformed/unsafe records fail closed;
- another developer can resume an active workstream from checked-in state.

### Existing implementation

DF-01 host foundation is already expected to be reusable rather than rewritten.

### Primary workstreams

- `development-foundation`
- `developer-tooling`

---

## 0.2-M02 — Build Provenance and Artifact Fingerprint

### Deliver

- versioned build fingerprint schema;
- deterministic digest;
- toolchain/target/features/flags inventory;
- source and patch identity;
- generated-input identity;
- third-party source identity;
- QEMU/firmware identity where relevant;
- artifact SHA-256 inventory;
- comparison CLI.

### Acceptance

- identical compatible inputs produce a deterministic match;
- source/toolchain/target/feature/artifact differences are classified;
- requested missing artifacts fail closed;
- secret-like values are redacted;
- shared CI recognizes the registered workstream;
- a release artifact can be traced back to exact compatible inputs.

### Primary workstreams

- `build-provenance`
- `developer-tooling`
- `ci-acceptance`

---

## 0.2-M03 — License / SBOM / Release Compliance Foundation

### Deliver

- component inventory;
- model-license inventory;
- source/license evidence inventory;
- explicit unknown-license state;
- SPDX SBOM generation;
- NOTICE candidate generation;
- inventory diff;
- privacy/path scrubbing;
- review queue.

### Acceptance

- all legal metadata files parse;
- deterministic SBOM fixture tests pass;
- repository inventory executes offline after sources are present;
- unknown/conflicting metadata remains explicit;
- generated output contains no host secrets/absolute private paths;
- release workflow can generate an SBOM/NOTICE candidate from an immutable source revision.

### Primary workstreams

- `license-sbom`
- `documentation`
- `ci-acceptance`

---

## 0.2-M04 — Diagnostics / Observability Foundation

### Deliver

- structured diagnostic event model;
- redaction;
- bounded sinks;
- crash capture;
- health registry;
- snapshot/report generation;
- correlation IDs;
- source/build identity linkage;
- explicit Activity bridge for user-meaningful events.

### Acceptance

- secrets are redacted in tests;
- sink failure does not crash the producer;
- bounded storage/rate behavior is deterministic;
- crash/health snapshot can be generated;
- diagnostic report identifies source/build fingerprint;
- debug events are not silently represented as user Activity.

### Primary workstreams

- `diagnostics`
- `build-provenance`

---

## 0.2-M05 — System Integration Test Platform

### Deliver

- deterministic host integration harness;
- fixtures;
- fake/virtual clock;
- failure injection;
- structured result format;
- target/QEMU adapter boundary;
- immutable artifact/run identity;
- CI integration policy.

### Acceptance

- host harness runs deterministically;
- failure injection is reproducible;
- result records source/artifact/run identity;
- target tests cannot be reported PASS from host fixtures;
- QEMU adapter can be added without changing product contracts;
- release runs are non-canceling and attached to immutable source/artifact identity.

### Primary workstreams

- `test-plat-01`
- `ci-acceptance`
- `diagnostics`

---

## 0.2-M06 — Localization / i18n Platform

### Deliver

- canonical message IDs;
- `en-US` resources;
- `ja-JP` resources;
- locale context;
- fallback rules;
- formatting helpers;
- catalog validation;
- localization developer guidance.

### Acceptance

- selected locale resolves expected text;
- missing Japanese resource falls back to English rather than raw key/blank text;
- malformed catalogs fail validation;
- English and Japanese representative screens/contracts are testable;
- internal IDs remain locale-neutral.

### Primary workstreams

- `localization-i18n`
- `ui-design-system`
- `documentation`

---

## 0.2-M07 — UI Design System Foundation

### Deliver

- shared UI tokens/primitives;
- layout;
- typography;
- focus/keyboard behavior;
- standard dialogs/menus/lists;
- inspector/sidebar patterns;
- accessibility metadata;
- localization-aware sizing;
- host/target adapter boundary.

### Acceptance

- shared components render/behave consistently in reference tests;
- keyboard navigation and focus state are testable;
- English/Japanese text does not require app-specific layout hacks;
- accessibility metadata exists on representative controls;
- first-party apps can consume the system without private widget forks.

### Primary workstreams

- `ui-design-system`
- `localization-i18n`

---

## 0.2-M08 — Identity, Session, Capability and Permission Integration

### Deliver

- UserId/ProfileId/SessionId contracts;
- guest/ephemeral session semantics;
- session recovery/lease rules;
- Principal/trusted-caller mapping;
- capability contract;
- permission decisions;
- attenuation/delegation rules;
- profile-scoped service access;
- trusted authorization boundary.

### Acceptance

- cross-profile reads fail;
- expired/invalid session authority fails;
- capability denial is deterministic;
- delegated authority cannot be strengthened;
- guest/ephemeral cleanup semantics are testable;
- AI/background work cannot self-elevate;
- representative service adapters bind requests to authenticated caller/session context.

### Primary workstreams

- `ident-01`
- `capability-permissions`
- `os-core`
- `platform-apis`

---

## 0.2-M09 — OS Core and Public ABI Stabilization

### Deliver

- 0.2-required kernel/ABI changes only;
- versioned handle/VMO/channel interfaces;
- service-facing caller context hooks;
- stable error mapping;
- compatibility fixtures;
- hardware abstraction boundaries retained.

### Acceptance

- 0.1 core behavior still passes regression;
- published ABI changes are versioned;
- rights attenuation remains release-blocking;
- no high-level product policy migrates into the kernel;
- x86-64 QEMU reference target remains stable.

### Primary workstreams

- `os-core`
- `platform-apis`

---

## 0.2-M10 — System Service IPC Contract

### Deliver

- service identity/version;
- operation identity;
- request/correlation ID;
- typed request/response;
- caller/session binding;
- cancellation/error contract;
- bounded payload rules;
- generated binding/adapter path;
- service registry compatibility mapping.

### Acceptance

- a reference service is discoverable and callable;
- incompatible versions fail explicitly;
- malformed/oversized requests fail closed;
- caller identity cannot be forged through ordinary payload fields;
- cancellation/failure results remain correlated;
- host contract tests and Nagi target service call both pass.

### Primary workstreams

- `system-service-ipc`
- `platform-apis`
- `os-core`

---

## 0.2-M11 — Settings / Configuration Service

### Deliver

- typed settings schema;
- system/user/profile/app scopes;
- defaults and validation;
- version/migration;
- atomic persistence;
- read/write policy;
- watch/subscription;
- corruption recovery;
- Settings adapter for first-party consumers.

### Acceptance

- invalid values are rejected;
- unauthorized scope writes fail;
- profile isolation passes;
- atomic write/recovery tests pass;
- subscribers receive versioned changes without duplicate ambiguity;
- language/notification/model representative settings use the common service.

### Primary workstreams

- `settings-configuration`
- `capability-permissions`
- `ident-01`
- `localization-i18n`

---

## 0.2-M12 — Application Manifest / Package Lifecycle Contract

### Deliver

- versioned application manifest;
- AppId/version/entry point;
- platform compatibility;
- declared capabilities/permissions;
- provided/consumed services;
- action declarations;
- background declarations;
- localization metadata;
- package lifecycle metadata;
- shared validation library.

### Acceptance

- valid fixture parses and validates;
- malformed IDs/versions/capabilities fail;
- duplicate/conflicting declarations fail deterministically;
- manifest can be consumed by SDK, installer, Home/App Registry and Supervisor without private copies;
- existing 0.1 package format is migrated rather than silently forked.

### Primary workstreams

- `app-lifecycle-manifest`
- `app-sdk-contract`
- `sdk`
- `platform-apis`

---

## 0.2-M13 — App Supervisor and Background Job Runtime

### Deliver

- app process lifecycle runtime;
- dependency start order;
- service readiness;
- health;
- crash/restart policy;
- bounded restart loop;
- JobId/state machine;
- retry/backoff;
- cancellation;
- dedupe/idempotency;
- background execution policy.

### Acceptance

- app start/ready/stop/crash paths are observable;
- repeated crashing does not loop forever;
- service dependency failures are explicit;
- job retry/backoff is deterministic with virtual clock tests;
- cancellation is cooperative and observable;
- background jobs cannot bypass capabilities/session ownership;
- target runtime launches a reference managed app and background job.

### Primary workstreams

- `app-lifecycle-supervisor`
- `job-01`
- `system-service-ipc`
- `capability-permissions`
- `diagnostics`

---

## 0.2-M14 — Notification Service

### Deliver

- production notification service binding;
- durable/profile-scoped persistence;
- lifecycle/read state;
- grouping/expiry;
- quiet/focus policy binding;
- action revalidation;
- diagnostics;
- first-party UI adapter contract.

### Acceptance

- host foundation tests remain PASS;
- target service publish/query/read/dismiss works;
- cross-profile isolation passes;
- sensitive content is redacted at storage/query/export boundaries;
- notification action revalidates current authority;
- quiet/focus policy behaves deterministically;
- notification service survives restart with durable state;
- representative first-party notification can be displayed through the UI adapter.

### Primary workstreams

- `notify-01`
- `settings-configuration`
- `ident-01`
- `capability-permissions`
- `diagnostics`

---

## 0.2-M15 — Update / Installation Runtime

### Deliver

- registered installer package;
- root workspace/CI integration;
- package inventory;
- install/update/uninstall;
- atomic commit;
- rollback;
- interrupted-operation recovery;
- policy hooks;
- provenance linkage;
- optional A/B system integration boundary.

### Acceptance

- existing 24+ host foundation tests remain PASS;
- target/reference package installs and launches;
- interrupted install recovers;
- failed switch rolls back;
- unsafe paths/symlinks are rejected;
- user data survives app/system update where contract requires;
- active inventory reflects durable installed state;
- package provenance can be reported.

### Primary workstreams

- `update-installation`
- `app-lifecycle-manifest`
- `app-sdk-contract`
- `build-provenance`
- `capability-permissions`

---

## 0.2-M16 — Model Runtime / Model Store Integration

### Deliver

- model manifest/store;
- artifact verification;
- provider abstraction;
- model lifecycle;
- memory pressure integration;
- Granite provider/backend;
- request/result/cancel contract;
- diagnostics;
- offline fallback behavior.

### Acceptance

- invalid/digest-mismatched artifact fails;
- model registry survives restart;
- provider can load/unload under policy;
- local Granite produces a real target response;
- cancellation/timeouts are observable;
- model runtime cannot invoke privileged actions directly;
- OS remains usable when model runtime is absent or unloaded.

### Primary workstreams

- `model-runtime`
- `ai-runtime`
- `diagnostics`
- `settings-configuration`

---

## 0.2-M17 — Platform Resource / Document / Object / Action Contracts

### Deliver

- ResourceId;
- Document identity;
- ObjectId;
- Revision;
- provenance;
- Action schema;
- risk/reversibility metadata;
- typed results/errors;
- provider contracts;
- compatibility/migration fixtures.

### Acceptance

- identity survives representative rename/move/reopen scenarios where provider supports it;
- Action schema rejects malformed inputs;
- unknown risk is not treated as safe;
- public contract has no first-party-only secret APIs;
- representative Files/Notes operations can be expressed through the shared model.

### Primary workstreams

- `platform-apis`
- `app-sdk-contract`
- `capability-permissions`
- `first-party-integration`

---

## 0.2-M18 — Activity / Transaction / Revision / Checkpoint / Wayback Platform

### Deliver

- Activity model;
- actor/provenance;
- Transaction grouping;
- revision/checkpoint contracts;
- diff provider;
- restore preview;
- restore-as-copy;
- recovery checkpoint;
- privacy/redaction;
- durable target provider.

### Acceptance

- USER and AGENT actions are distinguishable;
- failed/partial work is not marked success;
- target Activity survives restart;
- secret payloads are not copied by default;
- supported document/resource can checkpoint and restore;
- in-place restore preserves a recovery point;
- restore itself records Activity;
- irreversible external effects do not expose fake Undo.

### Primary workstreams

- `wayback-activity-ledger`
- `first-party-integration`
- `diagnostics`
- `capability-permissions`

---

## 0.2-M19 — Search / Index / Workspace Platform

### Deliver

- provider-based Search service;
- lexical and metadata search;
- temporal/activity search;
- semantic provider boundary;
- permission filtering;
- incremental indexing;
- Workspace model;
- Workspace membership/reference APIs;
- explanation metadata where feasible.

### Acceptance

- exact filename/title queries work without AI;
- unauthorized resources do not appear or leak existence;
- deletion/update propagates incrementally;
- Activity/temporal query works;
- Workspace can reference resources without physically moving them;
- semantic provider can be disabled without breaking basic search;
- target persistence survives restart.

### Primary workstreams

- `platform-apis`
- `first-party-integration`
- `ai-runtime`
- predecessor M19 Search code

---

## 0.2-M20 — Context / App Session / Surface Foundations

### Deliver

- Context Broker;
- active app/document/object publication;
- AppSessionId;
- ExecutionInstanceId separation;
- SurfaceId;
- NodeId descriptor boundary;
- continuity-ready semantic session state;
- privacy/capability rules.

### Acceptance

- app publishes explicit context;
- core does not inspect arbitrary app memory;
- process restart does not force logical session identity loss where supported;
- context access is permission/capability filtered;
- desktop surface works without hard-coding future device classes into public contracts.

### Primary workstreams

- `platform-apis`
- `os-core`
- `app-sdk-contract`
- `first-party-integration`

---

## 0.2-M21 — SDK / App Ecosystem Contract

### Deliver

- Rust SDK;
- C-facing stable boundary where required;
- Nagi IDL/bindings;
- manifest tooling;
- action/context/search/activity helpers;
- package tooling;
- out-of-tree sample app;
- migration/version guidance.

### Acceptance

- out-of-tree sample builds using published SDK only;
- package validates/installs/launches;
- sample app receives only declared authority;
- SDK API versions are explicit;
- generated bindings roundtrip contract fixtures;
- sample can opt into Level 0 then deeper integration without private first-party APIs.

### Primary workstreams

- `sdk`
- `app-sdk-contract`
- `platform-apis`
- `developer-tooling`

---

## 0.2-M22 — First-Party Common Framework Integration

### Deliver

- common first-party adapters for:
  - identity/session;
  - capability;
  - Actions;
  - Context;
  - Search;
  - Activity;
  - Wayback;
  - Workspace;
  - localization;
  - UI system;
- shared test fixtures;
- migration of existing host first-party foundations.

### Acceptance

- Files/Notes/Home/Search/Activity/Wayback use common contracts rather than duplicated local types where public contracts exist;
- host reference tests remain PASS;
- target adapters are real, not fixtures;
- cross-app test proves Resource/Object/Action/Activity identity consistency;
- no first-party app gets undeclared hidden authority.

### Primary workstreams

- `first-party-integration`
- `first-party-apps`
- `platform-apis`

---

## 0.2-M23 — Files 0.2

### Deliver

- native Files surface;
- real Nagi storage/provider adapter;
- typed Files Actions;
- Trash;
- metadata/tags;
- ResourceId retention where supported;
- Context;
- Workspace references;
- Search provider;
- Activity;
- Wayback/checkpoint integration;
- English/Japanese UI.

### Acceptance

- create/copy/move/rename/duplicate/trash/restore/permanent-delete work on target;
- destructive permanent delete requires correct confirmation/policy;
- same-resource move preserves identity where the provider supports it;
- selected-resource Context publishes correctly;
- Workspace add/remove does not move the file;
- Search returns authorized resources;
- agent/app mutation records Activity;
- supported mutations create truthful checkpoint/restore behavior.

### Primary workstreams

- `first-party-apps`
- `first-party-integration`
- existing Files implementation

---

## 0.2-M24 — Notes 0.2

### Deliver

- native Notes surface;
- persistent Nagi Document provider;
- stable note/block IDs;
- autosave;
- revisions;
- Quick Note;
- Trash;
- Search provider;
- Activity;
- Wayback;
- cross-app Resource/Object references;
- English/Japanese UI.

### Acceptance

- create/edit/save/close/reopen persists;
- autosave failure retains dirty state and does not claim success;
- revisions remain immutable/ordered;
- restore/restore-as-copy works where supported;
- Search finds note metadata/content according to policy;
- Activity contains meaningful edit groups rather than every keystroke;
- a typed reference from another app can roundtrip without flattening identity.

### Primary workstreams

- `first-party-apps`
- `first-party-integration`
- existing Notes implementation

---

## 0.2-M25 — Activity + Wayback Applications

### Deliver

- native Activity timeline;
- actor/workspace/app filters;
- transaction grouping;
- details/diff;
- open target;
- Undo/Wayback routing;
- native Wayback timeline/preview;
- compare;
- restore;
- restore-as-copy;
- pin checkpoint;
- partial Workspace restore UI.

### Acceptance

- USER/AGENT/AUTOMATION/SYSTEM are visibly distinguishable;
- failed/partial transaction is truthful;
- target resource opens from Activity;
- reversible event routes to supported undo/restore;
- irreversible event does not show fake Undo;
- Wayback can restore a supported document;
- restore-as-copy succeeds;
- pinned checkpoint survives cleanup;
- secret content is not indiscriminately rendered from Activity metadata.

### Primary workstreams

- `first-party-apps`
- `first-party-integration`
- existing Activity/Wayback track
- `wayback-activity-ledger`

---

## 0.2-M26 — Home + Search Applications

### Deliver

- native Home surface;
- app registry;
- recent/workspace/resource projections;
- universal Search surface;
- provider coordination;
- cancellation/stale-request suppression;
- typed result actions;
- English/Japanese UI.

### Acceptance

- installed/available apps are listed from the real registry;
- unlaunchable package is not falsely shown as successfully launchable;
- Search queries real providers;
- unauthorized private result is filtered;
- stale/cancelled query does not overwrite newer results;
- lexical search works with AI disabled;
- result actions use typed platform Actions.

### Primary workstreams

- `first-party-apps`
- `first-party-integration`
- existing Home/Search track

---

## 0.2-M27 — Albert / Terminal / Core Experience Platform Adoption

### Deliver

- Albert retained on Servo;
- Albert platform Actions/Context/Search/Activity integration;
- browser Resource/Object references;
- permission-aware downloads/uploads/clipboard/IME path;
- Terminal platform integration;
- session identity and diagnostics;
- Home/Files/Notes/Activity/Wayback/Search interoperability.

### Acceptance

- Albert renders representative real HTTPS sites on Nagi target;
- current page/selection can be published as bounded untrusted Context;
- browser history/bookmark/session persistence remains intact;
- a browser page/reference can be consumed by another first-party app through public contracts;
- Terminal runs without bypassing capability boundaries;
- platform integration does not regress 0.1 browser acceptance.

### Primary workstreams

- `first-party-apps`
- `first-party-integration`
- predecessor Albert M18 implementation
- `platform-apis`

---

## 0.2-M28 — Full Platform Integration / Security / Stress / Accessibility

### Reference load

```text
QEMU x86-64
4 vCPU
8 GB RAM

Desktop
Home
Files
Notes
Albert 3-5 tabs
Terminal
Activity
Wayback
Search
Notification service
Background jobs
Granite available
Audio playback
```

### Deliver

- end-to-end integration suite;
- restart/persistence suite;
- permission-negative suite;
- service failure injection;
- stress/resource-pressure suite;
- accessibility checks;
- English/Japanese smoke suites;
- clean artifact fingerprint.

### Acceptance

- no kernel OOM under reference load;
- desktop remains interactive;
- a representative local Granite inference request can complete within the
  8 GB reference envelope without making the desktop unusable;
- memory pressure can throttle or unload model work before core shell/Files/
  foreground usability is sacrificed;
- no sustained audio failure caused by background/platform work;
- model runtime does not monopolize the machine;
- no major handle/memory leak;
- denied capabilities remain denied across restart;
- one crashed service does not silently corrupt unrelated service state;
- persisted user data survives service/system restart;
- first-party core remains usable without model runtime;
- keyboard/focus path works on representative core apps;
- English/Japanese smoke tests pass;
- integration result is tied to immutable source/build fingerprint.

### Primary workstreams

All release-blocking 0.2 workstreams.

---

## 0.2-M29 — Release Hardening / Documentation / Reference Hardware Preparation

No major new platform feature should be introduced here.

### Focus

- bug fixes;
- migration cleanup;
- version/API freeze;
- developer docs;
- SDK docs;
- onboarding;
- first-party help;
- diagnostics quality;
- performance cleanup;
- license/SBOM review;
- sample apps;
- screenshots;
- clean build;
- reference-hardware selection notes;
- generic hardware abstraction review;
- ARM64 architecture boundary review.

### Acceptance

- clean checkout can reproduce the build;
- public contracts have version documentation;
- known limitations are explicit;
- release SBOM/NOTICE candidates are generated;
- no release-blocking unresolved security/authority defect;
- no hidden requirement for broad physical hardware support;
- x86-64 reference-device plan does not leak device-specific behavior into high-level services.

### Primary workstreams

- `documentation`
- `developer-tooling`
- `build-provenance`
- `license-sbom`
- `ci-acceptance`
- `integration-next-phase`

---

## 0.2-M30 — Nagi OS 0.2 Release

### Produce

```text
Nagi-OS-0.2.qcow2
SHA256SUMS
source revision
build/provenance manifest
SBOM
licenses/notices
architecture docs
platform API docs
SDK docs
first-party app docs
migration notes
contribution guide
known limitations
roadmap
Nagi 0.3 target document
```

### Acceptance

- M00-M29 required release gates PASS;
- immutable release source commit;
- release QEMU acceptance PASS;
- artifact hashes verified;
- clean-environment reproduction verified;
- 0.2 first-party core acceptance PASS;
- release documentation matches actual capability;
- 0.3 roadmap is documented but not falsely presented as 0.2 functionality.

---

# 33. Milestone completion rule

Do not proceed past a milestone merely because “most code exists”.

A release-blocking milestone should end with:

1. inspect current state;
2. implement the smallest architecture-correct increment;
3. format/static validation;
4. focused tests;
5. host integration tests where relevant;
6. Nagi target build where relevant;
7. QEMU/runtime acceptance where relevant;
8. negative/failure tests;
9. log review;
10. state update;
11. documentation update;
12. coherent commit;
13. push of the owning branch;
14. CI/run evidence attached to exact SHA;
15. clean resumable worktree.

`PASS` means the milestone's own acceptance boundary passed.

---

# 34. Nagi 0.2 Definition of Done

Nagi 0.2 is complete when all of the following are true.

## 34.1 Development and release engineering

- durable workstream state is valid;
- reproducible build provenance exists;
- release artifacts are hashed;
- SBOM/NOTICE candidates can be generated;
- clean checkout/rebuild path is documented and verified;
- acceptance evidence is tied to exact source/artifact identity.

## 34.2 Platform services

- identity/session works;
- capability/permission checks are enforced;
- service IPC is versioned and real;
- settings persist and migrate;
- supervisor lifecycle works;
- background jobs are bounded/cancellable;
- notifications persist and revalidate actions;
- package install/update/rollback works;
- diagnostics are structured/redacted;
- model runtime is local/offline-capable and non-privileged.

## 34.3 Public platform contracts

- Resource/Object/Document identity;
- Action;
- Context;
- Workspace;
- Activity;
- Transaction;
- Revision;
- Checkpoint/Wayback;
- Search provider;
- App manifest;
- SDK

exist as versioned, tested public contracts.

## 34.4 First-party core

Home, Albert, Files, Notes, Terminal, Activity, Wayback and Search run as a
coherent core experience on the official QEMU target.

Their shared behavior uses common platform contracts.

## 34.5 Security

- rights cannot be strengthened through transfer/delegation;
- profile isolation works;
- denied authority fails closed;
- notification/job/agent/model paths do not create authority;
- restore/undo behavior is truthful;
- Search does not leak inaccessible resources;
- diagnostics/activity do not indiscriminately store secrets.

## 34.6 AI independence

With model runtime disabled:

- desktop works;
- Files works;
- Notes works;
- Terminal works;
- Search lexical/metadata works;
- Activity works;
- Wayback for supported non-AI operations works;
- package/settings/core platform works.

---

# 35. Parallel work policy

The roadmap is ordered by dependency, not by a requirement that only one Codex
session may run.

Parallel work is encouraged when:

- ownership paths do not overlap;
- shared contracts are versioned;
- activation gates permit it;
- a workstream has an exact next action;
- results can be independently tested;
- shared files remain Integration Owner controlled.

Do not create concurrency merely to increase branch count.

Prefer workstreams that can produce independently testable foundations.

---

# 36. Integration checkpoint policy

Before integrating a feature branch:

1. inspect branch state;
2. verify owned paths;
3. verify dependency contracts;
4. run focused tests;
5. run shared compatibility tests;
6. apply required registry/shared-file updates centrally;
7. rerun host CI;
8. run target acceptance when required;
9. update the owning state;
10. record integration commit and evidence.

Merge conflict resolution must not substitute for architectural review.

---

# 37. Existing foundation preservation rules

The following work must be preserved and adapted unless tests show it is
incorrect:

- DF-01 durable development state;
- Diagnostics structured event/redaction/snapshot work;
- UI Design System foundation;
- Localization foundation;
- Model Runtime/Store host contracts;
- NOTIFY-01 host foundation;
- UPD-01 host installer foundation;
- JOB-01 scheduler foundation;
- IDENT-01 host identity model;
- Capability/Permission host foundation;
- TEST-PLAT-01 host harness;
- System Service IPC host contract;
- App Lifecycle Manifest host foundation;
- Build Provenance implementation;
- License/SBOM tooling;
- Activity/Wayback host model;
- Home/Search host/reference implementation;
- Files host/reference implementation;
- Notes host/reference implementation;
- first-party host integration harness;
- Albert/Servo 0.1 implementation.

Do not rewrite one of these simply to give a 0.2 milestone a “clean” directory
structure.

Migration is preferred to duplication.

---

# 38. Phase 2 first-party parallel development

Writer, Sheets, Slides, Mail, Calendar and Automations may continue while
0.2 release-blocking platform work proceeds.

Rules:

- do not duplicate unfinished shared platform services inside the app;
- use adapters/interfaces when the production service is not yet available;
- host/reference work must remain truthfully labeled;
- no Phase 2 app may weaken a 0.2 release gate;
- no Phase 2 app is required for 0.2 release unless explicitly promoted.

The value of parallel Phase 2 work is to exercise the public platform contracts
early and reveal missing abstractions before 1.0.

---

# 39. Hardware roadmap after 0.2

The hardware strategy is intentionally narrow.

## Stage A — Current

```text
QEMU x86-64
```

Primary development and deterministic acceptance target.

## Stage B — First physical Nagi reference PC

Select one specific x86-64 UEFI mini-PC model/configuration.

Define exact:

- CPU;
- firmware/UEFI;
- storage;
- display/GPU;
- Ethernet;
- USB/input;
- audio.

The goal is **one fully supported reference device**, not broad PC support.

## Stage C — Generic ARM64 virtual target

Add:

```text
QEMU aarch64 virt
```

Use it to validate architecture separation for:

- kernel;
- MMU/interrupt path;
- runtime;
- platform services;
- applications.

## Stage D — Raspberry Pi preparation

Use available QEMU Raspberry Pi models only as limited board-specific practice.

Do not confuse generic ARM64 PASS with Raspberry Pi 5 PASS.

## Stage E — Raspberry Pi 5 real hardware

Only when generic ARM64 is stable, perform the final board-specific work:

- boot/firmware;
- BCM2712/RP1 integration;
- USB;
- storage;
- Ethernet;
- display/GPU;
- audio;
- device tree;
- board-specific interrupts/timers.

Real hardware is needed for final board acceptance, but not for most generic
ARM64 platform development.

---

# 40. Nagi 0.3 target

After Nagi 0.2 Platform Foundation:

# **Nagi OS 0.3 — Agentic System**

The 0.3 master goal is:

> **The operating system can understand context and safely execute cross-app
> user intent through the same public capability-checked contracts used by
> ordinary applications.**

Candidate 0.3 pillars:

1. Agent Runtime
2. System Context Graph
3. Personal System Memory
4. Cross-App Action Orchestration
5. Capability-aware Agent
6. AI Activity Ledger
7. Wayback-backed Agent Undo
8. Natural-language Automations
9. Unified Nagi Command Interface
10. Explainable action plans and user confirmation policy

0.3 remains a future target until its own implementation specification is
accepted.

---

# 41. Nagi 1.0 direction

Nagi 1.0 is the intended first formal public OSS release once the operating
system has a coherent feature set and can be used and developed by people other
than the original developer.

The 1.0 bar is not “every PC works”.

The intended bar is closer to:

- reproducible source build;
- QEMU official support;
- at least one fully supported x86-64 reference device;
- documented experimental/secondary architectures where available;
- stable core platform contracts;
- usable first-party software;
- SDK and sample apps;
- contribution documentation;
- update/recovery;
- legal/SBOM material;
- security/permission model;
- Agentic System capability mature enough to represent Nagi's identity.

Before 1.0, 0.x public APIs may change with explicit migration.

At 1.0, selected public contracts should enter a compatibility/stability policy.

---

# 42. Codex autonomous implementation loop

For an active workstream:

1. read `AGENTS.md`;
2. run/inspect `./nagi dev status`;
3. run/inspect `./nagi dev resume`;
4. run/inspect `./nagi dev verify`;
5. inspect Git branch/HEAD/worktree;
6. read the owning state file;
7. read this 0.2 milestone;
8. read the specific workstream/architecture contract;
9. inspect existing implementation;
10. choose the exact next acceptance item;
11. implement;
12. test;
13. diagnose;
14. repair;
15. rerun;
16. update state;
17. update relevant docs;
18. commit coherent checkpoint;
19. push only the owning branch;
20. inspect CI by immutable run ID/head SHA;
21. continue while an in-scope unblocked acceptance item remains.

Do not stop after planning when implementation is authorized.

Do not expand into another workstream merely because the current stream is
blocked.

---

# 43. Failure and retry policy

Use the existing failure classes:

- `SOURCE`
- `BUILD`
- `LINK`
- `ABI`
- `RUNTIME`
- `BOOT`
- `DEVICE`
- `STORAGE`
- `GRAPHICS`
- `NETWORK`
- `MODEL`
- `PERMISSION`
- `ACCEPTANCE`
- `CI_INFRA`
- `HOST_ENV`
- `UNKNOWN`

For a failure:

1. record the exact command/stage;
2. preserve raw evidence;
3. classify from evidence;
4. change the hypothesis before meaningful retry;
5. do not repeatedly run an expensive identical command with no changed input;
6. after the workstream's retry budget, record `BLOCKED` with the next distinct
   experiment.

Transient infrastructure failures may be retried with bounded backoff.

---

# 44. Fake-success prohibitions

The following are explicitly prohibited:

- constant-true acceptance tests;
- dummy persistence described as durable;
- host fixtures reported as target runtime;
- fake Undo;
- fake notification delivery;
- fake app launch;
- fake search authorization;
- swallowing an install/update failure and returning success;
- AI-generated side effects without Activity where Activity is required;
- treating an unregistered branch CI failure as a product feature failure when
  evidence shows only registry integration is missing;
- weakening 0.1 acceptance to unblock 0.2.

---

# 45. Compatibility policy

Version separately:

- kernel syscall ABI;
- system-service protocol;
- public platform IDL;
- manifest schema;
- SDK package/API;
- persisted storage schema;
- Activity/Wayback schema;
- Search index schema;
- model manifest;
- diagnostic event schema.

A change to one does not automatically require changing all others.

When a persisted/public contract changes:

- increment/version it;
- provide migration where required;
- keep compatibility fixtures;
- document incompatible cases;
- test old-to-new transition;
- fail explicitly when migration cannot be safely performed.

---

# 46. Security acceptance themes

Every relevant milestone should test:

- unauthorized caller;
- expired session;
- wrong profile;
- stale Resource/Object revision;
- malformed payload;
- oversized payload;
- duplicate/replay request;
- capability attenuation;
- service/provider unavailable;
- partial persistence failure;
- crash/restart during mutation;
- corrupted durable state;
- secret/redaction boundary.

Security tests are not postponed to M28 if the feature can be tested earlier.

M28 repeats them in integrated form.

---

# 47. Performance policy

Optimize after obtaining representative measurements.

Priorities:

1. UI/input responsiveness;
2. audio continuity;
3. foreground app responsiveness;
4. storage correctness;
5. networking;
6. background platform work;
7. AI/model work.

Background indexing, models, jobs and diagnostics may be throttled or paused to
protect core usability.

Do not optimize by weakening correctness or authority checks.

---

# 48. Resource-pressure policy

Under pressure, prefer:

1. trim caches;
2. pause indexing;
3. pause background jobs;
4. reduce diagnostic buffers within retention policy;
5. unload inactive model providers;
6. shrink model context if safe;
7. suspend background apps;
8. preserve shell/Files/foreground app/audio as long as possible.

Model availability is not more important than basic OS usability.

---

# 49. Documentation requirements

By 0.2 release, document:

- build prerequisites;
- QEMU launch;
- architecture;
- kernel/user-space boundary;
- service IPC;
- capability/permission;
- identity/session;
- settings;
- app manifest/lifecycle;
- jobs;
- notifications;
- update/install;
- model runtime/store;
- diagnostics;
- localization;
- UI design system;
- Resource/Document/Object/Action;
- Context/Workspace;
- Activity/Wayback;
- Search;
- SDK;
- first-party app integration;
- package development;
- troubleshooting;
- known limitations;
- hardware roadmap;
- 0.3 roadmap.

---

# 50. Release naming

Recommended release identity:

# **Nagi OS 0.2 — Platform Foundation**

This communicates that 0.2 is the version in which Nagi becomes an application
and system-service platform.

Recommended next release identity:

# **Nagi OS 0.3 — Agentic System**

The release names are product guidance, not substitutes for acceptance criteria.

---

# 51. Final Codex instruction

Implement Nagi OS 0.2 incrementally according to the milestone order and gates
in this document.

Preserve already-completed foundations.

Do not attempt to finish the entire release in one uncontrolled branch.

For each milestone and workstream:

1. inspect the actual repository and durable state;
2. prefer existing implementation over duplication;
3. honor ownership and activation gates;
4. implement the smallest complete architecture-correct increment;
5. build and test;
6. diagnose evidence before retrying;
7. repair;
8. rerun acceptance;
9. update the owning state and documentation;
10. commit and push a resumable checkpoint;
11. integrate through the explicit Integration Owner boundary;
12. keep target/runtime claims separate from host/reference claims.

The primary success condition is not that the 0.2 repository contains many new
crates or branches.

The primary success condition is:

> **Nagi 0.2 is a real, testable platform whose shared services, public contracts,
> first-party applications, authority model and release engineering all agree
> with one another — and whose architecture is ready for Nagi 0.3 to become
> agentic without bypassing those boundaries.**
