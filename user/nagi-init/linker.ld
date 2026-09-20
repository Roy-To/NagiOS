ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data   PT_LOAD FLAGS(6);
    bss    PT_LOAD FLAGS(6);
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

    .data ALIGN(4096) :
    {
        *(.data .data.* .ldata*)
        *(.data.rel.ro .data.rel.ro.*)
        *(.got .got.*)
        *(.ldata .ldata.*)
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
