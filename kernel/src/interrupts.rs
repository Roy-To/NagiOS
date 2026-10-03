use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::{outb, serial_write};

const DEFAULT_APIC_BASE: u64 = 0xFEE0_0000;
const APIC_ID: u64 = 0x020;
const APIC_EOI: u64 = 0x0B0;
const APIC_ICR_LOW: u64 = 0x300;
const APIC_ICR_HIGH: u64 = 0x310;
const APIC_SPURIOUS_INTERRUPT: u64 = 0x0F0;
const APIC_LVT_TIMER: u64 = 0x320;
const APIC_TIMER_INITIAL_COUNT: u64 = 0x380;
const APIC_TIMER_CURRENT_COUNT: u64 = 0x390;
const APIC_TIMER_DIVIDE: u64 = 0x3E0;
const TIMER_VECTOR: u8 = 32;
const TIMER_PERIODIC: u32 = 1 << 17;
const PIC_MASTER_DATA: u16 = 0x21;
const PIC_SLAVE_DATA: u16 = 0xA1;
const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;

#[repr(C, packed)]
struct Gdtr {
    limit: u16,
    base: u64,
}

/// BSP GDT after the M5 switch: null, kernel code/data, user data/code, and
/// the 16-byte TSS descriptor at selector 0x28 (ADR 0047).
static mut SYSCALL_GDT: [u64; 7] = [
    0,
    0x00af_9a00_0000_ffff,
    0x00cf_9200_0000_ffff,
    0x00cf_f200_0000_ffff,
    0x00af_fa00_0000_ffff,
    0,
    0,
];
const TSS_SELECTOR: u16 = 0x28;
const FAULT_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct FaultStack([u8; FAULT_STACK_SIZE]);

/// RSP0: the kernel stack a ring-3 exception switches to on the BSP.
static mut USER_FAULT_STACK: FaultStack = FaultStack([0; FAULT_STACK_SIZE]);
/// IST1: a separate stack for #DF so a kernel stack fault is still reported.
static mut DOUBLE_FAULT_STACK: FaultStack = FaultStack([0; FAULT_STACK_SIZE]);
static mut BSP_TSS: nagi_kernel::cpu_tables::TaskStateSegment =
    nagi_kernel::cpu_tables::TaskStateSegment::new(0, 0);
/// BSP-only IDT installed with the M5 GDT. The shared M3 IDT keeps serving
/// APs through their boot GDT selectors.
static mut BSP_IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
static APIC_BASE: AtomicU64 = AtomicU64::new(DEFAULT_APIC_BASE);
static TIMER_TIMEKEEPER_APIC_ID: AtomicU32 = AtomicU32::new(u32::MAX);
static BSP_APIC_ID: AtomicU32 = AtomicU32::new(u32::MAX);
static BSP_TIMER_LAST_COUNT: AtomicU64 = AtomicU64::new(0);

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        options: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn new(handler: u64, selector: u16) -> Self {
        Self::with_ist(handler, selector, 0)
    }

    fn with_ist(handler: u64, selector: u16, ist: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            options: nagi_kernel::cpu_tables::interrupt_gate_options(ist),
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];

