//! Settings input and presentation for the Linux desktop.

use super::preferences::Preferences;
use libnagi::{
    InputEvent, INPUT_EVENT_KEY, INPUT_KEY_DOWN, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE,
    INPUT_KEY_SPACE, INPUT_KEY_TAB, INPUT_KEY_UP,
};
use nagi_localization::Locale;
use nagi_ui::ThemeMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsAction {
    Ignored,
    Redraw,
    Save(Preferences),
    ChangePassword,
    Reload,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsStatus {
    Idle,
    SaveFailed,
    ReloadRequired,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    English,
    Japanese,
    Light,
    Dark,
    Password,
    Reload,
    Close,
}

const ROWS: [Row; 7] = [
    Row::English,
    Row::Japanese,
    Row::Light,
    Row::Dark,
    Row::Password,
    Row::Reload,
    Row::Close,
];

pub(crate) struct SettingsPanel {
    focus: usize,
    preferences: Preferences,
    status: SettingsStatus,
}

impl SettingsPanel {
    pub(crate) fn new(preferences: Preferences) -> Self {
        Self {
            focus: 0,
            preferences,
            status: SettingsStatus::Idle,
        }
    }

    /// Called after the owner observes preferences or confirms their save.
    pub(crate) fn show_preferences(&mut self, preferences: Preferences) {
        self.preferences = preferences;
        self.status = SettingsStatus::Idle;
    }

    pub(crate) fn show_save_error(&mut self, status: SettingsStatus) {
        self.status = status;
    }

    pub(crate) fn on_key(&mut self, event: InputEvent) -> SettingsAction {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            return SettingsAction::Ignored;
        }
        match event.code {
            INPUT_KEY_TAB | INPUT_KEY_DOWN => {
                self.focus = (self.focus + 1) % ROWS.len();
                SettingsAction::Redraw
            }
            INPUT_KEY_UP => {
                self.focus = (self.focus + ROWS.len() - 1) % ROWS.len();
                SettingsAction::Redraw
            }
            INPUT_KEY_ENTER | INPUT_KEY_SPACE => self.activate(ROWS[self.focus]),
            INPUT_KEY_ESCAPE => SettingsAction::Close,
            _ => SettingsAction::Ignored,
        }
    }

    fn activate(&self, row: Row) -> SettingsAction {
        let mut preferences = self.preferences;
        match row {
            Row::English => preferences.locale = Locale::EnUs,
            Row::Japanese => preferences.locale = Locale::JaJp,
            Row::Light => preferences.theme = ThemeMode::Light,
            Row::Dark => preferences.theme = ThemeMode::Dark,
            Row::Password => return SettingsAction::ChangePassword,
            Row::Reload => return SettingsAction::Reload,
            Row::Close => return SettingsAction::Close,
        }
        if self.status == SettingsStatus::ReloadRequired {
            SettingsAction::Ignored
        } else {
            SettingsAction::Save(preferences)
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "m10-desktop"))]
    pub(crate) fn render(&self, painter: &mut crate::ui::Painter<'_>) {
        use crate::ui::Rect;
        use nagi_localization::text;
        use nagi_ui::{color, ColorRole};

        let Preferences { locale, theme } = self.preferences;
        let pixel = |role| color(theme, role).to_pixel();
        let label = |key| text(locale, key).as_bytes();
        painter.fill(Rect::new(0, 0, 320, 200), pixel(ColorRole::Canvas));
        painter.fill(Rect::new(0, 0, 320, 20), pixel(ColorRole::SurfaceSunken));
        painter.text(
            8,
            6,
            label("desktop.settings.title"),
            pixel(ColorRole::TextPrimary),
        );

        for (index, row) in ROWS.into_iter().enumerate() {
            let y = 24 + index as i32 * 20;
            let bounds = Rect::new(8, y, 304, 18);
            let (key, selected) = match row {
                Row::English => ("desktop.settings.option.en-US", locale == Locale::EnUs),
                Row::Japanese => ("desktop.settings.option.ja-JP", locale == Locale::JaJp),
                Row::Light => ("settings.theme.light", theme == ThemeMode::Light),
                Row::Dark => ("settings.theme.dark", theme == ThemeMode::Dark),
                Row::Password => ("settings.password.title", false),
                Row::Reload => ("settings.preferences.reload", false),
                Row::Close => ("settings.preferences.close", false),
            };
            painter.fill(bounds, pixel(ColorRole::Surface));
            if index == self.focus {
                painter.frame(bounds, pixel(ColorRole::Focus));
            }
            if selected {
                painter.text(14, y + 5, b"*", pixel(ColorRole::TextPrimary));
            }
            painter.text(25, y + 5, label(key), pixel(ColorRole::TextPrimary));
        }

        let error = match self.status {
            SettingsStatus::Idle => return,
            SettingsStatus::SaveFailed => "settings.preferences.save-failed",
            SettingsStatus::ReloadRequired => "settings.preferences.reload-required",
        };
        painter.text(8, 173, label(error), pixel(ColorRole::Danger));
    }
}

#[cfg(test)]
mod tests;
