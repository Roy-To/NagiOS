use super::*;
use nagi_bootinfo::{BootInfo, BootInfoError, FramebufferInfo, MemoryMapInfo};

#[test]
fn variable_topology_rejects_duplicates_missing_bsp_empty_and_capacity() {
    for ids in [&[0][..], &[0, 7][..], &[0, 7, 12, 19][..]] {
        assert_eq!(validate_topology(ids, 0, 4), Ok(()));
    }
    for ids in [&[][..], &[0, 0][..], &[7, 12][..], &[0, 1, 2, 3, 4][..]] {
        assert_eq!(validate_topology(ids, 0, 4), Err(ContractError::Topology));
    }
    assert_eq!(validate_topology(&[0], 0, 0), Err(ContractError::Topology));
}

#[test]
fn firmware_and_headless_proposal_are_separate_from_v5() {
    assert_eq!(
        validate_future_boot(6, Firmware::DeviceTree(0x4000), None),
        Ok(())
    );
    assert_eq!(
        validate_future_boot(6, Firmware::Acpi(0x8000), Some((1920, 1080, 1920))),
        Ok(())
    );
    for version in [0, 5, 7, u32::MAX] {
        assert_eq!(
            validate_future_boot(version, Firmware::DeviceTree(0x4000), None),
            Err(ContractError::Version)
        );
    }
    for address in [0, 1, 7] {
        assert_eq!(
            validate_future_boot(6, Firmware::DeviceTree(address), None),
            Err(ContractError::Firmware)
        );
    }
    assert_eq!(
        validate_future_boot(6, Firmware::Acpi(8), Some((32, 20, 31))),
        Err(ContractError::Display)
    );
    // Actual production BootInfo v5 rejects a future version and missing firmware/display.
    let mut boot = BootInfo {
        memory_map: MemoryMapInfo {
            address: 0x1000,
            entry_count: 1,
            entry_size: 40,
            entry_version: 1,
            _reserved: 0,
        },
        framebuffer: FramebufferInfo {
            address: 0xe0000000,
            byte_size: 4096,
            width: 32,
            height: 20,
            pixels_per_scanline: 32,
            pixel_format: 1,
        },
        acpi_rsdp: 0x8000,
        ..BootInfo::new()
    };
    assert_eq!(boot.validate(), Ok(()));
    boot.version = 6;
    assert_eq!(boot.validate(), Err(BootInfoError::UnsupportedVersion));
    boot.version = 5;
    boot.acpi_rsdp = 0;
    assert_eq!(boot.validate(), Err(BootInfoError::MissingAcpi));
    boot.acpi_rsdp = 0x8000;
    boot.framebuffer = FramebufferInfo::default();
    assert_eq!(boot.validate(), Err(BootInfoError::MissingFramebuffer));
}

#[test]
fn mapping_proposal_rejects_overflow_unaligned_out_of_range_and_wx() {
    assert_eq!(validate_mapping(4096, 4096, 8192, false, true), Ok(()));
    assert_eq!(
        validate_mapping(4096, 4096, 8192, true, true),
        Err(ContractError::WritableExecutable)
    );
    for (start, bytes, limit) in [
        (1, 4096, 8192),
        (4096, 1, 8192),
        (4096, 0, 8192),
        (4096, 8192, 8192),
        (u64::MAX - 4095, 4096, u64::MAX),
    ] {
        assert_eq!(
            validate_mapping(start, bytes, limit, false, false),
            Err(ContractError::Address)
        );
    }
}

#[test]
fn elf_routing_is_explicit_and_production_still_rejects_arm64() {
    assert_eq!(validate_machine(Architecture::Aarch64, 183), Ok(()));
    assert_eq!(validate_machine(Architecture::X86_64, 62), Ok(()));
    assert_eq!(
        validate_machine(Architecture::X86_64, 183),
        Err(ContractError::Machine)
    );
    assert_eq!(
        validate_machine(Architecture::Aarch64, 62),
        Err(ContractError::Machine)
    );
    let mut elf = [0u8; 64];
    elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    elf[16..18].copy_from_slice(&2u16.to_le_bytes());
    elf[18..20].copy_from_slice(&183u16.to_le_bytes());
    elf[20..24].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        loader_elf::parse(&elf),
        Err(loader_elf::ElfError::UnsupportedMachine)
    );
    assert_eq!(
        user_elf::parse(&elf),
        Err(user_elf::UserElfError::UnsupportedMachine)
    );
}
