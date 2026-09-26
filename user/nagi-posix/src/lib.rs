#![no_std]
#![cfg_attr(target_os = "nagi", feature(linkage))]

pub mod errno;
pub mod fs;
pub mod net;
pub mod process;

#[cfg(any(target_os = "nagi", test))]
mod thread_diagnostics;

#[cfg(target_os = "nagi")]
mod abi;
#[cfg(target_os = "nagi")]
mod runtime;
#[cfg(target_os = "nagi")]
pub use abi::{nagi_posix_initialize_filesystem, nagi_posix_initialize_network};

/// Copy the current guest process name into a C buffer through the kernel's
/// process-info ABI. The result is NUL-terminated when capacity is nonzero.
#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn nagi_posix_copy_process_name(output: *mut u8, capacity: usize) -> isize {
    if output.is_null() || capacity == 0 {
        errno::set_errno(EINVAL);
        return -1;
    }
    let mut info = libnagi::ProcessInfo {
        pid: 0,
        parent_pid: 0,
        state: 0,
        flags: 0,
        image_pages: 0,
        stack_pages: 0,
        name: [0; libnagi::MAX_PROCESS_NAME],
    };
    if !libnagi::process_info(&mut info) {
        errno::set_errno(ENOSYS);
        return -1;
    }
    let length = info
        .name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(info.name.len())
        .min(capacity - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(info.name.as_ptr(), output, length);
        output.add(length).write(0);
    }
    length as isize
}

#[cfg(target_os = "nagi")]
pub fn nagi_posix_network_http_get(
    target: nagi_net::Ipv4Address,
    target_port: u16,
    path: &[u8],
    expected_body: &[u8],
    response: &mut [u8],
) -> Option<usize> {
    runtime::http_get(target, target_port, path, expected_body, response).ok()
}

#[cfg(target_os = "nagi")]
pub fn nagi_posix_network_default_gateway() -> Option<nagi_net::Ipv4Address> {
    runtime::default_gateway().ok()
}

use core::ptr;

#[cfg(target_os = "nagi")]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use errno::{set_errno, EBADF, EINVAL, ENOMEM, ENOSYS};
#[cfg(target_os = "nagi")]
use nagi_pal::time::{Clock, GuestClock};

#[cfg(any(target_os = "nagi", test))]
const POSIX_HEAP_SIZE: usize = 64 * 1024 * 1024;
const BLOCK_HEADER_SIZE: usize = 16;
const ALLOCATION_HEADER_SIZE: usize = 16;
const USER_POINTER_OFFSET: usize = BLOCK_HEADER_SIZE + ALLOCATION_HEADER_SIZE;
#[cfg(any(target_os = "nagi", test))]
const ALIGNED_BACKREF_OFFSET: usize = 3 * core::mem::size_of::<usize>();
#[cfg(target_os = "nagi")]
const MIN_BLOCK_SIZE: usize = USER_POINTER_OFFSET + 16;
#[cfg(target_os = "nagi")]
const ALLOCATED_MARKER: usize = usize::MAX;
#[cfg(target_os = "nagi")]
const ALLOCATION_MAGIC: usize = 0x4e41_4749_414c_4c4f;
#[cfg(target_os = "nagi")]
const ALIGNED_ALLOCATION_MAGIC: usize = 0x4e41_4749_414c_414e;

#[cfg(target_os = "nagi")]
static POSIX_HEAP_BASE: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_os = "nagi")]
static POSIX_HEAP_FREE_HEAD: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_os = "nagi")]
static POSIX_HEAP_LOCK: AtomicBool = AtomicBool::new(false);

#[cfg(any(target_os = "nagi", test))]
#[inline]
fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

fn aligned_reservation_size(size: usize, alignment: usize) -> Option<usize> {
    if alignment < core::mem::size_of::<usize>() || !alignment.is_power_of_two() {
        return None;
    }
    size.max(1)
        .checked_add(alignment.checked_sub(1)?)?
        .checked_add(USER_POINTER_OFFSET)
}

#[cfg(any(target_os = "nagi", test))]
fn aligned_allocation_layout(
    raw_pointer: usize,
    size: usize,
    alignment: usize,
) -> Option<(usize, usize)> {
    let reservation = aligned_reservation_size(size, alignment)?;
    let payload_start = raw_pointer.checked_add(USER_POINTER_OFFSET)?;
    let aligned_pointer = align_up(payload_start, alignment)?;
    let allocation_end = aligned_pointer.checked_add(size.max(1))?;
    let reservation_end = raw_pointer.checked_add(reservation)?;
    if aligned_pointer.checked_sub(raw_pointer)? < USER_POINTER_OFFSET
        || aligned_pointer.checked_sub(ALIGNED_BACKREF_OFFSET)? < raw_pointer
        || allocation_end > reservation_end
    {
        return None;
    }
    Some((aligned_pointer, reservation))
}

