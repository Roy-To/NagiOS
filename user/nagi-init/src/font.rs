#[cfg(target_arch = "x86_64")]
use core::arch::asm;

use libnagi::{SURFACE_HEIGHT, SURFACE_WIDTH};

const LATIN_GLYPH_WIDTH: i32 = 6;
#[cfg(target_arch = "x86_64")]
const GLYPH_HEIGHT: usize = 7;
const LATIN_CELL_WIDTH: i32 = 7;
const JAPANESE_GLYPH_WIDTH: i32 = 9;
const JAPANESE_CELL_WIDTH: i32 = 10;
const GLYPH_COUNT: usize = 63;

#[no_mangle]
static NAGI_FONT_GLYPHS: [[u8; 8]; GLYPH_COUNT] = [
    [b'A', 0x1c, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x22],
    [b'a', 0x00, 0x1c, 0x02, 0x1e, 0x22, 0x26, 0x1a],
    [b'C', 0x1e, 0x20, 0x20, 0x20, 0x20, 0x20, 0x1e],
    [b'c', 0x00, 0x1c, 0x22, 0x20, 0x20, 0x22, 0x1c],
    [b'F', 0x3e, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x20],
    [b'f', 0x0c, 0x10, 0x10, 0x3c, 0x10, 0x10, 0x10],
    [b'G', 0x1e, 0x20, 0x20, 0x2e, 0x22, 0x22, 0x1e],
    [b'g', 0x00, 0x1a, 0x26, 0x22, 0x1e, 0x02, 0x1c],
    [b'N', 0x22, 0x32, 0x32, 0x2a, 0x26, 0x26, 0x22],
    [b'n', 0x00, 0x2c, 0x32, 0x22, 0x22, 0x22, 0x22],
    [b'h', 0x20, 0x20, 0x2c, 0x32, 0x22, 0x22, 0x22],
    [b'P', 0x3c, 0x22, 0x22, 0x3c, 0x20, 0x20, 0x20],
    [b'p', 0x00, 0x3c, 0x22, 0x3c, 0x20, 0x20, 0x20],
    [b'S', 0x1e, 0x20, 0x20, 0x1c, 0x02, 0x02, 0x3c],
    [b's', 0x00, 0x1e, 0x20, 0x1c, 0x02, 0x02, 0x3c],
    [b'T', 0x3e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08],
    [b't', 0x10, 0x10, 0x3c, 0x10, 0x10, 0x12, 0x0c],
    [b'0', 0x1c, 0x22, 0x26, 0x2a, 0x32, 0x22, 0x1c],
    [b'1', 0x08, 0x18, 0x08, 0x08, 0x08, 0x08, 0x1c],
    [b'2', 0x1c, 0x22, 0x02, 0x04, 0x08, 0x10, 0x3e],
    [b'3', 0x3c, 0x02, 0x02, 0x1c, 0x02, 0x02, 0x3c],
    [b'4', 0x04, 0x0c, 0x14, 0x24, 0x3e, 0x04, 0x04],
    [b'=', 0x00, 0x00, 0x3e, 0x00, 0x3e, 0x00, 0x00],
    [b'+', 0x00, 0x08, 0x08, 0x3e, 0x08, 0x08, 0x00],
    [b'-', 0x00, 0x00, 0x00, 0x3e, 0x00, 0x00, 0x00],
    [b'$', 0x08, 0x1e, 0x28, 0x1c, 0x0a, 0x3c, 0x08],
    [b' ', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [b'B', 0x3c, 0x22, 0x22, 0x3c, 0x22, 0x22, 0x3c],
    [b'D', 0x3c, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3c],
    [b'E', 0x3e, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x3e],
    [b'H', 0x22, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x22],
    [b'I', 0x1c, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c],
    [b'J', 0x0e, 0x04, 0x04, 0x04, 0x04, 0x24, 0x18],
    [b'K', 0x22, 0x24, 0x28, 0x30, 0x28, 0x24, 0x22],
    [b'L', 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x3e],
    [b'M', 0x22, 0x36, 0x2a, 0x2a, 0x22, 0x22, 0x22],
    [b'O', 0x1c, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1c],
    [b'Q', 0x1c, 0x22, 0x22, 0x22, 0x2a, 0x24, 0x1a],
    [b'R', 0x3c, 0x22, 0x22, 0x3c, 0x28, 0x24, 0x22],
    [b'U', 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1c],
    [b'V', 0x22, 0x22, 0x22, 0x22, 0x22, 0x14, 0x08],
    [b'W', 0x22, 0x22, 0x22, 0x2a, 0x2a, 0x36, 0x22],
    [b'X', 0x22, 0x22, 0x14, 0x08, 0x14, 0x22, 0x22],
    [b'Y', 0x22, 0x22, 0x14, 0x08, 0x08, 0x08, 0x08],
    [b'Z', 0x3e, 0x02, 0x04, 0x08, 0x10, 0x20, 0x3e],
    [b'%', 0x32, 0x32, 0x04, 0x08, 0x10, 0x26, 0x26],
    [b'e', 0x00, 0x1c, 0x22, 0x3e, 0x20, 0x22, 0x1c],
    [b'i', 0x08, 0x00, 0x18, 0x08, 0x08, 0x08, 0x1c],
    [b'l', 0x18, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c],
    [b'm', 0x00, 0x36, 0x2a, 0x2a, 0x2a, 0x2a, 0x22],
    [b'o', 0x00, 0x1c, 0x22, 0x22, 0x22, 0x22, 0x1c],
    [b'r', 0x00, 0x2e, 0x30, 0x20, 0x20, 0x20, 0x20],
    [b'u', 0x00, 0x22, 0x22, 0x22, 0x22, 0x26, 0x1a],
    [b'x', 0x00, 0x22, 0x14, 0x08, 0x14, 0x22, 0x22],
    [b':', 0x00, 0x08, 0x08, 0x00, 0x08, 0x08, 0x00],
    [b'.', 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18],
    [b'b', 0x20, 0x20, 0x2c, 0x32, 0x22, 0x22, 0x3c],
    [b'w', 0x00, 0x22, 0x22, 0x2a, 0x2a, 0x2a, 0x14],
    [b'y', 0x00, 0x22, 0x22, 0x22, 0x1e, 0x02, 0x1c],
    [b'd', 0x02, 0x02, 0x1a, 0x26, 0x22, 0x22, 0x1e],
    [b'k', 0x20, 0x20, 0x24, 0x28, 0x30, 0x28, 0x24],
    [b'z', 0x00, 0x00, 0x3e, 0x04, 0x08, 0x10, 0x3e],
    [b'*', 0x00, 0x08, 0x2a, 0x1c, 0x2a, 0x08, 0x00],
];

#[path = "font_japanese_generated.rs"]
mod generated;

#[derive(Clone, Copy)]
enum Glyph {
    Latin([u8; 7]),
    Japanese([u16; 9]),
}

#[inline(never)]
pub fn draw_utf8(surface: &mut [u32], x: i32, y: i32, text: &[u8], color: u32) -> i32 {
    let mut index = 0;
    let mut cursor = x;
    while index < text.len() {
        let (character, length) = decode_utf8(text, index);
        match glyph_for_char(character) {
            Some(Glyph::Latin(rows)) => {
                draw_bitmap(
                    surface,
                    cursor,
                    y,
                    rows.map(u16::from),
                    LATIN_GLYPH_WIDTH,
                    color,
                );
                cursor += LATIN_CELL_WIDTH;
            }
            Some(Glyph::Japanese(rows)) => {
                draw_bitmap(surface, cursor, y, rows, JAPANESE_GLYPH_WIDTH, color);
                cursor += JAPANESE_CELL_WIDTH;
            }
            None if character.is_ascii() => {
                draw_bitmap(
                    surface,
                    cursor,
                    y,
                    ASCII_REPLACEMENT.map(u16::from),
                    LATIN_GLYPH_WIDTH,
                    color,
                );
                cursor += LATIN_CELL_WIDTH;
            }
            None => {
                draw_bitmap(
                    surface,
                    cursor,
                    y,
                    JAPANESE_REPLACEMENT,
                    JAPANESE_GLYPH_WIDTH,
                    color,
                );
                cursor += JAPANESE_CELL_WIDTH;
            }
        }
        index += length;
    }
    cursor
}

pub(crate) fn advance_for_char(character: char) -> i32 {
    if character.is_ascii() {
        LATIN_CELL_WIDTH
    } else {
        JAPANESE_CELL_WIDTH
    }
}

fn glyph_for_char(character: char) -> Option<Glyph> {
    if character.is_ascii() {
        let byte = character as u8;
        if let Some(rows) = glyph(byte) {
            return Some(Glyph::Latin(rows));
        }
        return generated::ASCII_FALLBACK_GLYPHS
            .binary_search_by_key(&byte, |(value, _)| *value)
            .ok()
            .map(|index| Glyph::Latin(generated::ASCII_FALLBACK_GLYPHS[index].1));
    }

    let codepoint = character as u32;
    generated::JAPANESE_GLYPHS
        .binary_search_by_key(&codepoint, |(value, _)| *value)
        .ok()
        .map(|index| Glyph::Japanese(generated::JAPANESE_GLYPHS[index].1))
}

fn decode_utf8(bytes: &[u8], index: usize) -> (char, usize) {
    let first = bytes[index];
    if first < 0x80 {
        return (first as char, 1);
    }

    let (length, mut codepoint, minimum) = match first {
        0xc2..=0xdf => (2, u32::from(first & 0x1f), 0x80),
        0xe0..=0xef => (3, u32::from(first & 0x0f), 0x800),
        0xf0..=0xf4 => (4, u32::from(first & 0x07), 0x10000),
        _ => return (char::REPLACEMENT_CHARACTER, 1),
    };
    if index + length > bytes.len() {
        return (char::REPLACEMENT_CHARACTER, 1);
    }
    for offset in 1..length {
        let continuation = bytes[index + offset];
        if continuation & 0xc0 != 0x80 {
            return (char::REPLACEMENT_CHARACTER, 1);
        }
        codepoint = (codepoint << 6) | u32::from(continuation & 0x3f);
    }
    if codepoint < minimum || (0xd800..=0xdfff).contains(&codepoint) || codepoint > 0x10ffff {
        return (char::REPLACEMENT_CHARACTER, 1);
    }
    (
        char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER),
        length,
    )
}

