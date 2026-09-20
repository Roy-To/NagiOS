#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

use core::mem;
use core::ptr;

use nagi_bootinfo::{BootInfo, FramebufferInfo, InitImageInfo, MemoryMapInfo};
use nagi_loader::elf::{parse, LoadPlan};
use uefi::boot::{AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileMode, FileType, RegularFile};
use uefi::system::with_config_table;
use uefi::table::cfg::ConfigTableEntry;

const PAGE_SIZE: u64 = 4096;
const MAX_KERNEL_IMAGE_SIZE: usize = 4 * 1024 * 1024;
const MAX_INIT_IMAGE_SIZE: usize = 4 * 1024 * 1024;

static mut KERNEL_IMAGE: [u8; MAX_KERNEL_IMAGE_SIZE] = [0; MAX_KERNEL_IMAGE_SIZE];
static mut INIT_IMAGE: [u8; MAX_INIT_IMAGE_SIZE] = [0; MAX_INIT_IMAGE_SIZE];
static mut BOOT_INFO: BootInfo = BootInfo::new();

#[entry]
fn main() -> Status {
    if let Err(error) = uefi::helpers::init() {
        let _ = error;
        return fail(error_message("Nagi Loader: helper init failed"));
    }

    let kernel_size = match read_kernel(boot::image_handle()) {
        Ok(size) => size,
        Err(message) => return fail(message),
    };
    let plan = {
        let bytes = unsafe { &KERNEL_IMAGE[..kernel_size] };
        match parse(bytes) {
            Ok(plan) => plan,
            Err(_) => return fail(error_message("Nagi Loader: invalid ELF")),
        }
    };
    if let Err(message) = load_segments(plan, kernel_size) {
        return fail(message);
    }
    let init_image = match read_init(boot::image_handle()) {
        Ok(info) => info,
        Err(message) => return fail(message),
    };

    let framebuffer = match gather_framebuffer() {
        Ok(info) => info,
        Err(_) => return fail(error_message("Nagi Loader: GOP unavailable")),
    };
    let acpi_rsdp = find_acpi_rsdp();
    if acpi_rsdp == 0 {
        return fail(error_message("Nagi Loader: ACPI RSDP unavailable"));
    }

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
    };
    unsafe {
        ptr::write_volatile(&raw mut BOOT_INFO, boot_info);
        mem::forget(memory_map);
        let entry: unsafe extern "win64" fn(*const BootInfo) -> ! =
            mem::transmute(plan.entry as usize);
        entry(&raw const BOOT_INFO);
    }
}

fn read_kernel(image_handle: Handle) -> Result<usize, &'static str> {
    let mut filesystem = boot::get_image_file_system(image_handle)
        .map_err(|_| error_message("Nagi Loader: filesystem unavailable"))?;
    let mut root = filesystem
        .open_volume()
        .map_err(|_| error_message("Nagi Loader: volume unavailable"))?;
    let handle = root
        .open(
            cstr16!("\\EFI\\NAGI\\KERNEL.ELF"),
            FileMode::Read,
            FileAttribute::empty(),
        )
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

fn read_init(image_handle: Handle) -> Result<InitImageInfo, &'static str> {
    let mut filesystem = boot::get_image_file_system(image_handle)
        .map_err(|_| error_message("Nagi Loader: filesystem unavailable"))?;
    let mut root = filesystem
        .open_volume()
        .map_err(|_| error_message("Nagi Loader: volume unavailable"))?;
    let handle = root
        .open(
            cstr16!("\\EFI\\NAGI\\INIT.ELF"),
            FileMode::Read,
            FileAttribute::empty(),
        )
        .map_err(|_| error_message("Nagi Loader: INIT.ELF not found"))?;
    let mut file = match handle
        .into_type()
        .map_err(|_| error_message("Nagi Loader: init file type failed"))?
    {
        FileType::Regular(file) => file,
        FileType::Dir(_) => return Err(error_message("Nagi Loader: INIT.ELF is a directory")),
    };
    let buffer = unsafe {
        core::slice::from_raw_parts_mut(
            ptr::addr_of_mut!(INIT_IMAGE).cast::<u8>(),
            MAX_INIT_IMAGE_SIZE,
        )
    };
    let size = read_bounded_regular_file(
        &mut file,
        buffer,
        "Nagi Loader: init size failed",
        "Nagi Loader: init size is unsupported",
        "Nagi Loader: init rewind failed",
        "Nagi Loader: init read failed",
        "Nagi Loader: short init read",
    )?;
    let pages = init_image_page_count(size)
        .ok_or(error_message("Nagi Loader: init page count overflow"))?;
    let allocation = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .map_err(|_| error_message("Nagi Loader: init allocation failed"))?;
    unsafe {
        ptr::write_bytes(allocation.as_ptr(), 0, pages * PAGE_SIZE as usize);
        ptr::copy_nonoverlapping(buffer.as_ptr(), allocation.as_ptr(), size);
    }
    Ok(InitImageInfo {
        address: allocation.as_ptr() as u64,
        size: size as u64,
    })
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
    for segment in &plan.segments[..plan.segment_count] {
        let pages = segment
            .memory_size
            .checked_add(PAGE_SIZE - 1)
            .ok_or(error_message("Nagi Loader: segment size overflow"))?
            / PAGE_SIZE;
        if pages == 0 {
            return Err(error_message("Nagi Loader: empty load segment"));
        }
        let allocation = boot::allocate_pages(
            AllocateType::Address(segment.physical_address),
            MemoryType::LOADER_DATA,
            usize::try_from(pages)
                .map_err(|_| error_message("Nagi Loader: segment page count overflow"))?,
        )
        .map_err(|_| error_message("Nagi Loader: segment allocation failed"))?;
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
