use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use nagi_abi::{DisplayInfo, SURFACE_BYTES, SURFACE_HEIGHT, SURFACE_WIDTH};
use nagi_bootinfo::FramebufferInfo;

use crate::memory::PAGE_SIZE;

pub const SURFACE_PAGE_COUNT: usize = SURFACE_BYTES.div_ceil(PAGE_SIZE as usize);
pub const USER_SURFACE_BASE: u64 = crate::user_elf::USER_IMAGE_BASE + 0x0060_0000;
const PIXEL_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayError {
    InvalidFramebuffer,
    UnsupportedPixelFormat,
    NotInitialized,
}

#[repr(C, align(4096))]
pub(crate) struct SurfaceVmo {
    pub(crate) bytes: [u8; SURFACE_PAGE_COUNT * PAGE_SIZE as usize],
}

impl SurfaceVmo {
    const fn empty() -> Self {
        Self {
            bytes: [0; SURFACE_PAGE_COUNT * PAGE_SIZE as usize],
        }
    }
}

static mut SURFACE_VMO: SurfaceVmo = SurfaceVmo::empty();
static mut FRAMEBUFFER: Option<FramebufferInfo> = None;
static DISPLAY_CAPABILITY: AtomicU64 = AtomicU64::new(0);

pub(crate) fn clear_surface() {
    unsafe {
        ptr::addr_of_mut!(SURFACE_VMO.bytes)
            .cast::<u8>()
            .write_bytes(0, SURFACE_PAGE_COUNT * PAGE_SIZE as usize);
    }
}

pub fn initialize(framebuffer: FramebufferInfo) -> Result<(), DisplayError> {
    validate_framebuffer(framebuffer)?;
    unsafe {
        ptr::write_volatile(&raw mut FRAMEBUFFER, Some(framebuffer));
    }
    DISPLAY_CAPABILITY.store(make_capability(framebuffer), Ordering::Release);
    Ok(())
}

pub fn user_capability() -> u64 {
    DISPLAY_CAPABILITY.load(Ordering::Acquire)
}

pub fn capability_matches(capability: u64) -> bool {
    capability != 0 && capability == user_capability()
}

pub fn surface_page_address(index: usize) -> Option<u64> {
    if index >= SURFACE_PAGE_COUNT {
        return None;
    }
    Some(unsafe { (&raw const SURFACE_VMO.bytes[index * PAGE_SIZE as usize]) as u64 })
}

pub fn info() -> Option<DisplayInfo> {
    let framebuffer = unsafe { ptr::addr_of!(FRAMEBUFFER).read_volatile()? };
    Some(DisplayInfo {
        surface_address: USER_SURFACE_BASE,
        surface_bytes: SURFACE_BYTES as u32,
        width: SURFACE_WIDTH,
        height: SURFACE_HEIGHT,
        stride: SURFACE_WIDTH * PIXEL_BYTES as u32,
        pixel_format: nagi_abi::PIXEL_FORMAT_RGBA8888,
    })
    .filter(|_| framebuffer.address != 0)
}

pub fn present_surface(capability: u64) -> Result<(), DisplayError> {
    if !capability_matches(capability) {
        return Err(DisplayError::NotInitialized);
    }
    let framebuffer = unsafe { ptr::addr_of!(FRAMEBUFFER).read_volatile() }
        .ok_or(DisplayError::NotInitialized)?;
    let source = USER_SURFACE_BASE as *const u8;
    let destination = framebuffer.address as *mut u8;
    let width = SURFACE_WIDTH.min(framebuffer.width);
    let height = SURFACE_HEIGHT.min(framebuffer.height);
    let destination_stride = framebuffer.pixels_per_scanline as usize * PIXEL_BYTES;
    let source_stride = SURFACE_WIDTH as usize * PIXEL_BYTES;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let source_pixel = unsafe { source.add(y * source_stride + x * PIXEL_BYTES) };
            let destination_pixel =
                unsafe { destination.add(y * destination_stride + x * PIXEL_BYTES) };
            let red = unsafe { source_pixel.read_volatile() };
            let green = unsafe { source_pixel.add(1).read_volatile() };
            let blue = unsafe { source_pixel.add(2).read_volatile() };
            let alpha = unsafe { source_pixel.add(3).read_volatile() };
            let (first, third) = if framebuffer.pixel_format == 1 {
                (blue, red)
            } else {
                (red, blue)
            };
            unsafe {
                destination_pixel.write_volatile(first);
                destination_pixel.add(1).write_volatile(green);
                destination_pixel.add(2).write_volatile(third);
                destination_pixel.add(3).write_volatile(alpha);
            }
        }
    }
    Ok(())
}

fn validate_framebuffer(framebuffer: FramebufferInfo) -> Result<(), DisplayError> {
    if framebuffer.address == 0
        || framebuffer.width == 0
        || framebuffer.height == 0
        || framebuffer.pixels_per_scanline < framebuffer.width
        || framebuffer.pixel_format > 1
    {
        return Err(if framebuffer.pixel_format > 1 {
            DisplayError::UnsupportedPixelFormat
        } else {
            DisplayError::InvalidFramebuffer
        });
    }
    let bytes_per_row = u64::from(framebuffer.pixels_per_scanline) * PIXEL_BYTES as u64;
    let required = bytes_per_row
        .checked_mul(u64::from(framebuffer.height))
        .ok_or(DisplayError::InvalidFramebuffer)?;
    if framebuffer.byte_size < required {
        return Err(DisplayError::InvalidFramebuffer);
    }
    Ok(())
}

const fn make_capability(framebuffer: FramebufferInfo) -> u64 {
    let value = 0x4e41_4749_4449_5350_u64
        ^ framebuffer.address.rotate_left(13)
        ^ framebuffer.byte_size.rotate_right(7)
        ^ ((framebuffer.width as u64) << 32)
        ^ framebuffer.height as u64;
    if value == 0 {
        1
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{make_capability, validate_framebuffer, DisplayError};
    use nagi_bootinfo::FramebufferInfo;

    fn framebuffer() -> FramebufferInfo {
        FramebufferInfo {
            address: 0xe000_0000,
            byte_size: 1024 * 768 * 4,
            width: 1024,
            height: 768,
            pixels_per_scanline: 1024,
            pixel_format: 0,
        }
    }

    #[test]
    fn validates_a_realistic_scanout_description() {
        assert_eq!(validate_framebuffer(framebuffer()), Ok(()));
        assert_ne!(make_capability(framebuffer()), 0);
    }

    #[test]
    fn rejects_unsupported_pixel_formats_and_short_buffers() {
        let mut info = framebuffer();
        info.pixel_format = 2;
        assert_eq!(
            validate_framebuffer(info),
            Err(DisplayError::UnsupportedPixelFormat)
        );
        let mut info = framebuffer();
        info.byte_size = 1;
        assert_eq!(
            validate_framebuffer(info),
            Err(DisplayError::InvalidFramebuffer)
        );
    }
}
