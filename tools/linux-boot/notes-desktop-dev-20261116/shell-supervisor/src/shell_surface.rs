//! The single parent-owned physical display/focus adapter. Apps send frames;
//! this owner alone writes the framebuffer and commits keyboard focus after a
//! successful full-frame flush.

use crate::notes_bootstrap::SurfaceId;
use nagi_files::{render_three_pane, FilesApp, FilesErrorKind, Location, ViewLoadState};
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};

const FRAME_WIDTH: usize = 320;
const FRAME_HEIGHT: usize = 200;
const PIXEL_COUNT: usize = FRAME_WIDTH * FRAME_HEIGHT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Shell,
    Notes,
    Files,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgba8888,
    Bgra8888,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedDisplayLayout {
    pub width: usize,
    pub height: usize,
    pub stride_bytes: usize,
    pub mapped_bytes: usize,
    pub format: PixelFormat,
}

impl ValidatedDisplayLayout {
    pub fn validate(self) -> Result<Self, DisplayError> {
        let row_bytes = self
            .width
            .checked_mul(4)
            .ok_or(DisplayError::InvalidLayout)?;
        let last_row_end = self
            .stride_bytes
            .checked_mul(self.height.saturating_sub(1))
            .and_then(|offset| offset.checked_add(row_bytes))
            .ok_or(DisplayError::InvalidLayout)?;
        if self.width != FRAME_WIDTH
            || self.height != FRAME_HEIGHT
            || self.stride_bytes < row_bytes
            || self.mapped_bytes < last_row_end
        {
            return Err(DisplayError::InvalidLayout);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayError {
    InvalidLayout,
    InvalidFrame,
    Io(io::ErrorKind),
    GenerationExhausted,
    NotesSurfaceUnavailable,
}

impl From<io::Error> for DisplayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentedFocus {
    pub previous_surface_id: SurfaceId,
    pub surface_id: SurfaceId,
    pub owner_pid: u32,
    pub generation: u64,
    pub visible: bool,
    pub presented: bool,
    pub keyboard_active: bool,
    pub focus_committed: bool,
}

/// Unique owner of the passed framebuffer descriptor. It cannot clone it or
/// hand the device FD to Notes/Files. The boot caller must pass the descriptor
/// after its existing Linux display grant/format validation.
pub struct FramebufferPresenter {
    file: File,
    layout: ValidatedDisplayLayout,
    next_generation: u64,
}

impl FramebufferPresenter {
    pub fn new(file: File, layout: ValidatedDisplayLayout) -> Result<Self, DisplayError> {
        Ok(Self {
            file,
            layout: layout.validate()?,
            next_generation: 0,
        })
    }

    pub fn present(&mut self, frame: &SurfaceFrame) -> Result<u64, DisplayError> {
        if frame.pixels.len() != PIXEL_COUNT {
            return Err(DisplayError::InvalidFrame);
        }
        let row_bytes = self.layout.width * 4;
        let mut row = Vec::with_capacity(row_bytes);
        for y in 0..FRAME_HEIGHT {
            let offset = y
                .checked_mul(self.layout.stride_bytes)
                .ok_or(DisplayError::InvalidLayout)?;
            self.file.seek(SeekFrom::Start(offset as u64))?;
            row.clear();
            let source = &frame.pixels[y * FRAME_WIDTH..(y + 1) * FRAME_WIDTH];
            for pixel in source {
                row.extend_from_slice(&convert_pixel(*pixel, self.layout.format).to_ne_bytes());
            }
            self.file.write_all(&row)?;
        }
        self.file.flush()?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(DisplayError::GenerationExhausted)?;
        Ok(self.next_generation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SurfaceFrame {
    pub surface_id: SurfaceId,
    pub pixels: Vec<u32>,
}

pub struct ShellSurfaceOwner {
    display: FramebufferPresenter,
    shell_id: SurfaceId,
    notes_id: SurfaceId,
    files_id: SurfaceId,
    focused: Surface,
    files: FilesApp,
    locale: nagi_files::localization::Locale,
}

impl ShellSurfaceOwner {
    pub fn new(
        display: FramebufferPresenter,
        shell_id: SurfaceId,
        notes_id: SurfaceId,
        files_id: SurfaceId,
    ) -> Result<(Self, PresentedFocus), DisplayError> {
        if shell_id == notes_id || shell_id == files_id || notes_id == files_id {
            return Err(DisplayError::InvalidLayout);
        }
        let mut files = FilesApp::new(Location::root());
        files.state.load_state = ViewLoadState::Error(FilesErrorKind::ProviderUnavailable);
        let mut owner = Self {
            display,
            shell_id,
            notes_id,
            files_id,
            focused: Surface::Shell,
            files,
            locale: nagi_files::localization::Locale::EnUs,
        };
        let commit = owner.present_focus(Surface::Shell, None)?;
        Ok((owner, commit))
    }

    pub const fn focused(&self) -> Surface {
        self.focused
    }

    pub fn input_owner(&self) -> SurfaceId {
        self.surface_id(self.focused)
    }

    pub fn files_frame_text(&self) -> String {
        render_three_pane(
            &self.files.state.layout(),
            "Filesystem provider unavailable",
            self.locale,
        )
    }

    /// Focus is changed only after this owner writes and flushes the selected
    /// frame. `None` is accepted for Shell and Files; Notes requires an actual
    /// child-produced frame carrying the assigned surface ID.
    pub fn present_focus(
        &mut self,
        requested: Surface,
        notes: Option<&SurfaceFrame>,
    ) -> Result<PresentedFocus, DisplayError> {
        let frame = self.render_surface(requested, notes)?;
        let generation = self.display.present(&frame)?;
        let previous_surface_id = self.surface_id(self.focused);
        let surface_id = self.surface_id(requested);
        self.focused = requested;
        Ok(PresentedFocus {
            previous_surface_id,
            surface_id,
            owner_pid: std::process::id(),
            generation,
            visible: true,
            presented: true,
            keyboard_active: true,
            focus_committed: true,
        })
    }

    pub fn present_next(
        &mut self,
        notes: Option<&SurfaceFrame>,
    ) -> Result<PresentedFocus, DisplayError> {
        let requested = match self.focused {
            Surface::Shell if notes.is_some() => Surface::Notes,
            Surface::Shell => Surface::Files,
            Surface::Notes => Surface::Files,
            Surface::Files if notes.is_some() => Surface::Shell,
            Surface::Files => Surface::Shell,
        };
        self.present_focus(requested, notes)
    }

    pub fn display_ack(&self, notes: &SurfaceFrame, committed: &PresentedFocus) -> bool {
        self.focused == Surface::Notes
            && committed.surface_id == self.notes_id
            && notes.surface_id == self.notes_id
            && committed.generation > 0
            && committed.presented
            && committed.focus_committed
    }

    fn render_surface(
        &self,
        surface: Surface,
        notes: Option<&SurfaceFrame>,
    ) -> Result<SurfaceFrame, DisplayError> {
        let (surface_id, pixels) = match surface {
            Surface::Shell => (
                self.shell_id,
                // Home is the launch surface; Shell is the single boot-owned
                // process that retains its display and input ownership.
                render_text_pixels("NAGI HOME\nENTER: OPEN NOTES\nTAB: CHANGE FOCUS"),
            ),
            Surface::Notes => {
                let notes = notes.ok_or(DisplayError::NotesSurfaceUnavailable)?;
                if notes.surface_id != self.notes_id || notes.pixels.len() != PIXEL_COUNT {
                    return Err(DisplayError::InvalidFrame);
                }
                (self.notes_id, notes.pixels.clone())
            }
            Surface::Files => (self.files_id, render_text_pixels(&self.files_frame_text())),
        };
        Ok(SurfaceFrame { surface_id, pixels })
    }

    fn surface_id(&self, surface: Surface) -> SurfaceId {
        match surface {
            Surface::Shell => self.shell_id,
            Surface::Notes => self.notes_id,
            Surface::Files => self.files_id,
        }
    }
}

fn render_text_pixels(text: &str) -> Vec<u32> {
    let mut pixels = vec![0x0018_2028; PIXEL_COUNT];
    let mut x = 8usize;
    let mut y = 8usize;
    for character in text.chars() {
        if character == '\n' || x + 6 >= FRAME_WIDTH {
            x = 8;
            y += 9;
            if character == '\n' || y + 7 >= FRAME_HEIGHT {
                continue;
            }
        }
        if y + 7 >= FRAME_HEIGHT {
            break;
        }
        let glyph = surface_glyph(character.to_ascii_uppercase());
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    pixels[(y + row) * FRAME_WIDTH + x + column] = 0x00f0_f2f5;
                }
            }
        }
        x += 6;
    }
    pixels
}

fn surface_glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0; 7],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

fn convert_pixel(pixel: u32, format: PixelFormat) -> u32 {
    match format {
        PixelFormat::Rgba8888 => pixel,
        PixelFormat::Bgra8888 => {
            let red = pixel & 0x0000_00ff;
            let blue = pixel & 0x00ff_0000;
            (pixel & 0xff00_ff00) | (red << 16) | (blue >> 16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn temporary_framebuffer() -> (File, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "nagi-task28-framebuffer-{}-{}.raw",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (file, path)
    }

    #[test]
    fn real_files_renderer_is_presented_before_focus_and_input_owner_commit() {
        let (file, path) = temporary_framebuffer();
        let layout = ValidatedDisplayLayout {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            stride_bytes: FRAME_WIDTH * 4,
            mapped_bytes: PIXEL_COUNT * 4,
            format: PixelFormat::Rgba8888,
        };
        let display = FramebufferPresenter::new(file, layout).unwrap();
        let (mut owner, initial) = ShellSurfaceOwner::new(
            display,
            SurfaceId::new(1).unwrap(),
            SurfaceId::new(2).unwrap(),
            SurfaceId::new(3).unwrap(),
        )
        .unwrap();
        assert_eq!(initial.surface_id.get(), 1);
        assert_eq!(owner.input_owner().get(), 1);
        let text = owner.files_frame_text();
        assert!(text.contains("Filesystem provider unavailable"));
        let files_focus = owner.present_focus(Surface::Files, None).unwrap();
        assert_eq!(files_focus.previous_surface_id.get(), 1);
        assert_eq!(files_focus.surface_id.get(), 3);
        assert_eq!(owner.input_owner().get(), 3);
        assert!(
            files_focus.visible
                && files_focus.presented
                && files_focus.keyboard_active
                && files_focus.focus_committed
        );
        let notes = SurfaceFrame {
            surface_id: SurfaceId::new(2).unwrap(),
            pixels: vec![0x0012_3456; PIXEL_COUNT],
        };
        let notes_focus = owner.present_focus(Surface::Notes, Some(&notes)).unwrap();
        assert!(owner.display_ack(&notes, &notes_focus));
        assert_eq!(owner.input_owner().get(), 2);
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(metadata.len(), (PIXEL_COUNT * 4) as u64);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn mismatched_notes_frame_does_not_move_focus() {
        let (file, path) = temporary_framebuffer();
        let display = FramebufferPresenter::new(
            file,
            ValidatedDisplayLayout {
                width: FRAME_WIDTH,
                height: FRAME_HEIGHT,
                stride_bytes: FRAME_WIDTH * 4,
                mapped_bytes: PIXEL_COUNT * 4,
                format: PixelFormat::Bgra8888,
            },
        )
        .unwrap();
        let (mut owner, _) = ShellSurfaceOwner::new(
            display,
            SurfaceId::new(11).unwrap(),
            SurfaceId::new(12).unwrap(),
            SurfaceId::new(13).unwrap(),
        )
        .unwrap();
        let wrong = SurfaceFrame {
            surface_id: SurfaceId::new(99).unwrap(),
            pixels: vec![0; PIXEL_COUNT],
        };
        assert_eq!(
            owner.present_focus(Surface::Notes, Some(&wrong)),
            Err(DisplayError::InvalidFrame)
        );
        assert_eq!(owner.focused(), Surface::Shell);
        assert_eq!(owner.input_owner().get(), 11);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }
}
