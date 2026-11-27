// Standalone host harness for the exact nagi-init native font module.
// Run with the host-native Rust toolchain:
// rustc --edition=2021 --test tools/fonts/native_bitmap_font_host_test.rs -o <temp> && <temp>
#![allow(dead_code)]

extern crate self as libnagi;

pub const SURFACE_WIDTH: u32 = 320;
pub const SURFACE_HEIGHT: u32 = 200;

#[path = "../../user/nagi-init/src/font.rs"]
mod font;
