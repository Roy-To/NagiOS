use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use nagi_bootinfo::BootInfo;
use nagi_kernel::acpi::{CpuTopology, MAX_CPUS};
use nagi_kernel::memory;
use nagi_kernel::scheduler::{wake_transition, BLOCKED, DONE, RUNNABLE, RUNNING};

use super::interrupts;
use super::memory::{PageAllocator, PAGE_SIZE};

const TRAMPOLINE_LIMIT: u64 = 0x1_0000;
const STACK_SIZE: usize = 16 * 1024;
const WORKLOAD_STEPS: u32 = 3;
const THREAD_COUNT: usize = 2;
const CONTEXT_WORDS: usize = 18;
const NO_CURRENT_TASK: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmpError {
    InvalidTopology,
    NoLowMemory,
    TrampolineTooLarge,
    AddressAbove32Bit,
    AddressSpaceNotReady,
    ApStartupFailed,
    ApTimeout,
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(align(16))]
#[derive(Clone, Copy)]
struct CpuStack([u8; STACK_SIZE]);

static mut CPU_STACKS: [CpuStack; MAX_CPUS] = [CpuStack([0; STACK_SIZE]); MAX_CPUS];
static mut THREAD_STACKS: [CpuStack; MAX_CPUS * THREAD_COUNT] =
    [CpuStack([0; STACK_SIZE]); MAX_CPUS * THREAD_COUNT];
