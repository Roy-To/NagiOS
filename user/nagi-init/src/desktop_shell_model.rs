use crate::desktop_chrome::{draw_top_bar, draw_window};
use crate::ui::{Painter, Rect};
use libnagi::{
    InputEvent, INPUT_EVENT_KEY, INPUT_KEY_DOWN, INPUT_KEY_ENTER, INPUT_KEY_SPACE, INPUT_KEY_TAB,
    INPUT_KEY_UP,
};
use nagi_localization::Locale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppId {
    Notes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellFocus {
    App(AppId),
    Logout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShellAction {
    Redraw,
    Launch(AppId),
    Logout,
    Ignored,
}

pub(super) struct ShellModel {
    focus: ShellFocus,
    locale: Locale,
}

const NOTES_TILE: Rect = Rect::new(167, 24, 145, 70);
const LOGOUT_BUTTON: Rect = Rect::new(244, 1, 74, 18);

impl ShellModel {
    pub(super) const fn new(locale: Locale) -> Self {
        Self {
            focus: ShellFocus::App(AppId::Notes),
            locale,
        }
    }

    pub(super) fn on_key(&mut self, event: InputEvent) -> ShellAction {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            return ShellAction::Ignored;
        }
        match event.code {
            INPUT_KEY_TAB | INPUT_KEY_UP | INPUT_KEY_DOWN => {
                self.focus = match self.focus {
                    ShellFocus::App(_) => ShellFocus::Logout,
                    ShellFocus::Logout => ShellFocus::App(AppId::Notes),
                };
                ShellAction::Redraw
            }
            INPUT_KEY_ENTER | INPUT_KEY_SPACE => match self.focus {
                ShellFocus::App(app) => ShellAction::Launch(app),
                ShellFocus::Logout => ShellAction::Logout,
            },
            _ => ShellAction::Ignored,
        }
    }

    pub(super) fn render(&self, painter: &mut Painter<'_>) {
        draw_top_bar(painter);
        draw_window(
            painter,
            NOTES_TILE,
            nagi_localization::text(self.locale, "desktop.notes.title").as_bytes(),
            self.focus == ShellFocus::App(AppId::Notes),
        );
        draw_window(
            painter,
            LOGOUT_BUTTON,
            nagi_localization::text(self.locale, "desktop.shell.logout").as_bytes(),
            self.focus == ShellFocus::Logout,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_chrome::FOCUS;

    fn key(code: u16, value: i32) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value,
        }
    }

    #[test]
    fn focus_cycles_and_only_key_press_activates() {
        let mut model = ShellModel::new(Locale::EnUs);
        assert_eq!(model.on_key(key(INPUT_KEY_ENTER, 0)), ShellAction::Ignored);
        assert_eq!(model.on_key(key(INPUT_KEY_ENTER, 2)), ShellAction::Ignored);
        assert_eq!(
            model.on_key(key(INPUT_KEY_ENTER, 1)),
            ShellAction::Launch(AppId::Notes)
        );
        assert_eq!(model.on_key(key(INPUT_KEY_TAB, 1)), ShellAction::Redraw);
        assert_eq!(model.on_key(key(INPUT_KEY_SPACE, 1)), ShellAction::Logout);
        assert_eq!(model.on_key(key(INPUT_KEY_UP, 1)), ShellAction::Redraw);
        assert_eq!(
            model.on_key(key(INPUT_KEY_ENTER, 1)),
            ShellAction::Launch(AppId::Notes)
        );
    }

    #[test]
    fn focus_border_follows_the_selected_tile() {
        let mut model = ShellModel::new(Locale::EnUs);
        let mut pixels = vec![0; 320 * 200];
        let corner = |rect: Rect| (rect.y * 320 + rect.x) as usize;
        model.render(&mut Painter::new(&mut pixels));
        assert_eq!(pixels[corner(NOTES_TILE)], FOCUS);
        assert_ne!(pixels[corner(LOGOUT_BUTTON)], FOCUS);
        model.on_key(key(INPUT_KEY_DOWN, 1));
        model.render(&mut Painter::new(&mut pixels));
        assert_ne!(pixels[corner(NOTES_TILE)], FOCUS);
        assert_eq!(pixels[corner(LOGOUT_BUTTON)], FOCUS);
    }
}
