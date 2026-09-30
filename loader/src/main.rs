#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

use core::mem;
use core::ptr;
#[cfg(feature = "m27-ab-slot-acceptance")]
use core::time::Duration;

#[cfg(feature = "m27-ab-slot-acceptance")]
use nagi_bootinfo::{BOOT_READY_RECORD_SIZE, BootReadyRecord};
use nagi_bootinfo::{
    BootControlInfo, BootInfo, FirmwareDateTime, FramebufferInfo, InitImageInfo, MemoryMapInfo,
    REALTIME_UNAVAILABLE_NS, firmware_time_to_unix_ns,
};
use nagi_loader::ab::SystemSlot;
use nagi_loader::elf::{LoadPlan, parse};
use uefi::boot::{AllocateType, MemoryType, SearchType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::proto::media::file::{Directory, File, FileAttribute, FileMode, FileType, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::proto::media::partition::PartitionInfo;
use uefi::system::with_config_table;
use uefi::table::cfg::ConfigTableEntry;

const PAGE_SIZE: u64 = 4096;
const MAX_KERNEL_IMAGE_SIZE: usize = 4 * 1024 * 1024;
const MAX_INIT_IMAGE_SIZE: usize = 128 * 1024 * 1024;
const INIT_READ_CHUNK_SIZE: usize = 1024 * 1024;
const INIT_IMAGE_MAX_ADDRESS: u64 = 0xFFFF_FFFF;
const ESP_UNIQUE_GUID: uefi::Guid = uefi::guid!("4e414702-0001-4e41-4749-000000000000");
const SYSTEM_A_UNIQUE_GUID: uefi::Guid = uefi::guid!("4e414702-0001-4e41-4749-000000000001");
const SYSTEM_B_UNIQUE_GUID: uefi::Guid = uefi::guid!("4e414702-0001-4e41-4749-000000000002");
const RECOVERY_UNIQUE_GUID: uefi::Guid = uefi::guid!("4e414702-0001-4e41-4749-000000000004");

static mut KERNEL_IMAGE: [u8; MAX_KERNEL_IMAGE_SIZE] = [0; MAX_KERNEL_IMAGE_SIZE];
static mut BOOT_INFO: BootInfo = BootInfo::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootImageSelection {
    Default,
    #[allow(dead_code)]
    SystemA,
    SystemB,
    Recovery,
}

impl BootImageSelection {
    const fn system_slot(self) -> Option<SystemSlot> {
        match self {
            Self::SystemA => Some(SystemSlot::A),
            Self::SystemB => Some(SystemSlot::B),
            Self::Default | Self::Recovery => None,
        }
    }
}

#[entry]
fn main() -> Status {
    if let Err(error) = uefi::helpers::init() {
        let _ = error;
        return fail(error_message("Nagi Loader: helper init failed"));
    }

    #[cfg(feature = "m27-ab-slot-acceptance")]
    let (image_selection, boot_control) = match m27_boot_control_decision() {
        Ok((selection, context)) => (selection, context),
        Err(message) => return fail(error_message(message)),
    };
    #[cfg(not(feature = "m27-ab-slot-acceptance"))]
    let (image_selection, boot_control) = (BootImageSelection::Default, BootControlInfo::default());
    let selected_slot = image_selection.system_slot();

    let kernel_size = match read_kernel(boot::image_handle(), image_selection) {
        Ok(size) => size,
        Err(message) => {
            report_m27_trial_payload_rejection(selected_slot);
            return fail(message);
        }
    };
    let plan = {
        let bytes = unsafe { &KERNEL_IMAGE[..kernel_size] };
        match parse(bytes) {
            Ok(plan) => plan,
            Err(_) => {
                report_m27_trial_payload_rejection(selected_slot);
                return fail(error_message("Nagi Loader: invalid ELF"));
            }
        }
    };
    if let Err(message) = load_segments(plan, kernel_size) {
        report_m27_trial_payload_rejection(selected_slot);
        return fail(message);
    }
    let init_image = match read_init(boot::image_handle(), image_selection) {
        Ok(info) => info,
        Err(message) => {
            report_m27_trial_payload_rejection(selected_slot);
            return fail(message);
        }
    };

    let framebuffer = match gather_framebuffer() {
        Ok(info) => info,
        Err(_) => return fail(error_message("Nagi Loader: GOP unavailable")),
    };
    let acpi_rsdp = find_acpi_rsdp();
    if acpi_rsdp == 0 {
        return fail(error_message("Nagi Loader: ACPI RSDP unavailable"));
    }

    let realtime_epoch_ns = uefi::runtime::get_time()
        .map(|time| {
            firmware_time_to_unix_ns(FirmwareDateTime {
                year: time.year(),
                month: time.month(),
                day: time.day(),
                hour: time.hour(),
                minute: time.minute(),
                second: time.second(),
                nanosecond: time.nanosecond(),
                time_zone: time.time_zone(),
                daylight_flags: time.daylight().bits(),
            })
        })
        .unwrap_or(REALTIME_UNAVAILABLE_NS);

    let memory_map = unsafe { boot::exit_boot_services(None) };
    let metadata = memory_map.meta();
    if memory_map.is_empty() {
        return halt_after_exit();
    }
    let boot_info = BootInfo {
        magic: nagi_bootinfo::BOOT_INFO_MAGIC,
        version: nagi_bootinfo::BOOT_INFO_VERSION,
        size: mem::size_of::<BootInfo>() as u32,
        memory_map: MemoryMapInfo {
            address: memory_map.buffer().as_ptr() as u64,
            entry_count: memory_map.len() as u64,
            entry_size: metadata.desc_size as u64,
            entry_version: metadata.desc_version,
            _reserved: 0,
        },
        framebuffer,
        acpi_rsdp,
        init_image,
        realtime_epoch_ns,
        boot_control,
    };
    unsafe {
        ptr::write_volatile(&raw mut BOOT_INFO, boot_info);
        mem::forget(memory_map);
        let entry: unsafe extern "win64" fn(*const BootInfo) -> ! =
            mem::transmute(plan.entry as usize);
        entry(&raw const BOOT_INFO);
    }
}

#[cfg(feature = "m27-ab-slot-acceptance")]
fn m27_boot_control_decision() -> Result<(BootImageSelection, BootControlInfo), &'static str> {
    use nagi_loader::ab::uefi_store::{
        NAGI_BOOT_CONTROL_VENDOR, NAGI_BOOT_READY_VARIABLE, UefiVariableBootControlStore,
    };
    use nagi_loader::ab::{BootControlJournal, BootControlState};
    use uefi::runtime::{self, VariableAttributes};

    let mut journal = BootControlJournal::new(UefiVariableBootControlStore::new());
    let mut state = journal
        .load()
        .map_err(|_| "Nagi Loader: M27 boot-control journal read failed")?;

    if runtime::variable_exists(NAGI_BOOT_READY_VARIABLE, &NAGI_BOOT_CONTROL_VENDOR)
        .unwrap_or(false)
    {
        let mut bytes = [0; BOOT_READY_RECORD_SIZE];
        let record = runtime::get_variable(
            NAGI_BOOT_READY_VARIABLE,
            &NAGI_BOOT_CONTROL_VENDOR,
            &mut bytes,
        )
        .ok()
        .and_then(|(value, attributes)| {
            let required = VariableAttributes::NON_VOLATILE
                .union(VariableAttributes::BOOTSERVICE_ACCESS)
                .union(VariableAttributes::RUNTIME_ACCESS);
            (attributes == required)
                .then(|| BootReadyRecord::decode(value))
                .flatten()
        });
        if let Some(record) = record {
            let matching_pending = match state.pending_slot() {
                Some(SystemSlot::A) => record.slot == SystemSlot::A as u8,
                Some(SystemSlot::B) => record.slot == SystemSlot::B as u8,
                None => false,
            } && record.attempt == state.attempts()
                && record.journal_generation == state.generation();
            if matching_pending {
                let slot = if record.slot == SystemSlot::A as u8 {
                    SystemSlot::A
                } else {
                    SystemSlot::B
                };
                journal
                    .mark_boot_success(slot)
                    .map_err(|_| "Nagi Loader: M27 readiness promotion failed")?;
                uefi::println!(
                    "Nagi M27 readiness record consumed slot={} PASS",
                    if slot == SystemSlot::A { "A" } else { "B" }
                );
                state = journal
                    .load()
                    .map_err(|_| "Nagi Loader: M27 promoted journal read failed")?;
            }
        }
        let _ = runtime::delete_variable(NAGI_BOOT_READY_VARIABLE, &NAGI_BOOT_CONTROL_VENDOR);
    }

    let selection = m27_boot_menu(state);
    if selection == BootImageSelection::Recovery {
        uefi::println!("Nagi M27 manual selection: Recovery; boot journal unchanged PASS");
        return Ok((BootImageSelection::Recovery, BootControlInfo::default()));
    }
    if selection.system_slot() == Some(state.confirmed_slot()) {
        uefi::println!(
            "Nagi M27 manual selection: confirmed slot={} (pending trial preserved) PASS",
            if state.confirmed_slot() == SystemSlot::A {
                "A"
            } else {
                "B"
            }
        );
        uefi::println!("Nagi M27 UEFI variable journal persistence PASS");
        return Ok((selection, BootControlInfo::default()));
    }

    let selected_pending_candidate = selection
        .system_slot()
        .filter(|slot| state.pending_slot() == Some(*slot));
    if selected_pending_candidate.is_none() && state.generation() == 0 {
        // The acceptance fixture seeds System B on its first automatic boot.
        // Recovery and explicit confirmed-slot selection never reach this path.
        journal
            .stage_update(SystemSlot::B)
            .map_err(|_| "Nagi Loader: M27 boot-control trial staging failed")?;
    }

    let decision = journal
        .begin_boot()
        .map_err(|_| "Nagi Loader: M27 boot-control decision failed")?;
    if decision.rolled_back {
        uefi::println!("Nagi M27 persistence decision: rollback slot=A");
    } else if decision.trial_attempt == 0 {
        let state = journal
            .load()
            .map_err(|_| "Nagi Loader: M27 confirmed journal read failed")?;
        uefi::println!(
            "Nagi M27 persistence decision: confirmed slot={}",
            if state.confirmed_slot() == SystemSlot::A {
                "A"
            } else {
                "B"
            }
        );
    } else {
        uefi::println!(
            "Nagi M27 persistence decision: trial attempt={} slot={}",
            decision.trial_attempt,
            if decision.slot == SystemSlot::A {
                "A"
            } else {
                "B"
            }
        );
    }
    uefi::println!("Nagi M27 UEFI variable journal persistence PASS");

    let context = if decision.trial_attempt == 0 {
        BootControlInfo::default()
    } else {
        let state: BootControlState = journal
            .load()
            .map_err(|_| "Nagi Loader: M27 trial context read failed")?;
        if state.pending_slot() != Some(decision.slot) || state.attempts() != decision.trial_attempt
        {
            return Err("Nagi Loader: M27 trial context mismatch");
        }
        let system_table =
            uefi::table::system_table_raw().ok_or("Nagi Loader: M27 runtime table unavailable")?;
        // SAFETY: UEFI initialized the system table, which remains valid while
        // the loader is running with boot services active.
        let system_table = unsafe { system_table.as_ref() };
        // SAFETY: the firmware supplies a valid runtime-services table.
        let runtime_services = unsafe { system_table.runtime_services.as_ref() }
            .ok_or("Nagi Loader: M27 runtime services unavailable")?;
        BootControlInfo {
            set_variable_address: runtime_services.set_variable as *const () as u64,
            journal_generation: state.generation(),
            slot: decision.slot as u8,
            attempt: decision.trial_attempt,
            reserved: [0; 6],
        }
    };
    Ok((
        match decision.slot {
            SystemSlot::A => BootImageSelection::SystemA,
            SystemSlot::B => BootImageSelection::SystemB,
        },
        context,
    ))
}

#[cfg(feature = "m27-ab-slot-acceptance")]
fn m27_boot_menu(state: nagi_loader::ab::BootControlState) -> BootImageSelection {
    use uefi::proto::console::text::Key;

    uefi::println!(
        "Nagi M27 boot menu: confirmed={} pending={} (A=System A, B=System B, R=Recovery)",
        if state.confirmed_slot() == SystemSlot::A {
            "A"
        } else {
            "B"
        },
        match state.pending_slot() {
            Some(SystemSlot::A) => "A",
            Some(SystemSlot::B) => "B",
            None => "none",
        }
    );
    uefi::println!("Nagi M27 Recovery boot menu READY");

    for _ in 0..30 {
        let key = uefi::system::with_stdin(|stdin| stdin.read_key().ok().flatten());
        if let Some(Key::Printable(key)) = key {
            let key = char::from(key).to_ascii_uppercase();
            let requested = match key {
                'A' => Some(SystemSlot::A),
                'B' => Some(SystemSlot::B),
                'R' => return BootImageSelection::Recovery,
                _ => None,
            };
            if let Some(slot) = requested {
                if slot == state.confirmed_slot() || state.pending_slot() == Some(slot) {
                    return match slot {
                        SystemSlot::A => BootImageSelection::SystemA,
                        SystemSlot::B => BootImageSelection::SystemB,
                    };
                }
                uefi::println!(
                    "Nagi M27 boot menu: System {} unavailable (no staged image)",
                    if slot == SystemSlot::A { "A" } else { "B" }
                );
            }
        }
        uefi::boot::stall(Duration::from_millis(100));
    }
    uefi::println!("Nagi M27 boot menu timeout; using automatic A/B policy");
    BootImageSelection::Default
}

fn report_m27_trial_payload_rejection(selected_slot: Option<SystemSlot>) {
    #[cfg(feature = "m27-ab-slot-acceptance")]
    if selected_slot == Some(SystemSlot::B) {
        uefi::println!("Nagi M27 trial payload rejected slot=B");
    }
    #[cfg(not(feature = "m27-ab-slot-acceptance"))]
    let _ = selected_slot;
}

fn read_kernel(image_handle: Handle, selection: BootImageSelection) -> Result<usize, &'static str> {
    let (mut root, from_partition) = open_selected_volume_root(image_handle, selection)?;
    let path = if from_partition {
        cstr16!("\\KERNEL.ELF")
    } else {
        match selection {
            BootImageSelection::SystemA => cstr16!("\\EFI\\NAGI\\SYSTEMA\\KERNEL.ELF"),
            BootImageSelection::SystemB => cstr16!("\\EFI\\NAGI\\SYSTEMB\\KERNEL.ELF"),
            BootImageSelection::Recovery => cstr16!("\\EFI\\NAGI\\RECOVERY\\KERNEL.ELF"),
            BootImageSelection::Default => cstr16!("\\EFI\\NAGI\\KERNEL.ELF"),
        }
    };
    let handle = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .map_err(|_| error_message("Nagi Loader: KERNEL.ELF not found"))?;
    let mut file = match handle
        .into_type()
        .map_err(|_| error_message("Nagi Loader: kernel file type failed"))?
    {
        FileType::Regular(file) => file,
        FileType::Dir(_) => return Err(error_message("Nagi Loader: KERNEL.ELF is a directory")),
    };
    let buffer = unsafe {
        core::slice::from_raw_parts_mut(
            ptr::addr_of_mut!(KERNEL_IMAGE).cast::<u8>(),
            MAX_KERNEL_IMAGE_SIZE,
        )
    };
    read_bounded_regular_file(
        &mut file,
        buffer,
        "Nagi Loader: kernel size failed",
        "Nagi Loader: kernel size is unsupported",
        "Nagi Loader: kernel rewind failed",
        "Nagi Loader: kernel read failed",
        "Nagi Loader: short kernel read",
    )
}

