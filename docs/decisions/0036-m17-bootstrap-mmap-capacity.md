# ADR 0036: M17 bootstrap mmap capacity

Status: accepted for the M17 bootstrap
Date: 2026-09-28
Milestone: M17 — Servo Bootstrap

## Context

Public CI run #302 (`36351615434`, head
`fb9ba58f3c0f01b13335a894edd33a1999a92da5`) passed both host jobs and all
target build stages through the UEFI loader. Real QEMU entered Servo's
`ScriptThread debugger global creation` after constructing the Softpipe GL
context and initializing GC chunks, then failed to allocate another GC chunk.
The guest emitted four equivalent diagnostics:

```text
SYS_MEMORY_MAP rejected: no contiguous range request_pages=256 protection=6
free_reservation_slots=32 free_pages=4 largest_free_run_pages=4
```

The request is 1 MiB. The per-process bootstrap mmap allocator therefore had
only 16 KiB free in its entire 128 MiB window; its reservation table still had
32 free identities. The failure is address/backing capacity exhaustion, not
reservation-slot exhaustion, alignment behavior, page-table creation, or a
host mapping fallback. Servo then received a null context, panicked, and QEMU
timed out at the unchanged 120-second acceptance bound. No web-pixel checksum
or M17 PASS marker was produced.

`BootstrapStorage` currently contains one statically backed `PageBytes` entry
per mmap virtual page. Its 128 MiB virtual window consequently commits 128 MiB
of kernel-owned backing storage even before considering the kernel's other
state. The pinned reference QEMU configuration has 8 GiB RAM, and CI shows the
real M17 startup path consumes essentially the full existing window before
the first frame.

## Decision

- Increase the bounded bootstrap process mmap window and its statically backed
  pages from 128 MiB to 256 MiB. This provides another 128 MiB for the already
  observed Servo startup path while keeping the M17 bootstrap finite.
- Keep the 64 live reservation identities, per-page ownership checks,
  page-aligned partial `munmap`/`mprotect`, exact-address remap restrictions,
  and serialized mapping mutation unchanged.
- Continue to use Nagi-owned kernel backing pages and Nagi page tables. Do not
  add a host mapping, weaken ownership validation, or change the real QEMU
  acceptance timeout or pixel conditions.
- Accept the bounded additional 128 MiB kernel BSS cost for the official
  8-GiB QEMU reference machine. Replacing the bootstrap's statically backed
  pages with the general physical-frame VM service is a separate architectural
  evolution; it is not needed to make this measured M17 capacity repair.
- Supersede the 128 MiB bootstrap-window limit in ADRs 0027, 0029, 0031, 0034,
  and 0035 for this M17 configuration. Their thread, heap, descriptor, and
  partial-range decisions remain in force.

## Consequences

- The mmap window now has 65,536 pages and 128 page-table pages. The fixed
  owner ledger grows from 32 KiB to 64 KiB, and the user-process backing store
  grows by 128 MiB.
- M17 can request at least one additional full 128 MiB window's worth of
  mapped/reserved capacity before reaching the same bounded failure. The
  reason-coded rejection statistics continue to identify any later limit.
- Public CI #303 (`36355494134`, head
  `31bf815b7230f2658f654643e6d6c898d9881d77`) loaded the larger kernel and
  passed real-QEMU M17 acceptance. Servo produced a nonzero software-rendered
  frame checksum, copied the frame to Nagi Surface, and presented it before
  printing the M17 PASS marker. M17 is `PASS`; M18 remains `NOT STARTED`.

## Verification

Kernel tests assert the 256 MiB window, check the last mapped page, and reject
ranges beyond the new limit. Local verification passed the kernel unit tests,
the x86-64 Linux kernel-test compilation, the release Nagi kernel build,
formatting, and diff checks. Public CI #303 then loaded the larger kernel BSS
and passed the real M17 QEMU first-web-pixel acceptance.
