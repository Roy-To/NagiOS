use alloc::string::String;
#[cfg(target_arch = "x86_64")]
use core::arch::asm;

#[cfg(all(not(test), not(feature = "linux-development-smoke")))]
use libnagi::storage::SyscallBlockDevice;
#[cfg(any(
    test,
    all(feature = "linux-development-smoke", not(target_os = "linux"))
))]
use libnagi::storage::Vfs;
#[cfg(any(test, feature = "linux-development-smoke"))]
use libnagi::storage::{BlockDevice, ReadOnlyBlockDevice, SECTOR_SIZE};
use libnagi::storage::{StorageError, MAX_SMALL_FILE_SIZE};
use libnagi::{DisplayInfo, InputEvent};
use notes_desktop_localization_adapter::NotesLocalization;
use notes_event_bridge_candidate::NotesEventBridge;
#[cfg(feature = "dev-host-preview")]
use notes_event_bridge_candidate::{
    EvdevRecordError, LinuxEvdevTarget, NotesEventStream, NotesStreamError, NotesStreamPoll,
};
use notes_files_codec_candidate::{
    AppError, HostPreviewStore, NoteDocument, ObjectId, SandboxError,
};
#[cfg(all(feature = "linux-development-smoke", target_os = "linux", not(test)))]
use notes_files_codec_candidate::{SharedVfs, StoreError};
use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
pub(super) use crate::m19_runtime::files;
use crate::ui::{Painter, Rect};
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
#[path = "m19_files_panel.rs"]
pub(super) mod files_panel;
#[path = "recovery_layout.rs"]
mod recovery_layout;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_MANAGE_KEY: u16 = 60; // Linux/VirtIO input F2.
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_PANEL: Rect = Rect::new(17, 25, 286, 167);
use nagi_ui::{color, ColorRole, ThemeMode};

#[cfg(all(not(test), not(feature = "linux-development-smoke")))]
pub(super) type UserDataVolume = Vfs<SyscallBlockDevice>;

/// Host unit tests use the real VFS over a disposable in-memory block device.
/// This is test storage only; it does not construct or stand in for a runtime
/// block capability.
#[cfg(any(test, feature = "linux-development-smoke"))]
pub struct DesktopTestBlockDevice {
    sectors: std::vec::Vec<[u8; SECTOR_SIZE]>,
}

#[cfg(any(test, feature = "linux-development-smoke"))]
impl DesktopTestBlockDevice {
    pub fn new() -> Self {
        Self {
            // The existing Nagi VFS format uses 8192 1KiB blocks.
            sectors: std::vec![[0; SECTOR_SIZE]; 16_384],
        }
    }
}

#[cfg(any(test, feature = "linux-development-smoke"))]
impl ReadOnlyBlockDevice for DesktopTestBlockDevice {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let index = usize::try_from(sector).map_err(|_| StorageError::Block)?;
        let source = self.sectors.get(index).ok_or(StorageError::Block)?;
        destination.copy_from_slice(source);
        Ok(())
    }
}

#[cfg(any(test, feature = "linux-development-smoke"))]
impl BlockDevice for DesktopTestBlockDevice {
    fn write_sector(
        &mut self,
        sector: u64,
        source: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let index = usize::try_from(sector).map_err(|_| StorageError::Block)?;
        let destination = self.sectors.get_mut(index).ok_or(StorageError::Block)?;
        destination.copy_from_slice(source);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        Ok(())
    }
}

#[cfg(all(test, not(feature = "linux-development-smoke")))]
pub(super) type UserDataVolume = Vfs<DesktopTestBlockDevice>;

#[cfg(any(
    test,
    all(feature = "linux-development-smoke", not(target_os = "linux"))
))]
pub type UserDataVolume = Vfs<DesktopTestBlockDevice>;

#[cfg(all(feature = "linux-development-smoke", target_os = "linux", not(test)))]
pub type UserDataVolume = SharedVfs;

#[cfg(any(
    test,
    all(feature = "linux-development-smoke", not(target_os = "linux"))
))]
pub type LinuxDevelopmentVolume = UserDataVolume;

#[cfg(any(
    test,
    all(feature = "linux-development-smoke", not(target_os = "linux"))
))]
pub fn new_linux_development_volume() -> Result<LinuxDevelopmentVolume, StorageError> {
    Vfs::mount_or_format(DesktopTestBlockDevice::new()).map(|(volume, _)| volume)
}

#[cfg(all(feature = "linux-development-smoke", target_os = "linux", not(test)))]
pub type LinuxDevelopmentVolume = SharedVfs;

#[cfg(all(feature = "linux-development-smoke", target_os = "linux", not(test)))]
pub fn new_linux_development_volume(
    path: &Path,
    format_if_missing: bool,
) -> Result<(LinuxDevelopmentVolume, bool), StoreError> {
    if format_if_missing {
        SharedVfs::mount_or_format(path)
    } else {
        SharedVfs::mount_existing(path).map(|volume| (volume, false))
    }
}

const SYSTEM_LANGUAGE_PATH: &[u8] = b"system-language";

const APP_COUNT: usize = 4;
const WINDOW_WIDTH: i32 = 145;
const WINDOW_HEIGHT: i32 = 70;
const TITLE_HEIGHT: i32 = 14;
const POINTER_START_X: i32 = 80;
const POINTER_START_Y: i32 = 58;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_SEARCH_QUERY_CAPACITY: usize = 32;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_SEARCH_TITLE_CAPACITY: usize = 20;
const PREVIEW_THEME: ThemeMode = ThemeMode::Light;
const BACKGROUND: u32 = color(PREVIEW_THEME, ColorRole::Canvas).to_pixel();
const PANEL: u32 = color(PREVIEW_THEME, ColorRole::Surface).to_pixel();
const TITLE: u32 = color(PREVIEW_THEME, ColorRole::Accent).to_pixel();
const BORDER: u32 = color(PREVIEW_THEME, ColorRole::BorderStrong).to_pixel();
const TEXT: u32 = color(PREVIEW_THEME, ColorRole::TextPrimary).to_pixel();
const TITLE_TEXT: u32 = color(PREVIEW_THEME, ColorRole::TextOnAccent).to_pixel();
const FOCUS: u32 = color(PREVIEW_THEME, ColorRole::Focus).to_pixel();
const SETTINGS_BUTTON: Rect = Rect::new(252, 1, 66, 18);
/// Taller with both extra rows (permissions and password) so neither
/// overlaps the language options.
#[cfg(all(feature = "desktop-login", feature = "consent-dialog"))]
const SETTINGS_PANEL: Rect = Rect::new(34, 34, 252, 150);
#[cfg(not(all(feature = "desktop-login", feature = "consent-dialog")))]
const SETTINGS_PANEL: Rect = Rect::new(34, 34, 252, 132);
const ENGLISH_OPTION: Rect = Rect::new(48, 83, 224, 25);
const JAPANESE_OPTION: Rect = Rect::new(48, 116, 224, 25);
/// Opens the permissions view (ADR 0065); shown only with consent.
#[cfg(feature = "consent-dialog")]
const PERMISSIONS_OPTION: Rect = Rect::new(48, 146, 224, 16);
/// Opens the change-password form (ADR 0066); shown only with sign-in.
#[cfg(all(feature = "desktop-login", feature = "consent-dialog"))]
const PASSWORD_OPTION: Rect = Rect::new(48, 164, 224, 16);
#[cfg(all(feature = "desktop-login", not(feature = "consent-dialog")))]
const PASSWORD_OPTION: Rect = Rect::new(48, 146, 224, 16);

#[no_mangle]
static NAGI_M10_READY: [u8; b"Nagi M10 desktop READY\r\n".len()] = *b"Nagi M10 desktop READY\r\n";
#[no_mangle]
static NAGI_M10_CHECKSUM_PREFIX: [u8; b"Nagi M10 surface checksum=".len()] =
    *b"Nagi M10 surface checksum=";
#[no_mangle]
static NAGI_M10_LINE_END: [u8; 2] = *b"\r\n";
#[no_mangle]
static NAGI_M10_CALCULATOR: [u8; b"Nagi M10 Calculator focus PASS\r\n".len()] =
    *b"Nagi M10 Calculator focus PASS\r\n";
#[no_mangle]
static NAGI_M10_NOTES: [u8; b"Nagi M10 Notes focus PASS\r\n".len()] =
    *b"Nagi M10 Notes focus PASS\r\n";
#[no_mangle]
static NAGI_M10_FILES: [u8; b"Nagi M10 Files focus PASS\r\n".len()] =
    *b"Nagi M10 Files focus PASS\r\n";
#[no_mangle]
static NAGI_M10_TERMINAL: [u8; b"Nagi M10 GUI Terminal focus PASS\r\n".len()] =
    *b"Nagi M10 GUI Terminal focus PASS\r\n";
#[no_mangle]
static NAGI_M10_JAPANESE: [u8; b"Nagi M10 Japanese input PASS\r\n".len()] =
    *b"Nagi M10 Japanese input PASS\r\n";
#[no_mangle]
static NAGI_M10_ACCEPTANCE: [u8; b"Nagi M10 acceptance PASS\r\n".len()] =
    *b"Nagi M10 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M10_FAIL: [u8; b"Nagi M10 acceptance FAIL\r\n".len()] =
    *b"Nagi M10 acceptance FAIL\r\n";
#[no_mangle]
static NAGI_M29_JAPANESE_SELECTED: [u8; b"Nagi M29 settings locale PASS locale=ja-JP\r\n".len()] =
    *b"Nagi M29 settings locale PASS locale=ja-JP\r\n";
#[no_mangle]
static NAGI_M29_ACCEPTANCE: [u8; b"Nagi M29 settings acceptance PASS\r\n".len()] =
    *b"Nagi M29 settings acceptance PASS\r\n";
#[no_mangle]
static NAGI_M29_KEYBOARD_LOCALE_SELECTION: [u8;
    b"Nagi M29 keyboard locale selection PASS locale=ja-JP\r\n".len()] =
    *b"Nagi M29 keyboard locale selection PASS locale=ja-JP\r\n";
#[no_mangle]
static NAGI_M29_DESKTOP_KEYBOARD_FOCUS: [u8; b"Nagi M29 desktop keyboard focus PASS\r\n".len()] =
    *b"Nagi M29 desktop keyboard focus PASS\r\n";
#[no_mangle]
static NAGI_M29_JAPANESE_PERSISTED: [u8;
    b"Nagi M29 settings locale persisted PASS locale=ja-JP\r\n".len()] =
    *b"Nagi M29 settings locale persisted PASS locale=ja-JP\r\n";
#[no_mangle]
static NAGI_M29_ENGLISH_PERSISTED: [u8;
    b"Nagi M29 settings locale persisted PASS locale=en-US\r\n".len()] =
    *b"Nagi M29 settings locale persisted PASS locale=en-US\r\n";
#[no_mangle]
static NAGI_M29_JAPANESE_RESTORED: [u8;
    b"Nagi M29 settings preference restored PASS locale=ja-JP\r\n".len()] =
    *b"Nagi M29 settings preference restored PASS locale=ja-JP\r\n";
#[no_mangle]
static NAGI_M29_ENGLISH_RESTORED: [u8;
    b"Nagi M29 settings preference restored PASS locale=en-US\r\n".len()] =
    *b"Nagi M29 settings preference restored PASS locale=en-US\r\n";
#[no_mangle]
static NAGI_SYSTEM_LANGUAGE_PREFERENCE_INVALID: [u8;
    b"Nagi system language preference invalid; using en-US\r\n".len()] =
    *b"Nagi system language preference invalid; using en-US\r\n";
#[no_mangle]
static NAGI_SYSTEM_LANGUAGE_PREFERENCE_UNAVAILABLE: [u8;
    b"Nagi system language preference unavailable; using en-US\r\n".len()] =
    *b"Nagi system language preference unavailable; using en-US\r\n";
#[no_mangle]
static NAGI_SETTINGS_LOCALE_PERSIST_FAIL: [u8; b"Nagi system language persistence FAIL\r\n".len()] =
    *b"Nagi system language persistence FAIL\r\n";

#[no_mangle]
static NOTES_KANA: [u8; 3] = *b"\xe3\x81\x82";
#[no_mangle]
static FILES_TEXT: [u8; 19] = *b"nagi-persistent.txt";
#[no_mangle]
static TERMINAL_TEXT: [u8; 9] = *b"$ nagi ps";

macro_rules! message {
    ($symbol:ident, $length:expr) => {{
        &$symbol[..$length]
    }};
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsFocus {
    Button,
    English,
    Japanese,
    #[cfg(feature = "consent-dialog")]
    Permissions,
    #[cfg(feature = "desktop-login")]
    Password,
}

/// The settings controls in focus order, for Tab and Up/Down.
const SETTINGS_ORDER: &[SettingsFocus] = &[
    SettingsFocus::English,
    SettingsFocus::Japanese,
    #[cfg(feature = "consent-dialog")]
    SettingsFocus::Permissions,
    #[cfg(feature = "desktop-login")]
    SettingsFocus::Password,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesktopFocus {
    SettingsButton,
    Application(usize),
}

#[derive(Debug)]
pub enum HostPreviewAttachError {
    Root(SandboxError),
    Notes(AppError),
}

impl core::fmt::Display for HostPreviewAttachError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Root(error) => write!(formatter, "Notes preview root: {error}"),
            Self::Notes(error) => write!(formatter, "Notes preview document: {error}"),
        }
    }
}

impl std::error::Error for HostPreviewAttachError {}

/// The control after (`forward`) or before `current` in [`SETTINGS_ORDER`],
/// wrapping. From the Settings button, forward starts at the first control
/// and backward at the last.
fn settings_neighbor(current: Option<SettingsFocus>, forward: bool) -> SettingsFocus {
    let last = SETTINGS_ORDER.len() - 1;
    let position = current.and_then(|focus| SETTINGS_ORDER.iter().position(|item| *item == focus));
    let index = match (position, forward) {
        (None, true) => 0,
        (None, false) => last,
        (Some(index), true) => (index + 1) % SETTINGS_ORDER.len(),
        (Some(index), false) => (index + last) % SETTINGS_ORDER.len(),
    };
    SETTINGS_ORDER[index]
}