fn read_init(
    image_handle: Handle,
    selection: BootImageSelection,
) -> Result<InitImageInfo, &'static str> {
    let (mut root, from_partition) = open_selected_volume_root(image_handle, selection)?;
    let path = if from_partition {
        cstr16!("\\INIT.ELF")
    } else {
        match selection {
            BootImageSelection::SystemA => cstr16!("\\EFI\\NAGI\\SYSTEMA\\INIT.ELF"),
            BootImageSelection::SystemB => cstr16!("\\EFI\\NAGI\\SYSTEMB\\INIT.ELF"),
            BootImageSelection::Recovery => cstr16!("\\EFI\\NAGI\\RECOVERY\\INIT.ELF"),
            BootImageSelection::Default => cstr16!("\\EFI\\NAGI\\INIT.ELF"),
        }
    };
    let handle = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .map_err(|_| error_message("Nagi Loader: INIT.ELF not found"))?;
    let mut file = match handle
        .into_type()
        .map_err(|_| error_message("Nagi Loader: init file type failed"))?
    {
        FileType::Regular(file) => file,
        FileType::Dir(_) => return Err(error_message("Nagi Loader: INIT.ELF is a directory")),
    };
    file.set_position(RegularFile::END_OF_FILE)
        .map_err(|_| error_message("Nagi Loader: init size failed"))?;
    let size = usize::try_from(
        file.get_position()
            .map_err(|_| error_message("Nagi Loader: init size failed"))?,
    )
    .map_err(|_| error_message("Nagi Loader: init size is unsupported"))?;
    if size == 0 || size > MAX_INIT_IMAGE_SIZE {
        return Err(error_message("Nagi Loader: init size is unsupported"));
    }
    let pages = init_image_page_count(size)
        .ok_or(error_message("Nagi Loader: init page count overflow"))?;
    let allocation = boot::allocate_pages(
        AllocateType::MaxAddress(INIT_IMAGE_MAX_ADDRESS),
        MemoryType::LOADER_DATA,
        pages,
    )
    .map_err(|_| error_message("Nagi Loader: init allocation below 4 GiB failed"))?;
    let allocation_start = allocation.as_ptr() as u64;
    let allocation_bytes = (pages as u64)
        .checked_mul(PAGE_SIZE)
        .ok_or(error_message("Nagi Loader: init allocation size overflow"))?;
    let allocation_end = allocation_start
        .checked_add(allocation_bytes)
        .ok_or(error_message(
            "Nagi Loader: init allocation address overflow",
        ))?;
    if allocation_end > INIT_IMAGE_MAX_ADDRESS + 1 {
        return Err(error_message("Nagi Loader: init allocation exceeds 4 GiB"));
    }
    unsafe {
        ptr::write_bytes(allocation.as_ptr(), 0, allocation_bytes as usize);
    }
    file.set_position(0)
        .map_err(|_| error_message("Nagi Loader: init rewind failed"))?;
    let buffer = unsafe { core::slice::from_raw_parts_mut(allocation.as_ptr(), size) };
    let mut offset = 0;
    while offset < size {
        let chunk_end = (offset + INIT_READ_CHUNK_SIZE).min(size);
        let count = file.read(&mut buffer[offset..chunk_end]).map_err(|error| {
            uefi::println!(
                "Nagi Loader: init read failed status={:?} offset={:#x} request={:#x} file={:#x}",
                error,
                offset,
                chunk_end - offset,
                size,
            );
            error_message("Nagi Loader: init read failed")
        })?;
        if count == 0 {
            return Err(error_message("Nagi Loader: short init read"));
        }
        offset += count;
    }
    Ok(InitImageInfo {
        address: allocation_start,
        size: size as u64,
    })
}

