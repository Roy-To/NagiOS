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
    USER_TLS_LIMIT,
};

#[cfg(not(test))]
use core::arch::{asm, global_asm};
#[cfg(not(test))]
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(not(test))]
use super::{
    halt_forever, interrupts, serial_log_read, serial_read_byte, serial_write, serial_write_decimal,
};

#[cfg(not(test))]
use nagi_abi::{is_valid_bootstrap_user_thread_stack_size, BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE};
#[cfg(not(test))]
use nagi_kernel::scheduler::{BootstrapUserThreads, JoinOutcome};

pub use nagi_abi::{
    BLOCK_SECTOR_SIZE, MAX_CONSOLE_READ, MAX_CONSOLE_WRITE, MAX_LOG_READ, MAX_RANDOM_BYTES,
    SYS_AUDIO_CAPTURE, SYS_AUDIO_PLAY, SYS_BLOCK_FLUSH, SYS_BLOCK_READ, SYS_BLOCK_WRITE,
    SYS_BOOT_READY, SYS_CHANNEL_WAIT_READABLE, SYS_CONSOLE_READ, SYS_CONSOLE_WRITE,
    SYS_DISPLAY_INFO, SYS_DISPLAY_PRESENT, SYS_INPUT_READ, SYS_LOG_READ, SYS_MEMORY_INFO,
    SYS_MEMORY_MAP, SYS_MEMORY_MAP_AT, SYS_MEMORY_PROTECT, SYS_MEMORY_UNMAP, SYS_PROCESS_EXIT,
    SYS_PROCESS_INFO, SYS_RANDOM_GET, SYS_THREAD_CREATE, SYS_THREAD_DETACH, SYS_THREAD_EXIT,
    SYS_THREAD_JOIN, SYS_THREAD_SELF, SYS_THREAD_SLEEP, SYS_TIME_READ, SYS_TIME_REALTIME,
    THREAD_CREATE_DETACHED,
};

#[cfg(not(test))]
use nagi_abi::{
    ChannelEndpoints, ChannelReceiveResult, ChannelSendRequest, DisplayInfo, InputEvent,
    MemoryInfo, ProcessInfo, MAX_AUDIO_BUFFER, MAX_NET_FRAME_SIZE, SYS_CHANNEL_CREATE,
    SYS_CHANNEL_SEND, SYS_CHANNEL_TRY_RECEIVE, SYS_HANDLE_CLOSE, SYS_NET_RECEIVE, SYS_NET_SEND,
};
#[cfg(not(test))]
use nagi_abi::{
    ProcessExitStatus, ProcessSpawnRequest, PROCESS_EXIT_KIND_EXITED, PROCESS_EXIT_KIND_FAULTED,
    PROCESS_WAIT_RETRY, SYS_PROCESS_SPAWN, SYS_PROCESS_WAIT, SYS_UPDATE_SLOT_CLAIM,
    SYS_UPDATE_SLOT_STAGE,
};
#[cfg(not(test))]
use nagi_kernel::process_exit::{ExitKind, ExitTable, WaitOutcome};
#[cfg(not(test))]
use nagi_kernel::user_process::child::{self as child_process, INIT_PROCESS_ID};

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
static REALTIME_EPOCH_NS: AtomicU64 = AtomicU64::new(nagi_bootinfo::REALTIME_UNAVAILABLE_NS);

#[cfg(all(not(test), feature = "m18-browser-threads"))]
static M18_SCHEDULER_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(not(test), feature = "m18-browser-threads"))]
static M18_SCHEDULER_YIELD_DIAGNOSTIC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(not(test))]
pub fn set_realtime_epoch_ns(epoch_ns: u64) {
    REALTIME_EPOCH_NS.store(epoch_ns, Ordering::Release);
}

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
static mut NAGI_USER_THREADS: BootstrapUserThreads = BootstrapUserThreads::new();
#[cfg(not(test))]
static mut NAGI_THREAD_JOIN_OUT: [u64; nagi_abi::BOOTSTRAP_USER_THREAD_COUNT] =
    [0; nagi_abi::BOOTSTRAP_USER_THREAD_COUNT];
#[cfg(not(test))]
static mut NAGI_THREAD_CONTEXTS: [UserThreadContext; nagi_abi::BOOTSTRAP_USER_THREAD_COUNT] =
    [const { UserThreadContext::empty() }; nagi_abi::BOOTSTRAP_USER_THREAD_COUNT];
#[cfg(not(test))]
#[no_mangle]
static mut NAGI_SYSCALL_NEXT_CONTEXT: u64 = 0;
#[cfg(not(test))]
static M17_USER_THREAD_TRACE_EVENTS: AtomicUsize = AtomicUsize::new(0);

#[cfg(not(test))]
fn trace_user_thread_event(event: &[u8], from: u8, to: u8) {
    const MAX_EVENTS: usize = 128;
    if M17_USER_THREAD_TRACE_EVENTS.fetch_add(1, Ordering::Relaxed) >= MAX_EVENTS {
        return;
    }

    fn thread_digit(thread: u8) -> u8 {
        if thread < 10 {
            b'0' + thread
        } else {
            b'a' + (thread - 10)
        }
    }

    let mut line = [0_u8; 96];
    let prefix = b"Nagi M17 trace: thread ";
    let from_label = b" from=";
    let to_label = b" to=";
    let mut length = prefix.len();
    line[..length].copy_from_slice(prefix);
    line[length..length + event.len()].copy_from_slice(event);
    length += event.len();
    line[length..length + from_label.len()].copy_from_slice(from_label);
    length += from_label.len();
    line[length] = thread_digit(from);
    length += 1;
    line[length..length + to_label.len()].copy_from_slice(to_label);
    length += to_label.len();
    line[length] = thread_digit(to);
    length += 1;
    line[length..length + 2].copy_from_slice(b"\r\n");
    length += 2;
    serial_write(&line[..length]);
}

