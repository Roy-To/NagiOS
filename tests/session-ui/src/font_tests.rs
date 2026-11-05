//! Pixel-content regressions for the real bootstrap bitmap renderer.
use crate::font;
use libnagi::{SURFACE_HEIGHT, SURFACE_WIDTH};
use std::{collections::BTreeSet, vec, vec::Vec};

const WIDTH: usize = SURFACE_WIDTH as usize;
const HEIGHT: usize = SURFACE_HEIGHT as usize;

const FG: u32 = 0xff12_3456;
const BG: u32 = 0xff65_4321;
const JAPANESE_TOFU: [u8; 7] = [0x7e, 0x42, 0x5a, 0x42, 0x5a, 0x42, 0x7e];
const ASCII_TOFU: [u8; 7] = [0x3e, 0x22, 0x2a, 0x22, 0x2a, 0x22, 0x3e];

// Independent pixel goldens for the exact summary label and unavailable text.
// The dakuten/handakuten, kana strokes, 要 and 糸/勺 parts must remain visible;
// merely changing a frame hash or replacing every character with a box fails.
pub(crate) fn summary_rows(character: char) -> [u8; 7] {
    match character {
        'ペ' => [0x02, 0x05, 0x12, 0x28, 0x44, 0x02, 0x00],
        'ー' => [0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00],
        'ジ' => [0x25, 0x4a, 0x22, 0x02, 0x04, 0x08, 0x70],
        '要' => [0x7f, 0x14, 0x3e, 0x2a, 0x7f, 0x14, 0x6b],
        '約' => [0x24, 0x4f, 0x29, 0x75, 0x21, 0x51, 0x22],
        'を' => [0x10, 0x7c, 0x20, 0x3a, 0x4c, 0x08, 0x06],
        '利' => [0x31, 0x13, 0x7b, 0x3b, 0x51, 0x11, 0x13],
        '用' => [0x3e, 0x2a, 0x3e, 0x2a, 0x3e, 0x22, 0x26],
        'で' => [0x7d, 0x05, 0x0a, 0x10, 0x10, 0x08, 0x06],
        'き' => [0x10, 0x7e, 0x08, 0x3f, 0x04, 0x20, 0x1c],
        'ま' => [0x08, 0x7e, 0x08, 0x3e, 0x08, 0x3e, 0x2a],
        'せ' => [0x22, 0x22, 0x7f, 0x22, 0x26, 0x20, 0x1e],
        'ん' => [0x10, 0x10, 0x20, 0x38, 0x24, 0x44, 0x43],
        _ => panic!("character outside summary pixel fixture: {character}"),
    }
}

pub(crate) fn assert_summary_pixels(
    surface: &[u32],
    x: usize,
    y: usize,
    text: &str,
    foreground: u32,
    background: u32,
) {
    for (cell, character) in text.chars().enumerate() {
        let rows = summary_rows(character);
        for (row, bits) in rows.into_iter().enumerate() {
            for column in 0..8 {
                let expected = if column < 7 && bits & (1 << (6 - column)) != 0 {
                    foreground
                } else {
                    background
                };
                assert_eq!(
                    surface[(y + row) * WIDTH + x + cell * 8 + column],
                    expected,
                    "{character} at cell {cell}, row {row}, column {column}"
                );
            }
        }
    }
}

fn localized<'a>(source: &'a str, key: &str) -> &'a str {
    source
        .lines()
        .find_map(|line| {
            line.split_once('=')
                .filter(|(found, _)| *found == key)
                .map(|(_, value)| value)
        })
        .expect("localized key")
}
const JA: &str = include_str!("../../../user/nagi-localization/locales/ja-JP.lang");
const EN: &str = include_str!("../../../user/nagi-localization/locales/en-US.lang");

fn cell(surface: &mut [u32], character: char) -> ([u8; 7], i32) {
    surface.fill(BG);
    let mut bytes = [0; 4];
    let advance = font::draw_utf8(
        surface,
        0,
        0,
        character.encode_utf8(&mut bytes).as_bytes(),
        FG,
    );
    let width = if character.is_ascii() { 6 } else { 7 };
    let mut rows = [0; 7];
    for (row, bits) in rows.iter_mut().enumerate() {
        for column in 0..width {
            if surface[row * WIDTH + column] == FG {
                *bits |= 1 << (width - 1 - column);
            }
        }
    }
    (rows, advance)
}

