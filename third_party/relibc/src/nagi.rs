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
    fmt, mem, ptr,
};

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
    fn nagi_posix_fdopendir(fd: c_int) -> *mut c_void;
    fn nagi_posix_resolve_ipv4(name: *const c_char, output: *mut NagiIpv4Address) -> c_int;
    fn nagi_posix_mmap_file(length: usize, protection: c_int, fd: c_int, offset: usize) -> *mut u8;
    fn nagi_posix_mmap_at(address: *mut u8, length: usize, protection: c_int) -> *mut u8;
    fn nagi_posix_munmap(address: *mut u8, length: usize) -> c_int;
    fn nagi_posix_mprotect(address: *mut u8, length: usize, protection: c_int) -> c_int;
    fn nagi_posix_errno_location() -> *mut c_int;
    fn abort() -> !;
}

const EINVAL: c_int = 22;
const ENOSYS: c_int = 38;
const ENOMEM: c_int = 12;
const EBADF: c_int = 9;
const EOVERFLOW: c_int = 75;
const ERANGE: c_int = 34;
const EOF: c_int = -1;
const MAP_FIXED: c_int = 0x0010;
const MAP_ANONYMOUS: c_int = 0x0020;

const NAGI_FILE_MEMORY: u32 = 1;
const NAGI_FILE_FD: u32 = 2;

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
    buffer: ptr::null_mut(),
    length: 0,
    capacity: 0,
    bufp: ptr::null_mut(),
    sizep: ptr::null_mut(),
};

#[unsafe(no_mangle)]
pub static mut stderr: *mut c_void = ptr::addr_of_mut!(NAGI_STDERR).cast();

#[inline]
unsafe fn set_errno(error: c_int) {
    let location = unsafe { nagi_posix_errno_location() };
    if !location.is_null() {
        unsafe { location.write(error) };
    }
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
            buffer: ptr::null_mut(),
            length: 0,
            capacity: 0,
            bufp,
            sizep,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(stream: *mut c_void) -> c_int {
    if stream.is_null() {
        unsafe { set_errno(EINVAL) };
        return EOF;
    }
    let stream = unsafe { &mut *stream.cast::<NagiFile>() };
    if stream.kind == NAGI_FILE_FD {
        return if unsafe { nagi_posix_close(stream.fd) } == 0 {
            0
        } else {
            EOF
        };
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
pub unsafe extern "C" fn fdopendir(fd: c_int) -> *mut c_void {
    unsafe { nagi_posix_fdopendir(fd) }
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
