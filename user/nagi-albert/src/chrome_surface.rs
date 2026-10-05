//! Bounded guest-side Albert chrome renderer for Servo RGBA frames.

use crate::ui::BrowserChromeView;
use crate::{permission_prompt::PermissionPromptView, permission_prompt::PromptLayout};

const TOOLBAR_HEIGHT: u32 = 48;
/// Status strip (load status dot and page title) below the toolbar.
const STATUS_HEIGHT: u32 = 14;
/// First surface row of page content; the chrome owns every row above it.
pub const PAGE_TOP: u32 = TOOLBAR_HEIGHT + STATUS_HEIGHT;
const TAB_BG: [u8; 3] = [22, 32, 44];
const ACTIVE_TAB: [u8; 3] = [43, 60, 75];
const TOOLBAR_BG: [u8; 3] = [31, 44, 58];
const ADDRESS_BG: [u8; 3] = [245, 248, 249];
const ADDRESS_TEXT: [u8; 3] = [24, 39, 48];
const ICON: [u8; 3] = [214, 226, 230];
const ACCENT: [u8; 3] = [49, 180, 154];
const INVALID: [u8; 3] = [202, 62, 73];
const BORDER: [u8; 3] = [70, 88, 102];
const PROMPT_SCRIM: [u8; 3] = [12, 18, 26];
const PROMPT_BG: [u8; 3] = [244, 247, 248];
const PROMPT_TEXT: [u8; 3] = [22, 35, 45];
const PROMPT_CANCEL: [u8; 3] = [90, 105, 117];
const PROMPT_DENY: [u8; 3] = [178, 53, 67];
const PROMPT_ALLOW: [u8; 3] = [34, 139, 111];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChromeRenderError {
    InvalidDimensions,
    InvalidStride,
    TruncatedFrame,
    TextTooWide,
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Draw browser chrome into a copy of a Servo RGBA frame using guest memory only.
pub fn render_chrome(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    view: &BrowserChromeView,
) -> Result<(), ChromeRenderError> {
    if width == 0 || height < PAGE_TOP {
        return Err(ChromeRenderError::InvalidDimensions);
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or(ChromeRenderError::InvalidStride)?;
    if stride < row_bytes {
        return Err(ChromeRenderError::InvalidStride);
    }
    let required = stride
        .checked_mul(height as usize)
        .ok_or(ChromeRenderError::InvalidStride)?;
    if frame.len() < required {
        return Err(ChromeRenderError::TruncatedFrame);
    }

    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 0,
            y: 0,
            width,
            height: 20,
        },
        TAB_BG,
    );
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 0,
            y: 20,
            width,
            height: TOOLBAR_HEIGHT - 20,
        },
        TOOLBAR_BG,
    );

    let active_index = view
        .tabs
        .iter()
        .position(|tab| tab.active)
        .unwrap_or_default();
    let visible_start = active_index.saturating_sub(2);
    let visible_end = visible_start.saturating_add(3).min(view.tabs.len());
    let visible_count = visible_end.saturating_sub(visible_start);
    let hidden_count = view.tabs.len().saturating_sub(visible_count);
    let tab_area_width = width.saturating_sub(56);
    let tab_width = (tab_area_width.saturating_sub(8) / 3).clamp(40, 170);
    for (slot, tab) in view
        .tabs
        .iter()
        .skip(visible_start)
        .take(visible_count)
        .enumerate()
    {
        let x = 4 + slot as u32 * (tab_width + 3);
        fill(
            frame,
            width,
            height,
            stride,
            Rect {
                x,
                y: 2,
                width: tab_width,
                height: 17,
            },
            if tab.active { ACTIVE_TAB } else { TOOLBAR_BG },
        );
        stroke(
            frame,
            width,
            height,
            stride,
            Rect {
                x,
                y: 2,
                width: tab_width,
                height: 17,
            },
            BORDER,
        );
        draw_text(
            frame,
            width,
            height,
            stride,
            x + 6,
            7,
            &tab.title,
            ICON,
            tab_width.saturating_sub(12),
        );
    }
    if hidden_count > 0 {
        let badge = width.saturating_sub(52);
        fill(
            frame,
            width,
            height,
            stride,
            Rect {
                x: badge,
                y: 3,
                width: 20,
                height: 14,
            },
            ACTIVE_TAB,
        );
        draw_text(
            frame,
            width,
            height,
            stride,
            badge + 3,
            7,
            &format!("+{hidden_count}"),
            ICON,
            16,
        );
    }
    let add_x = width.saturating_sub(20);
    draw_line(
        frame,
        width,
        height,
        stride,
        add_x + 6,
        10,
        add_x + 14,
        10,
        ICON,
    );
    draw_line(
        frame,
        width,
        height,
        stride,
        add_x + 10,
        6,
        add_x + 10,
        14,
        ICON,
    );

    draw_back_icon(frame, width, height, stride, 12, 31, view.can_go_back);
    draw_forward_icon(frame, width, height, stride, 37, 31, view.can_go_forward);
    draw_reload_icon(frame, width, height, stride, 62, 31, view.loading);
    let address = Rect {
        x: 79,
        y: 23,
        width: width.saturating_sub(84),
        height: 20,
    };
    fill(frame, width, height, stride, address, ADDRESS_BG);
    stroke(
        frame,
        width,
        height,
        stride,
        address,
        match (view.address_invalid, view.address_focused) {
            (true, _) => INVALID,
            (false, true) => ACCENT,
            (false, false) => BORDER,
        },
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        address.x + 5,
        address.y + 6,
        &view.address_text,
        ADDRESS_TEXT,
        address.width.saturating_sub(10),
    );
    if view.loading {
        fill(
            frame,
            width,
            height,
            stride,
            Rect {
                x: 0,
                y: TOOLBAR_HEIGHT - 2,
                width,
                height: 2,
            },
            ACCENT,
        );
    }
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 0,
            y: TOOLBAR_HEIGHT,
            width,
            height: STATUS_HEIGHT,
        },
        TAB_BG,
    );
    let status_color = match view.page_status.as_str() {
        "browser.status.loading" => ACCENT,
        "browser.status.complete" => [60, 158, 117],
        "browser.status.failed" => INVALID,
        _ => ICON,
    };
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 8,
            y: TOOLBAR_HEIGHT + 4,
            width: 6,
            height: 6,
        },
        status_color,
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        20,
        TOOLBAR_HEIGHT + 4,
        &view.page_title,
        ICON,
        width.saturating_sub(26),
    );
    Ok(())
}

