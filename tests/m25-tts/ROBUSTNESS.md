# M25 local TTS: malformed-input and lifecycle robustness (host)

Scope: `crates/nagi-tts-provider` bytes loader (`JbonsaiBackend::from_bytes`,
the path a Nagi guest uses with Model Store bytes) and provider lifecycle after
errors. Host evidence only; nothing here runs on Nagi/QEMU, and guest
AudioService playback remains NOT_RUN. Output format (S16LE stereo 48 kHz /
mono 16 kHz), the 1 KiB text cap, the 1 MiB utterance cap, the pinned voice,
dictionary and licenses are unchanged.

## Findings

Upstream parsers trust their input. On Nagi the engine is built with
`panic = abort`, so each of these would end the provider process instead of
returning an error.

| Input | Upstream behaviour before this change | Now |
|---|---|---|
| Voice tree node naming an undeclared question | jbonsai 0.4.2 `convert_tree`: `unwrap()` on `None` while loading | `VoiceInvalid` |
| Voice tree node referencing a missing node id | same `unwrap()` panic while loading | `VoiceInvalid` |
| Leaf PDF index 0 | `pdf_index - 1` underflow during synthesis | `VoiceInvalid` |
| Leaf PDF index above the tree's PDF count | index out of bounds during synthesis | `VoiceInvalid` |
| No tree for a state synthesis looks up (duration: 2; streams: 2..=NUM_STATES+1) | `todo!("index not found!")` during synthesis | `VoiceInvalid` |
| Single node whose branches are the same node | `todo!()` while loading | `VoiceInvalid` |
| Tree whose node links form a cycle | `Tree::search_node` loops forever (code reading) | `VoiceInvalid` |
| `VECTOR_LENGTH * NUM_WINDOWS * 2` overflow | multiply overflow panic while loading (debug) / wrap (release) | `VoiceInvalid` (bounded fields) |
| Header number with more than 18 digits | jbonsai header deserializer multiplies unchecked | `VoiceInvalid` |
| Fewer than three streams | vocoder indexes streams 0, 1, 2 unconditionally (code reading) | `VoiceInvalid` |
| Duration PDF mean `f32::MAX` or `+inf` (review finding 1) | jbonsai `DurationEstimator` casts `mean.round()` to `usize::MAX`; the provider's planner summed state durations unchecked: reproduced `attempt to add with overflow` in `JbonsaiBackend::plan` (debug, pinned voice); release would wrap and pass the budget check. jbonsai's own generator then panics with `capacity overflow` (synthetic voice) | `VoiceInvalid`; planning is also checked per state (`TooLong`) |
| Duration PDF mean NaN / negative / `-inf`, variance NaN / `+inf` / 0 / negative, or mean above 2,000 frames | no panic reproduced: NaN and negatives clamp to 1 frame; a 1e9-frame mean ran into the budget (`Failed`) | `VoiceInvalid` (means must be in [0, 2000] frames, variances in (0, 1e6]) |
| Stream / GV PDF value non-finite, negative variance, MSD weight outside [0, 1] | no panic reproduced; NaN/inf spectrum or LPF values produced all non-finite samples (already rejected at output) | `VoiceInvalid` (zero variances stay accepted: the pinned LPF stream has them) |
| Log F0 stream (index 1) `VECTOR_LENGTH` other than 1 (review finding 2a) | `SpeechGenerator::new` panic "The size of lf0 static vector must be 1." (synthetic voice, reproduced) | `VoiceInvalid` |
| LPF stream (index 2) even `VECTOR_LENGTH` (review finding 2a) | `SpeechGenerator::new` panic "The number of low-pass filter coefficient must be odd numbers." (synthetic voice, reproduced) | `VoiceInvalid` |
| More `STREAM_WIN` rows than `NUM_WINDOWS`, e.g. a duplicated range (review finding 2b) | `MlpgAdjust::create` index out of bounds (reproduced on the synthetic voice and on the pinned voice: LPF "len is 31 but the index is 31", MCP 105/105, LF0 3/3) | `VoiceInvalid` |
| Fewer `STREAM_WIN` rows than `NUM_WINDOWS`, even window width, non-finite coefficient | no panic reproduced (dynamic features silently dropped / NaN output) | `VoiceInvalid` |
| `STREAM_WIN` row whose coefficient count disagrees with its header, or without a count | jbonsai parse error, no panic | `VoiceInvalid` earlier |
| `NUM_STREAMS` 0 while `STREAM_TYPE` lists 3 streams (review finding 3) | jbonsai sizes `gv_weight`/`msd_threshold` with `NUM_STREAMS`; synthesis panicked "index out of bounds: the len is 0 but the index is 0" at `engine.rs:334` (synthetic voice, reproduced) | `VoiceInvalid` (`NUM_STREAMS` must be 1..=8 and equal the `STREAM_TYPE` count) |
| `NUM_STREAMS` 2 (review finding 3) | LPF stream skipped; `SpeechGenerator::new` panic "The number of low-pass filter coefficient must be odd numbers." (reproduced) | `VoiceInvalid` |
| `NUM_STREAMS` 999999999999999999 (review finding 3) | `[0.5].repeat(n)` in `Condition::load_model`: "memory allocation of 7999999999999999992 bytes failed", process aborted (SIGABRT) while *loading*, in a child under `ulimit -v` (reproduced) | `VoiceInvalid` before jbonsai sees the bytes |
| `NUM_STREAMS` 4 with 3 streams, or missing | no panic (4: synthesizes; missing: jbonsai parse error) | `VoiceInvalid` |
| Spectrum `OPTION` `GAMMA=999999999999999999` (review finding 4) | MGLSA stage count; `vec![vec![0.0; nmcp]; stage]` at `vocoder/mglsa.rs:11` panicked "capacity overflow" (reproduced) | `VoiceInvalid` |
| `GAMMA=100000000000000` | "memory allocation of 2400000000000000 bytes failed", SIGABRT, in the limited child (reproduced) | `VoiceInvalid` |
| `GAMMA=1` (any non-zero stage) | synthesizes (MGLSA path); outside the pinned voice's MLSA contract | `VoiceInvalid` (only absent or `0` supported) |
| `ALPHA` inf / NaN | synthesizes all non-finite samples (no panic) | `VoiceInvalid` (`ALPHA` must be finite in [0, 1)) |
| `ALPHA` 1.0 / negative, duplicated `ALPHA` | synthesizes (no panic) | `VoiceInvalid` |
| `GAMMA=-1`, `LN_GAIN=2`, non-numeric `ALPHA` | jbonsai option parse error, no panic | `VoiceInvalid` earlier |
| Random 1-4 byte corruption of the pinned voice | 12 of 80 seeded trials panicked (unwrap) | 0 panics |
| `matrix.mtx` truncated / odd length / trailing bytes / negative or oversized shape | lindera-dictionary 3.0.7: length `assert` or overflow panic while loading, or index out of bounds on the first cost lookup | `DictionaryInvalid` |
| `dict.vals` context ids outside the matrix | index out of bounds during tokenization | `DictionaryInvalid` |
| `dict.vals` partial trailing entry | silently ignored | `DictionaryInvalid` |
| `dict.wordsidx` offsets past `dict.words` or out of order | `start..end` slice panic in jpreprocess / lindera | `DictionaryInvalid` |
| `dict.words` preamble not `jpreprocess*` (e.g. first byte corrupted) | tokens routed to lindera's decoder, which slices jpreprocess records unchecked: panic | `DictionaryInvalid` |
| Unknown-word entries / category references out of range (after rkyv load) | unchecked indexing during tokenization | `DictionaryInvalid` |

