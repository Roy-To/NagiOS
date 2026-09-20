use core::slice;

use nagi_bootinfo::{BootInfo, MemoryMapEntry};

pub const MAX_CPUS: usize = 4;
const ACPI_HEADER_SIZE: usize = 36;
const RSDP_V1_SIZE: usize = 20;
const RSDP_V2_SIZE: usize = 36;
const MAX_TABLE_SIZE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpiError {
    InvalidRsdp,
    InvalidTable,
    MissingMadt,
    InvalidMadt,
    TooManyCpus,
    DuplicateCpu,
    BspNotFound,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CpuInfo {
    pub apic_id: u32,
    pub uid: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct CpuTopology {
    cpus: [CpuInfo; MAX_CPUS],
    count: usize,
    bsp_index: usize,
    local_apic_address: u64,
}

impl CpuTopology {
    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn bsp_index(&self) -> usize {
        self.bsp_index
    }

    pub const fn local_apic_address(&self) -> u64 {
        self.local_apic_address
    }

    pub fn cpu(&self, index: usize) -> Option<CpuInfo> {
        self.cpus.get(index).copied().filter(|_| index < self.count)
    }
}

/// # Safety
///
/// `rsdp_address` must point to a readable ACPI RSDP and the referenced
/// tables must remain readable for the duration of this call. The addresses
/// must be guest physical addresses identity-mapped in the current address
/// space. `bsp_apic_id` must be the Local APIC ID of the executing CPU.
pub unsafe fn discover(info: &BootInfo, bsp_apic_id: u32) -> Result<CpuTopology, AcpiError> {
    if info.validate().is_err() {
        return Err(AcpiError::InvalidRsdp);
    }
    let rsdp_address = info.acpi_rsdp;
    if rsdp_address == 0 {
        return Err(AcpiError::InvalidRsdp);
    }
    let rsdp_prefix = bytes(info, rsdp_address, RSDP_V1_SIZE)?;
    if &rsdp_prefix[0..8] != b"RSD PTR " || !checksum_ok(rsdp_prefix) {
        return Err(AcpiError::InvalidRsdp);
    }
    let revision = rsdp_prefix[15];
    let (root_address, entry_size) = if revision >= 2 {
        let rsdp = bytes(info, rsdp_address, RSDP_V2_SIZE)?;
        let length = read_u32(rsdp, 20) as usize;
        if !(RSDP_V2_SIZE..=MAX_TABLE_SIZE).contains(&length) {
            return Err(AcpiError::InvalidRsdp);
        }
        if !checksum_ok(bytes(info, rsdp_address, length)?) {
            return Err(AcpiError::InvalidRsdp);
        }
        let xsdt = read_u64(rsdp, 24);
        if xsdt != 0 {
            (xsdt, 8)
        } else {
            (u64::from(read_u32(rsdp_prefix, 16)), 4)
        }
    } else {
        (u64::from(read_u32(rsdp_prefix, 16)), 4)
    };
    if root_address == 0 {
        return Err(AcpiError::InvalidTable);
    }

    let root = table(info, root_address)?;
    if entry_size == 8 && &root[0..4] != b"XSDT" {
        return Err(AcpiError::InvalidTable);
    }
    if entry_size == 4 && &root[0..4] != b"RSDT" {
        return Err(AcpiError::InvalidTable);
    }
    let payload_size = root
        .len()
        .checked_sub(ACPI_HEADER_SIZE)
        .ok_or(AcpiError::InvalidTable)?;
    if payload_size % entry_size != 0 {
        return Err(AcpiError::InvalidTable);
    }
    let mut madt_address = None;
    for offset in (ACPI_HEADER_SIZE..root.len()).step_by(entry_size) {
        let address = if entry_size == 8 {
            read_u64(root, offset)
        } else {
            u64::from(read_u32(root, offset))
        };
        if address == 0 {
            continue;
        }
        let candidate = table(info, address)?;
        if &candidate[0..4] == b"APIC" {
            madt_address = Some(address);
            break;
        }
    }
    let madt = table(info, madt_address.ok_or(AcpiError::MissingMadt)?)?;
    parse_madt(madt, bsp_apic_id)
}

unsafe fn bytes(info: &BootInfo, address: u64, length: usize) -> Result<&'static [u8], AcpiError> {
    if length == 0 || !range_is_readable(info, address, length)? {
        return Err(AcpiError::InvalidTable);
    }
    Ok(slice::from_raw_parts(address as *const u8, length))
}

unsafe fn table(info: &BootInfo, address: u64) -> Result<&'static [u8], AcpiError> {
    if address == 0 || !range_is_readable(info, address, 8)? {
        return Err(AcpiError::InvalidTable);
    }
    let prefix = bytes(info, address, 8)?;
    let length = read_u32(prefix, 4) as usize;
    if !(ACPI_HEADER_SIZE..=MAX_TABLE_SIZE).contains(&length) {
        return Err(AcpiError::InvalidTable);
    }
    let table = bytes(info, address, length)?;
    if !checksum_ok(table) {
        return Err(AcpiError::InvalidTable);
    }
    Ok(table)
}

