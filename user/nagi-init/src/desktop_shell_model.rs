use crate::desktop_chrome::{draw_top_bar, draw_window};
use crate::ui::{Painter, Rect};
use libnagi::{
    InputEvent, INPUT_EVENT_KEY, INPUT_KEY_DOWN, INPUT_KEY_ENTER, INPUT_KEY_SPACE, INPUT_KEY_TAB,
    INPUT_KEY_UP,
};
use nagi_localization::Locale;
use nagi_ui::ThemeMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AppId {
    Notes,
    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
    Calendar,
    #[cfg(linux_task24_settings)]
    Settings,
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
    theme: ThemeMode,
    #[cfg(linux_task24_settings)]
    settings: bool,
}

#[cfg(linux_task24_settings)]
#[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
const SETTINGS_TILE: Rect = Rect::new(216, 24, 96, 70);
#[cfg(all(
    linux_task24_settings,
    not(all(linux_task24_calendar_app, not(nagi_release_build)))
))]
const SETTINGS_TILE: Rect = Rect::new(8, 24, 145, 70);
#[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
const CALENDAR_TILE: Rect = Rect::new(8, 24, 96, 70);
#[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
const NOTES_TILE: Rect = Rect::new(112, 24, 96, 70);
#[cfg(not(all(linux_task24_calendar_app, not(nagi_release_build))))]
const NOTES_TILE: Rect = Rect::new(167, 24, 145, 70);
const LOGOUT_BUTTON: Rect = Rect::new(244, 1, 74, 18);

impl ShellModel {
    pub(super) const fn new(locale: Locale) -> Self {
        Self {
            focus: ShellFocus::App(AppId::Notes),
            locale,
            theme: ThemeMode::Light,
            #[cfg(linux_task24_settings)]
            settings: false,
        }
    }

    pub(super) fn locale(&self) -> Locale {
        self.locale
    }

    #[cfg(linux_task24_settings)]
    pub(super) fn apply_preferences(
        &mut self,
        preferences: crate::settings_store::preferences::Preferences,
    ) {
        self.locale = preferences.locale;
        self.theme = preferences.theme;
        self.settings = true;
    }

    pub(super) fn on_key(&mut self, event: InputEvent) -> ShellAction {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            return ShellAction::Ignored;
        }
        match event.code {
            INPUT_KEY_TAB | INPUT_KEY_UP | INPUT_KEY_DOWN => {
                let order = [
                    ShellFocus::App(AppId::Notes),
                    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
                    ShellFocus::App(AppId::Calendar),
                    #[cfg(linux_task24_settings)]
                    ShellFocus::App(AppId::Settings),
                    ShellFocus::Logout,
                ];
                let current = order
                    .iter()
                    .position(|focus| *focus == self.focus)
                    .unwrap_or(0);
                let step = if event.code == INPUT_KEY_UP {
                    order.len() - 1
                } else {
                    1
                };
                let mut next = current;
                for _ in 0..order.len() {
                    next = (next + step) % order.len();
                    #[cfg(linux_task24_settings)]
                    if order[next] == ShellFocus::App(AppId::Settings) && !self.settings {
                        continue;
                    }
                    break;
                }
                self.focus = order[next];
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
        draw_top_bar(painter, self.theme);
        #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
        draw_window(
            painter,
            CALENDAR_TILE,
            nagi_localization::text(self.locale, "calendar.title").as_bytes(),
            self.focus == ShellFocus::App(AppId::Calendar),
            self.theme,
        );
        draw_window(
            painter,
            NOTES_TILE,
            nagi_localization::text(self.locale, "desktop.notes.title").as_bytes(),
            self.focus == ShellFocus::App(AppId::Notes),
            self.theme,
        );
        draw_window(
            painter,
            LOGOUT_BUTTON,
            nagi_localization::text(self.locale, "desktop.shell.logout").as_bytes(),
            self.focus == ShellFocus::Logout,
            self.theme,
        );
        #[cfg(linux_task24_settings)]
        if self.settings {
            draw_window(
                painter,
                SETTINGS_TILE,
                nagi_localization::text(self.locale, "desktop.settings.title").as_bytes(),
                self.focus == ShellFocus::App(AppId::Settings),
                self.theme,
            );
        }
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
        #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
        assert_eq!(
            model.on_key(key(INPUT_KEY_SPACE, 1)),
            ShellAction::Launch(AppId::Calendar)
        );
        assert_eq!(model.on_key(key(INPUT_KEY_TAB, 1)), ShellAction::Redraw);
        assert_eq!(model.on_key(key(INPUT_KEY_SPACE, 1)), ShellAction::Logout);
        assert_eq!(model.on_key(key(INPUT_KEY_UP, 1)), ShellAction::Redraw);
        #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
        assert_eq!(
            model.on_key(key(INPUT_KEY_ENTER, 1)),
            ShellAction::Launch(AppId::Calendar)
        );
        #[cfg(not(all(linux_task24_calendar_app, not(nagi_release_build))))]
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
        #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
        {
            assert_ne!(pixels[corner(NOTES_TILE)], FOCUS);
            assert_eq!(pixels[corner(CALENDAR_TILE)], FOCUS);
            model.on_key(key(INPUT_KEY_DOWN, 1));
            model.render(&mut Painter::new(&mut pixels));
            assert_eq!(pixels[corner(LOGOUT_BUTTON)], FOCUS);
        }
        #[cfg(not(all(linux_task24_calendar_app, not(nagi_release_build))))]
        {
            assert_ne!(pixels[corner(NOTES_TILE)], FOCUS);
            assert_eq!(pixels[corner(LOGOUT_BUTTON)], FOCUS);
        }
    }
}