Already sound before this change (now covered by tests): truncated voice
(`[POSITION]` ranges, fb0a15c), bad section magic, PDF count/size mismatch,
zero-width windows, `metadata.json`, `char_def.bin`, `unk.bin` and `dict.da`
structural corruption (serde/rkyv/daachorse validate), NaN/inf samples
(provider rejects non-finite output), empty components (`DictionaryMissing`).

Not changed (reported only): an `HTS_VOICE_VERSION` other than `1.0` still
loads when the rest of the voice is consistent; nothing breaks, and content integrity remains the
SHA-256 pin. The lindera character-category lookup table is private, so its
internal boundaries cannot be checked beyond rkyv's structural validation.

Lifecycle: no defect found. Failed `begin` (over budget, unsupported
language, unloaded, empty text), mid-stream backend failure (stereo and
mono), non-finite output, undersized destination and cancel all leave the
provider clear; the next utterance is byte-identical to a fresh provider's
(including the 16 kHz decimator state); `next_pcm_chunk` after End, error or
cancel writes nothing to the caller's buffer. A failed bytes load or failed
reload leaves no partial state; unload after an error reports `Unavailable`
until a successful reload. An utterance already in progress keeps its own
generator after `unload()`; cancel it first if its memory must be released
immediately (no hard cancellation of an in-flight engine step is claimed).

## Validation added