#[cfg(not(test))]
fn append_thread_trace_decimal(line: &mut [u8], length: &mut usize, mut value: u64) {
    let mut digits = [0_u8; 20];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let digit_count = digits.len() - start;
    line[*length..*length + digit_count].copy_from_slice(&digits[start..]);
    *length += digit_count;
}

#[cfg(not(test))]
fn trace_user_thread_sleep_wait(thread: u8, tick: u64, wake_at: u64) {
    const MAX_EVENTS: usize = 128;
    if M17_USER_THREAD_TRACE_EVENTS.fetch_add(1, Ordering::Relaxed) >= MAX_EVENTS {
        return;
    }

    let mut line = [0_u8; 112];
    let prefix = b"Nagi M17 trace: thread sleep-wait id=";
    let tick_label = b" tick=";
    let wake_label = b" wake=";
    let mut length = prefix.len();
    line[..length].copy_from_slice(prefix);
    line[length] = if thread < 10 {
        b'0' + thread
    } else {
        b'a' + (thread - 10)
    };
    length += 1;
    line[length..length + tick_label.len()].copy_from_slice(tick_label);
    length += tick_label.len();
    append_thread_trace_decimal(&mut line, &mut length, tick);
    line[length..length + wake_label.len()].copy_from_slice(wake_label);
    length += wake_label.len();
    append_thread_trace_decimal(&mut line, &mut length, wake_at);
    line[length..length + 2].copy_from_slice(b"\r\n");
    length += 2;
    serial_write(&line[..length]);
}

#[cfg(not(test))]
fn trace_user_thread_sleep_wake(thread: u8, next: u8, tick: u64) {
    const MAX_EVENTS: usize = 128;
    if M17_USER_THREAD_TRACE_EVENTS.fetch_add(1, Ordering::Relaxed) >= MAX_EVENTS {
        return;
    }

    let mut line = [0_u8; 112];
    let prefix = b"Nagi M17 trace: thread sleep-wake id=";
    let next_label = b" next=";
    let tick_label = b" tick=";
    let mut length = prefix.len();
    line[..length].copy_from_slice(prefix);
    line[length] = if thread < 10 {
        b'0' + thread
    } else {
        b'a' + (thread - 10)
    };
    length += 1;
    line[length..length + next_label.len()].copy_from_slice(next_label);
    length += next_label.len();
    line[length] = if next < 10 {
        b'0' + next
    } else {
        b'a' + (next - 10)
    };
    length += 1;
    line[length..length + tick_label.len()].copy_from_slice(tick_label);
    length += tick_label.len();
    append_thread_trace_decimal(&mut line, &mut length, tick);
    line[length..length + 2].copy_from_slice(b"\r\n");
    length += 2;
    serial_write(&line[..length]);
}

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
global_asm!(
    r#"
.global nagi_resume_user_context
nagi_resume_user_context:
    mov rsp, rdi
    mov rax, qword ptr [rsp + {fs_base_offset}]
    mov rdx, rax
    shr rdx, 32
    mov ecx, 0xC0000100
    wrmsr
    fxrstor64 [rsp + 144]
    mov rax, qword ptr [rsp + 0]
    mov rbx, qword ptr [rsp + 8]
    mov rdx, qword ptr [rsp + 24]
    mov rsi, qword ptr [rsp + 32]
    mov rdi, qword ptr [rsp + 40]
    mov rbp, qword ptr [rsp + 48]
    mov r8, qword ptr [rsp + 56]
    mov r9, qword ptr [rsp + 64]
    mov r10, qword ptr [rsp + 72]
    mov r12, qword ptr [rsp + 88]
    mov r13, qword ptr [rsp + 96]
    mov r14, qword ptr [rsp + 104]
    mov r15, qword ptr [rsp + 112]
    mov rcx, qword ptr [rsp + 120]
    mov r11, qword ptr [rsp + 128]
    mov rsp, qword ptr [rsp + 136]
    sysretq
"#,
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
    let caller = thread_table().current_owner();
    let result = if caller == INIT_PROCESS_ID {
        dispatch_init(frame)
    } else {
        dispatch_isolated(caller, frame)
    };
    sync_address_space();
    result
}

/// Syscalls available to a Supervisor-spawned isolated process (ADR 0043).
/// Device, memory-mapping, thread, diagnostics and spawn syscalls are not
/// reachable; the child holds no device capabilities either.
#[cfg(not(test))]
fn dispatch_isolated(caller: u32, frame: &SyscallFrame) -> u64 {
    match frame.number {
        SYS_CONSOLE_WRITE => console_write(frame.arg1, frame.arg2),
        SYS_PROCESS_EXIT => isolated_process_exit(caller, frame.arg1),
        SYS_TIME_READ => time_read(),
        SYS_THREAD_SLEEP => thread_sleep(frame.arg1, frame),
        SYS_RANDOM_GET => random_get(frame.arg1, frame.arg2),
        SYS_CHANNEL_CREATE => channel_create(frame.arg1, frame.arg2),
        SYS_CHANNEL_SEND => channel_send(frame.arg1, frame.arg2, frame.arg3),
        SYS_CHANNEL_TRY_RECEIVE => channel_try_receive(frame.arg1, frame.arg2, frame.arg3),
        SYS_HANDLE_CLOSE => handle_close(frame.arg1),
        SYS_CHANNEL_WAIT_READABLE => channel_wait_readable(frame.arg1, frame),
        _ => u64::MAX,
    }
}

/// Load the address space of the thread selected to run next. Called after
/// every syscall; a CR3 write happens only when the owning process changes.
#[cfg(not(test))]
fn sync_address_space() {
    let owner = thread_table().current_owner();
    if owner == child_process::active_process() {
        return;
    }
    let cr3 = if owner == INIT_PROCESS_ID {
        nagi_kernel::user_process::init_cr3()
    } else {
        match child_process::cr3_of(owner) {
            Some(cr3) => cr3,
            None => {
                serial_write(b"Nagi scheduler selected a thread with no address space\r\n");
                halt_forever()
            }
        }
    };
    unsafe { asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags)) };
    child_process::set_active_process(owner);
}

