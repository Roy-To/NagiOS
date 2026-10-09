# M24 real embedding provider — integration proposal

Base SHA: `edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4` · Branch: `hark/m24-embedding`
· Owner: Hark M24 embedding stream · Paths: see `registration-proposal.md`.

M24 status after this stream: **PARTIAL**. Host real inference is implemented
and measured; guest inference is not run by this stream (guest wiring is in
Codex-owned paths). Host inference is never presented as guest inference.

## 1. Model selection (verified from official sources)

| Item | Value | Source |
| --- | --- | --- |
| Model | `intfloat/multilingual-e5-small` (spec §52 candidate) | https://huggingface.co/intfloat/multilingual-e5-small |
| Immutable revision | `614241f622f53c4eeff9890bdc4f31cfecc418b3` (HEAD at selection time, last modified 2026-04-02) | https://huggingface.co/api/models/intfloat/multilingual-e5-small (`sha`) |
| Gated / click-through | No (`gated: false`); no acceptance or paid service involved | same API |
| Model license | MIT (`license: mit` in model card front matter, line 97 of README.md at the pinned revision) | https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/README.md |
| Base model | `microsoft/Multilingual-MiniLM-L12-H384`, license MIT (card front matter) | https://huggingface.co/microsoft/Multilingual-MiniLM-L12-H384 (rev `6e8c1ec6b4ec4e3fc6eb7d2cd834fcd582b61daf`) |
| Tokenizer lineage | XLM-RoBERTa SentencePiece (`tokenizer_class: XLMRobertaTokenizer`), `FacebookAI/xlm-roberta-base` license MIT | https://huggingface.co/FacebookAI/xlm-roberta-base (rev `e73636d4f797dec63c3081bb6ed5c7b0bb3f2089`) |
| Architecture | `BertModel`, 12 layers, hidden 384, 12 heads, FFN 1536, GELU, LayerNorm eps 1e-12, absolute positions (512), type vocab 2, vocab 250037 rows | `config.json` at the pinned revision |
| Pooling | mean over all tokens (`1_Pooling/config.json`: `pooling_mode_mean_tokens: true`), then L2 normalize | `1_Pooling/config.json`, README usage section |
| Prefixes | `query: ` / `passage: ` required ("Each input text should start with ...") | README FAQ §1 |
| Max length | 512 tokens (`model_max_length`, `max_seq_length`); README "Long texts will be truncated to at most 512 tokens" | `tokenizer_config.json`, `sentence_bert_config.json`, README Limitations |
| Training-data disclosure | listed in README Training Details (mC4, CC News, NLLB, Wikipedia, Reddit, S2ORC, Stackexchange, xP3, MS MARCO, NQ, …); paper arXiv:2402.05672 | README |

Pinned input artifacts (SHA-256 equals the Hugging Face LFS `oid`, re-verified after download):

| File | Bytes | SHA-256 |
| --- | --- | --- |
| `model.safetensors` | 470641600 | `1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477` |
| `tokenizer.json` | 17082730 | `0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39` |
| `config.json` | 655 | `69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959` (git blob `60a2a840…`) |
| `onnx/model.onnx` (reference only, not shipped) | 470268510 | `ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665` |

### Runtime decision

**The pinned llama.cpp is not used.** Verified against
`ggml-org/llama.cpp@c85b92c69c955961621193cd51da194f3cbcedf3` (the
`third_party/sources.lock` pin):

- `conversion/bert.py` `BertModel.set_vocab` (lines 40–63) builds the vocab with
  `get_vocab_base()` and writes `tokenizer_model = "bert"` (WordPiece with
  phantom-space mapping). multilingual-e5-small declares `BertModel` but uses an
  XLM-R SentencePiece **Unigram** tokenizer, so this path tokenizes incorrectly.
  https://github.com/ggml-org/llama.cpp/blob/c85b92c69c955961621193cd51da194f3cbcedf3/conversion/bert.py
- No `multilingual-e5` entry exists in `convert_hf_to_gguf_update.py` or in the
  pre-tokenizer hash table of `conversion/base.py` (`get_vocab_base_pre`).