`src/validate.rs`, run inside `JbonsaiBackend::from_bytes` before the
upstream parsers: a conservative structural pass over the `.htsvoice`
(sections, bounded header numbers, question/tree cross references, PDF
counts and sizes, required states, acyclic trees) and over the dictionary
components (matrix shape, entry ids, word offsets, jpreprocess preamble,
loaded unknown-word references), plus numeric invariants of every PDF
(finite values; duration means in [0, 2000] frames and variances in
(0, 1e6]; stream/GV variances >= 0; MSD weights in [0, 1]), role checks for
the vocoder streams (log F0 width 1, LPF width odd) and `STREAM_WIN` rows
(exactly `NUM_WINDOWS` rows, each an odd width up to 15 with finite
coefficients), `NUM_STREAMS` equal to the `STREAM_TYPE` count, and the
spectrum stream's options (`GAMMA` absent or `0`, `LN_GAIN` absent/`0`/`1`,
`ALPHA` finite in [0, 1), each key once). Duplicate rows are still accepted when their count matches
`NUM_WINDOWS`: that indexes inside the PDF and synthesizes. It accepts the pinned voice and
dictionary and jbonsai's bundled `nitech_jp_atr503_m001` voice. Over 129
corrupted voices it rejected none that jbonsai had handled safely. For the
dictionary it deliberately also rejects some corruptions upstream tolerated
silently (a partial `dict.vals` entry or word offset, a zero-sized matrix),
because those are partial reads, not usable dictionaries.

Duration planning (`plan_frames` in `src/jbonsai_backend.rs`) no longer
trusts the validator alone: every state duration and every running sum is
compared with a frame limit before it is added (`checked_add`), so an
oversized plan is `TooLong` (reported as `Failed`, before any parameter
generation) instead of an overflow. `start()` uses the output budget plus at
most 4,000 frames of trimmed edge silence as the limit; the emitted-frames
budget check and the 1 MiB cap are unchanged.

## Tests

Executed in the dedicated workflow (`M25 local TTS (hark)`) at 5a33d6e,
run 37905113176 (job A log: `test result: ok. 43 passed; 0 failed; 0
ignored` for the unit binary and `0 passed; 0 failed; 3 ignored` /
`12 ignored` / `1 ignored` for the real binaries; job C log: real_engine
`test result: ok. 12 passed`, real_adversarial `running 3 tests` with
`corrupted_dictionary_bytes_fail_closed_without_panic`,
`corrupted_voice_bytes_fail_closed_without_panic`,
`lifecycle_after_load_and_synthesis_errors_retains_nothing` and
`test result: ok. 3 passed; 0 failed; 0 ignored`, real_hostile_model
`hostile_duration_pdfs_and_window_rows_fail_closed ... ok` and
`test result: ok. 1 passed`). Findings 3/4 raise the unit count to 46
(3 new tests); the real counts are unchanged.

- Model-free (`cargo test`, CI job A): `src/validate/tests.rs` builds a small
  three-stream synthetic voice that jbonsai parses and synthesizes; every
  finding above has a case; every truncation and 1,500 seeded byte mutations
  must be rejected or synthesize to completion, never panic. Dictionary
  component checks use synthetic bytes. `src/tests.rs` adds four lifecycle
  tests over the deterministic test backend. Review findings 1/2 add
  `duration_pdf_values_must_be_finite_and_bounded`,
  `stream_pdf_values_must_be_finite`,
  `vocoder_stream_shapes_are_role_checked`,
  `window_rows_must_match_num_windows` (each failure message states what
  jbonsai does with the case when the validator is bypassed) and four
  `plan_tests` over `plan_frames` with `usize::MAX` durations. Review
  findings 3/4 add `stream_count_and_spectrum_options_are_checked_in_a_limited_child`:
  each `NUM_STREAMS` / `OPTION` case is loaded in a child process of the
  test binary under `ulimit -v` (2 GiB) and `ulimit -t` (60 s), so a
  regression that reached jbonsai's allocation cannot affect the host; five
  in-range controls must still synthesize. `header_case_child` is that
  child (a no-op pass without its environment variables) and
  `header_cases_bypass_record` (opt-in, prints only) records jbonsai's
  behaviour with the checks bypassed.
- Real artifacts (`#[ignore]`, need `NAGI_TTS_VOICE`/`NAGI_TTS_DICT`; a
  missing artifact FAILs):
  - `tests/real_adversarial.rs` (3 tests): targeted and 32 seeded
    corruptions of the pinned voice, targeted and 16 seeded corruptions of
    the dictionary, and real-engine lifecycle after load and synthesis
    errors.
  - `tests/real_hostile_model.rs` (1 test): structurally consistent edits of
    the pinned voice's duration PDFs (`f32::MAX`, `+inf`, NaN, negative,
    1e9 frames, NaN/`+inf` variance), duplicated `STREAM_WIN` rows (MCP,
    LF0, LPF), `NUM_STREAMS` 0/2, `GAMMA=999999999999999999` and
    `ALPHA=inf` are `VoiceInvalid`; in-range edits still speak.
- CI: the dedicated workflow's real-engine job runs
  `tests/m25-tts/acceptance.sh`, which runs both binaries with the pinned
  artifacts, echoes their test names and result lines into the job log,
  requires exactly 3 and 1 passes, checks the model-free run reports them as
  3 and 1 ignored, and checks each fails without artifacts.

```
cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml --test real_adversarial -- --ignored --test-threads=1
cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml --test real_hostile_model -- --ignored --test-threads=1
```
