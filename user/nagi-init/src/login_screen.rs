//! OS-owned desktop login (ADR 0063).
//!
//! First run creates the owner account; later boots unlock it. The screen
//! is drawn by init and takes every key event until the user is signed in.
//! Only a salted PBKDF2 credential is written to User Data
//! (`owner-account`); the password itself is never stored and is cleared
//! from memory once it has been checked.

use libnagi::credential::{
    AccountRecord, AccountRecordError, Credential, ACCOUNT_RECORD_BYTES, DEFAULT_ITERATIONS,
    SALT_BYTES,
};
use libnagi::login::{
    LanguagePicker, LoginAction, LoginField, LoginForm, LoginMode, LoginProblem, PickerAction,
};
use libnagi::security::{AccountStore, Role, Session};
use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_SMALL_FILE_SIZE};
use libnagi::InputEvent;
use nagi_localization::Locale;
use nagi_ui::{color, ColorRole, ThemeMode};

use crate::ui::{Painter, Rect};

type UserDataVolume = Vfs<SyscallBlockDevice>;

const ACCOUNT_PATH: &[u8] = b"owner-account";
const THEME: ThemeMode = ThemeMode::Light;
const PANEL: Rect = Rect::new(40, 30, 240, 140);
const FIELD_WIDTH: i32 = 200;
const FIELD_HEIGHT: i32 = 16;

pub enum LoginStart {
    Create,
    Unlock(AccountRecord),
    /// The account file exists but is unreadable or corrupt. Nothing can
    /// sign in; the screen stays locked and Recovery is the way out.
    Unavailable,
}

pub enum LoginOutcome {
    Ignored,
    Changed,
    /// First run: the user chose the system language (M29 onboarding).
    LanguageChosen(Locale),
    SignedIn(Session),
}

/// Languages offered on first run, in display order.
const LANGUAGES: [Locale; 2] = [Locale::EnUs, Locale::JaJp];
const LANGUAGE_OPTIONS: [Rect; 2] = [Rect::new(60, 72, 200, 22), Rect::new(60, 102, 200, 22)];

pub struct LoginScreen {
    /// First run starts with the language step.
    language: Option<LanguagePicker>,
    form: LoginForm,
    account: Option<AccountRecord>,
    unavailable: bool,
}

impl LoginScreen {
    pub fn new(start: LoginStart) -> Self {
        match start {
            LoginStart::Create => Self {
                language: Some(LanguagePicker::new(LANGUAGES.len(), 0)),
                form: LoginForm::create(),
                account: None,
                unavailable: false,
            },
            LoginStart::Unlock(record) => Self {
                language: None,
                form: LoginForm::unlock(),
                account: Some(record),
                unavailable: false,
            },
            LoginStart::Unavailable => Self {
                language: None,
                form: LoginForm::unlock(),
                account: None,
                unavailable: true,
            },
        }
    }

    pub fn mode(&self) -> LoginMode {
        self.form.mode()
    }

    /// Start the language step with `locale` focused.
    pub fn focus_language(&mut self, locale: Locale) {
        if let Some(index) = LANGUAGES.iter().position(|offered| *offered == locale) {
            if self.language.is_some() {
                self.language = Some(LanguagePicker::new(LANGUAGES.len(), index));
            }
        }
    }

