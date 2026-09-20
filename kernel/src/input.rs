use core::arch::asm;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, AtomicU64, AtomicUsize, Ordering};

use nagi_abi::InputEvent;

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_INPUT_LEGACY_ID: u16 = 0x1011;
const VIRTIO_INPUT_MODERN_ID: u16 = 0x1052;
const PCI_COMMAND_OFFSET: u8 = 0x04;
const PCI_BAR0_OFFSET: u8 = 0x10;
const PCI_BAR4_OFFSET: u8 = 0x20;
const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
const LEGACY_QUEUE_SIZE: u16 = 0x0c;
const LEGACY_QUEUE_SELECT: u16 = 0x0e;
const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
const LEGACY_DEVICE_STATUS: u16 = 0x12;
const QUEUE_SIZE: usize = 64;
const MAX_DEVICES: usize = 2;
const INPUT_EVENT_BYTES: usize = 8;
const QUEUE_USED_RING_OFFSET: usize = 4096;
const QUEUE_AVAILABLE_END: usize = size_of::<[Descriptor; QUEUE_SIZE]>()
    + size_of::<u16>()
    + size_of::<u16>()
    + size_of::<[u16; QUEUE_SIZE]>()
    + size_of::<u16>();
const MAX_EVENTS: usize = 128;
const DESC_F_WRITE: u16 = 2;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UsedElement {
    id: u32,
    length: u32,
}

#[repr(C, align(4096))]
struct InputQueue {
    descriptors: [Descriptor; QUEUE_SIZE],
    available_flags: u16,
    available_index: u16,
    available_ring: [u16; QUEUE_SIZE],
    used_event: u16,
    _padding: [u8; QUEUE_USED_RING_OFFSET - QUEUE_AVAILABLE_END],
    used_flags: u16,
    used_index: u16,
    used_ring: [UsedElement; QUEUE_SIZE],
    available_event: u16,
}

