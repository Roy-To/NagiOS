use core::arch::asm;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, AtomicBool, Ordering};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const VIRTIO_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_NET_LEGACY_ID: u16 = 0x1000;
const PCI_COMMAND_OFFSET: u8 = 0x04;
const PCI_BAR0_OFFSET: u8 = 0x10;
const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
const LEGACY_QUEUE_SIZE: u16 = 0x0c;
const LEGACY_QUEUE_SELECT: u16 = 0x0e;
const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
const LEGACY_DEVICE_STATUS: u16 = 0x12;
const QUEUE_SIZE: usize = 256;
const QUEUE_USED_RING_OFFSET: usize = 8192;
const QUEUE_AVAILABLE_END: usize =
    size_of::<Descriptor>() * QUEUE_SIZE + 2 + 2 + 2 * QUEUE_SIZE + 2;
const MAX_FRAME_SIZE: usize = 1536;
const VIRTIO_NET_HEADER_SIZE: usize = 10;
const fn device_frame_capacity(frame_size: usize) -> Option<usize> {
    frame_size.checked_add(VIRTIO_NET_HEADER_SIZE)
}
const MAX_DEVICE_FRAME_SIZE: usize = match device_frame_capacity(MAX_FRAME_SIZE) {
    Some(size) => size,
    None => 0,
};
const MAX_REQUEST_SPINS: usize = 5_000_000;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const DESC_F_WRITE: u16 = 2;