#[test]
fn summary_localizations_match_actual_glyph_pixels_and_advance() {
    let mut surface = vec![BG; WIDTH * HEIGHT];
    for key in [
        "desktop.ai.summarize_page",
        "desktop.ai.status.summary_unavailable",
    ] {
        let text = localized(JA, key);
        surface.fill(BG);
        assert_eq!(
            font::draw_utf8(&mut surface, 3, 4, text.as_bytes(), FG),
            3 + text.chars().count() as i32 * 8
        );
        assert_summary_pixels(&surface, 3, 4, text, FG, BG);
    }
}

#[test]
fn every_bar_localization_character_has_a_nonreplacement_glyph() {
    let mut surface = vec![BG; WIDTH * HEIGHT];
    for source in [JA, EN] {
        let characters: BTreeSet<_> = source
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(key, _)| key.starts_with("desktop.ai.") || *key == "desktop.lock")
            .flat_map(|(_, value)| value.chars())
            .collect();
        for character in characters {
            let (rows, advance) = cell(&mut surface, character);
            assert_eq!(
                advance,
                if character.is_ascii() { 7 } else { 8 },
                "{character}"
            );
            if character == ' ' {
                assert_eq!(rows, [0; 7]);
                continue;
            }
            assert_ne!(rows, [0; 7], "blank glyph for {character}");
            assert_ne!(
                rows,
                if character.is_ascii() {
                    ASCII_TOFU
                } else {
                    JAPANESE_TOFU
                },
                "replacement glyph for {character}"
            );
        }
    }
}

#[test]
fn unsupported_glyphs_keep_their_visible_replacement() {
    let mut surface = vec![BG; WIDTH * HEIGHT];
    assert_eq!(cell(&mut surface, '龘'), (JAPANESE_TOFU, 8));
    assert_eq!(cell(&mut surface, '~'), (ASCII_TOFU, 7));
}

#[test]
fn representative_unchanged_latin_and_japanese_glyph_pixels_are_retained() {
    let mut surface = vec![BG; WIDTH * HEIGHT];
    assert_eq!(
        cell(&mut surface, 'A'),
        ([0x1c, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x22], 7)
    );
    assert_eq!(
        cell(&mut surface, 'の'),
        ([0x1c, 0x2a, 0x4a, 0x4a, 0x52, 0x24, 0x00], 8)
    );
}

#[test]
fn summary_glyphs_clip_at_surface_edges_without_touching_guards() {
    let mut guarded: Vec<u32> = vec![BG; WIDTH * HEIGHT + 2];
    let x = WIDTH - 3;
    let y = HEIGHT - 2;
    let surface = &mut guarded[1..WIDTH * HEIGHT + 1];
    font::draw_utf8(surface, x as i32, y as i32, "要".as_bytes(), FG);
    let rows = summary_rows('要');
    for (row, bits) in rows.iter().take(2).enumerate() {
        for column in 0..3 {
            assert_eq!(
                surface[(y + row) * WIDTH + x + column],
                if bits & (1 << (6 - column)) != 0 {
                    FG
                } else {
                    BG
                }
            );
        }
    }
    assert_eq!(guarded[0], BG);
    assert_eq!(*guarded.last().unwrap(), BG);
}

#[test]
fn voicing_marks_remain_distinct_pixel_features() {
    let mut surface = vec![BG; WIDTH * HEIGHT];
    let (plain, _) = cell(&mut surface, 'シ');
    let (voiced, _) = cell(&mut surface, 'ジ');
    assert_eq!(&plain[2..], &voiced[2..]);
    assert_eq!(plain[0] ^ voiced[0], 0x05);
    assert_eq!(plain[1] ^ voiced[1], 0x0a);
    let (plain, _) = cell(&mut surface, 'へ');
    let (marked, _) = cell(&mut surface, 'ペ');
    assert_eq!(&plain[3..], &marked[3..]);
    assert_eq!(marked[0], 0x02);
    assert_eq!(marked[1] & 0x07, 0x05); // hollow handakuten centre, not a filled box
    assert_eq!(marked[2] & 0x07, 0x02);
}
