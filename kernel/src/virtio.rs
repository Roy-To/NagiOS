use core::arch::asm;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, AtomicBool, Ordering};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_BLOCK_LEGACY_ID: u16 = 0x1001;
const VIRTIO_BLOCK_MODERN_ID: u16 = 0x1042;
const PCI_COMMAND_OFFSET: u8 = 0x04;
const PCI_BAR0_OFFSET: u8 = 0x10;
const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
const LEGACY_QUEUE_SIZE: u16 = 0x0c;
const LEGACY_QUEUE_SELECT: u16 = 0x0e;
const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
const LEGACY_DEVICE_STATUS: u16 = 0x12;
const LEGACY_DEVICE_CONFIG: u16 = 0x14;
const QUEUE_SIZE: usize = 256;
const QUEUE_USED_RING_OFFSET: usize = 8192;
const QUEUE_AVAILABLE_END: usize = 4614;
const MAX_REQUEST_SPINS: usize = 5_000_000;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;
const BLOCK_IN: u32 = 0;
const BLOCK_OUT: u32 = 1;
const BLOCK_SECTOR_SIZE: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    NotInitialized,
    PciUnavailable,
    InvalidBar,
    UnsupportedQueue,
    AddressOutOfRange,
    DeviceFailure,
    RequestTimeout,
    QueueCorrupt,
    SectorOutOfRange,
    Busy,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BlockRequestHeader {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

#[derive(Clone, Copy)]
struct DeviceState {
    io_base: u16,
    capacity_sectors: u64,
    capability: u64,
}

static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
static mut QUEUE: LegacyQueue = LegacyQueue::empty();
static mut REQUEST_HEADER: BlockRequestHeader = BlockRequestHeader {
    request_type: BLOCK_IN,
    reserved: 0,
    sector: 0,
};
static mut REQUEST_DATA: [u8; BLOCK_SECTOR_SIZE] = [0; BLOCK_SECTOR_SIZE];
static mut REQUEST_STATUS: u8 = 0xff;
static mut DEVICE: Option<DeviceState> = None;

pub fn initialize() -> Result<(), BlockError> {
    if unsafe { ptr::addr_of!(DEVICE).read_volatile() }.is_some() {
        return Ok(());
    }
    let Some(candidate) = discover_largest_block_device() else {
        return Err(BlockError::PciUnavailable);
    };
    if candidate.io_base == 0 {
        return Err(BlockError::InvalidBar);
    }
    let queue_size = unsafe { io_read16(candidate.io_base + LEGACY_QUEUE_SIZE) } as usize;
    if queue_size < QUEUE_SIZE {
        return Err(BlockError::UnsupportedQueue);
    }
    let queue_address = ptr::addr_of!(QUEUE) as u64;
    if queue_address & 0xfff != 0 || queue_address >> 12 > u64::from(u32::MAX) {
        return Err(BlockError::AddressOutOfRange);
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
    let capability = make_capability(
        candidate.bus,
        candidate.device,
        candidate.function,
        candidate.capacity_sectors,
    );
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(DEVICE),
            Some(DeviceState {
                io_base: candidate.io_base,
                capacity_sectors: candidate.capacity_sectors,
                capability,
            }),
        );
    }
    Ok(())
}

pub fn user_capability() -> u64 {
    unsafe {
        ptr::addr_of!(DEVICE)
            .read_volatile()
            .map(|device| device.capability)
            .unwrap_or(0)
    }
}

pub fn capability_matches(capability: u64) -> bool {
    capability != 0 && capability == user_capability()
}

pub fn capacity_sectors() -> Option<u64> {
    unsafe {
        ptr::addr_of!(DEVICE)
            .read_volatile()
            .map(|device| device.capacity_sectors)
    }
}

pub fn read_sector(
    sector: u64,
    destination: &mut [u8; BLOCK_SECTOR_SIZE],
) -> Result<(), BlockError> {
    transfer(sector, destination, false)
}

