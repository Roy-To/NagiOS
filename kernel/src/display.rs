use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use nagi_abi::{DisplayInfo, SURFACE_BYTES, SURFACE_HEIGHT, SURFACE_WIDTH};
use nagi_bootinfo::FramebufferInfo;

use crate::memory::PAGE_SIZE;

pub const SURFACE_PAGE_COUNT: usize = SURFACE_BYTES.div_ceil(PAGE_SIZE as usize);
pub const USER_SURFACE_BASE: u64 = crate::user_elf::USER_IMAGE_LIMIT + 0x0060_0000;
const PIXEL_BYTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScanoutViewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

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
    let viewport = scanout_viewport(framebuffer.width, framebuffer.height)
        .ok_or(DisplayError::InvalidFramebuffer)?;
    let source = USER_SURFACE_BASE as *const u32;
    let destination = framebuffer.address as *mut u32;
    let destination_stride = framebuffer.pixels_per_scanline as usize;

    // Clear only the letterbox bars. A full-size viewport overwrites all UEFI
    // pixels when the scaled logical surface is copied below.
    if viewport.x != 0
        || viewport.y != 0
        || viewport.width != framebuffer.width
        || viewport.height != framebuffer.height
    {
        let right = viewport.x + viewport.width;
        let bottom = viewport.y + viewport.height;
        let opaque_black = u32::from_le_bytes([0, 0, 0, 255]);
        for y in 0..framebuffer.height {
            for x in 0..framebuffer.width {
                if x < viewport.x || x >= right || y < viewport.y || y >= bottom {
                    let destination_pixel =
                        unsafe { destination.add(y as usize * destination_stride + x as usize) };
                    unsafe { destination_pixel.write_volatile(opaque_black) };
                }
            }
        }
    }

    for y in 0..viewport.height {
        let source_y = scale_coordinate(y, SURFACE_HEIGHT, viewport.height) as usize;
        for x in 0..viewport.width {
            let source_x = scale_coordinate(x, SURFACE_WIDTH, viewport.width) as usize;
            let source_pixel = unsafe { source.add(source_y * SURFACE_WIDTH as usize + source_x) };
            let destination_x = (viewport.x + x) as usize;
            let destination_y = (viewport.y + y) as usize;
            let destination_pixel =
                unsafe { destination.add(destination_y * destination_stride + destination_x) };
            let pixel = unsafe { source_pixel.read_volatile() };
            unsafe {
                destination_pixel.write_volatile(convert_pixel(pixel, framebuffer.pixel_format))
            };
        }
    }
    Ok(())
}

fn convert_pixel(pixel: u32, pixel_format: u32) -> u32 {
    if pixel_format == 1 {
        (pixel & 0xff00_ff00) | ((pixel & 0x0000_00ff) << 16) | ((pixel & 0x00ff_0000) >> 16)
    } else {
        pixel
    }
}

fn scanout_viewport(framebuffer_width: u32, framebuffer_height: u32) -> Option<ScanoutViewport> {
    if framebuffer_width == 0 || framebuffer_height == 0 {
        return None;
    }

    let framebuffer_width = u64::from(framebuffer_width);
    let framebuffer_height = u64::from(framebuffer_height);
    let surface_width = u64::from(SURFACE_WIDTH);
    let surface_height = u64::from(SURFACE_HEIGHT);

    let (width, height) =
        if framebuffer_width * surface_height <= framebuffer_height * surface_width {
            (
                framebuffer_width,
                (framebuffer_width * surface_height / surface_width).max(1),
            )
        } else {
            (
                (framebuffer_height * surface_width / surface_height).max(1),
                framebuffer_height,
            )
        };

    Some(ScanoutViewport {
        x: ((framebuffer_width - width) / 2) as u32,
        y: ((framebuffer_height - height) / 2) as u32,
        width: width as u32,
        height: height as u32,
    })
}

fn scale_coordinate(output_coordinate: u32, source_size: u32, output_size: u32) -> u32 {
    ((u64::from(output_coordinate) * u64::from(source_size)) / u64::from(output_size)) as u32
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
    use super::{
        convert_pixel, make_capability, scale_coordinate, scanout_viewport, validate_framebuffer,
        DisplayError, ScanoutViewport,
    };
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

    #[test]
    fn scales_the_logical_surface_to_the_full_scanout_without_stretching() {
        assert_eq!(
            scanout_viewport(1280, 800),
            Some(ScanoutViewport {
                x: 0,
                y: 0,
                width: 1280,
                height: 800,
            })
        );
        assert_eq!(
            scanout_viewport(1024, 768),
            Some(ScanoutViewport {
                x: 0,
                y: 64,
                width: 1024,
                height: 640,
            })
        );
        assert_eq!(scanout_viewport(0, 800), None);
        assert_eq!(scale_coordinate(1279, 320, 1280), 319);
        assert_eq!(scale_coordinate(799, 200, 800), 199);
        assert_eq!(convert_pixel(0xff03_0201, 0), 0xff03_0201);
        assert_eq!(convert_pixel(0xff03_0201, 1), 0xff01_0203);
    }
}