pub struct Desktop {
    windows: [Rect; APP_COUNT],
    focused: [bool; APP_COUNT],
    desktop_focus: Option<DesktopFocus>,
    keyboard_app_focus: [bool; APP_COUNT],
    keyboard_focus_pass_printed: bool,
    pointer_x: i32,
    pointer_y: i32,
    notes_has_input: bool,
    notes_bridge: Option<NotesEventBridge>,
    notes_document: Option<NoteDocument>,
    notes_recovery_message: Option<(&'static str, Option<&'static str>, Option<&'static str>)>,
    notes_localization: RefCell<Option<(nagi_localization::Locale, NotesLocalization)>>,
    locale: nagi_localization::Locale,
    settings_open: bool,
    settings_focus: Option<SettingsFocus>,
    #[cfg(feature = "consent-dialog")]
    consent: Option<crate::consent_dialog::ConsentDialog>,
    #[cfg(feature = "consent-dialog")]
    consent_decision: Option<(libnagi::launch::ConsentRequest, libnagi::launch::GrantCheck)>,
    #[cfg(feature = "consent-dialog-acceptance")]
    consent_answered: bool,
    #[cfg(feature = "consent-dialog")]
    permissions: Option<crate::consent_settings::PermissionsPanel>,
    /// The OS-owned login screen, shown until the owner signs in.
    #[cfg(feature = "desktop-login")]
    login: Option<crate::login_screen::LoginScreen>,
    #[cfg(feature = "desktop-login")]
    session: Option<libnagi::security::Session>,
    /// The modal change-password form (ADR 0066).
    #[cfg(feature = "desktop-login")]
    password_change: Option<crate::login_screen::PasswordChangeScreen>,
    /// The ADR 0066 acceptance ends only after a successful saved change.
    #[cfg(feature = "desktop-password-change-acceptance")]
    password_change_succeeded: bool,
    /// Search input and the first visible result in the signed-in Files panel.
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_query: [u8; FILES_SEARCH_QUERY_CAPACITY],
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_panel: files_panel::Panel,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_query_len: usize,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_pending: bool,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_complete: bool,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_unavailable: bool,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_result_count: usize,
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_first_title: [u8; FILES_SEARCH_TITLE_CAPACITY],
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    files_search_first_title_len: usize,
}

impl Desktop {
    pub const fn new(locale: nagi_localization::Locale) -> Self {
        Self {
            windows: [
                Rect::new(8, 24, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(167, 24, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(8, 104, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(167, 104, WINDOW_WIDTH, WINDOW_HEIGHT),
            ],
            focused: [false; APP_COUNT],
            desktop_focus: None,
            keyboard_app_focus: [false; APP_COUNT],
            keyboard_focus_pass_printed: false,
            pointer_x: POINTER_START_X,
            pointer_y: POINTER_START_Y,
            notes_has_input: false,
            notes_bridge: None,
            notes_document: None,
            notes_recovery_message: None,
            notes_localization: RefCell::new(None),
            locale,
            settings_open: false,
            settings_focus: None,
            #[cfg(feature = "consent-dialog")]
            consent: None,
            #[cfg(feature = "consent-dialog")]
            consent_decision: None,
            #[cfg(feature = "consent-dialog-acceptance")]
            consent_answered: false,
            #[cfg(feature = "consent-dialog")]
            permissions: None,
            #[cfg(feature = "desktop-login")]
            login: None,
            #[cfg(feature = "desktop-login")]
            session: None,
            #[cfg(feature = "desktop-login")]
            password_change: None,
            #[cfg(feature = "desktop-password-change-acceptance")]
            password_change_succeeded: false,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_query: [0; FILES_SEARCH_QUERY_CAPACITY],
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_panel: files_panel::Panel::new(),
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_query_len: 0,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_pending: false,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_complete: false,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_unavailable: false,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_result_count: 0,
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_first_title: [0; FILES_SEARCH_TITLE_CAPACITY],
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            files_search_first_title_len: 0,
        }
    }

    /// Attach a real Notes controller to this desktop's Notes window.
    /// Bridge construction and any authorization remain with the caller; this
    /// method only connects the selected document to desktop state.
    pub fn attach_notes(
        &mut self,
        mut bridge: NotesEventBridge,
        note_id: ObjectId,
    ) -> Result<(), AppError> {
        let document = bridge.open_note(note_id)?;
        self.notes_bridge = Some(bridge);
        self.notes_document = Some(document);
        self.notes_has_input = false;
        Ok(())
    }

    pub fn current_notes_document(&self) -> Option<&NoteDocument> {
        self.notes_document.as_ref()
    }

    /// Present a localized recovery status in the existing Notes window.
    /// This is only a rendering attempt; it does not assert that a person saw
    /// or acknowledged the framebuffer output.
    pub fn set_notes_recovery_message(
        &mut self,
        message_id: Option<&'static str>,
        hint_id: Option<&'static str>,
    ) {
        self.set_notes_recovery_message_with_cause(message_id, hint_id, None);
    }

    /// Present a localized recovery status and a safe, pre-classified cause.
    /// Callers must pass a localization key, never raw I/O errors or paths.
    pub fn set_notes_recovery_message_with_cause(
        &mut self,
        message_id: Option<&'static str>,
        hint_id: Option<&'static str>,
        cause_id: Option<&'static str>,
    ) {
        self.notes_recovery_message = message_id.map(|message| (message, hint_id, cause_id));
        if message_id.is_some() {
            self.desktop_focus = Some(DesktopFocus::Application(1));
            self.focused[1] = true;
            self.keyboard_app_focus[1] = true;
        }
    }

    /// Make the final, single flush attempt against the currently attached
    /// Notes session before an input-loss recovery owner is created.
    pub fn flush_notes_before_input_loss(&mut self) -> Result<(), String> {
        let Some(bridge) = self.notes_bridge.as_ref() else {
            return Ok(());
        };
        match bridge.flush_active().map_err(|error| error.to_string())? {
            Some(document) => self.notes_document = Some(document),
            None => {}
        }
        Ok(())
    }

    /// Retry the same active Notes session after a verified input reconnect
    /// and an explicit user command.
    pub fn retry_notes_save(&mut self) -> Result<(), String> {
        let bridge = self
            .notes_bridge
            .as_ref()
            .ok_or_else(|| "Notes session is unavailable".to_owned())?;
        if let Some(document) = bridge.retry_active().map_err(|error| error.to_string())? {
            self.notes_document = Some(document);
        }
        Ok(())
    }

    #[cfg(feature = "linux-development-smoke")]
    pub fn linux_development_notes_window_active(&self) -> bool {
        self.focused[1] && self.keyboard_app_focus[1]
    }

    fn localized_notes_label(&self, message_id: &str) -> String {
        let mut cached = self.notes_localization.borrow_mut();
        if !matches!(cached.as_ref(), Some((locale, _)) if *locale == self.locale) {
            *cached = NotesLocalization::new(self.locale)
                .ok()
                .map(|localization| (self.locale, localization));
        }
        cached
            .as_ref()
            .and_then(|(_, localization)| localization.text(message_id).ok())
            .unwrap_or_else(|| nagi_localization::text(self.locale, message_id).to_owned())
    }

    /// Construct a desktop with an existing Notes bridge and selected note.
    pub fn new_with_notes(
        locale: nagi_localization::Locale,
        bridge: NotesEventBridge,
        note_id: ObjectId,
    ) -> Result<Self, AppError> {
        let mut desktop = Self::new(locale);
        desktop.attach_notes(bridge, note_id)?;
        Ok(desktop)
    }

    /// Development-only constructor rooted at an explicit HostPreviewStore path.
    /// This does not adapt the desktop's `UserDataVolume` or add authorization.
    pub fn new_with_host_preview_root(
        locale: nagi_localization::Locale,
        root: &Path,
        note_id: ObjectId,
    ) -> Result<Self, HostPreviewAttachError> {
        let store = Arc::new(HostPreviewStore::open(root).map_err(HostPreviewAttachError::Root)?);
        Self::new_with_notes(locale, NotesEventBridge::new(store), note_id)
            .map_err(HostPreviewAttachError::Notes)
    }

    pub fn render(&self, surface: &mut [u32]) {
        let mut painter = Painter::new(surface);
        #[cfg(feature = "desktop-login")]
        if let Some(login) = &self.login {
            login.render(&mut painter, self.locale);
            return;
        }
        painter.fill(Rect::new(0, 0, 320, 200), BACKGROUND);
        painter.fill(
            Rect::new(0, 0, 320, 20),
            color(PREVIEW_THEME, ColorRole::SurfaceSunken).to_pixel(),
        );
        painter.fill(
            SETTINGS_BUTTON,
            if self.settings_open { TITLE } else { PANEL },
        );
        painter.frame(
            SETTINGS_BUTTON,
            if self.settings_focus == Some(SettingsFocus::Button)
                || (!self.settings_open && self.desktop_focus == Some(DesktopFocus::SettingsButton))
            {
                FOCUS
            } else {
                BORDER
            },
        );
        painter.text(
            SETTINGS_BUTTON.x + 4,
            SETTINGS_BUTTON.y + 5,
            nagi_localization::text(self.locale, "desktop.settings.button").as_bytes(),
            TEXT,
        );
        self.render_app(
            &mut painter,
            0,
            nagi_localization::text(self.locale, "desktop.calculator.title").as_bytes(),
            b"1 + 2 = 3",
        );
        let notes_text = self.notes_document.as_ref().map(NoteDocument::search_text);
        let notes_default_content = self.localized_notes_label("desktop.notes.content");
        let notes_content = notes_text
            .as_ref()
            .map_or_else(|| notes_default_content.as_bytes(), String::as_bytes);
        let notes_title = self.localized_notes_label("desktop.notes.title");
        self.render_app(&mut painter, 1, notes_title.as_bytes(), notes_content);
        if let Some((message_id, hint_id, cause_id)) = self.notes_recovery_message {
            let notes_window = self.windows[1];
            let has_cause = cause_id.is_some();
            painter.text(
                notes_window.x + 5,
                notes_window.y + TITLE_HEIGHT + if has_cause { 27 } else { 37 },
                self.localized_notes_label(message_id).as_bytes(),
                FOCUS,
            );
            if let Some(hint_id) = hint_id {
                painter.text(
                    notes_window.x + 5,
                    notes_window.y + TITLE_HEIGHT + if has_cause { 38 } else { 48 },
                    self.localized_notes_label(hint_id).as_bytes(),
                    TEXT,
                );
            }
            if let Some(cause_id) = cause_id {
                if let Some(cause_y) = recovery_layout::cause_line_y(
                    notes_window.y,
                    notes_window.height,
                    libnagi::SURFACE_HEIGHT as i32,
                    TITLE_HEIGHT,
                    38,
                    49,
                    crate::font::GLYPH_HEIGHT,
                ) {
                    painter.text(
                        notes_window.x + 5,
                        cause_y,
                        self.localized_notes_label(cause_id).as_bytes(),
                        TEXT,
                    );
                }
            }
        }
        if self.notes_document.is_none() && self.notes_has_input {
            painter.text(176, 57, message!(NOTES_KANA, 3), TEXT);
        }
        #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
        {
            self.render_app(
                &mut painter,
                2,
                nagi_localization::text(self.locale, "desktop.files.title").as_bytes(),
                b"",
            );
            self.render_files_search(&mut painter);
        }
        #[cfg(not(all(feature = "m19-runtime", feature = "desktop-login")))]
        self.render_app(
            &mut painter,
            2,
            nagi_localization::text(self.locale, "desktop.files.title").as_bytes(),
            message!(FILES_TEXT, 19),
        );
        self.render_app(
            &mut painter,
            3,
            nagi_localization::text(self.locale, "desktop.terminal.title").as_bytes(),
            message!(TERMINAL_TEXT, 9),
        );
        #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
        if self.files_panel.open {
            self.render_files_panel(&mut painter);
        }
        if self.settings_open {
            self.render_settings(&mut painter);
        }
        #[cfg(feature = "consent-dialog")]
        if let Some(panel) = &self.permissions {
            panel.render(&mut painter, self.locale);
        }
        #[cfg(feature = "consent-dialog")]
        if let Some(dialog) = &self.consent {
            dialog.render(&mut painter, self.locale);
        }
        #[cfg(feature = "desktop-login")]
        if let Some(screen) = &self.password_change {
            screen.render(&mut painter, self.locale);
        }
        painter.fill(
            Rect::new(self.pointer_x - 1, self.pointer_y - 1, 3, 3),
            color(PREVIEW_THEME, ColorRole::Focus).to_pixel(),
        );
    }

    pub fn handle_event(&mut self, event: InputEvent, volume: &mut UserDataVolume) -> bool {
        #[cfg(feature = "desktop-login")]
        if let Some(login) = &mut self.login {
            use crate::login_screen::LoginOutcome;
            return match login.handle_event(event, volume) {
                LoginOutcome::Ignored => false,
                LoginOutcome::Changed => true,
                LoginOutcome::LanguageChosen(locale) => {
                    // M29 onboarding: the first choice becomes the system
                    // language for this and later boots.
                    self.locale = locale;
                    if persist_locale(volume, locale) {
                        print(b"Nagi onboarding language PASS locale=");
                        print(locale.code().as_bytes());
                        print(b"\r\n");
                    } else {
                        print(message!(NAGI_SETTINGS_LOCALE_PERSIST_FAIL, 39));
                    }
                    true
                }
                LoginOutcome::SignedIn(session) => {
                    self.login = None;
                    self.session = Some(session);
                    // Decisions belong to the signed-in owner: restore them
                    // and ask only now (ADR 0060/0063).
                    #[cfg(feature = "consent-dialog-acceptance")]
                    {
                        let start = start_consent_acceptance(self, volume, &session);
                        report_consent_start(self, start);
                    }
                    #[cfg(all(
                        feature = "consent-dialog",
                        not(feature = "consent-dialog-acceptance")
                    ))]
                    if crate::consent_dialog::restore_decisions(volume, &session)
                        == crate::consent_dialog::RestoredDecisions::Invalid
                    {
                        print(b"Nagi consent decisions invalid; every grant asks again\r\n");
                    }
                    // Readiness means a signed-in desktop (ADR 0063).
                    if libnagi::report_boot_ready() {
                        print(b"Nagi login readiness reported PASS\r\n");
                    } else {
                        print(b"Nagi login readiness report FAIL\r\n");
                    }
                    true
                }
            };
        }
        if event.event_type == libnagi::INPUT_EVENT_REL {
            if event.code == libnagi::INPUT_REL_X {
                self.pointer_x = clamp(self.pointer_x.saturating_add(event.value), 0, 319);
                return event.value != 0;
            }
            if event.code == libnagi::INPUT_REL_Y {
                self.pointer_y = clamp(self.pointer_y.saturating_add(event.value), 0, 199);
                return event.value != 0;
            }
        }
        #[cfg(feature = "desktop-login")]
        if self.password_change.is_some() {
            return self.handle_password_change_event(event, volume);
        }
        #[cfg(feature = "consent-dialog")]
        if self.consent.is_some() {
            return self.handle_consent_event(event, volume);
        }
        #[cfg(feature = "consent-dialog")]
        if self.permissions.is_some() {
            return self.handle_permissions_event(event, volume);
        }
        #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
        if event.event_type == libnagi::INPUT_EVENT_KEY && event.value != 0 && !self.settings_open {
            if self.files_panel.open {
                if event.code == libnagi::INPUT_KEY_LEFT {
                    self.click_files_panel();
                } else if event.value == 1 {
                    self.files_panel.key(event.code);
                }
                return true;
            }
            let window = self.windows[2];
            let manage = Rect::new(window.x + 82, window.y + TITLE_HEIGHT + 1, 57, 11);
            if (event.code == FILES_MANAGE_KEY
                && self.desktop_focus == Some(DesktopFocus::Application(2)))
                || (event.code == libnagi::INPUT_KEY_LEFT
                    && manage.contains(self.pointer_x, self.pointer_y))
            {
                self.desktop_focus = Some(DesktopFocus::Application(2));
                self.activate_application(2, false);
                self.files_panel.open();
                return true;
            }
        }
        if let Some(handled) =
            self.handle_keyboard_prefix(event, |desktop| desktop.activate_desktop_focus(volume))
        {
            return handled;
        }
        if event.event_type == libnagi::INPUT_EVENT_KEY && event.value != 0 {
            if event.code == libnagi::INPUT_KEY_LEFT {
                if SETTINGS_BUTTON.contains(self.pointer_x, self.pointer_y) {
                    self.desktop_focus = Some(DesktopFocus::SettingsButton);
                    self.settings_open = !self.settings_open;
                    self.settings_focus = Some(if self.settings_open {
                        SettingsFocus::English
                    } else {
                        SettingsFocus::Button
                    });
                    return true;
                }
                if self.settings_open {
                    if ENGLISH_OPTION.contains(self.pointer_x, self.pointer_y) {
                        return self.select_locale(volume, nagi_localization::Locale::EnUs, false);
                    }
                    if JAPANESE_OPTION.contains(self.pointer_x, self.pointer_y) {
                        return self.select_locale(volume, nagi_localization::Locale::JaJp, false);
                    }
                    #[cfg(feature = "desktop-login")]
                    if PASSWORD_OPTION.contains(self.pointer_x, self.pointer_y) {
                        self.settings_focus = Some(SettingsFocus::Password);
                        return self.open_password_change(volume);
                    }
                    if !SETTINGS_PANEL.contains(self.pointer_x, self.pointer_y) {
                        self.settings_open = false;
                        self.settings_focus = Some(SettingsFocus::Button);
                        self.desktop_focus = Some(DesktopFocus::SettingsButton);
                        return true;
                    }
                    return true;
                }
                if let Some(index) = self
                    .windows
                    .iter()
                    .position(|window| window.contains(self.pointer_x, self.pointer_y))
                {
                    self.desktop_focus = Some(DesktopFocus::Application(index));
                    self.settings_focus = None;
                    return self.activate_application(index, false);
                }
            } else if self.focused[1]
                && !self.notes_has_input
                && !matches!(
                    event.code,
                    libnagi::INPUT_KEY_ESCAPE
                        | libnagi::INPUT_KEY_TAB
                        | libnagi::INPUT_KEY_ENTER
                        | libnagi::INPUT_KEY_SPACE
                        | libnagi::INPUT_KEY_UP
                        | libnagi::INPUT_KEY_DOWN
                )
            {
                self.notes_has_input = true;
                print(message!(NAGI_M10_JAPANESE, 30));
                return true;
            }
        }
        false
    }

