ENTRY(_start)

PHDRS {
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    /* Stay above the reference OVMF's reservations observed in ADR 0023. */
    . = 0x04000000;
    __nagi_kernel_start = .;

    .text ALIGN(0x1000) : {
        KEEP(*(.text.entry))
        *(.text*)
    } :text

    .rodata ALIGN(0x1000) : {
        *(.rodata*)
    } :rodata

    .data ALIGN(0x1000) : {
        *(.data*)
    } :data

    .bss ALIGN(0x1000) : {
        *(.bss*)
        *(COMMON)
    } :data

    __nagi_kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment*)
    }
}
