//! Minimal FAT32 formatter and root-file reader over 512-byte sectors.
//!
//! Used by the in-guest system update installer (ADR 0062) to write an
//! inactive system slot that UEFI firmware can read, and to read files back
//! for verification. Only the root directory is supported, which is all a
//! system slot needs (`KERNEL.ELF`, `INIT.ELF`, `SLOT.MAN`).
//!
//! The formatter writes in an order that never leaves a half-written volume
//! looking valid: it first invalidates both boot sectors, then writes file
//! data, the FATs, the root directory and FSInfo, and the primary boot
//! sector last.
//!
//! The crate is `no_std` and allocation-free; sector sizes are fixed at 512.

#![no_std]

pub const SECTOR_SIZE: usize = 512;
/// Root directory entries that fit in one cluster of the smallest
/// supported size are far more than a slot needs; files are bounded here.
pub const MAX_ROOT_FILES: usize = 8;
const RESERVED_SECTORS: u32 = 32;
const FAT_COUNT: u32 = 2;
const FSINFO_SECTOR: u32 = 1;
const BACKUP_BOOT_SECTOR: u32 = 6;
const ROOT_CLUSTER: u32 = 2;
const FAT32_EOC: u32 = 0x0fff_ffff;
const FAT32_EOC_MIN: u32 = 0x0fff_fff8;
const MIN_FAT32_CLUSTERS: u32 = 65_525;
const MAX_FAT32_CLUSTERS: u32 = 0x0fff_ffef;
const ENTRIES_PER_FAT_SECTOR: u32 = (SECTOR_SIZE / 4) as u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatError {
    Device,
    /// The volume size or cluster size cannot form a valid FAT32 volume.
    Geometry,
    /// Too many files, an empty file, or a duplicate name.
    Files,
    /// The files do not fit in the volume.
    Space,
    InvalidVolume,
    NotFound,
    OutOfRange,
}

/// Sector access relative to the start of the volume.
pub trait SectorDevice {
    fn read_sector(&mut self, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), FatError>;
    fn write_sector(&mut self, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> Result<(), FatError>;
}

/// A file to place in the root directory, with an 8.3 name in directory
/// form (`b"KERNEL  ELF"`).
#[derive(Clone, Copy)]
pub struct RootFile<'a> {
    pub name: [u8; 11],
    pub contents: &'a [u8],
}

