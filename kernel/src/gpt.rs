pub const SECTOR_SIZE: usize = 512;
pub const ENTRY_COUNT: usize = 128;
pub const ENTRY_SIZE: usize = 128;
pub const ENTRY_ARRAY_SECTORS: u64 = (ENTRY_COUNT * ENTRY_SIZE / SECTOR_SIZE) as u64;
pub const USER_DATA_TYPE_GUID: [u8; 16] = [
    0x01, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 3,
];
pub const MODEL_STORE_TYPE_GUID: [u8; 16] = [
    0x01, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 5,
];
/// System A and System B slot partition types (ADR-0013).
pub const SYSTEM_A_TYPE_GUID: [u8; 16] = [
    0x01, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 1,
];
pub const SYSTEM_B_TYPE_GUID: [u8; 16] = [
    0x01, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 2,
];

const HEADER_SIGNATURE: &[u8; 8] = b"EFI PART";
const GPT_REVISION_1_0: u32 = 0x0001_0000;
const GPT_HEADER_MIN_SIZE: u32 = 92;
const MBR_PARTITION_OFFSET: usize = 446;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionRange {
    pub start_lba: u64,
    pub sector_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionLayout {
    pub user_data: PartitionRange,
    pub model_store: Option<PartitionRange>,
    /// Present only when exactly one partition of the type exists.
    pub system_a: Option<PartitionRange>,
    pub system_b: Option<PartitionRange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GptError {
    Read,
    ProtectiveMbr,
    Header,
    HeaderChecksum,
    EntryArray,
    EntryChecksum,
    PartitionBounds,
    PartitionOverlap,
    DuplicateGuid,
    MissingUserData,
    MultipleUserData,
    MultipleModelStore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Header {
    current_lba: u64,
    backup_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    disk_guid: [u8; 16],
    entries_lba: u64,
    entries_crc: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Extent {
    first_lba: u64,
    last_lba: u64,
    unique_guid: [u8; 16],
}

pub fn find_user_data<F>(mut read_sector: F, disk_sectors: u64) -> Result<PartitionRange, GptError>
where
    F: FnMut(u64, &mut [u8; SECTOR_SIZE]) -> Result<(), ()>,
{
    find_partitions(&mut read_sector, disk_sectors).map(|partitions| partitions.user_data)
}

pub fn find_partitions<F>(
    mut read_sector: F,
    disk_sectors: u64,
) -> Result<PartitionLayout, GptError>
where
    F: FnMut(u64, &mut [u8; SECTOR_SIZE]) -> Result<(), ()>,
{
    if disk_sectors < ENTRY_ARRAY_SECTORS + 4 {
        return Err(GptError::Header);
    }

    let mut mbr = [0u8; SECTOR_SIZE];
    read_sector(0, &mut mbr).map_err(|()| GptError::Read)?;
    validate_protective_mbr(&mbr, disk_sectors)?;

    let mut primary_bytes = [0u8; SECTOR_SIZE];
    read_sector(1, &mut primary_bytes).map_err(|()| GptError::Read)?;
    let primary = parse_header(&mut primary_bytes)?;
    let backup_lba = disk_sectors - 1;
    if primary.current_lba != 1
        || primary.backup_lba != backup_lba
        || primary.entries_lba != 2
        || primary.first_usable_lba < 2 + ENTRY_ARRAY_SECTORS
        || primary.last_usable_lba >= backup_lba - ENTRY_ARRAY_SECTORS
        || primary.first_usable_lba > primary.last_usable_lba
    {
        return Err(GptError::Header);
    }

    let mut backup_bytes = [0u8; SECTOR_SIZE];
    read_sector(backup_lba, &mut backup_bytes).map_err(|()| GptError::Read)?;
    let backup = parse_header(&mut backup_bytes)?;
    if backup.current_lba != backup_lba
        || backup.backup_lba != 1
        || backup.entries_lba != backup_lba - ENTRY_ARRAY_SECTORS
        || backup.first_usable_lba != primary.first_usable_lba
        || backup.last_usable_lba != primary.last_usable_lba
        || !bytes_equal(&backup.disk_guid, &primary.disk_guid)
        || backup.entries_crc != primary.entries_crc
    {
        return Err(GptError::Header);
    }

    let mut extents: [Option<Extent>; ENTRY_COUNT] = [None; ENTRY_COUNT];
    let mut user_data = None;
    let mut model_store = None;
    let mut user_data_count = 0usize;
    let mut model_store_count = 0usize;
    let mut system_slots = [None; 2];
    let mut system_slot_counts = [0usize; 2];
    let mut crc = !0u32;
    for sector_index in 0..ENTRY_ARRAY_SECTORS {
        let mut bytes = [0u8; SECTOR_SIZE];
        read_sector(primary.entries_lba + sector_index, &mut bytes).map_err(|()| GptError::Read)?;
        crc = crc32_update(crc, &bytes);
        for slot in 0..SECTOR_SIZE / ENTRY_SIZE {
            let index = sector_index as usize * (SECTOR_SIZE / ENTRY_SIZE) + slot;
            let start = slot * ENTRY_SIZE;
            let entry = &bytes[start..start + ENTRY_SIZE];
            let mut type_guid = [0u8; 16];
            type_guid.copy_from_slice(&entry[..16]);
            if guid_is_zero(&type_guid) {
                if entry.iter().any(|byte| *byte != 0) {
                    return Err(GptError::EntryArray);
                }
                continue;
            }

            let mut unique_guid = [0u8; 16];
            unique_guid.copy_from_slice(&entry[16..32]);
            if guid_is_zero(&unique_guid) {
                return Err(GptError::EntryArray);
            }
            let first_lba = read_u64(entry, 32);
            let last_lba = read_u64(entry, 40);
            if first_lba < primary.first_usable_lba
                || first_lba > last_lba
                || last_lba > primary.last_usable_lba
            {
                return Err(GptError::PartitionBounds);
            }
            let extent = Extent {
                first_lba,
                last_lba,
                unique_guid,
            };
            for previous in extents.iter().flatten() {
                if bytes_equal(&extent.unique_guid, &previous.unique_guid) {
                    return Err(GptError::DuplicateGuid);
                }
                if extent.first_lba <= previous.last_lba && previous.first_lba <= extent.last_lba {
                    return Err(GptError::PartitionOverlap);
                }
            }
            extents[index] = Some(extent);
            if bytes_equal(&type_guid, &USER_DATA_TYPE_GUID) {
                user_data_count += 1;
                user_data = Some(PartitionRange {
                    start_lba: first_lba,
                    sector_count: last_lba
                        .checked_sub(first_lba)
                        .and_then(|sectors| sectors.checked_add(1))
                        .ok_or(GptError::PartitionBounds)?,
                });
            } else if let Some(index) = [SYSTEM_A_TYPE_GUID, SYSTEM_B_TYPE_GUID]
                .iter()
                .position(|guid| bytes_equal(&type_guid, guid))
            {
                system_slot_counts[index] += 1;
                system_slots[index] = Some(PartitionRange {
                    start_lba: first_lba,
                    sector_count: last_lba
                        .checked_sub(first_lba)
                        .and_then(|sectors| sectors.checked_add(1))
                        .ok_or(GptError::PartitionBounds)?,
                });
            } else if bytes_equal(&type_guid, &MODEL_STORE_TYPE_GUID) {
                model_store_count += 1;
                model_store = Some(PartitionRange {
                    start_lba: first_lba,
                    sector_count: last_lba
                        .checked_sub(first_lba)
                        .and_then(|sectors| sectors.checked_add(1))
                        .ok_or(GptError::PartitionBounds)?,
                });
            }
        }
    }
    if crc32_finish(crc) != primary.entries_crc {
        return Err(GptError::EntryChecksum);
    }

    let mut backup_crc = !0u32;
    for sector_index in 0..ENTRY_ARRAY_SECTORS {
        let mut bytes = [0u8; SECTOR_SIZE];
        read_sector(backup.entries_lba + sector_index, &mut bytes).map_err(|()| GptError::Read)?;
        let mut primary_copy = [0u8; SECTOR_SIZE];
        read_sector(primary.entries_lba + sector_index, &mut primary_copy)
            .map_err(|()| GptError::Read)?;
        if !bytes_equal(&bytes, &primary_copy) {
            return Err(GptError::EntryArray);
        }
        backup_crc = crc32_update(backup_crc, &bytes);
    }
    if crc32_finish(backup_crc) != primary.entries_crc {
        return Err(GptError::EntryChecksum);
    }
    if user_data_count == 0 {
        return Err(GptError::MissingUserData);
    }
    if user_data_count != 1 {
        return Err(GptError::MultipleUserData);
    }
    if model_store_count > 1 {
        return Err(GptError::MultipleModelStore);
    }
    // An ambiguous slot type exposes neither copy for writing.
    let slot = |index: usize| {
        (system_slot_counts[index] == 1)
            .then_some(system_slots[index])
            .flatten()
    };
    Ok(PartitionLayout {
        user_data: user_data.ok_or(GptError::MissingUserData)?,
        model_store,
        system_a: slot(0),
        system_b: slot(1),
    })
}

fn validate_protective_mbr(mbr: &[u8; SECTOR_SIZE], disk_sectors: u64) -> Result<(), GptError> {
    if mbr[510] != 0x55 || mbr[511] != 0xaa {
        return Err(GptError::ProtectiveMbr);
    }
    for index in 0..4 {
        let offset = MBR_PARTITION_OFFSET + index * 16;
        let entry = &mbr[offset..offset + 16];
        if index == 0 {
            let expected_size = u32::try_from(disk_sectors - 1).unwrap_or(u32::MAX);
            if entry[4] != 0xee || read_u32(entry, 8) != 1 || read_u32(entry, 12) != expected_size {
                return Err(GptError::ProtectiveMbr);
            }
        } else if entry.iter().any(|byte| *byte != 0) {
            return Err(GptError::ProtectiveMbr);
        }
    }
    Ok(())
}

fn parse_header(bytes: &mut [u8; SECTOR_SIZE]) -> Result<Header, GptError> {
    if !bytes_equal(&bytes[..8], HEADER_SIGNATURE)
        || read_u32(bytes, 8) != GPT_REVISION_1_0
        || read_u32(bytes, 12) < GPT_HEADER_MIN_SIZE
        || read_u32(bytes, 12) as usize > SECTOR_SIZE
        || read_u32(bytes, 20) != 0
        || read_u32(bytes, 80) as usize != ENTRY_COUNT
        || read_u32(bytes, 84) as usize != ENTRY_SIZE
    {
        return Err(GptError::Header);
    }
    let header_size = read_u32(bytes, 12) as usize;
    let expected_crc = read_u32(bytes, 16);
    bytes[16..20].fill(0);
    let actual_crc = crc32(&bytes[..header_size]);
    if actual_crc != expected_crc {
        return Err(GptError::HeaderChecksum);
    }
    let mut disk_guid = [0u8; 16];
    disk_guid.copy_from_slice(&bytes[56..72]);
    if guid_is_zero(&disk_guid) {
        return Err(GptError::Header);
    }
    Ok(Header {
        current_lba: read_u64(bytes, 24),
        backup_lba: read_u64(bytes, 32),
        first_usable_lba: read_u64(bytes, 40),
        last_usable_lba: read_u64(bytes, 48),
        disk_guid,
        entries_lba: read_u64(bytes, 72),
        entries_crc: read_u32(bytes, 88),
    })
}

fn guid_is_zero(guid: &[u8; 16]) -> bool {
    let mut combined = 0u8;
    for byte in guid {
        combined |= *byte;
    }
    combined == 0
}

fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..left.len() {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn crc32(bytes: &[u8]) -> u32 {
    crc32_finish(crc32_update(!0u32, bytes))
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    crc
}

fn crc32_finish(crc: u32) -> u32 {
    !crc
}

#[cfg(test)]
mod tests {
    use std::vec;
    use std::vec::Vec;

    use super::{
        find_partitions, find_user_data, GptError, ENTRY_ARRAY_SECTORS, MODEL_STORE_TYPE_GUID,
        SECTOR_SIZE, USER_DATA_TYPE_GUID,
    };

    const DISK_SECTORS: u64 = 128;
    const DATA_START: u64 = 40;
    const DATA_END: u64 = 80;
    const MODEL_START: u64 = 82;
    const MODEL_END: u64 = 88;

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    fn test_disk() -> Vec<u8> {
        let mut disk = vec![0u8; DISK_SECTORS as usize * SECTOR_SIZE];
        disk[510..512].copy_from_slice(&[0x55, 0xaa]);
        disk[450] = 0xee;
        write_u32(&mut disk, 454, 1);
        write_u32(&mut disk, 458, DISK_SECTORS as u32 - 1);

        let entries_start = 2 * SECTOR_SIZE;
        let data = &mut disk[entries_start..entries_start + 128];
        data[..16].copy_from_slice(&USER_DATA_TYPE_GUID);
        data[16..32].copy_from_slice(&[3; 16]);
        write_u64(data, 32, DATA_START);
        write_u64(data, 40, DATA_END);
        let model = &mut disk[entries_start + 128..entries_start + 256];
        model[..16].copy_from_slice(&MODEL_STORE_TYPE_GUID);
        model[16..32].copy_from_slice(&[5; 16]);
        write_u64(model, 32, MODEL_START);
        write_u64(model, 40, MODEL_END);
        let entries_crc = crc32(
            &disk[entries_start..entries_start + (ENTRY_ARRAY_SECTORS as usize * SECTOR_SIZE)],
        );
        let backup_entries_lba = DISK_SECTORS - ENTRY_ARRAY_SECTORS - 1;
        let backup_entries_start = backup_entries_lba as usize * SECTOR_SIZE;
        let entries = disk
            [entries_start..entries_start + ENTRY_ARRAY_SECTORS as usize * SECTOR_SIZE]
            .to_vec();
        disk[backup_entries_start..backup_entries_start + entries.len()].copy_from_slice(&entries);

        write_header(&mut disk, 1, DISK_SECTORS - 1, 2, entries_crc);
        write_header(
            &mut disk,
            DISK_SECTORS - 1,
            1,
            backup_entries_lba,
            entries_crc,
        );
        disk
    }

    fn write_header(
        disk: &mut [u8],
        lba: u64,
        backup_lba: u64,
        entries_lba: u64,
        entries_crc: u32,
    ) {
        let start = lba as usize * SECTOR_SIZE;
        let header = &mut disk[start..start + SECTOR_SIZE];
        header[..8].copy_from_slice(b"EFI PART");
        write_u32(header, 8, 0x0001_0000);
        write_u32(header, 12, 92);
        write_u64(header, 24, lba);
        write_u64(header, 32, backup_lba);
        write_u64(header, 40, 34);
        write_u64(header, 48, DISK_SECTORS - 34);
        header[56..72].copy_from_slice(&[9; 16]);
        write_u64(header, 72, entries_lba);
        write_u32(header, 80, 128);
        write_u32(header, 84, 128);
        write_u32(header, 88, entries_crc);
        let checksum = crc32(&header[..92]);
        write_u32(header, 16, checksum);
    }

    fn find(disk: &[u8], disk_sectors: u64) -> Result<super::PartitionRange, GptError> {
        find_user_data(
            |lba, output| {
                let start = usize::try_from(lba).map_err(|_| ())? * SECTOR_SIZE;
                let sector = disk.get(start..start + SECTOR_SIZE).ok_or(())?;
                output.copy_from_slice(sector);
                Ok(())
            },
            disk_sectors,
        )
    }

    fn find_layout(disk: &[u8], disk_sectors: u64) -> Result<super::PartitionLayout, GptError> {
        find_partitions(
            |lba, output| {
                let start = usize::try_from(lba).map_err(|_| ())? * SECTOR_SIZE;
                let sector = disk.get(start..start + SECTOR_SIZE).ok_or(())?;
                output.copy_from_slice(sector);
                Ok(())
            },
            disk_sectors,
        )
    }

    #[test]
    fn returns_only_a_checksum_valid_user_data_extent() {
        let disk = test_disk();
        assert_eq!(
            find(&disk, DISK_SECTORS),
            Ok(super::PartitionRange {
                start_lba: DATA_START,
                sector_count: DATA_END - DATA_START + 1,
            })
        );
    }

    #[test]
    fn exposes_the_model_store_as_a_separate_validated_extent() {
        let disk = test_disk();
        assert_eq!(
            find_layout(&disk, DISK_SECTORS),
            Ok(super::PartitionLayout {
                user_data: super::PartitionRange {
                    start_lba: DATA_START,
                    sector_count: DATA_END - DATA_START + 1,
                },
                model_store: Some(super::PartitionRange {
                    start_lba: MODEL_START,
                    sector_count: MODEL_END - MODEL_START + 1,
                }),
                system_a: None,
                system_b: None,
            })
        );
    }

    #[test]
    fn keeps_legacy_user_data_images_valid_without_a_model_store() {
        let mut disk = test_disk();
        disk[2 * SECTOR_SIZE + 128..2 * SECTOR_SIZE + 256].fill(0);
        refresh_entries_crc(&mut disk);
        assert_eq!(find_layout(&disk, DISK_SECTORS).unwrap().model_store, None);
        assert!(find(&disk, DISK_SECTORS).is_ok());
    }

    fn add_entry(
        disk: &mut [u8],
        index: usize,
        type_guid: [u8; 16],
        unique: u8,
        first: u64,
        last: u64,
    ) {
        let entry = 2 * SECTOR_SIZE + index * super::ENTRY_SIZE;
        disk[entry..entry + 16].copy_from_slice(&type_guid);
        disk[entry + 16..entry + 32].copy_from_slice(&[unique; 16]);
        write_u64(disk, entry + 32, first);
        write_u64(disk, entry + 40, last);
        refresh_entries_crc(disk);
    }

    #[test]
    fn exposes_unique_system_slots_only() {
        let mut disk = test_disk();
        add_entry(&mut disk, 2, super::SYSTEM_B_TYPE_GUID, 7, 90, 92);
        let layout = find_layout(&disk, DISK_SECTORS).unwrap();
        assert_eq!(layout.system_a, None);
        assert_eq!(
            layout.system_b,
            Some(super::PartitionRange {
                start_lba: 90,
                sector_count: 3,
            })
        );
        // A second System B entry makes the slot ambiguous: neither is
        // exposed.
        add_entry(&mut disk, 3, super::SYSTEM_B_TYPE_GUID, 8, 93, 94);
        assert_eq!(find_layout(&disk, DISK_SECTORS).unwrap().system_b, None);
    }

    #[test]
    fn rejects_multiple_model_store_entries() {
        let mut disk = test_disk();
        let second_entry = 2 * SECTOR_SIZE + 2 * super::ENTRY_SIZE;
        disk[second_entry..second_entry + 16].copy_from_slice(&MODEL_STORE_TYPE_GUID);
        disk[second_entry + 16..second_entry + 32].copy_from_slice(&[6; 16]);
        write_u64(&mut disk, second_entry + 32, 90);
        write_u64(&mut disk, second_entry + 40, 92);
        refresh_entries_crc(&mut disk);
        assert_eq!(
            find_layout(&disk, DISK_SECTORS),
            Err(GptError::MultipleModelStore)
        );
    }

    #[test]
    fn rejects_corrupt_primary_backup_and_partition_arrays() {
        let mut primary = test_disk();
        primary[SECTOR_SIZE + 56] ^= 1;
        assert_eq!(find(&primary, DISK_SECTORS), Err(GptError::HeaderChecksum));

        let mut backup = test_disk();
        backup[(DISK_SECTORS as usize - 1) * SECTOR_SIZE + 56] ^= 1;
        assert_eq!(find(&backup, DISK_SECTORS), Err(GptError::HeaderChecksum));

        let mut entries = test_disk();
        entries[2 * SECTOR_SIZE + 48] ^= 1;
        assert_eq!(find(&entries, DISK_SECTORS), Err(GptError::EntryChecksum));
    }

    #[test]
    fn rejects_missing_overlapping_and_out_of_usable_data_entries() {
        let mut missing = test_disk();
        missing[2 * SECTOR_SIZE..2 * SECTOR_SIZE + super::ENTRY_SIZE].fill(0);
        refresh_entries_crc(&mut missing);
        assert_eq!(find(&missing, DISK_SECTORS), Err(GptError::MissingUserData));

        let mut overlapping = test_disk();
        let second_entry = 2 * SECTOR_SIZE + super::ENTRY_SIZE;
        overlapping[second_entry..second_entry + 16].copy_from_slice(&[4; 16]);
        overlapping[second_entry + 16..second_entry + 32].copy_from_slice(&[5; 16]);
        write_u64(&mut overlapping, second_entry + 32, DATA_START - 1);
        write_u64(&mut overlapping, second_entry + 40, DATA_START + 1);
        refresh_entries_crc(&mut overlapping);
        assert_eq!(
            find(&overlapping, DISK_SECTORS),
            Err(GptError::PartitionOverlap)
        );

        let mut out_of_bounds = test_disk();
        write_u64(&mut out_of_bounds, 2 * SECTOR_SIZE + 40, DISK_SECTORS - 1);
        refresh_entries_crc(&mut out_of_bounds);
        assert_eq!(
            find(&out_of_bounds, DISK_SECTORS),
            Err(GptError::PartitionBounds)
        );
    }

    fn refresh_entries_crc(disk: &mut [u8]) {
        let primary_start = 2 * SECTOR_SIZE;
        let table_len = ENTRY_ARRAY_SECTORS as usize * SECTOR_SIZE;
        let entries_crc = crc32(&disk[primary_start..primary_start + table_len]);
        let backup_start = (DISK_SECTORS - ENTRY_ARRAY_SECTORS - 1) as usize * SECTOR_SIZE;
        let primary_table = disk[primary_start..primary_start + table_len].to_vec();
        disk[backup_start..backup_start + table_len].copy_from_slice(&primary_table);
        for header_lba in [1, DISK_SECTORS - 1] {
            let start = header_lba as usize * SECTOR_SIZE;
            let header = &mut disk[start..start + SECTOR_SIZE];
            write_u32(header, 88, entries_crc);
            write_u32(header, 16, 0);
            let checksum = crc32(&header[..92]);
            write_u32(header, 16, checksum);
        }
    }
}