/// Isolated-process IDs, exit records, and the Supervisor waiter
/// (ADR 0048). Mutated only from syscall and exception paths on the BSP.
#[cfg(not(test))]
static mut EXIT_TABLE: ExitTable = ExitTable::new();

#[cfg(not(test))]
fn exit_table() -> &'static mut ExitTable {
    unsafe { &mut *core::ptr::addr_of_mut!(EXIT_TABLE) }
}

/// `SYS_PROCESS_WAIT` (init only). A blocked caller is resumed with
/// `PROCESS_WAIT_RETRY` when the process exits and then consumes the status.
#[cfg(not(test))]
fn process_wait(process_id: u64, address: u64, length: u64, frame: &SyscallFrame) -> u64 {
    if length != core::mem::size_of::<ProcessExitStatus>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<ProcessExitStatus>(),
        )
    {
        return u64::MAX;
    }
    let Ok(process_id) = u32::try_from(process_id) else {
        return u64::MAX;
    };
    let caller = current_user_thread();
    match exit_table().wait(process_id, caller) {
        WaitOutcome::Ready(record) => {
            let (kind, fault_vector) = match record.kind {
                ExitKind::Exited => (PROCESS_EXIT_KIND_EXITED, 0),
                ExitKind::Faulted { vector } => (PROCESS_EXIT_KIND_FAULTED, u64::from(vector)),
            };
            copy_kernel_bytes_to_user(
                address,
                &ProcessExitStatus {
                    process_id: record.process_id,
                    kind,
                    code: record.code,
                    fault_vector,
                },
            );
            0
        }
        WaitOutcome::Blocked => {
            save_current_thread_context(frame, PROCESS_WAIT_RETRY);
            if !thread_table().block_current_on_channel() {
                let _ = exit_table().cancel_wait(caller);
                return u64::MAX;
            }
            let now = interrupts::timer_ticks();
            if let Some(next) = thread_table()
                .select_runnable(now)
                .or_else(|| wait_until_runnable(false))
            {
                switch_to_thread(caller, next, b"process-wait");
                PROCESS_WAIT_RETRY
            } else {
                // Nothing else can run, so the process can never exit.
                let _ = exit_table().cancel_wait(caller);
                let _ = thread_table().abort_current_channel_wait();
                u64::MAX
            }
        }
        WaitOutcome::Invalid => u64::MAX,
    }
}

#[cfg(not(test))]
fn process_spawn(address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<ProcessSpawnRequest>() as u64
        || !nagi_kernel::user_process::is_user_readable_range_mapped(
            address,
            core::mem::size_of::<ProcessSpawnRequest>(),
        )
    {
        return u64::MAX;
    }
    let request = copy_user_value_from_user::<ProcessSpawnRequest>(address);
    let Ok(image_len) = usize::try_from(request.image_len) else {
        return u64::MAX;
    };
    if request.reserved != 0
        || image_len == 0
        || image_len > child_process::MAX_CHILD_ELF_BYTES
        || !nagi_kernel::user_process::is_user_readable_range_mapped(
            request.image_address,
            image_len,
        )
    {
        serial_write(b"Nagi ADR0043 spawn rejected: invalid request\r\n");
        return u64::MAX;
    }
    let Some(rights) = nagi_kernel::handles::Rights::from_bits(request.endpoint_rights) else {
        return u64::MAX;
    };
    let image =
        unsafe { core::slice::from_raw_parts(request.image_address as *const u8, image_len) };
    let active_pml4 =
        unsafe { &*(nagi_kernel::memory::current_cr3() as *const nagi_kernel::memory::PageTable) };
    let Ok(process_id) = exit_table().reserve() else {
        serial_write(b"Nagi ADR0048 spawn rejected: exit records full or process live\r\n");
        return u64::MAX;
    };
    let context = match child_process::prepare_child(image, active_pml4, process_id) {
        Ok(context) => context,
        Err(_) => {
            serial_write(b"Nagi ADR0043 spawn rejected: child image\r\n");
            return u64::MAX;
        }
    };
    let child_endpoint = match nagi_kernel::user_ipc::register_spawned_process(
        INIT_PROCESS_ID,
        context.process_id,
        request.endpoint,
        rights,
    ) {
        Ok(handle) => handle,
        Err(_) => {
            unsafe { child_process::release_child(context.process_id) };
            serial_write(b"Nagi ADR0043 spawn rejected: endpoint transfer\r\n");
            return u64::MAX;
        }
    };
    let Some(thread) = thread_table().allocate_for_process(context.process_id) else {
        let _ = nagi_kernel::user_ipc::exit_process(context.process_id);
        unsafe { child_process::release_child(context.process_id) };
        serial_write(b"Nagi ADR0043 spawn rejected: thread pool full\r\n");
        return u64::MAX;
    };
    if !exit_table().commit_spawn(context.process_id) {
        let _ = thread_table().exit_process(context.process_id, interrupts::timer_ticks());
        let _ = nagi_kernel::user_ipc::exit_process(context.process_id);
        unsafe { child_process::release_child(context.process_id) };
        return u64::MAX;
    }
    let thread_context = &mut thread_contexts()[thread as usize];
    *thread_context = UserThreadContext::empty();
    thread_context.rdi = child_endpoint;
    thread_context.rsi = u64::from(context.process_id);
    thread_context.user_rip = context.entry;
    thread_context.user_rsp = context.user_stack_top;
    thread_context.user_fs_base = 0;
    serial_write(b"Nagi ADR0043 isolated process spawned pid=");
    serial_write_decimal(context.process_id as usize);
    serial_write(b"\r\n");
    u64::from(context.process_id)
}

