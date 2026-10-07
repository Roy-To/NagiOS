//! Desktop login form (ADR 0063).
//!
//! The OS-owned login screen has two modes:
//!
//! - **Create**: first run. The user picks an owner name and enters a
//!   password twice.
//! - **Unlock**: later boots. The user enters the owner's password.
//!
//! This module is the input state machine only. The desktop renders it and
//! performs the credential work it asks for, so the policy is host-testable.
//! Keys map to lowercase ASCII letters, digits and `-` (US layout scancodes);
//! Tab moves between fields, Backspace edits, Enter advances or submits.

use crate::credential::{
    valid_name, MAX_ACCOUNT_NAME_BYTES, MAX_PASSWORD_BYTES, MIN_PASSWORD_BYTES,
};
use crate::{INPUT_KEY_ENTER, INPUT_KEY_TAB};

pub const INPUT_KEY_BACKSPACE: u16 = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginMode {
    Create,
    Unlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginField {
    Name,
    Password,
    Confirm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginProblem {
    InvalidName,
    PasswordTooShort,
    PasswordMismatch,
    WrongPassword,
    /// Too many failures: wait before trying again.
    TooManyAttempts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginAction {
    Ignored,
    Changed,
    /// Create mode: name and password are valid and confirmed.
    Create,
    /// Unlock mode: check the password.
    Unlock,
}

#[derive(Clone, Copy)]
struct Text<const N: usize> {
    bytes: [u8; N],
    length: usize,
}

impl<const N: usize> Text<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            length: 0,
        }
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.length == N {
            return false;
        }
        self.bytes[self.length] = byte;
        self.length += 1;
        true
    }

    fn pop(&mut self) -> bool {
        if self.length == 0 {
            return false;
        }
        self.length -= 1;
        self.bytes[self.length] = 0;
        true
    }

    fn clear(&mut self) {
        self.bytes = [0; N];
        self.length = 0;
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

/// US-layout scancode to character for the login fields.
pub const fn key_char(code: u16) -> Option<u8> {
    const LETTERS: [(u16, u8); 26] = [
        (16, b'q'),
        (17, b'w'),
        (18, b'e'),
        (19, b'r'),
        (20, b't'),
        (21, b'y'),
        (22, b'u'),
        (23, b'i'),
        (24, b'o'),
        (25, b'p'),
        (30, b'a'),
        (31, b's'),
        (32, b'd'),
        (33, b'f'),
        (34, b'g'),
        (35, b'h'),
        (36, b'j'),
        (37, b'k'),
        (38, b'l'),
        (44, b'z'),
        (45, b'x'),
        (46, b'c'),
        (47, b'v'),
        (48, b'b'),
        (49, b'n'),
        (50, b'm'),
    ];
    match code {
        2..=10 => Some(b'1' + (code - 2) as u8),
        11 => Some(b'0'),
        12 => Some(b'-'),
        _ => {
            let mut index = 0;
            while index < LETTERS.len() {
                if LETTERS[index].0 == code {
                    return Some(LETTERS[index].1);
                }
                index += 1;
            }
            None
        }
    }
}

/// First-run language step (M29 onboarding): pick one of the offered
/// languages before the owner account is created. Up/Down/Tab move the
/// focus; Enter or Space chooses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguagePicker {
    focus: usize,
    count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerAction {
    Ignored,
    Changed,
    Chosen(usize),
}

impl LanguagePicker {
    /// `count` options, with focus on `initial` (clamped).
    pub const fn new(count: usize, initial: usize) -> Self {
        let count = if count == 0 { 1 } else { count };
        Self {
            focus: if initial < count { initial } else { 0 },
            count,
        }
    }

    pub const fn focus(&self) -> usize {
        self.focus
    }

    pub fn handle_key(&mut self, code: u16, pressed: bool) -> PickerAction {
        if !pressed {
            return PickerAction::Ignored;
        }
        match code {
            crate::INPUT_KEY_DOWN | INPUT_KEY_TAB => {
                self.focus = (self.focus + 1) % self.count;
                PickerAction::Changed
            }
            crate::INPUT_KEY_UP => {
                self.focus = (self.focus + self.count - 1) % self.count;
                PickerAction::Changed
            }
            INPUT_KEY_ENTER | crate::INPUT_KEY_SPACE => PickerAction::Chosen(self.focus),
            _ => PickerAction::Ignored,
        }
    }
}

/// Failed unlock attempts allowed back to back before a wait (ADR 0064).
pub const FREE_ATTEMPTS: u32 = 3;
const FIRST_WAIT_MS: u64 = 5_000;
const MAX_WAIT_MS: u64 = 60_000;

/// The wait after `failures` consecutive failures: none for the first
/// `FREE_ATTEMPTS`, then 5 s doubling to a 60 s cap.
pub const fn throttle_wait_ms(failures: u32) -> u64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = failures - FREE_ATTEMPTS;
    if doublings >= 4 {
        return MAX_WAIT_MS;
    }
    let wait = FIRST_WAIT_MS << doublings;
    if wait > MAX_WAIT_MS {
        MAX_WAIT_MS
    } else {
        wait
    }
}

/// Unlock rate limiting. Times are a monotonic clock in milliseconds
/// supplied by the caller. The failure count is meant to be persisted, so a
/// restart does not reset it: a throttled count restored at boot waits from
/// the boot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginThrottle {
    failures: u32,
    blocked_until_ms: u64,
}