/// Convert `"NAME.EXT"` to directory form. Accepts only uppercase ASCII
/// letters, digits and `_`, with an 1–8 character base and 0–3 character
/// extension.
pub fn short_name(name: &[u8]) -> Option<[u8; 11]> {
    let (base, extension) = match name.iter().position(|byte| *byte == b'.') {
        Some(dot) => (&name[..dot], &name[dot + 1..]),
        None => (name, &[][..]),
    };
    let valid = |part: &[u8]| {
        part.iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    };
    if base.is_empty() || base.len() > 8 || extension.len() > 3 || !valid(base) || !valid(extension)
    {
        return None;
    }
    let mut short = [b' '; 11];
    short[..base.len()].copy_from_slice(base);
    short[8..8 + extension.len()].copy_from_slice(extension);
    Some(short)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Geometry {
    total_sectors: u32,
    sectors_per_cluster: u32,
    fat_sectors: u32,
    cluster_count: u32,
}

impl Geometry {
    fn compute(total_sectors: u32, sectors_per_cluster: u32) -> Result<Self, FatError> {
        if sectors_per_cluster == 0
            || sectors_per_cluster > 128
            || !sectors_per_cluster.is_power_of_two()
        {
            return Err(FatError::Geometry);
        }
        let clusters_for = |fat_sectors: u32| -> Result<u32, FatError> {
            let overhead = RESERVED_SECTORS
                .checked_add(
                    FAT_COUNT
                        .checked_mul(fat_sectors)
                        .ok_or(FatError::Geometry)?,
                )
                .ok_or(FatError::Geometry)?;
            if overhead >= total_sectors {
                return Err(FatError::Geometry);
            }
            Ok((total_sectors - overhead) / sectors_per_cluster)
        };
        let accept = |fat_sectors: u32, clusters: u32| {
            if !(MIN_FAT32_CLUSTERS..=MAX_FAT32_CLUSTERS).contains(&clusters) {
                return Err(FatError::Geometry);
            }
            Ok(Self {
                total_sectors,
                sectors_per_cluster,
                fat_sectors,
                cluster_count: clusters,
            })
        };
        let mut fat_sectors = 1u32;
        let mut previous = 0u32;
        for _ in 0..16 {
            let clusters = clusters_for(fat_sectors)?;
            let required = (clusters + 2).div_ceil(ENTRIES_PER_FAT_SECTOR);
            if required == fat_sectors {
                return accept(fat_sectors, clusters);
            }
            if required == previous {
                // A 2-cycle: no FAT size is exactly what its own cluster
                // count needs (adding one FAT sector drops the requirement
                // below it). The larger size covers its clusters with
                // slack; the smaller one would be too small.
                let fat_sectors = fat_sectors.max(required);
                let clusters = clusters_for(fat_sectors)?;
                if (clusters + 2).div_ceil(ENTRIES_PER_FAT_SECTOR) > fat_sectors {
                    return Err(FatError::Geometry);
                }
                return accept(fat_sectors, clusters);
            }
            previous = fat_sectors;
            fat_sectors = required;
        }
        Err(FatError::Geometry)
    }

    const fn first_data_sector(self) -> u64 {
        RESERVED_SECTORS as u64 + FAT_COUNT as u64 * self.fat_sectors as u64
    }

    const fn cluster_bytes(self) -> u64 {
        self.sectors_per_cluster as u64 * SECTOR_SIZE as u64
    }

    fn cluster_sector(self, cluster: u32) -> Result<u64, FatError> {
        if cluster < 2 || cluster >= self.cluster_count + 2 {
            return Err(FatError::InvalidVolume);
        }
        Ok(self.first_data_sector() + u64::from(cluster - 2) * u64::from(self.sectors_per_cluster))
    }
}

#[derive(Clone, Copy)]
struct Placement {
    first_cluster: u32,
    clusters: u32,
}

/// Format the whole volume as FAT32 holding `files` in its root directory.
pub fn format<D: SectorDevice>(
    device: &mut D,
    total_sectors: u32,
    sectors_per_cluster: u32,
    label: &[u8; 11],
    files: &[RootFile<'_>],
) -> Result<(), FatError> {
    let geometry = Geometry::compute(total_sectors, sectors_per_cluster)?;
    if files.len() > MAX_ROOT_FILES {
        return Err(FatError::Files);
    }
    for (index, file) in files.iter().enumerate() {
        if file.contents.is_empty()
            || u32::try_from(file.contents.len()).is_err()
            || files[..index].iter().any(|other| other.name == file.name)
        {
            return Err(FatError::Files);
        }
    }
    // Plan contiguous chains after the one-cluster root directory.
    let mut placements = [Placement {
        first_cluster: 0,
        clusters: 0,
    }; MAX_ROOT_FILES];
    let mut next = ROOT_CLUSTER + 1;
    for (placement, file) in placements.iter_mut().zip(files) {
        let clusters =
            u32::try_from((file.contents.len() as u64).div_ceil(geometry.cluster_bytes()))
                .map_err(|_| FatError::Space)?;
        let end = next.checked_add(clusters).ok_or(FatError::Space)?;
        if end > geometry.cluster_count + 2 {
            return Err(FatError::Space);
        }
        *placement = Placement {
            first_cluster: next,
            clusters,
        };
        next = end;
    }
    let placements = &placements[..files.len()];

    // 1. Invalidate any previous volume.
    let zero = [0u8; SECTOR_SIZE];
    device.write_sector(0, &zero)?;
    device.write_sector(u64::from(BACKUP_BOOT_SECTOR), &zero)?;

    // 2. File data.
    for (placement, file) in placements.iter().zip(files) {
        let first = geometry.cluster_sector(placement.first_cluster)?;
        for (index, chunk) in file.contents.chunks(SECTOR_SIZE).enumerate() {
            let mut sector = [0u8; SECTOR_SIZE];
            sector[..chunk.len()].copy_from_slice(chunk);
            device.write_sector(first + index as u64, &sector)?;
        }
    }

    // 3. Both FATs, generated one sector at a time.
    for fat in 0..FAT_COUNT {
        let fat_start = u64::from(RESERVED_SECTORS + fat * geometry.fat_sectors);
        for fat_sector in 0..geometry.fat_sectors {
            let mut sector = [0u8; SECTOR_SIZE];
            for slot in 0..ENTRIES_PER_FAT_SECTOR {
                let cluster = fat_sector * ENTRIES_PER_FAT_SECTOR + slot;
                let value = fat_entry(cluster, placements);
                let offset = slot as usize * 4;
                sector[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            device.write_sector(fat_start + u64::from(fat_sector), &sector)?;
        }
    }

    // 4. Root directory: one cluster, fully cleared.
    let root_first = geometry.cluster_sector(ROOT_CLUSTER)?;
    for index in 0..geometry.sectors_per_cluster {
        let mut sector = [0u8; SECTOR_SIZE];
        if index == 0 {
            let mut entry = [0u8; 32];
            entry[..11].copy_from_slice(label);
            entry[11] = 0x08;
            sector[..32].copy_from_slice(&entry);
            for (position, (placement, file)) in placements.iter().zip(files).enumerate() {
                let offset = 32 * (position + 1);
                let entry = &mut sector[offset..offset + 32];
                entry[..11].copy_from_slice(&file.name);
                entry[11] = 0x20;
                entry[20..22]
                    .copy_from_slice(&((placement.first_cluster >> 16) as u16).to_le_bytes());
                entry[26..28].copy_from_slice(&(placement.first_cluster as u16).to_le_bytes());
                entry[28..32].copy_from_slice(&(file.contents.len() as u32).to_le_bytes());
            }
        }
        device.write_sector(root_first + u64::from(index), &sector)?;
    }

    // 5. FSInfo (primary and backup), backup boot sector, then the primary.
    let fsinfo = fsinfo_sector(geometry, next);
    device.write_sector(u64::from(FSINFO_SECTOR), &fsinfo)?;
    device.write_sector(u64::from(BACKUP_BOOT_SECTOR + FSINFO_SECTOR), &fsinfo)?;
    let boot = boot_sector(geometry, label);
    device.write_sector(u64::from(BACKUP_BOOT_SECTOR), &boot)?;
    device.write_sector(0, &boot)
}

fn fat_entry(cluster: u32, placements: &[Placement]) -> u32 {
    match cluster {
        0 => 0x0fff_fff8,
        1 | ROOT_CLUSTER => FAT32_EOC,
        _ => placements
            .iter()
            .find(|placement| {
                cluster >= placement.first_cluster
                    && cluster < placement.first_cluster + placement.clusters
            })
            .map_or(0, |placement| {
                if cluster + 1 == placement.first_cluster + placement.clusters {
                    FAT32_EOC
                } else {
                    cluster + 1
                }
            }),
    }
}

fn boot_sector(geometry: Geometry, label: &[u8; 11]) -> [u8; SECTOR_SIZE] {
    let mut boot = [0u8; SECTOR_SIZE];
    boot[..3].copy_from_slice(&[0xeb, 0x58, 0x90]);
    boot[3..11].copy_from_slice(b"NAGIOS  ");
    boot[11..13].copy_from_slice(&(SECTOR_SIZE as u16).to_le_bytes());
    boot[13] = geometry.sectors_per_cluster as u8;
    boot[14..16].copy_from_slice(&(RESERVED_SECTORS as u16).to_le_bytes());
    boot[16] = FAT_COUNT as u8;
    boot[21] = 0xf8;
    boot[24..26].copy_from_slice(&63u16.to_le_bytes());
    boot[26..28].copy_from_slice(&255u16.to_le_bytes());
    boot[32..36].copy_from_slice(&geometry.total_sectors.to_le_bytes());
    boot[36..40].copy_from_slice(&geometry.fat_sectors.to_le_bytes());
    boot[44..48].copy_from_slice(&ROOT_CLUSTER.to_le_bytes());
    boot[48..50].copy_from_slice(&(FSINFO_SECTOR as u16).to_le_bytes());
    boot[50..52].copy_from_slice(&(BACKUP_BOOT_SECTOR as u16).to_le_bytes());
    boot[64] = 0x80;
    boot[66] = 0x29;
    boot[67..71].copy_from_slice(&(geometry.total_sectors ^ 0x4e41_4749).to_le_bytes());
    boot[71..82].copy_from_slice(label);
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510..512].copy_from_slice(&[0x55, 0xaa]);
    boot
}

fn fsinfo_sector(geometry: Geometry, next_free: u32) -> [u8; SECTOR_SIZE] {
    let mut info = [0u8; SECTOR_SIZE];
    info[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    info[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    let free = geometry.cluster_count.saturating_sub(next_free - 2);
    info[488..492].copy_from_slice(&free.to_le_bytes());
    info[492..496].copy_from_slice(&next_free.to_le_bytes());
    info[508..512].copy_from_slice(&0xaa55_0000u32.to_le_bytes());
    info
}

/// A file found in the root directory of a FAT32 volume.
pub struct RootFileReader<'d, D> {
    device: &'d mut D,
    geometry: Geometry,
    first_cluster: u32,
    size: u32,
}

impl<'d, D: SectorDevice> RootFileReader<'d, D> {
    /// Open `name` (directory form) in the root directory of the volume of
    /// `volume_sectors` sectors.
    pub fn open(device: &'d mut D, volume_sectors: u64, name: &[u8; 11]) -> Result<Self, FatError> {
        let mut boot = [0u8; SECTOR_SIZE];
        device.read_sector(0, &mut boot)?;
        let geometry = parse_boot_sector(&boot, volume_sectors)?;
        let root = u32::from_le_bytes([boot[44], boot[45], boot[46], boot[47]]) & 0x0fff_ffff;
        let mut cluster = root;
        // Bound the walk: a slot's root directory needs a handful of
        // clusters at most.
        for _ in 0..16 {
            let first = geometry.cluster_sector(cluster)?;
            for index in 0..geometry.sectors_per_cluster {
                let mut sector = [0u8; SECTOR_SIZE];
                device.read_sector(first + u64::from(index), &mut sector)?;
                for entry in sector.chunks_exact(32) {
                    if entry[0] == 0 {
                        return Err(FatError::NotFound);
                    }
                    if entry[0] == 0xe5 || entry[11] & 0x18 != 0 || entry[..11] != name[..] {
                        continue;
                    }
                    let high = u32::from(u16::from_le_bytes([entry[20], entry[21]]));
                    let low = u32::from(u16::from_le_bytes([entry[26], entry[27]]));
                    let size = u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]);
                    return Ok(Self {
                        device,
                        geometry,
                        first_cluster: high << 16 | low,
                        size,
                    });
                }
            }
            match next_cluster(device, geometry, cluster)? {
                Some(next) => cluster = next,
                None => return Err(FatError::NotFound),
            }
        }
        Err(FatError::InvalidVolume)
    }

    pub const fn len(&self) -> u32 {
        self.size
    }

    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Stream the whole file through `sink`, one sector-sized chunk at a
    /// time (the last chunk is trimmed to the file size).
    pub fn for_each_chunk(&mut self, mut sink: impl FnMut(&[u8])) -> Result<(), FatError> {
        let mut remaining = u64::from(self.size);
        let mut cluster = self.first_cluster;
        while remaining > 0 {
            let first = self.geometry.cluster_sector(cluster)?;
            for index in 0..self.geometry.sectors_per_cluster {
                if remaining == 0 {
                    break;
                }
                let mut sector = [0u8; SECTOR_SIZE];
                self.device
                    .read_sector(first + u64::from(index), &mut sector)?;
                let take = remaining.min(SECTOR_SIZE as u64) as usize;
                sink(&sector[..take]);
                remaining -= take as u64;
            }
            if remaining > 0 {
                cluster = next_cluster(self.device, self.geometry, cluster)?
                    .ok_or(FatError::InvalidVolume)?;
            }
        }
        Ok(())
    }

    /// Read the whole file into `output`, which must be at least `len`.
    pub fn read_all(&mut self, output: &mut [u8]) -> Result<usize, FatError> {
        let size = self.size as usize;
        if output.len() < size {
            return Err(FatError::OutOfRange);
        }
        let mut offset = 0;
        self.for_each_chunk(|chunk| {
            output[offset..offset + chunk.len()].copy_from_slice(chunk);
            offset += chunk.len();
        })?;
        Ok(offset)
    }
}

fn parse_boot_sector(boot: &[u8; SECTOR_SIZE], volume_sectors: u64) -> Result<Geometry, FatError> {
    let invalid = FatError::InvalidVolume;
    if boot[510..512] != [0x55, 0xaa]
        || u16::from_le_bytes([boot[11], boot[12]]) != SECTOR_SIZE as u16
        || u16::from_le_bytes([boot[17], boot[18]]) != 0
        || u16::from_le_bytes([boot[22], boot[23]]) != 0
    {
        return Err(invalid);
    }
    let sectors_per_cluster = u32::from(boot[13]);
    let reserved = u32::from(u16::from_le_bytes([boot[14], boot[15]]));
    let fats = u32::from(boot[16]);
    let total_sectors = u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]);
    let fat_sectors = u32::from_le_bytes([boot[36], boot[37], boot[38], boot[39]]);
    if reserved != RESERVED_SECTORS
        || fats != FAT_COUNT
        || u64::from(total_sectors) > volume_sectors
    {
        return Err(invalid);
    }
    let geometry = Geometry::compute(total_sectors, sectors_per_cluster).map_err(|_| invalid)?;
    if geometry.fat_sectors != fat_sectors {
        return Err(invalid);
    }
    Ok(geometry)
}

