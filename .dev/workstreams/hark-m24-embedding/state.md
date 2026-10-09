# hark-m24-embedding — state

- Status: **PARTIAL** (host real inference implemented and measured; guest not run).
- Base: `edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4`; branch `hark/m24-embedding`.
- Model: `intfloat/multilingual-e5-small` @ `614241f622f53c4eeff9890bdc4f31cfecc418b3`, MIT.
- Runtime: Nagi-owned no_std Rust encoder + SentencePiece-Unigram tokenizer
  (pinned llama.cpp `c85b92c6` cannot tokenize this model correctly; see
  `integration-proposal.md` §1).
- Converted artifact: `multilingual-e5-small.nemb` SHA-256
  `7fb0a34528feecae52e13a3cb0ef6a0edcbab981ae8926c585373b1a4d71a287`.

## Done

- Proposals: `registration-proposal.md`, `integration-proposal.md`.
- `crates/nagi-embedding-provider` (own workspace + Cargo.lock) implementing
  `nagi_search::semantic::EmbeddingProvider` read-only by path.
- `tools/embedding`: manifest, fetch (pinned revision + SHA-256), deterministic
  converter, ONNX reference generator, tokenizer parity corpus generator.
- Tests: model-free contract suite, tokenizer unit tests, real-inference suite;
  evidence in `tests/m24-embedding/EVIDENCE.md`.

## Verify

```sh
tests/m24-embedding/run.sh            # host; downloads ~470 MB once
```

## Open gates

1. Registry row + CI step (integration owner) — `registration-proposal.md`.
2. `third_party/models.lock`, `THIRD_PARTY_NOTICES.md`,
   `docs/implementation_status.md` minimal diffs — `integration-proposal.md` §3.
3. Guest inference wiring (nagi-init / M19 runtime / model service / ModelStore)
   — Codex-owned; required before M24 can move beyond PARTIAL.
4. Hybrid ranking, explanations, content-producer sync — outside this slice.
