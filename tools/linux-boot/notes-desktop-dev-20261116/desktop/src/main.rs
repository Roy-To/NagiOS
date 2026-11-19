use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_ulong};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
use libnagi::storage::{StorageError, BLOCK_SIZE, MAX_FILE_SIZE};
use libnagi::{
    InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE, INPUT_KEY_SPACE, INPUT_KEY_TAB,
    SURFACE_BYTES, SURFACE_HEIGHT, SURFACE_WIDTH,
};
use linux_evdev_decoder::LinuxEvdevTarget;
use linux_evdev_stream::{EvdevStream, StreamError, StreamItem};
use nagi_desktop_notes_integration_candidate::desktop::{
    new_linux_development_volume, Desktop, UserDataVolume,
};
use nagi_localization::Locale;
use notes_event_bridge_candidate::NotesEventBridge;
#[cfg(test)]
use notes_files_codec_candidate::InMemoryNoteStore;
use notes_files_codec_candidate::{NoteDocument, NoteStore, ObjectId, Timestamp, VfsNoteStore};

mod input_failure_guard;
mod input_reconnect;
mod input_recovery;
mod linux_input_reopen;
mod notification_stdio;
mod recovery_main_dispatch;
mod recovery_stdio;
mod recovery_ui_commands;

use input_failure_guard::guard_input_failure;
use input_recovery::{
    RecoveryHost, RecoveryNotice, RecoveryOwner, RecoveryPhase, RecoveryStart, RecoveryStep,
};
use linux_input_reopen::{LinuxGrantedReopen, LinuxInputGrant, ReopenAttemptError};
use recovery_main_dispatch::{
    dispatch_initial_grant, dispatch_reconnected_input_loss, next_recovery_wait,
    InitialGrantDispatch, ReconnectedInputLoss, ReconnectedInputLossDispatch,
};
use recovery_ui_commands::{RecoveryUiCommand, RecoveryUiCommandAdapter};

const NOTE_ID: ObjectId = ObjectId(0x4e41_4749_0000_0001);
const EXPECTED_TITLE: &str = "Desktop reboot persistence";
const EXPECTED_BODY: &str = "a s";
const INITIAL_BODY: &str = "";
const RECOVERY_UI_SMOKE: bool = cfg!(feature = "linux-recovery-ui-smoke");
#[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
const RECOVERY_FILL_MIN_BYTES: usize = 6 * 1024 * 1024;

const FBIOGET_VSCREENINFO: c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: c_ulong = 0x4602;
const FB_TYPE_PACKED_PIXELS: u32 = 0;
const FB_VISUAL_TRUECOLOR: u32 = 2;
const KEY_MAX: usize = 0x2ff;
const KEY_BITMAP_BYTES: usize = (KEY_MAX + 8) / 8;
const EVDEV_KEY_A: u16 = 30;
const EVDEV_KEY_S: u16 = 31;
const LINUX_O_NOFOLLOW: i32 = 0x0002_0000;

const fn linux_ioc(direction: u32, kind: u32, number: u32, size: usize) -> c_ulong {
    ((direction << 30) | ((size as u32) << 16) | (kind << 8) | number) as c_ulong
}

