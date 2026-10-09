//! Scripted HOST orchestration tests. Nagi-target checks compile real services.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;
#[cfg(target_os = "nagi")]
#[path = "../../../user/nagi-init/src/model_service.rs"]
pub mod model_service;
#[path = "../../../user/nagi-init/src/session_services.rs"]
pub mod session_services;
#[path = "../../../user/nagi-init/src/session_ui.rs"]
pub mod session_ui;
#[cfg(test)]
mod tests;

#[path = "../../../user/nagi-init/src/bar_adapter.rs"]
pub mod bar_adapter;
#[path = "../../../user/nagi-init/src/bar_panel.rs"]
pub mod bar_panel;
#[cfg(feature = "desktop-check")]
#[path = "../../../user/nagi-init/src/desktop.rs"]
pub mod desktop;
#[path = "../../../user/nagi-init/src/font.rs"]
pub mod font;
#[cfg(feature = "desktop-check")]
#[path = "../../../user/nagi-init/src/login_screen.rs"]
pub mod login_screen;
#[cfg(feature = "desktop-check")]
#[path = "../../../user/nagi-init/src/m19_runtime.rs"]
pub mod m19_runtime;
#[cfg(feature = "desktop-check")]
#[path = "../../../user/nagi-init/src/m19_storage.rs"]
pub mod m19_storage;
#[path = "../../../user/nagi-init/src/ui.rs"]
pub mod ui;
