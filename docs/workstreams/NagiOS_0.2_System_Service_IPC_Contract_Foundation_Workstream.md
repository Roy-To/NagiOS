# Nagi OS 0.2 — System Service / IPC Contract Foundation Workstream

## 0. Workstream identity

- **Workstream ID:** `SVC-IPC-01`
- **Recommended branch:** `codex/0.2-system-service-ipc`
- **Primary scope:** provider-neutral system-service contracts, request/response envelopes, service discovery/registration abstractions, error/cancellation semantics, in-process reference transport, mocks and contract tests
- **Repository:** `/Users/tozawa/Developer/NagiOS`
- **Execution model:** dedicated worktree / dedicated branch
- **Default status at start:** `PLANNED` or actual repository state

This is an implementation workstream. Do not finish with only architecture notes. Continue through code, tests, fixes, state update, commit and push as far as ownership permits.

---

## 1. Goal

Define the stable contract by which Nagi applications and first-party components can call **system services without linking directly to each service's internal implementation**.

The foundation must be usable by future services such as:

- Files;
- Activity;
- Wayback;
- Search;
- Models;
- Notifications;
- app/package management;
- other future platform services.

The goal is not to implement all those services. The goal is to establish a reliable, typed, testable boundary so service providers and service clients can evolve independently.

---

## 2. Non-goals / prohibited scope

Do not implement product-level behavior belonging to another workstream.

### Explicitly out of scope

- M18 browser integration or Servo/Mesa/relibc work;
- Files application functionality;
- Activity ledger storage implementation;
- Wayback snapshot implementation;
- Search indexing/ranking implementation;
- Model Runtime internals;
- Notifications UI;
- app install/update transaction engine;
- network RPC between physical machines;
- distributed consensus;
- public Internet API gateway;
- cryptographic identity/signing system unless an existing Nagi primitive is already mandatory;
- permission policy decisions owned by Capability / Permission.

### Shared integration files

Do not edit Integration Owner-owned shared registries or broad CI orchestration unless ownership is explicitly granted. Use a registration/integration proposal in this workstream's owned state area when necessary.

---

## 3. Required repository inspection

Before editing, inspect and follow:

