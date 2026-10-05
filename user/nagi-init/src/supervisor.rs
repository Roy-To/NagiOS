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
//!   OS-owned dialog answers it through `resolve_consent` (ADR 0053).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use libnagi::launch::{
    AppManifest, GrantCheck, GrantDecision, LaunchError, LaunchPlacement, LaunchRecord,
    LaunchRegistry,
};
#[cfg(feature = "consent-dialog-acceptance")]
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
#[cfg(feature = "consent-dialog-acceptance")]
pub fn request_consent(
    app_id: AppId,
    app_session_id: nagi_model::AppSessionId,
    capability: &[u8],
) -> Result<ConsentRequest, GrantCheck> {
    with_registry(|registry| registry.request_consent(app_id, app_session_id, capability))
        .unwrap_or(Err(GrantCheck::NotLive))
}

/// The oldest prompt waiting for the consent dialog.
#[cfg(feature = "consent-dialog-acceptance")]
pub fn next_consent_request() -> Option<ConsentRequest> {
    with_registry(|registry| registry.next_consent_request()).flatten()
}

/// Close a prompt with the user's answer from the OS-owned dialog. `None`
/// (dismissed) records nothing.
#[cfg(feature = "consent-dialog-acceptance")]
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
#[cfg(feature = "consent-dialog-acceptance")]
pub fn encode_decisions(output: &mut [u8; MAX_ENCODED_DECISIONS]) -> usize {
    with_registry(|registry| registry.encode_decisions(output)).unwrap_or(0)
}

/// Apply decisions persisted in the user's User Data.
#[cfg(feature = "consent-dialog-acceptance")]
pub fn restore_decisions(user: &Session, bytes: &[u8]) -> Result<usize, DecisionStoreError> {
    with_registry(|registry| registry.restore_decisions(user, bytes))
        .unwrap_or(Err(DecisionStoreError::Malformed))
}

/// The acceptance user's authenticated session. The desktop has no login
/// UI yet, so acceptance scenarios use this fixture account as the
/// signed-in user. Its decisions come either from explicit acceptance
/// inputs (`record_user_decision`) or, with ADR 0053, from real input on
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
    const DECISIONS: [(&[u8], &[u8]); 5] = [
        (b"org.nagi.acceptance.m19-search", b"search.query"),
        (b"org.nagi.acceptance.m19-search", b"files.search"),
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
