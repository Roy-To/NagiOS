//! Paint-only chrome for the Linux development launcher.
use crate::ui::{Painter, Rect};
use nagi_ui::{color, ColorRole, ThemeMode};

#[cfg(test)]
pub(crate) const FOCUS: u32 = color(ThemeMode::Light, ColorRole::Focus).to_pixel();

pub(crate) fn draw_top_bar(painter: &mut Painter<'_>, theme: ThemeMode) {
    painter.fill(
        Rect::new(0, 0, 320, 200),
        color(theme, ColorRole::Canvas).to_pixel(),
    );
    painter.fill(
        Rect::new(0, 0, 320, 20),
        color(theme, ColorRole::SurfaceSunken).to_pixel(),
    );
}

pub(crate) fn draw_window(
    painter: &mut Painter<'_>,
    rect: Rect,
    title: &[u8],
    focused: bool,
    theme: ThemeMode,
) {
    let pixel = |role| color(theme, role).to_pixel();
    painter.fill(rect, pixel(ColorRole::Surface));
    painter.frame(
        rect,
        pixel(if focused {
            ColorRole::Focus
        } else {
            ColorRole::BorderStrong
        }),
    );
    painter.fill(
        Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 14),
        pixel(ColorRole::Accent),
    );
    painter.text(
        rect.x + 6,
        rect.y + 4,
        title,
        pixel(ColorRole::TextOnAccent),
    );
}
