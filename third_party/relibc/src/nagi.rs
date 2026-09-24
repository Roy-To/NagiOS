//! Nagi user-space backend for the pinned relibc source.
//!
//! The upstream relibc headers are organized around Linux and Redox system
//! calls. Nagi deliberately does not expose either syscall ABI. This backend
//! therefore keeps the relibc C symbol surface but forwards each operation to
//! the Nagi POSIX facade, which in turn uses Nagi VFS and services.

use core::{
    ffi::{
        VaList, c_char, c_double, c_float, c_int, c_long, c_longlong, c_uint, c_ulong, c_ulonglong,
        c_void,
    },
    fmt, mem, ptr, slice,
};
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
        .text
        .globl __setjmp
        .globl _setjmp
        .globl setjmp
__setjmp:
_setjmp:
setjmp:
        mov [rdi], rbx
        mov [rdi + 8], rbp
        mov [rdi + 16], r12
        mov [rdi + 24], r13
        mov [rdi + 32], r14
        mov [rdi + 40], r15
        lea rdx, [rsp + 8]
        mov [rdi + 48], rdx
        mov rdx, [rsp]
        mov [rdi + 56], rdx
        xor eax, eax
        ret

        .globl _longjmp
        .globl longjmp
_longjmp:
longjmp:
        mov eax, esi
        test eax, eax
        jne 1f
        inc eax
1:
        mov rbx, [rdi]
        mov rbp, [rdi + 8]
        mov r12, [rdi + 16]
        mov r13, [rdi + 24]
        mov r14, [rdi + 32]
        mov r15, [rdi + 40]
        mov rdx, [rdi + 48]
        mov rsp, rdx
        mov rdx, [rdi + 56]
        jmp rdx

        # x86-64 SysV passes the target's C long double in x87 ST(0).
        # Keep the ABI exact instead of narrowing it to Rust f64.
        .globl fabsl
fabsl:
        fabs
        ret
"#
);

unsafe extern "C" {
    fn nagi_posix_malloc(size: usize) -> *mut u8;
    fn nagi_posix_malloc_usable_size(pointer: *mut u8) -> usize;
    fn nagi_posix_free(pointer: *mut u8);
    fn nagi_posix_write_fd(fd: c_int, bytes: *const u8, length: usize) -> isize;
    fn nagi_posix_ioctl(fd: c_int, request: c_ulong, out: *mut c_void) -> c_int;
    fn nagi_posix_accept(socket: c_int, address: *mut c_void, address_len: *mut c_uint) -> c_int;
    fn nagi_posix_getsockopt(
        socket: c_int,
        level: c_int,
        option_name: c_int,
        option_value: *mut c_void,
        option_len: *mut c_uint,
    ) -> c_int;
    fn nagi_posix_initialize_filesystem(capability: u64) -> c_int;
    fn nagi_posix_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int;
    fn nagi_posix_read(fd: c_int, bytes: *mut u8, length: usize) -> isize;
    fn nagi_posix_close(fd: c_int) -> c_int;
    fn nagi_posix_lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn nagi_posix_lstat(path: *const c_char, buf: *mut c_void) -> c_int;
    fn nagi_posix_isatty(fd: c_int) -> c_int;
    fn nagi_posix_openat(fd: c_int, path: *const c_char, flags: c_int, mode: c_int) -> c_int;
    fn nagi_posix_unlink(path: *const c_char) -> c_int;
    fn nagi_posix_unlinkat(fd: c_int, path: *const c_char, flags: c_int) -> c_int;
    fn nagi_posix_mkdir(path: *const c_char, mode: c_uint) -> c_int;
    fn nagi_posix_rmdir(path: *const c_char) -> c_int;
    fn nagi_posix_opendir(path: *const c_char) -> *mut c_void;
    fn nagi_posix_readdir(directory: *mut c_void) -> *mut c_void;
    fn nagi_posix_readdir_r(
        directory: *mut c_void,
        entry: *mut c_void,
        result: *mut *mut c_void,
    ) -> c_int;
    fn nagi_posix_closedir(directory: *mut c_void) -> c_int;
    fn nagi_posix_fdopendir(fd: c_int) -> *mut c_void;
    fn nagi_posix_resolve_ipv4(name: *const c_char, output: *mut NagiIpv4Address) -> c_int;
    fn nagi_posix_mmap_file(length: usize, protection: c_int, fd: c_int, offset: usize) -> *mut u8;
    fn nagi_posix_mmap_at(address: *mut u8, length: usize, protection: c_int) -> *mut u8;
    fn nagi_posix_munmap(address: *mut u8, length: usize) -> c_int;
    fn nagi_posix_mprotect(address: *mut u8, length: usize, protection: c_int) -> c_int;
    fn nagi_posix_errno_location() -> *mut c_int;
    fn nagi_posix_dup2(old_fd: c_int, new_fd: c_int) -> c_int;
    fn nagi_posix_chdir(path: *const c_char) -> c_int;
    fn nagi_posix_chroot(path: *const c_char) -> c_int;
    fn nagi_posix_exit(code: c_int) -> !;
    fn nagi_posix_setpgid(pid: c_int, pgid: c_int) -> c_int;
    fn nagi_posix_setgid(gid: c_uint) -> c_int;
    fn nagi_posix_setuid(uid: c_uint) -> c_int;
    fn nagi_posix_getpid() -> c_int;
    fn nagi_posix_getuid() -> c_int;
    fn nagi_posix_geteuid() -> c_int;
    fn nagi_posix_getgid() -> c_int;
    fn nagi_posix_getegid() -> c_int;
    fn nagi_posix_setsid() -> c_int;
    fn nagi_posix_signal(signal: c_int, handler: *mut c_void) -> *mut c_void;
    fn nagi_posix_waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn clock_gettime(clock: c_int, output: *mut NagiTimespec) -> c_int;
    fn abort() -> !;
}

const EINVAL: c_int = 22;
const EACCES: c_int = 13;
const ENOSYS: c_int = 38;
const ENOMEM: c_int = 12;
const EBADF: c_int = 9;
const EBUSY: c_int = 16;
const EAGAIN: c_int = 11;
const EOVERFLOW: c_int = 75;
const ERANGE: c_int = 34;
const EOF: c_int = -1;
const NAGI_F_OK: c_int = 0;
const NAGI_R_OK: c_int = 4;
const NAGI_W_OK: c_int = 2;
const NAGI_X_OK: c_int = 1;
const NAGI_IOFBF: c_int = 0;
const NAGI_IOLBF: c_int = 1;
const NAGI_IONBF: c_int = 2;
const MAP_FIXED: c_int = 0x0010;
const MAP_ANONYMOUS: c_int = 0x0020;

const NAGI_FILE_MEMORY: u32 = 1;
const NAGI_FILE_FD: u32 = 2;
const NAGI_O_CREAT: c_int = 0x0200_0000;
const NAGI_O_TRUNC: c_int = 0x0400_0000;

#[repr(C)]
pub struct NagiDivT {
    pub quot: c_int,
    pub rem: c_int,
}

const AF_UNSPEC: c_int = 0;
const AF_INET: c_int = 2;
const SOCK_STREAM: c_int = 1;
const AI_PASSIVE: c_int = 1;
const AI_CANONNAME: c_int = 2;
const AI_NUMERICHOST: c_int = 4;
const AI_NUMERICSERV: c_int = 0x400;
const EAI_BADFLAGS: c_int = -1;
const EAI_NONAME: c_int = -2;
const EAI_FAIL: c_int = -4;
const EAI_FAMILY: c_int = -6;
const EAI_SERVICE: c_int = -8;
const EAI_MEMORY: c_int = -10;

#[repr(C)]
struct NagiIpv4Address {
    octets: [u8; 4],
}

#[repr(C)]
struct NagiSockaddrIn {
    family: u16,
    port_be: u16,
    address: u32,
    zero: [u8; 8],
}

#[repr(C)]
struct NagiAddrInfo {
    flags: c_int,
    family: c_int,
    socktype: c_int,
    protocol: c_int,
    addrlen: u32,
    canonname: *mut c_char,
    address: *mut c_void,
    next: *mut NagiAddrInfo,
}

unsafe fn c_string_len(pointer: *const c_char, limit: usize) -> Option<usize> {
    if pointer.is_null() {
        return None;
    }
    for length in 0..limit {
        if unsafe { pointer.add(length).read() } == 0 {
            return Some(length);
        }
    }
    None
}

fn parse_ipv4(bytes: &[u8]) -> Option<[u8; 4]> {
    let mut octets = [0_u8; 4];
    let mut octet = 0;
    let mut value = 0_u16;
    let mut digits = 0;
    for &byte in bytes.iter().chain(core::iter::once(&b'.')) {
        if byte.is_ascii_digit() {
            value = value.checked_mul(10)?.checked_add(u16::from(byte - b'0'))?;
            if value > u16::from(u8::MAX) {
                return None;
            }
            digits += 1;
        } else if byte == b'.' && digits != 0 {
            if octet >= octets.len() {
                return None;
            }
            octets[octet] = value as u8;
            octet += 1;
            value = 0;
            digits = 0;
        } else {
            return None;
        }
    }
    (octet == octets.len()).then_some(octets)
}

fn parse_port(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() {
        return Some(0);
    }
    let mut value = 0_u32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    u16::try_from(value).ok()
}

unsafe fn free_addrinfo_node(info: *mut NagiAddrInfo) {
    if info.is_null() {
        return;
    }
    let node = unsafe { info.read() };
    if !node.address.is_null() {
        unsafe { nagi_posix_free(node.address.cast()) };
    }
    if !node.canonname.is_null() {
        unsafe { nagi_posix_free(node.canonname.cast()) };
    }
    unsafe { nagi_posix_free(info.cast()) };
}

/// Target-owned POSIX name lookup for the network path used by Servo.
///
/// Numeric IPv4 literals are parsed locally; names are resolved through the
/// real Nagi DNS/POSIX boundary. The result uses the standard `addrinfo` ABI,
/// while its sockaddr layout remains compatible with Nagi's IPv4 connect
/// boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getaddrinfo(
    node: *const c_char,
    service: *const c_char,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> c_int {
    if result.is_null() {
        return EAI_FAIL;
    }
    unsafe { result.write(ptr::null_mut()) };

    let (flags, family, socktype, protocol) = if hints.is_null() {
        (0, AF_UNSPEC, 0, 0)
    } else {
        let hint = unsafe { &*hints.cast::<NagiAddrInfo>() };
        (hint.flags, hint.family, hint.socktype, hint.protocol)
    };
    if family != AF_UNSPEC && family != AF_INET {
        return EAI_FAMILY;
    }
    if flags & !(AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST | AI_NUMERICSERV) != 0 {
        return EAI_BADFLAGS;
    }
    if flags & AI_CANONNAME != 0 && node.is_null() {
        return EAI_BADFLAGS;
    }

    let node_length = if node.is_null() {
        0
    } else if let Some(length) = unsafe { c_string_len(node, 256) } {
        length
    } else {
        return EAI_NONAME;
    };
    let node_bytes = if node.is_null() {
        if flags & AI_PASSIVE != 0 {
            b"0.0.0.0".as_slice()
        } else {
            b"127.0.0.1".as_slice()
        }
    } else {
        unsafe { core::slice::from_raw_parts(node.cast::<u8>(), node_length) }
    };
    if node_bytes.is_empty() {
        return EAI_NONAME;
    }

    let service_bytes = if service.is_null() {
        &[][..]
    } else if let Some(length) = unsafe { c_string_len(service, 32) } {
        unsafe { core::slice::from_raw_parts(service.cast::<u8>(), length) }
    } else {
        return EAI_SERVICE;
    };
    let port = if flags & AI_NUMERICSERV != 0 || !service_bytes.is_empty() {
        match parse_port(service_bytes) {
            Some(port) => port,
            None => return EAI_SERVICE,
        }
    } else {
        0
    };

    let octets = if let Some(octets) = parse_ipv4(node_bytes) {
        octets
    } else {
        if flags & AI_NUMERICHOST != 0 || node_bytes.len() >= 256 {
            return EAI_NONAME;
        }
        let mut query = [0_u8; 256];
        unsafe {
            ptr::copy_nonoverlapping(node_bytes.as_ptr(), query.as_mut_ptr(), node_bytes.len());
            query[node_bytes.len()] = 0;
        }
        let mut address = NagiIpv4Address { octets: [0; 4] };
        if unsafe { nagi_posix_resolve_ipv4(query.as_ptr().cast(), &mut address) } != 0 {
            return EAI_FAIL;
        }
        address.octets
    };

    let address = unsafe { nagi_posix_malloc(mem::size_of::<NagiSockaddrIn>()) };
    if address.is_null() {
        return EAI_MEMORY;
    }
    unsafe {
        address.cast::<NagiSockaddrIn>().write(NagiSockaddrIn {
            family: AF_INET as u16,
            port_be: port.to_be(),
            address: u32::from_ne_bytes(octets),
            zero: [0; 8],
        });
    }

    let canonname = if flags & AI_CANONNAME != 0 {
        let memory = unsafe { nagi_posix_malloc(node_length + 1) };
        if memory.is_null() {
            unsafe { nagi_posix_free(address) };
            return EAI_MEMORY;
        }
        unsafe {
            ptr::copy_nonoverlapping(node_bytes.as_ptr(), memory, node_bytes.len());
            memory.add(node_bytes.len()).write(0);
        }
        memory.cast::<c_char>()
    } else {
        ptr::null_mut()
    };
    let info = unsafe { nagi_posix_malloc(mem::size_of::<NagiAddrInfo>()) };
    if info.is_null() {
        unsafe {
            nagi_posix_free(address);
            if !canonname.is_null() {
                nagi_posix_free(canonname.cast());
            }
        }
        return EAI_MEMORY;
    }
    unsafe {
        info.cast::<NagiAddrInfo>().write(NagiAddrInfo {
            flags,
            family: AF_INET,
            socktype: if socktype == 0 { SOCK_STREAM } else { socktype },
            protocol,
            addrlen: mem::size_of::<NagiSockaddrIn>() as u32,
            canonname,
            address: address.cast(),
            next: ptr::null_mut(),
        });
        result.write(info.cast());
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeaddrinfo(mut result: *mut c_void) {
    while !result.is_null() {
        let info = result.cast::<NagiAddrInfo>();
        let next = unsafe { (*info).next };
        unsafe { free_addrinfo_node(info) };
        result = next.cast();
    }
}

/// Nagi creates processes through its spawn-oriented service boundary; fork's
/// shared-address-space semantics are not part of the 0.1 kernel contract.
/// Keep the ABI truthful by reporting the supported error rather than
/// returning a fabricated child PID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fork() -> c_int {
    unsafe { set_errno(ENOSYS) };
    -1
}

/// Nagi user images are statically linked and do not expose a dynamic loader
/// namespace.  Keep the ABI truthful: a dynamic lookup fails closed instead
/// of returning a fabricated function pointer or consulting the host process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlsym(
    _handle: *mut c_void,
    _symbol: *const c_char,
) -> *mut c_void {
    unsafe { set_errno(ENOSYS) };
    ptr::null_mut()
}

// `pthread_once_t` is a four-byte target ABI object in the selected relibc
// headers.  0 means not started, 1 means an initializer is in progress, and 2
// means completed.  The state lives in guest memory supplied by the caller;
// no host pthread or process-global lock is involved.
const NAGI_ONCE_RUNNING: u32 = 1;
const NAGI_ONCE_COMPLETE: u32 = 2;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_once(
    once_control: *mut c_void,
    init_routine: Option<extern "C" fn()>,
) -> c_int {
    if once_control.is_null() || init_routine.is_none() {
        return EINVAL;
    }
    let state = unsafe { &*once_control.cast::<AtomicU32>() };
    loop {
        match state.load(Ordering::Acquire) {
            NAGI_ONCE_COMPLETE => return 0,
            0 => {
                if state
                    .compare_exchange(
                        0,
                        NAGI_ONCE_RUNNING,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    unsafe { init_routine.unwrap_unchecked()() };
                    state.store(NAGI_ONCE_COMPLETE, Ordering::Release);
                    return 0;
                }
            }
            NAGI_ONCE_RUNNING => core::hint::spin_loop(),
            _ => return EINVAL,
        }
    }
}

const NAGI_ATEXIT_LIMIT: usize = 32;
static NAGI_ATEXIT_COUNT: AtomicU32 = AtomicU32::new(0);
static mut NAGI_ATEXIT_HANDLERS: [Option<extern "C" fn()>; NAGI_ATEXIT_LIMIT] =
    [None; NAGI_ATEXIT_LIMIT];

