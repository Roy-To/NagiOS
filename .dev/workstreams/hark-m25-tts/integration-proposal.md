# Integration proposal — hark-m25-tts

Base: `edad2e7`. Branch: `hark/m25-local-tts`. Owned paths:
`crates/nagi-tts-provider/**`, `tools/tts/**`, `tests/m25-tts/**`,
`.dev/workstreams/hark-m25-tts/**`. Everything in section 2 is a proposal for
the shared-file owner; none of it is applied by this stream.

## 1. What this stream delivers (owned paths)

- `crates/nagi-tts-provider`: standalone crate (own `[workspace]` and
  `Cargo.lock`) implementing `nagi_audio::speech::TextToSpeechProvider` with a
  pure-Rust Japanese engine (jpreprocess 0.15.0 + jbonsai 0.4.2) and the
  `tohoku-f01-neutral` HTS voice. Depends read-only on `user/nagi-audio` by
  path. The provider core (`LocalTtsProvider`, feature `engine-jbonsai` off)
  is `no_std` + `alloc`.
- `tools/tts/fetch.sh` + `tools/tts/tts-artifacts.lock`: pinned, checksummed
  artifact fetch into ignored `out/cache/tts/`.
- `tools/tts/notices/` + `tools/tts/NOTICES.sha256`: byte-exact copies of the
  upstream license/notice files with their sources and SHA-256 digests.
- `tools/tts/ENGINE_SELECTION.md`: engine/voice comparison with license
  evidence.
- `tests/m25-tts/`: `acceptance.sh` (host acceptance runner),
  `measure.sh` (host measurements), `MEASUREMENTS.md` (results, streaming
  behaviour, std/Nagi write-up), `STD_INVENTORY.txt` + `std_inventory.py`,
  `CRATE_LICENSES.md`, and `evidence/` (raw logs).

### Loading API (what a guest calls)

The guest path involves no filesystem path:

```rust
use nagi_tts_provider::jbonsai_backend::{DictionaryBytes, JbonsaiBackend};
use nagi_tts_provider::LocalTtsProvider;

// voice: &[u8] and the eight dictionary components, all obtained as bytes
// through the read-only Model Store capability.
let dictionary = DictionaryBytes {
    metadata_json, char_def_bin, matrix_mtx, dict_da,
    dict_vals, dict_wordsidx, dict_words, unk_bin, // each a Vec<u8>
};
let backend = JbonsaiBackend::from_bytes(voice, dictionary)?;
let provider = LocalTtsProvider::new(backend)?;
```

`JbonsaiBackend::load(voice_path, dictionary_dir)` and `load_provider(..)`
are host conveniences that read the same files into memory and then call
`from_bytes`; the test `bytes_loader_matches_path_loader` checks both
produce identical PCM. Limits: voice ≤ 16 MiB, each dictionary component
≤ 64 MiB.

### Over-length behaviour (current, measured)

- Text > 1 KiB (`MAX_SPEECH_SYNTHESIS_TEXT_BYTES`): rejected by the provider
  at `begin`; through `SpeechSynthesisService::speak` it is `TextTooLong`
  before the provider is called.
- Text ≤ 1 KiB whose predicted PCM exceeds 1 MiB
  (`MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES`): the provider runs the front end
  and duration model at `begin`, predicts the exact emitted length, and
  rejects it **at `begin`, before any PCM chunk is produced**. The shared
  contract has no provider-level "too long" error, so the provider returns
  `SpeechProviderError::Failed`, which `SpeechSynthesisService::speak`
  reports as **`SpeechSynthesisError::ProviderFailed`, not
  `UtteranceTooLong`**. The playback sink receives no audio
  (`playback_sink_sees_no_audio_for_over_budget_text`).
- If emitted bytes ever exceeded the cap mid-stream (prediction mismatch),
  the provider fails closed with `Failed` and never reports truncated audio
  as success.

Distinguishing over-length from other provider failures needs a contract
change in `user/nagi-audio` (see gate G-CAP).

## 2. Minimal diffs proposed for shared files

### 2a. `third_party/models.lock` (append)

