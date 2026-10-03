//! nagi-init's Supervisor launch registry (ADR 0046).
//!
//! The registry is the only way for init to start an isolated application:
//!
//! - Every launch must name a manifest-declared application. The registry
//!   checks the launch before spawning, then records
//!   `ProcessId -> LaunchRecord`.
//! - Services resolve kernel-stamped sender IDs through `resolve`.
//! - Services consult `has_grant` for capabilities. A grant exists only while
//!   the launched session is live.
//! - `reap` observes exit and revokes the launch.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use libnagi::launch::{AppManifest, LaunchError, LaunchPlacement, LaunchRecord, LaunchRegistry};
use libnagi::{
    channel_create_pair, channel_send, handle_close, process_spawn, sleep_ns, ChannelSendRequest,
    RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
use nagi_model::AppId;

/// Supervisor-owned application declarations embedded in the system image.
const MANIFESTS: [&[u8]; 5] = [
    include_bytes!("../manifests/org.nagi.acceptance.isolated-app.manifest"),
    include_bytes!("../manifests/org.nagi.acceptance.m19-search.manifest"),
    include_bytes!("../manifests/org.nagi.acceptance.m22-files.manifest"),
    include_bytes!("../manifests/org.nagi.acceptance.foreign-client.manifest"),
    include_bytes!("../manifests/org.nagi.acceptance.faulting-app.manifest"),
];

#[cfg(feature = "isolated-process-acceptance")]
pub const ISOLATED_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.isolated-app");
#[cfg(feature = "isolated-process-acceptance")]
pub const FAULTING_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.faulting-app");
/// Declared application that may query Search but owns no fixture objects
/// and holds no file-action grants.
#[cfg(feature = "m19-search-ipc")]
pub const FOREIGN_APP: AppId = AppId::from_identifier(b"org.nagi.acceptance.foreign-client");

// Stay below the Channel queue capacity so a live peer cannot make a probe
// send fail with QueueFull and be mistaken for an exit.
const EXIT_OBSERVATION_YIELDS: usize = 8;

struct SupervisorState {
    locked: AtomicBool,
    loaded: UnsafeCell<bool>,
    registry: UnsafeCell<LaunchRegistry>,
}

// Access is serialized by `locked`.
unsafe impl Sync for SupervisorState {}

static SUPERVISOR: SupervisorState = SupervisorState {
    locked: AtomicBool::new(false),
    loaded: UnsafeCell::new(false),
    registry: UnsafeCell::new(LaunchRegistry::new()),
};

/// Run `operation` on the registry, loading the embedded manifests on first
/// use. Returns `None` if any embedded manifest is invalid, so a broken
/// manifest disables launches instead of granting defaults.
fn with_registry<R>(operation: impl FnOnce(&mut LaunchRegistry) -> R) -> Option<R> {
    while SUPERVISOR
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let registry = unsafe { &mut *SUPERVISOR.registry.get() };
    let loaded = unsafe { &mut *SUPERVISOR.loaded.get() };
    if !*loaded {
        *loaded = MANIFESTS.iter().all(|text| {
            AppManifest::parse(text)
                .is_ok_and(|manifest| registry.register_manifest(manifest).is_ok())
        });
    }
    let result = loaded.then(|| operation(registry));
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
    Channel,
    Spawn,
}

/// Launch `elf` as the declared application `app_id` at `placement`. The
/// registry check runs before the process exists. The child receives one
/// read/write/wait endpoint and no transfer right.
pub fn launch(
    elf: &[u8],
    app_id: AppId,
    placement: LaunchPlacement,
) -> Result<Launched, LaunchFailure> {
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
    with_registry(|registry| registry.has_grant(app_id, app_session_id, capability))
        .unwrap_or(false)
}

/// Check launch preconditions for `app_id` at `placement` without spawning.
#[cfg(feature = "isolated-process-acceptance")]
pub fn check(app_id: AppId, placement: LaunchPlacement) -> Result<(), LaunchFailure> {
    with_registry(|registry| registry.check_launch(app_id, placement))
        .ok_or(LaunchFailure::ManifestsUnavailable)?
        .map_err(LaunchFailure::Registry)
}

/// Observe the launched process's exit (its endpoint becomes unreachable),
/// then revoke its launch record and release the endpoint.
pub fn reap(launched: Launched) -> bool {
    let mut exited = false;
    for _ in 0..EXIT_OBSERVATION_YIELDS {
        sleep_ns(0);
        if !channel_send(launched.endpoint, &ChannelSendRequest::new(0, 0, 0, 0)) {
            exited = true;
            break;
        }
    }
    if !exited {
        return false;
    }
    let revoked = with_registry(|registry| registry.record_exit(launched.record.process_id))
        .flatten()
        .is_some_and(|record| record == launched.record);
    let still_granted = resolve(launched.record.process_id).is_some();
    handle_close(launched.endpoint) && revoked && !still_granted
}
