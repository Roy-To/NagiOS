# ADR 0021: Bounded static ELF TLS for the bootstrap process

Status: accepted for Nagi OS 0.1

## Context

Public CI run `36006860116` (#167) reached the real `nagi-init` link with zero
undefined symbols after the libc++ string-growth provider was added. The link
then rejected target-owned Mesa and relibc objects containing ELF TLS symbols
because the Nagi user linker script emitted no `PT_TLS` program header.

M5 already reserves a fixed user TLS mapping and installs an x86-64 FS base,
but its ELF loader did not read a TLS template. Servo's statically linked
Mesa/relibc graph now needs real initial-exec and local-exec TLS, including
initialized data such as Mesa's initial GL dispatch pointer.

## Decision

- Support at most one `PT_TLS` segment in the statically linked bootstrap
  executable. Its initialized and zero-fill data must fit in one 4 KiB page,
  and `p_align` must be 0, 1, or a power of two no larger than 4 KiB; `p_vaddr`
  and `p_offset` must be congruent modulo that alignment.
- Use the x86-64 variant-II layout for the bounded bootstrap's two native
  thread slots (the initial thread and one child): each slot has one static TLS
  data page immediately before one FS-base/control page. Place the TLS template
  at the end of each data page, respecting `p_align`, copy `p_filesz` bytes
  from the validated init image, and leave the rest zeroed. Set each FS base to
  its control page and initialize its first word to that thread-pointer address,
  which Nagi's x86-64 TLS access sequence reads from `FS:0`.
- Keep a kernel-only copy of the initial TLS page. Before reusing the child
  slot, restore its data and control pages from the initial template so one
  child's TLS mutations do not leak into the next child. Save the active FS
  base in the kernel thread context and restore it on each context switch.
- Emit `.tdata` and `.tbss` in both the `PT_LOAD` data mapping and the `PT_TLS`
  template description. The user ELF parser validates the unique template,
  its file bounds, alignment, address range, and containment in a loadable
  segment before the kernel copies it.
- Reject duplicate, malformed, misaligned, or oversized TLS templates. Do not
  add dynamic TLS modules or a general dynamic-loader namespace; existing
  `dlopen` behavior remains fail-closed.

This keeps TLS within the kernel's existing bounded, per-bootstrap address
space. The user mapping reserves four pages for two isolated thread slots; the
kernel keeps one additional page for the immutable initial template. No host
TLS runtime, host library, or host process state is involved. M5's single
thread mapping is extended to two pages per slot, with FS base at the beginning
of the control page following the static TLS data, as required by the x86-64
layout.

## Verification

The parser and address-space tests cover ELF validation, template placement,
initialized bytes, zero-fill, both thread slots, and resetting the child slot.
Ubuntu target CI remains the authority for the real Servo/Mesa link and QEMU
first-web-pixel acceptance.
