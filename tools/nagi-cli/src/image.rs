use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const IMAGE_SIZE: usize = 1_474_560;
// One sector of boot data, 12 sectors in each FAT, 14 root-directory sectors,
// and exactly 4,084 32 KiB data clusters. This is the largest valid FAT12
// volume geometry and leaves room for the current Servo init ELF plus kernel.
pub const M17_IMAGE_SIZE: usize = 261_415 * SECTOR_SIZE;
pub const PERSISTENT_DISK_SIZE: u64 = 18 * 1024 * 1024;
pub const NAGI_WRITE_MARKER: &str = "Nagi M7 persistent write PASS";
pub const GUEST_ACCEPTANCE_MARKER: &str = "Nagi M7 acceptance PASS";
const M9_GUI_READY_MARKER: &str = "Nagi M9 window READY";
const M9_GUI_EVENTS: [&str; 2] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":48}},{"type":"rel","data":{"axis":"y","value":16}},{"type":"btn","data":{"button":"left","down":true}},{"type":"btn","data":{"button":"left","down":false}},{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
];
const SECTOR_SIZE: usize = 512;
const RESERVED_SECTORS: usize = 1;
const FAT_COUNT: usize = 2;
const SECTORS_PER_FAT: usize = 9;
const ROOT_ENTRY_COUNT: usize = 224;
const M17_SECTORS_PER_CLUSTER: usize = 64;
#[cfg(test)]
const ROOT_DIRECTORY_SECTORS: usize = ROOT_ENTRY_COUNT * 32 / SECTOR_SIZE;
#[cfg(test)]
const ROOT_OFFSET: usize = (RESERVED_SECTORS + FAT_COUNT * SECTORS_PER_FAT) * SECTOR_SIZE;
#[cfg(test)]
const DATA_OFFSET: usize =
    (RESERVED_SECTORS + FAT_COUNT * SECTORS_PER_FAT + ROOT_DIRECTORY_SECTORS) * SECTOR_SIZE;
const END_OF_CHAIN: u16 = 0x0fff;
const LEGACY_PERSISTENT_DISK_SIZE: u64 = 16 * 1024 * 1024;
const USER_DATA_START_LBA: u64 = 2048;
const REFERENCE_DISK_SIZE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const REFERENCE_DISK_SECTORS: u64 = REFERENCE_DISK_SIZE_BYTES / 512;
const MIB_SECTORS: u64 = 1024 * 1024 / 512;
const GIB_SECTORS: u64 = 1024 * MIB_SECTORS;
const QMP_MAX_LINE_BYTES: usize = 64 * 1024;
const QMP_TIMEOUT_DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fat12Geometry {
    image_size: usize,
    sectors_per_cluster: usize,
    sectors_per_fat: usize,
    root_entry_count: usize,
}

impl Fat12Geometry {
    fn legacy() -> Self {
        Self {
            image_size: IMAGE_SIZE,
            sectors_per_cluster: 1,
            sectors_per_fat: SECTORS_PER_FAT,
            root_entry_count: ROOT_ENTRY_COUNT,
        }
    }

    fn new(
        image_size: usize,
        sectors_per_cluster: usize,
        root_entry_count: usize,
    ) -> Result<Self, String> {
        // `usize::is_multiple_of` is unstable on the repository's pinned
        // nightly; retain the equivalent check until that API is available.
        #[allow(unknown_lints, clippy::manual_is_multiple_of)]
        if image_size == 0 || image_size % SECTOR_SIZE != 0 {
            return Err("FAT12 image size must be a nonzero whole number of sectors".to_owned());
        }
        if sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || sectors_per_cluster > 64
        {
            return Err("FAT12 cluster size must be a power of two up to 32 KiB".to_owned());
        }
        if root_entry_count == 0 || root_entry_count > u16::MAX as usize {
            return Err("FAT12 root directory entry count is unsupported".to_owned());
        }

        let total_sectors = image_size / SECTOR_SIZE;
        if total_sectors > u32::MAX as usize {
            return Err("FAT12 image sector count exceeds the BPB limit".to_owned());
        }
        let root_directory_sectors = (root_entry_count * 32).div_ceil(SECTOR_SIZE);
        let mut sectors_per_fat = 1;
        for _ in 0..8 {
            let overhead = RESERVED_SECTORS
                .checked_add(FAT_COUNT * sectors_per_fat)
                .and_then(|sectors| sectors.checked_add(root_directory_sectors))
                .ok_or_else(|| "FAT12 metadata size overflow".to_owned())?;
            let data_sectors = total_sectors
                .checked_sub(overhead)
                .ok_or_else(|| "FAT12 image is too small for its metadata".to_owned())?;
            let cluster_count = data_sectors / sectors_per_cluster;
            if cluster_count == 0 || cluster_count > 4_084 {
                return Err(format!(
                    "FAT12 image geometry requires {cluster_count} data clusters; expected 1..=4084"
                ));
            }
            let fat_bytes = ((cluster_count + 2) * 3).div_ceil(2);
            let required_sectors_per_fat = fat_bytes.div_ceil(SECTOR_SIZE);
            if required_sectors_per_fat == sectors_per_fat {
                return Ok(Self {
                    image_size,
                    sectors_per_cluster,
                    sectors_per_fat,
                    root_entry_count,
                });
            }
            sectors_per_fat = required_sectors_per_fat;
        }
        Err("could not resolve FAT12 allocation-table geometry".to_owned())
    }

    fn total_sectors(self) -> usize {
        self.image_size / SECTOR_SIZE
    }

    fn root_directory_sectors(self) -> usize {
        (self.root_entry_count * 32).div_ceil(SECTOR_SIZE)
    }

    fn root_offset(self) -> usize {
        (RESERVED_SECTORS + FAT_COUNT * self.sectors_per_fat) * SECTOR_SIZE
    }

    fn data_offset(self) -> usize {
        self.root_offset() + self.root_directory_sectors() * SECTOR_SIZE
    }

    fn data_sectors(self) -> usize {
        self.total_sectors() - self.data_offset() / SECTOR_SIZE
    }

    fn data_clusters(self) -> usize {
        self.data_sectors() / self.sectors_per_cluster
    }

    fn cluster_size(self) -> usize {
        self.sectors_per_cluster * SECTOR_SIZE
    }

    fn max_file_size(self) -> usize {
        self.data_clusters() * self.cluster_size()
    }

    fn fat_offset(self, fat_index: usize) -> usize {
        (RESERVED_SECTORS + fat_index * self.sectors_per_fat) * SECTOR_SIZE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLayout {
    pub bootloader_start_cluster: u16,
    pub bootloader_clusters: usize,
    pub kernel_start_cluster: u16,
    pub kernel_clusters: usize,
    pub init_start_cluster: u16,
    pub init_clusters: usize,
}

pub fn build_fat12_image(bootloader: &[u8], kernel: &[u8], init: &[u8]) -> Result<Vec<u8>, String> {
    build_fat12_image_with_geometry(bootloader, kernel, init, Fat12Geometry::legacy())
}

pub fn build_m17_fat12_image(
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
) -> Result<Vec<u8>, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    build_fat12_image_with_geometry(bootloader, kernel, init, geometry)
}

fn build_fat12_image_with_geometry(
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    geometry: Fat12Geometry,
) -> Result<Vec<u8>, String> {
    if bootloader.is_empty() {
        return Err("UEFI bootloader is empty".to_owned());
    }
    if kernel.is_empty() {
        return Err("Nagi kernel is empty".to_owned());
    }
    if init.is_empty() {
        return Err("nagi-init user ELF is empty".to_owned());
    }
    let max_file_size = geometry.max_file_size();
    for (name, contents) in [
        ("UEFI bootloader", bootloader),
        ("Nagi kernel", kernel),
        ("nagi-init user ELF", init),
    ] {
        if contents.len() > max_file_size {
            return Err(format!(
                "{name} is {} bytes; FAT12 image capacity is {max_file_size} bytes per file",
                contents.len()
            ));
        }
    }

    let bootloader_clusters = clusters_for(bootloader.len(), geometry.cluster_size());
    let kernel_clusters = clusters_for(kernel.len(), geometry.cluster_size());
    let init_clusters = clusters_for(init.len(), geometry.cluster_size());
    let required_clusters = 3 + bootloader_clusters + kernel_clusters + init_clusters;
    if required_clusters > geometry.data_clusters() {
        return Err(format!(
            "guest files require {required_clusters} FAT12 clusters; image has {} data clusters",
            geometry.data_clusters()
        ));
    }
    let layout = ImageLayout {
        bootloader_start_cluster: 5,
        bootloader_clusters,
        kernel_start_cluster: 5 + bootloader_clusters as u16,
        kernel_clusters,
        init_start_cluster: 5 + bootloader_clusters as u16 + kernel_clusters as u16,
        init_clusters,
    };

    let mut image = vec![0; geometry.image_size];
    write_boot_sector(&mut image, geometry);
    initialize_fats(&mut image, geometry);
    write_chain(&mut image, geometry, 2, 1);
    write_chain(&mut image, geometry, 3, 1);
    write_chain(&mut image, geometry, 4, 1);
    write_chain(
        &mut image,
        geometry,
        layout.bootloader_start_cluster,
        layout.bootloader_clusters,
    );
    write_chain(
        &mut image,
        geometry,
        layout.kernel_start_cluster,
        layout.kernel_clusters,
    );
    write_chain(
        &mut image,
        geometry,
        layout.init_start_cluster,
        layout.init_clusters,
    );

    let efi_cluster = 2;
    let boot_cluster = 3;
    let nagi_cluster = 4;
    write_directory(
        &mut image,
        geometry,
        efi_cluster,
        0,
        &[
            (short_name("BOOT", ""), 0x10, boot_cluster, 0),
            (short_name("NAGI", ""), 0x10, nagi_cluster, 0),
        ],
    );
    write_directory(
        &mut image,
        geometry,
        boot_cluster,
        efi_cluster,
        &[(
            short_name("BOOTX64", "EFI"),
            0x20,
            layout.bootloader_start_cluster,
            u32::try_from(bootloader.len()).map_err(|_| "bootloader size overflow".to_owned())?,
        )],
    );
    write_directory(
        &mut image,
        geometry,
        nagi_cluster,
        efi_cluster,
        &[
            (
                short_name("KERNEL", "ELF"),
                0x20,
                layout.kernel_start_cluster,
                u32::try_from(kernel.len()).map_err(|_| "kernel size overflow".to_owned())?,
            ),
            (
                short_name("INIT", "ELF"),
                0x20,
                layout.init_start_cluster,
                u32::try_from(init.len()).map_err(|_| "init size overflow".to_owned())?,
            ),
        ],
    );
    write_root_directory(
        &mut image,
        geometry,
        &[(short_name("EFI", ""), 0x10, efi_cluster, 0)],
    );
    write_file(
        &mut image,
        geometry,
        layout.bootloader_start_cluster,
        bootloader,
    );
    write_file(&mut image, geometry, layout.kernel_start_cluster, kernel);
    write_file(&mut image, geometry, layout.init_start_cluster, init);
    Ok(image)
}

pub fn write_fat12_image(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    _recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    write_fat12_image_with_geometry(path, bootloader, kernel, init, Fat12Geometry::legacy())
}

pub fn write_m17_fat12_image(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    _recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    write_fat12_image_with_geometry(path, bootloader, kernel, init, geometry)
}

/// Build the M27 acceptance image with a valid System A and intentionally
/// malformed System B kernel ELF.
pub fn write_m27_broken_slot_image(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    let (image, layout) = build_fat12_ab_image(
        bootloader,
        AbSlotImages {
            system_a: SlotPayload { kernel, init },
            system_b: SlotPayload {
                kernel: b"Nagi M27 intentionally invalid slot B kernel ELF",
                init,
            },
            recovery: recovery_init.map(|init| SlotPayload { kernel, init }),
        },
        geometry,
    )?;
    fs::write(path, image).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(layout)
}

/// Build an M27 acceptance image with the same valid kernel and init in both
/// System A and the System B trial candidate.
pub fn write_m27_healthy_slot_image(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    let (image, layout) = build_fat12_ab_image(
        bootloader,
        AbSlotImages {
            system_a: SlotPayload { kernel, init },
            system_b: SlotPayload { kernel, init },
            recovery: recovery_init.map(|init| SlotPayload { kernel, init }),
        },
        geometry,
    )?;
    fs::write(path, image).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(layout)
}

/// Build a Recovery-only acceptance image whose System A and B kernels are
/// both intentionally malformed while the separate Recovery payload is valid.
pub fn write_m27_recovery_image(
    path: &Path,
    bootloader: &[u8],
    recovery_kernel: &[u8],
    _system_init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    let recovery_init = recovery_init.unwrap_or(b"invalid recovery init");
    let (image, layout) = build_fat12_ab_image(
        bootloader,
        AbSlotImages {
            system_a: SlotPayload {
                kernel: b"invalid System A kernel",
                init: b"invalid System A init",
            },
            system_b: SlotPayload {
                kernel: b"invalid System B kernel",
                init: b"invalid System B init",
            },
            recovery: Some(SlotPayload {
                kernel: recovery_kernel,
                init: recovery_init,
            }),
        },
        geometry,
    )?;
    fs::write(path, image).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(layout)
}

pub fn write_reference_disk_qcow2(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    write_reference_disk_qcow2_with_system_b_kernel(
        path,
        bootloader,
        kernel,
        kernel,
        init,
        recovery_init,
    )
}

/// Build a disposable GPT image containing the deterministic M20 FAT32 reader
/// fixture in Model Store. Release images continue to use the empty store.
pub fn write_m20_model_store_fixture_reference_disk_qcow2(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    let filename = m20_model_store_fixture_filename()?;
    let files = [super::fat32::VolumeFile {
        path: &filename,
        contents: &super::m20_model_store_fixture::FIXTURE_BYTES,
    }];
    write_reference_disk_qcow2_with_system_b_kernel_and_model_store(
        path,
        bootloader,
        kernel,
        kernel,
        init,
        recovery_init,
        &files,
    )
}

/// Build a GPT acceptance image with a deliberately malformed System B
/// kernel while keeping System A and Recovery bootable.
pub fn write_m27_gpt_broken_system_b_qcow2(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    write_reference_disk_qcow2_with_system_b_kernel(
        path,
        bootloader,
        kernel,
        b"invalid System B kernel",
        init,
        recovery_init,
    )
}

fn write_reference_disk_qcow2_with_system_b_kernel(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    system_b_kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
) -> Result<ImageLayout, String> {
    write_reference_disk_qcow2_with_system_b_kernel_and_model_store(
        path,
        bootloader,
        kernel,
        system_b_kernel,
        init,
        recovery_init,
        &[],
    )
}

fn write_reference_disk_qcow2_with_system_b_kernel_and_model_store(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    system_b_kernel: &[u8],
    init: &[u8],
    recovery_init: Option<&[u8]>,
    model_store_files: &[super::fat32::VolumeFile<'_>],
) -> Result<ImageLayout, String> {
    if bootloader.is_empty() || kernel.is_empty() || system_b_kernel.is_empty() || init.is_empty() {
        return Err("GPT image requires non-empty loader, system kernels, and init ELF".to_owned());
    }
    let recovery_init = recovery_init
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| "release image requires the M27 Recovery init ELF".to_owned())?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(format!(
                "refusing to overwrite release image {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect release image path {}: {error}",
                path.display()
            ));
        }
    }
    let staging_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("cannot create release staging identifier: {error}"))?
        .as_nanos();
    let raw_path = path_with_suffix(path, &format!(".raw-staging-{staging_id}"));
    let qcow_staging_path = path_with_suffix(path, &format!(".qcow2-staging-{staging_id}"));
    for staging in [&raw_path, &qcow_staging_path] {
        match fs::symlink_metadata(staging) {
            Ok(_) => {
                return Err(format!(
                    "release image staging file already exists; preserving it: {}",
                    staging.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect release image staging path {}: {error}",
                    staging.display()
                ));
            }
        }
    }

    let mut raw = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&raw_path)
        .map_err(|error| format!("cannot create release disk staging file: {error}"))?;
    raw.set_len(REFERENCE_DISK_SIZE_BYTES)
        .map_err(|error| format!("cannot size 64 GiB release disk: {error}"))?;
    let partitions = reference_partitions()?;
    super::gpt::write_gpt(&mut raw, REFERENCE_DISK_SECTORS, &partitions)?;
    let esp = &partitions[0];
    let system_a = &partitions[1];
    let system_b = &partitions[2];
    let user_data = &partitions[3];
    let recovery = &partitions[4];
    let model_store = &partitions[5];

    let esp_layout = super::fat32::format_partition(
        &mut raw,
        esp.first_lba,
        partition_sector_count(esp)?,
        "NAGI ESP",
        &[super::fat32::VolumeFile {
            path: "EFI/BOOT/BOOTX64.EFI",
            contents: bootloader,
        }],
    )?;
    let a_layout = super::fat32::format_partition(
        &mut raw,
        system_a.first_lba,
        partition_sector_count(system_a)?,
        "NAGI SYS A",
        &[
            super::fat32::VolumeFile {
                path: "KERNEL.ELF",
                contents: kernel,
            },
            super::fat32::VolumeFile {
                path: "INIT.ELF",
                contents: init,
            },
        ],
    )?;
    super::fat32::format_partition(
        &mut raw,
        system_b.first_lba,
        partition_sector_count(system_b)?,
        "NAGI SYS B",
        &[
            super::fat32::VolumeFile {
                path: "KERNEL.ELF",
                contents: system_b_kernel,
            },
            super::fat32::VolumeFile {
                path: "INIT.ELF",
                contents: init,
            },
        ],
    )?;
    if partition_sector_count(user_data)? < 16_384 {
        return Err("release User Data partition is smaller than the VFS geometry".to_owned());
    }
    super::fat32::format_partition(
        &mut raw,
        recovery.first_lba,
        partition_sector_count(recovery)?,
        "NAGI RECOV",
        &[
            super::fat32::VolumeFile {
                path: "KERNEL.ELF",
                contents: kernel,
            },
            super::fat32::VolumeFile {
                path: "INIT.ELF",
                contents: recovery_init,
            },
        ],
    )?;
    super::fat32::format_partition(
        &mut raw,
        model_store.first_lba,
        partition_sector_count(model_store)?,
        "NAGI MODELS",
        model_store_files,
    )?;
    raw.sync_all()
        .map_err(|error| format!("cannot flush raw release image: {error}"))?;
    drop(raw);

    let conversion = ProcessCommand::new("qemu-img")
        .args(["convert", "-S", "4k", "-f", "raw", "-O", "qcow2"])
        .arg(&raw_path)
        .arg(&qcow_staging_path)
        .output()
        .map_err(|error| format!("cannot run qemu-img convert: {error}"))?;
    if !conversion.status.success() {
        return Err(format!(
            "qemu-img convert failed ({}): {}",
            conversion.status,
            String::from_utf8_lossy(&conversion.stderr).trim()
        ));
    }
    validate_reference_disk_qcow2(&qcow_staging_path)?;
    fs::hard_link(&qcow_staging_path, path).map_err(|error| {
        format!(
            "cannot install release qcow2 image {}: {error}",
            path.display()
        )
    })?;
    fs::remove_file(&qcow_staging_path).map_err(|error| {
        format!(
            "release image is ready but staging cleanup failed at {}: {error}",
            qcow_staging_path.display()
        )
    })?;
    fs::remove_file(&raw_path).map_err(|error| {
        format!(
            "release image is ready but raw staging cleanup failed at {}: {error}",
            raw_path.display()
        )
    })?;

    let bootloader = esp_layout
        .first()
        .ok_or_else(|| "release ESP omitted its loader".to_owned())?;
    let kernel = a_layout
        .first()
        .ok_or_else(|| "System A volume omitted its kernel".to_owned())?;
    let init = a_layout
        .get(1)
        .ok_or_else(|| "System A volume omitted its init".to_owned())?;
    Ok(ImageLayout {
        bootloader_start_cluster: u16::try_from(bootloader.first_cluster)
            .map_err(|_| "release ESP loader cluster exceeds the diagnostic field".to_owned())?,
        bootloader_clusters: bootloader.cluster_count as usize,
        kernel_start_cluster: u16::try_from(kernel.first_cluster)
            .map_err(|_| "System A kernel cluster exceeds the diagnostic field".to_owned())?,
        kernel_clusters: kernel.cluster_count as usize,
        init_start_cluster: u16::try_from(init.first_cluster)
            .map_err(|_| "System A init cluster exceeds the diagnostic field".to_owned())?,
        init_clusters: init.cluster_count as usize,
    })
}