pub fn write_sector(sector: u64, source: &[u8; BLOCK_SECTOR_SIZE]) -> Result<(), BlockError> {
    let mut buffer = [0; BLOCK_SECTOR_SIZE];
    copy_bytes(&mut buffer, source);
    transfer(sector, &mut buffer, true)
}

fn transfer(
    sector: u64,
    buffer: &mut [u8; BLOCK_SECTOR_SIZE],
    write_request: bool,
) -> Result<(), BlockError> {
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(BlockError::NotInitialized);
    };
    validate_sector(sector, device.capacity_sectors)?;
    if REQUEST_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(BlockError::Busy);
    }
    let result = unsafe { transfer_locked(device, sector, buffer, write_request) };
    REQUEST_LOCK.store(false, Ordering::Release);
    result
}

unsafe fn transfer_locked(
    device: DeviceState,
    sector: u64,
    buffer: &mut [u8; BLOCK_SECTOR_SIZE],
    write_request: bool,
) -> Result<(), BlockError> {
    ptr::write_volatile(
        ptr::addr_of_mut!(REQUEST_HEADER),
        request_header(if write_request { BLOCK_OUT } else { BLOCK_IN }, sector),
    );
    if write_request {
        copy_bytes(&mut *ptr::addr_of_mut!(REQUEST_DATA), &*buffer);
    }
    ptr::write_volatile(ptr::addr_of_mut!(REQUEST_STATUS), 0xff);

    let header_address = ptr::addr_of!(REQUEST_HEADER) as u64;
    let data_address = ptr::addr_of!(REQUEST_DATA) as u64;
    let status_address = ptr::addr_of!(REQUEST_STATUS) as u64;
    if header_address > u64::from(u32::MAX) << 12
        || data_address > u64::from(u32::MAX) << 12
        || status_address > u64::from(u32::MAX) << 12
    {
        return Err(BlockError::AddressOutOfRange);
    }

    let queue = &mut *ptr::addr_of_mut!(QUEUE);
    let previous_used = queue.used_index;
    queue.descriptors[0] = Descriptor {
        address: header_address,
        length: size_of::<BlockRequestHeader>() as u32,
        flags: DESC_F_NEXT,
        next: 1,
    };
    queue.descriptors[1] = Descriptor {
        address: data_address,
        length: BLOCK_SECTOR_SIZE as u32,
        flags: descriptor_flags(write_request, true),
        next: 2,
    };
    queue.descriptors[2] = Descriptor {
        address: status_address,
        length: 1,
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
            return Err(BlockError::RequestTimeout);
        }
        spins += 1;
        core::hint::spin_loop();
    }
    let used_slot = usize::from(previous_used) % QUEUE_SIZE;
    let used = ptr::read_volatile(queue.used_ring.as_ptr().add(used_slot));
    if used.id != 0 || used.length == 0 {
        return Err(BlockError::QueueCorrupt);
    }
    if ptr::read_volatile(ptr::addr_of!(REQUEST_STATUS)) != 0 {
        return Err(BlockError::DeviceFailure);
    }
    if !write_request {
        copy_bytes(&mut *buffer, &*ptr::addr_of!(REQUEST_DATA));
    }
    Ok(())
}

fn discover_largest_block_device() -> Option<DeviceCandidate> {
    let mut best = None;
    for device in 0..32 {
        for function in 0..8 {
            let identity = unsafe { pci_config_read32(0, device, function, 0) };
            let vendor = identity as u16;
            let device_id = (identity >> 16) as u16;
            if vendor != VIRTIO_VENDOR_ID
                || (device_id != VIRTIO_BLOCK_LEGACY_ID && device_id != VIRTIO_BLOCK_MODERN_ID)
            {
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
            let low = unsafe { io_read32(io_base + LEGACY_DEVICE_CONFIG) } as u64;
            let high = unsafe { io_read32(io_base + LEGACY_DEVICE_CONFIG + 4) } as u64;
            let capacity_sectors = low | high << 32;
            if capacity_sectors == 0 {
                continue;
            }
            let candidate = DeviceCandidate {
                bus: 0,
                device,
                function,
                io_base,
                capacity_sectors,
            };
            if best.is_none_or(|current: DeviceCandidate| {
                candidate.capacity_sectors > current.capacity_sectors
            }) {
                best = Some(candidate);
            }
        }
    }
    best
}

#[derive(Clone, Copy)]
struct DeviceCandidate {
    bus: u8,
    device: u8,
    function: u8,
    io_base: u16,
    capacity_sectors: u64,
}

const fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset & 0xfc) as u32
}

