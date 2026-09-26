#![no_std]

pub mod boot;
pub mod security;
pub mod service;
pub mod storage;

use core::arch::asm;
use core::mem::MaybeUninit;
use core::ptr;

pub use nagi_abi::{
    DisplayInfo, InputEvent, MemoryInfo, ProcessInfo, BLOCK_SECTOR_SIZE,
    BOOTSTRAP_USER_THREAD_COUNT, INPUT_EVENT_ABS, INPUT_EVENT_KEY, INPUT_EVENT_REL, INPUT_KEY_LEFT,
    INPUT_REL_X, INPUT_REL_Y, MAX_AUDIO_BUFFER, MAX_CONSOLE_READ, MAX_CONSOLE_WRITE, MAX_LOG_READ,
    MAX_NET_FRAME_SIZE, MAX_PROCESS_NAME, MAX_RANDOM_BYTES, PIXEL_FORMAT_RGBA8888, PROT_EXEC,
    PROT_NONE, PROT_READ, PROT_WRITE, SURFACE_BYTES, SURFACE_HEIGHT, SURFACE_WIDTH,
    SYS_AUDIO_CAPTURE, SYS_AUDIO_PLAY, SYS_BLOCK_FLUSH, SYS_BLOCK_READ, SYS_BLOCK_WRITE,
    SYS_CONSOLE_READ, SYS_CONSOLE_WRITE, SYS_DISPLAY_INFO, SYS_DISPLAY_PRESENT, SYS_INPUT_READ,
    SYS_LOG_READ, SYS_MEMORY_INFO, SYS_MEMORY_MAP, SYS_MEMORY_MAP_AT, SYS_MEMORY_PROTECT,
    SYS_MEMORY_UNMAP, SYS_NET_RECEIVE, SYS_NET_SEND, SYS_PROCESS_EXIT, SYS_PROCESS_INFO,
    SYS_RANDOM_GET, SYS_THREAD_CREATE, SYS_THREAD_DETACH, SYS_THREAD_EXIT, SYS_THREAD_JOIN,
    SYS_THREAD_SELF, SYS_THREAD_SLEEP, SYS_TIME_READ, SYS_TIME_REALTIME, THREAD_CREATE_DETACHED,
};

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    let original = destination;
    let mut index = 0;
    while index < count {
        let byte = unsafe { core::ptr::read_volatile(source.add(index)) };
        unsafe { core::ptr::write_volatile(destination.add(index), byte) };
        index += 1;
    }
    original
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn memmove(destination: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    if (destination as usize) <= (source as usize) {
        unsafe { memcpy(destination, source, count) };
    } else {
        let mut index = count;
        while index != 0 {
            index -= 1;
            let byte = unsafe { core::ptr::read_volatile(source.add(index)) };
            unsafe { core::ptr::write_volatile(destination.add(index), byte) };
        }
    }
    destination
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, count: usize) -> *mut u8 {
    let byte = value as u8;
    let mut index = 0;
    while index < count {
        unsafe { core::ptr::write_volatile(destination.add(index), byte) };
        index += 1;
    }
    destination
}

/// Checked copy entry point emitted by Clang for fortified C memory calls.
///
/// The destination bound is supplied by the caller's object-size analysis;
/// violating it terminates the guest process instead of silently weakening
/// the check.  The implementation remains target-owned and does not call a
/// host libc routine.
#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn __memcpy_chk(
    destination: *mut u8,
    source: *const u8,
    count: usize,
    destination_size: usize,
) -> *mut u8 {
    if count > destination_size
        || (count != 0 && (destination.is_null() || source.is_null()))
        || destination.cast::<u8>().addr().checked_add(count).is_none()
    {
        exit(134);
    }
    memcpy(destination, source, count)
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, count: usize) -> i32 {
    let mut index = 0;
    while index < count {
        let left_byte = unsafe { core::ptr::read_volatile(left.add(index)) };
        let right_byte = unsafe { core::ptr::read_volatile(right.add(index)) };
        if left_byte != right_byte {
            return i32::from(left_byte) - i32::from(right_byte);
        }
        index += 1;
    }
    0
}

#[cfg(target_arch = "x86_64")]
#[repr(C, align(16))]
struct FpuState([u8; 512]);

