//! nagi-init's Supervisor launch registry (ADR 0046, ADR 0049).
//!
//! The registry is the only way for init to start an isolated application:
//!
//! - Every launch is a signed M16 `.xapp` package. The Supervisor verifies
//!   its Ed25519 signature against the pinned trust key, and takes the
//!   application identity and grants only from the signed manifest. It then
//!   checks the launch before spawning the package's ELF, and records
//!   `ProcessId -> LaunchRecord`.
//! - Services resolve kernel-stamped sender IDs through `resolve`.
//! - Services consult `has_grant` for capabilities. A grant exists only while
//!   the launched session is live, its signed manifest requests it, and an
//!   authenticated user allowed it (ADR 0051).
//! - `reap` waits for the kernel exit status and revokes the launch.
//! - A `ConsentRequired` use is queued with `request_consent`; the desktop's
//!   OS-owned dialog answers it through `resolve_consent` (ADR 0060).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use libnagi::launch::{
    AppManifest, GrantCheck, GrantDecision, LaunchError, LaunchPlacement, LaunchRecord,
    LaunchRegistry,
};
#[cfg(feature = "consent-dialog")]
use libnagi::launch::{ConsentRequest, DecisionStoreError, MAX_ENCODED_DECISIONS};
use libnagi::security::{AccountStore, Role, Session};
use libnagi::{
    channel_create_pair, channel_send, handle_close, process_spawn, process_wait,
    ChannelSendRequest, RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
use libnagi::{ProcessExitStatus, PROCESS_EXIT_KIND_EXITED};
use nagi_model::AppId;
use nagi_package::PackageView;

/// Embed a signed acceptance package built by `./nagi` into
/// `NAGI_ACCEPTANCE_PACKAGES` (see `user/nagi-init/manifests/README.md`).
#[macro_export]
macro_rules! acceptance_package {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("NAGI_ACCEPTANCE_PACKAGES"),
            "/",
            $name,
            ".xapp"
        ))
    };
}

#[cfg(feature = "isolated-process-acceptance")]
pub const ISOLATED_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.isolated-app");
#[cfg(feature = "isolated-process-acceptance")]
pub const FAULTING_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.faulting-app");
/// Declared application that may query Search but owns no fixture objects
/// and holds no file-action grants.
#[cfg(feature = "m19-search-ipc")]
pub const FOREIGN_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.foreign-client");

struct SupervisorState {
    locked: AtomicBool,
    registry: UnsafeCell<LaunchRegistry>,
}

// Access is serialized by `locked`.
unsafe impl Sync for SupervisorState {}

static SUPERVISOR: SupervisorState = SupervisorState {
    locked: AtomicBool::new(false),
    registry: UnsafeCell::new(LaunchRegistry::new()),
};

/// Run `operation` on the registry.
fn with_registry<R>(operation: impl FnOnce(&mut LaunchRegistry) -> R) -> Option<R> {
    while SUPERVISOR
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let registry = unsafe { &mut *SUPERVISOR.registry.get() };
    let result = Some(operation(registry));
    SUPERVISOR.locked.store(false, Ordering::Release);
    result
}

