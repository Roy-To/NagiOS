# M24 embedding provider — measured evidence (HOST only)

**Scope.** Everything below is host inference on a development machine
(aarch64 Linux, 2 vCPU). No guest (Nagi/QEMU) inference was run; M24 remains
`PARTIAL`. The Nagi user target was compiled, not executed.

## Artifact

| Item | Value |
| --- | --- |
| Upstream | `intfloat/multilingual-e5-small` @ `614241f622f53c4eeff9890bdc4f31cfecc418b3` (MIT) |
| Inputs | `model.safetensors` `1a55775f…c81c98477`, `tokenizer.json` `0b44a9d7…6ebdf4c39` (SHA-256 = HF LFS oid) |
| Converted | `multilingual-e5-small.nemb`, 474,604,256 bytes, SHA-256 `7fb0a34528feecae52e13a3cb0ef6a0edcbab981ae8926c585373b1a4d71a287` |
| Determinism | two independent conversions produced byte-identical output |

## Results (`tests/m24-embedding/run.sh`)

| Check | Result |
| --- | --- |
| Token-id parity vs HF `tokenizers` (788 localization + seeded multi-script random strings, incl. NUL, ZWJ emoji, combining marks, half/full-width) | 788/788 identical |
| Vector parity vs upstream `onnx/model.onnx` via onnxruntime 1.30.0 (21 JA/EN/edge-case inputs) | worst cosine 0.999999949, worst max \|Δ\| 1.42e-7 |
| JA/EN semantic neighbors through canonical `PersistentVectorIndex` (10 docs, 16 queries) | 16/16 meet rank requirement; 13/16 at rank 1 |
| `"the Servo article I looked at yesterday"` | rank 1: Japanese Servo article (0.8749), rank 2: English Servo doc (0.8569) |
| Cross-lingual paraphrase (JA↔EN Rust ownership) vs unrelated JA | 0.8938 vs 0.7799 |
| Known weakness (recorded, not hidden) | cross-lingual `"how to cook pumpkin"` → JA pumpkin recipe rank 2 (0.8026) behind EN bread recipe (0.8215); `"パンの焼き方"` → EN bread rank 2; `"how to write a patent specification"` → JA patent doc rank 2 |
| Load (SHA-256 verify + parse + decode) | 1.7–2.2 s |
| Latency (single thread, opt-level 3) | 11 tokens 33–51 ms; 138 tokens 0.43–0.54 s; 490 tokens 2.0–2.6 s |
| Peak RSS (load + one embedding) | 552 MiB |
| Model-free contract tests (missing / not-pinned digest / every truncation / corrupt header fields / NaN weight / size cap / empty / over-limit bytes & tokens / invalid config / deadline / space mismatch at load and in index / special-token injection) | 13/13 pass |
| Tokenizer unit tests | 5/5 pass |
| `cargo fmt --check`, `clippy --all-targets -D warnings`, `clippy --lib --no-default-features -D warnings` | pass |
| `cargo -Z build-std=core,alloc build --release --no-default-features --target targets/x86_64-unknown-nagi-user.json` | pass (compile only) |

Real-inference tests are `#[ignore]` by default and fail (not skip) when run
with `--ignored` without `NAGI_EMBEDDING_MODEL`; no fixture vector is counted
as a real-inference pass.

## Not verified

- Guest inference in QEMU (requires Codex-owned nagi-init / model service /
  ModelStore wiring).
- x86-64 reference-machine latency and memory (host here is aarch64).
- The spec's full M24 acceptance (hybrid ranking with explanations over real
  content producers).
