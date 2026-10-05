use std::io::{Read, Seek, SeekFrom, Write};

pub const SECTOR_SIZE: u64 = 512;
pub const GPT_ENTRY_COUNT: u32 = 128;
pub const GPT_ENTRY_SIZE: u32 = 128;
pub const GPT_ENTRY_ARRAY_SECTORS: u64 =
    (GPT_ENTRY_COUNT as u64 * GPT_ENTRY_SIZE as u64) / SECTOR_SIZE;
pub const GPT_FIRST_USABLE_LBA: u64 = 2 + GPT_ENTRY_ARRAY_SECTORS;
pub const NAGI_USER_DATA_TYPE_GUID: [u8; 16] = nagi_guid(1, 3);
pub const USER_DATA_PARTITION_GUID: [u8; 16] = nagi_guid(2, 3);
pub const SYSTEM_A_TYPE_GUID: [u8; 16] = nagi_guid(1, 1);
pub const SYSTEM_B_TYPE_GUID: [u8; 16] = nagi_guid(1, 2);
pub const RECOVERY_TYPE_GUID: [u8; 16] = nagi_guid(1, 4);
pub const MODEL_STORE_TYPE_GUID: [u8; 16] = nagi_guid(1, 5);
pub const ESP_PARTITION_GUID: [u8; 16] = nagi_guid(2, 0);
pub const SYSTEM_A_PARTITION_GUID: [u8; 16] = nagi_guid(2, 1);
pub const SYSTEM_B_PARTITION_GUID: [u8; 16] = nagi_guid(2, 2);
pub const RECOVERY_PARTITION_GUID: [u8; 16] = nagi_guid(2, 4);
pub const MODEL_STORE_PARTITION_GUID: [u8; 16] = nagi_guid(2, 5);
const EFI_SYSTEM_PARTITION_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];
const DISK_GUID: [u8; 16] = [
    0x03, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 1,
];
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const GPT_REVISION_1_0: u32 = 0x0001_0000;
const GPT_HEADER_SIZE: u32 = 92;
const GPT_HEADER_SIGNATURE: &[u8; 8] = b"EFI PART";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartitionRange {
    pub start_lba: u64,
    pub sector_count: u64,
}

#[derive(Clone, Copy)]
struct ParsedHeader {
    current_lba: u64,
    backup_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    disk_guid: [u8; 16],
    entries_lba: u64,
    entries_crc: u32,
}

#[derive(Clone, Copy)]
struct ParsedPartition {
    first_lba: u64,
    last_lba: u64,
    unique_guid: [u8; 16],
}