/// Terminate an isolated process: close its handles, free its thread, then
/// leave its address space before scrubbing it. Never halts the system.
#[cfg(not(test))]
fn isolated_process_exit(caller: u32, code: u64) -> u64 {
    let current = current_user_thread();
    let next = terminate_isolated_process(caller, code, ExitKind::Exited);
    switch_to_thread(current, next, b"process-exit");
    u64::MAX
}

/// Tear down an isolated process and return the thread to run next. The
/// process's handles are closed, its thread is removed, and init's address
/// space is restored before the child's pages are scrubbed. Shared by
/// `SYS_PROCESS_EXIT` and ring-3 fault containment (ADR 0047).
#[cfg(not(test))]
fn terminate_isolated_process(caller: u32, code: u64, kind: ExitKind) -> u8 {
    serial_write(b"Nagi ADR0043 isolated process exit pid=");
    serial_write_decimal(caller as usize);
    serial_write(b" code=");
    serial_write_decimal(code as usize);
    serial_write(b"\r\n");
    let current = current_user_thread();
    nagi_kernel::user_ipc::cancel_waiter(u32::from(current));
    let _ = nagi_kernel::user_ipc::exit_process(caller);
    // Publish the exit status, then wake a Supervisor thread blocked in
    // SYS_PROCESS_WAIT; it resumes with PROCESS_WAIT_RETRY and consumes it.
    if let Some(waiter) = exit_table().record_exit(caller, code, kind) {
        let _ = thread_table().wake_channel_waiter(waiter);
    }
    let next = thread_table()
        .exit_process(caller, interrupts::timer_ticks())
        .flatten();
    unsafe {
        asm!(
            "mov cr3, {}",
            in(reg) nagi_kernel::user_process::init_cr3(),
            options(nostack, preserves_flags)
        );
    }
    child_process::set_active_process(INIT_PROCESS_ID);
    unsafe { child_process::release_child(caller) };
    next.or_else(|| wait_until_runnable(false))
        .unwrap_or_else(|| halt_forever())
}

#[cfg(not(test))]
fn serial_write_hex(value: u64) {
    let mut digits = [0_u8; 18];
    digits[0] = b'0';
    digits[1] = b'x';
    for (index, digit) in digits[2..].iter_mut().enumerate() {
        *digit = b"0123456789abcdef"[((value >> (60 - index * 4)) & 0xf) as usize];
    }
    serial_write(&digits);
}

/// CPU-exception entry (ADR 0047, ADR 0048). `frame` points at
/// `[vector, error, rip, cs, rflags, rsp, ss]`.
///
/// - **Fault on an AP.** APs run only kernel code. The fault, including a
///   #DF taken on the AP's IST1 stack, is reported and that AP halts.
/// - **Kernel fault (CPL 0).** Still fatal, but reported.
/// - **Fault in init.** Still fatal: init is the Supervisor.
/// - **Fault in an isolated process.** Only that process is terminated.
///   Its exit code is 128 + vector, and the next runnable thread resumes.
#[cfg(not(test))]
pub(crate) extern "sysv64" fn exception_entry(frame: *const u64) -> ! {
    let word = |index: usize| unsafe { frame.add(index).read_volatile() };
    let (vector, error, rip, cs) = (word(0), word(1), word(2), word(3));
    let fault_address: u64;
    unsafe { asm!("mov {}, cr2", out(reg) fault_address, options(nomem, nostack)) };
    if !interrupts::is_bsp() {
        serial_write(b"Nagi AP exception apic=");
        serial_write_decimal(interrupts::local_apic_id() as usize);
        serial_write(b" vector=");
        serial_write_decimal(vector as usize);
        serial_write(b" error=");
        serial_write_hex(error);
        serial_write(b" rip=");
        serial_write_hex(rip);
        serial_write(b" rsp=");
        serial_write_hex(word(5));
        serial_write(b" cr2=");
        serial_write_hex(fault_address);
        serial_write(b"\r\n");
        halt_forever()
    }
    let from_user = cs & 3 == 3;
    let owner = thread_table().current_owner();
    if !from_user || owner == INIT_PROCESS_ID {
        serial_write(if from_user {
            b"Nagi init process fault vector="
        } else {
            b"Nagi kernel exception vector="
        });
        serial_write_decimal(vector as usize);
        serial_write(b" error=");
        serial_write_hex(error);
        serial_write(b" rip=");
        serial_write_hex(rip);
        serial_write(b" cr2=");
        serial_write_hex(fault_address);
        serial_write(b"\r\n");
        halt_forever()
    }
    serial_write(b"Nagi ADR0047 isolated process fault pid=");
    serial_write_decimal(owner as usize);
    serial_write(b" vector=");
    serial_write_decimal(vector as usize);
    serial_write(b" rip=");
    serial_write_hex(rip);
    serial_write(b" cr2=");
    serial_write_hex(fault_address);
    serial_write(b"\r\n");
    let next = terminate_isolated_process(
        owner,
        nagi_kernel::cpu_tables::fault_exit_code(vector),
        ExitKind::Faulted {
            vector: vector as u8,
        },
    );
    trace_user_thread_event(b"fault-exit", next, next);
    sync_address_space();
    unsafe { nagi_resume_user_context(core::ptr::addr_of!(NAGI_THREAD_CONTEXTS[next as usize])) }
}

#[cfg(not(test))]
extern "sysv64" {
    /// Resume a saved user thread context with `sysretq`, exactly as the
    /// syscall return path does after a thread switch.
    fn nagi_resume_user_context(context: *const UserThreadContext) -> !;
}

