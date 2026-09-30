use std::io::{Seek, SeekFrom, Write};

const SECTOR_SIZE: u64 = 512;
const SECTORS_PER_CLUSTER: u32 = 8;
const RESERVED_SECTORS: u32 = 32;
const FAT_COUNT: u32 = 2;
const FSINFO_SECTOR: u32 = 1;
const BACKUP_BOOT_SECTOR: u32 = 6;
const FAT32_EOC: u32 = 0x0fff_ffff;
const MAX_FAT32_CLUSTER: u32 = 0x0fff_ffef;

#[derive(Clone, Copy)]
pub struct VolumeFile<'a> {
    pub path: &'static str,
    pub contents: &'a [u8],
}

#[derive(Clone, Copy)]
pub struct FilePlacement {
    pub first_cluster: u32,
    pub cluster_count: u32,
}

struct Directory {
    path: String,
    first_cluster: u32,
    parent_cluster: u32,
    entries: Vec<DirectoryEntry>,
}

#[derive(Clone, Copy)]
enum DirectoryEntry {
    Directory {
        name: [u8; 11],
        first_cluster: u32,
    },
    File {
        name: [u8; 11],
        first_cluster: u32,
        size: u32,
    },
}

struct PendingFile<'a> {
    first_cluster: u32,
    cluster_count: u32,
    contents: &'a [u8],
}