const EVIOCGBIT_KEY: c_ulong = linux_ioc(2, b'E' as u32, 0x20 + 1, KEY_BITMAP_BYTES);
const EVIOCGKEY: c_ulong = linux_ioc(2, b'E' as u32, 0x18, KEY_BITMAP_BYTES);
const EVIOCGVERSION: c_ulong = linux_ioc(2, b'E' as u32, 0x01, core::mem::size_of::<c_int>());

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbVarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FbFixScreenInfo {
    id: [c_char; 16],
    smem_start: c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FramebufferLayout {
    line_length: usize,
    pixel_format: FramebufferPixelFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FramebufferPixelFormat {
    Rgba8888,
    Bgra8888,
}

enum InputItem {
    Event(InputEvent),
    NeedsResync { tab_down: bool, enter_down: bool },
}

#[derive(Debug)]
struct InputReadFailure {
    message: String,
    partial_record_bytes: usize,
}

impl std::fmt::Display for InputReadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for InputReadFailure {}

enum InputLoopExit {
    Completed,
    InputLost {
        desktop: Desktop,
        error: Box<dyn Error>,
        partial_record_bytes: usize,
    },
}

enum RecoveryDecision {
    Saved(EvdevStream<File>),
    Discarded,
    InputLost(ReconnectedInputLoss),
}

#[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
fn fill_development_vfs_to_capacity(
    volume: &mut UserDataVolume,
) -> Result<(usize, usize), io::Error> {
    let payload = vec![0x5a; MAX_FILE_SIZE];
    let mut successful_files = 0usize;
    let mut successful_bytes = 0usize;
    let mut reached_capacity = false;

    for index in 0..48usize {
        let path = format!("/recovery-fill-{index:02}");
        let handle = volume.create_path(path.as_bytes()).map_err(|error| {
            io::Error::other(format!("create recovery filler {index} failed: {error:?}"))
        })?;
        match volume.write(handle, &payload) {
            Ok(()) => {
                successful_files += 1;
                successful_bytes = successful_bytes
                    .checked_add(payload.len())
                    .ok_or_else(|| io::Error::other("recovery filler byte count overflow"))?;
            }
            Err(StorageError::Capacity) => {
                // The oversized write is atomic and leaves the newly created
                // file empty. Use the same handle to consume the remaining
                // blocks; a failed MAX_FILE_SIZE write alone can leave enough
                // free space for the small Notes revision.
                let max_blocks = MAX_FILE_SIZE.div_ceil(BLOCK_SIZE);
                let mut low_blocks = 0usize;
                let mut high_blocks = max_blocks;
                while high_blocks - low_blocks > 1 {
                    let middle_blocks = low_blocks + (high_blocks - low_blocks) / 2;
                    let byte_len = middle_blocks
                        .checked_mul(BLOCK_SIZE)
                        .ok_or_else(|| io::Error::other("recovery filler size overflow"))?;
                    match volume.write(handle, &payload[..byte_len]) {
                        Ok(()) => low_blocks = middle_blocks,
                        Err(StorageError::Capacity) => high_blocks = middle_blocks,
                        Err(error) => {
                            return Err(io::Error::other(format!(
                                "recovery remainder write failed before capacity: {error:?}"
                            )));
                        }
                    }
                }
                let final_bytes = low_blocks
                    .checked_mul(BLOCK_SIZE)
                    .ok_or_else(|| io::Error::other("recovery filler byte count overflow"))?;
                if final_bytes > 0 {
                    successful_files += 1;
                    successful_bytes = successful_bytes
                        .checked_add(final_bytes)
                        .ok_or_else(|| io::Error::other("recovery filler byte count overflow"))?;
                }
                let next_len = high_blocks
                    .checked_mul(BLOCK_SIZE)
                    .ok_or_else(|| io::Error::other("recovery capacity probe overflow"))?;
                if !matches!(
                    volume.write(handle, &payload[..next_len]),
                    Err(StorageError::Capacity)
                ) {
                    return Err(io::Error::other(
                        "recovery filler did not prove the next block is unavailable",
                    ));
                }
                reached_capacity = true;
                break;
            }
            Err(error) => {
                return Err(io::Error::other(format!(
                    "recovery filler write failed before capacity: {error:?}"
                )));
            }
        }
    }

    if !reached_capacity || successful_bytes < RECOVERY_FILL_MIN_BYTES {
        return Err(io::Error::other(format!(
            "development VFS did not reach block capacity after sufficient writes: files={successful_files} bytes={successful_bytes} capacity={reached_capacity}"
        )));
    }
    volume.flush().map_err(|error| {
        io::Error::other(format!("flush full development VFS failed: {error:?}"))
    })?;
    Ok((successful_files, successful_bytes))
}

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

fn validate_framebuffer() -> Result<(File, FramebufferLayout), Box<dyn Error>> {
    let framebuffer = OpenOptions::new()
        .write(true)
        .custom_flags(LINUX_O_NOFOLLOW)
        .open("/dev/fb0")?;
    if !framebuffer.metadata()?.file_type().is_char_device() {
        return Err("/dev/fb0 is not a character device".into());
    }
    let mut variable = FbVarScreenInfo::default();
    let mut fixed = FbFixScreenInfo::default();
    if core::mem::size_of::<FbVarScreenInfo>() != 160
        || core::mem::size_of::<FbFixScreenInfo>() != 80
        || ioctl_into(&framebuffer, FBIOGET_VSCREENINFO, &mut variable).is_err()
        || ioctl_into(&framebuffer, FBIOGET_FSCREENINFO, &mut fixed).is_err()
    {
        return Err("fbdev geometry/format query failed".into());
    }
    let layout = match validate_framebuffer_layout(&variable, &fixed) {
        Ok(layout) => layout,
        Err(error) => {
            recovery_stdio::log_stderr(&format!(
                "NAGI_FBDEV_LAYOUT error={error} visible={}x{} virtual={}x{} offset={}x{} bpp={} red={}/{}/{} green={}/{}/{} blue={}/{}/{} alpha={}/{}/{} type={} visual={} stride={} memory={} rotate={}",
                variable.xres,
                variable.yres,
                variable.xres_virtual,
                variable.yres_virtual,
                variable.xoffset,
                variable.yoffset,
                variable.bits_per_pixel,
                variable.red.offset,
                variable.red.length,
                variable.red.msb_right,
                variable.green.offset,
                variable.green.length,
                variable.green.msb_right,
                variable.blue.offset,
                variable.blue.length,
                variable.blue.msb_right,
                variable.transp.offset,
                variable.transp.length,
                variable.transp.msb_right,
                fixed.kind,
                fixed.visual,
                fixed.line_length,
                fixed.smem_len,
                variable.rotate,
            ));
            return Err(error.into());
        }
    };
    Ok((framebuffer, layout))
}

fn ioctl_into<T>(file: &File, request: c_ulong, value: &mut T) -> io::Result<()> {
    // SAFETY: the request is a Linux framebuffer/evdev ioctl and `value` points to
    // the exact repr(C) userspace structure or fixed-size byte buffer for it.
    let result = unsafe { ioctl(file.as_raw_fd(), request, value as *mut T) };
    if ioctl_succeeded(result) {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn ioctl_succeeded(result: c_int) -> bool {
    result >= 0
}

fn validate_framebuffer_layout(
    variable: &FbVarScreenInfo,
    fixed: &FbFixScreenInfo,
) -> Result<FramebufferLayout, &'static str> {
    let channel = |field: FbBitfield, offset| {
        field.offset == offset && field.length == 8 && field.msb_right == 0
    };
    let alpha = variable.transp.length == 0 || channel(variable.transp, 24);
    let pixel_format = if channel(variable.red, 0)
        && channel(variable.green, 8)
        && channel(variable.blue, 16)
    {
        FramebufferPixelFormat::Rgba8888
    } else if channel(variable.red, 16) && channel(variable.green, 8) && channel(variable.blue, 0) {
        FramebufferPixelFormat::Bgra8888
    } else {
        return Err("fbdev uses an unsupported packed truecolor channel layout");
    };
    let row_bytes = usize::try_from(SURFACE_WIDTH)
        .ok()
        .and_then(|width| width.checked_mul(core::mem::size_of::<u32>()))
        .ok_or("framebuffer row size overflow")?;
    let line_length = usize::try_from(fixed.line_length).map_err(|_| "invalid fbdev stride")?;
    let required_bytes = u64::from(fixed.line_length)
        .checked_mul(u64::from(SURFACE_HEIGHT))
        .ok_or("framebuffer capacity calculation overflow")?;

    if variable.xres != SURFACE_WIDTH
        || variable.yres != SURFACE_HEIGHT
        || variable.xres_virtual < SURFACE_WIDTH
        || variable.yres_virtual < SURFACE_HEIGHT
        || variable.xoffset != 0
        || variable.yoffset != 0
        || variable.bits_per_pixel != 32
        || variable.grayscale != 0
        || variable.nonstd != 0
        || !alpha
        || fixed.kind != FB_TYPE_PACKED_PIXELS
        || fixed.visual != FB_VISUAL_TRUECOLOR
        || variable.rotate != 0
        || line_length < row_bytes
        || required_bytes > u64::from(fixed.smem_len)
    {
        return Err("fbdev does not match the supported Nagi 320x200 truecolor surface layouts");
    }
    Ok(FramebufferLayout {
        line_length,
        pixel_format,
    })
}

fn framebuffer_pixel(pixel: u32, format: FramebufferPixelFormat) -> u32 {
    match format {
        FramebufferPixelFormat::Rgba8888 => pixel,
        FramebufferPixelFormat::Bgra8888 => {
            let red = pixel & 0x0000_00ff;
            let blue = pixel & 0x00ff_0000;
            (pixel & 0xff00_ff00) | (red << 16) | (blue >> 16)
        }
    }
}

fn present(
    framebuffer: &mut File,
    layout: FramebufferLayout,
    desktop: &Desktop,
) -> Result<(), Box<dyn Error>> {
    let mut pixels = vec![0u32; SURFACE_BYTES / core::mem::size_of::<u32>()];
    desktop.render(&mut pixels);
    let width = SURFACE_WIDTH as usize;
    let row_bytes = width * core::mem::size_of::<u32>();
    let mut row_buffer = Vec::with_capacity(row_bytes);
    for row in 0..SURFACE_HEIGHT as usize {
        let start = row * width;
        let offset = row
            .checked_mul(layout.line_length)
            .ok_or("framebuffer row offset overflow")?;
        let row_pixels = &pixels[start..start + width];
        row_buffer.clear();
        for pixel in row_pixels {
            row_buffer
                .extend_from_slice(&framebuffer_pixel(*pixel, layout.pixel_format).to_ne_bytes());
        }
        framebuffer.seek(SeekFrom::Start(offset as u64))?;
        framebuffer.write_all(&row_buffer)?;
    }
    framebuffer.flush()?;
    Ok(())
}

fn key_bit(bitmap: &[u8; KEY_BITMAP_BYTES], key: u16) -> bool {
    let index = usize::from(key);
    bitmap[index / 8] & (1 << (index % 8)) != 0
}

fn validate_keyboard_bitmap(bitmap: &[u8; KEY_BITMAP_BYTES]) -> Result<(), &'static str> {
    if ![
        INPUT_KEY_TAB,
        INPUT_KEY_ENTER,
        EVDEV_KEY_A,
        EVDEV_KEY_S,
        INPUT_KEY_SPACE,
    ]
    .into_iter()
    .all(|key| key_bit(bitmap, key))
    {
        return Err("selected evdev source lacks keys required by the Notes route and editor");
    }
    Ok(())
}

fn validate_keyboard(device: &File) -> Result<(), Box<dyn Error>> {
    let mut version: c_int = 0;
    ioctl_into(device, EVIOCGVERSION, &mut version)?;
    if version <= 0 {
        return Err("selected evdev source reported an invalid ABI version".into());
    }
    let mut bitmap = [0u8; KEY_BITMAP_BYTES];
    ioctl_into(device, EVIOCGBIT_KEY, &mut bitmap)?;
    validate_keyboard_bitmap(&bitmap)?;
    Ok(())
}

fn open_keyboard(path: &Path) -> Result<File, Box<dyn Error>> {
    let device = OpenOptions::new()
        .read(true)
        .custom_flags(LINUX_O_NOFOLLOW)
        .open(path)?;
    if !device.metadata()?.file_type().is_char_device() {
        return Err("selected evdev source is not a character device".into());
    }
    validate_keyboard(&device)?;
    Ok(device)
}

fn read_key_state(device: &File) -> Result<[u8; KEY_BITMAP_BYTES], Box<dyn Error>> {
    let mut bitmap = [0u8; KEY_BITMAP_BYTES];
    ioctl_into(device, EVIOCGKEY, &mut bitmap)?;
    Ok(bitmap)
}

fn next_input_item<R: Read>(
    stream: &mut EvdevStream<R>,
    read_key_state: impl FnOnce() -> Result<[u8; KEY_BITMAP_BYTES], Box<dyn Error>>,
) -> Result<InputItem, Box<dyn Error>> {
    match stream.next_item() {
        Ok(StreamItem::Event(event)) => Ok(InputItem::Event(event)),
        Ok(StreamItem::NeedsResync) => {
            let key_state = read_key_state().map_err(|error| {
                Box::new(InputReadFailure {
                    message: format!("evdev key-state resync failed: {error}"),
                    partial_record_bytes: stream.buffered_bytes(),
                }) as Box<dyn Error>
            })?;
            if !stream.reset_after_resync() {
                return Err(Box::new(InputReadFailure {
                    message: "evdev parser was not waiting for resynchronization".to_owned(),
                    partial_record_bytes: stream.buffered_bytes(),
                }));
            }
            Ok(InputItem::NeedsResync {
                tab_down: key_bit(&key_state, INPUT_KEY_TAB),
                enter_down: key_bit(&key_state, INPUT_KEY_ENTER),
            })
        }
        Ok(StreamItem::WouldBlock { buffered_bytes }) => Err(Box::new(InputReadFailure {
            message: "blocking evdev source returned WouldBlock".to_owned(),
            partial_record_bytes: buffered_bytes,
        })),
        Ok(StreamItem::Eof) => Err(Box::new(InputReadFailure {
            message: "evdev source closed".to_owned(),
            partial_record_bytes: 0,
        })),
        Err(StreamError::TruncatedRecord { received, .. }) => Err(Box::new(InputReadFailure {
            message: "evdev record was truncated".to_owned(),
            partial_record_bytes: received,
        })),
        Err(StreamError::Decode(error)) => Err(Box::new(InputReadFailure {
            message: format!("evdev record was invalid: {error:?}"),
            partial_record_bytes: 0,
        })),
        Err(StreamError::Io(_)) => Err(Box::new(InputReadFailure {
            message: "evdev read failed".to_owned(),
            partial_record_bytes: stream.buffered_bytes(),
        })),
    }
}

fn preflight_before_storage<D, V, E>(
    preflight: impl FnOnce() -> Result<D, E>,
    open_storage: impl FnOnce() -> Result<V, E>,
) -> Result<(D, V), E> {
    let devices = preflight()?;
    let volume = open_storage()?;
    Ok((devices, volume))
}

fn ensure_initial_note(store: &dyn NoteStore) -> Result<(), Box<dyn Error>> {
    match store.load(NOTE_ID)? {
        None => {
            let initial = NoteDocument::new(NOTE_ID, EXPECTED_TITLE, Timestamp(1));
            let saved = store.commit(&initial, 0)?;
            if saved.revision != 1 || saved.search_text() != INITIAL_BODY {
                return Err("initial note must be empty revision 1".into());
            }
        }
        Some(existing)
            if existing.revision == 1
                && existing.title == EXPECTED_TITLE
                && existing.search_text() == INITIAL_BODY
                && !existing.deleted =>
        {
            // An earlier save launch can fail after creating the empty fixture
            // but before reaching the normal input loop. Resume that exact
            // state; never replace a later or different note.
        }
        Some(_) => return Err("save phase requires an unused or untouched Notes identity".into()),
    }
    Ok(())
}

/// Own the same Desktop used by the real input path. An input failure transfers
/// that owner to the recovery lifecycle instead of returning and dropping it.
/// Tests inject only input and presentation boundaries; event dispatch and
/// Desktop ownership remain shared with production.
fn run_desktop_input_loop<I, P>(
    mut desktop: Desktop,
    volume: &mut UserDataVolume,
    store: &dyn NoteStore,
    phase: &str,
    mut next_input: I,
    mut present_desktop: P,
) -> Result<InputLoopExit, Box<dyn Error>>
where
    I: FnMut() -> Result<InputItem, Box<dyn Error>>,
    P: FnMut(&Desktop) -> Result<(), Box<dyn Error>>,
{
    present_desktop(&desktop)?;
    println!(
        "NAGI_NOTES_DESKTOP_INPUT_READY phase={phase} route=normal-desktop-handle-event framebuffer=fb0 store=nagi-vfs"
    );

    let mut tabs = 0usize;
    let mut notes_activated = false;
    loop {
        let item = match next_input() {
            Ok(item) => item,
            Err(error) => {
                let partial_record_bytes = error
                    .downcast_ref::<InputReadFailure>()
                    .map_or(0, |failure| failure.partial_record_bytes);
                return Ok(InputLoopExit::InputLost {
                    desktop,
                    error,
                    partial_record_bytes,
                });
            }
        };
        let event = match item {
            InputItem::Event(event) => event,
            InputItem::NeedsResync {
                tab_down,
                enter_down,
            } => {
                tabs = 0;
                recovery_stdio::log_stderr(&format!(
                    "NAGI_NOTES_INPUT_RESYNC tab_down={tab_down} enter_down={enter_down} tab_sequence_reset=1"
                ));
                continue;
            }
        };
        if event.event_type != INPUT_EVENT_KEY {
            continue;
        }
        let redraw = desktop.handle_event(event, volume);
        if redraw {
            present_desktop(&desktop)?;
        }
        if event.value != 1 {
            continue;
        }

        if !notes_activated {
            if event.code == INPUT_KEY_TAB {
                tabs += 1;
                continue;
            }
            if event.code == INPUT_KEY_ENTER && tabs >= 3 {
                if !desktop.linux_development_notes_window_active() {
                    return Err("normal Tab/Enter route did not activate the Notes window".into());
                }
                notes_activated = true;
                if phase == "read" {
                    let note = store.load(NOTE_ID)?.ok_or("reopened note disappeared")?;
                    if note.revision != 2 || note.search_text() != EXPECTED_BODY {
                        return Err(
                            "Notes UI did not retain the reopened persistent document".into()
                        );
                    }
                    present_desktop(&desktop)?;
                    println!(
                        "NAGI_NOTES_DESKTOP_READ_PASS id={} revision={} body={:?} normal_input=PASS fbdev=PASS nagi_vfs=PASS",
                        NOTE_ID.0,
                        note.revision,
                        note.search_text()
                    );
                    return Ok(InputLoopExit::Completed);
                }
                continue;
            }
            continue;
        }

        if phase == "save" {
            let note = desktop
                .current_notes_document()
                .ok_or("Desktop lost its active Notes document")?;
            if note.revision == 2 && note.search_text() == EXPECTED_BODY {
                let persisted = store.load(NOTE_ID)?.ok_or("VFS store lost saved note")?;
                if persisted != *note {
                    return Err("persisted VFS note does not match the Desktop document".into());
                }
                println!(
                    "NAGI_NOTES_DESKTOP_SAVE_PASS id={} revision={} body={:?} normal_input=PASS fbdev=PASS nagi_vfs=PASS",
                    NOTE_ID.0,
                    persisted.revision,
                    persisted.search_text()
                );
                return Ok(InputLoopExit::Completed);
            }
        }
    }
}

type SharedRecoveryDesktop = Arc<Mutex<Desktop>>;

fn lock_recovery_desktop(desktop: &SharedRecoveryDesktop) -> MutexGuard<'_, Desktop> {
    desktop
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct DesktopRecoveryHost<'a> {
    desktop: SharedRecoveryDesktop,
    framebuffer: &'a mut File,
    layout: FramebufferLayout,
    last_message: Option<&'static str>,
    last_hint: Option<&'static str>,
    last_cause: Option<&'static str>,
    last_diagnostic: Option<String>,
    #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
    presentation_failures_remaining: u8,
}

impl<'a> DesktopRecoveryHost<'a> {
    fn new(
        desktop: SharedRecoveryDesktop,
        framebuffer: &'a mut File,
        layout: FramebufferLayout,
    ) -> Self {
        Self {
            desktop,
            framebuffer,
            layout,
            last_message: None,
            last_hint: None,
            last_cause: None,
            last_diagnostic: None,
            #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
            presentation_failures_remaining: 0,
        }
    }

    #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
    fn inject_presentation_failures(&mut self, count: u8) {
        self.presentation_failures_remaining = count;
    }

    fn show_status(
        &mut self,
        message_id: &'static str,
        hint_id: Option<&'static str>,
    ) -> Result<(), String> {
        self.show_status_with_cause(message_id, hint_id, None)
    }

    fn show_status_with_cause(
        &mut self,
        message_id: &'static str,
        hint_id: Option<&'static str>,
        cause_id: Option<&'static str>,
    ) -> Result<(), String> {
        #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
        if self.presentation_failures_remaining > 0 {
            self.presentation_failures_remaining -= 1;
            recovery_stdio::log_stderr(
                "NAGI_NOTES_RECOVERY_NOTICE_FAILURE_INJECTED phase=recovery-ui-smoke source=framebuffer_status",
            );
            return Err("development recovery status presentation failure injection".to_owned());
        }
        let desktop = Arc::clone(&self.desktop);
        let mut desktop = lock_recovery_desktop(&desktop);
        desktop.set_notes_recovery_message_with_cause(Some(message_id), hint_id, cause_id);
        present(self.framebuffer, self.layout, &desktop).map_err(|error| error.to_string())?;
        self.last_message = Some(message_id);
        self.last_hint = hint_id;
        self.last_cause = cause_id;
        Ok(())
    }

    fn initial_failure_notice(&mut self, diagnostic: &str) -> Result<(), String> {
        self.log_diagnostic(diagnostic);
        self.show_status_with_cause(
            "desktop.notes.recovery.retry_failed",
            Some("desktop.notes.recovery.reconnect_lost_hint"),
            Some(cause_for_recovery_errors(None, Some(diagnostic), None)),
        )
    }

    fn log_diagnostic(&mut self, diagnostic: &str) {
        if self.last_diagnostic.as_deref() != Some(diagnostic) {
            recovery_stdio::log_stderr(&format!("NAGI_NOTES_RECOVERY {diagnostic}"));
            self.last_diagnostic = Some(diagnostic.to_owned());
        }
    }
}

impl RecoveryHost<SharedRecoveryDesktop> for DesktopRecoveryHost<'_> {
    type SaveError = String;
    type NotificationError = String;

    fn retry_save(&mut self, draft: &mut SharedRecoveryDesktop) -> Result<(), Self::SaveError> {
        let mut desktop = lock_recovery_desktop(draft);
        desktop.retry_notes_save()
    }

    fn notify(&mut self, notice: RecoveryNotice) -> Result<(), Self::NotificationError> {
        let (message_id, hint_id) = recovery_ui_commands::status_for_notice(&notice);
        let cause_id = cause_for_notice(&notice);
        self.log_diagnostic(&format!("notice={notice:?}"));
        if self.last_message == Some(message_id) && self.last_cause == cause_id {
            return Ok(());
        }
        self.show_status_with_cause(message_id, hint_id, cause_id)
    }
}

