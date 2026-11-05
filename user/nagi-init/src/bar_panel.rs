//! Input/display adapter for the normal Desktop's Nagi Bar. Text has no authority.
use super::session_ui::Controller;
use crate::ui::{Painter, Rect};
use libnagi::InputEvent;
use nagi_localization::{text, Locale};
use nagi_ui::{color, ColorRole, ThemeMode};

pub const BUTTON: Rect = Rect::new(5, 1, 62, 18);
pub const LOCK_BUTTON: Rect = Rect::new(73, 1, 62, 18);
const PANEL: Rect = Rect::new(17, 25, 286, 167);
const INPUT: Rect = Rect::new(25, 46, 270, 17);
const SEND: Rect = Rect::new(25, 67, 75, 15);
const CANCEL: Rect = Rect::new(107, 67, 75, 15);
pub const SUMMARY: Rect = Rect::new(189, 67, 106, 15);
pub const SUMMARY_KEY: u16 = 63; // VirtIO/Linux F5, only inside the open Bar.
const ACCEPT: Rect = Rect::new(25, 151, 270, 17);
const COLUMNS: usize = 33;
pub(super) const WRAPPED_LINE_CAPACITY: usize = COLUMNS * 4;
const ROWS: usize = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Send,
    Cancel,
    Summary,
    Terms,
}
pub struct Panel {
    pub controller: Controller,
    pub open: bool,
    pub cancel_requested: bool,
    pub summary_requested: bool,
    summary_epoch: Option<u64>,
    wire_claimed_epoch: Option<u64>,
    focus: Focus,
    scroll: usize,
}
impl Panel {
    pub const fn new() -> Self {
        Self {
            controller: Controller::new(),
            open: false,
            cancel_requested: false,
            summary_requested: false,
            summary_epoch: Some(0),
            wire_claimed_epoch: None,
            focus: Focus::Input,
            scroll: 0,
        }
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.summary_requested = false;
        }
        self.focus = Focus::Input;
    }
    /// Per-instance monotonic identity of the current explicit local gesture.
    /// Hiding/cancelling/draining intent cannot revive it; overflow latches shut.
    pub fn summary_intent(&self) -> Option<u64> {
        if self.open && self.summary_requested && !self.cancel_requested {
            self.summary_epoch.filter(|id| *id != 0)
        } else {
            None
        }
    }
    /// Claim once per Desktop gesture, even across endpoint replacement. A
    /// failed send/clock check cannot silently retry the gesture on a new child.
    pub fn claim_summary_wire(&mut self) -> Option<u64> {
        let id = self.summary_intent()?;
        if self.wire_claimed_epoch == Some(id) {
            return None;
        }
        self.wire_claimed_epoch = Some(id);
        Some(id)
    }
    pub fn wire_summary_intent(&self) -> Option<u64> {
        self.summary_intent()
            .filter(|id| self.wire_claimed_epoch == Some(*id))
    }
    pub fn clear_presentation(&mut self) {
        self.open = false;
        self.cancel_requested = false;
        self.summary_requested = false;
        self.focus = Focus::Input;
        self.scroll = 0;
    }
    fn terms_needed(&self) -> bool {
        self.controller.terms().is_some_and(|t| !t.acknowledged())
    }
    fn activate(&mut self, focus: Focus) {
        self.focus = focus;
        match focus {
            Focus::Input | Focus::Send => {
                if self.controller.request_submit() {
                    self.scroll = 0;
                }
            }
            Focus::Cancel => {
                self.summary_requested = false;
                self.cancel_requested = true;
            }
            Focus::Summary => {
                // Record intent only. The Desktop validates its live owner,
                // Session/readiness and explicit terms on the next service step.
                if !self.controller.is_busy() {
                    self.summary_epoch = self.summary_epoch.and_then(|id| id.checked_add(1));
                    self.summary_requested = self.summary_epoch.is_some();
                }
            }
            Focus::Terms => {
                self.controller.accept_request();
            }
        }
    }
    pub fn event(&mut self, event: InputEvent, x: i32, y: i32) -> bool {
        if event.event_type != libnagi::INPUT_EVENT_KEY || event.value != 1 {
            return false;
        }
        if event.code == libnagi::INPUT_KEY_LEFT {
            if INPUT.contains(x, y) {
                self.focus = Focus::Input;
            } else if SEND.contains(x, y) {
                self.activate(Focus::Send);
            } else if CANCEL.contains(x, y) {
                self.activate(Focus::Cancel);
            } else if SUMMARY.contains(x, y) {
                self.activate(Focus::Summary);
            } else if self.terms_needed() && ACCEPT.contains(x, y) {
                self.activate(Focus::Terms);
            }
            return true;
        }
        match event.code {
            SUMMARY_KEY => self.activate(Focus::Summary),
            libnagi::INPUT_KEY_ESCAPE => self.open = false,
            libnagi::INPUT_KEY_TAB => {
                self.focus = match self.focus {
                    Focus::Input => Focus::Send,
                    Focus::Send => Focus::Cancel,
                    Focus::Cancel => Focus::Summary,
                    Focus::Summary if self.terms_needed() => Focus::Terms,
                    _ => Focus::Input,
                }
            }
            libnagi::INPUT_KEY_ENTER => self.activate(self.focus),
            libnagi::INPUT_KEY_UP => self.scroll = self.scroll.saturating_sub(1),
            libnagi::INPUT_KEY_DOWN => {
                self.scroll = (self.scroll + 1).min(
                    self.controller
                        .output()
                        .chars()
                        .count()
                        .div_ceil(COLUMNS)
                        .saturating_sub(ROWS),
                )
            }
            libnagi::login::INPUT_KEY_BACKSPACE if self.focus == Focus::Input => {
                self.controller.backspace();
            }
            libnagi::INPUT_KEY_SPACE if self.focus == Focus::Input => {
                self.controller.push_text(" ");
            }
            _ if self.focus == Focus::Input => {
                if let Some(byte) = libnagi::login::key_char(event.code) {
                    let bytes = [byte];
                    if let Ok(text) = core::str::from_utf8(&bytes) {
                        self.controller.push_text(text);
                    }
                }
            }
            _ => {}
        }
        true
    }
    pub fn render(&self, painter: &mut Painter<'_>, locale: Locale) {
        let theme = ThemeMode::Light;
        let background = color(theme, ColorRole::Surface).to_pixel();
        let foreground = color(theme, ColorRole::TextPrimary).to_pixel();
        let border = color(theme, ColorRole::BorderStrong).to_pixel();
        let focus_color = color(theme, ColorRole::Focus).to_pixel();
        for (rect, key) in [(BUTTON, "desktop.ai.title"), (LOCK_BUTTON, "desktop.lock")] {
            painter.fill(rect, background);
            painter.frame(rect, border);
            painter.text(
                rect.x + 4,
                rect.y + 6,
                text(locale, key).as_bytes(),
                foreground,
            );
        }
        if !self.open {
            return;
        }
        painter.fill(PANEL, background);
        painter.frame(PANEL, border);
        painter.text(
            25,
            33,
            text(locale, "desktop.ai.title").as_bytes(),
            foreground,
        );
        for (rect, focus, key) in [
            (INPUT, Focus::Input, None),
            (SEND, Focus::Send, Some("desktop.ai.send")),
            (CANCEL, Focus::Cancel, Some("desktop.ai.cancel")),
            (SUMMARY, Focus::Summary, Some("desktop.ai.summarize_page")),
        ] {
            painter.frame(
                rect,
                if self.focus == focus {
                    focus_color
                } else {
                    border
                },
            );
            if let Some(key) = key {
                painter.text(
                    rect.x + 4,
                    rect.y + 4,
                    text(locale, key).as_bytes(),
                    foreground,
                );
            }
        }
        let input = self.controller.input();
        let start = input
            .char_indices()
            .rev()
            .nth(COLUMNS - 1)
            .map_or(0, |(n, _)| n);
        painter.text(29, 51, &input.as_bytes()[start..], foreground);
        painter.text(
            25,
            88,
            text(locale, self.controller.status_key()).as_bytes(),
            foreground,
        );
        if self.terms_needed() {
            if let Some(terms) = self.controller.terms() {
                let mut line = [0u8; WRAPPED_LINE_CAPACITY];
                painter.text(
                    25,
                    103,
                    wrapped_line(terms.name(), 0, &mut line).as_bytes(),
                    foreground,
                );
                for row in 0..3 {
                    let wrapped = wrapped_line(terms.reference(), row, &mut line);
                    painter.text(25, 115 + row as i32 * 11, wrapped.as_bytes(), foreground);
                }
            }
            painter.frame(
                ACCEPT,
                if self.focus == Focus::Terms {
                    focus_color
                } else {
                    border
                },
            );
            painter.text(
                29,
                156,
                text(locale, "desktop.ai.accept_session").as_bytes(),
                foreground,
            );
        } else {
            let mut line = [0u8; WRAPPED_LINE_CAPACITY];
            for row in 0..ROWS {
                let wrapped = wrapped_line(self.controller.output(), self.scroll + row, &mut line);
                painter.text(25, 103 + row as i32 * 11, wrapped.as_bytes(), foreground);
            }
        }
    }
}
/// Character-based wrapping retains complete UTF-8 and replaces controls with
/// a visible safe glyph. Generated text is never interpreted as a command.
pub(super) fn wrapped_line<'a>(
    text: &str,
    row: usize,
    output: &'a mut [u8; WRAPPED_LINE_CAPACITY],
) -> &'a str {
    let first = row.saturating_mul(COLUMNS);
    let mut length = 0;
    for character in text.chars().skip(first).take(COLUMNS) {
        if character.is_control() {
            output[length] = b'?';
            length += 1;
            continue;
        }
        let mut encoded = [0u8; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        let end = length + bytes.len();
        output[length..end].copy_from_slice(bytes);
        length = end;
    }
    core::str::from_utf8(&output[..length]).unwrap_or("")
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod intent_tests {
    use super::*;
    #[test]
    fn local_gesture_close_reopen_cancel_and_new_gesture_never_reuse_intent() {
        let mut p = Panel::new();
        p.toggle();
        p.activate(Focus::Summary);
        let first = p.summary_intent().unwrap();
        p.toggle();
        p.toggle();
        assert_eq!(p.summary_intent(), None);
        p.activate(Focus::Summary);
        assert!(p.summary_intent().unwrap() > first);
        p.activate(Focus::Cancel);
        assert_eq!(p.summary_intent(), None);
        p.cancel_requested = false;
        p.activate(Focus::Summary);
        assert!(p.summary_intent().unwrap() > first + 1);
    }
    #[test]
    fn intent_overflow_stays_closed_after_more_gestures_and_presentation_reset() {
        let mut p = Panel::new();
        p.toggle();
        p.summary_epoch = Some(u64::MAX);
        p.activate(Focus::Summary);
        assert_eq!(p.summary_intent(), None);
        p.clear_presentation();
        p.toggle();
        p.activate(Focus::Summary);
        assert_eq!(p.summary_intent(), None);
    }
    #[test]
    fn wire_claim_is_once_per_gesture_and_survives_endpoint_or_presentation_changes() {
        let mut p = Panel::new();
        p.toggle();
        p.activate(Focus::Summary);
        let id = p.claim_summary_wire().unwrap();
        assert_eq!(p.wire_summary_intent(), Some(id));
        assert_eq!(p.claim_summary_wire(), None);
        p.clear_presentation();
        p.toggle();
        assert_eq!(p.claim_summary_wire(), None);
        p.activate(Focus::Summary);
        assert!(p.claim_summary_wire().unwrap() > id);
    }
}
