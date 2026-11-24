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
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(linux_task24_initial_login_runtime)]
use std::{os::unix::net::UnixStream, time::Instant};

#[cfg(linux_task24_initial_login_runtime)]
#[path = "../../../tools/linux-boot/task24-login/input.rs"]
pub(super) mod task24_input;

const O_NOFOLLOW: i32 = 0x20000;
const FBIOGET_VSCREENINFO: u64 = 0x4600;
const FBIOGET_FSCREENINFO: u64 = 0x4602;
const EVIOCGRAB: u64 = 0x40044590;

#[cfg(linux_task24_owner_co_location)]
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    returned: i16,
}

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    #[cfg(linux_task24_owner_co_location)]
    fn poll(fds: *mut PollFd, len: usize, timeout: i32) -> i32;
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
    Io(io::Error),
    CredentialUnavailable,
    Save(E),
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) enum UnlockInputOutcome {
    Submitted(UnlockInput),
    Closed,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) struct UnlockInput {
    account: Vec<u8>,
    password: zeroize::Zeroizing<Vec<u8>>,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) enum UnlockSubmissionError {
    DeadlineExpired,
    Unavailable,
}

#[cfg(linux_task24_initial_login_runtime)]
impl UnlockInput {
    pub(super) fn submit(
        self,
        control: &mut UnixStream,
        route: task24_input::RequestRoute,
        deadline: Instant,
    ) -> Result<(), UnlockSubmissionError> {
        let mut failure = UnlockSubmissionError::Unavailable;
        let sent = task24_input::send(
            |bytes| {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    failure = UnlockSubmissionError::DeadlineExpired;
                    return false;
                }
                if control.set_write_timeout(Some(remaining)).is_err() {
                    return false;
                }
                match control.write_all(bytes) {
                    Ok(()) => true,
                    Err(error) => {
                        if error.kind() == io::ErrorKind::TimedOut
                            || (error.kind() == io::ErrorKind::WouldBlock
                                && Instant::now() >= deadline)
                        {
                            failure = UnlockSubmissionError::DeadlineExpired;
                        }
                        false
                    }
                }
            },
            route,
            &self.account,
            &self.password,
        );
        if sent {
            Ok(())
        } else {
            Err(failure)
        }
    }
}

pub(super) struct ConsoleKey {
    pub(super) event: InputEvent,
    #[cfg(all(
        linux_task24_owner_co_location,
        linux_task24_initial_login_runtime,
        linux_task24_notes_consumer,
        linux_dev_notes_performance,
        not(nagi_release_build)
    ))]
    pub(super) monotonic_ns: Option<u64>,
}

#[derive(Debug)]
pub(super) struct KeyboardEventsLost;

impl std::fmt::Display for KeyboardEventsLost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("keyboard events were lost")
    }
}

impl std::error::Error for KeyboardEventsLost {}

pub(super) struct BootOwnerConsole {
    #[cfg(all(
        linux_task24_owner_co_location,
        linux_task24_initial_login_runtime,
        linux_task24_notes_consumer,
        linux_dev_notes_performance,
        not(nagi_release_build)
    ))]
    keyboard_clock_ready_ns: Option<u64>,
    keyboard: File,
    discard_until_sync_report: bool,
    framebuffer: Framebuffer,
    pixels: Vec<u32>,
    frame_presented: Arc<AtomicBool>,
}