fn recovery_command_for_key(phase: RecoveryPhase, event: InputEvent) -> Option<RecoveryUiCommand> {
    if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
        return None;
    }
    // WaitingForInput intentionally has no keyboard command: recovery actions
    // are accepted only after LinuxGrantedReopen has returned an identity-
    // verified stream and run_recovery_controls is polling that stream.
    match (phase, event.code) {
        (RecoveryPhase::NotificationUnavailable, INPUT_KEY_ENTER) => {
            Some(RecoveryUiCommand::RetryNotification)
        }
        (RecoveryPhase::AwaitingUserChoice, INPUT_KEY_ENTER) => Some(RecoveryUiCommand::RetrySave),
        (RecoveryPhase::AwaitingUserChoice, INPUT_KEY_ESCAPE) => {
            Some(RecoveryUiCommand::BeginDiscard)
        }
        (RecoveryPhase::ConfirmingDiscard, INPUT_KEY_ENTER) => {
            Some(RecoveryUiCommand::ConfirmDiscard)
        }
        (RecoveryPhase::ConfirmingDiscard, INPUT_KEY_ESCAPE) => {
            Some(RecoveryUiCommand::CancelDiscard)
        }
        _ => None,
    }
}

fn next_reconnect_backoff(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(Duration::from_secs(30))
}

