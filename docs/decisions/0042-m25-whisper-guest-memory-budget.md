# ADR 0042: M25 Whisper guest memory budget

Status: accepted for M25 Whisper fixture inference
Date: 2026-10-03
Milestone: M25 — Voice

## Context

Real QEMU attempts verified the pinned model through the Model Store and
parsed its model header. A 64 MiB heap failed at the 487,005,696-byte CPU
weight buffer; 768 MiB loaded the weights but failed while allocating a
78.91 MiB state arena; 1 GiB advanced through the 128.02 MiB encode buffer but
still failed at that state arena. The default kernel mmap window is bounded at
256 MiB. The guest therefore needs a larger finite budget to hold the locked
487,601,967-byte model and run its CPU inference path, despite the official
QEMU target having 8 GiB RAM.

The first full guest inference attempt loaded the locked model and completed
state allocation, but spent more than two hours in CPU-bound QEMU TCG inference
without a transcript marker. The sample contains about 1.48 seconds of speech;
whisper.cpp's default encoder context still reserves and computes a 30-second
window, including its zero-padded tail.

The memory failure is in the guest allocator and address-space budget. The
model is loaded through Nagi's read-only Model Store capability; routing model
bytes through the host or bypassing guest allocation would invalidate the
acceptance.

## Decision

- Add an opt-in kernel `m25-whisper-memory` feature that gives this acceptance
  image a bounded 1.5 GiB bootstrap mmap window. Keep the 256 MiB default and
  512 MiB M18 window unchanged.
- Add an opt-in POSIX `m25-whisper-memory` feature that caps the guest heap at
  1.25 GiB, leaving 256 MiB of the mmap window for other guest mappings.
- Enable both features only for the real M25 Whisper inference QEMU image.
  The artifact-digest-only acceptance, standard M30 image, and other
  milestones keep their current memory budgets.
- Set whisper.cpp's supported `audio_ctx` to the context required by the actual
  PCM sample count plus one second of zero-tail, rounded up to one encoder
  position per 320 samples (20 ms at 16 kHz), and capped at the model's
  declared maximum. This retains every supplied PCM sample and avoids running
  the encoder over the unused remainder of the fixed 30-second window.
- Preserve guest-only backing, mmap ownership checks, checked allocation
  failures, and the official 8 GiB QEMU RAM limit. Do not add a host allocator
  or a model-specific kernel syscall.

## Consequences

The M25 acceptance kernel reserves 1.25 GiB more static mmap backing than the
default kernel and 1 GiB more than the M18 kernel. This is a finite bootstrap
resource budget for loading the locked Whisper-small model and its inference
state. The M25 address-space layout needs a second extra page directory for the
final four page-table entries. Replacing static bootstrap backing with
dynamically allocated process VM remains a separate architecture change. The
model context remains unmodified; only per-utterance encoder work is bounded
by the fixture duration plus one second. QEMU fixture acceptance must still
verify the expected Japanese text, since the context limit is an inference
parameter.

## Verification

Kernel tests validate the M25-only 1.5 GiB mapping bound and its two extra
page-directory boundaries; POSIX tests validate the M25-only 1.25 GiB heap
bound. Compile-time checks validate the sample-to-context calculation. The
Nagi-target library and provider build passed. QEMU run
`1790978330938078000` loaded the locked artifact through the guest Model Store,
passed the artifact digest and Model Store capability checks, and emitted the
Japanese fixture inference PASS marker after the real transcript contained the
expected phrase. The QEMU log, first-boot log, target build log, PCM fixture,
expected text, OVMF variables, image, and README are covered by the evidence
directory's verified eight-entry `SHA256SUMS`; `qemu-img check` reported no
errors. The optimized run took about 37 minutes under TCG. This validates only
the short fixture and does not measure sustained or live-microphone inference.
All non-M25 image feature graphs retain their existing mmap and heap budgets.
