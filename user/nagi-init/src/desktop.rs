use alloc::string::String;
use core::arch::asm;

use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_SMALL_FILE_SIZE};
use libnagi::{DisplayInfo, InputEvent};

#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
pub(super) use crate::m19_runtime::files;
#[cfg(feature = "m20-model-service")]
#[path = "bar_adapter.rs"]
mod bar_adapter;
#[cfg(feature = "m20-model-service")]
#[path = "bar_panel.rs"]
mod bar_panel;
#[cfg(feature = "m20-model-service")]
#[path = "session_ui.rs"]
pub(crate) mod session_ui;
use crate::ui::{Painter, Rect};
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
#[path = "m19_files_panel.rs"]
mod files_panel;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_MANAGE_KEY: u16 = 60; // Linux/VirtIO input F2.
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
const FILES_PANEL: Rect = Rect::new(17, 25, 286, 167);
use nagi_ui::{color, ColorRole, ThemeMode};

pub(super) type UserDataVolume = Vfs<SyscallBlockDevice>;

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
        let address: *const u8;
        unsafe {
            asm!(
                "lea {address}, [rip + {symbol}]",
                address = out(reg) address,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            core::slice::from_raw_parts(address, $length)
        }
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
    #[cfg(feature = "m20-model-service")]
    bar: bar_panel::Panel,
    #[cfg(feature = "m20-model-service")]
    model_services_ready: bool,
    #[cfg(feature = "m20-model-service")]
    model_services: Option<bar_adapter::Adapter>,
    #[cfg(feature = "m20-model-service")]
    lock_requested: bool,
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
            #[cfg(feature = "m20-model-service")]
            bar: bar_panel::Panel::new(),
            #[cfg(feature = "m20-model-service")]
            model_services_ready: false,
            #[cfg(feature = "m20-model-service")]
            model_services: None,
            #[cfg(feature = "m20-model-service")]
            lock_requested: false,
            pointer_x: POINTER_START_X,
            pointer_y: POINTER_START_Y,
            notes_has_input: false,
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
        self.render_app(
            &mut painter,
            1,
            nagi_localization::text(self.locale, "desktop.notes.title").as_bytes(),
            nagi_localization::text(self.locale, "desktop.notes.content").as_bytes(),
        );
        if self.notes_has_input {
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
        #[cfg(feature = "m20-model-service")]
        self.bar.render(&mut painter, self.locale);
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
                    let ready = libnagi::report_boot_ready();
                    #[cfg(feature = "m20-model-service")]
                    {
                        self.model_services_ready = ready;
                    }
                    if ready {
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
        #[cfg(feature = "m20-model-service")]
        if event.event_type == libnagi::INPUT_EVENT_KEY
            && event.value == 1
            && (event.code == 62 // F4 locks this OS-owned session.
                || (event.code == libnagi::INPUT_KEY_LEFT
                    && bar_panel::LOCK_BUTTON.contains(self.pointer_x, self.pointer_y)))
        {
            self.lock_model_session(volume);
            self.lock_requested = true;
            return true;
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
        #[cfg(feature = "m20-model-service")]
        if !self.settings_open && event.event_type == libnagi::INPUT_EVENT_KEY {
            if event.value == 1
                && (event.code == 61 // F3 opens the Nagi Bar.
                || (event.code == libnagi::INPUT_KEY_LEFT
                    && bar_panel::BUTTON.contains(self.pointer_x, self.pointer_y)))
            {
                self.bar.toggle();
                self.files_panel.open = false;
                return true;
            }
            if self.bar.open {
                if event.value == 1
                    && event.code == libnagi::INPUT_KEY_LEFT
                    && SETTINGS_BUTTON.contains(self.pointer_x, self.pointer_y)
                {
                    self.bar.open = false;
                } else {
                    return self.bar.event(event, self.pointer_x, self.pointer_y);
                }
            }
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
        if event.event_type == libnagi::INPUT_EVENT_KEY && event.value != 0 {
            if event.code == libnagi::INPUT_KEY_TAB {
                if self.settings_open {
                    self.focus_next_settings_control();
                } else {
                    self.focus_next_desktop_control();
                }
                return true;
            }
            if event.code == libnagi::INPUT_KEY_UP || event.code == libnagi::INPUT_KEY_DOWN {
                return self.move_settings_focus(event.code == libnagi::INPUT_KEY_DOWN);
            }
            if event.code == libnagi::INPUT_KEY_ESCAPE && self.settings_open {
                self.settings_open = false;
                self.settings_focus = Some(SettingsFocus::Button);
                self.desktop_focus = Some(DesktopFocus::SettingsButton);
                return true;
            }
            #[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
            if self.handle_files_search_key(event.code) {
                return true;
            }
            if event.code == libnagi::INPUT_KEY_ENTER || event.code == libnagi::INPUT_KEY_SPACE {
                return self.activate_desktop_focus(volume);
            }
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

    #[cfg(feature = "m20-model-service")]
    fn lock_model_session(&mut self, volume: &mut UserDataVolume) {
        // This runs inside handle_event too, so a nested Files consent loop
        // immediately sees the invalidated session and aborts its old launch.
        if let Some(services) = self.model_services.as_mut() {
            self.bar.controller.lock_or_signout(services);
        }
        if let Some(session) = self.session.as_mut() {
            session.lock();
        }
        self.session = None;
        self.model_services_ready = false;
        self.bar.clear_presentation();
        self.files_panel = files_panel::Panel::new();
        self.clear_files_search();
        self.password_change = None;
        self.settings_open = false;
        self.desktop_focus = None;
        self.focused.fill(false);
        self.notes_has_input = false;
        #[cfg(feature = "consent-dialog")]
        {
            self.consent = None;
            self.permissions = None;
            self.consent_decision = None;
        }
        let mut login = crate::login_screen::LoginScreen::new(crate::login_screen::load(volume));
        login.focus_language(self.locale);
        login.resume_throttle(volume);
        self.login = Some(login);
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
        #[cfg(feature = "production-session")]
        return false;
        #[cfg(not(feature = "production-session"))]
        {
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
                let desktop_ready =
                    self.focused.iter().all(|focused| *focused) && self.notes_has_input;
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
    mut volume: UserDataVolume,
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
    let mut desktop = Desktop::new(preference.locale());
    #[cfg(feature = "m20-model-service")]
    {
        desktop.model_services = Some(bar_adapter::Adapter::new(
            crate::session_services::boot_model_store_capability(),
        ));
    }
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
        #[cfg(feature = "m20-model-service")]
        {
            let old_status = desktop.bar.controller.status();
            if let Some(services) = desktop.model_services.as_mut() {
                if desktop.bar.cancel_requested {
                    desktop.bar.cancel_requested = false;
                    desktop.bar.controller.cancel(services);
                }
                session_ui::idle_step(
                    &mut desktop.bar.controller,
                    desktop.session.as_ref(),
                    desktop.model_services_ready,
                    services,
                    || {
                        let _ = libnagi::thread_yield();
                    },
                );
                if desktop.bar.controller.status() == session_ui::Status::Ready
                    && desktop.bar.controller.terms().is_none()
                {
                    if let Some((name, reference)) = services.terms() {
                        desktop.bar.controller.offer_terms(name, reference);
                    }
                }
            }
            if desktop.bar.open && old_status != desktop.bar.controller.status() {
                desktop.render(surface);
                if !libnagi::display_present(display_capability) {
                    libnagi::exit(1);
                }
            }
        }
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
            // Cooperate even while locked, idle or without a pending request.
            let _ = libnagi::thread_yield();
            continue;
        }
        if desktop.handle_event(event, &mut volume) {
            #[cfg(feature = "m20-model-service")]
            if desktop.lock_requested {
                desktop.lock_requested = false;
                search_runtime = None;
            }
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
                unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
            }
        }
    }
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
    libnagi::console_write(bytes);
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
