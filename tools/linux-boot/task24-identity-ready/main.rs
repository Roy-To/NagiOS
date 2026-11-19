//! Signed development Identity process with separate C2 and C3 connections.
#![no_std]
#![no_main]

#[path = "../../../user/nagi-init/src/linux_task24_identity_readiness_wire.rs"]
mod wire;

use core::arch::{asm, global_asm};
use core::ffi::CStr;
use core::panic::PanicInfo;
use nagi_sdk::{app_contract::IpcEnvelope, AppId};

const CONTROL_FD: usize = 3;
const IDENTITY_APP: AppId = AppId::from_identifier(b"org.nagi.dev.task24-identity");
const NIPC_HEADER: usize = 78;
const REQUEST_FRAME: usize = NIPC_HEADER + wire::REQUEST_TYPE.len();
const REPLY_FRAME: usize = NIPC_HEADER + wire::RESPONSE_TYPE.len() + wire::CHALLENGE_BYTES;
const REQUEST_BUDGET_MS: i64 = 5000;

global_asm!(
    ".global _start",
    "_start:",
    "mov rdi, rsp",
    "and rsp, -16",
    "call task24_identity_start",
);

unsafe fn syscall(number: i64, args: [usize; 6]) -> isize {
    let result;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") args[0], in("rsi") args[1], in("rdx") args[2],
            in("r10") args[3], in("r8") args[4], in("r9") args[5],
            lateout("rcx") _, lateout("r11") _, options(nostack),
        );
    }
    result
}

fn call(number: i64, a: usize, b: usize, c: usize) -> isize {
    unsafe { syscall(number, [a, b, c, 0, 0, 0]) }
}

fn exit(status: usize) -> ! {
    call(libc::SYS_exit_group, status, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    exit(69)
}

#[no_mangle]
extern "C" fn task24_identity_start(stack: *const usize) -> ! {
    let ok = unsafe { *stack == 2 && run(CStr::from_ptr(*stack.add(2) as *const i8).to_bytes()) };
    exit(if ok { 0 } else { 69 })
}

fn run(locator: &[u8]) -> bool {
    let Some(path) = broker_path(locator) else {
        return false;
    };
    // C3 transfers one end of the child-created control socketpair to
    // PID 1. SO_PEERCRED records the creating child on both ends.
    let pid = call(libc::SYS_getpid, 0, 0, 0);
    let uid = call(libc::SYS_geteuid, 0, 0, 0);
    let gid = call(libc::SYS_getegid, 0, 0, 0);
    if !peer_matches(CONTROL_FD, Some(pid), uid, gid)
        || call(libc::SYS_prctl, libc::PR_SET_DUMPABLE as usize, 0, 0) != 0
    {
        return false;
    }

    let mut address: libc::sockaddr_un = unsafe { core::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(path) {
        *slot = *byte as libc::c_char;
    }
    let fd = call(
        libc::SYS_socket,
        libc::AF_UNIX as usize,
        (libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK) as usize,
        0,
    );
    if fd < 0 {
        return false;
    }
    let broker = fd as usize;
    if call(
        libc::SYS_connect,
        broker,
        &address as *const _ as usize,
        3 + path.len(),
    ) != 0
        || !peer_matches(broker, None, 0, 0)
    {
        return false;
    }

    // PID 1 is outside this PID namespace. The root credentials are an
    // early filter. Only a matching confirmation from the C3 control FD
    // authorizes a reply on this broker stream.
    match wait_for_request(broker) {
        Some(false) => true,
        Some(true) => serve(broker) && wait_for_request(broker) == Some(false),
        None => false,
    }
}

fn broker_path(argument: &[u8]) -> Option<&[u8]> {
    let path = argument.strip_prefix(b"--nagi-service-endpoint-v1=")?;
    let leaf = path.strip_prefix(b"/run/nagi/ipc/")?;
    (leaf.len() == 40
        && leaf[0] == b'r'
        && &leaf[17..19] == b"-e"
        && &leaf[35..] == b".sock"
        && leaf[1..17]
            .iter()
            .chain(&leaf[19..35])
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)))
    .then_some(path)
}

fn peer_matches(fd: usize, pid: Option<isize>, uid: isize, gid: isize) -> bool {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = core::mem::size_of_val(&credentials) as libc::socklen_t;
    let result = unsafe {
        syscall(
            libc::SYS_getsockopt,
            [
                fd,
                libc::SOL_SOCKET as usize,
                libc::SO_PEERCRED as usize,
                (&mut credentials as *mut libc::ucred) as usize,
                (&mut length as *mut libc::socklen_t) as usize,
                0,
            ],
        )
    };
    result == 0
        && length as usize == core::mem::size_of_val(&credentials)
        && pid.is_none_or(|expected| credentials.pid as isize == expected)
        && credentials.uid as isize == uid
        && credentials.gid as isize == gid
}

