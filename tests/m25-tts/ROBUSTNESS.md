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

Not changed (reported only): an `HTS_VOICE_VERSION` other than `1.0`, or a
`NUM_STREAMS` that disagrees with `STREAM_TYPE`, still loads when the rest of
the voice is consistent; nothing breaks, and content integrity remains the
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
loaded unknown-word references). It accepts the pinned voice and
dictionary and jbonsai's bundled `nitech_jp_atr503_m001` voice. Over 129
corrupted voices it rejected none that jbonsai had handled safely. For the
dictionary it deliberately also rejects some corruptions upstream tolerated
silently (a partial `dict.vals` entry or word offset, a zero-sized matrix),
because those are partial reads, not usable dictionaries.

## Tests

- Model-free (`cargo test`, CI job A): `src/validate/tests.rs` builds a small
  three-stream synthetic voice that jbonsai parses and synthesizes; every
  finding above has a case; every truncation and 1,500 seeded byte mutations
  must be rejected or synthesize to completion, never panic. Dictionary
  component checks use synthetic bytes. `src/tests.rs` adds four lifecycle
  tests over the deterministic test backend.
- Real artifacts (`tests/real_adversarial.rs`, `#[ignore]`, needs
  `NAGI_TTS_VOICE`/`NAGI_TTS_DICT`; not run by the dedicated workflow, which
  runs only `--test real_engine`): targeted and 32 seeded corruptions of the
  pinned voice, targeted and 16 seeded corruptions of the dictionary, and
  real-engine lifecycle after load and synthesis errors.

```
cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml --test real_adversarial -- --ignored --test-threads=1
```