#[cfg(not(test))]
fn dispatch_init(frame: &SyscallFrame) -> u64 {
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
        SYS_THREAD_SLEEP => thread_sleep(frame.arg1, frame),
        SYS_MEMORY_MAP => memory_map(frame.arg1, frame.arg2),
        SYS_MEMORY_MAP_AT => memory_map_at(frame.arg1, frame.arg2, frame.arg3),
        SYS_MEMORY_UNMAP => memory_unmap(frame.arg1, frame.arg2),
        SYS_MEMORY_PROTECT => memory_protect(frame.arg1, frame.arg2, frame.arg3),
        SYS_THREAD_CREATE => thread_create(frame),
        SYS_THREAD_JOIN => thread_join(frame.arg1, frame.arg2, frame),
        SYS_THREAD_EXIT => thread_exit(frame.arg1),
        SYS_THREAD_SELF => u64::from(current_user_thread()),
        SYS_THREAD_DETACH => thread_detach(frame.arg1),
        SYS_AUDIO_PLAY => audio_play(frame.arg1, frame.arg2, frame.arg3, frame.arg4),
        SYS_AUDIO_CAPTURE => audio_capture(frame.arg1, frame.arg2, frame.arg3, frame.arg4),
        SYS_RANDOM_GET => random_get(frame.arg1, frame.arg2),
        SYS_BOOT_READY => boot_ready(),
        SYS_CHANNEL_CREATE => channel_create(frame.arg1, frame.arg2),
        SYS_CHANNEL_SEND => channel_send(frame.arg1, frame.arg2, frame.arg3),
        SYS_CHANNEL_TRY_RECEIVE => channel_try_receive(frame.arg1, frame.arg2, frame.arg3),
        SYS_HANDLE_CLOSE => handle_close(frame.arg1),
        SYS_CHANNEL_WAIT_READABLE => channel_wait_readable(frame.arg1, frame),
        SYS_PROCESS_SPAWN => process_spawn(frame.arg1, frame.arg2),
        SYS_PROCESS_WAIT => process_wait(frame.arg1, frame.arg2, frame.arg3, frame),
        SYS_UPDATE_SLOT_CLAIM => update_slot_claim(frame.arg1, frame.arg2),
        SYS_UPDATE_SLOT_STAGE => update_slot_stage(frame.arg1),
        _ => u64::MAX,
    }
}

#[cfg(not(test))]
fn current_process() -> u32 {
    thread_table().current_owner()
}

