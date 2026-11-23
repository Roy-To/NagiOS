//! Backend-neutral drawing contract for the three-pane Files view.
//!
//! A Shell adapter maps the semantic color and type roles to its existing
//! Nagi palette and text painter. This module owns Files layout only; it does
//! not create a window, read input, or access a filesystem.

use nagi_ui::{ColorRole, TypeRole};

use crate::{
    localization::{self, Locale},
    EntryKind, FilesLayout, InspectorTab,
};

const OUTER_INSET: i32 = 5;
const PANE_GAP: i32 = 4;
const BREADCRUMB_HEIGHT: i32 = 22;
const PANE_TITLE_HEIGHT: i32 = 22;
const ROW_HEIGHT: i32 = 18;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesCanvasRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl FilesCanvasRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Minimal drawing surface implemented by a Shell's existing Painter.
///
/// `text` draws one line at the top-left of `bounds`, applies the supplied
/// Nagi typography and color roles, and clips the line to those bounds.
pub trait FilesCanvas {
    fn fill_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole);
    fn frame_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole);
    fn text(
        &mut self,
        bounds: FilesCanvasRect,
        value: &str,
        typography: TypeRole,
        color: ColorRole,
    );
}

/// Draw a `FilesLayout` using a Shell-provided canvas and the shared Nagi UI
/// color/typography roles. `bounds` is the component's allotted pixel area.
pub fn draw_three_pane(
    canvas: &mut impl FilesCanvas,
    bounds: FilesCanvasRect,
    layout: &FilesLayout,
    empty_message: &str,
    locale: Locale,
) {
    if bounds.width <= 0 || bounds.height <= 0 {
        return;
    }

    canvas.fill_rect(bounds, ColorRole::Canvas);

    let breadcrumb = FilesCanvasRect::new(
        bounds.x + OUTER_INSET,
        bounds.y + 2,
        (bounds.width - OUTER_INSET * 2).max(0),
        (BREADCRUMB_HEIGHT - 2).min(bounds.height),
    );
    canvas.text(
        breadcrumb,
        &breadcrumb_text(layout, locale),
        TypeRole::Caption,
        ColorRole::TextSecondary,
    );

    let content_x = bounds.x + OUTER_INSET;
    let content_y = bounds.y + BREADCRUMB_HEIGHT;
    let content_width = (bounds.width - OUTER_INSET * 2 - PANE_GAP * 2).max(0);
    let content_height = (bounds.height - BREADCRUMB_HEIGHT - OUTER_INSET).max(0);
    let sidebar_width = content_width * 26 / 100;
    let resources_width = content_width * 44 / 100;
    let inspector_width = content_width - sidebar_width - resources_width;

    let sidebar = FilesCanvasRect::new(content_x, content_y, sidebar_width, content_height);
    let resources = FilesCanvasRect::new(
        content_x + sidebar_width + PANE_GAP,
        content_y,
        resources_width,
        content_height,
    );
    let inspector = FilesCanvasRect::new(
        resources.x + resources.width + PANE_GAP,
        content_y,
        inspector_width,
        content_height,
    );

    draw_pane(canvas, sidebar, localized(locale, "files.pane.locations"));
    draw_pane(canvas, resources, localized(locale, "files.pane.resources"));
    draw_pane(canvas, inspector, localized(locale, "files.pane.inspector"));

    draw_sidebar(canvas, sidebar, layout, locale);
    draw_resources(canvas, resources, layout, empty_message);
    draw_inspector(canvas, inspector, layout, locale);
}

fn draw_pane(canvas: &mut impl FilesCanvas, bounds: FilesCanvasRect, title: &str) {
    canvas.fill_rect(bounds, ColorRole::Surface);
    canvas.frame_rect(bounds, ColorRole::Border);
    let title_bounds = FilesCanvasRect::new(
        bounds.x + 4,
        bounds.y + 1,
        (bounds.width - 8).max(0),
        (PANE_TITLE_HEIGHT - 2).min(bounds.height),
    );
    canvas.text(title_bounds, title, TypeRole::Label, ColorRole::TextPrimary);
}

fn draw_sidebar(
    canvas: &mut impl FilesCanvas,
    bounds: FilesCanvasRect,
    layout: &FilesLayout,
    locale: Locale,
) {
    let mut row_y = bounds.y + PANE_TITLE_HEIGHT;
    for item in &layout.sidebar {
        if row_y + ROW_HEIGHT > bounds.y + bounds.height {
            break;
        }
        let row = FilesCanvasRect::new(bounds.x + 4, row_y, (bounds.width - 8).max(0), ROW_HEIGHT);
        canvas.text(
            row,
            localized(locale, item.label_key),
            TypeRole::Caption,
            ColorRole::TextSecondary,
        );
        row_y += ROW_HEIGHT;
    }
}

