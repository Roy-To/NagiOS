//! Dependency-free host raster probe of the exact production font source.
//! Geometry constants only; no guest, service, inference or platform rendering.
//! Run on an installed x86-64 Rust host without Cargo/dependency acquisition:
//! rustc --edition 2021 --test tests/session-ui/font-host.rs -o /tmp/nagi-font-tests
//! /tmp/nagi-font-tests
#![cfg_attr(not(test), no_std)]
extern crate self as libnagi;
pub const SURFACE_WIDTH: u32 = 320;
pub const SURFACE_HEIGHT: u32 = 200;
#[path = "../../user/nagi-init/src/font.rs"]
pub mod font;
#[cfg(test)]
#[path = "src/font_tests.rs"]
mod font_tests;