/// Keep normal `exit` handlers in target-owned storage. `_exit` deliberately
/// bypasses this table, matching POSIX process semantics; `exit` drains it in
/// reverse registration order before crossing the Nagi process boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atexit(function: Option<extern "C" fn()>) -> c_int {
    let Some(function) = function else {
        unsafe { set_errno(EINVAL) };
        return -1;
    };
    let index = NAGI_ATEXIT_COUNT.fetch_add(1, Ordering::AcqRel) as usize;
    if index >= NAGI_ATEXIT_LIMIT {
        NAGI_ATEXIT_COUNT.fetch_sub(1, Ordering::AcqRel);
        unsafe { set_errno(ENOMEM) };
        return -1;
    }
    unsafe { NAGI_ATEXIT_HANDLERS[index] = Some(function) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _exit(code: c_int) -> ! {
    unsafe { nagi_posix_exit(code) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup2(old_fd: c_int, new_fd: c_int) -> c_int {
    unsafe { nagi_posix_dup2(old_fd, new_fd) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exit(code: c_int) -> ! {
    let count = (NAGI_ATEXIT_COUNT.load(Ordering::Acquire) as usize).min(NAGI_ATEXIT_LIMIT);
    for index in (0..count).rev() {
        let handler = unsafe { NAGI_ATEXIT_HANDLERS[index].take() };
        if let Some(handler) = handler {
            handler();
        }
    }
    unsafe { nagi_posix_exit(code) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setgid(gid: c_uint) -> c_int {
    unsafe { nagi_posix_setgid(gid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setuid(uid: c_uint) -> c_int {
    unsafe { nagi_posix_setuid(uid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpid() -> c_int {
    unsafe { nagi_posix_getpid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getuid() -> c_int {
    unsafe { nagi_posix_getuid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn geteuid() -> c_int {
    unsafe { nagi_posix_geteuid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getgid() -> c_int {
    unsafe { nagi_posix_getgid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getegid() -> c_int {
    unsafe { nagi_posix_getegid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setpgid(pid: c_int, pgid: c_int) -> c_int {
    unsafe { nagi_posix_setpgid(pid, pgid) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setsid() -> c_int {
    unsafe { nagi_posix_setsid() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal(
    signal_number: c_int,
    handler: Option<extern "C" fn(c_int)>,
) -> Option<extern "C" fn(c_int)> {
    let handler = handler.map_or(core::ptr::null_mut(), |function| {
        function as *mut c_void
    });
    let result = unsafe { nagi_posix_signal(signal_number, handler) };
    unsafe { core::mem::transmute::<usize, Option<extern "C" fn(c_int)>>(result as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    unsafe { nagi_posix_chdir(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn chroot(path: *const c_char) -> c_int {
    unsafe { nagi_posix_chroot(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    unsafe { nagi_posix_waitpid(pid, status, options) }
}

// The target-selected relibc pthread header exposes a four-byte opaque
// pthread_rwlock_t. Keep that ABI size and implement the lock directly with a
// Nagi user-space atomic word: bit 31 is the writer state and the lower bits
// count active readers. This is a real guest synchronization primitive; it
// does not call a host pthread or silently turn a read lock into a no-op.
const NAGI_RWLOCK_WRITER: u32 = 1 << 31;
const NAGI_RWLOCK_READERS: u32 = NAGI_RWLOCK_WRITER - 1;

unsafe fn rwlock_word(lock: *mut c_void) -> &'static AtomicU32 {
    unsafe { &*lock.cast::<AtomicU32>() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_init(
    lock: *mut c_void,
    _attributes: *const c_void,
) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    unsafe { ptr::write(lock.cast::<AtomicU32>(), AtomicU32::new(0)) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_rdlock(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    let lock = unsafe { rwlock_word(lock) };
    loop {
        let state = lock.load(Ordering::Acquire);
        if state & NAGI_RWLOCK_WRITER != 0 {
            core::hint::spin_loop();
            continue;
        }
        if state & NAGI_RWLOCK_READERS == NAGI_RWLOCK_READERS {
            return EAGAIN;
        }
        if lock
            .compare_exchange(
                state,
                state + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return 0;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_tryrdlock(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    let lock = unsafe { rwlock_word(lock) };
    let state = lock.load(Ordering::Acquire);
    if state & NAGI_RWLOCK_WRITER != 0 || state & NAGI_RWLOCK_READERS == NAGI_RWLOCK_READERS {
        return EBUSY;
    }
    if lock
        .compare_exchange(state, state + 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        0
    } else {
        EBUSY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_wrlock(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    let lock = unsafe { rwlock_word(lock) };
    loop {
        if lock
            .compare_exchange(0, NAGI_RWLOCK_WRITER, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return 0;
        }
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_trywrlock(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    let lock = unsafe { rwlock_word(lock) };
    if lock
        .compare_exchange(0, NAGI_RWLOCK_WRITER, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        0
    } else {
        EBUSY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_unlock(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    let lock = unsafe { rwlock_word(lock) };
    loop {
        let state = lock.load(Ordering::Acquire);
        if state & NAGI_RWLOCK_WRITER != 0 {
            if lock
                .compare_exchange(
                    state,
                    0,
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return 0;
            }
            continue;
        }
        if state & NAGI_RWLOCK_READERS == 0 {
            return EINVAL;
        }
        if lock
            .compare_exchange(
                state,
                state - 1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return 0;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_rwlock_destroy(lock: *mut c_void) -> c_int {
    if lock.is_null() {
        return EINVAL;
    }
    if unsafe { rwlock_word(lock) }.load(Ordering::Acquire) == 0 {
        0
    } else {
        EBUSY
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __assert_fail(
    _expression: *const c_char,
    _file: *const c_char,
    _line: c_uint,
    _function: *const c_char,
) -> ! {
    unsafe { abort() }
}

/// The target build does not compile relibc's Linux/Redox stdio module.  Keep
/// the opaque C FILE ABI small and Nagi-owned for the real target facilities
/// that Mesa needs.  A memory stream owns its backing allocation until the
/// caller closes the stream; the caller then owns `*bufp`, as required by
/// POSIX `open_memstream`.
#[repr(C)]
struct NagiFile {
    kind: u32,
    fd: c_int,
    owned: bool,
    eof: bool,
    error: bool,
    buffer: *mut u8,
    length: usize,
    capacity: usize,
    bufp: *mut *mut c_char,
    sizep: *mut usize,
}

// The target build does not compile relibc's upstream stdio module, so the
// standard stream objects must be owned by this backend.  The pointer keeps
// the generated C ABI (`FILE *stderr`) while the stream itself forwards writes
// to Nagi's real descriptor-2 facade.
static mut NAGI_STDERR: NagiFile = NagiFile {
    kind: NAGI_FILE_FD,
    fd: 2,
    owned: false,
    eof: false,
    error: false,
    buffer: ptr::null_mut(),
    length: 0,
    capacity: 0,
    bufp: ptr::null_mut(),
    sizep: ptr::null_mut(),
};

#[unsafe(no_mangle)]
pub static mut stderr: *mut c_void = ptr::addr_of_mut!(NAGI_STDERR).cast();

// Keep stdout as a real Nagi descriptor-backed FILE object.  Mesa/libc++ use
// the C symbol directly in diagnostics and stream helpers; exposing the
// target descriptor here preserves the guest stdout boundary instead of
// routing output through a host stdio object.
static mut NAGI_STDOUT: NagiFile = NagiFile {
    kind: NAGI_FILE_FD,
    fd: 1,
    owned: false,
    eof: false,
    error: false,
    buffer: ptr::null_mut(),
    length: 0,
    capacity: 0,
    bufp: ptr::null_mut(),
    sizep: ptr::null_mut(),
};

#[unsafe(no_mangle)]
pub static mut stdout: *mut c_void = ptr::addr_of_mut!(NAGI_STDOUT).cast();

// Nagi user processes currently start with an explicitly empty environment.
// Keep the standard environ object real and writable at the ABI boundary; a
// future process-service environment can replace this pointer during startup
// without changing the relibc symbol contract. A null vector is the POSIX
// representation of an environment containing no entries.
#[unsafe(no_mangle)]
pub static mut environ: *mut *mut c_char = ptr::null_mut();

#[inline]
unsafe fn set_errno(error: c_int) {
    let location = unsafe { nagi_posix_errno_location() };
    if !location.is_null() {
        unsafe { location.write(error) };
    }
}

/// Expose the target's capability-scoped errno slot to code compiled against
/// the normal C ABI.  This never aliases a host libc TLS object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut c_int {
    unsafe { nagi_posix_errno_location() }
}

unsafe fn grow_memory_stream(stream: &mut NagiFile, required: usize) -> bool {
    if required <= stream.capacity {
        return true;
    }

    let mut capacity = stream.capacity.max(64);
    while capacity < required {
        let Some(next) = capacity.checked_mul(2) else {
            unsafe { set_errno(EOVERFLOW) };
            return false;
        };
        capacity = next;
    }

    let replacement = if stream.buffer.is_null() {
        unsafe { nagi_posix_malloc(capacity) }
    } else {
        let old_size = unsafe { stream.buffer.sub(16).cast::<usize>().read() };
        let replacement = unsafe { nagi_posix_malloc(capacity) };
        if !replacement.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(stream.buffer, replacement, old_size.min(stream.length));
            }
        }
        replacement
    };
    if replacement.is_null() {
        unsafe { set_errno(ENOMEM) };
        return false;
    }

    // Allocate-and-copy keeps this stream independent of the allocator's
    // non-in-place growth semantics and preserves its bounded guest-memory
    // contract.
    stream.buffer = replacement;
    stream.capacity = capacity;
    true
}

unsafe fn publish_memory_stream(stream: &mut NagiFile) {
    unsafe {
        stream.buffer.add(stream.length).write(0);
        stream.bufp.write(stream.buffer.cast());
        stream.sizep.write(stream.length);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open_memstream(bufp: *mut *mut c_char, sizep: *mut usize) -> *mut c_void {
    if bufp.is_null() || sizep.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }

    unsafe {
        bufp.write(ptr::null_mut());
        sizep.write(0);
    }

    let stream = unsafe { nagi_posix_malloc(mem::size_of::<NagiFile>()) }.cast::<NagiFile>();
    if stream.is_null() {
        unsafe { set_errno(ENOMEM) };
        return ptr::null_mut();
    }
    unsafe {
        stream.write(NagiFile {
            kind: NAGI_FILE_MEMORY,
            fd: -1,
            owned: false,
            eof: false,
            error: false,
            buffer: ptr::null_mut(),
            length: 0,
            capacity: 0,
            bufp,
            sizep,
        });
    }
    stream.cast()
}

/// Check a path through the real Nagi VFS descriptor boundary.  Nagi 0.1's
/// user VFS exposes readable files but does not yet publish POSIX permission
/// or executable-bit metadata, so W_OK/X_OK fail closed instead of claiming a
/// permission result that the capability runtime cannot prove.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
    if path.is_null() || mode & !(NAGI_F_OK | NAGI_R_OK | NAGI_W_OK | NAGI_X_OK) != 0 {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    if mode & (NAGI_W_OK | NAGI_X_OK) != 0 {
        unsafe { set_errno(EACCES) };
        return EOF;
    }

    let fd = unsafe { nagi_posix_open(path, 0, 0) };
    if fd < 0 {
        return EOF;
    }
    if unsafe { nagi_posix_close(fd) } == 0 {
        0
    } else {
        EOF
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut c_void {
    if path.is_null() || mode.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }

    let first = unsafe { *mode.cast::<u8>() };
    let flags = match first {
        b'r' => 0,
        b'w' => NAGI_O_CREAT | NAGI_O_TRUNC,
        b'a' => NAGI_O_CREAT,
        _ => {
            unsafe { set_errno(EINVAL) };
            return ptr::null_mut();
        }
    };
    let fd = unsafe { nagi_posix_open(path, flags, 0o666) };
    if fd < 0 {
        return ptr::null_mut();
    }
    if first == b'a' && unsafe { nagi_posix_lseek(fd, 0, 2) } < 0 {
        unsafe { nagi_posix_close(fd) };
        return ptr::null_mut();
    }

    let stream = unsafe { nagi_posix_malloc(mem::size_of::<NagiFile>()) }.cast::<NagiFile>();
    if stream.is_null() {
        unsafe { nagi_posix_close(fd) };
        unsafe { set_errno(ENOMEM) };
        return ptr::null_mut();
    }
    unsafe {
        stream.write(NagiFile {
            kind: NAGI_FILE_FD,
            fd,
            owned: true,
            eof: false,
            error: false,
            buffer: ptr::null_mut(),
            length: 0,
            capacity: 0,
            bufp: ptr::null_mut(),
            sizep: ptr::null_mut(),
        });
    }
    stream.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(
    bytes: *const c_void,
    size: usize,
    count: usize,
    stream: *mut c_void,
) -> usize {
    if size == 0 || count == 0 {
        return 0;
    }
    let Some(length) = size.checked_mul(count) else {
        unsafe { set_errno(EOVERFLOW) };
        return 0;
    };
    if bytes.is_null() || stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }

    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind == NAGI_FILE_FD {
        let written = unsafe { nagi_posix_write_fd(stream.fd, bytes.cast(), length) };
        return if written <= 0 {
            0
        } else {
            (written as usize) / size
        };
    }
    if stream.kind != NAGI_FILE_MEMORY {
        unsafe { set_errno(EBADF) };
        return 0;
    }
    let Some(required) = stream.length.checked_add(length + 1) else {
        unsafe { set_errno(EOVERFLOW) };
        return 0;
    };
    if !unsafe { grow_memory_stream(stream, required) } {
        return 0;
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.cast::<u8>(), stream.buffer.add(stream.length), length);
    }
    stream.length += length;
    unsafe { publish_memory_stream(stream) };
    count
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(
    bytes: *mut c_void,
    size: usize,
    count: usize,
    stream: *mut c_void,
) -> usize {
    if size == 0 || count == 0 {
        return 0;
    }
    let Some(length) = size.checked_mul(count) else {
        unsafe { set_errno(EOVERFLOW) };
        return 0;
    };
    if bytes.is_null() || stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }

    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD {
        // open_memstream is a write stream in this target ABI. Do not treat
        // its output buffer as an implicit readable file or claim bytes that
        // were not read through a Nagi descriptor.
        unsafe { set_errno(EBADF) };
        return 0;
    }

    let read = unsafe { nagi_posix_read(stream.fd, bytes.cast(), length) };
    if read < 0 {
        stream.error = true;
        0
    } else if read == 0 {
        stream.eof = true;
        0
    } else {
        (read as usize) / size
    }
}

/// Read one line from a real Nagi descriptor-backed stream.  This is the
/// minimal unbuffered target stdio operation required by the pinned Mesa and
/// libc++ sources; every byte still crosses the Nagi descriptor/VFS boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(
    output: *mut c_char,
    length: c_int,
    stream: *mut c_void,
) -> *mut c_char {
    if output.is_null() || stream.is_null() || length <= 0 {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }

    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD {
        unsafe { set_errno(EBADF) };
        stream.error = true;
        return ptr::null_mut();
    }
    if length == 1 {
        unsafe { output.write(0) };
        return output;
    }

    let mut written = 0usize;
    while written < (length as usize - 1) {
        let mut byte = 0u8;
        let read = unsafe { nagi_posix_read(stream.fd, &mut byte, 1) };
        if read < 0 {
            stream.error = true;
            break;
        }
        if read == 0 {
            stream.eof = true;
            break;
        }

        unsafe { output.add(written).cast::<u8>().write(byte) };
        written += 1;
        if byte == b'\n' {
            break;
        }
    }

    unsafe { output.add(written).write(0) };
    if written == 0 {
        ptr::null_mut()
    } else {
        output
    }
}

/// Return the EOF state recorded by the target descriptor-backed stream.
/// Reading to end-of-file is the only operation that sets this flag; a
/// successful read clears neither EOF nor error, matching C stream state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feof(stream: *mut c_void) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }
    let stream = unsafe { &*stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD {
        unsafe { set_errno(EBADF) };
        return 0;
    }
    if stream.eof { 1 } else { 0 }
}

fn nagi_errno_message(error: c_int) -> &'static [u8] {
    match error {
        9 => b"Bad file descriptor\n",
        11 => b"Resource temporarily unavailable\n",
        12 => b"Out of memory\n",
        22 => b"Invalid argument\n",
        34 => b"Numerical result out of range\n",
        38 => b"Function not implemented\n",
        75 => b"Value too large for defined data type\n",
        _ => b"Unknown error\n",
    }
}

/// Write a truthful Nagi diagnostic to the guest stderr descriptor.  The
/// target backend has no host `errno` string table or host stderr; both the
/// prefix and the error text use the Nagi descriptor boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perror(prefix: *const c_char) {
    let error = unsafe {
        let location = nagi_posix_errno_location();
        if location.is_null() {
            0
        } else {
            location.read()
        }
    };
    if !prefix.is_null() {
        if let Some(length) = unsafe { c_string_len(prefix, 4096) } {
            unsafe { nagi_posix_write_fd(2, prefix.cast(), length) };
            unsafe { nagi_posix_write_fd(2, b": ".as_ptr(), 2) };
        }
    }
    let message = nagi_errno_message(error);
    unsafe { nagi_posix_write_fd(2, message.as_ptr(), message.len()) };
}

/// Target-owned seek for descriptor-backed FILE streams.  The target stdio
/// backend deliberately has no host file object; cursor movement is forwarded
/// to Nagi's descriptor runtime so file-size probes and cursor-dependent
/// operations observe the guest VFS position.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseek(
    stream: *mut c_void,
    offset: c_long,
    whence: c_int,
) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD {
        unsafe { set_errno(EBADF) };
        return EOF;
    }
    if unsafe { nagi_posix_lseek(stream.fd, offset as i64, whence) } < 0 {
        EOF
    } else {
        0
    }
}

/// Return the descriptor-backed FILE cursor from Nagi's real VFS runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftell(stream: *mut c_void) -> c_long {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return -1;
    }
    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD {
        unsafe { set_errno(EBADF) };
        return -1;
    }
    let position = unsafe { nagi_posix_lseek(stream.fd, 0, 1) };
    if position < 0 {
        -1
    } else {
        position as c_long
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fflush(stream: *mut c_void) -> c_int {
    if stream.is_null() {
        // There is no global stream registry in the target backend.  Mesa's
        // memory-stream users always pass their FILE explicitly.
        return 0;
    }
    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind == NAGI_FILE_FD {
        return 0;
    }
    if stream.kind != NAGI_FILE_MEMORY {
        unsafe { set_errno(EBADF) };
        return EOF;
    }
    unsafe { publish_memory_stream(stream) };
    0
}

/// Nagi target FILE streams are deliberately unbuffered: each write crosses
/// the descriptor or guest-memory stream boundary immediately.  Accept the
/// corresponding `_IONBF` request and report unsupported buffered modes
/// explicitly rather than pretending to install host-owned buffering.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setvbuf(
    stream: *mut c_void,
    _buffer: *mut c_char,
    mode: c_int,
    _size: usize,
) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    if !matches!(mode, NAGI_IOFBF | NAGI_IOLBF | NAGI_IONBF) {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let stream = unsafe { &*stream.cast::<NagiFile>() };
    if stream.kind != NAGI_FILE_FD && stream.kind != NAGI_FILE_MEMORY {
        unsafe { set_errno(EBADF) };
        return EOF;
    }
    if mode == NAGI_IONBF {
        0
    } else {
        unsafe { set_errno(ENOSYS) };
        EOF
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(stream: *mut c_void) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind == NAGI_FILE_FD {
        let result = if unsafe { nagi_posix_close(stream.fd) } == 0 {
            0
        } else {
            EOF
        };
        if stream.owned {
            unsafe { nagi_posix_free((stream as *mut NagiFile).cast()) };
        }
        return result;
    }
    if stream.kind != NAGI_FILE_MEMORY {
        unsafe { set_errno(EBADF) };
        return EOF;
    }
    unsafe { publish_memory_stream(stream) };
    // The FILE object is released through the Nagi allocator. The published
    // caller-owned buffer remains live until the caller releases it.
    unsafe { nagi_posix_free((stream as *mut NagiFile).cast::<u8>()) };
    0
}

/// Nagi 0.1 intentionally does not expose System V shared memory.  Mesa's
/// pinned static graph contains optional X11/DRI shared-memory objects, but
/// the M17 surfaceless Softpipe path must not turn that optional dependency
/// into a fake shared-memory handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shmget(_key: c_int, _size: usize, _flags: c_int) -> c_int {
    unsafe { set_errno(ENOSYS) };
    -1
}

/// Nagi 0.1 does not expose System V shared-memory mappings.  Keep the
/// remaining ABI entry points explicit as well: returning a host pointer or
/// accepting an untracked detach/control operation would cross the guest
/// memory boundary and falsely claim a capability that the target runtime
/// does not provide.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shmat(
    _shmid: c_int,
    _shmaddr: *const c_void,
    _shmflg: c_int,
) -> *mut c_void {
    unsafe { set_errno(ENOSYS) };
    (-1isize) as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shmdt(_shmaddr: *const c_void) -> c_int {
    unsafe { set_errno(ENOSYS) };
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shmctl(
    _shmid: c_int,
    _cmd: c_int,
    _buf: *mut c_void,
) -> c_int {
    unsafe { set_errno(ENOSYS) };
    -1
}

#[repr(C)]
struct NagiTimespec {
    tv_sec: c_longlong,
    tv_nsec: c_longlong,
}

const NAGI_CLOCK_REALTIME: c_int = 1;

/// Use the real guest realtime clock exposed by Nagi POSIX.  The underlying
/// path is the Nagi kernel realtime syscall; no host clock is consulted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn time(tloc: *mut c_longlong) -> c_longlong {
    let mut timespec = NagiTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { clock_gettime(NAGI_CLOCK_REALTIME, &mut timespec) } != 0 {
        return -1;
    }
    if !tloc.is_null() {
        unsafe { tloc.write(timespec.tv_sec) };
    }
    timespec.tv_sec
}

#[repr(C)]
struct NagiTm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

static NAGI_C_LOCALE: &[u8] = b"C\0";
static NAGI_UTC_ZONE: &[u8] = b"UTC\0";

// Nagi 0.1 exposes UTC only. Keep the POSIX timezone globals in guest memory
// so time consumers never resolve them through a host libc or timezone DB.
#[unsafe(no_mangle)]
pub static mut tzname: [*mut c_char; 2] = [
    NAGI_UTC_ZONE.as_ptr() as *mut c_char,
    NAGI_UTC_ZONE.as_ptr() as *mut c_char,
];

#[unsafe(no_mangle)]
pub static mut daylight: c_int = 0;

#[inline]
fn nagi_is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Convert a Unix timestamp to a proleptic Gregorian UTC date without a host
/// time library. The layout matches relibc's x86-64 `struct tm` ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn localtime_r(
    timer: *const c_longlong,
    result: *mut NagiTm,
) -> *mut NagiTm {
    if timer.is_null() || result.is_null() {
        return ptr::null_mut();
    }

    let timestamp = unsafe { *timer };
    const SECONDS_PER_DAY: i64 = 86_400;
    let days = timestamp.div_euclid(SECONDS_PER_DAY);
    let seconds = timestamp.rem_euclid(SECONDS_PER_DAY);

    // Howard Hinnant's civil_from_days algorithm, with 1970-01-01 as day 0.
    let adjusted_days = days + 719_468;
    let era = if adjusted_days >= 0 {
        adjusted_days
    } else {
        adjusted_days - 146_096
    } / 146_097;
    let day_of_era = adjusted_days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        / 365;
    let year_base = year_of_era + era * 400;
    let day_of_year_from_march = day_of_era
        - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year_from_march + 2) / 153;
    let day = day_of_year_from_march - (153 * month_from_march + 2) / 5 + 1;
    let month = month_from_march + if month_from_march < 10 { 3 } else { -9 };
    let year = year_base + i64::from(month <= 2);

    let month_lengths = [31_i64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut yday = day - 1;
    for month_index in 0..(month - 1) {
        yday += month_lengths[month_index as usize];
    }
    if month > 2 && nagi_is_leap_year(year) {
        yday += 1;
    }

    unsafe {
        result.write(NagiTm {
            tm_sec: (seconds % 60) as c_int,
            tm_min: ((seconds / 60) % 60) as c_int,
            tm_hour: (seconds / 3_600) as c_int,
            tm_mday: day as c_int,
            tm_mon: (month - 1) as c_int,
            tm_year: (year - 1_900) as c_int,
            tm_wday: (days + 4).rem_euclid(7) as c_int,
            tm_yday: yday as c_int,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: NAGI_UTC_ZONE.as_ptr().cast(),
        });
    }
    result
}

/// Nagi's M17 target exposes only the C/POSIX locale and does not consult a
/// host locale database. The empty locale name selects that same default.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setlocale(_category: c_int, locale: *const c_char) -> *mut c_char {
    if locale.is_null() {
        return NAGI_C_LOCALE.as_ptr().cast_mut().cast();
    }

    let mut index = 0;
    let mut matches_c = true;
    let mut matches_posix = true;
    loop {
        let byte = unsafe { locale.cast::<u8>().add(index).read() };
        if byte == 0 {
            break;
        }
        if byte != *b"C\0".get(index).unwrap_or(&0) {
            matches_c = false;
        }
        if byte != *b"POSIX\0".get(index).unwrap_or(&0) {
            matches_posix = false;
        }
        index += 1;
    }
    if index == 0 || (matches_c && index == 1) || (matches_posix && index == 5) {
        NAGI_C_LOCALE.as_ptr().cast_mut().cast()
    } else {
        ptr::null_mut()
    }
}

static NAGI_STRERROR_UNKNOWN: &[u8] = b"Unknown error\0";
static NAGI_STRERROR_BAD_FD: &[u8] = b"Bad file descriptor\0";
static NAGI_STRERROR_TRY_AGAIN: &[u8] = b"Resource temporarily unavailable\0";
static NAGI_STRERROR_NO_MEMORY: &[u8] = b"Out of memory\0";
static NAGI_STRERROR_PERMISSION: &[u8] = b"Permission denied\0";
static NAGI_STRERROR_BUSY: &[u8] = b"Device or resource busy\0";
static NAGI_STRERROR_INVALID: &[u8] = b"Invalid argument\0";
static NAGI_STRERROR_RANGE: &[u8] = b"Numerical result out of range\0";
static NAGI_STRERROR_NOT_IMPLEMENTED: &[u8] = b"Function not implemented\0";
static NAGI_STRERROR_OVERFLOW: &[u8] = b"Value too large for defined data type\0";

/// Return Nagi-owned errno text without importing a host libc string table.
/// The returned storage is static target data, matching the ordinary C
/// `strerror` lifetime contract for this single-locale Nagi backend.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strerror(error: c_int) -> *mut c_char {
    let message = match error {
        EBADF => NAGI_STRERROR_BAD_FD,
        EAGAIN => NAGI_STRERROR_TRY_AGAIN,
        ENOMEM => NAGI_STRERROR_NO_MEMORY,
        EACCES => NAGI_STRERROR_PERMISSION,
        EBUSY => NAGI_STRERROR_BUSY,
        EINVAL => NAGI_STRERROR_INVALID,
        ERANGE => NAGI_STRERROR_RANGE,
        ENOSYS => NAGI_STRERROR_NOT_IMPLEMENTED,
        EOVERFLOW => NAGI_STRERROR_OVERFLOW,
        _ => NAGI_STRERROR_UNKNOWN,
    };
    message.as_ptr().cast_mut().cast()
}

static NAGI_RAND_STATE: AtomicU32 = AtomicU32::new(1);

/// Target-local C pseudo-random state.  This is intentionally deterministic
/// until the caller seeds it; entropy-sensitive code must use Nagi getrandom,
/// not `rand`/`srand`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srand(seed: c_uint) {
    NAGI_RAND_STATE.store(seed.max(1), Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rand() -> c_int {
    let mut current = NAGI_RAND_STATE.load(Ordering::Relaxed);
    loop {
        let next = current
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12_345);
        match NAGI_RAND_STATE.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return ((next >> 1) & 0x7fff_ffff) as c_int,
            Err(observed) => current = observed,
        }
    }
}

// Nagi 0.1 exposes guest wall-clock time as UTC and does not import a host
// timezone database. Keep the legacy POSIX global and setter at that explicit
// target contract for freestanding consumers.
#[unsafe(no_mangle)]
pub static mut timezone: c_long = 0;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tzset() {
    unsafe { timezone = 0 };
}

/// Bounded C string length for the Nagi target. The upstream relibc string
/// module is not selected for `target_os = "nagi"`; keep all reads in guest
/// memory and stop at the caller-provided bound.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strnlen(string: *const c_char, maximum: usize) -> usize {
    if string.is_null() {
        return 0;
    }
    let mut length = 0;
    while length < maximum && unsafe { string.cast::<u8>().add(length).read() } != 0 {
        length += 1;
    }
    length
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn div(numerator: c_int, denominator: c_int) -> NagiDivT {
    if denominator == 0 || (numerator == c_int::MIN && denominator == -1) {
        unsafe { abort() };
    }
    NagiDivT {
        quot: numerator / denominator,
        rem: numerator % denominator,
    }
}

/// Basic C string comparison for the Nagi target.  The upstream relibc
/// implementation is not compiled under `target_os = "nagi"`; keep this
/// entry point in the target-owned backend instead of linking a host libc.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcmp(first: *const c_char, second: *const c_char) -> c_int {
    let mut index = 0;
    loop {
        let left = unsafe { *first.cast::<u8>().add(index) };
        let right = unsafe { *second.cast::<u8>().add(index) };
        if left != right {
            return c_int::from(left) - c_int::from(right);
        }
        if left == 0 {
            return 0;
        }
        index += 1;
    }
}

#[inline]
fn nagi_ascii_lower(byte: u8) -> u8 {
    if byte.is_ascii_uppercase() {
        byte + (b'a' - b'A')
    } else {
        byte
    }
}

/// Locale-independent ASCII case-insensitive comparison for the Nagi target.
/// UTF-8 bytes outside ASCII compare bytewise; no host locale table is read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcasecmp(
    first: *const c_char,
    second: *const c_char,
) -> c_int {
    if first.is_null() || second.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }
    let mut index = 0;
    loop {
        let left = nagi_ascii_lower(unsafe { first.cast::<u8>().add(index).read() });
        let right = nagi_ascii_lower(unsafe { second.cast::<u8>().add(index).read() });
        if left != right {
            return c_int::from(left) - c_int::from(right);
        }
        if left == 0 {
            return 0;
        }
        index += 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isalnum(value: c_int) -> c_int {
    if (b'0' as c_int..=b'9' as c_int).contains(&value)
        || (b'a' as c_int..=b'z' as c_int).contains(&value)
        || (b'A' as c_int..=b'Z' as c_int).contains(&value)
    {
        1
    } else {
        0
    }
}

/// Target-owned ASCII lower-case classification. The upstream relibc ctype
/// module is not selected for `target_os = "nagi"`; keep the C locale rule in
/// the guest backend instead of importing a host locale table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn islower(value: c_int) -> c_int {
    c_int::from((b'a' as c_int..=b'z' as c_int).contains(&value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isspace(value: c_int) -> c_int {
    if matches!(value, 0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20) {
        1
    } else {
        0
    }
}

/// Target-owned forward character search. The Nagi target does not select
/// relibc's upstream string module, so keep the C ABI on the guest memory
/// boundary rather than importing a host libc implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strchr(string: *const c_char, needle: c_int) -> *mut c_char {
    if string.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    let needle = needle as u8;
    let mut index = 0;
    loop {
        let current = unsafe { string.add(index).read() as u8 };
        if current == needle {
            return unsafe { string.add(index).cast_mut() };
        }
        if current == 0 {
            return ptr::null_mut();
        }
        index += 1;
    }
}

/// Target-owned reverse character search, including the terminating NUL when
/// requested with `needle == 0` as required by the C string contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strrchr(string: *const c_char, needle: c_int) -> *mut c_char {
    if string.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    let needle = needle as u8;
    let mut last = ptr::null_mut();
    let mut index = 0;
    loop {
        let current = unsafe { string.add(index).read() as u8 };
        if current == needle {
            last = unsafe { string.add(index).cast_mut() };
        }
        if current == 0 {
            return last;
        }
        index += 1;
    }
}

/// Target-owned NUL-terminated substring search for the Nagi relibc backend.
/// The search stays inside guest memory and returns the first haystack
/// position that contains the complete needle, including the empty-needle
/// contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strstr(
    haystack: *const c_char,
    needle: *const c_char,
) -> *mut c_char {
    if haystack.is_null() || needle.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    let mut needle_length = 0;
    while unsafe { needle.add(needle_length).read() } != 0 {
        needle_length += 1;
    }
    if needle_length == 0 {
        return haystack.cast_mut();
    }

    let mut position = 0;
    loop {
        if unsafe { haystack.add(position).read() } == 0 {
            return ptr::null_mut();
        }
        let mut index = 0;
        while index < needle_length {
            let haystack_byte = unsafe { haystack.add(position + index).read() };
            let needle_byte = unsafe { needle.add(index).read() };
            if haystack_byte == 0 || haystack_byte != needle_byte {
                break;
            }
            index += 1;
        }
        if index == needle_length {
            return unsafe { haystack.add(position).cast_mut() };
        }
        position += 1;
    }
}

/// Return the length of the initial guest-memory segment containing none of
/// the reject bytes. The scan is bounded only by the C NUL terminators and
/// never consults host string routines.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcspn(
    input: *const c_char,
    reject: *const c_char,
) -> usize {
    if input.is_null() || reject.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }
    let mut length = 0;
    loop {
        let current = unsafe { input.cast::<u8>().add(length).read() };
        if current == 0 {
            return length;
        }
        let mut reject_index = 0;
        loop {
            let rejected = unsafe { reject.cast::<u8>().add(reject_index).read() };
            if rejected == 0 {
                break;
            }
            if rejected == current {
                return length;
            }
            reject_index += 1;
        }
        length += 1;
    }
}

/// Return the length of the initial guest-memory segment containing only
/// bytes from `accept`.  This is the companion operation to `strcspn` used by
/// Mesa's XML/configuration parser; it never consults a host libc table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strspn(
    input: *const c_char,
    accept: *const c_char,
) -> usize {
    if input.is_null() || accept.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0;
    }
    let mut length = 0;
    loop {
        let current = unsafe { input.cast::<u8>().add(length).read() };
        if current == 0 {
            return length;
        }
        let mut accept_index = 0;
        let mut matched = false;
        loop {
            let accepted = unsafe { accept.cast::<u8>().add(accept_index).read() };
            if accepted == 0 {
                break;
            }
            if accepted == current {
                matched = true;
                break;
            }
            accept_index += 1;
        }
        if !matched {
            return length;
        }
        length += 1;
    }
}

/// Fortified target memory set.  The destination object size is supplied by
/// the caller's checked libc expansion; reject an inconsistent request at the
/// Nagi process boundary instead of writing beyond the guest object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __memset_chk(
    destination: *mut c_void,
    value: c_int,
    length: usize,
    destination_length: usize,
) -> *mut c_void {
    if length > destination_length {
        unsafe { abort() };
    }
    if length == 0 {
        return destination;
    }
    if destination.is_null() {
        unsafe { abort() };
    }
    unsafe {
        ptr::write_bytes(destination.cast::<u8>(), value as u8, length);
    }
    destination
}

/// Fortified overlap-safe target memory move. `ptr::copy` is Rust's checked
/// implementation of the C `memmove` overlap semantics and remains entirely
/// within the supplied guest pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __memmove_chk(
    destination: *mut c_void,
    source: *const c_void,
    length: usize,
    destination_length: usize,
) -> *mut c_void {
    if length > destination_length {
        unsafe { abort() };
    }
    if length == 0 {
        return destination;
    }
    if destination.is_null() || source.is_null() {
        unsafe { abort() };
    }
    unsafe {
        ptr::copy(source.cast::<u8>(), destination.cast::<u8>(), length);
    }
    destination
}

/// Target-owned NUL-terminated copy for the Nagi relibc backend.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcpy(
    destination: *mut c_char,
    source: *const c_char,
) -> *mut c_char {
    if destination.is_null() || source.is_null() {
        unsafe { set_errno(EINVAL) };
        return destination;
    }
    let mut index = 0;
    loop {
        let byte = unsafe { source.add(index).read() };
        unsafe { destination.add(index).write(byte) };
        if byte == 0 {
            return destination;
        }
        index += 1;
    }
}

/// Target-owned bounded string copy.  Match the C contract: copy at most `n`
/// source bytes and pad the remainder with NUL bytes when the source ends
/// early.  All accesses remain in guest memory; no host libc is involved.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncpy(
    destination: *mut c_char,
    source: *const c_char,
    length: usize,
) -> *mut c_char {
    if length == 0 {
        return destination;
    }
    if destination.is_null() || source.is_null() {
        unsafe { set_errno(EINVAL) };
        return destination;
    }
    let mut index = 0;
    while index < length {
        let byte = unsafe { source.add(index).read() };
        unsafe { destination.add(index).write(byte) };
        index += 1;
        if byte == 0 {
            while index < length {
                unsafe { destination.add(index).write(0) };
                index += 1;
            }
            break;
        }
    }
    destination
}

/// Duplicate a NUL-terminated string through the Nagi allocator.  The target
/// copy is bounded by the guest string contract and can be released with the
/// matching target `free`; it never crosses into a host allocator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strdup(source: *const c_char) -> *mut c_char {
    let Some(length) = (unsafe { c_string_len(source, 16 * 1024 * 1024) }) else {
        unsafe { set_errno(if source.is_null() { EINVAL } else { EOVERFLOW }) };
        return ptr::null_mut();
    };
    let Some(allocation_length) = length.checked_add(1) else {
        unsafe { set_errno(EOVERFLOW) };
        return ptr::null_mut();
    };
    let allocation = unsafe { nagi_posix_malloc(allocation_length) }.cast::<c_char>();
    if allocation.is_null() {
        unsafe { set_errno(ENOMEM) };
        return ptr::null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(source, allocation, allocation_length);
    }
    allocation
}

/// Duplicate at most `length` bytes from a guest NUL-terminated string.  The
/// result is always NUL-terminated and allocated by the Nagi allocator, so it
/// remains compatible with the target `free` boundary used by Mesa.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strndup(source: *const c_char, length: usize) -> *mut c_char {
    if source.is_null() {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    let mut source_length = 0;
    while source_length < length
        && unsafe { source.cast::<u8>().add(source_length).read() } != 0
    {
        source_length += 1;
    }
    let Some(allocation_length) = source_length.checked_add(1) else {
        unsafe { set_errno(EOVERFLOW) };
        return ptr::null_mut();
    };
    let allocation = unsafe { nagi_posix_malloc(allocation_length) }.cast::<c_char>();
    if allocation.is_null() {
        unsafe { set_errno(ENOMEM) };
        return ptr::null_mut();
    }
    unsafe {
        ptr::copy_nonoverlapping(source, allocation, source_length);
        allocation.add(source_length).write(0);
    }
    allocation
}

/// Target-owned byte search for the Nagi relibc backend. The upstream string
/// module is not selected for `target_os = "nagi"`; keep this bounded loop
/// independent of host libc or the registry `memchr` implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memchr(
    haystack: *const c_void,
    needle: c_int,
    length: usize,
) -> *mut c_void {
    if haystack.is_null() || length == 0 {
        return ptr::null_mut();
    }
    let bytes = unsafe { slice::from_raw_parts(haystack.cast::<u8>(), length) };
    let needle = needle as u8;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == needle {
            return unsafe { haystack.cast::<u8>().add(index).cast_mut().cast() };
        }
    }
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strcat(destination: *mut c_char, source: *const c_char) -> *mut c_char {
    if destination.is_null() || source.is_null() {
        unsafe { set_errno(EINVAL) };
        return destination;
    }
    let mut destination_length = 0;
    while unsafe { destination.add(destination_length).read() } != 0 {
        destination_length += 1;
    }
    let mut source_index = 0;
    loop {
        let byte = unsafe { source.add(source_index).read() };
        unsafe {
            destination
                .add(destination_length + source_index)
                .write(byte)
        };
        if byte == 0 {
            break;
        }
        source_index += 1;
    }
    destination
}

/// Target-owned bounded string append. Both strings remain in guest memory;
/// no host libc string routine is consulted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncat(
    destination: *mut c_char,
    source: *const c_char,
    maximum: usize,
) -> *mut c_char {
    if destination.is_null() || source.is_null() {
        unsafe { set_errno(EINVAL) };
        return destination;
    }
    let mut destination_length = 0;
    while unsafe { destination.add(destination_length).read() } != 0 {
        destination_length += 1;
    }
    let mut source_index = 0;
    while source_index < maximum {
        let byte = unsafe { source.add(source_index).read() };
        unsafe {
            destination
                .add(destination_length + source_index)
                .write(byte);
        }
        source_index += 1;
        if byte == 0 {
            return destination;
        }
    }
    unsafe {
        destination
            .add(destination_length + source_index)
            .write(0);
    }
    destination
}

type BsearchComparator = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsearch(
    key: *const c_void,
    base: *const c_void,
    count: usize,
    size: usize,
    comparator: Option<BsearchComparator>,
) -> *mut c_void {
    if key.is_null() || base.is_null() || size == 0 || comparator.is_none() {
        return ptr::null_mut();
    }
    let comparator = comparator.expect("checked comparator");
    let mut first = 0;
    let mut remaining = count;
    while remaining != 0 {
        let middle = first + remaining / 2;
        let Some(offset) = middle.checked_mul(size) else {
            return ptr::null_mut();
        };
        let candidate = unsafe { base.cast::<u8>().add(offset).cast::<c_void>() };
        let comparison = unsafe { comparator(key, candidate) };
        if comparison == 0 {
            return candidate.cast_mut();
        }
        if comparison < 0 {
            remaining /= 2;
        } else {
            first = middle + 1;
            remaining -= remaining / 2 + 1;
        }
    }
    ptr::null_mut()
}

type QsortComparator = extern "C" fn(*const c_void, *const c_void) -> c_int;

/// Bounded in-place sorting for the target backend. The standard relibc
/// sorting module is excluded from the Nagi target, so use a deterministic
/// selection sort with byte swaps and no host allocator dependency. The
/// comparator and element storage remain caller-owned, as required by qsort.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qsort(
    base: *mut c_void,
    count: usize,
    width: usize,
    comparator: Option<QsortComparator>,
) {
    let Some(comparator) = comparator else {
        return;
    };
    if base.is_null() || count < 2 || width == 0 {
        return;
    }

    for index in 0..count - 1 {
        let Some(index_offset) = index.checked_mul(width) else {
            return;
        };
        let mut selected = index;
        for candidate in index + 1..count {
            let Some(candidate_offset) = candidate.checked_mul(width) else {
                return;
            };
            let Some(selected_offset) = selected.checked_mul(width) else {
                return;
            };
            let candidate_pointer = unsafe { base.cast::<u8>().add(candidate_offset) };
            let selected_pointer = unsafe { base.cast::<u8>().add(selected_offset) };
            if comparator(candidate_pointer.cast(), selected_pointer.cast()) < 0 {
                selected = candidate;
            }
        }
        if selected == index {
            continue;
        }
        let Some(selected_offset) = selected.checked_mul(width) else {
            return;
        };
        let first = unsafe { base.cast::<u8>().add(index_offset) };
        let second = unsafe { base.cast::<u8>().add(selected_offset) };
        for offset in 0..width {
            unsafe {
                let byte = first.add(offset).read();
                first.add(offset).write(second.add(offset).read());
                second.add(offset).write(byte);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strncmp(
    first: *const c_char,
    second: *const c_char,
    length: usize,
) -> c_int {
    let mut index = 0;
    while index < length {
        let left = unsafe { *first.cast::<u8>().add(index) };
        let right = unsafe { *second.cast::<u8>().add(index) };
        if left != right {
            return c_int::from(left) - c_int::from(right);
        }
        if left == 0 {
            return 0;
        }
        index += 1;
    }
    0
}

static GAI_BADFLAGS: &[u8] = b"Invalid flags\0";
static GAI_NONAME: &[u8] = b"Name does not resolve\0";
static GAI_AGAIN: &[u8] = b"Try again\0";
static GAI_FAIL: &[u8] = b"Non-recoverable error\0";
static GAI_NODATA: &[u8] = b"Unknown error\0";
static GAI_FAMILY: &[u8] = b"Unrecognized address family or invalid length\0";
static GAI_SOCKTYPE: &[u8] = b"Unrecognized socket type\0";
static GAI_SERVICE: &[u8] = b"Unrecognized service\0";
static GAI_ADDRFAMILY: &[u8] = b"Address family for name not supported\0";
static GAI_MEMORY: &[u8] = b"Out of memory\0";
static GAI_SYSTEM: &[u8] = b"System error\0";
static GAI_OVERFLOW: &[u8] = b"Overflow\0";

/// Return the target-owned resolver diagnostic strings.  The Nagi target does
/// not compile relibc's netdb module, so this pure ABI table belongs here
/// rather than being supplied by a host resolver library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gai_strerror(error: c_int) -> *const c_char {
    let message = match error {
        -1 => GAI_BADFLAGS,
        -2 => GAI_NONAME,
        -3 => GAI_AGAIN,
        -4 => GAI_FAIL,
        -5 => GAI_NODATA,
        -6 => GAI_FAMILY,
        -7 => GAI_SOCKTYPE,
        -8 => GAI_SERVICE,
        -9 => GAI_ADDRFAMILY,
        -10 => GAI_MEMORY,
        -11 => GAI_SYSTEM,
        -12 => GAI_OVERFLOW,
        _ => GAI_NODATA,
    };
    message.as_ptr().cast()
}

const NAGI_POW_NAN: c_double = f64::from_bits(0x7ff8_0000_0000_0000);
const NAGI_POW_INF: c_double = f64::from_bits(0x7ff0_0000_0000_0000);
const NAGI_POW_LN2: c_double = 0.6931471805599453;
const NAGI_POW_TWO53: c_double = 9_007_199_254_740_992.0;
const NAGI_PI: c_double = 3.141592653589793;
const NAGI_TWO_PI: c_double = 6.283185307179586;
const NAGI_HALF_PI: c_double = 1.5707963267948966;

#[inline]
fn nagi_pow_is_nan(value: c_double) -> bool {
    (value.to_bits() & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000
        && (value.to_bits() & 0x000f_ffff_ffff_ffff) != 0
}

#[inline]
fn nagi_pow_is_inf(value: c_double) -> bool {
    (value.to_bits() & 0x7fff_ffff_ffff_ffff) == 0x7ff0_0000_0000_0000
}

#[inline]
fn nagi_pow_abs(value: c_double) -> c_double {
    c_double::from_bits(value.to_bits() & 0x7fff_ffff_ffff_ffff)
}

#[inline]
fn nagi_pow_is_negative(value: c_double) -> bool {
    (value.to_bits() >> 63) != 0
}

fn nagi_pow_integer_info(value: c_double) -> (bool, bool) {
    let absolute = nagi_pow_abs(value);
    if absolute >= NAGI_POW_TWO53 {
        return (true, false);
    }
    let integer = value as i64;
    ((integer as c_double) == value, (integer & 1) != 0)
}

fn nagi_pow_ln_positive(value: c_double) -> c_double {
    let mut bits = value.to_bits();
    let raw_exponent = ((bits >> 52) & 0x7ff) as i32;
    let exponent = if raw_exponent == 0 {
        bits = (value * NAGI_POW_TWO53).to_bits();
        (((bits >> 52) & 0x7ff) as i32 - 1023) - 52
    } else {
        raw_exponent - 1023
    };
    let mantissa = c_double::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let z = (mantissa - 1.0) / (mantissa + 1.0);
    let z_squared = z * z;
    let mut term = z;
    let mut sum = 0.0;
    let mut denominator = 1.0;
    let mut index = 0;
    while index < 24 {
        sum += term / denominator;
        term *= z_squared;
        denominator += 2.0;
        index += 1;
    }
    (exponent as c_double) * NAGI_POW_LN2 + 2.0 * sum
}

fn nagi_pow_exp(value: c_double) -> c_double {
    if value > 709.782712893384 || nagi_pow_is_inf(value) {
        return NAGI_POW_INF;
    }
    if value < -745.1332191019411 {
        return 0.0;
    }
    let scaled = value / NAGI_POW_LN2;
    let exponent = if scaled >= 0.0 {
        (scaled + 0.5) as i32
    } else {
        (scaled - 0.5) as i32
    };
    let reduced = value - (exponent as c_double) * NAGI_POW_LN2;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut divisor = 1.0;
    let mut index = 1;
    while index <= 24 {
        divisor *= index as c_double;
        term *= reduced;
        sum += term / divisor;
        index += 1;
    }
    if exponent > 1023 {
        return NAGI_POW_INF;
    }
    if exponent < -1074 {
        return 0.0;
    }
    let scale = if exponent >= -1022 {
        c_double::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        c_double::from_bits(1_u64 << (exponent + 1074))
    };
    sum * scale
}

fn nagi_pow_real(base: c_double, exponent: c_double) -> c_double {
    if exponent == 0.0 || base == 1.0 {
        return 1.0;
    }
    if nagi_pow_is_nan(base) || nagi_pow_is_nan(exponent) {
        return NAGI_POW_NAN;
    }
    let base_negative = nagi_pow_is_negative(base);
    let (exponent_integer, exponent_odd) = nagi_pow_integer_info(exponent);
    if base_negative && !exponent_integer {
        return NAGI_POW_NAN;
    }
    let absolute_base = nagi_pow_abs(base);
    if nagi_pow_is_inf(exponent) {
        if absolute_base == 1.0 {
            return 1.0;
        }
        let grows = absolute_base > 1.0;
        let positive = nagi_pow_is_negative(exponent) != grows;
        let result = if positive { NAGI_POW_INF } else { 0.0 };
        return if base_negative && exponent_odd {
            -result
        } else {
            result
        };
    }
    if absolute_base == 0.0 {
        let result = if nagi_pow_is_negative(exponent) {
            NAGI_POW_INF
        } else {
            0.0
        };
        return if base_negative && exponent_odd {
            -result
        } else {
            result
        };
    }
    if nagi_pow_is_inf(absolute_base) {
        let result = if nagi_pow_is_negative(exponent) {
            0.0
        } else {
            NAGI_POW_INF
        };
        return if base_negative && exponent_odd {
            -result
        } else {
            result
        };
    }
    let magnitude = nagi_pow_exp(exponent * nagi_pow_ln_positive(absolute_base));
    if base_negative && exponent_odd {
        -magnitude
    } else {
        magnitude
    }
}

fn nagi_exp_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) {
        return NAGI_POW_NAN;
    }
    if nagi_pow_is_inf(value) {
        return if value.is_sign_negative() {
            0.0
        } else {
            NAGI_POW_INF
        };
    }
    nagi_pow_exp(value)
}

fn nagi_log_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) || value < 0.0 {
        return NAGI_POW_NAN;
    }
    if value == 0.0 {
        return -NAGI_POW_INF;
    }
    if nagi_pow_is_inf(value) {
        return NAGI_POW_INF;
    }
    nagi_pow_ln_positive(value)
}

fn nagi_tanh_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) {
        return NAGI_POW_NAN;
    }
    if nagi_pow_is_inf(value) {
        return if value.is_sign_negative() { -1.0 } else { 1.0 };
    }

    // Evaluate the stable form (1 - exp(-2*|x|)) / (1 + exp(-2*|x|)) so
    // large finite inputs cannot turn the equivalent exp(2*x) form into
    // infinity minus infinity. Restore the sign, including signed zero,
    // after evaluating the non-negative magnitude.
    let absolute = nagi_pow_abs(value);
    let decay = nagi_exp_real(-2.0 * absolute);
    let magnitude = (1.0 - decay) / (1.0 + decay);
    if value.is_sign_negative() {
        -magnitude
    } else {
        magnitude
    }
}

fn nagi_trig_reduce(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) || nagi_pow_is_inf(value) {
        return NAGI_POW_NAN;
    }
    let turns = if value >= 0.0 {
        (value / NAGI_TWO_PI + 0.5) as i64
    } else {
        (value / NAGI_TWO_PI - 0.5) as i64
    };
    value - (turns as c_double) * NAGI_TWO_PI
}

fn nagi_sin_real(value: c_double) -> c_double {
    let mut reduced = nagi_trig_reduce(value);
    if nagi_pow_is_nan(reduced) {
        return reduced;
    }
    if reduced > NAGI_HALF_PI {
        reduced = NAGI_PI - reduced;
    } else if reduced < -NAGI_HALF_PI {
        reduced = -NAGI_PI - reduced;
    }
    let square = reduced * reduced;
    let mut factor = 1.0 / 3_628_800.0;
    factor = factor * square - 1.0 / 5040.0;
    factor = factor * square + 1.0 / 120.0;
    factor = factor * square - 1.0 / 6.0;
    factor = factor * square + 1.0;
    factor * reduced
}

fn nagi_cos_real(value: c_double) -> c_double {
    let mut reduced = nagi_trig_reduce(value);
    if nagi_pow_is_nan(reduced) {
        return reduced;
    }
    let sign = if reduced > NAGI_HALF_PI {
        reduced = NAGI_PI - reduced;
        -1.0
    } else if reduced < -NAGI_HALF_PI {
        reduced = -NAGI_PI - reduced;
        -1.0
    } else {
        1.0
    };
    let square = reduced * reduced;
    let mut factor = 1.0 / 40_320.0;
    factor = factor * square - 1.0 / 720.0;
    factor = factor * square + 1.0 / 24.0;
    factor = factor * square - 1.0 / 2.0;
    factor = factor * square + 1.0;
    sign * factor
}

fn nagi_sqrt_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) || value < 0.0 {
        return NAGI_POW_NAN;
    }
    if value == 0.0 || nagi_pow_is_inf(value) {
        return value;
    }

    // The inverse-trigonometric helpers call this for [0, 1], while hypot
    // calls it for [1, 2]. Newton's method with a fixed iteration count keeps
    // the implementation freestanding and converges from the upper bound for
    // both bounded intervals.
    let mut estimate = 1.0;
    let mut iteration = 0;
    while iteration < 32 {
        estimate = 0.5 * (estimate + value / estimate);
        iteration += 1;
    }
    estimate
}

fn nagi_hypot_real(first: c_double, second: c_double) -> c_double {
    let first = nagi_pow_abs(first);
    let second = nagi_pow_abs(second);
    if nagi_pow_is_inf(first) || nagi_pow_is_inf(second) {
        // POSIX hypot gives infinity precedence over a NaN companion.
        return NAGI_POW_INF;
    }
    if nagi_pow_is_nan(first) || nagi_pow_is_nan(second) {
        return NAGI_POW_NAN;
    }

    let (largest, smallest) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    if largest == 0.0 {
        return 0.0;
    }

    // Scaling by the largest argument avoids the overflow and underflow that
    // a direct x*x + y*y would introduce before the square root.
    let ratio = smallest / largest;
    largest * nagi_sqrt_real(1.0 + ratio * ratio)
}

fn nagi_atan_reduced(value: c_double) -> c_double {
    // This range is reached after the pi/4 argument reduction below. The
    // alternating Taylor series therefore converges rapidly without libc/libm.
    let square = value * value;
    let mut term = value;
    let mut result = 0.0;
    let mut index = 0;
    while index < 24 {
        let denominator = (index * 2 + 1) as c_double;
        if index & 1 == 0 {
            result += term / denominator;
        } else {
            result -= term / denominator;
        }
        term *= square;
        index += 1;
    }
    result
}

fn nagi_atan_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) {
        return NAGI_POW_NAN;
    }
    if nagi_pow_is_inf(value) {
        return if value.is_sign_negative() {
            -NAGI_HALF_PI
        } else {
            NAGI_HALF_PI
        };
    }

    let negative = value.is_sign_negative();
    let absolute = nagi_pow_abs(value);
    let reduced = if absolute > 1.0 {
        NAGI_HALF_PI - nagi_atan_real(1.0 / absolute)
    } else if absolute > 0.41421356237309503 {
        NAGI_PI / 4.0 + nagi_atan_reduced((absolute - 1.0) / (absolute + 1.0))
    } else {
        nagi_atan_reduced(absolute)
    };
    if negative { -reduced } else { reduced }
}

fn nagi_atan2_real(y: c_double, x: c_double) -> c_double {
    if nagi_pow_is_nan(x) || nagi_pow_is_nan(y) {
        return NAGI_POW_NAN;
    }
    if x > 0.0 {
        return nagi_atan_real(y / x);
    }
    if x < 0.0 {
        let angle = nagi_atan_real(y / x);
        return if y.is_sign_negative() {
            angle - NAGI_PI
        } else {
            angle + NAGI_PI
        };
    }
    if y.is_sign_negative() {
        -NAGI_HALF_PI
    } else if y > 0.0 {
        NAGI_HALF_PI
    } else {
        y
    }
}

fn nagi_asin_real(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) || value < -1.0 || value > 1.0 {
        return NAGI_POW_NAN;
    }
    if value == 1.0 {
        return NAGI_HALF_PI;
    }
    if value == -1.0 {
        return -NAGI_HALF_PI;
    }
    nagi_atan2_real(value, nagi_sqrt_real((1.0 - value) * (1.0 + value)))
}

/// Target-owned freestanding power implementation. The normal relibc math
/// module is excluded for `target_os = "nagi"`; this bounded IEEE-aware
/// implementation keeps Servo/Mesa numeric code off the host libc boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pow(x: c_double, y: c_double) -> c_double {
    nagi_pow_real(x, y)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn powf(x: c_float, y: c_float) -> c_float {
    nagi_pow_real(c_double::from(x), c_double::from(y)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp(x: c_double) -> c_double {
    nagi_exp_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn expf(x: c_float) -> c_float {
    nagi_exp_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp2(x: c_double) -> c_double {
    nagi_exp_real(x * NAGI_POW_LN2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn exp2f(x: c_float) -> c_float {
    nagi_exp_real(c_double::from(x) * NAGI_POW_LN2) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn log(x: c_double) -> c_double {
    nagi_log_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn logf(x: c_float) -> c_float {
    nagi_log_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hypot(x: c_double, y: c_double) -> c_double {
    nagi_hypot_real(x, y)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn hypotf(x: c_float, y: c_float) -> c_float {
    nagi_hypot_real(c_double::from(x), c_double::from(y)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn abs(value: c_int) -> c_int {
    if value < 0 {
        value.wrapping_neg()
    } else {
        value
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sin(x: c_double) -> c_double {
    nagi_sin_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cos(x: c_double) -> c_double {
    nagi_cos_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sinf(x: c_float) -> c_float {
    nagi_sin_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn cosf(x: c_float) -> c_float {
    nagi_cos_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sincosf(
    x: c_float,
    sine: *mut c_float,
    cosine: *mut c_float,
) {
    if sine.is_null() || cosine.is_null() {
        unsafe { abort() };
    }
    unsafe {
        *sine = nagi_sin_real(c_double::from(x)) as c_float;
        *cosine = nagi_cos_real(c_double::from(x)) as c_float;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn asin(x: c_double) -> c_double {
    nagi_asin_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn asinf(x: c_float) -> c_float {
    nagi_asin_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atan(x: c_double) -> c_double {
    nagi_atan_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atanf(x: c_float) -> c_float {
    nagi_atan_real(c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atan2(y: c_double, x: c_double) -> c_double {
    nagi_atan2_real(y, x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atan2f(y: c_float, x: c_float) -> c_float {
    nagi_atan2_real(c_double::from(y), c_double::from(x)) as c_float
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn acos(x: c_double) -> c_double {
    let result = nagi_asin_real(x);
    if nagi_pow_is_nan(result) {
        result
    } else {
        NAGI_HALF_PI - result
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn acosf(x: c_float) -> c_float {
    unsafe { acos(c_double::from(x)) as c_float }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tan(x: c_double) -> c_double {
    let sine = nagi_sin_real(x);
    let cosine = nagi_cos_real(x);
    if nagi_pow_is_nan(sine) || nagi_pow_is_nan(cosine) {
        return NAGI_POW_NAN;
    }
    if cosine == 0.0 {
        return if sine.is_sign_negative() {
            -NAGI_POW_INF
        } else {
            NAGI_POW_INF
        };
    }
    sine / cosine
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tanf(x: c_float) -> c_float {
    unsafe { tan(c_double::from(x)) as c_float }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tanh(x: c_double) -> c_double {
    nagi_tanh_real(x)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tanhf(x: c_float) -> c_float {
    nagi_tanh_real(c_double::from(x)) as c_float
}

/// Target-owned base-2 logarithm. The implementation reuses the same
/// mantissa/exponent reduction as Nagi's real `pow` path, so it does not
/// depend on a host libm or silently return a placeholder value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn log2(value: c_double) -> c_double {
    if nagi_pow_is_nan(value) || value < 0.0 {
        return NAGI_POW_NAN;
    }
    if value == 0.0 {
        return -NAGI_POW_INF;
    }
    if nagi_pow_is_inf(value) {
        return NAGI_POW_INF;
    }
    nagi_pow_ln_positive(value) / NAGI_POW_LN2
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn log2f(value: c_float) -> c_float {
    unsafe { log2(c_double::from(value)) as c_float }
}

/// Target-owned binary exponent scaling. The multiplier is assembled from
/// exact IEEE powers of two, so this path does not import a host libm
/// implementation or approximate an integer exponent through `exp`.
fn nagi_scale_pow2(value: c_double, exponent: c_int) -> c_double {
    if value == 0.0 || nagi_pow_is_nan(value) || nagi_pow_is_inf(value) || exponent == 0 {
        return value;
    }

    let negative = exponent < 0;
    let mut remaining = if negative {
        (-(exponent as i64)) as u32
    } else {
        exponent as u32
    };
    let mut result = value;
    let large_factor = if negative {
        f64::from_bits(0x1ff0_0000_0000_0000)
    } else {
        f64::from_bits(0x5ff0_0000_0000_0000)
    };

    while remaining >= 512 {
        result *= large_factor;
        remaining -= 512;
        if result == 0.0 || result.is_infinite() {
            return result;
        }
    }

    if remaining == 0 {
        return result;
    }
    let factor = if negative {
        if remaining > 1022 {
            0.0
        } else {
            f64::from_bits(((1023 - remaining) as u64) << 52)
        }
    } else {
        f64::from_bits(((1023 + remaining) as u64) << 52)
    };
    result * factor
}

/// Target-owned binary exponent scaling. This uses exact freestanding IEEE
/// powers and therefore does not delegate to a host `exp`/`ldexp` provider.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ldexp(value: c_double, exponent: c_int) -> c_double {
    nagi_scale_pow2(value, exponent)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ldexpf(value: c_float, exponent: c_int) -> c_float {
    unsafe { ldexp(c_double::from(value), exponent) as c_float }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn scalbn(value: c_double, exponent: c_int) -> c_double {
    nagi_scale_pow2(value, exponent)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn scalbnf(value: c_float, exponent: c_int) -> c_float {
    unsafe { scalbn(c_double::from(value), exponent) as c_float }
}

/// Target-owned nearest-integer conversion used by Mesa's color and format
/// helpers. The Nagi target excludes relibc's normal libm module, so expose
/// the bounded C ABI directly instead of leaving `lrintf` to a host libm.
/// This follows the default C round-to-nearest, ties-to-even mode used by the
/// freestanding target; out-of-range and non-finite inputs fail closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrintf(value: c_float) -> c_long {
    if value.is_nan() || value.is_infinite() {
        return 0;
    }

    let truncated = value as c_long;
    let fraction = value - truncated as c_float;
    if fraction > 0.5 {
        truncated.saturating_add(1)
    } else if fraction < -0.5 {
        truncated.saturating_sub(1)
    } else if (fraction == 0.5 || fraction == -0.5) && (truncated & 1) != 0 {
        if value.is_sign_negative() {
            truncated.saturating_sub(1)
        } else {
            truncated.saturating_add(1)
        }
    } else {
        truncated
    }
}

/// Target-owned nearest-integer conversion for double-precision callers.
/// Keep this in the Nagi relibc ABI so Mesa and Servo do not import a host
/// libm provider for `lrint`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lrint(value: c_double) -> c_long {
    if value.is_nan() || value.is_infinite() {
        return 0;
    }

    let truncated = value as c_long;
    let fraction = value - truncated as c_double;
    if fraction > 0.5 {
        truncated.saturating_add(1)
    } else if fraction < -0.5 {
        truncated.saturating_sub(1)
    } else if (fraction == 0.5 || fraction == -0.5) && (truncated & 1) != 0 {
        if value.is_sign_negative() {
            truncated.saturating_sub(1)
        } else {
            truncated.saturating_add(1)
        }
    } else {
        truncated
    }
}

/// Target-owned long-long variant of `lrint`; it follows the same default
/// round-to-nearest, ties-to-even mode as the target's `lrint` entry point.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn llrint(value: c_double) -> c_longlong {
    if value.is_nan() || value.is_infinite() {
        return 0;
    }

    let truncated = value as c_longlong;
    let fraction = value - truncated as c_double;
    if fraction > 0.5 {
        truncated.saturating_add(1)
    } else if fraction < -0.5 {
        truncated.saturating_sub(1)
    } else if (fraction == 0.5 || fraction == -0.5) && (truncated & 1) != 0 {
        if value.is_sign_negative() {
            truncated.saturating_sub(1)
        } else {
            truncated.saturating_add(1)
        }
    } else {
        truncated
    }
}

/// Target-owned floating-point predicates used by Mesa's freestanding math
/// and format code. Keep both the POSIX spelling and openlibm's float helper
/// in the Nagi ABI; neither is delegated to a host libm.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isnan(value: c_double) -> c_int {
    if value.is_nan() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __isnanf(value: c_float) -> c_int {
    if value.is_nan() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __isfinite(value: c_double) -> c_int {
    if !nagi_pow_is_nan(value) && !nagi_pow_is_inf(value) {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __isfinitef(value: c_float) -> c_int {
    unsafe { __isfinite(c_double::from(value)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __isnormal(value: c_double) -> c_int {
    let exponent = (value.to_bits() >> 52) & 0x7ff;
    if exponent != 0 && exponent != 0x7ff { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __isnormalf(value: c_float) -> c_int {
    let exponent = (value.to_bits() >> 23) & 0xff;
    if exponent != 0 && exponent != 0xff { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isfinite(value: c_double) -> c_int {
    unsafe { __isfinite(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isfinitef(value: c_float) -> c_int {
    unsafe { __isfinitef(value) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isnanf(value: c_float) -> c_int {
    unsafe { __isnanf(value) }
}

fn nagi_frexp_real(value: c_double, exponent: *mut c_int) -> c_double {
    if exponent.is_null() {
        unsafe { abort() };
    }
    if value == 0.0 || value.is_nan() || value.is_infinite() {
        unsafe { *exponent = 0 };
        return value;
    }

    let mut bits = value.to_bits();
    let mut scale_exponent = 0;
    let mut exponent_bits = (bits >> 52) & 0x7ff;
    if exponent_bits == 0 {
        // Scale subnormal values into the normal range before extracting the
        // IEEE exponent. This is a pure target arithmetic operation, not a
        // delegation to libm or a host floating-point provider.
        bits = (value * 18_014_398_509_481_984.0).to_bits();
        scale_exponent = -54;
        exponent_bits = (bits >> 52) & 0x7ff;
    }

    let fraction = bits & ((1_u64 << 52) - 1);
    let normalized = f64::from_bits((bits & (1_u64 << 63)) | (1022_u64 << 52) | fraction);
    unsafe { *exponent = exponent_bits as c_int - 1022 + scale_exponent };
    normalized
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexp(value: c_double, exponent: *mut c_int) -> c_double {
    nagi_frexp_real(value, exponent)
}

fn nagi_frexpf_real(value: c_float, exponent: *mut c_int) -> c_float {
    if exponent.is_null() {
        unsafe { abort() };
    }
    if value == 0.0 || value.is_nan() || value.is_infinite() {
        unsafe { *exponent = 0 };
        return value;
    }

    let mut bits = value.to_bits();
    let mut scale_exponent = 0;
    let mut exponent_bits = (bits >> 23) & 0xff;
    if exponent_bits == 0 {
        bits = (value * 33_554_432.0).to_bits();
        scale_exponent = -25;
        exponent_bits = (bits >> 23) & 0xff;
    }

    let fraction = bits & ((1_u32 << 23) - 1);
    let normalized = f32::from_bits((bits & (1_u32 << 31)) | (126_u32 << 23) | fraction);
    unsafe { *exponent = exponent_bits as c_int - 126 + scale_exponent };
    normalized
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn frexpf(value: c_float, exponent: *mut c_int) -> c_float {
    nagi_frexpf_real(value, exponent)
}

/// Nagi's target ctype contract is ASCII/UTF-8 and locale-independent for
/// the M17 freestanding path. Keep the C ABI result independent of host
/// locale tables.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isdigit(value: c_int) -> c_int {
    if (b'0' as c_int..=b'9' as c_int).contains(&value) {
        1
    } else {
        0
    }
}

#[inline]
fn nagi_lroundf_real(value: c_float) -> c_longlong {
    if value.is_nan() || value.is_infinite() {
        return 0;
    }
    let truncated = value as c_longlong;
    let fraction = value - truncated as c_float;
    if fraction >= 0.5 {
        truncated.saturating_add(1)
    } else if fraction <= -0.5 {
        truncated.saturating_sub(1)
    } else {
        truncated
    }
}

#[inline]
fn nagi_lround_real(value: c_double) -> c_longlong {
    if value.is_nan() || value.is_infinite() {
        return 0;
    }
    let truncated = value as c_longlong;
    let fraction = value - truncated as c_double;
    if fraction >= 0.5 {
        truncated.saturating_add(1)
    } else if fraction <= -0.5 {
        truncated.saturating_sub(1)
    } else {
        truncated
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lroundf(value: c_float) -> c_long {
    nagi_lroundf_real(value) as c_long
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lround(value: c_double) -> c_long {
    nagi_lround_real(value) as c_long
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn llround(value: c_double) -> c_longlong {
    nagi_lround_real(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn accept(
    socket: c_int,
    address: *mut c_void,
    address_len: *mut c_uint,
) -> c_int {
    unsafe { nagi_posix_accept(socket, address, address_len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getsockopt(
    socket: c_int,
    level: c_int,
    option_name: c_int,
    option_value: *mut c_void,
    option_len: *mut c_uint,
) -> c_int {
    unsafe { nagi_posix_getsockopt(socket, level, option_name, option_value, option_len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, buf: *mut c_void) -> c_int {
    unsafe { nagi_posix_lstat(path, buf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn isatty(fd: c_int) -> c_int {
    unsafe { nagi_posix_isatty(fd) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(fd: c_int, path: *const c_char, flags: c_int, _args: ...) -> c_int {
    unsafe { nagi_posix_openat(fd, path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(path: *const c_char) -> c_int {
    unsafe { nagi_posix_unlink(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlinkat(fd: c_int, path: *const c_char, flags: c_int) -> c_int {
    unsafe { nagi_posix_unlinkat(fd, path, flags) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: c_uint) -> c_int {
    unsafe { nagi_posix_mkdir(path, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> c_int {
    unsafe { nagi_posix_rmdir(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut c_void {
    unsafe { nagi_posix_opendir(path) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir(directory: *mut c_void) -> *mut c_void {
    unsafe { nagi_posix_readdir(directory) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir_r(
    directory: *mut c_void,
    entry: *mut c_void,
    result: *mut *mut c_void,
) -> c_int {
    unsafe { nagi_posix_readdir_r(directory, entry, result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn closedir(directory: *mut c_void) -> c_int {
    unsafe { nagi_posix_closedir(directory) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fdopendir(fd: c_int) -> *mut c_void {
    unsafe { nagi_posix_fdopendir(fd) }
}

/// Nagi creates processes through its spawn-oriented service boundary and does
/// not provide Unix exec-in-place semantics.  Returning the real ENOSYS error
/// keeps execvp fail-closed instead of pretending that a guest process was
/// replaced or routing execution through the host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn execvp(_file: *const c_char, _argv: *const *const c_char) -> c_int {
    unsafe { set_errno(ENOSYS) };
    -1
}

#[derive(Clone, Copy, Default)]
struct NagiFormatSpec {
    alternate: bool,
    left: bool,
    plus: bool,
    space: bool,
    zero: bool,
    width: Option<usize>,
    precision: Option<usize>,
    longness: u8,
}

struct NagiFormatWriter {
    output: *mut u8,
    capacity: usize,
    written: usize,
}

impl NagiFormatWriter {
    fn new(output: *mut c_char, capacity: usize) -> Self {
        Self {
            output: output.cast(),
            capacity,
            written: 0,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        let Some(next) = self.written.checked_add(bytes.len()) else {
            self.written = usize::MAX;
            return;
        };
        if !self.output.is_null() && self.capacity > 0 {
            let writable = (self.capacity - 1).saturating_sub(self.written);
            let amount = writable.min(bytes.len());
            if amount != 0 {
                unsafe {
                    ptr::copy_nonoverlapping(bytes.as_ptr(), self.output.add(self.written), amount);
                }
            }
        }
        self.written = next;
    }

    fn finish(self) -> c_int {
        let count = self.written.min(c_int::MAX as usize) as c_int;
        if !self.output.is_null() && self.capacity > 0 {
            let end = self.written.min(self.capacity - 1);
            unsafe { self.output.add(end).write(0) };
        }
        count
    }
}

impl fmt::Write for NagiFormatWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_bytes(value.as_bytes());
        Ok(())
    }
}

struct NagiFormatBuffer {
    bytes: [u8; 256],
    length: usize,
}

impl NagiFormatBuffer {
    fn new() -> Self {
        Self {
            bytes: [0; 256],
            length: 0,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

impl fmt::Write for NagiFormatBuffer {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let amount = (self.bytes.len() - self.length).min(value.len());
        self.bytes[self.length..self.length + amount].copy_from_slice(&value.as_bytes()[..amount]);
        self.length += amount;
        Ok(())
    }
}

fn nagi_emit_padding(writer: &mut NagiFormatWriter, byte: u8, count: usize) {
    for _ in 0..count {
        writer.write_bytes(core::slice::from_ref(&byte));
    }
}

fn nagi_emit_field(
    writer: &mut NagiFormatWriter,
    prefix: &[u8],
    body: &[u8],
    spec: NagiFormatSpec,
) {
    let total = prefix.len().saturating_add(body.len());
    let padding = spec.width.unwrap_or(0).saturating_sub(total);
    if !spec.left && !spec.zero {
        nagi_emit_padding(writer, b' ', padding);
    }
    writer.write_bytes(prefix);
    if !spec.left && spec.zero {
        nagi_emit_padding(writer, b'0', padding);
    }
    writer.write_bytes(body);
    if spec.left {
        nagi_emit_padding(writer, b' ', padding);
    }
}

fn nagi_emit_unsigned(
    writer: &mut NagiFormatWriter,
    value: u64,
    negative: bool,
    base: u8,
    uppercase: bool,
    spec: NagiFormatSpec,
    pointer: bool,
) {
    let mut digits = [0u8; 64];
    let mut length = 0;
    let alphabet = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    if value == 0 {
        digits[0] = b'0';
        length = 1;
    } else {
        let mut remaining = value;
        while remaining != 0 {
            digits[length] = alphabet[(remaining % u64::from(base)) as usize];
            length += 1;
            remaining /= u64::from(base);
        }
        digits[..length].reverse();
    }
    let precision = spec.precision.unwrap_or(0).max(length);
    let mut body = [0u8; 64];
    let body_length = precision.min(body.len());
    let leading = body_length.saturating_sub(length);
    body[..leading].fill(b'0');
    body[leading..body_length].copy_from_slice(&digits[..length.min(body_length)]);
    let sign = if negative {
        b'-'
    } else if spec.plus {
        b'+'
    } else if spec.space {
        b' '
    } else {
        0
    };
    let mut prefix = [0u8; 3];
    let prefix_length = if sign != 0 {
        prefix[0] = sign;
        1
    } else {
        0
    };
    let alternate_length = if pointer || (spec.alternate && base == 16 && value != 0) {
        prefix[prefix_length] = b'0';
        prefix[prefix_length + 1] = if uppercase { b'X' } else { b'x' };
        2
    } else if spec.alternate && base == 8 && body_length > 0 && body[0] != b'0' {
        prefix[prefix_length] = b'0';
        1
    } else {
        0
    };
    nagi_emit_field(
        writer,
        &prefix[..prefix_length + alternate_length],
        &body[..body_length],
        NagiFormatSpec {
            zero: spec.zero && spec.precision.is_none(),
            ..spec
        },
    );
}

unsafe fn nagi_emit_c_string(
    writer: &mut NagiFormatWriter,
    value: *const c_char,
    precision: Option<usize>,
    spec: NagiFormatSpec,
) {
    let fallback = b"(null)";
    let pointer = if value.is_null() {
        fallback.as_ptr()
    } else {
        value.cast::<u8>()
    };
    let mut length = 0;
    while precision.is_none_or(|limit| length < limit) && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    let padding = spec.width.unwrap_or(0).saturating_sub(length);
    if !spec.left {
        nagi_emit_padding(writer, b' ', padding);
    }
    for index in 0..length {
        writer.write_bytes(unsafe { core::slice::from_raw_parts(pointer.add(index), 1) });
    }
    if spec.left {
        nagi_emit_padding(writer, b' ', padding);
    }
}

unsafe fn nagi_vsnprintf(
    output: *mut c_char,
    capacity: usize,
    format: *const c_char,
    mut args: VaList,
) -> c_int {
    let mut writer = NagiFormatWriter::new(output, capacity);
    let mut index = 0;
    loop {
        let byte = unsafe { *format.cast::<u8>().add(index) };
        if byte == 0 {
            break;
        }
        if byte != b'%' {
            writer.write_bytes(core::slice::from_ref(&byte));
            index += 1;
            continue;
        }
        index += 1;
        let mut spec = NagiFormatSpec::default();
        loop {
            let flag = unsafe { *format.cast::<u8>().add(index) };
            match flag {
                b'#' => spec.alternate = true,
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'0' => spec.zero = true,
                _ => break,
            }
            index += 1;
        }
        if unsafe { *format.cast::<u8>().add(index) } == b'*' {
            let width = unsafe { args.arg::<c_int>() };
            if width < 0 {
                spec.left = true;
                spec.width = Some(width.unsigned_abs() as usize);
            } else {
                spec.width = Some(width as usize);
            }
            index += 1;
        } else {
            let mut width = 0usize;
            while unsafe { *format.cast::<u8>().add(index) }.is_ascii_digit() {
                width = width.saturating_mul(10).saturating_add(usize::from(unsafe {
                    *format.cast::<u8>().add(index) - b'0'
                }));
                index += 1;
            }
            if width != 0 {
                spec.width = Some(width);
            }
        }
        if unsafe { *format.cast::<u8>().add(index) } == b'.' {
            index += 1;
            if unsafe { *format.cast::<u8>().add(index) } == b'*' {
                let precision = unsafe { args.arg::<c_int>() };
                if precision >= 0 {
                    spec.precision = Some(precision as usize);
                }
                index += 1;
            } else {
                let mut precision = 0usize;
                while unsafe { *format.cast::<u8>().add(index) }.is_ascii_digit() {
                    precision = precision
                        .saturating_mul(10)
                        .saturating_add(usize::from(unsafe {
                            *format.cast::<u8>().add(index) - b'0'
                        }));
                    index += 1;
                }
                spec.precision = Some(precision);
            }
        }
        let length = unsafe { *format.cast::<u8>().add(index) };
        match length {
            b'h' => {
                spec.longness = 1;
                index += 1;
                if unsafe { *format.cast::<u8>().add(index) } == b'h' {
                    spec.longness = 2;
                    index += 1;
                }
            }
            b'l' => {
                spec.longness = 3;
                index += 1;
                if unsafe { *format.cast::<u8>().add(index) } == b'l' {
                    spec.longness = 4;
                    index += 1;
                }
            }
            b'z' | b'j' | b't' => {
                spec.longness = 3;
                index += 1;
            }
            _ => {}
        }
        let conversion = unsafe { *format.cast::<u8>().add(index) };
        if conversion == 0 {
            break;
        }
        index += 1;
        match conversion {
            b'%' => writer.write_bytes(b"%"),
            b's' => unsafe {
                nagi_emit_c_string(
                    &mut writer,
                    args.arg::<*const c_char>(),
                    spec.precision,
                    spec,
                )
            },
            b'c' => {
                let value = (unsafe { args.arg::<c_int>() }) as u8;
                nagi_emit_field(&mut writer, &[], core::slice::from_ref(&value), spec);
            }
            b'd' | b'i' => {
                let value = match spec.longness {
                    3 | 4 => (unsafe { args.arg::<c_longlong>() }) as i64,
                    _ => (unsafe { args.arg::<c_int>() }) as i64,
                };
                nagi_emit_unsigned(
                    &mut writer,
                    value.unsigned_abs(),
                    value < 0,
                    10,
                    false,
                    spec,
                    false,
                );
            }
            b'u' | b'o' | b'x' | b'X' => {
                let value = match spec.longness {
                    3 | 4 => (unsafe { args.arg::<c_ulonglong>() }) as u64,
                    _ => (unsafe { args.arg::<c_uint>() }) as u64,
                };
                let base = if conversion == b'o' {
                    8
                } else if conversion == b'u' {
                    10
                } else {
                    16
                };
                nagi_emit_unsigned(
                    &mut writer,
                    value,
                    false,
                    base,
                    conversion == b'X',
                    spec,
                    false,
                );
            }
            b'p' => {
                let value = unsafe { args.arg::<*const c_void>() } as usize as u64;
                nagi_emit_unsigned(&mut writer, value, false, 16, false, spec, true);
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                let value = unsafe { args.arg::<c_double>() };
                let mut buffer = NagiFormatBuffer::new();
                let precision = spec.precision.unwrap_or(6);
                match conversion {
                    b'e' => {
                        let _ = fmt::write(&mut buffer, format_args!("{:.*e}", precision, value));
                    }
                    b'E' => {
                        let _ = fmt::write(&mut buffer, format_args!("{:.*E}", precision, value));
                    }
                    _ => {
                        let _ = fmt::write(&mut buffer, format_args!("{:.*}", precision, value));
                    }
                }
                let mut prefix = [0u8; 1];
                let mut body = buffer.as_slice();
                if body.first() == Some(&b'-') {
                    prefix[0] = b'-';
                    body = &body[1..];
                } else if spec.plus {
                    prefix[0] = b'+';
                } else if spec.space {
                    prefix[0] = b' ';
                }
                nagi_emit_field(
                    &mut writer,
                    &prefix[..if prefix[0] == 0 { 0 } else { 1 }],
                    body,
                    spec,
                );
            }
            b'n' => unsafe {
                let count = writer.written as c_int;
                match spec.longness {
                    3 | 4 => args.arg::<*mut c_longlong>().write(count as c_longlong),
                    _ => args.arg::<*mut c_int>().write(count),
                }
            },
            _ => {
                writer.write_bytes(b"%");
                writer.write_bytes(core::slice::from_ref(&conversion));
            }
        }
    }
    writer.finish()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsnprintf(
    output: *mut c_char,
    capacity: usize,
    format: *const c_char,
    args: VaList,
) -> c_int {
    unsafe { nagi_vsnprintf(output, capacity, format, args) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn snprintf(
    output: *mut c_char,
    capacity: usize,
    format: *const c_char,
    mut args: ...
) -> c_int {
    unsafe { nagi_vsnprintf(output, capacity, format, args.as_va_list()) }
}

#[inline]
unsafe fn nagi_scan_skip_space(mut cursor: *const c_char) -> *const c_char {
    while matches!(unsafe { nagi_byte(cursor) }, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
        cursor = unsafe { cursor.add(1) };
    }
    cursor
}

unsafe fn nagi_vsscanf(
    input: *const c_char,
    format: *const c_char,
    mut args: VaList,
) -> c_int {
    if input.is_null() || format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let mut cursor = input;
    let mut format_index = 0;
    let mut assigned = 0;
    loop {
        let format_byte = unsafe { format.cast::<u8>().add(format_index).read() };
        if format_byte == 0 {
            return assigned;
        }
        if format_byte.is_ascii_whitespace() {
            while unsafe { format.cast::<u8>().add(format_index).read() }.is_ascii_whitespace() {
                format_index += 1;
            }
            cursor = unsafe { nagi_scan_skip_space(cursor) };
            continue;
        }
        if format_byte != b'%' {
            if unsafe { nagi_byte(cursor) } != format_byte {
                return assigned;
            }
            cursor = unsafe { cursor.add(1) };
            format_index += 1;
            continue;
        }

        format_index += 1;
        if unsafe { format.cast::<u8>().add(format_index).read() } == b'%' {
            if unsafe { nagi_byte(cursor) } != b'%' {
                return assigned;
            }
            cursor = unsafe { cursor.add(1) };
            format_index += 1;
            continue;
        }

        let suppress = if unsafe { format.cast::<u8>().add(format_index).read() } == b'*' {
            format_index += 1;
            true
        } else {
            false
        };
        let mut width = 0usize;
        while unsafe { format.cast::<u8>().add(format_index).read() }.is_ascii_digit() {
            width = width
                .saturating_mul(10)
                .saturating_add(usize::from(unsafe {
                    format.cast::<u8>().add(format_index).read() - b'0'
                }));
            format_index += 1;
        }
        let mut longness = 0_u8;
        match unsafe { format.cast::<u8>().add(format_index).read() } {
            b'h' => {
                longness = 1;
                format_index += 1;
                if unsafe { format.cast::<u8>().add(format_index).read() } == b'h' {
                    longness = 2;
                    format_index += 1;
                }
            }
            b'l' => {
                longness = 3;
                format_index += 1;
                if unsafe { format.cast::<u8>().add(format_index).read() } == b'l' {
                    longness = 4;
                    format_index += 1;
                }
            }
            b'z' | b'j' | b't' => {
                longness = 3;
                format_index += 1;
            }
            _ => {}
        }
        let conversion = unsafe { format.cast::<u8>().add(format_index).read() };
        if conversion == 0 {
            unsafe { set_errno(EINVAL) };
            return assigned;
        }
        format_index += 1;

        match conversion {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                let signed = matches!(conversion, b'd' | b'i');
                let base = match conversion {
                    b'i' => 0,
                    b'd' | b'u' => 10,
                    b'o' => 8,
                    _ => 16,
                };
                let original = cursor;
                let mut end = ptr::null_mut();
                let (value, negative) = unsafe {
                    nagi_parse_unsigned(cursor, &mut end, base, signed)
                };
                if end.is_null() || end.cast_const() == original {
                    return assigned;
                }
                cursor = end.cast_const();
                if !suppress {
                    if signed {
                        let value = value as i64;
                        match longness {
                            3 | 4 => unsafe { args.arg::<*mut c_longlong>().write(value) },
                            1 => unsafe { args.arg::<*mut i16>().write(value as i16) },
                            2 => unsafe { args.arg::<*mut i8>().write(value as i8) },
                            _ => unsafe { args.arg::<*mut c_int>().write(value as c_int) },
                        }
                    } else {
                        let value = if negative { 0_u64.wrapping_sub(value) } else { value };
                        match longness {
                            3 | 4 => unsafe { args.arg::<*mut c_ulonglong>().write(value) },
                            1 => unsafe { args.arg::<*mut u16>().write(value as u16) },
                            2 => unsafe { args.arg::<*mut u8>().write(value as u8) },
                            _ => unsafe { args.arg::<*mut c_uint>().write(value as c_uint) },
                        }
                    }
                    assigned += 1;
                }
            }
            b's' => {
                cursor = unsafe { nagi_scan_skip_space(cursor) };
                let destination = if suppress {
                    ptr::null_mut()
                } else {
                    unsafe { args.arg::<*mut c_char>() }
                };
                if !suppress && destination.is_null() {
                    unsafe { set_errno(EINVAL) };
                    return assigned;
                }
                let limit = if width == 0 { 16 * 1024 * 1024 } else { width };
                let mut length = 0;
                while length < limit {
                    let byte = unsafe { nagi_byte(cursor) };
                    if byte == 0 || byte.is_ascii_whitespace() {
                        break;
                    }
                    if !suppress {
                        unsafe { destination.add(length).write(byte as c_char) };
                    }
                    length += 1;
                    cursor = unsafe { cursor.add(1) };
                }
                if length == 0 {
                    return assigned;
                }
                if !suppress {
                    unsafe { destination.add(length).write(0) };
                    assigned += 1;
                }
            }
            b'c' => {
                let count = if width == 0 { 1 } else { width };
                let destination = if suppress {
                    ptr::null_mut()
                } else {
                    unsafe { args.arg::<*mut c_char>() }
                };
                if !suppress && destination.is_null() {
                    unsafe { set_errno(EINVAL) };
                    return assigned;
                }
                for index in 0..count {
                    let byte = unsafe { nagi_byte(cursor) };
                    if byte == 0 {
                        return assigned;
                    }
                    if !suppress {
                        unsafe { destination.add(index).write(byte as c_char) };
                    }
                    cursor = unsafe { cursor.add(1) };
                }
                if !suppress {
                    assigned += 1;
                }
            }
            b'p' => {
                let original = cursor;
                let mut end = ptr::null_mut();
                let (value, _) = unsafe { nagi_parse_unsigned(cursor, &mut end, 16, false) };
                if end.is_null() || end.cast_const() == original {
                    return assigned;
                }
                cursor = end.cast_const();
                if !suppress {
                    let destination = unsafe { args.arg::<*mut *mut c_void>() };
                    if destination.is_null() {
                        unsafe { set_errno(EINVAL) };
                        return assigned;
                    }
                    unsafe { destination.write(value as *mut c_void) };
                    assigned += 1;
                }
            }
            b'n' => {
                if !suppress {
                    let consumed = unsafe { cursor.offset_from(input) } as c_int;
                    match longness {
                        3 | 4 => unsafe { args.arg::<*mut c_longlong>().write(consumed as c_longlong) },
                        1 => unsafe { args.arg::<*mut i16>().write(consumed as i16) },
                        2 => unsafe { args.arg::<*mut i8>().write(consumed as i8) },
                        _ => unsafe { args.arg::<*mut c_int>().write(consumed) },
                    }
                }
            }
            _ => {
                unsafe { set_errno(ENOSYS) };
                return assigned;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sscanf(
    input: *const c_char,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    unsafe { nagi_vsscanf(input, format, args.as_va_list()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fprintf(
    stream: *mut c_void,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    if stream.is_null() || format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }

    // Mesa's target diagnostics are bounded and the formatter already
    // reports the complete length even when a destination is truncated. Keep
    // the FILE write bounded; an oversized diagnostic fails closed instead of
    // claiming that bytes were emitted when they were not.
    let mut buffer = [0_u8; 4096];
    let written = unsafe {
        nagi_vsnprintf(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format,
            args.as_va_list(),
        )
    };
    if written < 0 || written as usize >= buffer.len() {
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    }
    let count = written as usize;
    let emitted = unsafe { fwrite(buffer.as_ptr().cast(), 1, count, stream) };
    if emitted == count {
        written
    } else {
        EOF
    }
}

/// The target backend cannot use relibc's Linux/Redox `stdio` module, but
/// variadic callers still need the same descriptor-backed formatting path as
/// `fprintf`.  Keep the `va_list` ABI at this boundary and never introduce a
/// host FILE object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vfprintf(
    stream: *mut c_void,
    format: *const c_char,
    args: VaList,
) -> c_int {
    if stream.is_null() || format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }

    let mut buffer = [0_u8; 4096];
    let written = unsafe { nagi_vsnprintf(buffer.as_mut_ptr().cast(), buffer.len(), format, args) };
    if written < 0 || written as usize >= buffer.len() {
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    }
    let count = written as usize;
    if unsafe { fwrite(buffer.as_ptr().cast(), 1, count, stream) } == count {
        written
    } else {
        EOF
    }
}

/// GCC's fortified stdio entry points preserve the same real Nagi FILE and
/// descriptor boundary as `fprintf`/`vfprintf`.  The flag is a compile-time
/// fortify mode, not a permission to bypass the bounded formatter, so it is
/// intentionally ignored after the existing checked path is selected.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vfprintf_chk(
    stream: *mut c_void,
    _flag: c_int,
    format: *const c_char,
    args: VaList,
) -> c_int {
    unsafe { vfprintf(stream, format, args) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __fprintf_chk(
    stream: *mut c_void,
    _flag: c_int,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    unsafe { vfprintf(stream, format, args.as_va_list()) }
}

static mut NAGI_SYSLOG_IDENT: *const c_char = ptr::null();
static mut NAGI_SYSLOG_OPTIONS: c_int = 0;
static mut NAGI_SYSLOG_FACILITY: c_int = 0;

/// Keep syslog inside Nagi's real descriptor-backed diagnostic boundary.  The
/// target has no host syslog socket or daemon ABI; descriptor 2 is the Nagi
/// console service boundary already used by `perror` and target stdio.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openlog(
    ident: *const c_char,
    options: c_int,
    facility: c_int,
) {
    unsafe {
        NAGI_SYSLOG_IDENT = ident;
        NAGI_SYSLOG_OPTIONS = options;
        NAGI_SYSLOG_FACILITY = facility;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn closelog() {
    unsafe {
        NAGI_SYSLOG_IDENT = ptr::null();
        NAGI_SYSLOG_OPTIONS = 0;
        NAGI_SYSLOG_FACILITY = 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn syslog(
    _priority: c_int,
    format: *const c_char,
    mut args: ...,
) {
    if format.is_null() {
        return;
    }
    let mut buffer = [0_u8; 4096];
    let written = unsafe {
        nagi_vsnprintf(buffer.as_mut_ptr().cast(), buffer.len(), format, args.as_va_list())
    };
    if written <= 0 {
        return;
    }
    let count = (written as usize).min(buffer.len() - 1);
    unsafe {
        nagi_posix_write_fd(2, buffer.as_ptr(), count);
    }
    let _ = unsafe { NAGI_SYSLOG_IDENT };
    let _ = unsafe { NAGI_SYSLOG_OPTIONS };
    let _ = unsafe { NAGI_SYSLOG_FACILITY };
}

const NAGI_FORMAT_ALLOCATION_LIMIT: usize = 16 * 1024 * 1024;

/// Format into a Nagi-owned allocation.  `VaList::with_copy` is the Rust
/// equivalent of `va_copy`, so the sizing pass and the emitting pass consume
/// independent target argument lists.  The allocation is deliberately bounded
/// to keep a malformed guest diagnostic from exhausting the process service.
unsafe fn nagi_vasprintf(
    output: *mut *mut c_char,
    format: *const c_char,
    args: VaList,
) -> c_int {
    if output.is_null() || format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let required = unsafe {
        args.with_copy(|copy| nagi_vsnprintf(ptr::null_mut(), 0, format, copy))
    };
    if required < 0 || required as usize >= NAGI_FORMAT_ALLOCATION_LIMIT {
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    }
    let length = required as usize;
    let Some(capacity) = length.checked_add(1) else {
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    };
    let buffer = unsafe { nagi_posix_malloc(capacity) };
    if buffer.is_null() {
        unsafe { set_errno(ENOMEM) };
        return EOF;
    }
    let written = unsafe {
        args.with_copy(|copy| nagi_vsnprintf(buffer.cast(), capacity, format, copy))
    };
    if written != required {
        unsafe { nagi_posix_free(buffer) };
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    }
    unsafe { output.write(buffer.cast()) };
    written
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vasprintf(
    output: *mut *mut c_char,
    format: *const c_char,
    args: VaList,
) -> c_int {
    unsafe { nagi_vasprintf(output, format, args) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn asprintf(
    output: *mut *mut c_char,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    unsafe { nagi_vasprintf(output, format, args.as_va_list()) }
}

/// GCC's fortified stdio entry point.  `object_size` is an upper bound from
/// the caller's real guest object; honoring it keeps this ABI from writing
/// beyond the Nagi allocation even when the caller was compiled with _FORTIFY_SOURCE.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vsnprintf_chk(
    output: *mut c_char,
    capacity: usize,
    _flag: c_int,
    object_size: usize,
    format: *const c_char,
    args: VaList,
) -> c_int {
    if format.is_null() || (output.is_null() && capacity != 0) {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    unsafe { nagi_vsnprintf(output, capacity.min(object_size), format, args) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __snprintf_chk(
    output: *mut c_char,
    capacity: usize,
    flag: c_int,
    object_size: usize,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    unsafe {
        __vsnprintf_chk(
            output,
            capacity,
            flag,
            object_size,
            format,
            args.as_va_list(),
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn printf(format: *const c_char, mut args: ...) -> c_int {
    if format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }

    // stdout is the real Nagi descriptor 1. Keep the bounded formatting and
    // write path identical to fprintf without fabricating a FILE object.
    let mut buffer = [0_u8; 4096];
    let written = unsafe {
        nagi_vsnprintf(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format,
            args.as_va_list(),
        )
    };
    if written < 0 || written as usize >= buffer.len() {
        unsafe { set_errno(EOVERFLOW) };
        return EOF;
    }

    let count = written as usize;
    let emitted = unsafe { nagi_posix_write_fd(1, buffer.as_ptr(), count) };
    if emitted == count as isize {
        written
    } else {
        EOF
    }
}

/// Write a NUL-terminated line through the real Nagi stdout descriptor.
/// `puts` is kept separate from `printf` so it has no formatting parser and
/// cannot accidentally interpret guest content as a format string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(input: *const c_char) -> c_int {
    let Some(length) = (unsafe { c_string_len(input, 16 * 1024 * 1024) }) else {
        unsafe { set_errno(if input.is_null() { EINVAL } else { EOVERFLOW }) };
        return EOF;
    };

    let stream = unsafe { stdout };
    let emitted = unsafe { fwrite(input.cast(), 1, length, stream) };
    if emitted != length {
        return EOF;
    }
    let newline = b"\n";
    if unsafe { fwrite(newline.as_ptr().cast(), 1, 1, stream) } != 1 {
        return EOF;
    }
    (length + 1) as c_int
}

/// Write guest string bytes to a caller-selected Nagi FILE stream.  The
/// stream remains descriptor-backed or guest-memory-backed through `fwrite`;
/// no host stdio object is used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(input: *const c_char, stream: *mut c_void) -> c_int {
    let Some(length) = (unsafe { c_string_len(input, 16 * 1024 * 1024) }) else {
        unsafe { set_errno(if input.is_null() { EINVAL } else { EOVERFLOW }) };
        return EOF;
    };
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    if unsafe { fwrite(input.cast(), 1, length, stream) } != length {
        return EOF;
    }
    0
}

/// Write one byte through the target-owned FILE boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputc(value: c_int, stream: *mut c_void) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let byte = [value as u8];
    if unsafe { fwrite(byte.as_ptr().cast(), 1, 1, stream) } == 1 {
        (value as u8) as c_int
    } else {
        EOF
    }
}

// The normal relibc regex module depends on the host-oriented `posix_regex`
// crate and is intentionally not compiled for target_os=nagi.  Servo's C
// dependencies still use the POSIX ABI, so the Nagi backend owns a bounded
// freestanding implementation here.  It supports literals, '.', bracket
// expressions (including the common POSIX named classes), anchors, grouping,
// alternation, and the POSIX repetition operators.  Compilation stores only
// guest-owned bytes allocated through the Nagi process allocator.
const NAGI_REGEX_MAX_PATTERN: usize = 4096;
const NAGI_REGEX_MAX_RECURSION: usize = 4096;
const NAGI_REGEX_MAX_GROUPS: usize = 32;
const NAGI_REGEX_MAX_INPUT: usize = 16 * 1024 * 1024;
const NAGI_REG_EXTENDED: c_int = 1;
const NAGI_REG_ICASE: c_int = 2;
const NAGI_REG_NOSUB: c_int = 4;
const NAGI_REG_NEWLINE: c_int = 8;
const NAGI_REG_NOTBOL: c_int = 16;
const NAGI_REG_NOTEOL: c_int = 32;
const NAGI_REG_NOMATCH: c_int = 1;
const NAGI_REG_BADPAT: c_int = 2;
const NAGI_REG_ECOLLATE: c_int = 3;
const NAGI_REG_ECTYPE: c_int = 4;
const NAGI_REG_EESCAPE: c_int = 5;
const NAGI_REG_ESUBREG: c_int = 6;
const NAGI_REG_EBRACK: c_int = 7;
const NAGI_REG_ENOSYS: c_int = 8;
const NAGI_REG_EPAREN: c_int = 9;
const NAGI_REG_EBRACE: c_int = 10;
const NAGI_REG_BADBR: c_int = 11;
const NAGI_REG_ERANGE: c_int = 12;
const NAGI_REG_ESPACE: c_int = 13;
const NAGI_REG_BADRPT: c_int = 14;

#[repr(C)]
pub struct NagiRegex {
    ptr: *mut c_void,
    cflags: c_int,
    re_nsub: usize,
}

#[repr(C)]
pub struct NagiRegmatch {
    rm_so: usize,
    rm_eo: usize,
}

#[repr(C)]
struct NagiRegexProgram {
    pattern: *mut u8,
    length: usize,
    cflags: c_int,
    groups: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct NagiRegexCaptures {
    starts: [usize; NAGI_REGEX_MAX_GROUPS],
    ends: [usize; NAGI_REGEX_MAX_GROUPS],
}

impl NagiRegexCaptures {
    const fn empty() -> Self {
        Self {
            starts: [usize::MAX; NAGI_REGEX_MAX_GROUPS],
            ends: [usize::MAX; NAGI_REGEX_MAX_GROUPS],
        }
    }
}

#[derive(Clone, Copy)]
struct NagiRegexContext {
    pattern: *const u8,
    pattern_length: usize,
    input: *const u8,
    input_length: usize,
    cflags: c_int,
    eflags: c_int,
}

#[inline]
unsafe fn nagi_regex_pattern_byte(context: NagiRegexContext, index: usize) -> u8 {
    unsafe { context.pattern.add(index).read() }
}

#[inline]
unsafe fn nagi_regex_input_byte(context: NagiRegexContext, index: usize) -> u8 {
    unsafe { context.input.add(index).read() }
}

#[inline]
fn nagi_regex_extended(context: NagiRegexContext) -> bool {
    context.cflags & NAGI_REG_EXTENDED != 0
}

fn nagi_regex_group_open(context: NagiRegexContext, index: usize) -> usize {
    if index >= context.pattern_length {
        return 0;
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, index) };
    if byte == b'(' && nagi_regex_extended(context) {
        1
    } else if byte == b'\\'
        && index + 1 < context.pattern_length
        && unsafe { nagi_regex_pattern_byte(context, index + 1) } == b'('
        && !nagi_regex_extended(context)
    {
        2
    } else {
        0
    }
}

fn nagi_regex_group_close(context: NagiRegexContext, index: usize) -> usize {
    if index >= context.pattern_length {
        return 0;
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, index) };
    if byte == b')' && nagi_regex_extended(context) {
        1
    } else if byte == b'\\'
        && index + 1 < context.pattern_length
        && unsafe { nagi_regex_pattern_byte(context, index + 1) } == b')'
        && !nagi_regex_extended(context)
    {
        2
    } else {
        0
    }
}

fn nagi_regex_class_end(context: NagiRegexContext, start: usize, end: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut first = true;
    while index < end {
        let byte = unsafe { nagi_regex_pattern_byte(context, index) };
        if byte == b'\\' {
            index = index.saturating_add(2);
            first = false;
            continue;
        }
        if byte == b']' && !first {
            return Some(index + 1);
        }
        first = false;
        index += 1;
    }
    None
}

fn nagi_regex_find_group_close(
    context: NagiRegexContext,
    start: usize,
    end: usize,
) -> Option<usize> {
    let open = nagi_regex_group_open(context, start);
    if open == 0 {
        return None;
    }
    let mut depth = 1usize;
    let mut index = start + open;
    while index < end {
        let byte = unsafe { nagi_regex_pattern_byte(context, index) };
        if byte == b'\\' {
            index = index.saturating_add(2);
            continue;
        }
        if byte == b'[' {
            if let Some(class_end) = nagi_regex_class_end(context, index, end) {
                index = class_end;
                continue;
            }
            return None;
        }
        if nagi_regex_group_open(context, index) != 0 {
            depth += 1;
            index += nagi_regex_group_open(context, index);
            continue;
        }
        if nagi_regex_group_close(context, index) != 0 {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
            index += nagi_regex_group_close(context, index);
            continue;
        }
        index += 1;
    }
    None
}

fn nagi_regex_atom_end(context: NagiRegexContext, start: usize, end: usize) -> Option<usize> {
    if start >= end {
        return None;
    }
    if nagi_regex_group_open(context, start) != 0 {
        let close = nagi_regex_find_group_close(context, start, end)?;
        return Some(close + nagi_regex_group_close(context, close));
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, start) };
    if byte == b'[' {
        return nagi_regex_class_end(context, start, end);
    }
    if byte == b'\\' {
        return (start + 2 <= end).then_some(start + 2);
    }
    Some(start + 1)
}

fn nagi_regex_group_index(context: NagiRegexContext, target: usize) -> usize {
    let mut index = 0;
    let mut groups = 0;
    while index < target && index < context.pattern_length {
        if nagi_regex_group_open(context, index) != 0 {
            groups += 1;
            index += nagi_regex_group_open(context, index);
        } else if unsafe { nagi_regex_pattern_byte(context, index) } == b'[' {
            index = nagi_regex_class_end(context, index, target).unwrap_or(target);
        } else if unsafe { nagi_regex_pattern_byte(context, index) } == b'\\' {
            index += 2;
        } else {
            index += 1;
        }
    }
    groups
}

fn nagi_regex_decimal(context: NagiRegexContext, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut index = start;
    let mut value = 0usize;
    let mut digits = 0;
    while index < end {
        let byte = unsafe { nagi_regex_pattern_byte(context, index) };
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add(usize::from(byte - b'0'))?;
        digits += 1;
        index += 1;
    }
    (digits != 0).then_some((value, index))
}

fn nagi_regex_quantifier(
    context: NagiRegexContext,
    start: usize,
    end: usize,
) -> (usize, usize, usize) {
    if start >= end {
        return (1, 1, start);
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, start) };
    let extended = nagi_regex_extended(context);
    let (kind, mut index) = if matches!(byte, b'*' | b'+' | b'?') && (extended || byte == b'*') {
        (byte, start + 1)
    } else if !extended
        && byte == b'\\'
        && start + 1 < end
        && matches!(unsafe { nagi_regex_pattern_byte(context, start + 1) }, b'+' | b'?')
    {
        (unsafe { nagi_regex_pattern_byte(context, start + 1) }, start + 2)
    } else {
        (0, start)
    };
    if kind != 0 {
        return match kind {
            b'*' => (0, usize::MAX, index),
            b'+' => (1, usize::MAX, index),
            _ => (0, 1, index),
        };
    }

    let brace = if byte == b'{' && extended {
        Some(start + 1)
    } else if !extended
        && byte == b'\\'
        && start + 1 < end
        && unsafe { nagi_regex_pattern_byte(context, start + 1) } == b'{'
    {
        Some(start + 2)
    } else {
        None
    };
    let Some(brace_start) = brace else {
        return (1, 1, start);
    };
    let Some((minimum, after_minimum)) = nagi_regex_decimal(context, brace_start, end) else {
        return (1, 1, start);
    };
    index = after_minimum;
    let maximum = if index < end && unsafe { nagi_regex_pattern_byte(context, index) } == b',' {
        index += 1;
        if let Some((maximum, after_maximum)) = nagi_regex_decimal(context, index, end) {
            index = after_maximum;
            maximum
        } else {
            usize::MAX
        }
    } else {
        minimum
    };
    let close = if extended {
        index < end && unsafe { nagi_regex_pattern_byte(context, index) } == b'}'
    } else {
        index + 1 < end
            && unsafe { nagi_regex_pattern_byte(context, index) } == b'\\'
            && unsafe { nagi_regex_pattern_byte(context, index + 1) } == b'}'
    };
    if !close || maximum < minimum {
        return (1, 1, start);
    }
    (minimum, maximum, if extended { index + 1 } else { index + 2 })
}

fn nagi_regex_class_named(name: &[u8], byte: u8) -> bool {
    let alpha = byte.is_ascii_alphabetic();
    let digit = byte.is_ascii_digit();
    let space = byte.is_ascii_whitespace();
    let value = match name {
        b"alnum" => alpha || digit,
        b"alpha" => alpha,
        b"blank" => byte == b' ' || byte == b'\t',
        b"cntrl" => byte < 0x20 || byte == 0x7f,
        b"digit" => digit,
        b"graph" => (0x21..=0x7e).contains(&byte),
        b"lower" => byte.is_ascii_lowercase(),
        b"print" => (0x20..=0x7e).contains(&byte),
        b"punct" => (0x21..=0x7e).contains(&byte) && !alpha && !digit,
        b"space" => space,
        b"upper" => byte.is_ascii_uppercase(),
        b"xdigit" => digit || (b'a'..=b'f').contains(&byte) || (b'A'..=b'F').contains(&byte),
        _ => false,
    };
    value
}

fn nagi_regex_equal(context: NagiRegexContext, left: u8, right: u8) -> bool {
    if context.cflags & NAGI_REG_ICASE == 0 {
        left == right
    } else {
        left.eq_ignore_ascii_case(&right)
    }
}

fn nagi_regex_class_matches(
    context: NagiRegexContext,
    start: usize,
    end: usize,
    value: u8,
) -> bool {
    let mut index = start + 1;
    let mut negate = false;
    if index < end && unsafe { nagi_regex_pattern_byte(context, index) } == b'^' {
        negate = true;
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < end {
        let byte = unsafe { nagi_regex_pattern_byte(context, index) };
        if byte == b']' && !first {
            break;
        }
        first = false;
        if byte == b'[' && index + 1 < end && unsafe { nagi_regex_pattern_byte(context, index + 1) } == b':' {
            let name_start = index + 2;
            let mut close = name_start;
            while close + 1 < end
                && !(unsafe { nagi_regex_pattern_byte(context, close) } == b':'
                    && unsafe { nagi_regex_pattern_byte(context, close + 1) } == b']')
            {
                close += 1;
            }
            if close + 1 < end {
                let name = unsafe {
                    core::slice::from_raw_parts(context.pattern.add(name_start), close - name_start)
                };
                matched |= nagi_regex_class_named(name, value);
                index = close + 2;
                continue;
            }
        }
        let (left, next) = if byte == b'\\' && index + 1 < end {
            (unsafe { nagi_regex_pattern_byte(context, index + 1) }, index + 2)
        } else {
            (byte, index + 1)
        };
        if next + 1 < end
            && unsafe { nagi_regex_pattern_byte(context, next) } == b'-'
            && unsafe { nagi_regex_pattern_byte(context, next + 1) } != b']'
        {
            let right = unsafe { nagi_regex_pattern_byte(context, next + 1) };
            let left_cmp = if context.cflags & NAGI_REG_ICASE != 0 {
                left.to_ascii_lowercase()
            } else {
                left
            };
            let right_cmp = if context.cflags & NAGI_REG_ICASE != 0 {
                right.to_ascii_lowercase()
            } else {
                right
            };
            let value_cmp = if context.cflags & NAGI_REG_ICASE != 0 {
                value.to_ascii_lowercase()
            } else {
                value
            };
            matched |= left_cmp <= value_cmp && value_cmp <= right_cmp;
            index = next + 2;
        } else {
            matched |= nagi_regex_equal(context, left, value);
            index = next;
        }
    }
    if negate { !matched } else { matched }
}

fn nagi_regex_match_atom(
    context: NagiRegexContext,
    start: usize,
    end: usize,
    position: usize,
    captures: NagiRegexCaptures,
    depth: usize,
) -> Option<(usize, NagiRegexCaptures)> {
    if position > context.input_length || start >= end || depth > NAGI_REGEX_MAX_RECURSION {
        return None;
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, start) };
    if nagi_regex_group_open(context, start) != 0 {
        let open = nagi_regex_group_open(context, start);
        let close = nagi_regex_find_group_close(context, start, end)?;
        let (next, mut output) = nagi_regex_match_expression(
            context,
            start + open,
            close,
            position,
            captures,
            depth + 1,
        )?;
        let group = nagi_regex_group_index(context, start);
        if group < NAGI_REGEX_MAX_GROUPS {
            output.starts[group] = position;
            output.ends[group] = next;
        }
        return Some((next, output));
    }
    if byte == b'[' {
        if position < context.input_length
            && nagi_regex_class_matches(context, start, end, unsafe {
                nagi_regex_input_byte(context, position)
            })
        {
            return Some((position + 1, captures));
        }
        return None;
    }
    let value = if byte == b'\\' && start + 1 < end {
        match unsafe { nagi_regex_pattern_byte(context, start + 1) } {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'f' => 0x0c,
            b'v' => 0x0b,
            value => value,
        }
    } else {
        byte
    };
    if value == b'.' && byte == b'.' {
        if position < context.input_length
            && (context.cflags & NAGI_REG_NEWLINE == 0
                || unsafe { nagi_regex_input_byte(context, position) } != b'\n')
        {
            return Some((position + 1, captures));
        }
    } else if position < context.input_length
        && nagi_regex_equal(context, value, unsafe { nagi_regex_input_byte(context, position) })
    {
        return Some((position + 1, captures));
    }
    None
}

fn nagi_regex_match_repeat(
    context: NagiRegexContext,
    atom_start: usize,
    atom_end: usize,
    next: usize,
    sequence_end: usize,
    minimum: usize,
    maximum: usize,
    count: usize,
    position: usize,
    captures: NagiRegexCaptures,
    depth: usize,
) -> Option<(usize, NagiRegexCaptures)> {
    if depth > NAGI_REGEX_MAX_RECURSION {
        return None;
    }
    if count < maximum {
        if let Some((after_atom, after_captures)) = nagi_regex_match_atom(
            context,
            atom_start,
            atom_end,
            position,
            captures,
            depth + 1,
        ) {
            if after_atom != position {
                if let Some(result) = nagi_regex_match_repeat(
                    context,
                    atom_start,
                    atom_end,
                    next,
                    sequence_end,
                    minimum,
                    maximum,
                    count + 1,
                    after_atom,
                    after_captures,
                    depth + 1,
                ) {
                    return Some(result);
                }
            }
        }
    }
    if count >= minimum {
        nagi_regex_match_sequence(context, next, sequence_end, position, captures, depth + 1)
    } else {
        None
    }
}

fn nagi_regex_match_sequence(
    context: NagiRegexContext,
    start: usize,
    end: usize,
    position: usize,
    captures: NagiRegexCaptures,
    depth: usize,
) -> Option<(usize, NagiRegexCaptures)> {
    if depth > NAGI_REGEX_MAX_RECURSION || start > end {
        return None;
    }
    if start >= end {
        return Some((position, captures));
    }
    let byte = unsafe { nagi_regex_pattern_byte(context, start) };
    if byte == b'^' && start == 0 {
        if context.eflags & NAGI_REG_NOTBOL != 0 || position != 0 {
            return None;
        }
        return nagi_regex_match_sequence(context, start + 1, end, position, captures, depth + 1);
    }
    if byte == b'$' && start + 1 == end {
        if context.eflags & NAGI_REG_NOTEOL != 0 || position != context.input_length {
            return None;
        }
        return Some((position, captures));
    }
    let atom_end = nagi_regex_atom_end(context, start, end)?;
    let (minimum, maximum, after_quantifier) = nagi_regex_quantifier(context, atom_end, end);
    nagi_regex_match_repeat(
        context,
        start,
        atom_end,
        after_quantifier,
        end,
        minimum,
        maximum,
        0,
        position,
        captures,
        depth + 1,
    )
}

fn nagi_regex_match_expression(
    context: NagiRegexContext,
    start: usize,
    end: usize,
    position: usize,
    captures: NagiRegexCaptures,
    depth: usize,
) -> Option<(usize, NagiRegexCaptures)> {
    if depth > NAGI_REGEX_MAX_RECURSION {
        return None;
    }
    let mut index = start;
    let mut nested = 0usize;
    while index < end {
        if unsafe { nagi_regex_pattern_byte(context, index) } == b'\\' {
            index += 2;
            continue;
        }
        if unsafe { nagi_regex_pattern_byte(context, index) } == b'[' {
            index = nagi_regex_class_end(context, index, end).unwrap_or(end);
            continue;
        }
        if nagi_regex_group_open(context, index) != 0 {
            nested += 1;
            index += nagi_regex_group_open(context, index);
        } else if nagi_regex_group_close(context, index) != 0 {
            nested = nested.saturating_sub(1);
            index += nagi_regex_group_close(context, index);
        } else if nested == 0 && unsafe { nagi_regex_pattern_byte(context, index) } == b'|' {
            if let Some(result) = nagi_regex_match_sequence(
                context,
                start,
                index,
                position,
                captures,
                depth + 1,
            ) {
                return Some(result);
            }
            return nagi_regex_match_expression(
                context,
                index + 1,
                end,
                position,
                captures,
                depth + 1,
            );
        } else {
            index += 1;
        }
    }
    nagi_regex_match_sequence(context, start, end, position, captures, depth + 1)
}

fn nagi_regex_validate(context: NagiRegexContext) -> Result<usize, c_int> {
    let mut index = 0usize;
    let mut groups = 0usize;
    let mut depth = 0usize;
    while index < context.pattern_length {
        let byte = unsafe { nagi_regex_pattern_byte(context, index) };
        if byte == b'\\' {
            if index + 1 >= context.pattern_length {
                return Err(NAGI_REG_EESCAPE);
            }
            index += 2;
            continue;
        }
        if byte == b'[' {
            let Some(next) = nagi_regex_class_end(context, index, context.pattern_length) else {
                return Err(NAGI_REG_EBRACK);
            };
            index = next;
            continue;
        }
        if nagi_regex_group_open(context, index) != 0 {
            depth += 1;
            groups += 1;
            index += nagi_regex_group_open(context, index);
            continue;
        }
        if nagi_regex_group_close(context, index) != 0 {
            if depth == 0 {
                return Err(NAGI_REG_EPAREN);
            }
            depth -= 1;
            index += nagi_regex_group_close(context, index);
            continue;
        }
        index += 1;
    }
    if depth != 0 {
        return Err(NAGI_REG_EPAREN);
    }
    if groups > NAGI_REGEX_MAX_GROUPS {
        return Err(NAGI_REG_ESPACE);
    }
    Ok(groups)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regcomp(
    output: *mut NagiRegex,
    pattern: *const c_char,
    cflags: c_int,
) -> c_int {
    if output.is_null() || pattern.is_null() {
        unsafe { set_errno(EINVAL) };
        return NAGI_REG_BADPAT;
    }
    let Some(length) = (unsafe { c_string_len(pattern, NAGI_REGEX_MAX_PATTERN) }) else {
        unsafe { set_errno(EOVERFLOW) };
        return NAGI_REG_ESPACE;
    };
    let pattern_copy = unsafe { nagi_posix_malloc(length.max(1)) };
    if pattern_copy.is_null() {
        unsafe { set_errno(ENOMEM) };
        return NAGI_REG_ESPACE;
    }
    unsafe { ptr::copy_nonoverlapping(pattern.cast::<u8>(), pattern_copy, length) };
    let context = NagiRegexContext {
        pattern: pattern_copy,
        pattern_length: length,
        input: ptr::null(),
        input_length: 0,
        cflags,
        eflags: 0,
    };
    let groups = match nagi_regex_validate(context) {
        Ok(groups) => groups,
        Err(error) => {
            unsafe { nagi_posix_free(pattern_copy) };
            return error;
        }
    };
    let program = unsafe { nagi_posix_malloc(mem::size_of::<NagiRegexProgram>()) }
        .cast::<NagiRegexProgram>();
    if program.is_null() {
        unsafe { nagi_posix_free(pattern_copy) };
        unsafe { set_errno(ENOMEM) };
        return NAGI_REG_ESPACE;
    }
    unsafe {
        program.write(NagiRegexProgram {
            pattern: pattern_copy,
            length,
            cflags,
            groups,
        });
        output.write(NagiRegex {
            ptr: program.cast(),
            cflags,
            re_nsub: groups,
        });
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regfree(regex: *mut NagiRegex) {
    if regex.is_null() {
        return;
    }
    let program = unsafe { (*regex).ptr.cast::<NagiRegexProgram>() };
    if !program.is_null() {
        let pattern = unsafe { (*program).pattern };
        if !pattern.is_null() {
            unsafe { nagi_posix_free(pattern) };
        }
        unsafe { nagi_posix_free(program.cast()) };
    }
    unsafe {
        (*regex).ptr = ptr::null_mut();
        (*regex).re_nsub = 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regexec(
    regex: *const NagiRegex,
    input: *const c_char,
    nmatch: usize,
    matches: *mut NagiRegmatch,
    eflags: c_int,
) -> c_int {
    if regex.is_null() || input.is_null() {
        return NAGI_REG_NOMATCH;
    }
    let program = unsafe { (*regex).ptr.cast::<NagiRegexProgram>() };
    if program.is_null() {
        return NAGI_REG_NOMATCH;
    }
    let Some(input_length) = (unsafe { c_string_len(input, NAGI_REGEX_MAX_INPUT) }) else {
        return NAGI_REG_NOMATCH;
    };
    let program_ref = unsafe { &*program };
    let context = NagiRegexContext {
        pattern: program_ref.pattern,
        pattern_length: program_ref.length,
        input: input.cast(),
        input_length,
        cflags: program_ref.cflags,
        eflags,
    };
    let anchored = context.pattern_length > 0
        && unsafe { nagi_regex_pattern_byte(context, 0) } == b'^';
    let last_start = if anchored { 0 } else { input_length };
    let mut start = 0usize;
    while start <= last_start {
        let captures = NagiRegexCaptures::empty();
        if let Some((end, captures)) = nagi_regex_match_expression(
            context,
            0,
            context.pattern_length,
            start,
            captures,
            0,
        ) {
            if program_ref.cflags & NAGI_REG_NOSUB == 0 && !matches.is_null() {
                if nmatch > 0 {
                    unsafe { matches.write(NagiRegmatch { rm_so: start, rm_eo: end }) };
                }
                let group_count = nmatch.saturating_sub(1).min(NAGI_REGEX_MAX_GROUPS);
                for group in 0..group_count {
                    let (rm_so, rm_eo) = if captures.starts[group] == usize::MAX {
                        (usize::MAX, usize::MAX)
                    } else {
                        (captures.starts[group], captures.ends[group])
                    };
                    unsafe {
                        matches.add(group + 1).write(NagiRegmatch { rm_so, rm_eo });
                    }
                }
            }
            return 0;
        }
        if anchored {
            break;
        }
        start += 1;
    }
    NAGI_REG_NOMATCH
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn regerror(
    code: c_int,
    _regex: *const NagiRegex,
    output: *mut c_char,
    capacity: usize,
) -> usize {
    let message: &[u8] = match code {
        0 => b"No error\0",
        NAGI_REG_NOMATCH => b"No match\0",
        NAGI_REG_BADPAT => b"Invalid regexp\0",
        NAGI_REG_ECOLLATE => b"Unknown collating element\0",
        NAGI_REG_ECTYPE => b"Unknown character class name\0",
        NAGI_REG_EESCAPE => b"Trailing backslash\0",
        NAGI_REG_ESUBREG => b"Invalid back reference\0",
        NAGI_REG_EBRACK => b"Missing ]\0",
        NAGI_REG_ENOSYS => b"Unsupported operation\0",
        NAGI_REG_EPAREN => b"Missing )\0",
        NAGI_REG_EBRACE => b"Missing }\0",
        NAGI_REG_BADBR => b"Invalid repetition\0",
        NAGI_REG_ERANGE => b"Invalid character range\0",
        NAGI_REG_ESPACE => b"Out of memory\0",
        NAGI_REG_BADRPT => b"Invalid repetition operator\0",
        _ => b"Unknown regexp error\0",
    };
    if !output.is_null() && capacity != 0 {
        unsafe { ptr::copy_nonoverlapping(message.as_ptr(), output.cast(), message.len().min(capacity)) };
    }
    message.len().saturating_sub(1)
}

/// Nagi's user VFS commits each descriptor write through its service boundary
/// before returning.  There is no process-local stdio or host filesystem
/// cache for `sync` to flush, so the POSIX void operation is a truthful
/// completed barrier with no host side effect.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sprintf(
    output: *mut c_char,
    format: *const c_char,
    mut args: ...,
) -> c_int {
    if output.is_null() || format.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    unsafe { nagi_vsnprintf(output, usize::MAX, format, args.as_va_list()) }
}

/// The Nagi target does not import a host libc for numeric conversion. Keep
/// the relibc conversion entry points in this target-owned backend so libc++
/// can use its normal numeric facets without disabling localization globally.
/// The `_l` variants intentionally implement the C/POSIX locale semantics
/// currently provided by Nagi; they never read host locale state.
#[inline]
unsafe fn nagi_byte(pointer: *const c_char) -> u8 {
    unsafe { *pointer.cast::<u8>() }
}

#[inline]
fn nagi_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

#[inline]
unsafe fn nagi_set_endptr(endptr: *mut *mut c_char, cursor: *const c_char) {
    if !endptr.is_null() {
        unsafe { endptr.write(cursor.cast_mut()) };
    }
}

unsafe fn nagi_parse_unsigned(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
    signed: bool,
) -> (u64, bool) {
    if input.is_null() {
        unsafe { set_errno(EINVAL) };
        return (0, false);
    }

    let original = input;
    let mut cursor = input;
    while matches!(
        unsafe { nagi_byte(cursor) },
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
    ) {
        cursor = unsafe { cursor.add(1) };
    }

    let negative = match unsafe { nagi_byte(cursor) } {
        b'-' => {
            cursor = unsafe { cursor.add(1) };
            true
        }
        b'+' => {
            cursor = unsafe { cursor.add(1) };
            false
        }
        _ => false,
    };

    if base != 0 && !(2..=36).contains(&base) {
        unsafe {
            nagi_set_endptr(endptr, cursor);
            set_errno(EINVAL);
        }
        return (0, false);
    }

    let mut radix = base;
    if radix == 0 {
        radix = if unsafe { nagi_byte(cursor) } == b'0' {
            if matches!(unsafe { nagi_byte(cursor.add(1)) }, b'x' | b'X') {
                cursor = unsafe { cursor.add(2) };
                16
            } else {
                8
            }
        } else {
            10
        };
    } else if radix == 16
        && unsafe { nagi_byte(cursor) } == b'0'
        && matches!(unsafe { nagi_byte(cursor.add(1)) }, b'x' | b'X')
        && nagi_digit(unsafe { nagi_byte(cursor.add(2)) }).is_some_and(|digit| digit < 16)
    {
        cursor = unsafe { cursor.add(2) };
    }

    let digits_start = cursor;
    let limit = if signed {
        if negative {
            1_u64 << 63
        } else {
            i64::MAX as u64
        }
    } else {
        u64::MAX
    };
    let mut value = 0_u64;
    let mut overflow = false;

    loop {
        let Some(digit) = nagi_digit(unsafe { nagi_byte(cursor) }) else {
            break;
        };
        if digit >= radix as u32 {
            break;
        }
        if !overflow {
            match value
                .checked_mul(radix as u64)
                .and_then(|value| value.checked_add(u64::from(digit)))
            {
                Some(next) if next <= limit => value = next,
                _ => {
                    value = limit;
                    overflow = true;
                }
            }
        }
        cursor = unsafe { cursor.add(1) };
    }

    if cursor == digits_start {
        unsafe {
            nagi_set_endptr(endptr, original);
            set_errno(EINVAL);
        }
        return (0, negative);
    }
    if overflow {
        unsafe { set_errno(ERANGE) };
    }
    unsafe { nagi_set_endptr(endptr, cursor) };

    if signed && negative && !overflow {
        (0_u64.wrapping_sub(value), true)
    } else if !signed && negative && !overflow {
        (0_u64.wrapping_sub(value), false)
    } else {
        (value, negative)
    }
}

unsafe fn nagi_parse_float(input: *const c_char, endptr: *mut *mut c_char) -> c_double {
    if input.is_null() {
        unsafe { set_errno(EINVAL) };
        return 0.0;
    }

    let mut cursor = input;
    while matches!(
        unsafe { nagi_byte(cursor) },
        b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c
    ) {
        cursor = unsafe { cursor.add(1) };
    }
    let sign = match unsafe { nagi_byte(cursor) } {
        b'-' => {
            cursor = unsafe { cursor.add(1) };
            -1.0
        }
        b'+' => {
            cursor = unsafe { cursor.add(1) };
            1.0
        }
        _ => 1.0,
    };
    let value_start = cursor;

    let mut word_match = |word: &[u8]| {
        let mut probe = cursor;
        for expected in word {
            if !unsafe { nagi_byte(probe) }.eq_ignore_ascii_case(expected) {
                return false;
            }
            probe = unsafe { probe.add(1) };
        }
        cursor = probe;
        true
    };
    if word_match(b"inf") {
        let _ = word_match(b"inity");
        unsafe { nagi_set_endptr(endptr, cursor) };
        return sign * c_double::INFINITY;
    }
    if word_match(b"nan") {
        unsafe { nagi_set_endptr(endptr, cursor) };
        return c_double::NAN;
    }

    let mut radix = 10_u32;
    let mut exponent_marker = b'e';
    if unsafe { nagi_byte(cursor) } == b'0'
        && matches!(unsafe { nagi_byte(cursor.add(1)) }, b'x' | b'X')
    {
        radix = 16;
        exponent_marker = b'p';
        cursor = unsafe { cursor.add(2) };
    }

    let mut value = 0.0_f64;
    let mut digits = 0_usize;
    while let Some(digit) = nagi_digit(unsafe { nagi_byte(cursor) })
        && digit < radix
    {
        value = value * radix as f64 + digit as f64;
        digits += 1;
        cursor = unsafe { cursor.add(1) };
    }
    if unsafe { nagi_byte(cursor) } == b'.' {
        cursor = unsafe { cursor.add(1) };
        let mut divisor = radix as f64;
        while let Some(digit) = nagi_digit(unsafe { nagi_byte(cursor) })
            && digit < radix
        {
            value += digit as f64 / divisor;
            divisor *= radix as f64;
            digits += 1;
            cursor = unsafe { cursor.add(1) };
        }
    }

    if digits == 0 {
        unsafe { nagi_set_endptr(endptr, value_start) };
        return 0.0;
    }

    let exponent_start = cursor;
    let mut exponent_negative = false;
    let mut exponent = 0_u32;
    if unsafe { nagi_byte(cursor) }.eq_ignore_ascii_case(&exponent_marker) {
        cursor = unsafe { cursor.add(1) };
        match unsafe { nagi_byte(cursor) } {
            b'-' => {
                exponent_negative = true;
                cursor = unsafe { cursor.add(1) };
            }
            b'+' => cursor = unsafe { cursor.add(1) },
            _ => {}
        }
        let exponent_digits = cursor;
        while let Some(digit) = nagi_digit(unsafe { nagi_byte(cursor) })
            && digit < 10
        {
            exponent = exponent.saturating_mul(10).saturating_add(digit);
            cursor = unsafe { cursor.add(1) };
        }
        if cursor == exponent_digits {
            cursor = exponent_start;
        }
    }

    let mut scale = 1.0_f64;
    let mut power = if radix == 16 { 2.0 } else { 10.0 };
    let mut remaining = exponent;
    while remaining != 0 {
        if remaining & 1 != 0 {
            scale *= power;
        }
        remaining >>= 1;
        if remaining != 0 {
            power *= power;
        }
    }
    if exponent_negative {
        scale = 1.0 / scale;
    }
    let result = sign * value * scale;
    if result == c_double::INFINITY || result == c_double::NEG_INFINITY {
        unsafe { set_errno(ERANGE) };
    }
    unsafe { nagi_set_endptr(endptr, cursor) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod_l(
    input: *const c_char,
    endptr: *mut *mut c_char,
    _locale: *mut c_void,
) -> c_double {
    unsafe { nagi_parse_float(input, endptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(input: *const c_char, endptr: *mut *mut c_char) -> c_double {
    unsafe { strtod_l(input, endptr, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn atof(input: *const c_char) -> c_double {
    unsafe { strtod(input, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof_l(
    input: *const c_char,
    endptr: *mut *mut c_char,
    _locale: *mut c_void,
) -> c_float {
    unsafe { nagi_parse_float(input, endptr) as c_float }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof(input: *const c_char, endptr: *mut *mut c_char) -> c_float {
    unsafe { strtof_l(input, endptr, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll_l(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
    _locale: *mut c_void,
) -> c_longlong {
    let (value, negative) = unsafe { nagi_parse_unsigned(input, endptr, base, true) };
    if negative {
        value as i64
    } else {
        value as c_longlong
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoll(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
) -> c_longlong {
    unsafe { strtoll_l(input, endptr, base, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull_l(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
    _locale: *mut c_void,
) -> c_ulonglong {
    let (value, _) = unsafe { nagi_parse_unsigned(input, endptr, base, false) };
    value
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoull(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
) -> c_ulonglong {
    unsafe { strtoull_l(input, endptr, base, ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtol(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
) -> c_long {
    unsafe { strtoll_l(input, endptr, base, ptr::null_mut()) as c_long }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtoul(
    input: *const c_char,
    endptr: *mut *mut c_char,
    base: c_int,
) -> c_ulong {
    unsafe { strtoull_l(input, endptr, base, ptr::null_mut()) as c_ulong }
}

/// Decimal integer conversion for the Nagi target.  This delegates to the
/// target-owned parser above so whitespace, sign handling, overflow and errno
/// behavior stay aligned with the other relibc numeric entry points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atoi(input: *const c_char) -> c_int {
    unsafe { strtol(input, ptr::null_mut(), 10) as c_int }
}

/// Marker used by the Nagi guest acceptance app to ensure the backend archive
/// was linked into the image.
#[unsafe(no_mangle)]
pub extern "C" fn nagi_backend_probe() -> u32 {
    0x4e41_4749
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    unsafe { nagi_posix_malloc(size.max(1)) }.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(pointer: *mut c_void) {
    unsafe { nagi_posix_free(pointer.cast()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        return core::ptr::null_mut();
    };
    let pointer = unsafe { malloc(total) };
    if !pointer.is_null() {
        unsafe { core::ptr::write_bytes(pointer.cast::<u8>(), 0, total) };
    }
    pointer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
    if pointer.is_null() {
        return unsafe { malloc(size) };
    }
    if size == 0 {
        unsafe { free(pointer) };
        return core::ptr::null_mut();
    }
    // Nagi's bounded allocator records the allocation size immediately before
    // the returned pointer. Preserve the common realloc contract without a
    // host allocator or an unbounded heap.
    let old_size = unsafe { pointer.cast::<u8>().sub(16).cast::<usize>().read() };
    let replacement = unsafe { malloc(size) };
    if !replacement.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(
                pointer.cast::<u8>(),
                replacement.cast::<u8>(),
                old_size.min(size),
            );
            free(pointer);
        }
    }
    replacement
}

/// Return the bounded payload recorded by Nagi's user-space allocator.
///
/// `std::alloc::System` uses the relibc malloc ABI on the Nagi target and
/// Servo's allocator introspection calls this libc API for profiling. Forward
/// the query to the Nagi POSIX allocator, which validates its own metadata and
/// reports the requested payload without consulting host allocator state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc_usable_size(pointer: *mut c_void) -> usize {
    unsafe { nagi_posix_malloc_usable_size(pointer.cast()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, bytes: *const u8, length: usize) -> isize {
    unsafe { nagi_posix_write_fd(fd, bytes, length) }
}

/// Forward the C ioctl boundary to Nagi's user-space POSIX facade.  Unsupported
/// requests fail closed there; this never invokes a host ioctl implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ioctl(fd: c_int, request: c_ulong, out: *mut c_void) -> c_int {
    unsafe { nagi_posix_ioctl(fd, request, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    unsafe { nagi_posix_open(path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, bytes: *mut u8, length: usize) -> isize {
    unsafe { nagi_posix_read(fd, bytes, length) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    unsafe { nagi_posix_close(fd) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    unsafe { nagi_posix_lseek(fd, offset, whence) }
}

/// POSIX mmap entry point for the Nagi target.
///
/// The target-only backend deliberately does not use relibc's Linux/Redox
/// syscall implementation.  Forward mappings to the Nagi POSIX facade, which
/// owns the bounded VMO mapping and VFS file-read path.  `MAP_FAILED` is
/// returned on failure so callers such as Mesa do not mistake a null pointer
/// for a successful mapping.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(
    address: *mut c_void,
    length: usize,
    protection: c_int,
    flags: c_int,
    fd: c_int,
    offset: i64,
) -> *mut c_void {
    if offset < 0 {
        unsafe { set_errno(EINVAL) };
        return usize::MAX as *mut c_void;
    }

    if flags & MAP_FIXED != 0 {
        if address.is_null() || fd >= 0 || offset != 0 || flags & MAP_ANONYMOUS == 0 {
            unsafe { set_errno(EINVAL) };
            return usize::MAX as *mut c_void;
        }
        let mapping = unsafe { nagi_posix_mmap_at(address.cast(), length, protection) };
        return if mapping.is_null() {
            usize::MAX as *mut c_void
        } else {
            mapping.cast()
        };
    }

    if fd < 0 && flags & MAP_ANONYMOUS == 0 {
        unsafe { set_errno(EINVAL) };
        return usize::MAX as *mut c_void;
    }

    // A non-fixed address is only a hint. Nagi chooses from its bounded,
    // capability-owned mapping arena and never honors a host virtual address.
    let mapping = unsafe { nagi_posix_mmap_file(length, protection, fd, offset as usize) };
    if mapping.is_null() {
        usize::MAX as *mut c_void
    } else {
        mapping.cast()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(address: *mut c_void, length: usize) -> c_int {
    unsafe { nagi_posix_munmap(address.cast(), length) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int {
    unsafe { nagi_posix_mprotect(address.cast(), length, protection) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_relibc_initialize_filesystem(capability: u64) -> c_int {
    unsafe { nagi_posix_initialize_filesystem(capability) }
}