impl BootOwnerConsole {
    pub(super) fn open() -> io::Result<Self> {
        let (_, keyboard) = select_development_keyboard()
            .map_err(|_| io::Error::other("trusted boot keyboard unavailable"))?;
        // EVIOCGRAB gives this retained descriptor exclusive input delivery.
        if unsafe { ioctl(keyboard.as_raw_fd(), EVIOCGRAB, 1i32) } < 0 {
            return Err(io::Error::last_os_error());
        }
        #[cfg(all(
            linux_task24_owner_co_location,
            linux_task24_initial_login_runtime,
            linux_task24_notes_consumer,
            linux_dev_notes_performance,
            not(nagi_release_build)
        ))]
        let keyboard_clock_ready_ns =
            crate::task24_notes_performance::monotonic_ns().and_then(|before| {
                // Linux x86-64 EVIOCSCLOCKID takes a pointer to a clockid_t.
                const EVIOCSCLOCKID: u64 = 0x4004_45a0;
                // SAFETY: the retained evdev FD is live and ioctl reads one clockid_t.
                let result =
                    unsafe { ioctl(keyboard.as_raw_fd(), EVIOCSCLOCKID, &libc::CLOCK_MONOTONIC) };
                (result == 0)
                    .then(|| crate::task24_notes_performance::monotonic_ns().unwrap_or(before))
            });
        Ok(Self {
            #[cfg(all(
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                linux_task24_notes_consumer,
                linux_dev_notes_performance,
                not(nagi_release_build)
            ))]
            keyboard_clock_ready_ns,
            keyboard,
            discard_until_sync_report: false,
            framebuffer: Framebuffer::open()?,
            pixels: vec![0; (SURFACE_WIDTH * SURFACE_HEIGHT) as usize],
            frame_presented: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn present(
        &mut self,
        render: impl FnOnce(&mut crate::ui::Painter<'_>),
    ) -> io::Result<()> {
        self.frame_presented.store(false, Ordering::Release);
        render(&mut crate::ui::Painter::new(&mut self.pixels));
        self.framebuffer.present(&self.pixels)?;
        self.frame_presented.store(true, Ordering::Release);
        Ok(())
    }

    fn read_key_event(&mut self) -> io::Result<Option<ConsoleKey>> {
        // x86-64 input_event ends in u16 type, u16 code, and i32 value.
        let mut data = [0u8; 24];
        if let Err(error) = self.keyboard.read_exact(&mut data) {
            self.frame_presented.store(false, Ordering::Release);
            return Err(error);
        }
        let event_type = u16::from_le_bytes([data[16], data[17]]);
        let code = u16::from_le_bytes([data[18], data[19]]);
        let value = i32::from_le_bytes([data[20], data[21], data[22], data[23]]);
        if self.discard_until_sync_report {
            // evdev requires discarding every record through the next
            // SYN_REPORT, including keys queued after SYN_DROPPED.
            self.discard_until_sync_report = (event_type, code) != (0, 0);
            return Ok(None);
        }
        if event_type == 0 && code == 3 {
            self.discard_until_sync_report = true;
            #[cfg(all(
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                linux_task24_notes_consumer,
                linux_dev_notes_performance,
                not(nagi_release_build)
            ))]
            // Clock setup can flush a pending startup packet and enqueue
            // SYN_DROPPED. No input state has been consumed at that point.
            if crate::task24_notes_performance::clock_setup_dropped(
                self.keyboard_clock_ready_ns,
                &data,
            ) {
                return Ok(None);
            }
            self.frame_presented.store(false, Ordering::Release);
            return Err(io::Error::other(KeyboardEventsLost));
        }
        Ok(
            (event_type == libnagi::INPUT_EVENT_KEY && (value == 0 || value == 1)).then(|| {
                ConsoleKey {
                    event: InputEvent {
                        event_type,
                        code,
                        value,
                    },
                    #[cfg(all(
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        linux_task24_notes_consumer,
                        linux_dev_notes_performance,
                        not(nagi_release_build)
                    ))]
                    monotonic_ns: self
                        .keyboard_clock_ready_ns
                        .filter(|_| value == 1)
                        .and_then(|ready_ns| {
                            crate::task24_notes_performance::evdev_monotonic_ns(&data)
                                .filter(|received_ns| *received_ns >= ready_ns)
                        }),
                }
            }),
        )
    }

    pub(super) fn next_key(&mut self) -> io::Result<InputEvent> {
        loop {
            if let Some(event) = self.read_key_event()? {
                return Ok(event.event);
            }
        }
    }

    #[cfg(all(linux_task24_owner_co_location, linux_task24_initial_login_runtime))]
    pub(super) fn next_key_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> io::Result<Option<InputEvent>> {
        self.next_console_key_timeout(timeout)
            .map(|key| key.map(|key| key.event))
    }

    #[cfg(all(linux_task24_owner_co_location, linux_task24_initial_login_runtime))]
    pub(super) fn next_console_key_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> io::Result<Option<ConsoleKey>> {
        const POLLIN: i16 = 0x001;
        let mut descriptor = PollFd {
            fd: self.keyboard.as_raw_fd(),
            events: POLLIN,
            returned: 0,
        };
        let milliseconds = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: the retained evdev descriptor is live and poll receives one
        // writable pollfd. This console is the only keyboard reader here.
        let result = unsafe { poll(&mut descriptor, 1, milliseconds) };
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(error)
            };
        }
        if result == 0 {
            return Ok(None);
        }
        if descriptor.returned != POLLIN {
            return Err(io::Error::other("keyboard poll returned an invalid event"));
        }
        self.read_key_event()
    }

    pub(super) fn run_registration<SaveAccount, E>(
        &mut self,
        mut screen: LinuxOwnerRegistration<SaveAccount>,
        mut next_key: impl FnMut(&mut Self) -> io::Result<InputEvent>,
    ) -> Result<(), RegistrationConsoleError<E>>
    where
        SaveAccount: FnOnce(AccountRecord) -> Result<(), E>,
    {
        use RegistrationConsoleError::*;
        self.present(|painter| screen.render(painter)).map_err(Io)?;
        #[cfg(linux_task24_owner_co_location)]
        if let Ok(connection) = self.consent_connection() {
            use super::pid1_live_peer_supplier::ConsentSource;
            if matches!(connection.connected(), Ok(true)) {
                super::log_pid1_stderr(format_args!("NAGI_LINUX_OWNER_UI_CONNECTED"));
            }
        }
        super::log_pid1_stderr(format_args!(
            "NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT"
        ));
        loop {
            let event = match next_key(self) {
                Ok(event) => event,
                Err(error)
                    if error
                        .get_ref()
                        .is_some_and(|cause| cause.is::<KeyboardEventsLost>()) =>
                {
                    screen.reset_after_input_loss();
                    self.present(|painter| screen.render(painter)).map_err(Io)?;
                    continue;
                }
                Err(error) => return Err(Io(error)),
            };
            if event.value == 0 {
                continue;
            }
            match screen.handle_event(event) {
                LinuxRegistrationOutcome::Ignored => {}
                LinuxRegistrationOutcome::Redraw => {
                    self.present(|painter| screen.render(painter)).map_err(Io)?;
                }
                LinuxRegistrationOutcome::CredentialUnavailable => {
                    self.present(|painter| screen.render(painter)).map_err(Io)?;
                    return Err(CredentialUnavailable);
                }
                LinuxRegistrationOutcome::Submitted(result) => {
                    self.present(|painter| screen.render(painter)).map_err(Io)?;
                    return result.map_err(Save);
                }
            }
        }
    }

    #[cfg(linux_task24_initial_login_runtime)]
    pub(super) fn present_unlock(
        &mut self,
        screen: &crate::login_screen::LinuxOwnerUnlock<'_>,
    ) -> io::Result<()> {
        self.present(|painter| screen.render(painter))
    }

    #[cfg(all(linux_task24_owner_co_location, linux_task24_initial_login_runtime))]
    pub(super) fn read_unlock(
        &mut self,
        screen: &mut crate::login_screen::LinuxOwnerUnlock<'_>,
        mut poll_required_identity_exit: impl FnMut() -> io::Result<bool>,
    ) -> io::Result<UnlockInputOutcome> {
        use super::{poll_task24_input, Task24InputPoll};
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT"));
        loop {
            if screen.tick() {
                super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LOGIN_RETRY_ALLOWED"));
                self.present_unlock(screen)?;
            }
            let wait = screen.remaining_wait();
            let interval = std::time::Duration::from_millis(100);
            let slice = if wait.is_zero() {
                interval
            } else {
                interval.min(wait)
            };
            let event = match poll_task24_input(
                || self.next_key_timeout(slice),
                &mut poll_required_identity_exit,
            )? {
                Task24InputPoll::Pending => continue,
                Task24InputPoll::Input(event) => event,
                Task24InputPoll::RequiredServiceExited => {
                    return Err(io::Error::other("required Identity exited during unlock"));
                }
            };
            if event.code == libnagi::INPUT_KEY_ESCAPE && event.value == 1 {
                screen.clear_secrets();
                self.present_unlock(screen)?;
                return Ok(UnlockInputOutcome::Closed);
            }
            let mut input = None;
            match screen.handle_event(event, |account, password| {
                input = Some(UnlockInput {
                    account: account.to_vec(),
                    password: zeroize::Zeroizing::new(password.to_vec()),
                });
                Ok::<(), io::Error>(())
            }) {
                LinuxRegistrationOutcome::Redraw => self.present_unlock(screen)?,
                LinuxRegistrationOutcome::Submitted(result) => {
                    self.present_unlock(screen)?;
                    result?;
                    return input
                        .map(UnlockInputOutcome::Submitted)
                        .ok_or_else(|| io::Error::other("Task24 unlock input missing"));
                }
                _ => {}
            }
        }
    }

    /// The authenticated desktop and Supervisor still own the consent
    /// request, its Session, and recording its answer.
    pub(super) fn wait_for_consent(
        &mut self,
        dialog: &mut crate::consent_dialog::ConsentDialog,
        user: &libnagi::security::Session,
        locale: nagi_localization::Locale,
    ) -> io::Result<crate::consent_dialog::ConsentAnswer> {
        use crate::consent_dialog::PromptOutcome;

        if user.token() == 0
            || user.is_locked()
            || dialog.is_armed()
            || crate::supervisor::next_consent_request() != Some(dialog.request())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "consent dialog unavailable",
            ));
        }
        self.present(|painter| dialog.render(painter, locale))?;
        dialog.arm();
        loop {
            match dialog.handle_event(self.next_key()?, 0, 0) {
                PromptOutcome::Ignored => {}
                PromptOutcome::Changed => self.present(|painter| dialog.render(painter, locale))?,
                PromptOutcome::Answered(answer) => {
                    if crate::supervisor::next_consent_request() != Some(dialog.request()) {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "consent request no longer pending",
                        ));
                    }
                    return Ok(answer);
                }
            }
        }
    }

    #[cfg(linux_task24_owner_co_location)]
    pub(super) fn consent_connection(&self) -> io::Result<BootConsentUiConnection> {
        if !self.frame_presented.load(Ordering::Acquire) {
            return Err(io::Error::other("no frame presented by the owner UI"));
        }
        Ok(BootConsentUiConnection {
            keyboard: self.keyboard.try_clone()?,
            framebuffer: self.framebuffer.device.try_clone()?,
            frame_presented: Arc::clone(&self.frame_presented),
        })
    }
}

