# Registration proposal — hark-m25-whisper (M25 Whisper production hardening)

Status: PROPOSED. Not applied to `.dev/workstreams.json`; the shared registry
is outside this stream's owned paths.

## Baseline

- Base commit: `f74f128abe40e9e99d7cb05f3f8dc7b90b1448c9` (main after PR #36;
  work began at `edad2e7`, no owned path changed in between).
- Branch: `hark/m25-whisper-production` (dedicated; draft PR only; never
  merged or marked ready by this stream).
- Milestone: M25 — Voice. Scope is the Whisper speech-to-text provider only:
  production/fixture separation, multi-utterance lifecycle (consecutive
  utterances, cancel, unload/reload, resume after failure), and measured
  accuracy/time/RAM on unseen, openly licensed Japanese speech.
- Existing guest result kept as-is: the short Japanese PCM fixture inference
  already passes in QEMU (ADR 0042, run `1790978330938078000`); it is a
  fixture check and is not counted as recognition evidence. This stream
  does not reimplement Whisper and does not change the pinned whisper.cpp
  revision, patches, model artifact, or the ADR 0042 memory budget.

## Proposed registry entry

```json
{
  "id": "hark-m25-whisper",
  "owner": "Hark M25 Whisper stream",
  "owner_branch": "hark/m25-whisper-production",
  "recommended_worktree": "../NagiOS-m25-whisper",
  "state_file": ".dev/workstreams/hark-m25-whisper/state.json",
  "dependencies": ["development-foundation", "model-runtime"],
  "allowed_paths": [
    "user/nagi-init/src/m25_whisper.rs",
    "tools/whisper/**",
    "tests/m25-whisper/**",
    ".dev/workstreams/hark-m25-whisper/**"
  ],
  "forbidden_paths": [
    "Cargo.toml", "Cargo.lock", "user/nagi-init/Cargo.toml",
    "user/nagi-init/src/main.rs", "user/nagi-init/build.rs",
    "user/nagi-init/src/desktop.rs",
    "user/nagi-init/src/m19_files_client.rs",
    "user/nagi-init/src/m19_files_runtime.rs",
    "user/nagi-init/src/m19_files_search.rs",
    "user/libnagi/src/storage.rs",
    "user/nagi-audio/**",
    "tools/nagi-cli/**",
    "third_party/**", "THIRD_PARTY_NOTICES.md",
    "docs/implementation_status.md",
    ".github/workflows/**", ".dev/workstreams.json", ".dev/schemas/**",
    "kernel/**", "loader/**"
  ],
  "activation_gate": "Provider lifecycle code, host harness, and host measurements may be prepared on the dedicated branch. Exposing the production provider outside the opt-in acceptance feature, CI registration, Model Store/runtime routing, microphone, UI, permission, and command execution require the respective owners to apply integration-proposal.md.",
  "merge_boundary": "Draft PR to main only; merge and release require separate user confirmation."
}
```

The current `owner_branch` schema pattern does not admit `hark/*`; the
minimal schema diff is in `integration-proposal.md` §1. With that diff the
entry validates against `.dev/schemas/workstreams.schema.json`.

## Overlap check record

| When (JST) | Check | Result |
|---|---|---|
| 2026-10-09 09:35 | Open PRs (#37 `codex/0.1-session-model-service`, #38 `hark/m24-embedding`) changed files vs owned paths; `codex/*`, `claude/*` branches since `edad2e7`; `.dev/workstreams.json` mentions of whisper | No overlap (recorded by dispatcher) |
| 2026-10-09 09:45 | Path ownership of the four allowed paths | Confirmed for this stream; Model/Granite owner does not change this range |
| 2026-10-09 10:25 | Open PRs #37, #38 changed files; branches matching `whisper`/`m25`; `edad2e7..f74f128` diff | No overlap (only `hark/m25-local-tts`, disjoint paths) |

If a Codex / Model-owner change to `user/nagi-init/src/m25_whisper.rs`
appears, this stream stops editing that file, records it here, and keeps
working only in `tools/whisper/**`, `tests/m25-whisper/**`, and this
directory.

## Read-only dependencies

- `user/nagi-audio` (path dependency): `speech::SpeechToTextProvider`,
  `SpeechOptions`, `SpeechLanguage`, `SpeechProviderError`, `PcmFormat`,
  `MAX_SPEECH_PCM_CHUNK_BYTES` (4 KiB), `MAX_SPEECH_UTTERANCE_BYTES` (1 MiB),
  `MAX_SPEECH_TRANSCRIPT_BYTES` (1 KiB).
- `user/nagi-model-manager`: `ArtifactId`, `Fat32ArtifactReader`,
  `ModelArtifactReader` (guest only).
- `user/nagi-init/src/main.rs`: `SyscallModelStoreReader`, the
  `m25-whisper-inference-acceptance` module gate, and the
  `m25_whisper::run` call / serial markers (unchanged).
- `third_party/models.lock` `[models.whisper_small_multilingual]` and
  `third_party/sources.lock` `[sources.whisper_cpp]` with patches
  `0001`–`0003` (unchanged; host harness applies them to a scratch copy).
- ADR 0042 memory budget (unchanged).