fn open_selected_volume_root(
    image_handle: Handle,
    selection: BootImageSelection,
) -> Result<(Directory, bool), &'static str> {
    let unique_guid = match selection {
        BootImageSelection::Default | BootImageSelection::SystemA => SYSTEM_A_UNIQUE_GUID,
        BootImageSelection::SystemB => SYSTEM_B_UNIQUE_GUID,
        BootImageSelection::Recovery => RECOVERY_UNIQUE_GUID,
    };
    if let Some(root) = open_partition_root(unique_guid)? {
        if selection != BootImageSelection::Recovery {
            uefi::println!(
                "Nagi M30 GPT partition boot: System {} PASS",
                if selection == BootImageSelection::SystemB {
                    "B"
                } else {
                    "A"
                }
            );
        } else {
            uefi::println!("Nagi M30 GPT partition boot: Recovery PASS");
        }
        return Ok((root, true));
    }

    let mut filesystem = boot::get_image_file_system(image_handle)
        .map_err(|_| error_message("Nagi Loader: filesystem unavailable"))?;
    let root = filesystem
        .open_volume()
        .map_err(|_| error_message("Nagi Loader: volume unavailable"))?;
    Ok((root, false))
}

fn open_partition_root(unique_guid: uefi::Guid) -> Result<Option<Directory>, &'static str> {
    let handles = match boot::locate_handle_buffer(SearchType::from_proto::<PartitionInfo>()) {
        Ok(handles) => handles,
        Err(_) => return Ok(None),
    };
    let mut matched_handle = None;
    let mut found_nagi_boot_partition = false;
    for handle in handles.iter().copied() {
        let partition = boot::open_protocol_exclusive::<PartitionInfo>(handle)
            .map_err(|_| error_message("Nagi Loader: GPT partition info unavailable"))?;
        let Some(entry) = partition.gpt_partition_entry() else {
            continue;
        };
        let candidate_guid = entry.unique_partition_guid;
        found_nagi_boot_partition |= guid_matches(candidate_guid, ESP_UNIQUE_GUID)
            || guid_matches(candidate_guid, SYSTEM_A_UNIQUE_GUID)
            || guid_matches(candidate_guid, SYSTEM_B_UNIQUE_GUID)
            || guid_matches(candidate_guid, RECOVERY_UNIQUE_GUID);
        if guid_matches(candidate_guid, unique_guid) {
            if matched_handle.is_some() {
                return Err(error_message("Nagi Loader: duplicate GPT partition GUID"));
            }
            matched_handle = Some(handle);
        }
    }
    let Some(handle) = matched_handle else {
        if !found_nagi_boot_partition {
            return Ok(None);
        }
        return Err(error_message(
            "Nagi Loader: selected GPT partition is missing",
        ));
    };
    let mut filesystem = boot::open_protocol_exclusive::<SimpleFileSystem>(handle)
        .map_err(|_| error_message("Nagi Loader: GPT partition filesystem unavailable"))?;
    filesystem
        .open_volume()
        .map(Some)
        .map_err(|_| error_message("Nagi Loader: GPT partition volume unavailable"))
}

