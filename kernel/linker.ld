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

    /* Link-time fields of the entry GDT/IDT in kernel/src/main.rs (ADR 0051).
       x86 descriptors split addresses into pieces that the assembler cannot
       relocate, so the linker computes them here. */
    __nagi_early_tss_base_0_15 = nagi_early_tss & 0xFFFF;
    __nagi_early_tss_base_16_23 = (nagi_early_tss >> 16) & 0xFF;
    __nagi_early_tss_base_24_31 = (nagi_early_tss >> 24) & 0xFF;
    __nagi_early_tss_base_32_63 = nagi_early_tss >> 32;
    __nagi_early_nmi_0_15 = nagi_fault_stub_2 & 0xFFFF;
    __nagi_early_nmi_16_31 = (nagi_fault_stub_2 >> 16) & 0xFFFF;
    __nagi_early_nmi_32_63 = nagi_fault_stub_2 >> 32;
    __nagi_early_df_0_15 = nagi_fault_stub_8 & 0xFFFF;
    __nagi_early_df_16_31 = (nagi_fault_stub_8 >> 16) & 0xFFFF;
    __nagi_early_df_32_63 = nagi_fault_stub_8 >> 32;
    __nagi_early_mc_0_15 = nagi_fault_stub_18 & 0xFFFF;
    __nagi_early_mc_16_31 = (nagi_fault_stub_18 >> 16) & 0xFFFF;
    __nagi_early_mc_32_63 = nagi_fault_stub_18 >> 32;

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment*)
    }
}
