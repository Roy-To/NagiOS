#![no_std]

use core::mem::size_of;

pub const BOOT_INFO_MAGIC: u64 = 0x4E41_4749_424F_4F54;
pub const BOOT_INFO_VERSION: u32 = 4;
pub const REALTIME_UNAVAILABLE_NS: u64 = u64::MAX;
pub const BOOT_READY_RECORD_SIZE: usize = 20;

const EFI_RUNTIME_SERVICES_CODE: u32 = 5;
const EFI_MEMORY_RUNTIME: u64 = 1 << 63;
const PAGE_SIZE: u64 = 4096;
const MAX_MEMORY_MAP_DESCRIPTORS: u64 = 4096;

/// RTC fields copied from UEFI before boot services end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirmwareDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanosecond: u32,
    /// Minutes east of UTC. `None` is interpreted as UTC for Nagi's QEMU
    /// reference machine, which is launched with `-rtc base=utc`.
    pub time_zone: Option<i16>,
    /// UEFI daylight flags. Nagi currently fails closed for daylight-adjusted
    /// values because the firmware data does not identify the adjustment.
    pub daylight_flags: u8,
}

/// Convert a validated UEFI RTC value to nanoseconds since the Unix epoch.
/// Invalid, unavailable, pre-epoch, or daylight-adjusted values fail closed.
pub fn firmware_time_to_unix_ns(time: FirmwareDateTime) -> u64 {
    if !(1970..=9999).contains(&time.year)
        || !(1..=12).contains(&time.month)
        || time.hour > 23
        || time.minute > 59
        || time.second > 59
        || time.nanosecond > 999_999_999
        || time.daylight_flags != 0
        || time
            .time_zone
            .is_some_and(|offset| !(-1440..=1440).contains(&offset))
    {
        return REALTIME_UNAVAILABLE_NS;
    }

    let month_days = match time.month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(time.year) => 29,
        2 => 28,
        _ => return REALTIME_UNAVAILABLE_NS,
    };
    if time.day == 0 || time.day > month_days {
        return REALTIME_UNAVAILABLE_NS;
    }

    let days = days_before_year(time.year)
        + days_before_month(time.year, time.month)
        + u64::from(time.day - 1);
    let seconds = days * 86_400
        + u64::from(time.hour) * 3_600
        + u64::from(time.minute) * 60
        + u64::from(time.second);
    let offset_seconds = i64::from(time.time_zone.unwrap_or(0)) * 60;
    let utc_seconds = i64::try_from(seconds)
        .ok()
        .and_then(|seconds| seconds.checked_sub(offset_seconds));
    let Some(utc_seconds) = utc_seconds.filter(|seconds| *seconds >= 0) else {
        return REALTIME_UNAVAILABLE_NS;
    };

    u64::try_from(utc_seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000_000))
        .and_then(|nanoseconds| nanoseconds.checked_add(u64::from(time.nanosecond)))
        .unwrap_or(REALTIME_UNAVAILABLE_NS)
}

#[allow(clippy::manual_is_multiple_of)]
const fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_before_year(year: u16) -> u64 {
    let year = u64::from(year - 1);
    let epoch_year = 1969;
    year * 365 + year / 4 - year / 100 + year / 400
        - (epoch_year * 365 + epoch_year / 4 - epoch_year / 100 + epoch_year / 400)
}