fn draw_resources(
    canvas: &mut impl FilesCanvas,
    bounds: FilesCanvasRect,
    layout: &FilesLayout,
    empty_message: &str,
) {
    let rows_top = bounds.y + PANE_TITLE_HEIGHT;
    let visible_rows = ((bounds.y + bounds.height - rows_top).max(0) / ROW_HEIGHT) as usize;
    if layout.resources.is_empty() {
        let empty = FilesCanvasRect::new(
            bounds.x + 6,
            rows_top + 2,
            (bounds.width - 12).max(0),
            ROW_HEIGHT,
        );
        if empty.y < bounds.y || empty.y + empty.height > bounds.y + bounds.height {
            return;
        }
        canvas.text(
            empty,
            empty_message,
            TypeRole::Caption,
            ColorRole::TextSecondary,
        );
        return;
    }

    for (index, entry) in layout.resources.iter().take(visible_rows).enumerate() {
        let row = FilesCanvasRect::new(
            bounds.x + 3,
            rows_top + index as i32 * ROW_HEIGHT,
            (bounds.width - 6).max(0),
            ROW_HEIGHT,
        );
        let selected = layout.selected.contains(&entry.id);
        if selected {
            canvas.fill_rect(row, ColorRole::Selection);
            canvas.frame_rect(row, ColorRole::Focus);
        }
        let value = format!("{} {}", kind_marker(entry.kind), entry.name);
        canvas.text(
            FilesCanvasRect::new(row.x + 4, row.y, (row.width - 8).max(0), row.height),
            &value,
            TypeRole::Caption,
            ColorRole::TextPrimary,
        );
    }
}

fn draw_inspector(
    canvas: &mut impl FilesCanvas,
    bounds: FilesCanvasRect,
    layout: &FilesLayout,
    locale: Locale,
) {
    let Some(entry) = layout.inspector.as_ref() else {
        let value = localized(locale, "files.inspector.no_selection");
        draw_inspector_text(
            canvas,
            bounds,
            bounds.y + PANE_TITLE_HEIGHT + 2,
            value,
            TypeRole::SecondaryBody,
            ColorRole::TextSecondary,
        );
        return;
    };

    let mut row_y = bounds.y + PANE_TITLE_HEIGHT + 2;
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        entry.name.as_str(),
        TypeRole::Label,
        ColorRole::TextPrimary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT + 2;

    let tab = inspector_tab_text(locale, layout.inspector_tab);
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        tab,
        TypeRole::Caption,
        ColorRole::TextSecondary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT;

    let kind = format!(
        "{}: {}",
        localized(locale, "files.inspector.kind"),
        entry_kind_text(locale, entry.kind)
    );
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        &kind,
        TypeRole::Caption,
        ColorRole::TextSecondary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT;

    let size = entry
        .size_bytes
        .map_or_else(|| "—".to_owned(), |bytes| format!("{bytes} B"));
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        localized(locale, "files.inspector.size"),
        TypeRole::Caption,
        ColorRole::TextSecondary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT;
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        &size,
        TypeRole::Caption,
        ColorRole::TextPrimary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT;

    let location = if entry.location.is_root() {
        localized(locale, "files.sidebar.home").to_owned()
    } else {
        entry.location.as_str().replace('/', " - ")
    };
    if !draw_inspector_text(
        canvas,
        bounds,
        row_y,
        localized(locale, "files.inspector.location"),
        TypeRole::Caption,
        ColorRole::TextSecondary,
    ) {
        return;
    }
    row_y += ROW_HEIGHT;
    draw_inspector_text(
        canvas,
        bounds,
        row_y,
        &location,
        TypeRole::Caption,
        ColorRole::TextPrimary,
    );
}

fn draw_inspector_text(
    canvas: &mut impl FilesCanvas,
    bounds: FilesCanvasRect,
    row_y: i32,
    value: &str,
    typography: TypeRole,
    color: ColorRole,
) -> bool {
    if row_y < bounds.y || row_y + ROW_HEIGHT > bounds.y + bounds.height {
        return false;
    }
    canvas.text(
        FilesCanvasRect::new(bounds.x + 6, row_y, (bounds.width - 12).max(0), ROW_HEIGHT),
        value,
        typography,
        color,
    );
    true
}

fn breadcrumb_text(layout: &FilesLayout, locale: Locale) -> String {
    if layout.breadcrumbs.is_empty() {
        return localized(locale, "files.title").to_owned();
    }
    layout
        .breadcrumbs
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>()
        .join(" - ")
}

fn localized<'a>(locale: Locale, key: &'a str) -> &'a str {
    localization::text(locale, key).unwrap_or(key)
}

fn inspector_tab_text(locale: Locale, tab: InspectorTab) -> &'static str {
    let key = match tab {
        InspectorTab::Preview => "files.inspector.preview",
        InspectorTab::Metadata => "files.inspector.metadata",
        InspectorTab::Activity => "files.inspector.activity",
        InspectorTab::Versions => "files.inspector.versions",
        InspectorTab::Provenance => "files.inspector.provenance",
    };
    localized(locale, key)
}

fn entry_kind_text(locale: Locale, kind: EntryKind) -> &'static str {
    let key = match kind {
        EntryKind::Folder => "files.kind.folder",
        EntryKind::File => "files.kind.file",
        EntryKind::Symlink => "files.kind.symlink",
        EntryKind::Unsupported => "files.kind.unsupported",
    };
    localized(locale, key)
}

fn kind_marker(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "D:",
        EntryKind::File => "F:",
        EntryKind::Symlink => "L:",
        EntryKind::Unsupported => "U:",
    }
}

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;
