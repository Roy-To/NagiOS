#[cfg(test)]
use crate::user_elf::{USER_IMAGE_BASE, USER_IMAGE_LIMIT};
#[cfg(test)]
use crate::user_process::{
    USER_MMAP_BASE, USER_MMAP_LIMIT, USER_STACK_BASE, USER_STACK_LIMIT, USER_TLS_BASE,
    USER_TLS_LIMIT,
};
#[cfg(not(test))]
use nagi_kernel::user_elf::{USER_IMAGE_BASE, USER_IMAGE_LIMIT};
#[cfg(not(test))]
use nagi_kernel::user_process::{
    USER_MMAP_BASE, USER_MMAP_LIMIT, USER_STACK_BASE, USER_STACK_LIMIT, USER_TLS_BASE,
    USER_TLS_CHILD_CONTROL_BASE, USER_TLS_CONTROL_BASE, USER_TLS_LIMIT,
};

#[cfg(not(test))]
use core::arch::{asm, global_asm};

#[cfg(not(test))]
use super::{halt_forever, interrupts, serial_log_read, serial_read_byte, serial_write};

#[cfg(not(test))]
use core::sync::atomic::{AtomicU8, Ordering};

pub use nagi_abi::{
    BLOCK_SECTOR_SIZE, MAX_CONSOLE_READ, MAX_CONSOLE_WRITE, MAX_LOG_READ, MAX_RANDOM_BYTES,
    SYS_AUDIO_CAPTURE, SYS_AUDIO_PLAY, SYS_BLOCK_FLUSH, SYS_BLOCK_READ, SYS_BLOCK_WRITE,
    SYS_CONSOLE_READ, SYS_CONSOLE_WRITE, SYS_DISPLAY_INFO, SYS_DISPLAY_PRESENT, SYS_INPUT_READ,
    SYS_LOG_READ, SYS_MEMORY_INFO, SYS_MEMORY_MAP, SYS_MEMORY_MAP_AT, SYS_MEMORY_PROTECT,
    SYS_MEMORY_UNMAP, SYS_PROCESS_EXIT, SYS_PROCESS_INFO, SYS_RANDOM_GET, SYS_THREAD_CREATE,
    SYS_THREAD_EXIT, SYS_THREAD_JOIN, SYS_THREAD_SELF, SYS_THREAD_SLEEP, SYS_TIME_READ,
    SYS_TIME_REALTIME,
};

#[cfg(not(test))]
use nagi_abi::{
    DisplayInfo, InputEvent, MemoryInfo, ProcessInfo, MAX_AUDIO_BUFFER, MAX_NET_FRAME_SIZE,
    SYS_NET_RECEIVE, SYS_NET_SEND,
};

#[cfg(not(test))]
const IA32_EFER: u32 = 0xC000_0080;
#[cfg(not(test))]
const IA32_STAR: u32 = 0xC000_0081;
#[cfg(not(test))]
const IA32_LSTAR: u32 = 0xC000_0082;
#[cfg(not(test))]
const IA32_FMASK: u32 = 0xC000_0084;
const EFER_SCE: u64 = 1;
const KERNEL_CODE_SELECTOR: u64 = 0x08;
const SYSRET_SELECTOR_BASE: u64 = 0x13;
#[cfg(not(test))]
const SYSCALL_FMASK: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 18);

#[cfg(not(test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyscallError {
    Unsupported,
}

#[cfg(not(test))]
#[repr(C, align(16))]
struct SyscallStack([u8; 16 * 1024]);

#[cfg(not(test))]
#[no_mangle]
static mut NAGI_SYSCALL_STACK: SyscallStack = SyscallStack([0; 16 * 1024]);

#[cfg(not(test))]
#[no_mangle]
static mut NAGI_SYSCALL_USER_RSP: u64 = 0;

#[cfg(not(test))]
#[repr(C)]
struct SyscallFrame {
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
    user_rip: u64,
    user_rflags: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

#[cfg(not(test))]
#[repr(C, align(16))]
struct UserThreadContext {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    user_rip: u64,
    user_rflags: u64,
    user_rsp: u64,
    fpu: [u8; 512],
    user_fs_base: u64,
}

#[cfg(not(test))]
impl UserThreadContext {
    const fn empty() -> Self {
        let mut fpu = [0; 512];
        fpu[0] = 0x7f;
        fpu[1] = 0x03;
        fpu[24] = 0x80;
        fpu[25] = 0x1f;
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            user_rip: 0,
            user_rflags: 1 << 1,
            user_rsp: 0,
            fpu,
            user_fs_base: 0,
        }
    }
}

#[cfg(not(test))]
const THREAD_EMPTY: u8 = 0;
#[cfg(not(test))]
const THREAD_ACTIVE: u8 = 1;
#[cfg(not(test))]
const THREAD_DONE: u8 = 2;
#[cfg(not(test))]
const MAX_NATIVE_THREAD_STACK: u64 = 4 * 4096;

#[cfg(not(test))]
static NAGI_THREAD_STATE: AtomicU8 = AtomicU8::new(THREAD_EMPTY);
#[cfg(not(test))]
static NAGI_CURRENT_THREAD: AtomicU8 = AtomicU8::new(0);
#[cfg(not(test))]
static mut NAGI_THREAD_EXIT_CODE: u64 = 0;
#[cfg(not(test))]
static mut NAGI_THREAD_CONTEXTS: [UserThreadContext; 2] =
    [UserThreadContext::empty(), UserThreadContext::empty()];