fn guid_matches(left: uefi::Guid, right: uefi::Guid) -> bool {
    let left = left.to_bytes();
    let right = right.to_bytes();
    let mut difference = 0u8;
    for index in 0..left.len() {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

#[allow(clippy::too_many_arguments)]
fn read_bounded_regular_file(
    file: &mut RegularFile,
    buffer: &mut [u8],
    size_error: &'static str,
    unsupported_size: &'static str,
    rewind_error: &'static str,
    read_error: &'static str,
    short_read: &'static str,
) -> Result<usize, &'static str> {
    file.set_position(RegularFile::END_OF_FILE)
        .map_err(|_| error_message(size_error))?;
    let size = file.get_position().map_err(|_| error_message(size_error))?;
    let size = usize::try_from(size).map_err(|_| error_message(unsupported_size))?;
    if size == 0 || size > buffer.len() {
        return Err(error_message(unsupported_size));
    }
    file.set_position(0)
        .map_err(|_| error_message(rewind_error))?;
    let mut offset = 0;
    while offset < size {
        let count = file
            .read(&mut buffer[offset..size])
            .map_err(|_| error_message(read_error))?;
        if count == 0 {
            return Err(error_message(short_read));
        }
        offset += count;
    }
    Ok(size)
}

fn init_image_page_count(size: usize) -> Option<usize> {
    if size == 0 {
        return None;
    }
    size.checked_add(PAGE_SIZE as usize - 1)
        .map(|rounded| rounded / PAGE_SIZE as usize)
}

fn load_segments(plan: LoadPlan, _kernel_size: usize) -> Result<(), &'static str> {
    for (index, segment) in plan.segments[..plan.segment_count].iter().enumerate() {
        let pages = segment
            .memory_size
            .checked_add(PAGE_SIZE - 1)
            .ok_or(error_message("Nagi Loader: segment size overflow"))?
            / PAGE_SIZE;
        if pages == 0 {
            return Err(error_message("Nagi Loader: empty load segment"));
        }
        let allocation = match boot::allocate_pages(
            AllocateType::Address(segment.physical_address),
            MemoryType::LOADER_DATA,
            usize::try_from(pages)
                .map_err(|_| error_message("Nagi Loader: segment page count overflow"))?,
        ) {
            Ok(allocation) => allocation,
            Err(error) => {
                uefi::println!(
                    "Nagi Loader: segment allocation failed index={} address={:#x} bytes={:#x} pages={} status={:?}",
                    index,
                    segment.physical_address,
                    segment.memory_size,
                    pages,
                    error.status(),
                );
                log_segment_memory_map(segment.physical_address, pages);
                return Err(error_message("Nagi Loader: segment allocation failed"));
            }
        };
        if allocation.as_ptr() as u64 != segment.physical_address {
            return Err(error_message(
                "Nagi Loader: segment allocated at wrong address",
            ));
        }
        unsafe {
            ptr::write_bytes(allocation.as_ptr(), 0, (pages * PAGE_SIZE) as usize);
            ptr::copy_nonoverlapping(
                ptr::addr_of!(KERNEL_IMAGE)
                    .cast::<u8>()
                    .add(segment.file_offset as usize),
                allocation.as_ptr(),
                segment.file_size as usize,
            );
        }
    }
    Ok(())
}

