# M24 real embedding provider — registration proposal (hark-m24-embedding)

Status of this document: PROPOSAL. Nothing in the shared registry
(`.dev/workstreams.json`), shared CI, root workspace, `third_party/*.lock`,
`THIRD_PARTY_NOTICES.md`, or `docs/implementation_status.md` is changed by this
branch. Every shared-file change below is a minimal diff for the owning
integrator to apply.

## Identity

| Field | Value |
| --- | --- |
| Workstream id (proposed) | `hark-m24-embedding` |
| Milestone | Nagi OS 0.1 M24 — Embedding / Semantic AI (model-backed `EmbeddingProvider` slice only) |
| Owner | Hark M24 embedding stream |
| Branch | `hark/m24-embedding` |
| Base SHA | `edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4` (`origin/main` at start) |
| Recommended worktree | `../wt-m24-embedding` |
| Merge boundary | Draft PR to `main` only; no merge, no ready-for-review, no release image change without a separate user/integration decision |

## Owned paths (allowed)

- `crates/nagi-embedding-provider/**` — standalone crate, own `[workspace]`, own `Cargo.lock`
- `tools/embedding/**` — pinned download, checksum, conversion, reference scripts
- `tests/m24-embedding/**` — acceptance runner, corpus, evidence
- `.dev/workstreams/hark-m24-embedding/**` — this proposal, integration proposal, state

## Forbidden paths (read-only or untouched)

- `Cargo.toml`, `Cargo.lock` (root), `user/nagi-init/Cargo.toml`, any shared `Cargo.lock`
- `user/**` (including `user/nagi-search/**`, `user/nagi-init/src/main.rs`,
  `user/nagi-init/src/desktop.rs`, `user/libnagi/src/storage.rs`,
  M19 files client/runtime/search sources and existing M19 tests)
- model service / backend / ModelStore sources (`user/nagi-model-manager/**`)
- `tools/nagi-cli/**` command tables, `tools/nagi-bootstrap/**`
- `.github/workflows/**`, `.dev/workstreams.json`, `.dev/schemas/**`
- `third_party/**` (including `models.lock`, `sources.lock`), `THIRD_PARTY_NOTICES.md`
- `docs/implementation_status.md`, `docs/workstreams/**`
- release image / `out/**`, `target/**`
- 0.2 SEARCH-CORE-01 M30 activation gate (not reused or lifted)

## Read-only dependencies (existing APIs used, never copied or forked)

| API | Location | Use |
| --- | --- | --- |
| `EmbeddingProvider` trait (`embed(&self, EmbeddingPurpose, &str) -> Result<Embedding, SemanticError>`) | `user/nagi-search/src/semantic.rs` | implemented by the new provider |
| `EmbeddingPurpose::{Query, Passage}` | same | mapped to `query: ` / `passage: ` prefixes |
| `EmbeddingSpaceId([u8; 32])` | same | provider-neutral space fingerprint attached to every vector |
| `Embedding::try_from_values_in_space` | same | final normalization + space tagging |
| `SemanticError` | same | coarse error surface of the trait; the crate also exposes a detailed error |
| `MAX_SEMANTIC_QUERY_BYTES`, `MAX_SEMANTIC_CHUNK_BYTES` | same | input byte caps |
| `PersistentVectorIndex`, `VectorIndex`, `chunk_text` | `user/nagi-search/src/{persistent_semantic_index,semantic}.rs` | tests only (space mismatch, end-to-end neighbor search) |

Dependency form: `nagi-search = { path = "../../user/nagi-search" }` from the
standalone crate. A path dependency does not edit the dependency and does not
join the root workspace; resolution is recorded only in
`crates/nagi-embedding-provider/Cargo.lock`.

## Registry entry (proposed minimal diff to `.dev/workstreams.json`)

Append one object to `workstreams` (keep-both resolution if other rows land first):

```json
{
  "id": "hark-m24-embedding",
  "owner": "Hark M24 embedding stream",
  "owner_branch": "hark/m24-embedding",
  "recommended_worktree": "../wt-m24-embedding",
  "state_file": ".dev/workstreams/hark-m24-embedding/state.json",
  "dependencies": ["development-foundation"],
  "allowed_paths": [
    "crates/nagi-embedding-provider/**",
    "tools/embedding/**",
    "tests/m24-embedding/**",
    ".dev/workstreams/hark-m24-embedding/**"
  ],
  "forbidden_paths": [
    "Cargo.toml", "Cargo.lock", ".github/workflows/**", ".dev/workstreams.json",
    ".dev/schemas/**", "docs/implementation_status.md", "third_party/**",
    "THIRD_PARTY_NOTICES.md", "user/**", "kernel/**", "loader/**", "tools/nagi-cli/**",
    "tools/nagi-bootstrap/**", "out/**", "target/**"
  ],
  "activation_gate": "Host provider, conversion tooling and host real-inference tests only. Guest wiring (nagi-init/M19 runtime, ModelStore placement, model service routing, release image) requires an explicit integration-owner checkpoint naming hark-m24-embedding.",
  "merge_boundary": "Draft PR from hark/m24-embedding to main; merge only by explicit owner decision."
}
```

Condition (AGENTS.md / registry convention): the shared CI test
`dev_status_resume_and_verify_read_the_registered_workstream` requires the
current branch to be registered in `.dev/workstreams.json`. Until the
integration owner applies the row above, shared CI on this branch may fail
that one test for registration reasons only (same situation recorded by
SEARCH-CORE-01). Independent work in owned paths continues meanwhile.

## Model selection evidence summary

See `integration-proposal.md` § Model selection for full evidence. Chosen:
`intfloat/multilingual-e5-small` at immutable Hugging Face commit
`614241f622f53c4eeff9890bdc4f31cfecc418b3`, license MIT; runtime = Nagi-owned
no_std Rust BERT encoder + SentencePiece-Unigram tokenizer (pinned
llama.cpp `c85b92c6` does not support this model's tokenizer through its
`BertModel` conversion path).