/// Convert internal diagnostics to short localized categories. Raw paths and
/// backend error strings stay in diagnostics and never reach the framebuffer.
fn cause_for_recovery_errors(
    input_error: Option<&str>,
    save_error: Option<&str>,
    reconnect_error: Option<&str>,
) -> &'static str {
    if let Some(error) = reconnect_error {
        let error = error.to_ascii_lowercase();
        if error.contains("identity")
            || error.contains("unique id")
            || error.contains("does not match")
            || error.contains("gate rejected")
        {
            return "desktop.notes.recovery.cause.identity";
        }
        return "desktop.notes.recovery.cause.device_unavailable";
    }
    if let Some(error) = save_error {
        let error = error.to_ascii_lowercase();
        if error.contains("capacity") || error.contains("directoryfull") {
            return "desktop.notes.recovery.cause.storage_full";
        }
        if error.contains("conflict") {
            return "desktop.notes.recovery.cause.save_conflict";
        }
        return "desktop.notes.recovery.cause.save_failed";
    }
    if let Some(error) = input_error {
        let error = error.to_ascii_lowercase();
        if error.contains("truncated") || error.contains("invalid") {
            return "desktop.notes.recovery.cause.input_invalid";
        }
        if error.contains("closed") || error.contains("disconnect") {
            return "desktop.notes.recovery.cause.input_closed";
        }
        if error.contains("resync") || error.contains("key-state") {
            return "desktop.notes.recovery.cause.input_resync";
        }
        return "desktop.notes.recovery.cause.input_read";
    }
    "desktop.notes.recovery.cause.recovery_failed"
}

fn cause_for_notice(notice: &RecoveryNotice) -> Option<&'static str> {
    match notice {
        RecoveryNotice::InitialInputSaveFailed {
            input_error,
            save_error,
        } => Some(cause_for_recovery_errors(
            Some(input_error),
            Some(save_error),
            None,
        )),
        RecoveryNotice::InputReconnected {
            input_error,
            initial_save_error,
            ..
        } => Some(cause_for_recovery_errors(
            Some(input_error),
            Some(initial_save_error),
            None,
        )),
        RecoveryNotice::InputReconnectUnavailable { reason, .. } => {
            Some(cause_for_recovery_errors(None, None, Some(reason)))
        }
        RecoveryNotice::RetryFailed(error) => {
            Some(cause_for_recovery_errors(None, Some(error), None))
        }
        RecoveryNotice::RetrySaved | RecoveryNotice::DraftDiscarded => None,
    }
}

fn run_recovery_controls(
    recovery: &mut LinuxGrantedReopen<SharedRecoveryDesktop>,
    mut reopened: linux_input_reopen::ReopenedGrantedInput,
    host: &mut DesktopRecoveryHost<'_>,
) -> RecoveryDecision {
    let resync_device = match reopened.stream.reader().try_clone() {
        Ok(device) => device,
        Err(error) => {
            recovery_stdio::log_stderr(&format!(
                "NAGI_NOTES_RECOVERY_INPUT_LOST resync_clone={error}"
            ));
            return RecoveryDecision::InputLost(ReconnectedInputLoss::from_read_error(
                format!("reopened evdev handle could not be cloned: {error}"),
                reopened.stream.buffered_bytes(),
            ));
        }
    };

    if recovery.owner().phase() == RecoveryPhase::NotificationUnavailable {
        let _ = host.show_status_with_cause(
            "desktop.notes.recovery.notice_retry",
            Some("desktop.notes.recovery.notice_retry_hint"),
            Some("desktop.notes.recovery.cause.notification_failed"),
        );
    }

    loop {
        let item = match next_input_item(&mut reopened.stream, || read_key_state(&resync_device)) {
            Ok(item) => item,
            Err(error) => {
                recovery_stdio::log_stderr(&format!(
                    "NAGI_NOTES_RECOVERY_INPUT_LOST error={error}"
                ));
                let partial_record_bytes = error
                    .as_ref()
                    .downcast_ref::<InputReadFailure>()
                    .map(|failure| failure.partial_record_bytes)
                    .unwrap_or_else(|| reopened.stream.buffered_bytes());
                return RecoveryDecision::InputLost(ReconnectedInputLoss::from_read_error(
                    error.to_string(),
                    partial_record_bytes,
                ));
            }
        };
        let InputItem::Event(event) = item else {
            continue;
        };
        let phase = recovery.owner().phase();
        let Some(command) = recovery_command_for_key(phase, event) else {
            continue;
        };
        let result = {
            let mut ui = RecoveryUiCommandAdapter::borrow(recovery.owner_mut());
            ui.apply(command, host)
        };
        let retry_failed = matches!(&result, Ok(RecoveryStep::RetryFailed { .. }));
        match result {
            Ok(step) => {
                recovery_stdio::log_stderr(&format!("NAGI_NOTES_RECOVERY_STEP step={step:?}"))
            }
            Err(error) => {
                recovery_stdio::log_stderr(&format!(
                    "NAGI_NOTES_RECOVERY_COMMAND_REJECTED error={error:?}"
                ));
                continue;
            }
        }
        match recovery.owner().phase() {
            RecoveryPhase::Saved => return RecoveryDecision::Saved(reopened.stream),
            RecoveryPhase::Discarded => return RecoveryDecision::Discarded,
            RecoveryPhase::ConfirmingDiscard => {
                let cause_id =
                    cause_for_recovery_errors(None, Some(recovery.owner().last_save_error()), None);
                let _ = host.show_status_with_cause(
                    "desktop.notes.recovery.confirm_discard",
                    Some("desktop.notes.recovery.confirm_discard_hint"),
                    Some(cause_id),
                );
            }
            RecoveryPhase::AwaitingUserChoice => {
                let cause_id =
                    cause_for_recovery_errors(None, Some(recovery.owner().last_save_error()), None);
                if retry_failed {
                    let _ = host.show_status_with_cause(
                        "desktop.notes.recovery.retry_failed",
                        Some("desktop.notes.recovery.retry_failed_hint"),
                        Some(cause_id),
                    );
                } else {
                    let _ = host.show_status_with_cause(
                        "desktop.notes.recovery.ready",
                        Some("desktop.notes.recovery.ready_hint"),
                        Some(cause_id),
                    );
                }
            }
            RecoveryPhase::NotificationUnavailable => {
                let _ = host.show_status_with_cause(
                    "desktop.notes.recovery.notice_retry",
                    Some("desktop.notes.recovery.notice_retry_hint"),
                    Some("desktop.notes.recovery.cause.notification_failed"),
                );
            }
            RecoveryPhase::WaitingForInput => {}
        }
    }
}

