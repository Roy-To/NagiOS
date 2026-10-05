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
}