/// A launched isolated application and the Supervisor's endpoint to it.
pub struct Launched {
    pub endpoint: u64,
    pub record: LaunchRecord,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchFailure {
    Registry(LaunchError),
    ManifestsUnavailable,
    /// The bytes are not a well-formed `.xapp`.
    InvalidPackage,
    /// The package signature is missing or does not verify.
    UnsignedPackage,
    /// The signed package declares a different application than requested.
    WrongApplication,
    Channel,
    Spawn,
}

/// Launch the signed package `package` as application `app_id` at
/// `placement`. The order is:
///
/// 1. verify the signature;
/// 2. register or match the signed declaration (identity plus grants);
/// 3. check the launch;
/// 4. spawn the package's ELF.
///
/// The child receives one read/write/wait endpoint and no transfer right.
pub fn launch(
    package: &[u8],
    app_id: AppId,
    placement: LaunchPlacement,
) -> Result<Launched, LaunchFailure> {
    let view = PackageView::parse(package).map_err(|_| LaunchFailure::InvalidPackage)?;
    if !view.is_signed() {
        return Err(LaunchFailure::UnsignedPackage);
    }
    let signed = view.manifest();
    let manifest = AppManifest::from_declaration(signed.id(), signed.grants())
        .map_err(LaunchFailure::Registry)?;
    if manifest.app_id() != app_id || signed.app_id() != app_id {
        return Err(LaunchFailure::WrongApplication);
    }
    with_registry(|registry| registry.register_or_match(manifest))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)?;
    let elf = view.executable();
    with_registry(|registry| registry.check_launch(app_id, placement))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)?;
    let endpoints = channel_create_pair().ok_or(LaunchFailure::Channel)?;
    let Some(process_id) = process_spawn(
        elf,
        endpoints.endpoint_b,
        RIGHT_READ | RIGHT_WRITE | RIGHT_WAIT,
    ) else {
        let _ = handle_close(endpoints.endpoint_a);
        let _ = handle_close(endpoints.endpoint_b);
        return Err(LaunchFailure::Spawn);
    };
    match with_registry(|registry| registry.record_launch(process_id, app_id, placement)) {
        Some(Ok(record)) => Ok(Launched {
            endpoint: endpoints.endpoint_a,
            record,
        }),
        // The check above makes this unreachable; if it happens, close our
        // end so the unrecorded child sees its peer gone and exits, and its
        // messages are never attributed to anyone.
        Some(Err(error)) => {
            let _ = handle_close(endpoints.endpoint_a);
            Err(LaunchFailure::Registry(error))
        }
        None => {
            let _ = handle_close(endpoints.endpoint_a);
            Err(LaunchFailure::ManifestsUnavailable)
        }
    }
}

/// The launch record of a kernel-stamped sender, if it is a live launch.
pub fn resolve(process_id: u32) -> Option<LaunchRecord> {
    with_registry(|registry| registry.resolve(process_id)).flatten()
}

/// Received on an endpoint owned by this Supervisor launch. Construction is
/// private: caller identity is never accepted from a payload or supplied PID.
/// This proves only app sender binding, NOT an authenticated Desktop Session,
/// summary protocol, user intent, Browser sharing or model terms consent.
/// Borrowing the launch prevents the ordinary owner from reaping it while this
/// receipt is in use. Recheck live_launch before every authority-sensitive use.
pub struct ReceivedAppMessage<'a> {
    source: &'a Launched,
    envelope: libnagi::ChannelReceiveResult,
}

impl ReceivedAppMessage<'_> {
    pub fn live_launch(&self) -> Option<LaunchRecord> {
        resolve(self.source.record.process_id).filter(|record| *record == self.source.record)
    }

    /// Protocol/version/opcode/request/payload remain untrusted and must be
    /// validated by the consumer. They never select the authenticated actor.
    pub fn envelope(&self) -> &libnagi::ChannelReceiveResult {
        &self.envelope
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveFailure {
    LaunchExpired,
    EndpointUnavailable,
    SenderMismatch,
    Malformed,
    HandleCleanupFailed,
}

/// Bounded, nonblocking receive suitable for a future Desktop owner step.
/// The sender comes only from the real kernel receive result; registry state
/// is checked before and after receive. This installs no Bar endpoint or actor.
pub fn try_receive_from(
    source: &Launched,
) -> Result<Option<ReceivedAppMessage<'_>>, ReceiveFailure> {
    try_receive_from_with(source, libnagi::channel_try_receive, handle_close)
}