impl InputQueue {
    const fn empty() -> Self {
        Self {
            descriptors: [Descriptor {
                address: 0,
                length: 0,
                flags: 0,
                next: 0,
            }; QUEUE_SIZE],
            available_flags: 0,
            available_index: 0,
            available_ring: [0; QUEUE_SIZE],
            used_event: 0,
            _padding: [0; QUEUE_USED_RING_OFFSET - QUEUE_AVAILABLE_END],
            used_flags: 0,
            used_index: 0,
            used_ring: [UsedElement { id: 0, length: 0 }; QUEUE_SIZE],
            available_event: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct DeviceState {
    io_base: u16,
    notify_address: u64,
    modern: bool,
    queue_count: u16,
    used_index: u16,
}

#[derive(Clone, Copy)]
struct ModernConfig {
    common_address: u64,
    notify_address: u64,
    notify_multiplier: u32,
}

static mut QUEUES: [InputQueue; MAX_DEVICES] = [const { InputQueue::empty() }; MAX_DEVICES];
static mut EVENT_BUFFERS: [[[u8; INPUT_EVENT_BYTES]; QUEUE_SIZE]; MAX_DEVICES] =
    [[[0; INPUT_EVENT_BYTES]; QUEUE_SIZE]; MAX_DEVICES];
static mut DEVICES: [Option<DeviceState>; MAX_DEVICES] = [None; MAX_DEVICES];
static INPUT_CAPABILITY: AtomicU64 = AtomicU64::new(0);
static INPUT_INITIALIZATION_ERROR: AtomicUsize = AtomicUsize::new(0);
static mut EVENT_RING: [InputEvent; MAX_EVENTS] = [InputEvent {
    event_type: 0,
    code: 0,
    value: 0,
}; MAX_EVENTS];
static EVENT_HEAD: AtomicUsize = AtomicUsize::new(0);
static EVENT_TAIL: AtomicUsize = AtomicUsize::new(0);

pub fn initialize() -> usize {
    if INPUT_CAPABILITY.load(Ordering::Acquire) != 0 {
        return device_count();
    }
    let mut count = 0;
    let mut last_error = 1;
    for device in 0..32 {
        for function in 0..8 {
            if count == MAX_DEVICES {
                break;
            }
            let identity = unsafe { pci_config_read32(0, device, function, 0) };
            let vendor = identity as u16;
            let device_id = (identity >> 16) as u16;
            if vendor != VIRTIO_VENDOR_ID
                || (device_id != VIRTIO_INPUT_LEGACY_ID && device_id != VIRTIO_INPUT_MODERN_ID)
            {
                continue;
            }
            let setup = if device_id == VIRTIO_INPUT_LEGACY_ID {
                let Some(io_base) = enable_io_device(device, function) else {
                    last_error = 2;
                    continue;
                };
                setup_legacy_device(count, io_base)
            } else {
                setup_modern_device(count, device, function)
            };
            match setup {
                Err(error) => last_error = error,
                Ok(device_state) => {
                    let capability = make_capability(device, function, count as u8);
                    unsafe {
                        ptr::write_volatile(ptr::addr_of_mut!(DEVICES[count]), Some(device_state));
                    }
                    INPUT_CAPABILITY.fetch_xor(capability, Ordering::AcqRel);
                    count += 1;
                }
            }
        }
        if count == MAX_DEVICES {
            break;
        }
    }
    if count != 0 {
        let capability = INPUT_CAPABILITY.load(Ordering::Acquire);
        INPUT_CAPABILITY.store(
            if capability == 0 { 1 } else { capability },
            Ordering::Release,
        );
    } else {
        INPUT_INITIALIZATION_ERROR.store(last_error, Ordering::Release);
    }
    count
}

pub fn initialization_error() -> usize {
    INPUT_INITIALIZATION_ERROR.load(Ordering::Acquire)
}

pub fn user_capability() -> u64 {
    INPUT_CAPABILITY.load(Ordering::Acquire)
}

pub fn capability_matches(capability: u64) -> bool {
    capability != 0 && capability == user_capability()
}

pub fn read_event(capability: u64) -> Option<InputEvent> {
    if !capability_matches(capability) {
        return None;
    }
    poll_devices();
    let tail = EVENT_TAIL.load(Ordering::Acquire);
    let head = EVENT_HEAD.load(Ordering::Acquire);
    if tail == head {
        return None;
    }
    let event = unsafe { ptr::read_volatile(&EVENT_RING[tail % MAX_EVENTS]) };
    EVENT_TAIL.store(tail.wrapping_add(1), Ordering::Release);
    Some(event)
}

fn device_count() -> usize {
    let devices = ptr::addr_of!(DEVICES).cast::<Option<DeviceState>>();
    (0..MAX_DEVICES)
        .filter(|index| unsafe { ptr::read_volatile(devices.add(*index)) }.is_some())
        .count()
}

fn setup_legacy_device(index: usize, io_base: u16) -> Result<DeviceState, usize> {
    unsafe {
        io_write8(io_base + LEGACY_DEVICE_STATUS, 0);
        io_write8(io_base + LEGACY_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
        io_write8(
            io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
        );
        let queue_size = io_read16(io_base + LEGACY_QUEUE_SIZE) as usize;
        if queue_size == 0 {
            return Err(3);
        }
        let count = queue_size.min(QUEUE_SIZE);
        ptr::write_bytes(
            ptr::addr_of_mut!(QUEUES[index]).cast::<u8>(),
            0,
            size_of::<InputQueue>(),
        );
        for slot in 0..count {
            let event_address = ptr::addr_of_mut!(EVENT_BUFFERS[index][slot]) as u64;
            QUEUES[index].descriptors[slot] = Descriptor {
                address: event_address,
                length: INPUT_EVENT_BYTES as u32,
                flags: DESC_F_WRITE,
                next: 0,
            };
            QUEUES[index].available_ring[slot] = slot as u16;
        }
        QUEUES[index].available_index = count as u16;
        io_write16(io_base + LEGACY_QUEUE_SELECT, 0);
        let queue_address = ptr::addr_of!(QUEUES[index]) as u64;
        if queue_address & 0xfff != 0 || queue_address >> 12 > u64::from(u32::MAX) {
            return Err(4);
        }
        io_write32(io_base + LEGACY_QUEUE_ADDRESS, (queue_address >> 12) as u32);
        io_write8(
            io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
        io_write16(io_base + LEGACY_QUEUE_NOTIFY, 0);
        Ok(DeviceState {
            io_base,
            notify_address: 0,
            modern: false,
            queue_count: count as u16,
            used_index: 0,
        })
    }
}

fn setup_modern_device(index: usize, device: u8, function: u8) -> Result<DeviceState, usize> {
    let config = unsafe { find_modern_config(device, function) }?;
    let common = config.common_address;
    unsafe {
        mmio_write8(common, 20, 0);
        mmio_write8(common, 20, STATUS_ACKNOWLEDGE);
        mmio_write8(common, 20, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        mmio_write32(common, 0, 0);
        let _ = mmio_read32(common, 4);
        mmio_write32(common, 8, 0);
        mmio_write32(common, 12, 0);
        mmio_write16(common, 22, 0);
        let queue_size = usize::from(mmio_read16(common, 24));
        if queue_size == 0 {
            return Err(3);
        }
        let count = queue_size.min(QUEUE_SIZE);
        ptr::write_bytes(
            ptr::addr_of_mut!(QUEUES[index]).cast::<u8>(),
            0,
            size_of::<InputQueue>(),
        );
        for slot in 0..count {
            let event_address = ptr::addr_of_mut!(EVENT_BUFFERS[index][slot]) as u64;
            QUEUES[index].descriptors[slot] = Descriptor {
                address: event_address,
                length: INPUT_EVENT_BYTES as u32,
                flags: DESC_F_WRITE,
                next: 0,
            };
            QUEUES[index].available_ring[slot] = slot as u16;
        }
        QUEUES[index].available_index = count as u16;
        let descriptor_address = ptr::addr_of!(QUEUES[index].descriptors) as u64;
        let driver_address = ptr::addr_of!(QUEUES[index].available_flags) as u64;
        let device_address = ptr::addr_of!(QUEUES[index].used_flags) as u64;
        if descriptor_address & 0xfff != 0 || driver_address & 1 != 0 || device_address & 1 != 0 {
            return Err(4);
        }
        mmio_write64(common, 32, descriptor_address);
        mmio_write64(common, 40, driver_address);
        mmio_write64(common, 48, device_address);
        let notify_offset = u64::from(mmio_read16(common, 30));
        mmio_write16(common, 28, 1);
        mmio_write8(
            common,
            20,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
        Ok(DeviceState {
            io_base: 0,
            notify_address: config
                .notify_address
                .saturating_add(notify_offset.saturating_mul(u64::from(config.notify_multiplier))),
            modern: true,
            queue_count: count as u16,
            used_index: 0,
        })
    }
}

fn poll_devices() {
    for index in 0..MAX_DEVICES {
        let Some(mut device) = (unsafe { DEVICES[index] }) else {
            continue;
        };
        let used_index = unsafe { ptr::read_volatile(&QUEUES[index].used_index) };
        while device.used_index != used_index {
            let used_slot = usize::from(device.used_index) % usize::from(device.queue_count);
            let used = unsafe { ptr::read_volatile(&QUEUES[index].used_ring[used_slot]) };
            if (used.id as usize) < usize::from(device.queue_count) {
                let event = unsafe { decode_event(&EVENT_BUFFERS[index][used.id as usize]) };
                push_event(event);
                unsafe {
                    let available_slot = usize::from(QUEUES[index].available_index)
                        % usize::from(device.queue_count);
                    QUEUES[index].available_ring[available_slot] = used.id as u16;
                    fence(Ordering::SeqCst);
                    QUEUES[index].available_index = QUEUES[index].available_index.wrapping_add(1);
                    if device.modern {
                        mmio_write16(device.notify_address, 0, 0);
                    } else {
                        io_write16(device.io_base + LEGACY_QUEUE_NOTIFY, 0);
                    }
                }
            }
            device.used_index = device.used_index.wrapping_add(1);
        }
        unsafe { ptr::write_volatile(ptr::addr_of_mut!(DEVICES[index]), Some(device)) };
    }
}

fn push_event(event: InputEvent) {
    let head = EVENT_HEAD.load(Ordering::Relaxed);
    let tail = EVENT_TAIL.load(Ordering::Acquire);
    if head.wrapping_sub(tail) >= MAX_EVENTS {
        EVENT_TAIL.store(tail.wrapping_add(1), Ordering::Release);
    }
    unsafe { ptr::write_volatile(&raw mut EVENT_RING[head % MAX_EVENTS], event) };
    EVENT_HEAD.store(head.wrapping_add(1), Ordering::Release);
}

fn decode_event(bytes: &[u8; INPUT_EVENT_BYTES]) -> InputEvent {
    InputEvent {
        event_type: u16::from_le_bytes([bytes[0], bytes[1]]),
        code: u16::from_le_bytes([bytes[2], bytes[3]]),
        value: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    }
}

fn enable_io_device(device: u8, function: u8) -> Option<u16> {
    let bar0 = unsafe { pci_config_read32(0, device, function, PCI_BAR0_OFFSET) };
    let bar4 = unsafe { pci_config_read32(0, device, function, PCI_BAR4_OFFSET) };
    let bar = if bar0 & 1 != 0 {
        bar0
    } else if bar4 & 1 != 0 {
        bar4
    } else {
        return None;
    };
    let io_base = (bar & 0xfffc) as u16;
    if io_base == 0 {
        return None;
    }
    let command = unsafe { pci_config_read16(0, device, function, PCI_COMMAND_OFFSET) };
    unsafe {
        pci_config_write16(0, device, function, PCI_COMMAND_OFFSET, command | 0x0005);
    }
    Some(io_base)
}

unsafe fn find_modern_config(device: u8, function: u8) -> Result<ModernConfig, usize> {
    const PCI_CAPABILITY_VENDOR_SPECIFIC: u8 = 0x09;
    const VIRTIO_PCI_COMMON_CFG: u8 = 1;
    const VIRTIO_PCI_NOTIFY_CFG: u8 = 2;
    let mut common = None;
    let mut notify = None;
    let mut multiplier = 0;
    let mut capability = pci_config_read8(0, device, function, 0x34);
    if capability < 0x40 {
        return Err(6);
    }
    let mut steps = 0;
    let mut found_vendor = false;
    let mut found_common_type = false;
    let mut found_common_length = false;
    while capability >= 0x40 && steps < 48 {
        if pci_config_read8(0, device, function, capability) == PCI_CAPABILITY_VENDOR_SPECIFIC {
            found_vendor = true;
            let cfg_type = pci_config_read8(0, device, function, capability + 3);
            let bar = pci_config_read8(0, device, function, capability + 4);
            let offset = pci_config_read32(0, device, function, capability + 8);
            let length = pci_config_read32(0, device, function, capability + 12);
            if cfg_type == VIRTIO_PCI_COMMON_CFG {
                found_common_type = true;
                if length != 0 {
                    found_common_length = true;
                }
            }
            if length != 0 {
                let Some(base) = read_bar_base(device, function, bar) else {
                    return Err(10);
                };
                let Some(address) = base.checked_add(u64::from(offset)) else {
                    return Err(10);
                };
                match cfg_type {
                    VIRTIO_PCI_COMMON_CFG => common = Some(address),
                    VIRTIO_PCI_NOTIFY_CFG => {
                        notify = Some(address);
                        multiplier = pci_config_read32(0, device, function, capability + 16);
                    }
                    _ => {}
                }
            }
        }
        let next = pci_config_read8(0, device, function, capability + 1);
        if next == 0 || next == capability {
            break;
        }
        capability = next;
        steps += 1;
    }
    if !found_vendor {
        return Err(7);
    }
    let Some(common_address) = common else {
        return Err(if !found_common_type {
            11
        } else if !found_common_length {
            12
        } else {
            8
        });
    };
    let Some(notify_address) = notify else {
        return Err(9);
    };
    Ok(ModernConfig {
        common_address,
        notify_address,
        notify_multiplier: multiplier,
    })
}

unsafe fn read_bar_base(device: u8, function: u8, bar: u8) -> Option<u64> {
    if bar >= 6 {
        return None;
    }
    let offset = PCI_BAR0_OFFSET.checked_add(bar.checked_mul(4)?)?;
    let low = pci_config_read32(0, device, function, offset);
    if low & 1 != 0 {
        return None;
    }
    let memory_type = (low >> 1) & 0x3;
    let base = u64::from(low & 0xffff_fff0);
    if memory_type == 0x2 {
        let high = pci_config_read32(0, device, function, offset + 4);
        Some(base | (u64::from(high) << 32))
    } else {
        Some(base)
    }
}

const fn make_capability(device: u8, function: u8, index: u8) -> u64 {
    let value = 0x4e41_4749_494e_5054_u64
        ^ ((device as u64) << 16)
        ^ ((function as u64) << 8)
        ^ index as u64;
    if value == 0 {
        1
    } else {
        value
    }
}

unsafe fn pci_config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    io_write32(
        PCI_CONFIG_ADDRESS,
        pci_config_address(bus, device, function, offset),
    );
    io_read32(PCI_CONFIG_DATA)
}

unsafe fn pci_config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let value = pci_config_read32(bus, device, function, offset & 0xfc);
    (value >> u32::from((offset & 3) * 8)) as u8
}

unsafe fn pci_config_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let value = pci_config_read32(bus, device, function, offset & 0xfc);
    (value >> u32::from((offset & 2) * 8)) as u16
}

unsafe fn pci_config_write16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    io_write32(
        PCI_CONFIG_ADDRESS,
        pci_config_address(bus, device, function, offset),
    );
    let current = io_read32(PCI_CONFIG_DATA);
    let shift = u32::from((offset & 2) * 8);
    let mask = 0xffff_u32 << shift;
    io_write32(
        PCI_CONFIG_DATA,
        (current & !mask) | (u32::from(value) << shift),
    );
}

const fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset & 0xfc) as u32
}

unsafe fn io_read16(port: u16) -> u16 {
    let value: u16;
    asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack, preserves_flags));
    value
}