#[cfg(not(test))]
fn channel_create(address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<ChannelEndpoints>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<ChannelEndpoints>(),
        )
    {
        return u64::MAX;
    }
    match nagi_kernel::user_ipc::create_pair(current_process()) {
        Ok(endpoints) => {
            copy_kernel_bytes_to_user(address, &endpoints);
            0
        }
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn channel_send(endpoint: u64, address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<ChannelSendRequest>() as u64
        || !nagi_kernel::user_process::is_user_readable_range_mapped(
            address,
            core::mem::size_of::<ChannelSendRequest>(),
        )
    {
        return u64::MAX;
    }
    let request = copy_user_value_from_user::<ChannelSendRequest>(address);
    match nagi_kernel::user_ipc::send(current_process(), endpoint, request) {
        Ok(woken) => {
            for waiter_id in woken.iter() {
                if let Ok(thread) = u8::try_from(waiter_id) {
                    let _ = thread_table().wake_channel_waiter(thread);
                }
            }
            0
        }
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn channel_wait_readable(endpoint: u64, frame: &SyscallFrame) -> u64 {
    let caller = current_user_thread();
    save_current_thread_context(frame, 0);
    match nagi_kernel::user_ipc::wait_readable(
        current_process(),
        endpoint,
        u32::from(caller),
        || thread_table().block_current_on_channel(),
    ) {
        Ok(nagi_kernel::user_ipc::ChannelWaitOutcome::Readable) => 0,
        Ok(nagi_kernel::user_ipc::ChannelWaitOutcome::Blocked) => {
            let now = interrupts::timer_ticks();
            let next = thread_table()
                .select_runnable(now)
                .or_else(|| wait_until_runnable(false));
            if let Some(next) = next {
                switch_to_thread(caller, next, b"channel-wait");
                0
            } else {
                nagi_kernel::user_ipc::cancel_waiter(u32::from(caller));
                if thread_table().abort_current_channel_wait() {
                    u64::MAX
                } else if let Some(next) = thread_table().select_runnable(now) {
                    switch_to_thread(caller, next, b"channel-wait-wake");
                    0
                } else {
                    halt_forever()
                }
            }
        }
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn channel_try_receive(endpoint: u64, address: u64, length: u64) -> u64 {
    if length != core::mem::size_of::<ChannelReceiveResult>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<ChannelReceiveResult>(),
        )
    {
        return u64::MAX;
    }
    match nagi_kernel::user_ipc::try_receive(current_process(), endpoint) {
        Ok(Some(result)) => {
            copy_kernel_bytes_to_user(address, &result);
            1
        }
        Ok(None) => 0,
        Err(_) => u64::MAX,
    }
}

#[cfg(not(test))]
fn handle_close(handle: u64) -> u64 {
    match nagi_kernel::user_ipc::close(current_process(), handle) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// ADR 0062: hand the inactive-slot update capability to init, once.
#[cfg(not(test))]
fn update_slot_claim(address: u64, size: u64) -> u64 {
    if current_process() != INIT_PROCESS_ID
        || size != core::mem::size_of::<nagi_abi::UpdateSlotInfo>() as u64
        || !nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<nagi_abi::UpdateSlotInfo>(),
        )
    {
        return u64::MAX;
    }
    let Some((capability, sector_count, slot)) = nagi_kernel::virtio::claim_update_slot() else {
        serial_write(b"Nagi update slot claim REFUSED\r\n");
        return u64::MAX;
    };
    let info = nagi_abi::UpdateSlotInfo {
        capability,
        sector_count,
        slot,
        reserved: [0; 7],
    };
    unsafe {
        (address as *mut nagi_abi::UpdateSlotInfo).write_unaligned(info);
    }
    serial_write(b"Nagi update slot claimed slot=");
    serial_write(if slot == 0 { b"A" } else { b"B" });
    serial_write(b"\r\n");
    0
}

/// ADR 0062: ask the loader to trial the inactive slot on the next boot.
#[cfg(not(test))]
fn update_slot_stage(capability: u64) -> u64 {
    use nagi_kernel::boot_control::BootStageOutcome;

    if current_process() != INIT_PROCESS_ID
        || !nagi_kernel::virtio::update_capability_matches(capability)
    {
        return u64::MAX;
    }
    match nagi_kernel::boot_control::stage_update() {
        BootStageOutcome::Staged(record) | BootStageOutcome::AlreadyStaged(record) => {
            serial_write(b"Nagi update stage request persisted slot=");
            serial_write(if record.slot == 0 { b"A" } else { b"B" });
            serial_write(b" generation=");
            serial_write_decimal(usize::try_from(record.journal_generation).unwrap_or(usize::MAX));
            serial_write(b" PASS\r\n");
            0
        }
        BootStageOutcome::Unavailable | BootStageOutcome::Failed => {
            serial_write(b"Nagi update stage request FAIL\r\n");
            u64::MAX
        }
    }
}

#[cfg(not(test))]
fn boot_ready() -> u64 {
    use nagi_kernel::boot_control::BootReadinessOutcome;

    match nagi_kernel::boot_control::report() {
        BootReadinessOutcome::NoTrial => 0,
        BootReadinessOutcome::Persisted(record)
        | BootReadinessOutcome::AlreadyPersisted(record) => {
            serial_write(b"Nagi M27 readiness persisted slot=");
            serial_write(if record.slot == 0 { b"A" } else { b"B" });
            serial_write(b" attempt=");
            serial_write_decimal(usize::from(record.attempt));
            serial_write(b" generation=");
            serial_write_decimal(usize::try_from(record.journal_generation).unwrap_or(usize::MAX));
            serial_write(b" PASS\r\n");
            0
        }
        BootReadinessOutcome::PersistenceFailed(_) | BootReadinessOutcome::InProgress => {
            serial_write(b"Nagi M27 readiness persistence FAIL\r\n");
            u64::MAX
        }
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
    nagi_abi::realtime_ns_at_ticks(
        REALTIME_EPOCH_NS.load(Ordering::Acquire),
        interrupts::timer_ticks(),
    )
}

#[cfg(not(test))]
fn current_user_thread() -> u8 {
    unsafe { (&*core::ptr::addr_of!(NAGI_USER_THREADS)).current() }
}

#[cfg(not(test))]
fn thread_table() -> &'static mut BootstrapUserThreads {
    unsafe { &mut *core::ptr::addr_of_mut!(NAGI_USER_THREADS) }
}

#[cfg(not(test))]
fn thread_contexts() -> &'static mut [UserThreadContext; nagi_abi::BOOTSTRAP_USER_THREAD_COUNT] {
    unsafe { &mut *core::ptr::addr_of_mut!(NAGI_THREAD_CONTEXTS) }
}

#[cfg(not(test))]
fn switch_to_thread(from: u8, thread: u8, event: &[u8]) {
    trace_user_thread_event(event, from, thread);
    unsafe {
        NAGI_SYSCALL_NEXT_CONTEXT =
            core::ptr::addr_of_mut!(NAGI_THREAD_CONTEXTS[thread as usize]) as u64;
    }
}

#[cfg(not(test))]
fn save_current_thread_context(frame: &SyscallFrame, result: u64) {
    let thread = current_user_thread() as usize;
    let context = &mut thread_contexts()[thread];
    context.rax = result;
    context.user_fs_base = if thread_table().current_owner() == INIT_PROCESS_ID {
        nagi_kernel::user_process::user_tls_control_base(thread).unwrap_or(0)
    } else {
        0
    };
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
    context.user_rflags = frame.user_rflags & !(1 << 9);
    context.user_rsp = unsafe { NAGI_SYSCALL_USER_RSP };
    let source = unsafe { (frame as *const SyscallFrame).cast::<u8>().sub(520) };
    unsafe { core::ptr::copy_nonoverlapping(source, context.fpu.as_mut_ptr(), 512) };
}

#[cfg(not(test))]
fn wait_until_runnable(abort_deadlocked_join: bool) -> Option<u8> {
    loop {
        let now = interrupts::timer_ticks();
        if let Some(thread) = thread_table().select_runnable(now) {
            return Some(thread);
        }
        if let Some(ticks) = thread_table().ticks_until_wake(now) {
            interrupts::wait_for_timer_ticks(ticks.max(1));
            continue;
        }
        if abort_deadlocked_join {
            let current = thread_table().abort_blocked_join()?;
            thread_contexts()[current as usize].rax = u64::MAX;
            unsafe { NAGI_THREAD_JOIN_OUT[current as usize] = 0 };
            return Some(current);
        }
        return None;
    }
}

#[cfg(not(test))]
fn write_thread_exit_code(address: u64, exit_code: u64) -> bool {
    if !valid_thread_exit_code_address(address) {
        return false;
    }
    unsafe { (address as *mut u64).write_volatile(exit_code) };
    true
}

#[cfg(not(test))]
fn valid_thread_exit_code_address(address: u64) -> bool {
    address.is_multiple_of(core::mem::align_of::<u64>() as u64)
        && nagi_kernel::user_process::is_user_writable_range_mapped(
            address,
            core::mem::size_of::<u64>(),
        )
}

#[cfg(not(test))]
fn thread_yield(frame: &SyscallFrame) -> u64 {
    let current = current_user_thread();
    save_current_thread_context(frame, 0);
    let tick = interrupts::timer_ticks();
    let Some(next) = thread_table().yield_current(tick) else {
        #[cfg(feature = "m18-browser-threads")]
        trace_m18_scheduler_yield(current, None, tick);
        return u64::MAX;
    };
    #[cfg(feature = "m18-browser-threads")]
    if next != current {
        let sequence = M18_SCHEDULER_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
        if sequence < 64 || sequence.is_multiple_of(1024) {
            serial_write(b"Nagi M18 scheduler handoff #");
            serial_write_decimal(sequence);
            serial_write(b" thread");
            serial_write_decimal(current as usize);
            serial_write(b" -> thread");
            serial_write_decimal(next as usize);
            serial_write(b"\r\n");
        }
    }
    #[cfg(feature = "m18-browser-threads")]
    trace_m18_scheduler_yield(current, Some(next), tick);
    switch_to_thread(current, next, b"yield");
    0
}

#[cfg(all(not(test), feature = "m18-browser-threads"))]
fn trace_m18_scheduler_yield(current: u8, next: Option<u8>, tick: u64) {
    let handoffs = M18_SCHEDULER_TRACE_COUNT.load(Ordering::Relaxed);
    if handoffs < 98_304 || handoffs % 1_024 >= 16 {
        return;
    }

    let sequence = M18_SCHEDULER_YIELD_DIAGNOSTIC_COUNT.fetch_add(1, Ordering::Relaxed);
    if sequence >= 512 {
        return;
    }

    let mut runnable = 0_usize;
    let mut running = 0_usize;
    let mut sleeping = 0_usize;
    for id in 0..nagi_abi::BOOTSTRAP_USER_THREAD_COUNT {
        match thread_table().state(id as u8) {
            Some(nagi_kernel::scheduler::UserThreadState::Runnable) => runnable += 1,
            Some(nagi_kernel::scheduler::UserThreadState::Running) => running += 1,
            Some(nagi_kernel::scheduler::UserThreadState::Sleeping { .. }) => sleeping += 1,
            _ => {}
        }
    }

    serial_write(b"Nagi M18 scheduler yield state #");
    serial_write_decimal(sequence);
    serial_write(b" handoffs=");
    serial_write_decimal(handoffs);
    serial_write(b" from=");
    serial_write_decimal(current as usize);
    serial_write(b" next=");
    if let Some(next) = next {
        serial_write_decimal(next as usize);
    } else {
        serial_write(b"none");
    }
    serial_write(b" tick=");
    serial_write_decimal(tick as usize);
    serial_write(b" running=");
    serial_write_decimal(running);
    serial_write(b" runnable=");
    serial_write_decimal(runnable);
    serial_write(b" sleeping=");
    serial_write_decimal(sleeping);
    serial_write(b"\r\n");
}

#[cfg(not(test))]
fn thread_sleep(duration_ns: u64, frame: &SyscallFrame) -> u64 {
    if duration_ns == 0 {
        return thread_yield(frame);
    }
    let current = current_user_thread();
    save_current_thread_context(frame, 0);
    let now = interrupts::timer_ticks();
    let sleep_ticks = duration_ns.div_ceil(10_000_000).max(1);
    let wake_at = now.saturating_add(sleep_ticks);
    if let Some(next) = thread_table().sleep_current(wake_at, now) {
        switch_to_thread(current, next, b"sleep");
        return 0;
    }
    trace_user_thread_sleep_wait(current, now, wake_at);
    if let Some(next) = wait_until_runnable(false) {
        trace_user_thread_sleep_wake(current, next, interrupts::timer_ticks());
        switch_to_thread(current, next, b"sleep");
        0
    } else {
        halt_forever()
    }
}

#[cfg(not(test))]
fn memory_map(length: u64, protection: u64) -> u64 {
    match nagi_kernel::user_process::mmap_user_with_diagnostics(length, protection) {
        Ok(address) => address,
        Err(failure) => {
            serial_write(b"Nagi M17 trace: SYS_MEMORY_MAP rejected: ");
            serial_write(failure.reason());
            serial_write(b" request_pages=");
            serial_write_decimal(failure.requested_pages);
            serial_write(b" protection=");
            serial_write_decimal(failure.protection as usize);
            serial_write(b" free_reservation_slots=");
            serial_write_decimal(failure.free_reservation_slots);
            serial_write(b" free_pages=");
            serial_write_decimal(failure.free_pages);
            serial_write(b" largest_free_run_pages=");
            serial_write_decimal(failure.largest_free_run_pages);
            serial_write(b"\r\n");
            u64::MAX
        }
    }
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
    if frame.arg5 & !THREAD_CREATE_DETACHED != 0 {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: unknown flags\r\n",
        );
    }
    let Some(stack_end) = frame.arg3.checked_add(frame.arg4) else {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack range overflow\r\n",
        );
    };
    if !nagi_kernel::user_process::is_user_executable_range_mapped(frame.arg1, 1) {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: entry is not executable\r\n",
        );
    }
    if frame.arg3 < USER_MMAP_BASE || stack_end > USER_MMAP_LIMIT {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack outside mmap window\r\n",
        );
    }
    if !is_valid_bootstrap_user_thread_stack_size(frame.arg4 as usize)
        || !frame
            .arg3
            .is_multiple_of(BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE as u64)
    {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: invalid stack size or alignment\r\n",
        );
    }
    if !nagi_kernel::user_process::is_user_writable_range_mapped(frame.arg3, frame.arg4 as usize) {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: stack is not writable\r\n",
        );
    }
    let Some(thread) = thread_table().allocate() else {
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: bootstrap thread pool full\r\n",
        );
    };
    if frame.arg5 & THREAD_CREATE_DETACHED != 0 && !thread_table().detach(thread) {
        let _ = thread_table().discard_unstarted(thread);
        return thread_create_rejected(
            b"Nagi M17 trace: SYS_THREAD_CREATE rejected: detach initialization failed\r\n",
        );
    }
    if !nagi_kernel::user_process::reset_user_thread_tls(thread as usize) {
        let _ = thread_table().discard_unstarted(thread);
        return u64::MAX;
    }
    let context = &mut thread_contexts()[thread as usize];
    *context = UserThreadContext::empty();
    context.rdi = frame.arg2;
    context.user_rip = frame.arg1;
    context.user_rsp = stack_end - 8;
    context.user_fs_base =
        nagi_kernel::user_process::user_tls_control_base(thread as usize).unwrap_or(0);
    trace_user_thread_event(b"create", current_user_thread(), thread);
    u64::from(thread)
}

