use crate::font;
use libnagi::{SURFACE_HEIGHT, SURFACE_WIDTH};

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

pub struct Painter<'a> {
    surface: &'a mut [u32],
}

impl<'a> Painter<'a> {
    pub fn new(surface: &'a mut [u32]) -> Self {
        Self { surface }
    }

    pub fn fill(&mut self, rect: Rect, color: u32) {
        let left = rect.x.max(0).min(SURFACE_WIDTH as i32);
        let top = rect.y.max(0).min(SURFACE_HEIGHT as i32);
        let right = (rect.x + rect.width).max(left).min(SURFACE_WIDTH as i32);
        let bottom = (rect.y + rect.height).max(top).min(SURFACE_HEIGHT as i32);
        let mut y = top;
        while y < bottom {
            let mut x = left;
            while x < right {
                let index = y as usize * SURFACE_WIDTH as usize + x as usize;
                unsafe {
                    core::ptr::write(self.surface.as_mut_ptr().add(index), color);
                }
                x += 1;
            }
            y += 1;
        }
    }

    pub fn frame(&mut self, rect: Rect, color: u32) {
        self.fill(Rect::new(rect.x, rect.y, rect.width, 1), color);
        self.fill(
            Rect::new(rect.x, rect.y + rect.height - 1, rect.width, 1),
            color,
        );
        self.fill(Rect::new(rect.x, rect.y, 1, rect.height), color);
        self.fill(
            Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height),
            color,
        );
    }

    pub fn text(&mut self, x: i32, y: i32, text: &[u8], color: u32) {
        let _ = font::draw_utf8(self.surface, x, y, text, color);
    }
}

pub const fn rgba(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16) | (0xff << 24)
}
