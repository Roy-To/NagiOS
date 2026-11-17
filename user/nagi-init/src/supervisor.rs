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
#[cfg(target_os = "nagi")]
use libnagi::{
    channel_create_pair, channel_send, handle_close, process_spawn, process_wait,
    ChannelSendRequest, RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
#[cfg(target_os = "nagi")]
use libnagi::{ProcessExitStatus, PROCESS_EXIT_KIND_EXITED};
use nagi_model::AppId;
#[cfg(target_os = "nagi")]
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

struct RegistryUnlockGuard;

impl Drop for RegistryUnlockGuard {
    fn drop(&mut self) {
        SUPERVISOR.locked.store(false, Ordering::Release);
    }
}

/// Run `operation` on the registry.
fn with_registry<R>(operation: impl FnOnce(&mut LaunchRegistry) -> R) -> Option<R> {
    while SUPERVISOR
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let _unlock = RegistryUnlockGuard;
    let registry = unsafe { &mut *SUPERVISOR.registry.get() };
    Some(operation(registry))
}

/// Match a declaration in the Supervisor-owned registry.
///
/// The caller remains responsible for authenticating the declaration's source;
/// this function only performs the existing registry operation.
pub(crate) fn register_or_match_manifest(manifest: AppManifest) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.register_or_match(manifest))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)?;
    Ok(())
}