#[cfg(not(test))]
fn thread_join(thread: u64, result_address: u64, frame: &SyscallFrame) -> u64 {
    if !valid_thread_exit_code_address(result_address) {
        return u64::MAX;
    }
    let Ok(target) = u8::try_from(thread) else {
        return u64::MAX;
    };
    let caller = current_user_thread();
    save_current_thread_context(frame, 0);
    match thread_table().join_current(target, interrupts::timer_ticks()) {
        JoinOutcome::Completed(exit_code) => {
            if write_thread_exit_code(result_address, exit_code) {
                0
            } else {
                u64::MAX
            }
        }
        JoinOutcome::Blocked => {
            unsafe { NAGI_THREAD_JOIN_OUT[caller as usize] = result_address };
            let current = thread_table().current();
            if current != caller {
                switch_to_thread(caller, current, b"join");
                0
            } else if let Some(next) = wait_until_runnable(true) {
                switch_to_thread(caller, next, b"join");
                0
            } else {
                u64::MAX
            }
        }
        JoinOutcome::Invalid => u64::MAX,
    }
}

#[cfg(not(test))]
fn thread_exit(code: u64) -> u64 {
    let current = current_user_thread();
    let now = interrupts::timer_ticks();
    let Some(outcome) = thread_table().exit_current(code, now) else {
        return u64::MAX;
    };
    if let Some((joiner, exit_code)) = outcome.woken_joiner {
        let result_address = unsafe { NAGI_THREAD_JOIN_OUT[joiner as usize] };
        let joiner_context = &mut thread_contexts()[joiner as usize];
        joiner_context.rax = if write_thread_exit_code(result_address, exit_code) {
            0
        } else {
            u64::MAX
        };
        unsafe { NAGI_THREAD_JOIN_OUT[joiner as usize] = 0 };
    }
    let next = outcome
        .next_thread
        .or_else(|| wait_until_runnable(false))
        .unwrap_or_else(|| halt_forever());
    switch_to_thread(current, next, b"exit");
    u64::MAX
}