pub fn format_partition<W: Write + Seek>(
    disk: &mut W,
    start_lba: u64,
    sector_count: u64,
    volume_label: &str,
    files: &[VolumeFile<'_>],
) -> Result<Vec<FilePlacement>, String> {
    if volume_label.len() > 11 || !volume_label.is_ascii() {
        return Err("FAT32 volume label must be at most 11 ASCII bytes".to_owned());
    }
    let total_sectors = u32::try_from(sector_count)
        .map_err(|_| "FAT32 partition exceeds the 32-bit BPB sector limit".to_owned())?;
    let (fat_sectors, cluster_count) = geometry(total_sectors)?;
    let mut directories = vec![Directory {
        path: String::new(),
        first_cluster: 2,
        parent_cluster: 2,
        entries: Vec::new(),
    }];
    let mut next_cluster = 3u32;
    let mut pending_files = Vec::with_capacity(files.len());
    let mut placements = Vec::with_capacity(files.len());

    for file in files {
        if file.contents.is_empty() {
            return Err(format!("FAT32 file is empty: {}", file.path));
        }
        let size = u32::try_from(file.contents.len())
            .map_err(|_| format!("FAT32 file exceeds 4 GiB: {}", file.path))?;
        let mut components = file.path.split('/').collect::<Vec<_>>();
        let filename = components
            .pop()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("FAT32 file path is empty: {}", file.path))?;
        let file_name = short_name(filename)?;
        if components.iter().any(|component| component.is_empty()) {
            return Err(format!("FAT32 path has an empty directory: {}", file.path));
        }
        let mut directory_path = String::new();
        let mut parent_index = 0usize;
        for component in components {
            if !directory_path.is_empty() {
                directory_path.push('/');
            }
            directory_path.push_str(component);
            let name = short_name(component)?;
            let directory_index = match directories
                .iter()
                .position(|directory| directory.path == directory_path)
            {
                Some(index) => index,
                None => {
                    let first_cluster = allocate_cluster(&mut next_cluster, cluster_count)?;
                    directories[parent_index]
                        .entries
                        .push(DirectoryEntry::Directory {
                            name,
                            first_cluster,
                        });
                    directories.push(Directory {
                        path: directory_path.clone(),
                        first_cluster,
                        parent_cluster: directories[parent_index].first_cluster,
                        entries: Vec::new(),
                    });
                    directories.len() - 1
                }
            };
            parent_index = directory_index;
        }
        if directories[parent_index]
            .entries
            .iter()
            .any(|entry| entry_name(*entry) == file_name)
        {
            return Err(format!("duplicate FAT32 directory entry: {}", file.path));
        }
        let cluster_size = usize::try_from(SECTORS_PER_CLUSTER as u64 * SECTOR_SIZE)
            .expect("fixed FAT32 cluster size fits usize");
        let cluster_count_for_file = u32::try_from(file.contents.len().div_ceil(cluster_size))
            .map_err(|_| format!("FAT32 file has too many clusters: {}", file.path))?;
        let first_cluster =
            allocate_chain(&mut next_cluster, cluster_count_for_file, cluster_count)?;
        directories[parent_index]
            .entries
            .push(DirectoryEntry::File {
                name: file_name,
                first_cluster,
                size,
            });
        pending_files.push(PendingFile {
            first_cluster,
            cluster_count: cluster_count_for_file,
            contents: file.contents,
        });
        placements.push(FilePlacement {
            first_cluster,
            cluster_count: cluster_count_for_file,
        });
    }

    let fat_length = usize::try_from(u64::from(fat_sectors) * SECTOR_SIZE)
        .map_err(|_| "FAT32 table is too large for host memory".to_owned())?;
    let mut fat = vec![0u8; fat_length];
    set_fat_entry(&mut fat, 0, 0x0fff_fff8)?;
    set_fat_entry(&mut fat, 1, FAT32_EOC)?;
    for directory in &directories {
        set_fat_entry(&mut fat, directory.first_cluster, FAT32_EOC)?;
    }
    for file in &pending_files {
        set_fat_chain(&mut fat, file.first_cluster, file.cluster_count)?;
    }

    let data_start_sector = RESERVED_SECTORS
        .checked_add(
            FAT_COUNT
                .checked_mul(fat_sectors)
                .ok_or("FAT32 geometry overflow")?,
        )
        .ok_or("FAT32 geometry overflow")?;
    let root_data_start = absolute_offset(start_lba, u64::from(data_start_sector))?;
    write_boot_sector(disk, start_lba, total_sectors, fat_sectors, volume_label)?;
    write_fsinfo(disk, start_lba, cluster_count, next_cluster)?;
    write_at_lba(
        disk,
        start_lba + u64::from(BACKUP_BOOT_SECTOR),
        &boot_sector(start_lba, total_sectors, fat_sectors, volume_label)?,
    )?;
    write_fsinfo(
        disk,
        start_lba + 1 + u64::from(BACKUP_BOOT_SECTOR),
        cluster_count,
        next_cluster,
    )?;
    for fat_index in 0..FAT_COUNT {
        let fat_start = start_lba + u64::from(RESERVED_SECTORS + fat_index * fat_sectors);
        write_at_lba(disk, fat_start, &fat)?;
    }

    for directory in &directories {
        let mut bytes = vec![0u8; SECTORS_PER_CLUSTER as usize * SECTOR_SIZE as usize];
        let mut offset = 0usize;
        if !directory.path.is_empty() {
            let mut dot = [b' '; 11];
            dot[0] = b'.';
            write_directory_entry(
                &mut bytes[offset..offset + 32],
                dot,
                0x10,
                directory.first_cluster,
                0,
            );
            offset += 32;
            let mut dotdot = [b' '; 11];
            dotdot[..2].copy_from_slice(b"..");
            write_directory_entry(
                &mut bytes[offset..offset + 32],
                dotdot,
                0x10,
                directory.parent_cluster,
                0,
            );
            offset += 32;
        }
        for entry in &directory.entries {
            if offset + 32 > bytes.len() {
                return Err(format!(
                    "FAT32 directory has too many entries: {}",
                    directory.path
                ));
            }
            match *entry {
                DirectoryEntry::Directory {
                    name,
                    first_cluster,
                } => {
                    write_directory_entry(
                        &mut bytes[offset..offset + 32],
                        name,
                        0x10,
                        first_cluster,
                        0,
                    );
                }
                DirectoryEntry::File {
                    name,
                    first_cluster,
                    size,
                } => {
                    write_directory_entry(
                        &mut bytes[offset..offset + 32],
                        name,
                        0x20,
                        first_cluster,
                        size,
                    );
                }
            }
            offset += 32;
        }
        let data_offset = cluster_offset(root_data_start, directory.first_cluster)?;
        write_at(disk, data_offset, &bytes)?;
    }

    let cluster_size = SECTORS_PER_CLUSTER as usize * SECTOR_SIZE as usize;
    for file in &pending_files {
        let mut remaining = file.contents;
        let mut cluster = file.first_cluster;
        while !remaining.is_empty() {
            let data_offset = cluster_offset(root_data_start, cluster)?;
            let count = remaining.len().min(cluster_size);
            write_at(disk, data_offset, &remaining[..count])?;
            remaining = &remaining[count..];
            cluster += 1;
        }
    }
    Ok(placements)
}