    /// Shared ordinary key dispatch used by the runtime handler. The caller
    /// supplies the activation action so this key-routing logic stays separate
    /// from the caller-owned volume authority.
    fn handle_keyboard_prefix(
        &mut self,
        event: InputEvent,
        activate_focus: impl FnOnce(&mut Self) -> bool,
    ) -> Option<bool> {
        if event.event_type != libnagi::INPUT_EVENT_KEY || event.value == 0 {
            return None;
        }
        if event.code == libnagi::INPUT_KEY_TAB {
            if self.settings_open {
                self.focus_next_settings_control();
            } else {
                self.focus_next_desktop_control();
            }
            return Some(true);
        }
        if event.code == libnagi::INPUT_KEY_UP || event.code == libnagi::INPUT_KEY_DOWN {
            return Some(self.move_settings_focus(event.code == libnagi::INPUT_KEY_DOWN));
        }
        if event.code == libnagi::INPUT_KEY_ESCAPE && self.settings_open {
            self.settings_open = false;
            self.settings_focus = Some(SettingsFocus::Button);
            self.desktop_focus = Some(DesktopFocus::SettingsButton);
            return Some(true);
        }
        #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
        if self.handle_files_search_key(event.code) {
            return Some(true);
        }
        if let Some(handled) = self.route_notes_event(event) {
            return Some(handled);
        }
        if event.code == libnagi::INPUT_KEY_ENTER || event.code == libnagi::INPUT_KEY_SPACE {
            return Some(activate_focus(self));
        }
        None
    }

    /// Select only key actions that belong to the active Notes editor. This
    /// is called from `handle_event` before the desktop's generic Enter action.
    fn route_notes_event(&mut self, event: InputEvent) -> Option<bool> {
        if event.event_type != libnagi::INPUT_EVENT_KEY
            || event.value == 0
            || self.settings_open
            || self.desktop_focus != Some(DesktopFocus::Application(1))
            || !self.focused[1]
            || self
                .notes_bridge
                .as_ref()
                .and_then(NotesEventBridge::current_note)
                .is_none()
            || matches!(
                event.code,
                libnagi::INPUT_KEY_LEFT
                    | libnagi::INPUT_KEY_TAB
                    | libnagi::INPUT_KEY_UP
                    | libnagi::INPUT_KEY_DOWN
            )
        {
            return None;
        }
        self.handle_notes_event(event)
    }

    /// Host-preview test entry; routes an actual ABI event through the same
    /// Notes branch while explicitly selecting the Notes window for the smoke.
    #[cfg(feature = "dev-host-preview")]
    pub fn dispatch_host_preview_event(&mut self, event: InputEvent) -> bool {
        self.desktop_focus = Some(DesktopFocus::Application(1));
        self.focused[1] = true;
        self.route_notes_event(event).unwrap_or(false)
    }