#[cfg(target_arch = "x86_64")]
pub fn fpu_state_is_initial() -> bool {
    let mut state = MaybeUninit::<FpuState>::uninit();
    let address = state.as_mut_ptr().cast::<u8>();
    unsafe {
        asm!(
            "fxsave64 [{address}]",
            address = in(reg) address,
            options(nostack, preserves_flags),
        );
    }
    let control_word = unsafe { ptr::read_unaligned(address.cast::<u16>()) };
    let status_word = unsafe { ptr::read_unaligned(address.add(2).cast::<u16>()) };
    let tag_word = unsafe { ptr::read_unaligned(address.add(4).cast::<u8>()) };
    let mxcsr = unsafe { ptr::read_unaligned(address.add(24).cast::<u32>()) };
    let mut offset = 32;
    while offset < 416 {
        if unsafe { ptr::read(address.add(offset)) } != 0 {
            return false;
        }
        offset += 1;
    }
    control_word == 0x037f && status_word == 0 && tag_word == 0 && mxcsr == 0x1f80
}

#[inline]
pub fn console_write(bytes: &[u8]) -> usize {
    let count = bytes.len().min(MAX_CONSOLE_WRITE);
    let mut result = SYS_CONSOLE_WRITE;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") bytes.as_ptr(),
            in("rsi") count,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result as usize
}

#[inline]
pub fn console_read(byte: &mut [u8; MAX_CONSOLE_READ]) -> bool {
    let mut result = SYS_CONSOLE_READ;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") byte.as_mut_ptr(),
            in("rsi") MAX_CONSOLE_READ,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == MAX_CONSOLE_READ as u64
}

