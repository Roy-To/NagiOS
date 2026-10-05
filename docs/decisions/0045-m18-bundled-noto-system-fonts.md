# ADR 0045: Bundled Noto system fonts for Servo

Status: accepted for M18 (font choice confirmed by the user, 2026-10-05)
Date: 2026-10-05
Milestone: M18 — Albert Browser

## Context

Nagi's Servo font platform (`platform/nagi/font_list.rs`, from patch 0006)
reported an empty system-font registry "until the Nagi font package service is
present". Pages without web fonts, including `example.com`, therefore painted
no text at all: the three M18 HTTPS frames were the same background-only image
(checksum `0x5a9955c5`), and form fields showed no characters. The M18
acceptance only required TLS evidence and a nonzero frame checksum, so it
accepted blank pages.

Servo's FreeType backend loads local fonts by path (`File::open` plus `mmap`).
The writable User Data VFS is not suitable for multi-megabyte system assets,
and the host filesystem must not be used.

## Decision

- Bundle Noto Sans Regular and Bold (Latin) and Noto Sans JP Regular (JP
  region subset OTF), all SIL OFL 1.1, as Nagi 0.1's system fonts. Japanese is
  a first-class Nagi language, so a Japanese face ships from the start.
- Pin each font and license text in `third_party/fonts.lock` by URL at an
  immutable upstream revision, byte size, and SHA-256. `./nagi fetch` downloads
  them into the ignored `out/cache/fonts/` cache, rejects mismatches, and
  writes a manifest. Font bytes are not committed to the repository.
- The Servo-enabled `nagi-init` build embeds the manifest's files and, before
  Servo starts, publishes them through a new bounded read-only static-file
  table in `nagi-posix` under `/system/fonts/`. `open`, `read`, `fstat`,
  `stat`, and file-backed `mmap` work; writes and truncation fail. Only
  `/system/` paths may be published.
- Servo patch 0026 reports these families when their files exist, falls back
  to Noto Sans JP first for Japanese text and CJK punctuation, and resolves
  every generic family to Noto Sans.
- The M18 serial evidence now reports `ink_pixels` (non-background pixels in
  Servo's own frame) for each HTTPS page, and the host validator requires at
  least 200, so a background-only page fails.

## Consequences

- Web pages render text in the guest, including Japanese.
- The init image grows by about 5.8 MiB. A later font package service can
  replace the embedded table without changing Servo's path contract.
- Only a sans-serif design is available; serif and monospace requests use
  Noto Sans in 0.1.
- Adding the Servo patch requires regenerating the pinned Servo checkout.
