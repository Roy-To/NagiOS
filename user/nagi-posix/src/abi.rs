//! C symbol surface used by the Nagi-built Rust `std` and relibc backend.
//!
//! These symbols are user-space adapters.  They do not map to host libc or
//! add high-level filesystem/network syscalls to the kernel.

use core::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use core::time::Duration;

use crate::errno::{
    set_errno, EAGAIN, EBADF, EINVAL, ENOMEM, ENOPROTOOPT, ENOSYS, ENOTDIR, ENOTSUP, ENOTTY,
    ERANGE, ETIMEDOUT,
};
use libnagi::storage::{DirectoryEntry, MAX_DIRECTORY_ENTRIES, MAX_NAME_LENGTH};
use nagi_pal::time::{Clock, GuestClock};

const TLS_SLOTS: usize = 64;
const THREAD_SLOTS: usize = 2;
static NEXT_TLS_KEY: AtomicUsize = AtomicUsize::new(1);
static mut TLS_VALUES: [[usize; TLS_SLOTS]; THREAD_SLOTS] = [[0; TLS_SLOTS]; THREAD_SLOTS];
const THREAD_NAME_LENGTH: usize = 16;
static mut THREAD_NAMES: [[u8; THREAD_NAME_LENGTH]; THREAD_SLOTS] =
    [[0; THREAD_NAME_LENGTH]; THREAD_SLOTS];

type PthreadStart = extern "C" fn(*mut c_void) -> *mut c_void;

#[repr(C)]
struct PthreadStartRecord {
    start: Option<PthreadStart>,
    argument: *mut c_void,
}

unsafe impl Sync for PthreadStartRecord {}

static mut PTHREAD_START_RECORD: PthreadStartRecord = PthreadStartRecord {
    start: None,
    argument: ptr::null_mut(),
};
static mut PTHREAD_STACK: *mut u8 = ptr::null_mut();

#[inline]
fn current_thread_slot() -> usize {
    (libnagi::thread_self() as usize).min(THREAD_SLOTS - 1)
}

extern "C" fn pthread_trampoline(record: *mut c_void) -> ! {
    let (start, argument) = unsafe {
        let record = &*(record.cast::<PthreadStartRecord>());
        (record.start, record.argument)
    };
    let result = start
        .map(|routine| routine(argument))
        .unwrap_or(ptr::null_mut());
    libnagi::thread_exit(result as u64)
}