fn geometry(total_sectors: u32) -> Result<(u32, u32), String> {
    let mut fat_sectors = 1u32;
    for _ in 0..16 {
        let overhead = RESERVED_SECTORS
            .checked_add(
                FAT_COUNT
                    .checked_mul(fat_sectors)
                    .ok_or("FAT32 geometry overflow")?,
            )
            .ok_or("FAT32 geometry overflow")?;
        if overhead >= total_sectors {
            return Err("FAT32 partition is too small".to_owned());
        }
        let clusters = (total_sectors - overhead) / SECTORS_PER_CLUSTER;
        let required_fat_bytes = u64::from(clusters + 2) * 4;
        let required_fat_sectors = u32::try_from(required_fat_bytes.div_ceil(SECTOR_SIZE))
            .map_err(|_| "FAT32 table size overflow".to_owned())?;
        if required_fat_sectors == fat_sectors {
            if !(65_525..=MAX_FAT32_CLUSTER).contains(&clusters) {
                return Err(format!(
                    "FAT32 cluster count is outside the valid range: {clusters}"
                ));
            }
            return Ok((fat_sectors, clusters));
        }
        fat_sectors = required_fat_sectors;
    }
    Err("FAT32 geometry did not converge".to_owned())
}

fn allocate_cluster(next_cluster: &mut u32, cluster_count: u32) -> Result<u32, String> {
    allocate_chain(next_cluster, 1, cluster_count)
}

fn allocate_chain(next_cluster: &mut u32, count: u32, cluster_count: u32) -> Result<u32, String> {
    if count == 0 {
        return Err("FAT32 file requires at least one cluster".to_owned());
    }
    let end_exclusive = next_cluster
        .checked_add(count)
        .ok_or_else(|| "FAT32 cluster allocation overflow".to_owned())?;
    if end_exclusive > cluster_count + 2 || end_exclusive > MAX_FAT32_CLUSTER {
        return Err("FAT32 volume has insufficient free clusters".to_owned());
    }
    let first = *next_cluster;
    *next_cluster = end_exclusive;
    Ok(first)
}

fn set_fat_chain(fat: &mut [u8], first_cluster: u32, count: u32) -> Result<(), String> {
    for index in 0..count {
        let cluster = first_cluster + index;
        let next = if index + 1 == count {
            FAT32_EOC
        } else {
            cluster + 1
        };
        set_fat_entry(fat, cluster, next)?;
    }
    Ok(())
}

fn set_fat_entry(fat: &mut [u8], cluster: u32, value: u32) -> Result<(), String> {
    let offset = usize::try_from(u64::from(cluster) * 4)
        .map_err(|_| "FAT32 entry offset overflow".to_owned())?;
    let entry = fat
        .get_mut(offset..offset + 4)
        .ok_or_else(|| "FAT32 entry exceeds allocated table".to_owned())?;
    entry.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn entry_name(entry: DirectoryEntry) -> [u8; 11] {
    match entry {
        DirectoryEntry::Directory { name, .. } | DirectoryEntry::File { name, .. } => name,
    }
}

fn short_name(name: &str) -> Result<[u8; 11], String> {
    if !name.is_ascii() || name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(format!("unsupported FAT32 short name: {name}"));
    }
    let mut pieces = name.split('.');
    let base = pieces.next().unwrap_or_default();
    let extension = pieces.next().unwrap_or_default();
    if pieces.next().is_some() || base.is_empty() || base.len() > 8 || extension.len() > 3 {
        return Err(format!("FAT32 path is not an 8.3 name: {name}"));
    }
    const ALLOWED: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_$~!#%&-{}()@'`^";
    if !base
        .bytes()
        .chain(extension.bytes())
        .all(|byte| ALLOWED.as_bytes().contains(&byte.to_ascii_uppercase()))
    {
        return Err(format!(
            "FAT32 name contains unsupported characters: {name}"
        ));
    }
    let mut result = [b' '; 11];
    for (index, byte) in base.bytes().enumerate() {
        result[index] = byte.to_ascii_uppercase();
    }
    for (index, byte) in extension.bytes().enumerate() {
        result[8 + index] = byte.to_ascii_uppercase();
    }
    Ok(result)
}

fn write_boot_sector<W: Write + Seek>(
    disk: &mut W,
    start_lba: u64,
    total_sectors: u32,
    fat_sectors: u32,
    volume_label: &str,
) -> Result<(), String> {
    let boot = boot_sector(start_lba, total_sectors, fat_sectors, volume_label)?;
    write_at_lba(disk, start_lba, &boot)
}