fn next_cluster<D: SectorDevice>(
    device: &mut D,
    geometry: Geometry,
    cluster: u32,
) -> Result<Option<u32>, FatError> {
    let fat_sector = u64::from(RESERVED_SECTORS) + u64::from(cluster / ENTRIES_PER_FAT_SECTOR);
    let mut sector = [0u8; SECTOR_SIZE];
    device.read_sector(fat_sector, &mut sector)?;
    let offset = (cluster % ENTRIES_PER_FAT_SECTOR) as usize * 4;
    let value = u32::from_le_bytes([
        sector[offset],
        sector[offset + 1],
        sector[offset + 2],
        sector[offset + 3],
    ]) & 0x0fff_ffff;
    if value >= FAT32_EOC_MIN {
        return Ok(None);
    }
    if value < 2 || value >= geometry.cluster_count + 2 {
        return Err(FatError::InvalidVolume);
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// A sparse in-memory disk: only written sectors are stored.
    struct MemoryDisk {
        sectors: std::collections::BTreeMap<u64, [u8; SECTOR_SIZE]>,
        limit: u64,
        writes: u64,
    }

    impl MemoryDisk {
        fn new(limit: u64) -> Self {
            Self {
                sectors: std::collections::BTreeMap::new(),
                limit,
                writes: 0,
            }
        }
    }

    impl SectorDevice for MemoryDisk {
        fn read_sector(
            &mut self,
            sector: u64,
            buffer: &mut [u8; SECTOR_SIZE],
        ) -> Result<(), FatError> {
            if sector >= self.limit {
                return Err(FatError::Device);
            }
            *buffer = self
                .sectors
                .get(&sector)
                .copied()
                .unwrap_or([0; SECTOR_SIZE]);
            Ok(())
        }

        fn write_sector(
            &mut self,
            sector: u64,
            buffer: &[u8; SECTOR_SIZE],
        ) -> Result<(), FatError> {
            if sector >= self.limit {
                return Err(FatError::Device);
            }
            self.sectors.insert(sector, *buffer);
            self.writes += 1;
            Ok(())
        }
    }

    /// The 4 GiB reference system-slot size.
    const SLOT_SECTORS: u32 = 8_388_608;
    const LABEL: &[u8; 11] = b"NAGI SYS B ";

    fn pattern(length: usize, seed: u8) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31) ^ seed)
            .collect()
    }

    #[test]
    fn formatted_slots_read_back_exactly() {
        let kernel = pattern(312_377, 1);
        let init = pattern(127_464, 2);
        let manifest = pattern(400, 3);
        let mut disk = MemoryDisk::new(u64::from(SLOT_SECTORS));
        let files = [
            RootFile {
                name: short_name(b"KERNEL.ELF").unwrap(),
                contents: &kernel,
            },
            RootFile {
                name: short_name(b"INIT.ELF").unwrap(),
                contents: &init,
            },
            RootFile {
                name: short_name(b"SLOT.MAN").unwrap(),
                contents: &manifest,
            },
        ];
        format(&mut disk, SLOT_SECTORS, 64, LABEL, &files).expect("format");
        for file in &files {
            let mut reader =
                RootFileReader::open(&mut disk, u64::from(SLOT_SECTORS), &file.name).expect("open");
            assert_eq!(reader.len() as usize, file.contents.len());
            let mut output = vec![0; file.contents.len()];
            assert_eq!(reader.read_all(&mut output), Ok(file.contents.len()));
            assert_eq!(output, file.contents);
        }
        assert_eq!(
            RootFileReader::open(&mut disk, u64::from(SLOT_SECTORS), b"MISSING    ").err(),
            Some(FatError::NotFound)
        );
        // 32 KiB clusters keep the FATs small enough for TCG installs.
        assert!(disk.writes < 3_000, "writes={}", disk.writes);
    }

    #[test]
    fn the_guest_model_store_reader_accepts_formatted_volumes() {
        use nagi_model_manager::{
            model_store_short_name, ArtifactId, ArtifactReadError, Fat32ArtifactReader,
            ModelArtifactReader, ModelStoreSectorReader, FAT32_SECTOR_SIZE,
        };

        struct Reader<'a>(&'a mut MemoryDisk);
        impl ModelStoreSectorReader for Reader<'_> {
            fn read_sector(
                &mut self,
                sector: u64,
                destination: &mut [u8; FAT32_SECTOR_SIZE],
            ) -> Result<(), ArtifactReadError> {
                self.0
                    .read_sector(sector, destination)
                    .map_err(|_| ArtifactReadError::Unavailable)
            }
        }

        let artifact = ArtifactId::new("org.nagi.test.fat32").expect("artifact id");
        let contents = pattern(70_000, 9);
        let mut disk = MemoryDisk::new(u64::from(SLOT_SECTORS));
        format(
            &mut disk,
            SLOT_SECTORS,
            64,
            LABEL,
            &[RootFile {
                name: model_store_short_name(&artifact),
                contents: &contents,
            }],
        )
        .expect("format");
        let mut reader =
            Fat32ArtifactReader::open(Reader(&mut disk), u64::from(SLOT_SECTORS), artifact)
                .expect("independent reader opens the volume");
        let mut output = vec![0; contents.len()];
        let mut offset = 0;
        while offset < output.len() {
            let read = reader
                .read_at(offset as u64, &mut output[offset..])
                .expect("read");
            assert!(read > 0);
            offset += read;
        }
        assert_eq!(output, contents);
    }

    #[test]
    fn reformatting_replaces_a_previous_volume() {
        let mut disk = MemoryDisk::new(u64::from(SLOT_SECTORS));
        let old = pattern(100_000, 4);
        let new = pattern(5_000, 5);
        let name = short_name(b"INIT.ELF").unwrap();
        format(
            &mut disk,
            SLOT_SECTORS,
            64,
            LABEL,
            &[RootFile {
                name,
                contents: &old,
            }],
        )
        .unwrap();
        format(
            &mut disk,
            SLOT_SECTORS,
            64,
            LABEL,
            &[RootFile {
                name,
                contents: &new,
            }],
        )
        .unwrap();
        let mut reader = RootFileReader::open(&mut disk, u64::from(SLOT_SECTORS), &name).unwrap();
        let mut output = vec![0; new.len()];
        reader.read_all(&mut output).unwrap();
        assert_eq!(output, new);
    }

    #[test]
    fn two_cycle_geometries_use_the_larger_fat() {
        // 512 FAT sectors leave 65,535 clusters, which need 513; 513 leave
        // 65,534, which need 512. The larger FAT is the valid choice.
        let geometry = Geometry::compute(4_195_296, 64).expect("geometry");
        assert_eq!(
            (geometry.fat_sectors, geometry.cluster_count),
            (513, 65_534)
        );
        // Cycles whose cluster count is below the FAT32 minimum stay refused.
        assert_eq!(Geometry::compute(65_681, 1), Err(FatError::Geometry));
        assert_eq!(Geometry::compute(524_310, 8), Err(FatError::Geometry));
        // The reference slot is unchanged.
        let slot = Geometry::compute(SLOT_SECTORS, 64).expect("slot");
        assert_eq!((slot.fat_sectors, slot.cluster_count), (1024, 131_039));

        let contents = pattern(3_000, 7);
        let name = short_name(b"SLOT.MAN").unwrap();
        let mut disk = MemoryDisk::new(4_195_296);
        format(
            &mut disk,
            4_195_296,
            64,
            LABEL,
            &[RootFile {
                name,
                contents: &contents,
            }],
        )
        .expect("format");
        let mut reader = RootFileReader::open(&mut disk, 4_195_296, &name).expect("open");
        let mut output = vec![0; contents.len()];
        assert_eq!(reader.read_all(&mut output), Ok(contents.len()));
        assert_eq!(output, contents);
    }

    #[test]
    fn geometry_around_the_fat32_cluster_minimum() {
        // (total sectors, sectors per cluster, expected (fat, clusters)).
        // Each block: one below the 65,525-cluster minimum, the minimum,
        // the last 512-sector fixed point, the 2-cycle sizes that now take
        // 513 sectors, and the first 513-sector fixed point.
        type Case = (u32, u32, Option<(u32, u32)>);
        let cases: &[Case] = &[
            (4_194_655, 64, None),
            (4_194_656, 64, Some((512, 65_525))),
            (4_195_295, 64, Some((512, 65_534))),
            (4_195_296, 64, Some((513, 65_534))),
            (4_195_297, 64, Some((513, 65_534))),
            (4_195_298, 64, Some((513, 65_535))),
            (525_255, 8, None),
            (525_256, 8, Some((512, 65_525))),
            (525_335, 8, Some((512, 65_534))),
            (525_336, 8, Some((513, 65_534))),
            (525_337, 8, Some((513, 65_534))),
            (525_338, 8, Some((513, 65_535))),
            (66_580, 1, None),
            (66_581, 1, Some((512, 65_525))),
            (66_590, 1, Some((512, 65_534))),
            (66_591, 1, Some((513, 65_533))),
            (66_592, 1, Some((513, 65_534))),
            (66_593, 1, Some((513, 65_535))),
        ];
        for &(total, spc, expected) in cases {
            let actual = Geometry::compute(total, spc)
                .ok()
                .map(|geometry| (geometry.fat_sectors, geometry.cluster_count));
            assert_eq!(actual, expected, "{total} sectors, {spc} per cluster");
            if let Some((fat, clusters)) = actual {
                assert!(fat * ENTRIES_PER_FAT_SECTOR >= clusters + 2);
                assert!(RESERVED_SECTORS + FAT_COUNT * fat + clusters * spc <= total);
            }
        }
    }

    #[test]
    fn the_guest_model_store_reader_accepts_a_two_cycle_geometry() {
        use nagi_model_manager::{
            model_store_short_name, ArtifactId, ArtifactReadError, Fat32ArtifactReader,
            ModelArtifactReader, ModelStoreSectorReader, FAT32_SECTOR_SIZE,
        };

        struct Reader<'a>(&'a mut MemoryDisk);
        impl ModelStoreSectorReader for Reader<'_> {
            fn read_sector(
                &mut self,
                sector: u64,
                destination: &mut [u8; FAT32_SECTOR_SIZE],
            ) -> Result<(), ArtifactReadError> {
                self.0
                    .read_sector(sector, destination)
                    .map_err(|_| ArtifactReadError::Unavailable)
            }
        }

        const SECTORS: u32 = 4_195_296;
        let artifact = ArtifactId::new("org.nagi.test.fat32").expect("artifact id");
        let contents = pattern(70_000, 11);
        let mut disk = MemoryDisk::new(u64::from(SECTORS));
        format(
            &mut disk,
            SECTORS,
            64,
            LABEL,
            &[RootFile {
                name: model_store_short_name(&artifact),
                contents: &contents,
            }],
        )
        .expect("format");
        let mut reader = Fat32ArtifactReader::open(Reader(&mut disk), u64::from(SECTORS), artifact)
            .expect("independent reader opens the volume");
        let mut output = vec![0; contents.len()];
        let mut offset = 0;
        while offset < output.len() {
            let read = reader
                .read_at(offset as u64, &mut output[offset..])
                .expect("read");
            assert!(read > 0);
            offset += read;
        }
        assert_eq!(output, contents);
    }

    #[test]
    fn invalid_inputs_are_refused() {
        let mut disk = MemoryDisk::new(u64::from(SLOT_SECTORS));
        let name = short_name(b"A.B").unwrap();
        assert_eq!(
            format(&mut disk, 1_000, 64, LABEL, &[]),
            Err(FatError::Geometry)
        );
        assert_eq!(
            format(&mut disk, SLOT_SECTORS, 3, LABEL, &[]),
            Err(FatError::Geometry)
        );
        assert_eq!(
            format(
                &mut disk,
                SLOT_SECTORS,
                64,
                LABEL,
                &[RootFile {
                    name,
                    contents: &[]
                }]
            ),
            Err(FatError::Files)
        );
        assert_eq!(
            format(
                &mut disk,
                SLOT_SECTORS,
                64,
                LABEL,
                &[
                    RootFile {
                        name,
                        contents: b"x"
                    },
                    RootFile {
                        name,
                        contents: b"y"
                    }
                ]
            ),
            Err(FatError::Files)
        );
        assert_eq!(disk.writes, 0, "refused formats write nothing");
        // An unformatted disk is not a volume.
        assert_eq!(
            RootFileReader::open(&mut disk, u64::from(SLOT_SECTORS), &name).err(),
            Some(FatError::InvalidVolume)
        );
        assert_eq!(short_name(b"kernel.elf"), None);
        assert_eq!(short_name(b"TOOLONGNAME.ELF"), None);
        assert_eq!(short_name(b"KERNEL.ELF"), Some(*b"KERNEL  ELF"));
    }
}