// Private injection seam for host boundary tests; callers outside this module
// cannot mint a receipt from freely constructed ChannelReceiveResult fields.
fn try_receive_from_with(
    source: &Launched,
    receive: impl FnOnce(u64, &mut libnagi::ChannelReceiveResult) -> Option<bool>,
    mut close: impl FnMut(u64) -> bool,
) -> Result<Option<ReceivedAppMessage<'_>>, ReceiveFailure> {
    if resolve(source.record.process_id) != Some(source.record) {
        return Err(ReceiveFailure::LaunchExpired);
    }
    let mut envelope = libnagi::ChannelReceiveResult::default();
    match receive(source.endpoint, &mut envelope) {
        Some(false) => return Ok(None),
        None => return Err(ReceiveFailure::EndpointUnavailable),
        Some(true) => {}
    }
    // This entry accepts inline data only. Reject and release every actually
    // received handle, including malformed counts, before discarding an envelope.
    if envelope.transfer_count != 0 {
        let mut cleaned = true;
        for handle in envelope
            .handles
            .iter()
            .take((envelope.transfer_count as usize).min(envelope.handles.len()))
            .copied()
            .filter(|handle| *handle != 0)
        {
            cleaned &= close(handle);
        }
        return Err(if cleaned {
            ReceiveFailure::Malformed
        } else {
            ReceiveFailure::HandleCleanupFailed
        });
    }
    if envelope.payload_len as usize > envelope.payload.len() {
        return Err(ReceiveFailure::Malformed);
    }
    if envelope.sender_process_id != source.record.process_id {
        return Err(ReceiveFailure::SenderMismatch);
    }
    if resolve(envelope.sender_process_id) != Some(source.record) {
        return Err(ReceiveFailure::LaunchExpired);
    }
    Ok(Some(ReceivedAppMessage { source, envelope }))
}

/// Whether the live application session may exercise `capability`.
pub fn has_grant(
    app_id: AppId,
    app_session_id: nagi_model::AppSessionId,
    capability: &[u8],
) -> bool {
    check_grant(app_id, app_session_id, capability) == GrantCheck::Granted
}

/// Why `capability` is or is not effective for the live session.
pub fn check_grant(
    app_id: AppId,
    app_session_id: nagi_model::AppSessionId,
    capability: &[u8],
) -> GrantCheck {
    with_registry(|registry| registry.check_grant(app_id, app_session_id, capability))
        .unwrap_or(GrantCheck::NotLive)
}

/// Record an authenticated user's decision for `app_id`'s use of
/// `capability`. This is the OS-owned consent path; launched processes have
/// no route to it.
pub fn record_user_decision(
    user: &Session,
    app_id: AppId,
    capability: &[u8],
    decision: GrantDecision,
) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.record_decision(user, app_id, capability, decision))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Queue a prompt for the OS-owned consent dialog when `capability` needs
/// the user's answer. Any other grant state is returned unchanged.
#[cfg(feature = "consent-dialog")]
pub fn request_consent(
    app_id: AppId,
    app_session_id: nagi_model::AppSessionId,
    capability: &[u8],
) -> Result<ConsentRequest, GrantCheck> {
    with_registry(|registry| registry.request_consent(app_id, app_session_id, capability))
        .unwrap_or(Err(GrantCheck::NotLive))
}

/// The oldest prompt waiting for the consent dialog.
#[cfg(feature = "consent-dialog")]
pub fn next_consent_request() -> Option<ConsentRequest> {
    with_registry(|registry| registry.next_consent_request()).flatten()
}

/// Close a prompt with the user's answer from the OS-owned dialog. `None`
/// (dismissed) records nothing.
#[cfg(feature = "consent-dialog")]
pub fn resolve_consent(
    user: &Session,
    request: &ConsentRequest,
    decision: Option<GrantDecision>,
) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.resolve_consent(user, request, decision))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Encode the decisions that persist across restarts.
#[cfg(feature = "consent-dialog")]
pub fn encode_decisions(output: &mut [u8; MAX_ENCODED_DECISIONS]) -> usize {
    with_registry(|registry| registry.encode_decisions(output)).unwrap_or(0)
}

/// The recorded decisions, for the settings view (ADR 0065).
#[cfg(feature = "consent-dialog")]
pub fn list_decisions(
    output: &mut [Option<libnagi::launch::DecisionView>; libnagi::launch::MAX_CONSENT_DECISIONS],
) -> usize {
    with_registry(|registry| registry.decisions(output)).unwrap_or(0)
}

/// Apply decisions persisted in the user's User Data.
#[cfg(feature = "consent-dialog")]
pub fn restore_decisions(user: &Session, bytes: &[u8]) -> Result<usize, DecisionStoreError> {
    with_registry(|registry| registry.restore_decisions(user, bytes))
        .unwrap_or(Err(DecisionStoreError::Malformed))
}