#[cfg(not(test))]
#[no_mangle]
static mut NAGI_SYSCALL_NEXT_CONTEXT: u64 = 0;

#[cfg(not(test))]
global_asm!(
    r#"
.global nagi_syscall_entry
nagi_syscall_entry:
    mov qword ptr [rip + NAGI_SYSCALL_USER_RSP], rsp
    lea rsp, [rip + NAGI_SYSCALL_STACK + 16384]
    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx
    push r11
    push rcx
    push r9
    push r8
    push r10
    push rdx
    push rsi
    push rdi
    push rax
    sub rsp, 8
    sub rsp, 512
    fxsave64 [rsp]
    fninit
    fldz
    fldz
    fldz
    fldz
    fldz
    fldz
    fldz
    fldz
    fninit
    sub rsp, 8
    mov dword ptr [rsp], 0x1f80
    ldmxcsr [rsp]
    add rsp, 8
    pxor xmm0, xmm0
    pxor xmm1, xmm1
    pxor xmm2, xmm2
    pxor xmm3, xmm3
    pxor xmm4, xmm4
    pxor xmm5, xmm5
    pxor xmm6, xmm6
    pxor xmm7, xmm7
    pxor xmm8, xmm8
    pxor xmm9, xmm9
    pxor xmm10, xmm10
    pxor xmm11, xmm11
    pxor xmm12, xmm12
    pxor xmm13, xmm13
    pxor xmm14, xmm14
    pxor xmm15, xmm15
    lea rdi, [rsp + 520]
    call {dispatch}
    mov qword ptr [rsp + 520], rax
    mov rdx, qword ptr [rip + NAGI_SYSCALL_NEXT_CONTEXT]
    test rdx, rdx
    jz 2f
    mov qword ptr [rip + NAGI_SYSCALL_NEXT_CONTEXT], 0
    mov rsp, rdx
    mov rax, qword ptr [rsp + {fs_base_offset}]
    mov rdx, rax
    shr rdx, 32
    mov ecx, 0xC0000100
    wrmsr
    fxrstor64 [rsp + 144]
    mov rax, qword ptr [rsp + 0]
    mov rbx, qword ptr [rsp + 8]
    mov rcx, qword ptr [rsp + 16]
    mov rdx, qword ptr [rsp + 24]
    mov rsi, qword ptr [rsp + 32]
    mov rdi, qword ptr [rsp + 40]
    mov rbp, qword ptr [rsp + 48]
    mov r8, qword ptr [rsp + 56]
    mov r9, qword ptr [rsp + 64]
    mov r10, qword ptr [rsp + 72]
    mov r11, qword ptr [rsp + 80]
    mov r12, qword ptr [rsp + 88]
    mov r13, qword ptr [rsp + 96]
    mov r14, qword ptr [rsp + 104]
    mov r15, qword ptr [rsp + 112]
    mov rcx, qword ptr [rsp + 120]
    mov r11, qword ptr [rsp + 128]
    mov rsp, qword ptr [rsp + 136]
    sysretq
2:
    fxrstor64 [rsp]
    add rsp, 512
    mov rax, qword ptr [rsp + 8]
    add rsp, 16
    pop rdi
    pop rsi
    pop rdx
    pop r10
    pop r8
    pop r9
    pop rcx
    pop r11
    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15
    mov rsp, qword ptr [rip + NAGI_SYSCALL_USER_RSP]
    sysretq
"#,
    dispatch = sym dispatch,
    fs_base_offset = const core::mem::offset_of!(UserThreadContext, user_fs_base),
);

#[cfg(not(test))]
extern "C" {
    static nagi_syscall_entry: u8;
}

#[cfg(test)]
pub fn is_valid_user_read(address: u64, length: usize) -> bool {
    if length == 0 || length > MAX_CONSOLE_WRITE || address < USER_IMAGE_BASE {
        return false;
    }
    address
        .checked_add(length as u64)
        .is_some_and(|end| end <= USER_IMAGE_LIMIT)
}

