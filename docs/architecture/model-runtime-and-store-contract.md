# Model Runtime and Model Store Contract

`user/nagi-model-manager` is the user-space foundation for model metadata,
discovery, compatibility, selection, provider sessions, and local Store state.
It is separate from `crates/nagi-model`, which remains the shared identity
crate. The contract does not implement a production inference engine, model
downloader, filesystem service, or App SDK/Capability integration.

## Manifest v1

The machine-readable schema is
[`../schemas/nagi-model-manifest-v1.schema.json`](../schemas/nagi-model-manifest-v1.schema.json).
`ModelManifest::parse_json` applies the strict typed JSON shape and semantic
validation in the crate. Unknown fields are rejected; schema and runtime API
versions are explicit. A schema update is a versioned compatibility change.

The manifest separates model identity/version, provider/family, artifact
format and opaque Store reference, capabilities and modalities, context and
resource limits, backend/architecture compatibility, roles, license/terms and
NOTICE requirements, optional source pin, and optional integrity data.
`ArtifactReference::ModelStore` cannot encode a host filesystem path. A source
pin is accepted only with a positive artifact size and SHA-256 metadata. A
local model with no verified SHA-256 is descriptive metadata only: the registry
marks it `MissingIntegrity`, the Store rejects install intent, and the runtime
refuses to load it.

Manifest v1 also accepts optional `runtime_class` and `resource_class` IDs.
They are additive so older v1 profiles remain valid. Runtime class IDs are open
data (`generative_llm`, `system_one`, or a future class); backend descriptors
declare the classes and capabilities they support. Resource class is a named
tier for policy and display; numeric manifest requirements remain authoritative
for fit checks. Availability and lifecycle are registry entry state, not claims
made by a static manifest.

The three files under `user/nagi-model-manager/tests/fixtures/` are typed
manifest examples for Qwen3 4B, Granite 4.2 3B, and Gemma 3 1B. The current
contract test checks the schema document's JSON syntax and selected version,
required-field, and closed-object declarations, then checks that each example
passes the strict typed parser; it does not run a Draft 2020-12 JSON Schema
evaluator. The examples do not describe downloadable packages:
context/resource values and provider-term references are illustrative, and
source, file size, and integrity are unset. Before any model can be
distributed, replace those placeholders with the exact model revision,
filename, hash, authoritative license/terms reference, and required notices.
No third-party license text is copied into the fixtures.

## Discovery and selection

`ModelRegistry::discover` validates one manifest at a time and records a
structured availability reason: missing artifact, missing or mismatched
integrity, unsupported runtime/backend, resource incompatibility, or available. A
rejected manifest leaves prior entries untouched. `ModelRegistry::unregister`
removes only unloaded, failed, or disabled entries; it rejects loading, ready,
busy, and unloading entries so a live or transitional model is not detached.
`LifecycleState` separately tracks loading, ready, busy, unloading, failed,
and disabled transitions.

Selection requires a declared capability supported by a compatible backend and
can add a role, minimum context, current RAM/storage/CPU/GPU budget, target
architecture, and offline-only constraint. The caller's preferred model is
considered first, followed by a separate `RoleDefaultPolicy` value, then
available Standard models, Lite models, and stable model-ID order. An
unavailable configured default is excluded before fallback selection.
Provider/model names are not inspected by generic selection logic. The Granite
standard, Qwen alternative, and Gemma lite direction is represented by manifest
role tags plus caller-supplied policy, not by runtime conditionals.

## Provider boundary

`ModelBackend` owns backend-specific load/infer/unload work. Its
`BackendDescriptor` declares supported formats, runtime APIs, and
architectures. `ModelRuntime::load` accepts a `ModelArtifactReader` only when
the Store artifact ID, declared size, and verified manifest SHA-256 match.
The reader exposes bounded offset reads so a backend can stream or map a large
GGUF artifact without requiring a second whole-file buffer.

`GenerativeProvider` accepts a bounded model request with a stable request ID,
optional caller `AppId`, declared capability, optional system prompt, text
input, provider-neutral sampling options, input-token count when known,
output-token limit, timeout, and cancellation token. It supports both complete
responses and text chunks through a consumer sink. Streaming requires the
model and backend to declare `text.stream`. Cancellation is checked before
invocation and passed through for cooperative cancellation; a backend checks it
between streamed chunks and must honor nonzero deadlines during inference. The
synchronous contract cannot forcibly interrupt an uncooperative backend.
Responses carry provider/model/backend identity and token usage, but no
permissions or authority. Callers still validate all generated content before
any side effect.

Backends expose typed health and resource reports, including active sessions,
concurrency limit when known, and resident memory when reported by the backend.
The manager does not invent machine-specific memory budgets. A loaded session
holds an exclusive mutable borrow of its runtime instance, which limits that
instance to one active session; independent runtime instances remain separate.

Runtime-class and capability IDs are data, so a `system_one` backend or a
`decision.boolean` capability can be described, compatibility-checked, and
selected without vendor-specific registry branches. This crate's invocation
contract is the generative text path; it does not implement a System One model,
DecisionProvider, or `jev`. Non-generative typed providers remain at their
separate architecture boundary and can share the descriptor and registry
contracts.

## Local Store metadata

`ModelStoreRecord` retains the manifest, version/integrity/license data,
acknowledgement state, installation state, update availability, and removal
eligibility. Install intent requires an integrity digest; transitions require
verification before `Installed`;
failed updates preserve the prior installed version and retry target. A model
whose terms require acknowledgement cannot enter install until the exact
manifest terms reference is acknowledged. Required NOTICE entries remain
separate metadata. Invariant-bearing record fields are private and exposed
through read-only accessors; callers can change them only through the checked
acknowledgement, install, update, transition, and removal methods. Removal can
be rejected while in use or marked as required by the system.

This is a deterministic service contract only. It does not write files,
download models, or alter the existing VFS/Capability contracts.