fn m20_model_store_fixture_filename() -> Result<String, String> {
    let artifact_id =
        nagi_model_manager::ArtifactId::new(super::m20_model_store_fixture::ARTIFACT_ID)
            .map_err(|error| format!("invalid M20 fixture artifact ID: {error}"))?;
    let short_name = nagi_model_manager::model_store_short_name(&artifact_id);
    let basename = std::str::from_utf8(&short_name[..8])
        .map_err(|error| format!("invalid M20 fixture short name: {error}"))?;
    let extension = std::str::from_utf8(&short_name[8..])
        .map_err(|error| format!("invalid M20 fixture extension: {error}"))?;
    Ok(format!("{basename}.{extension}"))
}

pub fn validate_reference_disk_qcow2(path: &Path) -> Result<(), String> {
    let info = ProcessCommand::new("qemu-img")
        .args(["info", "--output=json"])
        .arg(path)
        .output()
        .map_err(|error| format!("cannot inspect release qcow2 image: {error}"))?;
    if !info.status.success() {
        return Err(format!(
            "qemu-img info failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&info.stderr).trim()
        ));
    }
    let info = String::from_utf8_lossy(&info.stdout);
    if json_string_field(&info, "format") != Some("qcow2".to_owned())
        || json_u64_field(&info, "virtual-size") != Some(REFERENCE_DISK_SIZE_BYTES)
    {
        return Err(format!(
            "release image {} has unexpected format or virtual size: {}",
            path.display(),
            info.trim()
        ));
    }
    Ok(())
}

fn reference_partitions() -> Result<Vec<super::gpt::GptPartition>, String> {
    let mut start = 2048u64;
    let mut partitions = Vec::with_capacity(6);
    let sizes = [
        512 * MIB_SECTORS,
        4 * GIB_SECTORS,
        4 * GIB_SECTORS,
        16 * GIB_SECTORS,
        4 * GIB_SECTORS,
        32 * GIB_SECTORS,
    ];
    let names = [
        "ESP",
        "System A",
        "System B",
        "User Data",
        "Recovery",
        "Model Store",
    ];
    for (index, size) in sizes.into_iter().enumerate() {
        let first_lba = start;
        let last_lba = first_lba
            .checked_add(size)
            .and_then(|end| end.checked_sub(1))
            .ok_or_else(|| "release partition end overflow".to_owned())?;
        let partition = if index == 0 {
            super::gpt::efi_system_partition(first_lba, last_lba, super::gpt::ESP_PARTITION_GUID)
        } else {
            super::gpt::nagi_partition(index as u8, first_lba, last_lba, names[index])
        };
        partitions.push(partition);
        start = last_lba
            .checked_add(1)
            .and_then(|end| end.checked_add(2047))
            .map(|end| end & !2047)
            .ok_or_else(|| "release partition alignment overflow".to_owned())?;
    }
    let last_usable_lba = REFERENCE_DISK_SECTORS - 34;
    if partitions
        .last()
        .is_some_and(|partition| partition.last_lba > last_usable_lba)
    {
        return Err("64 GiB release layout does not fit the disk".to_owned());
    }
    Ok(partitions)
}

fn partition_sector_count(partition: &super::gpt::GptPartition) -> Result<u64, String> {
    partition
        .last_lba
        .checked_sub(partition.first_lba)
        .and_then(|sectors| sectors.checked_add(1))
        .ok_or_else(|| format!("invalid GPT partition bounds for {}", partition.name))
}

fn json_string_field(json: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let after_key = json.get(json.rfind(&key)? + key.len()..)?;
    let value = after_key.get(after_key.find(':')? + 1..)?.trim_start();
    let value = value.strip_prefix('"')?;
    Some(value.get(..value.find('"')?)?.to_owned())
}

fn json_u64_field(json: &str, field: &str) -> Option<u64> {
    let key = format!("\"{field}\"");
    let after_key = json.get(json.rfind(&key)? + key.len()..)?;
    let value = after_key.get(after_key.find(':')? + 1..)?.trim_start();
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    value.get(..digits)?.parse().ok()
}

#[derive(Clone, Copy)]
struct SlotPayload<'a> {
    kernel: &'a [u8],
    init: &'a [u8],
}

#[derive(Clone, Copy)]
struct AbSlotImages<'a> {
    system_a: SlotPayload<'a>,
    system_b: SlotPayload<'a>,
    recovery: Option<SlotPayload<'a>>,
}