fn log_segment_memory_map(address: u64, pages: u64) {
    let end = address.saturating_add(pages.saturating_mul(PAGE_SIZE));
    let Ok(memory_map) = boot::memory_map(MemoryType::LOADER_DATA) else {
        uefi::println!("Nagi Loader: memory map unavailable after segment allocation failure");
        return;
    };
    let mut found = false;
    for descriptor in memory_map.entries() {
        let descriptor_end = descriptor
            .phys_start
            .saturating_add(descriptor.page_count.saturating_mul(PAGE_SIZE));
        if descriptor.phys_start < end && address < descriptor_end {
            found = true;
            uefi::println!(
                "Nagi Loader: overlapping memory map type={:?} start={:#x} pages={} end={:#x}",
                descriptor.ty,
                descriptor.phys_start,
                descriptor.page_count,
                descriptor_end,
            );
        }
    }
    if !found {
        uefi::println!("Nagi Loader: no memory map descriptor covers requested segment range");
    }
}

fn gather_framebuffer() -> Result<FramebufferInfo, uefi::Error> {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle)?;
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();
    let pixel_format = match mode.pixel_format() {
        PixelFormat::Rgb => 0,
        PixelFormat::Bgr => 1,
        PixelFormat::Bitmask => 2,
        PixelFormat::BltOnly => return Err(Status::UNSUPPORTED.into()),
    };
    let mut framebuffer = gop.frame_buffer();
    Ok(FramebufferInfo {
        address: framebuffer.as_mut_ptr() as u64,
        byte_size: framebuffer.size() as u64,
        width: width as u32,
        height: height as u32,
        pixels_per_scanline: mode.stride() as u32,
        pixel_format,
    })
}

fn find_acpi_rsdp() -> u64 {
    let mut address = 0;
    with_config_table(|entries| {
        for entry in entries {
            if entry.guid == ConfigTableEntry::ACPI2_GUID
                || (address == 0 && entry.guid == ConfigTableEntry::ACPI_GUID)
            {
                address = entry.address as u64;
                if entry.guid == ConfigTableEntry::ACPI2_GUID {
                    break;
                }
            }
        }
    });
    address
}

fn fail(message: &'static str) -> Status {
    uefi::println!("{}", message);
    Status::LOAD_ERROR
}

fn error_message(message: &'static str) -> &'static str {
    message
}

fn halt_after_exit() -> Status {
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_image_pages_round_up_and_reject_zero() {
        assert_eq!(init_image_page_count(1), Some(1));
        assert_eq!(init_image_page_count(4096), Some(1));
        assert_eq!(init_image_page_count(4097), Some(2));
        assert_eq!(init_image_page_count(0), None);
    }
}
