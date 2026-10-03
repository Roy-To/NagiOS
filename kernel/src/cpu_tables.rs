//! x86-64 Task State Segment and its GDT system descriptor (ADR 0047).
//!
//! Ring-3 code needs a TSS so that a CPU exception raised in user mode
//! switches to a known kernel stack (RSP0). Without one, the exception can
//! push nowhere valid and the CPU triple-faults. The bootstrap therefore
//! could not survive any fault in an isolated process.

/// 64-bit TSS (Intel SDM Vol. 3A, Figure 8-11).
#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
pub struct TaskStateSegment {
    reserved0: u32,
    /// Stack pointers loaded on a privilege change to rings 0-2.
    pub rsp: [u64; 3],
    reserved1: u64,
    /// Interrupt Stack Table entries 1-7 (index 0 is IST1).
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    pub iomap_base: u16,
}

pub const TSS_SIZE: usize = core::mem::size_of::<TaskStateSegment>();
const _: () = assert!(TSS_SIZE == 104);

impl TaskStateSegment {
    /// A TSS with RSP0 and IST1 set and no I/O permission bitmap. An I/O map
    /// base at or beyond the limit denies all user port access.
    pub const fn new(rsp0: u64, ist1: u64) -> Self {
        Self {
            reserved0: 0,
            rsp: [rsp0, 0, 0],
            reserved1: 0,
            ist: [ist1, 0, 0, 0, 0, 0, 0],
            reserved2: 0,
            reserved3: 0,
            iomap_base: TSS_SIZE as u16,
        }
    }

    /// A TSS with RSP0 and IST1..=IST3 set to `ist_tops` (see
    /// [`exception_ist`]) and no I/O permission bitmap.
    pub const fn with_ist_stacks(rsp0: u64, ist_tops: [u64; IST_STACK_COUNT]) -> Self {
        let mut tss = Self::new(rsp0, 0);
        let mut index = 0;
        while index < IST_STACK_COUNT {
            tss.ist[index] = ist_tops[index];
            index += 1;
        }
        tss
    }
}

/// Encode the 16-byte available 64-bit TSS descriptor (type 9, DPL 0,
/// present) for `base`. Returns the two GDT words, low then high.
pub const fn tss_descriptor(base: u64) -> [u64; 2] {
    let limit = (TSS_SIZE - 1) as u64;
    let low = (limit & 0xffff)
        | ((base & 0x00ff_ffff) << 16)
        | (0x89 << 40)
        | (((limit >> 16) & 0xf) << 48)
        | (((base >> 24) & 0xff) << 56);
    [low, base >> 32]
}

/// Interrupt Stack Table slots used by Nagi. Each exception that may arrive
/// while the current kernel stack is unusable gets its own known-good stack:
/// #DF after a stack fault (ADR 0047/0048), and NMI and #MC, which are
/// asynchronous and can interrupt any instruction (ADR 0049).
pub const DOUBLE_FAULT_IST: u8 = 1;
pub const NMI_IST: u8 = 2;
pub const MACHINE_CHECK_IST: u8 = 3;
/// Number of IST stacks each CPU provides (IST1..=IST3).
pub const IST_STACK_COUNT: usize = 3;

/// IST index for an exception vector's gate, or 0 to stay on the current
/// stack.
pub const fn exception_ist(vector: u8) -> u8 {
    match vector {
        2 => NMI_IST,
        8 => DOUBLE_FAULT_IST,
        18 => MACHINE_CHECK_IST,
        _ => 0,
    }
}

/// 64-bit kernel code and flat kernel data descriptors, as at selectors 0x08
/// and 0x10 of the BSP's M5 GDT.
pub const KERNEL_CODE_DESCRIPTOR: u64 = 0x00af_9a00_0000_ffff;
pub const KERNEL_DATA_DESCRIPTOR: u64 = 0x00cf_9200_0000_ffff;

/// Kernel-only GDT for one application processor. It keeps the BSP's
/// selector layout (kernel code 0x08, kernel data 0x10, TSS 0x28) so the
/// same gate selector and `ltr` value work on every CPU. The user code and
/// data slots stay null: APs never enter ring 3, so a stray user selector
/// faults instead of loading.
pub const fn ap_gdt(tss_base: u64) -> [u64; 7] {
    let [tss_low, tss_high] = tss_descriptor(tss_base);
    [
        0,
        KERNEL_CODE_DESCRIPTOR,
        KERNEL_DATA_DESCRIPTOR,
        0,
        0,
        tss_low,
        tss_high,
    ]
}

/// Interrupt-gate options word: present, DPL 0, 64-bit interrupt gate, with
/// an optional IST index (1-7).
pub const fn interrupt_gate_options(ist: u8) -> u16 {
    0x8E00 | (ist as u16 & 0x7)
}

/// CPU exceptions that push an error code (Intel SDM Vol. 3A, Table 6-1).
pub const fn exception_has_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30)
}

/// Exit code reported for an isolated process terminated by `vector`,
/// following the POSIX convention of 128 + signal-like number.
pub const fn fault_exit_code(vector: u64) -> u64 {
    128 + vector
}

