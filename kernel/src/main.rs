#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nagi_bootinfo::{boot_info_from_ptr, BootInfoError};

use nagi_kernel::memory;

mod interrupts;
mod smp;
mod syscall;

const COM1: u16 = 0x3F8;
const SERIAL_LOG_CAPACITY: usize = 4096;
static SERIAL_LOCK: AtomicBool = AtomicBool::new(false);
static mut SERIAL_LOG: [u8; SERIAL_LOG_CAPACITY] = [0; SERIAL_LOG_CAPACITY];
static SERIAL_LOG_LENGTH: AtomicUsize = AtomicUsize::new(0);

#[no_mangle]
#[link_section = ".text.entry"]
pub extern "win64" fn _start(boot_info: *const nagi_bootinfo::BootInfo) -> ! {
    serial_init();
    let boot_info = match unsafe { boot_info_from_ptr(boot_info) } {
        Ok(info) => info,
        Err(error) => {
            serial_write(b"Nagi Kernel rejected BootInfo\r\n");
            serial_write(match error {
                BootInfoError::Null => b"reason: null\r\n",
                BootInfoError::BadMagic => b"reason: magic\r\n",
                BootInfoError::UnsupportedVersion => b"reason: version\r\n",
                BootInfoError::BadSize => b"reason: size\r\n",
                BootInfoError::MissingMemoryMap => b"reason: memory-map\r\n",
                BootInfoError::BadMemoryMapStride => b"reason: memory-map-stride\r\n",
                BootInfoError::MissingFramebuffer => b"reason: framebuffer\r\n",
                BootInfoError::MissingAcpi => b"reason: acpi\r\n",
                BootInfoError::MissingInitImage => b"reason: init-image\r\n",
            });
            halt_forever();
        }
    };

    serial_write(b"Nagi Kernel started\r\n");
    let mut allocator = match unsafe { memory::PageAllocator::from_boot_info(boot_info) } {
        Ok(allocator) => allocator,
        Err(_) => {
            serial_write(b"Nagi M2 page allocator FAIL\r\n");
            halt_forever();
        }
    };
    if allocator.range_count() == 0 {
        serial_write(b"Nagi M2 page allocator FAIL\r\n");
        halt_forever();
    }
    let Some(page) = allocator.allocate_page() else {
        serial_write(b"Nagi M2 page allocator FAIL\r\n");
        halt_forever();
    };
    if !allocator.free_page(page) || allocator.allocate_page() != Some(page) {
        serial_write(b"Nagi M2 page allocation/free FAIL\r\n");
        halt_forever();
    }
    let Some(mut heap) = memory::KernelHeap::new(page, memory::PAGE_SIZE) else {
        serial_write(b"Nagi M2 memory primitives FAIL\r\n");
        halt_forever();
    };
    let page_flags = memory::PageTableEntry::PRESENT | memory::PageTableEntry::WRITABLE;
    let Some(page_table_entry) = memory::PageTableEntry::new(page, page_flags) else {
        serial_write(b"Nagi M2 memory primitives FAIL\r\n");
        halt_forever();
    };
    if !page_table_entry.is_present()
        || page_table_entry.raw() != page | page_flags
        || (memory::PageTableEntry::USER & page_flags) != 0
        || {
            let mut page_table = memory::PageTable::empty();
            !page_table.map(0, page_table_entry)
                || page_table.entry(0) != Some(page_table_entry)
                || page_table.unmap(0) != Some(page_table_entry)
        }
        || heap.allocate(64, 8).is_none()
    {
        serial_write(b"Nagi M2 memory primitives FAIL\r\n");
        halt_forever();
    }
    heap.reset();
    serial_write(b"Nagi M2 page allocation/free PASS\r\n");

    serial_write(b"Nagi M3 ACPI discovery START\r\n");
    let topology =
        match unsafe { nagi_kernel::acpi::discover(boot_info, interrupts::local_apic_id()) } {
            Ok(topology) => topology,
            Err(_) => {
                serial_write(b"Nagi M3 ACPI discovery FAIL\r\n");
                halt_forever();
            }
        };
    if !interrupts::set_local_apic_base(topology.local_apic_address()) {
        serial_write(b"Nagi M3 APIC base FAIL\r\n");
        halt_forever();
    }
    serial_write(b"Nagi M3 ACPI discovery PASS\r\n");
    unsafe { interrupts::initialize() };
    while interrupts::timer_ticks() < 3 {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
    serial_write(b"Nagi M2 timer interrupts PASS\r\n");
    unsafe { interrupts::trigger_expected_page_fault() };
    serial_write(b"Nagi M3 SMP startup START\r\n");
    if smp::initialize(&topology, &mut allocator, boot_info).is_err() {
        serial_write(b"Nagi M3 SMP startup FAIL\r\n");
        halt_forever();
    }
    for index in 0..nagi_kernel::acpi::MAX_CPUS {
        if !smp::cpu_pass(index) {
            serial_write(b"Nagi M3 CPU workload FAIL\r\n");
            halt_forever();
        }
        match index {
            0 => serial_write(b"Nagi M3 CPU 0 online/workload PASS\r\n"),
            1 => serial_write(b"Nagi M3 CPU 1 online/workload PASS\r\n"),
            2 => serial_write(b"Nagi M3 CPU 2 online/workload PASS\r\n"),
            3 => serial_write(b"Nagi M3 CPU 3 online/workload PASS\r\n"),
            _ => unreachable!(),
        }
    }
    serial_write(b"Nagi M3 scheduler workloads PASS\r\n");
    serial_write(b"Nagi M3 acceptance PASS\r\n");
    serial_write(b"Nagi M4 handles/VMO/IPC START\r\n");
    if !nagi_kernel::m4::run_acceptance() {
        serial_write(b"Nagi M4 acceptance FAIL\r\n");
        halt_forever();
    }
    serial_write(b"Nagi M4 VMO basics PASS\r\n");
    serial_write(b"Nagi M4 channel round-trip PASS\r\n");
    serial_write(b"Nagi M4 rights attenuation PASS\r\n");
    serial_write(b"Nagi M4 wait primitives PASS\r\n");
    serial_write(b"Nagi M4 acceptance PASS\r\n");
    serial_write(b"Nagi M7 storage START\r\n");
    if nagi_kernel::virtio::initialize().is_err() {
        serial_write(b"Nagi M7 VirtIO Block FAIL\r\n");
        halt_forever();
    }
    serial_write(b"Nagi M7 VirtIO Block PASS\r\n");
    match nagi_kernel::net::initialize() {
        Ok(()) => serial_write(b"Nagi M12 VirtIO Net PASS\r\n"),
        Err(error) => {
            serial_write(b"Nagi M12 VirtIO Net FAIL: ");
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
        }
    }
    match nagi_kernel::audio::initialize() {
        Ok(()) => serial_write(b"Nagi M14 VirtIO Sound PASS\r\n"),
        Err(error) => {
            serial_write(b"Nagi M14 VirtIO Sound FAIL: ");
            serial_write(match error {
                nagi_kernel::audio::SoundError::NotInitialized => b"not-initialized\r\n",
                nagi_kernel::audio::SoundError::PciUnavailable => b"pci\r\n",
                nagi_kernel::audio::SoundError::InvalidBar => b"bar\r\n",
                nagi_kernel::audio::SoundError::UnsupportedQueue => b"queue\r\n",
                nagi_kernel::audio::SoundError::AddressOutOfRange => b"address\r\n",
                nagi_kernel::audio::SoundError::DeviceFailure => b"device\r\n",
                nagi_kernel::audio::SoundError::RequestTimeout => b"timeout\r\n",
                nagi_kernel::audio::SoundError::QueueCorrupt => b"corrupt\r\n",
                nagi_kernel::audio::SoundError::InvalidStream => b"stream\r\n",
                nagi_kernel::audio::SoundError::InvalidBuffer => b"buffer\r\n",
                nagi_kernel::audio::SoundError::UnsupportedFormat => b"format\r\n",
                nagi_kernel::audio::SoundError::Busy => b"busy\r\n",
            });
            halt_forever();
        }
    }
    if nagi_kernel::display::initialize(boot_info.framebuffer).is_err() {
        serial_write(b"Nagi M9 display setup FAIL\r\n");
        halt_forever();
    }
    serial_write(b"Nagi M9 display setup PASS\r\n");
    if nagi_kernel::input::initialize() == 0 {
        match nagi_kernel::input::initialization_error() {
            2 => serial_write(b"Nagi M9 input setup FAIL: BAR\r\n"),
            3 => serial_write(b"Nagi M9 input setup FAIL: queue\r\n"),
            4 => serial_write(b"Nagi M9 input setup FAIL: address\r\n"),
            5 => serial_write(b"Nagi M9 input setup FAIL: capabilities\r\n"),
            6 => serial_write(b"Nagi M9 input setup FAIL: capability pointer\r\n"),
            7 => serial_write(b"Nagi M9 input setup FAIL: vendor capability\r\n"),
            8 => serial_write(b"Nagi M9 input setup FAIL: common config\r\n"),
            9 => serial_write(b"Nagi M9 input setup FAIL: notify config\r\n"),
            10 => serial_write(b"Nagi M9 input setup FAIL: capability BAR\r\n"),
            11 => serial_write(b"Nagi M9 input setup FAIL: common type\r\n"),
            12 => serial_write(b"Nagi M9 input setup FAIL: common length\r\n"),
            _ => serial_write(b"Nagi M9 input setup FAIL: device\r\n"),
        }
        halt_forever();
    }
    serial_write(b"Nagi M9 input setup PASS\r\n");
    serial_write(b"Nagi M5 user process START\r\n");
    if boot_info.validate_for_user_bootstrap().is_err() {
        serial_write(b"Nagi M5 init image FAIL\r\n");
        halt_forever();
    }
    let context = match nagi_kernel::user_process::prepare(boot_info, &mut allocator) {
        Ok(context) => context,
        Err(error) => {
            serial_write(b"Nagi M5 user address space FAIL\r\n");
            serial_write(match error {
                nagi_kernel::user_process::UserProcessError::InvalidBootInfo(_) => {
                    b"reason: boot-info\r\n"
                }
                nagi_kernel::user_process::UserProcessError::ImageTooLarge => {
                    b"reason: image-too-large\r\n"
                }
                nagi_kernel::user_process::UserProcessError::ImageNotMapped => {
                    b"reason: image-not-mapped\r\n"
                }
                nagi_kernel::user_process::UserProcessError::InvalidElf(_) => {
                    b"reason: invalid-elf\r\n"
                }
                nagi_kernel::user_process::UserProcessError::InvalidLoadPlan => {
                    b"reason: invalid-load-plan\r\n"
                }
                nagi_kernel::user_process::UserProcessError::ImageOutOfBounds => {
                    b"reason: image-out-of-bounds\r\n"
                }
                nagi_kernel::user_process::UserProcessError::KernelUserSlotOccupied => {
                    b"reason: user-slot-occupied\r\n"
                }
                nagi_kernel::user_process::UserProcessError::InvalidPhysicalAddress => {
                    b"reason: physical-address\r\n"
                }
                nagi_kernel::user_process::UserProcessError::PhysicalMemoryExhausted => {
                    b"reason: physical-memory-exhausted\r\n"
                }
            });
            halt_forever();
        }
    };
    if syscall::initialize().is_err() {
        serial_write(b"Nagi M5 syscall initialization FAIL\r\n");
        halt_forever();
    }
    unsafe { nagi_kernel::user_process::enter(context) }
}

#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    serial_write(b"Nagi Kernel panic\r\n");
    halt_forever();
}

