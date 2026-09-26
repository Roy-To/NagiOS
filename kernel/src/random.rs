use core::arch::asm;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, AtomicBool, Ordering};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_RNG_LEGACY_ID: u16 = 0x1005;
const VIRTIO_RNG_MODERN_ID: u16 = 0x1044;
const PCI_COMMAND_OFFSET: u8 = 0x04;
const PCI_BAR0_OFFSET: u8 = 0x10;
const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
const LEGACY_QUEUE_SIZE: u16 = 0x0c;
const LEGACY_QUEUE_SELECT: u16 = 0x0e;
const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
const LEGACY_DEVICE_STATUS: u16 = 0x12;
const MAX_REQUEST_SPINS: usize = 5_000_000;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const DESC_F_WRITE: u16 = 2;
const QUEUE_SIZE: usize = 8;
const QUEUE_USED_RING_OFFSET: usize = 4096;
const QUEUE_AVAILABLE_END: usize =
    size_of::<Descriptor>() * QUEUE_SIZE + 2 + 2 + 2 * QUEUE_SIZE + 2;

pub const MAX_RANDOM_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RandomError {
    NotInitialized,
    PciUnavailable,
    InvalidBar,
    UnsupportedQueue,
    AddressOutOfRange,
    DeviceFailure,
    RequestTimeout,
    QueueCorrupt,
    InvalidBuffer,
    Busy,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UsedElement {
    id: u32,
    length: u32,
}

#[repr(C, align(4096))]
struct LegacyQueue {
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

impl LegacyQueue {
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
}

static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
static mut QUEUE: LegacyQueue = LegacyQueue::empty();
static mut RANDOM_BUFFER: [u8; MAX_RANDOM_BYTES] = [0; MAX_RANDOM_BYTES];
static mut DEVICE: Option<DeviceState> = None;

/// Initialize the guest's legacy VirtIO entropy device.
pub fn initialize() -> Result<(), RandomError> {
    if unsafe { ptr::addr_of!(DEVICE).read_volatile() }.is_some() {
        return Ok(());
    }
    let Some(candidate) = discover_device() else {
        return Err(RandomError::PciUnavailable);
    };
    if candidate.io_base == 0 {
        return Err(RandomError::InvalidBar);
    }
    let queue_size = unsafe { io_read16(candidate.io_base + LEGACY_QUEUE_SIZE) } as usize;
    if queue_size < QUEUE_SIZE {
        return Err(RandomError::UnsupportedQueue);
    }
    let queue_address = ptr::addr_of!(QUEUE) as u64;
    if !queue_address_is_usable(queue_address) {
        return Err(RandomError::AddressOutOfRange);
    }

    unsafe {
        io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, 0);
        io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
        io_write8(
            candidate.io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
        );
        io_write32(candidate.io_base + 0x04, 0);
        io_write16(candidate.io_base + LEGACY_QUEUE_SELECT, 0);
        ptr::write_bytes(
            ptr::addr_of_mut!(QUEUE).cast::<u8>(),
            0,
            size_of::<LegacyQueue>(),
        );
        io_write32(
            candidate.io_base + LEGACY_QUEUE_ADDRESS,
            (queue_address >> 12) as u32,
        );
        io_write8(
            candidate.io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
    }
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(DEVICE),
            Some(DeviceState {
                io_base: candidate.io_base,
            }),
        );
    }
    Ok(())
}

/// Fill a bounded buffer from the real guest VirtIO RNG device.
pub fn fill(destination: &mut [u8]) -> Result<(), RandomError> {
    if destination.is_empty() {
        return Ok(());
    }
    if destination.len() > MAX_RANDOM_BYTES {
        return Err(RandomError::InvalidBuffer);
    }
    initialize()?;
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(RandomError::NotInitialized);
    };
    if REQUEST_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(RandomError::Busy);
    }
    let result = unsafe { fill_locked(device, destination) };
    REQUEST_LOCK.store(false, Ordering::Release);
    result
}

