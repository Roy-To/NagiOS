# ADR 0035: M17 bootstrap partial mmap ranges

Status: accepted for the M17 bootstrap
Date: 2026-09-28
Milestone: M17 — Servo Bootstrap

## Context

Public CI run #300 (`36341631295`, head
`06392ccf5acc742cb3b2ba70b09e9cea8e5b2e7a`) passed the host jobs, target
dependency checks, Mesa Softpipe, the M16 package, kernel, real `nagi-init`
link, and UEFI loader. Both QEMU boots reached SpiderMonkey's first GC chunk
alignment path. The target timed out with exit code 4 before producing the
first-web-pixel checksum or M17 PASS marker.

The new checkpoints show that the downward hint mapping was rejected and
cleaned up, the upward hint mapping succeeded, and execution then entered the
upward prefix `munmap` without returning. The Nagi POSIX wrapper delegates to
`munmap_user`, which only recognized an exact full mapping record. Its
`EINVAL` failure conflicts with SpiderMonkey's assertion that an unmap failure
reports `ENOMEM`. More generally, the implementation status exposed that the
bootstrap range table represented whole calls to `mmap`, while the Nagi 0.1
specification requires POSIX `munmap` and `mprotect` through the compatibility
layer.

## Decision

- Keep the 128 MiB process mmap window and the 64 live bootstrap reservation
  identities established by ADRs 0027, 0029, 0031, and 0034.
- Track reservation ownership per 4 KiB page in a fixed 32 KiB owner array.
  Owner zero means free; nonzero owners identify one of the 64 active
  reservations. Reservation slots retain a live-page count and are released
  when their final page is unmapped.
- Allocate first-fit runs only from owner-zero pages. Zero the backing pages
  before publishing their reservation owner, including when a freed range is
  reused.
- Let `munmap` release any page-aligned range wholly covered by owned pages,
  including ranges that trim one reservation, split one reservation, or span
  adjacent reservations. Validate the complete range and all reservation
  accounting before changing page tables or ownership.
- Let `mprotect` change any page-aligned range wholly covered by owned pages,
  including `PROT_NONE` reservations and ranges spanning adjacent owners.
  Ownership is independent of PTE presence and protection.
- Keep exact-address `mmap_user_at` narrow: its complete range must be one
  contiguous live fragment of one reservation identity, with no same-owner
  page immediately before or after it. It cannot cross a free hole or combine
  adjacent independent reservations.
- Preserve the single bootstrap address space, its existing syscall and page
  table protection checks, and its serialized mapping-mutation model. Do not
  add a host mapping path, expand the virtual window, or weaken address
  ownership checks.
- Invalidate each changed mmap translation in the active address space after
  mapping, unmapping, or protection changes so re-used pages cannot retain a
  stale TLB entry.

## Consequences

- The additional ownership metadata is fixed at 32 KiB in the kernel's
  bootstrap storage; mapping and range validation require no heap allocation
  or variable-size kernel-stack scratch space.
- The 64-reservation resource bound remains in force while each reservation
  can be fragmented into any set of pages in the finite mmap window.
- `munmap` and `mprotect` now follow the subrange behavior needed by Servo's
  GC chunk alignment while retaining the bounded bootstrap design.
- Mapping and protection updates invalidate the affected active-address-space
  translations, including pages changed before a failed remap is reported.
- M17 remains `BLOCKED` until public target CI reports the real nonzero
  first-web-pixel checksum and M17 PASS marker. M18 remains `NOT STARTED`.

## Verification

The kernel tests cover first-fit ownership; prefix, suffix, and middle unmap;
adjacent-owner unmap and protection; atomic hole rejection; `PROT_NONE`
ownership; exact remapping of surviving fragments; and reservation-slot
release/reuse. All 110 kernel unit tests pass on the x86_64 macOS target under
Rosetta 2. `cargo check -p nagi-kernel --lib --tests --target
x86_64-unknown-linux-gnu --locked` and the release `x86_64-unknown-nagi`
kernel build pass. Public Ubuntu target CI remains authoritative for the real
guest VM path and M17 acceptance.