    pub fn render(&self, painter: &mut Painter<'_>, locale: Locale) {
        if let Some(picker) = &self.language {
            self.render_language(painter, locale, picker);
            return;
        }
        let text = color(THEME, ColorRole::TextPrimary).to_pixel();
        let border = color(THEME, ColorRole::BorderStrong).to_pixel();
        let focus = color(THEME, ColorRole::Focus).to_pixel();
        painter.fill(
            Rect::new(0, 0, 320, 200),
            color(THEME, ColorRole::Canvas).to_pixel(),
        );
        painter.fill(PANEL, color(THEME, ColorRole::Surface).to_pixel());
        painter.frame(PANEL, border);
        painter.fill(
            Rect::new(PANEL.x + 1, PANEL.y + 1, PANEL.width - 2, 16),
            color(THEME, ColorRole::Accent).to_pixel(),
        );
        let title = match self.form.mode() {
            LoginMode::Create => "login.title.create",
            LoginMode::Unlock => "login.title.unlock",
        };
        painter.text(
            PANEL.x + 8,
            PANEL.y + 5,
            nagi_localization::text(locale, title).as_bytes(),
            color(THEME, ColorRole::TextOnAccent).to_pixel(),
        );
        let mut y = PANEL.y + 24;
        let fields: &[(LoginField, &str)] = match self.form.mode() {
            LoginMode::Create => &[
                (LoginField::Name, "login.name"),
                (LoginField::Password, "login.password"),
                (LoginField::Confirm, "login.confirm"),
            ],
            LoginMode::Unlock => &[(LoginField::Password, "login.password")],
        };
        if let (LoginMode::Unlock, Some(account)) = (self.form.mode(), &self.account) {
            painter.text(PANEL.x + 20, y, account.name(), text);
            y += 14;
        }
        for (field, label) in fields {
            painter.text(
                PANEL.x + 20,
                y,
                nagi_localization::text(locale, label).as_bytes(),
                text,
            );
            let rect = Rect::new(PANEL.x + 20, y + 9, FIELD_WIDTH, FIELD_HEIGHT);
            painter.fill(rect, color(THEME, ColorRole::SurfaceRaised).to_pixel());
            painter.frame(
                rect,
                if self.form.focus() == *field {
                    focus
                } else {
                    border
                },
            );
            let length = self.form.length(*field).min(26);
            let mut content = [b'*'; 26];
            if *field == LoginField::Name {
                content[..length].copy_from_slice(&self.form.name()[..length]);
            }
            painter.text(rect.x + 4, rect.y + 5, &content[..length], text);
            y += 30;
        }
        let problem = if self.unavailable {
            Some("login.error.wrong")
        } else {
            self.form.problem().map(|problem| match problem {
                LoginProblem::InvalidName => "login.error.name",
                LoginProblem::PasswordTooShort => "login.error.short",
                LoginProblem::PasswordMismatch => "login.error.mismatch",
                LoginProblem::WrongPassword => "login.error.wrong",
            })
        };
        if let Some(key) = problem {
            painter.text(
                PANEL.x + 20,
                PANEL.y + PANEL.height - 12,
                nagi_localization::text(locale, key).as_bytes(),
                color(THEME, ColorRole::Danger).to_pixel(),
            );
        }
    }

