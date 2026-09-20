//! Nagi user-space backend for the pinned relibc source.
//!
//! The upstream relibc headers are organized around Linux and Redox system
//! calls. Nagi deliberately does not expose either syscall ABI. This backend
//! therefore keeps the relibc C symbol surface but forwards each operation to
//! the Nagi POSIX facade, which in turn uses Nagi VFS and services.

use core::{
    ffi::{c_char, c_int, c_void},
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
    fn nagi_posix_munmap(address: *mut u8, length: usize) -> c_int;
    fn nagi_posix_mprotect(address: *mut u8, length: usize, protection: c_int) -> c_int;
    fn nagi_posix_errno_location() -> *mut c_int;
}

const EINVAL: c_int = 22;
const ENOMEM: c_int = 12;
const EBADF: c_int = 9;
const EOVERFLOW: c_int = 75;
const EOF: c_int = -1;

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
    _flags: c_int,
    fd: c_int,
    offset: i64,
) -> *mut c_void {
    if !address.is_null() || offset < 0 {
        unsafe { set_errno(EINVAL) };
        return usize::MAX as *mut c_void;
    }

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