fn guest_frame_payload_length(device_length: usize) -> Option<usize> {
    if !(VIRTIO_NET_HEADER_SIZE + 14..=MAX_DEVICE_FRAME_SIZE).contains(&device_length) {
        return None;
    }
    Some(device_length - VIRTIO_NET_HEADER_SIZE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetError {
    NotInitialized,
    PciUnavailable,
    InvalidBar,
    UnsupportedQueue,
    AddressOutOfRange,
    DeviceFailure,
    RequestTimeout,
    QueueCorrupt,
    FrameTooLarge,
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
#[derive(Clone, Copy)]
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
    capability: u64,
}

static RECEIVE_LOCK: AtomicBool = AtomicBool::new(false);
static TRANSMIT_LOCK: AtomicBool = AtomicBool::new(false);
static mut RECEIVE_QUEUE: LegacyQueue = LegacyQueue::empty();
static mut TRANSMIT_QUEUE: LegacyQueue = LegacyQueue::empty();
static mut RECEIVE_BUFFERS: [[u8; MAX_DEVICE_FRAME_SIZE]; QUEUE_SIZE] =
    [[0; MAX_DEVICE_FRAME_SIZE]; QUEUE_SIZE];
static mut TRANSMIT_BUFFER: [u8; MAX_DEVICE_FRAME_SIZE] = [0; MAX_DEVICE_FRAME_SIZE];
static mut RECEIVE_LAST_USED: u16 = 0;
static mut DEVICE: Option<DeviceState> = None;

pub fn initialize() -> Result<(), NetError> {
    if unsafe { ptr::addr_of!(DEVICE).read_volatile() }.is_some() {
        return Ok(());
    }
    let Some(candidate) = discover_net_device() else {
        return Err(NetError::PciUnavailable);
    };
    if candidate.io_base == 0 {
        return Err(NetError::InvalidBar);
    }
    let queue_size = unsafe { io_read16(candidate.io_base + LEGACY_QUEUE_SIZE) } as usize;
    if queue_size != QUEUE_SIZE {
        return Err(NetError::UnsupportedQueue);
    }
    let receive_address = ptr::addr_of!(RECEIVE_QUEUE) as u64;
    let transmit_address = ptr::addr_of!(TRANSMIT_QUEUE) as u64;
    if !queue_address_is_usable(receive_address) || !queue_address_is_usable(transmit_address) {
        return Err(NetError::AddressOutOfRange);
    }

    unsafe {
        io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, 0);
        io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
        io_write8(
            candidate.io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
        );
        io_write32(candidate.io_base + 0x04, 0);
        setup_queue(candidate.io_base, 0, receive_address)?;
        setup_queue(candidate.io_base, 1, transmit_address)?;
        prepare_receive_descriptors(candidate.io_base)?;
        io_write8(
            candidate.io_base + LEGACY_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );
    }

    let capability = make_capability(candidate.bus, candidate.device, candidate.function);
    unsafe {
        ptr::write_volatile(
            ptr::addr_of_mut!(DEVICE),
            Some(DeviceState {
                io_base: candidate.io_base,
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

pub fn transmit(frame: &[u8]) -> Result<usize, NetError> {
    if frame.is_empty() || frame.len() > MAX_FRAME_SIZE {
        return Err(NetError::FrameTooLarge);
    }
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(NetError::NotInitialized);
    };
    if TRANSMIT_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(NetError::Busy);
    }
    let result = unsafe { transmit_locked(device, frame) };
    TRANSMIT_LOCK.store(false, Ordering::Release);
    result.map(|()| frame.len())
}

pub fn receive(frame: &mut [u8]) -> Result<Option<usize>, NetError> {
    if frame.len() < MAX_FRAME_SIZE {
        return Err(NetError::FrameTooLarge);
    }
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(NetError::NotInitialized);
    };
    if RECEIVE_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(NetError::Busy);
    }
    let result = unsafe { receive_locked(device, frame) };
    RECEIVE_LOCK.store(false, Ordering::Release);
    result
}

unsafe fn setup_queue(io_base: u16, index: u16, address: u64) -> Result<(), NetError> {
    io_write16(io_base + LEGACY_QUEUE_SELECT, index);
    if (io_read16(io_base + LEGACY_QUEUE_SIZE) as usize) < QUEUE_SIZE {
        return Err(NetError::UnsupportedQueue);
    }
    io_write32(io_base + LEGACY_QUEUE_ADDRESS, (address >> 12) as u32);
    Ok(())
}

unsafe fn prepare_receive_descriptors(io_base: u16) -> Result<(), NetError> {
    let queue = &mut *ptr::addr_of_mut!(RECEIVE_QUEUE);
    ptr::write_bytes(
        ptr::addr_of_mut!(RECEIVE_QUEUE).cast::<u8>(),
        0,
        size_of::<LegacyQueue>(),
    );
    let buffers = ptr::addr_of_mut!(RECEIVE_BUFFERS);
    for index in 0..QUEUE_SIZE {
        let address = buffers.cast::<u8>().add(index * MAX_FRAME_SIZE) as u64;
        if !dma_address_is_usable(address) {
            return Err(NetError::AddressOutOfRange);
        }
        queue.descriptors[index] = Descriptor {
            address,
            length: MAX_DEVICE_FRAME_SIZE as u32,
            flags: DESC_F_WRITE,
            next: 0,
        };
        queue.available_ring[index] = index as u16;
    }
    fence(Ordering::SeqCst);
    queue.available_index = QUEUE_SIZE as u16;
    ptr::write_volatile(ptr::addr_of_mut!(RECEIVE_LAST_USED), 0);
    fence(Ordering::SeqCst);
    io_write16(io_base + LEGACY_QUEUE_NOTIFY, 0);
    Ok(())
}

unsafe fn transmit_locked(device: DeviceState, frame: &[u8]) -> Result<(), NetError> {
    let buffer = ptr::addr_of_mut!(TRANSMIT_BUFFER).cast::<u8>();
    let mut header_index = 0;
    while header_index < VIRTIO_NET_HEADER_SIZE {
        ptr::write_volatile(buffer.add(header_index), 0);
        header_index += 1;
    }
    for (index, byte) in frame.iter().enumerate() {
        ptr::write_volatile(buffer.add(VIRTIO_NET_HEADER_SIZE + index), *byte);
    }
    let queue = &mut *ptr::addr_of_mut!(TRANSMIT_QUEUE);
    let previous_used = queue.used_index;
    queue.descriptors[0] = Descriptor {
        address: buffer as u64,
        length: (VIRTIO_NET_HEADER_SIZE + frame.len()) as u32,
        flags: 0,
        next: 0,
    };
    let available_slot = usize::from(queue.available_index) % QUEUE_SIZE;
    queue.available_ring[available_slot] = 0;
    fence(Ordering::SeqCst);
    queue.available_index = queue.available_index.wrapping_add(1);
    fence(Ordering::SeqCst);
    io_write16(device.io_base + LEGACY_QUEUE_NOTIFY, 1);
    wait_for_used(queue, previous_used)
}

unsafe fn receive_locked(device: DeviceState, frame: &mut [u8]) -> Result<Option<usize>, NetError> {
    let queue = &mut *ptr::addr_of_mut!(RECEIVE_QUEUE);
    let current_used = ptr::read_volatile(&queue.used_index);
    let previous_used = ptr::read_volatile(ptr::addr_of!(RECEIVE_LAST_USED));
    if current_used == previous_used {
        return Ok(None);
    }
    let used_slot = usize::from(previous_used) % QUEUE_SIZE;
    let used = ptr::read_volatile(queue.used_ring.as_ptr().add(used_slot));
    let buffer_id = usize::try_from(used.id).map_err(|_| NetError::QueueCorrupt)?;
    let length = usize::try_from(used.length).map_err(|_| NetError::QueueCorrupt)?;
    let Some(payload_length) = guest_frame_payload_length(length) else {
        return Err(NetError::QueueCorrupt);
    };
    if buffer_id >= QUEUE_SIZE {
        return Err(NetError::QueueCorrupt);
    }
    let source = ptr::addr_of!(RECEIVE_BUFFERS)
        .cast::<u8>()
        .add(buffer_id * MAX_FRAME_SIZE);
    let mut index = 0;
    while index < payload_length {
        frame[index] = ptr::read_volatile(source.add(VIRTIO_NET_HEADER_SIZE + index));
        index += 1;
    }
    ptr::write_volatile(
        ptr::addr_of_mut!(RECEIVE_LAST_USED),
        previous_used.wrapping_add(1),
    );
    queue.available_ring[usize::from(queue.available_index) % QUEUE_SIZE] = buffer_id as u16;
    queue.available_index = queue.available_index.wrapping_add(1);
    fence(Ordering::SeqCst);
    io_write16(device.io_base + LEGACY_QUEUE_NOTIFY, 0);
    Ok(Some(payload_length))
}

unsafe fn wait_for_used(queue: &mut LegacyQueue, previous_used: u16) -> Result<(), NetError> {
    let mut spins = 0;
    while ptr::read_volatile(&queue.used_index) == previous_used {
        if spins == MAX_REQUEST_SPINS {
            return Err(NetError::RequestTimeout);
        }
        spins += 1;
        core::hint::spin_loop();
    }
    let used_slot = usize::from(previous_used) % QUEUE_SIZE;
    let used = ptr::read_volatile(queue.used_ring.as_ptr().add(used_slot));
    if used.id != 0 {
        return Err(NetError::QueueCorrupt);
    }
    Ok(())
}

fn discover_net_device() -> Option<DeviceCandidate> {
    for device in 0..32 {
        for function in 0..8 {
            let identity = unsafe { pci_config_read32(0, device, function, 0) };
            if identity as u16 != VIRTIO_VENDOR_ID
                || (identity >> 16) as u16 != VIRTIO_NET_LEGACY_ID
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
            return Some(DeviceCandidate {
                bus: 0,
                device,
                function,
                io_base,
            });
        }
    }
    None
}

#[derive(Clone, Copy)]
struct DeviceCandidate {
    bus: u8,
    device: u8,
    function: u8,
    io_base: u16,
}

const fn queue_address_is_usable(address: u64) -> bool {
    address & 0xfff == 0 && dma_address_is_usable(address)
}

const fn dma_address_is_usable(address: u64) -> bool {
    address >> 12 <= u32::MAX as u64
}

const fn make_capability(bus: u8, device: u8, function: u8) -> u64 {
    let bdf = ((bus as u64) << 16) | ((device as u64) << 8) | function as u64;
    let capability = 0x4e41_4749_4e45_5401_u64 ^ bdf.rotate_left(17);
    if capability == 0 {
        1
    } else {
        capability
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
        device_frame_capacity, guest_frame_payload_length, make_capability, pci_config_address,
        queue_address_is_usable, MAX_FRAME_SIZE, QUEUE_SIZE, QUEUE_USED_RING_OFFSET,
    };

    #[test]
    fn network_capability_is_nonzero_and_bdf_bound() {
        assert_ne!(make_capability(0, 3, 0), 0);
        assert_ne!(make_capability(0, 3, 0), make_capability(0, 4, 0));
    }

    #[test]
    fn queue_contract_is_bounded() {
        assert_eq!(QUEUE_SIZE, 256);
        assert_eq!(QUEUE_USED_RING_OFFSET, 8192);
        assert!(queue_address_is_usable(0x1000));
        assert!(!queue_address_is_usable(0x1001));
        assert!(!queue_address_is_usable((u64::from(u32::MAX) + 1) << 12));
    }

    #[test]
    fn encodes_pci_configuration_address() {
        assert_eq!(pci_config_address(0, 3, 0, 0x10), 0x8000_1810);
    }

    #[test]
    fn legacy_net_frame_capacity_reserves_the_device_header() {
        assert_eq!(
            device_frame_capacity(MAX_FRAME_SIZE),
            Some(MAX_FRAME_SIZE + 10)
        );
    }

    #[test]
    fn receive_length_excludes_the_virtio_net_header() {
        assert_eq!(guest_frame_payload_length(74), Some(64));
    }
}
