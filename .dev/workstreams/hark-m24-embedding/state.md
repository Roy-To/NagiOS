# hark-m24-embedding — state (human summary)

Machine-readable durable state: `state.json` (follows
`.dev/schemas/workstream-state.schema.json`; referenced by the proposed
registry row in `registration-proposal.md`). This file is a short summary.

- Status: **PARTIAL** — host real inference implemented and measured; guest
  (Nagi/QEMU) inference NOT RUN; branch-local dedicated CI
  (`.github/workflows/hark-m24-embedding.yml`) green on x86_64 (push run
  37872411191 at `1f30c30`) — host CI only, not a substitute for owner
  registration (still missing).
- Base: `edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4`; branch `hark/m24-embedding`.
- Model: `intfloat/multilingual-e5-small` @ `614241f622f53c4eeff9890bdc4f31cfecc418b3`, MIT.
- Runtime: Nagi-owned no_std Rust encoder + SentencePiece-Unigram tokenizer
  (pinned llama.cpp `c85b92c6` cannot tokenize this model correctly; see
  `integration-proposal.md` §1).
- Converted artifact: `multilingual-e5-small.nemb`, 474,604,256 bytes, SHA-256
  `7fb0a34528feecae52e13a3cb0ef6a0edcbab981ae8926c585373b1a4d71a287`; its header
  embeds the weights and tokenizer SHA-256 and the revision
  (`tools/embedding/verify_pins.py`).
- Cross-architecture reference vectors: regenerated onnxruntime vectors are
  accepted against the committed aarch64 copy only if texts and token ids are
  exact, `tokenizer_parity.json` is byte-identical, worst cosine >= 0.999999
  AND worst max |diff| <= 1e-5, and the pinned artifacts and `.nemb` digest are
  unchanged. Fixed thresholds: beyond a bound = FAIL, investigate, never
  loosen. Reference match only; no other acceptance threshold changes.
  x86_64 runs 37872411191 / 37872863618: worst max |diff| 1.0e-7 (PASS);
  record `tests/m24-embedding/evidence/x86_64-reference-tolerance-20261009.json`.
- Deadline/cancel: cooperative checkpoints from call entry through pooling,
  30 s default budget; guarantees in the crate docs.

## Verify

```sh
tests/m24-embedding/run.sh            # host; downloads ~470 MB once
tools/embedding/regenerate_reference.sh   # hash-pinned reference regeneration
```

Shared repository CI does not build this standalone crate and is not evidence
for it.

## Open gates

1. Registry row (integration owner) — `registration-proposal.md`; keep or fold
   the dedicated workflow into shared CI (shared CI owner) —
   `integration-proposal.md` §3.5.
2. `third_party/models.lock`, `THIRD_PARTY_NOTICES.md`,
   `docs/implementation_status.md` minimal diffs — `integration-proposal.md` §3.
3. Guest inference wiring (nagi-init / M19 runtime / model service / ModelStore)
   — Codex-owned; required before M24 can move beyond PARTIAL.
4. Hybrid ranking, explanations, content-producer sync — outside this slice.