fn build_fat12_ab_image(
    bootloader: &[u8],
    slots: AbSlotImages<'_>,
    geometry: Fat12Geometry,
) -> Result<(Vec<u8>, ImageLayout), String> {
    let system_a_kernel = slots.system_a.kernel;
    let system_a_init = slots.system_a.init;
    let system_b_kernel = slots.system_b.kernel;
    let system_b_init = slots.system_b.init;
    let recovery_kernel = slots
        .recovery
        .map(|recovery| recovery.kernel)
        .unwrap_or(system_a_kernel);
    let recovery_init = slots
        .recovery
        .map(|recovery| recovery.init)
        .unwrap_or(system_a_init);
    for (name, contents) in [
        ("UEFI bootloader", bootloader),
        ("System A kernel", system_a_kernel),
        ("System A init", system_a_init),
        ("System B kernel", system_b_kernel),
        ("System B init", system_b_init),
        ("Recovery kernel", recovery_kernel),
        ("Recovery init", recovery_init),
    ] {
        if contents.is_empty() {
            return Err(format!("{name} is empty"));
        }
        if contents.len() > geometry.max_file_size() {
            return Err(format!(
                "{name} is {} bytes; FAT12 image capacity is {} bytes per file",
                contents.len(),
                geometry.max_file_size()
            ));
        }
    }

    let bootloader_clusters = clusters_for(bootloader.len(), geometry.cluster_size());
    let a_kernel_clusters = clusters_for(system_a_kernel.len(), geometry.cluster_size());
    let a_init_clusters = clusters_for(system_a_init.len(), geometry.cluster_size());
    let b_kernel_clusters = clusters_for(system_b_kernel.len(), geometry.cluster_size());
    let b_init_clusters = clusters_for(system_b_init.len(), geometry.cluster_size());
    let recovery_kernel_clusters = clusters_for(recovery_kernel.len(), geometry.cluster_size());
    let recovery_init_clusters = clusters_for(recovery_init.len(), geometry.cluster_size());
    let required_clusters = [
        6,
        bootloader_clusters,
        a_kernel_clusters,
        a_init_clusters,
        b_kernel_clusters,
        b_init_clusters,
        recovery_kernel_clusters,
        recovery_init_clusters,
    ]
    .into_iter()
    .try_fold(0usize, usize::checked_add)
    .ok_or_else(|| "A/B image cluster count overflow".to_owned())?;
    if required_clusters > geometry.data_clusters() {
        return Err(format!(
            "A/B guest files require {required_clusters} FAT12 clusters; image has {} data clusters",
            geometry.data_clusters()
        ));
    }

    const EFI_CLUSTER: u16 = 2;
    const BOOT_CLUSTER: u16 = 3;
    const NAGI_CLUSTER: u16 = 4;
    const SYSTEM_A_CLUSTER: u16 = 5;
    const SYSTEM_B_CLUSTER: u16 = 6;
    const RECOVERY_CLUSTER: u16 = 7;
    let mut next_cluster = 8_u16;
    let bootloader_start_cluster = allocate_chain_start(&mut next_cluster, bootloader_clusters)?;
    let a_kernel_start_cluster = allocate_chain_start(&mut next_cluster, a_kernel_clusters)?;
    let a_init_start_cluster = allocate_chain_start(&mut next_cluster, a_init_clusters)?;
    let b_kernel_start_cluster = allocate_chain_start(&mut next_cluster, b_kernel_clusters)?;
    let b_init_start_cluster = allocate_chain_start(&mut next_cluster, b_init_clusters)?;
    let recovery_kernel_start_cluster =
        allocate_chain_start(&mut next_cluster, recovery_kernel_clusters)?;
    let recovery_init_start_cluster =
        allocate_chain_start(&mut next_cluster, recovery_init_clusters)?;
    let layout = ImageLayout {
        bootloader_start_cluster,
        bootloader_clusters,
        kernel_start_cluster: a_kernel_start_cluster,
        kernel_clusters: a_kernel_clusters,
        init_start_cluster: a_init_start_cluster,
        init_clusters: a_init_clusters,
    };

    let mut image = vec![0; geometry.image_size];
    write_boot_sector(&mut image, geometry);
    initialize_fats(&mut image, geometry);
    for directory_cluster in [
        EFI_CLUSTER,
        BOOT_CLUSTER,
        NAGI_CLUSTER,
        SYSTEM_A_CLUSTER,
        SYSTEM_B_CLUSTER,
        RECOVERY_CLUSTER,
    ] {
        write_chain(&mut image, geometry, directory_cluster, 1);
    }
    for (start_cluster, cluster_count) in [
        (bootloader_start_cluster, bootloader_clusters),
        (a_kernel_start_cluster, a_kernel_clusters),
        (a_init_start_cluster, a_init_clusters),
        (b_kernel_start_cluster, b_kernel_clusters),
        (b_init_start_cluster, b_init_clusters),
        (recovery_kernel_start_cluster, recovery_kernel_clusters),
        (recovery_init_start_cluster, recovery_init_clusters),
    ] {
        write_chain(&mut image, geometry, start_cluster, cluster_count);
    }

    write_directory(
        &mut image,
        geometry,
        EFI_CLUSTER,
        0,
        &[
            (short_name("BOOT", ""), 0x10, BOOT_CLUSTER, 0),
            (short_name("NAGI", ""), 0x10, NAGI_CLUSTER, 0),
        ],
    );
    write_directory(
        &mut image,
        geometry,
        BOOT_CLUSTER,
        EFI_CLUSTER,
        &[(
            short_name("BOOTX64", "EFI"),
            0x20,
            bootloader_start_cluster,
            u32::try_from(bootloader.len()).map_err(|_| "bootloader size overflow".to_owned())?,
        )],
    );
    write_directory(
        &mut image,
        geometry,
        NAGI_CLUSTER,
        EFI_CLUSTER,
        &[
            (short_name("SYSTEMA", ""), 0x10, SYSTEM_A_CLUSTER, 0),
            (short_name("SYSTEMB", ""), 0x10, SYSTEM_B_CLUSTER, 0),
            (short_name("RECOVERY", ""), 0x10, RECOVERY_CLUSTER, 0),
        ],
    );
    write_directory(
        &mut image,
        geometry,
        RECOVERY_CLUSTER,
        NAGI_CLUSTER,
        &[
            (
                short_name("KERNEL", "ELF"),
                0x20,
                recovery_kernel_start_cluster,
                u32::try_from(recovery_kernel.len())
                    .map_err(|_| "Recovery kernel size overflow".to_owned())?,
            ),
            (
                short_name("INIT", "ELF"),
                0x20,
                recovery_init_start_cluster,
                u32::try_from(recovery_init.len())
                    .map_err(|_| "Recovery init size overflow".to_owned())?,
            ),
        ],
    );
    write_directory(
        &mut image,
        geometry,
        SYSTEM_A_CLUSTER,
        NAGI_CLUSTER,
        &[
            (
                short_name("KERNEL", "ELF"),
                0x20,
                a_kernel_start_cluster,
                u32::try_from(system_a_kernel.len())
                    .map_err(|_| "System A kernel size overflow".to_owned())?,
            ),
            (
                short_name("INIT", "ELF"),
                0x20,
                a_init_start_cluster,
                u32::try_from(system_a_init.len())
                    .map_err(|_| "System A init size overflow".to_owned())?,
            ),
        ],
    );
    write_directory(
        &mut image,
        geometry,
        SYSTEM_B_CLUSTER,
        NAGI_CLUSTER,
        &[
            (
                short_name("KERNEL", "ELF"),
                0x20,
                b_kernel_start_cluster,
                u32::try_from(system_b_kernel.len())
                    .map_err(|_| "System B kernel size overflow".to_owned())?,
            ),
            (
                short_name("INIT", "ELF"),
                0x20,
                b_init_start_cluster,
                u32::try_from(system_b_init.len())
                    .map_err(|_| "System B init size overflow".to_owned())?,
            ),
        ],
    );
    write_root_directory(
        &mut image,
        geometry,
        &[(short_name("EFI", ""), 0x10, EFI_CLUSTER, 0)],
    );
    write_file(&mut image, geometry, bootloader_start_cluster, bootloader);
    write_file(
        &mut image,
        geometry,
        a_kernel_start_cluster,
        system_a_kernel,
    );
    write_file(&mut image, geometry, a_init_start_cluster, system_a_init);
    write_file(
        &mut image,
        geometry,
        b_kernel_start_cluster,
        system_b_kernel,
    );
    write_file(&mut image, geometry, b_init_start_cluster, system_b_init);
    write_file(
        &mut image,
        geometry,
        recovery_kernel_start_cluster,
        recovery_kernel,
    );
    write_file(
        &mut image,
        geometry,
        recovery_init_start_cluster,
        recovery_init,
    );
    Ok((image, layout))
}

fn allocate_chain_start(next_cluster: &mut u16, cluster_count: usize) -> Result<u16, String> {
    let start_cluster = *next_cluster;
    let cluster_count = u16::try_from(cluster_count)
        .map_err(|_| "A/B image chain length exceeds the FAT12 cluster range".to_owned())?;
    *next_cluster = next_cluster
        .checked_add(cluster_count)
        .ok_or_else(|| "A/B image cluster allocation overflow".to_owned())?;
    Ok(start_cluster)
}

fn write_fat12_image_with_geometry(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
    geometry: Fat12Geometry,
) -> Result<ImageLayout, String> {
    let image = build_fat12_image_with_geometry(bootloader, kernel, init, geometry)?;
    fs::write(path, image).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let bootloader_clusters = clusters_for(bootloader.len(), geometry.cluster_size());
    let kernel_clusters = clusters_for(kernel.len(), geometry.cluster_size());
    Ok(ImageLayout {
        bootloader_start_cluster: 5,
        bootloader_clusters,
        kernel_start_cluster: 5 + bootloader_clusters as u16,
        kernel_clusters,
        init_start_cluster: 5 + bootloader_clusters as u16 + kernel_clusters as u16,
        init_clusters: clusters_for(init.len(), geometry.cluster_size()),
    })
}

pub fn ensure_persistent_disk(path: &Path) -> Result<bool, String> {
    match fs::metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(format!(
                    "persistent data disk is not a regular file: {}",
                    path.display()
                ));
            }
            if metadata.len() < (super::gpt::GPT_ENTRY_ARRAY_SECTORS + 4) * 512
                || metadata.len() % 512 != 0
            {
                return Err(format!(
                    "persistent data disk has unsupported size {} bytes: {}",
                    metadata.len(),
                    path.display()
                ));
            }
            let sectors = metadata.len() / 512;
            let mut disk = fs::File::open(path).map_err(|error| {
                format!(
                    "cannot read persistent data disk {}: {error}",
                    path.display()
                )
            })?;
            match super::gpt::read_user_data_partition(&mut disk, sectors) {
                Ok(partition) => {
                    if partition.sector_count < 16_384 {
                        return Err(format!(
                            "persistent data partition is smaller than the 8 MiB VFS geometry: {}",
                            path.display()
                        ));
                    }
                    Ok(true)
                }
                Err(gpt_error) if metadata.len() == LEGACY_PERSISTENT_DISK_SIZE => {
                    if looks_like_gpt(path)? {
                        return Err(format!(
                            "persistent data disk has invalid GPT metadata ({}): {}",
                            gpt_error,
                            path.display()
                        ));
                    }
                    migrate_legacy_persistent_disk(path)
                }
                Err(gpt_error) => Err(format!(
                    "persistent data disk has invalid GPT metadata ({}): {}",
                    gpt_error,
                    path.display()
                )),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_persistent_data_disk(path)?;
            Ok(false)
        }
        Err(error) => Err(format!(
            "cannot inspect persistent data disk {}: {error}",
            path.display()
        )),
    }
}

fn create_persistent_data_disk(path: &Path) -> Result<(), String> {
    let mut disk = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "cannot create persistent data disk {}: {error}",
                path.display()
            )
        })?;
    if let Err(error) = initialize_partitioned_data_disk(&mut disk, None) {
        drop(disk);
        let _ = fs::remove_file(path);
        return Err(format!(
            "cannot initialize persistent data disk {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

fn migrate_legacy_persistent_disk(path: &Path) -> Result<bool, String> {
    let temporary_path = path_with_suffix(path, ".gpt-migration");
    let backup_path = path_with_suffix(path, ".legacy-raw");
    if fs::symlink_metadata(&temporary_path).is_ok() || fs::symlink_metadata(&backup_path).is_ok() {
        return Err(format!(
            "persistent data migration sidecar already exists; preserving all files: {}",
            path.display()
        ));
    }

    let mut migrated = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&temporary_path)
    {
        Ok(file) => file,
        Err(error) => {
            return Err(format!(
                "cannot create GPT migration image {}: {error}",
                temporary_path.display()
            ));
        }
    };
    let contains_data = match initialize_partitioned_data_disk(&mut migrated, Some(path)) {
        Ok(contains_data) => contains_data,
        Err(error) => {
            drop(migrated);
            let _ = fs::remove_file(&temporary_path);
            return Err(format!("cannot migrate persistent data disk: {error}"));
        }
    };
    drop(migrated);

    fs::rename(path, &backup_path).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        format!(
            "cannot preserve legacy persistent data disk {}: {error}",
            path.display()
        )
    })?;
    if let Err(error) = fs::rename(&temporary_path, path) {
        return match fs::rename(&backup_path, path) {
            Ok(()) => Err(format!(
                "cannot install migrated persistent data disk {}: {error}",
                path.display()
            )),
            Err(restore_error) => Err(format!(
                "cannot install migrated disk ({error}) or restore original; original data remains at {} ({restore_error})",
                backup_path.display()
            )),
        };
    }
    Ok(contains_data)
}

fn initialize_partitioned_data_disk(
    disk: &mut fs::File,
    legacy_path: Option<&Path>,
) -> Result<bool, String> {
    disk.set_len(PERSISTENT_DISK_SIZE)
        .map_err(|error| format!("cannot size disk: {error}"))?;
    let contains_data = if let Some(legacy_path) = legacy_path {
        copy_legacy_disk_data(disk, legacy_path)?
    } else {
        false
    };
    let disk_sectors = PERSISTENT_DISK_SIZE / 512;
    super::gpt::write_gpt(
        disk,
        disk_sectors,
        &[super::gpt::user_data_partition(
            USER_DATA_START_LBA,
            disk_sectors - 34,
            super::gpt::USER_DATA_PARTITION_GUID,
        )],
    )?;
    disk.sync_all()
        .map_err(|error| format!("cannot flush GPT disk: {error}"))?;
    let partition = super::gpt::read_user_data_partition(disk, disk_sectors)?;
    if partition.start_lba != USER_DATA_START_LBA || partition.sector_count < 16_384 {
        return Err("generated GPT does not expose the expected User Data extent".to_owned());
    }
    Ok(contains_data)
}

fn copy_legacy_disk_data(disk: &mut fs::File, legacy_path: &Path) -> Result<bool, String> {
    let mut legacy = fs::File::open(legacy_path).map_err(|error| {
        format!(
            "cannot open legacy data disk {}: {error}",
            legacy_path.display()
        )
    })?;
    let legacy_length = legacy
        .metadata()
        .map_err(|error| format!("cannot inspect legacy data disk: {error}"))?
        .len();
    if legacy_length != LEGACY_PERSISTENT_DISK_SIZE {
        return Err(format!(
            "legacy data disk changed size during migration: {} bytes",
            legacy_length
        ));
    }
    let data_offset = USER_DATA_START_LBA
        .checked_mul(512)
        .ok_or_else(|| "User Data offset overflow".to_owned())?;
    disk.seek(SeekFrom::Start(data_offset))
        .map_err(|error| format!("cannot seek to User Data partition: {error}"))?;
    let mut contains_data = false;
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = legacy
            .read(&mut buffer)
            .map_err(|error| format!("cannot read legacy data disk: {error}"))?;
        if count == 0 {
            break;
        }
        contains_data |= buffer[..count].iter().any(|byte| *byte != 0);
        disk.write_all(&buffer[..count])
            .map_err(|error| format!("cannot copy legacy data disk: {error}"))?;
        copied += count as u64;
    }
    if copied != legacy_length {
        return Err(format!(
            "copied {copied} bytes from a {legacy_length}-byte legacy disk"
        ));
    }
    Ok(contains_data)
}

fn looks_like_gpt(path: &Path) -> Result<bool, String> {
    let mut disk = fs::File::open(path).map_err(|error| {
        format!(
            "cannot inspect persistent data disk {}: {error}",
            path.display()
        )
    })?;
    let mut mbr = [0u8; 512];
    disk.read_exact(&mut mbr)
        .map_err(|error| format!("cannot read persistent data disk MBR: {error}"))?;
    let mut header = [0u8; 8];
    disk.seek(SeekFrom::Start(512))
        .and_then(|_| disk.read_exact(&mut header))
        .map_err(|error| format!("cannot read persistent data disk header: {error}"))?;
    Ok(mbr[450] == 0xee || &header == b"EFI PART")
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub struct QemuConfig<'a> {
    pub qemu: &'a Path,
    pub ovmf_code: &'a Path,
    pub ovmf_vars_template: &'a Path,
    pub disk_image: &'a Path,
    pub persistent_disk: &'a Path,
    pub vars_copy: &'a Path,
    pub serial_log: &'a Path,
    pub acceptance_marker: &'a str,
    pub timeout: Duration,
}

