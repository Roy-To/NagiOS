#![no_std]

use nagi_sdk::{AppSessionId, Application, PresentationContext, HELLO_APP_ID, HELLO_EXECUTABLE};

pub const APP: Application = Application::new(HELLO_APP_ID, AppSessionId(1));

pub fn package_manifest() -> &'static [u8] {
    nagi_sdk::HELLO_MANIFEST
}

pub fn package_executable() -> &'static [u8] {
    HELLO_EXECUTABLE
}

pub fn compact_surface() -> nagi_sdk::PresentationSurface {
    APP.presentation(PresentationContext::compact(320, 200))
}
