extern crate self as libnagi;

pub const SURFACE_WIDTH: u32 = 145;
pub const SURFACE_HEIGHT: u32 = 94;

#[path = "../src/font.rs"]
mod font;
#[path = "../src/recovery_layout.rs"]
mod recovery_layout;
#[path = "../src/ui.rs"]
mod ui;

use std::collections::{HashMap, HashSet};
use ui::{Painter, Rect};

const WINDOW_Y: i32 = 24;
const WINDOW_WIDTH: i32 = 145;
const WINDOW_HEIGHT: i32 = 70;
const TITLE_HEIGHT: i32 = 14;
const HINT_OFFSET: i32 = 38;
const PREFERRED_CAUSE_OFFSET: i32 = 49;
const TEXT_X: i32 = 5;
const PANEL: u32 = 0x101010;
const BORDER: u32 = 0x707070;
const TEXT: u32 = 0xf0f0f0;

const EN_CAUSE: &str = "Cause: storage full";
const JA_CAUSE: &str = "原因: 容量不足";
const JA_LOCALE: &str = include_str!("../../../../../user/nagi-localization/locales/ja-JP.lang");

fn cause_labels(locale: &'static str) -> Vec<(&'static str, &'static str)> {
    locale
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.starts_with("desktop.notes.recovery.cause.")
                .then_some((key, value))
        })
        .collect()
}

fn cause_y(viewport_height: i32) -> Option<i32> {
    recovery_layout::cause_line_y(
        WINDOW_Y,
        WINDOW_HEIGHT,
        viewport_height,
        TITLE_HEIGHT,
        HINT_OFFSET,
        PREFERRED_CAUSE_OFFSET,
        font::GLYPH_HEIGHT,
    )
}

fn render_cause(label: &str, y: i32) -> (Vec<u32>, i32) {
    let mut surface = vec![PANEL; SURFACE_WIDTH as usize * SURFACE_HEIGHT as usize];
    let window = Rect::new(0, WINDOW_Y, WINDOW_WIDTH, WINDOW_HEIGHT);
    let mut painter = Painter::new(&mut surface);
    painter.fill(window, PANEL);
    painter.frame(window, BORDER);
    let cursor_x = painter.text(TEXT_X, y, label.as_bytes(), TEXT);
    (surface, cursor_x)
}

fn row_pixels(surface: &[u32], y: i32, color: u32) -> usize {
    if y < 0 || y >= SURFACE_HEIGHT as i32 {
        return 0;
    }
    let row = y as usize * SURFACE_WIDTH as usize;
    surface[row..row + SURFACE_WIDTH as usize]
        .iter()
        .filter(|pixel| **pixel == color)
        .count()
}

fn glyph_mask(surface: &[u32], x: i32, y: i32) -> [bool; 49] {
    let mut mask = [false; 49];
    for row in 0..font::GLYPH_HEIGHT {
        for column in 0..7 {
            let pixel_x = x + column;
            let pixel_y = y + row;
            if pixel_x >= 0
                && pixel_x < SURFACE_WIDTH as i32
                && pixel_y >= 0
                && pixel_y < SURFACE_HEIGHT as i32
            {
                let index = pixel_y as usize * SURFACE_WIDTH as usize + pixel_x as usize;
                mask[row as usize * 7 + column as usize] = surface[index] == TEXT;
            }
        }
    }
    mask
}

