//! Isolated application used by the ADR 0043 Supervisor acceptance.
//!
//! The kernel starts this ELF in its own address space with the moved
//! Channel endpoint in `rdi` and its kernel Process ID in `rsi`. The app:
//!
//! 1. probes that init-only memory and privileged syscalls are unreachable;
//! 2. sends a request whose payload falsely claims to be the system app; and
//! 3. reports which checks held before exiting.
//!
//! The Supervisor must decide from the kernel-stamped sender ID, not from the
//! payload.
#![cfg_attr(target_os = "nagi", no_std)]
#![cfg_attr(target_os = "nagi", no_main)]

#[cfg(target_os = "nagi")]
mod app {
    use core::arch::asm;
    use core::panic::PanicInfo;

    use nagi_abi::{
        ChannelReceiveResult, ChannelSendRequest, SYS_BLOCK_READ, SYS_CHANNEL_SEND,
        SYS_CHANNEL_TRY_RECEIVE, SYS_CHANNEL_WAIT_READABLE, SYS_CONSOLE_WRITE, SYS_MEMORY_MAP,
        SYS_PROCESS_EXIT, SYS_PROCESS_INFO, SYS_PROCESS_SPAWN, SYS_THREAD_CREATE,
    };

    // Keep in sync with `user/nagi-init/src/isolated_process.rs`.
    const PROTOCOL_ID: u16 = 0x4f43;
    const PROTOCOL_VERSION: u16 = 1;
    const OPCODE_REQUEST: u16 = 1;
    const OPCODE_REPLY: u16 = 2;
    const OPCODE_REPORT: u16 = 3;
    const FORGED_CLAIM: &[u8] = b"caller=org.nagi.system;pid=1";
    const INIT_PROCESS_ID: u32 = 1;
    const DECISION_DENY: u8 = 0;

    // Init-only user windows from `kernel/src/user_process.rs`: the bootstrap
    // TLS area and the bounded mmap window. Neither exists in this process.
    const INIT_TLS_ADDRESS: u64 = 0x0000_4000_2040_0000;
    const INIT_MMAP_ADDRESS: u64 = 0x0000_4000_2080_0000;

    const CHECK_STARTED_AS_CHILD: u32 = 1 << 0;
    const CHECK_INIT_TLS_UNMAPPED: u32 = 1 << 1;
    const CHECK_INIT_MMAP_UNMAPPED: u32 = 1 << 2;
    const CHECK_BLOCK_DENIED: u32 = 1 << 3;
    const CHECK_MEMORY_MAP_DENIED: u32 = 1 << 4;
    const CHECK_THREAD_CREATE_DENIED: u32 = 1 << 5;
    const CHECK_SPAWN_DENIED: u32 = 1 << 6;
    const CHECK_PROCESS_INFO_DENIED: u32 = 1 << 7;
    const CHECK_REPLY_FROM_INIT: u32 = 1 << 8;
    const CHECK_REPLY_NAMES_THIS_PID: u32 = 1 << 9;
    const CHECK_FORGED_CLAIM_DENIED: u32 = 1 << 10;

    #[inline(always)]
    fn syscall(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
        let mut result = number;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") result,
                in("rdi") arg1,
                in("rsi") arg2,
                in("rdx") arg3,
                in("r10") 0_u64,
                in("r8") 0_u64,
                in("r9") 0_u64,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        result
    }

    fn write(message: &[u8]) {
        let _ = syscall(
            SYS_CONSOLE_WRITE,
            message.as_ptr() as u64,
            message.len() as u64,
            0,
        );
    }

    fn exit(code: u64) -> ! {
        let _ = syscall(SYS_PROCESS_EXIT, code, 0, 0);
        loop {
            core::hint::spin_loop();
        }
    }

