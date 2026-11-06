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
    ChangeAction, ChangeField, ChangeProblem, LanguagePicker, LoginAction, LoginField, LoginForm,
    LoginMode, LoginProblem, LoginThrottle, PasswordChangeForm, PickerAction, FREE_ATTEMPTS,
};
use libnagi::security::{AccountStore, Role, Session};
use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_SMALL_FILE_SIZE};
use libnagi::InputEvent;
use nagi_localization::Locale;
use nagi_ui::{color, ColorRole, ThemeMode};

use crate::ui::{Painter, Rect};

type UserDataVolume = Vfs<SyscallBlockDevice>;

const ACCOUNT_PATH: &[u8] = b"owner-account";
/// A changed account record is written here first, then swapped in with one
/// directory update (ADR 0066).
const ACCOUNT_NEXT_PATH: &[u8] = b"owner-account-next";
/// Consecutive failed unlocks, kept across restarts (ADR 0064).
const THROTTLE_PATH: &[u8] = b"login-throttle";
const THROTTLE_MAGIC: &[u8; 4] = b"NLT1";

/// Monotonic milliseconds since boot (10 ms timer ticks).
fn now_ms() -> u64 {
    libnagi::time_ticks().saturating_mul(10)
}
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
    throttle: LoginThrottle,
    /// Whether "retry allowed" has been reported for the current wait.
    retry_announced: bool,
}

impl LoginScreen {
    pub fn new(start: LoginStart) -> Self {
        match start {
            LoginStart::Create => Self {
                language: Some(LanguagePicker::new(LANGUAGES.len(), 0)),
                form: LoginForm::create(),
                account: None,
                unavailable: false,
                throttle: LoginThrottle::resume(0, 0),
                retry_announced: true,
            },
            LoginStart::Unlock(record) => Self {
                language: None,
                form: LoginForm::unlock(),
                account: Some(record),
                unavailable: false,
                throttle: LoginThrottle::resume(0, 0),
                retry_announced: true,
            },
            LoginStart::Unavailable => Self {
                language: None,
                form: LoginForm::unlock(),
                account: None,
                unavailable: true,
                throttle: LoginThrottle::resume(0, 0),
                retry_announced: true,
            },
        }
    }

    /// Restore the persisted failure count. A count past the free attempts
    /// waits from this boot, so restarting does not skip the wait.
    pub fn resume_throttle(&mut self, volume: &mut UserDataVolume) {
        if self.form.mode() != LoginMode::Unlock {
            return;
        }
        let failures = load_failures(volume);
        self.throttle = LoginThrottle::resume(failures, now_ms());
        if failures > 0 {
            say_number(
                b"Nagi login throttle restored failures=",
                u64::from(failures),
            );
        }
        // The notice appears only when someone tries during the wait.
        if !self.throttle.allows(now_ms()) {
            self.retry_announced = false;
        }
    }

    /// Clear the wait notice once attempts are allowed again. Returns
    /// whether the screen changed.
    pub fn tick(&mut self) -> bool {
        if self.retry_announced || !self.throttle.allows(now_ms()) {
            return false;
        }
        self.retry_announced = true;
        libnagi::console_write(b"Nagi login retry allowed\r\n");
        self.form.clear_throttle_notice()
    }