#[cfg(linux_task24_owner_co_location)]
pub(super) struct BootConsentUiConnection {
    keyboard: File,
    framebuffer: File,
    frame_presented: Arc<AtomicBool>,
}

#[cfg(linux_task24_owner_co_location)]
impl super::pid1_live_peer_supplier::ConsentSource for BootConsentUiConnection {
    fn connected(&self) -> Result<bool, super::pid1_live_peer_supplier::PeerDenial> {
        use super::pid1_live_peer_supplier::PeerDenial;

        let mut fds = [
            PollFd {
                fd: self.keyboard.as_raw_fd(),
                events: 0,
                returned: 0,
            },
            PollFd {
                fd: self.framebuffer.as_raw_fd(),
                events: 0,
                returned: 0,
            },
        ];
        // No read, wait, callback, or decision under the broker/registry lock.
        let result = unsafe { poll(fds.as_mut_ptr(), fds.len(), 0) };
        if result < 0 {
            return Err(PeerDenial::ConsentUnavailable);
        }
        Ok(self.frame_presented.load(Ordering::Acquire) && fds.iter().all(|fd| fd.returned == 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn registration_clears_input_after_syn_dropped_and_accepts_fresh_submission() {
        let (keyboard, mut writer) = UnixStream::pair().unwrap();
        let mut record = |event_type: u16, code: u16, value: i32| {
            let mut bytes = [0; 24];
            bytes[16..18].copy_from_slice(&event_type.to_le_bytes());
            bytes[18..20].copy_from_slice(&code.to_le_bytes());
            bytes[20..].copy_from_slice(&value.to_le_bytes());
            writer.write_all(&bytes).unwrap();
        };
        let enter = libnagi::INPUT_KEY_ENTER;
        let tab = libnagi::INPUT_KEY_TAB;
        for code in [enter, 30, tab, 30, 30, 30, 30, tab, 30, 30, 30, 30] {
            record(libnagi::INPUT_EVENT_KEY, code, 1);
        }
        record(0, 3, 0); // SYN_DROPPED, with a complete draft pending.
        record(libnagi::INPUT_EVENT_KEY, 30, 1);
        record(libnagi::INPUT_EVENT_KEY, enter, 1);
        record(0, 0, 0); // SYN_REPORT.
        for code in [48, tab, 48, 48, 48, 48, tab, 48, 48, 48, 48, enter] {
            record(libnagi::INPUT_EVENT_KEY, code, 1);
        }
        drop(writer);

        let width = SURFACE_WIDTH as usize;
        let height = SURFACE_HEIGHT as usize;
        let mut console = BootOwnerConsole {
            #[cfg(all(
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                linux_task24_notes_consumer,
                linux_dev_notes_performance
            ))]
            keyboard_clock_ready_ns: None,
            keyboard: File::from(OwnedFd::from(keyboard)),
            discard_until_sync_report: false,
            framebuffer: Framebuffer {
                device: File::options().write(true).open("/dev/null").unwrap(),
                stride: u64::from(SURFACE_WIDTH) * 4,
                width,
                height,
                swap_red_blue: false,
            },
            pixels: vec![0; width * height],
            frame_presented: Arc::new(AtomicBool::new(false)),
        };
        let locale = nagi_localization::Locale::EnUs;
        let submitted = Cell::new(false);
        let screen = LinuxOwnerRegistration::new(locale, |record: AccountRecord| {
            assert!(!submitted.replace(true));
            assert_eq!(record.name(), b"b");
            assert!(record.credential.verify(b"bbbb"));
            Ok::<_, ()>(())
        });
        let mut blank = LinuxOwnerRegistration::new(locale, |_| Ok::<_, ()>(()));
        blank.handle_event(InputEvent {
            event_type: libnagi::INPUT_EVENT_KEY,
            code: enter,
            value: 1,
        });
        let mut blank_pixels = vec![0; width * height];
        blank.render(&mut crate::ui::Painter::new(&mut blank_pixels));

        let mut checked_reset = false;
        console
            .run_registration(screen, |console| {
                // The previous read reported loss. The registration loop must
                // redraw before reading the boundary and any further keys.
                if console.discard_until_sync_report {
                    assert!(!checked_reset);
                    assert!(!submitted.get());
                    let notice_row = 158 * width;
                    assert_eq!(console.pixels[..notice_row], blank_pixels[..notice_row]);
                    assert_ne!(console.pixels[notice_row..], blank_pixels[notice_row..]);
                    checked_reset = true;
                }
                console.next_key()
            })
            .unwrap();
        assert!(checked_reset && submitted.get());
    }
}
