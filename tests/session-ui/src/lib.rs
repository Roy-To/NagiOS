//! Host orchestration regressions for the production session UI and adapter.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[path = "../../../user/nagi-init/src/bar_adapter.rs"]
pub mod bar_adapter;
#[path = "../../../user/nagi-init/src/bar_panel.rs"]
pub mod bar_panel;
#[path = "../../../user/nagi-init/src/font.rs"]
pub mod font;
#[cfg(target_os = "nagi")]
#[path = "../../../user/nagi-init/src/model_service.rs"]
pub mod model_service;
#[path = "session_services.rs"]
pub mod session_services;
#[path = "../../../user/nagi-init/src/session_ui.rs"]
pub mod session_ui;
#[cfg(test)]
mod tests;
#[path = "../../../user/nagi-init/src/ui.rs"]
pub mod ui;

#[cfg(test)]
mod summary_tests;

#[cfg(test)]
mod font_tests;