    /// Whether a sign-in wait is running, so the desktop polls `tick`.
    pub const fn is_waiting(&self) -> bool {
        !self.retry_announced
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
                LoginProblem::TooManyAttempts => "login.error.wait",
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
            LoginAction::Unlock => self.unlock(volume),
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

    fn unlock(&mut self, volume: &mut UserDataVolume) -> LoginOutcome {
        let Some(record) = self.account else {
            return LoginOutcome::Ignored;
        };
        let now = now_ms();
        // While throttled the password is not even checked (ADR 0064).
        if !self.throttle.allows(now) {
            self.form.throttled();
            say_number(
                b"Nagi login throttled remaining_ms=",
                self.throttle.remaining_ms(now),
            );
            return LoginOutcome::Changed;
        }
        let outcome = self.sign_in(record);
        match outcome {
            LoginOutcome::SignedIn(_) => {
                self.throttle.record_success();
                if !store_failures(volume, 0) {
                    libnagi::console_write(b"Nagi login throttle persistence FAIL\r\n");
                }
            }
            _ => {
                self.throttle.record_failure(now_ms());
                if !store_failures(volume, self.throttle.failures()) {
                    libnagi::console_write(b"Nagi login throttle persistence FAIL\r\n");
                }
                if self.throttle.failures() >= FREE_ATTEMPTS {
                    self.retry_announced = false;
                    say_number(
                        b"Nagi login throttle engaged failures=",
                        u64::from(self.throttle.failures()),
                    );
                }
            }
        }
        outcome
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

fn say_number(prefix: &[u8], value: u64) {
    let mut digits = [0u8; 20];
    let mut length = 0;
    let mut remaining = value;
    loop {
        digits[length] = b'0' + (remaining % 10) as u8;
        length += 1;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    digits[..length].reverse();
    libnagi::console_write(prefix);
    libnagi::console_write(&digits[..length]);
    libnagi::console_write(b"\r\n");
}

/// The persisted failure count. A missing file is zero; an unreadable or
/// corrupt one fails closed to the free-attempt limit (one wait).
fn load_failures(volume: &mut UserDataVolume) -> u32 {
    let handle = match volume.open_path(THROTTLE_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => return 0,
        Err(_) => return FREE_ATTEMPTS,
    };
    let mut bytes = [0; MAX_SMALL_FILE_SIZE];
    match volume.read(handle, &mut bytes) {
        Ok(8) if bytes[..4] == *THROTTLE_MAGIC => {
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
        }
        _ => FREE_ATTEMPTS,
    }
}

fn store_failures(volume: &mut UserDataVolume, failures: u32) -> bool {
    let handle = match volume.open_path(THROTTLE_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => match volume.create_path(THROTTLE_PATH) {
            Ok(handle) => handle,
            Err(error) => {
                crate::m19_storage::trace_storage(b"throttle.create", error);
                return false;
            }
        },
        Err(error) => {
            crate::m19_storage::trace_storage(b"throttle.open", error);
            return false;
        }
    };
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(THROTTLE_MAGIC);
    bytes[4..].copy_from_slice(&failures.to_le_bytes());
    volume
        .write(handle, &bytes)
        .map_err(|error| crate::m19_storage::trace_storage(b"throttle.write", error))
        .and_then(|()| {
            volume
                .flush()
                .map_err(|error| crate::m19_storage::trace_storage(b"throttle.flush", error))
        })
        .is_ok()
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

/// Replace the stored owner record without a window in which it is missing
/// or half-written: write `owner-account-next`, flush, then swap it in with
/// `Vfs::replace`, which updates the directory in one block write (ADR 0066).
fn persist_replacing(volume: &mut UserDataVolume, record: &AccountRecord) -> bool {
    let handle = match volume.open_path(ACCOUNT_NEXT_PATH) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => match volume.create_path(ACCOUNT_NEXT_PATH) {
            Ok(handle) => handle,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    if volume
        .write(handle, &record.encode())
        .and_then(|()| volume.flush())
        .is_err()
    {
        return false;
    }
    volume
        .replace(ACCOUNT_NEXT_PATH, ACCOUNT_PATH)
        .and_then(|_| volume.flush())
        .is_ok()
}

pub enum ChangeOutcome {
    Ignored,
    Changed,
    /// Escape closed the form without changing the password.
    Cancelled,
    /// The new credential was atomically saved.
    PasswordChanged,
}

/// The modal "change password" screen, opened from Settings by a signed-in
/// owner (ADR 0066). The current password is checked against the stored
/// credential before anything is written, and wrong attempts count toward
/// the same persisted throttle as the lock screen (ADR 0064).
pub struct PasswordChangeScreen {
    form: PasswordChangeForm,
    account: AccountRecord,
    throttle: LoginThrottle,
}

impl PasswordChangeScreen {
    /// Open the form for the stored owner. `None` when the account record
    /// cannot be read, so nothing is offered to change.
    pub fn open(volume: &mut UserDataVolume) -> Option<Self> {
        let LoginStart::Unlock(account) = load(volume) else {
            return None;
        };
        let failures = load_failures(volume);
        Some(Self {
            form: PasswordChangeForm::new(),
            account,
            throttle: LoginThrottle::resume(failures, now_ms()),
        })
    }

    pub fn render(&self, painter: &mut Painter<'_>, locale: Locale) {
        let text = color(THEME, ColorRole::TextPrimary).to_pixel();
        let border = color(THEME, ColorRole::BorderStrong).to_pixel();
        let focus = color(THEME, ColorRole::Focus).to_pixel();
        painter.fill(PANEL, color(THEME, ColorRole::Surface).to_pixel());
        painter.frame(PANEL, border);
        painter.fill(
            Rect::new(PANEL.x + 1, PANEL.y + 1, PANEL.width - 2, 16),
            color(THEME, ColorRole::Accent).to_pixel(),
        );
        painter.text(
            PANEL.x + 8,
            PANEL.y + 5,
            nagi_localization::text(locale, "settings.password.title").as_bytes(),
            color(THEME, ColorRole::TextOnAccent).to_pixel(),
        );
        let fields = [
            (ChangeField::Current, "settings.password.current"),
            (ChangeField::New, "settings.password.new"),
            (ChangeField::Confirm, "login.confirm"),
        ];
        let mut y = PANEL.y + 22;
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
                if self.form.focus() == field {
                    focus
                } else {
                    border
                },
            );
            let length = self.form.length(field).min(26);
            painter.text(rect.x + 4, rect.y + 5, &[b'*'; 26][..length], text);
            y += 30;
        }
        if let Some(problem) = self.form.problem() {
            let key = match problem {
                ChangeProblem::WrongCurrent => "login.error.wrong",
                ChangeProblem::PasswordTooShort => "login.error.short",
                ChangeProblem::PasswordMismatch => "login.error.mismatch",
                ChangeProblem::SameAsCurrent => "settings.password.error.same",
                ChangeProblem::TooManyAttempts => "login.error.wait",
                ChangeProblem::NotSaved => "settings.password.error.save",
            };
            painter.text(
                PANEL.x + 20,
                PANEL.y + PANEL.height - 12,
                nagi_localization::text(locale, key).as_bytes(),
                color(THEME, ColorRole::Danger).to_pixel(),
            );
        }
    }

    pub fn handle_event(
        &mut self,
        event: InputEvent,
        volume: &mut UserDataVolume,
    ) -> ChangeOutcome {
        if event.event_type != libnagi::INPUT_EVENT_KEY {
            return ChangeOutcome::Ignored;
        }
        match self.form.handle_key(event.code, event.value != 0) {
            ChangeAction::Ignored => ChangeOutcome::Ignored,
            ChangeAction::Changed => ChangeOutcome::Changed,
            ChangeAction::Cancel => {
                libnagi::console_write(b"Nagi password change cancelled\r\n");
                ChangeOutcome::Cancelled
            }
            ChangeAction::Submit => self.submit(volume),
        }
    }

    fn submit(&mut self, volume: &mut UserDataVolume) -> ChangeOutcome {
        let now = now_ms();
        // While throttled the current password is not even checked.
        if !self.throttle.allows(now) {
            self.form.throttled();
            say_number(
                b"Nagi password change throttled remaining_ms=",
                self.throttle.remaining_ms(now),
            );
            return ChangeOutcome::Changed;
        }
        if !self.account.credential.verify(self.form.current()) {
            self.throttle.record_failure(now_ms());
            if !store_failures(volume, self.throttle.failures()) {
                libnagi::console_write(b"Nagi login throttle persistence FAIL\r\n");
            }
            self.form.reject_current();
            libnagi::console_write(b"Nagi password change REJECTED current\r\n");
            return ChangeOutcome::Changed;
        }
        self.throttle.record_success();
        if !store_failures(volume, 0) {
            libnagi::console_write(b"Nagi login throttle persistence FAIL\r\n");
        }
        let mut salt = [0; SALT_BYTES];
        let record = libnagi::random_fill(&mut salt)
            .then(|| Credential::derive(self.form.new_password(), salt, DEFAULT_ITERATIONS))
            .flatten()
            .and_then(|credential| AccountRecord::new(self.account.name(), credential).ok());
        let Some(record) = record else {
            self.form.not_saved();
            libnagi::console_write(b"Nagi password change FAIL credential\r\n");
            return ChangeOutcome::Changed;
        };
        if !persist_replacing(volume, &record) {
            self.form.not_saved();
            libnagi::console_write(b"Nagi password change FAIL storage\r\n");
            return ChangeOutcome::Changed;
        }
        self.form.clear_secrets();
        self.account = record;
        libnagi::console_write(b"Nagi password change PASS\r\n");
        ChangeOutcome::PasswordChanged
    }
}