/// Draw an opaque first-party modal over page content. All geometry is
/// derived from the checked viewport; the page cannot draw over the prompt.
pub fn render_permission_prompt(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    prompt: &PermissionPromptView<'_>,
) -> Result<(), ChromeRenderError> {
    let Some(layout) = PromptLayout::new(width, height) else {
        return Err(ChromeRenderError::InvalidDimensions);
    };
    validate_frame(frame, width, height, stride)?;
    if text_pixel_width(prompt.origin) > layout.card.2.saturating_sub(70) {
        return Err(ChromeRenderError::TextTooWide);
    }
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        PROMPT_SCRIM,
    );
    let (card_x, card_y, card_width, card_height) = layout.card;
    let card = Rect {
        x: card_x,
        y: card_y,
        width: card_width,
        height: card_height,
    };
    fill(frame, width, height, stride, card, PROMPT_BG);
    stroke(frame, width, height, stride, card, ACCENT);
    draw_text(
        frame,
        width,
        height,
        stride,
        card_x + 8,
        card_y + 9,
        prompt.labels.title,
        PROMPT_TEXT,
        card_width.saturating_sub(16),
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        card_x + 8,
        card_y + 34,
        prompt.labels.origin,
        BORDER,
        48,
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        card_x + 62,
        card_y + 34,
        prompt.origin,
        PROMPT_TEXT,
        card_width.saturating_sub(70),
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        card_x + 8,
        card_y + 57,
        prompt.labels.feature,
        BORDER,
        72,
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        card_x + 86,
        card_y + 57,
        prompt.feature,
        PROMPT_TEXT,
        card_width.saturating_sub(94),
    );

    let button_y = layout.buttons_y;
    let buttons = [
        (prompt.labels.cancel, PROMPT_CANCEL),
        (prompt.labels.deny, PROMPT_DENY),
        (prompt.labels.allow, PROMPT_ALLOW),
    ];
    for (slot, (label, color)) in buttons.into_iter().enumerate() {
        let x = layout.buttons_x + slot as u32 * (layout.button_width + layout.button_gap);
        let button = Rect {
            x,
            y: button_y,
            width: layout.button_width,
            height: layout.buttons_bottom - button_y,
        };
        fill(frame, width, height, stride, button, color);
        let label_width = text_pixel_width(label);
        let text_x = x + layout.button_width.saturating_sub(label_width) / 2;
        draw_text(
            frame,
            width,
            height,
            stride,
            text_x,
            button_y + 7,
            label,
            [255, 255, 255],
            layout.button_width.saturating_sub(4),
        );
    }
    Ok(())
}

/// Content of Albert's trusted file picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePickerView<'a> {
    pub title: &'a str,
    pub entries: &'a [crate::file_picker::PickerEntry],
    pub selected: Option<usize>,
}

