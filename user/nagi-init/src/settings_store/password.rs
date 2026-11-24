//! Password entry for the Linux Settings panel.

use crate::ui::{Painter, Rect};
use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE, INPUT_KEY_TAB};
use nagi_account_credential::{MAX_PASSWORD_BYTES, MIN_PASSWORD_BYTES};
use nagi_localization::{text, Locale};
use nagi_ui::{color, ColorRole, ThemeMode};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasswordResult {
    Changed,
    WrongCurrent,
    Wait,
    Same,
    Invalid,
    Unavailable,
    OutcomeUnknown,
}

pub(crate) struct PasswordPanel {
    secret: Zeroizing<[[u8; MAX_PASSWORD_BYTES]; 3]>,
    lengths: [usize; 3],
    field: usize,
    status: Option<&'static str>,
    finished: bool,
}

impl PasswordPanel {
    pub(crate) fn new() -> Self {
        Self {
            secret: Zeroizing::new([[0; MAX_PASSWORD_BYTES]; 3]),
            lengths: [0; 3],
            field: 0,
            status: None,
            finished: false,
        }
    }

    pub(crate) fn appearance(&self) -> (usize, [bool; 3], Option<&'static str>, bool) {
        (
            self.field,
            self.lengths.map(|length| length != 0),
            self.status,
            self.finished,
        )
    }

    pub(crate) fn input_lost(&mut self) {
        self.secret.zeroize();
        self.lengths = [0; 3];
        if !self.finished {
            self.field = 0;
            self.status = Some("login.error.input_lost");
        }
    }

    /// Returns true when the panel should close.
    pub(crate) fn on_key(
        &mut self,
        event: InputEvent,
        change: impl FnOnce(&[u8], &[u8]) -> PasswordResult,
    ) -> bool {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            return false;
        }
        if event.code == INPUT_KEY_ESCAPE {
            return true;
        }
        if self.status == Some("login.error.input_lost") {
            return false;
        }
        if self.finished {
            return event.code == INPUT_KEY_ENTER;
        }
        let length = self.lengths[self.field];
        match event.code {
            INPUT_KEY_TAB => self.field = (self.field + 1) % 3,
            14 if length > 0 => {
                self.secret[self.field][length - 1..length].zeroize();
                self.lengths[self.field] -= 1;
            }
            INPUT_KEY_ENTER if self.field < 2 => self.field += 1,
            INPUT_KEY_ENTER => self.submit(change),
            code if length < MAX_PASSWORD_BYTES => {
                if let Some(byte) = key_byte(code) {
                    self.secret[self.field][length] = byte;
                    self.lengths[self.field] += 1;
                    self.status = None;
                }
            }
            _ => {}
        }
        false
    }

    fn submit(&mut self, change: impl FnOnce(&[u8], &[u8]) -> PasswordResult) {
        let current = &self.secret[0][..self.lengths[0]];
        let next = &self.secret[1][..self.lengths[1]];
        let confirm = &self.secret[2][..self.lengths[2]];
        let status = if current.is_empty() {
            "login.error.wrong"
        } else if next.len() < MIN_PASSWORD_BYTES {
            "login.error.short"
        } else if next != confirm {
            "login.error.mismatch"
        } else if next == current {
            "settings.password.error.same"
        } else {
            let result = change(current, next);
            self.finished = matches!(
                result,
                PasswordResult::Changed | PasswordResult::OutcomeUnknown
            );
            match result {
                PasswordResult::Changed => "settings.password.changed",
                PasswordResult::WrongCurrent => "login.error.wrong",
                PasswordResult::Wait => "login.error.wait",
                PasswordResult::Same => "settings.password.error.same",
                PasswordResult::Invalid => "login.error.short",
                PasswordResult::Unavailable => "settings.password.error.save",
                PasswordResult::OutcomeUnknown => "settings.password.error.unknown",
            }
        };
        self.secret.zeroize();
        self.lengths = [0; 3];
        self.field = 0;
        self.status = Some(status);
    }

    pub(crate) fn render(&self, painter: &mut Painter<'_>, locale: Locale, theme: ThemeMode) {
        let pixel = |role| color(theme, role).to_pixel();
        painter.fill(Rect::new(0, 0, 320, 200), pixel(ColorRole::Canvas));
        painter.text(
            12,
            12,
            text(locale, "settings.password.title").as_bytes(),
            pixel(ColorRole::TextPrimary),
        );
        for (field, key) in [
            "settings.password.current",
            "settings.password.new",
            "settings.password.confirm",
        ]
        .into_iter()
        .enumerate()
        {
            let y = 38 + 30 * field as i32;
            let row = Rect::new(12, y, 296, 24);
            painter.fill(row, pixel(ColorRole::Surface));
            if field == self.field && !self.finished {
                painter.frame(row, pixel(ColorRole::Focus));
            }
            painter.text(
                20,
                y + 8,
                text(locale, key).as_bytes(),
                pixel(ColorRole::TextPrimary),
            );
            if self.lengths[field] != 0 {
                painter.text(180, y + 8, b"********", pixel(ColorRole::TextPrimary));
            }
        }
        if let Some(status) = self.status {
            painter.text(
                12,
                138,
                text(locale, status).as_bytes(),
                pixel(ColorRole::TextPrimary),
            );
        }
    }
}

