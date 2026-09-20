# Decision and Generative AI Architecture

Status: normative Nagi OS 0.1 architecture alignment

This document defines the capability and provider boundary for Nagi AI. The
primary implementation specification remains authoritative for the Nagi 0.1
milestones; this document supplies the shared terminology and contracts used
by those milestones.

## 1. Purpose

Nagi depends on typed AI capabilities and policy-bounded provider interfaces,
not on a particular model family, vendor, cloud service, or inference
architecture. A future specialized decision model can therefore be added or
replaced without redesigning Nagi Core.

The architecture preserves these existing boundaries:

- AI is user-space and untrusted.
- AI is optional for ordinary OS, GUI, keyboard, and mouse operation.
- Nagi is offline-first and does not require a cloud decision service.
- Kernel authority, capabilities, permissions, policy, and execution remain
  deterministic boundaries.
- Mutating work remains connected to Transaction, Undo, Wayback, and the AI
  Activity Ledger contracts.

## 2. Terminology

`Decision capability` is the architectural concept. `System 1` is a useful
descriptive label for a low-latency, bounded decision path, not a Nagi Core
type, model requirement, or permanent product taxonomy. A provider may use a
different internal technique or name while implementing the same typed
decision capabilities.

- **Tier 0 — Deterministic Fast Path:** parsing and known state transitions
  that do not require probabilistic inference.
- **Tier 1 — Decision Path:** bounded boolean, choice, score, ranking,
  classification, routing, candidate-pruning, or batch decisions.
- **Tier 2 — Generative / Reasoning Path:** open-ended language
  understanding, complex intent interpretation, reasoning, planning,
  summarization, explanation, and generation.
- **Provider:** a user-space implementation of one or more typed capability
  contracts. A provider is not an authority grant.
- **Role:** a policy-facing use such as `standard`, `lite`, `decision`, or
  `embedding`. Roles are distinct from model identity.
- **Capability:** the operation a caller requests, such as
  `text.generate`, `structured.generate`, `decision.choice`, or
  `decision.batch`.

## 3. Three AI lanes

All lanes feed the same constrained action boundary:

```text
User / App / constrained Context
              |
              v
       Context Resolver / Router
          /         |          \
         v          v           v
   Tier 0       Tier 1        Tier 2
 deterministic  Decision      Generative /
 Fast Path      capability    Reasoning
         \         |           /
          +--------+----------+
                   v
          Action / NagiPlan candidate
                   v
       Deterministic Validator / Policy
          / Permission / Executor
                   v
          Transaction / Undo / Ledger
```

### 3.1 Tier 0 — Deterministic Fast Path

Known commands and bounded state transitions should bypass model loading and
inference whenever possible. Examples include a recognized volume change,
mute, opening a known application, or showing system status. Failure in a
probabilistic lane must not be used as a reason to weaken a Tier 0 policy.

### 3.2 Tier 1 — Decision capabilities

Tier 1 selects or filters from bounded inputs. The contract must be able to
represent at least:

- `Boolean`
- `Choice`
- `Score`

The same capability family may later expose ranking, classification, routing,
candidate pruning, and bounded batch evaluation. Tier 1 is not a free-form
text-generation contract. Its output is a candidate for deterministic
validation, never an authorization decision.

### 3.3 Tier 2 — Generative / Reasoning capabilities

Tier 2 covers open-ended language understanding, multi-step reasoning,
structured plan generation, summarization, explanation, and content
generation. Its output remains untrusted and must pass the same Validator,
Policy, Permission, and Executor boundaries as Tier 1 output.

## 4. Provider model

Nagi routes by requested capability, role, policy, availability, and resource
constraints. Applications request a capability or role; they do not select a
vendor or model name when a capability request is sufficient.

The conceptual provider families are:

```text
Model Router
├── GenerativeProvider
│   └── local Generative LLM runtime (Nagi 0.1: llama.cpp / GGUF)
├── DecisionProvider
│   ├── LlmDecisionAdapter (Nagi 0.1 fallback)
│   ├── future local specialized decision provider
│   └── future optional external/provider adapter
├── EmbeddingProvider
├── SpeechToTextProvider
└── TextToSpeechProvider
```

`DecisionProvider` and `GenerativeProvider` are conceptual typed boundaries
for future user-space implementation. They must not be introduced as kernel
APIs, and this checkpoint does not implement them.

### 4.1 DecisionProvider contract

The contract is provider-neutral and must support these concepts:

`DecisionRequest` contains a stable request ID, `DecisionKind`, bounded input
or candidate set, constrained context, requested capability, sensitivity and
privacy classification, optional latency budget, local-only/cloud-allowed
policy, caller identity, and applicable `AppId`, `AppSessionId`, `ObjectId`,
`WorkspaceId`, and `NodeId` context.

`DecisionResult` contains a selected bounded value or bounded result, optional
score/distribution and confidence, provider/model identity, capability
metadata, and fallback/escalation metadata. It contains neither a
deterministic validation state nor an authority grant. Providers that do not
produce confidence must be representable without inventing one.

`DecisionBatchRequest` and `DecisionBatchResult` extend the same boundary for
bounded collections of independent decisions. Batch limits, privacy policy,
per-item result bounds, and failure semantics remain explicit; an
implementation must not require unbounded raw distributions or hidden
reasoning storage.

A provider may reject an unsupported capability, exceed a bound, or report
unavailability. Those outcomes are routed through the fallback policy rather
than treated as permission to execute.

### 4.2 GenerativeProvider contract

`GenerativeProvider` exposes the capability needed for language and
structured-generation work, including `text.generate`,
`structured.generate`, and `reasoning` where supported. It returns an
untrusted candidate subject to schema validation and policy. Provider
identity, model identity, runtime/backend, locality, and health are metadata
for routing and observability, not application authority.

For Nagi 0.1, the Generative LLM runtime is the local `llama.cpp` / GGUF path.
That choice does not make `llama.cpp` or GGUF the runtime contract for
DecisionProvider, EmbeddingProvider, STT, TTS, or future specialized
providers. `LlmDecisionAdapter` may reuse the local generative runtime when a
dedicated decision provider is unavailable, but this is an adapter choice,
not a global restriction.

## 5. Model Router

The Router considers, at minimum:

- requested capability and role;
- provider/model availability and health;
- local/offline state and privacy/sensitivity policy;
- latency and memory budgets;
- loaded state and resource pressure;
- user routing preference;
- fallback availability.

The Router must not make `Granite`, `Qwen`, `Gemma`, or any future provider
name the application contract. A model name may appear in an administrative
selection or diagnostic view, but normal callers request a capability or
role.

Routing or escalation based on confidence is allowed only inside this
bounded selection policy. It does not alter Permission, Capability, Policy,
Owner, or Executor decisions.

## 6. Model Manager and capability registry

The Model Manager is a capability registry and lifecycle manager, not an LLM
name list. Registry metadata may include:

- stable model/provider identity and display metadata;
- supported capabilities and `DecisionKind` values;
- batch and confidence support;
- role and preference metadata;
- local/cloud classification and runtime/backend;
- version, architecture, parameter count, quantization, size, RAM guidance;
- languages, license/source, integrity hash, and modification state;
- availability, loaded state, health, and privacy constraints.

Capability examples include `text.generate`, `structured.generate`,
`reasoning`, `decision.boolean`, `decision.choice`, `decision.score`,
`decision.ranking`, `decision.classification`, `decision.routing`,
`decision.candidate_pruning`, `decision.batch`, `embedding`, `speech.stt`,
`speech.tts`, `vision`, and `code`.

The existing Nagi 0.1 model policy remains unchanged: IBM Granite 4.2 3B is
the Default Standard LLM, Qwen3 4B is the alternative Standard LLM, and
Gemma 3 1B is the Lite LLM. These models may satisfy different requested
capabilities as supported, but their names are not a substitute for the
capability contract.

The existing model-store compatibility is preserved. A future decision model
may be classified separately from generative, embedding, STT, and TTS models
without forcing a 0.1 physical directory migration.

`crates/nagi-model` remains the common identity/model-level crate containing
types such as `AppId`, `NodeId`, and `ObjectId`. It is not the AI runtime,
Model Manager, or provider implementation crate.

## 7. Fallback and escalation

The standard decision path is:

```text
DecisionRequest
  -> preferred compatible DecisionProvider
  -> compatible local DecisionProvider, if available
  -> LlmDecisionAdapter using an existing local GenerativeProvider
  -> Generative / Reasoning path where the task permits
  -> deterministic filtering or manual UI fallback where applicable
```

The exact fallback is constrained by the request's sensitivity, capability,
and policy. A dangerous action does not become safe merely because a provider
failed or a fallback was selected. Every candidate still passes deterministic
validation, policy, permission, and execution checks.