/// Draw the file picker as an opaque first-party modal; page content cannot
/// draw over it or imitate it inside the chrome-owned surface.
pub fn render_file_picker(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    picker: &FilePickerView<'_>,
) -> Result<(), ChromeRenderError> {
    use crate::file_picker::{PickerLayout, PICKER_ROW_HEIGHT};
    let Some(layout) = PickerLayout::new(width, height) else {
        return Err(ChromeRenderError::InvalidDimensions);
    };
    validate_frame(frame, width, height, stride)?;
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        PROMPT_SCRIM,
    );
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: layout.card_x,
            y: layout.card_y,
            width: layout.card_width,
            height: layout.card_height,
        },
        PROMPT_BG,
    );
    draw_text(
        frame,
        width,
        height,
        stride,
        layout.card_x + 8,
        layout.card_y + 7,
        picker.title,
        PROMPT_TEXT,
        layout.card_width.saturating_sub(16),
    );
    let visible_rows =
        (layout.card_y + layout.card_height).saturating_sub(layout.rows_y) / PICKER_ROW_HEIGHT;
    for (index, entry) in picker
        .entries
        .iter()
        .enumerate()
        .take(visible_rows as usize)
    {
        let row_y = layout.rows_y + index as u32 * PICKER_ROW_HEIGHT;
        let selected = picker.selected == Some(index);
        if selected {
            fill(
                frame,
                width,
                height,
                stride,
                Rect {
                    x: layout.card_x + 4,
                    y: row_y,
                    width: layout.card_width.saturating_sub(8),
                    height: PICKER_ROW_HEIGHT - 1,
                },
                ACCENT,
            );
        }
        let color = if selected { ADDRESS_BG } else { PROMPT_TEXT };
        let label = format!("{}  {} B", entry.name, entry.size);
        draw_text(
            frame,
            width,
            height,
            stride,
            layout.card_x + 8,
            row_y + 2,
            &label,
            color,
            layout.card_width.saturating_sub(16),
        );
    }
    Ok(())
}

/// Copy a page frame of `width` x `page_height` RGBA pixels into `frame`
/// starting at [`PAGE_TOP`], so page content never sits under the chrome.
pub fn place_page(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    page: &[u8],
    page_height: u32,
) -> Result<(), ChromeRenderError> {
    validate_frame(frame, width, height, stride)?;
    if page_height == 0 || PAGE_TOP.checked_add(page_height) != Some(height) {
        return Err(ChromeRenderError::InvalidDimensions);
    }
    let row_bytes = width as usize * 4;
    if page.len() < row_bytes * page_height as usize {
        return Err(ChromeRenderError::TruncatedFrame);
    }
    for (row, source) in page
        .chunks_exact(row_bytes)
        .take(page_height as usize)
        .enumerate()
    {
        let start = (PAGE_TOP as usize + row) * stride;
        frame[start..start + row_bytes].copy_from_slice(source);
    }
    Ok(())
}