global_asm!(
    r#"
.global nagi_timer_stub
nagi_timer_stub:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    and rsp, -16
    call {timer_interrupt}
    mov rsp, rax
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    iretq

.global nagi_page_fault_stub
nagi_page_fault_stub:
    cli
    mov rdi, qword ptr [rsp]
    mov rsi, rsp
    call {page_fault_interrupt}
    test al, al
    jnz 1f
    cli
    hlt
    jmp nagi_page_fault_stub+8
1:
    add rsp, 8
    iretq

.global nagi_fault_stub_0
nagi_fault_stub_0:
    push 0
    push 0
    jmp nagi_fault_common
.global nagi_fault_stub_1
nagi_fault_stub_1:
    push 0
    push 1
    jmp nagi_fault_common
.global nagi_fault_stub_2
nagi_fault_stub_2:
    push 0
    push 2
    jmp nagi_fault_common
.global nagi_fault_stub_3
nagi_fault_stub_3:
    push 0
    push 3
    jmp nagi_fault_common
.global nagi_fault_stub_4
nagi_fault_stub_4:
    push 0
    push 4
    jmp nagi_fault_common
.global nagi_fault_stub_5
nagi_fault_stub_5:
    push 0
    push 5
    jmp nagi_fault_common
.global nagi_fault_stub_6
nagi_fault_stub_6:
    push 0
    push 6
    jmp nagi_fault_common
.global nagi_fault_stub_7
nagi_fault_stub_7:
    push 0
    push 7
    jmp nagi_fault_common
.global nagi_fault_stub_8
nagi_fault_stub_8:
    push 8
    jmp nagi_fault_common
.global nagi_fault_stub_9
nagi_fault_stub_9:
    push 0
    push 9
    jmp nagi_fault_common
.global nagi_fault_stub_10
nagi_fault_stub_10:
    push 10
    jmp nagi_fault_common
.global nagi_fault_stub_11
nagi_fault_stub_11:
    push 11
    jmp nagi_fault_common
.global nagi_fault_stub_12
nagi_fault_stub_12:
    push 12
    jmp nagi_fault_common
.global nagi_fault_stub_13
nagi_fault_stub_13:
    push 13
    jmp nagi_fault_common
.global nagi_fault_stub_14
nagi_fault_stub_14:
    push 14
    jmp nagi_fault_common
.global nagi_fault_stub_15
nagi_fault_stub_15:
    push 0
    push 15
    jmp nagi_fault_common
.global nagi_fault_stub_16
nagi_fault_stub_16:
    push 0
    push 16
    jmp nagi_fault_common
.global nagi_fault_stub_17
nagi_fault_stub_17:
    push 17
    jmp nagi_fault_common
.global nagi_fault_stub_18
nagi_fault_stub_18:
    push 0
    push 18
    jmp nagi_fault_common
.global nagi_fault_stub_19
nagi_fault_stub_19:
    push 0
    push 19
    jmp nagi_fault_common
.global nagi_fault_stub_20
nagi_fault_stub_20:
    push 0
    push 20
    jmp nagi_fault_common
.global nagi_fault_stub_21
nagi_fault_stub_21:
    push 21
    jmp nagi_fault_common
.global nagi_fault_stub_22
nagi_fault_stub_22:
    push 0
    push 22
    jmp nagi_fault_common
.global nagi_fault_stub_23
nagi_fault_stub_23:
    push 0
    push 23
    jmp nagi_fault_common
.global nagi_fault_stub_24
nagi_fault_stub_24:
    push 0
    push 24
    jmp nagi_fault_common
.global nagi_fault_stub_25
nagi_fault_stub_25:
    push 0
    push 25
    jmp nagi_fault_common
.global nagi_fault_stub_26
nagi_fault_stub_26:
    push 0
    push 26
    jmp nagi_fault_common
.global nagi_fault_stub_27
nagi_fault_stub_27:
    push 0
    push 27
    jmp nagi_fault_common
.global nagi_fault_stub_28
nagi_fault_stub_28:
    push 0
    push 28
    jmp nagi_fault_common
.global nagi_fault_stub_29
nagi_fault_stub_29:
    push 29
    jmp nagi_fault_common
.global nagi_fault_stub_30
nagi_fault_stub_30:
    push 30
    jmp nagi_fault_common
.global nagi_fault_stub_31
nagi_fault_stub_31:
    push 0
    push 31
    jmp nagi_fault_common
nagi_fault_common:
    cli
    mov rdi, rsp
    and rsp, -16
    call {exception_entry}
2:
    hlt
    jmp 2b

.global nagi_fault_stub_table
nagi_fault_stub_table:
    .quad nagi_fault_stub_0
    .quad nagi_fault_stub_1
    .quad nagi_fault_stub_2
    .quad nagi_fault_stub_3
    .quad nagi_fault_stub_4
    .quad nagi_fault_stub_5
    .quad nagi_fault_stub_6
    .quad nagi_fault_stub_7
    .quad nagi_fault_stub_8
    .quad nagi_fault_stub_9
    .quad nagi_fault_stub_10
    .quad nagi_fault_stub_11
    .quad nagi_fault_stub_12
    .quad nagi_fault_stub_13
    .quad nagi_fault_stub_14
    .quad nagi_fault_stub_15
    .quad nagi_fault_stub_16
    .quad nagi_fault_stub_17
    .quad nagi_fault_stub_18
    .quad nagi_fault_stub_19
    .quad nagi_fault_stub_20
    .quad nagi_fault_stub_21
    .quad nagi_fault_stub_22
    .quad nagi_fault_stub_23
    .quad nagi_fault_stub_24
    .quad nagi_fault_stub_25
    .quad nagi_fault_stub_26
    .quad nagi_fault_stub_27
    .quad nagi_fault_stub_28
    .quad nagi_fault_stub_29
    .quad nagi_fault_stub_30
    .quad nagi_fault_stub_31

.global nagi_trigger_expected_page_fault
nagi_trigger_expected_page_fault:
    mov rax, 0xfffffffffffff000
    mov al, byte ptr [rax]
    ret

.global nagi_page_fault_resume
nagi_page_fault_resume:
    call {page_fault_resumed}
    ret
    "#,
    timer_interrupt = sym timer_interrupt,
    page_fault_interrupt = sym page_fault_interrupt,
    page_fault_resumed = sym page_fault_resumed,
    exception_entry = sym super::syscall::exception_entry,
);

