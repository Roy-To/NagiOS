//! Remote-page runtime entry points for Albert's M18-A networking workstream.

/// Controlled HTTP fixture reached through the guest VirtIO network device.
pub const CONTROLLED_FIXTURE_URL: &str = "http://10.0.2.2:18081/redirect";

/// The fixture title proves the final redirected response reached Servo.
pub const CONTROLLED_FIXTURE_TITLE: &str = "Nagi M18A Controlled Remote Fixture";

/// Load the controlled remote page through Servo and report its frame to Nagi Surface.
#[cfg(target_os = "nagi")]
pub fn run_remote_web_page(display_capability: u64) -> ! {
    super::guest::run_web_page(display_capability, CONTROLLED_FIXTURE_URL, true)
}