pub fn is_valid_user_console_read(address: u64, length: usize) -> bool {
    if length == 0 || length > MAX_CONSOLE_WRITE {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    (address >= USER_IMAGE_BASE && end <= USER_IMAGE_LIMIT)
        || (address >= USER_STACK_BASE && end <= USER_STACK_LIMIT)
        || (address >= USER_TLS_BASE && end <= USER_TLS_LIMIT)
        || (address >= USER_MMAP_BASE && end <= USER_MMAP_LIMIT)
}

const fn star_value() -> u64 {
    (SYSRET_SELECTOR_BASE << 48) | (KERNEL_CODE_SELECTOR << 32)
}

const fn efer_with_sce(efer: u64) -> u64 {
    efer | EFER_SCE
}

#[cfg(not(test))]
pub fn initialize() -> Result<(), SyscallError> {
    unsafe { interrupts::disable_interrupts() };
    let maximum_extended_leaf = unsafe { core::arch::x86_64::__cpuid(0x8000_0000).eax };
    if maximum_extended_leaf < 0x8000_0001
        || unsafe { core::arch::x86_64::__cpuid(0x8000_0001).edx } & (1 << 11) == 0
    {
        return Err(SyscallError::Unsupported);
    }

    unsafe {
        interrupts::install_syscall_gdt();
        write_msr(IA32_STAR, star_value());
        write_msr(IA32_LSTAR, core::ptr::addr_of!(nagi_syscall_entry) as u64);
        write_msr(IA32_FMASK, SYSCALL_FMASK);
        write_msr(IA32_EFER, efer_with_sce(read_msr(IA32_EFER)));
    }
    Ok(())
}

#[cfg(not(test))]
extern "sysv64" fn dispatch(frame: &SyscallFrame) -> u64 {
    match frame.number {
        SYS_CONSOLE_WRITE => console_write(frame.arg1, frame.arg2),
        SYS_PROCESS_EXIT => process_exit(frame.arg1),
        SYS_BLOCK_READ => block_read(frame.arg1, frame.arg2, frame.arg3),
        SYS_BLOCK_WRITE => block_write(frame.arg1, frame.arg2, frame.arg3),
        SYS_BLOCK_FLUSH => block_flush(frame.arg1),
        SYS_CONSOLE_READ => console_read(frame.arg1, frame.arg2),
        SYS_PROCESS_INFO => process_info(frame.arg1, frame.arg2),
        SYS_MEMORY_INFO => memory_info(frame.arg1, frame.arg2),
        SYS_LOG_READ => log_read(frame.arg1, frame.arg2),
        SYS_DISPLAY_INFO => display_info(frame.arg1, frame.arg2),
        SYS_DISPLAY_PRESENT => display_present(frame.arg1),
        SYS_INPUT_READ => input_read(frame.arg1, frame.arg2, frame.arg3),
        SYS_NET_SEND => net_send(frame.arg1, frame.arg2, frame.arg3),
        SYS_NET_RECEIVE => net_receive(frame.arg1, frame.arg2, frame.arg3),
        SYS_TIME_READ => time_read(),
        SYS_TIME_REALTIME => time_realtime(),
        SYS_THREAD_SLEEP => thread_sleep(frame.arg1),
        SYS_MEMORY_MAP => memory_map(frame.arg1, frame.arg2),
        SYS_MEMORY_MAP_AT => memory_map_at(frame.arg1, frame.arg2, frame.arg3),
        SYS_MEMORY_UNMAP => memory_unmap(frame.arg1, frame.arg2),
        SYS_MEMORY_PROTECT => memory_protect(frame.arg1, frame.arg2, frame.arg3),
        SYS_THREAD_CREATE => thread_create(frame),
        SYS_THREAD_JOIN => thread_join(frame.arg1, frame),
        SYS_THREAD_EXIT => thread_exit(frame.arg1),
        SYS_THREAD_SELF => u64::from(NAGI_CURRENT_THREAD.load(Ordering::Acquire)),
        SYS_AUDIO_PLAY => audio_play(frame.arg1, frame.arg2, frame.arg3, frame.arg4),
        SYS_AUDIO_CAPTURE => audio_capture(frame.arg1, frame.arg2, frame.arg3, frame.arg4),
        SYS_RANDOM_GET => random_get(frame.arg1, frame.arg2),
        _ => u64::MAX,
    }
}

#[cfg(not(test))]
fn console_write(address: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        return u64::MAX;
    };
    if !is_valid_user_console_read(address, length)
        || !nagi_kernel::user_process::is_user_readable_range_mapped(address, length)
    {
        return u64::MAX;
    }

    let mut buffer = [0_u8; MAX_CONSOLE_WRITE];
    for (index, destination) in buffer[..length].iter_mut().enumerate() {
        *destination = unsafe { (address as *const u8).add(index).read_volatile() };
    }
    serial_write(&buffer[..length]);
    length as u64
}

#[cfg(not(test))]
fn audio_play(capability: u64, stream_id: u64, address: u64, length: u64) -> u64 {
    let Ok(stream_id) = u32::try_from(stream_id) else {
        return u64::MAX;
    };
    let Ok(length) = usize::try_from(length) else {
        return u64::MAX;
    };
    if length == 0
        || length > MAX_AUDIO_BUFFER
        || !nagi_kernel::audio::capability_matches(capability)
        || !nagi_kernel::user_process::is_user_readable_range_mapped(address, length)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; MAX_AUDIO_BUFFER];
    for (index, destination) in buffer[..length].iter_mut().enumerate() {
        *destination = unsafe { (address as *const u8).add(index).read_volatile() };
    }
    match nagi_kernel::audio::play(capability, stream_id, &buffer[..length]) {
        Ok(()) => length as u64,
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn audio_capture(capability: u64, stream_id: u64, address: u64, length: u64) -> u64 {
    let Ok(stream_id) = u32::try_from(stream_id) else {
        return u64::MAX;
    };
    let Ok(length) = usize::try_from(length) else {
        return u64::MAX;
    };
    if length == 0
        || length > MAX_AUDIO_BUFFER
        || !nagi_kernel::audio::capability_matches(capability)
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, length)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; MAX_AUDIO_BUFFER];
    let Ok(captured) = nagi_kernel::audio::capture(capability, stream_id, &mut buffer[..length])
    else {
        return u64::MAX;
    };
    for (index, source) in buffer[..captured].iter().enumerate() {
        unsafe { (address as *mut u8).add(index).write_volatile(*source) };
    }
    captured as u64
}

