# Architecture

Nagi uses a capability-based hybrid kernel. The kernel owns execution, memory,
IPC, and authority; files, networking, windows, audio, packages, and AI are
user-space services.

The authoritative architecture is defined in
`docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`.

The post-M13 unified device/application model is summarized in
`docs/architecture/unified-device-application-model.md` and decided by
`docs/decisions/0014-unified-device-application-model.md`. A desktop Window is
a Presentation Surface primitive; it is not the universal application root.

The common language rules are defined in
`docs/architecture/language-architecture.md` and decided by
`docs/decisions/0015-language-architecture.md`. English is the canonical
internal language; `en-US` and `ja-JP` are equal first-class Nagi 0.1 user
languages.

The shared first-party UI tokens, components, interaction contracts, and
renderer integration boundary are defined in
[`ui-design-system.md`](ui-design-system.md).

The common AI provider rules are defined in
[`decision-and-generative-ai-architecture.md`](decision-and-generative-ai-architecture.md)
and decided by
[`ADR-0018`](../decisions/0018-decision-and-generative-model-architecture.md).
Nagi
routes by typed capability and role across deterministic, decision, and
generative lanes; specialized Decision Providers remain optional, and the
Nagi 0.1 Generative LLM runtime remains the local `llama.cpp` / GGUF path.

The user-space Model Manager and local Store metadata contracts are documented
in [`model-runtime-and-store-contract.md`](model-runtime-and-store-contract.md).