static CPU_APIC_IDS: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_ONLINE: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_WORKLOAD: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_PREEMPTIONS: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_PREEMPT_REQUEST: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_CONTEXT_SWITCHES: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_WORKLOAD_DONE: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static TASK_PROGRESS: [AtomicU32; MAX_CPUS * THREAD_COUNT] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static TASK_STATE: [AtomicU32; MAX_CPUS * THREAD_COUNT] = [
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
    AtomicU32::new(BLOCKED),
];
static CPU_WAKE_EVENTS: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static TASK_CONTEXTS: [AtomicU64; MAX_CPUS * THREAD_COUNT] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CPU_BOOTSTRAP_CONTEXT: [AtomicU64; MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CPU_CURRENT_CONTEXT: [AtomicU64; MAX_CPUS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CPU_CURRENT_TASK: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(NO_CURRENT_TASK),
    AtomicU32::new(NO_CURRENT_TASK),
    AtomicU32::new(NO_CURRENT_TASK),
    AtomicU32::new(NO_CURRENT_TASK),
];
static CPU_TABLE_READY: AtomicBool = AtomicBool::new(false);

global_asm!(
    r#"
.section .text.ap_trampoline,"ax"
.p2align 4
.global nagi_ap_trampoline_start
.global nagi_ap_trampoline_end
.global nagi_trampoline_cr3
.global nagi_trampoline_entry
.global nagi_trampoline_stack
.global nagi_trampoline_cpu_index
.global nagi_trampoline_gdt_pointer
.global nagi_trampoline_code_selector
.global nagi_trampoline_data_selector

.code16
nagi_ap_trampoline_start:
    .Ltrampoline_start:
    cli
    mov ax, cs
    mov ds, ax
    mov ss, ax
    mov sp, 0x0ff0
    mov di, ax
    .byte 0x66, 0x31, 0xdb
    mov bx, di
    .byte 0x66, 0xc1, 0xe3, 0x04
    xor si, si
    mov eax, ebx
    .byte 0x66, 0x05
    .long TRAMP_LOCAL_GDT
    mov dword ptr [si + TRAMP_LOCAL_GDT_POINTER + 2], eax
    lgdt [si + TRAMP_LOCAL_GDT_POINTER]
    mov eax, ebx
    .byte 0x66, 0x05
    .long TRAMP_PROTECTED_ENTRY
    mov dword ptr [si + TRAMP_PROTECTED_FAR], eax
    .byte 0xb8
    .word 0x0008
    mov word ptr [si + TRAMP_PROTECTED_FAR + 2], ax
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    ljmp [si + TRAMP_PROTECTED_FAR]

.code32
nagi_ap_trampoline_protected:
    .byte 0x66, 0xb8
    .word 0x0010
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov esp, ebx
    .byte 0x81, 0xc4
    .long TRAMP_BOOTSTRAP_STACK_TOP
    lgdt [ebx + TRAMP_GDT_POINTER]
    mov ax, word ptr [ebx + TRAMP_DATA_SELECTOR]
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov eax, cr4
    or eax, 0x20
    mov cr4, eax
    mov eax, dword ptr [ebx + TRAMP_CR3]
    mov cr3, eax
    mov ecx, 0xc0000080
    rdmsr
    or eax, 0x100
    wrmsr
    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax
    mov eax, ebx
    .byte 0x05
    .long TRAMP_LONG_ENTRY
    mov dword ptr [ebx + TRAMP_LONG_FAR], eax
    mov ax, word ptr [ebx + TRAMP_CODE_SELECTOR]
    mov word ptr [ebx + TRAMP_LONG_FAR + 4], ax
    ljmp [ebx + TRAMP_LONG_FAR]

.code64
nagi_ap_trampoline_long:
    mov rsp, qword ptr [rbx + TRAMP_STACK]
    mov edi, dword ptr [rbx + TRAMP_CPU_INDEX]
    mov rax, qword ptr [rbx + TRAMP_ENTRY]
    call rax
3:
    hlt
    jmp 3b

.align 8
nagi_trampoline_local_gdt_pointer:
    .word nagi_trampoline_local_gdt_end - nagi_trampoline_local_gdt - 1
    .long 0
nagi_trampoline_local_gdt:
    .quad 0
    .quad 0x00cf9a000000ffff
    .quad 0x00cf92000000ffff
nagi_trampoline_local_gdt_end:

.align 8
nagi_trampoline_cr3:
    .quad 0
nagi_trampoline_entry:
    .quad 0
nagi_trampoline_stack:
    .quad 0
nagi_trampoline_cpu_index:
    .long 0
nagi_trampoline_gdt_pointer:
    .word 0
    .long 0
nagi_trampoline_code_selector:
    .word 0
nagi_trampoline_data_selector:
    .word 0
nagi_trampoline_protected_far_ptr:
    .word 0
    .word 0
nagi_trampoline_long_far_ptr:
    .long 0
    .word 0

nagi_ap_trampoline_end:
.set TRAMP_CR3, nagi_trampoline_cr3 - .Ltrampoline_start
.set TRAMP_ENTRY, nagi_trampoline_entry - .Ltrampoline_start
.set TRAMP_STACK, nagi_trampoline_stack - .Ltrampoline_start
.set TRAMP_CPU_INDEX, nagi_trampoline_cpu_index - .Ltrampoline_start
.set TRAMP_GDT_POINTER, nagi_trampoline_gdt_pointer - .Ltrampoline_start
.set TRAMP_CODE_SELECTOR, nagi_trampoline_code_selector - .Ltrampoline_start
.set TRAMP_DATA_SELECTOR, nagi_trampoline_data_selector - .Ltrampoline_start
.set TRAMP_PROTECTED_FAR, nagi_trampoline_protected_far_ptr - .Ltrampoline_start
.set TRAMP_LONG_FAR, nagi_trampoline_long_far_ptr - .Ltrampoline_start
.set TRAMP_PROTECTED_ENTRY, nagi_ap_trampoline_protected - .Ltrampoline_start
.set TRAMP_LONG_ENTRY, nagi_ap_trampoline_long - .Ltrampoline_start
.set TRAMP_LOCAL_GDT_POINTER, nagi_trampoline_local_gdt_pointer - .Ltrampoline_start
.set TRAMP_LOCAL_GDT, nagi_trampoline_local_gdt - .Ltrampoline_start
.set TRAMP_LOCAL_CODE_SELECTOR, 0x08
.set TRAMP_BOOTSTRAP_STACK_TOP, 0x0ff0
"#
);

extern "C" {
    static nagi_ap_trampoline_start: u8;
    static nagi_ap_trampoline_end: u8;
    static __nagi_kernel_start: u8;
    static __nagi_kernel_end: u8;
    static nagi_trampoline_cr3: u8;
    static nagi_trampoline_entry: u8;
    static nagi_trampoline_stack: u8;
    static nagi_trampoline_cpu_index: u8;
    static nagi_trampoline_gdt_pointer: u8;
    static nagi_trampoline_code_selector: u8;
    static nagi_trampoline_data_selector: u8;
}

pub fn initialize(
    topology: &CpuTopology,
    allocator: &mut PageAllocator,
    boot_info: &BootInfo,
) -> Result<(), SmpError> {
    if topology.count() != MAX_CPUS || topology.bsp_index() >= MAX_CPUS {
        return Err(SmpError::InvalidTopology);
    }
    let mut apic_ids = [0u32; MAX_CPUS];
    for index in 0..MAX_CPUS {
        let cpu = topology.cpu(index).ok_or(SmpError::InvalidTopology)?;
        if cpu.apic_id > u32::from(u8::MAX) {
            return Err(SmpError::InvalidTopology);
        }
        if apic_ids[..index].contains(&cpu.apic_id) {
            return Err(SmpError::InvalidTopology);
        }
        apic_ids[index] = cpu.apic_id;
        CPU_APIC_IDS[index].store(cpu.apic_id, Ordering::Release);
        CPU_ONLINE[index].store(0, Ordering::Release);
        CPU_WORKLOAD[index].store(0, Ordering::Release);
        CPU_PREEMPTIONS[index].store(0, Ordering::Release);
        CPU_PREEMPT_REQUEST[index].store(0, Ordering::Release);
        CPU_CONTEXT_SWITCHES[index].store(0, Ordering::Release);
        CPU_WORKLOAD_DONE[index].store(0, Ordering::Release);
        CPU_WAKE_EVENTS[index].store(0, Ordering::Release);
        CPU_BOOTSTRAP_CONTEXT[index].store(0, Ordering::Release);
        CPU_CURRENT_CONTEXT[index].store(0, Ordering::Release);
        CPU_CURRENT_TASK[index].store(NO_CURRENT_TASK, Ordering::Release);
        for task in 0..THREAD_COUNT {
            let task_index = index * THREAD_COUNT + task;
            TASK_PROGRESS[task_index].store(0, Ordering::Release);
            TASK_STATE[task_index].store(BLOCKED, Ordering::Release);
        }
    }
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    unsafe { initialize_thread_contexts(interrupts::current_code_selector()) };
    for index in 0..MAX_CPUS {
        TASK_STATE[index * THREAD_COUNT].store(RUNNABLE, Ordering::Release);
    }
    CPU_TABLE_READY.store(true, Ordering::Release);
    super::serial_write(b"Nagi M3 SMP state ready\r\n");

    let trampoline_page = allocator
        .allocate_page_below(TRAMPOLINE_LIMIT)
        .ok_or(SmpError::NoLowMemory)?;
    super::serial_write(b"Nagi M3 trampoline page ready\r\n");
    if trampoline_page < PAGE_SIZE || trampoline_page > 0xFF000 {
        return Err(SmpError::NoLowMemory);
    }
    let (gdt_base, gdt_limit, code_selector, data_selector) = current_gdt()?;
    let cr3 = current_cr3();
    if cr3 > u64::from(u32::MAX) || (ap_entry as usize) > u32::MAX as usize {
        return Err(SmpError::AddressAbove32Bit);
    }
    if !address_space_ready(
        cr3,
        gdt_base,
        gdt_limit,
        trampoline_page,
        topology.local_apic_address(),
        boot_info,
    ) {
        return Err(SmpError::AddressSpaceNotReady);
    }
    CPU_ONLINE[topology.bsp_index()].store(1, Ordering::Release);

    for index in 0..MAX_CPUS {
        if index == topology.bsp_index() {
            continue;
        }
        let stack = unsafe { CPU_STACKS[index].0.as_ptr() as u64 + STACK_SIZE as u64 };
        if stack > u64::from(u32::MAX) {
            return Err(SmpError::AddressAbove32Bit);
        }
        prepare_trampoline(
            trampoline_page,
            cr3,
            ap_entry as usize as u64,
            stack,
            index as u32,
            gdt_base,
            gdt_limit,
            code_selector,
            data_selector,
        )?;
        super::serial_write(b"Nagi M3 SIPI dispatch\r\n");
        if !unsafe {
            interrupts::send_init_sipi(apic_ids[index], (trampoline_page / PAGE_SIZE) as u8)
        } {
            return Err(SmpError::ApStartupFailed);
        }
        if !wait_for_online(index) {
            super::serial_write(b"Nagi M3 AP online timeout\r\n");
            return Err(SmpError::ApTimeout);
        }
        super::serial_write(b"Nagi M3 AP online\r\n");
    }

    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
    super::serial_write(b"Nagi M3 scheduler workload START\r\n");
    run_scheduler_workload(topology.bsp_index());
    if !wait_for_workloads() {
        super::serial_write(b"Nagi M3 scheduler workload timeout\r\n");
        return Err(SmpError::ApTimeout);
    }
    super::serial_write(b"Nagi M3 scheduler workload DONE\r\n");
    if !(0..MAX_CPUS).all(|index| {
        CPU_ONLINE[index].load(Ordering::Acquire) == 1
            && CPU_WORKLOAD[index].load(Ordering::Acquire) >= WORKLOAD_STEPS
            && CPU_PREEMPTIONS[index].load(Ordering::Acquire) != 0
            && CPU_CONTEXT_SWITCHES[index].load(Ordering::Acquire) != 0
    }) {
        return Err(SmpError::ApTimeout);
    }
    // Ring-3 M17 threads run on the BSP with IF clear. Keep the shared guest
    // clock advancing from one interrupt-enabled AP instead of summing timer
    // events from every online CPU.
    if let Some(timekeeper) = (0..MAX_CPUS).find(|&index| {
        index != topology.bsp_index() && CPU_ONLINE[index].load(Ordering::Acquire) == 1
    }) {
        interrupts::set_timer_timekeeper(apic_ids[timekeeper]);
    }
    Ok(())
}

fn address_space_ready(
    cr3: u64,
    gdt_base: u64,
    gdt_limit: u16,
    trampoline_page: u64,
    apic_base: u64,
    boot_info: &BootInfo,
) -> bool {
    let page = PAGE_SIZE;
    let cpu_stacks = ptr::addr_of!(CPU_STACKS) as u64;
    let thread_stacks = ptr::addr_of!(THREAD_STACKS) as u64;
    let task_contexts = ptr::addr_of!(TASK_CONTEXTS) as u64;
    let cpu_state = ptr::addr_of!(CPU_ONLINE) as u64;
    let kernel_start = (ptr::addr_of!(__nagi_kernel_start) as u64) & !(page - 1);
    let kernel_end = (ptr::addr_of!(__nagi_kernel_end) as u64 + page - 1) & !(page - 1);
    let ap_entry_page = (ap_entry as usize as u64) & !(page - 1);
    unsafe {
        memory::identity_mapped(cr3, trampoline_page, page)
            && memory::identity_mapped(cr3, gdt_base, u64::from(gdt_limit) + 1)
            && memory::identity_mapped(cr3, interrupts::idt_base(), page)
            && memory::identity_mapped(cr3, apic_base, page)
            && kernel_end > kernel_start
            && memory::identity_mapped(cr3, kernel_start, kernel_end - kernel_start)
            && memory::identity_mapped(cr3, ap_entry_page, page)
            && memory::identity_mapped(
                cr3,
                cpu_stacks,
                core::mem::size_of::<[CpuStack; MAX_CPUS]>() as u64,
            )
            && memory::identity_mapped(
                cr3,
                thread_stacks,
                core::mem::size_of::<[CpuStack; MAX_CPUS * THREAD_COUNT]>() as u64,
            )
            && memory::identity_mapped(
                cr3,
                task_contexts,
                core::mem::size_of_val(&TASK_CONTEXTS) as u64,
            )
            && memory::identity_mapped(cr3, cpu_state, core::mem::size_of_val(&CPU_ONLINE) as u64)
            && memory::identity_mapped(cr3, boot_info.acpi_rsdp & !(page - 1), page)
    }
}

fn prepare_trampoline(
    trampoline_page: u64,
    cr3: u64,
    entry: u64,
    stack: u64,
    cpu_index: u32,
    gdt_base: u64,
    gdt_limit: u16,
    code_selector: u16,
    data_selector: u16,
) -> Result<(), SmpError> {
    if gdt_base > u64::from(u32::MAX)
        || entry > u64::from(u32::MAX)
        || stack > u64::from(u32::MAX)
        || trampoline_page > u64::from(u32::MAX)
    {
        return Err(SmpError::AddressAbove32Bit);
    }
    let start = ptr::addr_of!(nagi_ap_trampoline_start);
    let end = ptr::addr_of!(nagi_ap_trampoline_end);
    let length = (end as usize)
        .checked_sub(start as usize)
        .ok_or(SmpError::TrampolineTooLarge)?;
    if length > PAGE_SIZE as usize {
        return Err(SmpError::TrampolineTooLarge);
    }
    let destination = trampoline_page as *mut u8;
    unsafe {
        ptr::copy_nonoverlapping(start, destination, length);
        write_u64(destination, start, ptr::addr_of!(nagi_trampoline_cr3), cr3);
        write_u64(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_entry),
            entry,
        );
        write_u64(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_stack),
            stack,
        );
        write_u32(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_cpu_index),
            cpu_index,
        );
        write_u16(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_gdt_pointer),
            gdt_limit,
        );
        write_u32(
            destination,
            start,
            (ptr::addr_of!(nagi_trampoline_gdt_pointer) as *const u8).add(2),
            gdt_base as u32,
        );
        write_u16(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_code_selector),
            code_selector,
        );
        write_u16(
            destination,
            start,
            ptr::addr_of!(nagi_trampoline_data_selector),
            data_selector,
        );
    }
    Ok(())
}