#[cfg(not(test))]
fn console_read(address: u64, length: u64) -> u64 {
    if length != MAX_CONSOLE_READ as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, MAX_CONSOLE_READ)
    {
        return u64::MAX;
    }
    let Some(byte) = serial_read_byte() else {
        return 0;
    };
    unsafe { (address as *mut u8).write_volatile(byte) };
    MAX_CONSOLE_READ as u64
}

#[cfg(not(test))]
fn process_info(address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<ProcessInfo>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<ProcessInfo>(),
        )
    {
        return u64::MAX;
    }
    let image_pages = nagi_kernel::user_process::current_image_pages();
    let snapshot = ProcessInfo {
        pid: 1,
        parent_pid: 0,
        state: 1,
        flags: 0,
        image_pages: image_pages as u32,
        stack_pages: nagi_kernel::user_process::USER_STACK_PAGES as u32,
        name: *b"nagi-init\0\0\0\0\0\0\0",
    };
    copy_kernel_bytes_to_user(address, &snapshot);
    core::mem::size_of::<ProcessInfo>() as u64
}

#[cfg(not(test))]
fn memory_info(address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<MemoryInfo>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<MemoryInfo>(),
        )
    {
        return u64::MAX;
    }
    let snapshot = MemoryInfo {
        image_pages: nagi_kernel::user_process::current_image_pages() as u64,
        stack_pages: nagi_kernel::user_process::USER_STACK_PAGES as u64,
        tls_pages: nagi_kernel::user_process::USER_TLS_PAGE_COUNT as u64,
        image_base: USER_IMAGE_BASE,
        image_limit: USER_IMAGE_LIMIT,
        stack_base: nagi_kernel::user_process::USER_STACK_BASE,
        stack_limit: nagi_kernel::user_process::USER_STACK_LIMIT,
    };
    copy_kernel_bytes_to_user(address, &snapshot);
    core::mem::size_of::<MemoryInfo>() as u64
}

#[cfg(not(test))]
fn log_read(address: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        return u64::MAX;
    };
    if length > MAX_LOG_READ
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, length)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; MAX_LOG_READ];
    let count = serial_log_read(&mut buffer[..length]);
    for (index, byte) in buffer[..count].iter().enumerate() {
        unsafe { (address as *mut u8).add(index).write_volatile(*byte) };
    }
    count as u64
}

#[cfg(not(test))]
fn display_info(address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<DisplayInfo>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<DisplayInfo>(),
        )
    {
        return u64::MAX;
    }
    let Some(info) = nagi_kernel::display::info() else {
        return u64::MAX;
    };
    copy_kernel_bytes_to_user(address, &info);
    core::mem::size_of::<DisplayInfo>() as u64
}

#[cfg(not(test))]
fn display_present(capability: u64) -> u64 {
    if !nagi_kernel::user_process::is_user_readable_range_mapped(
        nagi_kernel::display::USER_SURFACE_BASE,
        nagi_abi::SURFACE_BYTES,
    ) {
        return u64::MAX;
    }
    match nagi_kernel::display::present_surface(capability) {
        Ok(()) => nagi_abi::SURFACE_BYTES as u64,
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn input_read(capability: u64, address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<InputEvent>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<InputEvent>(),
        )
    {
        return u64::MAX;
    }
    let Some(event) = nagi_kernel::input::read_event(capability) else {
        return 0;
    };
    copy_kernel_bytes_to_user(address, &event);
    core::mem::size_of::<InputEvent>() as u64
}

#[cfg(not(test))]
fn net_send(capability: u64, address: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        return u64::MAX;
    };
    if length == 0
        || length > MAX_NET_FRAME_SIZE
        || !nagi_kernel::user_process::is_user_readable_range_mapped(address, length)
        || !nagi_kernel::net::capability_matches(capability)
    {
        return u64::MAX;
    }
    let mut frame = [0_u8; MAX_NET_FRAME_SIZE];
    for (index, byte) in frame[..length].iter_mut().enumerate() {
        *byte = unsafe { (address as *const u8).add(index).read_volatile() };
    }
    match nagi_kernel::net::transmit(&frame[..length]) {
        Ok(count) => count as u64,
        Err(error) => {
            serial_write(b"Nagi M12 net TX FAIL: ");
            serial_write(match error {
                nagi_kernel::net::NetError::NotInitialized => b"not-initialized\r\n",
                nagi_kernel::net::NetError::PciUnavailable => b"pci\r\n",
                nagi_kernel::net::NetError::InvalidBar => b"bar\r\n",
                nagi_kernel::net::NetError::UnsupportedQueue => b"queue\r\n",
                nagi_kernel::net::NetError::AddressOutOfRange => b"address\r\n",
                nagi_kernel::net::NetError::DeviceFailure => b"device\r\n",
                nagi_kernel::net::NetError::RequestTimeout => b"timeout\r\n",
                nagi_kernel::net::NetError::QueueCorrupt => b"corrupt\r\n",
                nagi_kernel::net::NetError::FrameTooLarge => b"frame\r\n",
                nagi_kernel::net::NetError::Busy => b"busy\r\n",
            });
            u64::MAX
        }
    }
}