#[derive(Clone, Copy)]
struct GuiQemuMode {
    boot_disk_read_only: bool,
    reuse_ovmf_vars: bool,
    inter_event_delay: Duration,
}

pub type InteractiveQemuConfig<'a> = QemuConfig<'a>;

fn write_boot_sector(image: &mut [u8], geometry: Fat12Geometry) {
    let boot = &mut image[..SECTOR_SIZE];
    boot[0..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    boot[3..11].copy_from_slice(b"NAGI OS ");
    write_u16(boot, 11, SECTOR_SIZE as u16);
    boot[13] = geometry.sectors_per_cluster as u8;
    write_u16(boot, 14, RESERVED_SECTORS as u16);
    boot[16] = FAT_COUNT as u8;
    write_u16(boot, 17, geometry.root_entry_count as u16);
    let total_sectors = geometry.total_sectors();
    let total_sectors_16 = u16::try_from(total_sectors).unwrap_or_default();
    write_u16(boot, 19, total_sectors_16);
    boot[21] = 0xf0;
    write_u16(boot, 22, geometry.sectors_per_fat as u16);
    write_u16(boot, 24, 18);
    write_u16(boot, 26, 2);
    write_u32(boot, 28, 0);
    write_u32(
        boot,
        32,
        if total_sectors_16 == 0 {
            total_sectors as u32
        } else {
            0
        },
    );
    boot[36] = 0;
    boot[38] = 0x29;
    write_u32(boot, 39, 0x4e41_4749);
    boot[43..54].copy_from_slice(b"NAGI ESP   ");
    boot[54..62].copy_from_slice(b"FAT12   ");
    boot[510..512].copy_from_slice(&[0x55, 0xaa]);
}

fn initialize_fats(image: &mut [u8], geometry: Fat12Geometry) {
    for fat_index in 0..FAT_COUNT {
        let offset = geometry.fat_offset(fat_index);
        image[offset..offset + 3].copy_from_slice(&[0xf0, 0xff, 0xff]);
    }
}

fn write_chain(
    image: &mut [u8],
    geometry: Fat12Geometry,
    first_cluster: u16,
    cluster_count: usize,
) {
    for index in 0..cluster_count {
        let cluster = first_cluster + index as u16;
        let next = if index + 1 == cluster_count {
            END_OF_CHAIN
        } else {
            cluster + 1
        };
        for fat_index in 0..FAT_COUNT {
            let fat = geometry.fat_offset(fat_index);
            set_fat12_entry(image, fat, cluster, next);
        }
    }
}

fn set_fat12_entry(image: &mut [u8], fat_offset: usize, cluster: u16, value: u16) {
    let offset = fat_offset + cluster as usize + cluster as usize / 2;
    if cluster & 1 == 0 {
        image[offset] = value as u8;
        image[offset + 1] = (image[offset + 1] & 0xf0) | ((value >> 8) as u8 & 0x0f);
    } else {
        image[offset] = (image[offset] & 0x0f) | ((value as u8) << 4);
        image[offset + 1] = (value >> 4) as u8;
    }
}

fn write_root_directory(
    image: &mut [u8],
    geometry: Fat12Geometry,
    entries: &[([u8; 11], u8, u16, u32)],
) {
    for (index, entry) in entries.iter().enumerate() {
        write_directory_entry(&mut image[geometry.root_offset()..], index, entry);
    }
}

fn write_directory(
    image: &mut [u8],
    geometry: Fat12Geometry,
    cluster: u16,
    parent_cluster: u16,
    entries: &[([u8; 11], u8, u16, u32)],
) {
    let offset = cluster_offset(cluster, geometry);
    let directory = &mut image[offset..offset + geometry.cluster_size()];
    write_directory_entry(directory, 0, &(short_name(".", ""), 0x10, cluster, 0));
    write_directory_entry(
        directory,
        1,
        &(short_name("..", ""), 0x10, parent_cluster, 0),
    );
    for (index, entry) in entries.iter().enumerate() {
        write_directory_entry(directory, index + 2, entry);
    }
}

fn write_directory_entry(directory: &mut [u8], index: usize, entry: &([u8; 11], u8, u16, u32)) {
    let offset = index * 32;
    directory[offset..offset + 11].copy_from_slice(&entry.0);
    directory[offset + 11] = entry.1;
    write_u16(directory, offset + 26, entry.2);
    write_u32(directory, offset + 28, entry.3);
}

fn write_file(image: &mut [u8], geometry: Fat12Geometry, first_cluster: u16, contents: &[u8]) {
    let offset = cluster_offset(first_cluster, geometry);
    image[offset..offset + contents.len()].copy_from_slice(contents);
}

fn cluster_offset(cluster: u16, geometry: Fat12Geometry) -> usize {
    geometry.data_offset() + (cluster as usize - 2) * geometry.cluster_size()
}

fn clusters_for(size: usize, cluster_size: usize) -> usize {
    size.div_ceil(cluster_size)
}

fn short_name(stem: &str, extension: &str) -> [u8; 11] {
    let mut name = [b' '; 11];
    for (target, source) in name[..8].iter_mut().zip(stem.bytes()) {
        *target = source.to_ascii_uppercase();
    }
    for (target, source) in name[8..].iter_mut().zip(extension.bytes()) {
        *target = source.to_ascii_uppercase();
    }
    name
}

fn write_u16(target: &mut [u8], offset: usize, value: u16) {
    target[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(target: &mut [u8], offset: usize, value: u32) {
    target[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn run_qemu(config: &QemuConfig<'_>) -> Result<i32, String> {
    run_qemu_with_boot_disk_mode(config, false)
}

pub fn run_qemu_until_any_acceptance_marker(
    config: &QemuConfig<'_>,
    acceptance_markers: &[&str],
) -> Result<i32, String> {
    run_qemu_with_qmp_file_markers(config, acceptance_markers, false, false)
}

pub fn run_qemu_with_read_only_boot_disk(config: &QemuConfig<'_>) -> Result<i32, String> {
    run_qemu_with_boot_disk_mode(config, true)
}

/// Launch QEMU without replacing the existing OVMF variable store.
pub fn run_qemu_reusing_ovmf_vars(config: &QemuConfig<'_>) -> Result<i32, String> {
    run_qemu_with_vars_mode(config, false, true)
}

/// Launch QEMU with an existing OVMF variable store and a read-only boot image.
pub fn run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(
    config: &QemuConfig<'_>,
) -> Result<i32, String> {
    run_qemu_with_vars_mode(config, true, true)
}

/// Initialize a per-run OVMF variable image from the configured template.
pub fn initialize_ovmf_vars(template: &Path, vars_copy: &Path) -> Result<(), String> {
    prepare_ovmf_vars(template, vars_copy, false)
}

fn run_qemu_with_boot_disk_mode(
    config: &QemuConfig<'_>,
    boot_disk_read_only: bool,
) -> Result<i32, String> {
    run_qemu_with_vars_mode(config, boot_disk_read_only, false)
}

fn run_qemu_with_vars_mode(
    config: &QemuConfig<'_>,
    boot_disk_read_only: bool,
    reuse_ovmf_vars: bool,
) -> Result<i32, String> {
    run_qemu_with_qmp_file_markers(
        config,
        &[config.acceptance_marker],
        boot_disk_read_only,
        reuse_ovmf_vars,
    )
}

fn run_qemu_with_qmp_file_markers(
    config: &QemuConfig<'_>,
    acceptance_markers: &[&str],
    boot_disk_read_only: bool,
    reuse_ovmf_vars: bool,
) -> Result<i32, String> {
    if acceptance_markers.is_empty() {
        return Err("QEMU acceptance requires at least one marker".to_owned());
    }
    let (qmp_listener, qmp_port) = reserve_local_tcp_listener("QMP")?;
    drop(qmp_listener);

    let serial_device = format!("file:{}", external_path(config.serial_log));
    let mut child = spawn_qemu_with_display_mode_and_vars(
        config,
        &serial_device,
        qmp_port,
        0,
        boot_disk_read_only,
        reuse_ovmf_vars,
        false,
    )?;
    let deadline = Instant::now() + config.timeout;
    let mut qmp_stream = match connect_guest_tcp(&mut child, qmp_port, deadline, "QMP endpoint") {
        Ok(stream) => stream,
        Err(error) => {
            terminate_qemu(&mut child, config.serial_log, &[]);
            return Err(error);
        }
    };
    qmp_stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| {
            terminate_qemu(&mut child, config.serial_log, &[]);
            format!("cannot configure QMP read timeout: {error}")
        })?;
    let greeting = match read_qmp_line(&mut qmp_stream, deadline) {
        Ok(greeting) => greeting,
        Err(error) => {
            terminate_qemu(&mut child, config.serial_log, &[]);
            return Err(error);
        }
    };
    if !greeting.contains("\"QMP\"") {
        terminate_qemu(&mut child, config.serial_log, &[]);
        return Err(format!("unexpected QMP greeting: {greeting}"));
    }
    if let Err(error) = qmp_exchange(
        &mut qmp_stream,
        r#"{"execute":"qmp_capabilities"}"#,
        deadline,
    ) {
        terminate_qemu(&mut child, config.serial_log, &[]);
        return Err(error);
    }

    wait_for_qemu_any_with_qmp(
        &mut child,
        &mut qmp_stream,
        config.serial_log,
        acceptance_markers,
        config.timeout,
    )
}

fn wait_for_qemu_any_with_qmp(
    child: &mut Child,
    qmp_stream: &mut TcpStream,
    serial_log: &Path,
    acceptance_markers: &[&str],
    timeout: Duration,
) -> Result<i32, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let serial = fs::read(serial_log).unwrap_or_default();
        if acceptance_markers
            .iter()
            .any(|marker| bytes_contain(&serial, marker.as_bytes()))
        {
            return quit_qemu_after_acceptance(child, qmp_stream, serial_log, &serial);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll QEMU: {error}"))?
        {
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            let diagnostics = capture_qmp_timeout_diagnostics(qmp_stream);
            terminate_qemu(child, serial_log, &serial);
            let log_result = append_qmp_timeout_diagnostics(serial_log, &diagnostics);
            let persisted = match log_result {
                Ok(()) => format!("diagnostics appended to {}", serial_log.display()),
                Err(error) => format!("could not append timeout diagnostics: {error}"),
            };
            return Err(format!(
                "QEMU did not reach acceptance within {} seconds; {persisted}",
                timeout.as_secs(),
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

pub fn run_qemu_interactive(config: &InteractiveQemuConfig<'_>) -> Result<i32, String> {
    let QemuConfig {
        qemu: _,
        serial_log,
        acceptance_marker,
        timeout,
        ..
    } = *config;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve serial TCP port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot inspect serial TCP port: {error}"))?
        .port();
    drop(listener);
    let serial_device = format!("tcp:127.0.0.1:{port},server,nowait");
    let mut child = spawn_qemu(config, &serial_device)?;
    let deadline = Instant::now() + timeout;
    let mut stream = None;
    while stream.is_none() {
        if let Ok(stream_value) = TcpStream::connect(("127.0.0.1", port)) {
            stream = Some(stream_value);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll QEMU before serial connect: {error}"))?
        {
            let _ = fs::File::create(serial_log);
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "QEMU did not open its serial TCP endpoint within {} seconds",
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut stream = stream.expect("serial stream assigned");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| format!("cannot configure serial TCP read timeout: {error}"))?;
    let mut serial = Vec::new();
    let mut input_receiver: Option<Receiver<Vec<u8>>> = None;
    let marker = acceptance_marker.as_bytes();
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                serial.extend_from_slice(&buffer[..count]);
                if input_receiver.is_none() && bytes_contain(&serial, b"nsh> ") {
                    let (sender, receiver) = mpsc::channel();
                    thread::spawn(move || {
                        let mut stdin = std::io::stdin();
                        let mut input = [0_u8; 1024];
                        loop {
                            match stdin.read(&mut input) {
                                Ok(0) => break,
                                Ok(count) => {
                                    if sender.send(input[..count].to_vec()).is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    });
                    input_receiver = Some(receiver);
                }
                if bytes_contain(&serial, marker) {
                    child.kill().map_err(|error| {
                        format!("guest reached acceptance but termination failed: {error}")
                    })?;
                    let status = child
                        .wait()
                        .map_err(|error| format!("cannot reap QEMU after acceptance: {error}"))?;
                    fs::write(serial_log, &serial).map_err(|error| {
                        format!(
                            "cannot write interactive serial log {}: {error}",
                            serial_log.display()
                        )
                    })?;
                    return Ok(status.code().unwrap_or(-1));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(format!("cannot read QEMU serial TCP stream: {error}")),
        }
        if let Some(receiver) = &input_receiver {
            while let Ok(input) = receiver.try_recv() {
                stream
                    .write_all(&input)
                    .map_err(|error| format!("cannot send nsh input: {error}"))?;
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll interactive QEMU: {error}"))?
        {
            fs::write(serial_log, &serial).map_err(|error| {
                format!(
                    "cannot write interactive serial log {}: {error}",
                    serial_log.display()
                )
            })?;
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::write(serial_log, &serial);
            return Err(format!(
                "interactive QEMU did not reach acceptance within {} seconds",
                timeout.as_secs()
            ));
        }
    }
    let _ = fs::write(serial_log, &serial);
    Ok(child
        .wait()
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(-1))
}

pub fn run_qemu_gui(config: &QemuConfig<'_>) -> Result<i32, String> {
    run_qemu_gui_with_events(config, M9_GUI_READY_MARKER, &M9_GUI_EVENTS)
}

pub fn run_qemu_gui_with_events(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: false,
            reuse_ovmf_vars: false,
            inter_event_delay: Duration::ZERO,
        },
    )
}

/// Run a GUI acceptance and save the guest's final display through QMP.
/// The screenshot is captured after the acceptance marker and before QEMU exits.
pub fn run_qemu_gui_with_events_and_screenshot(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    screenshot_path: &Path,
) -> Result<QemuGuiOutcome, String> {
    ensure_new_screenshot_path(screenshot_path)?;
    let outcome = run_qemu_gui_with_events_mode_and_serial_input_and_screenshot_timed(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: false,
            reuse_ovmf_vars: false,
            // VirtIO input devices are polled independently, so QMP mouse
            // and keyboard events need time to reach their guest queues in
            // the requested order.
            inter_event_delay: Duration::from_millis(100),
        },
        None,
        Some(screenshot_path),
    )?;
    if outcome.acceptance_reached {
        validate_png_screenshot(screenshot_path)?;
    }
    Ok(outcome)
}

/// Run a read-only boot-disk GUI acceptance and save its accepted display
/// through QMP without replacing an existing screenshot.
pub fn run_qemu_gui_with_read_only_boot_disk_and_events_and_failure_marker_and_screenshot(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: &str,
    screenshot_path: &Path,
) -> Result<QemuGuiOutcome, String> {
    ensure_new_screenshot_path(screenshot_path)?;
    let outcome = run_qemu_gui_with_events_mode_and_serial_input_and_screenshot_timed(
        config,
        ready_marker,
        events,
        Some(failure_marker),
        GuiQemuMode {
            boot_disk_read_only: true,
            reuse_ovmf_vars: false,
            inter_event_delay: Duration::from_millis(100),
        },
        None,
        Some(screenshot_path),
    )?;
    if outcome.acceptance_reached {
        validate_png_screenshot(screenshot_path)?;
    }
    Ok(outcome)
}

fn ensure_new_screenshot_path(screenshot_path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(screenshot_path) {
        Ok(_) => Err(format!(
            "refusing to overwrite existing QEMU screenshot {}",
            screenshot_path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot inspect QEMU screenshot path {}: {error}",
            screenshot_path.display()
        )),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QemuGuiOutcome {
    pub exit_status: i32,
    /// Host wall time from spawning QEMU to receiving the guest READY marker.
    pub ready_after: Option<Duration>,
    pub acceptance_reached: bool,
}

pub fn run_qemu_gui_with_read_only_boot_disk_and_events(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
) -> Result<i32, String> {
    // The guest polls independent VirtIO input queues in device order, which
    // is not guaranteed to match the order of QMP commands. Let it consume a
    // pointer click before the next command sends address-bar keystrokes.
    run_qemu_gui_with_events_mode(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: true,
            reuse_ovmf_vars: false,
            inter_event_delay: Duration::from_millis(100),
        },
    )
}

pub fn run_qemu_gui_with_read_only_boot_disk_and_events_and_serial_input(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    serial_input_marker: &str,
    serial_input: &[u8],
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: true,
            reuse_ovmf_vars: false,
            inter_event_delay: Duration::from_millis(100),
        },
        Some((serial_input_marker, serial_input)),
    )
}

/// Launch writable integrated GPT media with the existing OVMF journal state.
pub fn run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    serial_input_marker: &str,
    serial_input: &[u8],
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: false,
            reuse_ovmf_vars: true,
            inter_event_delay: Duration::from_millis(100),
        },
        Some((serial_input_marker, serial_input)),
    )
}

/// Launch a read-only boot image with the existing OVMF journal state.
pub fn run_qemu_gui_reusing_ovmf_vars_with_read_only_boot_disk_and_events_and_serial_input(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    serial_input_marker: &str,
    serial_input: &[u8],
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input(
        config,
        ready_marker,
        events,
        None,
        GuiQemuMode {
            boot_disk_read_only: true,
            reuse_ovmf_vars: true,
            inter_event_delay: Duration::from_millis(100),
        },
        Some((serial_input_marker, serial_input)),
    )
}

pub fn run_qemu_gui_with_read_only_boot_disk_and_events_and_failure_marker(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: &str,
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode(
        config,
        ready_marker,
        events,
        Some(failure_marker),
        GuiQemuMode {
            boot_disk_read_only: true,
            reuse_ovmf_vars: false,
            inter_event_delay: Duration::from_millis(100),
        },
    )
}

fn run_qemu_gui_with_events_mode(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: Option<&str>,
    mode: GuiQemuMode,
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input(
        config,
        ready_marker,
        events,
        failure_marker,
        mode,
        None,
    )
}

fn run_qemu_gui_with_events_mode_and_serial_input(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: Option<&str>,
    mode: GuiQemuMode,
    serial_input: Option<(&str, &[u8])>,
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input_and_screenshot(
        config,
        ready_marker,
        events,
        failure_marker,
        mode,
        serial_input,
        None,
    )
}

fn run_qemu_gui_with_events_mode_and_serial_input_and_screenshot(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: Option<&str>,
    mode: GuiQemuMode,
    serial_input: Option<(&str, &[u8])>,
    screenshot_path: Option<&Path>,
) -> Result<i32, String> {
    run_qemu_gui_with_events_mode_and_serial_input_and_screenshot_timed(
        config,
        ready_marker,
        events,
        failure_marker,
        mode,
        serial_input,
        screenshot_path,
    )
    .map(|outcome| outcome.exit_status)
}

fn run_qemu_gui_with_events_mode_and_serial_input_and_screenshot_timed(
    config: &QemuConfig<'_>,
    ready_marker: &str,
    events: &[&str],
    failure_marker: Option<&str>,
    mode: GuiQemuMode,
    serial_input: Option<(&str, &[u8])>,
    screenshot_path: Option<&Path>,
) -> Result<QemuGuiOutcome, String> {
    let serial_listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve GUI serial TCP port: {error}"))?;
    let serial_port = serial_listener
        .local_addr()
        .map_err(|error| format!("cannot inspect GUI serial TCP port: {error}"))?
        .port();
    drop(serial_listener);

    let qmp_listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve QMP TCP port: {error}"))?;
    let qmp_port = qmp_listener
        .local_addr()
        .map_err(|error| format!("cannot inspect QMP TCP port: {error}"))?
        .port();
    drop(qmp_listener);

    let vnc_listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve VNC TCP port: {error}"))?;
    let vnc_port = vnc_listener
        .local_addr()
        .map_err(|error| format!("cannot inspect VNC TCP port: {error}"))?
        .port();
    drop(vnc_listener);
    if vnc_port < 5900 {
        return Err(format!(
            "reserved VNC port {vnc_port} cannot be represented by QEMU"
        ));
    }

    let serial_device = format!("tcp:127.0.0.1:{serial_port},server,nowait");
    let mut child = spawn_qemu_with_display_mode_and_vars(
        config,
        &serial_device,
        qmp_port,
        vnc_port - 5900,
        mode.boot_disk_read_only,
        mode.reuse_ovmf_vars,
        true,
    )?;
    let qemu_started_at = Instant::now();
    let deadline = qemu_started_at + config.timeout;
    let mut serial = Vec::new();
    let mut serial_log = match fs::File::create(config.serial_log) {
        Ok(file) => file,
        Err(error) => {
            terminate_qemu(&mut child, config.serial_log, &serial);
            return Err(format!(
                "cannot create GUI serial log {}: {error}",
                config.serial_log.display()
            ));
        }
    };
    let mut serial_stream =
        match connect_guest_tcp(&mut child, serial_port, deadline, "GUI serial TCP endpoint") {
            Ok(stream) => stream,
            Err(error) => {
                terminate_qemu(&mut child, config.serial_log, &serial);
                return Err(error);
            }
        };
    serial_stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| {
            terminate_qemu(&mut child, config.serial_log, &serial);
            format!("cannot configure GUI serial read timeout: {error}")
        })?;

    let mut qmp_stream = match connect_guest_tcp(&mut child, qmp_port, deadline, "QMP endpoint") {
        Ok(stream) => stream,
        Err(error) => {
            terminate_qemu(&mut child, config.serial_log, &serial);
            return Err(error);
        }
    };
    qmp_stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| {
            terminate_qemu(&mut child, config.serial_log, &serial);
            format!("cannot configure QMP read timeout: {error}")
        })?;
    let greeting = match read_qmp_line(&mut qmp_stream, deadline) {
        Ok(greeting) => greeting,
        Err(error) => {
            terminate_qemu(&mut child, config.serial_log, &serial);
            return Err(error);
        }
    };
    if !greeting.contains("\"QMP\"") {
        terminate_qemu(&mut child, config.serial_log, &serial);
        return Err(format!("unexpected QMP greeting: {greeting}"));
    }
    if let Err(error) = qmp_exchange(
        &mut qmp_stream,
        r#"{"execute":"qmp_capabilities"}"#,
        deadline,
    ) {
        terminate_qemu(&mut child, config.serial_log, &serial);
        return Err(error);
    }

    // The M18 HTTPS acceptance passes its failure marker here. Keep a
    // low-rate QMP register trace beside the serial log so a stalled guest
    // remains diagnosable without opening a second QMP client connection.
    let mut qmp_diagnostics = if failure_marker.is_some() {
        let path = config.serial_log.with_extension("qmp-registers.log");
        OpenOptions::new().create(true).append(true).open(path).ok()
    } else {
        None
    };
    let qmp_trace_started = Instant::now();
    let mut last_qmp_trace = qmp_trace_started;

    let marker = config.acceptance_marker.as_bytes();
    let mut buffer = [0_u8; 4096];
    let mut events_sent = false;
    let mut serial_input_sent = false;
    let mut ready_after = None;
    loop {
        match serial_stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                serial.extend_from_slice(&buffer[..count]);
                if let Err(error) = serial_log
                    .write_all(&buffer[..count])
                    .and_then(|()| serial_log.flush())
                {
                    terminate_qemu(&mut child, config.serial_log, &serial);
                    return Err(format!(
                        "cannot append GUI serial log {}: {error}",
                        config.serial_log.display()
                    ));
                }
                let guest_ready = bytes_contain(&serial, ready_marker.as_bytes());
                if guest_ready && ready_after.is_none() {
                    ready_after = Some(qemu_started_at.elapsed());
                }
                if !events_sent && guest_ready {
                    for (index, event) in events.iter().enumerate() {
                        if let Err(error) = qmp_exchange(&mut qmp_stream, event, deadline) {
                            terminate_qemu(&mut child, config.serial_log, &serial);
                            return Err(error);
                        }
                        if index + 1 < events.len() && !mode.inter_event_delay.is_zero() {
                            thread::sleep(mode.inter_event_delay);
                        }
                    }
                    events_sent = true;
                }
                if !serial_input_sent {
                    if let Some((input_marker, input)) = serial_input {
                        if bytes_contain(&serial, input_marker.as_bytes()) {
                            if let Err(error) = serial_stream.write_all(input) {
                                terminate_qemu(&mut child, config.serial_log, &serial);
                                return Err(format!("cannot send Recovery console input: {error}"));
                            }
                            serial_input_sent = true;
                        }
                    }
                }
                if let Some(marker) =
                    failure_marker.filter(|marker| guest_reached_failure(&serial, marker))
                {
                    terminate_qemu(&mut child, config.serial_log, &serial);
                    return Err(format!("GUI QEMU guest printed failure marker `{marker}`"));
                }
                if bytes_contain(&serial, marker) {
                    let Some(ready_after) = ready_after else {
                        terminate_qemu(&mut child, config.serial_log, &serial);
                        return Err(format!(
                            "GUI QEMU guest printed acceptance marker before READY marker `{ready_marker}`"
                        ));
                    };
                    if let Some(screenshot_path) = screenshot_path {
                        let Some(path) = screenshot_path.to_str() else {
                            terminate_qemu(&mut child, config.serial_log, &serial);
                            return Err(format!(
                                "QEMU screenshot path is not valid UTF-8: {}",
                                screenshot_path.display()
                            ));
                        };
                        let filename = qmp_json_quote(&external_path(Path::new(path)));
                        let command = format!(
                            r#"{{"execute":"screendump","arguments":{{"filename":{filename},"format":"png"}}}}"#
                        );
                        if let Err(error) = qmp_exchange(&mut qmp_stream, &command, deadline) {
                            terminate_qemu(&mut child, config.serial_log, &serial);
                            return Err(format!(
                                "cannot capture QEMU screenshot {}: {error}",
                                screenshot_path.display()
                            ));
                        }
                    }
                    let exit_status = quit_qemu_after_acceptance(
                        &mut child,
                        &mut qmp_stream,
                        config.serial_log,
                        &serial,
                    )?;
                    return Ok(QemuGuiOutcome {
                        exit_status,
                        ready_after: Some(ready_after),
                        acceptance_reached: true,
                    });
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => {
                terminate_qemu(&mut child, config.serial_log, &serial);
                return Err(format!("cannot read GUI serial TCP stream: {error}"));
            }
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll GUI QEMU: {error}"))?
        {
            fs::write(config.serial_log, &serial).map_err(|error| {
                format!(
                    "cannot write GUI serial log {}: {error}",
                    config.serial_log.display()
                )
            })?;
            return Ok(QemuGuiOutcome {
                exit_status: status.code().unwrap_or(-1),
                ready_after,
                acceptance_reached: false,
            });
        }
        if Instant::now() >= deadline {
            if let Some(diagnostics) = qmp_diagnostics.as_mut() {
                let _ = diagnostics.flush();
            }
            drop(qmp_diagnostics);
            let timeout_diagnostics = capture_qmp_timeout_diagnostics(&mut qmp_stream);
            let _ = serial_log.flush();
            let _ = child.kill();
            let _ = child.wait();
            drop(serial_log);
            let serial_write = fs::write(config.serial_log, &serial).map_err(|error| {
                format!(
                    "cannot write timed-out GUI serial log {}: {error}",
                    config.serial_log.display()
                )
            });
            let diagnostics_write =
                append_qmp_timeout_diagnostics(config.serial_log, &timeout_diagnostics);
            let persisted = match (serial_write, diagnostics_write) {
                (Ok(()), Ok(())) => {
                    format!(
                        "QMP diagnostics appended to {}",
                        config.serial_log.display()
                    )
                }
                (Err(serial_error), Ok(())) => serial_error,
                (Ok(()), Err(diagnostics_error)) => diagnostics_error,
                (Err(serial_error), Err(diagnostics_error)) => {
                    format!("{serial_error}; {diagnostics_error}")
                }
            };
            return Err(format!(
                "GUI QEMU did not reach acceptance within {} seconds; {persisted}",
                config.timeout.as_secs(),
            ));
        }
        if last_qmp_trace.elapsed() >= Duration::from_secs(20) {
            last_qmp_trace = Instant::now();
            if let Some(diagnostics) = qmp_diagnostics.as_mut() {
                let trace_deadline = Instant::now() + Duration::from_secs(5);
                let response = qmp_exchange_response(
                    &mut qmp_stream,
                    r#"{"execute":"human-monitor-command","arguments":{"command-line":"info registers"}}"#,
                    trace_deadline,
                );
                let elapsed = qmp_trace_started.elapsed().as_secs();
                let _ = match response {
                    Ok(response) => writeln!(
                        diagnostics,
                        "M18 QMP registers after {elapsed}s: {}",
                        response.trim_end()
                    ),
                    Err(error) => writeln!(
                        diagnostics,
                        "M18 QMP register query after {elapsed}s failed: {error}"
                    ),
                };
                let _ = diagnostics.flush();
            }
        }
    }
    let _ = fs::write(config.serial_log, &serial);
    Ok(QemuGuiOutcome {
        exit_status: child
            .wait()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(-1),
        ready_after,
        acceptance_reached: false,
    })
}

fn spawn_qemu(config: &QemuConfig<'_>, serial_device: &str) -> Result<Child, String> {
    spawn_qemu_with_display(config, serial_device, 0, 0)
}

fn spawn_qemu_with_display(
    config: &QemuConfig<'_>,
    serial_device: &str,
    qmp_port: u16,
    vnc_display: u16,
) -> Result<Child, String> {
    spawn_qemu_with_display_mode(config, serial_device, qmp_port, vnc_display, false)
}

fn spawn_qemu_with_display_mode(
    config: &QemuConfig<'_>,
    serial_device: &str,
    qmp_port: u16,
    vnc_display: u16,
    boot_disk_read_only: bool,
) -> Result<Child, String> {
    spawn_qemu_with_display_mode_and_vars(
        config,
        serial_device,
        qmp_port,
        vnc_display,
        boot_disk_read_only,
        false,
        qmp_port != 0,
    )
}

fn spawn_qemu_with_display_mode_and_vars(
    config: &QemuConfig<'_>,
    serial_device: &str,
    qmp_port: u16,
    vnc_display: u16,
    boot_disk_read_only: bool,
    reuse_ovmf_vars: bool,
    show_vnc_display: bool,
) -> Result<Child, String> {
    let QemuConfig {
        qemu,
        ovmf_code,
        ovmf_vars_template,
        disk_image,
        persistent_disk,
        vars_copy,
        ..
    } = *config;
    prepare_ovmf_vars(ovmf_vars_template, vars_copy, reuse_ovmf_vars)?;
    if let Some(parent) = config.serial_log.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create serial log directory: {error}"))?;
    }
    fs::File::create(config.serial_log).map_err(|error| {
        format!(
            "cannot create serial log {}: {error}",
            config.serial_log.display()
        )
    })?;
    let code_drive = format!(
        "if=pflash,format=raw,readonly=on,file={}",
        external_path(ovmf_code)
    );
    let vars_drive = format!("if=pflash,format=raw,file={}", external_path(vars_copy));
    let disk_drive = image_drive_argument(disk_image, boot_disk_read_only);
    let single_disk = disk_image == persistent_disk;
    let persistent_drive = format!(
        "if=none,id=nagi-data,format=raw,file={}",
        external_path(persistent_disk)
    );
    let audio_device = format!(
        "driver={},id=nagi-audio",
        qemu_audio_driver_for_host(std::env::consts::OS)
    );
    let mut command = ProcessCommand::new(qemu);
    command.args([
        "-rtc",
        "base=utc",
        "-machine",
        "q35",
        "-cpu",
        "qemu64",
        "-smp",
        "4",
        "-m",
        "8G",
        "-nodefaults",
        "-device",
        "virtio-vga",
        "-device",
        "virtio-keyboard-pci,id=nagi-keyboard",
        "-device",
        "virtio-mouse-pci,id=nagi-mouse",
        "-device",
        "virtio-rng-pci,disable-modern=on",
        "-audiodev",
        &audio_device,
        "-device",
        "virtio-sound-pci,audiodev=nagi-audio,disable-modern=on",
        "-netdev",
        "user,id=nagi-net",
        "-device",
        "virtio-net-pci,netdev=nagi-net,disable-modern=on,mac=02:00:00:00:00:15",
        "-drive",
        &code_drive,
        "-drive",
        &vars_drive,
        "-drive",
        &disk_drive,
    ]);
    if !single_disk {
        command.arg("-drive").arg(&persistent_drive).args([
            "-device",
            "virtio-blk-pci,drive=nagi-data,disable-modern=on",
        ]);
    }
    command.args(["-serial", serial_device]);
    if qmp_port == 0 {
        command.args(["-display", "none", "-monitor", "none"]);
    } else if show_vnc_display {
        let qmp = format!("tcp:127.0.0.1:{qmp_port},server=on,wait=off");
        let display = format!("vnc=127.0.0.1:{vnc_display}");
        command.arg("-qmp").arg(qmp).arg("-display").arg(display);
    } else {
        let qmp = format!("tcp:127.0.0.1:{qmp_port},server=on,wait=off");
        command
            .arg("-qmp")
            .arg(qmp)
            .args(["-display", "none", "-monitor", "none"]);
    }
    command
        .args([
            "-no-reboot",
            "-no-shutdown",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
        ])
        .spawn()
        .map_err(|error| format!("cannot start QEMU {}: {error}", qemu.display()))
}

fn prepare_ovmf_vars(
    template: &Path,
    vars_copy: &Path,
    reuse_existing: bool,
) -> Result<(), String> {
    if reuse_existing {
        return match fs::metadata(vars_copy) {
            Ok(metadata) if metadata.is_file() => Ok(()),
            Ok(_) => Err(format!(
                "OVMF variables path is not a file: {}",
                vars_copy.display()
            )),
            Err(error) => Err(format!(
                "cannot reuse missing OVMF variables image {}: {error}",
                vars_copy.display()
            )),
        };
    }

    fs::copy(template, vars_copy).map(|_| ()).map_err(|error| {
        format!(
            "cannot copy OVMF variables template {} to {}: {error}",
            template.display(),
            vars_copy.display()
        )
    })
}

fn qemu_audio_driver_for_host(host_os: &str) -> &'static str {
    match host_os {
        "windows" => "dsound",
        "macos" => "coreaudio",
        // CI runners and many developer hosts are headless. The dummy backend
        // keeps VirtIO Sound available to the guest without requiring a host
        // audio server.
        _ => "none",
    }
}

fn image_drive_argument(path: &Path, read_only: bool) -> String {
    let format = if path
        .extension()
        .is_some_and(|extension| extension == "qcow2")
    {
        "qcow2"
    } else {
        "raw"
    };
    let mode = if read_only { ",readonly=on" } else { "" };
    format!(
        "if=virtio,format={format}{mode},file={}",
        external_path(path)
    )
}

fn terminate_qemu(child: &mut Child, serial_log: &Path, serial: &[u8]) {
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::write(serial_log, serial);
}

fn quit_qemu_after_acceptance(
    child: &mut Child,
    qmp_stream: &mut TcpStream,
    serial_log: &Path,
    serial: &[u8],
) -> Result<i32, String> {
    if let Err(error) = qmp_exchange(
        qmp_stream,
        r#"{"execute":"quit"}"#,
        Instant::now() + Duration::from_secs(5),
    ) {
        terminate_qemu(child, serial_log, serial);
        return Err(format!(
            "cannot request QEMU to quit after acceptance: {error}"
        ));
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                fs::write(serial_log, serial).map_err(|error| {
                    format!("cannot write serial log {}: {error}", serial_log.display())
                })?;
                return Ok(status.code().unwrap_or(-1));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                terminate_qemu(child, serial_log, serial);
                return Err("QEMU did not exit after the QMP quit request".to_owned());
            }
            Err(error) => {
                terminate_qemu(child, serial_log, serial);
                return Err(format!(
                    "cannot poll QEMU after the QMP quit request: {error}"
                ));
            }
        }
    }
}

fn reserve_local_tcp_listener(description: &str) -> Result<(TcpListener, u16), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("cannot reserve {description} TCP port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot inspect {description} TCP port: {error}"))?
        .port();
    Ok((listener, port))
}

