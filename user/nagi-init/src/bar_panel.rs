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
const ACCEPT: Rect = Rect::new(25, 151, 270, 17);
const COLUMNS: usize = 33;
const ROWS: usize = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Input,
    Send,
    Cancel,
    Terms,
}
pub struct Panel {
    pub controller: Controller,
    pub open: bool,
    pub cancel_requested: bool,
    focus: Focus,
    scroll: usize,
}
impl Panel {
    pub const fn new() -> Self {
        Self {
            controller: Controller::new(),
            open: false,
            cancel_requested: false,
            focus: Focus::Input,
            scroll: 0,
        }
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = Focus::Input;
    }
    pub fn clear_presentation(&mut self) {
        self.open = false;
        self.cancel_requested = false;
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
            Focus::Cancel => self.cancel_requested = true,
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
            } else if self.terms_needed() && ACCEPT.contains(x, y) {
                self.activate(Focus::Terms);
            }
            return true;
        }
        match event.code {
            libnagi::INPUT_KEY_ESCAPE => self.open = false,
            libnagi::INPUT_KEY_TAB => {
                self.focus = match self.focus {
                    Focus::Input => Focus::Send,
                    Focus::Send => Focus::Cancel,
                    Focus::Cancel if self.terms_needed() => Focus::Terms,
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
                painter.text(
                    25,
                    103,
                    wrapped_line(terms.name(), 0).as_bytes(),
                    foreground,
                );
                for row in 0..3 {
                    painter.text(
                        25,
                        115 + row as i32 * 11,
                        wrapped_line(terms.reference(), row).as_bytes(),
                        foreground,
                    );
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
            for row in 0..ROWS {
                painter.text(
                    25,
                    103 + row as i32 * 11,
                    wrapped_line(self.controller.output(), self.scroll + row).as_bytes(),
                    foreground,
                );
            }
        }
    }
}
/// Character-based wrapping retains complete UTF-8 and maps control characters
/// to the font's safe replacement glyph. It never interprets generated commands.
fn wrapped_line(text: &str, row: usize) -> &str {
    let start = text
        .char_indices()
        .nth(row * COLUMNS)
        .map_or(text.len(), |(i, _)| i);
    let end = text[start..]
        .char_indices()
        .nth(COLUMNS)
        .map_or(text.len(), |(i, _)| start + i);
    &text[start..end]
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}