impl LoginThrottle {
    /// Resume with `failures` recorded earlier, at time `now_ms`.
    pub const fn resume(failures: u32, now_ms: u64) -> Self {
        Self {
            failures,
            blocked_until_ms: now_ms.saturating_add(throttle_wait_ms(failures)),
        }
    }

    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Whether an attempt may be checked at `now_ms`.
    pub const fn allows(&self, now_ms: u64) -> bool {
        now_ms >= self.blocked_until_ms
    }

    pub const fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.blocked_until_ms.saturating_sub(now_ms)
    }

    pub fn record_failure(&mut self, now_ms: u64) {
        self.failures = self.failures.saturating_add(1);
        self.blocked_until_ms = now_ms.saturating_add(throttle_wait_ms(self.failures));
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.blocked_until_ms = 0;
    }
}

pub struct LoginForm {
    mode: LoginMode,
    focus: LoginField,
    name: Text<MAX_ACCOUNT_NAME_BYTES>,
    password: Text<MAX_PASSWORD_BYTES>,
    confirm: Text<MAX_PASSWORD_BYTES>,
    problem: Option<LoginProblem>,
    failures: u32,
}

impl LoginForm {
    pub const fn create() -> Self {
        Self {
            mode: LoginMode::Create,
            focus: LoginField::Name,
            name: Text::new(),
            password: Text::new(),
            confirm: Text::new(),
            problem: None,
            failures: 0,
        }
    }

    pub const fn unlock() -> Self {
        Self {
            mode: LoginMode::Unlock,
            focus: LoginField::Password,
            ..Self::create()
        }
    }

    pub const fn mode(&self) -> LoginMode {
        self.mode
    }

    pub const fn focus(&self) -> LoginField {
        self.focus
    }

    pub const fn problem(&self) -> Option<LoginProblem> {
        self.problem
    }

    pub const fn failures(&self) -> u32 {
        self.failures
    }

    pub fn name(&self) -> &[u8] {
        self.name.as_bytes()
    }

    pub fn password(&self) -> &[u8] {
        self.password.as_bytes()
    }

    /// Characters entered in `field`, for masked rendering.
    pub fn length(&self, field: LoginField) -> usize {
        match field {
            LoginField::Name => self.name.length,
            LoginField::Password => self.password.length,
            LoginField::Confirm => self.confirm.length,
        }
    }

    /// Record a failed unlock: clear the password and show the problem.
    pub fn reject(&mut self) {
        self.failures = self.failures.saturating_add(1);
        self.problem = Some(LoginProblem::WrongPassword);
        self.password.clear();
    }

    /// Refuse an attempt made while throttled: clear the password.
    pub fn throttled(&mut self) {
        self.problem = Some(LoginProblem::TooManyAttempts);
        self.password.clear();
    }

    /// Clear a throttle notice once attempts are allowed again.
    pub fn clear_throttle_notice(&mut self) -> bool {
        if self.problem == Some(LoginProblem::TooManyAttempts) {
            self.problem = None;
            return true;
        }
        false
    }

    /// Forget every secret once the desktop has consumed them.
    pub fn clear_secrets(&mut self) {
        self.password.clear();
        self.confirm.clear();
    }