extern "C" {
    static nagi_timer_stub: u8;
    static nagi_page_fault_stub: u8;
    static nagi_page_fault_resume: u8;
    static nagi_fault_stub_table: [u64; 32];
    fn nagi_trigger_expected_page_fault();
}

pub unsafe fn initialize() {
    load_idt();
    mask_pic();
    initialize_apic_timer();
    let bsp_apic_id = local_apic_id();
    BSP_APIC_ID.store(bsp_apic_id, Ordering::Release);
    TIMER_TIMEKEEPER_APIC_ID.store(bsp_apic_id, Ordering::Release);
    BSP_TIMER_LAST_COUNT.store(
        u64::from(unsafe { apic_read(APIC_TIMER_CURRENT_COUNT) }),
        Ordering::Release,
    );
    asm!("sti", options(nomem, nostack, preserves_flags));
}

pub unsafe fn initialize_ap() {
    load_idt_pointer();
    mask_pic();
    initialize_apic_timer();
}

/// Install the M5 five-entry GDT on the BSP. Interrupts must already be
/// disabled because the shared M3 IDT still serves APs using their boot GDT.
pub unsafe fn install_syscall_gdt() {
    asm!("cli", options(nomem, nostack));
    let rsp0 = ptr::addr_of!(USER_FAULT_STACK) as u64 + FAULT_STACK_SIZE as u64;
    let ist1 = ptr::addr_of!(DOUBLE_FAULT_STACK) as u64 + FAULT_STACK_SIZE as u64;
    BSP_TSS = nagi_kernel::cpu_tables::TaskStateSegment::new(rsp0, ist1);
    let [tss_low, tss_high] =
        nagi_kernel::cpu_tables::tss_descriptor(ptr::addr_of!(BSP_TSS) as u64);
    SYSCALL_GDT[5] = tss_low;
    SYSCALL_GDT[6] = tss_high;
    let gdtr = Gdtr {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: ptr::addr_of!(SYSCALL_GDT) as u64,
    };
    asm!(
        "lgdt [{gdtr}]",
        "push {kernel_code}",
        "lea rax, [rip + 2f]",
        "push rax",
        "retfq",
        "2:",
        "mov ax, {kernel_data}",
        "mov ss, ax",
        "mov ds, ax",
        "mov es, ax",
        "xor eax, eax",
        "mov fs, ax",
        "mov gs, ax",
        gdtr = in(reg) &gdtr,
        kernel_code = const KERNEL_CODE_SELECTOR,
        kernel_data = const KERNEL_DATA_SELECTOR,
        lateout("rax") _,
    );
    asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
    install_bsp_exception_idt();
}

/// Route every CPU exception on the BSP through `exception_entry` with the
/// M5 kernel code selector. #DF uses IST1. The timer gate is mirrored for
/// kernel wait loops that enable interrupts.
unsafe fn install_bsp_exception_idt() {
    let table = &*ptr::addr_of!(nagi_fault_stub_table);
    for (vector, handler) in table.iter().enumerate() {
        let ist = if vector == 8 { 1 } else { 0 };
        BSP_IDT[vector] = IdtEntry::with_ist(*handler, KERNEL_CODE_SELECTOR, ist);
    }
    BSP_IDT[usize::from(TIMER_VECTOR)] =
        IdtEntry::new(ptr::addr_of!(nagi_timer_stub) as u64, KERNEL_CODE_SELECTOR);
    let idtr = Idtr {
        limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: ptr::addr_of!(BSP_IDT) as u64,
    };
    asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack, preserves_flags));
}