const ASCII_REPLACEMENT: [u16; 7] = [0x3e, 0x22, 0x2a, 0x22, 0x2a, 0x22, 0x3e];
const JAPANESE_REPLACEMENT: [u16; 9] = [
    0x1ff, 0x101, 0x155, 0x101, 0x155, 0x101, 0x1ff, 0x000, 0x000,
];

fn draw_bitmap<const HEIGHT: usize>(
    surface: &mut [u32],
    x: i32,
    y: i32,
    rows: [u16; HEIGHT],
    width: i32,
    color: u32,
) {
    for (row, bits) in rows.into_iter().enumerate() {
        for column in 0..width {
            if bits & (1 << (width - 1 - column)) == 0 {
                continue;
            }
            let pixel_x = x + column;
            let pixel_y = y + row as i32;
            if pixel_x >= 0
                && pixel_x < SURFACE_WIDTH as i32
                && pixel_y >= 0
                && pixel_y < SURFACE_HEIGHT as i32
            {
                let index = pixel_y as usize * SURFACE_WIDTH as usize + pixel_x as usize;
                unsafe {
                    core::ptr::write(surface.as_mut_ptr().add(index), color);
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn glyph(byte: u8) -> Option<[u8; 7]> {
    let base: *const u8;
    unsafe {
        asm!(
            "lea {base}, [rip + NAGI_FONT_GLYPHS]",
            base = out(reg) base,
            options(nostack, preserves_flags, readonly),
        );
    }
    let mut index = 0;
    while index < GLYPH_COUNT {
        let entry = unsafe { base.add(index * 8) };
        if unsafe { core::ptr::read_volatile(entry) } == byte {
            let mut rows = [0_u8; 7];
            let mut row = 0;
            while row < GLYPH_HEIGHT {
                rows[row] = unsafe { core::ptr::read_volatile(entry.add(row + 1)) };
                row += 1;
            }
            return Some(rows);
        }
        index += 1;
    }
    None
}

// The native bitmap renderer is also compiled directly into its host-only
// regression harness on the developer's non-x86 macOS host. The production
// Nagi target remains x86_64 and keeps the original assembly lookup above.
#[cfg(not(target_arch = "x86_64"))]
#[inline(never)]
fn glyph(byte: u8) -> Option<[u8; 7]> {
    NAGI_FONT_GLYPHS.iter().find_map(|entry| {
        if entry[0] != byte {
            return None;
        }
        let mut rows = [0; 7];
        rows.copy_from_slice(&entry[1..]);
        Some(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::{advance_for_char, draw_utf8, glyph_for_char, Glyph};
    use libnagi::{SURFACE_HEIGHT, SURFACE_WIDTH};

    const MAIN_JA: &str = include_str!("../../nagi-localization/locales/ja-JP.lang");
    const NOTES_JA: &str = include_str!("../../nagi-notes/locales/ja-JP.properties");
    const FILES_JA: &str = include_str!("../../../apps/nagi-files/locales/ja-JP.properties");
    const I18N_JA: &str = include_str!("../../../crates/nagi-i18n/locales/ja-JP.json");
    const INK: u32 = 0xffff_ffff;

    fn catalog_values(source: &str, output: &mut Vec<String>) {
        output.extend(source.lines().filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once('=')
                .map(|(_, value)| value.trim().to_owned())
        }));
    }

    fn json_message_values(source: &str, output: &mut Vec<String>) {
        let mut messages_started = false;
        for line in source.lines() {
            let line = line.trim();
            if !messages_started {
                if line.starts_with("\"messages\"") {
                    messages_started = true;
                }
                continue;
            }
            if line == "}" {
                break;
            }
            if let Some((_, value)) = line.split_once(':') {
                let value = value.trim().trim_end_matches(',').trim();
                if value.starts_with('"') && value.ends_with('"') {
                    output.push(value[1..value.len() - 1].to_owned());
                }
            }
        }
    }

    fn clear_glyph_area(surface: &mut [u32]) {
        for row in 0..9 {
            let start = row * SURFACE_WIDTH as usize;
            surface[start..start + 10].fill(0);
        }
    }

    #[test]
    fn every_japanese_0_1_ui_string_and_calendar_date_renders_without_missing_glyphs() {
        let mut values = Vec::new();
        let before_main = values.len();
        catalog_values(MAIN_JA, &mut values);
        assert!(
            values.len() - before_main >= 100,
            "main ja-JP UI catalog was not loaded"
        );
        let before_notes = values.len();
        catalog_values(NOTES_JA, &mut values);
        assert!(
            values.len() - before_notes >= 25,
            "Notes ja-JP catalog was not loaded"
        );
        let before_files = values.len();
        catalog_values(FILES_JA, &mut values);
        assert!(
            values.len() - before_files >= 30,
            "Files ja-JP catalog was not loaded"
        );
        let before_i18n = values.len();
        json_message_values(I18N_JA, &mut values);
        assert!(
            values.len() - before_i18n >= 7,
            "nagi-i18n ja-JP catalog was not loaded"
        );
        values.extend(
            [
                "日本語",
                "設定",
                "パスワード変更",
                "カレンダー",
                "ログアウト",
                "2026年3月5日",
                "2026年11月23日",
                "2026年11月24日",
                "09:00-10:00 event a",
                "23:00-01:00+ event c",
                "-23:00-01:00 event b edited",
                "< Previous",
                "Next >",
                "0123456789v<>-:",
                "表示できません",
            ]
            .into_iter()
            .map(str::to_owned),
        );

        let mut surface = vec![0; SURFACE_WIDTH as usize * SURFACE_HEIGHT as usize];
        for text in &values {
            let mut missing = Vec::new();
            for character in text.chars() {
                if glyph_for_char(character).is_none() {
                    missing.push(format!("U+{:04X} {:?}", character as u32, character));
                }
            }
            assert!(
                missing.is_empty(),
                "missing glyphs in {text:?}: {missing:?}"
            );

            surface.fill(0);
            let final_x = draw_utf8(&mut surface, 0, 0, text.as_bytes(), INK);
            let expected_advance: i32 = text.chars().map(advance_for_char).sum();
            assert_eq!(final_x, expected_advance, "advance for {text:?}");
            if text.chars().any(|character| !character.is_whitespace()) {
                assert!(
                    surface.iter().any(|pixel| *pixel == INK),
                    "nothing drawn for {text:?}"
                );
            }
            for character in text.chars().filter(|character| !character.is_whitespace()) {
                clear_glyph_area(&mut surface);
                let mut bytes = [0; 4];
                let encoded = character.encode_utf8(&mut bytes).as_bytes();
                let advance = draw_utf8(&mut surface, 0, 0, encoded, INK);
                assert_eq!(advance, advance_for_char(character));
                assert!(
                    surface.iter().any(|pixel| *pixel == INK),
                    "empty bitmap for U+{:04X} {:?}",
                    character as u32,
                    character
                );
            }
        }
        assert!(
            values.len() >= 160,
            "Japanese catalogs were not fully loaded"
        );
    }

    #[test]
    fn unicode_and_calendar_ascii_advance_by_one_character() {
        let mut surface = vec![0; SURFACE_WIDTH as usize * SURFACE_HEIGHT as usize];
        assert_eq!(draw_utf8(&mut surface, 0, 0, "日本語".as_bytes(), INK), 30);
        assert_eq!(draw_utf8(&mut surface, 0, 0, "5v".as_bytes(), INK), 14);
        assert_eq!(draw_utf8(&mut surface, 0, 0, "a日".as_bytes(), INK), 17);
        for character in ['5', '6', '7', '8', '9', 'v'] {
            assert!(matches!(glyph_for_char(character), Some(Glyph::Latin(_))));
        }
    }
}