unsafe fn write_u16(destination: *mut u8, start: *const u8, symbol: *const u8, value: u16) {
    ptr::write_unaligned(
        destination
            .add(symbol as usize - start as usize)
            .cast::<u16>(),
        value,
    );
}

unsafe fn write_u32(destination: *mut u8, start: *const u8, symbol: *const u8, value: u32) {
    ptr::write_unaligned(
        destination
            .add(symbol as usize - start as usize)
            .cast::<u32>(),
        value,
    );
}

unsafe fn write_u64(destination: *mut u8, start: *const u8, symbol: *const u8, value: u64) {
    ptr::write_unaligned(
        destination
            .add(symbol as usize - start as usize)
            .cast::<u64>(),
        value,
    );
}

fn current_gdt() -> Result<(u64, u16, u16, u16), SmpError> {
    let mut gdtr = DescriptorTablePointer { limit: 0, base: 0 };
    let code_selector: u16;
    let data_selector: u16;
    unsafe {
        asm!("sgdt [{}]", in(reg) &mut gdtr, options(nostack, preserves_flags));
        asm!("mov {0:x}, cs", out(reg) code_selector, options(nomem, nostack, preserves_flags));
        asm!("mov {0:x}, ds", out(reg) data_selector, options(nomem, nostack, preserves_flags));
    }
    if gdtr.base > u64::from(u32::MAX) {
        return Err(SmpError::AddressAbove32Bit);
    }
    Ok((gdtr.base, gdtr.limit, code_selector, data_selector))
}