```toml
[models.tohoku_f01_neutral]
component = "htsvoice-tohoku-f01"
model_id = "icn-lab.tohoku-f01-neutral"
repository = "https://github.com/icn-lab/htsvoice-tohoku-f01"
revision = "8e3306021db135c265f5eda5f062dc489707ddf8"
file_name = "tohoku-f01-neutral.htsvoice"
format = "htsvoice"
size_bytes = 2154716
sha256 = "ded6acb4243b93cc50896199d67b00a7c487662bfcdcd3f9f30a4fb0444b0b46"
license = "CC-BY-4.0"
license_reference = "https://github.com/icn-lab/htsvoice-tohoku-f01/blob/8e3306021db135c265f5eda5f062dc489707ddf8/COPYRIGHT.txt"
notice_id = "cc-by-4.0"
notice_reference = "https://creativecommons.org/licenses/by/4.0/"
acknowledgement_required = true
artifact_id = "icn-lab.tohoku-f01-neutral"
storage = "model_store"

[models.naist_jdic_jpreprocess]
component = "naist-jdic-jpreprocess"
model_id = "jpreprocess.naist-jdic"
repository = "https://github.com/jpreprocess/jpreprocess"
revision = "07a78220c1e850a001998451bd1e7c88709ee379"
file_name = "naist-jdic-jpreprocess.tar.gz"
format = "lindera-dictionary-tar-gz"
size_bytes = 28668638
sha256 = "8a930bbc57bf4adcf521d53544c7dc9ab8ab3aa997a591b1b1608dc5539017b8"
license = "BSD-3-Clause"
license_reference = "https://github.com/jpreprocess/naist-jdic/blob/394af50de2da4a9c73b1bd04fab43e85bb6f03ff/COPYING"
acknowledgement_required = true
artifact_id = "jpreprocess.naist-jdic"
storage = "model_store"
```

The unpacked dictionary component digests are in
`tools/tts/tts-artifacts.lock` (`[artifacts.naist_jdic_jpreprocess.files]`);
the Model Store owner decides whether to store the archive or the eight
components (the guest loader takes the components as bytes).

### 2b. `THIRD_PARTY_NOTICES.md` (append this section verbatim)

The three copyright groups below are recorded separately on purpose: the
JPreprocess software, the dictionary data, and the voice have different
holders and licenses. Full texts are the files listed in
`tools/tts/NOTICES.sha256` (byte-exact upstream copies).

```markdown
## Local Japanese TTS (M25, proposed by hark-m25-tts)

These components are fetched or built only for the opt-in M25 local TTS
provider (`crates/nagi-tts-provider`). Rust crates are pinned by
`crates/nagi-tts-provider/Cargo.lock`; data artifacts by
`third_party/models.lock`. Byte-exact copies of every upstream license and
notice file, with SHA-256 digests, are in `tools/tts/notices/` and
`tools/tts/NOTICES.sha256`. Classification: B (reproducibly fetched).

### JPreprocess software (text front end)

- jpreprocess 0.15.0 (and its jpreprocess-* subcrates), tag v0.15.0 =
  `07a78220c1e850a001998451bd1e7c88709ee379`. BSD-3-Clause.
  Copyright (c) 2022 by JPreprocess Team.
  Includes source derived from Open JTalk: Copyright (c) 2008-2016 Nagoya
  Institute of Technology, Department of Computer Science (BSD-3-Clause,
  HTS Working Group terms; see `tools/tts/notices/jpreprocess-NOTICE.txt`).
- jbonsai 0.4.2 (acoustic engine), tag v0.4.2 =
  `4aa02e4dbe5ea4e3e8058477d612f116d00ed1b6`. BSD-3-Clause.
  Copyright (c) 2023 by JPreprocess Team.
  Includes source derived from hts_engine API: Copyright (c) 2001-2014
  Nagoya Institute of Technology, Department of Computer Science; 2001-2008
  Tokyo Institute of Technology, Interdisciplinary Graduate School of
  Science and Engineering (BSD-3-Clause; see
  `tools/tts/notices/jbonsai-NOTICE.txt`).
- lindera 3.0.7 / lindera-dictionary 3.0.7 (tokenizer), MIT.
  Copyright (c) 2019 by the project authors, as listed in the AUTHORS file.
- Remaining transitive crates: see `tests/m25-tts/CRATE_LICENSES.md`
  (declared licenses; to be verified against packaged license files by the
  SBOM owner before a binary release).

### Dictionary data (naist-jdic, jpreprocess binary form)

- naist-jdic as built by jpreprocess v0.15.0
  (`naist-jdic-jpreprocess.tar.gz`, SHA-256
  `8a930bbc57bf4adcf521d53544c7dc9ab8ab3aa997a591b1b1608dc5539017b8`),
  from jpreprocess/naist-jdic at
  `394af50de2da4a9c73b1bd04fab43e85bb6f03ff`. BSD-3-Clause. Its COPYING
  (`tools/tts/notices/naist-jdic-COPYING.txt`) carries four notices, each of
  which must be reproduced:
  - Copyright (c) 2009, Nara Institute of Science and Technology, Japan.
  - Copyright (c) 2011-2017, The UniDic Consortium.
  - Copyright (c) 2008-2016 Nagoya Institute of Technology, Department of
    Computer Science (Open JTalk).
  - Copyright (c) 2023, JPreprocess Team.
  The jpreprocess dictionary NOTICE
  (`tools/tts/notices/jpreprocess-naist-jdic-NOTICE.txt`) additionally
  records data from open_jtalk-1.11/mecab-naist-jdic and
  lindera-ipadic-neologd v0.25.0 (Lindera, MIT).

### Voice (HTS voice tohoku-f01, neutral)

- "HTS voice tohoku-f01-neutral" by Intelligent Communication Network
  (Ito-Nose) Laboratory, Tohoku University. Copyright (c) 2015 Intelligent
  Communication Network (Ito-Nose) Laboratory, Tohoku University.
  Licensed under the Creative Commons Attribution 4.0 International License
  (CC BY 4.0), https://creativecommons.org/licenses/by/4.0/ .
  Source: https://github.com/icn-lab/htsvoice-tohoku-f01 at
  `8e3306021db135c265f5eda5f062dc489707ddf8`
  (`tohoku-f01-neutral.htsvoice`, SHA-256
  `ded6acb4243b93cc50896199d67b00a7c487662bfcdcd3f9f30a4fb0444b0b46`).
  No changes were made to the voice. Speech produced with it is synthesized
  by Nagi OS; this attribution must accompany any distribution of the voice
  file (for example in a release image or model bundle).
```