#[cfg(not(test))]
fn net_receive(capability: u64, address: u64, length: u64) -> u64 {
    if length != MAX_NET_FRAME_SIZE as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, MAX_NET_FRAME_SIZE)
        || !nagi_kernel::net::capability_matches(capability)
    {
        return u64::MAX;
    }
    let mut frame = [0_u8; MAX_NET_FRAME_SIZE];
    let count = match nagi_kernel::net::receive(&mut frame) {
        Ok(Some(count)) => count,
        Ok(None) => return 0,
        Err(_) => {
            serial_write(b"Nagi M12 net RX FAIL\r\n");
            return u64::MAX;
        }
    };
    for (index, byte) in frame[..count].iter().enumerate() {
        unsafe { (address as *mut u8).add(index).write_volatile(*byte) };
    }
    count as u64
}

#[cfg(not(test))]
fn time_read() -> u64 {
    interrupts::timer_ticks()
}

#[cfg(not(test))]
fn time_realtime() -> u64 {
    let ticks = interrupts::timer_ticks();
    let Some(elapsed) = ticks.checked_mul(10_000_000) else {
        return u64::MAX;
    };
    nagi_abi::NAGI_REALTIME_EPOCH_NS.saturating_add(elapsed)
}

#[cfg(not(test))]
fn thread_sleep(duration_ns: u64) -> u64 {
    if duration_ns == 0 {
        return 0;
    }
    let ticks = duration_ns.div_ceil(10_000_000);
    interrupts::wait_for_timer_ticks(ticks);
    0
}

#[cfg(not(test))]
fn memory_map(length: u64, protection: u64) -> u64 {
    let Some(address) = nagi_kernel::user_process::mmap_user(length, protection) else {
        serial_write(b"Nagi M17 trace: SYS_MEMORY_MAP rejected by bootstrap mapper\r\n");
        return u64::MAX;
    };
    address
}

#[cfg(not(test))]
fn memory_map_at(address: u64, length: u64, protection: u64) -> u64 {
    nagi_kernel::user_process::mmap_user_at(address, length, protection).unwrap_or(u64::MAX)
}

#[cfg(not(test))]
fn memory_unmap(address: u64, length: u64) -> u64 {
    if nagi_kernel::user_process::munmap_user(address, length) {
        0
    } else {
        u64::MAX
    }
}

#[cfg(not(test))]
fn memory_protect(address: u64, length: u64, protection: u64) -> u64 {
    if nagi_kernel::user_process::mprotect_user(address, length, protection) {
        0
    } else {
        u64::MAX
    }
}

#[cfg(not(test))]
fn thread_create_rejected(message: &'static [u8]) -> u64 {
    serial_write(message);
    u64::MAX
}

#[cfg(not(test))]
fn thread_create(frame: &SyscallFrame) -> u64 {
    if NAGI_CURRENT_THREAD.load(Ordering::Acquire) != 0 {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: caller is child thread\r\n",
        );
    }
    if NAGI_THREAD_STATE
        .compare_exchange(
            THREAD_EMPTY,
            THREAD_ACTIVE,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: child slot occupied\r\n",
        );
    }
    let Some(stack_end) = frame.arg3.checked_add(frame.arg4) else {
        NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack range overflow\r\n",
        );
    };
    if !nagi_kernel::user_process::is_user_executable_range_mapped(frame.arg1, 1) {
        NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: entry is not executable\r\n",
        );
    }
    if frame.arg3 < USER_MMAP_BASE || stack_end > USER_MMAP_LIMIT {
        NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack outside mmap window\r\n",
        );
    }
    if frame.arg4 == 0
        || frame.arg4 > MAX_NATIVE_THREAD_STACK
        || !frame.arg3.is_multiple_of(4096)
        || !frame.arg4.is_multiple_of(4096)
    {
        NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: invalid stack size or alignment\r\n",
        );
    }
    if !nagi_kernel::user_process::is_user_writable_range_mapped(frame.arg3, frame.arg4 as usize) {
        NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack is not writable\r\n",
        );
    }
    nagi_kernel::user_process::reset_child_tls();
    let context = unsafe { &mut NAGI_THREAD_CONTEXTS[1] };
    *context = UserThreadContext::empty();
    context.rdi = frame.arg2;
    context.user_rip = frame.arg1;
    context.user_rsp = stack_end - 8;
    context.user_fs_base = USER_TLS_CHILD_CONTROL_BASE;
    1
}

#[cfg(not(test))]
fn save_current_thread_context(frame: &SyscallFrame) {
    let context = unsafe { &mut NAGI_THREAD_CONTEXTS[0] };
    context.rax = 0;
    context.user_fs_base = USER_TLS_CONTROL_BASE;
    context.rbx = frame.rbx;
    context.rcx = frame.user_rip;
    context.rdx = frame.arg3;
    context.rsi = frame.arg2;
    context.rdi = frame.arg1;
    context.rbp = frame.rbp;
    context.r8 = frame.arg5;
    context.r9 = frame.arg6;
    context.r10 = frame.arg4;
    context.r11 = frame.user_rflags;
    context.r12 = frame.r12;
    context.r13 = frame.r13;
    context.r14 = frame.r14;
    context.r15 = frame.r15;
    context.user_rip = frame.user_rip;
    context.user_rflags = frame.user_rflags;
    context.user_rsp = unsafe { NAGI_SYSCALL_USER_RSP };
    let source = unsafe { (frame as *const SyscallFrame).cast::<u8>().sub(520) };
    unsafe { core::ptr::copy_nonoverlapping(source, context.fpu.as_mut_ptr(), 512) };
}

