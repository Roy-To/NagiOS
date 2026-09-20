use core::arch::asm;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, AtomicBool, Ordering};

const PCI_CONFIG_ADDRESS: u16 = 0x0cf8;
const PCI_CONFIG_DATA: u16 = 0x0cfc;
const VIRTIO_VENDOR_ID: u16 = 0x1af4;
pub const AUDIO_DEVICE_ID: u16 = 0x1019;
const AUDIO_MODERN_DEVICE_ID: u16 = 0x1059;
const PCI_COMMAND_OFFSET: u8 = 0x04;
const PCI_BAR0_OFFSET: u8 = 0x10;
const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
const LEGACY_QUEUE_SIZE: u16 = 0x0c;
const LEGACY_QUEUE_SELECT: u16 = 0x0e;
const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
const LEGACY_DEVICE_STATUS: u16 = 0x12;
const LEGACY_DEVICE_CONFIG: u16 = 0x14;
const QUEUE_COUNT: usize = 4;
const QUEUE_SIZE: usize = 64;
const QUEUE_USED_RING_OFFSET: usize = 2048;
const QUEUE_AVAILABLE_END: usize = 6 + QUEUE_SIZE * 2;
const MAX_REQUEST_SPINS: usize = 5_000_000;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_FEATURES_OK: u8 = 8;
const STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_F_VERSION_1_HIGH: u32 = 1;
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;
const PCM_OUTPUT_STREAM: u32 = 0;
const PCM_INPUT_STREAM: u32 = 1;
const PCM_BUFFER_BYTES: usize = 16 * 1024;
const PCM_PERIOD_BYTES: usize = 4096;
const PCM_FORMAT_S16: u8 = 5;
const PCM_RATE_48000: u8 = 7;
const PCM_DIRECTION_OUTPUT: u8 = 0;
const PCM_DIRECTION_INPUT: u8 = 1;
const PCM_R_INFO: u32 = 0x0100;
const PCM_R_SET_PARAMS: u32 = 0x0101;
const PCM_R_PREPARE: u32 = 0x0102;
const PCM_R_START: u32 = 0x0104;
const SOUND_STATUS_OK: u32 = 0x8000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoundError {
    NotInitialized,
    PciUnavailable,
    InvalidBar,
    UnsupportedQueue,
    AddressOutOfRange,
    DeviceFailure,
    RequestTimeout,
    QueueCorrupt,
    InvalidStream,
    InvalidBuffer,
    UnsupportedFormat,
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
#[derive(Clone, Copy)]
struct SoundHdr {
    code: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct QueryInfo {
    hdr: SoundHdr,
    start_id: u32,
    count: u32,
    size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PcmInfo {
    hda_fn_nid: u32,
    features: u32,
    formats: u64,
    rates: u64,
    direction: u8,
    channels_min: u8,
    channels_max: u8,
    padding: [u8; 5],
}

impl PcmInfo {
    const EMPTY: Self = Self {
        hda_fn_nid: 0,
        features: 0,
        formats: 0,
        rates: 0,
        direction: 0,
        channels_min: 0,
        channels_max: 0,
        padding: [0; 5],
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PcmHdr {
    hdr: SoundHdr,
    stream_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SetParams {
    hdr: PcmHdr,
    buffer_bytes: u32,
    period_bytes: u32,
    features: u32,
    channels: u8,
    format: u8,
    rate: u8,
    padding: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PcmXfer {
    stream_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PcmStatus {
    status: u32,
    latency_bytes: u32,
}

#[derive(Clone, Copy)]
struct SoundDevice {
    io_base: u16,
    modern: bool,
    notify_addresses: [u64; QUEUE_COUNT],
    streams: u32,
    capability: u64,
}

#[derive(Clone, Copy)]
struct ModernConfig {
    common_address: u64,
    notify_address: u64,
    notify_multiplier: u32,
    device_config: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoundPcmFormat {
    pub channels: u8,
    pub format: u8,
    pub rate: u8,
}

static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
static mut QUEUES: [LegacyQueue; QUEUE_COUNT] = [const { LegacyQueue::empty() }; QUEUE_COUNT];
static mut CONTROL_REQUEST: [u8; 64] = [0; 64];
static mut CONTROL_RESPONSE: [u8; 256] = [0; 256];
static mut EVENT_BUFFER: [u8; 8] = [0; 8];
static mut PCM_XFER: PcmXfer = PcmXfer { stream_id: 0 };
static mut PCM_BUFFER: [u8; PCM_PERIOD_BYTES] = [0; PCM_PERIOD_BYTES];
static mut PCM_STATUS: PcmStatus = PcmStatus {
    status: 0,
    latency_bytes: 0,
};
static mut DEVICE: Option<SoundDevice> = None;
static mut STREAM_INFO: [PcmInfo; 2] = [PcmInfo::EMPTY; 2];
static mut STREAM_STARTED: [bool; 2] = [false; 2];

pub fn initialize() -> Result<(), SoundError> {
    if unsafe { ptr::addr_of!(DEVICE).read_volatile() }.is_some() {
        return Ok(());
    }
    let Some(candidate) = discover_sound_device() else {
        return Err(SoundError::PciUnavailable);
    };
    if !candidate.modern && candidate.io_base == 0 {
        return Err(SoundError::InvalidBar);
    }
    let modern_config = candidate.modern_config;
    let streams = unsafe {
        if candidate.modern {
            let config = modern_config.ok_or(SoundError::PciUnavailable)?;
            mmio_read32(config.device_config, 4)
        } else {
            io_read32(candidate.io_base + LEGACY_DEVICE_CONFIG + 4)
        }
    };
    if streams < 2 {
        return Err(SoundError::InvalidStream);
    }

    let notify_addresses = unsafe {
        if candidate.modern {
            let config = modern_config.ok_or(SoundError::PciUnavailable)?;
            negotiate_modern_features(config)?;
            let addresses = setup_modern_queues(config)?;
            populate_event_queue()?;
            mmio_write8(
                config.common_address,
                20,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            );
            addresses
        } else {
            let queue_size = io_read16(candidate.io_base + LEGACY_QUEUE_SIZE) as usize;
            if queue_size < QUEUE_SIZE {
                return Err(SoundError::UnsupportedQueue);
            }
            io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, 0);
            io_write8(candidate.io_base + LEGACY_DEVICE_STATUS, STATUS_ACKNOWLEDGE);
            io_write8(
                candidate.io_base + LEGACY_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER,
            );
            for queue in 0..QUEUE_COUNT {
                setup_queue(candidate.io_base, queue)?;
            }
            populate_event_queue()?;
            io_write8(
                candidate.io_base + LEGACY_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
            );
            [0; QUEUE_COUNT]
        }
    };

    let capability = make_capability(candidate.bus, candidate.device, candidate.function, streams);
    let device = SoundDevice {
        io_base: candidate.io_base,
        modern: candidate.modern,
        notify_addresses,
        streams,
        capability,
    };
    unsafe { notify_queue(device, 1) };
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(DEVICE), Some(device));
    }
    for stream_id in 0..2 {
        let info = query_stream_info(device, stream_id as u32)?;
        if (info.direction == PCM_DIRECTION_OUTPUT && stream_id != 0)
            || (info.direction == PCM_DIRECTION_INPUT && stream_id != 1)
        {
            return Err(SoundError::InvalidStream);
        }
        unsafe { ptr::write_volatile(ptr::addr_of_mut!(STREAM_INFO[stream_id]), info) };
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

pub fn stream_count() -> u32 {
    unsafe {
        ptr::addr_of!(DEVICE)
            .read_volatile()
            .map(|device| device.streams)
            .unwrap_or(0)
    }
}

pub fn play(capability: u64, stream_id: u32, data: &[u8]) -> Result<(), SoundError> {
    if !capability_matches(capability) || stream_id != PCM_OUTPUT_STREAM {
        return Err(SoundError::InvalidStream);
    }
    if data.is_empty() || data.len() > PCM_PERIOD_BYTES || !data.len().is_multiple_of(4) {
        return Err(SoundError::InvalidBuffer);
    }
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(SoundError::NotInitialized);
    };
    with_request_lock(|| {
        ensure_stream(device, stream_id)?;
        unsafe {
            let buffer = core::slice::from_raw_parts_mut(
                ptr::addr_of_mut!(PCM_BUFFER).cast::<u8>(),
                PCM_PERIOD_BYTES,
            );
            copy_bytes(buffer, data);
        };
        pcm_transfer(device, stream_id, data.len(), false).map(|_| ())
    })
}

pub fn capture(
    capability: u64,
    stream_id: u32,
    destination: &mut [u8],
) -> Result<usize, SoundError> {
    if !capability_matches(capability) || stream_id != PCM_INPUT_STREAM {
        return Err(SoundError::InvalidStream);
    }
    if destination.is_empty()
        || destination.len() > PCM_PERIOD_BYTES
        || !destination.len().is_multiple_of(4)
    {
        return Err(SoundError::InvalidBuffer);
    }
    let Some(device) = (unsafe { ptr::addr_of!(DEVICE).read_volatile() }) else {
        return Err(SoundError::NotInitialized);
    };
    with_request_lock(|| {
        ensure_stream(device, stream_id)?;
        let bytes = pcm_transfer(device, stream_id, destination.len(), true)?;
        unsafe {
            let buffer = core::slice::from_raw_parts(ptr::addr_of!(PCM_BUFFER).cast::<u8>(), bytes);
            copy_bytes(destination, buffer);
        };
        Ok(bytes)
    })
}

fn with_request_lock<T>(
    operation: impl FnOnce() -> Result<T, SoundError>,
) -> Result<T, SoundError> {
    if REQUEST_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(SoundError::Busy);
    }
    let result = operation();
    REQUEST_LOCK.store(false, Ordering::Release);
    result
}

fn ensure_stream(device: SoundDevice, stream_id: u32) -> Result<(), SoundError> {
    let index = usize::try_from(stream_id).map_err(|_| SoundError::InvalidStream)?;
    if index >= 2 {
        return Err(SoundError::InvalidStream);
    }
    if unsafe { ptr::addr_of!(STREAM_STARTED[index]).read_volatile() } {
        return Ok(());
    }
    let info = unsafe { ptr::addr_of!(STREAM_INFO[index]).read_volatile() };
    let format = SoundPcmFormat {
        channels: 2,
        format: PCM_FORMAT_S16,
        rate: PCM_RATE_48000,
    };
    if !stream_is_supported(
        format,
        info.channels_min,
        info.channels_max,
        info.formats,
        info.rates,
    ) {
        return Err(SoundError::UnsupportedFormat);
    }
    let params = SetParams {
        hdr: PcmHdr {
            hdr: SoundHdr {
                code: PCM_R_SET_PARAMS,
            },
            stream_id,
        },
        buffer_bytes: PCM_BUFFER_BYTES as u32,
        period_bytes: PCM_PERIOD_BYTES as u32,
        features: 0,
        channels: format.channels,
        format: format.format,
        rate: format.rate,
        padding: 0,
    };
    control_request(device, &params, size_of::<SetParams>())?;
    let header = PcmHdr {
        hdr: SoundHdr {
            code: PCM_R_PREPARE,
        },
        stream_id,
    };
    control_request(device, &header, size_of::<PcmHdr>())?;
    let header = PcmHdr {
        hdr: SoundHdr { code: PCM_R_START },
        stream_id,
    };
    control_request(device, &header, size_of::<PcmHdr>())?;
    unsafe { ptr::write_volatile(ptr::addr_of_mut!(STREAM_STARTED[index]), true) };
    Ok(())
}

fn query_stream_info(device: SoundDevice, stream_id: u32) -> Result<PcmInfo, SoundError> {
    let request = QueryInfo {
        hdr: SoundHdr { code: PCM_R_INFO },
        start_id: stream_id,
        count: 1,
        size: size_of::<PcmInfo>() as u32,
    };
    control_request_with_response(
        device,
        &request,
        size_of::<QueryInfo>(),
        size_of::<PcmInfo>(),
    )?;
    let response = unsafe { ptr::addr_of!(CONTROL_RESPONSE).cast::<u8>().add(4) };
    Ok(unsafe { ptr::read_unaligned(response.cast::<PcmInfo>()) })
}

fn control_request<T: Copy>(
    device: SoundDevice,
    request: &T,
    request_len: usize,
) -> Result<(), SoundError> {
    control_request_with_response(device, request, request_len, 0)
}

fn control_request_with_response<T: Copy>(
    device: SoundDevice,
    request: &T,
    request_len: usize,
    response_payload_len: usize,
) -> Result<(), SoundError> {
    let response_len = 4 + response_payload_len;
    if request_len > 64 || response_len > 256 {
        return Err(SoundError::InvalidBuffer);
    }
    unsafe {
        ptr::copy_nonoverlapping(
            (request as *const T).cast::<u8>(),
            ptr::addr_of_mut!(CONTROL_REQUEST).cast::<u8>(),
            request_len,
        );
        ptr::write_bytes(
            ptr::addr_of_mut!(CONTROL_RESPONSE).cast::<u8>(),
            0xff,
            response_len,
        );
        let request_address = ptr::addr_of!(CONTROL_REQUEST) as u64;
        let response_address = ptr::addr_of!(CONTROL_RESPONSE) as u64;
        submit_chain(
            device,
            0,
            [
                Descriptor {
                    address: request_address,
                    length: request_len as u32,
                    flags: DESC_F_NEXT,
                    next: 1,
                },
                Descriptor {
                    address: response_address,
                    length: response_len as u32,
                    flags: DESC_F_WRITE,
                    next: 0,
                },
                Descriptor {
                    address: 0,
                    length: 0,
                    flags: 0,
                    next: 0,
                },
            ],
            2,
        )?;
        let status = ptr::read_unaligned(ptr::addr_of!(CONTROL_RESPONSE).cast::<u32>());
        if status != SOUND_STATUS_OK {
            return Err(SoundError::DeviceFailure);
        }
    }
    Ok(())
}

fn pcm_transfer(
    device: SoundDevice,
    stream_id: u32,
    data_len: usize,
    capture: bool,
) -> Result<usize, SoundError> {
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(PCM_XFER), PcmXfer { stream_id });
        ptr::write_volatile(
            ptr::addr_of_mut!(PCM_STATUS),
            PcmStatus {
                status: 0,
                latency_bytes: 0,
            },
        );
        let xfer_address = ptr::addr_of!(PCM_XFER) as u64;
        let data_address = ptr::addr_of!(PCM_BUFFER) as u64;
        let status_address = ptr::addr_of!(PCM_STATUS) as u64;
        let data_flags = DESC_F_NEXT | if capture { DESC_F_WRITE } else { 0 };
        let queue = if capture { 3 } else { 2 };
        let used_length = submit_chain(
            device,
            queue,
            [
                Descriptor {
                    address: xfer_address,
                    length: size_of::<PcmXfer>() as u32,
                    flags: DESC_F_NEXT,
                    next: 1,
                },
                Descriptor {
                    address: data_address,
                    length: data_len as u32,
                    flags: data_flags,
                    next: 2,
                },
                Descriptor {
                    address: status_address,
                    length: size_of::<PcmStatus>() as u32,
                    flags: DESC_F_WRITE,
                    next: 0,
                },
            ],
            3,
        )?;
        let status = ptr::read_volatile(ptr::addr_of!(PCM_STATUS));
        if status.status != SOUND_STATUS_OK {
            return Err(SoundError::DeviceFailure);
        }
        if capture {
            let written = used_length.saturating_sub(size_of::<PcmStatus>() as u32) as usize;
            Ok(written.min(data_len))
        } else {
            Ok(data_len)
        }
    }
}

unsafe fn submit_chain(
    device: SoundDevice,
    queue_index: usize,
    descriptors: [Descriptor; 3],
    descriptor_count: usize,
) -> Result<u32, SoundError> {
    if queue_index >= QUEUE_COUNT || !(1..=3).contains(&descriptor_count) {
        return Err(SoundError::InvalidStream);
    }
    let queue = &mut *ptr::addr_of_mut!(QUEUES[queue_index]);
    for (index, descriptor) in descriptors.iter().take(descriptor_count).enumerate() {
        queue.descriptors[index] = *descriptor;
    }
    let previous_used = ptr::read_volatile(&queue.used_index);
    let available_slot = usize::from(queue.available_index) % QUEUE_SIZE;
    queue.available_ring[available_slot] = 0;
    fence(Ordering::SeqCst);
    queue.available_index = queue.available_index.wrapping_add(1);
    fence(Ordering::SeqCst);
    notify_queue(device, queue_index);
    let mut spins = 0;
    while ptr::read_volatile(&queue.used_index) == previous_used {
        if spins == MAX_REQUEST_SPINS {
            return Err(SoundError::RequestTimeout);
        }
        spins += 1;
        core::hint::spin_loop();
    }
    let used_slot = usize::from(previous_used) % QUEUE_SIZE;
    let used = ptr::read_volatile(queue.used_ring.as_ptr().add(used_slot));
    if used.id != 0 {
        return Err(SoundError::QueueCorrupt);
    }
    Ok(used.length)
}

unsafe fn notify_queue(device: SoundDevice, queue_index: usize) {
    if device.modern {
        mmio_write16(device.notify_addresses[queue_index], 0, 0);
    } else {
        io_write16(device.io_base + LEGACY_QUEUE_NOTIFY, queue_index as u16);
    }
}

unsafe fn setup_queue(io_base: u16, queue_index: usize) -> Result<(), SoundError> {
    if queue_index >= QUEUE_COUNT {
        return Err(SoundError::InvalidStream);
    }
    io_write16(io_base + LEGACY_QUEUE_SELECT, queue_index as u16);
    let queue_size = io_read16(io_base + LEGACY_QUEUE_SIZE) as usize;
    if queue_size < QUEUE_SIZE {
        return Err(SoundError::UnsupportedQueue);
    }
    let queue_address = ptr::addr_of!(QUEUES[queue_index]) as u64;
    if queue_address & 0xfff != 0 || queue_address >> 12 > u64::from(u32::MAX) {
        return Err(SoundError::AddressOutOfRange);
    }
    ptr::write_bytes(
        ptr::addr_of_mut!(QUEUES[queue_index]).cast::<u8>(),
        0,
        size_of::<LegacyQueue>(),
    );
    io_write32(io_base + LEGACY_QUEUE_ADDRESS, (queue_address >> 12) as u32);
    Ok(())
}

unsafe fn setup_modern_queues(config: ModernConfig) -> Result<[u64; QUEUE_COUNT], SoundError> {
    let common = config.common_address;
    let mut notify_addresses = [0_u64; QUEUE_COUNT];
    for queue_index in 0..QUEUE_COUNT {
        mmio_write16(common, 22, queue_index as u16);
        if usize::from(mmio_read16(common, 24)) < QUEUE_SIZE {
            return Err(SoundError::UnsupportedQueue);
        }
        mmio_write16(common, 26, QUEUE_SIZE as u16);
        ptr::write_bytes(
            ptr::addr_of_mut!(QUEUES[queue_index]).cast::<u8>(),
            0,
            size_of::<LegacyQueue>(),
        );
        let descriptor_address = ptr::addr_of!(QUEUES[queue_index].descriptors) as u64;
        let driver_address = ptr::addr_of!(QUEUES[queue_index].available_flags) as u64;
        let device_address = ptr::addr_of!(QUEUES[queue_index].used_flags) as u64;
        if descriptor_address & 0xfff != 0 || driver_address & 1 != 0 || device_address & 1 != 0 {
            return Err(SoundError::AddressOutOfRange);
        }
        mmio_write64(common, 32, descriptor_address);
        mmio_write64(common, 40, driver_address);
        mmio_write64(common, 48, device_address);
        let notify_offset = u64::from(mmio_read16(common, 30));
        notify_addresses[queue_index] = config
            .notify_address
            .saturating_add(notify_offset.saturating_mul(u64::from(config.notify_multiplier)));
        mmio_write16(common, 28, 1);
    }
    Ok(notify_addresses)
}

unsafe fn negotiate_modern_features(config: ModernConfig) -> Result<(), SoundError> {
    let common = config.common_address;
    mmio_write8(common, 20, 0);
    mmio_write8(common, 20, STATUS_ACKNOWLEDGE);
    mmio_write8(common, 20, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
    mmio_write32(common, 0, 0);
    let _device_features_low = mmio_read32(common, 4);
    mmio_write32(common, 0, 1);
    let device_features_high = mmio_read32(common, 4);
    if device_features_high & VIRTIO_F_VERSION_1_HIGH == 0 {
        return Err(SoundError::DeviceFailure);
    }
    mmio_write32(common, 8, 0);
    mmio_write32(common, 12, 0);
    mmio_write32(common, 8, 1);
    mmio_write32(common, 12, VIRTIO_F_VERSION_1_HIGH);
    mmio_write8(
        common,
        20,
        STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
    );
    if mmio_read8(common, 20) & STATUS_FEATURES_OK == 0 {
        return Err(SoundError::DeviceFailure);
    }
    Ok(())
}

unsafe fn populate_event_queue() -> Result<(), SoundError> {
    let event_address = ptr::addr_of!(EVENT_BUFFER) as u64;
    if event_address >> 12 > u64::from(u32::MAX) {
        return Err(SoundError::AddressOutOfRange);
    }
    let queue = &mut *ptr::addr_of_mut!(QUEUES[1]);
    queue.descriptors[0] = Descriptor {
        address: event_address,
        length: 8,
        flags: DESC_F_WRITE,
        next: 0,
    };
    let available_slot = usize::from(queue.available_index) % QUEUE_SIZE;
    queue.available_ring[available_slot] = 0;
    fence(Ordering::SeqCst);
    queue.available_index = queue.available_index.wrapping_add(1);
    fence(Ordering::SeqCst);
    Ok(())
}

fn discover_sound_device() -> Option<DeviceCandidate> {
    for device in 0..32 {
        for function in 0..8 {
            let identity = unsafe { pci_config_read32(0, device, function, 0) };
            if identity as u16 != VIRTIO_VENDOR_ID {
                continue;
            }
            let device_id = (identity >> 16) as u16;
            let (modern, io_base, modern_config) = if device_id == AUDIO_DEVICE_ID {
                let bar = unsafe { pci_config_read32(0, device, function, PCI_BAR0_OFFSET) };
                if bar & 1 == 0 {
                    continue;
                }
                let io_base = (bar & 0xfffc) as u16;
                if io_base == 0 {
                    continue;
                }
                (false, io_base, None)
            } else if device_id == AUDIO_MODERN_DEVICE_ID {
                let Ok(config) = (unsafe { find_modern_config(device, function) }) else {
                    continue;
                };
                (true, 0, Some(config))
            } else {
                continue;
            };
            let command = unsafe { pci_config_read16(0, device, function, PCI_COMMAND_OFFSET) };
            unsafe {
                pci_config_write16(0, device, function, PCI_COMMAND_OFFSET, command | 0x0005);
            }
            return Some(DeviceCandidate {
                bus: 0,
                device,
                function,
                io_base,
                modern,
                modern_config,
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
    modern: bool,
    modern_config: Option<ModernConfig>,
}

unsafe fn find_modern_config(device: u8, function: u8) -> Result<ModernConfig, SoundError> {
    const PCI_CAPABILITY_VENDOR_SPECIFIC: u8 = 0x09;
    const VIRTIO_PCI_COMMON_CFG: u8 = 1;
    const VIRTIO_PCI_NOTIFY_CFG: u8 = 2;
    const VIRTIO_PCI_DEVICE_CFG: u8 = 4;
    let mut common = None;
    let mut notify = None;
    let mut device_config = None;
    let mut multiplier = 0;
    let mut capability = pci_config_read8(0, device, function, 0x34);
    if capability < 0x40 {
        return Err(SoundError::DeviceFailure);
    }
    let mut steps = 0;
    while capability >= 0x40 && steps < 48 {
        if pci_config_read8(0, device, function, capability) == PCI_CAPABILITY_VENDOR_SPECIFIC {
            let cfg_type = pci_config_read8(0, device, function, capability + 3);
            let bar = pci_config_read8(0, device, function, capability + 4);
            let offset = pci_config_read32(0, device, function, capability + 8);
            let length = pci_config_read32(0, device, function, capability + 12);
            if length != 0 {
                let Some(base) = read_bar_base(device, function, bar) else {
                    return Err(SoundError::InvalidBar);
                };
                let Some(address) = base.checked_add(u64::from(offset)) else {
                    return Err(SoundError::AddressOutOfRange);
                };
                match cfg_type {
                    VIRTIO_PCI_COMMON_CFG => common = Some(address),
                    VIRTIO_PCI_NOTIFY_CFG => {
                        notify = Some(address);
                        multiplier = pci_config_read32(0, device, function, capability + 16);
                    }
                    VIRTIO_PCI_DEVICE_CFG => device_config = Some(address),
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
    Ok(ModernConfig {
        common_address: common.ok_or(SoundError::DeviceFailure)?,
        notify_address: notify.ok_or(SoundError::DeviceFailure)?,
        notify_multiplier: multiplier,
        device_config: device_config.ok_or(SoundError::DeviceFailure)?,
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
    let mut base = u64::from(low & 0xffff_fff0);
    if memory_type == 0x2 {
        let high = pci_config_read32(0, device, function, offset + 4);
        base |= u64::from(high) << 32;
    }
    Some(base)
}

pub fn stream_is_supported(
    format: SoundPcmFormat,
    channels_min: u8,
    channels_max: u8,
    formats: u64,
    rates: u64,
) -> bool {
    format.channels >= channels_min
        && format.channels <= channels_max
        && formats & (1_u64 << format.format) != 0
        && rates & (1_u64 << format.rate) != 0
}

pub const fn make_capability(bus: u8, device: u8, function: u8, streams: u32) -> u64 {
    let bdf = ((bus as u64) << 16) | ((device as u64) << 8) | function as u64;
    let capability =
        0x4e41_4749_534e_4401_u64 ^ bdf.rotate_left(17) ^ (streams as u64).rotate_right(11);
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

unsafe fn mmio_read16(address: u64, offset: u64) -> u16 {
    ptr::read_volatile((address + offset) as *const u16)
}

unsafe fn mmio_read8(address: u64, offset: u64) -> u8 {
    ptr::read_volatile((address + offset) as *const u8)
}

unsafe fn mmio_read32(address: u64, offset: u64) -> u32 {
    ptr::read_volatile((address + offset) as *const u32)
}

unsafe fn mmio_write8(address: u64, offset: u64, value: u8) {
    ptr::write_volatile((address + offset) as *mut u8, value);
}

unsafe fn mmio_write16(address: u64, offset: u64, value: u16) {
    ptr::write_volatile((address + offset) as *mut u16, value);
}

unsafe fn mmio_write32(address: u64, offset: u64, value: u32) {
    ptr::write_volatile((address + offset) as *mut u32, value);
}

unsafe fn mmio_write64(address: u64, offset: u64, value: u64) {
    ptr::write_volatile((address + offset) as *mut u64, value);
}

unsafe fn copy_bytes(destination: &mut [u8], source: &[u8]) {
    for (destination, source) in destination.iter_mut().zip(source.iter()) {
        ptr::write_volatile(destination, ptr::read_volatile(source));
    }
}

#[cfg(test)]
mod tests {
    use super::{make_capability, stream_is_supported, SoundPcmFormat, AUDIO_DEVICE_ID};

    #[test]
    fn recognizes_the_virtio_sound_device_and_derives_a_capability() {
        assert_eq!(AUDIO_DEVICE_ID, 0x1019);
        assert_ne!(make_capability(0, 6, 0, 2), 0);
        assert_ne!(make_capability(0, 6, 0, 2), make_capability(0, 7, 0, 2));
    }

    #[test]
    fn accepts_only_the_bounded_reference_pcm_format() {
        let format = SoundPcmFormat {
            channels: 2,
            format: 5,
            rate: 7,
        };
        assert!(stream_is_supported(format, 2, 2, 1 << 5, 1 << 7));
        assert!(!stream_is_supported(format, 1, 1, 1 << 5, 1 << 7));
        assert!(!stream_is_supported(format, 2, 2, 1 << 3, 1 << 7));
        assert!(!stream_is_supported(format, 2, 2, 1 << 5, 1 << 6));
    }
}