    /// A key press (`pressed`) or release. Only presses act.
    pub fn handle_key(&mut self, code: u16, pressed: bool) -> LoginAction {
        if !pressed {
            return LoginAction::Ignored;
        }
        match code {
            INPUT_KEY_TAB => {
                self.focus = self.next_field();
                LoginAction::Changed
            }
            INPUT_KEY_BACKSPACE => {
                if self.field_mut().pop() {
                    LoginAction::Changed
                } else {
                    LoginAction::Ignored
                }
            }
            INPUT_KEY_ENTER => self.enter(),
            _ => match key_char(code) {
                Some(byte) if self.field_mut().push(byte) => {
                    self.problem = None;
                    LoginAction::Changed
                }
                _ => LoginAction::Ignored,
            },
        }
    }

    fn next_field(&self) -> LoginField {
        match (self.mode, self.focus) {
            (LoginMode::Unlock, _) => LoginField::Password,
            (LoginMode::Create, LoginField::Name) => LoginField::Password,
            (LoginMode::Create, LoginField::Password) => LoginField::Confirm,
            (LoginMode::Create, LoginField::Confirm) => LoginField::Name,
        }
    }

    fn field_mut(&mut self) -> &mut dyn TextField {
        match self.focus {
            LoginField::Name => &mut self.name,
            LoginField::Password => &mut self.password,
            LoginField::Confirm => &mut self.confirm,
        }
    }

    fn enter(&mut self) -> LoginAction {
        match (self.mode, self.focus) {
            (LoginMode::Unlock, _) => {
                if self.password.length == 0 {
                    LoginAction::Ignored
                } else {
                    LoginAction::Unlock
                }
            }
            (LoginMode::Create, LoginField::Name | LoginField::Password) => {
                self.focus = self.next_field();
                LoginAction::Changed
            }
            (LoginMode::Create, LoginField::Confirm) => {
                let problem = if !valid_name(self.name.as_bytes()) {
                    Some((LoginProblem::InvalidName, LoginField::Name))
                } else if self.password.length < MIN_PASSWORD_BYTES {
                    Some((LoginProblem::PasswordTooShort, LoginField::Password))
                } else if self.password.as_bytes() != self.confirm.as_bytes() {
                    Some((LoginProblem::PasswordMismatch, LoginField::Password))
                } else {
                    None
                };
                match problem {
                    Some((problem, focus)) => {
                        self.problem = Some(problem);
                        self.focus = focus;
                        if focus == LoginField::Password {
                            self.clear_secrets();
                        }
                        LoginAction::Changed
                    }
                    None => LoginAction::Create,
                }
            }
        }
    }
}

