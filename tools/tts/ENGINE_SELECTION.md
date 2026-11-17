# M25 local TTS — engine and voice selection

Spec reference: `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md` §54 (evaluate
Piper, MeloTTS, or another local engine if licensing/portability is superior;
criteria: Japanese quality, CPU, RAM, porting difficulty, binary dependencies,
engine license, voice/model license). The Speech API stays fixed; this engine
sits behind `nagi_audio::speech::TextToSpeechProvider` and is replaceable.

Every license statement below was read from the cited upstream file at the
time of writing (2026-10-09). Nothing here accepts any distribution terms.

## Candidates

| Candidate | Engine / runtime | Japanese front end | Voice / model | Portability to Nagi | Decision |
|---|---|---|---|---|---|
| **jpreprocess + jbonsai** (HTS / Open JTalk lineage, pure Rust) | jbonsai 0.4.2, BSD-3-Clause ([LICENSE](https://github.com/jpreprocess/jbonsai/blob/4aa02e4dbe5ea4e3e8058477d612f116d00ed1b6/LICENSE), NOTICE carries hts_engine API BSD-3 terms) | jpreprocess 0.15.0, BSD-3-Clause ([NOTICE](https://github.com/jpreprocess/jpreprocess/blob/07a78220c1e850a001998451bd1e7c88709ee379/NOTICE): Open JTalk BSD-3); dictionary naist-jdic, BSD-3-Clause (NAIST 2009 + UniDic Consortium 2011-2017 BSD-3, [COPYING](https://github.com/jpreprocess/naist-jdic/blob/394af50de2da4a9c73b1bd04fab43e85bb6f03ff/COPYING)) | HTS voice `tohoku-f01-neutral`, CC BY 4.0 ([COPYRIGHT.txt](https://github.com/icn-lab/htsvoice-tohoku-f01/blob/8e3306021db135c265f5eda5f062dc489707ddf8/COPYRIGHT.txt)); 2,154,716 bytes | Pure Rust, no C/C++ libraries, no ONNX/PyTorch. Requires `std` (filesystem helpers, lindera); Nagi has a patched `std` (`third_party/rust-std`). Synthesis math is portable. | **Selected** |
| Piper (rhasspy/piper, MIT; archived 2025-10-06) → piper1-gpl | VITS/ONNX Runtime (C++). Successor [OHF-Voice/piper1-gpl](https://github.com/OHF-Voice/piper1-gpl) is GPL-3.0 and "embeds espeak-ng" (GPL-3.0) | espeak-ng (GPL-3.0); `ja_JP` config uses `"phoneme_type": "japanese"` | Only Japanese voice `ja_JP-hi_fi_captain-medium` (piper-voices rev `c10ece1aade47bb51c153c893d14e5bf8e5b7117`); its [MODEL_CARD](https://huggingface.co/rhasspy/piper-voices/blob/c10ece1aade47bb51c153c893d14e5bf8e5b7117/ja/ja_JP/hi_fi_captain/medium/MODEL_CARD) states dataset license CC BY-NC-SA 4.0 | ONNX Runtime C++ port to Nagi required (large); GPL engine line | Rejected: non-commercial voice, GPL engine/phonemizer, heavy runtime |
| MeloTTS (myshell-ai, MIT) | PyTorch (VITS2/Bert-VITS2 derived) | `mecab-python3`, `fugashi`, `unidic`, `unidic_lite`, `pykakasi`, plus `transformers` BERT ([requirements.txt](https://github.com/myshell-ai/MeloTTS/blob/main/requirements.txt)) | MeloTTS-Japanese weights + Japanese BERT | Python/PyTorch only; no C/Rust inference path; BERT adds hundreds of MB RAM | Rejected for 0.1: not portable to Nagi without a new inference runtime |
| Kokoro-82M | PyTorch/ONNX; weights Apache-2.0 (HF `hexgrad/Kokoro-82M` rev `f3ff3571791e39611d31c381e3a41a3af07b4987`, `license: apache-2.0`) | misaki `ja` extra: `fugashi`, `jaconv`, `mojimoji`, `unidic`, `pyopenjtalk` ([pyproject.toml](https://github.com/hexgrad/misaki/blob/main/pyproject.toml)) | 82M parameters | Needs ONNX/GGML-class runtime plus Python-side G2P port | Not selected for 0.1; viable later neural upgrade (license OK, port cost high) |
| VOICEVOX CORE (MIT core) | ONNX Runtime | Open JTalk | VVM voice models: "音声モデル（VVM ファイル）には利用規約が存在します" ([usage.md](https://github.com/VOICEVOX/voicevox_core/blob/main/docs/guide/user/usage.md)) | ONNX Runtime port | **Held**: voice models require accepting separate terms; not accepted |
| sherpa-onnx (Apache-2.0) | ONNX Runtime | per model | per model (no Japanese model selected) | ONNX Runtime port | Not selected (runtime port; no license-clean Japanese voice identified) |

## Why jpreprocess + jbonsai

- Japanese quality: Open JTalk-equivalent text analysis (accent, number
  reading, devoicing) feeding HMM synthesis; intelligible, natural-accent
  Japanese but clearly synthetic. The spec lists "imperfect TTS voice quality"
  as an allowed Developer Preview issue (§92).
- CPU: upstream benchmark reports jbonsai 1.6–2.2× faster than the C
  hts_engine API, 128 s of speech in ~0.9 s on Apple M2 / i5-13500 (README).
  Measured numbers for this sandbox are in `tests/m25-tts/MEASUREMENTS.md`.
- RAM: a 2.1 MB voice plus the naist-jdic dictionary; no neural weights.
- Portability: pure Rust; no GPL code, no C/C++ toolchain, no ONNX/PyTorch.
- Licenses: BSD-3-Clause (engine, front end, dictionary) and CC BY 4.0
  (voice). Both need attribution notices only; no click-through terms.

Obligations (proposed in `.dev/workstreams/hark-m25-tts/integration-proposal.md`):
BSD-3 binary-redistribution notices for jbonsai/hts_engine API,
jpreprocess/Open JTalk, lindera, naist-jdic (NAIST, UniDic Consortium), and CC
BY 4.0 attribution for tohoku-f01 (credit, licence link, change indication).

## Pins

See `tools/tts/tts-artifacts.lock` and `tools/tts/fetch.sh`.