fn run(phase: &str, disk: &Path, event_path: &Path) -> Result<(), Box<dyn Error>> {
    if phase != "save" && phase != "read" {
        return Err("phase must be save or read".into());
    }
    let recovery_ui_smoke = RECOVERY_UI_SMOKE && phase == "save";
    let (((mut framebuffer, framebuffer_layout), event_device), (mut volume, formatted)) =
        preflight_before_storage(
            || -> Result<_, Box<dyn Error>> {
                Ok((validate_framebuffer()?, open_keyboard(event_path)?))
            },
            || {
                new_linux_development_volume(disk, phase == "save")
                    .map_err(|error| -> Box<dyn Error> { Box::new(error) })
            },
        )?;
    if phase == "read" && formatted {
        return Err("read phase unexpectedly formatted the persistent VFS".into());
    }

    // The Desktop and Notes adapter share one VFS instance and lock. This is
    // Nagi's existing VFS on the explicitly supplied development block device;
    // it does not use the host root or the Nagi-only SyscallBlockDevice ABI.
    let store = Arc::new(VfsNoteStore::new(volume.clone()));
    match phase {
        "save" => {
            ensure_initial_note(store.as_ref())?;
            #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
            if recovery_ui_smoke {
                let (files, bytes) = fill_development_vfs_to_capacity(&mut volume)?;
                recovery_stdio::log_stderr(&format!(
                    "NAGI_NOTES_RECOVERY_VFS_FULL phase=recovery-ui-smoke successful_files={files} successful_bytes={bytes} failed_write=Capacity flushed=1"
                ));
            }
        }
        "read" => {
            let note = store
                .load(NOTE_ID)?
                .ok_or("read phase note is missing from the persistent Nagi VFS")?;
            if note.revision != 2 || note.search_text() != EXPECTED_BODY {
                return Err(format!(
                    "persistent note mismatch revision={} body={:?}",
                    note.revision,
                    note.search_text()
                )
                .into());
            }
        }
        _ => unreachable!(),
    }

    let bridge = NotesEventBridge::new(Arc::clone(&store));
    let mut desktop = Desktop::new_with_notes(Locale::EnUs, bridge, NOTE_ID)?;
    let mut event_stream = EvdevStream::new(event_device, LinuxEvdevTarget::X86_64);
    let mut resync_device = event_stream.reader().try_clone()?;
    let mut active_grant = Some(LinuxInputGrant::capture(
        event_path.to_path_buf(),
        event_stream.reader(),
    ));
    let mut fail_recovery_input_after_s = false;

    loop {
        let exit = run_desktop_input_loop(
            desktop,
            &mut volume,
            store.as_ref(),
            phase,
            || {
                if recovery_ui_smoke && fail_recovery_input_after_s {
                    fail_recovery_input_after_s = false;
                    recovery_stdio::log_stderr(
                        "NAGI_NOTES_RECOVERY_INPUT_FAILURE_INJECTED phase=recovery-ui-smoke after=KEY_S",
                    );
                    return Err(Box::new(InputReadFailure {
                        message: "development recovery input-loss injection after KEY_S".into(),
                        partial_record_bytes: 0,
                    }));
                }
                let item = next_input_item(&mut event_stream, || read_key_state(&resync_device))?;
                if recovery_ui_smoke
                    && matches!(
                        &item,
                        InputItem::Event(event)
                            if event.event_type == INPUT_EVENT_KEY
                                && event.code == EVDEV_KEY_S
                                && event.value == 1
                    )
                {
                    fail_recovery_input_after_s = true;
                }
                Ok(item)
            },
            |desktop| present(&mut framebuffer, framebuffer_layout, desktop),
        )?;
        match exit {
            InputLoopExit::Completed => return Ok(()),
            InputLoopExit::InputLost {
                desktop: lost_desktop,
                error,
                partial_record_bytes,
            } => {
                let input_error_message = error.to_string();
                let shared_desktop = Arc::new(Mutex::new(lost_desktop));
                let mut recovery_host = DesktopRecoveryHost::new(
                    Arc::clone(&shared_desktop),
                    &mut framebuffer,
                    framebuffer_layout,
                );
                #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
                if recovery_ui_smoke {
                    recovery_host.inject_presentation_failures(3);
                }
                let failure = guard_input_failure(
                    Arc::clone(&shared_desktop),
                    error,
                    |owner| lock_recovery_desktop(owner).flush_notes_before_input_loss(),
                    |_, diagnostic| recovery_host.initial_failure_notice(diagnostic),
                );
                match RecoveryOwner::from_input_failure(failure) {
                    RecoveryStart::FlushSucceeded { owner, input_error } => {
                        if recovery_ui_smoke {
                            return Err(
                                "recovery UI smoke expected a capacity-limited save failure".into(),
                            );
                        }
                        drop(owner);
                        return Err(input_error);
                    }
                    RecoveryStart::Held(owner) => {
                        let owner = owner;
                        if !recovery_ui_smoke {
                            let cause_id = cause_for_recovery_errors(
                                Some(owner.input_error()),
                                Some(owner.last_save_error()),
                                None,
                            );
                            let _ = recovery_host.show_status_with_cause(
                                "desktop.notes.recovery.retry_failed",
                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                Some(cause_id),
                            );
                        }
                        if recovery_ui_smoke {
                            if owner.phase() != RecoveryPhase::WaitingForInput {
                                return Err("recovery owner did not retain the dirty draft".into());
                            }
                            let dirty = lock_recovery_desktop(&shared_desktop)
                                .current_notes_document()
                                .cloned()
                                .ok_or("recovery owner lost the active Notes draft")?;
                            let persisted = store
                                .load(NOTE_ID)?
                                .ok_or("initial Notes revision disappeared during recovery")?;
                            if dirty.revision != 1 || dirty.search_text() != "a s" {
                                return Err(format!(
                                    "recovery did not retain the dirty revision-one draft: revision={} body={:?}",
                                    dirty.revision,
                                    dirty.search_text()
                                )
                                .into());
                            }
                            if persisted.revision != 1 || persisted.search_text() != INITIAL_BODY {
                                return Err(format!(
                                    "input-loss flush changed the persisted note despite capacity failure: revision={} body={:?}",
                                    persisted.revision,
                                    persisted.search_text()
                                )
                                .into());
                            }
                        }
                        let mut recovery = match dispatch_initial_grant(
                            active_grant.take(),
                            partial_record_bytes,
                            owner,
                        ) {
                            InitialGrantDispatch::Granted(recovery) => recovery,
                            InitialGrantDispatch::MissingOriginalGrant(owner) => {
                                recovery_stdio::log_stderr(
                                    "NAGI_NOTES_RECOVERY_BLOCKED reason=missing_original_grant",
                                );
                                // Keep the dirty owner alive in this wait state. There is no
                                // grant or path to retry, so this branch cannot reopen input.
                                let _held_owner = owner;
                                loop {
                                    thread::sleep(Duration::from_secs(30));
                                }
                            }
                        };
                        if !recovery_ui_smoke {
                            let _ = recovery_host.show_status(
                                "desktop.notes.recovery.retry_failed",
                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                            );
                        }
                        let mut reconnect_delay = Duration::from_secs(1);
                        let mut notification_retry_attempts = 0u8;
                        // Preserve a failed initial-save notice rather than
                        // replacing it with a reconnect notice while the
                        // original payload is still waiting for delivery.
                        let mut reconnect_notice_reported =
                            recovery.owner().notification_retry_available();
                        let mut reconnect_status_attempted = false;
                        #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
                        let mut smoke_reopen_failures_remaining =
                            if recovery_ui_smoke { 7 } else { 0 };
                        #[cfg(all(feature = "linux-recovery-ui-smoke", target_os = "linux"))]
                        let mut smoke_notice_retry_succeeded = false;
                        let reopened_stream = 'reconnect: loop {
                            let reopened = loop {
                                let notification_retry = {
                                    let mut ui =
                                        RecoveryUiCommandAdapter::borrow(recovery.owner_mut());
                                    ui.retry_notification_if_due(Instant::now(), &mut recovery_host)
                                };
                                if let Some(result) = notification_retry {
                                    notification_retry_attempts =
                                        notification_retry_attempts.saturating_add(1);
                                    recovery_stdio::log_stderr(&format!(
                                    "NAGI_NOTES_RECOVERY_NOTICE_RETRY attempt={notification_retry_attempts} result={result:?}"
                                ));
                                    #[cfg(all(
                                        feature = "linux-recovery-ui-smoke",
                                        target_os = "linux"
                                    ))]
                                    if recovery_ui_smoke {
                                        let dirty = lock_recovery_desktop(&shared_desktop)
                                            .current_notes_document()
                                            .cloned()
                                            .ok_or(
                                                "periodic notice retry lost the held Notes draft",
                                            )?;
                                        let persisted = store.load(NOTE_ID)?.ok_or(
                                            "periodic notice retry lost the persisted note",
                                        )?;
                                        if dirty.revision != 1
                                            || dirty.search_text() != "a s"
                                            || persisted.revision != 1
                                            || persisted.search_text() != INITIAL_BODY
                                            || recovery.owner().phase()
                                                != RecoveryPhase::WaitingForInput
                                        {
                                            return Err(
                                            "notice retry did not retain the dirty draft while input was unavailable"
                                                .into(),
                                        );
                                        }
                                        let notification_error = match &result {
                                            RecoveryStep::NotificationRetried {
                                                notification_error,
                                            } => notification_error,
                                            _ => {
                                                return Err(
                                                "periodic scheduler emitted a non-notification step"
                                                    .into(),
                                            );
                                            }
                                        };
                                        if notification_error.is_none() {
                                            if recovery_host.last_message
                                            != Some("desktop.notes.recovery.retry_failed")
                                            || recovery_host.last_hint
                                                != Some(
                                                    "desktop.notes.recovery.reconnect_lost_hint",
                                                )
                                            || recovery_host.last_cause
                                                != Some("desktop.notes.recovery.cause.storage_full")
                                            || recovery.owner().notification_retry_available()
                                        {
                                            return Err(
                                                "successful periodic retry did not present the safe unsaved status"
                                                    .into(),
                                            );
                                        }
                                            recovery_stdio::log_stderr(
                                            "NAGI_NOTES_RECOVERY_HELD phase=recovery-ui-smoke flush=failed dirty_owner=1 dirty_revision=1 dirty_body=a_s persisted_revision=1 persisted_body=empty fbdev_notice=save_failed_draft_held waiting_hint=draft_remains_in_memory",
                                        );
                                            smoke_notice_retry_succeeded = true;
                                            recovery_stdio::log_stderr(&format!(
                                            "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt={notification_retry_attempts} pending_payload=cleared owner=held user_action_prompt=none result=delivered"
                                        ));
                                        } else {
                                            if !recovery.owner().notification_retry_available()
                                                || recovery_host.last_message.is_some()
                                            {
                                                return Err(
                                                "failed notice retry lost its pending payload or invented a screen action state"
                                                    .into(),
                                            );
                                            }
                                            recovery_stdio::log_stderr(&format!(
                                            "NAGI_NOTES_RECOVERY_NOTICE_RETRY_INPUT_WAIT phase=recovery-ui-smoke attempt={notification_retry_attempts} pending_payload=retained owner=held user_action_prompt=none result=failed"
                                        ));
                                        }
                                        recovery_stdio::log_stderr(
                                        "NAGI_NOTES_RECOVERY_OWNER_INPUT_WAIT phase=recovery-ui-smoke draft=retained input_action_prompt=none",
                                    );
                                    }
                                }

                                #[cfg(all(
                                    feature = "linux-recovery-ui-smoke",
                                    target_os = "linux"
                                ))]
                                if recovery_ui_smoke && smoke_reopen_failures_remaining > 0 {
                                    smoke_reopen_failures_remaining -= 1;
                                    let cause_id = cause_for_recovery_errors(
                                        None,
                                        None,
                                        Some("development reconnect probe: device unavailable"),
                                    );
                                    if recovery.owner().notification_retry_available() {
                                        recovery_stdio::log_stderr(&format!(
                                            "NAGI_NOTES_RECOVERY_STATUS_DEFERRED phase=recovery-ui-smoke cause={cause_id} pending_save_notice=1 owner=held"
                                        ));
                                    } else {
                                        recovery_host
                                            .show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            )
                                            .map_err(|_| {
                                                "input-loss status could not present its safe cause category"
                                            })?;
                                        if recovery_host.last_cause != Some(cause_id) {
                                            return Err(
                                                "input-loss status did not present its safe cause category"
                                                    .into(),
                                            );
                                        }
                                    }
                                    recovery_stdio::log_stderr(&format!(
                                    "NAGI_NOTES_RECOVERY_REOPEN_UNAVAILABLE_INJECTED phase=recovery-ui-smoke remaining={smoke_reopen_failures_remaining} cause={cause_id}"
                                ));
                                    let wait = next_recovery_wait(
                                        recovery.owner(),
                                        reconnect_delay,
                                        Instant::now(),
                                    );
                                    thread::sleep(wait);
                                    reconnect_delay = next_reconnect_backoff(reconnect_delay);
                                    continue;
                                }
                                #[cfg(all(
                                    feature = "linux-recovery-ui-smoke",
                                    target_os = "linux"
                                ))]
                                if recovery_ui_smoke && !smoke_notice_retry_succeeded {
                                    return Err(
                                    "input wait ended before the periodic notice retry was exercised"
                                        .into(),
                                );
                                }
                                match recovery.try_reopen_once(open_keyboard, &mut recovery_host) {
                                    Ok(reopened) => {
                                        if recovery_ui_smoke {
                                            recovery_stdio::log_stderr(&format!(
                                            "NAGI_NOTES_RECOVERY_RECONNECTED phase=recovery-ui-smoke parser_epoch={} identity=matched partial_record_bytes={}",
                                            reopened.parser_epoch,
                                            reopened.discarded_partial_record_bytes
                                        ));
                                        }
                                        break reopened;
                                    }
                                    Err(ReopenAttemptError::Unavailable { reason, .. }) => {
                                        if !reconnect_notice_reported {
                                            let _ = recovery
                                                .owner_mut()
                                                .recovery()
                                                .report_reconnect_unavailable(
                                                    event_path.to_string_lossy().into_owned(),
                                                    reason.clone(),
                                                    &mut recovery_host,
                                                );
                                            reconnect_notice_reported = true;
                                        }
                                        recovery_host.log_diagnostic(
                                            "reopen_unavailable=input_not_currently_reopenable",
                                        );
                                        if recovery.owner().notification_retry_available() {
                                            recovery_stdio::log_stderr(
                                                "NAGI_NOTES_RECOVERY_STATUS_DEFERRED pending_save_notice=1 owner=held",
                                            );
                                        } else if !reconnect_status_attempted {
                                            let cause_id = cause_for_recovery_errors(
                                                None,
                                                None,
                                                Some(&reason),
                                            );
                                            if recovery_host.last_message
                                                != Some("desktop.notes.recovery.reconnect_lost")
                                                || recovery_host.last_cause != Some(cause_id)
                                            {
                                                let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            }
                                            reconnect_status_attempted = true;
                                        } else {
                                            let cause_id = cause_for_recovery_errors(
                                                None,
                                                None,
                                                Some(&reason),
                                            );
                                            if recovery_host.last_cause != Some(cause_id) {
                                                let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            }
                                        }
                                        let wait = next_recovery_wait(
                                            recovery.owner(),
                                            reconnect_delay,
                                            Instant::now(),
                                        );
                                        thread::sleep(wait);
                                        reconnect_delay = next_reconnect_backoff(reconnect_delay);
                                    }
                                    Err(error) => {
                                        let reason = format!("identity_reopen_rejected={error:?}");
                                        if !reconnect_notice_reported {
                                            let _ = recovery
                                                .owner_mut()
                                                .recovery()
                                                .report_reconnect_unavailable(
                                                    event_path.to_string_lossy().into_owned(),
                                                    reason.clone(),
                                                    &mut recovery_host,
                                                );
                                            reconnect_notice_reported = true;
                                        }
                                        recovery_host.log_diagnostic(
                                            "reopen_rejected=identity_or_device_check_failed",
                                        );
                                        if recovery.owner().notification_retry_available() {
                                            recovery_stdio::log_stderr(
                                                "NAGI_NOTES_RECOVERY_STATUS_DEFERRED pending_save_notice=1 owner=held",
                                            );
                                        } else if !reconnect_status_attempted {
                                            let cause_id = cause_for_recovery_errors(
                                                None,
                                                None,
                                                Some(&reason),
                                            );
                                            if recovery_host.last_message
                                                != Some("desktop.notes.recovery.reconnect_lost")
                                                || recovery_host.last_cause != Some(cause_id)
                                            {
                                                let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            }
                                            reconnect_status_attempted = true;
                                        } else {
                                            let cause_id = cause_for_recovery_errors(
                                                None,
                                                None,
                                                Some(&reason),
                                            );
                                            if recovery_host.last_cause != Some(cause_id) {
                                                let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            }
                                        }
                                        let wait = next_recovery_wait(
                                            recovery.owner(),
                                            reconnect_delay,
                                            Instant::now(),
                                        );
                                        thread::sleep(wait);
                                        reconnect_delay = next_reconnect_backoff(reconnect_delay);
                                    }
                                }
                            };
                            match run_recovery_controls(&mut recovery, reopened, &mut recovery_host)
                            {
                                RecoveryDecision::Saved(reopened_stream) => {
                                    break 'reconnect reopened_stream;
                                }
                                RecoveryDecision::Discarded => {
                                    let discard_notice_presented = recovery_host.last_message
                                        == Some("desktop.notes.recovery.discarded");
                                    drop(recovery);
                                    drop(recovery_host);
                                    drop(shared_desktop);
                                    if recovery_ui_smoke {
                                        if !discard_notice_presented {
                                            return Err(
                                                "explicit discard completed without presenting its status"
                                                    .into(),
                                            );
                                        }
                                        let persisted = store
                                            .load(NOTE_ID)?
                                            .ok_or("explicit discard removed the original note")?;
                                        if persisted.revision != 1
                                            || persisted.search_text() != INITIAL_BODY
                                        {
                                            return Err(format!(
                                                "explicit discard changed persisted Notes data: revision={} body={:?}",
                                                persisted.revision,
                                                persisted.search_text()
                                            )
                                            .into());
                                        }
                                        recovery_stdio::log_stderr(
                                            "NAGI_NOTES_RECOVERY_UI_PASS phase=recovery-ui-smoke explicit_discard=1 dirty_owner=discarded discard_notice=presented persisted_revision=1 persisted_body=empty",
                                        );
                                        return Ok(());
                                    }
                                    return Err(Box::new(InputReadFailure {
                                        message: format!(
                                            "input lost; unsaved draft explicitly discarded: {input_error_message}"
                                        ),
                                        partial_record_bytes,
                                    }));
                                }
                                RecoveryDecision::InputLost(loss) => {
                                    let reason = loss.reason().to_owned();
                                    let partial_record_bytes = loss.partial_record_bytes();
                                    match dispatch_reconnected_input_loss(
                                        &mut recovery,
                                        &loss,
                                        &mut recovery_host,
                                    ) {
                                        ReconnectedInputLossDispatch::RetryOriginalGrant {
                                            notification_error,
                                        } => {
                                            let cause_id = cause_for_recovery_errors(
                                                Some(&reason),
                                                None,
                                                None,
                                            );
                                            let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            recovery_host.log_diagnostic(&format!(
                                                "reconnected_input_lost partial_record_bytes={partial_record_bytes} notification_error={}",
                                                notification_error.as_deref().unwrap_or("none")
                                            ));
                                            // The same owner and identity gate now wait for a
                                            // fresh open. The next loop iteration also resumes
                                            // the owner's bounded notification timer.
                                            reconnect_notice_reported = true;
                                            reconnect_status_attempted = true;
                                            reconnect_delay = Duration::from_secs(1);
                                            continue 'reconnect;
                                        }
                                        ReconnectedInputLossDispatch::ReArmFailed { error } => {
                                            let detail = format!(
                                                "reconnected input loss could not re-arm the original identity gate: {error}"
                                            );
                                            recovery_host.log_diagnostic(&detail);
                                            let cause_id = cause_for_recovery_errors(
                                                Some(&detail),
                                                None,
                                                None,
                                            );
                                            let _ = recovery_host.show_status_with_cause(
                                                "desktop.notes.recovery.reconnect_lost",
                                                Some("desktop.notes.recovery.reconnect_lost_hint"),
                                                Some(cause_id),
                                            );
                                            // An impossible adapter-state mismatch must not drop
                                            // the dirty owner. Keep notification retries alive
                                            // and fail closed instead of accepting a new identity.
                                            loop {
                                                let retry = {
                                                    let mut ui = RecoveryUiCommandAdapter::borrow(
                                                        recovery.owner_mut(),
                                                    );
                                                    ui.retry_notification_if_due(
                                                        Instant::now(),
                                                        &mut recovery_host,
                                                    )
                                                };
                                                if let Some(result) = retry {
                                                    recovery_stdio::log_stderr(&format!(
                                                        "NAGI_NOTES_RECOVERY_NOTICE_RETRY result={result:?}"
                                                    ));
                                                }
                                                thread::sleep(Duration::from_secs(30));
                                            }
                                        }
                                    }
                                }
                            }
                        };
                        drop(recovery);
                        drop(recovery_host);
                        desktop = Arc::try_unwrap(shared_desktop)
                            .unwrap_or_else(|_| {
                                panic!("recovery desktop has unexpected extra owners")
                            })
                            .into_inner()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        event_stream = reopened_stream;
                        resync_device = event_stream.reader().try_clone()?;
                        active_grant = Some(LinuxInputGrant::capture(
                            event_path.to_path_buf(),
                            event_stream.reader(),
                        ));
                    }
                }
            }
        }
    }
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let phase = arguments
        .next()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let disk = arguments.next().map(std::path::PathBuf::from);
    let event = arguments.next().map(std::path::PathBuf::from);
    let result = match (disk, event, arguments.next()) {
        (Some(disk), Some(event), None) => run(&phase, &disk, &event),
        _ => Err("usage: nagi-linux-notes-desktop-dev <save|read> DISK EVDEV".into()),
    };
    if let Err(error) = result {
        recovery_stdio::log_stderr(&format!(
            "NAGI_NOTES_DESKTOP_FAIL phase={phase} error={error}"
        ));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod device_contract_tests {
    use super::*;

    fn field(offset: u32) -> FbBitfield {
        FbBitfield {
            offset,
            length: 8,
            msb_right: 0,
        }
    }

    #[test]
    fn ioctl_nonnegative_return_values_are_success() {
        assert!(ioctl_succeeded(0));
        assert!(ioctl_succeeded(KEY_BITMAP_BYTES as c_int));
        assert!(!ioctl_succeeded(-1));
    }

    fn supported_framebuffer() -> (FbVarScreenInfo, FbFixScreenInfo) {
        let variable = FbVarScreenInfo {
            xres: SURFACE_WIDTH,
            yres: SURFACE_HEIGHT,
            xres_virtual: SURFACE_WIDTH,
            yres_virtual: SURFACE_HEIGHT,
            bits_per_pixel: 32,
            red: field(0),
            green: field(8),
            blue: field(16),
            transp: field(24),
            ..FbVarScreenInfo::default()
        };
        let fixed = FbFixScreenInfo {
            smem_len: (1296 * SURFACE_HEIGHT) as u32,
            kind: FB_TYPE_PACKED_PIXELS,
            visual: FB_VISUAL_TRUECOLOR,
            line_length: 1296,
            ..FbFixScreenInfo::default()
        };
        (variable, fixed)
    }

    #[test]
    fn framebuffer_contract_accepts_rgba_and_padded_stride() {
        let (variable, fixed) = supported_framebuffer();
        assert_eq!(
            validate_framebuffer_layout(&variable, &fixed),
            Ok(FramebufferLayout {
                line_length: 1296,
                pixel_format: FramebufferPixelFormat::Rgba8888,
            })
        );
        assert_eq!(core::mem::size_of::<FbVarScreenInfo>(), 160);
        assert_eq!(core::mem::size_of::<FbFixScreenInfo>(), 80);
    }

    #[test]
    fn framebuffer_contract_accepts_bgra_and_swaps_red_blue_for_device_memory() {
        let (mut variable, fixed) = supported_framebuffer();
        variable.red = field(16);
        variable.blue = field(0);
        assert_eq!(
            validate_framebuffer_layout(&variable, &fixed),
            Ok(FramebufferLayout {
                line_length: 1296,
                pixel_format: FramebufferPixelFormat::Bgra8888,
            })
        );
        assert_eq!(
            framebuffer_pixel(0xaabb_ccdd, FramebufferPixelFormat::Bgra8888),
            0xaadd_ccbb
        );
        assert_eq!(
            framebuffer_pixel(0xaabb_ccdd, FramebufferPixelFormat::Rgba8888),
            0xaabb_ccdd
        );
    }

    #[test]
    fn framebuffer_contract_rejects_format_geometry_stride_and_capacity_mismatch() {
        let (variable, mut fixed) = supported_framebuffer();
        let mut wrong_format = variable;
        wrong_format.red = field(16);
        assert!(validate_framebuffer_layout(&wrong_format, &fixed).is_err());

        let mut wrong_geometry = variable;
        wrong_geometry.xres = 640;
        assert!(validate_framebuffer_layout(&wrong_geometry, &fixed).is_err());

        let mut grayscale = variable;
        grayscale.grayscale = 1;
        assert!(validate_framebuffer_layout(&grayscale, &fixed).is_err());

        let mut fourcc = variable;
        fourcc.grayscale = 2;
        assert!(validate_framebuffer_layout(&fourcc, &fixed).is_err());

        let mut nonstandard_pixel_format = variable;
        nonstandard_pixel_format.nonstd = 1;
        assert!(validate_framebuffer_layout(&nonstandard_pixel_format, &fixed).is_err());

        let mut rotated = variable;
        rotated.rotate = 1;
        assert!(validate_framebuffer_layout(&rotated, &fixed).is_err());

        fixed.line_length = 1279;
        assert!(validate_framebuffer_layout(&variable, &fixed).is_err());

        let (_, mut too_small) = supported_framebuffer();
        too_small.smem_len = 1296 * SURFACE_HEIGHT - 1;
        assert!(validate_framebuffer_layout(&variable, &too_small).is_err());
    }

    #[test]
    fn keyboard_contract_requires_every_key_used_by_notes_acceptance() {
        let mut bitmap = [0u8; KEY_BITMAP_BYTES];
        for key in [
            INPUT_KEY_TAB,
            INPUT_KEY_ENTER,
            EVDEV_KEY_A,
            EVDEV_KEY_S,
            INPUT_KEY_SPACE,
        ] {
            bitmap[usize::from(key) / 8] |= 1 << (key % 8);
        }
        assert_eq!(validate_keyboard_bitmap(&bitmap), Ok(()));
        for missing in [
            INPUT_KEY_TAB,
            INPUT_KEY_ENTER,
            EVDEV_KEY_A,
            EVDEV_KEY_S,
            INPUT_KEY_SPACE,
        ] {
            let mut incomplete = bitmap;
            incomplete[usize::from(missing) / 8] &= !(1 << (missing % 8));
            assert!(validate_keyboard_bitmap(&incomplete).is_err());
        }
    }

    #[test]
    fn preflight_failure_does_not_enter_storage_or_modify_its_input() {
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-preflight-{}-{}.raw",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"existing-valid-volume").unwrap();
        let mut storage_opened = false;
        let result: Result<((), ()), &'static str> = preflight_before_storage(
            || Err("device preflight failed"),
            || {
                storage_opened = true;
                std::fs::write(&path, b"changed").unwrap();
                Ok(())
            },
        );
        assert_eq!(result, Err("device preflight failed"));
        assert!(!storage_opened);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing-valid-volume");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn save_fixture_setup_resumes_only_its_untouched_initial_note() {
        let store = InMemoryNoteStore::new();
        ensure_initial_note(&store).unwrap();
        let initial = store.load(NOTE_ID).unwrap().unwrap();
        assert_eq!(initial.revision, 1);
        assert_eq!(initial.search_text(), INITIAL_BODY);
        ensure_initial_note(&store).unwrap();
        assert_eq!(store.load(NOTE_ID).unwrap().unwrap(), initial);

        let mut changed = initial;
        changed.title = "unexpected pre-existing note".to_owned();
        store.commit(&changed, 1).unwrap();
        assert!(ensure_initial_note(&store).is_err());
    }

    #[test]
    fn selected_keyboard_must_report_all_notes_input_capabilities() {
        let mut bitmap = [0u8; KEY_BITMAP_BYTES];
        for key in [
            INPUT_KEY_TAB,
            INPUT_KEY_ENTER,
            EVDEV_KEY_A,
            EVDEV_KEY_S,
            INPUT_KEY_SPACE,
        ] {
            bitmap[usize::from(key) / 8] |= 1 << (key % 8);
        }
        assert_eq!(validate_keyboard_bitmap(&bitmap), Ok(()));
        bitmap[usize::from(INPUT_KEY_ENTER) / 8] &= !(1 << (INPUT_KEY_ENTER % 8));
        assert!(validate_keyboard_bitmap(&bitmap).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod input_failure_drop_tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_DISK_BYTES: u64 = 8 * 1024 * 1024;

    struct TestDisk(PathBuf);

    impl TestDisk {
        fn create() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nagi-input-drop-{}-{nonce}.raw",
                std::process::id()
            ));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .expect("create isolated input-drop VFS disk");
            file.set_len(TEST_DISK_BYTES)
                .expect("size isolated input-drop VFS disk");
            file.sync_all().expect("sync isolated input-drop VFS disk");
            drop(file);
            Self(path)
        }
    }

    impl Drop for TestDisk {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[derive(Clone, Copy)]
    enum TerminalRead {
        Eof,
        WouldBlock,
        IoError,
    }

    struct FailureReader {
        bytes: Vec<u8>,
        offset: usize,
        terminal: TerminalRead,
    }

    impl FailureReader {
        fn new(terminal: TerminalRead, truncated_record: bool) -> Self {
            let mut bytes = Vec::new();
            for code in [
                INPUT_KEY_TAB,
                INPUT_KEY_TAB,
                INPUT_KEY_TAB,
                INPUT_KEY_ENTER,
                EVDEV_KEY_A,
            ] {
                bytes.extend_from_slice(&key_record(code));
            }
            if truncated_record {
                bytes.extend_from_slice(&[0x5a; 9]);
            }
            Self {
                bytes,
                offset: 0,
                terminal,
            }
        }
    }

    impl Read for FailureReader {
        fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
            if self.offset < self.bytes.len() {
                let count = destination.len().min(self.bytes.len() - self.offset);
                destination[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            match self.terminal {
                TerminalRead::Eof => Ok(0),
                TerminalRead::WouldBlock => Err(io::ErrorKind::WouldBlock.into()),
                TerminalRead::IoError => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }
    }

    fn key_record(code: u16) -> [u8; 24] {
        let mut record = [0; 24];
        record[16..18].copy_from_slice(&INPUT_EVENT_KEY.to_le_bytes());
        record[18..20].copy_from_slice(&code.to_le_bytes());
        record[20..24].copy_from_slice(&1i32.to_le_bytes());
        record
    }

    fn assert_input_failure_returns_owned_desktop_for_flush(
        terminal: TerminalRead,
        truncated_record: bool,
        expected_error: &str,
    ) {
        let disk = TestDisk::create();
        let (mut volume, formatted) =
            new_linux_development_volume(&disk.0, true).expect("mount test VFS");
        assert!(formatted);

        let store = Arc::new(VfsNoteStore::new(volume.clone()));
        ensure_initial_note(store.as_ref()).expect("seed initial Notes revision");
        let initial = store
            .load(NOTE_ID)
            .expect("load initial Notes revision")
            .expect("initial Notes revision exists");
        assert_eq!(initial.revision, 1);
        assert_eq!(initial.search_text(), INITIAL_BODY);

        let bridge = NotesEventBridge::new(Arc::clone(&store));
        let desktop = Desktop::new_with_notes(Locale::EnUs, bridge, NOTE_ID)
            .expect("attach real Notes event bridge");
        let mut reader = FailureReader::new(terminal, truncated_record);
        let mut stream = EvdevStream::new(&mut reader, LinuxEvdevTarget::X86_64);

        let result = run_desktop_input_loop(
            desktop,
            &mut volume,
            store.as_ref(),
            "save",
            || next_input_item(&mut stream, || Ok([0; KEY_BITMAP_BYTES])),
            |_| Ok(()),
        );
        let InputLoopExit::InputLost {
            desktop,
            error,
            partial_record_bytes,
        } = result.expect("input failure transfers Desktop ownership")
        else {
            panic!("fixture input must fail before acceptance completes");
        };
        assert_eq!(error.to_string(), expected_error);
        assert_eq!(partial_record_bytes, if truncated_record { 9 } else { 0 });

        let dirty = desktop
            .current_notes_document()
            .cloned()
            .expect("normal Tab/Enter route opened the Notes document");
        assert_eq!(
            dirty.revision, 1,
            "edit remains uncommitted while ownership is transferred"
        );
        assert_eq!(dirty.search_text(), "a");
        assert_eq!(
            store
                .load(NOTE_ID)
                .expect("read persisted Notes state while owner is held"),
            Some(initial.clone())
        );

        let failure = guard_input_failure(
            desktop,
            error,
            |desktop| desktop.flush_notes_before_input_loss(),
            |_, _| Ok(()),
        );
        match RecoveryOwner::from_input_failure(failure) {
            RecoveryStart::FlushSucceeded { owner, input_error } => {
                assert_eq!(input_error.to_string(), expected_error);
                drop(owner);
            }
            RecoveryStart::Held(_) => panic!("real VFS flush should succeed in this fixture"),
        }

        let after_drop = store
            .load(NOTE_ID)
            .expect("load Notes state after Desktop teardown")
            .expect("Notes document survives teardown");
        assert_eq!(after_drop.revision, 2);
        assert_eq!(after_drop.search_text(), "a");
        assert_eq!(
            store
                .load_revision(NOTE_ID, 1)
                .expect("load original revision"),
            Some(initial)
        );
    }

    #[test]
    fn complete_key_records_then_eof_transfers_owner_and_flushes() {
        assert_input_failure_returns_owned_desktop_for_flush(
            TerminalRead::Eof,
            false,
            "evdev source closed",
        );
    }

    #[test]
    fn complete_key_records_then_truncated_record_reach_desktop_drop_and_flush() {
        assert_input_failure_returns_owned_desktop_for_flush(
            TerminalRead::Eof,
            true,
            "evdev record was truncated",
        );
    }

    #[test]
    fn complete_key_records_then_would_block_reach_desktop_drop_and_flush() {
        assert_input_failure_returns_owned_desktop_for_flush(
            TerminalRead::WouldBlock,
            false,
            "blocking evdev source returned WouldBlock",
        );
    }

    #[test]
    fn complete_key_records_then_io_error_reach_desktop_drop_and_flush() {
        assert_input_failure_returns_owned_desktop_for_flush(
            TerminalRead::IoError,
            false,
            "evdev read failed",
        );
    }
}

#[cfg(test)]
mod recovery_input_policy_tests {
    use super::*;

    fn key(code: u16, value: i32) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value,
        }
    }

    #[test]
    fn repeated_key_value_does_not_trigger_recovery_commands() {
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::ConfirmingDiscard, key(INPUT_KEY_ENTER, 2)),
            None
        );
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::AwaitingUserChoice, key(INPUT_KEY_ENTER, 2)),
            None
        );
    }

    #[test]
    fn enter_after_discard_cancellation_retries_save_instead_of_discarding() {
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::ConfirmingDiscard, key(INPUT_KEY_ESCAPE, 1)),
            Some(RecoveryUiCommand::CancelDiscard)
        );
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::AwaitingUserChoice, key(INPUT_KEY_ENTER, 1)),
            Some(RecoveryUiCommand::RetrySave)
        );
    }

    #[test]
    fn recovery_actions_are_unavailable_until_verified_input_is_restored() {
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::WaitingForInput, key(INPUT_KEY_ENTER, 1)),
            None
        );
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::WaitingForInput, key(INPUT_KEY_ESCAPE, 1)),
            None
        );
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::AwaitingUserChoice, key(INPUT_KEY_ENTER, 1)),
            Some(RecoveryUiCommand::RetrySave)
        );
        assert_eq!(
            recovery_command_for_key(RecoveryPhase::AwaitingUserChoice, key(INPUT_KEY_ESCAPE, 1)),
            Some(RecoveryUiCommand::BeginDiscard)
        );
        assert_eq!(
            recovery_command_for_key(
                RecoveryPhase::NotificationUnavailable,
                key(INPUT_KEY_ENTER, 1)
            ),
            Some(RecoveryUiCommand::RetryNotification)
        );
    }

    #[test]
    fn recovery_screen_cause_is_localized_and_does_not_expose_raw_path() {
        assert_eq!(
            cause_for_recovery_errors(
                None,
                None,
                Some("identity gate rejected /dev/input/event6: unique ID mismatch")
            ),
            "desktop.notes.recovery.cause.identity"
        );
        assert_eq!(
            cause_for_recovery_errors(None, Some("DirectoryFull at /private/user/data"), None),
            "desktop.notes.recovery.cause.storage_full"
        );
        assert_eq!(
            cause_for_recovery_errors(Some("evdev source closed"), None, None),
            "desktop.notes.recovery.cause.input_closed"
        );
    }

    #[test]
    fn reconnect_retry_backoff_doubles_to_a_thirty_second_cap() {
        let mut delay = Duration::from_secs(1);
        let mut observed = Vec::new();
        for _ in 0..7 {
            delay = next_reconnect_backoff(delay);
            observed.push(delay.as_secs());
        }
        assert_eq!(observed, [2, 4, 8, 16, 30, 30, 30]);
    }
}