Where the attribution is shown to end users (About screen, model bundle
README) is a release decision for the shared owner (gate G-LIC).

### 2c. `third_party/sources.lock` — no entry needed

Rust crates are pinned by `crates/nagi-tts-provider/Cargo.lock` (crates.io
checksums). If the owner prefers git pins: jbonsai tag `v0.4.2` =
`4aa02e4dbe5ea4e3e8058477d612f116d00ed1b6`; jpreprocess tag `v0.15.0` =
`07a78220c1e850a001998451bd1e7c88709ee379`.

### 2d. `.github/workflows/ci.yml` (ubuntu-host job, new step)

```yaml
      - name: Standalone TTS provider (M25 hark-m25-tts)
        run: |
          cargo fmt --manifest-path crates/nagi-tts-provider/Cargo.toml -- --check
          cargo clippy --manifest-path crates/nagi-tts-provider/Cargo.toml --all-targets --locked -- -D warnings
          cargo clippy --manifest-path crates/nagi-tts-provider/Cargo.toml --no-default-features --all-targets --locked -- -D warnings
          cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml --locked
```

These run without the ~30 MB artifacts: the real-engine tests are
`#[ignore]` and are reported as ignored (never passed) by `cargo test`. An
optional slow job runs `tests/m25-tts/acceptance.sh`, which fetches the
pinned artifacts, runs the real-engine suite with `--ignored`, and fails if
the artifacts are missing (`FAIL: NAGI_TTS_VOICE is not set`). The engine
build emits 4 `libc` warnings from the patched `third_party/libc`
(unexpected `cfg` value `nagi` ×2, `setgroups` unused / redeclared); they are
in the dependency, not this crate, and do not fail `-D warnings`.

**Dedicated workstream CI (already on this branch, not owner registration).**
`.github/workflows/hark-m25-local-tts.yml` is the one workflow file this
stream owns. It triggers on pull_request/push touching the TTS paths (and
`workflow_dispatch`), uses `permissions: contents: read`, SHA-pinned actions,
`ubuntu-24.04` runners, no secrets, and only the public pinned downloads in
`tools/tts/tts-artifacts.lock`. Jobs: (A) fmt, clippy engine and no_std core,
model-free tests with the 12 real-engine tests asserted as ignored; (B)
`x86_64-unknown-nagi-user` no_std core check, engine-enabled check and
release rlib (build-std with the patched rust-src + crate-level libc), as
separate steps, rlib only; (C) `fetch.sh` with a lock/fetch pin cross-check,
notices manifest, the 12-test real-engine suite, fail-closed negatives
(unset artifacts and a nonexistent voice path must exit non-zero with
0 passed), `measure.sh` (no Whisper round trip) and `acceptance.sh`. It does
not change `ci.yml`, does not make the provider PASS, and guest playback
remains NOT_RUN. Whether 2d is still wanted in `ci.yml`, or the dedicated
workflow is adopted/registered instead, is the CI owner's decision.

The crate is not proposed as a root workspace member (root `Cargo.toml`
untouched); it builds with its own lock file like `nagi-clipboard-core`.

### 2e. `docs/implementation_status.md` (M25 sweep, proposed text)

