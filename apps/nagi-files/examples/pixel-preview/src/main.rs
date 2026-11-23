use std::{
    error::Error,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use nagi_files::{
    draw_three_pane, EntryAvailability, EntryKind, FileEntry, FileName, FilesCanvas,
    FilesCanvasRect, FilesViewState, Location, ResourceId,
};
use nagi_ui::{color, typography, ColorRole, FontWeight, ThemeMode, TypeRole};

extern crate nagi_abi as libnagi;

#[path = "../../../../../tools/linux-boot/notes-desktop-dev-20261116/desktop/src/font.rs"]
mod font;
#[allow(dead_code)]
#[path = "../../../../../tools/linux-boot/notes-desktop-dev-20261116/desktop/src/ui.rs"]
mod painter;

const PREVIEW_THEME: ThemeMode = ThemeMode::Light;
const BITMAP_BASE_SIZE_PX: i32 = 12;

struct NotesPainterCanvas<'a> {
    pixels: &'a mut [u32],
}

impl FilesCanvas for NotesPainterCanvas<'_> {
    fn fill_rect(&mut self, bounds: FilesCanvasRect, role: ColorRole) {
        painter::Painter::new(self.pixels).fill(to_rect(bounds), pixel(role));
    }

    fn frame_rect(&mut self, bounds: FilesCanvasRect, role: ColorRole) {
        painter::Painter::new(self.pixels).frame(to_rect(bounds), pixel(role));
    }

    fn text(
        &mut self,
        bounds: FilesCanvasRect,
        value: &str,
        typography_role: TypeRole,
        color_role: ColorRole,
    ) {
        let width = libnagi::SURFACE_WIDTH as usize;
        let mut glyphs = vec![0_u32; self.pixels.len()];
        painter::Painter::new(&mut glyphs).text(0, 0, value.as_bytes(), pixel(color_role));
        let tokens = typography(typography_role);
        let weight_extension = match tokens.weight {
            FontWeight::Regular => 0,
            FontWeight::Medium => 1,
            FontWeight::Semibold => 2,
        };
        let right = bounds.x + bounds.width.max(0);
        let bottom = bounds.y + bounds.height.max(0);

        for source_y in 0..font::GLYPH_HEIGHT as usize {
            let (top, scaled_bottom) = bitmap_cell_range(source_y, tokens.size_px);
            for source_x in 0..width {
                let glyph = glyphs[source_y * width + source_x];
                if glyph == 0 {
                    continue;
                }
                let (left, scaled_right) = bitmap_cell_range(source_x, tokens.size_px);
                for line_y in top..scaled_bottom {
                    let target_y = bounds.y + line_y;
                    if target_y < bounds.y
                        || target_y >= bottom
                        || !(0..libnagi::SURFACE_HEIGHT as i32).contains(&target_y)
                    {
                        continue;
                    }
                    for line_x in left..scaled_right + weight_extension {
                        let target_x = bounds.x + line_x;
                        if target_x < bounds.x
                            || target_x >= right
                            || !(0..libnagi::SURFACE_WIDTH as i32).contains(&target_x)
                        {
                            continue;
                        }
                        self.pixels[target_y as usize * width + target_x as usize] = glyph;
                    }
                }
            }
        }
    }
}

fn to_rect(bounds: FilesCanvasRect) -> painter::Rect {
    painter::Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

fn pixel(role: ColorRole) -> u32 {
    color(PREVIEW_THEME, role).to_pixel()
}

fn bitmap_cell_range(index: usize, size_px: u8) -> (i32, i32) {
    let index = index as i32;
    let size_px = i32::from(size_px);
    let start = index * size_px / BITMAP_BASE_SIZE_PX;
    let end = (index + 1) * size_px / BITMAP_BASE_SIZE_PX;
    (start, end.max(start + 1))
}

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "apps/nagi-files/out/files-three-pane-preview.ppm".to_owned());
    let mut state = FilesViewState::new(Location::root());
    state.entries = vec![
        entry(1, "Documents", EntryKind::Folder, None),
        entry(2, "readme.txt", EntryKind::File, Some(1435)),
    ];
    state.select(ResourceId(2), false)?;

    let mut pixels = vec![
        pixel(ColorRole::Canvas);
        libnagi::SURFACE_WIDTH as usize * libnagi::SURFACE_HEIGHT as usize
    ];
    let mut canvas = NotesPainterCanvas {
        pixels: &mut pixels,
    };
    draw_three_pane(
        &mut canvas,
        FilesCanvasRect::new(
            0,
            0,
            libnagi::SURFACE_WIDTH as i32,
            libnagi::SURFACE_HEIGHT as i32,
        ),
        &state.layout(),
        "This folder is empty.",
        nagi_files::localization::Locale::EnUs,
    );
    drop(canvas);

    write_ppm(Path::new(&output), &pixels)?;
    println!("Files pixel preview written: {output}");
    Ok(())
}

fn entry(id: u128, name: &str, kind: EntryKind, size_bytes: Option<u64>) -> FileEntry {
    FileEntry {
        id: ResourceId(id),
        name: FileName::parse(name).expect("preview entry has a valid name"),
        location: Location::root(),
        kind,
        size_bytes,
        created_at: None,
        modified_at: None,
        tags: Vec::new(),
        availability: EntryAvailability::Available,
    }
}

fn write_ppm(path: &Path, pixels: &[u32]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    let width = libnagi::SURFACE_WIDTH as usize;
    writeln!(writer, "P6\n{} {}\n255", width, libnagi::SURFACE_HEIGHT)?;
    let mut row = Vec::with_capacity(width * 3);
    for pixels in pixels.chunks_exact(width) {
        row.clear();
        for pixel in pixels {
            row.push(*pixel as u8);
            row.push((*pixel >> 8) as u8);
            row.push((*pixel >> 16) as u8);
        }
        writer.write_all(&row)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_type_role(role: TypeRole) -> Vec<usize> {
        let mut pixels =
            vec![0; libnagi::SURFACE_WIDTH as usize * libnagi::SURFACE_HEIGHT as usize];
        NotesPainterCanvas {
            pixels: &mut pixels,
        }
        .text(
            FilesCanvasRect::new(0, 0, 160, 24),
            "Files",
            role,
            ColorRole::TextPrimary,
        );
        pixels
            .into_iter()
            .enumerate()
            .filter_map(|(index, pixel)| (pixel != 0).then_some(index))
            .collect()
    }

    #[test]
    fn pixel_preview_text_reflects_type_role() {
        let caption = render_type_role(TypeRole::Caption);
        let secondary_body = render_type_role(TypeRole::SecondaryBody);
        let label = render_type_role(TypeRole::Label);

        assert_ne!(caption, secondary_body);
        assert_ne!(caption, label);
        assert_ne!(secondary_body, label);
    }
}
