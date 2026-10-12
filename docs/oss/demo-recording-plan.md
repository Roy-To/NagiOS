# QEMU demo recording plan

Status: **planned, not recorded or published by this preparation**.
Baseline: main `379d19eadb6f8792c27b9bc3f147e1ea48eac4a2`,
reviewed 2026-10-11. There is no tracked video at this baseline.
Do not add a player, thumbnail linked to a video, or placeholder video URL
until a real recording has been reviewed and uploaded.

日本語要約: 既存の QEMU 受け入れ証拠を基に、3〜4分の英語字幕付きデモを
計画します。撮影するコミットで再確認し、限定シナリオや自動入力を明示します。
静止画を動画として見せず、ホスト動作をゲスト動作として見せません。
推論・音声・AI操作は、個別の実証と本番統合の未完了を区別します。

## Audience and message

For first-time contributors and OSS application reviewers:
“Nagi boots independently, renders a native preview UI and a real Servo
browser, and is developing local AI within explicit authority boundaries.”

Target length: 3–4 minutes, 1280×800 guest capture at 30 fps where supported,
English captions plus Japanese subtitle file. Use a clean test account and
disposable User Data. Voiceover is optional; no background music is necessary.

## Evidence available before recording

These are prior records, not a fresh verification by this documentation branch:

| Existing reference | What it supports | Limits |
| --- | --- | --- |
| [M10 desktop PNG](../assets/screenshots/nagi-m10-qemu-desktop.png) and 2026-09-30 status checkpoint | Real QEMU fixed native panels, focus, Japanese text and serial acceptance. | Historical acceptance surface; current sign-in-enabled desktop must be reverified. |
| [M18 browser PNG](../assets/screenshots/nagi-m18-qemu-browser.png) and [CI 37601664873](https://github.com/RT-NISH/NagiOS/actions/runs/37601664873) | Guest Servo HTTPS pixels; later full interactive M18 CI acceptance. | The PNG predates interactive closure; a still does not prove navigation or upload/download. |
| [M29 Japanese Settings PNG](../assets/screenshots/nagi-m29-settings-ja-jp.png) | Selected-language UI checkpoint and recorded restart restoration. | Does not establish complete translations or accessibility. |
| [Implementation status](../implementation_status.md), login entries 2026-10-06–08 | First-run language, account creation, unlock/throttle, password change and bounded Files Search have local guest records. | Preserve command/image/commit identity; production Files Search grant route still awaits guest acceptance. |
| M19/M22 restart records in the same status file | Stable IDs, bounded Search/History/Undo fixtures. | Fixture actions, not a general user-facing autonomous workflow. |
| M20 local QEMU inference run `1791393002742512000` | Actual locked Granite inference inside Nagi. | M20 remains PARTIAL; hosted inference run failed before inference. Not a chat UI or normal lazy-loaded service. |

The status record includes local `out/evidence/` paths that are not committed
public downloads. Do not link them as remotely accessible artifacts. Preserve
them locally, and upload a reviewed subset separately if sharing evidence.

## Shot list

| Time | Shot and source command | Required observable proof | Caption / fallback |
| --- | --- | --- | --- |
| 0:00–0:15 | Title, source SHA and QEMU reference configuration. | Label “Nagi OS 0.1 Developer Preview — experimental.” | Explain independent OS and pending project license. |
| 0:15–0:45 | Boot / language / sign-in using `./nagi login`. | Guest kernel/login markers and real QEMU display; identify QMP automation if used. | “First-run and sign-in acceptance.” Use dummy credentials; exclude or redact password typing. If current run fails, report failure and omit success claim. |
| 0:45–1:10 | Native panels using `./nagi desktop` (separate acceptance image). | Current desktop READY/acceptance and actual pointer/key effects. | “Bounded native desktop acceptance surface.” Mark the cut to a separate image; do not imply these are finished apps. |
| 1:10–2:10 | Albert HTTPS, navigation, clipboard/kana, upload/download using `./nagi m18`. | Real guest TLS/render markers, visible page/input changes, trusted user activation and exact downloaded bytes in the acceptance log. | “Servo runs inside Nagi; in-process preview browser services.” Clearly identify automated input, fixture data and any separate clips. Omit unobservable operations or use labeled log evidence. |
| 2:10–2:45 | Japanese Settings selection and persistence using `./nagi m29`. | Select language, restart same User Data and show restored selection plus serial evidence. | “English / Japanese architecture; localization remains incomplete.” Label restart cuts and any time compression. |
| 2:45–3:15 | Optional bounded Files Search or restart/undo excerpt using `./nagi login`, `m19` or `m22`. | Reverified ID/state across restart, denial case where appropriate. | “Acceptance-scoped Search/History/Undo, milestones PARTIAL.” Do not portray a fixture as arbitrary real-world file management. |
| 3:15–3:45 | Optional Granite excerpt using `./nagi m20-granite-inference`. | Exact model SHA/size plus guest inference output and acceptance markers from this recording. | “Local guest inference experiment; M20 PARTIAL.” Label elapsed time/cuts. If it cannot be reverified, show a caption referencing the prior record, not simulated output. |
| Last 15 s | Scope and contribution information. | README/status/repository URL on closing slate. | M19–M30 PARTIAL, licenses under review, contribution/verification links. |

The primary story can end after Settings with a concise status slate.
Do not force optional experiments into a short demo. Speech recognition,
TTS, “Summarize this page,” general AI file edits, Qwen/Gemma inference,
mobile hardware, and polished recovery UX are excluded from success shots
unless their exact recording-build acceptance is independently established.
The prior Whisper PCM fixture is not a microphone voice-control experience.

## Recording preparation

1. Use a separate clean checkout for recording; do not repurpose an active
   implementation worktree or change any milestone. Record `git rev-parse HEAD`
   and `git status --short`, toolchain versions, host CPU/OS, QEMU version,
   OVMF and reference VM settings. Note TCG/KVM/HVF explicitly.
2. Read current `./nagi --help` and the relevant acceptance harness.
   Run `fetch` / `doctor` and required build preparation with adequate disk
   space. Keep model acquisition separate; do not fetch or accept Gemma
   terms as part of a basic demo.
3. Save the required acceptance output and serial markers at the exact SHA.
   Commands can have fixture-specific images and automated QMP input.
   Do not assume `./nagi image && ./nagi run` exposes every feature.
4. For each restart scene preserve the same disposable User Data disk and
   keep image / disk / OVMF-variable hashes plus ordered logs. Do not use
   private notes, real accounts, cookies or model conversations.
5. Capture the actual QEMU guest window with a host screen recorder. A side
   log panel is useful for proof but must be visibly labeled “host harness /
   guest serial log.” Host tools are not Nagi applications.
6. If a headless automated command exits too quickly, preserve its harness
   semantics in a recording-only setup, reviewed separately from this
   documentation change. Do not claim that current commands already have a
   recording mode. Keep failures and original uncropped recordings.

## Editing, attribution and publication gate

- Preserve raw footage, invocation/serial logs, screenshots and SHA256SUMS in
  one evidence directory per shot. List source SHA, image feature flags,
  start/end timestamps and all cuts/time-compression in a capture manifest.
- Match visible changes to guest evidence. No synthetic frames, host-browser
  substitutions, or static screenshots presented as live guest footage.
  Avoid boot-speed or inference-speed claims based on edited footage.
- Check captions in both languages for implemented versus planned behavior.
  Explain automation and separate acceptance images.
- Review external page content, fonts, artwork and voice/audio reuse terms
  with the [license review](apache-2.0-readiness.md). Do not use unlicensed
  music. Redact secrets without obscuring the feature proof.
- Upload only the reviewed real video. Verify playback and captions at the
  actual URL, then add that URL to both READMEs in a later documentation PR,
  with capture SHA/date and a brief statement of scope. Keep prior evidence
  and label superseded captures.

No video is produced, uploaded or linked by this branch.