fn key_byte(code: u16) -> Option<u8> {
    match code {
        2..=12 => Some(b"1234567890-"[(code - 2) as usize]),
        16..=25 => Some(b"qwertyuiop"[(code - 16) as usize]),
        30..=38 => Some(b"asdfghjkl"[(code - 30) as usize]),
        44..=50 => Some(b"zxcvbnm"[(code - 44) as usize]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value: 1,
        }
    }

    fn fill(panel: &mut PasswordPanel, keys: &[u16]) {
        for &code in keys {
            assert!(!panel.on_key(key(code), |_, _| panic!("unexpected credential submission")));
        }
    }

    #[test]
    fn submitting_masks_and_clears_passwords_before_accepting_more_input() {
        let mut panel = PasswordPanel::new();
        fill(&mut panel, &[30, 30, 30, 30, INPUT_KEY_ENTER]);
        fill(&mut panel, &[48, 48, 48, 48, INPUT_KEY_ENTER]);
        fill(&mut panel, &[48, 48, 48, 48]);
        assert!(!panel.on_key(key(INPUT_KEY_ENTER), |current, next| {
            assert_eq!(current, b"aaaa");
            assert_eq!(next, b"bbbb");
            PasswordResult::Changed
        }));
        assert_eq!(panel.lengths, [0; 3]);
        assert_eq!(*panel.secret, [[0; MAX_PASSWORD_BYTES]; 3]);
        assert!(panel.on_key(key(INPUT_KEY_ENTER), |_, _| panic!(
            "changed password was resubmitted"
        )));
    }

    #[test]
    fn masked_content_requires_redraw_only_when_its_visibility_changes() {
        let mut panel = PasswordPanel::new();
        let empty = panel.appearance();
        fill(&mut panel, &[30]);
        let masked = panel.appearance();
        assert_ne!(empty, masked);
        fill(&mut panel, &[30, 30, 30]);
        assert_eq!(masked, panel.appearance());
        fill(&mut panel, &[INPUT_KEY_ENTER, 48]);
        panel.input_lost();
        assert_eq!(
            panel.appearance(),
            (0, [false; 3], Some("login.error.input_lost"), false)
        );
        fill(
            &mut panel,
            &[
                30,
                INPUT_KEY_ENTER,
                48,
                INPUT_KEY_ENTER,
                48,
                INPUT_KEY_ENTER,
            ],
        );
        assert_eq!(*panel.secret, [[0; MAX_PASSWORD_BYTES]; 3]);
        assert!(panel.on_key(key(INPUT_KEY_ESCAPE), |_, _| panic!(
            "lost input was resubmitted"
        )));
    }

    #[test]
    fn mismatch_never_reaches_the_verifier() {
        let mut panel = PasswordPanel::new();
        fill(&mut panel, &[30, 30, 30, 30, INPUT_KEY_ENTER]);
        fill(&mut panel, &[48, 48, 48, 48, INPUT_KEY_ENTER]);
        fill(&mut panel, &[46, 46, 46, 46, INPUT_KEY_ENTER]);
        assert_eq!(panel.status, Some("login.error.mismatch"));
        assert_eq!(*panel.secret, [[0; MAX_PASSWORD_BYTES]; 3]);
    }
}