- The SentencePiece path (`_xlmroberta_set_vocab`) is reached only from
  `XLMRobertaModel` / `NomicBertModel`; `XLMRobertaModel` also applies a
  `pad_token_id + 1` position offset (`_xlmroberta_tokenizer_init`, lines 105–112)
  that is wrong for e5-small's BERT absolute positions starting at 0. Forcing the
  architecture would therefore require Nagi-owned patches to llama.cpp, which is a
  Codex-owned runtime.
- The spec allows it: §42 "DecisionProvider, EmbeddingProvider, … may use other
  runtimes"; AGENTS.md "llama.cpp / GGUF is the Nagi 0.1 Generative LLM runtime,
  not the universal runtime contract".

Chosen runtime: **Nagi-owned `no_std` + `alloc` Rust BERT encoder and
SentencePiece-Unigram tokenizer** in `crates/nagi-embedding-provider`,
reading a Nagi container (`.nemb`) converted deterministically from the pinned
`model.safetensors` + `tokenizer.json`. No C/C++ runtime, no host service, no
network at inference time. Third-party Rust dependencies (all permissive, all
`no_std`): `libm` (exp/erf/sqrt), `sha2` (artifact integrity),
`unicode-segmentation` (grapheme handling to match the reference normalizer).
Exact versions and licenses are in `crates/nagi-embedding-provider/Cargo.lock`
and § 4.

Reference used only for verification (never shipped): `onnxruntime` with the
repository's own `onnx/model.onnx` at the same revision, plus Hugging Face
`tokenizers` with the pinned `tokenizer.json`.

## 2. Provider contract

- `E5Provider` implements `nagi_search::semantic::EmbeddingProvider`.
- `EmbeddingPurpose::Query` → `"query: " + text`, `Passage` → `"passage: " + text`.
- `EmbeddingSpaceId` = SHA-256 over a canonical, provider-neutral descriptor:
  `"nagi.embedding-space.v1\0"` ‖ converted-artifact SHA-256 ‖ dims ‖ pooling
  (`mean`) ‖ normalization (`l2`) ‖ prefix scheme. Changing weights, tokenizer,
  pooling or prefixes changes the space; no vendor/model name is encoded.
- Bounded failures: empty/whitespace-only input; input byte caps
  (`MAX_SEMANTIC_QUERY_BYTES` for queries, `MAX_SEMANTIC_CHUNK_BYTES` for
  passages); token cap 512 including `<s>`/`</s>` (rejected, not silently
  truncated); artifact size cap; full-file SHA-256 check against the pinned
  digest; structural validation of every section; optional compute deadline
  checked between encoder layers; no panics on malformed artifacts.

## 3. Proposed minimal diffs to shared files (NOT applied here)

### 3.1 `third_party/models.lock` (append)

```toml
[models.multilingual_e5_small]
component = "multilingual-e5-small-embedding"
model_id = "intfloat.multilingual-e5-small"
repository = "https://huggingface.co/intfloat/multilingual-e5-small"
revision = "614241f622f53c4eeff9890bdc4f31cfecc418b3"
file_name = "model.safetensors"
format = "safetensors"
size_bytes = 470641600
sha256 = "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477"
tokenizer_file_name = "tokenizer.json"
tokenizer_size_bytes = 17082730
tokenizer_sha256 = "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39"
converted_file_name = "multilingual-e5-small.nemb"
converted_format = "nagi-nemb-v1"
converted_sha256 = "<see tools/embedding/manifest.toml converted.sha256>"
conversion = "tools/embedding/convert_e5.py"
license = "MIT"
license_reference = "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/README.md"
notice_id = "mit"
acknowledgement_required = true
artifact_id = "intfloat.multilingual-e5-small"
storage = "model_store"
```

### 3.2 `third_party/sources.lock`

No change: the runtime is Nagi-owned Rust; no external source tree is vendored.
Rust crate dependencies are pinned by `crates/nagi-embedding-provider/Cargo.lock`.

### 3.3 `THIRD_PARTY_NOTICES.md` (append one row to the model table and one paragraph)