#[test]
fn every_japanese_recovery_cause_has_real_font_glyphs_and_fits_the_window() {
    let labels = cause_labels(JA_LOCALE);
    assert_eq!(labels.len(), 11, "check every localized recovery cause");
    let viewport_bottom_cases = [94, 93];
    let y = cause_y(viewport_bottom_cases[0]).expect("the cause row fits");
    let (replacement_surface, _) = render_cause("㐀", y);
    let replacement = glyph_mask(&replacement_surface, TEXT_X, y);
    let mut glyphs_by_character = HashMap::<char, [bool; 49]>::new();
    let mut distinct_cause_glyphs = HashSet::<[bool; 49]>::new();

    for (key, label) in labels {
        for viewport_height in viewport_bottom_cases {
            let y = cause_y(viewport_height).expect("the cause row fits");
            let (surface, cursor_x) = render_cause(label, y);
            assert!(cursor_x < WINDOW_WIDTH - 1, "{key} extends past the window");

            let mut x = TEXT_X;
            for character in label.chars() {
                if character.is_ascii() {
                    x += 7;
                    continue;
                }
                let mask = glyph_mask(&surface, x, y);
                assert_ne!(
                    mask, replacement,
                    "{key} rendered replacement glyph for {character}"
                );
                assert!(
                    mask.iter().any(|pixel| *pixel),
                    "{key} rendered an empty glyph for {character}"
                );
                if let Some(previous) = glyphs_by_character.insert(character, mask) {
                    assert_eq!(previous, mask, "{character} changed glyph between labels");
                } else {
                    assert!(
                        distinct_cause_glyphs.insert(mask),
                        "{key} reuses another cause character's glyph for {character}"
                    );
                }
                x += 8;
            }
            assert_eq!(
                row_pixels(&surface, WINDOW_Y + WINDOW_HEIGHT - 1, TEXT),
                0,
                "{key} overwrote the Notes window border"
            );
            assert_eq!(
                row_pixels(&surface, viewport_height, TEXT),
                0,
                "{key} was cropped by the viewport"
            );
        }
    }
}

#[test]
fn localized_recovery_cause_stays_inside_the_notes_window_and_viewport() {
    let hint_y = WINDOW_Y + TITLE_HEIGHT + HINT_OFFSET;
    let window_border_y = WINDOW_Y + WINDOW_HEIGHT - 1;

    for (locale, label) in [("en-US", EN_CAUSE), ("ja-JP", JA_CAUSE)] {
        for viewport_height in [94, 93] {
            let y = cause_y(viewport_height).expect("viewport fits a complete cause line");
            let safe_bottom = window_border_y.min(viewport_height);
            assert_eq!(y, WINDOW_Y + TITLE_HEIGHT + 48);
            assert!(y >= hint_y + font::GLYPH_HEIGHT, "cause overlaps the hint");
            assert!(
                y + font::GLYPH_HEIGHT <= safe_bottom,
                "cause is clipped by the window border or viewport"
            );

            let (surface, cursor_x) = render_cause(label, y);
            let old_position = WINDOW_Y + TITLE_HEIGHT + PREFERRED_CAUSE_OFFSET;
            let (old_surface, _) = render_cause(label, old_position);
            assert!(
                cursor_x < WINDOW_WIDTH - 1,
                "{locale} reaches the right border"
            );
            assert!(
                (y..y + font::GLYPH_HEIGHT).any(|row| row_pixels(&surface, row, TEXT) > 0),
                "{locale} cause has no visible glyph pixels"
            );

            if viewport_height == 94 {
                assert!(
                    (0..SURFACE_WIDTH as usize).all(|x| surface
                        [window_border_y as usize * SURFACE_WIDTH as usize + x]
                        == BORDER),
                    "{locale} cause overwrote the bottom border"
                );
                assert!(
                    row_pixels(&old_surface, window_border_y, TEXT) > 0,
                    "{locale} previous position should reproduce border overlap"
                );
            } else {
                assert_eq!(
                    row_pixels(&surface, viewport_height, TEXT),
                    0,
                    "{locale} cause extends beyond the cropped viewport"
                );
                assert!(
                    row_pixels(&old_surface, viewport_height, TEXT) > 0,
                    "{locale} previous position should reproduce the cropped glyph row"
                );
            }
        }
    }
}

#[test]
fn recovery_cause_is_omitted_when_no_complete_nonoverlapping_row_fits() {
    assert_eq!(
        recovery_layout::cause_line_y(
            WINDOW_Y,
            WINDOW_HEIGHT,
            WINDOW_Y + TITLE_HEIGHT + HINT_OFFSET + font::GLYPH_HEIGHT,
            TITLE_HEIGHT,
            HINT_OFFSET,
            PREFERRED_CAUSE_OFFSET,
            font::GLYPH_HEIGHT,
        ),
        None
    );
    assert_eq!(
        recovery_layout::cause_line_y(
            WINDOW_Y,
            1,
            SURFACE_HEIGHT as i32,
            TITLE_HEIGHT,
            HINT_OFFSET,
            PREFERRED_CAUSE_OFFSET,
            font::GLYPH_HEIGHT,
        ),
        None
    );
}