fn days_before_month(year: u16, month: u8) -> u64 {
    let common_year_days: [u16; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    u64::from(common_year_days[usize::from(month - 1)]) + u64::from(month > 2 && is_leap_year(year))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryMapInfo {
    pub address: u64,
    pub entry_count: u64,
    pub entry_size: u64,
    pub entry_version: u32,
    pub _reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryMapEntry {
    pub memory_type: u32,
    pub _reserved: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub page_count: u64,
    pub attributes: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FramebufferInfo {
    pub address: u64,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub pixels_per_scanline: u32,
    pub pixel_format: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct InitImageInfo {
    pub address: u64,
    pub size: u64,
}

/// Trusted loader context for a pending A/B trial.
///
/// All fields are zero for ordinary boots. The function address is the UEFI
/// Runtime Services `SetVariable` entry point, not a caller-controlled
/// authority. Kernel code validates that it resides in a runtime-code memory
/// descriptor before invoking it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BootControlInfo {
    pub set_variable_address: u64,
    pub journal_generation: u64,
    pub slot: u8,
    pub attempt: u8,
    pub reserved: [u8; 6],
}

impl BootControlInfo {
    pub fn is_empty(self) -> bool {
        self.set_variable_address == 0
            && self.journal_generation == 0
            && self.slot == 0
            && self.attempt == 0
            && self.reserved == [0; 6]
    }

    pub fn is_trial(self) -> bool {
        self.set_variable_address != 0
            && self.journal_generation != 0
            && self.slot <= 1
            && self.attempt >= 1
            && self.attempt <= 3
            && self.reserved == [0; 6]
    }

    fn is_valid(self) -> bool {
        self.is_empty() || self.is_trial()
    }
}

/// One-shot record written by the kernel only after the desktop readiness
/// gate. The loader consumes it only when the trial coordinates still match.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootReadyRecord {
    pub slot: u8,
    pub attempt: u8,
    pub journal_generation: u64,
}

impl BootReadyRecord {
    const MAGIC: [u8; 4] = *b"NBRD";
    const VERSION: u8 = 1;

    pub fn encode(self) -> Option<[u8; BOOT_READY_RECORD_SIZE]> {
        if self.slot > 1 || !(1..=3).contains(&self.attempt) || self.journal_generation == 0 {
            return None;
        }
        let mut bytes = [0; BOOT_READY_RECORD_SIZE];
        bytes[..4].copy_from_slice(&Self::MAGIC);
        bytes[4] = Self::VERSION;
        bytes[5] = self.slot;
        bytes[6] = self.attempt;
        bytes[8..16].copy_from_slice(&self.journal_generation.to_le_bytes());
        let checksum = crc32(&bytes[..16]);
        bytes[16..20].copy_from_slice(&checksum.to_le_bytes());
        Some(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != BOOT_READY_RECORD_SIZE
            || bytes[..4] != Self::MAGIC
            || bytes[4] != Self::VERSION
            || bytes[7] != 0
            || crc32(&bytes[..16]) != u32::from_le_bytes(bytes[16..20].try_into().ok()?)
        {
            return None;
        }
        let record = Self {
            slot: bytes[5],
            attempt: bytes[6],
            journal_generation: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        };
        record.encode().map(|_| record)
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub size: u32,
    pub memory_map: MemoryMapInfo,
    pub framebuffer: FramebufferInfo,
    pub acpi_rsdp: u64,
    pub init_image: InitImageInfo,
    pub realtime_epoch_ns: u64,
    pub boot_control: BootControlInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootInfoError {
    Null,
    BadMagic,
    UnsupportedVersion,
    BadSize,
    MissingMemoryMap,
    BadMemoryMapStride,
    MissingFramebuffer,
    MissingAcpi,
    MissingInitImage,
    InvalidBootControl,
}

impl BootInfo {
    pub const fn new() -> Self {
        Self {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            size: size_of::<Self>() as u32,
            memory_map: MemoryMapInfo {
                address: 0,
                entry_count: 0,
                entry_size: 0,
                entry_version: 0,
                _reserved: 0,
            },
            framebuffer: FramebufferInfo {
                address: 0,
                byte_size: 0,
                width: 0,
                height: 0,
                pixels_per_scanline: 0,
                pixel_format: 0,
            },
            acpi_rsdp: 0,
            init_image: InitImageInfo {
                address: 0,
                size: 0,
            },
            realtime_epoch_ns: REALTIME_UNAVAILABLE_NS,
            boot_control: BootControlInfo {
                set_variable_address: 0,
                journal_generation: 0,
                slot: 0,
                attempt: 0,
                reserved: [0; 6],
            },
        }
    }

    pub fn validate(&self) -> Result<(), BootInfoError> {
        if self.magic != BOOT_INFO_MAGIC {
            return Err(BootInfoError::BadMagic);
        }
        if self.version != BOOT_INFO_VERSION {
            return Err(BootInfoError::UnsupportedVersion);
        }
        if self.size as usize != size_of::<Self>() {
            return Err(BootInfoError::BadSize);
        }
        if self.memory_map.address == 0 || self.memory_map.entry_count == 0 {
            return Err(BootInfoError::MissingMemoryMap);
        }
        if self.memory_map.entry_size < size_of::<MemoryMapEntry>() as u64
            || self.memory_map.entry_size & 7 != 0
        {
            return Err(BootInfoError::BadMemoryMapStride);
        }
        if self.framebuffer.address == 0
            || self.framebuffer.byte_size == 0
            || self.framebuffer.width == 0
            || self.framebuffer.height == 0
            || self.framebuffer.pixels_per_scanline < self.framebuffer.width
        {
            return Err(BootInfoError::MissingFramebuffer);
        }
        if self.acpi_rsdp == 0 {
            return Err(BootInfoError::MissingAcpi);
        }
        if !self.boot_control.is_valid() {
            return Err(BootInfoError::InvalidBootControl);
        }
        Ok(())
    }

    /// Validate that the pending-trial writer resides in a UEFI Runtime
    /// Services Code descriptor from the firmware memory map.
    ///
    /// # Safety
    ///
    /// If `boot_control` describes a trial, `memory_map.address` must point to
    /// the live, readable UEFI memory-map buffer described by this `BootInfo`.
    pub unsafe fn boot_control_writer_is_runtime_code(&self) -> bool {
        if !self.boot_control.is_trial()
            || self.memory_map.address == 0
            || self.memory_map.entry_count == 0
            || self.memory_map.entry_count > MAX_MEMORY_MAP_DESCRIPTORS
            || self.memory_map.entry_size < size_of::<MemoryMapEntry>() as u64
            || self.memory_map.entry_size & 7 != 0
        {
            return false;
        }

        let Ok(base) = usize::try_from(self.memory_map.address) else {
            return false;
        };
        let writer_address = self.boot_control.set_variable_address;
        let descriptors = self.memory_map.entry_count;
        let stride = self.memory_map.entry_size;
        for index in 0..descriptors {
            let Some(offset) = index.checked_mul(stride) else {
                return false;
            };
            let Ok(offset) = usize::try_from(offset) else {
                return false;
            };
            let Some(address) = base.checked_add(offset) else {
                return false;
            };
            // SAFETY: required by this method's contract; the stride and
            // descriptor count were validated above.
            let descriptor = unsafe { (address as *const MemoryMapEntry).read_unaligned() };
            if descriptor.memory_type != EFI_RUNTIME_SERVICES_CODE
                || descriptor.attributes & EFI_MEMORY_RUNTIME == 0
            {
                continue;
            }
            let Some(length) = descriptor.page_count.checked_mul(PAGE_SIZE) else {
                continue;
            };
            let Some(end) = descriptor.physical_start.checked_add(length) else {
                continue;
            };
            if descriptor.physical_start <= writer_address && writer_address < end {
                return true;
            }
        }
        false
    }

    /// Validate the boot contract required before entering an M5 user process.
    ///
    /// The general validation above intentionally remains compatible with the
    /// M1-M4 memory and ACPI paths, which do not consume an init image.
    pub fn validate_for_user_bootstrap(&self) -> Result<(), BootInfoError> {
        self.validate()?;
        if self.init_image.address == 0 || self.init_image.size == 0 {
            return Err(BootInfoError::MissingInitImage);
        }
        Ok(())
    }
}

impl Default for BootInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// # Safety
///
/// The caller must provide a readable pointer to a properly aligned
/// `BootInfo` object that remains valid for the returned lifetime.
pub unsafe fn boot_info_from_ptr<'a>(ptr: *const BootInfo) -> Result<&'a BootInfo, BootInfoError> {
    let Some(info) = ptr.as_ref() else {
        return Err(BootInfoError::Null);
    };
    info.validate()?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_boot_info() -> BootInfo {
        BootInfo {
            memory_map: MemoryMapInfo {
                address: 0x1000,
                entry_count: 4,
                entry_size: size_of::<MemoryMapEntry>() as u64,
                entry_version: 1,
                _reserved: 0,
            },
            framebuffer: FramebufferInfo {
                address: 0xE000_0000,
                byte_size: 1024 * 768 * 4,
                width: 1024,
                height: 768,
                pixels_per_scanline: 1024,
                pixel_format: 1,
            },
            acpi_rsdp: 0xF0000,
            init_image: InitImageInfo {
                address: 0x30_0000,
                size: 4096,
            },
            ..BootInfo::new()
        }
    }

    #[test]
    fn valid_boot_info_is_accepted() {
        assert_eq!(valid_boot_info().validate(), Ok(()));
    }

    #[test]
    fn pre_m5_validation_allows_absent_init_image_but_bootstrap_rejects_it() {
        let mut info = valid_boot_info();
        info.init_image = InitImageInfo::default();
        assert_eq!(info.validate(), Ok(()));
        assert_eq!(
            info.validate_for_user_bootstrap(),
            Err(BootInfoError::MissingInitImage)
        );
    }

    #[test]
    fn valid_boot_info_requires_a_persistent_init_image() {
        let mut info = valid_boot_info();
        info.init_image = InitImageInfo {
            address: 0x30_0000,
            size: 4096,
        };
        assert_eq!(info.validate_for_user_bootstrap(), Ok(()));
    }

    #[test]
    fn empty_init_image_is_rejected() {
        let mut info = valid_boot_info();
        info.init_image = InitImageInfo {
            address: 0x30_0000,
            size: 0,
        };
        assert_eq!(
            info.validate_for_user_bootstrap(),
            Err(BootInfoError::MissingInitImage)
        );
    }

    #[test]
    fn zero_address_init_image_is_rejected_for_user_bootstrap() {
        let mut info = valid_boot_info();
        info.init_image = InitImageInfo {
            address: 0,
            size: 4096,
        };
        assert_eq!(
            info.validate_for_user_bootstrap(),
            Err(BootInfoError::MissingInitImage)
        );
    }

    #[test]
    fn boot_info_v4_layout_is_c_compatible_and_stable() {
        assert_eq!(BOOT_INFO_VERSION, 4);
        assert_eq!(core::mem::size_of::<InitImageInfo>(), 16);
        assert_eq!(core::mem::size_of::<BootControlInfo>(), 24);
        assert_eq!(core::mem::size_of::<BootInfo>(), 136);
        assert_eq!(core::mem::offset_of!(BootInfo, magic), 0);
        assert_eq!(core::mem::offset_of!(BootInfo, version), 8);
        assert_eq!(core::mem::offset_of!(BootInfo, size), 12);
        assert_eq!(core::mem::offset_of!(BootInfo, memory_map), 16);
        assert_eq!(core::mem::offset_of!(BootInfo, framebuffer), 48);
        assert_eq!(core::mem::offset_of!(BootInfo, acpi_rsdp), 80);
        assert_eq!(core::mem::offset_of!(BootInfo, init_image), 88);
        assert_eq!(core::mem::offset_of!(BootInfo, realtime_epoch_ns), 104);
        assert_eq!(core::mem::offset_of!(BootInfo, boot_control), 112);
        assert_eq!(BootInfo::new().realtime_epoch_ns, REALTIME_UNAVAILABLE_NS);
    }

    #[test]
    fn boot_control_context_rejects_partial_or_out_of_policy_values() {
        let mut info = valid_boot_info();
        info.boot_control = BootControlInfo {
            set_variable_address: 0x8000,
            journal_generation: 9,
            slot: 1,
            attempt: 2,
            reserved: [0; 6],
        };
        assert_eq!(info.validate(), Ok(()));

        info.boot_control.attempt = 4;
        assert_eq!(info.validate(), Err(BootInfoError::InvalidBootControl));
        info.boot_control.attempt = 2;
        info.boot_control.reserved[0] = 1;
        assert_eq!(info.validate(), Err(BootInfoError::InvalidBootControl));
    }

    #[test]
    fn boot_control_writer_must_be_in_runtime_services_code() {
        let descriptors = [MemoryMapEntry {
            memory_type: EFI_RUNTIME_SERVICES_CODE,
            physical_start: 0x8000,
            page_count: 2,
            attributes: EFI_MEMORY_RUNTIME,
            ..MemoryMapEntry::default()
        }];
        let mut info = valid_boot_info();
        info.memory_map.address = descriptors.as_ptr() as u64;
        info.memory_map.entry_count = descriptors.len() as u64;
        info.memory_map.entry_size = size_of::<MemoryMapEntry>() as u64;
        info.boot_control = BootControlInfo {
            set_variable_address: 0x8123,
            journal_generation: 9,
            slot: 1,
            attempt: 2,
            reserved: [0; 6],
        };
        assert!(unsafe { info.boot_control_writer_is_runtime_code() });

        info.boot_control.set_variable_address = 0xa000;
        assert!(!unsafe { info.boot_control_writer_is_runtime_code() });
    }

    #[test]
    fn boot_ready_record_round_trips_and_rejects_corruption() {
        let record = BootReadyRecord {
            slot: 1,
            attempt: 3,
            journal_generation: 0x1020_3040_5060_7080,
        };
        let mut encoded = record.encode().expect("valid record");
        assert_eq!(BootReadyRecord::decode(&encoded), Some(record));
        encoded[7] = 1;
        assert_eq!(BootReadyRecord::decode(&encoded), None);
        encoded[7] = 0;
        encoded[9] ^= 0x40;
        assert_eq!(BootReadyRecord::decode(&encoded), None);
    }

    #[test]
    fn boot_ready_record_rejects_invalid_trial_coordinates() {
        for record in [
            BootReadyRecord {
                slot: 2,
                attempt: 1,
                journal_generation: 1,
            },
            BootReadyRecord {
                slot: 1,
                attempt: 0,
                journal_generation: 1,
            },
            BootReadyRecord {
                slot: 1,
                attempt: 1,
                journal_generation: 0,
            },
        ] {
            assert!(record.encode().is_none());
        }
    }

    #[test]
    fn invalid_magic_is_rejected() {
        let mut info = valid_boot_info();
        info.magic = 0;
        assert_eq!(info.validate(), Err(BootInfoError::BadMagic));
    }

    #[test]
    fn short_memory_map_stride_is_rejected() {
        let mut info = valid_boot_info();
        info.memory_map.entry_size = 8;
        assert_eq!(info.validate(), Err(BootInfoError::BadMemoryMapStride));
    }

    fn datetime(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> FirmwareDateTime {
        FirmwareDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second: 0,
            nanosecond: 0,
            time_zone: Some(0),
            daylight_flags: 0,
        }
    }

    #[test]
    fn firmware_time_conversion_handles_epoch_leap_day_and_nanoseconds() {
        assert_eq!(
            firmware_time_to_unix_ns(FirmwareDateTime {
                nanosecond: 123,
                ..datetime(1970, 1, 1, 0, 0)
            }),
            123
        );
        assert_eq!(
            firmware_time_to_unix_ns(datetime(2000, 2, 29, 0, 0)),
            951_782_400_000_000_000
        );
        assert_eq!(
            firmware_time_to_unix_ns(datetime(2026, 9, 28, 0, 0)),
            1_790_553_600_000_000_000
        );
    }

    #[test]
    fn firmware_time_conversion_applies_timezone_offset() {
        let utc = datetime(2026, 9, 28, 0, 0);
        let mut unspecified_zone = utc;
        unspecified_zone.time_zone = None;
        assert_eq!(
            firmware_time_to_unix_ns(unspecified_zone),
            firmware_time_to_unix_ns(utc)
        );

        let mut local = utc;
        local.time_zone = Some(330);
        assert_eq!(
            firmware_time_to_unix_ns(local),
            firmware_time_to_unix_ns(utc) - 330 * 60 * 1_000_000_000
        );
    }

    #[test]
    fn invalid_unavailable_and_daylight_adjusted_firmware_times_fail_closed() {
        for invalid in [
            datetime(1969, 12, 31, 23, 59),
            datetime(2025, 2, 29, 0, 0),
            datetime(2026, 4, 31, 0, 0),
            datetime(2026, 9, 28, 24, 0),
            datetime(2026, 9, 28, 0, 60),
            datetime(3000, 1, 1, 0, 0),
        ] {
            assert_eq!(firmware_time_to_unix_ns(invalid), REALTIME_UNAVAILABLE_NS);
        }

        let mut daylight = datetime(2026, 9, 28, 0, 0);
        daylight.daylight_flags = 1;
        assert_eq!(firmware_time_to_unix_ns(daylight), REALTIME_UNAVAILABLE_NS);

        let mut underflow = datetime(1970, 1, 1, 0, 0);
        underflow.time_zone = Some(1);
        assert_eq!(firmware_time_to_unix_ns(underflow), REALTIME_UNAVAILABLE_NS);

        let mut bad_timezone = datetime(2026, 9, 28, 0, 0);
        bad_timezone.time_zone = Some(1441);
        assert_eq!(
            firmware_time_to_unix_ns(bad_timezone),
            REALTIME_UNAVAILABLE_NS
        );
    }
}