#[inline]
unsafe fn write_errno_and_fail(error: i32) -> c_int {
    set_errno(error);
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __errno() -> *mut c_int {
    crate::errno::nagi_posix_errno_location()
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    crate::nagi_posix_malloc(size.max(1)).cast()
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(_pointer: *mut c_void) {}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        set_errno(ENOMEM);
        return ptr::null_mut();
    };
    let pointer = malloc(total.max(1));
    if !pointer.is_null() {
        ptr::write_bytes(pointer.cast::<u8>(), 0, total);
    }
    pointer
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
    if pointer.is_null() {
        return malloc(size);
    }
    if size == 0 {
        free(pointer);
        return ptr::null_mut();
    }
    // The bounded Nagi allocator intentionally has no host-style in-place
    // realloc.  Keep the operation explicit and copy only the requested new
    // extent; callers that require preservation use the PAL allocator.
    let old_size = pointer.cast::<u8>().sub(16).cast::<usize>().read();
    let replacement = malloc(size);
    if !replacement.is_null() {
        ptr::copy_nonoverlapping(
            pointer.cast::<u8>(),
            replacement.cast::<u8>(),
            core::cmp::min(old_size, size),
        );
    }
    replacement
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_memalign(
    result: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> c_int {
    if result.is_null() || alignment < core::mem::size_of::<usize>() || !alignment.is_power_of_two()
    {
        return write_errno_and_fail(EINVAL);
    }
    let pointer = malloc(size.max(1));
    if pointer.is_null() {
        return write_errno_and_fail(ENOMEM);
    }
    // The Nagi PAL's user allocation alignment is 16 bytes.  Refuse a
    // stronger alignment instead of returning a misaligned pointer.
    if (pointer as usize) & (alignment - 1) != 0 {
        return write_errno_and_fail(ENOMEM);
    }
    result.write(pointer);
    0
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_write_fd(fd: c_int, bytes: *const u8, length: usize) -> isize {
    if fd == 1 || fd == 2 {
        return crate::nagi_posix_write(fd, bytes, length);
    }
    if bytes.is_null() {
        return write_errno_and_fail(EINVAL) as isize;
    }
    let data = core::slice::from_raw_parts(bytes, length);
    match crate::runtime::write(fd, data) {
        Ok(count) => count as isize,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)) as isize,
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, bytes: *const u8, length: usize) -> isize {
    nagi_posix_write_fd(fd, bytes, length)
}

/// Nagi does not expose a host device-control ABI.  Keep the POSIX ioctl
/// boundary real and fail closed for requests without a Nagi service rather
/// than forwarding them to the development host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_ioctl(
    fd: c_int,
    _request: c_ulong,
    _out: *mut c_void,
) -> c_int {
    if fd < 0 {
        return write_errno_and_fail(EBADF);
    }
    write_errno_and_fail(ENOTTY)
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn writev(fd: c_int, vectors: *const Iovec, count: c_int) -> isize {
    const MAX_IOV: c_int = 1024;
    if count < 0 || count > MAX_IOV || (count != 0 && vectors.is_null()) {
        return write_errno_and_fail(EINVAL) as isize;
    }
    let mut written = 0_isize;
    for index in 0..count as usize {
        let vector = vectors.add(index).read();
        if vector.length == 0 {
            continue;
        }
        if vector.base.is_null() {
            return write_errno_and_fail(EINVAL) as isize;
        }
        let result = write(fd, vector.base, vector.length);
        if result < 0 {
            return if written == 0 { result } else { written };
        }
        written = written.saturating_add(result);
        if result as usize != vector.length {
            break;
        }
    }
    written
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readv(fd: c_int, vectors: *const Iovec, count: c_int) -> isize {
    const MAX_IOV: c_int = 1024;
    if count < 0 || count > MAX_IOV || (count != 0 && vectors.is_null()) {
        return write_errno_and_fail(EINVAL) as isize;
    }
    let mut read = 0_isize;
    for index in 0..count as usize {
        let vector = vectors.add(index).read();
        if vector.length == 0 {
            continue;
        }
        if vector.base.is_null() {
            return write_errno_and_fail(EINVAL) as isize;
        }
        let result = nagi_posix_read(fd, vector.base, vector.length);
        if result < 0 {
            return if read == 0 { result } else { read };
        }
        read = read.saturating_add(result);
        if result as usize != vector.length {
            break;
        }
    }
    read
}

#[repr(C)]
pub struct Iovec {
    base: *mut u8,
    length: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_initialize_filesystem(capability: u64) -> c_int {
    if crate::runtime::initialize(capability) {
        0
    } else {
        write_errno_and_fail(5)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_initialize_network(capability: u64) -> c_int {
    if crate::runtime::initialize_network(capability) {
        0
    } else {
        write_errno_and_fail(16)
    }
}

#[repr(C)]
pub struct NagiIpv4Address {
    pub octets: [u8; 4],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_resolve_ipv4(
    name: *const c_char,
    output: *mut NagiIpv4Address,
) -> c_int {
    if name.is_null() || output.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let mut bytes = [0_u8; 128];
    let mut length = 0;
    while length < bytes.len() {
        let byte = name.add(length).read() as u8;
        if byte == 0 {
            break;
        }
        bytes[length] = byte;
        length += 1;
    }
    if length == bytes.len() {
        return write_errno_and_fail(EINVAL);
    }
    let Ok(name) = core::str::from_utf8(&bytes[..length]) else {
        return write_errno_and_fail(EINVAL);
    };
    match crate::runtime::resolve_ipv4(name) {
        Ok(address) => {
            output.write(NagiIpv4Address {
                octets: address.octets(),
            });
            0
        }
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_default_gateway(output: *mut NagiIpv4Address) -> c_int {
    if output.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    match crate::runtime::default_gateway() {
        Ok(address) => {
            output.write(NagiIpv4Address {
                octets: address.octets(),
            });
            0
        }
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

const AF_INET: c_int = 2;
const SOCK_STREAM: c_int = 1;
const SOL_SOCKET: c_int = 1;
const SO_RCVTIMEO: c_int = 20;
const SO_SNDTIMEO: c_int = 21;
const IPPROTO_TCP: c_int = 6;
const TCP_NODELAY: c_int = 1;
const SHUT_RD: c_int = 0;
const SHUT_WR: c_int = 1;
const SHUT_RDWR: c_int = 2;

#[repr(C)]
pub struct NagiSockaddrIpv4 {
    pub family: u16,
    pub port_be: u16,
    pub address: [u8; 4],
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int {
    if domain != AF_INET || socket_type != SOCK_STREAM || protocol != 0 {
        return write_errno_and_fail(97);
    }
    match crate::runtime::socket() {
        Ok(fd) => fd,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn connect(
    fd: c_int,
    address: *const c_void,
    address_length: usize,
) -> c_int {
    if address.is_null() || address_length < core::mem::size_of::<NagiSockaddrIpv4>() {
        return write_errno_and_fail(EINVAL);
    }
    let address = &*address.cast::<NagiSockaddrIpv4>();
    if address.family as c_int != AF_INET || address.port_be == 0 {
        return write_errno_and_fail(EINVAL);
    }
    match crate::runtime::connect(
        fd,
        nagi_net::Ipv4Address::new(address.address),
        u16::from_be(address.port_be),
    ) {
        Ok(()) => 0,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getpeername(
    fd: c_int,
    address: *mut c_void,
    address_length: *mut c_uint,
) -> c_int {
    if address.is_null() || address_length.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let required = core::mem::size_of::<NagiSockaddrIpv4>() as c_uint;
    if address_length.read() < required {
        return write_errno_and_fail(EINVAL);
    }
    let (peer, port) = match crate::runtime::peer_name(fd) {
        Ok(peer) => peer,
        Err(error) => return write_errno_and_fail(crate::runtime::map_error(error)),
    };
    address.cast::<NagiSockaddrIpv4>().write(NagiSockaddrIpv4 {
        family: AF_INET as u16,
        port_be: port.to_be(),
        address: peer.0,
    });
    address_length.write(required);
    0
}

/// Nagi 0.1 currently exposes only a client TCP service. Listener creation is
/// not present in the user-space network service, so bind/listen fail closed
/// instead of claiming a server endpoint that the guest cannot accept.
#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bind(
    socket: c_int,
    _address: *const c_void,
    _address_length: c_uint,
) -> c_int {
    if socket < 0 {
        return write_errno_and_fail(EBADF);
    }
    write_errno_and_fail(ENOSYS)
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn listen(socket: c_int, _backlog: c_int) -> c_int {
    if socket < 0 {
        return write_errno_and_fail(EBADF);
    }
    write_errno_and_fail(ENOSYS)
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send(
    fd: c_int,
    bytes: *const c_void,
    length: usize,
    _flags: c_int,
) -> isize {
    if bytes.is_null() {
        return write_errno_and_fail(EINVAL) as isize;
    }
    match crate::runtime::write(fd, core::slice::from_raw_parts(bytes.cast(), length)) {
        Ok(count) => count as isize,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)) as isize,
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recv(
    fd: c_int,
    bytes: *mut c_void,
    length: usize,
    _flags: c_int,
) -> isize {
    if bytes.is_null() {
        return write_errno_and_fail(EINVAL) as isize;
    }
    match crate::runtime::read(fd, core::slice::from_raw_parts_mut(bytes.cast(), length)) {
        Ok(count) => count as isize,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)) as isize,
    }
}

#[repr(C)]
struct NagiTimeval {
    seconds: i64,
    microseconds: i64,
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shutdown(fd: c_int, how: c_int) -> c_int {
    if !matches!(how, SHUT_RD | SHUT_WR | SHUT_RDWR) {
        return write_errno_and_fail(EINVAL);
    }
    match crate::runtime::shutdown(fd, how) {
        Ok(()) => 0,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setsockopt(
    fd: c_int,
    level: c_int,
    option_name: c_int,
    option_value: *const c_void,
    option_length: u32,
) -> c_int {
    if option_value.is_null() && option_length != 0 {
        return write_errno_and_fail(EINVAL);
    }
    match (level, option_name) {
        (IPPROTO_TCP, TCP_NODELAY) => {
            if option_length as usize != core::mem::size_of::<c_int>() || option_value.is_null() {
                return write_errno_and_fail(EINVAL);
            }
            let bytes = core::slice::from_raw_parts(
                option_value.cast::<u8>(),
                core::mem::size_of::<c_int>(),
            );
            let enabled = c_int::from_ne_bytes(bytes.try_into().expect("c_int width")) != 0;
            match crate::runtime::set_tcp_nodelay(fd, enabled) {
                Ok(()) => 0,
                Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
            }
        }
        (SOL_SOCKET, SO_RCVTIMEO) | (SOL_SOCKET, SO_SNDTIMEO) => {
            if option_length as usize != core::mem::size_of::<NagiTimeval>()
                || option_value.is_null()
            {
                return write_errno_and_fail(EINVAL);
            }
            let timeval = option_value.cast::<NagiTimeval>().read_unaligned();
            if timeval.seconds < 0 || !(0..1_000_000).contains(&timeval.microseconds) {
                return write_errno_and_fail(EINVAL);
            }
            let timeout = if timeval.seconds == 0 && timeval.microseconds == 0 {
                None
            } else {
                Some(Duration::new(
                    timeval.seconds as u64,
                    timeval.microseconds as u32 * 1_000,
                ))
            };
            match crate::runtime::set_socket_timeout(fd, timeout) {
                Ok(()) => 0,
                Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
            }
        }
        _ => write_errno_and_fail(ENOPROTOOPT),
    }
}

/// Nagi 0.1 exposes a client TCP slice only; listener creation and accept are
/// not part of the user-space network service yet.  Keep this ABI explicit
/// and fail closed instead of returning a fabricated descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_accept(
    socket: c_int,
    _address: *mut c_void,
    _address_len: *mut c_uint,
) -> c_int {
    if socket < 0 {
        return write_errno_and_fail(EBADF);
    }
    write_errno_and_fail(ENOSYS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_getsockopt(
    socket: c_int,
    level: c_int,
    option_name: c_int,
    option_value: *mut c_void,
    option_len: *mut c_uint,
) -> c_int {
    if option_value.is_null() || option_len.is_null() {
        return write_errno_and_fail(EINVAL);
    }

    match (level, option_name) {
        (IPPROTO_TCP, TCP_NODELAY) => {
            let required = core::mem::size_of::<c_int>() as c_uint;
            if option_len.read() < required {
                return write_errno_and_fail(EINVAL);
            }
            let enabled = match crate::runtime::tcp_nodelay(socket) {
                Ok(enabled) => enabled,
                Err(error) => return write_errno_and_fail(crate::runtime::map_error(error)),
            };
            option_value
                .cast::<c_int>()
                .write_unaligned(if enabled { 1 } else { 0 });
            option_len.write(required);
            0
        }
        (SOL_SOCKET, SO_RCVTIMEO) | (SOL_SOCKET, SO_SNDTIMEO) => {
            let required = core::mem::size_of::<NagiTimeval>() as c_uint;
            if option_len.read() < required {
                return write_errno_and_fail(EINVAL);
            }
            let timeout = match crate::runtime::socket_timeout(socket) {
                Ok(timeout) => timeout,
                Err(error) => return write_errno_and_fail(crate::runtime::map_error(error)),
            };
            let timeval = timeout.map_or(
                NagiTimeval {
                    seconds: 0,
                    microseconds: 0,
                },
                |duration| NagiTimeval {
                    seconds: duration.as_secs() as i64,
                    microseconds: i64::from(duration.subsec_micros()),
                },
            );
            option_value.cast::<NagiTimeval>().write_unaligned(timeval);
            option_len.write(required);
            0
        }
        _ => write_errno_and_fail(ENOPROTOOPT),
    }
}

const O_CREAT: c_int = 0x0200_0000;
const O_TRUNC: c_int = 0x0400_0000;
const AT_FDCWD: c_int = -100;
const AT_REMOVEDIR: c_int = 0x0200;

#[repr(C)]
pub struct NagiDirent {
    pub d_ino: u64,
    pub d_off: c_long,
    pub d_reclen: u16,
    pub d_type: u8,
    pub d_name: [c_char; 256],
}

#[repr(C)]
pub struct NagiDir {
    entries: [DirectoryEntry; MAX_DIRECTORY_ENTRIES],
    count: usize,
    cursor: usize,
    current: NagiDirent,
}

unsafe fn c_path(path: *const c_char, output: &mut [u8]) -> Result<&[u8], c_int> {
    if path.is_null() {
        return Err(EINVAL);
    }
    let mut length = 0;
    while length < output.len() {
        let byte = path.add(length).read() as u8;
        if byte == 0 {
            let mut start = 0;
            while start < length && output[start] == b'/' {
                start += 1;
            }
            if start == length || output[start..length].contains(&b'/') {
                return Err(EINVAL);
            }
            return Ok(&output[start..length]);
        }
        output[length] = byte;
        length += 1;
    }
    Err(EINVAL)
}

unsafe fn is_root_path(path: *const c_char) -> Result<bool, c_int> {
    if path.is_null() {
        return Err(EINVAL);
    }
    let mut length = 0;
    let mut only_slashes = true;
    while length < 64 {
        let byte = path.add(length).read() as u8;
        if byte == 0 {
            return Ok(length != 0 && only_slashes);
        }
        if byte != b'/' {
            only_slashes = false;
        }
        length += 1;
    }
    Err(EINVAL)
}

/// Nagi 0.1 currently exposes one process root and a root-directory VFS.
/// Changing to that same root is a real no-op; other directory namespaces are
/// rejected until the VFS and process model provide them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_chdir(path: *const c_char) -> c_int {
    match is_root_path(path) {
        Ok(true) => 0,
        Ok(false) => write_errno_and_fail(ENOTSUP),
        Err(error) => write_errno_and_fail(error),
    }
}

/// Nagi processes already start in the single capability-scoped root. A
/// request to chroot to that root preserves the actual process namespace;
/// changing to another root is not represented as success without a VFS/root
/// capability implementation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_chroot(path: *const c_char) -> c_int {
    match is_root_path(path) {
        Ok(true) => 0,
        Ok(false) => write_errno_and_fail(ENOTSUP),
        Err(error) => write_errno_and_fail(error),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_open(path: *const c_char, flags: c_int, _mode: c_int) -> c_int {
    let mut bytes = [0_u8; 64];
    let name = match c_path(path, &mut bytes) {
        Ok(name) => name,
        Err(error) => return write_errno_and_fail(error),
    };
    match crate::runtime::open(name, flags & O_CREAT != 0, flags & O_TRUNC != 0) {
        Ok(fd) => fd,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_openat(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: c_int,
) -> c_int {
    if dirfd != AT_FDCWD {
        return write_errno_and_fail(ENOTSUP);
    }
    nagi_posix_open(path, flags, mode)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_unlinkat(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    if dirfd != AT_FDCWD {
        return write_errno_and_fail(ENOTSUP);
    }
    if flags & !AT_REMOVEDIR != 0 {
        return write_errno_and_fail(EINVAL);
    }
    let mut bytes = [0_u8; 64];
    let name = match c_path(path, &mut bytes) {
        Ok(name) => name,
        Err(error) => return write_errno_and_fail(error),
    };
    match crate::runtime::remove(name) {
        Ok(()) => 0,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_unlink(path: *const c_char) -> c_int {
    nagi_posix_unlinkat(AT_FDCWD, path, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_mkdir(path: *const c_char, _mode: c_uint) -> c_int {
    let mut bytes = [0_u8; 64];
    let name = match c_path(path, &mut bytes) {
        Ok(name) => name,
        Err(error) => return write_errno_and_fail(error),
    };
    match crate::runtime::mkdir(name) {
        Ok(()) => 0,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_rmdir(path: *const c_char) -> c_int {
    nagi_posix_unlinkat(AT_FDCWD, path, AT_REMOVEDIR)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_opendir(path: *const c_char) -> *mut c_void {
    match is_root_path(path) {
        Ok(true) => {}
        Ok(false) => {
            set_errno(ENOTSUP);
            return ptr::null_mut();
        }
        Err(error) => {
            set_errno(error);
            return ptr::null_mut();
        }
    }

    let mut entries = [DirectoryEntry::empty(); MAX_DIRECTORY_ENTRIES];
    let count = match crate::runtime::list_root(&mut entries) {
        Ok(count) => count,
        Err(error) => {
            set_errno(crate::runtime::map_error(error));
            return ptr::null_mut();
        }
    };
    let directory = crate::nagi_posix_malloc(core::mem::size_of::<NagiDir>()).cast::<NagiDir>();
    if directory.is_null() {
        set_errno(ENOMEM);
        return ptr::null_mut();
    }
    directory.write(NagiDir {
        entries,
        count,
        cursor: 0,
        current: NagiDirent {
            d_ino: 0,
            d_off: 0,
            d_reclen: 0,
            d_type: 0,
            d_name: [0; 256],
        },
    });
    directory.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_readdir(directory: *mut c_void) -> *mut c_void {
    if directory.is_null() {
        set_errno(EBADF);
        return ptr::null_mut();
    }
    let directory = &mut *directory.cast::<NagiDir>();
    let Some(entry) = directory.entries.get(directory.cursor).copied() else {
        return ptr::null_mut();
    };
    directory.cursor += 1;
    directory.current.d_ino = u64::from(entry.inode);
    directory.current.d_off = directory.cursor as c_long;
    directory.current.d_type = entry.file_type;
    directory.current.d_name = [0; 256];
    let length = usize::from(entry.name_len).min(MAX_NAME_LENGTH);
    for (destination, source) in directory.current.d_name[..length]
        .iter_mut()
        .zip(entry.name().iter().copied())
    {
        *destination = source as c_char;
    }
    directory.current.d_reclen = (19 + length + 1).next_multiple_of(8) as u16;
    (&mut directory.current as *mut NagiDirent).cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_readdir_r(
    directory: *mut c_void,
    entry: *mut c_void,
    result: *mut *mut c_void,
) -> c_int {
    if directory.is_null() || entry.is_null() || result.is_null() {
        if !result.is_null() {
            result.write(ptr::null_mut());
        }
        return write_errno_and_fail(EINVAL);
    }
    let current = nagi_posix_readdir(directory);
    if current.is_null() {
        result.write(ptr::null_mut());
        return 0;
    }
    ptr::copy_nonoverlapping(
        current.cast::<u8>(),
        entry.cast::<u8>(),
        core::mem::size_of::<NagiDirent>(),
    );
    result.write(entry);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_closedir(directory: *mut c_void) -> c_int {
    if directory.is_null() {
        return write_errno_and_fail(EBADF);
    }
    crate::nagi_posix_free(directory.cast());
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_fdopendir(fd: c_int) -> *mut c_void {
    match crate::runtime::size(fd) {
        Ok(_) => {
            set_errno(ENOTDIR);
        }
        Err(error) => {
            set_errno(crate::runtime::map_error(error));
        }
    }
    ptr::null_mut()
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int {
    nagi_posix_open(path, flags, mode)
}

#[repr(C)]
struct NagiStat {
    st_dev: i64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: i32,
    st_uid: u32,
    st_gid: u32,
    st_rdev: i64,
    st_size: i64,
    st_blksize: i64,
    st_blocks: u64,
    st_atime: i64,
    st_atime_nsec: i64,
    st_mtime: i64,
    st_mtime_nsec: i64,
    st_ctime: i64,
    st_ctime_nsec: i64,
    _pad: [c_char; 24],
}

unsafe fn fill_stat(fd: c_int, output: *mut NagiStat) -> c_int {
    if output.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let size = match crate::runtime::size(fd) {
        Ok(size) => size,
        Err(error) => return write_errno_and_fail(crate::runtime::map_error(error)),
    };
    output.write(NagiStat {
        st_dev: 0,
        st_ino: 1,
        st_nlink: 1,
        st_mode: 0o100644,
        st_uid: 0,
        st_gid: 0,
        st_rdev: 0,
        st_size: size as i64,
        st_blksize: 1024,
        st_blocks: size.div_ceil(512) as u64,
        st_atime: 0,
        st_atime_nsec: 0,
        st_mtime: 0,
        st_mtime_nsec: 0,
        st_ctime: 0,
        st_ctime_nsec: 0,
        _pad: [0; 24],
    });
    0
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, output: *mut c_void) -> c_int {
    fill_stat(fd, output.cast())
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, output: *mut c_void) -> c_int {
    let fd = nagi_posix_open(path, 0, 0);
    if fd < 0 {
        return -1;
    }
    let result = fill_stat(fd, output.cast());
    let _ = nagi_posix_close(fd);
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_lstat(path: *const c_char, output: *mut c_void) -> c_int {
    let fd = nagi_posix_open(path, 0, 0);
    if fd < 0 {
        return -1;
    }
    let result = fill_stat(fd, output.cast());
    let _ = nagi_posix_close(fd);
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_isatty(fd: c_int) -> c_int {
    if fd == 1 || fd == 2 {
        return 1;
    }
    if fd < 0 {
        set_errno(EBADF);
    } else {
        set_errno(ENOTTY);
    }
    0
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lstat(path: *const c_char, output: *mut c_void) -> c_int {
    nagi_posix_lstat(path, output)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    match crate::runtime::seek(fd, offset as isize, whence) {
        Ok(position) => position as i64,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)) as i64,
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    nagi_posix_lseek(fd, offset, whence)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getcwd(buffer: *mut c_char, length: usize) -> *mut c_char {
    if buffer.is_null() || length < 2 {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    buffer.write(b'/' as c_char);
    buffer.add(1).write(0);
    buffer
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realpath(path: *const c_char, resolved: *mut c_char) -> *mut c_char {
    if path.is_null() || resolved.is_null() {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    let mut bytes = [0_u8; 64];
    let name = match c_path(path, &mut bytes) {
        Ok(name) => name,
        Err(error) => {
            set_errno(error);
            return ptr::null_mut();
        }
    };
    let mut index = 0;
    resolved.add(index).write(b'/' as c_char);
    index += 1;
    for &byte in name {
        resolved.add(index).write(byte as c_char);
        index += 1;
    }
    resolved.add(index).write(0);
    resolved
}

const CLOCK_REALTIME: c_int = 1;
const CLOCK_MONOTONIC: c_int = 4;

#[repr(C)]
pub struct NagiTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn clock_gettime(clock: c_int, output: *mut NagiTimespec) -> c_int {
    if output.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let nanos = if clock == CLOCK_REALTIME {
        match GuestClock.realtime_ns() {
            Ok(nanos) => nanos,
            Err(_) => return write_errno_and_fail(ENOSYS),
        }
    } else if clock == CLOCK_MONOTONIC {
        match GuestClock.monotonic_ns() {
            Ok(nanos) => nanos,
            Err(_) => return write_errno_and_fail(ENOSYS),
        }
    } else {
        return write_errno_and_fail(EINVAL);
    };
    output.write(NagiTimespec {
        tv_sec: (nanos / 1_000_000_000) as i64,
        tv_nsec: (nanos % 1_000_000_000) as i64,
    });
    0
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gettimeofday(output: *mut NagiTimeval, _timezone: *mut c_void) -> c_int {
    if output.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let nanos = match GuestClock.realtime_ns() {
        Ok(nanos) => nanos,
        Err(_) => return write_errno_and_fail(ENOSYS),
    };
    output.write(NagiTimeval {
        seconds: (nanos / 1_000_000_000) as i64,
        microseconds: ((nanos % 1_000_000_000) / 1_000) as i64,
    });
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nanosleep(
    requested: *const NagiTimespec,
    _remaining: *mut NagiTimespec,
) -> c_int {
    if requested.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    let request = &*requested;
    if request.tv_sec < 0 || !(0..1_000_000_000).contains(&request.tv_nsec) {
        return write_errno_and_fail(EINVAL);
    }
    let Some(seconds) = (request.tv_sec as u64).checked_mul(1_000_000_000) else {
        return write_errno_and_fail(EINVAL);
    };
    let Some(nanos) = seconds.checked_add(request.tv_nsec as u64) else {
        return write_errno_and_fail(EINVAL);
    };
    match GuestClock.sleep_ns(nanos) {
        Ok(()) => 0,
        Err(_) => write_errno_and_fail(ENOSYS),
    }
}

/// Map a VFS file through the bounded anonymous VMO mapping path, then fill
/// the mapped pages from Nagi VFS.  This is the current bootstrap file-backed
/// mapping contract; it never reads a host file or creates a host mapping.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_mmap_file(
    length: usize,
    protection: i32,
    fd: c_int,
    offset: usize,
) -> *mut u8 {
    if fd < 0 {
        return crate::nagi_posix_mmap(length, protection);
    }
    let address = crate::nagi_posix_mmap(length, protection);
    if address.is_null() {
        return ptr::null_mut();
    }
    let destination = core::slice::from_raw_parts_mut(address, length);
    if crate::runtime::read_at(fd, offset, destination).is_err() {
        let _ = crate::nagi_posix_munmap(address, length);
        write_errno_and_fail(EINVAL);
        return ptr::null_mut();
    }
    address
}

#[repr(C)]
pub struct NagiPollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;

/// Bounded user-space readiness polling.  File descriptors are checked by
/// the VFS service and the timeout sleeps through the guest timer syscall.
/// Socket readiness uses the same entry point once a SocketApi descriptor is
/// installed; unsupported descriptor kinds fail closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_poll_fds(
    fds: *mut NagiPollFd,
    count: usize,
    timeout_ms: i32,
) -> c_int {
    if (count != 0 && fds.is_null()) || count > 32 || timeout_ms < -1 {
        return write_errno_and_fail(EINVAL);
    }
    let started = GuestClock.monotonic_ns().ok();
    loop {
        let mut ready = 0;
        for index in 0..count {
            let poll_fd = &mut *fds.add(index);
            poll_fd.revents = 0;
            match crate::runtime::readiness(poll_fd.fd, poll_fd.events) {
                Ok(revents) => {
                    poll_fd.revents = revents;
                    if revents != 0 {
                        ready += 1;
                    }
                }
                Err(crate::runtime::RuntimeError::InvalidFd) => {
                    poll_fd.revents = POLLERR;
                    ready += 1;
                }
                Err(_) => return write_errno_and_fail(ENOSYS),
            }
        }
        if ready != 0 || timeout_ms == 0 {
            return ready;
        }
        let Some(started) = started else {
            return write_errno_and_fail(ENOSYS);
        };
        let now = match GuestClock.monotonic_ns() {
            Ok(now) => now,
            Err(_) => return write_errno_and_fail(ENOSYS),
        };
        let elapsed_ms = now.saturating_sub(started) / 1_000_000;
        if timeout_ms >= 0 && elapsed_ms >= timeout_ms as u64 {
            return 0;
        }
        let sleep_ms = if timeout_ms < 0 {
            1
        } else {
            (timeout_ms as u64 - elapsed_ms).min(1)
        };
        if GuestClock
            .sleep_ns(sleep_ms.saturating_mul(1_000_000))
            .is_err()
        {
            return write_errno_and_fail(ENOSYS);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut NagiPollFd, count: usize, timeout_ms: c_int) -> c_int {
    nagi_posix_poll_fds(fds, count, timeout_ms)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe2(fds: *mut c_int, flags: c_int) -> c_int {
    if fds.is_null() {
        return write_errno_and_fail(EINVAL);
    }
    match crate::runtime::pipe2(flags) {
        Ok((reader, writer)) => {
            fds.write(reader);
            fds.add(1).write(writer);
            0
        }
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(fds: *mut c_int) -> c_int {
    pipe2(fds, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(fd: c_int, command: c_int, argument: c_int) -> c_int {
    match crate::runtime::fcntl(fd, command, argument) {
        Ok(value) => value,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_dup2(old_fd: c_int, new_fd: c_int) -> c_int {
    match crate::runtime::dup2(old_fd, new_fd) {
        Ok(value) => value,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_read(_fd: c_int, _bytes: *mut u8, _length: usize) -> isize {
    if _bytes.is_null() {
        return write_errno_and_fail(EINVAL) as isize;
    }
    let destination = core::slice::from_raw_parts_mut(_bytes, _length);
    match crate::runtime::read(_fd, destination) {
        Ok(count) => count as isize,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)) as isize,
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, bytes: *mut u8, length: usize) -> isize {
    nagi_posix_read(fd, bytes, length)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_close(_fd: c_int) -> c_int {
    match crate::runtime::close(_fd) {
        Ok(()) => 0,
        Err(error) => write_errno_and_fail(crate::runtime::map_error(error)),
    }
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    nagi_posix_close(fd)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(mut string: *const c_char) -> usize {
    if string.is_null() {
        return 0;
    }
    let mut length = 0;
    while string.read() != 0 {
        length += 1;
        string = string.add(1);
    }
    length
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strerror_r(error: c_int, buffer: *mut c_char, length: usize) -> c_int {
    if buffer.is_null() || length == 0 {
        return write_errno_and_fail(EINVAL);
    }
    let message = b"nagi errno\0";
    let copy_length = core::cmp::min(length - 1, message.len() - 1);
    ptr::copy_nonoverlapping(message.as_ptr().cast::<c_char>(), buffer, copy_length);
    buffer.add(copy_length).write(0);
    let _ = error;
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(_name: *const c_char) -> *mut c_char {
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn setgroups(_count: c_int, _groups: *const u32) -> c_int {
    write_errno_and_fail(ENOSYS)
}

/// Nagi 0.1 has capability-scoped identity, not a mutable POSIX gid store.
/// Report that boundary explicitly instead of returning fabricated success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_setgid(_gid: c_uint) -> c_int {
    write_errno_and_fail(ENOSYS)
}

/// Nagi 0.1 has capability-scoped identity, not a mutable POSIX uid store.
/// Report that boundary explicitly instead of returning fabricated success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_setuid(_uid: c_uint) -> c_int {
    write_errno_and_fail(ENOSYS)
}

/// The initial Nagi user process is the kernel's published root process with
/// pid 1. Return that real process identity through the POSIX facade rather
/// than consulting a host process or inventing a per-thread identifier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_getpid() -> c_int {
    1
}

/// Nagi 0.1 does not expose Unix process groups or sessions. Keep these
/// Tier-B POSIX operations explicit and fail closed instead of fabricating
/// process-group state in the spawn-oriented runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_setpgid(_pid: c_int, _pgid: c_int) -> c_int {
    write_errno_and_fail(ENOSYS)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_setsid() -> c_int {
    write_errno_and_fail(ENOSYS)
}

/// Unix signal delivery is not part of the current Nagi process ABI. Return
/// the real `SIG_ERR` pointer value while setting errno, so callers cannot
/// mistake the unsupported operation for a successfully installed handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_signal(_signal: c_int, _handler: *mut c_void) -> *mut c_void {
    set_errno(ENOSYS);
    usize::MAX as *mut c_void
}

// relibc owns the strong target libc abort implementation on Nagi. Keep this
// user-space ABI fallback weak only for the target link; host builds must keep
// a normal fallback because MSVC does not provide the target libc collision
// boundary and does not need a weak COFF symbol here.
#[cfg_attr(target_os = "nagi", linkage = "weak")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn abort() -> ! {
    libnagi::exit(134)
}

// C++ target images replace this weak lifecycle hook with the strong
// freestanding registry from tools/mesa/nagi-cxx-runtime.cpp. Other Nagi
// target images have no C++ static-destructor table and therefore retain the
// valid empty hook without importing a host runtime.
#[cfg(target_os = "nagi")]
#[cfg_attr(target_os = "nagi", linkage = "weak")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_cxx_finalize() {}

/// Terminate the current Nagi process through the published process-exit
/// syscall. The POSIX exit status is the low eight bits, matching the status
/// encoding used by the waitpid adapter below.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_exit(code: c_int) -> ! {
    #[cfg(target_os = "nagi")]
    unsafe {
        nagi_cxx_finalize();
    }
    libnagi::exit((code as u8) as u64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_waitpid(
    pid: c_int,
    status: *mut c_int,
    options: c_int,
) -> c_int {
    // M17's native process slice is deliberately spawn-oriented and exposes
    // one joinable child slot. Do not fabricate a PID or silently implement
    // unsupported wait options; map the real native child handle only.
    if pid != 1 {
        return write_errno_and_fail(EINVAL);
    }
    if options != 0 {
        return write_errno_and_fail(ENOTSUP);
    }
    match unsafe { crate::process::native_wait(pid as u64) } {
        Ok(code) => {
            if !status.is_null() {
                unsafe { status.write(((code & 0xff) << 8) as c_int) };
            }
            pid
        }
        Err(_) => write_errno_and_fail(EAGAIN),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_init(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return EINVAL;
    }
    // relibc's pthread_attr_t is a bounded 32-byte opaque object. The native
    // Nagi bridge deliberately chooses its own mapped 16 KiB child stack, so
    // the requested host-sized stack is metadata only at this layer.
    ptr::write_bytes(attributes.cast::<u8>(), 0, 32);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_setstacksize(
    attributes: *mut c_void,
    _stack_size: usize,
) -> c_int {
    if attributes.is_null() {
        return EINVAL;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_destroy(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return EINVAL;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sched_yield() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sysconf(name: c_int) -> isize {
    match name {
        // POSIX _SC_PAGE_SIZE. Nagi's memory syscalls use 4096-byte pages.
        30 => 4096,
        // POSIX _SC_THREAD_STACK_MIN. Nagi's bootstrap bridge has a fixed,
        // page-aligned 16 KiB child stack and does not expose host tunables.
        75 => 4096,
        _ => -1,
    }
}

/// POSIX thread creation is a bounded adapter over Nagi's native
/// entry/argument/stack bridge. The bootstrap process admits one child at a
/// time; the child returns through `thread_exit`, never through a host ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_create(
    thread: *mut usize,
    _attributes: *const c_void,
    start: Option<PthreadStart>,
    argument: *mut c_void,
) -> c_int {
    if thread.is_null() || start.is_none() {
        return EINVAL;
    }
    let stack = crate::nagi_posix_mmap(4 * 4096, 3);
    if stack.is_null() {
        return EAGAIN;
    }
    PTHREAD_START_RECORD = PthreadStartRecord { start, argument };
    let Some(thread_id) = libnagi::thread_create(
        pthread_trampoline as usize,
        core::ptr::addr_of_mut!(PTHREAD_START_RECORD) as usize,
        stack,
        4 * 4096,
    ) else {
        let _ = crate::nagi_posix_munmap(stack, 4 * 4096);
        PTHREAD_START_RECORD = PthreadStartRecord {
            start: None,
            argument: ptr::null_mut(),
        };
        set_errno(EAGAIN);
        return EAGAIN;
    };
    PTHREAD_STACK = stack;
    thread.write(thread_id as usize);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_join(thread: usize, result: *mut *mut c_void) -> c_int {
    if thread != 1 {
        return EINVAL;
    }
    let Some(code) = libnagi::thread_join(thread as u64) else {
        return EAGAIN;
    };
    if !result.is_null() {
        result.write(code as *mut c_void);
    }
    if !PTHREAD_STACK.is_null() {
        let stack = PTHREAD_STACK;
        PTHREAD_STACK = ptr::null_mut();
        let _ = crate::nagi_posix_munmap(stack, 4 * 4096);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_self() -> usize {
    libnagi::thread_self() as usize
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub extern "C" fn pthread_equal(first: *mut c_void, second: *mut c_void) -> c_int {
    (first == second) as c_int
}

/// Store the bounded thread name in Nagi user-space thread metadata. This is
/// the target fallback used when the relibc pthread object is not selected by
/// the final link; it never consults host thread state.
#[linkage = "weak"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_setname_np(_thread: *mut c_void, name: *const c_char) -> c_int {
    if name.is_null() {
        return EINVAL;
    }
    let slot = current_thread_slot();
    let names = core::ptr::addr_of_mut!(THREAD_NAMES);
    let destination = &mut (*names)[slot];
    destination.fill(0);
    let mut length = 0;
    while length < THREAD_NAME_LENGTH - 1 {
        let byte = name.cast::<u8>().add(length).read();
        if byte == 0 {
            break;
        }
        destination[length] = byte;
        length += 1;
    }
    if length == THREAD_NAME_LENGTH - 1 && name.cast::<u8>().add(length).read() != 0 {
        destination.fill(0);
        return ERANGE;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_thread_create(
    thread: *mut usize,
    start: Option<PthreadStart>,
    argument: *mut c_void,
) -> c_int {
    pthread_create(thread, ptr::null(), start, argument)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_posix_thread_join(thread: usize, result: *mut *mut c_void) -> c_int {
    pthread_join(thread, result)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_init(_attr: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_settype(_attr: *mut c_void, _kind: c_int) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_destroy(_attr: *mut c_void) -> c_int {
    0
}

#[inline]
unsafe fn mutex_word(mutex: *mut c_void) -> *mut u32 {
    mutex.cast()
}

const CONDITION_POLL_NS: u64 = 1_000_000;

#[inline]
unsafe fn condition_sequence(condition: *mut c_void) -> *mut AtomicU32 {
    condition.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(mutex: *mut c_void, _attr: *const c_void) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    mutex_word(mutex).write(0);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut c_void) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    let lock = &*(mutex_word(mutex) as *const core::sync::atomic::AtomicU32);
    while lock
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut c_void) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    let lock = &*(mutex_word(mutex) as *const core::sync::atomic::AtomicU32);
    if lock
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        0
    } else {
        16
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut c_void) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    let lock = &*(mutex_word(mutex) as *const core::sync::atomic::AtomicU32);
    lock.store(0, Ordering::Release);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_destroy(_mutex: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_init(_attr: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_setclock(_attr: *mut c_void, _clock: c_int) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_destroy(_attr: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_init(condition: *mut c_void, _attr: *const c_void) -> c_int {
    if condition.is_null() {
        return EINVAL;
    }
    unsafe { (&*condition_sequence(condition)).store(0, Ordering::Release) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_signal(condition: *mut c_void) -> c_int {
    if condition.is_null() {
        return EINVAL;
    }
    unsafe { (&*condition_sequence(condition)).fetch_add(1, Ordering::Release) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_broadcast(condition: *mut c_void) -> c_int {
    pthread_cond_signal(condition)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> c_int {
    if condition.is_null() || mutex.is_null() {
        return EINVAL;
    }
    let expected = unsafe { (&*condition_sequence(condition)).load(Ordering::Acquire) };
    let unlock_result = pthread_mutex_unlock(mutex);
    if unlock_result != 0 {
        return unlock_result;
    }
    loop {
        let current = unsafe { (&*condition_sequence(condition)).load(Ordering::Acquire) };
        if current != expected {
            return pthread_mutex_lock(mutex);
        }
        if GuestClock.sleep_ns(CONDITION_POLL_NS).is_err() {
            let _ = pthread_mutex_lock(mutex);
            return write_errno_and_fail(ENOSYS);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_timedwait(
    condition: *mut c_void,
    mutex: *mut c_void,
    abstime: *const NagiTimespec,
) -> c_int {
    if condition.is_null() || mutex.is_null() || abstime.is_null() {
        return EINVAL;
    }
    let deadline = &*abstime;
    if deadline.tv_sec < 0 || !(0..1_000_000_000).contains(&deadline.tv_nsec) {
        return EINVAL;
    }
    let Some(deadline_ns) = (deadline.tv_sec as u64)
        .checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(deadline.tv_nsec as u64))
    else {
        return EINVAL;
    };
    let expected = (&*condition_sequence(condition)).load(Ordering::Acquire);
    let unlock_result = pthread_mutex_unlock(mutex);
    if unlock_result != 0 {
        return unlock_result;
    }

    loop {
        let current = (&*condition_sequence(condition)).load(Ordering::Acquire);
        let now = match GuestClock.realtime_ns() {
            Ok(now) => now,
            Err(_) => {
                let _ = pthread_mutex_lock(mutex);
                return write_errno_and_fail(ENOSYS);
            }
        };
        if current != expected {
            return pthread_mutex_lock(mutex);
        }
        if now >= deadline_ns {
            let lock_result = pthread_mutex_lock(mutex);
            return if lock_result == 0 {
                ETIMEDOUT
            } else {
                lock_result
            };
        }
        let remaining = deadline_ns - now;
        if GuestClock
            .sleep_ns(remaining.min(CONDITION_POLL_NS))
            .is_err()
        {
            let _ = pthread_mutex_lock(mutex);
            return write_errno_and_fail(ENOSYS);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_destroy(condition: *mut c_void) -> c_int {
    if condition.is_null() {
        return EINVAL;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_create(
    key: *mut usize,
    _destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> c_int {
    if key.is_null() {
        return EINVAL;
    }
    let value = NEXT_TLS_KEY.fetch_add(1, Ordering::Relaxed);
    if value >= TLS_SLOTS {
        return ENOMEM;
    }
    key.write(value);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_delete(key: usize) -> c_int {
    if key >= TLS_SLOTS {
        return EINVAL;
    }
    let values = core::ptr::addr_of_mut!(TLS_VALUES);
    for index in 0..THREAD_SLOTS {
        (*values)[index][key] = 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_getspecific(key: usize) -> *mut c_void {
    if key >= TLS_SLOTS {
        return ptr::null_mut();
    }
    TLS_VALUES[current_thread_slot()][key] as *mut c_void
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_setspecific(key: usize, value: *const c_void) -> c_int {
    if key >= TLS_SLOTS {
        return EINVAL;
    }
    TLS_VALUES[current_thread_slot()][key] = value as usize;
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Unwind_GetIP(_context: *mut c_void) -> usize {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Unwind_Backtrace(
    _trace: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
    _context: *mut c_void,
) -> c_int {
    0
}
