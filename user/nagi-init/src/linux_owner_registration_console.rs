//! PID 1's development first-run display and exclusive keyboard input.

use super::select_development_keyboard;
use crate::login_screen::{LinuxOwnerRegistration, LinuxRegistrationOutcome};
use libnagi::credential::AccountRecord;
use libnagi::{InputEvent, SURFACE_HEIGHT, SURFACE_WIDTH};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

const O_NOFOLLOW: i32 = 0x20000;
const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOGET_FSCREENINFO: u64 = 0x4602;
const EVIOCGRAB: u64 = 0x40044590;

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

#[repr(C)]
#[derive(Default, PartialEq, Eq)]
struct Bitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default)]
struct FbVariable {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Bitfield,
    green: Bitfield,
    blue: Bitfield,
    alpha: Bitfield,
    nonstandard: u32,
    timing: [u32; 13],
    rotate: u32,
    remainder: [u32; 5],
}

#[repr(C)]
#[derive(Default)]
struct FbFixed {
    id: [u8; 16],
    memory_start: u64,
    memory_length: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: u64,
    mmio_length: u32,
    accelerator: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

struct Framebuffer {
    device: File,
    stride: u64,
    width: usize,
    height: usize,
    swap_red_blue: bool,
}

impl Framebuffer {
    fn open() -> io::Result<Self> {
        let device = OpenOptions::new()
            .write(true)
            .custom_flags(O_NOFOLLOW)
            .open("/dev/fb0")?;
        let metadata = device.metadata()?;
        if !metadata.file_type().is_char_device() || metadata.uid() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "untrusted framebuffer",
            ));
        }
        let mut variable = FbVariable::default();
        let mut fixed = FbFixed::default();
        // x86-64 fbdev ABI. Both pointers reference writable buffers of the
        // complete kernel struct size.
        const {
            assert!(size_of::<FbVariable>() == 160 && size_of::<FbFixed>() == 80);
        }
        unsafe {
            if ioctl(device.as_raw_fd(), FBIOGET_VSCREENINFO, &mut variable) < 0
                || ioctl(device.as_raw_fd(), FBIOGET_FSCREENINFO, &mut fixed) < 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        let channel = |offset| Bitfield {
            offset,
            length: 8,
            msb_right: 0,
        };
        let stride = u64::from(fixed.line_length);
        let swap_red_blue = variable.red == channel(16) && variable.blue == channel(0);
        let rgba = variable.red == channel(0) && variable.blue == channel(16);
        if variable.xres < SURFACE_WIDTH
            || variable.yres < SURFACE_HEIGHT
            || variable.xres > 4096
            || variable.yres > 2160
            || variable.xres_virtual < variable.xres
            || variable.yres_virtual < variable.yres
            || variable.xoffset != 0
            || variable.yoffset != 0
            || variable.bits_per_pixel != 32
            || variable.grayscale != 0
            || variable.nonstandard != 0
            || variable.rotate != 0
            || variable.green != channel(8)
            || (!rgba && !swap_red_blue)
            || (variable.alpha.length != 0 && variable.alpha != channel(24))
            || fixed.kind != 0
            || fixed.visual != 2
            || stride < u64::from(variable.xres) * 4
            || stride * u64::from(variable.yres) > u64::from(fixed.memory_length)
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported fbdev layout",
            ));
        }
        Ok(Self {
            device,
            stride,
            width: variable.xres as usize,
            height: variable.yres as usize,
            swap_red_blue,
        })
    }

    fn present(&mut self, pixels: &[u32]) -> io::Result<()> {
        let encode = |pixel: u32| {
            let mut channels = pixel.to_le_bytes();
            if self.swap_red_blue {
                channels.swap(0, 2);
            }
            channels
        };
        let width = SURFACE_WIDTH as usize;
        let height = SURFACE_HEIGHT as usize;
        let left = (self.width - width) / 2;
        let top = (self.height - height) / 2;
        let background = encode(pixels[0]).repeat(self.width);
        let mut row = background.clone();
        for y in 0..self.height {
            let bytes = if (top..top + height).contains(&y) {
                let source = &pixels[(y - top) * width..(y - top + 1) * width];
                for (dest, &pixel) in row[left * 4..(left + width) * 4]
                    .chunks_exact_mut(4)
                    .zip(source)
                {
                    dest.copy_from_slice(&encode(pixel));
                }
                &row
            } else {
                &background
            };
            self.device.seek(SeekFrom::Start(y as u64 * self.stride))?;
            self.device.write_all(bytes)?;
        }
        self.device.flush()
    }
}

#[derive(Debug)]
pub(super) enum RegistrationConsoleError<E> {
    Device,
    Io(io::Error),
    CredentialUnavailable,
    Save(E),
}

pub(super) fn run<SaveAccount, E>(
    mut screen: LinuxOwnerRegistration<SaveAccount>,
) -> Result<(), RegistrationConsoleError<E>>
where
    SaveAccount: FnOnce(AccountRecord) -> Result<(), E>,
{
    use RegistrationConsoleError::*;
    let (_, mut keyboard) = select_development_keyboard().map_err(|_| Device)?;
    // The kernel accepts the scalar argument for EVIOCGRAB. Ownership remains
    // on this retained descriptor and ends when the descriptor closes.
    if unsafe { ioctl(keyboard.as_raw_fd(), EVIOCGRAB, 1i32) } < 0 {
        return Err(Io(io::Error::last_os_error()));
    }
    let mut framebuffer = Framebuffer::open().map_err(Io)?;
    let mut pixels = vec![0; (SURFACE_WIDTH * SURFACE_HEIGHT) as usize];
    let mut redraw = |screen: &LinuxOwnerRegistration<SaveAccount>| {
        screen.render(&mut crate::ui::Painter::new(&mut pixels));
        framebuffer.present(&pixels).map_err(Io)
    };
    redraw(&screen)?;
    super::log_pid1_stderr(format_args!(
        "NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT"
    ));
    loop {
        // Linux input_event has a 16-byte timestamp followed by type, code and value.
        let mut data = [0u8; 24];
        keyboard.read_exact(&mut data).map_err(Io)?;
        let event_type = u16::from_le_bytes([data[16], data[17]]);
        let code = u16::from_le_bytes([data[18], data[19]]);
        let value = i32::from_le_bytes(data[20..24].try_into().unwrap());
        if event_type == 0 && code == 3 {
            return Err(Io(io::Error::other("keyboard events were lost")));
        }
        if event_type != libnagi::INPUT_EVENT_KEY || value != 1 {
            continue;
        }
        match screen.handle_event(InputEvent {
            event_type,
            code,
            value,
        }) {
            LinuxRegistrationOutcome::Ignored => {}
            LinuxRegistrationOutcome::Redraw => redraw(&screen)?,
            LinuxRegistrationOutcome::CredentialUnavailable => {
                redraw(&screen)?;
                return Err(CredentialUnavailable);
            }
            LinuxRegistrationOutcome::Submitted(result) => {
                redraw(&screen)?;
                return result.map_err(Save);
            }
        }
    }
}