fn serial_init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

pub(crate) fn serial_write(bytes: &[u8]) {
    while SERIAL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    for &byte in bytes {
        unsafe {
            let position = SERIAL_LOG_LENGTH.load(Ordering::Relaxed);
            SERIAL_LOG[position % SERIAL_LOG_CAPACITY] = byte;
            SERIAL_LOG_LENGTH.store(position.saturating_add(1), Ordering::Relaxed);
        }
        unsafe {
            while inb(COM1 + 5) & 0x20 == 0 {
                core::hint::spin_loop();
            }
            outb(COM1, byte);
        }
    }
    SERIAL_LOCK.store(false, Ordering::Release);
}

pub(crate) fn serial_read_byte() -> Option<u8> {
    unsafe {
        if inb(COM1 + 5) & 0x01 == 0 {
            None
        } else {
            Some(inb(COM1))
        }
    }
}

pub(crate) fn serial_log_read(destination: &mut [u8]) -> usize {
    while SERIAL_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let total = SERIAL_LOG_LENGTH.load(Ordering::Relaxed);
    let available = total.min(SERIAL_LOG_CAPACITY);
    let count = destination.len().min(available);
    let start = total.saturating_sub(count);
    for (index, byte) in destination.iter_mut().take(count).enumerate() {
        unsafe {
            *byte = SERIAL_LOG[(start + index) % SERIAL_LOG_CAPACITY];
        }
    }
    SERIAL_LOCK.store(false, Ordering::Release);
    count
}

fn halt_forever() -> ! {
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

unsafe fn outb(port: u16, value: u8) {
    asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, count: usize) -> *mut u8 {
    for index in 0..count {
        destination.add(index).write_volatile(value as u8);
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    for index in 0..count {
        destination
            .add(index)
            .write_volatile(source.add(index).read_volatile());
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memmove(destination: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    if (destination as usize) <= source as usize {
        for index in 0..count {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    } else {
        for index in (0..count).rev() {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
    destination
}
