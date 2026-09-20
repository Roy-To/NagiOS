#![allow(dead_code)]

mod font {
    pub fn draw_utf8(_surface: &mut [u32], x: i32, _y: i32, text: &[u8], _color: u32) -> i32 {
        x + text.len() as i32 * 6
    }
}

mod ui {
    pub const fn rgba(red: u8, green: u8, blue: u8) -> u32 {
        red as u32 | ((green as u32) << 8) | ((blue as u32) << 16) | (0xff << 24)
    }
}

#[path = "../../nagi-init/src/boot.rs"]
mod boot;
