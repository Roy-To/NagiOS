//! Paint-only chrome for the Linux development launcher.
use crate::ui::{Painter, Rect};
use nagi_ui::{ColorRole, ThemeMode, color};

const THEME: ThemeMode = ThemeMode::Light;
const BACKGROUND: u32 = color(THEME, ColorRole::Canvas).to_pixel();
const PANEL: u32 = color(THEME, ColorRole::Surface).to_pixel();
const TITLE: u32 = color(THEME, ColorRole::Accent).to_pixel();
const BORDER: u32 = color(THEME, ColorRole::BorderStrong).to_pixel();
const TITLE_TEXT: u32 = color(THEME, ColorRole::TextOnAccent).to_pixel();
pub(crate) const FOCUS: u32 = color(THEME, ColorRole::Focus).to_pixel();

pub(crate) fn draw_top_bar(painter: &mut Painter<'_>) {
    painter.fill(Rect::new(0, 0, 320, 200), BACKGROUND);
    painter.fill(
        Rect::new(0, 0, 320, 20),
        color(THEME, ColorRole::SurfaceSunken).to_pixel(),
    );
}

pub(crate) fn draw_window(painter: &mut Painter<'_>, rect: Rect, title: &[u8], focused: bool) {
    painter.fill(rect, PANEL);
    painter.frame(rect, if focused { FOCUS } else { BORDER });
    painter.fill(Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 14), TITLE);
    painter.text(rect.x + 6, rect.y + 4, title, TITLE_TEXT);
}
