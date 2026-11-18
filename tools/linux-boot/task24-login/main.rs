#![no_std]
#![no_main]

mod input;
#[path = "../../../user/nagi-init/src/linux_initial_login_sender.rs"]
mod linux_initial_login_sender;

use core::arch::{asm, global_asm};
use core::ffi::CStr;
use core::panic::PanicInfo;
use linux_initial_login_sender::{broker_path, send_initial_login};

const CONTROL_FD: usize = 3;

global_asm!(
    ".global _start",
    "_start:",
    "mov rdi, rsp",
    "and rsp, -16",
    "call task24_start",
);

unsafe fn syscall(number: i64, a: usize, b: usize, c: usize, d: usize) -> isize {
    let result;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            in("r10") d,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result
}

fn exit(status: usize) -> ! {
    unsafe { syscall(libc::SYS_exit_group, status, 0, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    exit(69)
}

#[no_mangle]
extern "C" fn task24_start(stack: *const usize) -> ! {
    // Linux supplies argc followed by its NUL-terminated argv strings. C3
    // supplies the executable name and the sole C2 locator.
    let ok = unsafe { *stack == 2 && run(CStr::from_ptr(*stack.add(2) as *const i8).to_bytes()) };
    exit(if ok { 0 } else { 69 })
}

fn run(argument: &[u8]) -> bool {
    unsafe {
        if syscall(libc::SYS_prctl, libc::PR_SET_DUMPABLE as usize, 0, 0, 0) != 0 {
            return false;
        }
    }

    let mut address: libc::sockaddr_un = unsafe { core::mem::zeroed() };
    let mut size = core::mem::size_of_val(&address) as libc::socklen_t;
    if unsafe {
        syscall(
            libc::SYS_getpeername,
            CONTROL_FD,
            (&mut address as *mut libc::sockaddr_un) as usize,
            (&mut size as *mut libc::socklen_t) as usize,
            0,
        )
    } != 0
        || address.sun_family != libc::AF_UNIX as libc::sa_family_t
    {
        return false;
    }

    let Some(path) = broker_path(argument) else {
        return false;
    };
    address.sun_path.fill(0);
    for (target, byte) in address.sun_path.iter_mut().zip(path) {
        *target = *byte as libc::c_char;
    }
    let broker = unsafe {
        syscall(
            libc::SYS_socket,
            libc::AF_UNIX as usize,
            (libc::SOCK_STREAM | libc::SOCK_CLOEXEC) as usize,
            0,
            0,
        )
    };
    if broker < 0
        || unsafe {
            syscall(
                libc::SYS_connect,
                broker as usize,
                (&address as *const libc::sockaddr_un) as usize,
                2 + path.len() + 1,
                0,
            )
        } != 0
    {
        return false;
    }

    if unsafe {
        syscall(
            libc::SYS_write,
            CONTROL_FD,
            &input::CONNECTED as *const u8 as usize,
            1,
            0,
        )
    } != 1
    {
        return false;
    }

    // No bytes go to C2 until the owner submits at the trusted boot UI.
    let Some(submitted) = input::receive(read_control_exact) else {
        return false;
    };
    let sent = send_initial_login(
        submitted.route.destination,
        submitted.route.request,
        submitted.account(),
        submitted.password(),
        |mut frame| {
            while !frame.is_empty() {
                let result = unsafe {
                    syscall(
                        libc::SYS_sendto,
                        broker as usize,
                        frame.as_ptr() as usize,
                        frame.len(),
                        (libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) as usize,
                    )
                };
                if result == -(libc::EINTR as isize) {
                    continue;
                }
                if result <= 0 {
                    return false;
                }
                frame = &frame[result as usize..];
            }
            true
        },
    );
    drop(submitted);

    // Keep the child and its stream live until PID 1 closes the control
    // channel after C2's post-read peer checks and the C1 transaction.
    sent && read_control(&mut [0]) == 0
}

fn read_control(buffer: &mut [u8]) -> isize {
    loop {
        let result = unsafe {
            syscall(
                libc::SYS_read,
                CONTROL_FD,
                buffer.as_mut_ptr() as usize,
                buffer.len(),
                0,
            )
        };
        if result != -(libc::EINTR as isize) {
            return result;
        }
    }
}

fn read_control_exact(mut buffer: &mut [u8]) -> bool {
    while !buffer.is_empty() {
        let result = read_control(buffer);
        if result <= 0 {
            return false;
        }
        buffer = &mut buffer[result as usize..];
    }
    true
}