- all relevant `AGENTS.md` files;
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`;
- existing IPC, message, channel, RPC, syscall, service, broker or registry abstractions;
- App SDK foundation if present;
- Capability / Permission public APIs;
- Diagnostics public event/error APIs if available;
- Model Runtime provider-neutral patterns that may offer useful conventions without creating a dependency;
- current `.dev` workstream state and schemas;
- existing error/result conventions;
- current Git/worktree state.

Prefer existing repository primitives. Do not create a second competing IPC framework if one already exists.

---

## 4. Architecture principles

The contract must be:

- provider-neutral;
- transport-neutral at the API boundary;
- deterministic;
- explicit about versions;
- explicit about request cancellation and deadlines if supported;
- usable in host-side tests without the target OS;
- compatible with capability enforcement without embedding policy decisions;
- suitable for later target transport implementation;
- minimal enough that first-party applications can adopt it early.

The reference implementation may be in-process. The public contract must not assume that in-process calls are the only future transport.

---

## 5. Required implementation

### 5.1 Service identity

Provide a strongly typed service identifier.

Minimum requirements:

- canonical `ServiceId` or repository-equivalent;
- validation;
- deterministic representation;
- suitable equality/hash/order behavior;
- no accidental filesystem/path interpretation;
- tests for valid/invalid identifiers.

### 5.2 Contract/interface version

Each service contract must have a version or compatibility identifier.

Implement:

- validated version representation;
- exact or documented compatible-match behavior;
- explicit unsupported-version error;
- tests for version negotiation/mismatch.

Do not invent a complex negotiation protocol if exact-major compatibility is enough for 0.2.

### 5.3 Request identity and envelope

Define a transport-neutral request envelope or equivalent typed metadata.

It should support at least:

- request/correlation ID;
- service ID;
- contract version;
- operation/method ID;
- caller principal/app identity through a neutral type or adapter;
- payload boundary;
- cancellation/deadline metadata when architecture supports it;
- optional tracing/diagnostic metadata without requiring Diagnostics.

Avoid opaque global mutable state.

### 5.4 Response/error contract

Define deterministic success/failure semantics.

Error categories should cover at least:

- service not found;
- unsupported contract version;
- operation not found;
- invalid request;
- permission denied / capability denied as a category only;
- unavailable/busy;
- cancelled;
- deadline exceeded if deadlines exist;
- provider failure/internal error;
- serialization/transport failure if applicable.

Errors must be structured. Callers must not parse prose strings.

Do not expose sensitive provider internals through default error messages.

### 5.5 Service descriptor

Define a service descriptor suitable for registration/discovery.

Minimum metadata:

- service ID;
- contract version(s);
- provider identity/class if useful;
- supported operation identifiers or a stable contract reference;
- required capability relationship if architecture has a standard representation;
- health/availability summary only if it can be expressed without depending on Diagnostics internals.

### 5.6 Registry / discovery abstraction

Implement an in-memory/reference service registry or equivalent abstraction.

Required behavior:

- register provider;
- reject duplicate/ambiguous registration deterministically;
- unregister provider;
- resolve service by ID and compatible version;
- enumerate descriptors if useful;
- no global singleton requirement;
- testable in isolation.

Do not make the reference registry the permanent kernel architecture by accident. Clearly mark transport/runtime boundaries.

### 5.7 Client abstraction

Implement a minimal client-facing API that can:

- resolve/call a service;
- return structured errors;
- support cancellation where present;
- preserve request/correlation identity;
- be replaced with another transport later.

Typed generated RPC code is not required for this foundation unless the repository already has code generation conventions.

### 5.8 Provider abstraction

Implement a provider/handler contract.

Requirements:

- provider receives validated request metadata/payload boundary;
- returns structured response/error;
- can be mocked;
- can be cancelled if the contract supports cancellation;
- does not require a GUI or target kernel to test.

### 5.9 In-process reference transport

Provide a host-testable reference transport that connects client and provider through the same public contract.

This transport is for:

- contract validation;
- unit/integration tests;
- early first-party app development.

It must not leak in-process assumptions into identifiers or protocol definitions.

### 5.10 Capability enforcement hook

Expose a narrow authorization hook/boundary so a capability/permission layer can decide whether a call is allowed.

Requirements:

- IPC layer supplies caller, service and operation context;
- policy result can allow or deny;
- denial becomes a structured IPC error;
- this workstream does not own grant storage, user prompts or permission UX;
- tests cover allow and deny hooks using fakes/mocks.

If Capability Foundation is not merged, use an adapter trait/interface and record the future integration point.

### 5.11 Cancellation and deadlines

If repository async/runtime primitives support it cleanly, implement:

- cancellation token or request cancellation mechanism;
- deterministic cancellation error;
- optional deadline/timeout metadata;
- no leaked/unfinished provider work in reference tests where practical.

If deadlines would force an inappropriate runtime dependency, implement cancellation first and document deadline support as a bounded follow-up. Do not block all work.

---

## 6. Initial service contract examples

To prove the framework without stealing another workstream's product scope, define one or more **minimal test/example contracts**.

Good examples:

- echo/ping test service;
- key/value test service held entirely in memory;
- fake Files-like metadata query with no real filesystem access.

Do not implement real Files/Wayback/Search/Model functionality here.

If another existing service already has a small stable interface, it may be adapted as a contract test only when changes stay within this workstream's ownership.

---

## 7. Serialization boundary

If messages are serialized:

- reuse repository conventions;
- make schema evolution explicit;
- reject malformed payloads deterministically;
- test round-trips;
- avoid architecture-specific pointers/handles in serialized forms;
- use bounded payload handling where an existing utility exists.

If serialization is not yet needed for the in-process reference transport, keep payload abstraction transport-neutral and document the future serialization boundary rather than prematurely selecting a full protocol stack.

---

## 8. Tests

Minimum coverage:

1. valid/invalid ServiceId;
2. contract version compatibility and rejection;
3. successful provider registration;
4. duplicate registration rejection;
5. unregister behavior;
6. service-not-found behavior;
7. successful call through reference transport;
8. operation-not-found behavior;
9. invalid request behavior;
10. structured provider failure;
11. authorization allow path;
12. authorization deny path;
13. cancellation behavior where implemented;
14. request/correlation ID preservation;
15. malformed serialization handling where serialization exists;
16. two independent registry instances remain isolated;
17. provider replacement/unregister edge cases;
18. concurrency behavior if the API claims thread-safe/concurrent use.

Tests must be offline and deterministic.

---

## 9. Documentation

Document:

- client/provider model;
- service identity/versioning;
- registration/discovery behavior;
- request/response/error model;
- capability hook boundary;
- transport-neutral guarantees;
- what the reference in-process transport does and does not guarantee;
- how future Files/Activity/Wayback/Search/Models services should adopt the contract;
- sample service implementation and sample client call.

A simple flow diagram in Markdown is useful if consistent with repository docs.

---

## 10. Workstream state and integration proposal

Use DF-01 state conventions.

Record:

- workstream ID and branch;
- base and current HEAD;
- owned paths;
- completed acceptance items;
- tests/checks and results;
- unresolved blockers;
- dependency status;
- any proposed shared registry entry;
- public contract paths intended for later consumers.

If shared workstream registration is outside ownership, create a registration proposal rather than modifying the shared registry.

---

## 11. Verification

Run relevant verification such as:

- crate/module tests;
- integration/contract tests;
- all-target tests where reasonable;
- format;
- warnings-denied Clippy/lint according to repository convention;
- package/workspace check;
- `./nagi doctor` when appropriate;
- `nagi dev verify` or equivalent workstream verification if registered/available.

Do not require M18 target browser linking to validate this independent foundation.

---

## 12. Long-running execution policy

After the initial repository audit, continue implementing.

Use this loop:

1. select the highest-value incomplete acceptance item;
2. implement;
3. run focused tests;
4. treat failures as the next debugging target;
5. isolate cause;
6. fix owned code;
7. rerun tests;
8. add missing contract/negative tests;
9. update docs/state;
10. run final quality gates;
11. commit;
12. push;
13. verify clean worktree and remote HEAD.

Do not stop merely because another workstream is not yet merged. Use adapters/proposals and continue all independent work.

---

## 13. Git safety

- Preserve current worktrees and unrelated changes.
- No destructive reset.
- No force push.
- Do not commit files owned by another active workstream without explicit necessity and justification.
- Prefer a dedicated worktree/branch.
- Commit only this workstream's changes.

Final report must state:

- status;
- branch;
- pushed HEAD SHA;
- clean/dirty worktree;
- tests run;
- key implementation paths;
- any remaining Integration Owner actions.

---

## 14. Acceptance criteria

- [ ] Typed validated service identity
- [ ] Versioned service contract model
- [ ] Request/correlation identity
- [ ] Transport-neutral request/response boundary
- [ ] Structured IPC/service errors
- [ ] Service descriptor
- [ ] Registration/discovery abstraction
- [ ] Client abstraction
- [ ] Provider abstraction
- [ ] In-process reference transport
- [ ] Capability/authorization hook without owning policy
- [ ] Cancellation semantics where repository primitives permit
- [ ] Version/service/operation failure handling
- [ ] Deterministic offline contract tests
- [ ] Example service/client proving the contract
- [ ] No implementation of unrelated Files/Wayback/Search/Model product behavior
- [ ] Documentation complete
- [ ] Workstream state complete
- [ ] Shared integration proposal recorded if needed
- [ ] Format/lint/check/tests pass for owned scope
- [ ] Commit and push complete