pub fn read_user_data_partition<R: Read + Seek>(
    disk: &mut R,
    disk_sectors: u64,
) -> Result<PartitionRange, String> {
    if disk_sectors < GPT_ENTRY_ARRAY_SECTORS + 4 {
        return Err("GPT disk is too small".to_owned());
    }
    let mbr = read_sector(disk, 0)?;
    validate_protective_mbr(&mbr, disk_sectors)?;
    let primary_bytes = read_sector(disk, 1)?;
    let primary = parse_header(primary_bytes)?;
    let backup_lba = disk_sectors - 1;
    if primary.current_lba != 1
        || primary.backup_lba != backup_lba
        || primary.entries_lba != 2
        || primary.first_usable_lba < GPT_FIRST_USABLE_LBA
        || primary.last_usable_lba >= backup_lba - GPT_ENTRY_ARRAY_SECTORS
        || primary.first_usable_lba > primary.last_usable_lba
    {
        return Err("invalid primary GPT bounds".to_owned());
    }

    let backup = parse_header(read_sector(disk, backup_lba)?)?;
    if backup.current_lba != backup_lba
        || backup.backup_lba != 1
        || backup.entries_lba != backup_lba - GPT_ENTRY_ARRAY_SECTORS
        || backup.first_usable_lba != primary.first_usable_lba
        || backup.last_usable_lba != primary.last_usable_lba
        || backup.disk_guid != primary.disk_guid
        || backup.entries_crc != primary.entries_crc
    {
        return Err("primary and backup GPT headers disagree".to_owned());
    }

    let entries_length = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE) as usize;
    let mut primary_entries = vec![0u8; entries_length];
    read_at_lba(disk, primary.entries_lba, &mut primary_entries)?;
    let mut backup_entries = vec![0u8; entries_length];
    read_at_lba(disk, backup.entries_lba, &mut backup_entries)?;
    if crc32(&primary_entries) != primary.entries_crc
        || crc32(&backup_entries) != primary.entries_crc
        || primary_entries != backup_entries
    {
        return Err("GPT partition entry arrays are corrupt or disagree".to_owned());
    }

    let mut partitions: Vec<ParsedPartition> = Vec::new();
    let mut user_data = None;
    let mut user_data_count = 0usize;
    for entry in primary_entries.chunks_exact(GPT_ENTRY_SIZE as usize) {
        let type_guid: [u8; 16] = entry[..16].try_into().expect("fixed-size GUID");
        if type_guid == [0; 16] {
            if entry.iter().any(|byte| *byte != 0) {
                return Err("unused GPT entry contains data".to_owned());
            }
            continue;
        }
        let unique_guid: [u8; 16] = entry[16..32].try_into().expect("fixed-size GUID");
        let first_lba = read_u64(entry, 32);
        let last_lba = read_u64(entry, 40);
        if unique_guid == [0; 16]
            || first_lba < primary.first_usable_lba
            || first_lba > last_lba
            || last_lba > primary.last_usable_lba
        {
            return Err("GPT partition has invalid identity or bounds".to_owned());
        }
        for other in &partitions {
            if unique_guid == other.unique_guid {
                return Err("GPT partition unique GUID repeats".to_owned());
            }
            if first_lba <= other.last_lba && other.first_lba <= last_lba {
                return Err("GPT partitions overlap".to_owned());
            }
        }
        partitions.push(ParsedPartition {
            first_lba,
            last_lba,
            unique_guid,
        });
        if type_guid == NAGI_USER_DATA_TYPE_GUID {
            user_data_count += 1;
            user_data = Some(PartitionRange {
                start_lba: first_lba,
                sector_count: last_lba - first_lba + 1,
            });
        }
    }
    if user_data_count != 1 {
        return Err("GPT must contain exactly one Nagi User Data partition".to_owned());
    }
    user_data.ok_or_else(|| "GPT has no Nagi User Data partition".to_owned())
}

fn validate_protective_mbr(
    mbr: &[u8; SECTOR_SIZE as usize],
    disk_sectors: u64,
) -> Result<(), String> {
    if mbr[510..512] != [0x55, 0xaa] {
        return Err("invalid protective MBR signature".to_owned());
    }
    for index in 0..4 {
        let start = 446 + index * 16;
        let entry = &mbr[start..start + 16];
        if index == 0 {
            let expected_size = u32::try_from(disk_sectors - 1).unwrap_or(u32::MAX);
            if entry[4] != 0xee || read_u32(entry, 8) != 1 || read_u32(entry, 12) != expected_size {
                return Err("invalid protective MBR partition".to_owned());
            }
        } else if entry.iter().any(|byte| *byte != 0) {
            return Err("unexpected legacy MBR partition".to_owned());
        }
    }
    Ok(())
}

fn parse_header(mut bytes: [u8; SECTOR_SIZE as usize]) -> Result<ParsedHeader, String> {
    let header_size = read_u32(&bytes, 12) as usize;
    if &bytes[..8] != GPT_HEADER_SIGNATURE
        || read_u32(&bytes, 8) != GPT_REVISION_1_0
        || !(GPT_HEADER_SIZE as usize..=SECTOR_SIZE as usize).contains(&header_size)
        || read_u32(&bytes, 20) != 0
        || read_u32(&bytes, 80) != GPT_ENTRY_COUNT
        || read_u32(&bytes, 84) != GPT_ENTRY_SIZE
    {
        return Err("invalid GPT header".to_owned());
    }
    let expected_crc = read_u32(&bytes, 16);
    bytes[16..20].fill(0);
    if crc32(&bytes[..header_size]) != expected_crc {
        return Err("GPT header checksum mismatch".to_owned());
    }
    let disk_guid: [u8; 16] = bytes[56..72].try_into().expect("fixed-size GUID");
    if disk_guid == [0; 16] {
        return Err("GPT disk GUID is zero".to_owned());
    }
    Ok(ParsedHeader {
        current_lba: read_u64(&bytes, 24),
        backup_lba: read_u64(&bytes, 32),
        first_usable_lba: read_u64(&bytes, 40),
        last_usable_lba: read_u64(&bytes, 48),
        disk_guid,
        entries_lba: read_u64(&bytes, 72),
        entries_crc: read_u32(&bytes, 88),
    })
}

