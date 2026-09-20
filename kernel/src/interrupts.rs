use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

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

static SYSCALL_GDT: [u64; 5] = [
    0,
    0x00af_9a00_0000_ffff,
    0x00cf_9200_0000_ffff,
    0x00cf_f200_0000_ffff,
    0x00af_fa00_0000_ffff,
];

static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
static APIC_BASE: AtomicU64 = AtomicU64::new(DEFAULT_APIC_BASE);

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
        Self {
            offset_low: handler as u16,
            selector,
            options: 0x8E00,
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
);

extern "C" {
    static nagi_timer_stub: u8;
    static nagi_page_fault_stub: u8;
    static nagi_page_fault_resume: u8;
    fn nagi_trigger_expected_page_fault();
}

pub unsafe fn initialize() {
    load_idt();
    mask_pic();
    initialize_apic_timer();
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
    let gdtr = Gdtr {
        limit: (core::mem::size_of_val(&SYSCALL_GDT) - 1) as u16,
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
    TIMER_TICKS.load(Ordering::Relaxed)
}

/// Wait for periodic local-APIC timer periods without entering an interrupt
/// handler from the syscall trampoline. The syscall path masks interrupts and
/// has its own fixed stack; polling the guest timer counter keeps this low-level
/// wait safe while remaining entirely inside Nagi hardware state.
pub fn wait_for_timer_ticks(periods: u64) {
    if periods == 0 {
        return;
    }
    let mut previous = unsafe { apic_read(APIC_TIMER_CURRENT_COUNT) };
    let mut elapsed = 0;
    while elapsed < periods {
        let current = unsafe { apic_read(APIC_TIMER_CURRENT_COUNT) };
        if current > previous {
            elapsed += 1;
        }
        previous = current;
        core::hint::spin_loop();
    }
    TIMER_TICKS.fetch_add(periods, Ordering::AcqRel);
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
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    let next = super::smp::timer_preempt(frame, local_apic_id());
    unsafe { apic_write(APIC_EOI, 0) };
    next
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