/// The acceptance user's authenticated session. The desktop has no login
/// UI yet, so acceptance scenarios use this fixture account as the
/// signed-in user. Its decisions come either from explicit acceptance
/// inputs (`record_user_decision`) or, with ADR 0060, from real input on
/// the OS-owned consent dialog.
pub fn acceptance_user() -> Option<Session> {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"user", Role::Standard, b"acceptance-user")
        .ok()?;
    accounts.authenticate(b"user", b"acceptance-user").ok()
}

/// The M19/M21/M22 acceptance user's decisions: allow each acceptance
/// application the capabilities its scenario exercises. Grants the signed
/// manifests do not request stay ineffective regardless.
#[cfg(feature = "m19-search-ipc")]
pub fn record_acceptance_consents() -> bool {
    const DECISIONS: [(&[u8], &[u8]); 6] = [
        (b"org.nagi.acceptance.m19-search", b"search.query"),
        (b"org.nagi.acceptance.m19-search", b"files.search"),
        (b"org.nagi.acceptance.m19-search", b"albert.history.search"),
        (b"org.nagi.acceptance.foreign-client", b"search.query"),
        (b"org.nagi.acceptance.m22-files", b"files.move"),
        (b"org.nagi.acceptance.m22-files", b"files.copy"),
    ];
    let Some(user) = acceptance_user() else {
        return false;
    };
    DECISIONS.iter().all(|(app, capability)| {
        record_user_decision(
            &user,
            AppId::from_identifier(app),
            capability,
            GrantDecision::Allow,
        )
        .is_ok()
    })
}

/// Check launch preconditions for `app_id` at `placement` without spawning.
#[cfg(feature = "isolated-process-acceptance")]
pub fn check(app_id: AppId, placement: LaunchPlacement) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.check_launch(app_id, placement))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Wait for the launched process to exit (`SYS_PROCESS_WAIT`, ADR 0048),
/// confirm the kernel closed its handles, then revoke its launch record and
/// release the endpoint. Returns the consumed exit status.
pub fn reap(launched: Launched) -> Option<ProcessExitStatus> {
    let process_id = launched.record.process_id;
    let status = process_wait(process_id)?;
    // After exit no process can reach the peer endpoint.
    let peer_closed = !channel_send(launched.endpoint, &ChannelSendRequest::new(0, 0, 0, 0));
    let revoked = with_registry(|registry| registry.record_exit(process_id))
        .flatten()
        .is_some_and(|record| record == launched.record);
    let closed = handle_close(launched.endpoint);
    (status.process_id == process_id
        && peer_closed
        && revoked
        && closed
        && resolve(process_id).is_none())
    .then_some(status)
}

/// Whether `status` is a clean `SYS_PROCESS_EXIT(0)`.
pub fn exited_cleanly(status: &ProcessExitStatus) -> bool {
    status.kind == PROCESS_EXIT_KIND_EXITED && status.code == 0
}

#[cfg(test)]
mod receive_tests {
    extern crate std;
    use super::*;
    use libnagi::ChannelReceiveResult;
    use nagi_model::{AppSessionId, NodeId};
    use std::sync::{Mutex, MutexGuard};
    static SERIAL: Mutex<()> = Mutex::new(());

