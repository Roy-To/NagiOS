ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
    bss    PT_LOAD FLAGS(6);
    tls    PT_TLS FLAGS(4);
}

SECTIONS
{
    . = 0x0000400000000000;

    .text ALIGN(4096) :
    {
        KEEP(*(.text._start))
        *(.text .text.* .ltext*)
    } :text

    .rodata ALIGN(4096) :
    {
        *(.rodata .rodata.* .lrodata*)
    } :rodata

    /* Static TLS is part of the data load mapping and has its own template
     * header. The kernel copies PT_TLS into the bounded per-process TLS area. */
    .tdata ALIGN(4096) :
    {
        *(.tdata .tdata.* .gnu.linkonce.td.*)
    } :data :tls

    /* If there is no initialized TLS section, keep a BSS-only data PT_LOAD
     * page aligned. Otherwise the input TLS sections supply their alignment. */
    . = ALIGN(SIZEOF(.tdata) == 0 ? 4096 : 1);
    .tbss (NOLOAD) :
    {
        *(.tbss .tbss.* .gnu.linkonce.tb.*)
    } :data :tls

    .data ALIGN(4096) :
    {
        *(.data .data.* .ldata*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.*)
        *(.ldata .ldata.*)
    } :data

    /* The user entry is a Nagi-specific capability ABI rather than a CRT
     * entrypoint. Keep the ELF constructor arrays so _start can run process
     * initialization before entering application code. */
    .preinit_array ALIGN(8) :
    {
        PROVIDE_HIDDEN(__preinit_array_start = .);
        KEEP(*(SORT_BY_INIT_PRIORITY(.preinit_array.*)))
        KEEP(*(.preinit_array))
        PROVIDE_HIDDEN(__preinit_array_end = .);
    } :data

    .init_array ALIGN(8) :
    {
        PROVIDE_HIDDEN(__init_array_start = .);
        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*)))
        KEEP(*(.init_array))
        PROVIDE_HIDDEN(__init_array_end = .);
    } :data

    .bss ALIGN(4096) (NOLOAD) :
    {
        *(.bss .bss.* .lbss*)
        *(COMMON)
        *(.lbss .lbss.*)
    } :bss

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
        *(.note .note.*)
    }
}
