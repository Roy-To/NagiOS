# ADR-0018: Decision and Generative Model Architecture

- Status: Accepted
- Date: 2026-09-19
- Applies after: M16 PASS and before M17 implementation; governs the future
  M20+ AI milestones

## Context

Nagi's existing AI plan correctly keeps AI local/offline-first, user-space,
untrusted, and behind deterministic planning, validation, policy, permission,
and execution boundaries. Its forward-facing descriptions, however, were
centered on a single generative LLM path: `nagi-ai-runtime -> llama.cpp ->
GGUF`. Future decision models may provide bounded boolean, choice, score,
ranking, routing, classification, candidate-pruning, or batch capabilities
without being generative LLMs or using that runtime.

Nagi needs a provider boundary that can accept such capabilities without
making Nagi Core depend on a model name, vendor, cloud service, or inference
architecture. The boundary must also preserve the existing Granite default,
Qwen/Gemma policy, offline operation, AI-disabled usability, capability
security, transaction/undo/Wayback, and Activity Ledger rules.

## Decision

Nagi uses capability- and role-based typed provider contracts. The shared
concepts are `GenerativeProvider`, `DecisionProvider`, `DecisionRequest`,
`DecisionResult`, `DecisionBatchRequest`, `DecisionBatchResult`,
`DecisionCapability`, and `DecisionKind`. `System 1` is descriptive language
for a bounded decision path; `Decision capability` is the durable architectural
concept.

Nagi AI is organized into three lanes:

1. Deterministic Fast Path for known commands and state transitions.
2. Decision Path for bounded decisions, including Boolean, Choice, Score, and
   future ranking/classification/routing/candidate-pruning/batch forms.
3. Generative/Reasoning Path for language understanding, reasoning, planning,
   summarization, explanation, and generation.

The Model Router and Model Manager select by capability, role, availability,
local/offline state, privacy/sensitivity policy, latency, memory pressure,
loaded state, provider health, and fallback availability. Applications do not
hardcode vendor/model names when a capability or role can be requested.

For Nagi 0.1, the GenerativeProvider path uses the local Granite/Qwen/Gemma
policy and the `llama.cpp` / GGUF Generative LLM runtime. `llama.cpp` and GGUF
are not the universal runtime contract for DecisionProvider or other provider
families.

When a dedicated DecisionProvider is unavailable, Nagi may satisfy the same
typed DecisionProvider contract through `LlmDecisionAdapter` backed by an
existing local GenerativeProvider. The standard fallback is therefore local
and Jev-free. A future specialized local provider or optional external
provider can be substituted at the DecisionProvider boundary.

All provider outputs remain untrusted candidates. Confidence, score, or
probability may affect routing, escalation, ranking, candidate pruning,
review, or an existing confirmation strategy, but never grants or strengthens
authority. Every side effect continues through deterministic validation,
policy, permission, capability-checked execution, and the existing
Transaction, Undo, Wayback, and Activity Ledger boundaries.

IBM Granite 4.2 3B remains the Default Standard LLM. Qwen3 4B remains the
alternative Standard LLM and Gemma 3 1B remains the Lite LLM. No specialized
decision model, Jev SDK/API, cloud decision service, or decision runtime is a
Nagi 0.1 dependency or release blocker.

## Alternatives considered

### Keep a single generative-model contract

Rejected. It would make future bounded decision providers appear to be
generative LLMs and would encourage applications to depend on the current
runtime rather than on capabilities.

### Make a dedicated System 1 subsystem mandatory in Nagi 0.1

Rejected. It would increase the 0.1 dependency surface, complicate offline
operation, and turn an optional optimization into a release requirement.
The typed DecisionProvider boundary and local adapter provide the required
architecture without requiring a native decision model.

### Integrate Jev directly into Nagi Core

Rejected. Jev is a possible future provider, not a Nagi contract. Direct SDK,
API, or Jev-specific types would create vendor coupling and would make cloud
availability a misleading part of the standard Nagi experience.

### Let decision confidence authorize actions

Rejected. Confidence is probabilistic metadata and cannot replace
capabilities, permissions, policy, deterministic validation, or user consent
requirements for external, destructive, credential, or privilege-changing
actions.

## Why retain the deterministic Fast Path

Known operations are faster, more diagnosable, and available when models are
disabled, unloaded, unavailable, or under memory pressure. It also reduces
unnecessary context exposure and keeps ordinary OS/GUI operation independent
of AI availability.

## Why `llama.cpp` remains the Generative runtime

The existing Nagi 0.1 plan uses a local Granite/Qwen/Gemma generative path
through `llama.cpp` and GGUF. Retaining that boundary preserves the current
model policy and implementation sequence while limiting its scope to
GenerativeProvider. Future DecisionProvider, embedding, speech, or specialized
runtime choices remain replaceable.

## Why a native System 1 model is optional in 0.1

Nagi 0.1 can establish and validate the typed contract, routing rules,
fallback behavior, security boundary, and deterministic acceptance behavior
with `LlmDecisionAdapter` and local generative models. Requiring a specialized
model would add a dependency without improving the core authority model.

## Security consequences

The architecture preserves user-space/untrusted AI, constrained context,
least privilege, capability attenuation, deterministic Validator/Policy/
Permission/Executor checks, and no AI Owner override. A provider result is
never an authority grant. Decision and Generative lanes share one side-effect
boundary, so a specialized provider cannot bypass existing safety controls.

## Offline consequences

Nagi 0.1 remains fully viable without Jev, any cloud DecisionProvider, or any
network connection. If a preferred provider is unavailable, routing may use a
compatible local provider, `LlmDecisionAdapter`, a suitable reasoning path,
or deterministic/manual fallback. If AI is disabled, ordinary OS and GUI
operation continues.

## Milestone consequences

- M17 Servo Bootstrap is unaffected and remains NOT STARTED at this checkpoint.
- M20 establishes the local GenerativeProvider/runtime and routing foundation;
  it does not require a native System 1 model.
- M21 integrates DecisionProvider, `LlmDecisionAdapter`, bounded routing,
  action preselection, fallback, and deterministic plan validation.
- M22 applies the same safety, transaction, undo, Wayback, and Activity
  boundary to Decision and Generative output.
- M23/M24 may pass constrained context and bounded search candidates through
  the same provider contracts; Decision is not required for semantic search.
- M26 extends automatic routing across roles/capabilities and preserves
  Granite as Default Standard without treating Jev as a 0.1 requirement.

## Future provider compatibility

A future local specialized decision model, Jev-like provider, or other
decision technology may implement the typed DecisionProvider contract and be
registered by capability, role, locality, policy, and health. Nagi Core and
the Action/Plan security boundary do not need to know its vendor or internal
inference method. `crates/nagi-model` remains the common identity/model-level
crate and is not repurposed as an AI runtime/provider crate.

## Consequences

- Architecture documentation gains a stable decision capability vocabulary.
- Nagi 0.1 retains the existing local model policy and generative runtime.
- Provider-specific optimization remains optional and replaceable.
- Future implementation must keep deterministic and probabilistic test paths
  separate, with CI independent of Jev availability.
- This ADR changes specifications only; it does not implement a provider,
  runtime, IDL, dependency, or acceptance behavior.