#[cfg(not(test))]
fn thread_join(thread: u64, frame: &SyscallFrame) -> u64 {
    if thread != 1 || NAGI_CURRENT_THREAD.load(Ordering::Acquire) != 0 {
        return u64::MAX;
    }
    match NAGI_THREAD_STATE.load(Ordering::Acquire) {
        THREAD_DONE => {
            let code = unsafe { NAGI_THREAD_EXIT_CODE };
            NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
            code
        }
        THREAD_ACTIVE => {
            save_current_thread_context(frame);
            NAGI_CURRENT_THREAD.store(1, Ordering::Release);
            unsafe {
                NAGI_SYSCALL_NEXT_CONTEXT = core::ptr::addr_of_mut!(NAGI_THREAD_CONTEXTS[1]) as u64;
            }
            0
        }
        _ => u64::MAX,
    }
}

#[cfg(not(test))]
fn thread_exit(code: u64) -> u64 {
    if NAGI_CURRENT_THREAD.load(Ordering::Acquire) != 1
        || NAGI_THREAD_STATE.load(Ordering::Acquire) != THREAD_ACTIVE
    {
        return u64::MAX;
    }
    unsafe { NAGI_THREAD_EXIT_CODE = code };
    // The parent receives the exit result in its restored RAX register during
    // this same context switch. There is no second join syscall in the
    // cooperative bootstrap bridge, so the single child slot is reusable as
    // soon as the result has been staged in the parent context.
    NAGI_THREAD_STATE.store(THREAD_EMPTY, Ordering::Release);
    NAGI_CURRENT_THREAD.store(0, Ordering::Release);
    unsafe {
        let parent = &mut NAGI_THREAD_CONTEXTS[0];
        parent.rax = code;
        NAGI_SYSCALL_NEXT_CONTEXT = core::ptr::addr_of_mut!(NAGI_THREAD_CONTEXTS[0]) as u64;
    }
    0
}

#[cfg(not(test))]
fn copy_kernel_bytes_to_user<T>(address: u64, value: &T) {
    let source = value as *const T as *const u8;
    let length = core::mem::size_of::<T>();
    for index in 0..length {
        unsafe {
            (address as *mut u8)
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
}

#[cfg(not(test))]
fn block_read(capability: u64, sector: u64, address: u64) -> u64 {
    if !nagi_kernel::virtio::capability_matches(capability)
        || !nagi_kernel::virtio::capacity_sectors().is_some_and(|capacity| sector < capacity)
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, BLOCK_SECTOR_SIZE)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; BLOCK_SECTOR_SIZE];
    if nagi_kernel::virtio::read_sector(sector, &mut buffer).is_err() {
        return u64::MAX;
    }
    for (index, byte) in buffer.iter().enumerate() {
        unsafe {
            (address as *mut u8).add(index).write_volatile(*byte);
        }
    }
    BLOCK_SECTOR_SIZE as u64
}

#[cfg(not(test))]
fn block_write(capability: u64, sector: u64, address: u64) -> u64 {
    if !nagi_kernel::virtio::capability_matches(capability)
        || !nagi_kernel::virtio::capacity_sectors().is_some_and(|capacity| sector < capacity)
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, BLOCK_SECTOR_SIZE)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; BLOCK_SECTOR_SIZE];
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte = unsafe { (address as *const u8).add(index).read_volatile() };
    }
    if nagi_kernel::virtio::write_sector(sector, &buffer).is_err() {
        return u64::MAX;
    }
    BLOCK_SECTOR_SIZE as u64
}

#[cfg(not(test))]
fn block_flush(capability: u64) -> u64 {
    if !nagi_kernel::virtio::capability_matches(capability) {
        return u64::MAX;
    }
    if nagi_kernel::virtio::flush().is_err() {
        return u64::MAX;
    }
    0
}

#[cfg(not(test))]
fn random_get(address: u64, length: u64) -> u64 {
    let Ok(length) = usize::try_from(length) else {
        serial_write(b"Nagi M17 trace: SYS_RANDOM_GET rejected: length overflow\r\n");
        return u64::MAX;
    };
    if length == 0 {
        return 0;
    }
    if length > MAX_RANDOM_BYTES {
        serial_write(b"Nagi M17 trace: SYS_RANDOM_GET rejected: length limit\r\n");
        return u64::MAX;
    }
    if !nagi_kernel::user_process::is_user_writable_range_mapped(address, length) {
        serial_write(b"Nagi M17 trace: SYS_RANDOM_GET rejected: user buffer range\r\n");
        return u64::MAX;
    }
    let mut buffer = [0_u8; MAX_RANDOM_BYTES];
    if let Err(error) = nagi_kernel::random::fill(&mut buffer[..length]) {
        serial_write(random_error_trace(error));
        return u64::MAX;
    }
    for (index, byte) in buffer[..length].iter().enumerate() {
        unsafe { (address as *mut u8).add(index).write_volatile(*byte) };
    }
    length as u64
}