#[cfg(not(test))]
fn thread_detach(thread: u64) -> u64 {
    let Ok(thread) = u8::try_from(thread) else {
        return u64::MAX;
    };
    if thread_table().detach(thread) {
        0
    } else {
        u64::MAX
    }
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
fn copy_user_value_from_user<T: Copy>(address: u64) -> T {
    let mut value = core::mem::MaybeUninit::<T>::uninit();
    let destination = value.as_mut_ptr().cast::<u8>();
    let length = core::mem::size_of::<T>();
    for index in 0..length {
        unsafe {
            destination
                .add(index)
                .write_volatile((address as *const u8).add(index).read_volatile());
        }
    }
    // The ABI type consists only of integer fields and byte arrays, so every
    // bit pattern is valid after the caller's readable-range preflight.
    unsafe { value.assume_init() }
}

#[cfg(not(test))]
fn block_read(capability: u64, sector: u64, address: u64) -> u64 {
    if !nagi_kernel::virtio::readable_capability_matches(capability)
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, BLOCK_SECTOR_SIZE)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; BLOCK_SECTOR_SIZE];
    if nagi_kernel::virtio::read_sector_for_capability(capability, sector, &mut buffer).is_err() {
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
    // The extent bound is enforced per capability by the block driver.
    if !nagi_kernel::virtio::writable_capability_matches(capability)
        || !nagi_kernel::user_process::is_user_writable_range_mapped(address, BLOCK_SECTOR_SIZE)
    {
        return u64::MAX;
    }
    let mut buffer = [0_u8; BLOCK_SECTOR_SIZE];
    for (index, byte) in buffer.iter_mut().enumerate() {
        *byte = unsafe { (address as *const u8).add(index).read_volatile() };
    }
    if nagi_kernel::virtio::write_sector_for_capability(capability, sector, &buffer).is_err() {
        return u64::MAX;
    }
    BLOCK_SECTOR_SIZE as u64
}

#[cfg(not(test))]
fn block_flush(capability: u64) -> u64 {
    if !nagi_kernel::virtio::writable_capability_matches(capability) {
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
        SYS_AUDIO_PLAY, SYS_BLOCK_FLUSH, SYS_BLOCK_READ, SYS_BLOCK_WRITE, SYS_BOOT_READY,
        SYS_CONSOLE_READ, SYS_CONSOLE_WRITE, SYS_DISPLAY_INFO, SYS_DISPLAY_PRESENT, SYS_INPUT_READ,
        SYS_LOG_READ, SYS_MEMORY_INFO, SYS_MEMORY_MAP, SYS_MEMORY_MAP_AT, SYS_MEMORY_PROTECT,
        SYS_MEMORY_UNMAP, SYS_PROCESS_EXIT, SYS_PROCESS_INFO, SYS_THREAD_CREATE, SYS_THREAD_EXIT,
        SYS_THREAD_JOIN, SYS_THREAD_SELF, SYS_THREAD_SLEEP, SYS_TIME_READ, SYS_TIME_REALTIME,
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
        assert!(install.find("\"cli\"").unwrap() < install.find("load_kernel_gdt(").unwrap());
        assert!(!install.contains("\"sti\""));
        let load = interrupts
            .split("unsafe fn load_kernel_gdt(")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        assert!(load.contains("\"lgdt"));
        assert!(!load.contains("\"sti\""));
        assert!(!initialize.contains("\"sti\""));
    }

    #[test]
    fn published_bootstrap_syscall_numbers_are_stable() {
        assert_eq!(SYS_CONSOLE_WRITE, 1);
        assert_eq!(SYS_PROCESS_EXIT, 2);
        assert_eq!(SYS_BLOCK_READ, 3);
        assert_eq!(SYS_BLOCK_WRITE, 4);
        assert_eq!(SYS_BLOCK_FLUSH, 28);
        assert_eq!(SYS_BOOT_READY, 30);
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