#[cfg(target_os = "nagi")]
#[inline]
fn acquire_heap_lock() {
    while POSIX_HEAP_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[cfg(target_os = "nagi")]
#[inline]
fn release_heap_lock() {
    POSIX_HEAP_LOCK.store(false, Ordering::Release);
}

#[cfg(target_os = "nagi")]
unsafe fn initialize_heap() -> Option<usize> {
    let existing = POSIX_HEAP_BASE.load(Ordering::Acquire);
    if existing != 0 {
        return Some(existing);
    }

    let base = libnagi::mmap_anonymous(POSIX_HEAP_SIZE, libnagi::PROT_READ | libnagi::PROT_WRITE)?
        as usize;
    if base == 0 || !base.is_multiple_of(16) {
        if base != 0 {
            let _ = libnagi::munmap(base as *mut u8, POSIX_HEAP_SIZE);
        }
        return None;
    }
    let first = base as *mut usize;
    first.write(POSIX_HEAP_SIZE);
    first.add(1).write(0);
    POSIX_HEAP_FREE_HEAD.store(base, Ordering::Release);
    POSIX_HEAP_BASE.store(base, Ordering::Release);
    Some(base)
}

#[cfg(target_os = "nagi")]
#[inline]
unsafe fn heap_bounds() -> Option<(usize, usize)> {
    let base = POSIX_HEAP_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    base.checked_add(POSIX_HEAP_SIZE).map(|end| (base, end))
}

#[cfg(target_os = "nagi")]
unsafe fn allocate_from_heap(size: usize) -> Option<*mut u8> {
    let requested = size;
    let payload = align_up(requested, 16)?;
    let required = align_up(USER_POINTER_OFFSET.checked_add(payload)?, 16)?;
    if required < MIN_BLOCK_SIZE {
        return None;
    }

    let (base, end) = heap_bounds()?;
    let mut previous = 0usize;
    let mut current = POSIX_HEAP_FREE_HEAD.load(Ordering::Acquire);
    while current != 0 {
        if current < base || current >= end || !current.is_multiple_of(16) {
            return None;
        }
        let block = current as *mut usize;
        let block_size = block.read();
        let next = block.add(1).read();
        let block_end = current.checked_add(block_size)?;
        if block_size < MIN_BLOCK_SIZE
            || !block_size.is_multiple_of(16)
            || block_end > end
            || (next != 0 && (next < base || next >= end || !next.is_multiple_of(16)))
        {
            return None;
        }
        if block_size >= required {
            let remainder = block_size - required;
            if remainder >= MIN_BLOCK_SIZE {
                let split = (current + required) as *mut usize;
                split.write(remainder);
                split.add(1).write(next);
                if previous == 0 {
                    POSIX_HEAP_FREE_HEAD.store(current + required, Ordering::Release);
                } else {
                    (previous as *mut usize).add(1).write(current + required);
                }
                block.write(required);
            } else {
                if previous == 0 {
                    POSIX_HEAP_FREE_HEAD.store(next, Ordering::Release);
                } else {
                    (previous as *mut usize).add(1).write(next);
                }
            }
            block.add(1).write(ALLOCATED_MARKER);
            let allocation_header = (current + BLOCK_HEADER_SIZE) as *mut usize;
            allocation_header.write(requested);
            allocation_header.add(1).write(ALLOCATION_MAGIC);
            return Some((current + USER_POINTER_OFFSET) as *mut u8);
        }
        previous = current;
        current = next;
    }
    None
}

#[cfg(target_os = "nagi")]
unsafe fn release_to_heap(pointer: *mut u8) {
    let Some((base, end)) = heap_bounds() else {
        return;
    };
    let mut address = pointer as usize;
    if address < base + USER_POINTER_OFFSET || address >= end {
        return;
    }
    let aligned_header = (address - ALLOCATION_HEADER_SIZE) as *mut usize;
    let requested_size = aligned_header.read();
    let magic = aligned_header.add(1).read();
    if magic == ALIGNED_ALLOCATION_MAGIC {
        let Some(backref_address) = address.checked_sub(ALIGNED_BACKREF_OFFSET) else {
            return;
        };
        let raw_pointer = (backref_address as *const usize).read();
        if raw_pointer < base + USER_POINTER_OFFSET
            || raw_pointer >= address
            || !raw_pointer.is_multiple_of(16)
        {
            return;
        }
        let Some(raw_header_address) = raw_pointer.checked_sub(ALLOCATION_HEADER_SIZE) else {
            return;
        };
        let raw_header = raw_header_address as *const usize;
        let raw_size = raw_header.read();
        if raw_header.add(1).read() != ALLOCATION_MAGIC
            || address
                .checked_add(requested_size)
                .map_or(true, |allocation_end| {
                    raw_pointer
                        .checked_add(raw_size)
                        .is_none_or(|raw_end| allocation_end > raw_end)
                })
        {
            return;
        }
        address = raw_pointer;
    } else if magic != ALLOCATION_MAGIC {
        return;
    }
    let Some(block_address) = address.checked_sub(USER_POINTER_OFFSET) else {
        return;
    };
    if address < base + USER_POINTER_OFFSET
        || address >= end
        || block_address < base
        || !block_address.is_multiple_of(16)
    {
        return;
    }

    let block = block_address as *mut usize;
    let block_size = block.read();
    let marker = block.add(1).read();
    let allocation_header = (address - ALLOCATION_HEADER_SIZE) as *mut usize;
    if marker != ALLOCATED_MARKER
        || block_size < MIN_BLOCK_SIZE
        || !block_size.is_multiple_of(16)
        || block_address
            .checked_add(block_size)
            .map_or(true, |value| value > end)
        || allocation_header.add(1).read() != ALLOCATION_MAGIC
        || allocation_header.read() > block_size - USER_POINTER_OFFSET
    {
        return;
    }

    let mut previous = 0usize;
    let mut current = POSIX_HEAP_FREE_HEAD.load(Ordering::Acquire);
    while current != 0 && current < block_address {
        previous = current;
        current = (current as *mut usize).add(1).read();
    }
    block.add(1).write(current);
    if previous == 0 {
        POSIX_HEAP_FREE_HEAD.store(block_address, Ordering::Release);
    } else {
        (previous as *mut usize).add(1).write(block_address);
    }

    if current != 0 {
        let current_size = (current as *mut usize).read();
        if block_address + block_size == current {
            block.write(block_size + current_size);
            block.add(1).write((current as *mut usize).add(1).read());
        }
    }
    if previous != 0 {
        let previous_block = previous as *mut usize;
        let previous_size = previous_block.read();
        if previous + previous_size == block_address {
            previous_block.write(previous_size + block.read());
            previous_block.add(1).write(block.add(1).read());
        }
    }
}

/// Allocate a POSIX-aligned block from the bounded guest heap.
///
/// The ordinary allocator returns 16-byte-aligned pointers. This entry point
/// reserves alignment slack inside the same bounded heap and records the
/// original allocation pointer immediately before the aligned result so that
/// `free`, `realloc`, and `malloc_usable_size` can retain their existing ABI.
#[no_mangle]
pub extern "C" fn nagi_posix_malloc_aligned(size: usize, alignment: usize) -> *mut u8 {
    if alignment < core::mem::size_of::<usize>() || !alignment.is_power_of_two() {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    let Some(reservation) = aligned_reservation_size(size, alignment) else {
        set_errno(ENOMEM);
        return ptr::null_mut();
    };
    #[cfg(target_os = "nagi")]
    {
        let raw_pointer = nagi_posix_malloc(reservation);
        if raw_pointer.is_null() {
            return ptr::null_mut();
        }
        let Some((aligned_pointer, _)) =
            aligned_allocation_layout(raw_pointer as usize, size, alignment)
        else {
            nagi_posix_free(raw_pointer);
            set_errno(ENOMEM);
            return ptr::null_mut();
        };
        let header = (aligned_pointer - ALIGNED_BACKREF_OFFSET) as *mut usize;
        unsafe {
            header.write(raw_pointer as usize);
            header.add(1).write(size);
            header.add(2).write(ALIGNED_ALLOCATION_MAGIC);
        }
        aligned_pointer as *mut u8
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = reservation;
        set_errno(ENOSYS);
        ptr::null_mut()
    }
}

/// C-compatible bounded allocation for guest libraries.
#[no_mangle]
pub extern "C" fn nagi_posix_malloc(size: usize) -> *mut u8 {
    if size == 0 {
        set_errno(EINVAL);
        return ptr::null_mut();
    }
    #[cfg(target_os = "nagi")]
    {
        acquire_heap_lock();
        let (heap_ready, result) = unsafe {
            let heap_ready = initialize_heap().is_some();
            let result = if heap_ready {
                allocate_from_heap(size)
            } else {
                None
            };
            (heap_ready, result)
        };
        release_heap_lock();
        if let Some(pointer) = result {
            return pointer;
        }
        let diagnostic = if heap_ready {
            b"Nagi M17 trace: POSIX allocator returned no block\r\n" as &[u8]
        } else {
            b"Nagi M17 trace: POSIX heap mapping unavailable\r\n"
        };
        let _ = libnagi::console_write(diagnostic);
        set_errno(ENOMEM);
        ptr::null_mut()
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = size;
        set_errno(ENOSYS);
        ptr::null_mut()
    }
}

/// Return the requested payload size recorded for a Nagi allocation.
///
/// This is the Nagi-owned implementation behind libc's
/// `malloc_usable_size`. It reports the validated payload extent rather than
/// exposing the allocator's internal block size or any host allocator state.
///
/// # Safety
///
/// `pointer` must be a pointer previously returned by
/// [`nagi_posix_malloc`] and not yet released by [`nagi_posix_free`].
#[no_mangle]
pub unsafe extern "C" fn nagi_posix_malloc_usable_size(pointer: *mut u8) -> usize {
    #[cfg(target_os = "nagi")]
    {
        if pointer.is_null() {
            return 0;
        }
        let header = unsafe { pointer.sub(ALLOCATION_HEADER_SIZE).cast::<usize>() };
        let magic = unsafe { header.add(1).read() };
        if magic != ALLOCATION_MAGIC && magic != ALIGNED_ALLOCATION_MAGIC {
            return 0;
        }
        return unsafe { header.read() };
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = pointer;
        0
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_free(pointer: *mut u8) {
    #[cfg(target_os = "nagi")]
    {
        if pointer.is_null() {
            return;
        }
        acquire_heap_lock();
        unsafe { release_to_heap(pointer) };
        release_heap_lock();
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = pointer;
    }
}

/// POSIX-like write for stdout/stderr. The bytes cross only the existing Nagi
/// console syscall boundary; no host file descriptor is consulted.
///
/// # Safety
///
/// `bytes` must be readable for `length` bytes when called in the Nagi guest.
#[no_mangle]
pub unsafe extern "C" fn nagi_posix_write(fd: i32, bytes: *const u8, length: usize) -> isize {
    if fd != 1 && fd != 2 {
        set_errno(EBADF);
        return -1;
    }
    if bytes.is_null() || length == 0 || length > libnagi::MAX_CONSOLE_WRITE {
        set_errno(EINVAL);
        return -1;
    }
    #[cfg(target_os = "nagi")]
    {
        let slice = core::slice::from_raw_parts(bytes, length);
        if libnagi::console_write(slice) == length {
            return length as isize;
        }
        set_errno(5);
        return -1;
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = (bytes, length);
        set_errno(ENOSYS);
        -1
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_mmap(_length: usize, _protection: i32) -> *mut u8 {
    #[cfg(target_os = "nagi")]
    {
        if _length == 0 || !_length.is_multiple_of(4096) || !(_protection >= 0 && _protection <= 7)
        {
            set_errno(EINVAL);
            return ptr::null_mut();
        }
        return libnagi::mmap_anonymous(_length, _protection as u64).unwrap_or_else(|| {
            set_errno(ENOMEM);
            ptr::null_mut()
        });
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = (_length, _protection);
        set_errno(ENOSYS);
        ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_mmap_at(
    _address: *mut u8,
    _length: usize,
    _protection: i32,
) -> *mut u8 {
    #[cfg(target_os = "nagi")]
    {
        if _address.is_null()
            || !(_address as usize).is_multiple_of(4096)
            || _length == 0
            || !_length.is_multiple_of(4096)
            || !(_protection >= 0 && _protection <= 7)
        {
            set_errno(EINVAL);
            return ptr::null_mut();
        }
        return libnagi::mmap_anonymous_at(_address, _length, _protection as u64).unwrap_or_else(
            || {
                set_errno(ENOMEM);
                ptr::null_mut()
            },
        );
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = (_address, _length, _protection);
        set_errno(ENOSYS);
        ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_munmap(address: *mut u8, length: usize) -> i32 {
    #[cfg(target_os = "nagi")]
    {
        if address.is_null() || length == 0 || !length.is_multiple_of(4096) {
            set_errno(EINVAL);
            return -1;
        }
        return if libnagi::munmap(address, length) {
            0
        } else {
            set_errno(EINVAL);
            -1
        };
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = (address, length);
        set_errno(ENOSYS);
        -1
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_mprotect(address: *mut u8, length: usize, protection: i32) -> i32 {
    #[cfg(target_os = "nagi")]
    {
        if address.is_null()
            || length == 0
            || !length.is_multiple_of(4096)
            || !(0..=7).contains(&protection)
        {
            set_errno(EINVAL);
            return -1;
        }
        return if libnagi::mprotect(address, length, protection as u64) {
            0
        } else {
            set_errno(EINVAL);
            -1
        };
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = (address, length, protection);
        set_errno(ENOSYS);
        -1
    }
}

#[no_mangle]
pub extern "C" fn nagi_posix_poll(timeout_ms: i32) -> i32 {
    if timeout_ms < 0 {
        set_errno(EINVAL);
        return -1;
    }
    #[cfg(target_os = "nagi")]
    {
        let duration = (timeout_ms as u64).saturating_mul(1_000_000);
        return if GuestClock.sleep_ns(duration).is_ok() {
            0
        } else {
            set_errno(ENOSYS);
            -1
        };
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = timeout_ms;
        set_errno(ENOSYS);
        -1
    }
}

/// Explicit Nagi PAL sleep entry used by compatibility tests and by the
/// target-specific relibc adapter while the full upstream POSIX symbol table
/// is being wired to the Nagi platform module.
#[no_mangle]
pub extern "C" fn nagi_posix_sleep_ns(duration: u64) -> i32 {
    #[cfg(target_os = "nagi")]
    {
        return if GuestClock.sleep_ns(duration).is_ok() {
            0
        } else {
            set_errno(ENOSYS);
            -1
        };
    }
    #[cfg(not(target_os = "nagi"))]
    {
        let _ = duration;
        set_errno(ENOSYS);
        -1
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aligned_allocation_layout, aligned_reservation_size, nagi_posix_malloc, nagi_posix_mmap,
        nagi_posix_mprotect, nagi_posix_munmap, nagi_posix_poll, nagi_posix_write,
        ALIGNED_BACKREF_OFFSET, POSIX_HEAP_SIZE, USER_POINTER_OFFSET,
    };
    use crate::errno::{errno, EINVAL, ENOSYS};

    #[test]
    fn servo_posix_heap_budget_is_64_mib() {
        assert_eq!(POSIX_HEAP_SIZE, 64 * 1024 * 1024);
    }

    #[test]
    fn posix_aligned_reservations_cover_padding_metadata_and_payload() {
        for (raw_pointer, alignment) in [(0x10000, 16), (0x10010, 64), (0x10030, 4096)] {
            let size = 512;
            let (aligned_pointer, reservation) =
                aligned_allocation_layout(raw_pointer, size, alignment).expect("layout");
            assert!(aligned_pointer.is_multiple_of(alignment));
            assert!(aligned_pointer - raw_pointer >= USER_POINTER_OFFSET);
            assert!(aligned_pointer - ALIGNED_BACKREF_OFFSET >= raw_pointer);
            assert!(aligned_pointer + size <= raw_pointer + reservation);
            assert_eq!(
                reservation,
                aligned_reservation_size(size, alignment).unwrap()
            );
        }
    }

    #[test]
    fn posix_aligned_reservations_reject_invalid_alignment_and_overflow() {
        for alignment in [0, 3, 4] {
            assert_eq!(aligned_reservation_size(512, alignment), None);
            assert_eq!(aligned_allocation_layout(0x10000, 512, alignment), None);
        }
        assert_eq!(aligned_reservation_size(usize::MAX, 16), None);
        assert_eq!(
            aligned_reservation_size(usize::MAX, 1usize << (usize::BITS - 1)),
            None
        );
    }

    #[test]
    fn invalid_c_arguments_fail_closed() {
        assert!(nagi_posix_malloc(0).is_null());
        assert_eq!(errno(), EINVAL);
        assert_eq!(unsafe { nagi_posix_write(1, core::ptr::null(), 1) }, -1);
        assert_eq!(errno(), EINVAL);
        assert!(nagi_posix_mmap(4096, (libnagi::PROT_READ | libnagi::PROT_WRITE) as i32).is_null());
        assert_eq!(errno(), ENOSYS);
        assert_eq!(
            nagi_posix_mprotect(
                core::ptr::null_mut(),
                4096,
                (libnagi::PROT_READ | libnagi::PROT_WRITE) as i32,
            ),
            -1
        );
        assert_eq!(errno(), ENOSYS);
        assert_eq!(nagi_posix_munmap(core::ptr::null_mut(), 4096), -1);
        assert_eq!(errno(), ENOSYS);
        assert_eq!(nagi_posix_poll(0), -1);
        assert_eq!(errno(), ENOSYS);
    }
}
