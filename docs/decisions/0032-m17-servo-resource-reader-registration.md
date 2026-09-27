# M17 Servo bundled-resource registration on Nagi

Status: accepted for the M17 bootstrap
Date: 2026-09-27
Milestone: M17 — Servo Bootstrap

## Context

Actions run 36292384786 (run #287, head
`63b2cc504e8c0ffb0f27581dd506970ef0577da7`) passed both host jobs and all
target builds. Real QEMU confirmed that the 32-slot thread pool now carries
Servo through storage-thread and constellation startup. During TLS prewarm, a
Constellation worker then panicked in
`third_party/servo/components/shared/embedder/resources.rs:66` with
`No resource reader registered`.

The pinned Servo feature graph includes `servo-default-resources` through
Servo's `bundled` feature, and that crate contains the real 11 embedded Servo
resources. Its `DefaultResourceReader` registers itself with
`inventory::submit_resource_reader!`. Nagi's user entry keeps and executes
`.init_array` constructors before application startup, but the locked
`inventory` 0.3.24 constructor macro assigns `.init_array` only for its
allowlisted ELF OS values; custom `target_os = "nagi"` is missing. The
registration static is therefore not run on Nagi, leaving Servo's registry
empty. This is an ELF target integration gap, not a missing asset, host
filesystem requirement, or rendering fallback.

## Decision

- Keep Servo's pinned bundled resource reader and all upstream resource bytes.
- Vendor the exact locked `inventory` 0.3.24 crate into Nagi's tracked
  `third_party` tree, record its upstream archive SHA-256 and Nagi patch in
  `third_party/sources.lock`, and route the workspace dependency through that
  Nagi-owned patched copy.
- Extend only `inventory`'s ELF constructor target list to include
  `target_os = "nagi"`. The existing Nagi linker script retains `.init_array`
  entries and the user entry executes them before `ServoBuilder::build()`.
- Add a guest preflight read of Servo's embedded domain-list resource before
  constructing Servo. Report completion only after the real reader returns
  nonempty embedded data.
- Do not change Servo's resource selection, introduce runtime host paths, or
  synthesize resource contents.

## Verification

Verify the vendored crate against its locked 0.3.24 archive hash and apply the
Nagi patch to a pristine source copy. Run the focused Nagi CLI source-contract
test, CI format and host jobs, custom Nagi target builds, and public
`nagi-target` QEMU acceptance. Require the guest preflight marker, successful
Servo/WebView startup, real first-web-pixel checksum, and M17 PASS marker.
Keep M17 `BLOCKED` until the authoritative target run shows all acceptance
evidence; M18 remains `NOT STARTED`.