fn connect_guest_tcp(
    child: &mut Child,
    port: u16,
    deadline: Instant,
    description: &str,
) -> Result<TcpStream, String> {
    loop {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            return Ok(stream);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll QEMU before {description} connect: {error}"))?
        {
            return Err(format!(
                "QEMU exited with status {} before opening {description}",
                status.code().unwrap_or(-1)
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "QEMU did not open {description} within the timeout"
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_qmp_line(stream: &mut TcpStream, deadline: Instant) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err("QMP closed before sending a response".into()),
            Ok(1) => {
                if bytes.len() >= QMP_MAX_LINE_BYTES {
                    return Err(format!(
                        "QMP response exceeds the {QMP_MAX_LINE_BYTES}-byte line limit"
                    ));
                }
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(String::from_utf8_lossy(&bytes).into_owned());
                }
            }
            Ok(_) => unreachable!(),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                if Instant::now() >= deadline {
                    return Err("QMP response timed out".into());
                }
            }
            Err(error) => return Err(format!("cannot read QMP response: {error}")),
        }
    }
}

fn capture_qmp_timeout_diagnostics(stream: &mut TcpStream) -> Vec<String> {
    let queries = [
        ("QMP query-status", r#"{"execute":"query-status"}"#),
        (
            "QMP CPU registers",
            r#"{"execute":"human-monitor-command","arguments":{"command-line":"info registers"}}"#,
        ),
        (
            "QMP CPU instruction window",
            r#"{"execute":"human-monitor-command","arguments":{"command-line":"x/12i $rip"}}"#,
        ),
    ];
    let deadline = Instant::now() + QMP_TIMEOUT_DIAGNOSTIC_TIMEOUT;
    let mut diagnostics = Vec::with_capacity(queries.len());
    for (label, command) in queries {
        if Instant::now() >= deadline {
            diagnostics.push(format!("{label} skipped: diagnostic time budget exhausted"));
            break;
        }
        match qmp_exchange_response(stream, command, deadline) {
            Ok(response) => diagnostics.push(format!("{label}: {}", response.trim_end())),
            Err(error) => diagnostics.push(format!("{label} failed: {error}")),
        }
    }
    diagnostics
}

fn append_qmp_timeout_diagnostics(path: &Path, diagnostics: &[String]) -> Result<(), String> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open {} for diagnostics: {error}", path.display()))?;
    writeln!(log, "\nNagi QEMU timeout diagnostics:")
        .and_then(|()| {
            for diagnostic in diagnostics {
                writeln!(log, "{diagnostic}")?;
            }
            log.flush()
        })
        .map_err(|error| format!("cannot append to {}: {error}", path.display()))
}