If all AI lanes are unavailable, ordinary OS and GUI operation continues.
Core functionality must not require a DecisionProvider, a specialized local
model, Jev, a cloud service, or network availability.

Confidence may influence routing, escalation, candidate pruning, ranking,
review requests, or an existing user-confirmation strategy. It must never
grant or strengthen a capability, authorize a permission, invoke Owner
override, or bypass Validator/Policy/Permission/Executor.

## 8. Action Registry integration

The Action Registry remains the source of bounded action schemas and policy
metadata. The preferred path is:

```text
Intent / constrained Context
  -> Action Registry
  -> deterministic filtering
  -> DecisionProvider or LlmDecisionAdapter
  -> small relevant action set
  -> Generative Planner / NagiPlan
  -> Validator / Policy / Permission
  -> Executor
```

The complete Action Registry must not be passed to a model on every request.
If a DecisionProvider is absent, deterministic filtering and the local
adapter/reasoning path remain available. No action is executed directly from
a decision result.

## 9. Semantic Search integration

Semantic search is not a prerequisite for DecisionProvider. A future local
workflow may use:

```text
lexical / metadata / embedding retrieval
  -> bounded candidate set
  -> decision ranking or pruning
  -> reasoning/summarization
```

Permission filtering and sensitivity rules apply before candidates are given
to any provider. Exact and metadata search remain valid when semantic AI is
disabled, and an embedding provider remains replaceable.

## 10. Activity, Transaction, Undo, and Wayback

Decision activity may record the request category, provider/model identity,
selected result, bounded confidence/score summary when useful, fallback,
escalation, denial, resulting Action/Plan, and Transaction ID. It must not
persist hidden chain-of-thought, credentials, API keys, unrestricted sensitive
context, or unlimited batch raw distributions.

Any side effect remains associated with the existing Activity Ledger,
Transaction, Undo, and Wayback behavior. A decision result is not itself a
transaction, permission, or proof that an external/destructive operation was
completed.

## 11. Offline and cloud policy

Local providers are preferred under Offline-first policy. A future cloud
DecisionProvider is optional and must be explicitly configured and separately
authorized. It must obey local-only resource policy, sensitivity checks,
network capability separation, credential protection, privacy-preserving
diagnostics, and offline fallback.

Nagi 0.1 does not require a cloud LLM or cloud DecisionProvider. Jev is not a
0.1 dependency, release blocker, or standard route.

## 12. Security boundary

All providers are untrusted user-space components. They receive only
constrained context and handles permitted by existing policy. They cannot
create or strengthen capabilities, elevate privileges, invoke Owner override,
execute arbitrary shell commands, or call kernel authority directly.

The only valid side-effect path is:

```text
Provider output
  -> schema/semantic validation
  -> deterministic Policy / Permission checks
  -> capability-checked Executor
  -> Transaction / Activity Ledger / Wayback where applicable
```

This boundary is identical for Tier 1 and Tier 2. A confidence, score,
probability, provider reputation, or model identity never becomes authority.

## 13. Nagi 0.1 implementation boundary

This checkpoint establishes architecture and specification only. Nagi 0.1
must preserve the following future-compatible shape:

- a typed `GenerativeProvider` boundary for the local Granite/Qwen/Gemma
  generative path;
- a typed `DecisionProvider` contract and `LlmDecisionAdapter` fallback;
- capability/role-based routing and Model Manager metadata;
- deterministic fallback, validation, policy, permission, executor, and
  activity/transaction boundaries;
- deterministic and real-model test separation.

Nagi 0.1 does not require a dedicated local System 1 model, Jev, any Jev SDK
or API, an external Decision API, an NPU/GPU decision runtime, or a
probabilistic kernel policy. The contract must make those future additions
replaceable without changing kernel authority or application action schemas.

## 14. Non-goals

This document does not implement a provider, runtime, model package, cloud
connector, kernel API, IDL, Action Registry, or Model Manager. It does not
change the Granite default, introduce Jev, prescribe an internal machine
learning architecture, or make `System 1` a permanent Nagi API name.

## 15. Related specifications

- [Nagi OS 0.1 primary implementation specification](../Nagi_OS_0.1_Codex_Implementation_Spec.md)
- [First-Party Models specification](../NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md#67-models)
- [Nagi implementation status](../implementation_status.md)
- [Decision ADR](../decisions/0018-decision-and-generative-model-architecture.md)
- [Common identity/application architecture](unified-device-application-model.md)
- [Language architecture](language-architecture.md)
