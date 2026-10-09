# Integration proposal — hark-m25-tts

Base: `edad2e7`. Branch: `hark/m25-local-tts`. Owned paths:
`crates/nagi-tts-provider/**`, `tools/tts/**`, `tests/m25-tts/**`,
`.dev/workstreams/hark-m25-tts/**`. Everything below is a proposal for the
shared-file owner; none of it is applied by this stream.

## 1. What this stream delivers (owned paths)

- `crates/nagi-tts-provider`: standalone crate (own `[workspace]` and
  `Cargo.lock`) implementing `nagi_audio::speech::TextToSpeechProvider` with a
  pure-Rust Japanese engine (jpreprocess 0.15.0 + jbonsai 0.4.2) and the
  `tohoku-f01-neutral` HTS voice. Depends read-only on `user/nagi-audio` by
  path.
- `tools/tts/fetch.sh` + `tools/tts/tts-artifacts.lock`: pinned, checksummed
  artifact fetch into ignored `out/cache/tts/`.
- `tools/tts/ENGINE_SELECTION.md`: engine/voice comparison with license
  evidence.
- `tests/m25-tts/`: measurement script and measured evidence.

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

### 2b. `THIRD_PARTY_NOTICES.md` (append section)

```markdown
## Local Japanese TTS (M25)

- jbonsai 0.4.2 — BSD-3-Clause. Includes hts_engine API source, Copyright (c)
  2001-2014 Nagoya Institute of Technology, Department of Computer Science;
  2001-2008 Tokyo Institute of Technology, Interdisciplinary Graduate School
  of Science and Engineering.
- jpreprocess 0.15.0 — BSD-3-Clause. Includes Open JTalk source, Copyright (c)
  2008-2016 Nagoya Institute of Technology, Department of Computer Science,
  and Lindera, Copyright (c) 2019 by the project authors (MIT).
- naist-jdic (jpreprocess binary form) — BSD-3-Clause. Copyright (c) 2009
  Nara Institute of Science and Technology; Copyright (c) 2011-2017 The UniDic
  Consortium; Copyright (c) 2008-2016 Nagoya Institute of Technology.
- HTS voice tohoku-f01 (neutral) — Copyright (c) 2015 Intelligent
  Communication Network (Ito-Nose) Laboratory, Tohoku University. Licensed
  under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). Used
  unmodified.
```

The full BSD-3 texts and the transitive crate licenses are listed by
`cargo metadata --manifest-path crates/nagi-tts-provider/Cargo.toml`; an SBOM
owner should add them to the release SBOM. Transitive-crate license summary:
`tests/m25-tts/CRATE_LICENSES.md`.

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
          cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml --locked
```

These run without the 30 MB artifacts: tests that need the real voice and
dictionary skip with an explicit `SKIP (artifacts absent)` line unless
`NAGI_TTS_VOICE`/`NAGI_TTS_DICT` are set. An optional slow job may run
`tools/tts/fetch.sh` first and then the same `cargo test`.

The crate is not proposed as a root workspace member (root `Cargo.toml`
untouched); it builds with its own lock file like `nagi-clipboard-core`.

### 2e. `docs/implementation_status.md` (M25 row / sweep, proposed text)

See section 5 (filled with measured values at the end of the stream).

### 2f. Guest wiring (Codex-owned `user/nagi-init`, `tools/nagi-cli`)

Not attempted here. Proposed shape for the owner:

1. Add an opt-in `m25-tts-acceptance` feature to `nagi-init` that depends on
   `nagi-tts-provider` (target `std` build via the existing
   `third_party/rust-std` patch and `-Zbuild-std=std,panic_abort`).
2. Load the voice through the read-only Model Store capability into memory
   and call `JbonsaiProvider::from_voice_bytes` (bytes-based load; no host
   path). The dictionary is loaded the same way once a bytes loader is
   available for the dictionary (see open gate G3).
3. Drive `SpeechSynthesisService::speak` into `AudioServicePlaybackSink` and
   emit `Nagi M25 TTS synthesis PASS` only if real PCM was accepted.

## 3. Verification commands (owned crate)

```sh
source "$HOME/.cargo/env"   # repository-pinned nightly-2025-08-01
tools/tts/fetch.sh
export NAGI_TTS_VOICE=out/cache/tts/tohoku-f01-neutral.htsvoice
export NAGI_TTS_DICT=out/cache/tts/naist-jdic
cargo fmt   --manifest-path crates/nagi-tts-provider/Cargo.toml -- --check
cargo clippy --manifest-path crates/nagi-tts-provider/Cargo.toml --all-targets --locked -- -D warnings
cargo test  --manifest-path crates/nagi-tts-provider/Cargo.toml --locked
tests/m25-tts/measure.sh
```

## 4. Evidence

Filled in `tests/m25-tts/MEASUREMENTS.md` and `.dev/workstreams/hark-m25-tts/state.json`.

## 5. Open gates

- G1 License review of CC BY 4.0 attribution placement (notice text above).
- G2 Guest playback: not run by this stream. Host synthesis is not guest
  evidence.
- G3 Dictionary loading on Nagi: jpreprocess loads the dictionary from a
  directory path; Model Store delivers bytes. Needs either a guest VFS path
  for the read-only dictionary or a bytes-based loader.
- G4 1 MiB output cap = 5.46 s of stereo 48 kHz audio; longer sentences fail
  closed with `UtteranceTooLong`. Raising the cap or emitting mono needs a
  contract decision by the nagi-audio owner.