fn qmp_exchange(stream: &mut TcpStream, command: &str, deadline: Instant) -> Result<(), String> {
    qmp_exchange_response(stream, command, deadline).map(|_| ())
}

fn qmp_exchange_response(
    stream: &mut TcpStream,
    command: &str,
    deadline: Instant,
) -> Result<String, String> {
    stream
        .write_all(command.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .map_err(|error| format!("cannot send QMP command: {error}"))?;
    loop {
        let response = read_qmp_line(stream, deadline)?;
        if response.contains("\"event\"") {
            continue;
        }
        if response.contains("\"error\"") {
            return Err(format!("QMP command failed: {response}"));
        }
        if !response.contains("\"return\"") {
            return Err(format!("unexpected QMP response: {response}"));
        }
        return Ok(response);
    }
}

fn qmp_json_quote(value: &str) -> String {
    use std::fmt::Write as _;

    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control <= '\u{1f}' => {
                let _ = write!(escaped, "\\u{:04x}", control as u32);
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

fn validate_png_screenshot(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("cannot inspect QEMU screenshot {}: {error}", path.display()))?;
    if metadata.len() < 24 {
        return Err(format!(
            "QEMU screenshot is too short to be a PNG: {}",
            path.display()
        ));
    }
    let mut file = fs::File::open(path)
        .map_err(|error| format!("cannot open QEMU screenshot {}: {error}", path.display()))?;
    let mut header = [0_u8; 24];
    file.read_exact(&mut header).map_err(|error| {
        format!(
            "cannot read QEMU screenshot header {}: {error}",
            path.display()
        )
    })?;
    if header[..8] != [137, 80, 78, 71, 13, 10, 26, 10] || &header[12..16] != b"IHDR" {
        return Err(format!(
            "QEMU screenshot has an invalid PNG header: {}",
            path.display()
        ));
    }
    let width = u32::from_be_bytes(header[16..20].try_into().expect("four-byte width"));
    let height = u32::from_be_bytes(header[20..24].try_into().expect("four-byte height"));
    if width == 0 || height == 0 {
        return Err(format!(
            "QEMU screenshot has invalid dimensions {width}x{height}: {}",
            path.display()
        ));
    }
    Ok(())
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn external_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    if let Some(path) = path.strip_prefix(r"\\?\") {
        return path.to_owned();
    }
    path.into_owned()
}

#[cfg(test)]
fn guest_reached_acceptance(serial: &str, acceptance_marker: &str) -> bool {
    serial.contains(acceptance_marker)
}

#[cfg(test)]
fn guest_reached_any_acceptance(serial: &str, acceptance_markers: &[&str]) -> bool {
    acceptance_markers
        .iter()
        .any(|marker| guest_reached_acceptance(serial, marker))
}

fn guest_reached_failure(serial: &[u8], failure_marker: &str) -> bool {
    bytes_contain(serial, failure_marker.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        append_qmp_timeout_diagnostics, build_fat12_ab_image, build_fat12_image,
        build_m17_fat12_image, capture_qmp_timeout_diagnostics, cluster_offset,
        ensure_new_screenshot_path, ensure_persistent_disk, guest_reached_acceptance,
        guest_reached_any_acceptance, guest_reached_failure, image_drive_argument, initialize_fats,
        json_string_field, json_u64_field, m20_model_store_fixture_filename, prepare_ovmf_vars,
        qemu_audio_driver_for_host, qmp_json_quote, read_qmp_line, reference_partitions,
        write_chain, AbSlotImages, Fat12Geometry, SlotPayload, DATA_OFFSET, FAT_COUNT,
        GUEST_ACCEPTANCE_MARKER, IMAGE_SIZE, LEGACY_PERSISTENT_DISK_SIZE, M17_IMAGE_SIZE,
        M17_SECTORS_PER_CLUSTER, PERSISTENT_DISK_SIZE, QMP_MAX_LINE_BYTES, REFERENCE_DISK_SECTORS,
        ROOT_ENTRY_COUNT, ROOT_OFFSET, SECTOR_SIZE, USER_DATA_START_LBA,
    };

    #[test]
    fn m20_fixture_filename_uses_the_model_manager_artifact_id_contract() {
        let artifact_id =
            nagi_model_manager::ArtifactId::new(super::super::m20_model_store_fixture::ARTIFACT_ID)
                .expect("valid M20 fixture artifact ID");
        let short_name = nagi_model_manager::model_store_short_name(&artifact_id);
        let expected = format!(
            "{}.{}",
            std::str::from_utf8(&short_name[..8]).expect("ASCII basename"),
            std::str::from_utf8(&short_name[8..]).expect("ASCII extension")
        );

        assert_eq!(m20_model_store_fixture_filename().unwrap(), expected);
        assert_eq!(
            super::super::m20_model_store_fixture::FIXTURE_BYTES.len(),
            5_000
        );
        assert_eq!(
            &super::super::m20_model_store_fixture::FIXTURE_BYTES[..4],
            b"GGUF"
        );
    }

    #[test]
    fn qmp_json_quote_escapes_control_and_path_characters() {
        assert_eq!(
            qmp_json_quote("line\n\"C:\\tmp\""),
            "\"line\\n\\\"C:\\\\tmp\\\"\""
        );
        assert_eq!(qmp_json_quote("日本語"), "\"日本語\"");
    }

    #[test]
    fn qemu_screenshot_path_refuses_to_replace_an_existing_file() {
        let path = unique_persistent_disk_path("qemu-screenshot");
        std::fs::write(&path, b"preserved screenshot evidence").expect("write existing file");

        let error = ensure_new_screenshot_path(&path).expect_err("existing file is protected");
        assert!(error.contains("refusing to overwrite existing QEMU screenshot"));
        assert_eq!(
            std::fs::read(&path).expect("read preserved file"),
            b"preserved screenshot evidence"
        );

        std::fs::remove_file(path).expect("remove screenshot fixture");
    }

    #[test]
    fn qmp_timeout_diagnostics_capture_vm_status_registers_and_instruction_window() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind QMP fixture");
        let address = listener.local_addr().expect("QMP fixture address");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept QMP fixture");
            let mut reader = BufReader::new(stream);

            let mut command = String::new();
            reader.read_line(&mut command).expect("read query-status");
            assert_eq!(command.trim(), r#"{"execute":"query-status"}"#);
            reader
                .get_mut()
                .write_all(br#"{"return":{"status":"running","running":true}}"#)
                .expect("write status response");
            reader
                .get_mut()
                .write_all(b"\r\n")
                .expect("terminate status response");

            command.clear();
            reader.read_line(&mut command).expect("read info registers");
            assert!(command.contains("info registers"));
            reader
                .get_mut()
                .write_all(br#"{"return":"CPU#0: RIP=0x1234\nRAX=0x5678"}"#)
                .expect("write register response");
            reader
                .get_mut()
                .write_all(b"\r\n")
                .expect("terminate register response");

            command.clear();
            reader
                .read_line(&mut command)
                .expect("read instruction window");
            assert!(command.contains(r#"x/12i $rip"#));
            reader
                .get_mut()
                .write_all(br#"{"return":"=> 0x1234:  mov %rax,%rbx\n   0x1237:  jmp 0x1234"}"#)
                .expect("write instruction response");
            reader
                .get_mut()
                .write_all(b"\r\n")
                .expect("terminate instruction response");
        });

        let mut qmp = TcpStream::connect(address).expect("connect QMP fixture");
        qmp.set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .expect("set QMP read timeout");
        let diagnostics = capture_qmp_timeout_diagnostics(&mut qmp);
        server.join().expect("QMP fixture thread");

        assert_eq!(diagnostics.len(), 3);
        assert!(diagnostics[0].contains(r#""status":"running""#));
        assert!(diagnostics[1].contains("RIP=0x1234"));
        assert!(diagnostics[2].contains("mov %rax,%rbx"));
    }

    #[test]
    fn qmp_timeout_diagnostics_append_to_the_serial_log() {
        let path = unique_persistent_disk_path("qmp-timeout");
        std::fs::write(&path, b"guest serial output\n").expect("write serial fixture");
        append_qmp_timeout_diagnostics(&path, &["QMP query-status: running".to_owned()])
            .expect("append QMP diagnostics");
        let log = std::fs::read_to_string(&path).expect("read serial fixture");
        assert!(log.starts_with("guest serial output\n"));
        assert!(log.contains("Nagi QEMU timeout diagnostics:"));
        assert!(log.contains("QMP query-status: running"));
        std::fs::remove_file(path).expect("remove serial fixture");
    }

    #[test]
    fn qmp_response_lines_are_bounded() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind QMP fixture");
        let address = listener.local_addr().expect("QMP fixture address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept QMP fixture");
            let line = vec![b'x'; QMP_MAX_LINE_BYTES + 1];
            stream.write_all(&line).expect("write oversized QMP line");
        });

        let mut qmp = TcpStream::connect(address).expect("connect QMP fixture");
        qmp.set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .expect("set QMP read timeout");
        let error = read_qmp_line(
            &mut qmp,
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        )
        .expect_err("reject oversized QMP line");
        server.join().expect("QMP fixture thread");
        assert!(error.contains("line limit"));
    }

    #[test]
    fn qemu_audio_backend_is_supported_by_the_host_platform() {
        assert_eq!(qemu_audio_driver_for_host("windows"), "dsound");
        assert_eq!(qemu_audio_driver_for_host("macos"), "coreaudio");
        assert_eq!(qemu_audio_driver_for_host("linux"), "none");
        assert_eq!(qemu_audio_driver_for_host("unknown"), "none");
    }

    #[test]
    fn qemu_can_mount_the_m17_boot_image_read_only() {
        assert_eq!(
            image_drive_argument(Path::new("m17.img"), true),
            "if=virtio,format=raw,readonly=on,file=m17.img"
        );
        assert_eq!(
            image_drive_argument(Path::new("m7.img"), false),
            "if=virtio,format=raw,file=m7.img"
        );
        assert_eq!(
            image_drive_argument(Path::new("release.qcow2"), false),
            "if=virtio,format=qcow2,file=release.qcow2"
        );
    }

    #[test]
    fn reference_release_layout_has_the_specified_order_sizes_and_guids() {
        let partitions = reference_partitions().expect("reference partition layout");
        assert_eq!(partitions.len(), 6);
        let sizes = [
            512 * 2048,
            4 * 1024 * 2048,
            4 * 1024 * 2048,
            16 * 1024 * 2048,
            4 * 1024 * 2048,
            32 * 1024 * 2048,
        ];
        for (index, (partition, expected_size)) in partitions.iter().zip(sizes).enumerate() {
            assert_eq!(partition.last_lba - partition.first_lba + 1, expected_size);
            if index > 0 {
                assert_eq!(partition.first_lba % 2048, 0);
                assert!(partition.first_lba > partitions[index - 1].last_lba);
            }
        }
        assert_eq!(partitions[0].unique_guid, crate::gpt::ESP_PARTITION_GUID);
        assert_eq!(partitions[1].type_guid, crate::gpt::SYSTEM_A_TYPE_GUID);
        assert_eq!(
            partitions[1].unique_guid,
            crate::gpt::SYSTEM_A_PARTITION_GUID
        );
        assert_eq!(partitions[2].type_guid, crate::gpt::SYSTEM_B_TYPE_GUID);
        assert_eq!(
            partitions[2].unique_guid,
            crate::gpt::SYSTEM_B_PARTITION_GUID
        );
        assert_eq!(
            partitions[3].type_guid,
            crate::gpt::NAGI_USER_DATA_TYPE_GUID
        );
        assert_eq!(
            partitions[3].unique_guid,
            crate::gpt::USER_DATA_PARTITION_GUID
        );
        assert_eq!(partitions[4].type_guid, crate::gpt::RECOVERY_TYPE_GUID);
        assert_eq!(
            partitions[4].unique_guid,
            crate::gpt::RECOVERY_PARTITION_GUID
        );
        assert_eq!(partitions[5].type_guid, crate::gpt::MODEL_STORE_TYPE_GUID);
        assert_eq!(
            partitions[5].unique_guid,
            crate::gpt::MODEL_STORE_PARTITION_GUID
        );
        assert!(partitions[5].last_lba < REFERENCE_DISK_SECTORS - 34);
    }

    #[test]
    fn release_image_info_reads_outer_format_and_virtual_size() {
        let info = r#"{
            "children": [{"info": {"format": "file", "virtual-size": 3801088}}],
            "virtual-size": 68719476736,
            "format": "qcow2"
        }"#;
        assert_eq!(json_string_field(info, "format"), Some("qcow2".to_owned()));
        assert_eq!(
            json_u64_field(info, "virtual-size"),
            Some(64 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn qemu_acceptance_requires_the_m7_guest_marker() {
        assert!(!guest_reached_acceptance(
            "Nagi M6 acceptance PASS\r\n",
            GUEST_ACCEPTANCE_MARKER
        ));
        assert!(guest_reached_acceptance(
            "Nagi M7 acceptance PASS\r\n",
            GUEST_ACCEPTANCE_MARKER
        ));
    }

    #[test]
    fn qemu_acceptance_can_stop_on_either_first_boot_state() {
        let markers = ["Nagi M7 reboot required PASS", "Nagi M7 acceptance PASS"];
        assert!(guest_reached_any_acceptance(
            "Nagi M7 reboot required PASS\r\n",
            &markers
        ));
        assert!(guest_reached_any_acceptance(
            "Nagi M7 acceptance PASS\r\n",
            &markers
        ));
        assert!(!guest_reached_any_acceptance(
            "Nagi M7 persistent write PASS\r\n",
            &markers
        ));
    }

    #[test]
    fn gui_qemu_failure_marker_stops_the_acceptance_wait() {
        assert!(guest_reached_failure(
            b"Nagi M18 browser FAIL HTTPS timeout\r\n",
            "Nagi M18 browser FAIL"
        ));
        assert!(!guest_reached_failure(
            b"Nagi M18 browser READY\r\n",
            "Nagi M18 browser FAIL"
        ));
    }

    #[test]
    fn persistent_disk_is_created_once_and_existing_data_is_preserved() {
        use std::io::{Read, Seek, SeekFrom, Write};

        let path = unique_persistent_disk_path("fresh");

        std::fs::write(&path, b"short").expect("write wrong-sized disk");
        assert!(ensure_persistent_disk(&path).is_err());
        assert_eq!(std::fs::metadata(&path).expect("short metadata").len(), 5);
        std::fs::remove_file(&path).expect("remove wrong-sized disk");

        assert!(!ensure_persistent_disk(&path).expect("create disk"));
        assert_eq!(
            std::fs::metadata(&path).expect("disk metadata").len(),
            PERSISTENT_DISK_SIZE
        );
        let mut disk = std::fs::File::open(&path).expect("open GPT disk");
        let partition = crate::gpt::read_user_data_partition(&mut disk, PERSISTENT_DISK_SIZE / 512)
            .expect("valid GPT data partition");
        assert_eq!(partition.start_lba, USER_DATA_START_LBA);
        assert!(partition.sector_count >= 16_384);
        let offset = USER_DATA_START_LBA * 512 + 8 * 1024 * 1024 - 1;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open disk");
        file.seek(SeekFrom::Start(offset)).expect("seek marker");
        file.write_all(&[0xa5]).expect("write marker");
        drop(file);

        assert!(ensure_persistent_disk(&path).expect("preserve disk"));
        let mut file = std::fs::File::open(&path).expect("reopen disk");
        let mut marker = [0; 1];
        file.seek(SeekFrom::Start(offset)).expect("seek marker");
        file.read_exact(&mut marker).expect("read marker");
        assert_eq!(marker, [0xa5]);
        let mut disk = std::fs::File::open(&path).expect("reopen GPT disk");
        assert_eq!(
            crate::gpt::read_user_data_partition(&mut disk, PERSISTENT_DISK_SIZE / 512),
            Ok(partition)
        );
        remove_persistent_disk_files(&path);
    }

    #[test]
    fn legacy_raw_disk_migration_preserves_the_full_disk_and_keeps_a_recovery_copy() {
        use std::io::{Read, Seek, SeekFrom, Write};

        let path = unique_persistent_disk_path("legacy");
        let mut legacy = std::fs::File::create(&path).expect("create legacy disk");
        legacy
            .set_len(LEGACY_PERSISTENT_DISK_SIZE)
            .expect("size legacy disk");
        legacy
            .write_all(&[0x4a, 0x47, 0x50, 0x54])
            .expect("write first marker");
        legacy
            .seek(SeekFrom::Start(LEGACY_PERSISTENT_DISK_SIZE - 1))
            .expect("seek last marker");
        legacy.write_all(&[0xa5]).expect("write last marker");
        drop(legacy);

        assert!(ensure_persistent_disk(&path).expect("migrate legacy disk"));
        assert_eq!(
            std::fs::metadata(&path).expect("migrated size").len(),
            PERSISTENT_DISK_SIZE
        );
        let backup = super::path_with_suffix(&path, ".legacy-raw");
        assert_eq!(
            std::fs::metadata(&backup)
                .expect("legacy recovery copy")
                .len(),
            LEGACY_PERSISTENT_DISK_SIZE
        );

        let mut migrated = std::fs::File::open(&path).expect("open migrated disk");
        let partition =
            crate::gpt::read_user_data_partition(&mut migrated, PERSISTENT_DISK_SIZE / 512)
                .expect("migrated GPT is valid");
        let data_offset = partition.start_lba * 512;
        migrated
            .seek(SeekFrom::Start(data_offset))
            .expect("seek first copied marker");
        let mut first_marker = [0u8; 4];
        migrated
            .read_exact(&mut first_marker)
            .expect("read first copied marker");
        assert_eq!(first_marker, [0x4a, 0x47, 0x50, 0x54]);
        migrated
            .seek(SeekFrom::Start(
                data_offset + LEGACY_PERSISTENT_DISK_SIZE - 1,
            ))
            .expect("seek last copied marker");
        let mut last_marker = [0u8; 1];
        migrated
            .read_exact(&mut last_marker)
            .expect("read last copied marker");
        assert_eq!(last_marker, [0xa5]);
        assert!(ensure_persistent_disk(&path).expect("keep migrated disk"));
        remove_persistent_disk_files(&path);
    }

    #[test]
    fn blank_legacy_disk_migration_requests_initial_vfs_format() {
        let path = unique_persistent_disk_path("blank-legacy");
        let legacy = std::fs::File::create(&path).expect("create blank legacy disk");
        legacy
            .set_len(LEGACY_PERSISTENT_DISK_SIZE)
            .expect("size blank legacy disk");
        drop(legacy);

        assert!(!ensure_persistent_disk(&path).expect("migrate blank legacy disk"));
        assert!(ensure_persistent_disk(&path).expect("recognize migrated GPT disk"));
        remove_persistent_disk_files(&path);
    }

    fn unique_persistent_disk_path(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nagi-persistent-disk-{}-{label}-{nonce}",
            std::process::id()
        ))
    }

    fn remove_persistent_disk_files(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        for suffix in [".legacy-raw", ".gpt-migration"] {
            let _ = std::fs::remove_file(super::path_with_suffix(path, suffix));
        }
    }

    #[test]
    fn qemu_can_reuse_one_initialized_ovmf_variables_image() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("nagi-ovmf-vars-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create temp directory");
        let template = root.join("template.fd");
        let vars = root.join("vars.fd");
        std::fs::write(&template, b"template vars").expect("write template");

        prepare_ovmf_vars(&template, &vars, false).expect("initialize vars");
        assert_eq!(
            std::fs::read(&vars).expect("read initialized vars"),
            b"template vars"
        );
        std::fs::write(&vars, b"firmware state").expect("write simulated firmware state");
        prepare_ovmf_vars(&template, &vars, true).expect("reuse vars");
        assert_eq!(
            std::fs::read(&vars).expect("read reused vars"),
            b"firmware state"
        );

        std::fs::remove_file(&vars).expect("remove vars");
        assert!(prepare_ovmf_vars(&template, &vars, true).is_err());
        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn writes_deterministic_fat12_image_with_expected_paths() {
        let bootloader = b"bootloader";
        let kernel = b"kernel payload";
        let init = b"init payload";
        let first = build_fat12_image(bootloader, kernel, init).expect("image");
        let second = build_fat12_image(bootloader, kernel, init).expect("image");
        assert_eq!(first, second);
        assert_eq!(first.len(), IMAGE_SIZE);
        assert_eq!(&first[510..512], &[0x55, 0xaa]);
        assert_eq!(&first[ROOT_OFFSET..ROOT_OFFSET + 3], b"EFI");
        assert_eq!(&first[DATA_OFFSET + 64..DATA_OFFSET + 68], b"BOOT");
        assert_eq!(&first[DATA_OFFSET + 96..DATA_OFFSET + 100], b"NAGI");
        assert_eq!(
            &first[DATA_OFFSET + 1024 + 64..DATA_OFFSET + 1024 + 75],
            b"KERNEL  ELF"
        );
        assert_eq!(
            &first[DATA_OFFSET + 1024 + 96..DATA_OFFSET + 1024 + 107],
            b"INIT    ELF"
        );
        assert_eq!(
            u16::from_le_bytes([
                first[DATA_OFFSET + 1024 + 64 + 26],
                first[DATA_OFFSET + 1024 + 64 + 27],
            ]),
            6
        );
        assert_eq!(
            u16::from_le_bytes([
                first[DATA_OFFSET + 1024 + 96 + 26],
                first[DATA_OFFSET + 1024 + 96 + 27],
            ]),
            7
        );
        assert_eq!(
            &first[DATA_OFFSET + 1536..DATA_OFFSET + 1536 + bootloader.len()],
            bootloader
        );
        assert_eq!(
            &first[DATA_OFFSET + 2048..DATA_OFFSET + 2048 + kernel.len()],
            kernel
        );
        assert_eq!(
            &first[DATA_OFFSET + 2560..DATA_OFFSET + 2560 + init.len()],
            init
        );
        assert_eq!(&first[DATA_OFFSET..DATA_OFFSET + 3], b".  ");
    }

    #[test]
    fn m27_ab_image_keeps_each_kernel_and_init_under_its_slot_directory() {
        let geometry =
            Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)
                .expect("M27 FAT12 geometry");
        let (image, layout) = build_fat12_ab_image(
            b"loader",
            AbSlotImages {
                system_a: SlotPayload {
                    kernel: b"kernel A",
                    init: b"init A",
                },
                system_b: SlotPayload {
                    kernel: b"broken B kernel",
                    init: b"init B",
                },
                recovery: Some(SlotPayload {
                    kernel: b"kernel A",
                    init: b"recovery init",
                }),
            },
            geometry,
        )
        .expect("build A/B image");

        assert_eq!(image.len(), M17_IMAGE_SIZE);
        let nagi_directory = cluster_offset(4, geometry);
        assert_eq!(
            &image[nagi_directory + 64..nagi_directory + 75],
            &super::short_name("SYSTEMA", "")
        );
        assert_eq!(
            &image[nagi_directory + 96..nagi_directory + 107],
            &super::short_name("SYSTEMB", "")
        );
        assert_eq!(
            &image[nagi_directory + 128..nagi_directory + 139],
            &super::short_name("RECOVERY", "")
        );
        let a_directory = cluster_offset(5, geometry);
        let b_directory = cluster_offset(6, geometry);
        for (directory, kernel_cluster, init_cluster) in
            [(a_directory, 9, 10), (b_directory, 11, 12)]
        {
            assert_eq!(
                &image[directory + 64..directory + 75],
                &super::short_name("KERNEL", "ELF")
            );
            assert_eq!(
                u16::from_le_bytes([image[directory + 64 + 26], image[directory + 64 + 27]]),
                kernel_cluster
            );
            assert_eq!(
                &image[directory + 96..directory + 107],
                &super::short_name("INIT", "ELF")
            );
            assert_eq!(
                u16::from_le_bytes([image[directory + 96 + 26], image[directory + 96 + 27]]),
                init_cluster
            );
        }
        let recovery_directory = cluster_offset(7, geometry);
        assert_eq!(
            &image[recovery_directory + 64..recovery_directory + 75],
            &super::short_name("KERNEL", "ELF")
        );
        assert_eq!(
            u16::from_le_bytes([
                image[recovery_directory + 64 + 26],
                image[recovery_directory + 64 + 27],
            ]),
            13
        );
        assert_eq!(
            &image[recovery_directory + 96..recovery_directory + 107],
            &super::short_name("INIT", "ELF")
        );
        assert_eq!(
            u16::from_le_bytes([
                image[recovery_directory + 96 + 26],
                image[recovery_directory + 96 + 27],
            ]),
            14
        );
        let a_kernel_offset = cluster_offset(layout.kernel_start_cluster, geometry);
        assert_eq!(&image[a_kernel_offset..a_kernel_offset + 8], b"kernel A");
        let b_kernel_offset = cluster_offset(11, geometry);
        assert_eq!(
            &image[b_kernel_offset..b_kernel_offset + b"broken B kernel".len()],
            b"broken B kernel"
        );
    }

    #[test]
    fn m27_recovery_payload_is_independent_of_both_broken_system_slots() {
        let geometry =
            Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)
                .expect("M27 FAT12 geometry");
        let (image, _) = build_fat12_ab_image(
            b"loader",
            AbSlotImages {
                system_a: SlotPayload {
                    kernel: b"bad A kernel",
                    init: b"bad A init",
                },
                system_b: SlotPayload {
                    kernel: b"bad B kernel",
                    init: b"bad B init",
                },
                recovery: Some(SlotPayload {
                    kernel: b"valid recovery kernel",
                    init: b"valid recovery init",
                }),
            },
            geometry,
        )
        .expect("build Recovery-only fixture");

        let a_directory = cluster_offset(5, geometry);
        let b_directory = cluster_offset(6, geometry);
        let recovery_directory = cluster_offset(7, geometry);
        let a_kernel_cluster =
            u16::from_le_bytes([image[a_directory + 64 + 26], image[a_directory + 64 + 27]]);
        let b_kernel_cluster =
            u16::from_le_bytes([image[b_directory + 64 + 26], image[b_directory + 64 + 27]]);
        let recovery_kernel_cluster = u16::from_le_bytes([
            image[recovery_directory + 64 + 26],
            image[recovery_directory + 64 + 27],
        ]);
        assert_eq!(
            &image[cluster_offset(a_kernel_cluster, geometry)
                ..cluster_offset(a_kernel_cluster, geometry) + b"bad A kernel".len()],
            b"bad A kernel"
        );
        assert_eq!(
            &image[cluster_offset(b_kernel_cluster, geometry)
                ..cluster_offset(b_kernel_cluster, geometry) + b"bad B kernel".len()],
            b"bad B kernel"
        );
        assert_eq!(
            &image[cluster_offset(recovery_kernel_cluster, geometry)
                ..cluster_offset(recovery_kernel_cluster, geometry)
                    + b"valid recovery kernel".len()],
            b"valid recovery kernel"
        );
    }

    #[test]
    fn m17_fat12_image_fits_servo_init_above_legacy_floppy_limit() {
        let bootloader = b"bootloader";
        let kernel = b"kernel payload";
        let init = vec![0xa5; 2 * 1024 * 1024 + 123];
        let image = build_m17_fat12_image(bootloader, kernel, &init).expect("M17 FAT12 image");

        assert_eq!(image.len(), M17_IMAGE_SIZE);
        assert_eq!(u16::from_le_bytes([image[11], image[12]]), 512);
        assert_eq!(image[13], 64);
        assert_eq!(u16::from_le_bytes([image[17], image[18]]), 224);
        assert_eq!(u16::from_le_bytes([image[19], image[20]]), 0);
        assert_eq!(
            u32::from_le_bytes(image[32..36].try_into().unwrap()),
            261_415
        );
        assert_eq!(u16::from_le_bytes([image[22], image[23]]), 12);
        assert_eq!(&image[54..62], b"FAT12   ");

        let root_offset = (1 + 2 * 12) * 512;
        let data_offset = (1 + 2 * 12 + 14) * 512;
        let cluster_size = usize::from(image[13]) * 512;
        assert_eq!(&image[root_offset..root_offset + 3], b"EFI");
        let efi_directory = data_offset;
        let nagi_cluster = u16::from_le_bytes([
            image[efi_directory + 3 * 32 + 26],
            image[efi_directory + 3 * 32 + 27],
        ]);
        assert_eq!(nagi_cluster, 4);
        let nagi_directory = data_offset + (nagi_cluster as usize - 2) * cluster_size;
        let init_entry = nagi_directory + 3 * 32;
        assert_eq!(&image[init_entry..init_entry + 11], b"INIT    ELF");
        assert_eq!(
            u32::from_le_bytes(image[init_entry + 28..init_entry + 32].try_into().unwrap()),
            init.len() as u32
        );
        let init_cluster = u16::from_le_bytes([image[init_entry + 26], image[init_entry + 27]]);
        let init_offset = data_offset + (init_cluster as usize - 2) * cluster_size;
        assert_eq!(image[init_offset], 0xa5);
        assert_eq!(image[init_offset + init.len() - 1], 0xa5);

        let init_clusters = init.len().div_ceil(cluster_size);
        let last_cluster = init_cluster + init_clusters as u16 - 1;
        let fat_entry_offset = 512 + last_cluster as usize + last_cluster as usize / 2;
        let packed_fat_entry =
            u16::from_le_bytes([image[fat_entry_offset], image[fat_entry_offset + 1]]);
        let fat_entry = if last_cluster & 1 == 0 {
            packed_fat_entry & 0x0fff
        } else {
            packed_fat_entry >> 4
        };
        assert_eq!(fat_entry, 0x0fff);
    }

    #[test]
    fn m17_fat12_long_init_chain_preserves_every_cluster_link() {
        // CI #175 measured this pinned Servo init ELF before the image was
        // expanded. Exercise its complete FAT chain without allocating the
        // 128 MiB payload or full disk image.
        let geometry =
            Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)
                .expect("M17 FAT12 geometry");
        let init_size = 127_747_368_usize;
        let init_clusters = init_size.div_ceil(geometry.cluster_size());
        let first_cluster = 100_u16;
        let last_cluster = first_cluster + init_clusters as u16 - 1;
        assert_eq!(init_clusters, 3_899);
        assert!(usize::from(last_cluster) <= geometry.data_clusters() + 1);

        let mut fat =
            vec![0; geometry.fat_offset(FAT_COUNT - 1) + geometry.sectors_per_fat * SECTOR_SIZE];
        initialize_fats(&mut fat, geometry);
        write_chain(&mut fat, geometry, first_cluster, init_clusters);

        for fat_index in 0..FAT_COUNT {
            for index in 0..init_clusters {
                let cluster = first_cluster + index as u16;
                let offset = geometry.fat_offset(fat_index)
                    + usize::from(cluster)
                    + usize::from(cluster) / 2;
                let packed = u16::from_le_bytes([fat[offset], fat[offset + 1]]);
                let actual = if cluster & 1 == 0 {
                    packed & 0x0fff
                } else {
                    packed >> 4
                };
                let expected = if index + 1 == init_clusters {
                    0x0fff
                } else {
                    cluster + 1
                };
                assert_eq!(actual, expected, "FAT {fat_index}, cluster {cluster}");
            }
        }
    }

    #[test]
    fn rejects_empty_guest_files() {
        assert!(build_fat12_image(&[], b"kernel", b"init").is_err());
        assert!(build_fat12_image(b"loader", &[], b"init").is_err());
        assert!(build_fat12_image(b"loader", b"kernel", &[]).is_err());
    }
}