```
| multilingual-e5-small (intfloat) | model weights + tokenizer | HF revision `614241f622f53c4eeff9890bdc4f31cfecc418b3` | MIT | converted by `tools/embedding/convert_e5.py` | Derived from microsoft/Multilingual-MiniLM-L12-H384 (MIT) and the XLM-RoBERTa SentencePiece vocabulary (MIT); preserve the MIT notice with any redistribution of the converted `.nemb` |
| Rust crates linked into the provider (`crates/nagi-embedding-provider/Cargo.lock`) | B | libm 0.2.16 (MIT); sha2 0.10.9, digest 0.10.7, block-buffer 0.10.4, crypto-common 0.1.7, cpufeatures 0.2.17, cfg-if 1.0.5, typenum 1.20.1, unicode-segmentation 1.13.3 (MIT OR Apache-2.0); generic-array 0.14.7 (MIT) | as listed | none | statically linked; host-only: libc 0.2.190 via cpufeatures on aarch64 Linux, build-only version_check 0.9.5 |
```

### 3.4 `docs/implementation_status.md` (replace the M24 row text tail)

Append to the M24 row (status stays `PARTIAL`):

> Host real inference: `crates/nagi-embedding-provider` runs multilingual-e5-small
> (HF `614241f6…`, MIT) through a Nagi-owned no_std Rust encoder; parity vs the
> upstream ONNX reference and Japanese/English neighbor tests pass on host
> (see `tests/m24-embedding/`). The crate type-checks for
> `x86_64-unknown-nagi-user`. Guest inference, ModelStore placement, content
> producer sync, hybrid ranking/explanations remain.

### 3.5 Workspace and CI registration

- Root `Cargo.toml`: **no change** (standalone crate with own workspace).
- `.dev/workstreams.json`: row in `registration-proposal.md`.
- `.github/workflows/ci.yml` (ubuntu-host job, after the SEARCH-CORE-01 step):

```yaml
      - name: Standalone embedding provider (M24, hark-m24-embedding)
        run: |
          cargo fmt --manifest-path crates/nagi-embedding-provider/Cargo.toml -- --check
          cargo clippy --manifest-path crates/nagi-embedding-provider/Cargo.toml --all-targets --locked -- -D warnings
          cargo test --manifest-path crates/nagi-embedding-provider/Cargo.toml --locked
```

  This runs all non-model tests (tokenizer/normalizer units, container
  validation, missing/corrupt artifact, empty/over-limit input, space
  mismatch). Real-inference tests are `#[ignore]` by default and **fail, not
  skip,** when run with `--ignored` and no model is present. A separate,
  manually dispatched or cached job may run
  `tests/m24-embedding/run.sh` (≈470 MB download, cached by SHA-256).

## 4. Verification commands

```sh
source $HOME/.cargo/env
M=crates/nagi-embedding-provider/Cargo.toml
cargo fmt --manifest-path $M -- --check
cargo clippy --manifest-path $M --all-targets --locked -- -D warnings
cargo test --manifest-path $M --locked
# real inference (host): downloads pinned files, verifies SHA-256, converts, runs
tests/m24-embedding/run.sh
# Nagi target type-check (no_std user target)
(cd crates/nagi-embedding-provider && cargo -Z build-std=core,alloc check --locked \
   --no-default-features --target ../../targets/x86_64-unknown-nagi-user.json)
```

## 5. Evidence

Measured host results are in `tests/m24-embedding/EVIDENCE.md` (token parity
788/788, vector parity worst cosine 0.999999949 vs the upstream ONNX export,
16/16 JA/EN neighbor requirements, latency/RSS, Nagi user-target compile).
Run logs are written outside the repository by `tests/m24-embedding/run.sh`.

## 6. Open gates

1. Registry row + CI step (integration owner).
2. models.lock / THIRD_PARTY_NOTICES / implementation_status edits (owners).
3. Guest inference: wiring the provider into nagi-init / M19 runtime / model
   service and placing the `.nemb` in the ModelStore are Codex-owned; until a
   guest QEMU run executes the real provider, M24 stays PARTIAL.
4. Hybrid ranking, match explanations, content-producer sync, stale-index
   invalidation and the spec acceptance query remain outside this slice.
5. Reference-scale latency/memory on the official x86-64 QEMU target not measured
   (host here is aarch64 Linux).