#[cfg(not(test))]
fn random_error_trace(error: nagi_kernel::random::RandomError) -> &'static [u8] {
    use nagi_kernel::random::RandomError;

    match error {
        RandomError::NotInitialized => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: not initialized\r\n"
        }
        RandomError::PciUnavailable => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: PCI device unavailable\r\n"
        }
        RandomError::InvalidBar => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: invalid BAR\r\n"
        }
        RandomError::UnsupportedQueue => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: queue too small\r\n"
        }
        RandomError::AddressOutOfRange => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: DMA address out of range\r\n"
        }
        RandomError::DeviceFailure => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: device rejected request\r\n"
        }
        RandomError::RequestTimeout => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: request timeout\r\n"
        }
        RandomError::QueueCorrupt => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: invalid used descriptor\r\n"
        }
        RandomError::InvalidBuffer => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: invalid buffer length\r\n"
        }
        RandomError::Busy => {
            b"Nagi M17 trace: SYS_RANDOM_GET VirtIO failure: request already active\r\n"
        }
    }
}

#[cfg(not(test))]
fn process_exit(code: u64) -> ! {
    if code == 0 {
        serial_write(b"Nagi M5 syscall PASS\r\n");
        serial_write(b"Nagi M5 acceptance PASS\r\n");
        serial_write(b"Nagi M6 acceptance PASS\r\n");
        serial_write(b"Nagi M7 acceptance PASS\r\n");
    } else if code == 2 {
        serial_write(b"Nagi M7 reboot required PASS\r\n");
    } else {
        serial_write(b"Nagi M5 process exit FAIL\r\n");
    }
    halt_forever()
}

#[cfg(not(test))]
unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nostack, preserves_flags)
    );
    (u64::from(high) << 32) | u64::from(low)
}

