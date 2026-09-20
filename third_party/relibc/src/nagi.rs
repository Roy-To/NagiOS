//! Nagi user-space backend for the pinned relibc source.
//!
//! The upstream relibc headers are organized around Linux and Redox system
//! calls. Nagi deliberately does not expose either syscall ABI. This backend
//! therefore keeps the relibc C symbol surface but forwards each operation to
//! the Nagi POSIX facade, which in turn uses Nagi VFS and services.

use core::{
    ffi::{c_char, c_double, c_float, c_int, c_long, c_longlong, c_ulong, c_ulonglong, c_void},
    mem, ptr,
};

unsafe extern "C" {
    fn nagi_posix_malloc(size: usize) -> *mut u8;
    fn nagi_posix_malloc_usable_size(pointer: *mut u8) -> usize;
    fn nagi_posix_free(pointer: *mut u8);
    fn nagi_posix_write_fd(fd: c_int, bytes: *const u8, length: usize) -> isize;
    fn nagi_posix_initialize_filesystem(capability: u64) -> c_int;
    fn nagi_posix_open(path: *const c_char, flags: c_int, mode: c_int) -> c_int;
    fn nagi_posix_read(fd: c_int, bytes: *mut u8, length: usize) -> isize;
    fn nagi_posix_close(fd: c_int) -> c_int;
    fn nagi_posix_lseek(fd: c_int, offset: i64, whence: c_int) -> i64;
    fn nagi_posix_mmap_file(length: usize, protection: c_int, fd: c_int, offset: usize) -> *mut u8;
    fn nagi_posix_mmap_at(address: *mut u8, length: usize, protection: c_int) -> *mut u8;
    fn nagi_posix_munmap(address: *mut u8, length: usize) -> c_int;
    fn nagi_posix_mprotect(address: *mut u8, length: usize, protection: c_int) -> c_int;
    fn nagi_posix_errno_location() -> *mut c_int;
}

const EINVAL: c_int = 22;
const ENOMEM: c_int = 12;
const EBADF: c_int = 9;
const EOVERFLOW: c_int = 75;
const ERANGE: c_int = 34;
const EOF: c_int = -1;
const MAP_FIXED: c_int = 0x0010;
const MAP_ANONYMOUS: c_int = 0x0020;

const NAGI_FILE_MEMORY: u32 = 1;

/// The target build does not compile relibc's Linux/Redox stdio module.  Keep
/// the opaque C FILE ABI small and Nagi-owned for the real target facilities
/// that Mesa needs.  A memory stream owns its backing allocation until the
/// caller closes the stream; the caller then owns `*bufp`, as required by
/// POSIX `open_memstream`.
#[repr(C)]
struct NagiFile {
    kind: u32,
    buffer: *mut u8,
    length: usize,
    capacity: usize,
    bufp: *mut *mut c_char,
    sizep: *mut usize,
}

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
