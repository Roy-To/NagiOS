use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command as ProcessCommand};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

pub const IMAGE_SIZE: usize = 1_474_560;
// One sector of boot data, 12 sectors in each FAT, 14 root-directory sectors,
// and exactly 4,084 32 KiB data clusters. This is the largest valid FAT12
// volume geometry and leaves room for the current Servo init ELF plus kernel.
pub const M17_IMAGE_SIZE: usize = 261_415 * SECTOR_SIZE;
pub const PERSISTENT_DISK_SIZE: u64 = 16 * 1024 * 1024;
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
        if image_size == 0 || !image_size.is_multiple_of(SECTOR_SIZE) {
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
) -> Result<ImageLayout, String> {
    write_fat12_image_with_geometry(path, bootloader, kernel, init, Fat12Geometry::legacy())
}

pub fn write_m17_fat12_image(
    path: &Path,
    bootloader: &[u8],
    kernel: &[u8],
    init: &[u8],
) -> Result<ImageLayout, String> {
    let geometry = Fat12Geometry::new(M17_IMAGE_SIZE, M17_SECTORS_PER_CLUSTER, ROOT_ENTRY_COUNT)?;
    write_fat12_image_with_geometry(path, bootloader, kernel, init, geometry)
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
            if metadata.len() != PERSISTENT_DISK_SIZE {
                return Err(format!(
                    "persistent data disk has size {} bytes; expected {} bytes: {}",
                    metadata.len(),
                    PERSISTENT_DISK_SIZE,
                    path.display()
                ));
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let file = fs::File::create(path).map_err(|create_error| {
                format!(
                    "cannot create persistent data disk {}: {create_error}",
                    path.display()
                )
            })?;
            file.set_len(PERSISTENT_DISK_SIZE)
                .map_err(|set_len_error| {
                    format!(
                        "cannot size persistent data disk {}: {set_len_error}",
                        path.display()
                    )
                })?;
            Ok(false)
        }
        Err(error) => Err(format!(
            "cannot inspect persistent data disk {}: {error}",
            path.display()
        )),
    }
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
    let QemuConfig {
        serial_log,
        acceptance_marker,
        timeout,
        ..
    } = *config;
    let serial_device = format!("file:{}", external_path(serial_log));
    let mut child = spawn_qemu(config, &serial_device)?;
    let status = wait_for_qemu(&mut child, serial_log, acceptance_marker, timeout)?;
    Ok(status)
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
    let mut child = spawn_qemu_with_display(config, &serial_device, qmp_port, vnc_port - 5900)?;
    let deadline = Instant::now() + config.timeout;
    let mut serial = Vec::new();
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

    let marker = config.acceptance_marker.as_bytes();
    let mut buffer = [0_u8; 4096];
    let mut events_sent = false;
    loop {
        match serial_stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                serial.extend_from_slice(&buffer[..count]);
                if !events_sent && bytes_contain(&serial, ready_marker.as_bytes()) {
                    for event in events {
                        if let Err(error) = qmp_exchange(&mut qmp_stream, event, deadline) {
                            terminate_qemu(&mut child, config.serial_log, &serial);
                            return Err(error);
                        }
                    }
                    events_sent = true;
                }
                if bytes_contain(&serial, marker) {
                    child.kill().map_err(|error| {
                        format!("guest reached acceptance but termination failed: {error}")
                    })?;
                    let status = child.wait().map_err(|error| {
                        format!("cannot reap QEMU after GUI acceptance: {error}")
                    })?;
                    fs::write(config.serial_log, &serial).map_err(|error| {
                        format!(
                            "cannot write GUI serial log {}: {error}",
                            config.serial_log.display()
                        )
                    })?;
                    return Ok(status.code().unwrap_or(-1));
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
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::write(config.serial_log, &serial);
            return Err(format!(
                "GUI QEMU did not reach acceptance within {} seconds",
                config.timeout.as_secs()
            ));
        }
    }
    let _ = fs::write(config.serial_log, &serial);
    Ok(child
        .wait()
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(-1))
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
    let QemuConfig {
        qemu,
        ovmf_code,
        ovmf_vars_template,
        disk_image,
        persistent_disk,
        vars_copy,
        ..
    } = *config;
    fs::copy(ovmf_vars_template, vars_copy).map_err(|error| {
        format!(
            "cannot copy OVMF variables template {} to {}: {error}",
            ovmf_vars_template.display(),
            vars_copy.display()
        )
    })?;
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
    let disk_drive = format!("if=virtio,format=raw,file={}", external_path(disk_image));
    let persistent_drive = format!(
        "if=none,id=nagi-data,format=raw,file={}",
        external_path(persistent_disk)
    );
    let mut command = ProcessCommand::new(qemu);
    command.args([
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
        "driver=dsound,id=nagi-audio",
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
        "-drive",
        &persistent_drive,
        "-device",
        "virtio-blk-pci,drive=nagi-data,disable-modern=on",
        "-serial",
        serial_device,
    ]);
    if qmp_port == 0 {
        command.args(["-display", "none", "-monitor", "none"]);
    } else {
        let qmp = format!("tcp:127.0.0.1:{qmp_port},server=on,wait=off");
        let display = format!("vnc=127.0.0.1:{vnc_display}");
        command.arg("-qmp").arg(qmp).arg("-display").arg(display);
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

fn terminate_qemu(child: &mut Child, serial_log: &Path, serial: &[u8]) {
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::write(serial_log, serial);
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

fn qmp_exchange(stream: &mut TcpStream, command: &str, deadline: Instant) -> Result<(), String> {
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
        return Ok(());
    }
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

fn wait_for_qemu(
    child: &mut Child,
    serial_log: &Path,
    acceptance_marker: &str,
    timeout: Duration,
) -> Result<i32, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if fs::read_to_string(serial_log)
            .map(|serial| guest_reached_acceptance(&serial, acceptance_marker))
            .unwrap_or(false)
        {
            child.kill().map_err(|error| {
                format!("guest reached acceptance but termination failed: {error}")
            })?;
            let status = child
                .wait()
                .map_err(|error| format!("cannot reap QEMU after acceptance: {error}"))?;
            return Ok(status.code().unwrap_or(-1));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll QEMU: {error}"))?
        {
            return Ok(status.code().unwrap_or(-1));
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .map_err(|error| format!("QEMU timeout and termination failed: {error}"))?;
            let _ = child.wait();
            return Err(format!(
                "QEMU did not exit within {} seconds",
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn guest_reached_acceptance(serial: &str, acceptance_marker: &str) -> bool {
    serial.contains(acceptance_marker)
}

#[cfg(test)]
mod tests {
    use super::{
        build_fat12_image, build_m17_fat12_image, ensure_persistent_disk, guest_reached_acceptance,
        DATA_OFFSET, GUEST_ACCEPTANCE_MARKER, IMAGE_SIZE, M17_IMAGE_SIZE, PERSISTENT_DISK_SIZE,
        ROOT_OFFSET,
    };

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
    fn persistent_disk_is_created_once_and_existing_data_is_preserved() {
        use std::io::{Read, Seek, SeekFrom, Write};

        let path =
            std::env::temp_dir().join(format!("nagi-persistent-disk-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        std::fs::write(&path, b"short").expect("write wrong-sized disk");
        assert!(ensure_persistent_disk(&path).is_err());
        assert_eq!(std::fs::metadata(&path).expect("short metadata").len(), 5);
        std::fs::remove_file(&path).expect("remove wrong-sized disk");

        assert!(!ensure_persistent_disk(&path).expect("create disk"));
        assert_eq!(
            std::fs::metadata(&path).expect("disk metadata").len(),
            PERSISTENT_DISK_SIZE
        );
        let offset = PERSISTENT_DISK_SIZE - 1;
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
        let _ = std::fs::remove_file(&path);
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
    fn rejects_empty_guest_files() {
        assert!(build_fat12_image(&[], b"kernel", b"init").is_err());
        assert!(build_fat12_image(b"loader", &[], b"init").is_err());
        assert!(build_fat12_image(b"loader", b"kernel", &[]).is_err());
    }
}