    /// Development-only evdev-record injection consumer. Framing, device opening,
    /// authorization, and live input polling remain with the external caller.
    #[cfg(feature = "dev-host-preview")]
    pub fn dispatch_host_preview_evdev_record(
        &mut self,
        target: LinuxEvdevTarget,
        record: &[u8],
    ) -> Result<bool, EvdevRecordError> {
        self.desktop_focus = Some(DesktopFocus::Application(1));
        let result = match self.notes_bridge.as_mut() {
            Some(bridge) => bridge.handle_evdev_record(target, record),
            None => return Ok(false),
        };
        let current = self
            .notes_bridge
            .as_ref()
            .and_then(NotesEventBridge::current_note);
        match result {
            Ok(Some(document)) => {
                self.notes_document = Some(document);
                self.notes_has_input = true;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(error @ EvdevRecordError::Decode(_)) => Err(error),
            Err(error @ EvdevRecordError::NeedsResync) => Err(error),
            Err(error @ EvdevRecordError::Notes(_)) => {
                if let Some(document) = current {
                    self.notes_document = Some(document);
                }
                self.notes_has_input = true;
                Err(error)
            }
        }
    }

    /// Poll one item from a caller-owned byte reader through the evdev stream,
    /// Notes bridge, and this Desktop's live document model.
    #[cfg(feature = "dev-host-preview")]
    pub fn poll_host_preview_evdev_stream<R: std::io::Read>(
        &mut self,
        stream: &mut NotesEventStream<R>,
    ) -> Result<NotesStreamPoll, NotesStreamError> {
        self.desktop_focus = Some(DesktopFocus::Application(1));
        let result = match self.notes_bridge.as_mut() {
            Some(bridge) => stream.poll(bridge),
            None => return Ok(NotesStreamPoll::Unhandled),
        };
        let current = self
            .notes_bridge
            .as_ref()
            .and_then(NotesEventBridge::current_note);
        match result {
            Ok(NotesStreamPoll::Updated(document)) => {
                self.notes_document = Some(document.clone());
                self.notes_has_input = true;
                Ok(NotesStreamPoll::Updated(document))
            }
            Ok(outcome) => Ok(outcome),
            Err(error @ NotesStreamError::Stream(_)) => Err(error),
            Err(error @ NotesStreamError::Notes(_)) => {
                if let Some(document) = current {
                    self.notes_document = Some(document);
                }
                self.notes_has_input = true;
                Err(error)
            }
        }
    }

    /// Apply one key action through the attached real Notes bridge and keep
    /// its latest in-memory document visible even when a save is rejected.
    fn handle_notes_event(&mut self, event: InputEvent) -> Option<bool> {
        let result = match self.notes_bridge.as_mut() {
            Some(bridge) => bridge.handle_event(event),
            None => return None,
        };
        let current = self
            .notes_bridge
            .as_ref()
            .and_then(NotesEventBridge::current_note);
        match result {
            Ok(Some(document)) => {
                self.notes_document = Some(document);
                self.notes_has_input = true;
                Some(true)
            }
            Ok(None) => None,
            Err(_) => {
                if let Some(document) = current {
                    self.notes_document = Some(document);
                }
                self.notes_has_input = true;
                Some(true)
            }
        }
    }

    #[cfg(feature = "desktop-login")]
    pub fn login_waiting(&self) -> bool {
        self.login.as_ref().is_some_and(|login| login.is_waiting())
    }

    /// Let the login screen notice that a sign-in wait has ended.
    #[cfg(feature = "desktop-login")]
    pub fn tick_login(&mut self) -> bool {
        self.login.as_mut().is_some_and(|login| login.tick())
    }

    pub fn acceptance_ready(&self) -> bool {
        #[cfg(feature = "desktop-password-change-acceptance")]
        {
            return self.session.is_some() && self.password_change_succeeded;
        }
        #[cfg(not(feature = "desktop-password-change-acceptance"))]
        {
            #[cfg(all(
                feature = "desktop-login-acceptance",
                not(feature = "m19-files-search-production")
            ))]
            if cfg!(feature = "desktop-login-acceptance") {
                return self.session.is_some();
            }
            #[cfg(feature = "consent-dialog-acceptance")]
            if self.consent_answered {
                return true;
            }
            let desktop_ready = self.focused.iter().all(|focused| *focused) && self.notes_has_input;
            if cfg!(feature = "consent-dialog-acceptance") {
                false
            } else if cfg!(feature = "m29-settings-acceptance") {
                desktop_ready
                    && self.settings_open
                    && self.locale == nagi_localization::Locale::JaJp
                    && !JAPANESE_OPTION.contains(self.pointer_x, self.pointer_y)
            } else {
                desktop_ready
            }
        }
    }

    /// Show the OS-owned consent dialog for `dialog`'s request.
    #[cfg(feature = "consent-dialog")]
    pub fn open_consent(&mut self, dialog: crate::consent_dialog::ConsentDialog) {
        self.consent = Some(dialog);
    }

    /// Accept input in the open dialog once its frame is on screen, and
    /// announce it then, so observers never act on an unpresented dialog.
    #[cfg(feature = "consent-dialog")]
    pub fn arm_consent(&mut self) {
        let Some(dialog) = &mut self.consent else {
            return;
        };
        if dialog.is_armed() {
            return;
        }
        dialog.arm();
        let request = dialog.request();
        print(b"Nagi consent dialog SHOWN app=");
        print(request.identifier());
        print(b" capability=");
        print(request.capability());
        print(b"\r\n");
    }

    #[cfg(feature = "consent-dialog")]
    pub fn take_consent_decision(
        &mut self,
    ) -> Option<(libnagi::launch::ConsentRequest, libnagi::launch::GrantCheck)> {
        self.consent_decision.take()
    }

    #[cfg(feature = "desktop-login")]
    pub fn signed_in_session(&self) -> Option<libnagi::security::Session> {
        self.session
    }

    #[cfg(feature = "desktop-login")]
    fn handle_password_change_event(
        &mut self,
        event: InputEvent,
        volume: &mut UserDataVolume,
    ) -> bool {
        use crate::login_screen::ChangeOutcome;
        let Some(screen) = &mut self.password_change else {
            return false;
        };
        match screen.handle_event(event, volume) {
            ChangeOutcome::Ignored => false,
            ChangeOutcome::Changed => true,
            ChangeOutcome::Cancelled => {
                self.password_change = None;
                true
            }
            ChangeOutcome::PasswordChanged => {
                self.password_change = None;
                #[cfg(feature = "desktop-password-change-acceptance")]
                {
                    self.password_change_succeeded = true;
                }
                true
            }
        }
    }

    #[cfg(feature = "consent-dialog")]
    fn handle_permissions_event(&mut self, event: InputEvent, volume: &mut UserDataVolume) -> bool {
        use crate::consent_settings::PermissionsOutcome;
        let user = self.signed_in_user();
        let Some(panel) = &mut self.permissions else {
            return false;
        };
        match panel.handle_event(event, user.as_ref(), volume) {
            PermissionsOutcome::Ignored => false,
            PermissionsOutcome::Changed => true,
            PermissionsOutcome::Closed => {
                self.permissions = None;
                true
            }
            PermissionsOutcome::Withdrawn => {
                #[cfg(feature = "consent-dialog-acceptance")]
                if crate::consent_dialog::acceptance::asks_again() {
                    print(b"Nagi consent withdrawn grant asks again PASS\r\n");
                    self.consent_answered = true;
                }
                true
            }
        }
    }

    /// The user whose decisions the consent dialog records: the signed-in
    /// owner with `desktop-login`, otherwise the acceptance fixture.
    #[cfg(feature = "consent-dialog")]
    fn signed_in_user(&self) -> Option<libnagi::security::Session> {
        #[cfg(feature = "desktop-login")]
        return self.session;
        #[cfg(not(feature = "desktop-login"))]
        crate::supervisor::acceptance_user()
    }

    #[cfg(feature = "consent-dialog")]
    fn handle_consent_event(&mut self, event: InputEvent, volume: &mut UserDataVolume) -> bool {
        use crate::consent_dialog::{self, ConsentAnswer, ConsentChoice, PromptOutcome};
        use libnagi::launch::GrantCheck;
        let Some(dialog) = &mut self.consent else {
            return false;
        };
        let answer = match dialog.handle_event(event, self.pointer_x, self.pointer_y) {
            PromptOutcome::Ignored => return false,
            PromptOutcome::Changed => return true,
            PromptOutcome::Answered(answer) => answer,
        };
        let request = dialog.request();
        self.consent = None;
        let Some(user) = self.signed_in_user() else {
            self.consent_decision = Some((request, GrantCheck::NotLive));
            #[cfg(feature = "consent-dialog-acceptance")]
            print(b"Nagi consent dialog acceptance FAIL user\r\n");
            return true;
        };
        let expected = match answer {
            ConsentAnswer::Chosen(ConsentChoice::Allow | ConsentChoice::AllowOnce) => {
                GrantCheck::Granted
            }
            ConsentAnswer::Chosen(ConsentChoice::Deny) => GrantCheck::Denied,
            ConsentAnswer::Dismissed => GrantCheck::ConsentRequired,
        };
        #[cfg(feature = "consent-dialog-acceptance")]
        let label: &[u8] = match answer {
            ConsentAnswer::Chosen(ConsentChoice::Allow) => b"allow",
            ConsentAnswer::Chosen(ConsentChoice::AllowOnce) => b"allow-once",
            ConsentAnswer::Chosen(ConsentChoice::Deny) => b"deny",
            ConsentAnswer::Dismissed => b"dismissed",
        };
        match consent_dialog::resolve(volume, &user, &request, answer) {
            Some(check) if check == expected => {
                self.consent_decision = Some((request, check));
                #[cfg(feature = "consent-dialog-acceptance")]
                print(b"Nagi consent dialog decision PASS decision=");
                #[cfg(feature = "consent-dialog-acceptance")]
                print(label);
                #[cfg(feature = "consent-dialog-acceptance")]
                print(b"\r\n");
                #[cfg(feature = "consent-dialog-acceptance")]
                if matches!(
                    answer,
                    ConsentAnswer::Chosen(ConsentChoice::Allow | ConsentChoice::Deny)
                ) {
                    print(b"Nagi consent decision persisted PASS\r\n");
                }
                // The restart half of the acceptance expects a persisted Allow.
                #[cfg(feature = "consent-dialog-acceptance")]
                if answer == ConsentAnswer::Chosen(ConsentChoice::Allow) {
                    self.consent_answered = true;
                }
            }
            _ => {
                self.consent_decision = Some((request, GrantCheck::NotLive));
                #[cfg(feature = "consent-dialog-acceptance")]
                print(b"Nagi consent dialog acceptance FAIL resolve\r\n");
            }
        }
        true
    }

    fn render_settings(&self, painter: &mut Painter<'_>) {
        painter.fill(SETTINGS_PANEL, PANEL);
        painter.frame(SETTINGS_PANEL, BORDER);
        painter.fill(
            Rect::new(
                SETTINGS_PANEL.x + 1,
                SETTINGS_PANEL.y + 1,
                SETTINGS_PANEL.width - 2,
                TITLE_HEIGHT + 2,
            ),
            TITLE,
        );
        painter.text(
            SETTINGS_PANEL.x + 8,
            SETTINGS_PANEL.y + 5,
            nagi_localization::text(self.locale, "desktop.settings.title").as_bytes(),
            PANEL,
        );
        painter.text(
            SETTINGS_PANEL.x + 14,
            SETTINGS_PANEL.y + 37,
            nagi_localization::text(self.locale, "desktop.settings.language").as_bytes(),
            TEXT,
        );
        self.render_locale_option(
            painter,
            ENGLISH_OPTION,
            nagi_localization::Locale::EnUs,
            "desktop.settings.option.en-US",
            SettingsFocus::English,
        );
        self.render_locale_option(
            painter,
            JAPANESE_OPTION,
            nagi_localization::Locale::JaJp,
            "desktop.settings.option.ja-JP",
            SettingsFocus::Japanese,
        );
        #[cfg(feature = "consent-dialog")]
        {
            painter.fill(
                PERMISSIONS_OPTION,
                color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
            );
            painter.frame(
                PERMISSIONS_OPTION,
                if self.settings_focus == Some(SettingsFocus::Permissions) {
                    FOCUS
                } else {
                    BORDER
                },
            );
            painter.text(
                PERMISSIONS_OPTION.x + 8,
                PERMISSIONS_OPTION.y + 5,
                nagi_localization::text(self.locale, "settings.permissions.title").as_bytes(),
                TEXT,
            );
        }
        #[cfg(feature = "desktop-login")]
        {
            painter.fill(
                PASSWORD_OPTION,
                color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
            );
            painter.frame(
                PASSWORD_OPTION,
                if self.settings_focus == Some(SettingsFocus::Password) {
                    FOCUS
                } else {
                    BORDER
                },
            );
            painter.text(
                PASSWORD_OPTION.x + 8,
                PASSWORD_OPTION.y + 5,
                nagi_localization::text(self.locale, "settings.password.title").as_bytes(),
                TEXT,
            );
        }
    }

    fn render_locale_option(
        &self,
        painter: &mut Painter<'_>,
        rect: Rect,
        locale: nagi_localization::Locale,
        key: &str,
        focus: SettingsFocus,
    ) {
        painter.fill(
            rect,
            color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
        );
        painter.frame(
            rect,
            if self.settings_focus == Some(focus) {
                FOCUS
            } else {
                BORDER
            },
        );
        let inner = Rect::new(rect.x + 2, rect.y + 2, rect.width - 4, rect.height - 4);
        painter.frame(inner, if self.locale == locale { TITLE } else { BORDER });
        let label_end = painter.text(
            rect.x + 8,
            rect.y + 8,
            nagi_localization::text(self.locale, key).as_bytes(),
            TEXT,
        );
        if self.locale == locale {
            painter.text(
                label_end + 8,
                rect.y + 8,
                nagi_localization::text(self.locale, "desktop.settings.option.selected").as_bytes(),
                TEXT,
            );
        }
    }

    fn focus_next_desktop_control(&mut self) {
        let next_focus = match self.desktop_focus {
            None => DesktopFocus::SettingsButton,
            Some(DesktopFocus::SettingsButton) => DesktopFocus::Application(0),
            Some(DesktopFocus::Application(index)) if index + 1 < APP_COUNT => {
                DesktopFocus::Application(index + 1)
            }
            Some(DesktopFocus::Application(_)) => DesktopFocus::SettingsButton,
        };
        self.desktop_focus = Some(next_focus);
        self.settings_focus = if next_focus == DesktopFocus::SettingsButton {
            Some(SettingsFocus::Button)
        } else {
            None
        };
    }

    fn focus_next_settings_control(&mut self) {
        self.settings_focus = Some(if self.settings_open {
            settings_neighbor(self.settings_focus, true)
        } else {
            SettingsFocus::Button
        });
    }

    fn activate_desktop_focus(&mut self, volume: &mut UserDataVolume) -> bool {
        if self.settings_open {
            return self.activate_settings_focus(volume, true);
        }
        match self.desktop_focus {
            Some(DesktopFocus::SettingsButton) => {
                self.settings_focus = Some(SettingsFocus::Button);
                self.activate_settings_focus(volume, true)
            }
            Some(DesktopFocus::Application(index)) => self.activate_application(index, true),
            None => false,
        }
    }

    fn activate_application(&mut self, index: usize, keyboard: bool) -> bool {
        if index >= APP_COUNT {
            return false;
        }
        if !self.focused[index] {
            self.focused[index] = true;
            match index {
                0 => print(message!(NAGI_M10_CALCULATOR, 32)),
                1 => print(message!(NAGI_M10_NOTES, 27)),
                2 => print(message!(NAGI_M10_FILES, 27)),
                3 => print(message!(NAGI_M10_TERMINAL, 34)),
                _ => return false,
            }
        }
        if keyboard {
            self.keyboard_app_focus[index] = true;
            if cfg!(feature = "m29-settings-acceptance")
                && !self.keyboard_focus_pass_printed
                && self.keyboard_app_focus.iter().all(|focused| *focused)
            {
                print(message!(
                    NAGI_M29_DESKTOP_KEYBOARD_FOCUS,
                    NAGI_M29_DESKTOP_KEYBOARD_FOCUS.len()
                ));
                self.keyboard_focus_pass_printed = true;
            }
        }
        true
    }

    fn move_settings_focus(&mut self, down: bool) -> bool {
        if !self.settings_open {
            return false;
        }
        self.settings_focus = Some(settings_neighbor(self.settings_focus, down));
        true
    }

    fn activate_settings_focus(&mut self, volume: &mut UserDataVolume, keyboard: bool) -> bool {
        match self.settings_focus {
            Some(SettingsFocus::Button) if !self.settings_open => {
                self.settings_open = true;
                self.settings_focus = Some(SettingsFocus::English);
                true
            }
            Some(SettingsFocus::English) if self.settings_open => {
                self.select_locale(volume, nagi_localization::Locale::EnUs, keyboard)
            }
            Some(SettingsFocus::Japanese) if self.settings_open => {
                self.select_locale(volume, nagi_localization::Locale::JaJp, keyboard)
            }
            #[cfg(feature = "consent-dialog")]
            Some(SettingsFocus::Permissions) if self.settings_open => {
                self.permissions = Some(crate::consent_settings::PermissionsPanel::open());
                true
            }
            #[cfg(feature = "desktop-login")]
            Some(SettingsFocus::Password) if self.settings_open => {
                self.open_password_change(volume)
            }
            _ => false,
        }
    }

    /// Open the change-password form for the signed-in owner.
    #[cfg(feature = "desktop-login")]
    fn open_password_change(&mut self, volume: &mut UserDataVolume) -> bool {
        if self.session.is_none() {
            return false;
        }
        match crate::login_screen::PasswordChangeScreen::open(volume) {
            Some(screen) => {
                self.password_change = Some(screen);
                print(b"Nagi password change READY\r\n");
                true
            }
            None => {
                print(b"Nagi password change FAIL account\r\n");
                false
            }
        }
    }

    fn select_locale(
        &mut self,
        volume: &mut UserDataVolume,
        locale: nagi_localization::Locale,
        keyboard: bool,
    ) -> bool {
        if !persist_locale(volume, locale) {
            print(message!(NAGI_SETTINGS_LOCALE_PERSIST_FAIL, 39));
            return false;
        }
        self.locale = locale;
        self.settings_focus = Some(match locale {
            nagi_localization::Locale::EnUs => SettingsFocus::English,
            nagi_localization::Locale::JaJp => SettingsFocus::Japanese,
        });
        if cfg!(feature = "m29-settings-acceptance") {
            if keyboard && locale == nagi_localization::Locale::JaJp {
                print(message!(
                    NAGI_M29_KEYBOARD_LOCALE_SELECTION,
                    NAGI_M29_KEYBOARD_LOCALE_SELECTION.len()
                ));
            }
            match locale {
                nagi_localization::Locale::EnUs => print(message!(
                    NAGI_M29_ENGLISH_PERSISTED,
                    NAGI_M29_ENGLISH_PERSISTED.len()
                )),
                nagi_localization::Locale::JaJp => {
                    print(message!(
                        NAGI_M29_JAPANESE_PERSISTED,
                        NAGI_M29_JAPANESE_PERSISTED.len()
                    ));
                    print(message!(NAGI_M29_JAPANESE_SELECTED, 44));
                }
            }
        }
        true
    }

    fn render_app(&self, painter: &mut Painter<'_>, index: usize, title: &[u8], content: &[u8]) {
        let window = self.windows[index];
        painter.fill(window, PANEL);
        let keyboard_focused = self.desktop_focus == Some(DesktopFocus::Application(index));
        painter.frame(
            window,
            if keyboard_focused {
                FOCUS
            } else if self.focused[index] {
                TITLE
            } else {
                BORDER
            },
        );
        painter.fill(
            Rect::new(window.x + 1, window.y + 1, window.width - 2, TITLE_HEIGHT),
            TITLE,
        );
        painter.text(window.x + 6, window.y + 4, title, TITLE_TEXT);
        painter.text(window.x + 8, window.y + TITLE_HEIGHT + 12, content, TEXT);
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn render_files_panel(&self, painter: &mut Painter<'_>) {
        use files_panel::Focus;
        let panel = &self.files_panel;
        painter.fill(FILES_PANEL, PANEL);
        painter.frame(FILES_PANEL, BORDER);
        painter.text(
            24,
            31,
            nagi_localization::text(self.locale, "desktop.files.title").as_bytes(),
            TEXT,
        );
        let mut button = |rect: Rect, key: &str, focus: Focus| {
            painter.fill(rect, PANEL);
            painter.frame(rect, if panel.focus == focus { FOCUS } else { BORDER });
            painter.text(
                rect.x + 3,
                rect.y + 3,
                nagi_localization::text(self.locale, key).as_bytes(),
                TEXT,
            );
        };
        if panel.editing {
            button(
                Rect::new(24, 162, 90, 14),
                "desktop.files.apply",
                Focus::Apply,
            );
            button(
                Rect::new(123, 162, 90, 14),
                "desktop.files.cancel",
                Focus::Cancel,
            );
        } else {
            button(
                Rect::new(24, 45, 125, 14),
                if panel.trash_view {
                    "desktop.files.show_files"
                } else {
                    "desktop.files.show_trash"
                },
                Focus::View,
            );
            if !panel.trash_view {
                button(Rect::new(24, 162, 80, 14), "desktop.files.new", Focus::New);
                button(
                    Rect::new(114, 162, 80, 14),
                    "desktop.files.rename",
                    Focus::Rename,
                );
            }
            button(
                Rect::new(204, 162, 91, 14),
                if panel.trash_view {
                    "desktop.files.restore"
                } else {
                    "desktop.files.trash"
                },
                Focus::Trash,
            );
        }
        let first_row = panel.selected.saturating_sub(7);
        for (row, entry) in panel.entries.iter().enumerate().skip(first_row).take(8) {
            let rect = Rect::new(24, 63 + (row - first_row) as i32 * 10, 271, 10);
            if row == panel.selected {
                painter.fill(
                    rect,
                    color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
                );
                painter.frame(
                    rect,
                    if panel.focus == Focus::List {
                        FOCUS
                    } else {
                        BORDER
                    },
                );
            }
            painter.text(rect.x + 3, rect.y + 1, entry.name.as_bytes(), TEXT);
        }
        if panel.entries.is_empty() && !panel.editing {
            painter.text(
                27,
                66,
                nagi_localization::text(self.locale, "desktop.files.list_empty").as_bytes(),
                TEXT,
            );
        }
        if panel.editing {
            let field = Rect::new(24, 145, 271, 14);
            painter.fill(
                field,
                color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
            );
            painter.frame(
                field,
                if panel.focus == Focus::Name {
                    FOCUS
                } else {
                    BORDER
                },
            );
            painter.text(field.x + 3, field.y + 3, panel.name.as_bytes(), TEXT);
        }
        if !panel.status.is_empty() {
            painter.text(
                24,
                180,
                nagi_localization::text(self.locale, panel.status).as_bytes(),
                TEXT,
            );
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn click_files_panel(&mut self) {
        use files_panel::Focus;
        let x = self.pointer_x;
        let y = self.pointer_y;
        if self.files_panel.editing {
            for (rect, focus) in [
                (Rect::new(24, 145, 271, 14), Focus::Name),
                (Rect::new(24, 162, 90, 14), Focus::Apply),
                (Rect::new(123, 162, 90, 14), Focus::Cancel),
            ] {
                if rect.contains(x, y) {
                    if focus == Focus::Name {
                        self.files_panel.focus = focus;
                    } else {
                        self.files_panel.activate(focus);
                    }
                    return;
                }
            }
        } else {
            if Rect::new(24, 45, 125, 14).contains(x, y) {
                self.files_panel.activate(Focus::View);
                return;
            }
            let first_row = self.files_panel.selected.saturating_sub(7);
            for row in 0..self
                .files_panel
                .entries
                .len()
                .saturating_sub(first_row)
                .min(8)
            {
                if Rect::new(24, 63 + row as i32 * 10, 271, 10).contains(x, y) {
                    self.files_panel.selected = first_row + row;
                    self.files_panel.focus = Focus::List;
                    return;
                }
            }
            for (rect, focus) in [
                (Rect::new(24, 162, 80, 14), Focus::New),
                (Rect::new(114, 162, 80, 14), Focus::Rename),
                (Rect::new(204, 162, 91, 14), Focus::Trash),
            ] {
                if rect.contains(x, y) {
                    self.files_panel.activate(focus);
                    return;
                }
            }
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn process_files_operations(
        &mut self,
        volume: &mut UserDataVolume,
        runtime: Option<&mut crate::m19_runtime::Runtime>,
    ) {
        use files_panel::Operation;
        if self.session.is_none() {
            return;
        }
        if let Some(operation) = self.files_panel.pending.take() {
            let (result, marker) = match operation {
                Operation::Create(name) => {
                    (files::create(volume, name.as_bytes()).map(|_| ()), "create")
                }
                Operation::Rename(entry, name) => (
                    files::rename(volume, &entry, name.as_bytes()).map(|_| ()),
                    "rename",
                ),
                Operation::Trash(entry) => (files::trash(volume, &entry), "trash"),
                Operation::Restore(entry) => (files::restore(volume, &entry), "restore"),
            };
            self.files_panel.completed(result);
            self.clear_files_search();
            // Indexing failure cannot undo or block a physical Files operation.
            // Every sync starts fail-closed, so stale IDs never reach Search.
            let synchronized = runtime.is_some_and(|runtime| runtime.sync_files(volume).is_ok());
            if result.is_ok() {
                print(b"Nagi Files UI operation PASS operation=");
                print(marker.as_bytes());
                print(b"\r\n");
                if !synchronized {
                    self.files_panel.status = "desktop.files.operation.saved_search_pending";
                }
            }
        }
        if self.files_panel.open && self.files_panel.refresh {
            self.files_panel.refresh = false;
            let result = files::initialize(volume).and_then(|()| {
                if self.files_panel.trash_view {
                    files::trash_entries(volume)
                } else {
                    files::list(volume)
                }
            });
            match result {
                Ok(entries) => self.files_panel.replace_entries(entries),
                Err(error) => {
                    self.files_panel.replace_entries(alloc::vec::Vec::new());
                    self.files_panel.completed(Err(error));
                    self.files_panel.refresh = false;
                }
            }
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn render_files_search(&self, painter: &mut Painter<'_>) {
        let window = self.windows[2];
        let label_y = window.y + TITLE_HEIGHT + 3;
        let field = Rect::new(window.x + 7, label_y + 10, window.width - 14, 12);
        painter.text(
            window.x + 7,
            label_y,
            nagi_localization::text(self.locale, "desktop.files.search.label").as_bytes(),
            TEXT,
        );
        painter.fill(
            field,
            color(PREVIEW_THEME, ColorRole::SurfaceRaised).to_pixel(),
        );
        painter.frame(
            field,
            if self.desktop_focus == Some(DesktopFocus::Application(2)) {
                FOCUS
            } else {
                BORDER
            },
        );
        let query = &self.files_search_query[..self.files_search_query_len];
        if query.is_empty() {
            painter.text(
                field.x + 3,
                field.y + 2,
                nagi_localization::text(self.locale, "desktop.files.search.hint").as_bytes(),
                TEXT,
            );
        } else {
            painter.text(field.x + 3, field.y + 2, query, TEXT);
        }

        let manage = Rect::new(window.x + 82, label_y - 2, 57, 11);
        painter.fill(manage, PANEL);
        painter.frame(manage, BORDER);
        painter.text(
            manage.x + 2,
            manage.y + 2,
            nagi_localization::text(self.locale, "desktop.files.manage").as_bytes(),
            TEXT,
        );

        let status_y = field.y + field.height + 3;
        if self.files_search_unavailable {
            painter.text(
                window.x + 7,
                status_y,
                nagi_localization::text(self.locale, "desktop.files.search.unavailable").as_bytes(),
                TEXT,
            );
        } else if self.files_search_complete && self.files_search_result_count == 0 {
            painter.text(
                window.x + 7,
                status_y,
                nagi_localization::text(self.locale, "desktop.files.search.empty").as_bytes(),
                TEXT,
            );
        } else if self.files_search_complete {
            let mut count_label = [0; 24];
            let mut count_length = append(
                &mut count_label,
                0,
                nagi_localization::text(self.locale, "desktop.files.search.matches").as_bytes(),
            );
            count_length = append(&mut count_label, count_length, b" ");
            count_length = append_decimal(
                &mut count_label,
                count_length,
                self.files_search_result_count as u64,
            );
            painter.text(window.x + 7, status_y, &count_label[..count_length], TEXT);
            painter.text(
                window.x + 7,
                status_y + 8,
                &self.files_search_first_title[..self.files_search_first_title_len],
                TEXT,
            );
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn handle_files_search_key(&mut self, code: u16) -> bool {
        if self.settings_open || self.desktop_focus != Some(DesktopFocus::Application(2)) {
            return false;
        }
        match code {
            libnagi::INPUT_KEY_ENTER => {
                if !self.focused[2] {
                    return self.activate_application(2, true);
                }
                if self.files_search_query_len == 0 {
                    self.files_search_pending = false;
                    self.files_search_complete = true;
                    self.files_search_unavailable = false;
                    self.files_search_result_count = 0;
                } else {
                    self.files_search_pending = true;
                    self.files_search_complete = false;
                    self.files_search_unavailable = false;
                }
                true
            }
            libnagi::INPUT_KEY_ESCAPE => {
                self.clear_files_search();
                true
            }
            libnagi::login::INPUT_KEY_BACKSPACE => {
                if self.files_search_query_len > 0 {
                    self.files_search_query_len -= 1;
                    self.files_search_query[self.files_search_query_len] = 0;
                    self.reset_files_search_results();
                }
                true
            }
            _ => {
                let Some(byte) = libnagi::login::key_char(code) else {
                    return false;
                };
                if self.files_search_query_len < self.files_search_query.len() {
                    self.files_search_query[self.files_search_query_len] = byte;
                    self.files_search_query_len += 1;
                    self.reset_files_search_results();
                }
                true
            }
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn clear_files_search(&mut self) {
        self.files_search_query = [0; FILES_SEARCH_QUERY_CAPACITY];
        self.files_search_query_len = 0;
        self.files_search_pending = false;
        self.reset_files_search_results();
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn reset_files_search_results(&mut self) {
        self.files_search_complete = false;
        self.files_search_unavailable = false;
        self.files_search_result_count = 0;
        self.files_search_first_title = [0; FILES_SEARCH_TITLE_CAPACITY];
        self.files_search_first_title_len = 0;
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn take_files_search_request(&mut self) -> Option<String> {
        if !self.files_search_pending {
            return None;
        }
        self.files_search_pending = false;
        let query =
            core::str::from_utf8(&self.files_search_query[..self.files_search_query_len]).ok()?;
        Some(String::from(query))
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn set_files_search_results(&mut self, titles: &[String]) {
        self.set_files_search_results_with_count(titles, titles.len());
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn set_files_search_results_with_count(&mut self, titles: &[String], visible_total: usize) {
        self.files_search_complete = true;
        self.files_search_unavailable = false;
        self.files_search_result_count = visible_total;
        self.files_search_first_title_len = 0;
        if let Some(title) = titles.first() {
            self.files_search_first_title_len =
                copy_utf8_prefix(&mut self.files_search_first_title, title);
        }
    }

    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    fn set_files_search_unavailable(&mut self) {
        self.files_search_complete = false;
        self.files_search_unavailable = true;
        self.files_search_result_count = 0;
        self.files_search_first_title_len = 0;
    }
}

pub fn run(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    volume: UserDataVolume,
) -> ! {
    run_with_notes_bridge(
        block_capability,
        display_capability,
        input_capability,
        volume,
        None,
    )
}

/// Existing desktop event loop with an optional caller-supplied Notes bridge.
/// The caller remains responsible for provider construction and authorization.
pub fn run_with_notes_bridge(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    mut volume: UserDataVolume,
    notes: Option<(NotesEventBridge, ObjectId)>,
) -> ! {
    #[cfg(not(all(feature = "m19-runtime", feature = "desktop-login")))]
    let _ = block_capability;
    let mut info = DisplayInfo::default();
    if !libnagi::display_info(&mut info)
        || info.surface_bytes as usize != libnagi::SURFACE_BYTES
        || info.width != libnagi::SURFACE_WIDTH
        || info.height != libnagi::SURFACE_HEIGHT
    {
        print(message!(NAGI_M10_FAIL, 26));
        libnagi::exit(1);
    }
    let surface = unsafe {
        core::slice::from_raw_parts_mut(
            info.surface_address as *mut u32,
            libnagi::SURFACE_BYTES / core::mem::size_of::<u32>(),
        )
    };
    let preference = load_locale(&mut volume);
    let mut desktop = match notes {
        Some((bridge, note_id)) => {
            match Desktop::new_with_notes(preference.locale(), bridge, note_id) {
                Ok(desktop) => desktop,
                Err(_) => {
                    print(message!(NAGI_M10_FAIL, 26));
                    libnagi::exit(1);
                }
            }
        }
        None => Desktop::new(preference.locale()),
    };
    #[cfg(feature = "desktop-login")]
    let login_mode = {
        let screen = crate::login_screen::LoginScreen::new(crate::login_screen::load(&mut volume));
        let mode = screen.mode();
        let mut screen = screen;
        screen.focus_language(preference.locale());
        screen.resume_throttle(&mut volume);
        desktop.login = Some(screen);
        mode
    };
    #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
    let mut search_runtime: Option<crate::m19_runtime::Runtime> = None;
    #[cfg(all(
        feature = "m19-runtime",
        feature = "desktop-login",
        not(feature = "desktop-login-acceptance")
    ))]
    let mut search_sync_failure_reported = false;
    #[cfg(all(
        feature = "m19-runtime",
        feature = "desktop-login",
        not(feature = "desktop-login-acceptance")
    ))]
    let mut search_startup_failure_reported = false;
    #[cfg(all(feature = "consent-dialog-acceptance", not(feature = "desktop-login")))]
    let consent_start = match crate::supervisor::acceptance_user() {
        Some(user) => start_consent_acceptance(&mut desktop, &mut volume, &user),
        None => ConsentStart {
            restored: crate::consent_dialog::RestoredDecisions::None,
            outcome: Err(b"user"),
        },
    };
    desktop.render(surface);
    if !libnagi::display_present(display_capability) {
        print(message!(NAGI_M10_FAIL, 26));
        libnagi::exit(1);
    }
    // With sign-in, readiness is reported only after the owner signs in.
    if !cfg!(feature = "desktop-login") && !libnagi::report_boot_ready() {
        print(message!(NAGI_M10_FAIL, 26));
        libnagi::exit(1);
    }
    #[cfg(feature = "consent-dialog")]
    desktop.arm_consent();
    let initial_checksum = checksum(surface);
    // Compare every pixel: a moved 3x3 pointer can miss the sampled checksum.
    let initial_frame = frame_hash(surface);
    print(message!(NAGI_M10_READY, 24));
    print_checksum(initial_checksum);
    #[cfg(feature = "desktop-login")]
    print(match login_mode {
        libnagi::login::LoginMode::Create => b"Nagi login READY mode=create\r\n",
        libnagi::login::LoginMode::Unlock => b"Nagi login READY mode=unlock\r\n",
    });
    #[cfg(all(feature = "consent-dialog-acceptance", not(feature = "desktop-login")))]
    report_consent_start(&mut desktop, consent_start);
    match preference {
        LocalePreference::Restored(nagi_localization::Locale::EnUs)
            if cfg!(any(
                feature = "m29-settings-acceptance",
                feature = "desktop-login-acceptance"
            )) =>
        {
            print(message!(
                NAGI_M29_ENGLISH_RESTORED,
                NAGI_M29_ENGLISH_RESTORED.len()
            ));
        }
        LocalePreference::Restored(nagi_localization::Locale::JaJp)
            if cfg!(any(
                feature = "m29-settings-acceptance",
                feature = "desktop-login-acceptance"
            )) =>
        {
            print(message!(
                NAGI_M29_JAPANESE_RESTORED,
                NAGI_M29_JAPANESE_RESTORED.len()
            ));
        }
        LocalePreference::Invalid => {
            print(message!(
                NAGI_SYSTEM_LANGUAGE_PREFERENCE_INVALID,
                NAGI_SYSTEM_LANGUAGE_PREFERENCE_INVALID.len()
            ));
        }
        LocalePreference::Unavailable => {
            print(message!(
                NAGI_SYSTEM_LANGUAGE_PREFERENCE_UNAVAILABLE,
                NAGI_SYSTEM_LANGUAGE_PREFERENCE_UNAVAILABLE.len()
            ));
        }
        LocalePreference::Restored(_) | LocalePreference::Missing => {}
    }
    loop {
        let mut event = InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            #[cfg(feature = "desktop-login")]
            if desktop.login_waiting() && desktop.tick_login() {
                desktop.render(surface);
                if !libnagi::display_present(display_capability) {
                    print(message!(NAGI_M10_FAIL, 26));
                    libnagi::exit(1);
                }
            }
            #[cfg(target_arch = "x86_64")]
            unsafe {
                asm!("pause", options(nomem, nostack, preserves_flags))
            };
            #[cfg(not(target_arch = "x86_64"))]
            core::hint::spin_loop();
            continue;
        }
        if desktop.handle_event(event, &mut volume) {
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            if desktop.session.is_some() {
                if search_runtime.is_none() {
                    match crate::m19_runtime::Runtime::open(block_capability, &mut volume) {
                        Ok(mut runtime) => {
                            #[cfg(feature = "desktop-login-acceptance")]
                            {
                                if runtime
                                    .acceptance_query(".nagi-m19-runtime-search.txt")
                                    .is_none()
                                {
                                    print(b"Nagi M19 signed-in desktop Files query FAIL\r\n");
                                    libnagi::exit(1);
                                }
                                print(b"Nagi M19 signed-in desktop Files query PASS\r\n");
                                if runtime.acceptance_verify_nested_search(&mut volume) {
                                    print(
                                        b"Nagi M19 signed-in desktop nested Files Search PASS\r\n",
                                    );
                                } else {
                                    print(
                                        b"Nagi M19 signed-in desktop nested Files Search FAIL\r\n",
                                    );
                                    libnagi::exit(1);
                                }
                                if runtime
                                    .search_files("\u{fffd}")
                                    .is_some_and(|object_ids| object_ids.is_empty())
                                {
                                    print(
                                        b"Nagi M19 signed-in desktop non-UTF-8 filename isolation PASS\r\n",
                                    );
                                } else {
                                    print(
                                        b"Nagi M19 signed-in desktop non-UTF-8 filename isolation FAIL\r\n",
                                    );
                                    libnagi::exit(1);
                                }
                                if runtime.acceptance_record_restored() {
                                    print(
                                        b"Nagi M19 signed-in desktop Files ObjectId restore PASS\r\n",
                                    );
                                } else {
                                    print(
                                        b"Nagi M19 signed-in desktop Files ObjectId initial persist PASS\r\n",
                                    );
                                }
                                if !runtime.acceptance_verify_rename(&mut volume) {
                                    print(
                                        b"Nagi M19 signed-in desktop Files rename identity FAIL\r\n",
                                    );
                                    libnagi::exit(1);
                                }
                                print(b"Nagi M19 signed-in desktop Files rename identity PASS\r\n");
                                if !runtime.acceptance_verify_file_lifecycle(&mut volume) {
                                    print(
                                        b"Nagi M19 signed-in desktop Files delete identity FAIL\r\n",
                                    );
                                    libnagi::exit(1);
                                }
                                print(b"Nagi M19 signed-in desktop Files delete identity PASS\r\n");
                            }
                            print(b"Nagi M19 signed-in desktop SearchService ready PASS\r\n");
                            search_runtime = Some(runtime);
                            #[cfg(not(feature = "desktop-login-acceptance"))]
                            {
                                search_startup_failure_reported = false;
                                search_sync_failure_reported = false;
                            }
                        }
                        Err(_) => {
                            #[cfg(feature = "desktop-login-acceptance")]
                            {
                                print(b"Nagi M19 signed-in desktop SearchService unavailable\r\n");
                                libnagi::exit(1);
                            }
                            #[cfg(not(feature = "desktop-login-acceptance"))]
                            if !search_startup_failure_reported {
                                print(b"Nagi M19 signed-in desktop SearchService unavailable\r\n");
                                search_startup_failure_reported = true;
                            }
                        }
                    }
                }
                desktop.process_files_operations(&mut volume, search_runtime.as_mut());
                if let Some(query) = desktop.take_files_search_request() {
                    if let Some(runtime) = search_runtime.as_mut() {
                        match runtime.sync_files(&mut volume) {
                            Ok(_) => {
                                #[cfg(not(feature = "desktop-login-acceptance"))]
                                {
                                    search_sync_failure_reported = false;
                                }
                            }
                            #[cfg(feature = "desktop-login-acceptance")]
                            Err(_) => {
                                print(b"Nagi M19 signed-in desktop Files sync FAIL\r\n");
                                libnagi::exit(1);
                            }
                            #[cfg(not(feature = "desktop-login-acceptance"))]
                            Err(_) => {
                                if !search_sync_failure_reported {
                                    print(b"Nagi M19 signed-in desktop Files sync unavailable\r\n");
                                    search_sync_failure_reported = true;
                                }
                                desktop.set_files_search_unavailable();
                            }
                        }
                        let synced = {
                            #[cfg(feature = "desktop-login-acceptance")]
                            {
                                true
                            }
                            #[cfg(not(feature = "desktop-login-acceptance"))]
                            {
                                !search_sync_failure_reported
                            }
                        };
                        if synced {
                            #[cfg(feature = "m19-files-search-production")]
                            {
                                let response = crate::m19_files_client::search(
                                    runtime,
                                    &query,
                                    &mut desktop,
                                    input_capability,
                                    display_capability,
                                    surface,
                                    &mut volume,
                                );
                                let mapped = response.and_then(|results| {
                                    runtime
                                        .visible_file_titles(results.ids())
                                        .map(|titles| (results, titles))
                                });
                                if let Some((results, titles)) = mapped {
                                    desktop.set_files_search_results_with_count(
                                        &titles,
                                        usize::from(results.visible_total),
                                    );
                                    #[cfg(feature = "desktop-login-acceptance")]
                                    {
                                        let expected = match query.as_str() {
                                            "runtime" => Some((
                                                ".nagi-m19-runtime-search.txt",
                                                "Nagi M19 signed-in desktop Files UI Search PASS\r\n",
                                            )),
                                            "nested" => Some((
                                                ".nagi-m19-nested-search.txt",
                                                "Nagi M19 signed-in desktop Files nested UI Search PASS\r\n",
                                            )),
                                            _ => None,
                                        };
                                        if let Some((expected_title, marker)) = expected {
                                            if titles
                                                .first()
                                                .is_some_and(|title| title == expected_title)
                                                && results.count > 0
                                            {
                                                print(marker.as_bytes());
                                                crate::m19_files_client::report_visible_object_id(
                                                    results.ids()[0],
                                                );
                                            } else {
                                                print(b"Nagi M19 signed-in desktop Files UI Search FAIL\r\n");
                                                libnagi::exit(1);
                                            }
                                        }
                                    }
                                } else {
                                    #[cfg(feature = "desktop-login-acceptance")]
                                    {
                                        print(
                                            b"Nagi M19 signed-in desktop Files UI Search FAIL\r\n",
                                        );
                                        libnagi::exit(1);
                                    }
                                    #[cfg(not(feature = "desktop-login-acceptance"))]
                                    desktop.set_files_search_unavailable();
                                }
                            }
                            #[cfg(not(feature = "m19-files-search-production"))]
                            if let Some(titles) = runtime.search_file_titles(&query) {
                                desktop.set_files_search_results(&titles);
                                #[cfg(feature = "desktop-login-acceptance")]
                                {
                                    let expected = match query.as_str() {
                                        "runtime" => Some((
                                            ".nagi-m19-runtime-search.txt",
                                            "Nagi M19 signed-in desktop Files UI Search PASS\r\n",
                                        )),
                                        "nested" => Some((
                                            ".nagi-m19-nested-search.txt",
                                            "Nagi M19 signed-in desktop Files nested UI Search PASS\r\n",
                                        )),
                                        _ => None,
                                    };
                                    if let Some((expected_title, marker)) = expected {
                                        if titles
                                            .first()
                                            .is_some_and(|title| title == expected_title)
                                        {
                                            print(marker.as_bytes());
                                        } else {
                                            print(b"Nagi M19 signed-in desktop Files UI Search FAIL\r\n");
                                            libnagi::exit(1);
                                        }
                                    }
                                }
                            } else {
                                #[cfg(feature = "desktop-login-acceptance")]
                                {
                                    print(b"Nagi M19 signed-in desktop Files UI Search FAIL\r\n");
                                    libnagi::exit(1);
                                }
                                #[cfg(not(feature = "desktop-login-acceptance"))]
                                {
                                    if !search_startup_failure_reported {
                                        print(b"Nagi M19 signed-in desktop Search unavailable\r\n");
                                        search_startup_failure_reported = true;
                                    }
                                    desktop.set_files_search_unavailable();
                                }
                            }
                        }
                    } else {
                        desktop.set_files_search_unavailable();
                    }
                }
            }
            desktop.render(surface);
            if !libnagi::display_present(display_capability) {
                print(message!(NAGI_M10_FAIL, 26));
                libnagi::exit(1);
            }
            // A dialog opened by this event accepts input only now that its
            // frame is on screen.
            #[cfg(feature = "consent-dialog")]
            desktop.arm_consent();
            #[cfg(feature = "consent-dialog")]
            if let Some(panel) = &mut desktop.permissions {
                panel.announce();
            }
            if frame_hash(surface) == initial_frame {
                print(message!(NAGI_M10_FAIL, 26));
                libnagi::exit(1);
            }
        }
        if desktop.acceptance_ready() {
            if cfg!(feature = "desktop-login-acceptance") {
                print(b"Nagi login acceptance PASS\r\n");
            } else if cfg!(feature = "consent-dialog-acceptance") {
                print(b"Nagi consent dialog acceptance PASS\r\n");
            } else {
                print(message!(NAGI_M10_ACCEPTANCE, 26));
            }
            if cfg!(feature = "m29-settings-acceptance") {
                print(message!(NAGI_M29_ACCEPTANCE, 35));
            }
            loop {
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    asm!("hlt", options(nomem, nostack, preserves_flags))
                };
                #[cfg(not(target_arch = "x86_64"))]
                core::hint::spin_loop();
            }
        }
    }
}

/// Development entry point using an explicit `HostPreviewStore` directory.
/// This does not treat the host root as the desktop's production UserDataVolume.
pub fn run_with_host_preview_root(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    volume: UserDataVolume,
    root: &Path,
    note_id: ObjectId,
) -> ! {
    let store = match HostPreviewStore::open(root) {
        Ok(store) => Arc::new(store),
        Err(_) => {
            print(message!(NAGI_M10_FAIL, 26));
            libnagi::exit(1);
        }
    };
    run_with_notes_bridge(
        block_capability,
        display_capability,
        input_capability,
        volume,
        Some((NotesEventBridge::new(store), note_id)),
    )
}

/// What the consent acceptance found before the first frame.
#[cfg(feature = "consent-dialog-acceptance")]
struct ConsentStart {
    restored: crate::consent_dialog::RestoredDecisions,
    outcome: Result<Option<libnagi::launch::GrantCheck>, &'static [u8]>,
}

/// ADR 0060: restore persisted decisions, launch the signed application,
/// and open the dialog if its requested capability needs an answer. The
/// dialog is part of the first frame.
#[cfg(feature = "consent-dialog-acceptance")]
fn start_consent_acceptance(
    desktop: &mut Desktop,
    volume: &mut UserDataVolume,
    user: &libnagi::security::Session,
) -> ConsentStart {
    use crate::consent_dialog::acceptance::{self, Start};
    let (restored, start) = acceptance::start(volume, user);
    let outcome = match start {
        Start::Prompt(dialog) => {
            desktop.open_consent(dialog);
            Ok(None)
        }
        Start::Decided(check) => Ok(Some(check)),
        Start::Failed(reason) => Err(reason),
    };
    ConsentStart { restored, outcome }
}

#[cfg(feature = "consent-dialog-acceptance")]
fn report_consent_start(desktop: &mut Desktop, start: ConsentStart) {
    use crate::consent_dialog::RestoredDecisions;
    use libnagi::launch::GrantCheck;
    match start.restored {
        RestoredDecisions::None => {}
        RestoredDecisions::Restored(count) => {
            let mut line = [0_u8; 64];
            let mut length = append(&mut line, 0, b"Nagi consent decisions restored count=");
            length = append_decimal(&mut line, length, count as u64);
            length = append(&mut line, length, b"\r\n");
            print(&line[..length]);
        }
        RestoredDecisions::Invalid => {
            print(b"Nagi consent decisions invalid; every grant asks again\r\n");
        }
    }
    match start.outcome {
        // The dialog is announced when it is armed on screen.
        Ok(None) if desktop.consent.is_some() => {}
        Ok(None) => print(b"Nagi consent dialog acceptance FAIL dialog\r\n"),
        Ok(Some(GrantCheck::Granted)) => {
            // The restart then withdraws it in Settings (ADR 0065).
            print(b"Nagi consent decision restored PASS decision=allow\r\n");
        }
        Ok(Some(_)) => print(b"Nagi consent dialog acceptance FAIL restored decision\r\n"),
        Err(reason) => {
            print(b"Nagi consent dialog acceptance FAIL ");
            print(reason);
            print(b"\r\n");
        }
    }
}

#[derive(Clone, Copy)]
enum LocalePreference {
    Missing,
    Restored(nagi_localization::Locale),
    Invalid,
    Unavailable,
}

impl LocalePreference {
    const fn locale(self) -> nagi_localization::Locale {
        match self {
            Self::Restored(locale) => locale,
            Self::Missing | Self::Invalid | Self::Unavailable => nagi_localization::Locale::EnUs,
        }
    }
}

fn load_locale(volume: &mut UserDataVolume) -> LocalePreference {
    let handle = match volume.open_path(SYSTEM_LANGUAGE_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => return LocalePreference::Missing,
        Err(_) => return LocalePreference::Unavailable,
    };
    let mut contents = [0; MAX_SMALL_FILE_SIZE];
    let length = match volume.read(handle, &mut contents) {
        Ok(length) => length,
        Err(_) => return LocalePreference::Unavailable,
    };
    match core::str::from_utf8(&contents[..length])
        .ok()
        .and_then(nagi_localization::Locale::parse)
    {
        Some(locale) => LocalePreference::Restored(locale),
        None => LocalePreference::Invalid,
    }
}

fn persist_locale(volume: &mut UserDataVolume, locale: nagi_localization::Locale) -> bool {
    let handle = match volume.open_path(SYSTEM_LANGUAGE_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => match volume.create_path(SYSTEM_LANGUAGE_PATH) {
            Ok(handle) => handle,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    volume
        .write(handle, locale.code().as_bytes())
        .and_then(|()| volume.flush())
        .is_ok()
}

#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
fn copy_utf8_prefix(destination: &mut [u8], source: &str) -> usize {
    let truncated = source.len() > destination.len();
    let mut length = if truncated {
        destination.len().saturating_sub(3)
    } else {
        source.len()
    };
    while !source.is_char_boundary(length) {
        length -= 1;
    }
    destination[..length].copy_from_slice(&source.as_bytes()[..length]);
    if truncated {
        destination[length..length + 3].copy_from_slice(b"...");
        length += 3;
    }
    length
}

fn clamp(value: i32, minimum: i32, maximum: i32) -> i32 {
    value.max(minimum).min(maximum)
}

fn checksum(surface: &[u32]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in surface.iter().step_by(17) {
        value ^= u64::from(*pixel);
        value = value.wrapping_mul(0x1000_0000_01b3);
    }
    value
}

fn frame_hash(surface: &[u32]) -> u64 {
    surface
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |value, pixel| {
            (value ^ u64::from(*pixel)).wrapping_mul(0x1000_0000_01b3)
        })
}

fn print(bytes: &[u8]) {
    #[cfg(not(test))]
    libnagi::console_write(bytes);
    #[cfg(test)]
    let _ = bytes;
}

fn print_checksum(value: u64) {
    let mut output = [0_u8; 80];
    let mut length = 0;
    length = append(&mut output, length, message!(NAGI_M10_CHECKSUM_PREFIX, 26));
    length = append_decimal(&mut output, length, value);
    length = append(&mut output, length, message!(NAGI_M10_LINE_END, 2));
    let message = unsafe { core::slice::from_raw_parts(output.as_ptr(), length) };
    print(message);
}

fn append(destination: &mut [u8], offset: usize, source: &[u8]) -> usize {
    unsafe {
        core::ptr::copy_nonoverlapping(
            source.as_ptr(),
            destination.as_mut_ptr().add(offset),
            source.len(),
        );
    }
    offset + source.len()
}

#[cfg(test)]
mod desktop_notes_integration_tests {
    use super::{Desktop, DesktopFocus, DesktopTestBlockDevice, UserDataVolume};
    use libnagi::storage::Vfs;
    use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER};
    use notes_event_bridge_candidate::{
        DecodeError, EvdevRecordError, LinuxEvdevTarget, NotesEventBridge, EV_KEY, EV_SYN,
        SYN_DROPPED, SYN_REPORT,
    };
    use notes_files_codec_candidate::{
        Block, BlockKind, HostPreviewStore, InMemoryNoteStore, NoteDocument, NoteStore,
        NotesUiController, ObjectId, Timestamp,
    };
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn notes_localization_adapter_preserves_existing_labels_and_unknown_key_fallback() {
        for locale in [
            nagi_localization::Locale::EnUs,
            nagi_localization::Locale::JaJp,
        ] {
            let desktop = Desktop::new(locale);
            for key in [
                "desktop.notes.title",
                "desktop.notes.content",
                "desktop.notes.unknown.compatibility-test",
            ] {
                assert_eq!(
                    desktop.localized_notes_label(key),
                    nagi_localization::text(locale, key),
                    "the adapter must preserve the existing visible value for {locale:?}/{key}"
                );
            }
        }
    }

    fn seeded_store() -> (Arc<InMemoryNoteStore>, ObjectId) {
        let store = Arc::new(InMemoryNoteStore::new());
        let document = NoteDocument::new(ObjectId(0x4e41_4749_0000_0001), "Notes UI", Timestamp(1));
        let id = store
            .commit(&document, 0)
            .expect("seed real Notes document")
            .id;
        (store, id)
    }

    fn key_event(code: u16) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value: 1,
        }
    }

    fn key_event_value(code: u16, value: i32) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value,
        }
    }

    fn mounted_test_volume() -> UserDataVolume {
        let (mut volume, formatted) =
            Vfs::mount_or_format(DesktopTestBlockDevice::new()).expect("format test VFS disk");
        assert!(formatted, "each test starts with a fresh memory disk");
        let handle = volume
            .create_path(b"/desktop-event-test")
            .expect("create a real VFS test file");
        volume
            .write(handle, b"mounted")
            .expect("write through the real VFS");
        volume.flush().expect("flush the real VFS");
        let mut contents = [0; 16];
        let length = volume
            .read(handle, &mut contents)
            .expect("read through the real VFS");
        assert_eq!(&contents[..length], b"mounted");
        volume
    }

    fn evdev_x86_64_record(event_type: u16, code: u16, value: i32) -> [u8; 24] {
        let mut record = [0; 24];
        record[16..18].copy_from_slice(&event_type.to_le_bytes());
        record[18..20].copy_from_slice(&code.to_le_bytes());
        record[20..24].copy_from_slice(&value.to_le_bytes());
        record
    }

    fn preview_root(label: &str) -> PathBuf {
        let base = std::env::var_os("CODEC_CANDIDATE_TMP")
            .map(PathBuf::from)
            .expect("runner sets CODEC_CANDIDATE_TMP inside codec-candidate");
        base.join(format!(
            "{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn focused_desktop(store: Arc<InMemoryNoteStore>, id: ObjectId) -> Desktop {
        let mut desktop = Desktop::new(nagi_localization::Locale::EnUs);
        desktop.desktop_focus = Some(DesktopFocus::Application(1));
        desktop.focused[1] = true;
        desktop
            .attach_notes(NotesEventBridge::new(store), id)
            .expect("attach actual Notes event bridge to desktop state");
        desktop
    }

    #[test]
    fn desktop_key_action_updates_real_note_state_and_rendered_content() {
        let (store, id) = seeded_store();
        let mut desktop = focused_desktop(store.clone(), id);
        let mut before = vec![0; libnagi::SURFACE_BYTES / core::mem::size_of::<u32>()];
        desktop.render(&mut before);

        assert_eq!(
            desktop.route_notes_event(key_event(30)),
            Some(true),
            "the desktop event dispatcher routes real key input to Notes"
        ); // Nagi ABI US-layout scancode `a`.
        assert_eq!(
            desktop.route_notes_event(key_event(libnagi::INPUT_KEY_LEFT)),
            None,
            "primary-button clicks remain on the desktop hit-test path"
        );
        assert_eq!(
            desktop.notes_document.as_ref().unwrap().search_text(),
            "a",
            "the bridge updates the real NoteDocument used by Desktop"
        );

        let mut after = vec![0; before.len()];
        desktop.render(&mut after);
        assert_ne!(
            before, after,
            "the Notes window renders the stored body text"
        );

        assert_eq!(
            desktop.route_notes_event(key_event(INPUT_KEY_ENTER)),
            Some(true)
        );
        let persisted = store.load(id).unwrap().unwrap();
        assert_eq!(persisted.revision, 2);
        assert_eq!(persisted.search_text(), "a");

        assert_eq!(
            desktop.route_notes_event(key_event(libnagi::INPUT_KEY_ESCAPE)),
            Some(true)
        );
        assert!(desktop
            .notes_bridge
            .as_ref()
            .unwrap()
            .current_note()
            .is_none());
        assert_eq!(desktop.notes_document.as_ref().unwrap(), &persisted);
    }

    #[test]
    fn ordinary_tab_enter_activates_notes_before_the_bridge_saves() {
        const CHILD: &str = "NAGI_ORDINARY_NOTES_ENTRY_READ_CHILD";
        const ROOT: &str = "NAGI_ORDINARY_NOTES_ENTRY_READ_ROOT";
        const ID: &str = "NAGI_ORDINARY_NOTES_ENTRY_READ_ID";
        const KEEP_ROOT: &str = "NAGI_ORDINARY_NOTES_ENTRY_KEEP_ROOT";
        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("child Notes root"));
            let id = ObjectId(
                std::env::var(ID)
                    .expect("child note ID")
                    .parse()
                    .expect("numeric child note ID"),
            );
            let store = HostPreviewStore::open(&root).expect("open Notes store in child process");
            let saved = store
                .load(id)
                .unwrap()
                .expect("child process reads the saved note");
            assert_eq!(saved.revision, 2);
            assert_eq!(saved.search_text(), "a");
            return;
        }

        let root = std::env::var_os(ROOT)
            .map(PathBuf::from)
            .unwrap_or_else(|| preview_root("desktop-user-volume-normal-entry"));
        let id = ObjectId(0x4e41_4749_0000_0007);
        let store = Arc::new(HostPreviewStore::open(&root).expect("open isolated Notes store"));
        let initial = store
            .commit(&NoteDocument::new(id, "Ordinary event", Timestamp(1)), 0)
            .expect("seed through the real Notes store API");
        let mut desktop = Desktop::new_with_notes(
            nagi_localization::Locale::EnUs,
            NotesEventBridge::new(store.clone()),
            id,
        )
        .expect("attach the file-backed Notes bridge");
        let mut volume = mounted_test_volume();

        for _ in 0..3 {
            assert_eq!(
                desktop.handle_event(key_event(libnagi::INPUT_KEY_TAB), &mut volume),
                true
            );
        }
        assert!(desktop.desktop_focus == Some(DesktopFocus::Application(1)));
        assert!(
            !desktop.focused[1],
            "Tab selects Notes but does not launch it"
        );

        assert_eq!(
            desktop.handle_event(key_event_value(INPUT_KEY_ENTER, 0), &mut volume),
            false,
            "key release must not launch or save Notes"
        );
        assert!(!desktop.focused[1]);
        assert_eq!(store.load(id).unwrap(), Some(initial.clone()));

        assert_eq!(
            desktop.handle_event(key_event(INPUT_KEY_ENTER), &mut volume),
            true
        );
        assert!(desktop.focused[1], "the first Enter launches Notes");
        assert!(desktop.keyboard_app_focus[1]);
        assert_eq!(store.load(id).unwrap(), Some(initial.clone()));
        assert_eq!(desktop.current_notes_document(), Some(&initial));

        assert_eq!(
            desktop.handle_event(key_event(30), &mut volume),
            true,
            "ordinary input is routed after the Notes window is active"
        );
        assert_eq!(desktop.current_notes_document().unwrap().search_text(), "a");
        assert_eq!(
            desktop.handle_event(key_event(INPUT_KEY_ENTER), &mut volume),
            true,
            "Enter in the already active Notes window saves the edit"
        );
        let reopened = HostPreviewStore::open(&root).expect("reopen file-backed Notes store");
        let saved = reopened.load(id).unwrap().unwrap();
        assert_eq!(saved.revision, initial.revision + 1);
        assert_eq!(saved.search_text(), "a");
        assert_eq!(desktop.current_notes_document(), Some(&saved));

        let child = Command::new(std::env::current_exe().expect("test executable path"))
            .arg("desktop::desktop_notes_integration_tests::ordinary_tab_enter_activates_notes_before_the_bridge_saves")
            .arg("--exact")
            .env(CHILD, "1")
            .env(ROOT, &root)
            .env(ID, id.0.to_string())
            .env(
                "CODEC_CANDIDATE_TMP",
                std::env::var_os("CODEC_CANDIDATE_TMP").expect("isolated test temp root"),
            )
            .status()
            .expect("run a separate process to reopen the saved NoteStore");
        assert!(
            child.success(),
            "separate process read failed: status={child:?}"
        );

        drop(desktop);
        drop(reopened);
        drop(store);
        if std::env::var_os(KEEP_ROOT).is_none() {
            std::fs::remove_dir_all(root).expect("remove isolated Notes test root");
        }
    }

    #[test]
    fn ordinary_enter_without_a_notes_bridge_still_launches_the_application() {
        let mut desktop = Desktop::new(nagi_localization::Locale::EnUs);
        let mut volume = mounted_test_volume();
        for _ in 0..3 {
            assert_eq!(
                desktop.handle_event(key_event(libnagi::INPUT_KEY_TAB), &mut volume),
                true
            );
        }
        assert_eq!(
            desktop.handle_event(key_event(INPUT_KEY_ENTER), &mut volume),
            true
        );
        assert!(desktop.focused[1]);
        assert!(desktop.keyboard_app_focus[1]);
        assert!(desktop.current_notes_document().is_none());
    }

    #[test]
    fn unsupported_and_post_close_keys_are_not_consumed_by_notes() {
        let (store, id) = seeded_store();
        let mut desktop = focused_desktop(store, id);

        assert_eq!(
            desktop.route_notes_event(key_event(libnagi::INPUT_KEY_LEFT)),
            None,
            "the bridge must leave a key it cannot edit for the desktop"
        );
        assert!(!desktop.notes_has_input);
        assert_eq!(desktop.route_notes_event(key_event(30)), Some(true));
        assert_eq!(desktop.notes_document.as_ref().unwrap().search_text(), "a");
        assert_eq!(
            desktop.route_notes_event(key_event(libnagi::INPUT_KEY_ESCAPE)),
            Some(true)
        );
        assert!(desktop
            .notes_bridge
            .as_ref()
            .unwrap()
            .current_note()
            .is_none());
        assert_eq!(
            desktop.route_notes_event(key_event(31)),
            None,
            "the closed Notes session must not keep consuming later keys"
        );
        assert_eq!(desktop.notes_document.as_ref().unwrap().search_text(), "a");
    }

    #[test]
    fn file_backed_desktop_save_reopens_and_renders_in_a_separate_process() {
        const CHILD: &str = "NAGI_DESKTOP_HOST_PREVIEW_CHILD";
        const ROOT: &str = "NAGI_DESKTOP_HOST_PREVIEW_ROOT";
        const ID: &str = "NAGI_DESKTOP_HOST_PREVIEW_ID";
        const PARENT_PID: &str = "NAGI_DESKTOP_HOST_PREVIEW_PARENT_PID";
        const TEST_NAME: &str = "desktop::desktop_notes_integration_tests::file_backed_desktop_save_reopens_and_renders_in_a_separate_process";

        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("child preview root"));
            let id = ObjectId(
                std::env::var(ID)
                    .expect("child note ID")
                    .parse()
                    .expect("u64 note ID"),
            );
            let parent_pid: u32 = std::env::var(PARENT_PID)
                .expect("parent PID")
                .parse()
                .expect("PID");
            assert_ne!(std::process::id(), parent_pid, "reader is a new process");

            let desktop =
                Desktop::new_with_host_preview_root(nagi_localization::Locale::EnUs, &root, id)
                    .expect("new Desktop opens actual HostPreviewStore");
            assert_eq!(
                desktop.notes_document.as_ref().unwrap().search_text(),
                "a s"
            );

            let mut reopened_surface = vec![0; libnagi::SURFACE_BYTES / 4];
            desktop.render(&mut reopened_surface);
            let mut default_surface = vec![0; reopened_surface.len()];
            Desktop::new(nagi_localization::Locale::EnUs).render(&mut default_surface);
            assert_ne!(
                reopened_surface, default_surface,
                "new Desktop renders the reopened file-backed body"
            );
            return;
        }

        let root = preview_root("desktop-host-preview-restart");
        let id = ObjectId(0x4e41_4749_0000_0002);
        let seed_store = HostPreviewStore::open(&root).expect("open explicit preview root");
        let seed = NoteDocument::new(id, "File-backed Desktop", Timestamp(1));
        seed_store
            .commit(&seed, 0)
            .expect("seed note through real HostPreviewStore");
        drop(seed_store);

        let mut desktop =
            Desktop::new_with_host_preview_root(nagi_localization::Locale::EnUs, &root, id)
                .expect("construct Desktop with explicit development root");
        desktop.desktop_focus = Some(DesktopFocus::Application(1));
        let dispatch = |desktop: &mut Desktop, event_type, code, value| {
            desktop.dispatch_host_preview_evdev_record(
                LinuxEvdevTarget::X86_64,
                &evdev_x86_64_record(event_type, code, value),
            )
        };
        assert_eq!(dispatch(&mut desktop, EV_KEY, 30, 1), Ok(true));
        assert_eq!(dispatch(&mut desktop, EV_KEY, 30, 0), Ok(false));
        assert_eq!(dispatch(&mut desktop, EV_KEY, 57, 1), Ok(true));
        assert_eq!(dispatch(&mut desktop, EV_KEY, 57, 0), Ok(false));
        assert_eq!(dispatch(&mut desktop, EV_KEY, 31, 1), Ok(true));
        assert_eq!(
            dispatch(&mut desktop, EV_KEY, 14, 1),
            Ok(false),
            "valid but unsupported backspace input remains unconsumed"
        );
        assert_eq!(
            dispatch(&mut desktop, EV_SYN, SYN_REPORT, 0),
            Ok(false),
            "non-key synchronization records do not reach Notes edits"
        );
        assert_eq!(
            desktop.dispatch_host_preview_evdev_record(LinuxEvdevTarget::X86_64, &[0; 23]),
            Err(EvdevRecordError::Decode(DecodeError::InvalidRecordLength {
                expected: 24,
                actual: 23
            }))
        );
        assert_eq!(
            desktop.dispatch_host_preview_evdev_record(
                LinuxEvdevTarget::X86_64,
                &evdev_x86_64_record(0xffff, 0, 1),
            ),
            Err(EvdevRecordError::Decode(DecodeError::UnsupportedEventType(
                0xffff
            )))
        );
        assert_eq!(
            desktop.dispatch_host_preview_evdev_record(
                LinuxEvdevTarget::X86_64,
                &evdev_x86_64_record(EV_SYN, SYN_DROPPED, 0),
            ),
            Err(EvdevRecordError::NeedsResync),
            "direct-record consumers receive an explicit recovery signal"
        );
        assert_eq!(
            desktop.notes_document.as_ref().unwrap().search_text(),
            "a s"
        );
        assert_eq!(desktop.notes_document.as_ref().unwrap().revision, 1);
        assert_eq!(
            dispatch(&mut desktop, EV_KEY, INPUT_KEY_ENTER, 1),
            Ok(true),
            "the existing Enter key action saves the note"
        );
        assert_eq!(desktop.notes_document.as_ref().unwrap().revision, 2);
        assert_eq!(
            dispatch(&mut desktop, EV_KEY, libnagi::INPUT_KEY_ESCAPE, 1),
            Ok(true)
        );

        let saved_store = HostPreviewStore::open(&root).expect("open fresh store for verification");
        let saved = saved_store.load(id).unwrap().unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.search_text(), "a s");
        drop(saved_store);
        drop(desktop);

        let child = Command::new(std::env::current_exe().expect("current integration test binary"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD, "1")
            .env(ROOT, &root)
            .env(ID, id.0.to_string())
            .env(PARENT_PID, std::process::id().to_string())
            .status()
            .expect("start separate Desktop reader process");
        let cleanup = std::fs::remove_dir_all(&root);
        assert!(
            child.success(),
            "child process failed with status {child:?}"
        );
        cleanup.expect("remove candidate-only HostPreviewStore root");
    }

    #[test]
    fn controller_text_input_roundtrips_contiguous_text_and_newline_after_reopen() {
        const CHILD: &str = "NAGI_NOTES_TEXT_REOPEN_CHILD";
        const ROOT: &str = "NAGI_NOTES_TEXT_REOPEN_ROOT";
        const ID: &str = "NAGI_NOTES_TEXT_REOPEN_ID";
        const PARENT_PID: &str = "NAGI_NOTES_TEXT_REOPEN_PARENT_PID";
        const TEST_NAME: &str = "desktop::desktop_notes_integration_tests::controller_text_input_roundtrips_contiguous_text_and_newline_after_reopen";

        if std::env::var_os(CHILD).is_some() {
            let root = PathBuf::from(std::env::var_os(ROOT).expect("child Notes root"));
            let id = ObjectId(
                std::env::var(ID)
                    .expect("child note ID")
                    .parse()
                    .expect("u64 note ID"),
            );
            let parent_pid: u32 = std::env::var(PARENT_PID)
                .expect("parent PID")
                .parse()
                .expect("PID");
            assert_ne!(std::process::id(), parent_pid, "reader is a new process");

            let store = Arc::new(HostPreviewStore::open(&root).expect("open saved Notes root"));
            let mut controller = NotesUiController::new(store);
            controller
                .open_note(id)
                .expect("reopen saved note in new process");
            assert_eq!(controller.current_note().unwrap().search_text(), "a s\nb");
            return;
        }

        let root = preview_root("notes-contiguous-text-reopen");
        let id = ObjectId(0x4e41_4749_0000_0004);
        let seed_store = HostPreviewStore::open(&root).expect("open explicit Notes root");
        let seed = NoteDocument::new(id, "Contiguous text", Timestamp(1));
        seed_store
            .commit(&seed, 0)
            .expect("seed actual Notes store");
        drop(seed_store);

        let store = Arc::new(HostPreviewStore::open(&root).expect("open Notes provider"));
        let mut controller = NotesUiController::new(store);
        controller.open_note(id).expect("open seeded note");
        for text in ["a", " ", "s", "\n", "b"] {
            assert!(controller.input_text(text).expect("apply text input"));
        }
        let draft = controller.current_note().expect("open note draft");
        assert_eq!(draft.blocks.len(), 1);
        assert!(matches!(
            &draft.blocks[0].kind,
            BlockKind::Paragraph(text) if text == "a s\nb"
        ));
        let saved = controller
            .save_active()
            .expect("flush Notes session")
            .expect("active note remains open");
        assert_eq!(saved.search_text(), "a s\nb");
        drop(controller);

        let child = Command::new(std::env::current_exe().expect("current integration test binary"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD, "1")
            .env(ROOT, &root)
            .env(ID, id.0.to_string())
            .env(PARENT_PID, std::process::id().to_string())
            .status()
            .expect("start separate Notes reader process");
        let cleanup = std::fs::remove_dir_all(&root);
        assert!(
            child.success(),
            "child process failed with status {child:?}"
        );
        cleanup.expect("remove candidate-only HostPreviewStore root");
    }

    #[test]
    fn controller_text_input_noops_without_session_or_empty_input_and_appends_after_nonparagraph() {
        let store = Arc::new(InMemoryNoteStore::new());
        let mut controller = NotesUiController::new(store.clone());
        assert!(!controller
            .input_text("no active note")
            .expect("input without an active session is a no-op"));
        assert!(!controller
            .input_text("")
            .expect("empty input without an active session is a no-op"));

        let id = ObjectId(0x4e41_4749_0000_0005);
        let mut note = NoteDocument::new(id, "Existing heading", Timestamp(1));
        note.blocks.push(Block::new(
            ObjectId(0x4e41_4749_0000_0006),
            BlockKind::Heading {
                level: 1,
                text: "Heading".to_owned(),
            },
        ));
        store.commit(&note, 0).expect("seed heading note");
        controller.open_note(id).expect("open heading note");
        assert!(!controller
            .input_text("")
            .expect("empty input in an active session is a no-op"));
        assert!(controller
            .input_text("body")
            .expect("append body paragraph"));

        let edited = controller.current_note().expect("active edited note");
        assert_eq!(edited.blocks.len(), 2);
        assert!(matches!(
            &edited.blocks[0].kind,
            BlockKind::Heading { level: 1, text } if text == "Heading"
        ));
        assert!(matches!(
            &edited.blocks[1].kind,
            BlockKind::Paragraph(text) if text == "body"
        ));
    }

    #[test]
    fn failed_enter_save_keeps_the_open_dirty_document_available_for_recovery() {
        let root = preview_root("desktop-host-preview-failed-save");
        let id = ObjectId(0x4e41_4749_0000_0003);
        let seed_store = HostPreviewStore::open(&root).expect("open explicit preview root");
        let seed = NoteDocument::new(id, "Notes UI", Timestamp(1));
        seed_store.commit(&seed, 0).expect("seed file-backed note");
        drop(seed_store);

        let mut desktop =
            Desktop::new_with_host_preview_root(nagi_localization::Locale::EnUs, &root, id)
                .expect("construct Desktop with explicit development root");
        desktop.desktop_focus = Some(DesktopFocus::Application(1));
        desktop.focused[1] = true;

        desktop
            .notes_bridge
            .as_ref()
            .unwrap()
            .set_title("\0")
            .expect("stage a document rejected by the existing Notes validator");
        assert_eq!(
            desktop.route_notes_event(key_event(INPUT_KEY_ENTER)),
            Some(true)
        );
        assert_eq!(desktop.notes_document.as_ref().unwrap().title, "\0");
        let store = HostPreviewStore::open(&root).expect("reopen store after rejected commit");
        assert_eq!(store.load(id).unwrap().unwrap().title, "Notes UI");

        desktop
            .notes_bridge
            .as_ref()
            .unwrap()
            .set_title("Recovered Notes UI")
            .expect("repair still-open session after rejected save");
        assert_eq!(
            desktop.route_notes_event(key_event(INPUT_KEY_ENTER)),
            Some(true)
        );
        assert_eq!(
            desktop.notes_document.as_ref().unwrap().title,
            "Recovered Notes UI"
        );
        assert_eq!(store.load(id).unwrap().unwrap().title, "Recovered Notes UI");
        drop(desktop);
        drop(store);
        std::fs::remove_dir_all(root).expect("remove candidate-only failed-save root");
    }
}

fn append_decimal(destination: &mut [u8], mut offset: usize, mut value: u64) -> usize {
    let start = offset;
    loop {
        unsafe {
            core::ptr::write(
                destination.as_mut_ptr().add(offset),
                b'0' + (value % 10) as u8,
            );
        }
        offset += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut left = start;
    let mut right = offset - 1;
    while left < right {
        unsafe {
            core::ptr::swap(
                destination.as_mut_ptr().add(left),
                destination.as_mut_ptr().add(right),
            );
        }
        left += 1;
        right -= 1;
    }
    offset
}