#[cfg(test)]
mod tests {
    use super::{
        ap_gdt, exception_has_error_code, exception_ist, fault_exit_code, interrupt_gate_options,
        tss_descriptor, TaskStateSegment as Tss, TaskStateSegment, DOUBLE_FAULT_IST,
        IST_STACK_COUNT, KERNEL_CODE_DESCRIPTOR, KERNEL_DATA_DESCRIPTOR, MACHINE_CHECK_IST,
        NMI_IST, TSS_SIZE,
    };

    #[test]
    fn nmi_double_fault_and_machine_check_use_distinct_ist_slots() {
        assert_eq!(exception_ist(2), NMI_IST);
        assert_eq!(exception_ist(8), DOUBLE_FAULT_IST);
        assert_eq!(exception_ist(18), MACHINE_CHECK_IST);
        for vector in (0..32).filter(|vector| ![2, 8, 18].contains(vector)) {
            assert_eq!(exception_ist(vector), 0, "vector {vector}");
        }
        let slots = [NMI_IST, DOUBLE_FAULT_IST, MACHINE_CHECK_IST];
        for slot in slots {
            assert!((1..=IST_STACK_COUNT as u8).contains(&slot));
            assert_eq!(slots.iter().filter(|other| **other == slot).count(), 1);
        }
        let tss = Tss::with_ist_stacks(0x1000, [0x2000, 0x3000, 0x4000]);
        let (rsp0, ist) = (tss.rsp[0], tss.ist);
        assert_eq!(rsp0, 0x1000);
        assert_eq!(ist, [0x2000, 0x3000, 0x4000, 0, 0, 0, 0]);
    }

    #[test]
    fn ap_gdt_has_kernel_segments_and_tss_at_bsp_selectors() {
        let gdt = ap_gdt(0x0400_1230);
        assert_eq!(gdt[0], 0, "null descriptor");
        assert_eq!(gdt[0x08 / 8], KERNEL_CODE_DESCRIPTOR);
        assert_eq!(gdt[0x10 / 8], KERNEL_DATA_DESCRIPTOR);
        assert_eq!(gdt[3], 0, "no user data segment on APs");
        assert_eq!(gdt[4], 0, "no user code segment on APs");
        assert_eq!(
            [gdt[0x28 / 8], gdt[0x28 / 8 + 1]],
            tss_descriptor(0x0400_1230)
        );
        // Long-mode code: L=1, D=0, present, DPL 0, execute/read.
        assert_eq!((KERNEL_CODE_DESCRIPTOR >> 53) & 1, 1);
        assert_eq!((KERNEL_CODE_DESCRIPTOR >> 54) & 1, 0);
        assert_eq!((KERNEL_CODE_DESCRIPTOR >> 40) & 0xff, 0x9a);
        assert_eq!((KERNEL_DATA_DESCRIPTOR >> 40) & 0xff, 0x92);
    }

    #[test]
    fn tss_layout_matches_the_architecture() {
        assert_eq!(TSS_SIZE, 104);
        let tss = TaskStateSegment::new(0x1111_2222_3333_4440, 0x5555_6666_7777_8880);
        let rsp0 = tss.rsp[0];
        let ist1 = tss.ist[0];
        let iomap = tss.iomap_base;
        assert_eq!(rsp0, 0x1111_2222_3333_4440);
        assert_eq!(ist1, 0x5555_6666_7777_8880);
        assert_eq!(iomap, 104);
        let base = core::ptr::addr_of!(tss) as usize;
        assert_eq!(core::ptr::addr_of!(tss.rsp) as usize - base, 4);
        assert_eq!(core::ptr::addr_of!(tss.ist) as usize - base, 36);
        assert_eq!(core::ptr::addr_of!(tss.iomap_base) as usize - base, 102);
    }

    #[test]
    fn tss_descriptor_encodes_base_limit_and_type() {
        let [low, high] = tss_descriptor(0xffff_8000_1234_5678);
        assert_eq!(low & 0xffff, 103, "limit[15:0]");
        assert_eq!((low >> 16) & 0xff_ffff, 0x34_5678, "base[23:0]");
        assert_eq!(
            (low >> 40) & 0xff,
            0x89,
            "present, DPL0, available 64-bit TSS"
        );
        assert_eq!((low >> 48) & 0xf, 0, "limit[19:16]");
        assert_eq!((low >> 56) & 0xff, 0x12, "base[31:24]");
        assert_eq!(high, 0xffff_8000, "base[63:32]");
    }

    #[test]
    fn gates_and_error_code_vectors_follow_the_sdm() {
        assert_eq!(interrupt_gate_options(0), 0x8E00);
        assert_eq!(interrupt_gate_options(1), 0x8E01);
        for vector in [8, 10, 11, 12, 13, 14, 17, 21, 29, 30] {
            assert!(exception_has_error_code(vector));
        }
        for vector in [0, 1, 3, 4, 5, 6, 7, 9, 16, 18, 19, 20, 31] {
            assert!(!exception_has_error_code(vector));
        }
        assert_eq!(fault_exit_code(14), 142);
    }
}