fn current_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value & !0xfff
}

fn wait_for_online(index: usize) -> bool {
    for _ in 0..10_000_000 {
        if CPU_ONLINE[index].load(Ordering::Acquire) == 1 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

extern "C" fn ap_entry(index: u32) -> ! {
    if index as usize >= MAX_CPUS {
        halt_ap();
    }
    unsafe { interrupts::initialize_ap() };
    CPU_ONLINE[index as usize].store(1, Ordering::Release);
    unsafe { interrupts::enable_interrupts() };
    run_scheduler_workload(index as usize);
    halt_ap()
}

fn run_scheduler_workload(index: usize) {
    while CPU_WORKLOAD_DONE[index].load(Ordering::Acquire) == 0 {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

fn wait_for_workloads() -> bool {
    for _ in 0..10_000_000 {
        if (0..MAX_CPUS).all(|index| CPU_WORKLOAD_DONE[index].load(Ordering::Acquire) != 0) {
            return true;
        }
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
    false
}

pub fn timer_preempt(frame: *mut u64, apic_id: u32) -> *mut u64 {
    if !CPU_TABLE_READY.load(Ordering::Acquire) {
        return frame;
    }
    let Some(index) =
        (0..MAX_CPUS).find(|index| CPU_APIC_IDS[*index].load(Ordering::Acquire) == apic_id)
    else {
        return frame;
    };
    // The M3 scheduler workload is a bounded kernel self-test. Once it has
    // completed, timer interrupts must return the active user frame rather
    // than trying to switch back into the retired test tasks. User-space
    // sleep still observes the timer tick increment performed by the APIC
    // handler.
    if CPU_WORKLOAD_DONE[index].load(Ordering::Acquire) != 0 {
        return frame;
    }
    CPU_PREEMPTIONS[index].fetch_add(1, Ordering::AcqRel);
    CPU_PREEMPT_REQUEST[index].store(1, Ordering::Release);
    if CPU_WORKLOAD_DONE[index].load(Ordering::Acquire) != 0 {
        return frame;
    }
    let _ = CPU_PREEMPT_REQUEST[index].swap(0, Ordering::AcqRel);
    let task_zero_index = index * THREAD_COUNT;
    let task_one_index = task_zero_index + 1;
    let current_task = CPU_CURRENT_TASK[index].load(Ordering::Acquire);
    let (next_task, next) = if current_task == NO_CURRENT_TASK {
        TASK_STATE[index * THREAD_COUNT].store(RUNNING, Ordering::Release);
        CPU_BOOTSTRAP_CONTEXT[index].store(frame as u64, Ordering::Release);
        (0, TASK_CONTEXTS[task_zero_index].load(Ordering::Acquire))
    } else if current_task == 0 {
        TASK_CONTEXTS[task_zero_index].store(frame as u64, Ordering::Release);
        TASK_STATE[task_zero_index].store(RUNNABLE, Ordering::Release);
        wake_task(task_one_index);
        TASK_STATE[task_one_index].store(RUNNING, Ordering::Release);
        (1, TASK_CONTEXTS[task_one_index].load(Ordering::Acquire))
    } else if current_task == 1 {
        TASK_CONTEXTS[task_one_index].store(frame as u64, Ordering::Release);
        let task_zero_done =
            TASK_PROGRESS[task_zero_index].load(Ordering::Acquire) >= WORKLOAD_STEPS;
        let task_one_done = TASK_PROGRESS[task_one_index].load(Ordering::Acquire) >= WORKLOAD_STEPS;
        if task_zero_done && task_one_done {
            TASK_STATE[task_zero_index].store(DONE, Ordering::Release);
            TASK_STATE[task_one_index].store(DONE, Ordering::Release);
            CPU_WORKLOAD_DONE[index].store(1, Ordering::Release);
            (
                NO_CURRENT_TASK,
                CPU_BOOTSTRAP_CONTEXT[index].load(Ordering::Acquire),
            )
        } else {
            TASK_STATE[task_one_index].store(RUNNABLE, Ordering::Release);
            TASK_STATE[task_zero_index].store(RUNNING, Ordering::Release);
            (0, TASK_CONTEXTS[task_zero_index].load(Ordering::Acquire))
        }
    } else {
        return frame;
    };
    CPU_CURRENT_TASK[index].store(next_task, Ordering::Release);
    CPU_CURRENT_CONTEXT[index].store(next, Ordering::Release);
    CPU_CONTEXT_SWITCHES[index].fetch_add(1, Ordering::AcqRel);
    next as *mut u64
}

pub fn cpu_pass(index: usize) -> bool {
    index < MAX_CPUS
        && CPU_ONLINE[index].load(Ordering::Acquire) == 1
        && CPU_WORKLOAD[index].load(Ordering::Acquire) >= WORKLOAD_STEPS
        && CPU_PREEMPTIONS[index].load(Ordering::Acquire) != 0
        && CPU_CONTEXT_SWITCHES[index].load(Ordering::Acquire) != 0
        && CPU_WAKE_EVENTS[index].load(Ordering::Acquire) != 0
        && TASK_STATE[index * THREAD_COUNT].load(Ordering::Acquire) == DONE
        && TASK_STATE[index * THREAD_COUNT + 1].load(Ordering::Acquire) == DONE
}

fn wake_task(task_index: usize) -> bool {
    let Some(target_state) = wake_transition(BLOCKED) else {
        return false;
    };
    let was_blocked = TASK_STATE[task_index]
        .compare_exchange(BLOCKED, target_state, Ordering::AcqRel, Ordering::Acquire)
        .is_ok();
    if was_blocked {
        CPU_WAKE_EVENTS[task_index / THREAD_COUNT].fetch_add(1, Ordering::AcqRel);
    }
    was_blocked
}

unsafe fn initialize_thread_contexts(code_selector: u16) {
    for cpu in 0..MAX_CPUS {
        for task in 0..THREAD_COUNT {
            let stack = &THREAD_STACKS[cpu * THREAD_COUNT + task].0;
            let stack_top = stack.as_ptr() as usize + STACK_SIZE - 8;
            let frame = (stack_top - CONTEXT_WORDS * core::mem::size_of::<u64>()) as *mut u64;
            for word in 0..CONTEXT_WORDS {
                frame.add(word).write_volatile(0);
            }
            frame.add(9).write_volatile(cpu as u64);
            frame.add(10).write_volatile(task as u64);
            frame.add(15).write_volatile(thread_entry as usize as u64);
            frame.add(16).write_volatile(u64::from(code_selector));
            frame.add(17).write_volatile(0x202);
            TASK_CONTEXTS[cpu * THREAD_COUNT + task].store(frame as u64, Ordering::Release);
        }
    }
}

extern "C" fn thread_entry(cpu: u32, task: u32) -> ! {
    let cpu = cpu as usize;
    let task = task as usize;
    if cpu >= MAX_CPUS || task >= THREAD_COUNT {
        halt_ap();
    }
    let stack_top =
        unsafe { THREAD_STACKS[cpu * THREAD_COUNT + task].0.as_ptr() as usize + STACK_SIZE - 8 };
    unsafe {
        asm!("mov rsp, {}", in(reg) stack_top, options(nostack, preserves_flags));
    }
    loop {
        TASK_PROGRESS[cpu * THREAD_COUNT + task].fetch_add(1, Ordering::AcqRel);
        CPU_WORKLOAD[cpu].fetch_add(1, Ordering::AcqRel);
        unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
    }
}

fn halt_ap() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

#[cfg(test)]
mod tests {
    use super::{timer_preempt, CPU_APIC_IDS, CPU_PREEMPTIONS, CPU_TABLE_READY};
    use core::sync::atomic::Ordering;

    #[test]
    fn timer_tick_marks_the_matching_cpu_for_preemption() {
        CPU_APIC_IDS[0].store(7, Ordering::Release);
        CPU_PREEMPTIONS[0].store(0, Ordering::Release);
        CPU_TABLE_READY.store(true, Ordering::Release);
        assert_eq!(
            timer_preempt(core::ptr::null_mut(), 7),
            core::ptr::null_mut()
        );
        assert_eq!(CPU_PREEMPTIONS[0].load(Ordering::Acquire), 1);
        CPU_TABLE_READY.store(false, Ordering::Release);
    }
}