fn boot_sector(
    start_lba: u64,
    total_sectors: u32,
    fat_sectors: u32,
    volume_label: &str,
) -> Result<[u8; 512], String> {
    let mut boot = [0u8; 512];
    boot[..3].copy_from_slice(&[0xeb, 0x58, 0x90]);
    boot[3..11].copy_from_slice(b"NAGIOS  ");
    put_u16(&mut boot, 11, 512);
    boot[13] = SECTORS_PER_CLUSTER as u8;
    put_u16(&mut boot, 14, RESERVED_SECTORS as u16);
    boot[16] = FAT_COUNT as u8;
    boot[21] = 0xf8;
    put_u16(&mut boot, 24, 63);
    put_u16(&mut boot, 26, 255);
    put_u32(&mut boot, 28, u32::try_from(start_lba).unwrap_or(u32::MAX));
    put_u32(&mut boot, 32, total_sectors);
    put_u32(&mut boot, 36, fat_sectors);
    put_u16(&mut boot, 40, 0);
    put_u16(&mut boot, 42, 0);
    put_u32(&mut boot, 44, 2);
    put_u16(&mut boot, 48, FSINFO_SECTOR as u16);
    put_u16(&mut boot, 50, BACKUP_BOOT_SECTOR as u16);
    boot[64] = 0x80;
    boot[66] = 0x29;
    put_u32(&mut boot, 67, start_lba as u32 ^ total_sectors);
    write_label(&mut boot[71..82], volume_label)?;
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510..512].copy_from_slice(&[0x55, 0xaa]);
    Ok(boot)
}

fn write_fsinfo<W: Write + Seek>(
    disk: &mut W,
    start_lba: u64,
    cluster_count: u32,
    next_cluster: u32,
) -> Result<(), String> {
    let mut info = [0u8; 512];
    put_u32(&mut info, 0, 0x4161_5252);
    put_u32(&mut info, 484, 0x6141_7272);
    put_u32(
        &mut info,
        488,
        cluster_count.saturating_sub(next_cluster - 2),
    );
    put_u32(&mut info, 492, next_cluster);
    put_u32(&mut info, 508, 0xaa55_0000);
    write_at_lba(disk, start_lba + u64::from(FSINFO_SECTOR), &info)
}

fn write_label(target: &mut [u8], label: &str) -> Result<(), String> {
    if target.len() != 11 || !label.is_ascii() || label.len() > 11 {
        return Err("invalid FAT32 volume label".to_owned());
    }
    target.fill(b' ');
    for (destination, source) in target.iter_mut().zip(label.bytes()) {
        *destination = source.to_ascii_uppercase();
    }
    Ok(())
}

fn write_directory_entry(
    target: &mut [u8],
    name: [u8; 11],
    attributes: u8,
    cluster: u32,
    size: u32,
) {
    target.fill(0);
    target[..11].copy_from_slice(&name);
    target[11] = attributes;
    put_u16(target, 20, (cluster >> 16) as u16);
    put_u16(target, 26, cluster as u16);
    put_u32(target, 28, size);
}

fn cluster_offset(data_start: u64, cluster: u32) -> Result<u64, String> {
    if cluster < 2 {
        return Err("FAT32 data cluster index is reserved".to_owned());
    }
    data_start
        .checked_add(u64::from(cluster - 2) * u64::from(SECTORS_PER_CLUSTER) * SECTOR_SIZE)
        .ok_or_else(|| "FAT32 data offset overflow".to_owned())
}

fn absolute_offset(start_lba: u64, sector: u64) -> Result<u64, String> {
    start_lba
        .checked_add(sector)
        .and_then(|lba| lba.checked_mul(SECTOR_SIZE))
        .ok_or_else(|| "FAT32 offset overflow".to_owned())
}

fn write_at_lba<W: Write + Seek>(disk: &mut W, lba: u64, bytes: &[u8]) -> Result<(), String> {
    let offset = lba
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| "FAT32 LBA offset overflow".to_owned())?;
    write_at(disk, offset, bytes)
}