unsafe fn range_is_readable(
    info: &BootInfo,
    address: u64,
    length: usize,
) -> Result<bool, AcpiError> {
    let length = u64::try_from(length).map_err(|_| AcpiError::InvalidTable)?;
    let end = address.checked_add(length).ok_or(AcpiError::InvalidTable)?;
    let entry_count =
        usize::try_from(info.memory_map.entry_count).map_err(|_| AcpiError::InvalidTable)?;
    let entry_size =
        usize::try_from(info.memory_map.entry_size).map_err(|_| AcpiError::InvalidTable)?;
    let base = info.memory_map.address as *const u8;
    for index in 0..entry_count {
        let offset = index
            .checked_mul(entry_size)
            .ok_or(AcpiError::InvalidTable)?;
        let entry = core::ptr::read_unaligned(base.add(offset).cast::<MemoryMapEntry>());
        let entry_end = entry
            .physical_start
            .checked_add(
                entry
                    .page_count
                    .checked_mul(4096)
                    .ok_or(AcpiError::InvalidTable)?,
            )
            .ok_or(AcpiError::InvalidTable)?;
        if address >= entry.physical_start && end <= entry_end {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_madt(madt: &[u8], bsp_apic_id: u32) -> Result<CpuTopology, AcpiError> {
    if &madt[0..4] != b"APIC" || madt.len() < 44 {
        return Err(AcpiError::InvalidMadt);
    }
    let mut topology = CpuTopology {
        cpus: [CpuInfo::default(); MAX_CPUS],
        count: 0,
        bsp_index: 0,
        local_apic_address: u64::from(read_u32(madt, 36)),
    };
    if topology.local_apic_address == 0 || topology.local_apic_address & 0xfff != 0 {
        return Err(AcpiError::InvalidMadt);
    }
    let mut offset = 44;
    while offset < madt.len() {
        if madt.len() - offset < 2 {
            return Err(AcpiError::InvalidMadt);
        }
        let entry_type = madt[offset];
        let entry_length = usize::from(madt[offset + 1]);
        if entry_length < 2 || entry_length > madt.len() - offset {
            return Err(AcpiError::InvalidMadt);
        }
        match entry_type {
            0 => {
                if entry_length != 8 {
                    return Err(AcpiError::InvalidMadt);
                }
                let flags = read_u32(madt, offset + 4);
                if flags & 1 != 0 {
                    add_cpu(
                        &mut topology,
                        CpuInfo {
                            apic_id: u32::from(madt[offset + 3]),
                            uid: u32::from(madt[offset + 2]),
                        },
                    )?;
                }
            }
            5 => {
                if entry_length != 12 {
                    return Err(AcpiError::InvalidMadt);
                }
                let address = read_u64(madt, offset + 4);
                if address == 0 || address & 0xfff != 0 {
                    return Err(AcpiError::InvalidMadt);
                }
                topology.local_apic_address = address;
            }
            9 => return Err(AcpiError::InvalidMadt),
            _ => {}
        }
        offset += entry_length;
    }
    if topology.count == 0 {
        return Err(AcpiError::InvalidMadt);
    }
    topology.bsp_index = topology
        .cpus
        .iter()
        .position(|cpu| cpu.apic_id == bsp_apic_id)
        .ok_or(AcpiError::BspNotFound)?;
    Ok(topology)
}

fn add_cpu(topology: &mut CpuTopology, cpu: CpuInfo) -> Result<(), AcpiError> {
    if topology.cpus[..topology.count]
        .iter()
        .any(|existing| existing.apic_id == cpu.apic_id)
    {
        return Err(AcpiError::DuplicateCpu);
    }
    if topology.count == MAX_CPUS {
        return Err(AcpiError::TooManyCpus);
    }
    topology.cpus[topology.count] = cpu;
    topology.count += 1;
    Ok(())
}

fn checksum_ok(table: &[u8]) -> bool {
    table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use nagi_bootinfo::{BootInfo, MemoryMapEntry};

    use super::{discover, AcpiError, MAX_CPUS};

    fn set_header(table: &mut [u8], signature: &[u8; 4], length: usize) {
        table[0..4].copy_from_slice(signature);
        table[4..8].copy_from_slice(&(length as u32).to_le_bytes());
    }

    fn finish_checksum(table: &mut [u8]) {
        table[9] = 0;
        let checksum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        table[9] = 0u8.wrapping_sub(checksum);
    }

    fn build_tables(
        rsdp: &mut [u8; 36],
        xsdt: &mut [u8; 44],
        madt: &mut [u8; 44 + (MAX_CPUS + 1) * 8],
        cpu_count: usize,
    ) {
        let madt_length = 44 + cpu_count * 8;
        set_header(madt, b"APIC", madt_length);
        madt[36..40].copy_from_slice(&0xFEE0_0000u32.to_le_bytes());
        for index in 0..cpu_count {
            let offset = 44 + index * 8;
            madt[offset] = 0;
            madt[offset + 1] = 8;
            madt[offset + 2] = index as u8;
            madt[offset + 3] = index as u8;
            madt[offset + 4..offset + 8].copy_from_slice(&1u32.to_le_bytes());
        }
        finish_checksum(&mut madt[..madt_length]);

        set_header(xsdt, b"XSDT", 44);
        xsdt[36..44].copy_from_slice(&(madt.as_ptr() as u64).to_le_bytes());
        finish_checksum(xsdt);

        rsdp[0..8].copy_from_slice(b"RSD PTR ");
        rsdp[15] = 2;
        rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());
        rsdp[24..32].copy_from_slice(&(xsdt.as_ptr() as u64).to_le_bytes());
        let checksum = rsdp[..20]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        rsdp[8] = 0u8.wrapping_sub(checksum);
        let extended = rsdp.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        rsdp[32] = 0u8.wrapping_sub(extended);
    }

    fn boot_info(
        rsdp: &mut [u8; 36],
        xsdt: &mut [u8; 44],
        madt: &mut [u8; 44 + (MAX_CPUS + 1) * 8],
        map_entry: &mut MemoryMapEntry,
    ) -> BootInfo {
        let start = (rsdp.as_ptr() as u64)
            .min(xsdt.as_ptr() as u64)
            .min(madt.as_ptr() as u64)
            & !4095;
        let end = (rsdp.as_ptr() as u64 + rsdp.len() as u64)
            .max(xsdt.as_ptr() as u64 + xsdt.len() as u64)
            .max(madt.as_ptr() as u64 + madt.len() as u64);
        map_entry.memory_type = 9;
        map_entry.physical_start = start;
        map_entry.page_count = (end - start).div_ceil(4096);
        BootInfo {
            memory_map: nagi_bootinfo::MemoryMapInfo {
                address: map_entry as *const _ as u64,
                entry_count: 1,
                entry_size: core::mem::size_of::<MemoryMapEntry>() as u64,
                entry_version: 1,
                _reserved: 0,
            },
            framebuffer: nagi_bootinfo::FramebufferInfo {
                address: 0xE000_0000,
                byte_size: 4096,
                width: 1,
                height: 1,
                pixels_per_scanline: 1,
                pixel_format: 0,
            },
            acpi_rsdp: rsdp.as_ptr() as u64,
            init_image: nagi_bootinfo::InitImageInfo {
                address: 0x30_0000,
                size: 4096,
            },
            ..BootInfo::new()
        }
    }

    #[test]
    fn discovers_four_enabled_processors_and_bsp() {
        let mut rsdp = [0u8; 36];
        let mut xsdt = [0u8; 44];
        let mut madt = [0u8; 44 + (MAX_CPUS + 1) * 8];
        let mut map_entry = MemoryMapEntry::default();
        build_tables(&mut rsdp, &mut xsdt, &mut madt, MAX_CPUS);
        let info = boot_info(&mut rsdp, &mut xsdt, &mut madt, &mut map_entry);
        let topology = unsafe { discover(&info, 0) }.expect("topology");
        assert_eq!(topology.count(), MAX_CPUS);
        assert_eq!(topology.bsp_index(), 0);
        assert_eq!(topology.cpu(3).expect("cpu").apic_id, 3);
    }

    #[test]
    fn rejects_bad_root_checksum() {
        let mut rsdp = [0u8; 36];
        let mut xsdt = [0u8; 44];
        let mut madt = [0u8; 44 + (MAX_CPUS + 1) * 8];
        let mut map_entry = MemoryMapEntry::default();
        build_tables(&mut rsdp, &mut xsdt, &mut madt, 2);
        xsdt[20] ^= 1;
        let info = boot_info(&mut rsdp, &mut xsdt, &mut madt, &mut map_entry);
        assert_eq!(unsafe { discover(&info, 0) }, Err(AcpiError::InvalidTable));
    }

    #[test]
    fn rejects_more_than_reference_cpu_bound() {
        let mut rsdp = [0u8; 36];
        let mut xsdt = [0u8; 44];
        let mut madt = [0u8; 44 + (MAX_CPUS + 1) * 8];
        let mut map_entry = MemoryMapEntry::default();
        build_tables(&mut rsdp, &mut xsdt, &mut madt, MAX_CPUS + 1);
        let info = boot_info(&mut rsdp, &mut xsdt, &mut madt, &mut map_entry);
        assert_eq!(unsafe { discover(&info, 0) }, Err(AcpiError::TooManyCpus));
    }

    #[test]
    fn accepts_local_apic_address_override() {
        let mut rsdp = [0u8; 36];
        let mut xsdt = [0u8; 44];
        let mut madt = [0u8; 44 + (MAX_CPUS + 1) * 8];
        let mut map_entry = MemoryMapEntry::default();
        build_tables(&mut rsdp, &mut xsdt, &mut madt, 1);
        let override_offset = 44 + 8;
        let length = override_offset + 12;
        madt[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        madt[override_offset] = 5;
        madt[override_offset + 1] = 12;
        madt[override_offset + 4..override_offset + 12]
            .copy_from_slice(&0xFEE0_1000u64.to_le_bytes());
        finish_checksum(&mut madt[..length]);
        let info = boot_info(&mut rsdp, &mut xsdt, &mut madt, &mut map_entry);
        let topology = unsafe { discover(&info, 0) }.expect("topology");
        assert_eq!(topology.local_apic_address(), 0xFEE0_1000);
    }

    #[test]
    fn rejects_malformed_local_apic_entry() {
        let mut rsdp = [0u8; 36];
        let mut xsdt = [0u8; 44];
        let mut madt = [0u8; 44 + (MAX_CPUS + 1) * 8];
        let mut map_entry = MemoryMapEntry::default();
        build_tables(&mut rsdp, &mut xsdt, &mut madt, 1);
        let malformed_offset = 44 + 8;
        let length = malformed_offset + 7;
        madt[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        madt[malformed_offset] = 0;
        madt[malformed_offset + 1] = 7;
        finish_checksum(&mut madt[..length]);
        let info = boot_info(&mut rsdp, &mut xsdt, &mut madt, &mut map_entry);
        assert_eq!(unsafe { discover(&info, 0) }, Err(AcpiError::InvalidMadt));
    }
}