fn validate_frame(
    frame: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<(), ChromeRenderError> {
    if width == 0 || height == 0 {
        return Err(ChromeRenderError::InvalidDimensions);
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or(ChromeRenderError::InvalidStride)?;
    if stride < row_bytes {
        return Err(ChromeRenderError::InvalidStride);
    }
    let required = stride
        .checked_mul(height as usize)
        .ok_or(ChromeRenderError::InvalidStride)?;
    if frame.len() < required {
        return Err(ChromeRenderError::TruncatedFrame);
    }
    Ok(())
}

fn text_pixel_width(text: &str) -> u32 {
    text.chars().fold(0, |width, character| {
        width.saturating_add(glyph_metrics(character).2)
    })
}

fn fill(frame: &mut [u8], width: u32, height: u32, stride: usize, rect: Rect, color: [u8; 3]) {
    let left = rect.x.min(width);
    let top = rect.y.min(height);
    let right = rect.x.saturating_add(rect.width).min(width);
    let bottom = rect.y.saturating_add(rect.height).min(height);
    for y in top..bottom {
        for x in left..right {
            let offset = y as usize * stride + x as usize * 4;
            frame[offset..offset + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
}

fn stroke(frame: &mut [u8], width: u32, height: u32, stride: usize, rect: Rect, color: [u8; 3]) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    fill(
        frame,
        width,
        height,
        stride,
        Rect { height: 1, ..rect },
        color,
    );
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            y: rect.y + rect.height - 1,
            height: 1,
            ..rect
        },
        color,
    );
    fill(
        frame,
        width,
        height,
        stride,
        Rect { width: 1, ..rect },
        color,
    );
    fill(
        frame,
        width,
        height,
        stride,
        Rect {
            x: rect.x + rect.width - 1,
            width: 1,
            ..rect
        },
        color,
    );
}

#[allow(clippy::too_many_arguments)] // Keep frame bounds explicit at each pixel write.
fn draw_text(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    x: u32,
    y: u32,
    text: &str,
    color: [u8; 3],
    max_width: u32,
) {
    let mut cursor = x;
    for character in text.chars() {
        let (bitmap, glyph_width, cell_width) = glyph_metrics(character);
        if cursor.saturating_add(glyph_width) > x.saturating_add(max_width) {
            break;
        }
        let pixel_width = if is_japanese_glyph(character) { 7 } else { 5 };
        for (row, bits) in bitmap.iter().enumerate() {
            for column in 0..pixel_width {
                if bits & (1 << (pixel_width - 1 - column)) != 0 {
                    fill(
                        frame,
                        width,
                        height,
                        stride,
                        Rect {
                            x: cursor + column,
                            y: y + row as u32,
                            width: 1,
                            height: 1,
                        },
                        color,
                    );
                }
            }
        }
        cursor = cursor.saturating_add(cell_width);
    }
}

fn glyph_metrics(character: char) -> ([u8; 7], u32, u32) {
    if let Some(bitmap) = japanese_glyph(character) {
        (bitmap, 7, 8)
    } else {
        (ascii_glyph(character), 5, 6)
    }
}

fn is_japanese_glyph(character: char) -> bool {
    japanese_glyph(character).is_some()
}

#[allow(clippy::too_many_arguments)] // Coordinates and row layout are bounds-checked together.
fn draw_line(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    color: [u8; 3],
) {
    let steps = x1.abs_diff(x2).max(y1.abs_diff(y2)).max(1);
    for step in 0..=steps {
        let x = x1 as i64 + (x2 as i64 - x1 as i64) * step as i64 / steps as i64;
        let y = y1 as i64 + (y2 as i64 - y1 as i64) * step as i64 / steps as i64;
        if x >= 0 && y >= 0 {
            fill(
                frame,
                width,
                height,
                stride,
                Rect {
                    x: x as u32,
                    y: y as u32,
                    width: 1,
                    height: 1,
                },
                color,
            );
        }
    }
}

fn draw_back_icon(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    x: u32,
    y: u32,
    enabled: bool,
) {
    let color = if enabled { ACCENT } else { ICON };
    draw_line(frame, width, height, stride, x + 7, y - 5, x + 2, y, color);
    draw_line(frame, width, height, stride, x + 2, y, x + 7, y + 5, color);
    draw_line(frame, width, height, stride, x + 2, y, x + 13, y, color);
}

fn draw_forward_icon(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    x: u32,
    y: u32,
    enabled: bool,
) {
    let color = if enabled { ACCENT } else { ICON };
    draw_line(frame, width, height, stride, x + 7, y - 5, x + 12, y, color);
    draw_line(frame, width, height, stride, x + 12, y, x + 7, y + 5, color);
    draw_line(frame, width, height, stride, x + 1, y, x + 12, y, color);
}

fn draw_reload_icon(
    frame: &mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    x: u32,
    y: u32,
    loading: bool,
) {
    let color = if loading { ACCENT } else { ICON };
    draw_line(
        frame,
        width,
        height,
        stride,
        x + 2,
        y - 4,
        x + 12,
        y - 4,
        color,
    );
    draw_line(
        frame,
        width,
        height,
        stride,
        x + 12,
        y - 4,
        x + 12,
        y + 4,
        color,
    );
    draw_line(
        frame,
        width,
        height,
        stride,
        x + 12,
        y + 4,
        x + 3,
        y + 4,
        color,
    );
    draw_line(
        frame,
        width,
        height,
        stride,
        x + 3,
        y + 4,
        x + 3,
        y - 2,
        color,
    );
}

const MISSING_GLYPH: [u8; 7] = [0x1f, 0x11, 0x15, 0x11, 0x15, 0x11, 0x1f];

fn ascii_glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0f, 0x10, 0x10, 0x13, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x19, 0x15, 0x13, 0x13, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        'a' => [0x00, 0x0e, 0x01, 0x0f, 0x11, 0x13, 0x0d],
        'b' => [0x10, 0x10, 0x1e, 0x11, 0x11, 0x11, 0x1e],
        'c' => [0x00, 0x0e, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'd' => [0x01, 0x01, 0x0f, 0x11, 0x11, 0x13, 0x0d],
        'e' => [0x00, 0x0e, 0x11, 0x1f, 0x10, 0x11, 0x0e],
        'f' => [0x06, 0x08, 0x08, 0x1e, 0x08, 0x08, 0x08],
        'g' => [0x00, 0x0d, 0x13, 0x11, 0x0f, 0x01, 0x0e],
        'h' => [0x10, 0x10, 0x1e, 0x11, 0x11, 0x11, 0x11],
        'i' => [0x04, 0x00, 0x0c, 0x04, 0x04, 0x04, 0x0e],
        'j' => [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0c],
        'k' => [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12],
        'l' => [0x0c, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'm' => [0x00, 0x1a, 0x15, 0x15, 0x15, 0x15, 0x15],
        'n' => [0x00, 0x1e, 0x11, 0x11, 0x11, 0x11, 0x11],
        'o' => [0x00, 0x0e, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'p' => [0x00, 0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10],
        'q' => [0x00, 0x0f, 0x11, 0x11, 0x0f, 0x01, 0x01],
        'r' => [0x00, 0x16, 0x19, 0x10, 0x10, 0x10, 0x10],
        's' => [0x00, 0x0f, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        't' => [0x08, 0x08, 0x1e, 0x08, 0x08, 0x09, 0x06],
        'u' => [0x00, 0x11, 0x11, 0x11, 0x11, 0x13, 0x0d],
        'v' => [0x00, 0x11, 0x11, 0x11, 0x0a, 0x0a, 0x04],
        'w' => [0x00, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'x' => [0x00, 0x11, 0x0a, 0x04, 0x04, 0x0a, 0x11],
        'y' => [0x00, 0x11, 0x11, 0x0f, 0x01, 0x11, 0x0e],
        'z' => [0x00, 0x1f, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x06, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e],
        '6' => [0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c],
        '-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1f],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '=' => [0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00, 0x00],
        '&' => [0x0c, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0d],
        '%' => [0x19, 0x1a, 0x02, 0x04, 0x08, 0x0b, 0x13],
        '#' => [0x0a, 0x1f, 0x0a, 0x0a, 0x1f, 0x0a, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1f, 0x04, 0x04, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0c, 0x04, 0x08],
        ';' => [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x04, 0x08],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '@' => [0x0e, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0e],
        '~' => [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00],
        '\'' => [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '[' => [0x0e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0e],
        ']' => [0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e],
        '*' => [0x00, 0x04, 0x15, 0x0e, 0x15, 0x04, 0x00],
        '$' => [0x04, 0x0f, 0x14, 0x0e, 0x05, 0x1e, 0x04],
        ' ' => [0; 7],
        _ => MISSING_GLYPH,
    }
}

/// Seven-pixel glyphs used by the first-party Japanese permission prompt.
/// The glyph set is intentionally local to this UI surface and covers every
/// Japanese label rendered by that prompt.
fn japanese_glyph(character: char) -> Option<[u8; 7]> {
    Some(match character {
        'サ' => [0x14, 0x14, 0x7f, 0x14, 0x24, 0x44, 0x04],
        'イ' => [0x08, 0x10, 0x10, 0x30, 0x10, 0x10, 0x10],
        'ト' => [0x08, 0x08, 0x08, 0x3e, 0x08, 0x08, 0x08],
        '機' => [0x28, 0x7e, 0x28, 0x3e, 0x2a, 0x3e, 0x2a],
        '能' => [0x7e, 0x42, 0x5a, 0x42, 0x5a, 0x42, 0x7e],
        '許' => [0x48, 0x7e, 0x08, 0x3e, 0x08, 0x08, 0x08],
        '可' => [0x7f, 0x41, 0x5d, 0x55, 0x55, 0x5d, 0x41],
        '拒' => [0x08, 0x7f, 0x08, 0x3e, 0x22, 0x22, 0x3e],
        '否' => [0x7f, 0x09, 0x09, 0x7f, 0x09, 0x09, 0x7f],
        '取' => [0x12, 0x7f, 0x12, 0x3e, 0x2a, 0x3e, 0x2a],
        '消' => [0x08, 0x7f, 0x08, 0x3e, 0x2a, 0x3e, 0x2a],
        '権' => [0x28, 0x7e, 0x28, 0x7f, 0x49, 0x7f, 0x49],
        '限' => [0x7f, 0x41, 0x5d, 0x55, 0x5d, 0x41, 0x7f],
        '位' => [0x20, 0x2e, 0x2a, 0x3e, 0x2a, 0x2a, 0x2e],
        '置' => [0x7f, 0x08, 0x7f, 0x41, 0x5d, 0x41, 0x7f],
        '情' => [0x08, 0x7f, 0x08, 0x7f, 0x49, 0x7f, 0x49],
        '報' => [0x7f, 0x49, 0x7f, 0x08, 0x3e, 0x2a, 0x3e],
        'カ' => [0x08, 0x14, 0x22, 0x7f, 0x02, 0x04, 0x08],
        'メ' => [0x10, 0x2a, 0x1c, 0x08, 0x14, 0x22, 0x00],
        'ラ' => [0x1c, 0x08, 0x08, 0x7f, 0x02, 0x04, 0x08],
        'マ' => [0x1c, 0x08, 0x08, 0x7f, 0x08, 0x08, 0x14],
        'ク' => [0x10, 0x28, 0x28, 0x7e, 0x01, 0x02, 0x04],
        '通' => [0x08, 0x3e, 0x2a, 0x3e, 0x08, 0x7f, 0x08],
        '知' => [0x3e, 0x22, 0x3e, 0x08, 0x7f, 0x49, 0x7f],
        'リ' => [0x42, 0x42, 0x42, 0x42, 0x42, 0x44, 0x38],
        'ッ' => [0x00, 0x08, 0x1c, 0x2a, 0x08, 0x08, 0x08],
        'プ' => [0x12, 0x2a, 0x1c, 0x08, 0x08, 0x08, 0x08],
        '読' => [0x48, 0x7e, 0x08, 0x3e, 0x2a, 0x3e, 0x2a],
        '書' => [0x7f, 0x08, 0x3e, 0x2a, 0x3e, 0x08, 0x7f],
        '込' => [0x10, 0x1f, 0x11, 0x15, 0x13, 0x11, 0x10],
        'フ' => [0x7e, 0x02, 0x02, 0x02, 0x04, 0x08, 0x10],
        'ァ' => [0x08, 0x3e, 0x08, 0x08, 0x04, 0x04, 0x08],
        'ル' => [0x42, 0x42, 0x42, 0x42, 0x42, 0x44, 0x38],
        '選' => [0x3e, 0x12, 0x7f, 0x2a, 0x3e, 0x08, 0x1c],
        '択' => [0x3e, 0x04, 0x7f, 0x24, 0x3e, 0x04, 0x0c],
        'ダ' => [0x08, 0x14, 0x22, 0x7f, 0x02, 0x24, 0x48],
        'ウ' => [0x08, 0x3e, 0x22, 0x22, 0x22, 0x24, 0x08],
        'ン' => [0x08, 0x08, 0x04, 0x08, 0x10, 0x22, 0x42],
        'ロ' => [0x7f, 0x41, 0x41, 0x41, 0x41, 0x41, 0x7f],
        'ー' => [0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00],
        'ド' => [0x12, 0x2a, 0x1c, 0x08, 0x08, 0x08, 0x08],
        '音' => [0x08, 0x7f, 0x08, 0x3e, 0x2a, 0x3e, 0x08],
        '声' => [0x7f, 0x08, 0x3e, 0x22, 0x3e, 0x08, 0x7f],
        '末' => [0x08, 0x08, 0x7f, 0x08, 0x18, 0x28, 0x48],
        '端' => [0x28, 0x7e, 0x28, 0x3e, 0x2a, 0x3e, 0x28],
        '背' => [0x7e, 0x42, 0x5a, 0x42, 0x7e, 0x08, 0x7f],
        '景' => [0x7e, 0x42, 0x5a, 0x42, 0x7e, 0x08, 0x3e],
        '同' => [0x7f, 0x41, 0x5d, 0x55, 0x5d, 0x41, 0x7f],
        '期' => [0x28, 0x7e, 0x28, 0x7f, 0x2a, 0x3e, 0x2a],
        '永' => [0x08, 0x1c, 0x2a, 0x08, 0x1c, 0x2a, 0x48],
        '続' => [0x28, 0x7e, 0x28, 0x3e, 0x2a, 0x7f, 0x08],
        '保' => [0x20, 0x2e, 0x2a, 0x3e, 0x2a, 0x2e, 0x20],
        '存' => [0x08, 0x7f, 0x08, 0x3e, 0x2a, 0x3e, 0x08],
        '画' => [0x7f, 0x41, 0x5d, 0x55, 0x5d, 0x41, 0x7f],
        '面' => [0x7f, 0x41, 0x5d, 0x55, 0x5d, 0x41, 0x7f],
        '維' => [0x28, 0x7e, 0x28, 0x7f, 0x49, 0x7f, 0x49],
        '持' => [0x08, 0x7f, 0x08, 0x3e, 0x2a, 0x3e, 0x08],
        'ゲ' => [0x10, 0x2a, 0x1c, 0x08, 0x24, 0x42, 0x00],
        'ム' => [0x08, 0x08, 0x14, 0x14, 0x22, 0x22, 0x41],
        'パ' => [0x12, 0x2a, 0x1c, 0x08, 0x08, 0x08, 0x08],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_state::BrowserState;
    use crate::permission_prompt::PermissionPromptLabels;
    use crate::ui::view;

    #[test]
    fn browser_chrome_composites_inside_the_real_rgba_viewport() {
        let width = 320;
        let height = 200;
        let mut frame = vec![0; width * height * 4];
        assert_eq!(
            render_chrome(
                &mut frame,
                width as u32,
                height as u32,
                width * 4,
                &view(&BrowserState::new())
            ),
            Ok(())
        );
        assert_eq!(&frame[0..4], &[TAB_BG[0], TAB_BG[1], TAB_BG[2], 255]);
        assert_eq!(
            &frame[(30 * width + 300) * 4..(30 * width + 300) * 4 + 4],
            &[ADDRESS_BG[0], ADDRESS_BG[1], ADDRESS_BG[2], 255]
        );
        // The chrome owns rows above PAGE_TOP and never draws page rows.
        let page_row = PAGE_TOP as usize * width * 4;
        assert_eq!(&frame[page_row..page_row + 4], &[0; 4]);
    }

    #[test]
    fn renderer_rejects_short_or_malformed_surfaces() {
        let view = view(&BrowserState::new());
        assert_eq!(
            render_chrome(&mut [0; 8], 320, 200, 1280, &view),
            Err(ChromeRenderError::TruncatedFrame)
        );
        assert_eq!(
            render_chrome(&mut [0; 8], 0, 200, 1280, &view),
            Err(ChromeRenderError::InvalidDimensions)
        );
    }

    #[test]
    fn renderer_shows_the_active_overflow_tab_and_page_status() {
        let mut chrome = view(&BrowserState::new());
        chrome.tabs = (0..4)
            .map(|index| crate::ui::TabView {
                id: crate::tabs::TabId(index + 1),
                title: format!("Tab {index}"),
                url: String::new(),
                active: index == 3,
                loading: false,
            })
            .collect();
        chrome.active_tab = Some(crate::tabs::TabId(4));
        chrome.page_title = "A".to_owned();
        chrome.page_status = "browser.status.failed".to_owned();

        let width = 320_usize;
        let height = 200_usize;
        let mut frame = vec![0; width * height * 4];
        render_chrome(&mut frame, width as u32, height as u32, width * 4, &chrome).unwrap();

        let active_tab_pixel = (3 * width + 181) * 4;
        assert_eq!(&frame[active_tab_pixel..active_tab_pixel + 3], &ACTIVE_TAB);
        let status_pixel = (52 * width + 8) * 4;
        assert_eq!(&frame[status_pixel..status_pixel + 3], &INVALID);
        let title_glyph_pixel = (52 * width + 21) * 4;
        assert_eq!(&frame[title_glyph_pixel..title_glyph_pixel + 3], &ICON);
    }

    #[test]
    fn every_url_character_has_an_address_bar_glyph() {
        let url_characters = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .chain("-._~:/?#[]@!$&'()*+,;=%".chars());
        for character in url_characters {
            assert_ne!(ascii_glyph(character), MISSING_GLYPH, "{character}");
        }
    }

    #[test]
    fn file_picker_covers_the_page_and_highlights_the_selection() {
        use crate::file_picker::{PickerEntry, PickerLayout, PICKER_ROW_HEIGHT};
        let width = 320_usize;
        let height = 200_usize;
        let mut frame = vec![255; width * height * 4];
        let entries = [
            PickerEntry {
                name: "a.txt".to_owned(),
                size: 3,
            },
            PickerEntry {
                name: "b.txt".to_owned(),
                size: 5,
            },
        ];
        let view = FilePickerView {
            title: "Choose a file",
            entries: &entries,
            selected: Some(1),
        };
        render_file_picker(&mut frame, width as u32, height as u32, width * 4, &view).unwrap();
        let layout = PickerLayout::new(width as u32, height as u32).unwrap();
        assert_eq!(&frame[0..3], &PROMPT_SCRIM);
        let selected_row = ((layout.rows_y + PICKER_ROW_HEIGHT + 1) as usize * width
            + (layout.card_x + layout.card_width - 6) as usize)
            * 4;
        assert_eq!(&frame[selected_row..selected_row + 3], &ACCENT);
        let unselected_row = ((layout.rows_y + 1) as usize * width
            + (layout.card_x + layout.card_width - 6) as usize)
            * 4;
        assert_eq!(&frame[unselected_row..unselected_row + 3], &PROMPT_BG);
    }

    #[test]
    fn page_is_placed_below_the_chrome_and_status_strip() {
        let width = 4_u32;
        let height = PAGE_TOP + 2;
        let mut frame = vec![0_u8; (width * height * 4) as usize];
        let page: Vec<u8> = (0..(width * 2 * 4)).map(|byte| byte as u8 + 1).collect();
        place_page(&mut frame, width, height, width as usize * 4, &page, 2).unwrap();
        let first_page_row = (PAGE_TOP * width * 4) as usize;
        assert!(frame[..first_page_row].iter().all(|byte| *byte == 0));
        assert_eq!(&frame[first_page_row..], &page[..]);
        assert_eq!(
            place_page(&mut frame, width, height, width as usize * 4, &page, 3),
            Err(ChromeRenderError::InvalidDimensions)
        );
        assert_eq!(
            place_page(&mut frame, width, height, width as usize * 4, &page[..8], 2),
            Err(ChromeRenderError::TruncatedFrame)
        );
    }

    #[test]
    fn status_strip_has_an_opaque_background() {
        let width = 320_usize;
        let height = 200_usize;
        let mut frame = vec![255; width * height * 4];
        render_chrome(
            &mut frame,
            width as u32,
            height as u32,
            width * 4,
            &view(&BrowserState::new()),
        )
        .unwrap();
        let strip_pixel = ((TOOLBAR_HEIGHT as usize + 1) * width + 300) * 4;
        assert_eq!(&frame[strip_pixel..strip_pixel + 3], &TAB_BG);
        let page_pixel = (PAGE_TOP as usize * width + 300) * 4;
        assert_eq!(&frame[page_pixel..page_pixel + 3], &[255, 255, 255]);
    }

    #[test]
    fn permission_prompt_covers_page_content_and_renders_all_three_actions() {
        let width = 320_usize;
        let height = 200_usize;
        let mut frame = vec![0; width * height * 4];
        let prompt = PermissionPromptView {
            origin: "https://camera.example",
            feature: "Camera",
            labels: crate::permission_prompt::PermissionPromptLabels::for_locale(
                nagi_localization::Locale::EnUs,
            ),
        };
        render_permission_prompt(&mut frame, 320, 200, width * 4, &prompt).unwrap();
        let layout = PromptLayout::new(320, 200).unwrap();
        let (x, y, _, _) = layout.card;
        let card_pixel = (y as usize * width + x as usize) * 4;
        assert_eq!(&frame[card_pixel..card_pixel + 3], &ACCENT);
        let scrim_pixel = (40 * width + 10) * 4;
        assert_eq!(&frame[scrim_pixel..scrim_pixel + 3], &PROMPT_SCRIM);

        for slot in 0..3 {
            let button_x = layout.buttons_x + slot * (layout.button_width + layout.button_gap);
            let pixel = (layout.buttons_y as usize * width + button_x as usize) * 4;
            assert_ne!(&frame[pixel..pixel + 3], &PROMPT_BG);
        }
    }

    #[test]
    fn permission_prompt_renders_japanese_copy() {
        let mut frame = vec![0; 320 * 200 * 4];
        let prompt = PermissionPromptView {
            origin: "https://camera.example",
            feature: PermissionPromptLabels::feature_name(
                nagi_localization::Locale::JaJp,
                crate::permissions::PermissionKind::Camera,
            ),
            labels: PermissionPromptLabels::for_locale(nagi_localization::Locale::JaJp),
        };
        assert_eq!(
            render_permission_prompt(&mut frame, 320, 200, 320 * 4, &prompt),
            Ok(())
        );
    }

    #[test]
    fn permission_prompt_rejects_invalid_frame_bounds() {
        let prompt = PermissionPromptView {
            origin: "https://example.test",
            feature: "Camera",
            labels: crate::permission_prompt::PermissionPromptLabels::for_locale(
                nagi_localization::Locale::EnUs,
            ),
        };
        assert_eq!(
            render_permission_prompt(&mut [0; 8], 320, 200, 1280, &prompt),
            Err(ChromeRenderError::TruncatedFrame)
        );
        assert_eq!(
            render_permission_prompt(&mut [0; 8], 200, 200, 800, &prompt),
            Err(ChromeRenderError::InvalidDimensions)
        );
    }

    #[test]
    fn permission_prompt_refuses_to_truncate_the_requesting_origin() {
        let mut frame = vec![0; 320 * 200 * 4];
        let origin = format!("https://{}.example", "long-subdomain-".repeat(5));
        let prompt = PermissionPromptView {
            origin: &origin,
            feature: "Camera",
            labels: crate::permission_prompt::PermissionPromptLabels::for_locale(
                nagi_localization::Locale::EnUs,
            ),
        };
        assert_eq!(
            render_permission_prompt(&mut frame, 320, 200, 320 * 4, &prompt),
            Err(ChromeRenderError::TextTooWide)
        );
    }
}
