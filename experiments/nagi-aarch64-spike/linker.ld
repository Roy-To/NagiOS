ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(5); rodata PT_LOAD FLAGS(4); data PT_LOAD FLAGS(6); }
SECTIONS {
    . = 0x40080000;
    .text : { KEEP(*(.text.entry)) *(.text*) } :text
    .rodata ALIGN(4096) : { *(.rodata*) } :rodata
    .data ALIGN(4096) : { *(.data*) } :data
    .bss ALIGN(4096) (NOLOAD) : {
        __bss_start = .;
        *(.bss*) *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
        . += 16384;
        __stack_top = .;
    } :data
    /DISCARD/ : { *(.eh_frame*) *(.comment*) }
}