```markdown
- M25 local TTS (hark-m25-tts, branch hark/m25-local-tts): PARTIAL.
  Engine: jpreprocess 0.15.0 + jbonsai 0.4.2 (BSD-3-Clause), naist-jdic
  (BSD-3-Clause), HTS voice tohoku-f01 neutral (CC BY 4.0). Host evidence
  only: model-free unit tests, real-engine tests and host measurements pass
  on Linux aarch64 (tests/m25-tts/MEASUREMENTS.md). Nagi target:
  `x86_64-unknown-nagi-user` check of the no_std core and check + release
  rlib build of the engine via `-Zbuild-std=std,panic_abort`; nothing is
  linked into nagi-init and nothing has run in the guest. Guest playback
  through AudioService: NOT RUN.
```

### 2f. Guest wiring (Codex-owned `user/nagi-init`, `tools/nagi-cli`)

Not attempted here. Proposed shape for the owner, following the existing
`m13-std` pattern (`m13-std = []` feature, built with
`-Zbuild-std=std,panic_abort`, `--target targets/x86_64-unknown-nagi-user.json`
and `__CARGO_TESTS_ONLY_SRC_ROOT` pointing at the patched rust-src prepared by
`prepare_nagi_rust_std_source`):

1. `user/nagi-init/Cargo.toml`:
   ```toml
   [features]
   m25-tts-std = ["m13-std", "dep:nagi-tts-provider"]

   [dependencies]
   nagi-tts-provider = { path = "../../crates/nagi-tts-provider", optional = true }
   ```
   The crate's own `[patch.crates-io] libc = { path = "../../third_party/libc" }`
   must be mirrored in the root manifest (patches only apply from the root
   workspace) — a root `Cargo.toml` / `Cargo.lock` change for the owner.
2. Read the voice and the eight dictionary components through the read-only
   Model Store capability into memory, build `DictionaryBytes`, and call
   `JbonsaiBackend::from_bytes(voice, dictionary)` →
   `LocalTtsProvider::new(backend)`. No guest filesystem path is needed.
3. Drive `SpeechSynthesisService::speak` into `AudioServicePlaybackSink`
   (stereo 48 kHz S16LE) and emit `Nagi M25 TTS synthesis PASS` only if real
   PCM was accepted by the sink. A `./nagi m25` marker / acceptance-registry
   entry is the CI owner's change.
4. Budget on the guest: ~86 MB resident after load and ~118 MiB peak during
   load on the host (see MEASUREMENTS.md); the guest heap must allow this.

## 3. Verification commands (owned crate)

```sh
source "$HOME/.cargo/env"   # repository-pinned nightly-2025-08-01
tests/m25-tts/acceptance.sh out/tts-acceptance/run   # fetch+fmt+clippy×2+unit+real+missing-assets
tests/m25-tts/measure.sh out/tts-eval                # host measurements
(cd tools/tts/notices && sha256sum -c ../NOTICES.sha256)
```

Nagi target commands are listed in `tests/m25-tts/MEASUREMENTS.md` §5.

## 4. Evidence

`tests/m25-tts/MEASUREMENTS.md`, `tests/m25-tts/evidence/`, and
`.dev/workstreams/hark-m25-tts/state.json`.

## 5. Open gates

- **G-LIC** License review: placement of the CC BY 4.0 attribution and the
  four dictionary notices in release artifacts (text in 2b). No license is
  accepted by this stream; VOICEVOX remains held (its voice terms require
  acceptance).
- **G-GUEST** Guest playback through `AudioServicePlaybackSink` on
  Nagi/QEMU: NOT RUN. Host synthesis is not guest evidence. Needs 2f and the
  Model Store registration (2a).
- **G-CAP** Output cap: 1 MiB = 5.46 s of stereo 48 kHz (32.77 s of mono
  16 kHz). Longer utterances are rejected at `begin` with `ProviderFailed`
  and no audio. A distinct `UtteranceTooLong` from the provider, a larger
  cap, or sentence-level splitting by the caller is a `nagi-audio` contract
  decision.
- **G-MMAP** `memmap2` is compiled in because jpreprocess,
  jpreprocess-core, and jpreprocess-dictionary depend on `lindera` with
  default features (`mmap`); the bytes loader never maps a file. Proposal:
  upstream `default-features = false` for `lindera` in jpreprocess (or a
  pinned `[patch]`), which removes `memmap2` and its libc calls from the
  guest build. Not changeable from this crate's manifest.
- **G-STD** Guest stack usage is unmeasured; the allocator is whatever
  `nagi-init`'s std provides. See MEASUREMENTS.md §5.
- **G-OWNER** Registry entry (`registration-proposal.md`), CI step (2d),
  model storage (2a), and guest wiring (2f) are shared-owner decisions.