pub unsafe fn disable_interrupts() {
    asm!("cli", options(nomem, nostack));
}

pub unsafe fn enable_interrupts() {
    asm!("sti", options(nomem, nostack, preserves_flags));
}

unsafe fn load_idt() {
    let selector = code_selector();
    IDT[usize::from(TIMER_VECTOR)] = IdtEntry::new(ptr::addr_of!(nagi_timer_stub) as u64, selector);
    IDT[14] = IdtEntry::new(ptr::addr_of!(nagi_page_fault_stub) as u64, selector);
    load_idt_pointer();
}

unsafe fn load_idt_pointer() {
    let idtr = Idtr {
        limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: ptr::addr_of!(IDT) as u64,
    };
    asm!("lidt [{}]", in(reg) &idtr, options(readonly, nostack, preserves_flags));
}

unsafe fn mask_pic() {
    outb(PIC_MASTER_DATA, 0xFF);
    outb(PIC_SLAVE_DATA, 0xFF);
}

unsafe fn initialize_apic_timer() {
    apic_write(APIC_SPURIOUS_INTERRUPT, 0x100 | 0xFF);
    apic_write(APIC_LVT_TIMER, u32::from(TIMER_VECTOR) | TIMER_PERIODIC);
    apic_write(APIC_TIMER_DIVIDE, 0xB);
    apic_write(APIC_TIMER_INITIAL_COUNT, 10_000_000);
}

pub fn timer_ticks() -> u64 {
    let timekeeper = TIMER_TIMEKEEPER_APIC_ID.load(Ordering::Acquire);
    if timekeeper == local_apic_id()
        && timekeeper == BSP_APIC_ID.load(Ordering::Acquire)
        && !interrupts_enabled()
    {
        account_bsp_timer_wrap();
    }
    TIMER_TICKS.load(Ordering::Relaxed)
}

fn interrupts_enabled() -> bool {
    let rflags: u64;
    unsafe {
        asm!(
            "pushfq",
            "pop {}",
            out(reg) rflags,
            options(nomem, preserves_flags),
        );
    }
    rflags & (1 << 9) != 0
}

/// Select the sole APIC timer that advances the guest's coarse clock. The BSP
/// owns it during early boot; after SMP startup an interrupt-enabled AP can
/// keep time while the BSP executes ring 3 with interrupts masked.
pub fn set_timer_timekeeper(apic_id: u32) {
    TIMER_TIMEKEEPER_APIC_ID.store(apic_id, Ordering::Release);
}

fn is_timer_timekeeper(timekeeper_apic_id: u32, interrupting_apic_id: u32) -> bool {
    timekeeper_apic_id == interrupting_apic_id
}

/// Poll the local timer counter only while the BSP is the timekeeper. This is
/// the single-CPU fallback for the cooperative user scheduler, where ring-3
/// interrupts remain disabled. The CAS prevents a timer ISR and a syscall
/// boundary from counting the same reload twice.
fn account_bsp_timer_wrap() {
    let current = u64::from(unsafe { apic_read(APIC_TIMER_CURRENT_COUNT) });
    loop {
        let previous = BSP_TIMER_LAST_COUNT.load(Ordering::Acquire);
        if current == previous {
            return;
        }
        if BSP_TIMER_LAST_COUNT
            .compare_exchange(previous, current, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            if current > previous {
                TIMER_TICKS.fetch_add(1, Ordering::AcqRel);
            }
            return;
        }
    }
}

/// Wait for the single designated APIC timekeeper to advance the shared guest
/// clock. On a single-CPU fallback, `timer_ticks` polls the BSP counter while
/// the syscall trampoline keeps interrupts masked.
pub fn wait_for_timer_ticks(periods: u64) {
    if periods == 0 {
        return;
    }
    let target = timer_ticks().saturating_add(periods);
    while timer_ticks() < target {
        core::hint::spin_loop();
    }
}

pub fn set_local_apic_base(address: u64) -> bool {
    if address == 0 || address & 0xfff != 0 {
        return false;
    }
    APIC_BASE.store(address, Ordering::Release);
    true
}

pub fn idt_base() -> u64 {
    ptr::addr_of!(IDT) as u64
}