unsafe fn io_read32(port: u16) -> u32 {
    let value: u32;
    asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
    value
}

unsafe fn io_write8(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

unsafe fn io_write16(port: u16, value: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
}

unsafe fn io_write32(port: u16, value: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
}

unsafe fn mmio_read16(base: u64, offset: usize) -> u16 {
    ptr::read_volatile((base + offset as u64) as *const u16)
}

unsafe fn mmio_read32(base: u64, offset: usize) -> u32 {
    ptr::read_volatile((base + offset as u64) as *const u32)
}

unsafe fn mmio_write8(base: u64, offset: usize, value: u8) {
    ptr::write_volatile((base + offset as u64) as *mut u8, value);
}

unsafe fn mmio_write16(base: u64, offset: usize, value: u16) {
    ptr::write_volatile((base + offset as u64) as *mut u16, value);
}

unsafe fn mmio_write32(base: u64, offset: usize, value: u32) {
    ptr::write_volatile((base + offset as u64) as *mut u32, value);
}

unsafe fn mmio_write64(base: u64, offset: usize, value: u64) {
    ptr::write_volatile((base + offset as u64) as *mut u64, value);
}

#[cfg(test)]
mod tests {
    use super::decode_event;

    #[test]
    fn decodes_little_endian_virtio_input_events() {
        let event = decode_event(&[2, 0, 1, 0, 0xfb, 0xff, 0xff, 0xff]);
        assert_eq!(event.event_type, 2);
        assert_eq!(event.code, 1);
        assert_eq!(event.value, -5);
    }
}
