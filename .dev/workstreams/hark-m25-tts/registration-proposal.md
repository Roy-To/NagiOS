# Registration proposal — hark-m25-tts (M25 local TTS provider)

Status: PROPOSED (not applied to `.dev/workstreams.json`; that file is
outside this stream's owned paths).

## Baseline

- Base commit: `edad2e7` (origin/main, "Merge pull request #31 from
  RT-NISH/hark/m19-files-search-client").
- Branch: `hark/m25-local-tts` (dedicated; never merged directly by this
  stream; draft PR only).
- Milestone: M25 — Voice, remaining blocker 3 in
  `docs/workstreams/NagiOS_M25_Voice_Workstream.md` ("Add a concrete local
  TTS engine behind the provider contract … and verify real playback").

## Proposed registry entry

```json
{
  "id": "hark-m25-tts",
  "name": "M25 local Japanese TTS provider",
  "owner": "Hark M25 TTS stream",
  "owner_branch": "hark/m25-local-tts",
  "state_file": ".dev/workstreams/hark-m25-tts/state.json",
  "dependencies": ["development-foundation"],
  "allowed_paths": [
    "crates/nagi-tts-provider/**",
    "tools/tts/**",
    "tests/m25-tts/**",
    ".dev/workstreams/hark-m25-tts/**"
  ],
  "forbidden_paths": [
    "Cargo.toml", "Cargo.lock",
    ".github/workflows/**", ".dev/workstreams.json", ".dev/schemas/**",
    "docs/implementation_status.md",
    "third_party/**", "THIRD_PARTY_NOTICES.md",
    "user/**", "kernel/**", "loader/**",
    "tools/nagi-cli/**", "out/**", "target/**"
  ],
  "activation_gate": "Host provider, engine selection, pinned fetch, and host synthesis evidence may be prepared on the dedicated branch. Guest wiring (nagi-init feature, Model Store artifact registration, ./nagi m25 marker, release image) requires the shared-file owner to apply integration-proposal.md.",
  "merge_boundary": "Draft PR to main only; merge requires the repository owner's review."
}
```

## Read-only dependencies (never edited by this stream)

- `user/nagi-audio` (path dependency): `speech::TextToSpeechProvider`,
  `SpeechSynthesisOptions`, `SpeechSynthesisLanguage`, `SynthesisPcmChunk`,
  `SpeechProviderError`, `SpeechSynthesisService`, `SpeechPlaybackSink`,
  `Stereo48KhzToMono16Khz`, `PcmFormat`, and the constants
  `MAX_SPEECH_SYNTHESIS_TEXT_BYTES` (1 KiB), `MAX_SPEECH_PCM_CHUNK_BYTES`
  (4 KiB), `MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES` (1 MiB).
- `AudioServicePlaybackSink` (target-only) accepts only stereo 48 kHz S16LE;
  the provider therefore emits stereo 48 kHz by default and 16 kHz mono only
  on request.

## AGENTS.md conditions

AGENTS.md does not define a separate owner-registration approval step; it
requires pinned third-party revisions with reproducible fetch, no fake
results, no host escape, and `docs/implementation_status.md` updates after
meaningful steps. `docs/implementation_status.md` is a shared file for this
stream, so the status text is supplied as a proposal in
`integration-proposal.md` instead of being edited here. Work proceeds only
inside the owned paths above.

## Open gates

1. Shared owner applies the registry entry (optional; the crate is
   standalone and builds without it).
2. Shared owner reviews the license/notice and lock additions.
3. Guest playback verification needs the integration diff and QEMU.
