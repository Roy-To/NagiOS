//! Deliberately faulting isolated application for ADR 0047 fault
//! containment.
//!
//! The Supervisor sends one launch argument naming the CPU exception to
//! raise. The kernel must terminate only this process and resume the system.
#![cfg_attr(target_os = "nagi", no_std)]
#![cfg_attr(target_os = "nagi", no_main)]

#[cfg(target_os = "nagi")]
#[path = "../sys.rs"]
mod sys;

#[cfg(target_os = "nagi")]
mod app {
    use core::arch::asm;
    use core::panic::PanicInfo;

    use nagi_abi::ChannelReceiveResult;

    use crate::sys::{exit, receive, write};

    // Keep in sync with `user/nagi-init/src/isolated_process.rs`.
    const PROTOCOL_ID: u16 = 0x4643;
    const OPCODE_LAUNCH_FAULT: u16 = 1;
    const FAULT_PAGE: u8 = 1;
    const FAULT_INVALID_OPCODE: u8 = 2;
    const FAULT_GENERAL_PROTECTION: u8 = 3;
    /// Init's TLS window; never mapped in an isolated process.
    const UNMAPPED_ADDRESS: u64 = 0x0000_4000_2040_0000;

    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, _process_id: u64) -> ! {
        let mut launch = ChannelReceiveResult::default();
        if !receive(endpoint, &mut launch)
            || launch.protocol_id != PROTOCOL_ID
            || launch.opcode != OPCODE_LAUNCH_FAULT
            || launch.payload_len != 1
        {
            exit(1);
        }
        write(b"Nagi faulting app raising exception\r\n");
        match launch.payload[0] {
            FAULT_PAGE => unsafe {
                (UNMAPPED_ADDRESS as *mut u64).write_volatile(0x4e41_4749);
            },
            FAULT_INVALID_OPCODE => unsafe { asm!("ud2", options(nomem, nostack)) },
            // HLT is privileged: CPL 3 raises #GP.
            FAULT_GENERAL_PROTECTION => unsafe { asm!("hlt", options(nomem, nostack)) },
            _ => {}
        }
        // Reaching here means the exception was not raised or not contained.
        write(b"Nagi faulting app survived its fault FAIL\r\n");
        exit(2)
    }

    #[panic_handler]
    fn panic(_info: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
