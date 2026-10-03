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
        exception_has_error_code, fault_exit_code, interrupt_gate_options, tss_descriptor,
        TaskStateSegment, TSS_SIZE,
    };

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