    fn send(endpoint: u64, opcode: u16, payload: &[u8]) -> bool {
        let mut request = ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 1, opcode);
        request.payload_len = payload.len() as u32;
        request.payload[..payload.len()].copy_from_slice(payload);
        syscall(
            SYS_CHANNEL_SEND,
            endpoint,
            &request as *const ChannelSendRequest as u64,
            core::mem::size_of::<ChannelSendRequest>() as u64,
        ) == 0
    }

    fn receive(endpoint: u64, message: &mut ChannelReceiveResult) -> bool {
        loop {
            match syscall(
                SYS_CHANNEL_TRY_RECEIVE,
                endpoint,
                message as *mut ChannelReceiveResult as u64,
                core::mem::size_of::<ChannelReceiveResult>() as u64,
            ) {
                1 => return true,
                0 if syscall(SYS_CHANNEL_WAIT_READABLE, endpoint, 0, 0) == 0 => {}
                _ => return false,
            }
        }
    }

    fn rejected(result: u64) -> bool {
        result == u64::MAX
    }

    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, process_id: u64) -> ! {
        write(b"Nagi isolated app started\r\n");
        let mut checks = 0_u32;
        if process_id > u64::from(INIT_PROCESS_ID) && process_id <= u64::from(u32::MAX) {
            checks |= CHECK_STARTED_AS_CHILD;
        }
        // The kernel validates every user pointer against this process's own
        // page tables; init's windows must look unmapped.
        if rejected(syscall(SYS_CONSOLE_WRITE, INIT_TLS_ADDRESS, 8, 0)) {
            checks |= CHECK_INIT_TLS_UNMAPPED;
        }
        if rejected(syscall(SYS_CONSOLE_WRITE, INIT_MMAP_ADDRESS, 8, 0)) {
            checks |= CHECK_INIT_MMAP_UNMAPPED;
        }
        let mut sector = [0_u8; 512];
        if rejected(syscall(SYS_BLOCK_READ, 1, 0, sector.as_mut_ptr() as u64)) {
            checks |= CHECK_BLOCK_DENIED;
        }
        if rejected(syscall(SYS_MEMORY_MAP, 4096, 3, 0)) {
            checks |= CHECK_MEMORY_MAP_DENIED;
        }
        if rejected(syscall(SYS_THREAD_CREATE, 0, 0, 0)) {
            checks |= CHECK_THREAD_CREATE_DENIED;
        }
        if rejected(syscall(SYS_PROCESS_SPAWN, 0, 0, 0)) {
            checks |= CHECK_SPAWN_DENIED;
        }
        if rejected(syscall(SYS_PROCESS_INFO, sector.as_mut_ptr() as u64, 64, 0)) {
            checks |= CHECK_PROCESS_INFO_DENIED;
        }

        if !send(endpoint, OPCODE_REQUEST, FORGED_CLAIM) {
            write(b"Nagi isolated app request send FAIL\r\n");
            exit(1);
        }
        let mut reply = ChannelReceiveResult::default();
        if !receive(endpoint, &mut reply) || reply.opcode != OPCODE_REPLY || reply.payload_len < 5 {
            write(b"Nagi isolated app reply FAIL\r\n");
            exit(1);
        }
        if reply.sender_process_id == INIT_PROCESS_ID {
            checks |= CHECK_REPLY_FROM_INIT;
        }
        let resolved = u32::from_le_bytes([
            reply.payload[0],
            reply.payload[1],
            reply.payload[2],
            reply.payload[3],
        ]);
        if u64::from(resolved) == process_id {
            checks |= CHECK_REPLY_NAMES_THIS_PID;
        }
        if reply.payload[4] == DECISION_DENY {
            checks |= CHECK_FORGED_CLAIM_DENIED;
        }

        if !send(endpoint, OPCODE_REPORT, &checks.to_le_bytes()) {
            write(b"Nagi isolated app report send FAIL\r\n");
            exit(1);
        }
        write(b"Nagi isolated app report sent\r\n");
        exit(0)
    }

    #[panic_handler]
    fn panic(_info: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