fn read_sector<R: Read + Seek>(
    disk: &mut R,
    lba: u64,
) -> Result<[u8; SECTOR_SIZE as usize], String> {
    let mut sector = [0u8; SECTOR_SIZE as usize];
    read_at_lba(disk, lba, &mut sector)?;
    Ok(sector)
}

fn read_at_lba<R: Read + Seek>(disk: &mut R, lba: u64, bytes: &mut [u8]) -> Result<(), String> {
    let offset = lba
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| "GPT offset overflow".to_owned())?;
    disk.seek(SeekFrom::Start(offset))
        .and_then(|_| disk.read_exact(bytes))
        .map_err(|error| format!("cannot read GPT at LBA {lba}: {error}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GptPartition {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
    pub name: &'static str,
}

pub const fn nagi_guid(prefix: u8, id: u8) -> [u8; 16] {
    [
        prefix, 0x47, 0x41, 0x4e, 0x01, 0x00, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, id,
    ]
}

pub fn nagi_partition(id: u8, first_lba: u64, last_lba: u64, name: &'static str) -> GptPartition {
    GptPartition {
        type_guid: nagi_guid(1, id),
        unique_guid: nagi_guid(2, id),
        first_lba,
        last_lba,
        attributes: 0,
        name,
    }
}

pub fn write_gpt<W: Write + Seek>(
    disk: &mut W,
    disk_sectors: u64,
    partitions: &[GptPartition],
) -> Result<(), String> {
    let last_usable_lba = disk_sectors
        .checked_sub(GPT_ENTRY_ARRAY_SECTORS + 2)
        .ok_or_else(|| "GPT disk is too small for primary and backup metadata".to_owned())?;
    validate_partitions(partitions, last_usable_lba)?;

    let mut entries = vec![0u8; (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE) as usize];
    for (index, partition) in partitions.iter().enumerate() {
        let offset = index * GPT_ENTRY_SIZE as usize;
        let entry = &mut entries[offset..offset + GPT_ENTRY_SIZE as usize];
        entry[0..16].copy_from_slice(&partition.type_guid);
        entry[16..32].copy_from_slice(&partition.unique_guid);
        put_u64(entry, 32, partition.first_lba);
        put_u64(entry, 40, partition.last_lba);
        put_u64(entry, 48, partition.attributes);
        write_partition_name(entry, partition.name);
    }
    let entries_crc = crc32(&entries);
    let backup_entries_lba = disk_sectors - GPT_ENTRY_ARRAY_SECTORS - 1;
    let backup_header_lba = disk_sectors - 1;

    write_protective_mbr(disk, disk_sectors)?;
    write_sectors(disk, 2, &entries)?;
    write_sectors(disk, backup_entries_lba, &entries)?;
    let primary_header = build_header(1, backup_header_lba, 2, last_usable_lba, entries_crc);
    let backup_header = build_header(
        backup_header_lba,
        1,
        backup_entries_lba,
        last_usable_lba,
        entries_crc,
    );
    write_sectors(disk, 1, &primary_header)?;
    write_sectors(disk, backup_header_lba, &backup_header)
}

pub fn user_data_partition(first_lba: u64, last_lba: u64, unique_guid: [u8; 16]) -> GptPartition {
    GptPartition {
        type_guid: NAGI_USER_DATA_TYPE_GUID,
        unique_guid,
        first_lba,
        last_lba,
        attributes: 0,
        name: "User Data",
    }
}

pub fn efi_system_partition(first_lba: u64, last_lba: u64, unique_guid: [u8; 16]) -> GptPartition {
    GptPartition {
        type_guid: EFI_SYSTEM_PARTITION_TYPE_GUID,
        unique_guid,
        first_lba,
        last_lba,
        attributes: 0,
        name: "ESP",
    }
}

fn validate_partitions(partitions: &[GptPartition], last_usable_lba: u64) -> Result<(), String> {
    if partitions.is_empty() || partitions.len() > GPT_ENTRY_COUNT as usize {
        return Err("GPT requires between one and 128 partitions".to_owned());
    }
    for (index, partition) in partitions.iter().enumerate() {
        if partition.type_guid == [0; 16] || partition.unique_guid == [0; 16] {
            return Err("GPT partition GUIDs must be nonzero".to_owned());
        }
        if partition.first_lba < GPT_FIRST_USABLE_LBA
            || partition.first_lba > partition.last_lba
            || partition.last_lba > last_usable_lba
        {
            return Err(format!(
                "GPT partition {} has invalid bounds",
                partition.name
            ));
        }
        if partition.name.encode_utf16().count() > 36 {
            return Err(format!(
                "GPT partition name is too long: {}",
                partition.name
            ));
        }
        for other in &partitions[..index] {
            if partition.unique_guid == other.unique_guid {
                return Err("GPT partition unique GUIDs must not repeat".to_owned());
            }
            if partition.first_lba <= other.last_lba && other.first_lba <= partition.last_lba {
                return Err(format!(
                    "GPT partitions {} and {} overlap",
                    partition.name, other.name
                ));
            }
        }
    }
    Ok(())
}

fn write_protective_mbr<W: Write + Seek>(disk: &mut W, disk_sectors: u64) -> Result<(), String> {
    let mut mbr = [0u8; SECTOR_SIZE as usize];
    let record = &mut mbr[446..462];
    record[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
    record[4] = 0xee;
    record[5..8].copy_from_slice(&[0xfe, 0xff, 0xff]);
    put_u32(record, 8, 1);
    put_u32(
        record,
        12,
        u32::try_from(disk_sectors - 1).unwrap_or(u32::MAX),
    );
    mbr[510..512].copy_from_slice(&[0x55, 0xaa]);
    write_sectors(disk, 0, &mbr)
}

fn build_header(
    current_lba: u64,
    backup_lba: u64,
    entries_lba: u64,
    last_usable_lba: u64,
    entries_crc: u32,
) -> [u8; SECTOR_SIZE as usize] {
    let mut header = [0u8; SECTOR_SIZE as usize];
    header[0..8].copy_from_slice(GPT_SIGNATURE);
    put_u32(&mut header, 8, GPT_REVISION_1_0);
    put_u32(&mut header, 12, GPT_HEADER_SIZE);
    put_u64(&mut header, 24, current_lba);
    put_u64(&mut header, 32, backup_lba);
    put_u64(&mut header, 40, GPT_FIRST_USABLE_LBA);
    put_u64(&mut header, 48, last_usable_lba);
    header[56..72].copy_from_slice(&DISK_GUID);
    put_u64(&mut header, 72, entries_lba);
    put_u32(&mut header, 80, GPT_ENTRY_COUNT);
    put_u32(&mut header, 84, GPT_ENTRY_SIZE);
    put_u32(&mut header, 88, entries_crc);
    let header_crc = crc32(&header[..GPT_HEADER_SIZE as usize]);
    put_u32(&mut header, 16, header_crc);
    header
}

fn write_partition_name(entry: &mut [u8], name: &str) {
    for (index, unit) in name.encode_utf16().enumerate() {
        let offset = 56 + index * 2;
        entry[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
    }
}

fn write_sectors<W: Write + Seek>(disk: &mut W, lba: u64, bytes: &[u8]) -> Result<(), String> {
    let offset = lba
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| "GPT offset overflow".to_owned())?;
    disk.seek(SeekFrom::Start(offset))
        .and_then(|_| disk.write_all(bytes))
        .map_err(|error| format!("cannot write GPT at LBA {lba}: {error}"))
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("four-byte integer"),
    )
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("eight-byte integer"),
    )
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        crc32, efi_system_partition, read_user_data_partition, user_data_partition, write_gpt,
        GPT_ENTRY_ARRAY_SECTORS, GPT_ENTRY_COUNT, GPT_ENTRY_SIZE, GPT_FIRST_USABLE_LBA,
        SECTOR_SIZE,
    };

    const DISK_SECTORS: u64 = 32_768;

    #[test]
    fn writes_protective_mbr_primary_and_backup_gpt() {
        let mut image = Cursor::new(vec![0; (DISK_SECTORS * SECTOR_SIZE) as usize]);
        let partitions = [
            efi_system_partition(2048, 4095, [1; 16]),
            user_data_partition(4096, DISK_SECTORS - 34, [2; 16]),
        ];
        write_gpt(&mut image, DISK_SECTORS, &partitions).expect("write GPT");
        let bytes = image.get_ref();

        assert_eq!(&bytes[510..512], &[0x55, 0xaa]);
        assert_eq!(bytes[450], 0xee);
        assert_eq!(u32::from_le_bytes(bytes[454..458].try_into().unwrap()), 1);

        let primary = &bytes[SECTOR_SIZE as usize..2 * SECTOR_SIZE as usize];
        let backup_offset = ((DISK_SECTORS - 1) * SECTOR_SIZE) as usize;
        let backup = &bytes[backup_offset..backup_offset + SECTOR_SIZE as usize];
        assert_eq!(&primary[0..8], b"EFI PART");
        assert_eq!(u64::from_le_bytes(primary[24..32].try_into().unwrap()), 1);
        assert_eq!(
            u64::from_le_bytes(backup[24..32].try_into().unwrap()),
            DISK_SECTORS - 1
        );
        assert_eq!(u64::from_le_bytes(primary[40..48].try_into().unwrap()), 34);
        assert_eq!(
            u64::from_le_bytes(primary[48..56].try_into().unwrap()),
            DISK_SECTORS - 34
        );

        let mut checked_primary = primary.to_vec();
        checked_primary[16..20].fill(0);
        assert_eq!(
            u32::from_le_bytes(primary[16..20].try_into().unwrap()),
            crc32(&checked_primary[..92])
        );
        let entries_length = (GPT_ENTRY_COUNT * GPT_ENTRY_SIZE) as usize;
        let primary_entries =
            &bytes[2 * SECTOR_SIZE as usize..2 * SECTOR_SIZE as usize + entries_length];
        let backup_entries_start =
            ((DISK_SECTORS - GPT_ENTRY_ARRAY_SECTORS - 1) * SECTOR_SIZE) as usize;
        let backup_entries = &bytes[backup_entries_start..backup_entries_start + entries_length];
        assert_eq!(primary_entries, backup_entries);
        assert_eq!(
            u32::from_le_bytes(primary[88..92].try_into().unwrap()),
            crc32(primary_entries)
        );
        assert_eq!(&primary_entries[16..32], &[1; 16]);
        assert_eq!(&primary_entries[128 + 16..128 + 32], &[2; 16]);
        assert_eq!(
            read_user_data_partition(&mut image, DISK_SECTORS),
            Ok(super::PartitionRange {
                start_lba: 4096,
                sector_count: DISK_SECTORS - 34 - 4096 + 1,
            })
        );
    }

    #[test]
    fn reader_rejects_corrupt_or_disagreeing_gpt_metadata() {
        let mut image = Cursor::new(vec![0; (DISK_SECTORS * SECTOR_SIZE) as usize]);
        write_gpt(
            &mut image,
            DISK_SECTORS,
            &[user_data_partition(2048, DISK_SECTORS - 34, [7; 16])],
        )
        .expect("write GPT");

        let mut corrupt = Cursor::new(image.into_inner());
        corrupt.get_mut()[SECTOR_SIZE as usize + 56] ^= 1;
        assert!(read_user_data_partition(&mut corrupt, DISK_SECTORS)
            .unwrap_err()
            .contains("checksum"));

        let mut disagreeing = Cursor::new(vec![0; (DISK_SECTORS * SECTOR_SIZE) as usize]);
        write_gpt(
            &mut disagreeing,
            DISK_SECTORS,
            &[user_data_partition(2048, DISK_SECTORS - 34, [7; 16])],
        )
        .expect("write GPT");
        let backup_entries = ((DISK_SECTORS - GPT_ENTRY_ARRAY_SECTORS - 1) * SECTOR_SIZE) as usize;
        disagreeing.get_mut()[backup_entries + 128 + 40] ^= 1;
        assert!(read_user_data_partition(&mut disagreeing, DISK_SECTORS)
            .unwrap_err()
            .contains("disagree"));
    }

    #[test]
    fn rejects_overlapping_and_out_of_bounds_partitions() {
        let mut image = Cursor::new(vec![0; (DISK_SECTORS * SECTOR_SIZE) as usize]);
        let overlapping = [
            efi_system_partition(2048, 4095, [1; 16]),
            user_data_partition(4095, DISK_SECTORS - 34, [2; 16]),
        ];
        assert!(write_gpt(&mut image, DISK_SECTORS, &overlapping)
            .unwrap_err()
            .contains("overlap"));

        let out_of_bounds = [user_data_partition(
            GPT_FIRST_USABLE_LBA,
            DISK_SECTORS - 1,
            [3; 16],
        )];
        assert!(write_gpt(&mut image, DISK_SECTORS, &out_of_bounds)
            .unwrap_err()
            .contains("invalid bounds"));
    }

    #[test]
    fn uses_the_nagi_data_partition_type_guid() {
        let partition = user_data_partition(2048, 4095, [8; 16]);
        assert_eq!(partition.type_guid, super::NAGI_USER_DATA_TYPE_GUID);
        assert_eq!(
            partition.type_guid,
            [1, 0x47, 0x41, 0x4e, 1, 0, 0x41, 0x4e, 0x47, 0x49, 0, 0, 0, 0, 0, 3]
        );
    }
}