#[cfg(not(test))]
unsafe fn write_msr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        efer_with_sce, is_valid_user_console_read, is_valid_user_read, star_value,
        BLOCK_SECTOR_SIZE, MAX_CONSOLE_READ, MAX_CONSOLE_WRITE, MAX_LOG_READ, SYS_AUDIO_CAPTURE,
        SYS_AUDIO_PLAY, SYS_BLOCK_FLUSH, SYS_BLOCK_READ, SYS_BLOCK_WRITE, SYS_CONSOLE_READ,
        SYS_CONSOLE_WRITE, SYS_DISPLAY_INFO, SYS_DISPLAY_PRESENT, SYS_INPUT_READ, SYS_LOG_READ,
        SYS_MEMORY_INFO, SYS_MEMORY_MAP, SYS_MEMORY_MAP_AT, SYS_MEMORY_PROTECT, SYS_MEMORY_UNMAP,
        SYS_PROCESS_EXIT, SYS_PROCESS_INFO, SYS_THREAD_CREATE, SYS_THREAD_EXIT, SYS_THREAD_JOIN,
        SYS_THREAD_SELF, SYS_THREAD_SLEEP, SYS_TIME_READ, SYS_TIME_REALTIME,
    };
    use crate::user_elf::{USER_IMAGE_BASE, USER_IMAGE_LIMIT};
    use crate::user_process::{USER_MMAP_BASE, USER_MMAP_LIMIT};

    // Inspect the actual entry body, not a second model of its save list. This
    // also covers registers the Rust ABI happens to preserve today.
    fn entry_body() -> &'static str {
        include_str!("syscall.rs")
            .split("r#\"")
            .find(|body| {
                body.split("\"#")
                    .next()
                    .unwrap()
                    .contains("call {dispatch}")
            })
            .unwrap()
            .split("\"#")
            .next()
            .unwrap()
    }

    #[test]
    fn entry_restores_every_saved_general_register_except_result() {
        let body = entry_body();
        let (save, restore) = body.split_once("call {dispatch}").unwrap();
        let mut stack = [""; 16];
        let mut saved = 0;
        for line in save.lines().map(str::trim) {
            if let Some(register) = line.strip_prefix("push ") {
                stack[saved] = register;
                saved += 1;
            }
        }
        for register in [
            "rdi", "rsi", "rdx", "r10", "r8", "r9", "rbx", "rbp", "r12", "r13", "r14", "r15",
            "rcx", "r11",
        ] {
            assert!(stack[..saved].contains(&register), "unsaved {register}");
        }
        // RAX's input is intentionally discarded; dispatch supplies the result.
        assert_eq!(stack[saved - 1], "rax");
        saved -= 1;
        for line in restore.lines().map(str::trim) {
            if let Some(register) = line.strip_prefix("pop ") {
                assert_ne!(register, "rax", "must keep dispatch result");
                saved -= 1;
                assert_eq!(register, stack[saved], "unbalanced register restore");
            }
        }
        assert_eq!(saved, 0, "saved user registers were not restored");
    }

    #[test]
    fn entry_preserves_floating_point_state_around_dispatch() {
        let body = entry_body();
        let (save, restore) = body.split_once("call {dispatch}").unwrap();
        assert!(save.contains("fxsave64 [rsp]"));
        assert!(restore.contains("fxrstor64 [rsp]"));

        let kernel_state = save.split("fxsave64 [rsp]").nth(1).unwrap();
        assert!(kernel_state.contains("fninit"));
        assert!(kernel_state.contains("ldmxcsr"));
        for register in 0..16 {
            assert!(
                kernel_state.contains(&std::format!("pxor xmm{register}, xmm{register}")),
                "kernel XMM{register} state is not sanitized"
            );
        }
    }

    #[test]
    fn gdt_transition_is_interrupt_disabled_at_caller_and_installation() {
        let source = include_str!("syscall.rs");
        let initialize = source
            .split("pub fn initialize()")
            .nth(1)
            .unwrap()
            .split("extern \"sysv64\" fn dispatch")
            .next()
            .unwrap();
        let disable = initialize
            .find("interrupts::disable_interrupts()")
            .expect("caller must disable interrupts");
        assert!(
            disable
                < initialize
                    .find("interrupts::install_syscall_gdt()")
                    .unwrap()
        );
        let interrupts = include_str!("interrupts.rs");
        let install = interrupts
            .split("pub unsafe fn install_syscall_gdt()")
            .nth(1)
            .unwrap()
            .split("pub unsafe fn enable_interrupts()")
            .next()
            .unwrap();
        assert!(install.find("\"cli\"").unwrap() < install.find("\"lgdt").unwrap());
        assert!(!install.contains("\"sti\""));
        assert!(!initialize.contains("\"sti\""));
    }

    #[test]
    fn published_bootstrap_syscall_numbers_are_stable() {
        assert_eq!(SYS_CONSOLE_WRITE, 1);
        assert_eq!(SYS_PROCESS_EXIT, 2);
        assert_eq!(SYS_BLOCK_READ, 3);
        assert_eq!(SYS_BLOCK_WRITE, 4);
        assert_eq!(SYS_BLOCK_FLUSH, 28);
        assert_eq!(BLOCK_SECTOR_SIZE, 512);
        assert_eq!(SYS_CONSOLE_READ, 5);
        assert_eq!(SYS_PROCESS_INFO, 6);
        assert_eq!(SYS_MEMORY_INFO, 7);
        assert_eq!(SYS_LOG_READ, 8);
        assert_eq!(SYS_DISPLAY_INFO, 9);
        assert_eq!(SYS_DISPLAY_PRESENT, 10);
        assert_eq!(SYS_AUDIO_PLAY, 24);
        assert_eq!(SYS_AUDIO_CAPTURE, 25);
        assert_eq!(SYS_INPUT_READ, 11);
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
        assert_eq!(MAX_CONSOLE_READ, 1);
        assert_eq!(MAX_LOG_READ, 512);
    }

    #[test]
    fn console_write_policy_accepts_only_bounded_user_ranges() {
        assert!(is_valid_user_read(USER_IMAGE_BASE, 1));
        assert!(is_valid_user_read(USER_IMAGE_BASE, MAX_CONSOLE_WRITE));
        assert!(!is_valid_user_read(USER_IMAGE_BASE, MAX_CONSOLE_WRITE + 1));
        assert!(!is_valid_user_read(0xffff_8000_0000_0000, 1));
    }

    #[test]
    fn console_read_policy_accepts_mapped_stack_and_tls_ranges() {
        assert!(is_valid_user_console_read(
            crate::user_process::USER_STACK_BASE,
            1
        ));
        assert!(is_valid_user_console_read(
            crate::user_process::USER_TLS_BASE,
            1
        ));
        assert!(is_valid_user_console_read(
            crate::user_process::USER_TLS_CONTROL_BASE,
            1
        ));
        assert!(is_valid_user_console_read(
            crate::user_process::USER_TLS_CHILD_CONTROL_BASE,
            1
        ));
        assert!(!is_valid_user_console_read(
            crate::user_process::USER_STACK_LIMIT - 1,
            2
        ));
    }

    #[test]
    fn console_write_policy_allows_bounded_mmap_ranges_for_mapping_validation() {
        assert!(is_valid_user_console_read(USER_MMAP_BASE, 1));
        assert!(is_valid_user_console_read(
            USER_MMAP_BASE,
            MAX_CONSOLE_WRITE
        ));
        assert!(is_valid_user_console_read(USER_MMAP_LIMIT - 1, 1));
        assert!(!is_valid_user_console_read(USER_MMAP_LIMIT - 1, 2));
    }

    #[test]
    fn console_write_policy_rejects_empty_overflowing_and_cross_boundary_ranges() {
        assert!(!is_valid_user_read(USER_IMAGE_BASE, 0));
        assert!(!is_valid_user_read(u64::MAX, 1));
        assert!(is_valid_user_read(USER_IMAGE_LIMIT - 1, 1));
        assert!(!is_valid_user_read(USER_IMAGE_LIMIT - 1, 2));
    }

    #[test]
    fn syscall_msrs_use_the_published_selector_layout() {
        assert_eq!(star_value(), (0x13_u64 << 48) | (0x08_u64 << 32));
        assert_eq!(efer_with_sce(1 << 11), (1 << 11) | 1);
    }
}