fn write_at<W: Write + Seek>(disk: &mut W, offset: u64, bytes: &[u8]) -> Result<(), String> {
    disk.seek(SeekFrom::Start(offset))
        .and_then(|_| disk.write_all(bytes))
        .map_err(|error| format!("cannot write FAT32 image at byte {offset}: {error}"))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom};

    use super::{SECTOR_SIZE, SECTORS_PER_CLUSTER, VolumeFile, format_partition, geometry};

    const ESP_SECTORS: u64 = 1_048_576;
    const ESP_START_LBA: u64 = 2048;

    #[test]
    fn fat32_partition_writes_valid_boot_metadata_and_directory_chains() {
        let path = unique_path();
        let mut disk = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create sparse disk");
        disk.set_len((ESP_START_LBA + ESP_SECTORS) * SECTOR_SIZE)
            .expect("size sparse disk");
        let payload = vec![0x5a; 4097];
        let placements = format_partition(
            &mut disk,
            ESP_START_LBA,
            ESP_SECTORS,
            "NAGI ESP",
            &[VolumeFile {
                path: "EFI/BOOT/BOOTX64.EFI",
                contents: &payload,
            }],
        )
        .expect("format FAT32 volume");
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].cluster_count, 2);

        let boot = read_at(&mut disk, ESP_START_LBA * SECTOR_SIZE, 512);
        assert_eq!(&boot[82..90], b"FAT32   ");
        assert_eq!(&boot[510..512], &[0x55, 0xaa]);
        assert_eq!(boot[13], SECTORS_PER_CLUSTER as u8);
        assert_eq!(u16::from_le_bytes([boot[14], boot[15]]), 32);
        let fat_sectors = u32::from_le_bytes(boot[36..40].try_into().unwrap());
        let data_start_sector = 32 + fat_sectors * 2;
        let root_offset = (ESP_START_LBA + u64::from(data_start_sector)) * SECTOR_SIZE;
        let root = read_at(&mut disk, root_offset, 4096);
        assert_eq!(&root[..11], b"EFI        ");
        let efi_cluster = directory_cluster(&root[..32]);
        assert_eq!(efi_cluster, 3);
        let efi = read_at(
            &mut disk,
            root_offset + u64::from(efi_cluster - 2) * 4096,
            4096,
        );
        assert_eq!(&efi[64..75], b"BOOT       ");
        let boot_cluster = directory_cluster(&efi[64..96]);
        assert_eq!(boot_cluster, 4);
        let boot_directory = read_at(
            &mut disk,
            root_offset + u64::from(boot_cluster - 2) * 4096,
            4096,
        );
        assert_eq!(&boot_directory[64..75], b"BOOTX64 EFI");
        let loader_cluster = directory_cluster(&boot_directory[64..96]);
        assert_eq!(loader_cluster, placements[0].first_cluster);

        let fat_offset = (ESP_START_LBA + 32) * SECTOR_SIZE;
        assert_eq!(
            fat_entry(&mut disk, fat_offset, loader_cluster),
            loader_cluster + 1
        );
        assert_eq!(
            fat_entry(&mut disk, fat_offset, loader_cluster + 1),
            0x0fff_ffff
        );
        let first_payload = read_at(
            &mut disk,
            root_offset + u64::from(loader_cluster - 2) * 4096,
            4096,
        );
        assert_eq!(first_payload, payload[..4096]);
        let last_payload = read_at(
            &mut disk,
            root_offset + u64::from(loader_cluster - 1) * 4096,
            1,
        );
        assert_eq!(last_payload, payload[4096..]);
        drop(disk);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn fat32_geometry_enforces_fat32_cluster_ranges() {
        assert!(geometry(100_000).is_err());
        assert!(geometry(ESP_SECTORS as u32).is_ok());
    }

    fn directory_cluster(entry: &[u8]) -> u32 {
        let high = u16::from_le_bytes([entry[20], entry[21]]) as u32;
        let low = u16::from_le_bytes([entry[26], entry[27]]) as u32;
        (high << 16) | low
    }

    fn fat_entry(disk: &mut std::fs::File, fat_offset: u64, cluster: u32) -> u32 {
        let bytes = read_at(disk, fat_offset + u64::from(cluster) * 4, 4);
        u32::from_le_bytes(bytes.try_into().unwrap()) & 0x0fff_ffff
    }

    fn read_at(disk: &mut std::fs::File, offset: u64, length: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; length];
        disk.seek(SeekFrom::Start(offset)).expect("seek disk");
        disk.read_exact(&mut bytes).expect("read disk");
        bytes
    }

    fn unique_path() -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("nagi-fat32-test-{}-{nonce}", std::process::id()))
    }
}
