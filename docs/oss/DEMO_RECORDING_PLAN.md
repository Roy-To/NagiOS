# Nagi OS QEMU Demo Recording Plan

**Status:** planned; no video has been recorded or published by this documentation change.
**Reference source:** bcb92dc (2026-10-12).
**Target:** a 2–3 minute, English-captioned introduction with Japanese subtitles.

This plan covers only behavior that has QEMU guest evidence in the repository.
It is a recording procedure, not a claim that a video exists. Before filming,
rerun each selected scenario on the exact source revision to be shown and read
the current [implementation status](../implementation_status.md). A historic
PASS does not prove a later source revision.

## Audience and message

For developers encountering Nagi for the first time:

> Nagi boots its own kernel, presents a native desktop, and runs the Servo-based
> Albert browser inside a QEMU guest. Local AI, history, and recovery are being
> developed behind explicit authority boundaries.

Identify Nagi as an experimental 0.1 Developer Preview. A short main cut can
show the M10 desktop, M18 Albert, and M29 Japanese Settings. Search, undo, and
local Granite inference are optional research excerpts: each is still partial
and must be labeled as a bounded acceptance scenario.

## Existing QEMU evidence

The following are previous records, not fresh verification by this document:

| Record | What it supports | Limits |
| --- | --- | --- |
| [M10 desktop image](../assets/screenshots/nagi-m10-qemu-desktop.png) and M10 row in the [status record](../implementation_status.md) | QEMU guest with four fixed Calculator, Notes, Files, and Terminal panels; M10 acceptance is PASS. | This is a bounded desktop surface, not a finished app suite. Reverify the sign-in-enabled flow on the capture source. |
| [M18 Albert image](../assets/screenshots/nagi-m18-qemu-browser.png) and [M18 target CI run 37601664873](https://github.com/RT-NISH/NagiOS/actions/runs/37601664873) | The image shows a real guest-rendered HTTPS page. The later CI run passed the full M18 scenario: three HTTPS pages, permission, clipboard, kana IME, upload, and user-activated download. | The image is dated 2026-10-01 and predates interactive CI closure on 2026-10-07. It does not prove those interactions. |
| [M29 Japanese Settings image](../assets/screenshots/nagi-m29-settings-ja-jp.png) and M29 row in the [status record](../implementation_status.md) | QEMU acceptance selected Japanese Settings and restored it after restarting with the same User Data disk. | Does not prove complete translation, accessibility, or propagation to other processes. |
| M20 local QEMU inference run 1791393002742512000 in the [status record](../implementation_status.md) | Granite 4.2 3B ran inside Nagi and produced a schema-constrained Japanese response. | M20 is PARTIAL. Model installation and normal service integration remain open; this is not a finished chat UI or Personal Agent. Keep it optional. |

Local evidence directories under out/evidence are not public downloads. Do not
link them as if viewers can access them.

## Shot list

| Time | Shot and QEMU command | Evidence to retain | Caption and boundary |
| --- | --- | --- | --- |
| 0:00–0:15 | Title card with source revision and QEMU reference configuration. | Record the source SHA, host, QEMU version, and accelerator. | “Nagi OS 0.1 Developer Preview — experimental.” State that Nagi's project license is not selected. |
| 0:15–0:50 | Native desktop with ./nagi desktop. | Guest READY/acceptance markers and visible keyboard or pointer input. | “M10 fixed desktop acceptance surface.” Do not present the panels as finished applications. Label QMP or host-harness input if used. |
| 0:50–1:55 | Albert browser scenario with ./nagi m18. | Guest TLS and render markers, visible page changes, and the scenario's interaction results. | “Servo browser running inside Nagi.” Mark automated QMP input and fixture content. Omit any interaction not visible or evidenced in this recording. |
| 1:55–2:35 | Japanese language selection and restart with ./nagi m29. | The selection, restart using the same disposable User Data disk, restored setting, and ordered serial output. | “Japanese Settings selection and restart restoration.” This does not mean the whole interface is translated. Label cuts and time compression. |
| 2:35–3:00 | Closing slate with links to README and implementation status. | Show repository and current milestone summary. | M19–M30 are PARTIAL; formal public OSS release is planned for 1.0; Nagi's license is unselected. |

End the main demo after Settings if the short runtime is preferred. Do not force
optional features into it.

### Optional research excerpts

| Scenario | Command | Include only when | Required caption |
| --- | --- | --- | --- |
| Bounded Search or History/Undo | ./nagi login, ./nagi m19, or ./nagi m22 | The exact QEMU scenario is rerun and its identifiers/state are visible across the accepted restart flow. | “Acceptance-scoped Search/History/Undo; milestone PARTIAL.” Do not imply general Files integration or autonomous AI actions. |
| Local Granite inference | ./nagi m20-granite-inference | The exact model digest, guest output, and acceptance markers are recorded on the filming source. | “Local guest inference experiment; M20 PARTIAL.” Identify fixture input, cuts, and elapsed time. This is not a chat UI or end-to-end Agent. |

Do not use speech recognition, TTS, page summarization, general AI file edits,
Qwen/Gemma inference, physical hardware, or polished Recovery UX as success
shots unless the exact filming revision has fresh target acceptance for them.
A Whisper PCM fixture is not a microphone voice-control demonstration.

## Recording preparation

1. Prepare a clean, separate checkout for the recording. Do not reuse an active
   implementation worktree. Record the source SHA, clean/dirty status, host OS
   and CPU, Rust toolchain, QEMU/OVMF versions, reference VM settings, and
   whether TCG, KVM, or HVF is used.
2. Read the current command list and the relevant acceptance harness. Run
   doctor and the required fetch/build preparation with adequate disk space.
   The current supported commands are listed by the repository's nagi tool.
   Do not fetch optional model weights or accept Gemma terms for the main demo.
3. Reverify each selected acceptance on the filming source. Keep the exact
   command output, guest serial log, relevant screenshots, and acceptance
   result. A host unit test or a source-code path is not guest evidence.
4. Use a disposable account and User Data disk. For the M29 restart shot,
   preserve the same disk across the restart. Do not show real credentials,
   personal data, private pages, cookies, or model conversations.
5. Capture the actual QEMU guest display with a host screen recorder. If a
   harness/log panel is visible, label it “host harness / guest serial log.”
   Host tools are not Nagi applications. Do not composite a host browser or
   synthesized output into the guest.
6. If a headless acceptance command exits before a useful capture, do not claim
   that a recording mode exists. Any recording-only setup must preserve the
   acceptance scenario and be reviewed separately. Keep failed attempts and
   the original uncropped capture.

## Capture and publication record

Store raw footage and evidence in a separate, reviewable directory per
recording. Include a manifest with at least:

~~~text
source_commit:
host_os_cpu:
rust_toolchain:
qemu_ovmf_accelerator:
reference_vm_configuration:
commands_and_exit_codes:
guest_acceptance_markers:
evidence_files_and_sha256:
recording_start_end:
cuts_or_time_compression:
host_harness_input:
captions_en_ja:
known_limits:
~~~

- Match each visible guest change to acceptance output from the same source.
  Do not use synthetic frames or old screenshots as live footage. If an
  archived still is inserted, label it as an earlier QEMU screenshot with its
  milestone and date.
- Keep original footage, logs, evidence hashes, and a record of edits. Do not
  make boot speed or inference latency claims based on time-compressed footage.
- Review English and Japanese captions for implemented, partial, and future
  behavior. Explain QMP automation, test data, restarts, and separate clips.
- Before publishing, review rights for page content, fonts, images, and audio.
  Do not add music without permission. Nagi's own project license remains
  unselected; third-party asset terms are separate.
- Upload only a reviewed recording. Verify the final video and subtitle files
  at their destination. A later change can add the real URL to both READMEs with
  the recording date, source SHA, and demonstrated scope.

No video file, thumbnail, or video URL is created by this plan.