unsafe fn fill_locked(device: DeviceState, destination: &mut [u8]) -> Result<(), RandomError> {
    let queue = &mut *ptr::addr_of_mut!(QUEUE);
    let buffer_address = ptr::addr_of!(RANDOM_BUFFER) as u64;
    if buffer_address > u64::from(u32::MAX) << 12 {
        return Err(RandomError::AddressOutOfRange);
    }

    let mut offset = 0;
    while offset < destination.len() {
        let request_length = destination.len() - offset;
        let previous_used = queue.used_index;
        queue.descriptors[0] = Descriptor {
            address: buffer_address,
            length: request_length as u32,
            flags: DESC_F_WRITE,
            next: 0,
        };
        let available_slot = usize::from(queue.available_index) % QUEUE_SIZE;
        queue.available_ring[available_slot] = 0;
        fence(Ordering::SeqCst);
        queue.available_index = queue.available_index.wrapping_add(1);
        fence(Ordering::SeqCst);
        io_write16(device.io_base + LEGACY_QUEUE_NOTIFY, 0);

        let mut spins = 0;
        while ptr::read_volatile(&queue.used_index) == previous_used {
            if spins == MAX_REQUEST_SPINS {
                return Err(RandomError::RequestTimeout);
            }
            spins += 1;
            core::hint::spin_loop();
        }
        let used_slot = usize::from(previous_used) % QUEUE_SIZE;
        let used = ptr::read_volatile(queue.used_ring.as_ptr().add(used_slot));
        if used.id != 0 || used.length == 0 || used.length > request_length as u32 {
            return Err(RandomError::QueueCorrupt);
        }
        let count = used.length as usize;
        copy_bytes(
            &mut destination[offset..offset + count],
            &RANDOM_BUFFER[..count],
        );
        offset += count;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct DeviceCandidate {
    io_base: u16,
}

fn discover_device() -> Option<DeviceCandidate> {
    for device in 0..32 {
        for function in 0..8 {
            let identity = unsafe { pci_config_read32(0, device, function, 0) };
            let vendor = identity as u16;
            let device_id = (identity >> 16) as u16;
            if !is_rng_device(vendor, device_id) {
                continue;
            }
            let bar = unsafe { pci_config_read32(0, device, function, PCI_BAR0_OFFSET) };
            if bar & 1 == 0 {
                continue;
            }
            let io_base = (bar & 0xfffc) as u16;
            if io_base == 0 {
                continue;
            }
            let command = unsafe { pci_config_read16(0, device, function, PCI_COMMAND_OFFSET) };
            unsafe {
                pci_config_write16(0, device, function, PCI_COMMAND_OFFSET, command | 0x0005);
            }
            return Some(DeviceCandidate { io_base });
        }
    }
    None
}

const fn is_rng_device(vendor: u16, device_id: u16) -> bool {
    vendor == VIRTIO_VENDOR_ID
        && (device_id == VIRTIO_RNG_LEGACY_ID || device_id == VIRTIO_RNG_MODERN_ID)
}

const fn queue_address_is_usable(address: u64) -> bool {
    address & 0xfff == 0 && address >> 12 <= u32::MAX as u64
}

fn copy_bytes(destination: &mut [u8], source: &[u8]) {
    for (destination, source) in destination.iter_mut().zip(source.iter()) {
        unsafe { ptr::write_volatile(destination, ptr::read_volatile(source)) };
    }
}

const fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset & 0xfc) as u32
}

unsafe fn pci_config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    io_write32(
        PCI_CONFIG_ADDRESS,
        pci_config_address(bus, device, function, offset),
    );
    io_read32(PCI_CONFIG_DATA)
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

#[cfg(test)]
mod tests {
    use super::{
        is_rng_device, pci_config_address, queue_address_is_usable, LegacyQueue, QUEUE_SIZE,
        QUEUE_USED_RING_OFFSET, VIRTIO_VENDOR_ID,
    };

    #[test]
    fn recognizes_transitional_and_modern_entropy_device_ids() {
        assert!(is_rng_device(VIRTIO_VENDOR_ID, 0x1005));
        assert!(is_rng_device(VIRTIO_VENDOR_ID, 0x1044));
        assert!(!is_rng_device(VIRTIO_VENDOR_ID, 0x1003));
        assert!(!is_rng_device(0xffff, 0x1005));
    }

    #[test]
    fn encodes_pci_configuration_address() {
        assert_eq!(pci_config_address(0, 2, 0, 0x10), 0x8000_1010);
        assert_eq!(pci_config_address(3, 7, 1, 0x3c), 0x8003_393c);
    }

    #[test]
    fn requires_page_aligned_legacy_queue_below_4gib() {
        assert!(queue_address_is_usable(0x0020_0000));
        assert!(!queue_address_is_usable(0x0020_0001));
        assert!(!queue_address_is_usable((u64::from(u32::MAX) + 1) << 12));
    }

    #[test]
    fn uses_a_single_descriptor_entropy_queue() {
        assert_eq!(QUEUE_SIZE, 8);
        assert_eq!(core::mem::size_of::<LegacyQueue>() % 4096, 0);
        assert_eq!(QUEUE_USED_RING_OFFSET, 4096);
    }
}