/// Check launch conditions against the Supervisor-owned registry.
pub(crate) fn check_launch(app_id: AppId, placement: LaunchPlacement) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.check_launch(app_id, placement))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Bind a process ID to an application already declared in the shared registry.
pub(crate) fn record_launch(
    process_id: u32,
    app_id: AppId,
    placement: LaunchPlacement,
) -> Result<LaunchRecord, LaunchFailure> {
    with_registry(|registry| registry.record_launch(process_id, app_id, placement))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Remove a launch only if the current registry entry still matches its identity.
fn record_exit_if_matches_in_registry(
    registry: &mut LaunchRegistry,
    process_id: u32,
    expected: &LaunchRecord,
) -> bool {
    process_id == expected.process_id
        && registry.resolve(process_id) == Some(*expected)
        && registry.record_exit(process_id) == Some(*expected)
}

/// Atomically compare and revoke an exited launch in the Supervisor registry.
/// A mismatch or an already-removed record leaves registry state untouched.
pub(crate) fn record_exit_if_matches(process_id: u32, expected: &LaunchRecord) -> bool {
    with_registry(|registry| record_exit_if_matches_in_registry(registry, process_id, expected))
        .unwrap_or(false)
}

/// A launched isolated application and the Supervisor's endpoint to it.
#[cfg(target_os = "nagi")]
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
#[cfg(target_os = "nagi")]
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
    register_or_match_manifest(manifest)?;
    let elf = view.executable();
    check_launch(app_id, placement)?;
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
    match record_launch(process_id, app_id, placement) {
        Ok(record) => Ok(Launched {
            endpoint: endpoints.endpoint_a,
            record,
        }),
        // The check above makes this unreachable; if it happens, close our
        // end so the unrecorded child sees its peer gone and exits, and its
        // messages are never attributed to anyone.
        Err(error) => {
            let _ = handle_close(endpoints.endpoint_a);
            Err(error)
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

/// Failure while validating a live launch and committing its nonblocking exec
/// gate under the same Supervisor registry lock.
pub(crate) enum LaunchCommitError<E> {
    RegistryUnavailable,
    RecordMismatch,
    Grant(GrantCheck),
    Commit(E),
}

fn authorize_launch_and_commit_in_registry<E>(
    registry: &mut LaunchRegistry,
    expected: &LaunchRecord,
    required_grants: &[&[u8]],
    commit_gate: impl FnOnce() -> Result<(), E>,
) -> Result<(), LaunchCommitError<E>> {
    if registry.resolve(expected.process_id) != Some(*expected) {
        return Err(LaunchCommitError::RecordMismatch);
    }
    for capability in required_grants {
        let check = registry.check_grant(expected.app_id, expected.app_session_id, capability);
        if check != GrantCheck::Granted {
            return Err(LaunchCommitError::Grant(check));
        }
    }
    commit_gate().map_err(LaunchCommitError::Commit)
}

/// Check the exact live launch and its required grants, then run one short,
/// nonblocking gate-commit callback while consent/revoke updates are excluded.
/// The callback must not wait, re-enter Supervisor, or call another registry.
pub(crate) fn authorize_launch_and_commit<E>(
    expected: &LaunchRecord,
    required_grants: &[&[u8]],
    commit_gate: impl FnOnce() -> Result<(), E>,
) -> Result<(), LaunchCommitError<E>> {
    with_registry(|registry| {
        authorize_launch_and_commit_in_registry(registry, expected, required_grants, commit_gate)
    })
    .unwrap_or(Err(LaunchCommitError::RegistryUnavailable))
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
    check_launch(app_id, placement)
}

/// Wait for the launched process to exit (`SYS_PROCESS_WAIT`, ADR 0048),
/// confirm the kernel closed its handles, then revoke its launch record and
/// release the endpoint. Returns the consumed exit status.
#[cfg(target_os = "nagi")]
pub fn reap(launched: Launched) -> Option<ProcessExitStatus> {
    let process_id = launched.record.process_id;
    let status = process_wait(process_id)?;
    // After exit no process can reach the peer endpoint.
    let peer_closed = !channel_send(launched.endpoint, &ChannelSendRequest::new(0, 0, 0, 0));
    let revoked = record_exit_if_matches(process_id, &launched.record);
    let closed = handle_close(launched.endpoint);
    (status.process_id == process_id
        && peer_closed
        && revoked
        && closed
        && resolve(process_id).is_none())
    .then_some(status)
}

/// Whether `status` is a clean `SYS_PROCESS_EXIT(0)`.
#[cfg(target_os = "nagi")]
pub fn exited_cleanly(status: &ProcessExitStatus) -> bool {
    status.kind == PROCESS_EXIT_KIND_EXITED && status.code == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::launch::GrantCheck;
    use nagi_model::{AppSessionId, NodeId, WorkspaceId};

    const PLACEMENT: LaunchPlacement = LaunchPlacement {
        app_session_id: AppSessionId(0x51),
        node_id: NodeId(0x52),
        workspace_id: Some(WorkspaceId(0x53)),
    };

    fn authenticated_user() -> Session {
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"user", Role::Standard, b"user-pass")
            .expect("account");
        accounts.authenticate(b"user", b"user-pass").expect("login")
    }

    fn launch_with_once_grant(
        registry: &mut LaunchRegistry,
        identifier: &[u8],
        process_id: u32,
        placement: LaunchPlacement,
    ) -> (AppId, LaunchRecord) {
        let manifest =
            AppManifest::from_declaration(identifier, [&b"files.search"[..]]).expect("manifest");
        let app_id = registry.register_or_match(manifest).expect("register");
        let record = registry
            .record_launch(process_id, app_id, placement)
            .expect("launch");
        registry
            .record_decision(
                &authenticated_user(),
                app_id,
                b"files.search",
                GrantDecision::AllowOnce(placement.app_session_id),
            )
            .expect("consent");
        (app_id, record)
    }

    fn assert_live_session_has_search_consent(
        registry: &LaunchRegistry,
        app_id: AppId,
        session: AppSessionId,
    ) {
        assert_eq!(
            registry.check_grant(app_id, session, b"files.search"),
            GrantCheck::Granted
        );
    }

    #[test]
    fn checked_exit_revokes_only_the_matching_record() {
        let mut registry = LaunchRegistry::new();
        let (app_id, record) =
            launch_with_once_grant(&mut registry, b"org.nagi.checked-exit", 41, PLACEMENT);
        assert_live_session_has_search_consent(&registry, app_id, PLACEMENT.app_session_id);

        assert!(record_exit_if_matches_in_registry(
            &mut registry,
            record.process_id,
            &record
        ));
        assert_eq!(registry.resolve(record.process_id), None);
        assert_eq!(
            registry.check_grant(app_id, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::NotLive
        );
        assert!(!record_exit_if_matches_in_registry(
            &mut registry,
            record.process_id,
            &record
        ));
    }

    #[test]
    fn mismatched_exit_keeps_live_record_and_once_consent() {
        let mut registry = LaunchRegistry::new();
        let (app_id, record) =
            launch_with_once_grant(&mut registry, b"org.nagi.checked-mismatch", 42, PLACEMENT);
        let stale = LaunchRecord {
            node_id: NodeId(0x99),
            ..record
        };

        assert!(!record_exit_if_matches_in_registry(
            &mut registry,
            record.process_id,
            &stale
        ));
        assert!(!record_exit_if_matches_in_registry(
            &mut registry,
            record.process_id + 1,
            &record
        ));
        assert_eq!(registry.resolve(record.process_id), Some(record));
        assert_live_session_has_search_consent(&registry, app_id, PLACEMENT.app_session_id);
    }

    #[test]
    fn allow_once_consent_is_scoped_to_one_live_session_and_revoked_on_exit() {
        let mut registry = LaunchRegistry::new();
        let (app_id, record) =
            launch_with_once_grant(&mut registry, b"org.nagi.once-session", 44, PLACEMENT);
        assert_live_session_has_search_consent(&registry, app_id, PLACEMENT.app_session_id);

        let other_placement = LaunchPlacement {
            app_session_id: AppSessionId(0x54),
            ..PLACEMENT
        };
        let other_record = registry
            .record_launch(45, app_id, other_placement)
            .expect("a distinct app session may launch");
        assert_eq!(
            registry.check_grant(app_id, other_placement.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );

        assert!(record_exit_if_matches_in_registry(
            &mut registry,
            record.process_id,
            &record
        ));
        assert_eq!(
            registry.check_grant(app_id, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::NotLive
        );

        registry
            .record_launch(record.process_id, app_id, PLACEMENT)
            .expect("the exited app session can be launched again");
        assert_eq!(
            registry.check_grant(app_id, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
        assert_eq!(
            registry.resolve(other_record.process_id),
            Some(other_record)
        );
    }

    #[test]
    fn reused_pid_with_a_different_record_is_not_revoked() {
        let mut registry = LaunchRegistry::new();
        let (_, old_record) =
            launch_with_once_grant(&mut registry, b"org.nagi.checked-old", 43, PLACEMENT);
        assert_eq!(
            registry.record_exit(old_record.process_id),
            Some(old_record)
        );

        let new_placement = LaunchPlacement {
            app_session_id: AppSessionId(0x61),
            ..PLACEMENT
        };
        let (new_app, new_record) = launch_with_once_grant(
            &mut registry,
            b"org.nagi.checked-new",
            old_record.process_id,
            new_placement,
        );
        assert_ne!(old_record, new_record);

        assert!(!record_exit_if_matches_in_registry(
            &mut registry,
            old_record.process_id,
            &old_record
        ));
        assert_eq!(registry.resolve(new_record.process_id), Some(new_record));
        assert_live_session_has_search_consent(&registry, new_app, new_placement.app_session_id);
    }

    #[test]
    fn launch_commit_calls_gate_once_without_consuming_session_allow_once() {
        let mut registry = LaunchRegistry::new();
        let (app_id, record) =
            launch_with_once_grant(&mut registry, b"org.nagi.commit-once", 51, PLACEMENT);
        let mut commits = 0;

        let result = authorize_launch_and_commit_in_registry(
            &mut registry,
            &record,
            &[b"files.search"],
            || {
                commits += 1;
                Ok::<(), ()>(())
            },
        );

        assert!(matches!(result, Ok(())));
        assert_eq!(commits, 1);
        assert_live_session_has_search_consent(&registry, app_id, PLACEMENT.app_session_id);
        registry
            .record_decision(
                &authenticated_user(),
                app_id,
                b"files.search",
                GrantDecision::Ask,
            )
            .expect("withdraw consent");
        assert_eq!(
            registry.check_grant(app_id, PLACEMENT.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
    }

    #[test]
    fn ask_or_deny_before_launch_commit_never_calls_gate_or_changes_record() {
        fn verify(
            decision: GrantDecision,
            expected: GrantCheck,
            identifier: &[u8],
            process_id: u32,
        ) {
            let mut registry = LaunchRegistry::new();
            let (app_id, record) =
                launch_with_once_grant(&mut registry, identifier, process_id, PLACEMENT);
            registry
                .record_decision(&authenticated_user(), app_id, b"files.search", decision)
                .expect("update decision");
            let mut commits = 0;

            let result = authorize_launch_and_commit_in_registry(
                &mut registry,
                &record,
                &[b"files.search"],
                || {
                    commits += 1;
                    Ok::<(), ()>(())
                },
            );

            assert!(matches!(result, Err(LaunchCommitError::Grant(check)) if check == expected));
            assert_eq!(commits, 0);
            assert_eq!(registry.resolve(process_id), Some(record));
        }

        verify(
            GrantDecision::Ask,
            GrantCheck::ConsentRequired,
            b"org.nagi.commit-ask",
            52,
        );
        verify(
            GrantDecision::Deny,
            GrantCheck::Denied,
            b"org.nagi.commit-deny",
            53,
        );
    }

    #[test]
    fn mismatched_launch_commit_never_calls_gate_or_revokes_current_record() {
        let mut registry = LaunchRegistry::new();
        let (app_id, record) =
            launch_with_once_grant(&mut registry, b"org.nagi.commit-mismatch", 54, PLACEMENT);
        let stale = LaunchRecord {
            node_id: NodeId(0x99),
            ..record
        };
        let mut commits = 0;

        let result = authorize_launch_and_commit_in_registry(
            &mut registry,
            &stale,
            &[b"files.search"],
            || {
                commits += 1;
                Ok::<(), ()>(())
            },
        );

        assert!(matches!(result, Err(LaunchCommitError::RecordMismatch)));
        assert_eq!(commits, 0);
        assert_eq!(registry.resolve(record.process_id), Some(record));
        assert_live_session_has_search_consent(&registry, app_id, PLACEMENT.app_session_id);
    }

    #[test]
    fn failed_gate_commit_leaves_the_launch_registry_unchanged() {
        let mut registry = LaunchRegistry::new();
        let (_, expected) =
            launch_with_once_grant(&mut registry, b"org.nagi.commit-gate-fail", 58, PLACEMENT);
        let (_, unrelated) =
            launch_with_once_grant(&mut registry, b"org.nagi.commit-other", 59, PLACEMENT);

        let result = authorize_launch_and_commit_in_registry(
            &mut registry,
            &expected,
            &[b"files.search"],
            || Err::<(), _>(()),
        );

        assert!(matches!(result, Err(LaunchCommitError::Commit(()))));
        assert_eq!(registry.resolve(expected.process_id), Some(expected));
        assert_eq!(registry.resolve(unrelated.process_id), Some(unrelated));
    }

    #[test]
    fn shared_registry_commit_releases_lock_before_later_revoke() {
        let manifest =
            AppManifest::from_declaration(b"org.nagi.commit-global", [&b"files.search"[..]])
                .expect("manifest");
        let placement = LaunchPlacement {
            app_session_id: AppSessionId(0x70),
            node_id: NodeId(0x71),
            workspace_id: None,
        };
        register_or_match_manifest(manifest).expect("register manifest");
        let record = record_launch(55, manifest.app_id(), placement).expect("record launch");
        let user = authenticated_user();
        record_user_decision(
            &user,
            manifest.app_id(),
            b"files.search",
            GrantDecision::AllowOnce(placement.app_session_id),
        )
        .expect("allow session");
        let mut commits = 0;

        let result = authorize_launch_and_commit(&record, &[b"files.search"], || {
            commits += 1;
            Ok::<(), ()>(())
        });

        assert!(matches!(result, Ok(())));
        assert_eq!(commits, 1);
        assert_eq!(
            check_grant(manifest.app_id(), placement.app_session_id, b"files.search"),
            GrantCheck::Granted
        );
        record_user_decision(
            &user,
            manifest.app_id(),
            b"files.search",
            GrantDecision::Ask,
        )
        .expect("withdraw after commit");
        assert_eq!(
            check_grant(manifest.app_id(), placement.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
        assert!(record_exit_if_matches(record.process_id, &record));
    }

    #[test]
    fn shared_registry_revoke_and_gate_commit_linearize_across_threads() {
        use std::sync::{mpsc, Arc, Barrier};
        use std::thread;

        fn register_allowed_launch(
            identifier: &[u8],
            process_id: u32,
            placement: LaunchPlacement,
        ) -> (AppId, LaunchRecord) {
            let manifest = AppManifest::from_declaration(identifier, [&b"files.search"[..]])
                .expect("manifest");
            let app_id = manifest.app_id();
            register_or_match_manifest(manifest).expect("register manifest");
            let record = record_launch(process_id, app_id, placement).expect("record launch");
            record_user_decision(
                &authenticated_user(),
                app_id,
                b"files.search",
                GrantDecision::AllowOnce(placement.app_session_id),
            )
            .expect("allow session");
            (app_id, record)
        }

        // Revoke first: the gate callback must not run after the grant changes
        // to Ask, even though the check and commit use the shared registry.
        let revoke_first_placement = LaunchPlacement {
            app_session_id: AppSessionId(0xA201),
            node_id: NodeId(0xA202),
            workspace_id: Some(WorkspaceId(0xA203)),
        };
        let (revoke_first_app, revoke_first_record) = register_allowed_launch(
            b"org.nagi.commit-race-revoke-first",
            0xA204_0001,
            revoke_first_placement,
        );
        let (revoke_done_tx, revoke_done_rx) = mpsc::channel();
        let revoke_first_revoker = thread::spawn(move || {
            record_user_decision(
                &authenticated_user(),
                revoke_first_app,
                b"files.search",
                GrantDecision::Ask,
            )
            .expect("revoke before gate");
            revoke_done_tx.send(()).expect("signal revoke completion");
        });
        let revoke_first_committer = thread::spawn(move || {
            revoke_done_rx.recv().expect("revoke completed first");
            let mut commits = 0;
            let result =
                authorize_launch_and_commit(&revoke_first_record, &[b"files.search"], || {
                    commits += 1;
                    Ok::<(), ()>(())
                });
            (
                matches!(
                    result,
                    Err(LaunchCommitError::Grant(GrantCheck::ConsentRequired))
                ),
                commits,
            )
        });
        revoke_first_revoker.join().expect("revoker thread");
        let (denied, commits) = revoke_first_committer.join().expect("committer thread");
        assert!(denied);
        assert_eq!(commits, 0);
        assert_eq!(
            check_grant(
                revoke_first_app,
                revoke_first_placement.app_session_id,
                b"files.search"
            ),
            GrantCheck::ConsentRequired
        );
        assert!(record_exit_if_matches(
            revoke_first_record.process_id,
            &revoke_first_record
        ));

        // Gate first: the callback signals the waiting revoker but does not
        // wait for it. The revoker then uses the same production registry path.
        let commit_first_placement = LaunchPlacement {
            app_session_id: AppSessionId(0xA211),
            node_id: NodeId(0xA212),
            workspace_id: Some(WorkspaceId(0xA213)),
        };
        let (commit_first_app, commit_first_record) = register_allowed_launch(
            b"org.nagi.commit-race-gate-first",
            0xA214_0001,
            commit_first_placement,
        );
        let (gate_committed_tx, gate_committed_rx) = mpsc::channel();
        let commit_first_revoker = thread::spawn(move || {
            gate_committed_rx
                .recv()
                .expect("gate callback ran before revoke");
            record_user_decision(
                &authenticated_user(),
                commit_first_app,
                b"files.search",
                GrantDecision::Ask,
            )
            .expect("revoke after gate");
        });
        let commit_first_committer = thread::spawn(move || {
            let mut commits = 0;
            let result =
                authorize_launch_and_commit(&commit_first_record, &[b"files.search"], || {
                    commits += 1;
                    assert!(super::SUPERVISOR.locked.load(super::Ordering::Acquire));
                    // An unbounded-channel send does not wait for the revoker;
                    // the callback remains short and does not re-enter Supervisor.
                    gate_committed_tx.send(()).expect("signal gate commit");
                    Ok::<(), ()>(())
                });
            (matches!(result, Ok(())), commits)
        });
        let (committed, commits) = commit_first_committer.join().expect("committer thread");
        commit_first_revoker.join().expect("revoker thread");
        assert!(committed);
        assert_eq!(commits, 1);
        assert_eq!(
            check_grant(
                commit_first_app,
                commit_first_placement.app_session_id,
                b"files.search"
            ),
            GrantCheck::ConsentRequired
        );
        assert!(record_exit_if_matches(
            commit_first_record.process_id,
            &commit_first_record
        ));

        // Also start both production operations together. Either lock order is
        // valid, but the result must match that one order and the final grant
        // must reflect the revoke.
        let racing_placement = LaunchPlacement {
            app_session_id: AppSessionId(0xA221),
            node_id: NodeId(0xA222),
            workspace_id: Some(WorkspaceId(0xA223)),
        };
        let (racing_app, racing_record) = register_allowed_launch(
            b"org.nagi.commit-race-simultaneous",
            0xA224_0001,
            racing_placement,
        );
        let start = Arc::new(Barrier::new(3));
        let commit_start = Arc::clone(&start);
        let revoke_start = Arc::clone(&start);
        let racing_committer = thread::spawn(move || {
            commit_start.wait();
            let mut commits = 0;
            let result = authorize_launch_and_commit(&racing_record, &[b"files.search"], || {
                commits += 1;
                assert!(super::SUPERVISOR.locked.load(super::Ordering::Acquire));
                Ok::<(), ()>(())
            });
            let authorized = result.is_ok();
            let denied = matches!(
                result,
                Err(LaunchCommitError::Grant(GrantCheck::ConsentRequired))
            );
            (authorized, denied, commits)
        });
        let racing_revoker = thread::spawn(move || {
            revoke_start.wait();
            record_user_decision(
                &authenticated_user(),
                racing_app,
                b"files.search",
                GrantDecision::Ask,
            )
            .expect("revoke during gate attempt");
        });
        start.wait();
        let (authorized, denied, commits) = racing_committer.join().expect("committer thread");
        racing_revoker.join().expect("revoker thread");
        assert!(authorized ^ denied);
        assert_eq!(commits, if authorized { 1 } else { 0 });
        assert_eq!(
            check_grant(racing_app, racing_placement.app_session_id, b"files.search"),
            GrantCheck::ConsentRequired
        );
        assert!(record_exit_if_matches(
            racing_record.process_id,
            &racing_record
        ));
    }

    #[test]
    fn registry_lock_is_released_when_operation_unwinds() {
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_registry(|_| panic!("injected registry operation panic"));
        }));

        assert!(panic.is_err());
        assert_eq!(with_registry(|_| 7), Some(7));
    }
}
