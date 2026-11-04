//! HOST-ONLY real Desktop, consent UI, SessionServices and renderer regressions.
// The imported Desktop source has a larger production feature matrix than this host harness.
#![allow(unexpected_cfgs)]
extern crate alloc;
#[cfg(feature = "consent-dialog")]
#[path = "../../../../user/nagi-init/src/consent_dialog.rs"]
pub mod consent_dialog;
#[cfg(feature = "consent-dialog")]
#[path = "../../../../user/nagi-init/src/consent_settings.rs"]
pub mod consent_settings;
#[path = "../../../../user/nagi-init/src/desktop.rs"]
pub mod desktop;
#[path = "../../../../user/nagi-init/src/font.rs"]
pub mod font;
#[path = "../../../../user/nagi-init/src/login_screen.rs"]
pub mod login_screen;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login", not(test)))]
#[path = "../../../../user/nagi-init/src/m19_runtime.rs"]
mod m19_runtime;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login", test))]
pub(crate) use desktop::host_m19_runtime as m19_runtime;
#[cfg(all(feature = "m19-runtime", feature = "desktop-login"))]
#[path = "../../../../user/nagi-init/src/m19_storage.rs"]
mod m19_storage;
#[cfg(feature = "m20-model-service")]
#[path = "session_services.rs"]
pub mod session_services;
#[cfg(feature = "consent-dialog")]
#[path = "../../../../user/nagi-init/src/supervisor.rs"]
pub mod supervisor;
#[path = "../../../../user/nagi-init/src/ui.rs"]
pub mod ui;

#[cfg(all(test, feature = "consent-dialog"))]
pub(crate) mod files_consent_wait {
    // Retained to compile the historical pause-only loop for red/green checks.
    use crate::desktop::{Desktop, UserDataVolume as PromptVolume};
    #[allow(unused_imports)]
    use core::arch::asm;
    use libnagi::launch::GrantCheck;
    use libnagi::{display_present, input_read, InputEvent};
    include!(concat!(env!("OUT_DIR"), "/files_consent_wait.rs"));
    pub(crate) fn run(
        desktop: &mut Desktop,
        surface: &mut [u32],
        volume: &mut PromptVolume,
        user: &libnagi::security::Session,
    ) -> Option<(libnagi::launch::ConsentRequest, GrantCheck)> {
        wait_for_answer(desktop, 1, 2, surface, volume, user)
    }
}
