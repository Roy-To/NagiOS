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
  digest; structural validation of every section; no panics on malformed
  artifacts.
- Deadline and cancellation (cooperative, **not** strict preemption): the
  budget starts at call entry, before prefixing and tokenization, and is
  polled together with an optional caller-owned `CancelSignal` at every
  checkpoint: `Start` (before tokenization), `Tokenizing` (before each
  pre-tokenized word), `Tokenized`, `LayerStart(i)` / `LayerMid(i)` for all 12
  layers, `Encoded` (after the last layer, before pooling) and `Pooled`
  (after pooling, before normalization). `now >= start + budget` is expired;
  cancellation wins over expiry at the same checkpoint; partial work is
  dropped. A successful result implies `Pooled` was reached in time; only
  O(384) normalization and space tagging run after it. Overrun after expiry
  is bounded by the longest uninterrupted unit (host: worst checkpoint gap
  136–143 ms at 501 tokens, cancel-to-return 51–65 ms; guest not
  measured). Loading
  is bounded by the artifact size cap, not by the deadline. The default
  config enables a 30 s budget (`DEFAULT_INFERENCE_BUDGET_NANOS`) with the
  host monotonic clock under `std`; `no_std` (guest) builds must pass a Nagi
  `Clock` or explicitly opt out with `max_inference_nanos: None`, otherwise
  loading fails with `InvalidConfig("clock")`. The `EmbeddingProvider::embed`
  trait method has no cancel parameter, so through the trait a call is
  deadline-bounded only; callers that need cancellation use
  `E5Provider::try_embed_cancellable`. Full statement: crate docs in
  `crates/nagi-embedding-provider/src/lib.rs`.

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
config_file_name = "config.json"
config_sha256 = "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959"
converted_file_name = "multilingual-e5-small.nemb"
converted_format = "nagi-nemb-v1"
converted_size_bytes = 474604256
converted_sha256 = "7fb0a34528feecae52e13a3cb0ef6a0edcbab981ae8926c585373b1a4d71a287"
conversion = "tools/embedding/convert_e5.py"
license = "MIT"
license_reference = "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/README.md"
notice_id = "mit"
notice_reference = "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/README.md"
acknowledgement_required = true
artifact_id = "intfloat.multilingual-e5-small"
storage = "model_store"
```

Provenance chain, checked end to end by `tools/embedding/verify_pins.py`
(run log: `tests/m24-embedding/logs/verify-pins-*.txt`):

| Link | Pin | How it is bound |
| --- | --- | --- |
| upstream revision | `614241f622f53c4eeff9890bdc4f31cfecc418b3` | immutable HF commit; `fetch.sh` refuses non-40-hex revisions and downloads `resolve/<revision>/<file>` only |
| original weights | `model.safetensors` 470,641,600 B, `1a55775f…c81c98477` | `fetch.sh` and `convert_e5.py` reject any other size/SHA-256 (= HF LFS oid) |
| tokenizer | `tokenizer.json` 17,082,730 B, `0b44a9d7…6ebdf4c39` | same; vocabulary, scores and the precompiled normalizer are copied into the `.nemb` |
| architecture | `config.json` 655 B, `69137736…f581abf959` | converter checks the pin and the expected BERT/GELU/absolute-position values, then writes hidden/layers/heads/FFN/positions/eps into the `.nemb` header (the config digest itself is not embedded) |
| converted artifact | `multilingual-e5-small.nemb` 474,604,256 B, `7fb0a345…1a4d71a287` | deterministic stdlib conversion (two independent runs byte-identical); its header embeds the weights SHA-256, the tokenizer SHA-256 and the revision |
| runtime pin | `E5_SMALL_NEMB_SHA256` / `E5_SMALL_NEMB_BYTES` / `E5_SMALL_REVISION` in `lib.rs` | `ProviderConfig::default()` rejects any artifact whose full-file SHA-256 differs (`ChecksumMismatch`) |
| embedding space | `space_id_for(.nemb SHA-256, 384)` | a change to weights, tokenizer, config, converter or prefix scheme changes the `.nemb` digest and therefore the `EmbeddingSpaceId`; stale vectors fail with `EmbeddingSpaceMismatch` |
| reference only | `onnx/model.onnx` `ca456c06…f1cba6bc8665` | used solely by `reference_e5.py` for parity; never converted or shipped |

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
- **Shared CI is not provider acceptance evidence.** The existing shared
  workflow (`ubuntu-host`, `windows-launcher`, target jobs) builds and tests the
  root workspace. This crate has its own `[workspace]` and is not a root
  member, so the shared jobs neither compile it nor run its tests. A green
  shared run on PR #38 says nothing about this provider and is not cited as
  evidence for it.
- Proposed dedicated CI registration, for the shared CI owner to add to
  `.github/workflows/ci.yml` (nothing applied here). Both jobs use the
  repository toolchain `nightly-2025-08-01`:

```yaml
  m24-embedding-provider:
    name: M24 embedding provider (hark-m24-embedding)
    runs-on: ubuntu-latest
    env:
      M: crates/nagi-embedding-provider/Cargo.toml
    steps:
      - uses: actions/checkout@v4
      - name: Toolchain
        run: rustup toolchain install nightly-2025-08-01 --profile minimal --component rustfmt --component clippy --component rust-src
      - name: fmt
        run: cargo +nightly-2025-08-01 fmt --manifest-path $M -- --check
      - name: clippy (std, all targets)
        run: cargo +nightly-2025-08-01 clippy --manifest-path $M --all-targets --locked -- -D warnings
      - name: clippy (no_std library)
        run: cargo +nightly-2025-08-01 clippy --manifest-path $M --lib --no-default-features --locked -- -D warnings
      - name: model-free tests (tokenizer units + contract suite)
        run: cargo +nightly-2025-08-01 test --manifest-path $M --locked
      - name: Nagi user-target build (compile only, not executed)
        working-directory: crates/nagi-embedding-provider
        run: cargo +nightly-2025-08-01 -Z build-std=core,alloc build --release --locked --no-default-features --target ../../targets/x86_64-unknown-nagi-user.json

  m24-embedding-real-inference:
    name: M24 embedding real inference (host)
    needs: m24-embedding-provider
    runs-on: ubuntu-latest
    env:
      NAGI_EMBEDDING_CACHE: ${{ github.workspace }}/.cache/nagi-embedding
    steps:
      - uses: actions/checkout@v4
      - uses: actions/cache@v4
        with:
          path: .cache/nagi-embedding
          key: m24-e5-small-614241f622f53c4eeff9890bdc4f31cfecc418b3-${{ hashFiles('tools/embedding/manifest.toml') }}
      - name: Toolchain
        run: rustup toolchain install nightly-2025-08-01 --profile minimal --component rustfmt --component clippy --component rust-src
      - name: Fetch (pinned, SHA-256 verified), convert, verify pins, real inference
        run: tests/m24-embedding/run.sh
