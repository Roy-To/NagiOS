//! Bounded guest-side Albert chrome renderer for Servo RGBA frames.

use crate::ui::BrowserChromeView;

const TOOLBAR_HEIGHT: u32 = 48;
const TAB_BG: [u8; 3] = [22, 32, 44];
const ACTIVE_TAB: [u8; 3] = [43, 60, 75];
const TOOLBAR_BG: [u8; 3] = [31, 44, 58];
const ADDRESS_BG: [u8; 3] = [245, 248, 249];
const ADDRESS_TEXT: [u8; 3] = [24, 39, 48];
const ICON: [u8; 3] = [214, 226, 230];
const ACCENT: [u8; 3] = [49, 180, 154];
const INVALID: [u8; 3] = [202, 62, 73];
const BORDER: [u8; 3] = [70, 88, 102];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChromeRenderError {
    InvalidDimensions,
    InvalidStride,
    TruncatedFrame,
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
    if width == 0 || height < TOOLBAR_HEIGHT {
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
        if view.address_invalid {
            INVALID
        } else {
            BORDER
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
        if cursor.saturating_add(5) > x.saturating_add(max_width) {
            break;
        }
        let bitmap = glyph(character);
        for (row, bits) in bitmap.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
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
        cursor = cursor.saturating_add(6);
    }
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

fn glyph(character: char) -> [u8; 7] {
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
        ' ' => [0; 7],
        _ => [0x1f, 0x11, 0x15, 0x11, 0x15, 0x11, 0x1f],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_state::BrowserState;
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
        assert_eq!(&frame[(60 * width) * 4..(60 * width) * 4 + 4], &[0; 4]);
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
}