#[inline]
pub fn process_info(info: &mut ProcessInfo) -> bool {
    let mut result = SYS_PROCESS_INFO;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") info as *mut ProcessInfo,
            in("rsi") core::mem::size_of::<ProcessInfo>(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == core::mem::size_of::<ProcessInfo>() as u64
}

#[inline]
pub fn memory_info(info: &mut MemoryInfo) -> bool {
    let mut result = SYS_MEMORY_INFO;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") info as *mut MemoryInfo,
            in("rsi") core::mem::size_of::<MemoryInfo>(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == core::mem::size_of::<MemoryInfo>() as u64
}

#[inline]
pub fn log_read(buffer: &mut [u8]) -> usize {
    let count = buffer.len().min(MAX_LOG_READ);
    if count == 0 {
        return 0;
    }
    let mut result = SYS_LOG_READ;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") buffer.as_mut_ptr(),
            in("rsi") count,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    if result == u64::MAX {
        0
    } else {
        usize::try_from(result).unwrap_or(0).min(count)
    }
}

#[inline]
pub fn display_info(info: &mut DisplayInfo) -> bool {
    let mut result = SYS_DISPLAY_INFO;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") info as *mut DisplayInfo,
            in("rsi") core::mem::size_of::<DisplayInfo>(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == core::mem::size_of::<DisplayInfo>() as u64
}

#[inline]
pub fn display_present(capability: u64) -> bool {
    let mut result = SYS_DISPLAY_PRESENT;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result != u64::MAX
}

#[inline]
pub fn input_read(capability: u64, event: &mut InputEvent) -> bool {
    let mut result = SYS_INPUT_READ;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") event as *mut InputEvent,
            in("rdx") core::mem::size_of::<InputEvent>(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == core::mem::size_of::<InputEvent>() as u64
}

#[inline]
pub fn block_read(capability: u64, sector: u64, buffer: &mut [u8; BLOCK_SECTOR_SIZE]) -> bool {
    let mut result = SYS_BLOCK_READ;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") sector,
            in("rdx") buffer.as_mut_ptr(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == BLOCK_SECTOR_SIZE as u64
}

#[inline]
pub fn block_write(capability: u64, sector: u64, buffer: &[u8; BLOCK_SECTOR_SIZE]) -> bool {
    let mut result = SYS_BLOCK_WRITE;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") sector,
            in("rdx") buffer.as_ptr(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == BLOCK_SECTOR_SIZE as u64
}

/// Flush writes through the capability-authorized block device when its
/// negotiated interface supports durable flushes.
#[inline]
pub fn block_flush(capability: u64) -> bool {
    let mut result = SYS_BLOCK_FLUSH;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == 0
}

#[inline]
pub fn audio_play(capability: u64, stream_id: u64, data: &[u8]) -> bool {
    if data.is_empty() || data.len() > MAX_AUDIO_BUFFER {
        return false;
    }
    let mut result = SYS_AUDIO_PLAY;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") stream_id,
            in("rdx") data.as_ptr(),
            in("r10") data.len(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == data.len() as u64
}

#[inline]
pub fn audio_capture(capability: u64, stream_id: u64, data: &mut [u8]) -> usize {
    if data.is_empty() || data.len() > MAX_AUDIO_BUFFER {
        return 0;
    }
    let mut result = SYS_AUDIO_CAPTURE;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") stream_id,
            in("rdx") data.as_mut_ptr(),
            in("r10") data.len(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    if result == u64::MAX {
        0
    } else {
        usize::try_from(result).unwrap_or(0).min(data.len())
    }
}

#[inline]
pub fn net_send(capability: u64, frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > MAX_NET_FRAME_SIZE {
        return false;
    }
    let mut result = SYS_NET_SEND;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") frame.as_ptr(),
            in("rdx") frame.len(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == frame.len() as u64
}

#[inline]
pub fn net_receive(capability: u64, frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> usize {
    let mut result = SYS_NET_RECEIVE;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") capability,
            in("rsi") frame.as_mut_ptr(),
            in("rdx") MAX_NET_FRAME_SIZE,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    if result == u64::MAX {
        0
    } else {
        usize::try_from(result).unwrap_or(0).min(MAX_NET_FRAME_SIZE)
    }
}

/// Fill a user buffer using Nagi's guest VirtIO RNG boundary.
#[cfg(target_os = "nagi")]
pub fn random_fill(bytes: &mut [u8]) -> bool {
    let mut offset = 0;
    while offset < bytes.len() {
        let count = core::cmp::min(MAX_RANDOM_BYTES, bytes.len() - offset);
        let mut result = SYS_RANDOM_GET;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") result,
                in("rdi") bytes[offset..offset + count].as_mut_ptr(),
                in("rsi") count,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
        if result != count as u64 {
            return false;
        }
        offset += count;
    }
    true
}

/// Fill a buffer from Nagi's guest VirtIO RNG syscall through a stable C ABI.
#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn __nagi_random_fill(destination: *mut u8, length: usize) -> i32 {
    if length == 0 {
        return 0;
    }
    if destination.is_null() {
        return -1;
    }
    let destination = unsafe { core::slice::from_raw_parts_mut(destination, length) };
    if random_fill(destination) {
        0
    } else {
        -1
    }
}

/// Rust std's Nagi random backend calls this compatibility entry point to seed
/// `RandomState` from the guest VirtIO RNG syscall.
#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "C" fn __nagi_std_random_fill(destination: *mut u8, length: usize) -> i32 {
    unsafe { __nagi_random_fill(destination, length) }
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    destination: *mut u8,
    length: usize,
) -> Result<(), getrandom::Error> {
    if length == 0 {
        return Ok(());
    }
    if destination.is_null() {
        return Err(getrandom::Error::new_custom(1));
    }
    let destination = unsafe { core::slice::from_raw_parts_mut(destination, length) };
    if random_fill(destination) {
        Ok(())
    } else {
        Err(getrandom::Error::new_custom(1))
    }
}

#[inline]
pub fn time_ticks() -> u64 {
    let mut result = SYS_TIME_READ;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result
}

#[inline]
pub fn time_realtime_ns() -> Option<u64> {
    let mut result = SYS_TIME_REALTIME;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (result != u64::MAX).then_some(result)
}

#[inline]
pub fn sleep_ns(duration: u64) -> bool {
    let mut result = SYS_THREAD_SLEEP;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") duration,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == 0
}

#[inline]
pub fn thread_yield() -> bool {
    sleep_ns(0)
}

#[inline]
pub fn mmap_anonymous(length: usize, protection: u64) -> Option<*mut u8> {
    let mut result = SYS_MEMORY_MAP;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") length as u64,
            in("rsi") protection,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (result != u64::MAX).then_some(result as *mut u8)
}

#[inline]
pub fn mmap_anonymous_at(address: *mut u8, length: usize, protection: u64) -> Option<*mut u8> {
    let mut result = SYS_MEMORY_MAP_AT;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") address as u64,
            in("rsi") length as u64,
            in("rdx") protection,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (result != u64::MAX).then_some(result as *mut u8)
}

#[inline]
pub fn munmap(address: *mut u8, length: usize) -> bool {
    let mut result = SYS_MEMORY_UNMAP;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") address as u64,
            in("rsi") length as u64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == 0
}

#[inline]
pub fn mprotect(address: *mut u8, length: usize, protection: u64) -> bool {
    let mut result = SYS_MEMORY_PROTECT;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") address as u64,
            in("rsi") length as u64,
            in("rdx") protection,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == 0
}

/// Create a bounded native child thread in the current Nagi process. The
/// entry point and stack remain inside its mapped address space; creation
/// enqueues the child without yielding to a host thread or runtime.
#[inline]
pub fn thread_create(
    entry: usize,
    argument: usize,
    stack: *mut u8,
    stack_size: usize,
) -> Option<u64> {
    thread_create_with_flags(entry, argument, stack, stack_size, 0)
}

#[inline]
pub fn thread_create_detached(
    entry: usize,
    argument: usize,
    stack: *mut u8,
    stack_size: usize,
) -> Option<u64> {
    thread_create_with_flags(entry, argument, stack, stack_size, THREAD_CREATE_DETACHED)
}

#[inline]
fn thread_create_with_flags(
    entry: usize,
    argument: usize,
    stack: *mut u8,
    stack_size: usize,
    flags: u64,
) -> Option<u64> {
    let mut result = SYS_THREAD_CREATE;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") entry as u64,
            in("rsi") argument as u64,
            in("rdx") stack as u64,
            in("r10") stack_size as u64,
            in("r8") flags,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (result != u64::MAX).then_some(result)
}

#[inline]
pub fn thread_join(thread: u64) -> Option<u64> {
    let mut exit_code = 0_u64;
    let mut result = SYS_THREAD_JOIN;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") thread,
            in("rsi") core::ptr::addr_of_mut!(exit_code),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (result == 0).then_some(exit_code)
}

#[inline]
pub fn thread_detach(thread: u64) -> bool {
    let mut result = SYS_THREAD_DETACH;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            in("rdi") thread,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result == 0
}

#[inline]
pub fn thread_exit(code: u64) -> ! {
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_THREAD_EXIT,
            in("rdi") code,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    loop {
        core::hint::spin_loop();
    }
}

#[inline]
pub fn thread_self() -> u64 {
    let mut result = SYS_THREAD_SELF;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") result,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result
}

#[inline]
pub fn exit(code: u64) -> ! {
    unsafe {
        asm!(
            "syscall",
            in("rax") SYS_PROCESS_EXIT,
            in("rdi") code,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_bootstrap_syscalls_are_stable() {
        assert_eq!(SYS_CONSOLE_WRITE, 1);
        assert_eq!(SYS_PROCESS_EXIT, 2);
        assert_eq!(SYS_BLOCK_READ, 3);
        assert_eq!(SYS_BLOCK_WRITE, 4);
        assert_eq!(SYS_BLOCK_FLUSH, 28);
        assert_eq!(BLOCK_SECTOR_SIZE, 512);
        assert_eq!(MAX_CONSOLE_WRITE, 256);
        assert_eq!(SYS_CONSOLE_READ, 5);
        assert_eq!(SYS_PROCESS_INFO, 6);
        assert_eq!(SYS_MEMORY_INFO, 7);
        assert_eq!(SYS_LOG_READ, 8);
        assert_eq!(SYS_DISPLAY_INFO, 9);
        assert_eq!(SYS_DISPLAY_PRESENT, 10);
        assert_eq!(SYS_INPUT_READ, 11);
        assert_eq!(SYS_NET_SEND, 12);
        assert_eq!(SYS_NET_RECEIVE, 13);
        assert_eq!(SYS_TIME_READ, 14);
        assert_eq!(SYS_TIME_REALTIME, 15);
        assert_eq!(SYS_THREAD_SLEEP, 16);
        assert_eq!(SYS_MEMORY_MAP, 17);
        assert_eq!(SYS_MEMORY_MAP_AT, 27);
        assert_eq!(SYS_MEMORY_UNMAP, 18);
        assert_eq!(SYS_MEMORY_PROTECT, 19);
        assert_eq!(SYS_THREAD_CREATE, 20);
        assert_eq!(SYS_THREAD_JOIN, 21);
        assert_eq!(SYS_THREAD_EXIT, 22);
        assert_eq!(SYS_THREAD_SELF, 23);
        assert_eq!(SYS_THREAD_DETACH, 29);
        assert_eq!(THREAD_CREATE_DETACHED, 1);
        assert_eq!(BOOTSTRAP_USER_THREAD_COUNT, 16);
        assert_eq!(nagi_abi::SYS_AUDIO_PLAY, 24);
        assert_eq!(nagi_abi::SYS_AUDIO_CAPTURE, 25);
        assert_eq!(SYS_RANDOM_GET, 26);
        assert_eq!(MAX_RANDOM_BYTES, 256);
        assert_eq!(MAX_NET_FRAME_SIZE, 1536);
        assert_eq!(MAX_CONSOLE_READ, 1);
        assert_eq!(MAX_LOG_READ, 512);
        assert_eq!(core::mem::size_of::<ProcessInfo>(), 48);
        assert_eq!(core::mem::size_of::<MemoryInfo>(), 56);
        assert_eq!(core::mem::size_of::<DisplayInfo>(), 32);
        assert_eq!(core::mem::size_of::<InputEvent>(), 8);
        assert_eq!(SURFACE_BYTES, 256_000);
    }
}
