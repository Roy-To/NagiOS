//! HOST-ONLY real Desktop event/render/login regressions. No model or guest PASS.
extern crate alloc;
#[path = "../../../../user/nagi-init/src/desktop.rs"]
pub mod desktop;
#[path = "../../../../user/nagi-init/src/font.rs"]
pub mod font;
#[path = "../../../../user/nagi-init/src/login_screen.rs"]
pub mod login_screen;
#[cfg(all(feature = "m19-runtime", not(test)))]
#[path = "../../../../user/nagi-init/src/m19_runtime.rs"]
pub mod m19_runtime;
#[path = "../../../../user/nagi-init/src/ui.rs"]
pub mod ui;
#[cfg(all(feature = "m19-runtime", test))]
pub(crate) use desktop::host_m19_runtime as m19_runtime;
#[path = "../../../../user/nagi-init/src/m19_storage.rs"]
pub mod m19_storage;
#[path = "../../../../user/nagi-init/src/session_services.rs"]
pub mod session_services;
