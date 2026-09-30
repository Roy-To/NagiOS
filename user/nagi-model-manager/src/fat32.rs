use sha2::{Digest, Sha256};

use crate::{ArtifactId, ArtifactReadError, IntegrityMetadata, ModelArtifactReader};

pub const FAT32_SECTOR_SIZE: usize = 512;
const FAT32_SIGNATURE: [u8; 2] = [0x55, 0xaa];
const FAT32_EOC_MIN: u32 = 0x0fff_fff8;
const FAT32_BAD_CLUSTER: u32 = 0x0fff_fff7;
const MAX_ROOT_DIRECTORY_CLUSTERS: u64 = 64;

/// Read-only sector access to the dedicated Model Store partition.
/// Sector indices are relative to that partition, not the physical disk.
pub trait ModelStoreSectorReader {
    fn read_sector(
        &mut self,
        partition_relative_sector: u64,
        destination: &mut [u8; FAT32_SECTOR_SIZE],
    ) -> Result<(), ArtifactReadError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fat32ArtifactError {
    BlockUnavailable,
    InvalidBootSector,
    UnsupportedGeometry,
    CorruptDirectory,
    ArtifactNotFound,
    InvalidArtifact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fat32Geometry {
    volume_sectors: u64,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    sectors_per_fat: u32,
    first_data_sector: u64,
    cluster_count: u64,
    root_cluster: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fat32File {
    first_cluster: u32,
    size_bytes: u64,
}

#[derive(Clone)]
struct FatSectorCache {
    sector: Option<u64>,
    bytes: [u8; FAT32_SECTOR_SIZE],
}

impl FatSectorCache {
    const fn empty() -> Self {
        Self {
            sector: None,
            bytes: [0; FAT32_SECTOR_SIZE],
        }
    }
}

pub struct Fat32ArtifactReader<R> {
    reader: R,
    artifact_id: ArtifactId,
    geometry: Fat32Geometry,
    file: Fat32File,
    cursor_cluster_index: u64,
    cursor_cluster: u32,
    fat_cache: FatSectorCache,
}

/// Derive the stable FAT32 short name used for a model artifact.
///
/// The eight-character basename is the first 40 bits of
/// `SHA256(UTF8(artifact_id))`, encoded as Crockford Base32. The `.GGF`
/// extension identifies the current GGUF placement contract while fitting
/// FAT32's 8.3 short-name limit.
pub fn model_store_short_name(artifact_id: &ArtifactId) -> [u8; 11] {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let digest = Sha256::digest(artifact_id.as_str().as_bytes());
    let prefix = (u64::from(digest[0]) << 32)
        | (u64::from(digest[1]) << 24)
        | (u64::from(digest[2]) << 16)
        | (u64::from(digest[3]) << 8)
        | u64::from(digest[4]);
    let mut name = [b' '; 11];
    for (index, output) in name[..8].iter_mut().enumerate() {
        let shift = 35 - index * 5;
        *output = ALPHABET[((prefix >> shift) & 0x1f) as usize];
    }
    name[8..].copy_from_slice(b"GGF");
    name
}

impl<R: ModelStoreSectorReader> Fat32ArtifactReader<R> {
    pub fn open(
        mut reader: R,
        partition_sectors: u64,
        artifact_id: ArtifactId,
    ) -> Result<Self, Fat32ArtifactError> {
        let mut boot_sector = [0; FAT32_SECTOR_SIZE];
        read_partition_sector(&mut reader, partition_sectors, 0, &mut boot_sector)?;
        let geometry = parse_geometry(&boot_sector, partition_sectors)?;
        let file_name = model_store_short_name(&artifact_id);
        let mut fat_cache = FatSectorCache::empty();
        let file = find_root_file(&mut reader, geometry, &mut fat_cache, &file_name)?;
        Ok(Self {
            reader,
            artifact_id,
            geometry,
            file,
            cursor_cluster_index: 0,
            cursor_cluster: file.first_cluster,
            fat_cache,
        })
    }

    pub fn into_inner(self) -> R {
        self.reader
    }

    fn cluster_for_index(&mut self, target_index: u64) -> Result<u32, ArtifactReadError> {
        if target_index >= self.geometry.cluster_count {
            return Err(ArtifactReadError::OutOfRange);
        }
        if target_index < self.cursor_cluster_index {
            self.cursor_cluster_index = 0;
            self.cursor_cluster = self.file.first_cluster;
        }
        while self.cursor_cluster_index < target_index {
            let Some(next) = next_cluster(
                &mut self.reader,
                self.geometry,
                &mut self.fat_cache,
                self.cursor_cluster,
            )?
            else {
                return Err(ArtifactReadError::Unavailable);
            };
            self.cursor_cluster = next;
            self.cursor_cluster_index += 1;
        }
        Ok(self.cursor_cluster)
    }

    fn cluster_sector(&self, cluster: u32, sector_in_cluster: u8) -> Option<u64> {
        if cluster < 2
            || u64::from(cluster) >= self.geometry.cluster_count.checked_add(2)?
            || sector_in_cluster >= self.geometry.sectors_per_cluster
        {
            return None;
        }
        self.geometry
            .first_data_sector
            .checked_add(
                u64::from(cluster - 2).checked_mul(u64::from(self.geometry.sectors_per_cluster))?,
            )?
            .checked_add(u64::from(sector_in_cluster))
    }
}

impl<R: ModelStoreSectorReader> ModelArtifactReader for Fat32ArtifactReader<R> {
    fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        None
    }

    fn len(&self) -> u64 {
        self.file.size_bytes
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        if offset > self.file.size_bytes {
            return Err(ArtifactReadError::OutOfRange);
        }
        if offset == self.file.size_bytes || destination.is_empty() {
            return Ok(0);
        }
        let wanted = usize::try_from(
            self.file
                .size_bytes
                .saturating_sub(offset)
                .min(destination.len() as u64),
        )
        .map_err(|_| ArtifactReadError::OutOfRange)?;
        let cluster_bytes = u64::from(self.geometry.sectors_per_cluster)
            .checked_mul(FAT32_SECTOR_SIZE as u64)
            .ok_or(ArtifactReadError::OutOfRange)?;
        let mut copied = 0usize;
        let mut sector_bytes = [0; FAT32_SECTOR_SIZE];
        while copied < wanted {
            let file_offset = offset
                .checked_add(copied as u64)
                .ok_or(ArtifactReadError::OutOfRange)?;
            let cluster_index = file_offset / cluster_bytes;
            let within_cluster = file_offset % cluster_bytes;
            let cluster = self.cluster_for_index(cluster_index)?;
            let sector_in_cluster = (within_cluster / FAT32_SECTOR_SIZE as u64) as u8;
            let sector_offset = (within_cluster % FAT32_SECTOR_SIZE as u64) as usize;
            let sector = self
                .cluster_sector(cluster, sector_in_cluster)
                .ok_or(ArtifactReadError::OutOfRange)?;
            read_partition_sector(
                &mut self.reader,
                self.geometry.volume_sectors,
                sector,
                &mut sector_bytes,
            )
            .map_err(|_| ArtifactReadError::Unavailable)?;
            let count = (FAT32_SECTOR_SIZE - sector_offset).min(wanted - copied);
            destination[copied..copied + count]
                .copy_from_slice(&sector_bytes[sector_offset..sector_offset + count]);
            copied += count;
        }
        Ok(copied)
    }
}

fn parse_geometry(
    boot_sector: &[u8; FAT32_SECTOR_SIZE],
    partition_sectors: u64,
) -> Result<Fat32Geometry, Fat32ArtifactError> {
    if boot_sector[510..512] != FAT32_SIGNATURE
        || read_u16(boot_sector, 11) != FAT32_SECTOR_SIZE as u16
        || read_u16(boot_sector, 17) != 0
        || read_u16(boot_sector, 22) != 0
    {
        return Err(Fat32ArtifactError::InvalidBootSector);
    }
    let sectors_per_cluster = boot_sector[13];
    let reserved_sectors = read_u16(boot_sector, 14);
    let fat_count = boot_sector[16];
    let volume_sectors = u64::from(read_u32(boot_sector, 32));
    let sectors_per_fat = read_u32(boot_sector, 36);
    let root_cluster = read_u32(boot_sector, 44) & 0x0fff_ffff;
    if sectors_per_cluster == 0
        || sectors_per_cluster > 128
        || !sectors_per_cluster.is_power_of_two()
        || reserved_sectors == 0
        || !(1..=2).contains(&fat_count)
        || volume_sectors == 0
        || volume_sectors > partition_sectors
        || sectors_per_fat == 0
        || root_cluster < 2
    {
        return Err(Fat32ArtifactError::UnsupportedGeometry);
    }
    let fat_area = u64::from(fat_count)
        .checked_mul(u64::from(sectors_per_fat))
        .ok_or(Fat32ArtifactError::UnsupportedGeometry)?;
    let first_data_sector = u64::from(reserved_sectors)
        .checked_add(fat_area)
        .ok_or(Fat32ArtifactError::UnsupportedGeometry)?;
    if first_data_sector >= volume_sectors {
        return Err(Fat32ArtifactError::UnsupportedGeometry);
    }
    let cluster_count = (volume_sectors - first_data_sector) / u64::from(sectors_per_cluster);
    let fat_entries = u64::from(sectors_per_fat)
        .checked_mul((FAT32_SECTOR_SIZE / 4) as u64)
        .ok_or(Fat32ArtifactError::UnsupportedGeometry)?;
    if cluster_count == 0
        || u64::from(root_cluster) >= cluster_count.saturating_add(2)
        || fat_entries < cluster_count.saturating_add(2)
    {
        return Err(Fat32ArtifactError::UnsupportedGeometry);
    }
    Ok(Fat32Geometry {
        volume_sectors,
        sectors_per_cluster,
        reserved_sectors,
        sectors_per_fat,
        first_data_sector,
        cluster_count,
        root_cluster,
    })
}

fn find_root_file<R: ModelStoreSectorReader>(
    reader: &mut R,
    geometry: Fat32Geometry,
    fat_cache: &mut FatSectorCache,
    wanted_name: &[u8; 11],
) -> Result<Fat32File, Fat32ArtifactError> {
    let mut cluster = geometry.root_cluster;
    let sectors_per_cluster = u64::from(geometry.sectors_per_cluster);
    for _ in 0..MAX_ROOT_DIRECTORY_CLUSTERS.min(geometry.cluster_count) {
        for sector_in_cluster in 0..geometry.sectors_per_cluster {
            let sector = cluster_sector(geometry, cluster, sector_in_cluster)
                .ok_or(Fat32ArtifactError::CorruptDirectory)?;
            let mut bytes = [0; FAT32_SECTOR_SIZE];
            read_partition_sector(reader, geometry.volume_sectors, sector, &mut bytes)
                .map_err(|_| Fat32ArtifactError::BlockUnavailable)?;
            for entry in bytes.chunks_exact(32) {
                if entry[0] == 0 {
                    return Err(Fat32ArtifactError::ArtifactNotFound);
                }
                let attributes = entry[11];
                if entry[0] == 0xe5 || attributes == 0x0f || attributes & 0x08 != 0 {
                    continue;
                }
                if &entry[..11] == wanted_name {
                    if attributes & 0x10 != 0 {
                        return Err(Fat32ArtifactError::InvalidArtifact);
                    }
                    let first_cluster =
                        (u32::from(read_u16(entry, 20)) << 16) | u32::from(read_u16(entry, 26));
                    let size_bytes = u64::from(read_u32(entry, 28));
                    let cluster_bytes = sectors_per_cluster * FAT32_SECTOR_SIZE as u64;
                    let required_clusters = size_bytes.div_ceil(cluster_bytes);
                    if size_bytes == 0
                        || first_cluster < 2
                        || u64::from(first_cluster) >= geometry.cluster_count.saturating_add(2)
                        || required_clusters > geometry.cluster_count
                    {
                        return Err(Fat32ArtifactError::InvalidArtifact);
                    }
                    return Ok(Fat32File {
                        first_cluster,
                        size_bytes,
                    });
                }
            }
        }
        match next_cluster(reader, geometry, fat_cache, cluster)
            .map_err(|_| Fat32ArtifactError::CorruptDirectory)?
        {
            Some(next) => cluster = next,
            None => return Err(Fat32ArtifactError::ArtifactNotFound),
        }
    }
    Err(Fat32ArtifactError::CorruptDirectory)
}

fn next_cluster<R: ModelStoreSectorReader>(
    reader: &mut R,
    geometry: Fat32Geometry,
    cache: &mut FatSectorCache,
    cluster: u32,
) -> Result<Option<u32>, ArtifactReadError> {
    if cluster < 2 || u64::from(cluster) >= geometry.cluster_count.saturating_add(2) {
        return Err(ArtifactReadError::Unavailable);
    }
    let fat_offset = u64::from(cluster)
        .checked_mul(4)
        .ok_or(ArtifactReadError::OutOfRange)?;
    let fat_sector_index = fat_offset / FAT32_SECTOR_SIZE as u64;
    if fat_sector_index >= u64::from(geometry.sectors_per_fat) {
        return Err(ArtifactReadError::Unavailable);
    }
    let fat_sector = u64::from(geometry.reserved_sectors)
        .checked_add(fat_sector_index)
        .ok_or(ArtifactReadError::OutOfRange)?;
    if cache.sector != Some(fat_sector) {
        read_partition_sector(
            reader,
            geometry.volume_sectors,
            fat_sector,
            &mut cache.bytes,
        )
        .map_err(|_| ArtifactReadError::Unavailable)?;
        cache.sector = Some(fat_sector);
    }
    let entry_offset = (fat_offset % FAT32_SECTOR_SIZE as u64) as usize;
    let next = read_u32(&cache.bytes, entry_offset) & 0x0fff_ffff;
    if next >= FAT32_EOC_MIN {
        return Ok(None);
    }
    if next == FAT32_BAD_CLUSTER
        || next < 2
        || u64::from(next) >= geometry.cluster_count.saturating_add(2)
    {
        return Err(ArtifactReadError::Unavailable);
    }
    Ok(Some(next))
}

fn cluster_sector(geometry: Fat32Geometry, cluster: u32, sector_in_cluster: u8) -> Option<u64> {
    if cluster < 2
        || u64::from(cluster) >= geometry.cluster_count.checked_add(2)?
        || sector_in_cluster >= geometry.sectors_per_cluster
    {
        return None;
    }
    geometry
        .first_data_sector
        .checked_add(u64::from(cluster - 2).checked_mul(u64::from(geometry.sectors_per_cluster))?)?
        .checked_add(u64::from(sector_in_cluster))
}

fn read_partition_sector<R: ModelStoreSectorReader>(
    reader: &mut R,
    partition_sectors: u64,
    sector: u64,
    destination: &mut [u8; FAT32_SECTOR_SIZE],
) -> Result<(), Fat32ArtifactError> {
    if sector >= partition_sectors {
        return Err(Fat32ArtifactError::UnsupportedGeometry);
    }
    reader
        .read_sector(sector, destination)
        .map_err(|_| Fat32ArtifactError::BlockUnavailable)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec;
    use std::vec::Vec;

    use super::{
        model_store_short_name, Fat32ArtifactError, Fat32ArtifactReader, ModelStoreSectorReader,
        FAT32_SECTOR_SIZE,
    };
    use crate::{ArtifactId, ArtifactReadError, ModelArtifactReader};

    const TEST_SECTORS: usize = 128;

    struct MemoryPartition(Vec<u8>);

    impl ModelStoreSectorReader for MemoryPartition {
        fn read_sector(
            &mut self,
            partition_relative_sector: u64,
            destination: &mut [u8; FAT32_SECTOR_SIZE],
        ) -> Result<(), ArtifactReadError> {
            let start = usize::try_from(partition_relative_sector)
                .ok()
                .and_then(|sector| sector.checked_mul(FAT32_SECTOR_SIZE))
                .ok_or(ArtifactReadError::OutOfRange)?;
            let source = self
                .0
                .get(start..start + FAT32_SECTOR_SIZE)
                .ok_or(ArtifactReadError::OutOfRange)?;
            destination.copy_from_slice(source);
            Ok(())
        }
    }

    fn fixture_partition(artifact_id: &ArtifactId, content: &[u8]) -> MemoryPartition {
        let mut disk = vec![0u8; TEST_SECTORS * FAT32_SECTOR_SIZE];
        disk[0] = 0xeb;
        disk[1] = 0x58;
        disk[2] = 0x90;
        disk[11..13].copy_from_slice(&512u16.to_le_bytes());
        disk[13] = 1;
        disk[14..16].copy_from_slice(&1u16.to_le_bytes());
        disk[16] = 1;
        disk[21] = 0xf8;
        disk[32..36].copy_from_slice(&(TEST_SECTORS as u32).to_le_bytes());
        disk[36..40].copy_from_slice(&1u32.to_le_bytes());
        disk[44..48].copy_from_slice(&2u32.to_le_bytes());
        disk[510..512].copy_from_slice(&[0x55, 0xaa]);

        let fat_start = FAT32_SECTOR_SIZE;
        for (cluster, next) in [(2u32, 0x0fff_ffffu32), (3, 4), (4, 0x0fff_ffff)] {
            let start = fat_start + cluster as usize * 4;
            disk[start..start + 4].copy_from_slice(&next.to_le_bytes());
        }

        let root_start = 2 * FAT32_SECTOR_SIZE;
        let name = model_store_short_name(artifact_id);
        disk[root_start..root_start + 11].copy_from_slice(&name);
        disk[root_start + 11] = 0x20;
        disk[root_start + 20..root_start + 22].copy_from_slice(&0u16.to_le_bytes());
        disk[root_start + 26..root_start + 28].copy_from_slice(&3u16.to_le_bytes());
        disk[root_start + 28..root_start + 32]
            .copy_from_slice(&(content.len() as u32).to_le_bytes());
        disk[root_start + 32] = 0;

        let first_data_sector = 2;
        let cluster_3_start = (first_data_sector + (3 - 2)) * FAT32_SECTOR_SIZE;
        let cluster_4_start = (first_data_sector + (4 - 2)) * FAT32_SECTOR_SIZE;
        let split = content.len().min(FAT32_SECTOR_SIZE);
        disk[cluster_3_start..cluster_3_start + split].copy_from_slice(&content[..split]);
        if content.len() > split {
            disk[cluster_4_start..cluster_4_start + content.len() - split]
                .copy_from_slice(&content[split..]);
        }
        MemoryPartition(disk)
    }

    #[test]
    fn derives_stable_distinct_fat32_names_from_artifact_ids() {
        let granite = ArtifactId::new("ibm.granite-4.2-3b").unwrap();
        let qwen = ArtifactId::new("qwen.qwen3-4b").unwrap();
        let granite_name = model_store_short_name(&granite);
        assert_eq!(granite_name, model_store_short_name(&granite));
        assert_eq!(&granite_name, b"FKH8HNA6GGF");
        assert_ne!(granite_name, model_store_short_name(&qwen));
        assert_eq!(&granite_name[8..], b"GGF");
        assert!(granite_name[..8]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()));
    }

    #[test]
    fn reads_sequential_and_random_ranges_across_fragmented_clusters() {
        let artifact_id = ArtifactId::new("fixture.split").unwrap();
        let content = (0..800)
            .map(|index| (index / 512 + 1) as u8)
            .collect::<Vec<_>>();
        let reader = fixture_partition(&artifact_id, &content);
        let mut artifact = Fat32ArtifactReader::open(reader, TEST_SECTORS as u64, artifact_id)
            .expect("open model artifact");

        assert_eq!(artifact.len(), content.len() as u64);
        let mut sequential = vec![0u8; 800];
        assert_eq!(artifact.read_at(0, &mut sequential), Ok(800));
        assert_eq!(sequential, content);
        let mut random = [0u8; 32];
        assert_eq!(artifact.read_at(500, &mut random), Ok(random.len()));
        assert_eq!(&random, &content[500..532]);
        assert_eq!(artifact.read_at(800, &mut random), Ok(0));
        assert_eq!(
            artifact.read_at(801, &mut random),
            Err(ArtifactReadError::OutOfRange)
        );
        assert!(artifact.verified_integrity().is_none());
    }

    #[test]
    fn reports_missing_artifacts_and_truncated_cluster_chains() {
        let artifact_id = ArtifactId::new("fixture.missing").unwrap();
        let other_id = ArtifactId::new("fixture.other").unwrap();
        let other_volume = fixture_partition(&other_id, &[1, 2, 3]);
        assert_eq!(
            Fat32ArtifactReader::open(other_volume, TEST_SECTORS as u64, artifact_id).err(),
            Some(Fat32ArtifactError::ArtifactNotFound)
        );

        let found_id = ArtifactId::new("fixture.short").unwrap();
        let mut reader = fixture_partition(&found_id, &[0x5a; 800]);
        let cluster_3_next = FAT32_SECTOR_SIZE + 3 * 4;
        reader.0[cluster_3_next..cluster_3_next + 4].copy_from_slice(&0x0fff_ffffu32.to_le_bytes());
        let mut artifact =
            Fat32ArtifactReader::open(reader, TEST_SECTORS as u64, found_id).unwrap();
        let mut all = [0u8; 800];
        assert_eq!(
            artifact.read_at(0, &mut all),
            Err(ArtifactReadError::Unavailable)
        );
    }
}