```

  The real-inference job downloads ~470 MB once (cache keyed by the immutable
  revision and the manifest hash), fails rather than skips when the model is
  absent, and is host evidence only. The registry row in
  `registration-proposal.md` is a precondition for the shared
  `dev_status_resume_and_verify_read_the_registered_workstream` check.

## 4. Verification commands

```sh
source $HOME/.cargo/env
M=crates/nagi-embedding-provider/Cargo.toml
cargo fmt --manifest-path $M -- --check
cargo clippy --manifest-path $M --all-targets --locked -- -D warnings
cargo test --manifest-path $M --locked
# real inference (host): downloads pinned files, verifies SHA-256, converts, runs
tests/m24-embedding/run.sh
# every pin agrees (manifest, .nemb header, Rust constants, models.lock proposal)
python3 tools/embedding/verify_pins.py --artifact <cache>/multilingual-e5-small.nemb \
   --input <cache>/614241f622f53c4eeff9890bdc4f31cfecc418b3 \
   --models-lock-proposal .dev/workstreams/hark-m24-embedding/integration-proposal.md
# reference data regeneration in a fresh hash-pinned venv (byte-identical check)
tools/embedding/regenerate_reference.sh
# Nagi target type-check (no_std user target)
(cd crates/nagi-embedding-provider && cargo -Z build-std=core,alloc check --locked \
   --no-default-features --target ../../targets/x86_64-unknown-nagi-user.json)
```

## 5. Evidence

Measured host results are in `tests/m24-embedding/EVIDENCE.md` (token parity
788/788, vector parity worst cosine 0.999999949 vs the upstream ONNX export,
16/16 JA/EN neighbor requirements, latency/RSS, Nagi user-target compile).
`tests/m24-embedding/run.sh` writes full run logs outside the repository; the
reference-regeneration and pin-verification logs are committed under
`tests/m24-embedding/logs/`.

## 6. Open gates

1. Registry row + the two dedicated CI jobs in §3.5 (integration / shared CI owner).
2. models.lock / THIRD_PARTY_NOTICES / implementation_status edits (owners).
3. Guest inference: wiring the provider into nagi-init / M19 runtime / model
   service and placing the `.nemb` in the ModelStore are Codex-owned; until a
   guest QEMU run executes the real provider, M24 stays PARTIAL.
4. Hybrid ranking, match explanations, content-producer sync, stale-index
   invalidation and the spec acceptance query remain outside this slice.
5. Reference-scale latency/memory on the official x86-64 QEMU target not measured
   (host here is aarch64 Linux).
