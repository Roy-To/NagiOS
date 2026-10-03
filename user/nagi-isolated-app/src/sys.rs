//! Raw Nagi syscalls available to a Supervisor-spawned isolated process
//! (ADR 0043). No allocator, TLS, or device capability is required.

use core::arch::asm;

use nagi_abi::{
    ChannelReceiveResult, ChannelSendRequest, SYS_CHANNEL_SEND, SYS_CHANNEL_TRY_RECEIVE,
    SYS_CHANNEL_WAIT_READABLE, SYS_CONSOLE_WRITE, SYS_PROCESS_EXIT,
};

#[inline(always)]
pub fn syscall(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
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

pub fn write(message: &[u8]) {
    let _ = syscall(
        SYS_CONSOLE_WRITE,
        message.as_ptr() as u64,
        message.len() as u64,
        0,
    );
}

pub fn exit(code: u64) -> ! {
    let _ = syscall(SYS_PROCESS_EXIT, code, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

pub fn send(endpoint: u64, request: &ChannelSendRequest) -> bool {
    syscall(
        SYS_CHANNEL_SEND,
        endpoint,
        request as *const ChannelSendRequest as u64,
        core::mem::size_of::<ChannelSendRequest>() as u64,
    ) == 0
}

pub fn receive(endpoint: u64, message: &mut ChannelReceiveResult) -> bool {
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
