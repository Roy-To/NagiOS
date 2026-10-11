#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use core::ptr::{addr_of, read_volatile, write_volatile};

global_asm!(include_str!("entry.S"));
const PL011: usize = 0x0900_0000;
const POLL_LIMIT: usize = 1_000_000;

#[no_mangle]
static mut exception_seen: u64 = 0;
extern "C" {
    static spike_vectors: u8;
    fn exercise_svc();
}

fn serial(bytes: &[u8]) {
    for byte in bytes {
        let mut ready = false;
        for _ in 0..POLL_LIMIT {
            // SAFETY: fixed QEMU virt PL011 MMIO, translation disabled.
            if unsafe { read_volatile((PL011 + 0x18) as *const u32) } & (1 << 5) == 0 {
                ready = true;
                break;
            }
        }
        if !ready {
            exit(1);
        }
        unsafe {
            write_volatile(PL011 as *mut u32, u32::from(*byte));
        }
    }
}

fn exit(code: u64) -> ! {
    let block = [0x20026u64, code]; // SYS_EXIT_EXTENDED, ADP_Stopped_ApplicationExit
    unsafe {
        asm!("hlt #0xf000", in("x0") 0x20u64, in("x1") block.as_ptr(), options(nostack));
    }
    // Safe single-core fallback if the development emulator does not terminate.
    loop {
        unsafe {
            asm!("dsb sy", "wfi", options(nostack));
        }
    }
}

#[no_mangle]
extern "C" fn spike_main() -> ! {
    let el: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) el, options(nomem, nostack));
    }
    if el != 4 {
        serial(b"SPIKE FAIL exception-level\n");
        exit(1);
    }
    let control: u64;
    unsafe {
        asm!("mrs {}, sctlr_el1", out(reg) control, options(nomem, nostack));
    }
    if control & 1 != 0 {
        serial(b"SPIKE FAIL unexpected-mmu\n");
        exit(1);
    }
    // Entry selected SP_EL1 before establishing the stack. Install vectors now.
    unsafe {
        asm!("msr vbar_el1, {}", "isb", in(reg) addr_of!(spike_vectors), options(nostack));
    }
    serial(b"SPIKE ARM64 EL1 PL011 PASS\n");
    unsafe {
        exercise_svc();
    }
    if unsafe { read_volatile(addr_of!(exception_seen)) } != 1 {
        serial(b"SPIKE FAIL svc-count\n");
        exit(1);
    }
    serial(b"SPIKE SVC VECTOR ERET PASS\n");
    let frequency: u64;
    let start: u64;
    unsafe {
        asm!("mrs {}, cntfrq_el0", "mrs {}, cntpct_el0", out(reg) frequency, out(reg) start, options(nomem, nostack));
    }
    let mut advanced = false;
    for _ in 0..POLL_LIMIT {
        let now: u64;
        unsafe {
            asm!("isb", "mrs {}, cntpct_el0", out(reg) now, options(nomem, nostack));
        }
        if now > start {
            advanced = true;
            break;
        }
    }
    if frequency == 0 || !advanced {
        serial(b"SPIKE FAIL counter\n");
        exit(1);
    }
    serial(b"SPIKE GENERIC COUNTER PASS\nSPIKE COMPLETE PASS\n");
    exit(0)
}

#[no_mangle]
extern "C" fn spike_fault() -> ! {
    serial(b"SPIKE FAIL unexpected-exception\n");
    exit(1)
}
#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    serial(b"SPIKE FAIL panic\n");
    exit(1)
}