const fn request_header(request_type: u32, sector: u64) -> BlockRequestHeader {
    BlockRequestHeader {
        request_type,
        reserved: 0,
        sector,
    }
}

const fn descriptor_flags(write_request: bool, next: bool) -> u16 {
    let mut flags = 0;
    if !write_request {
        flags |= DESC_F_WRITE;
    }
    if next {
        flags |= DESC_F_NEXT;
    }
    flags
}

const fn make_capability(bus: u8, device: u8, function: u8, capacity: u64) -> u64 {
    let bdf = ((bus as u64) << 16) | ((device as u64) << 8) | function as u64;
    let capability = 0x4e41_4749_424c_4b01_u64 ^ bdf.rotate_left(17) ^ capacity.rotate_right(11);
    if capability == 0 {
        1
    } else {
        capability
    }
}

fn validate_sector(sector: u64, capacity: u64) -> Result<(), BlockError> {
    if sector < capacity {
        Ok(())
    } else {
        Err(BlockError::SectorOutOfRange)
    }
}

fn copy_bytes(destination: &mut [u8], source: &[u8]) {
    let count = source.len();
    let mut index = 0;
    while index < count {
        unsafe {
            ptr::write_volatile(
                destination.as_mut_ptr().add(index),
                ptr::read_volatile(source.as_ptr().add(index)),
            );
        }
        index += 1;
    }
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
        descriptor_flags, make_capability, pci_config_address, request_header, validate_sector,
        BlockRequestHeader, LegacyQueue, BLOCK_IN, DESC_F_NEXT, DESC_F_WRITE, QUEUE_SIZE,
        QUEUE_USED_RING_OFFSET,
    };

    #[test]
    fn encodes_pci_configuration_address() {
        assert_eq!(pci_config_address(0, 2, 0, 0x10), 0x8000_1010);
        assert_eq!(pci_config_address(3, 7, 1, 0x3c), 0x8003_393c);
    }

    #[test]
    fn builds_read_and_write_descriptor_flags() {
        assert_eq!(descriptor_flags(false, true), DESC_F_NEXT | DESC_F_WRITE);
        assert_eq!(descriptor_flags(true, true), DESC_F_NEXT);
        assert_eq!(descriptor_flags(false, false), DESC_F_WRITE);
    }

    #[test]
    fn encodes_sector_request_header() {
        assert_eq!(
            request_header(BLOCK_IN, 0x1234_5678_9abc_def0),
            BlockRequestHeader {
                request_type: BLOCK_IN,
                reserved: 0,
                sector: 0x1234_5678_9abc_def0,
            }
        );
    }

    #[test]
    fn derives_nonzero_device_capability() {
        assert_ne!(make_capability(0, 2, 0, 131_072), 0);
        assert_ne!(
            make_capability(0, 2, 0, 131_072),
            make_capability(0, 3, 0, 131_072)
        );
    }

    #[test]
    fn rejects_sector_at_capacity() {
        assert!(validate_sector(131_071, 131_072).is_ok());
        assert!(validate_sector(131_072, 131_072).is_err());
    }

    #[test]
    fn uses_the_legacy_queue_size_and_used_ring_alignment() {
        assert_eq!(QUEUE_SIZE, 256);
        assert_eq!(core::mem::size_of::<LegacyQueue>(), 12 * 1024);
        let queue = LegacyQueue::empty();
        let base = core::ptr::addr_of!(queue) as usize;
        let used = core::ptr::addr_of!(queue.used_flags) as usize;
        assert_eq!(used - base, QUEUE_USED_RING_OFFSET);
    }
}