/// Fields of the password-change form (ADR 0066).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeField {
    Current,
    New,
    Confirm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeProblem {
    /// The current password did not match.
    WrongCurrent,
    PasswordTooShort,
    PasswordMismatch,
    /// The new password equals the current one.
    SameAsCurrent,
    /// Too many wrong current passwords: wait before trying again.
    TooManyAttempts,
    /// The new credential could not be derived or stored.
    NotSaved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeAction {
    Ignored,
    Changed,
    /// The three fields are filled and the new password is valid and
    /// confirmed. The caller checks the current password and stores the
    /// new credential.
    Submit,
    /// Escape: close the form without changing anything.
    Cancel,
}

/// Change the owner's password while signed in (ADR 0066). Like
/// [`LoginForm`], this is the input state machine only; the desktop
/// verifies the current password against the stored credential and writes
/// the new one.
///
/// Tab moves between fields, Enter advances or submits, Backspace edits,
/// Escape cancels. Every secret is cleared whenever a problem is shown.
pub struct PasswordChangeForm {
    focus: ChangeField,
    current: Text<MAX_PASSWORD_BYTES>,
    new: Text<MAX_PASSWORD_BYTES>,
    confirm: Text<MAX_PASSWORD_BYTES>,
    problem: Option<ChangeProblem>,
}

impl PasswordChangeForm {
    pub const fn new() -> Self {
        Self {
            focus: ChangeField::Current,
            current: Text::new(),
            new: Text::new(),
            confirm: Text::new(),
            problem: None,
        }
    }

    pub const fn focus(&self) -> ChangeField {
        self.focus
    }

    pub const fn problem(&self) -> Option<ChangeProblem> {
        self.problem
    }

    pub fn current(&self) -> &[u8] {
        self.current.as_bytes()
    }

    pub fn new_password(&self) -> &[u8] {
        self.new.as_bytes()
    }

    /// Characters entered in `field`, for masked rendering.
    pub fn length(&self, field: ChangeField) -> usize {
        match field {
            ChangeField::Current => self.current.length,
            ChangeField::New => self.new.length,
            ChangeField::Confirm => self.confirm.length,
        }
    }

    /// Forget every secret once the desktop has consumed them.
    pub fn clear_secrets(&mut self) {
        self.current.clear();
        self.new.clear();
        self.confirm.clear();
    }

    /// The current password was wrong: start over from that field.
    pub fn reject_current(&mut self) {
        self.show(ChangeProblem::WrongCurrent, ChangeField::Current);
    }

    /// Refuse an attempt made while throttled, without having checked it.
    pub fn throttled(&mut self) {
        self.show(ChangeProblem::TooManyAttempts, ChangeField::Current);
    }

    /// The new credential could not be derived or stored.
    pub fn not_saved(&mut self) {
        self.show(ChangeProblem::NotSaved, ChangeField::Current);
    }

    /// Clear a throttle notice once attempts are allowed again.
    pub fn clear_throttle_notice(&mut self) -> bool {
        if self.problem == Some(ChangeProblem::TooManyAttempts) {
            self.problem = None;
            return true;
        }
        false
    }

    fn show(&mut self, problem: ChangeProblem, focus: ChangeField) {
        self.clear_secrets();
        self.problem = Some(problem);
        self.focus = focus;
    }

    pub fn handle_key(&mut self, code: u16, pressed: bool) -> ChangeAction {
        if !pressed {
            return ChangeAction::Ignored;
        }
        match code {
            crate::INPUT_KEY_ESCAPE => {
                self.clear_secrets();
                ChangeAction::Cancel
            }
            INPUT_KEY_TAB => {
                self.focus = self.next_field();
                ChangeAction::Changed
            }
            INPUT_KEY_BACKSPACE => {
                if self.field_mut().pop() {
                    ChangeAction::Changed
                } else {
                    ChangeAction::Ignored
                }
            }
            INPUT_KEY_ENTER => self.enter(),
            _ => match key_char(code) {
                Some(byte) if self.field_mut().push(byte) => {
                    self.problem = None;
                    ChangeAction::Changed
                }
                _ => ChangeAction::Ignored,
            },
        }
    }

    const fn next_field(&self) -> ChangeField {
        match self.focus {
            ChangeField::Current => ChangeField::New,
            ChangeField::New => ChangeField::Confirm,
            ChangeField::Confirm => ChangeField::Current,
        }
    }

    fn field_mut(&mut self) -> &mut dyn TextField {
        match self.focus {
            ChangeField::Current => &mut self.current,
            ChangeField::New => &mut self.new,
            ChangeField::Confirm => &mut self.confirm,
        }
    }

    fn enter(&mut self) -> ChangeAction {
        match self.focus {
            ChangeField::Current => {
                if self.current.length == 0 {
                    return ChangeAction::Ignored;
                }
                self.focus = ChangeField::New;
                ChangeAction::Changed
            }
            ChangeField::New => {
                self.focus = ChangeField::Confirm;
                ChangeAction::Changed
            }
            ChangeField::Confirm => {
                let problem = if self.current.length == 0 {
                    Some((ChangeProblem::WrongCurrent, ChangeField::Current))
                } else if self.new.length < MIN_PASSWORD_BYTES {
                    Some((ChangeProblem::PasswordTooShort, ChangeField::New))
                } else if self.new.as_bytes() != self.confirm.as_bytes() {
                    Some((ChangeProblem::PasswordMismatch, ChangeField::New))
                } else if self.new.as_bytes() == self.current.as_bytes() {
                    Some((ChangeProblem::SameAsCurrent, ChangeField::New))
                } else {
                    None
                };
                match problem {
                    Some((problem, focus)) => {
                        if focus == ChangeField::Current {
                            self.show(problem, focus);
                        } else {
                            // The current password stays; only the new one is retyped.
                            self.new.clear();
                            self.confirm.clear();
                            self.problem = Some(problem);
                            self.focus = focus;
                        }
                        ChangeAction::Changed
                    }
                    None => ChangeAction::Submit,
                }
            }
        }
    }
}

impl Default for PasswordChangeForm {
    fn default() -> Self {
        Self::new()
    }
}

trait TextField {
    fn push(&mut self, byte: u8) -> bool;
    fn pop(&mut self) -> bool;
}

impl<const N: usize> TextField for Text<N> {
    fn push(&mut self, byte: u8) -> bool {
        Text::push(self, byte)
    }

    fn pop(&mut self) -> bool {
        Text::pop(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 30;
    const D: u16 = 32;
    const M: u16 = 50;
    const I: u16 = 23;
    const N: u16 = 49;
    const ONE: u16 = 2;

    fn type_keys(form: &mut LoginForm, keys: &[u16]) -> LoginAction {
        let mut last = LoginAction::Ignored;
        for key in keys {
            last = form.handle_key(*key, true);
            form.handle_key(*key, false);
        }
        last
    }

    #[test]
    fn scancodes_map_to_login_characters() {
        assert_eq!(key_char(A), Some(b'a'));
        assert_eq!(key_char(44), Some(b'z'));
        assert_eq!(key_char(ONE), Some(b'1'));
        assert_eq!(key_char(11), Some(b'0'));
        assert_eq!(key_char(12), Some(b'-'));
        assert_eq!(key_char(57), None);
    }

    #[test]
    fn first_run_creates_a_confirmed_owner() {
        let mut form = LoginForm::create();
        type_keys(&mut form, &[A, D, M, I, N]);
        assert_eq!(form.name(), b"admin");
        assert_eq!(
            type_keys(&mut form, &[INPUT_KEY_ENTER]),
            LoginAction::Changed
        );
        assert_eq!(form.focus(), LoginField::Password);
        type_keys(&mut form, &[N, A, M, I, ONE, INPUT_KEY_TAB]);
        assert_eq!(form.focus(), LoginField::Confirm);
        assert_eq!(
            type_keys(&mut form, &[N, A, M, I, ONE, INPUT_KEY_ENTER]),
            LoginAction::Create
        );
        assert_eq!(form.password(), b"nami1");
        form.clear_secrets();
        assert_eq!(form.password(), b"");
    }

    #[test]
    fn creation_problems_are_reported_and_secrets_cleared() {
        let mut form = LoginForm::create();
        type_keys(
            &mut form,
            &[
                ONE,
                INPUT_KEY_ENTER,
                A,
                A,
                A,
                A,
                INPUT_KEY_ENTER,
                A,
                A,
                A,
                A,
            ],
        );
        assert_eq!(
            type_keys(&mut form, &[INPUT_KEY_ENTER]),
            LoginAction::Changed
        );
        assert_eq!(form.problem(), Some(LoginProblem::InvalidName));
        assert_eq!(form.focus(), LoginField::Name);

        let mut form = LoginForm::create();
        type_keys(
            &mut form,
            &[
                A,
                INPUT_KEY_ENTER,
                A,
                A,
                INPUT_KEY_ENTER,
                A,
                A,
                INPUT_KEY_ENTER,
            ],
        );
        assert_eq!(form.problem(), Some(LoginProblem::PasswordTooShort));
        assert_eq!(form.length(LoginField::Password), 0);
        assert_eq!(form.length(LoginField::Confirm), 0);

        let mut form = LoginForm::create();
        type_keys(
            &mut form,
            &[
                A,
                INPUT_KEY_ENTER,
                A,
                A,
                A,
                A,
                INPUT_KEY_ENTER,
                A,
                A,
                A,
                D,
                INPUT_KEY_ENTER,
            ],
        );
        assert_eq!(form.problem(), Some(LoginProblem::PasswordMismatch));
        assert_eq!(form.focus(), LoginField::Password);
        // Typing again clears the problem.
        type_keys(&mut form, &[A]);
        assert_eq!(form.problem(), None);
    }

    #[test]
    fn unlock_submits_only_a_nonempty_password_and_counts_failures() {
        let mut form = LoginForm::unlock();
        assert_eq!(
            type_keys(&mut form, &[INPUT_KEY_ENTER]),
            LoginAction::Ignored
        );
        type_keys(&mut form, &[A, D, INPUT_KEY_BACKSPACE, M]);
        assert_eq!(form.password(), b"am");
        assert_eq!(
            type_keys(&mut form, &[INPUT_KEY_ENTER]),
            LoginAction::Unlock
        );
        form.reject();
        assert_eq!(form.failures(), 1);
        assert_eq!(form.problem(), Some(LoginProblem::WrongPassword));
        assert_eq!(form.password(), b"");
        // Tab cannot leave the password field in unlock mode.
        type_keys(&mut form, &[INPUT_KEY_TAB]);
        assert_eq!(form.focus(), LoginField::Password);
    }

    #[test]
    fn fields_are_bounded() {
        let mut form = LoginForm::create();
        for _ in 0..MAX_ACCOUNT_NAME_BYTES {
            assert_eq!(form.handle_key(A, true), LoginAction::Changed);
        }
        assert_eq!(form.handle_key(A, true), LoginAction::Ignored);
        assert_eq!(form.length(LoginField::Name), MAX_ACCOUNT_NAME_BYTES);
    }

    #[test]
    fn language_picker_moves_and_chooses() {
        let mut picker = LanguagePicker::new(2, 0);
        assert_eq!(
            picker.handle_key(crate::INPUT_KEY_DOWN, true),
            PickerAction::Changed
        );
        assert_eq!(picker.focus(), 1);
        assert_eq!(
            picker.handle_key(crate::INPUT_KEY_DOWN, false),
            PickerAction::Ignored
        );
        assert_eq!(
            picker.handle_key(INPUT_KEY_TAB, true),
            PickerAction::Changed
        );
        assert_eq!(picker.focus(), 0);
        assert_eq!(
            picker.handle_key(crate::INPUT_KEY_UP, true),
            PickerAction::Changed
        );
        assert_eq!(picker.focus(), 1);
        assert_eq!(picker.handle_key(A, true), PickerAction::Ignored);
        assert_eq!(
            picker.handle_key(INPUT_KEY_ENTER, true),
            PickerAction::Chosen(1)
        );
        assert_eq!(LanguagePicker::new(2, 9).focus(), 0);
    }

    #[test]
    fn throttle_waits_grow_after_the_free_attempts() {
        assert_eq!(throttle_wait_ms(0), 0);
        assert_eq!(throttle_wait_ms(2), 0);
        assert_eq!(throttle_wait_ms(3), 5_000);
        assert_eq!(throttle_wait_ms(4), 10_000);
        assert_eq!(throttle_wait_ms(5), 20_000);
        assert_eq!(throttle_wait_ms(6), 40_000);
        assert_eq!(throttle_wait_ms(7), 60_000);
        assert_eq!(throttle_wait_ms(u32::MAX), 60_000);
    }

    #[test]
    fn throttle_blocks_until_the_wait_passes_and_resets_on_success() {
        let mut throttle = LoginThrottle::resume(0, 1_000);
        for attempt in 0..FREE_ATTEMPTS {
            assert!(throttle.allows(1_000 + u64::from(attempt)));
            throttle.record_failure(1_000);
        }
        assert!(!throttle.allows(1_000));
        assert_eq!(throttle.remaining_ms(2_000), 4_000);
        assert!(throttle.allows(6_000));
        throttle.record_failure(6_000);
        assert!(!throttle.allows(15_999) && throttle.allows(16_000));
        throttle.record_success();
        assert_eq!(throttle.failures(), 0);
        assert!(throttle.allows(0));
    }

    #[test]
    fn a_restored_failure_count_waits_from_the_restart() {
        let throttle = LoginThrottle::resume(4, 500);
        assert!(!throttle.allows(10_499));
        assert!(throttle.allows(10_500));
        assert!(LoginThrottle::resume(2, 500).allows(500));
    }

    #[test]
    fn throttle_notice_clears_the_password() {
        let mut form = LoginForm::unlock();
        type_keys(&mut form, &[A, D]);
        form.throttled();
        assert_eq!(form.problem(), Some(LoginProblem::TooManyAttempts));
        assert_eq!(form.password(), b"");
        assert!(form.clear_throttle_notice());
        assert!(!form.clear_throttle_notice());
    }

    fn change_keys(form: &mut PasswordChangeForm, keys: &[u16]) -> ChangeAction {
        let mut last = ChangeAction::Ignored;
        for key in keys {
            last = form.handle_key(*key, true);
            form.handle_key(*key, false);
        }
        last
    }

    const ENTER: u16 = INPUT_KEY_ENTER;

    #[test]
    fn a_valid_change_submits_all_three_passwords() {
        let mut form = PasswordChangeForm::new();
        assert_eq!(form.focus(), ChangeField::Current);
        // Enter on an empty current password does nothing.
        assert_eq!(change_keys(&mut form, &[ENTER]), ChangeAction::Ignored);
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER]);
        assert_eq!(form.focus(), ChangeField::New);
        change_keys(&mut form, &[D, A, M, I, ONE, ENTER]);
        assert_eq!(form.focus(), ChangeField::Confirm);
        assert_eq!(
            change_keys(&mut form, &[D, A, M, I, ONE, ENTER]),
            ChangeAction::Submit
        );
        assert_eq!(form.current(), b"nami1");
        assert_eq!(form.new_password(), b"dami1");
        form.clear_secrets();
        assert_eq!(form.current(), b"");
        assert_eq!(form.new_password(), b"");
        assert_eq!(form.length(ChangeField::Confirm), 0);
    }

    #[test]
    fn change_problems_keep_the_current_password_when_it_is_not_at_fault() {
        let mut form = PasswordChangeForm::new();
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER, A, A, ENTER, A, A]);
        assert_eq!(change_keys(&mut form, &[ENTER]), ChangeAction::Changed);
        assert_eq!(form.problem(), Some(ChangeProblem::PasswordTooShort));
        assert_eq!(form.focus(), ChangeField::New);
        assert_eq!(form.current(), b"nami1");
        assert_eq!(form.length(ChangeField::New), 0);
        assert_eq!(form.length(ChangeField::Confirm), 0);

        let mut form = PasswordChangeForm::new();
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER]);
        change_keys(&mut form, &[D, A, M, I, ONE, ENTER, D, A, M, I, D, ENTER]);
        assert_eq!(form.problem(), Some(ChangeProblem::PasswordMismatch));
        assert_eq!(form.focus(), ChangeField::New);

        let mut form = PasswordChangeForm::new();
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER]);
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER, N, A, M, I, ONE, ENTER]);
        assert_eq!(form.problem(), Some(ChangeProblem::SameAsCurrent));
        assert_eq!(form.focus(), ChangeField::New);
        assert_eq!(form.current(), b"nami1");
    }

    #[test]
    fn a_wrong_or_throttled_current_password_clears_every_secret() {
        let mut form = PasswordChangeForm::new();
        change_keys(&mut form, &[N, A, M, I, ONE, ENTER, D, A, M, I, ONE]);
        form.reject_current();
        assert_eq!(form.problem(), Some(ChangeProblem::WrongCurrent));
        assert_eq!(form.focus(), ChangeField::Current);
        for field in [ChangeField::Current, ChangeField::New, ChangeField::Confirm] {
            assert_eq!(form.length(field), 0);
        }
        change_keys(&mut form, &[A]);
        assert_eq!(form.problem(), None);

        form.throttled();
        assert_eq!(form.problem(), Some(ChangeProblem::TooManyAttempts));
        assert_eq!(form.length(ChangeField::Current), 0);
        assert!(form.clear_throttle_notice());
        assert!(!form.clear_throttle_notice());

        form.not_saved();
        assert_eq!(form.problem(), Some(ChangeProblem::NotSaved));
        assert!(!form.clear_throttle_notice());
    }

    #[test]
    fn escape_cancels_and_forgets_the_secrets() {
        let mut form = PasswordChangeForm::new();
        change_keys(&mut form, &[N, A, M, I, ONE]);
        assert_eq!(
            change_keys(&mut form, &[crate::INPUT_KEY_ESCAPE]),
            ChangeAction::Cancel
        );
        assert_eq!(form.current(), b"");
        // Releases never act; Tab cycles; Backspace edits.
        assert_eq!(form.handle_key(ENTER, false), ChangeAction::Ignored);
        change_keys(&mut form, &[A, A]);
        change_keys(&mut form, &[INPUT_KEY_BACKSPACE]);
        assert_eq!(form.current(), b"a");
        change_keys(&mut form, &[INPUT_KEY_TAB, INPUT_KEY_TAB, INPUT_KEY_TAB]);
        assert_eq!(form.focus(), ChangeField::Current);
        assert_eq!(
            change_keys(&mut form, &[INPUT_KEY_BACKSPACE, INPUT_KEY_BACKSPACE]),
            ChangeAction::Ignored
        );
    }
}
