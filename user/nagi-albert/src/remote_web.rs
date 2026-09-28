//! Remote-page runtime entry points for Albert's M18-A networking workstream.

/// Controlled HTTP fixture reached through the guest VirtIO network device.
pub const CONTROLLED_FIXTURE_URL: &str = "http://10.0.2.2:18081/redirect";

/// The fixture title proves the final redirected response reached Servo.
pub const CONTROLLED_FIXTURE_TITLE: &str = "Nagi M18A Controlled Remote Fixture";
/// The final title proves the browser completed HTTPS download and upload requests.
pub const CONTROLLED_FIXTURE_TRANSFER_TITLE: &str =
    "Nagi M18A Controlled Remote Fixture Transfers PASS";
/// A failed browser transfer has a distinct title so acceptance fails closed.
pub const CONTROLLED_FIXTURE_TRANSFER_FAILURE_TITLE: &str =
    "Nagi M18A Controlled Remote Fixture Transfers FAIL";

/// Typed handoff from browser state to Albert's remote navigation runtime.
///
/// M18-B can pass the current address as `RemoteNavigationRequest::new(url)`;
/// this request does not grant certificate or network authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteNavigationRequest<'a> {
    url: &'a str,
}

impl<'a> RemoteNavigationRequest<'a> {
    pub const fn new(url: &'a str) -> Self {
        Self { url }
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteNavigationRequest;

    #[test]
    fn navigation_request_keeps_the_caller_url_without_granting_trust_overrides() {
        let request = RemoteNavigationRequest::new("https://example.invalid/page");
        assert_eq!(request.url, "https://example.invalid/page");
    }
}

#[cfg(target_os = "nagi")]
pub(super) const TEST_ROOT_PATH: &str = "/tmp/nagi-m18a-test-ca.pem";

/// Add the fixture's test CA to Servo's normal WebPKI roots through the guest VFS.
#[cfg(target_os = "nagi")]
pub(super) fn install_test_ca() -> bool {
    use core::ffi::c_char;

    const O_CREAT_TRUNC: i32 = 0x0200_0000 | 0x0400_0000;
    const PATH: &[u8] = b"/tmp/nagi-m18a-test-ca.pem\0";
    const ROOT_CERTIFICATE: &[u8] = include_bytes!("../../../tests/fixtures/m18a/tls/root.pem");

    if ROOT_CERTIFICATE.len() > 1024 {
        return false;
    }
    // SAFETY: PATH is a NUL-terminated static path and the guest POSIX adapter
    // consumes it synchronously.
    let descriptor = unsafe {
        nagi_posix::nagi_posix_open(PATH.as_ptr().cast::<c_char>(), O_CREAT_TRUNC, 0o600)
    };
    if descriptor < 0 {
        return false;
    }
    // SAFETY: ROOT_CERTIFICATE is a valid static byte slice for this call.
    let written = unsafe {
        nagi_posix::nagi_posix_write_fd(
            descriptor,
            ROOT_CERTIFICATE.as_ptr(),
            ROOT_CERTIFICATE.len(),
        )
    };
    // SAFETY: descriptor was returned by the guest POSIX open adapter above.
    let closed = unsafe { nagi_posix::nagi_posix_close(descriptor) };
    written == ROOT_CERTIFICATE.len() as isize && closed == 0
}

/// Load a remote HTTP or HTTPS URL through Servo using Nagi's normal trust roots.
#[cfg(target_os = "nagi")]
pub fn run_remote_web_page(display_capability: u64, request: RemoteNavigationRequest<'_>) -> ! {
    super::guest::run_web_page(display_capability, request.url, true, false)
}

/// Run the deterministic HTTPS redirect/transfer fixture used by M18-A acceptance.
#[cfg(target_os = "nagi")]
pub fn run_controlled_fixture(display_capability: u64) -> ! {
    super::guest::run_web_page(display_capability, CONTROLLED_FIXTURE_URL, true, true)
}
