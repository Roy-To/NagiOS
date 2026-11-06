// Standalone QEMU virt experiment. No Nagi production boot protocol.
.section .text.entry,"ax"
.global _start
_start:
    msr daifset, #0xf
    mrs x0, mpidr_el1
    and x0, x0, #0xff
    cbnz x0, idle
    mrs x0, CurrentEL
    cmp x0, #4
    b.ne 3f
    msr spsel, #1
3:  ldr x0, =__stack_top
    mov sp, x0
    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:  cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:  bl spike_main
idle:
    dsb sy
    wfi
    b idle

// Architectural 16-slot, 2048-byte-aligned vector table.
.balign 2048
.global spike_vectors
spike_vectors:
.rept 4
    b unexpected_exception
    .balign 128
.endr
    b svc_exception
    .balign 128
.rept 11
    b unexpected_exception
    .balign 128
.endr

svc_exception:
    stp x0, x1, [sp, #-32]!
    stp x2, x3, [sp, #16]
    mrs x0, esr_el1
    lsr x1, x0, #26
    cmp x1, #0x15
    b.ne unexpected_exception
    and x1, x0, #0xffff
    cmp x1, #0x4e
    b.ne unexpected_exception
    adrp x2, exception_seen
    add x2, x2, :lo12:exception_seen
    ldr x3, [x2]
    add x3, x3, #1
    str x3, [x2]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp], #32
    eret
unexpected_exception:
    b spike_fault

.global exercise_svc
exercise_svc:
    svc #0x4e
    ret