    struct Fixture(MutexGuard<'static, ()>);
    impl Fixture {
        fn new() -> Self {
            let guard = SERIAL.lock().unwrap();
            with_registry(|registry| *registry = LaunchRegistry::new()).unwrap();
            Self(guard)
        }
        fn launched(&self) -> Launched {
            // Synthetic registry fixture, not a signed package/kernel launch.
            // Tests cover receive orchestration only, never OS provenance.
            let app = AppId::from_identifier(b"org.nagi.bar");
            let record = with_registry(|registry| {
                registry
                    .register_manifest(AppManifest::from_declaration(b"org.nagi.bar", []).unwrap())
                    .unwrap();
                registry
                    .record_launch(
                        7,
                        app,
                        LaunchPlacement {
                            app_session_id: AppSessionId(8),
                            node_id: NodeId(9),
                            workspace_id: None,
                        },
                    )
                    .unwrap()
            })
            .unwrap();
            Launched {
                endpoint: 42,
                record,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            with_registry(|registry| *registry = LaunchRegistry::new()).unwrap();
            let _ = &self.0;
        }
    }
    fn envelope(sender: u32) -> ChannelReceiveResult {
        let mut message = ChannelReceiveResult {
            sender_process_id: sender,
            request_id: 5,
            ..ChannelReceiveResult::default()
        };
        let forged = b"caller=org.example.admin;pid=1";
        message.payload[..forged.len()].copy_from_slice(forged);
        message.payload_len = forged.len() as u32;
        message
    }
    fn receive(
        source: &Launched,
        message: ChannelReceiveResult,
    ) -> Result<Option<ReceivedAppMessage<'_>>, ReceiveFailure> {
        try_receive_from_with(
            source,
            |endpoint, result| {
                assert_eq!(endpoint, 42);
                *result = message;
                Some(true)
            },
            |_| panic!("inline fixture must not close any handle"),
        )
    }
    #[test]
    fn kernel_sender_binding_ignores_payload_identity_and_rechecks_revocation() {
        let fixture = Fixture::new();
        let source = fixture.launched();
        let receipt = receive(&source, envelope(7)).unwrap().unwrap();
        assert_eq!(receipt.live_launch(), Some(source.record));
        assert_eq!(receipt.envelope().request_id, 5);
        assert!(receipt
            .envelope()
            .payload
            .starts_with(b"caller=org.example.admin"));
        with_registry(|registry| registry.record_exit(7))
            .unwrap()
            .unwrap();
        assert_eq!(receipt.live_launch(), None);
    }
    #[test]
    fn forged_sender_cannot_use_expected_endpoint_launch() {
        let fixture = Fixture::new();
        let source = fixture.launched();
        for sender in [0, 1, 8] {
            assert!(matches!(
                receive(&source, envelope(sender)),
                Err(ReceiveFailure::SenderMismatch)
            ));
        }
    }
    #[test]
    fn stale_or_changed_launch_is_denied_before_receiving() {
        let fixture = Fixture::new();
        let mut source = fixture.launched();
        source.record.app_session_id = AppSessionId(99);
        assert!(matches!(
            try_receive_from_with(
                &source,
                |_, _| panic!("stale binding read endpoint"),
                |_| true
            ),
            Err(ReceiveFailure::LaunchExpired)
        ));
    }
    #[test]
    fn launch_revoked_during_receive_cannot_mint_receipt() {
        let fixture = Fixture::new();
        let source = fixture.launched();
        let result = try_receive_from_with(
            &source,
            |_, message| {
                *message = envelope(7);
                with_registry(|registry| registry.record_exit(7))
                    .unwrap()
                    .unwrap();
                Some(true)
            },
            |_| true,
        );
        assert!(matches!(result, Err(ReceiveFailure::LaunchExpired)));
    }
    #[test]
    fn empty_and_failed_endpoint_are_bounded_without_retry() {
        let fixture = Fixture::new();
        let source = fixture.launched();
        assert!(matches!(
            try_receive_from_with(&source, |_, _| Some(false), |_| true),
            Ok(None)
        ));
        assert!(matches!(
            try_receive_from_with(&source, |_, _| None, |_| true),
            Err(ReceiveFailure::EndpointUnavailable)
        ));
    }
    #[test]
    fn malformed_inline_and_transferred_handles_fail_closed_and_release_handles() {
        let fixture = Fixture::new();
        let source = fixture.launched();
        let mut message = envelope(7);
        message.payload_len = u32::MAX;
        assert!(matches!(
            receive(&source, message),
            Err(ReceiveFailure::Malformed)
        ));
        for cleanup_ok in [true, false] {
            let mut handles = std::vec::Vec::new();
            let result = try_receive_from_with(
                &source,
                |_, message| {
                    *message = envelope(7);
                    message.transfer_count = u32::MAX;
                    message.handles = [51, 52, 53, 54];
                    Some(true)
                },
                |handle| {
                    handles.push(handle);
                    cleanup_ok
                },
            );
            assert!(
                matches!(result, Err(ReceiveFailure::Malformed)) && cleanup_ok
                    || matches!(result, Err(ReceiveFailure::HandleCleanupFailed)) && !cleanup_ok
            );
            assert_eq!(handles, [51, 52, 53, 54]);
        }
    }
}