    fn render_language(&self, painter: &mut Painter<'_>, locale: Locale, picker: &LanguagePicker) {
        let text = color(THEME, ColorRole::TextPrimary).to_pixel();
        let border = color(THEME, ColorRole::BorderStrong).to_pixel();
        painter.fill(
            Rect::new(0, 0, 320, 200),
            color(THEME, ColorRole::Canvas).to_pixel(),
        );
        painter.fill(PANEL, color(THEME, ColorRole::Surface).to_pixel());
        painter.frame(PANEL, border);
        painter.fill(
            Rect::new(PANEL.x + 1, PANEL.y + 1, PANEL.width - 2, 16),
            color(THEME, ColorRole::Accent).to_pixel(),
        );
        painter.text(
            PANEL.x + 8,
            PANEL.y + 5,
            nagi_localization::text(locale, "login.language.title").as_bytes(),
            color(THEME, ColorRole::TextOnAccent).to_pixel(),
        );
        for (index, (offered, rect)) in LANGUAGES.iter().zip(LANGUAGE_OPTIONS).enumerate() {
            painter.fill(rect, color(THEME, ColorRole::SurfaceRaised).to_pixel());
            let focused = picker.focus() == index;
            painter.frame(
                rect,
                if focused {
                    color(THEME, ColorRole::Focus).to_pixel()
                } else {
                    border
                },
            );
            if focused {
                painter.frame(
                    Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2),
                    color(THEME, ColorRole::Focus).to_pixel(),
                );
            }
            // Each language is named in itself.
            let key = match offered {
                Locale::EnUs => "desktop.settings.option.en-US",
                Locale::JaJp => "desktop.settings.option.ja-JP",
            };
            painter.text(
                rect.x + 8,
                rect.y + 8,
                nagi_localization::text(*offered, key).as_bytes(),
                text,
            );
        }
    }

    pub fn handle_event(&mut self, event: InputEvent, volume: &mut UserDataVolume) -> LoginOutcome {
        if event.event_type != libnagi::INPUT_EVENT_KEY || self.unavailable {
            return LoginOutcome::Ignored;
        }
        if let Some(picker) = &mut self.language {
            return match picker.handle_key(event.code, event.value != 0) {
                PickerAction::Ignored => LoginOutcome::Ignored,
                PickerAction::Changed => LoginOutcome::Changed,
                PickerAction::Chosen(index) => {
                    self.language = None;
                    LoginOutcome::LanguageChosen(LANGUAGES[index.min(LANGUAGES.len() - 1)])
                }
            };
        }
        match self.form.handle_key(event.code, event.value != 0) {
            LoginAction::Ignored => LoginOutcome::Ignored,
            LoginAction::Changed => LoginOutcome::Changed,
            LoginAction::Create => self.create(volume),
            LoginAction::Unlock => self.unlock(),
        }
    }

    fn create(&mut self, volume: &mut UserDataVolume) -> LoginOutcome {
        let mut salt = [0; SALT_BYTES];
        let derived = libnagi::random_fill(&mut salt)
            .then(|| Credential::derive(self.form.password(), salt, DEFAULT_ITERATIONS))
            .flatten();
        let Some(credential) = derived else {
            self.form.clear_secrets();
            libnagi::console_write(b"Nagi login owner creation FAIL credential\r\n");
            return LoginOutcome::Changed;
        };
        let Ok(record) = AccountRecord::new(self.form.name(), credential) else {
            self.form.clear_secrets();
            return LoginOutcome::Changed;
        };
        if !persist(volume, &record) {
            self.form.clear_secrets();
            libnagi::console_write(b"Nagi login owner creation FAIL storage\r\n");
            return LoginOutcome::Changed;
        }
        libnagi::console_write(b"Nagi login owner created PASS name=");
        libnagi::console_write(record.name());
        libnagi::console_write(b"\r\n");
        self.account = Some(record);
        self.sign_in(record)
    }

    fn unlock(&mut self) -> LoginOutcome {
        match self.account {
            Some(record) => self.sign_in(record),
            None => LoginOutcome::Ignored,
        }
    }

    /// Authenticate the typed password against the stored credential and
    /// bind a session to it; the password is cleared either way.
    fn sign_in(&mut self, record: AccountRecord) -> LoginOutcome {
        let mut accounts = AccountStore::new();
        let session = accounts
            .add_account_with_credential(record.name(), Role::Owner, record.credential)
            .ok()
            .and_then(|()| {
                accounts
                    .authenticate(record.name(), self.form.password())
                    .ok()
            });
        match session {
            Some(session) => {
                self.form.clear_secrets();
                libnagi::console_write(b"Nagi login unlocked PASS\r\n");
                LoginOutcome::SignedIn(session)
            }
            None => {
                self.form.reject();
                libnagi::console_write(b"Nagi login unlock REJECTED\r\n");
                LoginOutcome::Changed
            }
        }
    }
}

/// Read the owner account from User Data.
pub fn load(volume: &mut UserDataVolume) -> LoginStart {
    let handle = match volume.open_path(ACCOUNT_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => return LoginStart::Create,
        Err(_) => return LoginStart::Unavailable,
    };
    let mut bytes = [0; MAX_SMALL_FILE_SIZE];
    let Ok(length) = volume.read(handle, &mut bytes) else {
        return LoginStart::Unavailable;
    };
    match AccountRecord::decode(&bytes[..length]) {
        Ok(record) => LoginStart::Unlock(record),
        Err(
            AccountRecordError::Malformed
            | AccountRecordError::Checksum
            | AccountRecordError::InvalidName,
        ) => LoginStart::Unavailable,
    }
}

fn persist(volume: &mut UserDataVolume, record: &AccountRecord) -> bool {
    const _: () = assert!(ACCOUNT_RECORD_BYTES <= MAX_SMALL_FILE_SIZE);
    let handle = match volume.open_path(ACCOUNT_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => match volume.create_path(ACCOUNT_PATH) {
            Ok(handle) => handle,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    volume
        .write(handle, &record.encode())
        .and_then(|()| volume.flush())
        .is_ok()
}
