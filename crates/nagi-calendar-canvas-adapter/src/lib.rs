//! Calendar agenda drawing component at a small canvas boundary.
//!
//! Time formatting stays with the caller: the formatter receives the existing
//! canonical `Schedule` and supplies the display string for the selected locale
//! and timezone policy. This crate does not resolve or convert event times.

use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::model::Schedule;
use nagi_ui::{color, spacing, typography, Color, ColorRole, SpacingRole, ThemeMode, TypeRole};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgendaCanvasRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl AgendaCanvasRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Minimal drawing operations that can be adapted to the existing Nagi Painter.
pub trait AgendaCanvas {
    fn fill_rect(&mut self, rect: AgendaCanvasRect, color: Color);
    fn frame_rect(&mut self, rect: AgendaCanvasRect, color: Color);
    fn text(&mut self, rect: AgendaCanvasRect, value: &str, color: Color, role: TypeRole);
}

/// Draw the visible agenda rows and return the number of complete rows drawn.
///
/// `format_schedule` receives the canonical schedule without conversion. A
/// caller can pass an existing locale/timezone formatter or provide a display
/// label at the integration boundary.
pub fn render_agenda<C, F>(
    canvas: &mut C,
    bounds: AgendaCanvasRect,
    agenda: &AgendaViewModel<'_>,
    theme: ThemeMode,
    mut format_schedule: F,
) -> usize
where
    C: AgendaCanvas,
    F: FnMut(&Schedule) -> String,
{
    if bounds.width <= 0 || bounds.height <= 0 {
        return 0;
    }

    canvas.fill_rect(bounds, color(theme, ColorRole::Canvas));
    canvas.frame_rect(bounds, color(theme, ColorRole::Border));

    let horizontal_padding = i32::from(spacing(SpacingRole::Small));
    let vertical_padding = horizontal_padding;
    let line_gap = i32::from(spacing(SpacingRole::XSmall));
    let title_line_height = i32::from(typography(TypeRole::BodyStrong).line_height_px);
    let schedule_line_height = i32::from(typography(TypeRole::SecondaryBody).line_height_px);
    let row_height = vertical_padding * 2 + title_line_height + line_gap + schedule_line_height;
    let visible_rows = (bounds.height / row_height) as usize;
    let text_width = bounds.width.saturating_sub(horizontal_padding * 2).max(0);

    let mut rows_drawn = 0;
    for (index, row) in agenda.rows().take(visible_rows).enumerate() {
        let y = bounds
            .y
            .saturating_add((index as i32).saturating_mul(row_height));
        let row_bounds = AgendaCanvasRect::new(bounds.x, y, bounds.width, row_height);
        let selected = agenda.is_selected(row.event_id());
        let surface = if selected {
            ColorRole::Selection
        } else {
            ColorRole::Surface
        };
        let border = if selected {
            ColorRole::Focus
        } else {
            ColorRole::Border
        };
        canvas.fill_rect(row_bounds, color(theme, surface));
        canvas.frame_rect(row_bounds, color(theme, border));

        let title_role = if selected {
            TypeRole::BodyStrong
        } else {
            TypeRole::Body
        };
        canvas.text(
            AgendaCanvasRect::new(
                bounds.x.saturating_add(horizontal_padding),
                y.saturating_add(vertical_padding),
                text_width,
                title_line_height,
            ),
            row.title(),
            color(theme, ColorRole::TextPrimary),
            title_role,
        );

        let schedule_label = format_schedule(row.schedule());
        canvas.text(
            AgendaCanvasRect::new(
                bounds.x.saturating_add(horizontal_padding),
                y.saturating_add(vertical_padding + title_line_height + line_gap),
                text_width,
                schedule_line_height,
            ),
            &schedule_label,
            color(theme, ColorRole::TextSecondary),
            TypeRole::SecondaryBody,
        );
        rows_drawn += 1;
    }

    rows_drawn
}
