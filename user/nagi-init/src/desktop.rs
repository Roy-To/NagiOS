use core::arch::asm;

use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_SMALL_FILE_SIZE};
use libnagi::{DisplayInfo, InputEvent};

use crate::ui::{Painter, Rect};
use nagi_ui::{color, ColorRole, ThemeMode};

type UserDataVolume = Vfs<SyscallBlockDevice>;

const SYSTEM_LANGUAGE_PATH: &[u8] = b"system-language";

const APP_COUNT: usize = 4;
const WINDOW_WIDTH: i32 = 145;
const WINDOW_HEIGHT: i32 = 70;
const TITLE_HEIGHT: i32 = 14;
const POINTER_START_X: i32 = 80;
const POINTER_START_Y: i32 = 58;
const PREVIEW_THEME: ThemeMode = ThemeMode::Light;
const BACKGROUND: u32 = color(PREVIEW_THEME, ColorRole::Canvas).to_pixel();
const PANEL: u32 = color(PREVIEW_THEME, ColorRole::Surface).to_pixel();
const TITLE: u32 = color(PREVIEW_THEME, ColorRole::Accent).to_pixel();
const BORDER: u32 = color(PREVIEW_THEME, ColorRole::BorderStrong).to_pixel();
const TEXT: u32 = color(PREVIEW_THEME, ColorRole::TextPrimary).to_pixel();
const TITLE_TEXT: u32 = color(PREVIEW_THEME, ColorRole::TextOnAccent).to_pixel();
const FOCUS: u32 = color(PREVIEW_THEME, ColorRole::Focus).to_pixel();
const SETTINGS_BUTTON: Rect = Rect::new(252, 1, 66, 18);
const SETTINGS_PANEL: Rect = Rect::new(34, 34, 252, 132);
const ENGLISH_OPTION: Rect = Rect::new(48, 83, 224, 25);
const JAPANESE_OPTION: Rect = Rect::new(48, 116, 224, 25);
/// Opens the permissions view (ADR 0065); shown only with consent.
#[cfg(feature = "consent-dialog-acceptance")]
const PERMISSIONS_OPTION: Rect = Rect::new(48, 146, 224, 16);

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
    #[cfg(feature = "consent-dialog-acceptance")]
    Permissions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesktopFocus {
    SettingsButton,
    Application(usize),
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
    #[cfg(feature = "consent-dialog-acceptance")]
    consent: Option<crate::consent_dialog::ConsentDialog>,
    #[cfg(feature = "consent-dialog-acceptance")]
    consent_answered: bool,
    #[cfg(feature = "consent-dialog-acceptance")]
    permissions: Option<crate::consent_settings::PermissionsPanel>,
    /// The OS-owned login screen, shown until the owner signs in.
    #[cfg(feature = "desktop-login")]
    login: Option<crate::login_screen::LoginScreen>,
    #[cfg(feature = "desktop-login")]
    session: Option<libnagi::security::Session>,
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
            locale,
            settings_open: false,
            settings_focus: None,
            #[cfg(feature = "consent-dialog-acceptance")]
            consent: None,
            #[cfg(feature = "consent-dialog-acceptance")]
            consent_answered: false,
            #[cfg(feature = "consent-dialog-acceptance")]
            permissions: None,
            #[cfg(feature = "desktop-login")]
            login: None,
            #[cfg(feature = "desktop-login")]
            session: None,
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
        if self.settings_open {
            self.render_settings(&mut painter);
        }
        #[cfg(feature = "consent-dialog-acceptance")]
        if let Some(panel) = &self.permissions {
            panel.render(&mut painter, self.locale);
        }
        #[cfg(feature = "consent-dialog-acceptance")]
        if let Some(dialog) = &self.consent {
            dialog.render(&mut painter, self.locale);
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
                    // Readiness means a signed-in desktop (ADR 0063).
                    if !libnagi::report_boot_ready() {
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
        #[cfg(feature = "consent-dialog-acceptance")]
        if self.consent.is_some() {
            return self.handle_consent_event(event, volume);
        }
        #[cfg(feature = "consent-dialog-acceptance")]
        if self.permissions.is_some() {
            return self.handle_permissions_event(event, volume);
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
        #[cfg(feature = "desktop-login")]
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

    /// Show the OS-owned consent dialog for `dialog`'s request.
    #[cfg(feature = "consent-dialog-acceptance")]
    pub fn open_consent(&mut self, dialog: crate::consent_dialog::ConsentDialog) {
        self.consent = Some(dialog);
    }

    /// Accept input in the open dialog once its frame is on screen, and
    /// announce it then, so observers never act on an unpresented dialog.
    #[cfg(feature = "consent-dialog-acceptance")]
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

    #[cfg(feature = "consent-dialog-acceptance")]
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
    #[cfg(feature = "consent-dialog-acceptance")]
    fn signed_in_user(&self) -> Option<libnagi::security::Session> {
        #[cfg(feature = "desktop-login")]
        return self.session;
        #[cfg(not(feature = "desktop-login"))]
        crate::supervisor::acceptance_user()
    }

    #[cfg(feature = "consent-dialog-acceptance")]
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
        let label: &[u8] = match answer {
            ConsentAnswer::Chosen(ConsentChoice::Allow) => b"allow",
            ConsentAnswer::Chosen(ConsentChoice::AllowOnce) => b"allow-once",
            ConsentAnswer::Chosen(ConsentChoice::Deny) => b"deny",
            ConsentAnswer::Dismissed => b"dismissed",
        };
        match consent_dialog::resolve(volume, &user, &request, answer) {
            Some(check) if check == expected => {
                print(b"Nagi consent dialog decision PASS decision=");
                print(label);
                print(b"\r\n");
                if matches!(
                    answer,
                    ConsentAnswer::Chosen(ConsentChoice::Allow | ConsentChoice::Deny)
                ) {
                    print(b"Nagi consent decision persisted PASS\r\n");
                }
                // The restart half of the acceptance expects a persisted Allow.
                if answer == ConsentAnswer::Chosen(ConsentChoice::Allow) {
                    self.consent_answered = true;
                }
            }
            _ => print(b"Nagi consent dialog acceptance FAIL resolve\r\n"),
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
        #[cfg(feature = "consent-dialog-acceptance")]
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
            match self.settings_focus {
                Some(SettingsFocus::English) => SettingsFocus::Japanese,
                #[cfg(feature = "consent-dialog-acceptance")]
                Some(SettingsFocus::Japanese) => SettingsFocus::Permissions,
                #[cfg(not(feature = "consent-dialog-acceptance"))]
                Some(SettingsFocus::Japanese) => SettingsFocus::English,
                #[cfg(feature = "consent-dialog-acceptance")]
                Some(SettingsFocus::Permissions) => SettingsFocus::English,
                Some(SettingsFocus::Button) | None => SettingsFocus::English,
            }
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
        self.settings_focus = Some(match (self.settings_focus, down) {
            #[cfg(feature = "consent-dialog-acceptance")]
            (Some(SettingsFocus::Japanese), true) => SettingsFocus::Permissions,
            #[cfg(feature = "consent-dialog-acceptance")]
            (Some(SettingsFocus::Permissions), true) => SettingsFocus::English,
            #[cfg(feature = "consent-dialog-acceptance")]
            (Some(SettingsFocus::English), false) => SettingsFocus::Permissions,
            #[cfg(feature = "consent-dialog-acceptance")]
            (Some(SettingsFocus::Permissions), false) => SettingsFocus::Japanese,
            (Some(SettingsFocus::English), _) => SettingsFocus::Japanese,
            (Some(SettingsFocus::Japanese), _) => SettingsFocus::English,
            (Some(SettingsFocus::Button) | None, true) => SettingsFocus::English,
            (Some(SettingsFocus::Button) | None, false) => SettingsFocus::Japanese,
        });
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
            #[cfg(feature = "consent-dialog-acceptance")]
            Some(SettingsFocus::Permissions) if self.settings_open => {
                self.permissions = Some(crate::consent_settings::PermissionsPanel::open());
                true
            }
            _ => false,
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
}

pub fn run(display_capability: u64, input_capability: u64, mut volume: UserDataVolume) -> ! {
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
    #[cfg(feature = "consent-dialog-acceptance")]
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
            unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
            continue;
        }
        if desktop.handle_event(event, &mut volume) {
            desktop.render(surface);
            if !libnagi::display_present(display_capability) {
                print(message!(NAGI_M10_FAIL, 26));
                libnagi::exit(1);
            }
            // A dialog opened by this event accepts input only now that its
            // frame is on screen.
            #[cfg(feature = "consent-dialog-acceptance")]
            desktop.arm_consent();
            #[cfg(feature = "consent-dialog-acceptance")]
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
