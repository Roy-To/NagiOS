#![no_std]

use core::mem::size_of;

pub const BOOT_INFO_MAGIC: u64 = 0x4E41_4749_424F_4F54;
pub const BOOT_INFO_VERSION: u32 = 2;

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
            || !self.memory_map.entry_size.is_multiple_of(8)
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
        Ok(())
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
    fn boot_info_v2_layout_is_c_compatible_and_stable() {
        assert_eq!(BOOT_INFO_VERSION, 2);
        assert_eq!(core::mem::size_of::<InitImageInfo>(), 16);
        assert_eq!(core::mem::size_of::<BootInfo>(), 104);
        assert_eq!(core::mem::offset_of!(BootInfo, magic), 0);
        assert_eq!(core::mem::offset_of!(BootInfo, version), 8);
        assert_eq!(core::mem::offset_of!(BootInfo, size), 12);
        assert_eq!(core::mem::offset_of!(BootInfo, memory_map), 16);
        assert_eq!(core::mem::offset_of!(BootInfo, framebuffer), 48);
        assert_eq!(core::mem::offset_of!(BootInfo, acpi_rsdp), 80);
        assert_eq!(core::mem::offset_of!(BootInfo, init_image), 88);
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
}