extern "C" fn timer_interrupt(frame: *mut u64) -> *mut u64 {
    let apic_id = local_apic_id();
    let timekeeper = TIMER_TIMEKEEPER_APIC_ID.load(Ordering::Acquire);
    if is_timer_timekeeper(timekeeper, apic_id) {
        if apic_id == BSP_APIC_ID.load(Ordering::Acquire) {
            // Keep the BSP counter baseline in sync while interrupts are
            // active. Cooperative ring-3 calls poll it only while IF is clear.
            BSP_TIMER_LAST_COUNT.store(
                u64::from(unsafe { apic_read(APIC_TIMER_CURRENT_COUNT) }),
                Ordering::Release,
            );
            TIMER_TICKS.fetch_add(1, Ordering::AcqRel);
        } else {
            // The selected AP is interrupt-enabled after SMP startup, so each
            // of its periodic timer interrupts advances exactly one tick.
            TIMER_TICKS.fetch_add(1, Ordering::AcqRel);
        }
    }
    let next = super::smp::timer_preempt(frame, apic_id);
    unsafe { apic_write(APIC_EOI, 0) };
    next
}

#[cfg(test)]
mod tests {
    use super::is_timer_timekeeper;

    #[test]
    fn only_the_designated_apic_advances_guest_time() {
        assert!(is_timer_timekeeper(0x23, 0x23));
        assert!(!is_timer_timekeeper(0x23, 0x07));
        assert!(!is_timer_timekeeper(u32::MAX, 0x23));
    }
}

extern "C" fn page_fault_interrupt(error_code: u64, frame: *mut u64) -> bool {
    serial_write(b"Nagi Page fault handled (vector 14)\r\n");
    if error_code & 1 == 0 {
        serial_write(b"Nagi invalid access diagnostic PASS\r\n");
        serial_write(b"Nagi M2 acceptance PASS\r\n");
        unsafe {
            *frame.add(1) = ptr::addr_of!(nagi_page_fault_resume) as u64;
        }
        true
    } else {
        serial_write(b"Nagi invalid access diagnostic FAIL\r\n");
        false
    }
}

extern "C" fn page_fault_resumed() {
    serial_write(b"Nagi Page fault resume PASS\r\n");
}

unsafe fn apic_write(offset: u64, value: u32) {
    let base = APIC_BASE.load(Ordering::Acquire);
    ptr::write_volatile((base + offset) as *mut u32, value);
}

unsafe fn apic_read(offset: u64) -> u32 {
    let base = APIC_BASE.load(Ordering::Acquire);
    ptr::read_volatile((base + offset) as *const u32)
}

pub fn local_apic_id() -> u32 {
    unsafe { apic_read(APIC_ID) >> 24 }
}

pub fn current_code_selector() -> u16 {
    code_selector()
}

pub fn current_stack_selector() -> u16 {
    let selector: u16;
    unsafe {
        asm!("mov {0:x}, ss", out(reg) selector, options(nomem, nostack, preserves_flags));
    }
    selector
}

pub unsafe fn send_init_sipi(apic_id: u32, vector: u8) -> bool {
    apic_write(APIC_ICR_HIGH, apic_id << 24);
    apic_write(APIC_ICR_LOW, 0x0000_4500);
    if !wait_for_icr_idle() {
        return false;
    }
    delay_ipi();

    apic_write(APIC_ICR_HIGH, apic_id << 24);
    apic_write(APIC_ICR_LOW, 0x0000_C500);
    if !wait_for_icr_idle() {
        return false;
    }
    delay_ipi();

    for _ in 0..2 {
        apic_write(APIC_ICR_HIGH, apic_id << 24);
        apic_write(APIC_ICR_LOW, 0x0000_4600 | u32::from(vector));
        if !wait_for_icr_idle() {
            return false;
        }
        delay_ipi();
    }
    true
}

unsafe fn wait_for_icr_idle() -> bool {
    for _ in 0..1_000_000 {
        if apic_read(APIC_ICR_LOW) & (1 << 12) == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn delay_ipi() {
    for _ in 0..10_000 {
        core::hint::spin_loop();
    }
}

pub unsafe fn trigger_expected_page_fault() {
    nagi_trigger_expected_page_fault();
}

fn code_selector() -> u16 {
    let selector: u16;
    unsafe {
        asm!("mov {0:x}, cs", out(reg) selector, options(nomem, nostack, preserves_flags));
    }
    selector
}