fn wait_for_request(broker: usize) -> Option<bool> {
    let mut fds = [
        poll_fd(CONTROL_FD, libc::POLLIN),
        poll_fd(broker, libc::POLLIN),
    ];
    loop {
        match call(
            libc::SYS_poll,
            fds.as_mut_ptr() as usize,
            fds.len(),
            usize::MAX,
        ) {
            value if value == -(libc::EINTR as isize) => continue,
            value if value < 1 => return None,
            _ => {}
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return (read_fd(CONTROL_FD, &mut [0]) == 0).then_some(false);
        }
        if fds
            .iter()
            .any(|fd| fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0)
        {
            return None;
        }
        if fds[1].revents & libc::POLLIN != 0 {
            return Some(true);
        }
    }
}

fn serve(broker: usize) -> bool {
    let Some(deadline) = monotonic_ms().and_then(|now| now.checked_add(REQUEST_BUDGET_MS)) else {
        return false;
    };
    let mut frame = [0; REQUEST_FRAME];
    if !read_exact(broker, &mut frame, deadline) {
        return false;
    }
    let Ok(request) = IpcEnvelope::decode(&frame) else {
        return false;
    };
    let Some(challenge) = wire::challenge(&request, wire::BROKER_APP, IDENTITY_APP) else {
        return false;
    };
    let mut confirmed = [0; wire::CONTROL_CONFIRMATION_BYTES];
    if !write_all(CONTROL_FD, &challenge, deadline)
        || !read_exact(CONTROL_FD, &mut confirmed, deadline)
    {
        return false;
    }
    let Some(reply) = wire::response(request, &challenge, &confirmed) else {
        return false;
    };
    let mut output = [0; REPLY_FRAME];
    reply
        .encode(&mut output)
        .is_ok_and(|length| write_all(broker, &output[..length], deadline))
}

fn poll_fd(fd: usize, events: i16) -> libc::pollfd {
    libc::pollfd {
        fd: fd as i32,
        events,
        revents: 0,
    }
}

fn monotonic_ms() -> Option<i64> {
    let mut time: libc::timespec = unsafe { core::mem::zeroed() };
    (call(
        libc::SYS_clock_gettime,
        libc::CLOCK_MONOTONIC as usize,
        &mut time as *mut _ as usize,
        0,
    ) == 0)
        .then(|| time.tv_sec * 1000 + time.tv_nsec / 1_000_000)
}

fn wait_until(fd: usize, events: i16, deadline: i64) -> bool {
    let mut fd = poll_fd(fd, events);
    loop {
        let Some(timeout) = monotonic_ms()
            .and_then(|now| deadline.checked_sub(now))
            .filter(|remaining| *remaining > 0)
        else {
            return false;
        };
        let result = call(
            libc::SYS_poll,
            &mut fd as *mut _ as usize,
            1,
            timeout as usize,
        );
        if result != -(libc::EINTR as isize) {
            return result == 1 && fd.revents & events != 0;
        }
    }
}

fn read_fd(fd: usize, bytes: &mut [u8]) -> isize {
    unsafe {
        syscall(
            libc::SYS_recvfrom,
            [
                fd,
                bytes.as_mut_ptr() as usize,
                bytes.len(),
                libc::MSG_DONTWAIT as usize,
                0,
                0,
            ],
        )
    }
}

fn read_exact(fd: usize, mut bytes: &mut [u8], deadline: i64) -> bool {
    while !bytes.is_empty() {
        if !wait_until(fd, libc::POLLIN, deadline) {
            return false;
        }
        let result = read_fd(fd, bytes);
        if result == -(libc::EINTR as isize) || result == -(libc::EAGAIN as isize) {
            continue;
        }
        if result <= 0 {
            return false;
        }
        bytes = &mut bytes[result as usize..];
    }
    true
}

fn write_all(fd: usize, mut bytes: &[u8], deadline: i64) -> bool {
    while !bytes.is_empty() {
        if !wait_until(fd, libc::POLLOUT, deadline) {
            return false;
        }
        let result = unsafe {
            syscall(
                libc::SYS_sendto,
                [
                    fd,
                    bytes.as_ptr() as usize,
                    bytes.len(),
                    (libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) as usize,
                    0,
                    0,
                ],
            )
        };
        if result == -(libc::EINTR as isize) || result == -(libc::EAGAIN as isize) {
            continue;
        }
        if result <= 0 {
            return false;
        }
        bytes = &bytes[result as usize..];
    }
    true
}
