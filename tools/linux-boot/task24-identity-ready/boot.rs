//! Task24 development initramfs entry that hands verified-input candidates
//! to the ordinary nagi-init. PID 1 still validates both signed artifacts.
#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;

global_asm!(
    ".global _start",
    "_start:",
    "mov rdi, rsp",
    "and rsp, -16",
    "call task24_identity_boot",
);

fn syscall(number: i64, a: usize, b: usize, c: usize) -> isize {
    let result;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") a, in("rsi") b, in("rdx") c,
            in("r10") 0usize, in("r8") 0usize, in("r9") 0usize,
            lateout("rcx") _, lateout("r11") _, options(nostack),
        );
    }
    result
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    stop()
}

fn stop() -> ! {
    let message = b"NAGI_DEV_TASK24_BOOT_INPUT_HANDOFF_FAILED\n";
    syscall(libc::SYS_write, 2, message.as_ptr() as usize, message.len());
    loop {
        syscall(libc::SYS_pause, 0, 0, 0);
    }
}

fn handoff(path: &[u8], flags: i32, destination: usize) -> bool {
    let fd = syscall(
        libc::SYS_open,
        path.as_ptr() as usize,
        (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | flags) as usize,
        0,
    );
    if fd < 0 {
        return false;
    }
    // If open selected fd 3, 4, or 5, stage it above those slots before
    // clearing close-on-exec with dup3.
    let staged = syscall(
        libc::SYS_fcntl,
        fd as usize,
        libc::F_DUPFD_CLOEXEC as usize,
        6,
    );
    syscall(libc::SYS_close, fd as usize, 0, 0);
    if staged < 0 {
        return false;
    }
    let installed =
        syscall(libc::SYS_dup3, staged as usize, destination, 0) == destination as isize;
    syscall(libc::SYS_close, staged as usize, 0, 0);
    installed
}

#[no_mangle]
extern "C" fn task24_identity_boot(stack: *const usize) -> ! {
    if syscall(libc::SYS_getpid, 0, 0, 0) != 1
        || !handoff(b"/package-store\0", libc::O_DIRECTORY, 3)
        || !handoff(b"/catalog.bin\0", 0, 4)
        || !handoff(b"/identity-ready-plan.bin\0", 0, 5)
    {
        stop();
    }
    let init = b"/nagi-init\0";
    let argv = [init.as_ptr() as usize, 0];
    // SAFETY: Linux supplies argc followed by argv, its null terminator,
    // then envp at process entry. execve receives the original envp.
    let envp = unsafe { stack.add(*stack + 2) };
    syscall(
        libc::SYS_execve,
        init.as_ptr() as usize,
        argv.as_ptr() as usize,
        envp as usize,
    );
    stop()
}
