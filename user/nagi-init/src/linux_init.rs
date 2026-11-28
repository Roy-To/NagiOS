//! Linux entry and lifecycle contract for the x86_64-unknown-linux-nagi reference.
//! target_os=linux keeps this Linux userland entry separate from target_os=nagi.
//!
//! The production init path remains unconnected. The private Supervisor adapter
//! below consumes caller-authenticated package/session inputs, while run() stays
//! fail-closed until production trust, consent, inventory, and transport inputs
//! are wired. This path does not use libnagi's Nagi-only syscall wrappers.

use std::convert::Infallible;
use std::ffi::CString;
use std::process::ExitCode;
#[cfg(feature = "linux-service-action-bridge")]
use std::time::Duration;

#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
use libnagi::launch::{AppManifest, GrantCheck, LaunchPlacement, LaunchRecord};
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
use libnagi::security::Session;
#[cfg(linux_dev_preview_smoke)]
use libnagi::security::{AccountStore, Role};
#[cfg(feature = "linux-service-action-bridge")]
use nagi_action_ipc::ActionResult;
#[cfg(unix)]
use nagi_init_package_provider::linux_package_persistence::LinuxInstalledPackagePersistence;
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
use nagi_init_package_provider::package_trust::{
    InstallationState, InstalledPackageInventory, PackageProviderError, PackageSecurityProvider,
};
#[cfg(linux_dev_preview_smoke)]
use nagi_init_package_provider::package_trust::{
    OwnedPackageInventorySnapshot, PackageServiceSessionOwner, PackageServiceSessionOwnerError,
};
#[cfg(feature = "linux-service-action-bridge")]
use nagi_init_service_call::{ActionCallError, CallError};
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
use nagi_model::AppId;
#[cfg(linux_dev_preview_smoke)]
use nagi_model::{AppSessionId, NodeId};
#[cfg(linux_dev_preview_smoke)]
use nagi_package::{InstallPolicy, PackageService, PackageView, TRUSTED_SIGNING_PUBLIC_KEY};
#[cfg(feature = "linux-service-action-bridge")]
use nagi_service_contract::{
    CancellationToken, ContractVersion, IpcErrorCode, OperationId, ServiceClient, ServiceId,
};
#[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
#[path = "linux_boot_package_inputs.rs"]
mod linux_boot_package_inputs;
#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    not(linux_dev_preview_smoke)
))]
#[path = "linux_c3_boot_launch.rs"]
mod linux_c3_boot_launch;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "linux_mount_prerequisites/mod.rs"]
mod linux_mount_prerequisites;
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
#[path = "linux_package_identity.rs"]
mod linux_package_identity;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
// The standalone lifecycle candidate has its own exit-status type and reaper state.
#[cfg_attr(test, allow(clippy::duplicate_mod))]
#[path = "pid1_child_reaper.rs"]
mod pid1_child_reaper;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "linux_init_c3_system_service_owner.rs"]
mod linux_init_c3_system_service_owner;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "linux_init_c3_slot_journal.rs"]
mod linux_init_c3_slot_journal;

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "linux_task24_c3_data_journal.rs"]
mod linux_task24_c3_data_journal;

// The standalone lifecycle candidate intentionally binds its own error/status types.
#[cfg_attr(test, allow(clippy::duplicate_mod))]
#[path = "pid1_owned_launch_lease.rs"]
mod pid1_owned_launch_lease;

#[cfg(all(
    linux_dev_preview_smoke,
    not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(test, target_os = "macos", target_arch = "x86_64")
    ))
))]
compile_error!("linux_dev_preview_smoke runtime is x86_64 Linux only; host tests may run on macOS");
#[cfg(all(linux_dev_preview_smoke, not(feature = "m10-desktop")))]
compile_error!("linux_dev_preview_smoke requires the m10-desktop process and package APIs");
#[cfg(all(linux_dev_preview_smoke, feature = "production-session"))]
compile_error!("linux_dev_preview_smoke is incompatible with production-session");
#[cfg(all(linux_desktop_dev_smoke, not(linux_dev_preview_smoke)))]
compile_error!("linux_desktop_dev_smoke requires linux_dev_preview_smoke");
#[cfg(all(linux_desktop_dev_smoke, feature = "production-session"))]
compile_error!("linux_desktop_dev_smoke is incompatible with production-session");
#[cfg(all(
    linux_dev_preview_smoke,
    any(nagi_release_build, not(debug_assertions))
))]
compile_error!("linux_dev_preview_smoke cannot be included in release builds");
#[cfg(all(
    feature = "linux-service-action-bridge",
    not(all(target_os = "linux", target_arch = "x86_64"))
))]
compile_error!("linux-service-action-bridge is available only on x86_64 Linux");
#[cfg(all(
    linux_pid1_safety_smoke,
    not(all(target_os = "linux", target_arch = "x86_64"))
))]
compile_error!("linux_pid1_safety_smoke is available only on x86_64 Linux");
#[cfg(all(
    linux_pid1_safety_smoke,
    any(nagi_release_build, not(debug_assertions))
))]
compile_error!("linux_pid1_safety_smoke cannot be included in release builds");
#[cfg(all(linux_pid1_safety_smoke, feature = "production-session"))]
compile_error!("linux_pid1_safety_smoke is incompatible with production-session");
#[cfg(all(linux_pid1_safety_smoke, linux_dev_preview_smoke))]
compile_error!("PID 1 safety smoke and dev preview smoke must be built separately");

const EX_USAGE: u8 = 64;
const EX_SOFTWARE: u8 = 70;
const EX_IOERR: u8 = 74;
const EX_TEMPFAIL: u8 = 75;
const EX_NOPERM: u8 = 77;
const EX_PROTOCOL: u8 = 76;
const EX_UNAVAILABLE: u8 = 69;

/// Stable application-level failure for a typed request sent to a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceFailureCode {
    InvalidRequest,
    Unauthorized,
    ConsentRequired,
    ConsentDenied,
    Revoked,
    Cancelled,
    NotFound,
    Failed,
}

/// A request retains its caller-owned, typed action payload end to end.
///
/// The Linux backend must bind Action to Nagi's canonical Public Action type.
/// It must not substitute shell text, host paths, Linux UID, or root status for
/// signed identity, owner consent, revocation, or deterministic authorization.
#[derive(Debug)]
pub(crate) struct ServiceRequest<Action> {
    pub(crate) request_id: u64,
    pub(crate) action: Action,
}

/// Result of one correlated service request.
#[derive(Debug)]
pub(crate) enum ServiceResponse<Reply> {
    Completed {
        request_id: u64,
        reply: Reply,
    },
    Rejected {
        request_id: u64,
        reason: ServiceFailureCode,
    },
}

/// Transport or protocol failure while connecting to a Nagi service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceIoError {
    Unavailable,
    Closed,
    TimedOut,
    ProtocolViolation,
    TransportFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendError {
    Unavailable,
    IdentityRejected,
    ConsentDenied,
    Revoked,
    Cancelled,
    ProtocolViolation,
    TransportFailure,
    ServiceFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownCause {
    Requested,
    RequiredServiceExited,
    BackendFailure,
    ProtocolViolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownSignal {
    Terminate,
    Interrupt,
    Hangup,
    Other(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceExitStatus {
    pub(crate) exit_code: Option<i32>,
    pub(crate) signal: Option<i32>,
}

/// Events returned by the platform backend's blocking lifecycle wait.
#[derive(Debug)]
pub(crate) enum LifecycleEvent<ServiceId> {
    /// Emit only after every mandatory service has authenticated and completed
    /// its protocol handshake. This is the sole event that makes init ready.
    Ready,
    ServiceExited {
        service: ServiceId,
        status: ServiceExitStatus,
        required: bool,
    },
    ShutdownRequested {
        signal: ShutdownSignal,
    },
}

/// Linux implementation boundary consumed by run_with_backend.
///
/// initialize must validate signature-derived service identities, consent,
/// cancellation and revocation, and deterministic policy before starting
/// anything. It must roll back partial startup before returning an error.
/// Process UID/root are isolation details only and grant no Nagi authority.
///
/// call_service is the minimum typed request/reply contract. The backend
/// owns authenticated handles and must correlate replies by request_id;
/// only typed Public Actions and their typed replies may be bound to the
/// generic parameters.
///
/// shutdown must stop the service set and revoke its grants before success.
pub(crate) trait InitBackend {
    type Action;
    type Reply;
    type Session;
    type ServiceHandle;
    type ServiceId: std::fmt::Debug;

    fn initialize(&mut self) -> Result<Self::Session, BackendError>;

    fn call_service(
        &mut self,
        session: &mut Self::Session,
        service: &mut Self::ServiceHandle,
        request: ServiceRequest<Self::Action>,
        #[cfg(feature = "linux-service-action-bridge")] cancellation: CancellationToken,
        #[cfg(feature = "linux-service-action-bridge")] timeout: Duration,
    ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError>;

    fn wait_event(
        &mut self,
        session: &mut Self::Session,
    ) -> Result<LifecycleEvent<Self::ServiceId>, BackendError>;

    fn shutdown(
        &mut self,
        session: &mut Self::Session,
        cause: ShutdownCause,
    ) -> Result<(), BackendError>;
}

/// Runtime-owned route input for the existing action@1 codec. The constructor
/// is crate-private so only trusted init runtime wiring can supply a client;
/// the client must use the runtime's authenticated peer and live-registration
/// transport. This handle does not establish identity, consent, or READY.
#[cfg(feature = "linux-service-action-bridge")]
pub(crate) struct RuntimeActionServiceHandle {
    client: ServiceClient,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
}

#[cfg(feature = "linux-service-action-bridge")]
impl RuntimeActionServiceHandle {
    pub(crate) fn from_authenticated_runtime(
        client: ServiceClient,
        service_id: ServiceId,
        contract_version: ContractVersion,
        operation_id: OperationId,
    ) -> Self {
        Self {
            client,
            service_id,
            contract_version,
            operation_id,
        }
    }
}

/// Adapt the actual `InitBackend` request types to the reviewed action@1 and
/// service-contract consumers. Route, cancellation, and timeout are required
/// runtime inputs; this function invents no route or product timeout.
#[cfg(feature = "linux-service-action-bridge")]
pub(crate) fn call_action_service(
    request: ServiceRequest<String>,
    service: &RuntimeActionServiceHandle,
    cancellation: CancellationToken,
    timeout: Duration,
) -> Result<ServiceResponse<ActionResult>, ServiceIoError> {
    let request_id = request.request_id;
    match nagi_init_service_call::call_action_blocking(
        &service.client,
        request_id,
        service.service_id.clone(),
        service.contract_version,
        service.operation_id.clone(),
        &request.action,
        cancellation,
        timeout,
    ) {
        Ok(reply) => Ok(ServiceResponse::Completed { request_id, reply }),
        Err(error) => service_call_error_for_action(request_id, error),
    }
}

#[cfg(feature = "linux-service-action-bridge")]
fn service_call_error_for_action(
    request_id: u64,
    error: ActionCallError,
) -> Result<ServiceResponse<ActionResult>, ServiceIoError> {
    match error {
        ActionCallError::InvalidRequestId
        | ActionCallError::EncodeIntent(_)
        | ActionCallError::BuildRequest(_) => Ok(ServiceResponse::Rejected {
            request_id,
            reason: ServiceFailureCode::InvalidRequest,
        }),
        ActionCallError::DecodeResult(_) => Err(ServiceIoError::ProtocolViolation),
        ActionCallError::Call(CallError::Cancelled) => Ok(ServiceResponse::Rejected {
            request_id,
            reason: ServiceFailureCode::Cancelled,
        }),
        ActionCallError::Call(CallError::TimedOut) => Err(ServiceIoError::TimedOut),
        // The call adapter rejects an unrepresentable runtime deadline before
        // resolving a service or entering its provider. Treat this as a local
        // unavailable runtime configuration, not a transport failure.
        ActionCallError::Call(CallError::InvalidTimeout) => Err(ServiceIoError::Unavailable),
        ActionCallError::Call(CallError::ResponseMismatch) => {
            Err(ServiceIoError::ProtocolViolation)
        }
        ActionCallError::Call(CallError::Discovery(error) | CallError::Remote(error)) => {
            service_response_for_contract_error(request_id, error.code())
        }
    }
}

#[cfg(feature = "linux-service-action-bridge")]
fn service_response_for_contract_error(
    request_id: u64,
    code: IpcErrorCode,
) -> Result<ServiceResponse<ActionResult>, ServiceIoError> {
    let reject = |reason| Ok(ServiceResponse::Rejected { request_id, reason });
    match code {
        IpcErrorCode::ServiceNotFound | IpcErrorCode::OperationNotFound => {
            reject(ServiceFailureCode::NotFound)
        }
        IpcErrorCode::InvalidRequest => reject(ServiceFailureCode::InvalidRequest),
        IpcErrorCode::PermissionDenied => reject(ServiceFailureCode::Unauthorized),
        IpcErrorCode::Unavailable => Err(ServiceIoError::Unavailable),
        IpcErrorCode::Cancelled => reject(ServiceFailureCode::Cancelled),
        IpcErrorCode::DeadlineExceeded => Err(ServiceIoError::TimedOut),
        IpcErrorCode::TransportFailure => Err(ServiceIoError::TransportFailure),
        // A stale route registration is not evidence that an app grant or
        // user authorization was revoked. The current public result enum has
        // no dedicated route-staleness variant, so keep it as a generic
        // failure until the canonical contract adds one.
        IpcErrorCode::StaleRegistration => reject(ServiceFailureCode::Failed),
        IpcErrorCode::UnsupportedContractVersion
        | IpcErrorCode::Busy
        | IpcErrorCode::ProviderFailure
        | IpcErrorCode::SerializationFailure
        | IpcErrorCode::DuplicateRegistration
        | IpcErrorCode::ResponseTooLarge => reject(ServiceFailureCode::Failed),
    }
}

/// Linux main enters only through this function.
macro_rules! pid1_log {
    ($($argument:tt)*) => {{
        $crate::linux_init::log_pid1_stderr(format_args!($($argument)*));
    }};
}

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    not(linux_dev_preview_smoke)
))]
#[path = "linux_initial_login_owner.rs"]
mod linux_initial_login_owner;

#[cfg(all(
    linux_task24_initial_login_runtime,
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "linux_initial_login_c2.rs"]
mod linux_initial_login_c2;

#[cfg(all(
    linux_task24_owner_co_location,
    target_os = "linux",
    target_arch = "x86_64",
    feature = "desktop-login",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "../candidates/linux-broker-stage25/stage25/pid1_live_peer_supplier.rs"]
pub(crate) mod pid1_live_peer_supplier;

#[cfg(all(
    linux_task24_owner_co_location,
    target_os = "linux",
    target_arch = "x86_64",
    feature = "desktop-login",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "linux_task24_endpoint_launch.rs"]
mod linux_task24_endpoint_launch;

#[cfg(all(
    linux_task24_owner_co_location,
    target_os = "linux",
    target_arch = "x86_64",
    feature = "desktop-login",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "linux_task24_identity_ready.rs"]
mod linux_task24_identity_ready;

#[cfg(all(
    any(linux_boot_owner_registration, linux_task24_owner_co_location),
    feature = "desktop-login",
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "linux_owner_registration_console.rs"]
mod linux_owner_registration_console;

#[cfg(not(all(
    linux_dev_system_mount,
    linux_dev_system_verity,
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
)))]
pub(crate) fn run() -> ExitCode {
    if std::process::id() != 1 {
        pid1_log!("nagi-init: refusing Linux init entry outside PID 1");
        return ExitCode::from(EX_USAGE);
    }

    #[cfg(all(linux_pid1_safety_smoke, target_os = "linux", target_arch = "x86_64"))]
    return linux_x86_64_process::run_pid1_safety_smoke();

    #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
    let mut backend = match linux_boot_package_inputs::BootPackageInputs::take_inherited() {
        Ok(inputs) => UnconnectedBackend::with_verified_boot_inputs(inputs),
        Err(error) => {
            pid1_log!("nagi-init: Linux boot package inputs unavailable ({error})");
            UnconnectedBackend::new()
        }
    };

    #[cfg(target_os = "linux")]
    {
        let app_storage_owners = crate::profile_volume_notes_owner::pending();
        match normal_startup_profile_notes_bundle(&app_storage_owners) {
            Err(error) => log_pid1_stderr(format_args!(
                "NAGI_NOTES_PROFILE_VOLUME_UNAVAILABLE error={error} fallback=none"
            )),
            Ok(bundle) => {
                // PID 1 has no connected C1 identity/C2 sealed-caller source
                // or Notes bridge launcher yet. Do not retain an orphaned
                // capability or switch to the development Notes store.
                drop(bundle);
                log_pid1_stderr(format_args!(
                    "NAGI_NOTES_PROFILE_VOLUME_UNAVAILABLE reason=c1_c2_consumer_unavailable fallback=none"
                ));
            }
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    match mount_devtmpfs_for_pid1() {
        Ok(()) => log_pid1_stderr(format_args!(
            "nagi-init: devtmpfs mounted before service startup"
        )),
        Err(error) => {
            log_pid1_stderr(format_args!(
                "nagi-init: failed to mount devtmpfs ({error}); no service is ready"
            ));
            wait_in_pid1(wait_for_pid1_os_event)
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    ensure_pid1_proc_and_sysfs();

    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        not(any(linux_dev_preview_smoke, nagi_release_build))
    ))]
    if backend.boot_catalog.is_some() {
        match linux_mount_prerequisites::data_mount_owner::mount_identity_data() {
            Ok(Some(data)) => {
                pid1_log!("NAGI_DEV_IDENTITY_DATA_MOUNTED retained_fd=true");
                backend.identity_data = Some(data);
            }
            Ok(None) => {}
            Err(reason) => pid1_log!("NAGI_DEV_IDENTITY_DATA_UNAVAILABLE reason={reason}"),
        }
    }

    #[cfg(all(
        linux_dev_system_mount,
        target_os = "linux",
        target_arch = "x86_64",
        not(any(linux_dev_preview_smoke, nagi_release_build))
    ))]
    match linux_mount_prerequisites::data_mount_owner::requested_system_data_probe() {
        Ok(Some(probe)) => match backend.identity_data.as_mut() {
            Some(data) => match probe.run(
                |token| data.observe_system_data(token),
                |line| log_pid1_stderr(format_args!("{line}")),
                || {
                    // The observer has synced the Data file and directory.
                    // A successful poweroff never returns to PID 1.
                    let result = unsafe { libc::reboot(libc::RB_POWER_OFF) };
                    if result == -1 {
                        let error = std::io::Error::last_os_error();
                        log_pid1_stderr(format_args!(
                            "SYSTEM_DATA_MOUNT_GUEST_FAIL stage=poweroff error={error}"
                        ));
                    } else {
                        log_pid1_stderr(format_args!(
                            "SYSTEM_DATA_MOUNT_GUEST_FAIL stage=poweroff reason=returned"
                        ));
                    }
                    wait_in_pid1(wait_for_pid1_os_event);
                },
            ) {
                Ok(()) => {}
                Err(error) => {
                    log_pid1_stderr(format_args!(
                        "SYSTEM_DATA_MOUNT_GUEST_FAIL stage=probe error={error}"
                    ));
                    wait_in_pid1(wait_for_pid1_os_event);
                }
            },
            None => {
                log_pid1_stderr(format_args!(
                    "SYSTEM_DATA_MOUNT_GUEST_FAIL stage=data_owner reason=unavailable"
                ));
                wait_in_pid1(wait_for_pid1_os_event);
            }
        },
        Ok(None) => {}
        Err(reason) => {
            log_pid1_stderr(format_args!(
                "SYSTEM_DATA_MOUNT_GUEST_FAIL stage=probe_token reason={reason}"
            ));
            wait_in_pid1(wait_for_pid1_os_event);
        }
    }

    // This branch is compiled only in the non-release development preview and
    // requires an explicit guest command-line token. It never authenticates a
    // Profile or promotes the ordinary Pending Notes owner.
    #[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
    let _ = linux_mount_prerequisites::maybe_run_development_data_mount_smoke();

    #[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
    let mut backend = LinuxDevPreviewBackend::new();
    #[cfg(not(target_os = "linux"))]
    let mut backend = UnconnectedBackend::new();
    run_pid1_lifecycle(&mut backend, wait_for_pid1_os_event)
}

#[cfg(all(
    linux_dev_system_mount,
    linux_dev_system_verity,
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
pub(crate) fn run() -> ExitCode {
    if std::process::id() != 1 {
        pid1_log!("nagi-init: refusing Linux System verity fixture outside PID 1");
        return ExitCode::from(EX_USAGE);
    }
    if let Err(error) = mount_devtmpfs_for_pid1() {
        log_pid1_stderr(format_args!(
            "NAGI_SYSTEM_VERITY_REJECTED stage=devtmpfs errno={error}"
        ));
        return ExitCode::FAILURE;
    }
    ensure_pid1_proc_and_sysfs();
    match linux_mount_prerequisites::data_mount_owner::system_verity::run_if_requested() {
        linux_mount_prerequisites::data_mount_owner::system_verity::ProbeOutcome::Accepted {
            root_digest,
            mount_id,
        } => log_pid1_stderr(format_args!(
            "NAGI_SYSTEM_VERITY_ACCEPTED root={root_digest} filesystem=ext4 readonly=true fixture=true mount_id={mount_id}"
        )),
        linux_mount_prerequisites::data_mount_owner::system_verity::ProbeOutcome::Rejected {
            stage,
            errno,
        } => log_pid1_stderr(format_args!(
            "NAGI_SYSTEM_VERITY_REJECTED stage={stage} errno={errno}"
        )),
    }
    let result = unsafe { libc::reboot(libc::RB_POWER_OFF) };
    let error = std::io::Error::last_os_error();
    log_pid1_stderr(format_args!(
        "NAGI_SYSTEM_VERITY_POWER_OFF_FAILED result={result} error={error}"
    ));
    wait_in_pid1(wait_for_pid1_os_event)
}

#[cfg(target_os = "linux")]
fn normal_startup_profile_notes_bundle(
    owner: &crate::profile_volume_notes_owner::AppStorageOwners,
) -> Result<
    crate::profile_volume_notes_owner::ProfileVolumeNotesBundle,
    crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
> {
    owner.issue_bundle()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn wait_for_pid1_os_event() {
    linux_x86_64_process::wait_for_os_event();
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn wait_for_pid1_os_event() {
    std::thread::park_timeout(std::time::Duration::from_secs(3600));
}

/// Mount the early virtual filesystems after devtmpfs and before backend
/// construction. Existing mounts are accepted only when the prerequisite
/// module verifies their filesystem type, mount identity, and required flags.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn ensure_pid1_proc_and_sysfs() {
    use linux_mount_prerequisites::ensure_proc_and_sysfs;

    match ensure_proc_and_sysfs() {
        Ok(report) => {
            log_pid1_stderr(format_args!(
                "nagi-init: procfs/sysfs mount prerequisites verified before service startup (procfs={:?}, sysfs={:?})",
                report.procfs, report.sysfs
            ));
        }
        Err(error) => {
            log_pid1_stderr(format_args!(
                "nagi-init: procfs/sysfs prerequisites failed ({error}); procfs may already be mounted, no service is ready"
            ));
            wait_in_pid1(wait_for_pid1_os_event)
        }
    }
}

/// PID 1 diagnostics are best effort: a closed stderr must not unwind the
/// fail-closed wait or the cleanup-unknown panic boundary.
fn log_pid1_stderr(arguments: std::fmt::Arguments<'_>) {
    use std::io::Write as _;

    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();
    let _ = stderr.write_fmt(arguments);
    let _ = stderr.write_all(b"\n");
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[link(name = "c")]
extern "C" {
    fn mount(
        source: *const std::os::raw::c_char,
        target: *const std::os::raw::c_char,
        filesystemtype: *const std::os::raw::c_char,
        mountflags: std::os::raw::c_ulong,
        data: *const core::ffi::c_void,
    ) -> std::os::raw::c_int;
    fn chown(
        path: *const std::os::raw::c_char,
        owner: std::os::raw::c_uint,
        group: std::os::raw::c_uint,
    ) -> std::os::raw::c_int;
}

/// An early `rdinit=/init` bypasses Linux `prepare_namespace()`, which is the
/// path that calls `devtmpfs_mount()`. Mounting here makes kernel-registered
/// device nodes (including `/dev/fb0`) available before services start.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn mount_devtmpfs_for_pid1() -> std::io::Result<()> {
    mount_devtmpfs_at(b"/dev\0")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn mount_devtmpfs_at(target: &'static [u8]) -> std::io::Result<()> {
    const MS_NOSUID: std::os::raw::c_ulong = 2;
    const MS_NOEXEC: std::os::raw::c_ulong = 8;
    const SOURCE: &[u8] = b"devtmpfs\0";
    const FILESYSTEM: &[u8] = b"devtmpfs\0";
    const OPTIONS: &[u8] = b"mode=0755\0";

    // SAFETY: these static NUL-terminated strings match the Linux mount(2)
    // contract; the production caller uses the initramfs /dev directory.
    let result = unsafe {
        mount(
            SOURCE.as_ptr().cast(),
            target.as_ptr().cast(),
            FILESYSTEM.as_ptr().cast(),
            MS_NOSUID | MS_NOEXEC,
            OPTIONS.as_ptr().cast(),
        )
    };
    if result == 0 {
        return Ok(());
    }

    // Any failure, including EBUSY, stays fail closed. PID 1 does not infer
    // the filesystem already mounted at /dev from a generic mount error.
    Err(std::io::Error::last_os_error())
}

fn wait_in_pid1(mut wait_for_os_event: impl FnMut()) -> ! {
    log_pid1_stderr(format_args!(
        "nagi-init: PID 1 remains alive in fail-closed wait; service readiness is not asserted"
    ));
    loop {
        wait_for_os_event();
    }
}

fn run_pid1_lifecycle<B: InitBackend>(backend: &mut B, mut wait_for_os_event: impl FnMut()) -> ! {
    let lifecycle =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_lifecycle(backend)));
    match lifecycle {
        Ok(result) => log_pid1_stderr(format_args!(
            "nagi-init: lifecycle returned {result:?}; PID 1 will remain alive"
        )),
        Err(_) => log_pid1_stderr(format_args!(
            "nagi-init: lifecycle panicked; session was dropped, service cleanup is unknown, and PID 1 will remain alive"
        )),
    }
    wait_in_pid1(&mut wait_for_os_event)
}

#[cfg(any(test, linux_task24_notes_consumer))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotesConsoleExit {
    ClosedByUser,
    ClosedWithoutSave,
    RequiredIdentityExited,
    StateUnknown,
}

#[cfg(any(test, linux_task24_notes_consumer))]
impl NotesConsoleExit {
    fn allows_normal_login_reconciliation(
        notes_exit: Option<Self>,
        login_child_exited_cleanly: bool,
    ) -> bool {
        login_child_exited_cleanly
            && matches!(
                notes_exit,
                None | Some(Self::ClosedByUser | Self::ClosedWithoutSave)
            )
    }
}

/// Read a keyboard slice, then observe the required service before accepting
/// either the input or a keyboard error. The caller owns the existing reaper.
#[cfg(any(test, linux_task24_initial_login_runtime))]
enum Task24InputPoll<Input> {
    Pending,
    Input(Input),
    RequiredServiceExited,
}

#[cfg(any(test, linux_task24_initial_login_runtime))]
fn poll_task24_input<Input>(
    wait_for_input: impl FnOnce() -> std::io::Result<Option<Input>>,
    poll_required_service: impl FnOnce() -> std::io::Result<bool>,
) -> std::io::Result<Task24InputPoll<Input>> {
    let input = wait_for_input();
    if poll_required_service()? {
        return Ok(Task24InputPoll::RequiredServiceExited);
    }
    Ok(match input? {
        Some(input) => Task24InputPoll::Input(input),
        None => Task24InputPoll::Pending,
    })
}

#[cfg(any(test, linux_task24_initial_login_runtime))]
fn wait_for_task24_input<Input>(
    mut wait_for_input: impl FnMut(std::time::Duration) -> std::io::Result<Option<Input>>,
    mut poll_required_service: impl FnMut() -> std::io::Result<bool>,
) -> std::io::Result<Option<Input>> {
    loop {
        match poll_task24_input(
            || wait_for_input(std::time::Duration::from_millis(100)),
            &mut poll_required_service,
        )? {
            Task24InputPoll::Pending => {}
            Task24InputPoll::RequiredServiceExited => return Ok(None),
            Task24InputPoll::Input(input) => return Ok(Some(input)),
        }
    }
}

/// Lifecycle core after the production PID 1 guard. Unit tests exercise only
/// this orchestration contract with a fake backend; they do not start processes.
///
/// This loop owns readiness, required-service exit, backend failure, and
/// shutdown behavior. A backend cannot make init report ready by merely
/// starting a process; it must deliver the explicit authenticated Ready
/// event after all required handshakes have completed.
pub(crate) fn run_lifecycle<B: InitBackend>(backend: &mut B) -> ExitCode {
    run_lifecycle_with_ready_publisher(backend, publish_lifecycle_ready)
}

fn run_lifecycle_with_ready_publisher<B: InitBackend>(
    backend: &mut B,
    mut publish_ready: impl FnMut(&mut bool) -> Result<(), BackendError>,
) -> ExitCode {
    let mut session = match backend.initialize() {
        Ok(session) => session,
        Err(error) => {
            pid1_log!("nagi-init: startup failed ({error:?}); no service is ready");
            return backend_exit_code(error);
        }
    };

    let mut ready = false;
    loop {
        let event = match backend.wait_event(&mut session) {
            Ok(event) => event,
            Err(error) => {
                return stop_after_failure(
                    backend,
                    &mut session,
                    ShutdownCause::BackendFailure,
                    error,
                );
            }
        };

        match event {
            LifecycleEvent::Ready => {
                if let Err(error) = publish_ready(&mut ready) {
                    let cause = if error == BackendError::ProtocolViolation {
                        ShutdownCause::ProtocolViolation
                    } else {
                        ShutdownCause::BackendFailure
                    };
                    return stop_after_failure(backend, &mut session, cause, error);
                }
            }
            LifecycleEvent::ServiceExited {
                service,
                status,
                required: true,
            } => {
                pid1_log!(
                    "nagi-init: required service {service:?} exited \
                     (status={status:?}, ready={ready})"
                );
                return stop_after_failure(
                    backend,
                    &mut session,
                    ShutdownCause::RequiredServiceExited,
                    BackendError::ServiceFailure,
                );
            }
            LifecycleEvent::ServiceExited {
                service,
                status,
                required: false,
            } => {
                pid1_log!(
                    "nagi-init: optional service {service:?} exited \
                     (status={status:?})"
                );
            }
            LifecycleEvent::ShutdownRequested { signal } => {
                pid1_log!("nagi-init: shutdown requested ({signal:?})");
                return match backend.shutdown(&mut session, ShutdownCause::Requested) {
                    Ok(()) if ready => ExitCode::SUCCESS,
                    Ok(()) => ExitCode::from(EX_TEMPFAIL),
                    Err(error) => {
                        pid1_log!("nagi-init: shutdown failed ({error:?})");
                        ExitCode::from(EX_SOFTWARE)
                    }
                };
            }
        }
    }
}

fn publish_lifecycle_ready(ready: &mut bool) -> Result<(), BackendError> {
    use std::os::fd::AsFd as _;

    publish_lifecycle_ready_fd(ready, std::io::stderr().lock().as_fd())
}

fn publish_lifecycle_ready_fd(
    ready: &mut bool,
    fd: std::os::fd::BorrowedFd<'_>,
) -> Result<(), BackendError> {
    // Write through an owned duplicate of the same descriptor. The standard
    // stderr writer can suppress an OS bad-descriptor error.
    publish_lifecycle_ready_to(ready, || fd.try_clone_to_owned().map(std::fs::File::from))
}

fn publish_lifecycle_ready_to<W: std::io::Write>(
    ready: &mut bool,
    open_writer: impl FnOnce() -> std::io::Result<W>,
) -> Result<(), BackendError> {
    if *ready {
        return Err(BackendError::ProtocolViolation);
    }
    let mut writer = open_writer().map_err(|_| BackendError::TransportFailure)?;
    writer
        .write_all(b"nagi-init: state=ready\n")
        .and_then(|()| writer.flush())
        .map_err(|_| BackendError::TransportFailure)?;
    *ready = true;
    Ok(())
}

fn stop_after_failure<B: InitBackend>(
    backend: &mut B,
    session: &mut B::Session,
    cause: ShutdownCause,
    failure: BackendError,
) -> ExitCode {
    pid1_log!("nagi-init: lifecycle failure ({failure:?}); stopping services");
    if let Err(cleanup_error) = backend.shutdown(session, cause) {
        pid1_log!("nagi-init: cleanup failed ({cleanup_error:?})");
        return ExitCode::from(EX_SOFTWARE);
    }
    backend_exit_code(failure)
}

fn backend_exit_code(error: BackendError) -> ExitCode {
    let code = match error {
        BackendError::Unavailable => EX_UNAVAILABLE,
        BackendError::IdentityRejected | BackendError::ConsentDenied | BackendError::Revoked => {
            EX_NOPERM
        }
        BackendError::Cancelled => EX_TEMPFAIL,
        BackendError::ProtocolViolation => EX_PROTOCOL,
        BackendError::TransportFailure => EX_IOERR,
        BackendError::ServiceFailure => EX_SOFTWARE,
    };
    ExitCode::from(code)
}

/// Internal process lifecycle engine. Its only way to make a child executable
/// is to release a Prepared handle after the existing registration/auth adapter
/// has returned a launch record. The production adapter remains disconnected.
trait ProcessStage {}
struct Prepared;
struct Running;
struct ReleaseUncertain;
impl ProcessStage for Prepared {}
impl ProcessStage for Running {}
impl ProcessStage for ReleaseUncertain {}

struct ProcessHandle<Stage: ProcessStage, Raw> {
    process_id: u32,
    raw: Raw,
    _stage: std::marker::PhantomData<Stage>,
}

impl<Stage: ProcessStage, Raw> ProcessHandle<Stage, Raw> {
    fn new(process_id: u32, raw: Raw) -> Self {
        Self {
            process_id,
            raw,
            _stage: std::marker::PhantomData,
        }
    }

    fn transition<Next: ProcessStage>(self) -> ProcessHandle<Next, Raw> {
        ProcessHandle::new(self.process_id, self.raw)
    }
}

/// This token can only be constructed after the existing registration and
/// authorization callback succeeds. ProcessOs::release requires it.
struct RegisteredLaunch<LaunchRecord> {
    record: LaunchRecord,
}

impl<LaunchRecord> RegisteredLaunch<LaunchRecord> {
    fn into_record(self) -> LaunchRecord {
        self.record
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseFailureReason {
    Cancelled,
    TimedOut,
    ExecError(i32),
    ChildExited(ServiceExitStatus),
    PartialErrorReport,
    ProtocolViolation,
    Io,
}

struct ReleaseFailure<Child> {
    child: Child,
    reason: ReleaseFailureReason,
    reaped: Option<ServiceExitStatus>,
}

type ExecAckResult<Raw> =
    Result<ProcessHandle<Running, Raw>, ReleaseFailure<ProcessHandle<ReleaseUncertain, Raw>>>;

/// One-shot capability to write the exec gate after registry authorization.
/// It prevents a caller from writing the gate twice or reporting success
/// without committing it.
struct GateCommit<'a, Os: ProcessOs, LaunchRecord, LaunchLease = ()> {
    os: &'a mut Os,
    child: &'a mut ProcessHandle<Prepared, Os::Raw>,
    registered: &'a RegisteredLaunch<LaunchRecord>,
    launch_lease:
        &'a mut Option<pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_retirement: &'a mut pid1_owned_launch_lease::LeaseRetirementState,
    require_launch_lease: bool,
    wait_budget: std::time::Duration,
    attempted: bool,
    duplicate_attempt: bool,
    started: Option<std::time::Instant>,
    failure: Option<ReleaseFailureReason>,
}

impl<'a, Os: ProcessOs, LaunchRecord, LaunchLease> GateCommit<'a, Os, LaunchRecord, LaunchLease> {
    fn new(
        os: &'a mut Os,
        child: &'a mut ProcessHandle<Prepared, Os::Raw>,
        registered: &'a RegisteredLaunch<LaunchRecord>,
        launch_lease: &'a mut Option<
            pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
        >,
        lease_retirement: &'a mut pid1_owned_launch_lease::LeaseRetirementState,
        require_launch_lease: bool,
        wait_budget: std::time::Duration,
    ) -> Self {
        Self {
            os,
            child,
            registered,
            launch_lease,
            lease_retirement,
            require_launch_lease,
            wait_budget,
            attempted: false,
            duplicate_attempt: false,
            started: None,
            failure: None,
        }
    }

    fn commit(&mut self) -> Result<(), BackendError> {
        if self.require_launch_lease && self.launch_lease.is_none() {
            return Err(BackendError::ProtocolViolation);
        }
        if self.attempted {
            self.duplicate_attempt = true;
            return Err(BackendError::ProtocolViolation);
        }
        self.attempted = true;
        match self
            .os
            .commit_release_gate(self.child, self.registered, self.wait_budget)
        {
            Ok(started) => {
                if let Some(lease) = self.launch_lease.as_mut() {
                    lease.gate_disposition =
                        pid1_owned_launch_lease::LaunchGateDisposition::Released;
                }
                self.started = Some(started);
                Ok(())
            }
            Err(reason) => {
                if let Some(lease) = self.launch_lease.as_mut() {
                    lease.gate_disposition =
                        pid1_owned_launch_lease::LaunchGateDisposition::Unknown;
                }
                self.failure = Some(reason);
                #[cfg(test)]
                eprintln!("C3_PROCESS_GATE_RELEASE_FAILED: {reason:?}");
                Err(release_failure_backend_error(reason))
            }
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn install_and_retain_spawn_lease(
        &mut self,
        install: impl FnOnce(
            nagi_init_spawn_contract::SpawnedProcessPidfd,
            &LaunchRecord,
        ) -> Result<LaunchLease, BackendError>,
    ) -> Result<(), BackendError>
    where
        LaunchRecord: Copy + Eq,
        LaunchLease: pid1_owned_launch_lease::ExactChildLaunchLease<LaunchRecord>,
    {
        if self.attempted || self.launch_lease.is_some() {
            return Err(BackendError::ProtocolViolation);
        }
        let proof = self
            .os
            .duplicate_pidfd_for_prepared(self.child)
            .map_err(|_| BackendError::Unavailable)?;
        let process_instance = proof.identity();
        let record = self.registered.record;
        let lease = install(proof, &record)?;
        pid1_owned_launch_lease::adopt_exact_child_lease(
            self.launch_lease,
            self.lease_retirement,
            process_instance,
            record,
            lease,
        )?;
        Ok(())
    }

    /// Give a bounded C3/C2 pre-exec adapter an opaque duplicate of this exact
    /// still-gated child. It conveys process identity only; ProcessLifecycle
    /// remains the sole owner of wait and reap.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn duplicate_prepared_spawn_pidfd(
        &mut self,
    ) -> Result<nagi_init_spawn_contract::SpawnedProcessPidfd, BackendError> {
        self.os
            .duplicate_pidfd_for_prepared(self.child)
            .map_err(|_| BackendError::Unavailable)
    }
}

fn release_failure_backend_error(reason: ReleaseFailureReason) -> BackendError {
    match reason {
        ReleaseFailureReason::Cancelled => BackendError::Cancelled,
        ReleaseFailureReason::ProtocolViolation => BackendError::ProtocolViolation,
        _ => BackendError::TransportFailure,
    }
}

/// Borrowed executable bytes, normally obtained from `PackageView::executable()`.
/// This validates only the static x86_64 ELF shape. It does not assert a signature,
/// grants, consent, Session validity, or launch authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExecutablePayload<'a> {
    bytes: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutableFormatError {
    Truncated,
    InvalidHeader,
    NotStaticExecutable,
    WrongArchitecture,
    DynamicSegment,
    MalformedProgramHeader,
    MissingExecutableEntry,
}

impl<'a> ExecutablePayload<'a> {
    /// The caller must pass the exact slice returned by the already-authorized
    /// `PackageView::executable()` path. This constructor is not an auth check.
    fn from_package_executable(bytes: &'a [u8]) -> Result<Self, ExecutableFormatError> {
        validate_static_x86_64_elf(bytes)?;
        Ok(Self { bytes })
    }

    /// Accept a raw static ELF only in the explicitly development-only Linux
    /// service smoke. The caller must first validate the exact CPIO inputs;
    /// this constructor performs format validation, not package authorization.
    #[cfg(linux_dev_preview_smoke)]
    fn from_development_preview_elf(bytes: &'a [u8]) -> Result<Self, ExecutableFormatError> {
        validate_development_static_x86_64_elf(bytes)?;
        Ok(Self { bytes })
    }

    fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

fn elf_u16(bytes: &[u8], offset: usize) -> Result<u16, ExecutableFormatError> {
    let end = offset
        .checked_add(2)
        .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
    let word = bytes
        .get(offset..end)
        .ok_or(ExecutableFormatError::Truncated)?;
    Ok(u16::from_le_bytes([word[0], word[1]]))
}

fn elf_u32(bytes: &[u8], offset: usize) -> Result<u32, ExecutableFormatError> {
    let end = offset
        .checked_add(4)
        .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
    let word = bytes
        .get(offset..end)
        .ok_or(ExecutableFormatError::Truncated)?;
    Ok(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

fn elf_u64(bytes: &[u8], offset: usize) -> Result<u64, ExecutableFormatError> {
    let end = offset
        .checked_add(8)
        .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
    let word = bytes
        .get(offset..end)
        .ok_or(ExecutableFormatError::Truncated)?;
    Ok(u64::from_le_bytes([
        word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
    ]))
}

fn validate_static_x86_64_elf(bytes: &[u8]) -> Result<(), ExecutableFormatError> {
    validate_x86_64_elf(bytes, false)
}

/// Development service artifacts are pinned static PIE files. Their ET_DYN
/// form is accepted only through the explicit development smoke path.
#[cfg(linux_dev_preview_smoke)]
fn validate_development_static_x86_64_elf(bytes: &[u8]) -> Result<(), ExecutableFormatError> {
    validate_x86_64_elf(bytes, true)
}

fn validate_x86_64_elf(bytes: &[u8], allow_static_pie: bool) -> Result<(), ExecutableFormatError> {
    const ELF_HEADER_BYTES: usize = 64;
    const PROGRAM_HEADER_BYTES: usize = 56;
    const ET_EXEC: u16 = 2;
    const ET_DYN: u16 = 3;
    const EM_X86_64: u16 = 62;
    const PT_LOAD: u32 = 1;
    const PT_DYNAMIC: u32 = 2;
    const PT_INTERP: u32 = 3;
    const PF_X: u32 = 1;

    if bytes.len() < ELF_HEADER_BYTES {
        return Err(ExecutableFormatError::Truncated);
    }
    if &bytes[..4] != b"\x7fELF" || bytes[4] != 2 || bytes[5] != 1 || bytes[6] != 1 {
        return Err(ExecutableFormatError::InvalidHeader);
    }
    let object_type = elf_u16(bytes, 16)?;
    let static_pie = allow_static_pie && object_type == ET_DYN;
    if object_type != ET_EXEC && !static_pie {
        return Err(ExecutableFormatError::NotStaticExecutable);
    }
    if elf_u16(bytes, 18)? != EM_X86_64 {
        return Err(ExecutableFormatError::WrongArchitecture);
    }
    if elf_u32(bytes, 20)? != 1 || elf_u16(bytes, 52)? as usize != ELF_HEADER_BYTES {
        return Err(ExecutableFormatError::InvalidHeader);
    }

    let entry = elf_u64(bytes, 24)?;
    let program_offset = usize::try_from(elf_u64(bytes, 32)?)
        .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?;
    let program_entry_size = usize::from(elf_u16(bytes, 54)?);
    let program_count = usize::from(elf_u16(bytes, 56)?);
    if program_offset < ELF_HEADER_BYTES
        || program_entry_size != PROGRAM_HEADER_BYTES
        || program_count == 0
        || program_count == usize::from(u16::MAX)
    {
        return Err(ExecutableFormatError::MalformedProgramHeader);
    }
    let program_bytes = program_entry_size
        .checked_mul(program_count)
        .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
    let program_end = program_offset
        .checked_add(program_bytes)
        .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
    if program_end > bytes.len() {
        return Err(ExecutableFormatError::Truncated);
    }

    let mut entry_is_executable = false;
    let mut load_segments = Vec::new();
    let mut dynamic_segment = None;
    for index in 0..program_count {
        let offset = program_offset
            .checked_add(
                index
                    .checked_mul(program_entry_size)
                    .ok_or(ExecutableFormatError::MalformedProgramHeader)?,
            )
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        let kind = elf_u32(bytes, offset)?;
        if kind == PT_INTERP {
            return Err(ExecutableFormatError::DynamicSegment);
        }
        if kind == PT_DYNAMIC {
            if !static_pie || dynamic_segment.is_some() {
                return Err(ExecutableFormatError::DynamicSegment);
            }
            let file_offset = usize::try_from(elf_u64(bytes, offset + 8)?)
                .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?;
            let virtual_address = elf_u64(bytes, offset + 16)?;
            let file_size = usize::try_from(elf_u64(bytes, offset + 32)?)
                .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?;
            let memory_size = elf_u64(bytes, offset + 40)?;
            let file_end = file_offset
                .checked_add(file_size)
                .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
            if file_size == 0
                || file_size % 16 != 0
                || u64::try_from(file_size)
                    .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?
                    > memory_size
                || file_end > bytes.len()
            {
                return Err(ExecutableFormatError::MalformedProgramHeader);
            }
            dynamic_segment = Some((file_offset, virtual_address, file_size, memory_size));
            continue;
        }
        if kind != PT_LOAD {
            continue;
        }

        let flags = elf_u32(bytes, offset + 4)?;
        let file_offset = usize::try_from(elf_u64(bytes, offset + 8)?)
            .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?;
        let virtual_address = elf_u64(bytes, offset + 16)?;
        let file_size = elf_u64(bytes, offset + 32)?;
        let memory_size = elf_u64(bytes, offset + 40)?;
        let file_size_usize = usize::try_from(file_size)
            .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?;
        let file_end = file_offset
            .checked_add(file_size_usize)
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        let virtual_end = virtual_address
            .checked_add(file_size)
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        virtual_address
            .checked_add(memory_size)
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        if file_end > bytes.len() || file_size > memory_size {
            return Err(ExecutableFormatError::MalformedProgramHeader);
        }
        load_segments.push((file_offset, file_size_usize, virtual_address, memory_size));
        if flags & PF_X != 0 && entry >= virtual_address && entry < virtual_end {
            entry_is_executable = true;
        }
    }

    if let Some((dynamic_offset, dynamic_address, dynamic_size, dynamic_memory_size)) =
        dynamic_segment
    {
        let dynamic_file_end = dynamic_offset
            .checked_add(dynamic_size)
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        let dynamic_address_end = dynamic_address
            .checked_add(dynamic_memory_size)
            .ok_or(ExecutableFormatError::MalformedProgramHeader)?;
        let dynamic_is_load_mapped = load_segments.iter().any(
            |(load_offset, load_file_size, load_address, load_memory_size)| {
                let Some(load_file_end) = load_offset.checked_add(*load_file_size) else {
                    return false;
                };
                let Some(load_address_end) = load_address.checked_add(*load_memory_size) else {
                    return false;
                };
                let Some(file_delta) = dynamic_offset.checked_sub(*load_offset) else {
                    return false;
                };
                let Ok(file_delta) = u64::try_from(file_delta) else {
                    return false;
                };
                let Some(expected_address) = load_address.checked_add(file_delta) else {
                    return false;
                };
                dynamic_offset >= *load_offset
                    && dynamic_file_end <= load_file_end
                    && dynamic_address == expected_address
                    && dynamic_address_end <= load_address_end
            },
        );
        if !static_pie || !dynamic_is_load_mapped {
            return Err(ExecutableFormatError::MalformedProgramHeader);
        }

        let mut saw_terminator = false;
        for entry_offset in (dynamic_offset..dynamic_file_end).step_by(16) {
            let tag = i64::from_le_bytes(
                bytes[entry_offset..entry_offset + 8]
                    .try_into()
                    .map_err(|_| ExecutableFormatError::MalformedProgramHeader)?,
            );
            let value = elf_u64(bytes, entry_offset + 8)?;
            if tag == 0 {
                if value != 0 {
                    return Err(ExecutableFormatError::MalformedProgramHeader);
                }
                saw_terminator = true;
                continue;
            }
            if saw_terminator {
                return Err(ExecutableFormatError::MalformedProgramHeader);
            }
            if matches!(
                tag,
                1 | 15
                    | 29
                    | 0x6000_000d
                    | 0x6000_000f
                    | 0x6fff_fefa
                    | 0x6fff_fefb
                    | 0x6fff_fefc
                    | 0x7fff_fffd
                    | 0x7fff_ffff
            ) {
                return Err(ExecutableFormatError::DynamicSegment);
            }
        }
        if !saw_terminator {
            return Err(ExecutableFormatError::MalformedProgramHeader);
        }
    }

    if !entry_is_executable {
        return Err(ExecutableFormatError::MissingExecutableEntry);
    }
    Ok(())
}

/// Internal, injectable boundary for materializing immutable executable bytes.
/// It proves byte preservation only; it is not an authorization interface.
trait SealedExecutableOps {
    type Handle;

    fn create(&mut self) -> std::io::Result<Self::Handle>;
    fn write(&mut self, handle: &Self::Handle, bytes: &[u8]) -> std::io::Result<usize>;
    fn seal(&mut self, handle: &Self::Handle) -> std::io::Result<()>;
}

fn create_sealed_executable<O: SealedExecutableOps>(
    ops: &mut O,
    bytes: &[u8],
) -> std::io::Result<O::Handle> {
    if bytes.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty executable payload",
        ));
    }
    let handle = ops.create()?;
    let mut written = 0;
    while written < bytes.len() {
        match ops.write(&handle, &bytes[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "short executable payload write",
                ));
            }
            Ok(count) if count <= bytes.len() - written => written += count,
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "executable writer exceeded input length",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    ops.seal(&handle)?;
    Ok(handle)
}

trait ProcessOs {
    type Raw;

    fn prepare(
        &mut self,
        executable: ExecutablePayload<'_>,
        arguments: &[&str],
        execution_identity: LinuxExecutionIdentity,
        endpoint_policy: ProcessEndpointPolicy,
        filesystem_policy: ProcessFilesystemPolicy,
    ) -> std::io::Result<ProcessHandle<Prepared, Self::Raw>>;

    #[cfg(target_os = "linux")]
    fn take_handoff_resources(
        &mut self,
        _child: &mut ProcessHandle<Running, Self::Raw>,
    ) -> std::io::Result<ProcessEndpointResources> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "process owner does not expose an endpoint handoff",
        ))
    }

    #[cfg(target_os = "linux")]
    fn duplicate_pidfd_for_prepared(
        &mut self,
        _child: &ProcessHandle<Prepared, Self::Raw>,
    ) -> std::io::Result<nagi_init_spawn_contract::SpawnedProcessPidfd> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "process owner does not expose its prepared clone3 pidfd",
        ))
    }

    #[cfg(target_os = "linux")]
    fn duplicate_pidfd_for_running(
        &mut self,
        _child: &mut ProcessHandle<Running, Self::Raw>,
    ) -> std::io::Result<nagi_init_spawn_contract::SpawnedProcessPidfd> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "process owner does not expose its running clone3 pidfd",
        ))
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn take_fd_delegation_restriction_attestation(
        &mut self,
        _child: &mut ProcessHandle<Running, Self::Raw>,
    ) -> std::io::Result<Option<FdDelegationRestrictionAttestation>> {
        Ok(None)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn is_process_live(&self, _child: &ProcessHandle<Running, Self::Raw>) -> std::io::Result<bool> {
        Ok(false)
    }

    /// Commit only the nonblocking exec-gate byte. The caller performs this
    /// while the matching Supervisor record and grants are locked.
    fn commit_release_gate<LaunchRecord>(
        &mut self,
        child: &mut ProcessHandle<Prepared, Self::Raw>,
        registered: &RegisteredLaunch<LaunchRecord>,
        wait_budget: std::time::Duration,
    ) -> Result<std::time::Instant, ReleaseFailureReason>;

    /// Wait for exec status only after the registry lock has been released.
    fn wait_exec_ack(
        &mut self,
        child: ProcessHandle<ReleaseUncertain, Self::Raw>,
        started: std::time::Instant,
        wait_budget: std::time::Duration,
        cancel_check_interval: std::time::Duration,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> ExecAckResult<Self::Raw>;

    fn try_wait<Stage: ProcessStage>(
        &mut self,
        child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<Option<ServiceExitStatus>>;

    fn terminate<Stage: ProcessStage>(
        &mut self,
        child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<()>;
}

/// Opaque evidence that the exact clone3 child installed the C3 endpoint
/// delegation restriction after its one authorized endpoint transfer and
/// before exec. Only the Linux child owner can construct this value.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) struct FdDelegationRestrictionAttestation {
    process_identity: nagi_init_spawn_contract::SpawnedProcessIdentity,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl FdDelegationRestrictionAttestation {
    fn from_verified_child(
        process_identity: nagi_init_spawn_contract::SpawnedProcessIdentity,
    ) -> Self {
        Self { process_identity }
    }

    pub(super) fn matches_process(
        &self,
        process_identity: &nagi_init_spawn_contract::SpawnedProcessIdentity,
    ) -> bool {
        self.process_identity.same_instance(process_identity)
    }

    pub(super) fn process_identity(&self) -> &nagi_init_spawn_contract::SpawnedProcessIdentity {
        &self.process_identity
    }
}

/// Nonblocking source for the exec error pipe. The production implementation
/// uses poll/read plus wait4(WNOHANG); tests inject this boundary without
/// creating host processes.
trait ExecStatusWait {
    fn read_status_nonblocking(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;
    fn wait_readable(&mut self, timeout: std::time::Duration) -> std::io::Result<bool>;
    fn child_exit_status(&mut self) -> std::io::Result<Option<ServiceExitStatus>>;
}

fn wait_for_exec_ack(
    wait: &mut impl ExecStatusWait,
    budget: std::time::Duration,
    cancel_check_interval: std::time::Duration,
    elapsed: &mut dyn FnMut() -> std::time::Duration,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(), ReleaseFailureReason> {
    if cancel_check_interval.is_zero() {
        return Err(ReleaseFailureReason::Io);
    }
    let mut error_bytes = [0u8; 4];
    let mut received = 0usize;
    loop {
        if cancelled() {
            return Err(ReleaseFailureReason::Cancelled);
        }
        if elapsed() >= budget {
            return Err(ReleaseFailureReason::TimedOut);
        }

        match wait.read_status_nonblocking(&mut error_bytes[received..]) {
            Ok(0) => {
                if received != 0 {
                    return Err(ReleaseFailureReason::PartialErrorReport);
                }
                if cancelled() {
                    return Err(ReleaseFailureReason::Cancelled);
                }
                // CLOEXEC EOF only says the error writer closed. A wait4 probe
                // prevents an already-dead pre-exec child from being marked live.
                // Neither result is service READY; that requires the later
                // authenticated service handshake, which remains unconnected.
                match wait.child_exit_status() {
                    Ok(Some(status)) => return Err(ReleaseFailureReason::ChildExited(status)),
                    Ok(None) => {}
                    Err(_) => return Err(ReleaseFailureReason::Io),
                }
                // Under the child protocol, successful exec closes the
                // close-on-exec error writer; setup/exec errors write errno
                // and exit. EOF plus WNOHANG=0 acknowledges that exec
                // transition at this probe, not service READY or durable
                // liveness. Recheck caller conditions at the success boundary.
                if cancelled() {
                    return Err(ReleaseFailureReason::Cancelled);
                }
                if elapsed() >= budget {
                    return Err(ReleaseFailureReason::TimedOut);
                }
                return Ok(());
            }
            Ok(count) => {
                if count > error_bytes.len() - received {
                    return Err(ReleaseFailureReason::Io);
                }
                received += count;
                if received == error_bytes.len() {
                    return Err(ReleaseFailureReason::ExecError(i32::from_ne_bytes(
                        error_bytes,
                    )));
                }
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(ReleaseFailureReason::Io),
        }

        if cancelled() {
            return Err(ReleaseFailureReason::Cancelled);
        }
        let remaining = budget.saturating_sub(elapsed());
        if remaining.is_zero() {
            return Err(ReleaseFailureReason::TimedOut);
        }
        // Poll in caller-selected slices so cancellation is sampled during a
        // long release budget. This slice is required input, not a product
        // startup deadline or a hidden default.
        let poll_budget = if remaining < cancel_check_interval {
            remaining
        } else {
            cancel_check_interval
        };
        match wait.wait_readable(poll_budget) {
            Ok(true) => {}
            Ok(false) => {
                if elapsed() >= budget {
                    return Err(ReleaseFailureReason::TimedOut);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ReleaseFailureReason::Io),
        }
    }
}
struct ProcessLaunch<'a, ServiceId> {
    service: ServiceId,
    executable: ExecutablePayload<'a>,
    /// Arguments after argv[0]. The Linux child receives an empty environment.
    arguments: &'a [&'a str],
    /// Numeric Linux credentials for this process only; they do not create a
    /// Nagi identity or grant application consent.
    execution_identity: LinuxExecutionIdentity,
    /// Endpoint inheritance is opt-in and carries no service role by itself.
    endpoint_policy: ProcessEndpointPolicy,
    /// Application children use a private tmpfs root and, when authorized,
    /// exactly one Session-bound app-data root.
    filesystem_policy: ProcessFilesystemPolicy,
    required: bool,
    /// Caller-selected release deadline. There is intentionally no product default.
    release_wait_budget: std::time::Duration,
    /// Caller-selected maximum poll slice for cancellation checks.
    cancel_check_interval: std::time::Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessEndpointPolicy {
    Disabled,
    InheritedPeerChannel,
}

enum ProcessFilesystemPolicy {
    /// Preserve the current caller environment for internal process fixtures.
    Inherited,
    /// Give a development-only executable a private tmpfs root without Data.
    PrivateTmpfs,
    /// Bind only the exact C1-authorized app root into a private tmpfs root.
    Application(crate::profile_volume_notes_owner::AuthorizedAppStorageRoot),
}

#[cfg(target_os = "linux")]
struct ProcessEndpointResources {
    pidfd: nagi_init_spawn_contract::SpawnedProcessPidfd,
    endpoint: std::os::fd::OwnedFd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LinuxExecutionIdentity {
    uid: u32,
    gid: u32,
}

impl LinuxExecutionIdentity {
    const DEV_SERVICE: Self = Self {
        uid: DEV_SERVICE_UID,
        gid: DEV_SERVICE_GID,
    };
    const DEV_NOTES_DESKTOP: Self = Self {
        uid: DEV_NOTES_DESKTOP_UID,
        gid: DEV_NOTES_DESKTOP_GID,
    };

    const APP_FILES: Self = Self {
        uid: APP_FILES_UID,
        gid: APP_FILES_GID,
    };

    const APP_NOTES: Self = Self {
        uid: APP_NOTES_UID,
        gid: APP_NOTES_GID,
    };

    fn for_application(app_id: nagi_model::AppId) -> Option<Self> {
        if app_id == nagi_model::AppId::from_identifier(b"org.nagi.notes") {
            Some(Self::APP_NOTES)
        } else if app_id == nagi_model::AppId::from_identifier(b"com.nagi.notes") {
            Some(Self::DEV_NOTES_DESKTOP)
        } else if app_id == nagi_model::AppId::from_identifier(b"org.nagi.files") {
            Some(Self::APP_FILES)
        } else {
            None
        }
    }
}

fn inherit_child_console_output(execution_identity: LinuxExecutionIdentity) -> bool {
    #[cfg(linux_desktop_dev_smoke)]
    {
        execution_identity == LinuxExecutionIdentity::DEV_NOTES_DESKTOP
    }
    #[cfg(not(linux_desktop_dev_smoke))]
    {
        let _ = execution_identity;
        false
    }
}

fn process_argv_storage(arguments: &[&str]) -> std::io::Result<Vec<CString>> {
    let mut storage = Vec::with_capacity(arguments.len() + 1);
    storage.push(CString::new("nagi-service").expect("fixed argv[0] has no NUL"));
    for argument in arguments {
        storage.push(CString::new(*argument).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "process argument contains NUL",
            )
        })?);
    }
    Ok(storage)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchLeaseActivationState {
    NotAttempted,
    Active,
    Unknown,
}

struct ManagedProcess<OsChild, ServiceId, LaunchRecord, LaunchLease = ()> {
    process_id: u32,
    service: ServiceId,
    required: bool,
    child: OsChild,
    record: LaunchRecord,
    reaped: Option<ServiceExitStatus>,
    launch_lease: Option<pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_retirement: pid1_owned_launch_lease::LeaseRetirementState,
    lease_activation: LaunchLeaseActivationState,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fd_delegation_attestation: Option<FdDelegationRestrictionAttestation>,
}

enum PendingChild<Raw> {
    Prepared(ProcessHandle<Prepared, Raw>),
    ReleaseUncertain(ProcessHandle<ReleaseUncertain, Raw>),
    Running(ProcessHandle<Running, Raw>),
}

struct QuarantinedProcess<Raw, LaunchRecord, LaunchLease = ()> {
    process_id: u32,
    child: PendingChild<Raw>,
    record: Option<LaunchRecord>,
    reaped: Option<ServiceExitStatus>,
    launch_lease: Option<pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_retirement: pid1_owned_launch_lease::LeaseRetirementState,
    lease_activation: LaunchLeaseActivationState,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fd_delegation_attestation: Option<FdDelegationRestrictionAttestation>,
}

type RunningChild<Raw, ServiceId, LaunchRecord, LaunchLease> =
    ManagedProcess<ProcessHandle<Running, Raw>, ServiceId, LaunchRecord, LaunchLease>;

struct ProcessLifecycle<Os: ProcessOs, ServiceId, LaunchRecord, LaunchLease = ()> {
    os: Os,
    children: Vec<RunningChild<Os::Raw, ServiceId, LaunchRecord, LaunchLease>>,
    /// Handles awaiting cleanup/revocation are hidden from live_record and block
    /// PID reuse until both obligations have completed.
    quarantine: Vec<QuarantinedProcess<Os::Raw, LaunchRecord, LaunchLease>>,
}

impl<Os: ProcessOs, ServiceId, LaunchRecord> ProcessLifecycle<Os, ServiceId, LaunchRecord, ()> {
    fn new(os: Os) -> Self {
        Self {
            os,
            children: Vec::new(),
            quarantine: Vec::new(),
        }
    }
}

impl<Os: ProcessOs, ServiceId, LaunchRecord, LaunchLease>
    ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>
{
    fn new_with_launch_lease(os: Os) -> Self {
        Self {
            os,
            children: Vec::new(),
            quarantine: Vec::new(),
        }
    }
}

impl<Os, ServiceId, LaunchRecord, LaunchLease>
    ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy + Eq,
{
    /// Test-only convenience shim for orchestration tests; production uses the
    /// authorization callback in spawn_authorized.
    #[cfg(test)]
    fn spawn(
        &mut self,
        launch: ProcessLaunch<'_, ServiceId>,
        cancelled: impl FnMut() -> bool,
        register: impl FnOnce(u32) -> Result<LaunchRecord, BackendError>,
        revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        self.spawn_authorized(
            launch,
            cancelled,
            register,
            |_, _, gate| gate.commit(),
            revoke,
        )
    }

    /// Prepare and register a child, authorize the now-live Supervisor record,
    /// then release the exec gate. `register` must leave no record on error;
    /// `authorize` runs after a record exists and on error keeps that exact
    /// record attached to cleanup/quarantine. No child executes on rejection.
    fn spawn_authorized(
        &mut self,
        launch: ProcessLaunch<'_, ServiceId>,
        mut cancelled: impl FnMut() -> bool,
        register: impl FnOnce(u32) -> Result<LaunchRecord, BackendError>,
        authorize: impl FnOnce(
            u32,
            &LaunchRecord,
            &mut GateCommit<'_, Os, LaunchRecord, LaunchLease>,
        ) -> Result<(), BackendError>,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        self.spawn_authorized_with_prepared(
            launch,
            &mut cancelled,
            |process_id, _, _| register(process_id),
            authorize,
            &mut revoke,
        )
    }

    /// Variant for the maintained C3 owner. Registration sees the actual
    /// prepared child and its ProcessOs owner before the exec gate is released,
    /// so it can bind the slot record to clone3's atomic pidfd rather than to
    /// a numeric PID claim.
    fn spawn_authorized_with_prepared(
        &mut self,
        launch: ProcessLaunch<'_, ServiceId>,
        cancelled: impl FnMut() -> bool,
        register: impl FnOnce(
            u32,
            &mut Os,
            &ProcessHandle<Prepared, Os::Raw>,
        ) -> Result<LaunchRecord, BackendError>,
        authorize: impl FnOnce(
            u32,
            &LaunchRecord,
            &mut GateCommit<'_, Os, LaunchRecord, LaunchLease>,
        ) -> Result<(), BackendError>,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        self.spawn_authorized_with_prepared_inner(
            launch,
            cancelled,
            register,
            authorize,
            &mut revoke,
            false,
            |_, _| Err(BackendError::ProtocolViolation),
        )
    }

    // Keep exact launch, authorization, and cleanup callbacks explicit.
    #[allow(clippy::too_many_arguments)]
    fn spawn_authorized_with_prepared_inner(
        &mut self,
        launch: ProcessLaunch<'_, ServiceId>,
        mut cancelled: impl FnMut() -> bool,
        register: impl FnOnce(
            u32,
            &mut Os,
            &ProcessHandle<Prepared, Os::Raw>,
        ) -> Result<LaunchRecord, BackendError>,
        authorize: impl FnOnce(
            u32,
            &LaunchRecord,
            &mut GateCommit<'_, Os, LaunchRecord, LaunchLease>,
        ) -> Result<(), BackendError>,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        require_launch_lease: bool,
        mut retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        if cancelled() {
            return Err(BackendError::Cancelled);
        }

        let mut prepared = self
            .os
            .prepare(
                launch.executable,
                launch.arguments,
                launch.execution_identity,
                launch.endpoint_policy,
                launch.filesystem_policy,
            )
            .inspect_err(|_error| {
                #[cfg(test)]
                eprintln!(
                    "C3_PROCESS_PREPARE_FAILED: kind={:?} errno={:?} error={_error}",
                    _error.kind(),
                    _error.raw_os_error()
                );
            })
            .map_err(|_| BackendError::TransportFailure)?;
        let process_id = prepared.process_id;
        let mut acquiring_lease = None;
        let mut lease_retirement = pid1_owned_launch_lease::LeaseRetirementState::NotAttempted;
        let mut lease_activation = LaunchLeaseActivationState::NotAttempted;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let mut fd_delegation_attestation = None;
        if process_id <= 1 {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                acquiring_lease,
                lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
                &mut revoke,
                &mut retire_lease,
            )?;
            return Err(BackendError::ProtocolViolation);
        }
        if self
            .children
            .iter()
            .any(|entry| entry.process_id == process_id)
            || self
                .quarantine
                .iter()
                .any(|entry| entry.process_id == process_id)
        {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                acquiring_lease,
                lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
                &mut revoke,
                &mut retire_lease,
            )?;
            return Err(BackendError::ProtocolViolation);
        }
        if cancelled() {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                acquiring_lease,
                lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
                &mut revoke,
                &mut retire_lease,
            )?;
            return Err(BackendError::Cancelled);
        }

        let record = match register(process_id, &mut self.os, &prepared) {
            Ok(record) => record,
            Err(error) => {
                self.discard_pending(
                    process_id,
                    PendingChild::Prepared(prepared),
                    None,
                    None,
                    acquiring_lease,
                    lease_retirement,
                    lease_activation,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    fd_delegation_attestation,
                    &mut revoke,
                    &mut retire_lease,
                )?;
                return Err(error);
            }
        };
        // Cancellation after registration revokes that record and cleans the
        // still-gated child before any grant check. It never calls release.
        if cancelled() {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                Some(record),
                None,
                acquiring_lease,
                lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
                &mut revoke,
                &mut retire_lease,
            )?;
            return Err(BackendError::Cancelled);
        }

        let registered = RegisteredLaunch { record };
        let (authorization, gate_attempted, duplicate_gate_attempt, release_started, gate_failure) = {
            let mut gate_commit = GateCommit::new(
                &mut self.os,
                &mut prepared,
                &registered,
                &mut acquiring_lease,
                &mut lease_retirement,
                require_launch_lease,
                launch.release_wait_budget,
            );
            let authorization = authorize(process_id, &record, &mut gate_commit);
            (
                authorization,
                gate_commit.attempted,
                gate_commit.duplicate_attempt,
                gate_commit.started,
                gate_commit.failure,
            )
        };

        let authorization_error = match authorization {
            Err(error) => Some(error),
            Ok(()) if duplicate_gate_attempt => Some(BackendError::ProtocolViolation),
            Ok(()) if !gate_attempted => Some(BackendError::ProtocolViolation),
            Ok(()) if release_started.is_none() => Some(
                gate_failure
                    .map(release_failure_backend_error)
                    .unwrap_or(BackendError::ProtocolViolation),
            ),
            Ok(()) => None,
        };
        if let Some(error) = authorization_error {
            let pending = if gate_attempted {
                PendingChild::ReleaseUncertain(prepared.transition())
            } else {
                PendingChild::Prepared(prepared)
            };
            self.discard_pending(
                process_id,
                pending,
                Some(registered.into_record()),
                None,
                acquiring_lease,
                lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
                &mut revoke,
                &mut retire_lease,
            )?;
            return Err(error);
        }

        let started = release_started.expect("successful gate commit has a start time");
        match self.os.wait_exec_ack(
            prepared.transition(),
            started,
            launch.release_wait_budget,
            launch.cancel_check_interval,
            &mut cancelled,
        ) {
            Ok(mut child) => {
                let record = registered.into_record();
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                if require_launch_lease {
                    let attestation = self
                        .os
                        .take_fd_delegation_restriction_attestation(&mut child)
                        .ok()
                        .flatten();
                    let exact_attestation = attestation.filter(|attestation| {
                        acquiring_lease.as_ref().is_some_and(|lease| {
                            attestation.matches_process(&lease.process_instance)
                                && lease.gate_disposition
                                    == pid1_owned_launch_lease::LaunchGateDisposition::Released
                        })
                    });
                    if exact_attestation.is_none() {
                        lease_activation = LaunchLeaseActivationState::Unknown;
                        self.discard_pending(
                            process_id,
                            PendingChild::Running(child),
                            Some(record),
                            None,
                            acquiring_lease,
                            lease_retirement,
                            lease_activation,
                            fd_delegation_attestation,
                            &mut revoke,
                            &mut retire_lease,
                        )?;
                        return Err(BackendError::ProtocolViolation);
                    }
                    fd_delegation_attestation = exact_attestation;
                }
                #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
                if require_launch_lease {
                    return Err(BackendError::ProtocolViolation);
                }
                self.children.push(ManagedProcess {
                    process_id,
                    service: launch.service,
                    required: launch.required,
                    child,
                    record,
                    reaped: None,
                    launch_lease: acquiring_lease,
                    lease_retirement,
                    lease_activation,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    fd_delegation_attestation,
                });
                Ok(process_id)
            }
            Err(failure) => {
                // The gate byte was committed. A cancellation, timeout, or
                // status-channel failure leaves execution uncertain, so reap
                // and revoke before reporting failure.
                let failure_error = release_failure_backend_error(failure.reason);
                self.discard_pending(
                    process_id,
                    PendingChild::ReleaseUncertain(failure.child),
                    Some(registered.into_record()),
                    failure.reaped,
                    acquiring_lease,
                    lease_retirement,
                    lease_activation,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    fd_delegation_attestation,
                    &mut revoke,
                    &mut retire_lease,
                )?;
                Err(failure_error)
            }
        }
    }

    /// Retry cleanup/revocation for quarantined launches before reporting a
    /// managed service exit. Quarantine never creates an IPC-visible record.
    fn poll(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<Option<LifecycleEvent<ServiceId>>, BackendError> {
        self.poll_with_launch_lease_inner(&mut revoke, |_, _| Err(BackendError::ProtocolViolation))
    }

    fn poll_with_launch_lease_inner(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        mut retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<Option<LifecycleEvent<ServiceId>>, BackendError> {
        let mut index = 0;
        let mut cleanup_failed = false;
        while index < self.quarantine.len() {
            let entry = &mut self.quarantine[index];
            let child_clean =
                Self::attempt_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped);
            let record_clean = if child_clean {
                Self::retry_record_revocation(entry.process_id, &mut entry.record, &mut revoke)
            } else {
                false
            };
            let lease_clean = if child_clean && record_clean {
                Self::retire_launch_lease(
                    &mut entry.launch_lease,
                    &mut entry.lease_retirement,
                    entry.reaped,
                    &mut retire_lease,
                )
            } else {
                false
            };
            if child_clean && record_clean && lease_clean {
                self.quarantine.remove(index);
            } else {
                cleanup_failed = true;
                index += 1;
            }
        }
        if cleanup_failed {
            return Err(BackendError::TransportFailure);
        }

        for index in 0..self.children.len() {
            if self.children[index].reaped.is_none() {
                let status = self
                    .os
                    .try_wait(&mut self.children[index].child)
                    .map_err(|_| BackendError::TransportFailure)?;
                if let Some(status) = status {
                    self.children[index].reaped = Some(status);
                }
            }

            let Some(status) = self.children[index].reaped else {
                continue;
            };
            let process_id = self.children[index].process_id;
            let record = &self.children[index].record;
            revoke(process_id, record)?;
            let child = self.children.remove(index);
            let ManagedProcess {
                process_id,
                service,
                required,
                child,
                record: _,
                reaped: _,
                mut launch_lease,
                mut lease_retirement,
                lease_activation,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                fd_delegation_attestation,
            } = child;
            if !Self::retire_launch_lease(
                &mut launch_lease,
                &mut lease_retirement,
                Some(status),
                &mut retire_lease,
            ) {
                self.quarantine.push(QuarantinedProcess {
                    process_id,
                    child: PendingChild::Running(child),
                    record: None,
                    reaped: Some(status),
                    launch_lease,
                    lease_retirement,
                    lease_activation,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    fd_delegation_attestation,
                });
                return Err(BackendError::TransportFailure);
            }
            return Ok(Some(LifecycleEvent::ServiceExited {
                service,
                status,
                required,
            }));
        }
        Ok(None)
    }

    /// A PID is not an identity. Only a released, unreaped process with a live
    /// registered record can be resolved here.
    fn live_record(&self, process_id: u32) -> Option<&LaunchRecord> {
        self.children
            .iter()
            .find(|entry| entry.process_id == process_id && entry.reaped.is_none())
            .map(|entry| &entry.record)
    }

    /// Transfer the endpoint exactly once, and only from the live child whose
    /// current launch record exactly matches the caller's registered record.
    /// The lifecycle keeps its original pidfd; the recipient gets a duplicate
    /// for the same opaque clone3 identity and takes ownership of the endpoint.
    #[cfg(target_os = "linux")]
    fn take_live_endpoint(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
    ) -> std::io::Result<ProcessEndpointResources>
    where
        LaunchRecord: Eq,
    {
        let entry = self
            .children
            .iter_mut()
            .find(|entry| {
                entry.process_id == process_id
                    && entry.reaped.is_none()
                    && entry.record == expected_record
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "process or exact launch record is not live",
                )
            })?;
        if let Some(status) = self.os.try_wait(&mut entry.child)? {
            entry.reaped = Some(status);
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "process exited before endpoint handoff",
            ));
        }
        self.os.take_handoff_resources(&mut entry.child)
    }

    #[cfg(target_os = "linux")]
    fn duplicate_live_pidfd(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
    ) -> std::io::Result<nagi_init_spawn_contract::SpawnedProcessPidfd>
    where
        LaunchRecord: Eq,
    {
        let entry = self
            .children
            .iter_mut()
            .find(|entry| {
                entry.process_id == process_id
                    && entry.reaped.is_none()
                    && entry.record == expected_record
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "process or exact launch record is not live",
                )
            })?;
        if let Some(status) = self.os.try_wait(&mut entry.child)? {
            entry.reaped = Some(status);
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "process exited before pidfd duplication",
            ));
        }
        self.os.duplicate_pidfd_for_running(&mut entry.child)
    }

    /// Take only the restriction evidence reported by the exact live child
    /// after its final pre-exec seccomp filter was installed and read back.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn take_live_fd_delegation_restriction_attestation(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
    ) -> std::io::Result<FdDelegationRestrictionAttestation>
    where
        LaunchRecord: Eq,
    {
        let entry = self
            .children
            .iter_mut()
            .find(|entry| {
                entry.process_id == process_id
                    && entry.reaped.is_none()
                    && entry.record == expected_record
            })
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "process or exact launch record is not live",
                )
            })?;
        if let Some(status) = self.os.try_wait(&mut entry.child)? {
            entry.reaped = Some(status);
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "process exited before delegation restriction evidence was retained",
            ));
        }
        let attestation = self
            .os
            .take_fd_delegation_restriction_attestation(&mut entry.child)?
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "live child has no verified FD delegation restriction",
                )
            })?;
        if attestation.process_identity.process_id() != process_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "FD delegation evidence belongs to a different clone3 child",
            ));
        }
        Ok(attestation)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn is_live_record(
        &self,
        process_id: u32,
        expected_record: LaunchRecord,
    ) -> std::io::Result<bool>
    where
        LaunchRecord: Eq,
    {
        let Some(entry) = self.children.iter().find(|entry| {
            entry.process_id == process_id
                && entry.reaped.is_none()
                && entry.record == expected_record
        }) else {
            return Ok(false);
        };
        self.os.is_process_live(&entry.child)
    }

    /// Stop exactly one process whose retained launch token still matches.
    /// Unreaped children and their exact record remain quarantined for the
    /// existing single-owner reaper; PID reuse cannot satisfy this selector.
    #[cfg(target_os = "linux")]
    fn terminate_exact(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<(), BackendError>
    where
        LaunchRecord: Eq,
    {
        self.terminate_exact_with_launch_lease_inner(
            process_id,
            expected_record,
            &mut revoke,
            |_, _| Err(BackendError::ProtocolViolation),
        )
    }

    #[cfg(target_os = "linux")]
    fn terminate_exact_with_launch_lease_inner(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        mut retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError>
    where
        LaunchRecord: Eq,
    {
        let Some(index) = self
            .children
            .iter()
            .position(|entry| entry.process_id == process_id && entry.record == expected_record)
        else {
            return Err(BackendError::Revoked);
        };
        let entry = self.children.remove(index);
        let mut child = PendingChild::Running(entry.child);
        let mut reaped = entry.reaped;
        let child_clean = Self::attempt_cleanup(&mut self.os, &mut child, &mut reaped);
        let mut record = Some(entry.record);
        let record_clean = if child_clean {
            Self::retry_record_revocation(process_id, &mut record, &mut revoke)
        } else {
            false
        };
        let mut launch_lease = entry.launch_lease;
        let mut lease_retirement = entry.lease_retirement;
        let lease_activation = entry.lease_activation;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let fd_delegation_attestation = entry.fd_delegation_attestation;
        let lease_clean = if child_clean && record_clean {
            Self::retire_launch_lease(
                &mut launch_lease,
                &mut lease_retirement,
                reaped,
                &mut retire_lease,
            )
        } else {
            false
        };
        if child_clean && record_clean && lease_clean {
            return Ok(());
        }
        self.quarantine.push(QuarantinedProcess {
            process_id,
            child,
            record,
            reaped,
            launch_lease,
            lease_retirement,
            lease_activation,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            fd_delegation_attestation,
        });
        Err(BackendError::TransportFailure)
    }

    fn active_len(&self) -> usize {
        self.children.len() + self.quarantine.len()
    }

    fn tracks_record(&self, expected_record: &LaunchRecord) -> bool
    where
        LaunchRecord: Eq,
    {
        self.children
            .iter()
            .any(|entry| &entry.record == expected_record)
            || self.quarantine.iter().any(|entry| {
                entry
                    .record
                    .as_ref()
                    .is_some_and(|record| record == expected_record)
            })
    }

    fn shutdown(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        self.shutdown_with_launch_lease_inner(&mut revoke, |_, _| {
            Err(BackendError::ProtocolViolation)
        })
    }

    fn shutdown_with_launch_lease_inner(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        mut retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        // Each Linux service is the init process of its own PID namespace.
        // Killing a tracked service therefore terminates its remaining namespace
        // members. Never signal untracked processes in PID 1's namespace.
        let mut failed = false;
        for entry in &mut self.children {
            if !Self::attempt_running_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped) {
                failed = true;
            }
        }

        let mut index = 0;
        while index < self.quarantine.len() {
            let entry = &mut self.quarantine[index];
            let child_clean =
                Self::attempt_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped);
            let record_clean = if child_clean {
                Self::retry_record_revocation(entry.process_id, &mut entry.record, &mut revoke)
            } else {
                false
            };
            let lease_clean = if child_clean && record_clean {
                Self::retire_launch_lease(
                    &mut entry.launch_lease,
                    &mut entry.lease_retirement,
                    entry.reaped,
                    &mut retire_lease,
                )
            } else {
                false
            };
            if child_clean && record_clean && lease_clean {
                self.quarantine.remove(index);
            } else {
                failed = true;
                index += 1;
            }
        }

        // Any child not reaped in this bounded pass moves intact into quarantine.
        // A reaped child has its exact record revoked before lease retirement.
        let mut index = 0;
        while index < self.children.len() {
            if self.children[index].reaped.is_none() {
                let entry = self.children.remove(index);
                self.quarantine.push(QuarantinedProcess {
                    process_id: entry.process_id,
                    child: PendingChild::Running(entry.child),
                    record: Some(entry.record),
                    reaped: entry.reaped,
                    launch_lease: entry.launch_lease,
                    lease_retirement: entry.lease_retirement,
                    lease_activation: entry.lease_activation,
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    fd_delegation_attestation: entry.fd_delegation_attestation,
                });
                failed = true;
            } else {
                let process_id = self.children[index].process_id;
                let record = &self.children[index].record;
                if revoke(process_id, record).is_err() {
                    failed = true;
                    index += 1;
                    continue;
                }
                let entry = self.children.remove(index);
                let mut launch_lease = entry.launch_lease;
                let mut lease_retirement = entry.lease_retirement;
                let lease_clean = Self::retire_launch_lease(
                    &mut launch_lease,
                    &mut lease_retirement,
                    entry.reaped,
                    &mut retire_lease,
                );
                if !lease_clean {
                    self.quarantine.push(QuarantinedProcess {
                        process_id,
                        child: PendingChild::Running(entry.child),
                        record: None,
                        reaped: entry.reaped,
                        launch_lease,
                        lease_retirement,
                        lease_activation: entry.lease_activation,
                        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                        fd_delegation_attestation: entry.fd_delegation_attestation,
                    });
                    failed = true;
                }
            }
        }

        if failed || !self.children.is_empty() || !self.quarantine.is_empty() {
            Err(BackendError::TransportFailure)
        } else {
            Ok(())
        }
    }

    // Keep the reap, revocation, and lease obligations explicit for this path.
    #[allow(clippy::too_many_arguments)]
    fn discard_pending(
        &mut self,
        process_id: u32,
        mut child: PendingChild<Os::Raw>,
        mut record: Option<LaunchRecord>,
        already_reaped: Option<ServiceExitStatus>,
        mut launch_lease: Option<
            pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
        >,
        mut lease_retirement: pid1_owned_launch_lease::LeaseRetirementState,
        lease_activation: LaunchLeaseActivationState,
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))] fd_delegation_attestation: Option<
            FdDelegationRestrictionAttestation,
        >,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        retire_lease: &mut impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let mut reaped = already_reaped;
        let child_clean = Self::attempt_cleanup(&mut self.os, &mut child, &mut reaped);
        let record_clean = if child_clean {
            Self::retry_record_revocation(process_id, &mut record, revoke)
        } else {
            false
        };
        let lease_clean = if child_clean && record_clean {
            Self::retire_launch_lease(
                &mut launch_lease,
                &mut lease_retirement,
                reaped,
                retire_lease,
            )
        } else {
            false
        };
        if child_clean && record_clean && lease_clean {
            return Ok(());
        }

        // Keep the OS handle, unrevoked record, and non-Clone lease together
        // until exact wait/reap, record revoke, and lease retirement all finish.
        self.quarantine.push(QuarantinedProcess {
            process_id,
            child,
            record,
            reaped,
            launch_lease,
            lease_retirement,
            lease_activation,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            fd_delegation_attestation,
        });
        Err(BackendError::TransportFailure)
    }

    fn retire_launch_lease(
        launch_lease: &mut Option<
            pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
        >,
        retirement: &mut pid1_owned_launch_lease::LeaseRetirementState,
        reaped: Option<ServiceExitStatus>,
        retire: &mut impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> bool {
        let Some(bound) = launch_lease.as_ref() else {
            return match *retirement {
                pid1_owned_launch_lease::LeaseRetirementState::NotAttempted => {
                    *retirement = pid1_owned_launch_lease::LeaseRetirementState::Complete;
                    true
                }
                pid1_owned_launch_lease::LeaseRetirementState::Complete => true,
                pid1_owned_launch_lease::LeaseRetirementState::Attempted
                | pid1_owned_launch_lease::LeaseRetirementState::Unknown => false,
            };
        };
        let Some(status) = reaped else {
            return false;
        };
        let expected_instance = bound.process_instance.clone();
        let expected_record = bound.record;
        pid1_owned_launch_lease::retire_exact_once(
            launch_lease,
            retirement,
            &expected_instance,
            expected_record,
            status,
            retire,
        )
    }

    fn retry_record_revocation(
        process_id: u32,
        record: &mut Option<LaunchRecord>,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> bool {
        let Some(current) = record.as_ref() else {
            return true;
        };
        if revoke(process_id, current).is_ok() {
            *record = None;
            true
        } else {
            false
        }
    }

    fn attempt_cleanup(
        os: &mut Os,
        child: &mut PendingChild<Os::Raw>,
        reaped: &mut Option<ServiceExitStatus>,
    ) -> bool {
        match child {
            PendingChild::Prepared(child) => Self::attempt_phase_cleanup(os, child, reaped),
            PendingChild::ReleaseUncertain(child) => Self::attempt_phase_cleanup(os, child, reaped),
            PendingChild::Running(child) => Self::attempt_running_cleanup(os, child, reaped),
        }
    }

    fn attempt_running_cleanup(
        os: &mut Os,
        child: &mut ProcessHandle<Running, Os::Raw>,
        reaped: &mut Option<ServiceExitStatus>,
    ) -> bool {
        Self::attempt_phase_cleanup(os, child, reaped)
    }

    fn attempt_phase_cleanup<Stage: ProcessStage>(
        os: &mut Os,
        child: &mut ProcessHandle<Stage, Os::Raw>,
        reaped: &mut Option<ServiceExitStatus>,
    ) -> bool {
        if reaped.is_some() {
            return true;
        }

        if let Ok(Some(status)) = os.try_wait(child) {
            *reaped = Some(status);
            return true;
        }

        // SIGKILL is asynchronous. Never block waiting for an uninterruptible
        // child; keep its handle quarantined until a later WNOHANG reap succeeds.
        let _ = os.terminate(child);
        Self::observe_exit(os, child, reaped)
    }

    fn observe_exit<Stage: ProcessStage>(
        os: &mut Os,
        child: &mut ProcessHandle<Stage, Os::Raw>,
        reaped: &mut Option<ServiceExitStatus>,
    ) -> bool {
        match os.try_wait(child) {
            Ok(Some(status)) => {
                *reaped = Some(status);
                true
            }
            Ok(None) | Err(_) => false,
        }
    }
}

impl<Os, ServiceId, LaunchRecord, LaunchLease>
    ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy + Eq,
    LaunchLease: pid1_owned_launch_lease::ExactChildLaunchLease<LaunchRecord>,
{
    /// Task24 entry point: the selected production launch path cannot release
    /// the exec gate until its exact-child lease has been adopted into this
    /// ProcessLifecycle and provides the retirement callback for failures.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn spawn_authorized_with_prepared_and_launch_lease(
        &mut self,
        launch: ProcessLaunch<'_, ServiceId>,
        cancelled: impl FnMut() -> bool,
        register: impl FnOnce(
            u32,
            &mut Os,
            &ProcessHandle<Prepared, Os::Raw>,
        ) -> Result<LaunchRecord, BackendError>,
        authorize: impl FnOnce(
            u32,
            &LaunchRecord,
            &mut GateCommit<'_, Os, LaunchRecord, LaunchLease>,
        ) -> Result<(), BackendError>,
        revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        self.spawn_authorized_with_prepared_inner(
            launch,
            cancelled,
            register,
            authorize,
            revoke,
            true,
            retire_lease,
        )
    }

    /// Activate the pending lease only after exec acknowledgement, while the
    /// same exact child remains live and the C3 FD restriction attestation is
    /// still held in its lifecycle slot.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn activate_current_launch_lease(
        &mut self,
        proof: &nagi_init_spawn_contract::SpawnedProcessPidfd,
        record: LaunchRecord,
        activate: impl FnOnce(
            &mut LaunchLease,
            &FdDelegationRestrictionAttestation,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let identity = proof.identity();
        let process_id = proof.process_id();
        let entry = self
            .children
            .iter_mut()
            .find(|entry| entry.process_id == process_id && entry.record == record)
            .ok_or(BackendError::IdentityRejected)?;
        if entry.reaped.is_some()
            || entry.lease_activation != LaunchLeaseActivationState::NotAttempted
            || !self
                .os
                .is_process_live(&entry.child)
                .map_err(|_| BackendError::TransportFailure)?
        {
            return Err(BackendError::Revoked);
        }
        let bound = entry
            .launch_lease
            .as_mut()
            .ok_or(BackendError::IdentityRejected)?;
        let attestation = entry
            .fd_delegation_attestation
            .as_ref()
            .ok_or(BackendError::IdentityRejected)?;
        let lease_matches = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bound.lease.matches_spawn(&identity, &record)
        }))
        .unwrap_or(false);
        if !bound.process_instance.same_instance(&identity)
            || bound.record != record
            || bound.gate_disposition != pid1_owned_launch_lease::LaunchGateDisposition::Released
            || !lease_matches
            || !attestation.matches_process(&identity)
        {
            return Err(BackendError::IdentityRejected);
        }

        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            activate(&mut bound.lease, attestation)
        })) {
            Ok(Ok(())) => {
                entry.lease_activation = LaunchLeaseActivationState::Active;
                Ok(())
            }
            Ok(Err(error)) => {
                entry.lease_activation = LaunchLeaseActivationState::Unknown;
                Err(error)
            }
            Err(_) => {
                entry.lease_activation = LaunchLeaseActivationState::Unknown;
                Err(BackendError::TransportFailure)
            }
        }
    }

    /// Return a lease borrowed from the current child slot only while the same
    /// clone3 process, record, FD attestation, and live pidfd all still match.
    /// Holding this reference prevents mutable poll/reap/replacement until the
    /// caller completes the initial checkout.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn borrow_current_launch_lease<'a>(
        &'a self,
        proof: &nagi_init_spawn_contract::SpawnedProcessPidfd,
        record: LaunchRecord,
    ) -> Result<&'a LaunchLease, BackendError> {
        let identity = proof.identity();
        let process_id = proof.process_id();
        let entry = self
            .children
            .iter()
            .find(|entry| entry.process_id == process_id && entry.record == record)
            .ok_or(BackendError::IdentityRejected)?;
        if entry.reaped.is_some()
            || entry.lease_activation != LaunchLeaseActivationState::Active
            || !self
                .os
                .is_process_live(&entry.child)
                .map_err(|_| BackendError::TransportFailure)?
        {
            return Err(BackendError::Revoked);
        }
        let bound = entry
            .launch_lease
            .as_ref()
            .ok_or(BackendError::IdentityRejected)?;
        let attestation = entry
            .fd_delegation_attestation
            .as_ref()
            .ok_or(BackendError::IdentityRejected)?;
        let lease_matches = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bound.lease.matches_spawn(&identity, &record)
        }))
        .unwrap_or(false);
        if !bound.process_instance.same_instance(&identity)
            || bound.record != record
            || bound.gate_disposition != pid1_owned_launch_lease::LaunchGateDisposition::Released
            || !lease_matches
            || !attestation.matches_process(&identity)
        {
            return Err(BackendError::IdentityRejected);
        }
        Ok(&bound.lease)
    }

    fn poll_with_launch_lease(
        &mut self,
        revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<Option<LifecycleEvent<ServiceId>>, BackendError> {
        self.poll_with_launch_lease_inner(revoke, retire_lease)
    }

    fn shutdown_with_launch_lease(
        &mut self,
        revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        self.shutdown_with_launch_lease_inner(revoke, retire_lease)
    }

    #[cfg(target_os = "linux")]
    fn terminate_exact_with_launch_lease(
        &mut self,
        process_id: u32,
        expected_record: LaunchRecord,
        revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        retire_lease: impl FnMut(
            &mut pid1_owned_launch_lease::BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        self.terminate_exact_with_launch_lease_inner(
            process_id,
            expected_record,
            revoke,
            retire_lease,
        )
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux_x86_64_process {
    use super::pid1_child_reaper::{
        drain_serialized, Pid1ChildReaper, ReapKernel, ReapTicket, ReapTicketReservation,
    };
    use super::*;
    use core::arch::asm;
    use nagi_init_spawn_contract::{
        clone3_pid1_service_child, Clone3Pid1Child, SpawnedProcessPidfd,
    };
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd as StdOwnedFd};
    use std::ptr;

    const NR_READ: usize = 0;
    const NR_WRITE: usize = 1;
    const NR_CLOSE: usize = 3;
    const NR_DUP2: usize = 33;
    const NR_DUP3: usize = 292;
    const NR_SOCKETPAIR: usize = 53;
    const NR_CLONE: usize = 56;
    const NR_UNSHARE: usize = 272;
    const NR_SETNS: usize = 308;
    const NR_SECCOMP: usize = 317;
    const NR_MKDIR: usize = 83;
    const NR_CHDIR: usize = 80;
    const NR_CHROOT: usize = 161;
    const NR_MOUNT: usize = 165;
    const NR_OPEN_TREE: usize = 428;
    const NR_MOVE_MOUNT: usize = 429;
    const NR_CLONE3: usize = 435;
    const NR_MOUNT_SETATTR: usize = 442;
    const NR_LANDLOCK_CREATE_RULESET: usize = 444;
    const NR_LANDLOCK_ADD_RULE: usize = 445;
    const NR_LANDLOCK_RESTRICT_SELF: usize = 446;
    const NR_EXECVEAT: usize = 322;
    const NR_MEMFD_CREATE: usize = 319;
    const NR_EXIT_GROUP: usize = 231;
    const NR_WAIT4: usize = 61;
    const NR_KILL: usize = 62;
    const NR_SETGROUPS: usize = 116;
    const NR_SETRESUID: usize = 117;
    const NR_SETRESGID: usize = 119;
    const NR_GETPID: usize = 39;
    const NR_GETGROUPS: usize = 115;
    const NR_GETRESUID: usize = 118;
    const NR_GETRESGID: usize = 120;
    const NR_SETFSUID: usize = 122;
    const NR_SETFSGID: usize = 123;
    const NR_CAPGET: usize = 125;
    const NR_CAPSET: usize = 126;
    const NR_PRCTL: usize = 157;
    const NR_OPENAT: usize = 257;
    const NR_PIPE2: usize = 293;
    const NR_FCNTL: usize = 72;
    const F_ADD_SEALS: usize = 1033;
    const F_SEAL_SEAL: usize = 0x0001;
    const F_SEAL_SHRINK: usize = 0x0002;
    const F_SEAL_GROW: usize = 0x0004;
    const F_SEAL_WRITE: usize = 0x0008;
    const MFD_CLOEXEC: usize = 0x0001;
    const MFD_ALLOW_SEALING: usize = 0x0002;
    const MFD_EXEC: usize = 0x0010;
    const AT_EMPTY_PATH: usize = 0x1000;
    const NR_SENDTO: usize = 44;
    const NR_SENDMSG: usize = 46;
    const NR_RECVMSG: usize = 47;
    const NR_FORK: usize = 57;
    const NR_VFORK: usize = 58;
    const NR_SENDMMSG: usize = 307;
    const NR_IO_URING_SETUP: usize = 425;
    const NR_IO_URING_ENTER: usize = 426;
    const NR_IO_URING_REGISTER: usize = 427;
    const NR_PIDFD_GETFD: usize = 438;
    const NR_GETSOCKOPT: usize = 55;
    const NR_WAITID: usize = 247;
    const NR_CLOSE_RANGE: usize = 436;
    const NR_PIDFD_SEND_SIGNAL: usize = 424;
    const NR_POLL: usize = 7;
    const NR_RT_SIGPROCMASK: usize = 14;
    const NR_SIGNALFD4: usize = 289;
    #[cfg(linux_pid1_safety_smoke)]
    const NR_NANOSLEEP: usize = 35;

    const EINTR: isize = 4;
    const ESRCH: isize = 3;
    const ECHILD: isize = 10;
    const EAGAIN: isize = 11;
    const EPERM: isize = 1;
    const ENOSYS: isize = 38;
    const EINVAL: isize = 22;
    const EOPNOTSUPP: isize = 95;
    const EOVERFLOW: isize = 75;
    #[cfg(test)]
    const EBADF: usize = 9;
    #[cfg(test)]
    const EMFILE: usize = 24;
    #[cfg(linux_pid1_safety_smoke)]
    const EBUSY: i32 = 16;
    const SIGCHLD: usize = 17;
    const SIGKILL: usize = 9;
    const SIG_BLOCK: usize = 0;
    const SIG_SETMASK: usize = 2;
    const CLONE_NEWPID: usize = 0x2000_0000;
    const WNOHANG: usize = 1;
    const WEXITED: usize = 4;
    const WNOWAIT: usize = 0x0100_0000;
    const P_ALL: usize = 0;
    const P_PID: usize = 1;
    const P_PIDFD: usize = 3;
    const SFD_CLOEXEC: usize = 0x80000;
    const SFD_NONBLOCK: usize = 0x800;
    const LINUX_SIGNAL_INFO_BYTES: usize = 128;
    const AF_UNIX: usize = 1;
    const SOCK_STREAM: usize = 1;
    const SOCK_CLOEXEC: usize = 0x80000;
    const O_RDWR: usize = 2;
    const O_WRONLY: usize = 1;
    const O_CREAT: usize = 0x40;
    const O_EXCL: usize = 0x80;
    const O_CLOEXEC: usize = 0x80000;
    const O_PATH: usize = 0x200000;
    const AT_FDCWD: usize = (-100isize) as usize;
    const F_DUPFD_CLOEXEC: usize = 1030;
    const F_GETFD: usize = 1;
    const F_SETFD: usize = 2;
    const FD_CLOEXEC: usize = 1;
    const F_SETFL: usize = 4;
    const O_NONBLOCK: usize = 0x800;
    const MSG_NOSIGNAL: usize = 0x4000;
    const MSG_CMSG_CLOEXEC: usize = 0x4000_0000;
    const MSG_CTRUNC: i32 = 0x0008;
    const MSG_TRUNC: i32 = 0x0020;
    const CLOSE_RANGE_CLOEXEC: usize = 1 << 2;
    const PR_CAPBSET_READ: usize = 23;
    const PR_CAPBSET_DROP: usize = 24;
    const PR_GET_SECUREBITS: usize = 27;
    const PR_SET_SECUREBITS: usize = 28;
    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const PR_GET_NO_NEW_PRIVS: usize = 39;
    const PR_GET_SECCOMP: usize = 21;
    const PR_CAP_AMBIENT: usize = 47;
    const PR_CAP_AMBIENT_IS_SET: usize = 1;
    const PR_CAP_AMBIENT_CLEAR_ALL: usize = 4;
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    const LINUX_CAPABILITY_U32S_3: usize = 2;
    const MAX_LINUX_CAPABILITIES: usize = LINUX_CAPABILITY_U32S_3 * 32;
    const SECBIT_NOROOT: usize = 1 << 0;
    const SECBIT_NOROOT_LOCKED: usize = 1 << 1;
    const SECBIT_NO_SETUID_FIXUP_LOCKED: usize = 1 << 3;
    const SECBIT_KEEP_CAPS_LOCKED: usize = 1 << 5;
    const SECBIT_NO_CAP_AMBIENT_RAISE: usize = 1 << 6;
    const SECBIT_NO_CAP_AMBIENT_RAISE_LOCKED: usize = 1 << 7;
    const SERVICE_SECUREBITS: usize = SECBIT_NOROOT
        | SECBIT_NOROOT_LOCKED
        | SECBIT_NO_SETUID_FIXUP_LOCKED
        | SECBIT_KEEP_CAPS_LOCKED
        | SECBIT_NO_CAP_AMBIENT_RAISE
        | SECBIT_NO_CAP_AMBIENT_RAISE_LOCKED;
    const SERVICE_SANDBOX_ID: usize = 65_534;
    const CLONE_NEWNS: usize = 0x0002_0000;
    const CLONE_NEWTIME: usize = 0x0000_0080;
    const CLONE_NEWCGROUP: usize = 0x0200_0000;
    const CLONE_NEWUTS: usize = 0x0400_0000;
    const CLONE_NEWIPC: usize = 0x0800_0000;
    const CLONE_NEWUSER: usize = 0x1000_0000;
    const CLONE_NEWNET: usize = 0x4000_0000;
    const CLONE_VM: usize = 0x0000_0100;
    const CLONE_FILES: usize = 0x0000_0400;
    const CLONE_SIGHAND: usize = 0x0000_0800;
    const CLONE_THREAD: usize = 0x0001_0000;
    const SERVICE_CLONE_THREAD_FLAGS: u32 =
        (CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD) as u32;
    const SERVICE_NAMESPACE_FLAGS: usize =
        CLONE_NEWNS | CLONE_NEWCGROUP | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET;
    const MS_NOSUID: usize = 2;
    const MS_NODEV: usize = 4;
    const MS_NOEXEC: usize = 8;
    const MS_REC: usize = 0x4000;
    const MS_BIND: usize = 0x1000;
    const MS_REMOUNT: usize = 32;
    const MS_PRIVATE: usize = 1 << 18;
    const OPEN_TREE_CLONE: usize = 1;
    const MOVE_MOUNT_F_EMPTY_PATH: usize = 0x0000_0004;
    const MOUNT_ATTR_NOSUID: u64 = 0x2;
    const MOUNT_ATTR_NODEV: u64 = 0x4;
    const MOUNT_ATTR_NOEXEC: u64 = 0x8;
    const MOUNT_ATTR_NOSYMFOLLOW: u64 = 0x0020_0000;
    const LANDLOCK_CREATE_RULESET_VERSION: usize = 1;
    const LANDLOCK_RULE_PATH_BENEATH: usize = 1;
    const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
    const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
    const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
    const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;
    const LANDLOCK_ACCESS_FS_IOCTL_DEV: u64 = 1 << 15;
    const LANDLOCK_ACCESS_FS_RESOLVE_UNIX: u64 = 1 << 16;
    const LANDLOCK_KNOWN_ABI_MAX: u32 = 9;
    // Deny list for namespace changes after setup. CLONE_NEWUSER appears here
    // only so the child seccomp filter rejects later user-namespace changes;
    // the initial child clone deliberately requests CLONE_NEWPID only.
    const CLONE_NAMESPACE_FLAGS: usize = CLONE_NEWNS
        | CLONE_NEWTIME
        | CLONE_NEWCGROUP
        | CLONE_NEWUTS
        | CLONE_NEWIPC
        | CLONE_NEWUSER
        | CLONE_NEWPID
        | CLONE_NEWNET;
    const X32_SYSCALL_BIT: u32 = 0x4000_0000;
    const SECCOMP_SET_MODE_FILTER: usize = 1;
    const SECCOMP_MODE_FILTER: isize = 2;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
    const RELEASE_BYTE: u8 = 1;
    const CHILD_SETUP_READY_WORD: u32 = 0x4353_5231;
    const CHILD_FD_DELEGATION_READY_WORD: u32 = 0x4346_4431;
    const ENDPOINT_MESSAGE_BYTE: u8 = 0xc3;
    const SERVICE_ENDPOINT_FD: i32 = 3;
    const SERVICE_ROOT_PATH: &[u8] = b"/tmp\0";
    const SERVICE_TMPFS_TYPE: &[u8] = b"tmpfs\0";
    const SERVICE_TMPFS_OPTIONS: &[u8] = b"mode=0755,size=16777216,nr_inodes=4096\0";
    #[cfg(test)]
    pub(super) const SERVICE_TMPFS_SIZE_BYTES: u64 = 16 * 1024 * 1024;
    #[cfg(test)]
    pub(super) const SERVICE_TMPFS_MAX_INODES: u64 = 4096;
    #[cfg(test)]
    pub(super) const APP_ISOLATION_TEST_FD_CANDIDATES: [i32; 5] = [128, 64, 32, 16, 8];
    const TASK24_ENDPOINT_ARGUMENT_PREFIX: &str = "--nagi-service-endpoint-v1=";
    const TASK24_ENDPOINT_ARGUMENT_FAMILY: &str = "--nagi-service-endpoint";
    const TASK24_ENDPOINT_SOCKET_ROOT: &str = "/run/nagi/ipc/";
    const TASK24_ENDPOINT_SOCKET_LEAF_LEN: usize = 40;
    const SOL_SOCKET: i32 = 1;
    const SCM_RIGHTS: i32 = 1;
    const SCM_CREDENTIALS: i32 = 2;
    const SOCK_DGRAM: usize = 2;
    const SO_PEERCRED: usize = 17;
    const POLLIN: i16 = 0x001;
    const POLLERR: i16 = 0x008;
    const POLLHUP: i16 = 0x010;
    const POLLNVAL: i16 = 0x020;
    const ENDPOINT_CONTROL_BYTES: usize = 256;
    const MAX_RECEIVED_RIGHTS: usize = 60;

    // Cache only a successfully created descriptor. Transient setup failures
    // (including fd exhaustion) must remain retryable on the next timed wait.
    static PID1_SIGNAL_FD: std::sync::OnceLock<OwnedFd> = std::sync::OnceLock::new();
    static PID1_CHILD_REAPER: std::sync::OnceLock<std::sync::Mutex<Pid1ChildReaper>> =
        std::sync::OnceLock::new();
    static PID1_REAPER_WARNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    pub(super) fn wait_for_os_event() {
        // A timed signalfd poll is only a wake hint. Periodic scans preserve
        // progress if another pre-existing thread receives SIGCHLD instead.
        let signal_fd = match pid1_sigchld_fd() {
            Ok(fd) => Some(fd),
            Err(error) => {
                warn_reaper_once("SIGCHLD descriptor setup failed", &error);
                None
            }
        };
        if let Err(error) = reap_pending_child_exits() {
            warn_reaper_once("child exit scan failed", &error);
            std::thread::park_timeout(std::time::Duration::from_millis(250));
            return;
        }
        if let Some(signal_fd) = signal_fd {
            if let Err(error) = wait_for_sigchld(signal_fd) {
                warn_reaper_once("SIGCHLD wait failed", &error);
            }
        } else {
            std::thread::park_timeout(std::time::Duration::from_millis(250));
        }
        if let Err(error) = reap_pending_child_exits() {
            warn_reaper_once("child reap failed", &error);
            std::thread::park_timeout(std::time::Duration::from_millis(250));
        }
    }

    fn warn_reaper_once(context: &str, error: &io::Error) {
        if !PID1_REAPER_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            super::log_pid1_stderr(format_args!(
                "nagi-init: {context} ({error}); retrying with timed wait"
            ));
        }
    }

    fn child_reaper() -> &'static std::sync::Mutex<Pid1ChildReaper> {
        PID1_CHILD_REAPER.get_or_init(|| std::sync::Mutex::new(Pid1ChildReaper::default()))
    }

    fn reap_pending_child_exits() -> io::Result<usize> {
        drain_serialized(child_reaper(), &mut LinuxReapKernel)
    }

    fn reap_tracked_child(ticket: &ReapTicket) -> io::Result<Option<ServiceExitStatus>> {
        reap_pending_child_exits().inspect_err(|_error| {
            #[cfg(test)]
            eprintln!("C3_REAPER_SCAN_ERROR: {_error}");
        })?;
        let status = ticket.exit_status();
        #[cfg(test)]
        if let Err(error) = &status {
            eprintln!("C3_REAPER_TICKET_ERROR: {error}");
        }
        status
    }

    #[cfg(test)]
    pub(super) fn initialize_reaper_wait_channel_for_test() -> io::Result<()> {
        pid1_sigchld_fd().map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn waitid_reports_untracked_child_reaped_for_test(
        process_id: u32,
    ) -> io::Result<bool> {
        let mut info = LinuxSigInfo::default();
        let result = unsafe {
            syscall5(
                NR_WAITID,
                P_PID,
                process_id as usize,
                &mut info as *mut LinuxSigInfo as usize,
                WEXITED | WNOHANG | WNOWAIT,
                0,
            )
        };
        if result == -(ECHILD) {
            return Ok(true);
        }
        syscall_result(result)?;
        if info.pid == 0 || info.pid as u32 == process_id {
            return Ok(false);
        }
        Err(io::Error::other(
            "waitid(P_PID) returned a different child process ID",
        ))
    }

    fn pid1_sigchld_fd() -> io::Result<&'static OwnedFd> {
        if let Some(fd) = PID1_SIGNAL_FD.get() {
            return Ok(fd);
        }

        let signal_mask = 1u64 << (SIGCHLD - 1);
        let blocked = unsafe {
            syscall4(
                NR_RT_SIGPROCMASK,
                SIG_BLOCK,
                &signal_mask as *const u64 as usize,
                0,
                core::mem::size_of::<u64>(),
            )
        };
        syscall_result(blocked)?;

        let fd = unsafe {
            syscall4(
                NR_SIGNALFD4,
                usize::MAX,
                &signal_mask as *const u64 as usize,
                core::mem::size_of::<u64>(),
                SFD_CLOEXEC | SFD_NONBLOCK,
            )
        };
        let fd = OwnedFd(syscall_result(fd)? as i32);
        if let Err(unused_fd) = PID1_SIGNAL_FD.set(fd) {
            drop(unused_fd);
        }
        PID1_SIGNAL_FD.get().ok_or_else(|| {
            io::Error::other("SIGCHLD descriptor initialization did not publish a descriptor")
        })
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn reap_after_notification_failure(
        error: io::Error,
        mut reap: impl FnMut() -> io::Result<(usize, bool)>,
    ) -> io::Result<usize> {
        let (reaped, _has_live_children) = reap()?;
        if reaped != 0 {
            Ok(reaped)
        } else {
            Err(error)
        }
    }

    /// Safety-smoke-only helper for fixture descendants. The normal PID-1
    /// lifecycle never consumes child status through wait4(-1).
    #[cfg(linux_pid1_safety_smoke)]
    fn reap_available_children() -> io::Result<(usize, bool)> {
        let mut reaped = 0usize;
        loop {
            let mut status = 0i32;
            let result = unsafe {
                syscall4(
                    NR_WAIT4,
                    usize::MAX,
                    &mut status as *mut i32 as usize,
                    WNOHANG,
                    0,
                )
            };
            if result > 0 {
                let exit = decode_wait_status(status);
                super::log_pid1_stderr(format_args!(
                    "nagi-init: PID 1 reaped child pid={} status={exit:?}",
                    result
                ));
                reaped += 1;
                continue;
            }
            if result == 0 {
                return Ok((reaped, true));
            }
            if result == -ECHILD {
                return Ok((reaped, false));
            }
            if result == -EINTR {
                continue;
            }
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
    }

    fn wait_for_sigchld(signal_fd: &OwnedFd) -> io::Result<()> {
        let mut poll_fd = PollFd {
            fd: signal_fd.raw(),
            events: POLLIN,
            revents: 0,
        };
        let ready = unsafe { syscall3(NR_POLL, &mut poll_fd as *mut PollFd as usize, 1, 250) };
        if ready == -(EINTR) || ready == 0 {
            return Ok(());
        }
        if ready < 0 {
            return Err(io::Error::from_raw_os_error((-ready) as i32));
        }
        if poll_fd.revents & POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(9));
        }
        if poll_fd.revents & (POLLIN | POLLERR | POLLHUP) == 0 {
            return Err(io::Error::other(
                "signalfd poll returned an unrelated event",
            ));
        }
        let mut signal_info = [0u8; LINUX_SIGNAL_INFO_BYTES];
        loop {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    signal_fd.raw() as usize,
                    signal_info.as_mut_ptr() as usize,
                    signal_info.len(),
                )
            };
            if result == LINUX_SIGNAL_INFO_BYTES as isize {
                return Ok(());
            }
            if result == -EINTR {
                continue;
            }
            if result == -EAGAIN {
                // Another waiter may have consumed the signalfd record after
                // poll reported it. The caller rescans child status either way.
                return Ok(());
            }
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "signalfd returned a short SIGCHLD record",
            ));
        }
    }

    #[repr(C)]
    struct LinuxSigInfo {
        signo: i32,
        errno: i32,
        code: i32,
        _pad0: i32,
        pid: i32,
        uid: u32,
        status: i32,
        _rest: [u8; 100],
    }

    impl Default for LinuxSigInfo {
        fn default() -> Self {
            Self {
                signo: 0,
                errno: 0,
                code: 0,
                _pad0: 0,
                pid: 0,
                uid: 0,
                status: 0,
                _rest: [0; 100],
            }
        }
    }

    const _: [(); LINUX_SIGNAL_INFO_BYTES] = [(); core::mem::size_of::<LinuxSigInfo>()];

    struct LinuxReapKernel;

    impl ReapKernel for LinuxReapKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            let mut info = LinuxSigInfo::default();
            let result = unsafe {
                syscall5(
                    NR_WAITID,
                    P_ALL,
                    0,
                    &mut info as *mut LinuxSigInfo as usize,
                    WEXITED | WNOHANG | WNOWAIT,
                    0,
                )
            };
            if let Err(error) = syscall_result(result) {
                if error.raw_os_error() == Some(ECHILD as i32) {
                    return Err(error);
                }
                return Err(io::Error::new(
                    error.kind(),
                    format!("waitid(P_ALL): {error}"),
                ));
            }
            if info.pid == 0 {
                return Ok(None);
            }
            if info.pid <= 1 {
                return Err(io::Error::other(
                    "waitid(P_ALL) returned an invalid child PID",
                ));
            }
            Ok(Some(info.pid as u32))
        }

        fn observe_cleanup_exit_nonconsuming(&mut self, process_id: u32) -> io::Result<bool> {
            if process_id <= 1 {
                return Err(io::Error::other("refusing to observe a non-child PID"));
            }
            let mut info = LinuxSigInfo::default();
            let result = unsafe {
                syscall5(
                    NR_WAITID,
                    P_PID,
                    process_id as usize,
                    &mut info as *mut LinuxSigInfo as usize,
                    WEXITED | WNOHANG | WNOWAIT,
                    0,
                )
            };
            syscall_result(result)
                .map_err(|error| io::Error::new(error.kind(), format!("waitid(P_PID): {error}")))?;
            if info.pid == 0 {
                return Ok(false);
            }
            if info.pid as u32 != process_id {
                return Err(io::Error::other(
                    "waitid(P_PID) returned a different child PID",
                ));
            }
            Ok(true)
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            if pidfd < 0 {
                return Err(io::Error::other("tracked process has no pidfd"));
            }
            let mut info = LinuxSigInfo::default();
            let result = unsafe {
                syscall5(
                    NR_WAITID,
                    P_PIDFD,
                    pidfd as usize,
                    &mut info as *mut LinuxSigInfo as usize,
                    WEXITED | WNOHANG | WNOWAIT,
                    0,
                )
            };
            syscall_result(result).map_err(|error| {
                io::Error::new(error.kind(), format!("waitid(P_PIDFD): {error}"))
            })?;
            if info.pid == 0 {
                return Ok(false);
            }
            if info.pid as u32 != process_id {
                return Err(io::Error::other(
                    "waitid(P_PIDFD) returned a different child PID",
                ));
            }
            Ok(true)
        }

        fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>> {
            if process_id <= 1 {
                return Err(io::Error::other("refusing to reap a non-child PID"));
            }
            let mut status = 0i32;
            let result = unsafe {
                syscall4(
                    NR_WAIT4,
                    process_id as usize,
                    &mut status as *mut i32 as usize,
                    WNOHANG,
                    0,
                )
            };
            let waited_pid = syscall_result(result).map_err(|error| {
                io::Error::new(error.kind(), format!("wait4(pid={process_id}): {error}"))
            })? as u32;
            if waited_pid == 0 {
                return Ok(None);
            }
            if waited_pid != process_id {
                return Err(io::Error::other("wait4 returned a different child PID"));
            }
            Ok(Some(decode_wait_status(status)))
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_any_child_event() -> io::Result<usize> {
        // Block SIGCHLD before the first wait4 probe. Otherwise an exit between
        // wait4(WNOHANG) and sigprocmask could be discarded under the default
        // disposition and leave PID 1 asleep with an unreaped zombie.
        let signal_fd = match pid1_sigchld_fd() {
            Ok(fd) => fd,
            Err(error) => return reap_after_notification_failure(error, reap_available_children),
        };
        loop {
            let (reaped, _has_live_children) = reap_available_children()?;
            if reaped != 0 {
                return Ok(reaped);
            }
            if let Err(error) = wait_for_sigchld(signal_fd) {
                return reap_after_notification_failure(error, reap_available_children);
            }
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_tracked_child(pid: u32) -> io::Result<ServiceExitStatus> {
        loop {
            if let Some(status) = wait_child_nonblocking(pid)? {
                return Ok(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    pub(super) fn run_pid1_safety_smoke() -> ! {
        pid1_log!("nagi-init: PID 1 safety smoke started; production services are disabled");

        match mount_tmpfs_on_dev_for_safety_smoke() {
            Ok(()) => match super::mount_devtmpfs_for_pid1() {
                Ok(()) => {
                    use std::os::unix::fs::FileTypeExt;
                    let null_is_device = std::fs::metadata("/dev/null")
                        .map(|metadata| metadata.file_type().is_char_device())
                        .unwrap_or(false);
                    if null_is_device {
                        pid1_log!("nagi-init: PASS foreign tmpfs at /dev was overmounted by devtmpfs; /dev/null is a character device");
                    } else {
                        pid1_log!("nagi-init: FAIL devtmpfs mount succeeded but /dev/null is not a character device");
                    }
                }
                Err(error) if error.raw_os_error() == Some(EBUSY) => pid1_log!(
                    "nagi-init: devtmpfs mount returned ambiguous EBUSY ({EBUSY}); no filesystem is inferred and startup fails closed"
                ),
                Err(error) => pid1_log!("nagi-init: FAIL devtmpfs mount over tmpfs failed: {error}"),
            },
            Err(error) => pid1_log!("nagi-init: FAIL could not mount test tmpfs at /dev: {error}"),
        }

        match super::mount_devtmpfs_at(b"/__nagi_missing_devtmpfs_target__\0") {
            Err(error) if error.raw_os_error() == Some(2) => pid1_log!(
                "nagi-init: PASS missing devtmpfs target returned ENOENT; no service startup is attempted"
            ),
            Err(error) => pid1_log!(
                "nagi-init: FAIL missing devtmpfs target returned unexpected error: {error}"
            ),
            Ok(()) => pid1_log!(
                "nagi-init: FAIL missing devtmpfs target unexpectedly mounted"
            ),
        }

        match spawn_pid1_orphan_fixture(1) {
            Ok(parent_pid) => match wait_for_tracked_child(parent_pid) {
                Ok(status) => {
                    pid1_log!("nagi-init: orphan fixture direct child exited ({status:?})");
                    match wait_for_any_child_event() {
                        Ok(reaped) if reaped > 0 => pid1_log!(
                            "nagi-init: PASS PID 1 reaped orphan descendant after direct child exit"
                        ),
                        Ok(_) => pid1_log!("nagi-init: FAIL orphan reaper woke without a reap"),
                        Err(error) => pid1_log!("nagi-init: FAIL orphan reaper failed: {error}"),
                    }
                }
                Err(error) => pid1_log!("nagi-init: FAIL direct child wait failed: {error}"),
            },
            Err(error) => pid1_log!("nagi-init: FAIL could not create orphan fixture: {error}"),
        }

        match run_reaper_setup_failure_smoke() {
            Ok(()) => pid1_log!(
                "nagi-init: PASS injected SIGCHLD setup failure used timed wait4 fallback and reaped an orphan"
            ),
            Err(error) => pid1_log!(
                "nagi-init: FAIL injected SIGCHLD setup failure did not reap an orphan: {error}"
            ),
        }

        match run_pid_namespace_boundary_smoke() {
            Ok(()) => pid1_log!(
                "nagi-init: PASS tracked service namespace was stopped and reaped while unrelated helper survived"
            ),
            Err(error) => pid1_log!(
                "nagi-init: FAIL service namespace boundary did not contain shutdown: {error}"
            ),
        }

        super::wait_in_pid1(wait_for_os_event)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn mount_tmpfs_on_dev_for_safety_smoke() -> io::Result<()> {
        const MS_NOSUID: std::os::raw::c_ulong = 2;
        const MS_NOEXEC: std::os::raw::c_ulong = 8;
        const SOURCE: &[u8] = b"tmpfs\0";
        const TARGET: &[u8] = b"/dev\0";
        const FILESYSTEM: &[u8] = b"tmpfs\0";
        const OPTIONS: &[u8] = b"mode=0755\0";
        let result = unsafe {
            super::mount(
                SOURCE.as_ptr().cast(),
                TARGET.as_ptr().cast(),
                FILESYSTEM.as_ptr().cast(),
                MS_NOSUID | MS_NOEXEC,
                OPTIONS.as_ptr().cast(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_pid1_orphan_fixture(descendant_delay_seconds: i64) -> io::Result<u32> {
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
        if child == 0 {
            let groups = unsafe { syscall2(NR_SETGROUPS, 0, 0) };
            let group_ids = unsafe {
                syscall3(
                    NR_SETRESGID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let user_ids = unsafe {
                syscall3(
                    NR_SETRESUID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let no_new_privs = unsafe { syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if groups < 0 || group_ids < 0 || user_ids < 0 || no_new_privs < 0 {
                exit_with_code(122);
            }

            let descendant = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
            if descendant == 0 {
                let delay = Timespec {
                    seconds: descendant_delay_seconds,
                    nanoseconds: 0,
                };
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
                exit_with_code(37);
            }
            if descendant < 0 {
                exit_with_code(123);
            }
            exit_with_code(0);
        }
        syscall_result(child).map(|pid| pid as u32)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn run_reaper_setup_failure_smoke() -> io::Result<()> {
        let parent = spawn_pid1_orphan_fixture(1)?;
        wait_for_child_bounded(parent, std::time::Duration::from_secs(5))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match reap_after_notification_failure(
                io::Error::from_raw_os_error(24),
                reap_available_children,
            ) {
                Ok(reaped) if reaped != 0 => return Ok(()),
                Ok(_) => return Err(io::Error::other("fallback returned without a reap")),
                Err(error) if error.raw_os_error() == Some(24) => {}
                Err(error) => return Err(error),
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "injected signalfd failure fallback did not reap within 5 seconds",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn run_pid_namespace_boundary_smoke() -> io::Result<()> {
        let helper_pid = spawn_unrelated_helper_fixture()?;
        let boundary_result = (|| {
            let (service_pid, _ready_read, descendant_hold_read) =
                spawn_pid_namespace_service_fixture()?;
            let mut service_pid_to_cleanup = Some(service_pid);
            let result = (|| {
                if wait_child_nonblocking(helper_pid)?.is_some() || !process_is_alive(helper_pid)? {
                    return Err(io::Error::other(
                        "unrelated helper exited before service shutdown",
                    ));
                }

                let killed = unsafe { syscall2(NR_KILL, service_pid as usize, SIGKILL) };
                if killed < 0 && killed != -(ESRCH) {
                    return Err(io::Error::from_raw_os_error((-killed) as i32));
                }
                wait_for_child_bounded(service_pid, std::time::Duration::from_secs(5))?;
                service_pid_to_cleanup = None;
                wait_for_pipe_eof(&descendant_hold_read, std::time::Duration::from_secs(5))?;

                if wait_child_nonblocking(helper_pid)?.is_some() || !process_is_alive(helper_pid)? {
                    return Err(io::Error::other(
                        "stopping the tracked service namespace also stopped the unrelated helper",
                    ));
                }
                Ok(())
            })();
            if let Some(service_pid) = service_pid_to_cleanup {
                let _ = terminate_and_reap(service_pid);
            }
            result
        })();

        let helper_cleanup = terminate_and_reap(helper_pid);
        match (boundary_result, helper_cleanup) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_unrelated_helper_fixture() -> io::Result<u32> {
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
        if child == 0 {
            let delay = Timespec {
                seconds: 30,
                nanoseconds: 0,
            };
            let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
            exit_with_code(0);
        }
        syscall_result(child).map(|pid| pid as u32)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_pid_namespace_service_fixture() -> io::Result<(u32, OwnedFd, OwnedFd)> {
        let (ready_read, ready_write) = create_pipe_pair()?;
        let (hold_read, hold_write) = create_pipe_pair()?;
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD | CLONE_NEWPID, 0, 0, 0, 0) };
        if child == 0 {
            unsafe {
                syscall1(NR_CLOSE, ready_read.raw() as usize);
                syscall1(NR_CLOSE, hold_read.raw() as usize);
            }
            let unblocked_signals = 0u64;
            let signals_unblocked = unsafe {
                syscall4(
                    NR_RT_SIGPROCMASK,
                    SIG_SETMASK,
                    &unblocked_signals as *const u64 as usize,
                    0,
                    core::mem::size_of::<u64>(),
                )
            };
            let groups = unsafe { syscall2(NR_SETGROUPS, 0, 0) };
            let group_ids = unsafe {
                syscall3(
                    NR_SETRESGID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let user_ids = unsafe {
                syscall3(
                    NR_SETRESUID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let no_new_privs = unsafe { syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if signals_unblocked < 0
                || groups < 0
                || group_ids < 0
                || user_ids < 0
                || no_new_privs < 0
            {
                exit_with_code(122);
            }

            let descendant = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
            if descendant == 0 {
                unsafe { syscall1(NR_CLOSE, ready_write.raw() as usize) };
                let delay = Timespec {
                    seconds: 30,
                    nanoseconds: 0,
                };
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
                exit_with_code(37);
            }
            if descendant < 0 {
                exit_with_code(123);
            }

            // Only the descendant retains the hold-pipe writer. EOF in the
            // outer PID 1 proves the whole service namespace has terminated.
            unsafe {
                syscall1(NR_CLOSE, hold_write.raw() as usize);
                let ready_byte = [1u8];
                let _ = syscall3(
                    NR_WRITE,
                    ready_write.raw() as usize,
                    ready_byte.as_ptr() as usize,
                    ready_byte.len(),
                );
                syscall1(NR_CLOSE, ready_write.raw() as usize);
            }
            let delay = Timespec {
                seconds: 30,
                nanoseconds: 0,
            };
            loop {
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
            }
        }

        let service_pid = match syscall_result(child) {
            Ok(pid) => pid as u32,
            Err(error) => return Err(error),
        };
        drop(ready_write);
        drop(hold_write);
        if let Err(error) = set_nonblocking(hold_read.raw()) {
            let _ = terminate_and_reap(service_pid);
            return Err(error);
        }
        let mut ready = 0u8;
        let read = unsafe {
            syscall3(
                NR_READ,
                ready_read.raw() as usize,
                &mut ready as *mut u8 as usize,
                1,
            )
        };
        if read != 1 || ready != 1 {
            let _ = terminate_and_reap(service_pid);
            return Err(if read < 0 {
                io::Error::from_raw_os_error((-read) as i32)
            } else {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "service namespace did not become ready",
                )
            });
        }
        Ok((service_pid, ready_read, hold_read))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn create_pipe_pair() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut descriptors = [-1i32; 2];
        syscall_result(unsafe { syscall2(NR_PIPE2, descriptors.as_mut_ptr() as usize, 0) })?;
        Ok((OwnedFd(descriptors[0]), OwnedFd(descriptors[1])))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn process_is_alive(process_id: u32) -> io::Result<bool> {
        let result = unsafe { syscall2(NR_KILL, process_id as usize, 0) };
        if result == 0 {
            Ok(true)
        } else if result == -ESRCH {
            Ok(false)
        } else {
            Err(io::Error::from_raw_os_error((-result) as i32))
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn terminate_and_reap(process_id: u32) -> io::Result<ServiceExitStatus> {
        if let Some(status) = wait_child_nonblocking(process_id)? {
            return Ok(status);
        }
        let killed = unsafe { syscall2(NR_KILL, process_id as usize, SIGKILL) };
        if killed < 0 && killed != -(ESRCH) {
            return Err(io::Error::from_raw_os_error((-killed) as i32));
        }
        wait_for_child_bounded(process_id, std::time::Duration::from_secs(5))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_child_bounded(
        process_id: u32,
        budget: std::time::Duration,
    ) -> io::Result<ServiceExitStatus> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            if let Some(status) = wait_child_nonblocking(process_id)? {
                return Ok(status);
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "tracked child did not exit before the smoke deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_pipe_eof(fd: &OwnedFd, budget: std::time::Duration) -> io::Result<()> {
        let deadline = std::time::Instant::now() + budget;
        let mut byte = 0u8;
        loop {
            let result =
                unsafe { syscall3(NR_READ, fd.raw() as usize, &mut byte as *mut u8 as usize, 1) };
            if result == 0 {
                return Ok(());
            }
            if result == -(EINTR) {
                continue;
            }
            if result < 0 && result != -EAGAIN {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service namespace descendant retained the pipe past the deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn exit_with_code(code: usize) -> ! {
        unsafe {
            loop {
                syscall1(NR_EXIT_GROUP, code);
            }
        }
    }

    #[inline(always)]
    unsafe fn syscall1(number: usize, a1: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall2(number: usize, a1: usize, a2: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall3(number: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall4(number: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall5(
        number: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall6(
        number: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
        a6: usize,
    ) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                in("r9") a6,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    fn syscall_result(result: isize) -> io::Result<usize> {
        if result < 0 {
            Err(io::Error::from_raw_os_error((-result) as i32))
        } else {
            Ok(result as usize)
        }
    }

    struct OwnedFd(i32);

    impl OwnedFd {
        fn from_result(result: isize) -> io::Result<Self> {
            syscall_result(result).map(|fd| Self(fd as i32))
        }

        fn raw(&self) -> i32 {
            self.0
        }

        fn close_now(&mut self) -> io::Result<()> {
            if self.0 < 0 {
                return Ok(());
            }
            let fd = self.0;
            self.0 = -1;
            unsafe { syscall_result(syscall1(NR_CLOSE, fd as usize)).map(|_| ()) }
        }
    }

    impl Drop for OwnedFd {
        fn drop(&mut self) {
            let _ = self.close_now();
        }
    }

    fn promote_fd(mut fd: OwnedFd) -> io::Result<OwnedFd> {
        if fd.raw() >= 3 {
            return Ok(fd);
        }
        let duplicated = unsafe { syscall3(NR_FCNTL, fd.raw() as usize, F_DUPFD_CLOEXEC, 3) };
        let promoted = OwnedFd::from_result(duplicated)?;
        fd.close_now()?;
        Ok(promoted)
    }

    struct LinuxSealedExecutableOps;

    impl SealedExecutableOps for LinuxSealedExecutableOps {
        type Handle = OwnedFd;

        fn create(&mut self) -> io::Result<Self::Handle> {
            let name = b"nagi-package-executable\0";
            let flags = MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_EXEC;
            let fd = unsafe { syscall2(NR_MEMFD_CREATE, name.as_ptr() as usize, flags) };
            promote_fd(OwnedFd::from_result(fd)?)
        }

        fn write(&mut self, handle: &Self::Handle, bytes: &[u8]) -> io::Result<usize> {
            syscall_result(unsafe {
                syscall3(
                    NR_WRITE,
                    handle.raw() as usize,
                    bytes.as_ptr() as usize,
                    bytes.len(),
                )
            })
        }

        fn seal(&mut self, handle: &Self::Handle) -> io::Result<()> {
            let seals = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL;
            syscall_result(unsafe { syscall3(NR_FCNTL, handle.raw() as usize, F_ADD_SEALS, seals) })
                .map(|_| ())
        }
    }

    fn set_nonblocking(fd: i32) -> io::Result<()> {
        syscall_result(unsafe { syscall3(NR_FCNTL, fd as usize, F_SETFL, O_NONBLOCK) }).map(|_| ())
    }

    fn open_null() -> io::Result<OwnedFd> {
        let path = b"/dev/null\0";
        let opened = unsafe {
            syscall4(
                NR_OPENAT,
                AT_FDCWD,
                path.as_ptr() as usize,
                O_RDWR | O_CLOEXEC,
                0,
            )
        };
        promote_fd(OwnedFd::from_result(opened)?)
    }

    struct Task24EndpointMountPaths {
        source: CString,
        target: CString,
    }

    #[repr(C)]
    struct LinuxMountAttr {
        attr_set: u64,
        attr_clr: u64,
        propagation: u64,
        userns_fd: u64,
    }

    #[repr(C)]
    struct LinuxLandlockRulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct LinuxLandlockPathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
        reserved: u32,
    }

    fn landlock_filesystem_rights(abi: u32) -> u64 {
        // The app executable is a sealed memfd on Landlock's internal
        // filesystem, which cannot be granted through a path-beneath rule.
        // Keep EXECUTE outside this ruleset; app-visible mounts are noexec.
        let mut rights = LANDLOCK_ACCESS_FS_WRITE_FILE
            | LANDLOCK_ACCESS_FS_READ_FILE
            | LANDLOCK_ACCESS_FS_READ_DIR
            | LANDLOCK_ACCESS_FS_REMOVE_DIR
            | LANDLOCK_ACCESS_FS_REMOVE_FILE
            | LANDLOCK_ACCESS_FS_MAKE_CHAR
            | LANDLOCK_ACCESS_FS_MAKE_DIR
            | LANDLOCK_ACCESS_FS_MAKE_REG
            | LANDLOCK_ACCESS_FS_MAKE_SOCK
            | LANDLOCK_ACCESS_FS_MAKE_FIFO
            | LANDLOCK_ACCESS_FS_MAKE_BLOCK
            | LANDLOCK_ACCESS_FS_MAKE_SYM;
        if abi >= 2 {
            rights |= LANDLOCK_ACCESS_FS_REFER;
        }
        if abi >= 3 {
            rights |= LANDLOCK_ACCESS_FS_TRUNCATE;
        }
        if abi >= 5 {
            rights |= LANDLOCK_ACCESS_FS_IOCTL_DEV;
        }
        if abi >= 9 {
            rights |= LANDLOCK_ACCESS_FS_RESOLVE_UNIX;
        }
        rights
    }

    #[cfg_attr(not(test), inline(always))]
    fn process_prepare_stage<T>(stage: &'static str, result: io::Result<T>) -> io::Result<T> {
        #[cfg(test)]
        if let Err(error) = &result {
            eprintln!(
                "C3_PROCESS_PREPARE_STAGE_FAILED: stage={stage} kind={:?} errno={:?} error={error}",
                error.kind(),
                error.raw_os_error()
            );
        }
        #[cfg(not(test))]
        let _ = stage;
        result
    }

    fn prepare_app_storage_mount(directory: std::os::fd::BorrowedFd<'_>) -> io::Result<OwnedFd> {
        let mount_tree = process_prepare_stage(
            "app_storage.open_tree",
            OwnedFd::from_result(unsafe {
                syscall3(
                    NR_OPEN_TREE,
                    directory.as_raw_fd() as usize,
                    c"".as_ptr() as usize,
                    OPEN_TREE_CLONE | O_CLOEXEC | AT_EMPTY_PATH,
                )
            }),
        )?;
        let attributes = LinuxMountAttr {
            attr_set: MOUNT_ATTR_NOSUID
                | MOUNT_ATTR_NODEV
                | MOUNT_ATTR_NOEXEC
                | MOUNT_ATTR_NOSYMFOLLOW,
            attr_clr: 0,
            propagation: MS_PRIVATE as u64,
            userns_fd: 0,
        };
        let changed = unsafe {
            syscall5(
                NR_MOUNT_SETATTR,
                mount_tree.raw() as usize,
                c"".as_ptr() as usize,
                AT_EMPTY_PATH,
                &attributes as *const LinuxMountAttr as usize,
                core::mem::size_of::<LinuxMountAttr>(),
            )
        };
        process_prepare_stage(
            "app_storage.mount_setattr",
            syscall_result(changed).map(|_| ()),
        )?;
        process_prepare_stage("app_storage.promote_mount_fd", promote_fd(mount_tree))
    }

    unsafe fn mount_service_tmpfs_at(target: *const i8) -> Result<(), usize> {
        let mounted = unsafe {
            syscall5(
                NR_MOUNT,
                0,
                target as usize,
                SERVICE_TMPFS_TYPE.as_ptr() as usize,
                MS_NOSUID | MS_NODEV | MS_NOEXEC,
                SERVICE_TMPFS_OPTIONS.as_ptr() as usize,
            )
        };
        if mounted < 0 {
            Err(child_syscall_errno(mounted))
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    pub(super) fn mount_service_tmpfs_for_test(target: &std::ffi::CStr) -> io::Result<()> {
        unsafe { mount_service_tmpfs_at(target.as_ptr()) }
            .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
    }

    #[cfg(test)]
    pub(super) fn attach_private_app_storage_mount_for_test(
        directory: std::os::fd::BorrowedFd<'_>,
        target: &std::ffi::CStr,
    ) -> io::Result<()> {
        let mount_tree = prepare_app_storage_mount(directory)?;
        let attached = unsafe {
            syscall5(
                NR_MOVE_MOUNT,
                mount_tree.raw() as usize,
                c"".as_ptr() as usize,
                AT_FDCWD,
                target.as_ptr() as usize,
                MOVE_MOUNT_F_EMPTY_PATH,
            )
        };
        if attached < 0 {
            Err(io::Error::from_raw_os_error((-attached) as i32))
        } else {
            Ok(())
        }
    }

    /// Validate the exact C2 run/slot locator and prepare a single-file
    /// projection into the service tmpfs. The pathname remains a locator;
    /// C2 still owns listener creation, peer authentication, and readiness.
    fn task24_endpoint_mount_paths(
        arguments: &[&str],
        endpoint_policy: ProcessEndpointPolicy,
        execution_identity: LinuxExecutionIdentity,
    ) -> io::Result<Option<Task24EndpointMountPaths>> {
        let mut endpoint_path = None;
        for argument in arguments {
            if !argument.starts_with(TASK24_ENDPOINT_ARGUMENT_FAMILY) {
                continue;
            }
            if endpoint_path.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Task24 launch contains duplicate broker endpoint arguments",
                ));
            }
            let path = argument
                .strip_prefix(TASK24_ENDPOINT_ARGUMENT_PREFIX)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Task24 launch contains an unsupported broker endpoint argument",
                    )
                })?;
            let Some(leaf) = path.strip_prefix(TASK24_ENDPOINT_SOCKET_ROOT) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Task24 broker endpoint is outside the private IPC directory",
                ));
            };
            let bytes = leaf.as_bytes();
            let is_lower_hex = |byte: &u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte);
            if bytes.len() != TASK24_ENDPOINT_SOCKET_LEAF_LEN
                || bytes[0] != b'r'
                || !bytes[1..17].iter().all(is_lower_hex)
                || &bytes[17..19] != b"-e"
                || !bytes[19..35].iter().all(is_lower_hex)
                || &bytes[35..] != b".sock"
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Task24 broker endpoint does not match the C2 run/slot locator format",
                ));
            }
            endpoint_path = Some(path);
        }

        let Some(path) = endpoint_path else {
            return Ok(None);
        };
        if endpoint_policy != ProcessEndpointPolicy::InheritedPeerChannel {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Task24 broker endpoint requires the isolated peer-channel launch policy",
            ));
        }

        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        for directory in ["/run", "/run/nagi", "/run/nagi/ipc"] {
            let metadata = std::fs::symlink_metadata(directory)?;
            if !metadata.file_type().is_dir()
                || metadata.uid() != 0
                || metadata.mode() & 0o022 != 0
                || metadata.mode() & 0o100 == 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Task24 broker path is not beneath a root-owned, owner-searchable, not group/world-writable directory",
                ));
            }
        }
        let socket_metadata = std::fs::symlink_metadata(path)?;
        if !socket_metadata.file_type().is_socket()
            || socket_metadata.uid() != execution_identity.uid
            || socket_metadata.gid() != execution_identity.gid
            || socket_metadata.mode() & 0o777 != 0o600
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Task24 broker path is not the exact child-owned C2 listener",
            ));
        }

        let source = CString::new(path).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Task24 broker path contains NUL",
            )
        })?;
        let mut target_bytes = Vec::with_capacity(b"/tmp".len() + path.len());
        target_bytes.extend_from_slice(b"/tmp");
        target_bytes.extend_from_slice(path.as_bytes());
        let target = CString::new(target_bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Task24 broker mount target contains NUL",
            )
        })?;
        Ok(Some(Task24EndpointMountPaths { source, target }))
    }

    fn decode_wait_status(status: i32) -> ServiceExitStatus {
        let terminating_signal = status & 0x7f;
        if terminating_signal == 0 {
            ServiceExitStatus {
                exit_code: Some((status >> 8) & 0xff),
                signal: None,
            }
        } else {
            ServiceExitStatus {
                exit_code: None,
                signal: Some(terminating_signal),
            }
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_child_nonblocking(pid: u32) -> io::Result<Option<ServiceExitStatus>> {
        let mut status = 0i32;
        let result = unsafe {
            syscall4(
                NR_WAIT4,
                pid as usize,
                &mut status as *mut i32 as usize,
                WNOHANG,
                0,
            )
        };
        // Return EINTR to the caller so a cleanup pass remains bounded; the
        // next poll/shutdown can retry without losing the child handle.
        let waited_pid = syscall_result(result)? as u32;
        if waited_pid == 0 {
            return Ok(None);
        }
        if waited_pid != pid {
            return Err(io::Error::other("wait4 returned a different child PID"));
        }
        Ok(Some(decode_wait_status(status)))
    }

    fn terminate_without_pidfd(pid: u32) -> io::Result<()> {
        let killed = unsafe { syscall2(NR_KILL, pid as usize, SIGKILL) };
        if killed < 0 && killed != -(ESRCH) {
            return Err(io::Error::from_raw_os_error((-killed) as i32));
        }
        Ok(())
    }

    fn wait_for_reap_ticket(ticket: &ReapTicket) -> io::Result<ServiceExitStatus> {
        loop {
            match reap_tracked_child(ticket)? {
                Some(status) => return Ok(status),
                None => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
    }

    /// Refuse to release a child when launch bookkeeping fails. Close the
    /// parent gate exactly once; if that close is ambiguous, make one exact
    /// pidfd signal attempt and leave all later status handling to the reaper.
    fn fail_closed_child_before_release(
        gate_parent: &mut OwnedFd,
        ticket: &ReapTicket,
        cause: &io::Error,
    ) -> io::Error {
        match gate_parent.close_now() {
            Ok(()) => io::Error::new(
                cause.kind(),
                format!(
                    "{cause}; child exec gate was closed before release and its pidfd remains registered for exact reap"
                ),
            ),
            Err(close_error) => {
                let signal_result = if ticket.pidfd() >= 0 {
                    syscall_result(unsafe {
                        syscall4(
                            NR_PIDFD_SEND_SIGNAL,
                            ticket.pidfd() as usize,
                            SIGKILL,
                            0,
                            0,
                        )
                    })
                    .map(|_| ())
                } else {
                    Err(io::Error::other("retained pidfd is unavailable"))
                };
                let signal_note = match signal_result {
                    Ok(()) => "one pidfd SIGKILL was sent".to_string(),
                    Err(error) => format!(
                        "one pidfd SIGKILL attempt failed or is ambiguous ({error}); it was not retried"
                    ),
                };
                io::Error::other(format!(
                    "{cause}; exec-gate close outcome is unknown ({close_error}) and was not retried; {signal_note}; the child remains registered for exact reap"
                ))
            }
        }
    }

    fn send_child_error(error_fd: i32, errno: usize) -> ! {
        send_child_error_code(error_fd, errno as u32)
    }

    #[derive(Clone, Copy)]
    #[repr(u32)]
    enum ChildSetupFailureStage {
        DescriptorSetup = 1,
        DescriptorCloseOnExec = 2,
        NamespaceIsolation = 3,
        CredentialHardening = 4,
        SignalMaskReset = 5,
        TestSecurityProbe = 6,
        AppStorageMount = 7,
    }

    #[derive(Clone, Copy)]
    struct ChildSetupFailure {
        stage: ChildSetupFailureStage,
        errno: usize,
    }

    impl From<usize> for ChildSetupFailure {
        fn from(errno: usize) -> Self {
            Self {
                stage: ChildSetupFailureStage::NamespaceIsolation,
                errno,
            }
        }
    }

    fn send_child_setup_error(error_fd: i32, stage: ChildSetupFailureStage, errno: usize) -> ! {
        #[cfg(test)]
        let code = (1u32 << 31) | ((stage as u32) << 24) | (errno as u32 & 0xffff);
        #[cfg(not(test))]
        let code = {
            let _ = stage;
            errno as u32
        };
        send_child_error_code(error_fd, code)
    }

    fn send_child_error_code(error_fd: i32, code: u32) -> ! {
        let bytes = code.to_ne_bytes();
        let mut written = 0usize;
        while written < bytes.len() {
            let result = unsafe {
                syscall3(
                    NR_WRITE,
                    error_fd as usize,
                    bytes.as_ptr().add(written) as usize,
                    bytes.len() - written,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result <= 0 {
                break;
            }
            written += result as usize;
        }
        unsafe {
            loop {
                syscall1(NR_EXIT_GROUP, 127);
            }
        }
    }

    #[repr(C)]
    #[derive(Default)]
    struct LinuxCapabilityHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct LinuxCapabilityData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LinuxSockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct LinuxSockFprog {
        len: u16,
        filter: *const LinuxSockFilter,
    }

    const SERVICE_NAMESPACE_SECCOMP_FILTER: [LinuxSockFilter; 23] = [
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 4,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 1,
            jf: 0,
            k: AUDIT_ARCH_X86_64,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_KILL_PROCESS,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: X32_SYSCALL_BIT,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 4,
            k: NR_UNSHARE as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 16,
        },
        LinuxSockFilter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: CLONE_NAMESPACE_FLAGS as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_SETNS as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_CLONE3 as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | ENOSYS as u32,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 4,
            k: NR_CLONE as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 16,
        },
        LinuxSockFilter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: CLONE_NAMESPACE_FLAGS as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ALLOW,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ALLOW,
        },
    ];

    // Installed only after C3 has transferred the one authorized endpoint fd
    // to PID 1. The preceding filter already kills non-x86_64 execution and
    // rejects x32 syscalls. This filter then closes every supported outbound
    // descriptor-passing route and prevents a separate process from inheriting
    // the connected endpoint. CLONE_THREAD remains available for ordinary
    // in-process worker threads; clone3 remains denied by the preceding filter.
    const SERVICE_FD_DELEGATION_SECCOMP_FILTER: [LinuxSockFilter; 31] = [
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_SENDMSG as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_SENDMMSG as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_FORK as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_VFORK as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_IO_URING_SETUP as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_IO_URING_ENTER as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_IO_URING_REGISTER as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: NR_PIDFD_GETFD as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 0,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 0,
            jf: 4,
            k: NR_CLONE as u32,
        },
        LinuxSockFilter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 16,
        },
        LinuxSockFilter {
            code: 0x54,
            jt: 0,
            jf: 0,
            k: SERVICE_CLONE_THREAD_FLAGS,
        },
        LinuxSockFilter {
            code: 0x15,
            jt: 1,
            jf: 0,
            k: SERVICE_CLONE_THREAD_FLAGS,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ERRNO | EPERM as u32,
        },
        LinuxSockFilter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: SECCOMP_RET_ALLOW,
        },
    ];

    fn child_syscall_errno(result: isize) -> usize {
        if result < 0 {
            (-result) as usize
        } else {
            0
        }
    }

    fn install_service_namespace_seccomp() -> Result<(), usize> {
        let mut program = LinuxSockFprog {
            len: SERVICE_NAMESPACE_SECCOMP_FILTER.len() as u16,
            filter: SERVICE_NAMESPACE_SECCOMP_FILTER.as_ptr(),
        };
        let installed = unsafe {
            syscall3(
                NR_SECCOMP,
                SECCOMP_SET_MODE_FILTER,
                0,
                &mut program as *mut LinuxSockFprog as usize,
            )
        };
        if installed < 0 {
            return Err(child_syscall_errno(installed));
        }
        let mode = unsafe { syscall5(NR_PRCTL, PR_GET_SECCOMP, 0, 0, 0, 0) };
        if mode != SECCOMP_MODE_FILTER {
            return Err(if mode < 0 {
                child_syscall_errno(mode)
            } else {
                EPERM as usize
            });
        }
        Ok(())
    }

    fn install_app_landlock() -> Result<(), usize> {
        let version = unsafe {
            syscall3(
                NR_LANDLOCK_CREATE_RULESET,
                0,
                0,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        if version == -(ENOSYS) || version == -(EOPNOTSUPP) {
            return Ok(());
        }
        if version < 0 {
            return Err(child_syscall_errno(version));
        }
        if version == 0 {
            return Err(EINVAL as usize);
        }

        let rights = landlock_filesystem_rights((version as u32).min(LANDLOCK_KNOWN_ABI_MAX));
        let ruleset_attributes = LinuxLandlockRulesetAttr {
            handled_access_fs: rights,
        };
        let ruleset_fd = unsafe {
            syscall3(
                NR_LANDLOCK_CREATE_RULESET,
                &ruleset_attributes as *const LinuxLandlockRulesetAttr as usize,
                core::mem::size_of::<LinuxLandlockRulesetAttr>(),
                0,
            )
        };
        if ruleset_fd < 0 {
            return Err(child_syscall_errno(ruleset_fd));
        }

        let root_fd = unsafe {
            syscall4(
                NR_OPENAT,
                AT_FDCWD,
                c"/".as_ptr() as usize,
                O_PATH | O_CLOEXEC,
                0,
            )
        };
        if root_fd < 0 {
            return Err(child_syscall_errno(root_fd));
        }

        // The chroot contains only noexec tmpfs plus this app-owned noexec
        // data mount. Landlock limits filesystem access here; execution of
        // the sealed package memfd uses the exact execveat path.
        let root_rule = LinuxLandlockPathBeneathAttr {
            allowed_access: rights,
            parent_fd: root_fd as i32,
            reserved: 0,
        };
        let root_added = unsafe {
            syscall4(
                NR_LANDLOCK_ADD_RULE,
                ruleset_fd as usize,
                LANDLOCK_RULE_PATH_BENEATH,
                &root_rule as *const LinuxLandlockPathBeneathAttr as usize,
                0,
            )
        };
        if root_added < 0 {
            return Err(child_syscall_errno(root_added));
        }

        let restricted = unsafe { syscall2(NR_LANDLOCK_RESTRICT_SELF, ruleset_fd as usize, 0) };
        if restricted < 0 {
            return Err(child_syscall_errno(restricted));
        }
        for fd in [root_fd as i32, ruleset_fd as i32] {
            let closed = unsafe { syscall1(NR_CLOSE, fd as usize) };
            if closed < 0 {
                return Err(child_syscall_errno(closed));
            }
        }
        Ok(())
    }

    fn install_service_fd_delegation_seccomp() -> Result<(), usize> {
        let mut program = LinuxSockFprog {
            len: SERVICE_FD_DELEGATION_SECCOMP_FILTER.len() as u16,
            filter: SERVICE_FD_DELEGATION_SECCOMP_FILTER.as_ptr(),
        };
        let installed = unsafe {
            syscall3(
                NR_SECCOMP,
                SECCOMP_SET_MODE_FILTER,
                0,
                &mut program as *mut LinuxSockFprog as usize,
            )
        };
        if installed < 0 {
            return Err(child_syscall_errno(installed));
        }
        let mode = unsafe { syscall5(NR_PRCTL, PR_GET_SECCOMP, 0, 0, 0, 0) };
        if mode != SECCOMP_MODE_FILTER {
            return Err(if mode < 0 {
                child_syscall_errno(mode)
            } else {
                EPERM as usize
            });
        }
        Ok(())
    }

    fn prctl_capability_bounding_state(capability: usize) -> isize {
        unsafe { syscall5(NR_PRCTL, PR_CAPBSET_READ, capability, 0, 0, 0) }
    }

    fn drop_service_capability_bounding_set() -> Result<(), usize> {
        let mut supported = 0usize;
        for capability in 0..MAX_LINUX_CAPABILITIES {
            let current = prctl_capability_bounding_state(capability);
            if current == -EINVAL {
                continue;
            }
            if current < 0 {
                return Err(child_syscall_errno(current));
            }
            if current > 1 {
                return Err(EINVAL as usize);
            }
            supported += 1;
            if current == 1 {
                let dropped = unsafe { syscall5(NR_PRCTL, PR_CAPBSET_DROP, capability, 0, 0, 0) };
                if dropped < 0 {
                    return Err(child_syscall_errno(dropped));
                }
            }
            if prctl_capability_bounding_state(capability) != 0 {
                return Err(EPERM as usize);
            }
        }
        if supported == 0 {
            return Err(EINVAL as usize);
        }
        let beyond_v3 = prctl_capability_bounding_state(MAX_LINUX_CAPABILITIES);
        if beyond_v3 != -EINVAL {
            return Err(if beyond_v3 < 0 {
                child_syscall_errno(beyond_v3)
            } else {
                EOVERFLOW as usize
            });
        }
        Ok(())
    }

    fn verify_service_capability_bounding_set_empty() -> Result<(), usize> {
        let mut supported = 0usize;
        for capability in 0..MAX_LINUX_CAPABILITIES {
            let current = prctl_capability_bounding_state(capability);
            if current == -EINVAL {
                continue;
            }
            if current < 0 {
                return Err(child_syscall_errno(current));
            }
            supported += 1;
            if current != 0 {
                return Err(EPERM as usize);
            }
        }
        if supported == 0 {
            return Err(EINVAL as usize);
        }
        let beyond_v3 = prctl_capability_bounding_state(MAX_LINUX_CAPABILITIES);
        if beyond_v3 != -EINVAL {
            return Err(if beyond_v3 < 0 {
                child_syscall_errno(beyond_v3)
            } else {
                EOVERFLOW as usize
            });
        }
        Ok(())
    }

    fn verify_service_capability_sets_empty() -> Result<(), usize> {
        let mut header = LinuxCapabilityHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mut data = [LinuxCapabilityData::default(); LINUX_CAPABILITY_U32S_3];
        let result = unsafe {
            syscall2(
                NR_CAPGET,
                &mut header as *mut LinuxCapabilityHeader as usize,
                data.as_mut_ptr() as usize,
            )
        };
        if result < 0 {
            return Err(child_syscall_errno(result));
        }
        if header.version != LINUX_CAPABILITY_VERSION_3
            || data
                .iter()
                .any(|word| word.effective != 0 || word.permitted != 0 || word.inheritable != 0)
        {
            return Err(EPERM as usize);
        }
        Ok(())
    }

    fn verify_service_ambient_set_empty() -> Result<(), usize> {
        let mut supported = 0usize;
        for capability in 0..MAX_LINUX_CAPABILITIES {
            let ambient = unsafe {
                syscall5(
                    NR_PRCTL,
                    PR_CAP_AMBIENT,
                    PR_CAP_AMBIENT_IS_SET,
                    capability,
                    0,
                    0,
                )
            };
            if ambient == -EINVAL {
                continue;
            }
            if ambient < 0 {
                return Err(child_syscall_errno(ambient));
            }
            supported += 1;
            if ambient != 0 {
                return Err(EPERM as usize);
            }
        }
        if supported == 0 {
            return Err(EINVAL as usize);
        }
        let beyond_v3 = unsafe {
            syscall5(
                NR_PRCTL,
                PR_CAP_AMBIENT,
                PR_CAP_AMBIENT_IS_SET,
                MAX_LINUX_CAPABILITIES,
                0,
                0,
            )
        };
        if beyond_v3 != -EINVAL {
            return Err(if beyond_v3 < 0 {
                child_syscall_errno(beyond_v3)
            } else {
                EOVERFLOW as usize
            });
        }
        Ok(())
    }

    fn verify_service_securebits() -> Result<(), usize> {
        let securebits = unsafe { syscall5(NR_PRCTL, PR_GET_SECUREBITS, 0, 0, 0, 0) };
        if securebits < 0 {
            return Err(child_syscall_errno(securebits));
        }
        if securebits as usize != SERVICE_SECUREBITS {
            return Err(EPERM as usize);
        }
        Ok(())
    }

    fn verify_service_identity(uid: u32, gid: u32) -> Result<(), usize> {
        let mut real_uid = u32::MAX;
        let mut effective_uid = u32::MAX;
        let mut saved_uid = u32::MAX;
        let uids = unsafe {
            syscall3(
                NR_GETRESUID,
                &mut real_uid as *mut u32 as usize,
                &mut effective_uid as *mut u32 as usize,
                &mut saved_uid as *mut u32 as usize,
            )
        };
        if uids < 0 {
            return Err(child_syscall_errno(uids));
        }
        let mut real_gid = u32::MAX;
        let mut effective_gid = u32::MAX;
        let mut saved_gid = u32::MAX;
        let gids = unsafe {
            syscall3(
                NR_GETRESGID,
                &mut real_gid as *mut u32 as usize,
                &mut effective_gid as *mut u32 as usize,
                &mut saved_gid as *mut u32 as usize,
            )
        };
        if gids < 0 {
            return Err(child_syscall_errno(gids));
        }
        let groups = unsafe { syscall2(NR_GETGROUPS, 0, 0) };
        if groups < 0 {
            return Err(child_syscall_errno(groups));
        }
        // Linux setresuid/setresgid normally synchronize the filesystem IDs
        // with the effective IDs. Read the actual values back as well so setup
        // cannot become ready if an LSM or kernel behavior leaves them apart.
        let fs_uid = unsafe { syscall1(NR_SETFSUID, u32::MAX as usize) };
        if fs_uid < 0 {
            return Err(child_syscall_errno(fs_uid));
        }
        let fs_gid = unsafe { syscall1(NR_SETFSGID, u32::MAX as usize) };
        if fs_gid < 0 {
            return Err(child_syscall_errno(fs_gid));
        }
        if [real_uid, effective_uid, saved_uid] != [uid; 3]
            || [real_gid, effective_gid, saved_gid] != [gid; 3]
            || fs_uid as usize != uid as usize
            || fs_gid as usize != gid as usize
            || groups != 0
        {
            return Err(EPERM as usize);
        }
        Ok(())
    }

    /// Apply irreversible Linux credential hardening in the clone3 child
    /// before it can report setup-ready or consume the exec release byte.
    ///
    /// # Safety
    /// Must run only in the child branch of `clone3_pid1_service_child()`.
    /// It permanently changes the calling process credentials and capability
    /// state; any error requires the caller to exit without executing a file.
    unsafe fn harden_service_child_credentials(
        uid: u32,
        gid: u32,
        app_filesystem: bool,
    ) -> Result<(), usize> {
        if uid == 0 || gid == 0 || uid == u32::MAX || gid == u32::MAX {
            return Err(EINVAL as usize);
        }
        let groups_dropped = syscall2(NR_SETGROUPS, 0, 0);
        if groups_dropped < 0 {
            return Err(child_syscall_errno(groups_dropped));
        }
        let group_ids_dropped = syscall3(NR_SETRESGID, gid as usize, gid as usize, gid as usize);
        if group_ids_dropped < 0 {
            return Err(child_syscall_errno(group_ids_dropped));
        }

        let securebits_set = syscall5(NR_PRCTL, PR_SET_SECUREBITS, SERVICE_SECUREBITS, 0, 0, 0);
        if securebits_set < 0 {
            return Err(child_syscall_errno(securebits_set));
        }

        let ambient_cleared = syscall5(NR_PRCTL, PR_CAP_AMBIENT, PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0);
        if ambient_cleared < 0 {
            return Err(child_syscall_errno(ambient_cleared));
        }
        drop_service_capability_bounding_set()?;

        let user_ids_dropped = syscall3(NR_SETRESUID, uid as usize, uid as usize, uid as usize);
        if user_ids_dropped < 0 {
            return Err(child_syscall_errno(user_ids_dropped));
        }

        let mut header = LinuxCapabilityHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let data = [LinuxCapabilityData::default(); LINUX_CAPABILITY_U32S_3];
        let capabilities_cleared = syscall2(
            NR_CAPSET,
            &mut header as *mut LinuxCapabilityHeader as usize,
            data.as_ptr() as usize,
        );
        if capabilities_cleared < 0 {
            return Err(child_syscall_errno(capabilities_cleared));
        }

        verify_service_identity(uid, gid)?;
        verify_service_securebits()?;
        verify_service_capability_sets_empty()?;
        verify_service_capability_bounding_set_empty()?;
        verify_service_ambient_set_empty()?;

        let no_new_privileges = syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        if no_new_privileges < 0 {
            return Err(child_syscall_errno(no_new_privileges));
        }
        let no_new_privileges_readback = syscall5(NR_PRCTL, PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0);
        if no_new_privileges_readback != 1 {
            return Err(if no_new_privileges_readback < 0 {
                child_syscall_errno(no_new_privileges_readback)
            } else {
                EPERM as usize
            });
        }
        if app_filesystem {
            install_app_landlock()?;
        }
        install_service_namespace_seccomp()?;
        Ok(())
    }

    /// Give endpoint-bearing system services a private mount/network/IPC/UTS/
    /// cgroup namespace and a tmpfs-only root before credentials and all
    /// namespace-changing authority are dropped. The exact C2 Task24 listener,
    /// when present, is the only pathname projected into that root. Their only
    /// inherited C3 service channel remains fd 3. Every setup error aborts
    /// before exec.
    ///
    /// # Safety
    /// Must run only in the still-privileged child branch of
    /// `clone3_pid1_service_child()`. It permanently changes the calling
    /// process's namespaces and root directory.
    unsafe fn bind_task24_endpoint_into_service_root(
        source: *const i8,
        target: *const i8,
    ) -> Result<(), usize> {
        for directory in [
            b"/tmp/run\0".as_slice(),
            b"/tmp/run/nagi\0".as_slice(),
            b"/tmp/run/nagi/ipc\0".as_slice(),
        ] {
            let created = unsafe { syscall2(NR_MKDIR, directory.as_ptr() as usize, 0o755) };
            if created < 0 {
                return Err(child_syscall_errno(created));
            }
        }

        // The mount target must exist. It is created in the fresh private
        // tmpfs, then replaced by a bind mount of this one C2 socket inode.
        let placeholder = unsafe {
            syscall4(
                NR_OPENAT,
                AT_FDCWD,
                target as usize,
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC,
                0o600,
            )
        };
        if placeholder < 0 {
            return Err(child_syscall_errno(placeholder));
        }
        let closed = unsafe { syscall1(NR_CLOSE, placeholder as usize) };
        if closed < 0 {
            return Err(child_syscall_errno(closed));
        }
        let mounted =
            unsafe { syscall5(NR_MOUNT, source as usize, target as usize, 0, MS_BIND, 0) };
        if mounted < 0 {
            return Err(child_syscall_errno(mounted));
        }
        Ok(())
    }

    unsafe fn bind_app_storage_mount_into_service_root(mount_tree_fd: i32) -> Result<(), usize> {
        if mount_tree_fd < 0 {
            return Err(EINVAL as usize);
        }
        let target = c"/tmp/data";
        let created = unsafe { syscall2(NR_MKDIR, target.as_ptr() as usize, 0o755) };
        if created < 0 {
            return Err(child_syscall_errno(created));
        }
        let attached = unsafe {
            syscall5(
                NR_MOVE_MOUNT,
                mount_tree_fd as usize,
                c"".as_ptr() as usize,
                AT_FDCWD,
                target.as_ptr() as usize,
                MOVE_MOUNT_F_EMPTY_PATH,
            )
        };
        if attached < 0 {
            return Err(child_syscall_errno(attached));
        }
        let closed = unsafe { syscall1(NR_CLOSE, mount_tree_fd as usize) };
        if closed < 0 {
            return Err(child_syscall_errno(closed));
        }
        Ok(())
    }

    unsafe fn isolate_system_service_child(
        task24_endpoint_source: *const i8,
        task24_endpoint_target: *const i8,
        app_storage_mount_fd: i32,
    ) -> Result<(), ChildSetupFailure> {
        if task24_endpoint_source.is_null() != task24_endpoint_target.is_null()
            || (!task24_endpoint_source.is_null() && app_storage_mount_fd >= 0)
        {
            return Err((EINVAL as usize).into());
        }
        let namespaces = unsafe { syscall1(NR_UNSHARE, SERVICE_NAMESPACE_FLAGS) };
        if namespaces < 0 {
            return Err(child_syscall_errno(namespaces).into());
        }

        // The child must not propagate mounts back to PID 1 or sibling
        // services. Failure is fatal; a shared-propagation mount is not an
        // acceptable partial sandbox.
        let private_root = unsafe {
            syscall5(
                NR_MOUNT,
                0,
                c"/".as_ptr() as usize,
                0,
                MS_REC | MS_PRIVATE,
                0,
            )
        };
        if private_root < 0 {
            return Err(child_syscall_errno(private_root).into());
        }

        unsafe { mount_service_tmpfs_at(SERVICE_ROOT_PATH.as_ptr().cast()) }
            .map_err(ChildSetupFailure::from)?;

        if !task24_endpoint_source.is_null() {
            unsafe {
                bind_task24_endpoint_into_service_root(
                    task24_endpoint_source,
                    task24_endpoint_target,
                )?
            };
        }

        if app_storage_mount_fd >= 0 {
            unsafe { bind_app_storage_mount_into_service_root(app_storage_mount_fd) }.map_err(
                |errno| ChildSetupFailure {
                    stage: ChildSetupFailureStage::AppStorageMount,
                    errno,
                },
            )?;
        }

        let entered_root = unsafe { syscall1(NR_CHDIR, SERVICE_ROOT_PATH.as_ptr() as usize) };
        if entered_root < 0 {
            return Err(child_syscall_errno(entered_root).into());
        }
        // Keep the service root searchable after dropping root credentials,
        // while giving it a separate bounded tmpfs scratch directory.
        let service_tmp = unsafe { syscall2(NR_MKDIR, c"tmp".as_ptr() as usize, 0o1777) };
        if service_tmp < 0 {
            return Err(child_syscall_errno(service_tmp).into());
        }
        let chrooted = unsafe { syscall1(NR_CHROOT, c".".as_ptr() as usize) };
        if chrooted < 0 {
            return Err(child_syscall_errno(chrooted).into());
        }
        Ok(())
    }

    unsafe fn close_child_fds_except(keep_fds: [i32; 3]) -> Result<(), usize> {
        let mut keep_fds = keep_fds;
        keep_fds.sort_unstable();
        let mut next_fd = 3u64;
        for fd in keep_fds {
            if fd < 3 || (fd as u64) < next_fd {
                continue;
            }
            let fd = fd as u32;
            if next_fd < fd as u64 {
                let closed =
                    unsafe { syscall3(NR_CLOSE_RANGE, next_fd as usize, (fd - 1) as usize, 0) };
                if closed < 0 {
                    return Err(child_syscall_errno(closed));
                }
            }
            next_fd = fd as u64 + 1;
        }
        if next_fd <= u32::MAX as u64 {
            let closed =
                unsafe { syscall3(NR_CLOSE_RANGE, next_fd as usize, u32::MAX as usize, 0) };
            if closed < 0 {
                return Err(child_syscall_errno(closed));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    unsafe fn install_post_setup_inheritable_test_fd(keep_fds: [i32; 3]) -> Result<(), usize> {
        for test_fd in APP_ISOLATION_TEST_FD_CANDIDATES {
            if keep_fds.contains(&test_fd) {
                continue;
            }
            // This deliberately undoes the earlier CLOSE_RANGE_CLOEXEC for a
            // descriptor created after sandbox setup. The app fixture proves
            // the final close sweep, rather than relying on exec-time CLOEXEC.
            let duplicated = unsafe { syscall3(NR_DUP3, 0, test_fd as usize, 0) };
            if duplicated >= 0 {
                return Ok(());
            }
            let errno = child_syscall_errno(duplicated);
            if errno != EBADF {
                return Err(errno);
            }
        }
        Err(EMFILE)
    }

    // These are the exact descriptors and argv/envp crossing the post-clone
    // exec boundary. Keeping them explicit avoids a wrapper with misleading
    // child-side close semantics.
    #[allow(clippy::too_many_arguments)]
    unsafe fn child_wait_and_exec(
        gate_parent: i32,
        gate_child: i32,
        error_read: i32,
        error_write: i32,
        null_fd: i32,
        executable_fd: i32,
        task24_endpoint_source: *const i8,
        task24_endpoint_target: *const i8,
        app_storage_mount_fd: i32,
        private_filesystem: bool,
        app_filesystem: bool,
        argv: *const *const i8,
        envp: *const *const i8,
        execution_identity: LinuxExecutionIdentity,
        endpoint_policy: ProcessEndpointPolicy,
    ) -> ! {
        unsafe {
            syscall1(NR_CLOSE, gate_parent as usize);
            syscall1(NR_CLOSE, error_read as usize);
        }

        unsafe {
            for target in 0..3usize {
                // The dedicated Notes development smoke writes its actual
                // startup and input errors to the inherited QEMU console.
                // Keep stdin detached and leave every other process on the
                // normal null-stdio path. Production builds never inherit it.
                if target != 0 && inherit_child_console_output(execution_identity) {
                    continue;
                }
                let result = syscall2(NR_DUP2, null_fd as usize, target);
                if result < 0 {
                    send_child_setup_error(
                        error_write,
                        ChildSetupFailureStage::DescriptorSetup,
                        (-result) as usize,
                    );
                }
            }
            syscall1(NR_CLOSE, null_fd as usize);
            let closed = syscall3(NR_CLOSE_RANGE, 3, u32::MAX as usize, CLOSE_RANGE_CLOEXEC);
            if closed < 0 {
                send_child_setup_error(
                    error_write,
                    ChildSetupFailureStage::DescriptorCloseOnExec,
                    (-closed) as usize,
                );
            }
            if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel || private_filesystem
            {
                let (endpoint_source, endpoint_target) =
                    if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel {
                        (task24_endpoint_source, task24_endpoint_target)
                    } else {
                        (ptr::null(), ptr::null())
                    };
                if let Err(failure) = isolate_system_service_child(
                    endpoint_source,
                    endpoint_target,
                    app_storage_mount_fd,
                ) {
                    send_child_setup_error(error_write, failure.stage, failure.errno);
                }
            }
            if let Err(errno) = harden_service_child_credentials(
                execution_identity.uid,
                execution_identity.gid,
                app_filesystem,
            ) {
                send_child_setup_error(
                    error_write,
                    ChildSetupFailureStage::CredentialHardening,
                    errno,
                );
            }
            // PID 1 blocks SIGCHLD for signalfd-based reaping. Services inherit
            // that mask across clone; keep SIGCHLD blocked until all credential
            // and seccomp hardening has completed. clone3 clears caught handlers
            // before the child can run this setup path.
            let unblocked_signals = 0u64;
            let signals_unblocked = syscall4(
                NR_RT_SIGPROCMASK,
                SIG_SETMASK,
                &unblocked_signals as *const u64 as usize,
                0,
                core::mem::size_of::<u64>(),
            );
            if signals_unblocked < 0 {
                send_child_setup_error(
                    error_write,
                    ChildSetupFailureStage::SignalMaskReset,
                    (-signals_unblocked) as usize,
                );
            }
            #[cfg(test)]
            if let Err(errno) =
                child_credential_negative_probe(execution_identity.uid, execution_identity.gid)
            {
                send_child_setup_error(
                    error_write,
                    ChildSetupFailureStage::TestSecurityProbe,
                    errno,
                );
            }
            if !send_child_setup_ready(error_write) {
                loop {
                    syscall1(NR_EXIT_GROUP, 126);
                }
            }
        }

        let mut command = 0u8;
        loop {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    gate_child as usize,
                    &mut command as *mut u8 as usize,
                    1,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result != 1 || command != RELEASE_BYTE {
                unsafe {
                    loop {
                        syscall1(NR_EXIT_GROUP, 125);
                    }
                }
            }
            break;
        }

        unsafe {
            if endpoint_policy == ProcessEndpointPolicy::Disabled {
                syscall1(NR_CLOSE, gate_child as usize);
            }

            let mut exec_fd = executable_fd;
            if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel {
                // The service endpoint occupies fd 3 after exec. Preserve the
                // sealed executable above the ordinary descriptor range first.
                let preserved_exec_fd = syscall3(NR_FCNTL, exec_fd as usize, F_DUPFD_CLOEXEC, 64);
                if preserved_exec_fd < 0 {
                    send_child_error(error_write, (-preserved_exec_fd) as usize);
                }
                syscall1(NR_CLOSE, exec_fd as usize);
                exec_fd = preserved_exec_fd as i32;

                let mut endpoint_fds = [-1i32; 2];
                let endpoint_pair = syscall4(
                    NR_SOCKETPAIR,
                    AF_UNIX,
                    SOCK_STREAM | SOCK_CLOEXEC,
                    0,
                    endpoint_fds.as_mut_ptr() as usize,
                );
                if endpoint_pair < 0 {
                    send_child_error(error_write, (-endpoint_pair) as usize);
                }
                let service_endpoint = endpoint_fds[0];
                let owner_endpoint = endpoint_fds[1];
                if service_endpoint == SERVICE_ENDPOINT_FD {
                    let inheritable = syscall3(NR_FCNTL, service_endpoint as usize, F_SETFD, 0);
                    if inheritable < 0 {
                        send_child_error(error_write, (-inheritable) as usize);
                    }
                } else {
                    let duplicated = syscall3(
                        NR_DUP3,
                        service_endpoint as usize,
                        SERVICE_ENDPOINT_FD as usize,
                        0,
                    );
                    if duplicated < 0 {
                        send_child_error(error_write, (-duplicated) as usize);
                    }
                    syscall1(NR_CLOSE, service_endpoint as usize);
                }

                let sent = send_endpoint_descriptor(gate_child, owner_endpoint);
                syscall1(NR_CLOSE, owner_endpoint as usize);
                syscall1(NR_CLOSE, gate_child as usize);
                if sent != 1 {
                    send_child_error(error_write, if sent < 0 { (-sent) as usize } else { 5 });
                }
                if let Err(errno) = install_service_fd_delegation_seccomp() {
                    send_child_error(error_write, errno);
                }
            }

            // CLOEXEC was set on inherited descriptors before setup. Close all
            // remaining unapproved descriptors now as well, including any
            // opened during setup; the only exec-bound descriptors are the
            // sealed ELF, the CLOEXEC error pipe, and (for services) fd 3.
            let delegated_endpoint_fd =
                if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel {
                    SERVICE_ENDPOINT_FD
                } else {
                    -1
                };
            #[cfg(test)]
            if app_filesystem {
                if let Err(errno) = install_post_setup_inheritable_test_fd([
                    exec_fd,
                    error_write,
                    delegated_endpoint_fd,
                ]) {
                    send_child_error(error_write, errno);
                }
            }
            if let Err(errno) =
                close_child_fds_except([exec_fd, error_write, delegated_endpoint_fd])
            {
                send_child_error(error_write, errno);
            }
            if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel
                && !send_child_fd_delegation_ready(error_write)
            {
                loop {
                    syscall1(NR_EXIT_GROUP, 126);
                }
            }

            let empty_path = b"\0";
            let executed = syscall5(
                NR_EXECVEAT,
                exec_fd as usize,
                empty_path.as_ptr() as usize,
                argv as usize,
                envp as usize,
                AT_EMPTY_PATH,
            );
            let errno = if executed < 0 {
                (-executed) as usize
            } else {
                5
            };
            send_child_error(error_write, errno);
        }
    }

    fn send_child_setup_ready(error_fd: i32) -> bool {
        let bytes = CHILD_SETUP_READY_WORD.to_ne_bytes();
        let mut written = 0usize;
        while written < bytes.len() {
            let result = unsafe {
                syscall3(
                    NR_WRITE,
                    error_fd as usize,
                    bytes.as_ptr().add(written) as usize,
                    bytes.len() - written,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result <= 0 {
                return false;
            }
            written += result as usize;
        }
        true
    }

    fn send_child_fd_delegation_ready(error_fd: i32) -> bool {
        let bytes = CHILD_FD_DELEGATION_READY_WORD.to_ne_bytes();
        let mut written = 0usize;
        while written < bytes.len() {
            let result = unsafe {
                syscall3(
                    NR_WRITE,
                    error_fd as usize,
                    bytes.as_ptr().add(written) as usize,
                    bytes.len() - written,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result <= 0 {
                return false;
            }
            written += result as usize;
        }
        true
    }

    fn await_child_setup_ready(error_fd: i32, budget: std::time::Duration) -> io::Result<()> {
        let started = std::time::Instant::now();
        let mut bytes = [0u8; core::mem::size_of::<u32>()];
        let mut received = 0usize;
        loop {
            if started.elapsed() >= budget {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service child credential setup did not report readiness",
                ));
            }
            let result = unsafe {
                syscall3(
                    NR_READ,
                    error_fd as usize,
                    bytes.as_mut_ptr().add(received) as usize,
                    bytes.len() - received,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result == -(EAGAIN) {
                let remaining = budget.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    continue;
                }
                let rounded_ms = remaining.as_millis().saturating_add(u128::from(
                    !remaining.subsec_nanos().is_multiple_of(1_000_000),
                ));
                let timeout_ms = rounded_ms.max(1).min(i32::MAX as u128) as i32;
                let mut descriptor = PollFd {
                    fd: error_fd,
                    events: POLLIN,
                    revents: 0,
                };
                let ready = unsafe {
                    syscall3(
                        NR_POLL,
                        &mut descriptor as *mut PollFd as usize,
                        1,
                        timeout_ms as usize,
                    )
                };
                if ready == -(EINTR) {
                    continue;
                }
                if ready < 0 {
                    return Err(io::Error::from_raw_os_error((-ready) as i32));
                }
                if descriptor.revents & POLLNVAL != 0 {
                    return Err(io::Error::from_raw_os_error(9));
                }
                continue;
            }
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            if result == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    if received == 0 {
                        "service child exited before credential setup readiness"
                    } else {
                        "service child sent a partial credential setup result"
                    },
                ));
            }
            received += result as usize;
            if received != bytes.len() {
                continue;
            }
            let word = u32::from_ne_bytes(bytes);
            if word == CHILD_SETUP_READY_WORD {
                return Ok(());
            }
            #[cfg(test)]
            if word & (1u32 << 31) != 0 {
                let stage = (word >> 24) & 0x7f;
                let errno = (word & 0xffff) as i32;
                let stage_name = match stage {
                    1 => "descriptor setup",
                    2 => "close-on-exec descriptor setup",
                    3 => "namespace/root isolation",
                    4 => "credential/capability/seccomp hardening",
                    5 => "signal mask reset",
                    6 => "test security negative probe",
                    7 => "app storage mount",
                    _ => "unknown child setup stage",
                };
                eprintln!("C3_CLONE3_CHILD_SETUP_FAILED: stage={stage_name} errno={errno}");
                return Err(io::Error::from_raw_os_error(errno));
            }
            if word == 0 || word > i32::MAX as u32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "service child sent an invalid credential setup result",
                ));
            }
            return Err(io::Error::from_raw_os_error(word as i32));
        }
    }

    #[cfg(test)]
    fn child_credential_negative_probe(uid: u32, gid: u32) -> Result<(), usize> {
        let mut header = LinuxCapabilityHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0,
        };
        let mut data = [LinuxCapabilityData::default(); LINUX_CAPABILITY_U32S_3];
        data[0].effective = 1 << 7; // CAP_SETUID
        data[0].permitted = 1 << 7;
        let capset = unsafe {
            syscall2(
                NR_CAPSET,
                &mut header as *mut LinuxCapabilityHeader as usize,
                data.as_ptr() as usize,
            )
        };
        if capset != -EPERM {
            return Err(if capset < 0 {
                child_syscall_errno(capset)
            } else {
                EPERM as usize
            });
        }

        let set_uid = unsafe { syscall3(NR_SETRESUID, 0, 0, 0) };
        if set_uid != -EPERM {
            return Err(if set_uid < 0 {
                child_syscall_errno(set_uid)
            } else {
                EPERM as usize
            });
        }
        let set_gid = unsafe { syscall3(NR_SETRESGID, 0, 0, 0) };
        if set_gid != -EPERM {
            return Err(if set_gid < 0 {
                child_syscall_errno(set_gid)
            } else {
                EPERM as usize
            });
        }
        let changed_securebits = SERVICE_SECUREBITS & !SECBIT_NOROOT_LOCKED;
        let securebits =
            unsafe { syscall5(NR_PRCTL, PR_SET_SECUREBITS, changed_securebits, 0, 0, 0) };
        if securebits != -EPERM {
            return Err(if securebits < 0 {
                child_syscall_errno(securebits)
            } else {
                EPERM as usize
            });
        }

        let mut sockets = [-1i32; 2];
        let pair = unsafe {
            syscall4(
                NR_SOCKETPAIR,
                AF_UNIX,
                SOCK_DGRAM | SOCK_CLOEXEC,
                0,
                sockets.as_mut_ptr() as usize,
            )
        };
        if pair < 0 {
            return Err(child_syscall_errno(pair));
        }
        let own_pid = unsafe { syscall1(NR_GETPID, 0) };
        if own_pid <= 0 || own_pid > i32::MAX as isize {
            unsafe {
                syscall1(NR_CLOSE, sockets[0] as usize);
                syscall1(NR_CLOSE, sockets[1] as usize);
            }
            return Err(EINVAL as usize);
        }
        let rejected_pid_forge = send_test_scm_credentials(
            sockets[0],
            UnixPeerCredentials {
                process_id: 0,
                uid,
                gid,
            },
        );
        let rejected_uid_gid_forge = send_test_scm_credentials(
            sockets[0],
            UnixPeerCredentials {
                process_id: own_pid as i32,
                uid: 0,
                gid: 0,
            },
        );
        unsafe {
            syscall1(NR_CLOSE, sockets[0] as usize);
            syscall1(NR_CLOSE, sockets[1] as usize);
        }
        if rejected_pid_forge != -EPERM || rejected_uid_gid_forge != -EPERM {
            return Err(EPERM as usize);
        }

        let seccomp_mode = unsafe { syscall5(NR_PRCTL, PR_GET_SECCOMP, 0, 0, 0, 0) };
        if seccomp_mode != 2 {
            return Err(EPERM as usize);
        }
        let unshare = unsafe { syscall1(NR_UNSHARE, CLONE_NEWUSER) };
        if unshare != -EPERM {
            return Err(if unshare < 0 {
                child_syscall_errno(unshare)
            } else {
                EPERM as usize
            });
        }
        let setns = unsafe { syscall2(NR_SETNS, usize::MAX, CLONE_NEWUSER) };
        if setns != -EPERM {
            return Err(if setns < 0 {
                child_syscall_errno(setns)
            } else {
                EPERM as usize
            });
        }
        let invalid_thread_namespace_clone =
            CLONE_NEWUSER | 0x0000_0100 | 0x0000_0800 | 0x0001_0000;
        let clone = unsafe { syscall5(NR_CLONE, invalid_thread_namespace_clone, 0, 0, 0, 0) };
        if clone != -EPERM {
            return Err(if clone < 0 {
                child_syscall_errno(clone)
            } else {
                EPERM as usize
            });
        }
        let x32_unshare = unsafe { syscall1(X32_SYSCALL_BIT as usize | NR_UNSHARE, CLONE_NEWUSER) };
        if x32_unshare != -EPERM {
            return Err(if x32_unshare < 0 {
                child_syscall_errno(x32_unshare)
            } else {
                EPERM as usize
            });
        }
        let clone3 = unsafe { syscall2(NR_CLONE3, 0, 0) };
        if clone3 != -ENOSYS {
            return Err(if clone3 < 0 {
                child_syscall_errno(clone3)
            } else {
                EPERM as usize
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn send_test_scm_credentials(fd: i32, credentials: UnixPeerCredentials) -> isize {
        let mut payload = 1u8;
        let mut vector = IoVec {
            base: &mut payload,
            len: 1,
        };
        let mut control = EndpointControlBuffer([0; ENDPOINT_CONTROL_BYTES]);
        let header_size = core::mem::size_of::<ControlMessageHeader>();
        let credential_size = core::mem::size_of::<UnixPeerCredentials>();
        let cmsg_space = (header_size + credential_size + 7) & !7;
        unsafe {
            ptr::write(
                control.0.as_mut_ptr().cast::<ControlMessageHeader>(),
                ControlMessageHeader {
                    len: header_size + credential_size,
                    level: SOL_SOCKET,
                    kind: SCM_CREDENTIALS,
                },
            );
            ptr::write_unaligned(
                control
                    .0
                    .as_mut_ptr()
                    .add(header_size)
                    .cast::<UnixPeerCredentials>(),
                credentials,
            );
        }
        let mut message = MessageHeader {
            name: ptr::null_mut(),
            name_len: 0,
            iov: &mut vector,
            iov_len: 1,
            control: control.0.as_mut_ptr(),
            control_len: cmsg_space,
            flags: 0,
        };
        unsafe {
            syscall3(
                NR_SENDMSG,
                fd as usize,
                &mut message as *mut MessageHeader as usize,
                MSG_NOSIGNAL,
            )
        }
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    #[repr(C)]
    struct IoVec {
        base: *mut u8,
        len: usize,
    }

    #[repr(C)]
    struct MessageHeader {
        name: *mut u8,
        name_len: u32,
        iov: *mut IoVec,
        iov_len: usize,
        control: *mut u8,
        control_len: usize,
        flags: i32,
    }

    #[repr(C)]
    struct ControlMessageHeader {
        len: usize,
        level: i32,
        kind: i32,
    }

    #[repr(C)]
    struct UnixPeerCredentials {
        process_id: i32,
        uid: u32,
        gid: u32,
    }

    fn endpoint_peer_credentials(fd: i32) -> io::Result<UnixPeerCredentials> {
        let mut credentials = UnixPeerCredentials {
            process_id: -1,
            uid: u32::MAX,
            gid: u32::MAX,
        };
        let mut length = core::mem::size_of::<UnixPeerCredentials>() as u32;
        let result = unsafe {
            syscall5(
                NR_GETSOCKOPT,
                fd as usize,
                SOL_SOCKET as usize,
                SO_PEERCRED,
                &mut credentials as *mut UnixPeerCredentials as usize,
                &mut length as *mut u32 as usize,
            )
        };
        if result < 0 {
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
        if length as usize != core::mem::size_of::<UnixPeerCredentials>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SO_PEERCRED returned an unexpected structure size",
            ));
        }
        Ok(credentials)
    }

    #[repr(align(8))]
    struct EndpointControlBuffer([u8; ENDPOINT_CONTROL_BYTES]);

    fn close_received_rights(descriptors: &[i32]) {
        for descriptor in descriptors.iter().copied().filter(|fd| *fd >= 0) {
            unsafe {
                syscall1(NR_CLOSE, descriptor as usize);
            }
        }
    }

    fn receive_endpoint_descriptor(gate_fd: i32) -> io::Result<StdOwnedFd> {
        let mut payload = [0u8; 16];
        let mut vector = IoVec {
            base: payload.as_mut_ptr(),
            len: payload.len(),
        };
        let mut control = EndpointControlBuffer([0; ENDPOINT_CONTROL_BYTES]);
        let mut message = MessageHeader {
            name: ptr::null_mut(),
            name_len: 0,
            iov: &mut vector,
            iov_len: 1,
            control: control.0.as_mut_ptr(),
            control_len: control.0.len(),
            flags: 0,
        };
        let received = unsafe {
            syscall3(
                NR_RECVMSG,
                gate_fd as usize,
                &mut message as *mut MessageHeader as usize,
                MSG_CMSG_CLOEXEC,
            )
        };
        if received < 0 {
            return Err(io::Error::from_raw_os_error((-received) as i32));
        }

        let supplied_control_len = message.control_len;
        let mut malformed = supplied_control_len > control.0.len()
            || message.flags & (MSG_CTRUNC | MSG_TRUNC) != 0
            || received != 1
            || payload[0] != ENDPOINT_MESSAGE_BYTE;
        let control_len = supplied_control_len.min(control.0.len());
        let mut descriptors = [-1i32; MAX_RECEIVED_RIGHTS];
        let mut descriptor_count = 0usize;
        let mut header_count = 0usize;
        let mut offset = 0usize;
        let header_size = core::mem::size_of::<ControlMessageHeader>();

        while offset < control_len {
            let remaining = control_len - offset;
            if remaining < header_size {
                malformed = true;
                break;
            }
            let header = unsafe {
                ptr::read_unaligned(
                    control
                        .0
                        .as_ptr()
                        .add(offset)
                        .cast::<ControlMessageHeader>(),
                )
            };
            if header.len < header_size || header.len > remaining {
                malformed = true;
                break;
            }
            header_count += 1;
            if header.level == SOL_SOCKET && header.kind == SCM_RIGHTS {
                let data_len = header.len - header_size;
                if !data_len.is_multiple_of(core::mem::size_of::<i32>()) {
                    malformed = true;
                }
                for index in 0..(data_len / core::mem::size_of::<i32>()) {
                    let raw_fd = unsafe {
                        ptr::read_unaligned(
                            control
                                .0
                                .as_ptr()
                                .add(offset + header_size + index * core::mem::size_of::<i32>())
                                .cast::<i32>(),
                        )
                    };
                    if raw_fd < 0 {
                        malformed = true;
                    } else if descriptor_count < descriptors.len() {
                        descriptors[descriptor_count] = raw_fd;
                        descriptor_count += 1;
                    } else {
                        malformed = true;
                        unsafe {
                            syscall1(NR_CLOSE, raw_fd as usize);
                        }
                    }
                }
            } else {
                malformed = true;
            }

            let aligned_len = header.len.saturating_add(7) & !7;
            if aligned_len > remaining {
                if header.len != remaining {
                    malformed = true;
                    break;
                }
                offset = control_len;
            } else {
                offset += aligned_len;
            }
        }

        if received == 0 && descriptor_count == 0 && supplied_control_len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "exec gate closed before endpoint transfer",
            ));
        }
        malformed |= header_count != 1 || descriptor_count != 1;
        if malformed {
            close_received_rights(&descriptors[..descriptor_count]);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "endpoint transfer ancillary data is malformed",
            ));
        }

        let raw_fd = descriptors[0];
        let descriptor_flags = unsafe { syscall3(NR_FCNTL, raw_fd as usize, F_GETFD, 0) };
        if descriptor_flags < 0 {
            close_received_rights(&descriptors[..descriptor_count]);
            return Err(io::Error::from_raw_os_error((-descriptor_flags) as i32));
        }
        if descriptor_flags as usize & FD_CLOEXEC == 0 {
            let set_flags = unsafe {
                syscall3(
                    NR_FCNTL,
                    raw_fd as usize,
                    F_SETFD,
                    descriptor_flags as usize | FD_CLOEXEC,
                )
            };
            if set_flags < 0 {
                close_received_rights(&descriptors[..descriptor_count]);
                return Err(io::Error::from_raw_os_error((-set_flags) as i32));
            }
        }
        descriptors[0] = -1;
        Ok(unsafe { StdOwnedFd::from_raw_fd(raw_fd) })
    }

    fn send_endpoint_descriptor(gate_fd: i32, endpoint_fd: i32) -> isize {
        let mut payload = ENDPOINT_MESSAGE_BYTE;
        let mut vector = IoVec {
            base: &mut payload,
            len: 1,
        };
        let mut control = EndpointControlBuffer([0; ENDPOINT_CONTROL_BYTES]);
        let header_bytes = core::mem::size_of::<ControlMessageHeader>();
        let cmsg_space = (header_bytes + core::mem::size_of::<i32>() + 7) & !7;
        unsafe {
            ptr::write(
                control.0.as_mut_ptr().cast::<ControlMessageHeader>(),
                ControlMessageHeader {
                    len: header_bytes + core::mem::size_of::<i32>(),
                    level: SOL_SOCKET,
                    kind: SCM_RIGHTS,
                },
            );
            ptr::write_unaligned(
                control.0.as_mut_ptr().add(header_bytes).cast::<i32>(),
                endpoint_fd,
            );
        }
        let mut message = MessageHeader {
            name: ptr::null_mut(),
            name_len: 0,
            iov: &mut vector,
            iov_len: 1,
            control: control.0.as_mut_ptr(),
            control_len: cmsg_space,
            flags: 0,
        };
        unsafe {
            syscall3(
                NR_SENDMSG,
                gate_fd as usize,
                &mut message as *mut MessageHeader as usize,
                MSG_NOSIGNAL,
            )
        }
    }

    pub(super) struct LinuxChildRaw {
        gate: Option<OwnedFd>,
        exec_error: Option<OwnedFd>,
        executable: Option<OwnedFd>,
        spawned: Option<SpawnedProcessPidfd>,
        reap: ReapTicket,
        execution_identity: LinuxExecutionIdentity,
        endpoint_policy: ProcessEndpointPolicy,
        app_storage: Option<crate::profile_volume_notes_owner::AuthorizedAppStorageRoot>,
        app_storage_revocation_sent: bool,
        endpoint: Option<StdOwnedFd>,
        fd_delegation_attestation: Option<FdDelegationRestrictionAttestation>,
    }

    enum ExecErrorPipeState {
        Pending,
        Eof,
        DelegationRestrictionReady,
        Error(i32),
        Partial,
    }

    fn read_exec_error_pipe(
        fd: i32,
        bytes: &mut [u8; 4],
        received: &mut usize,
    ) -> Result<ExecErrorPipeState, ReleaseFailureReason> {
        loop {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    fd as usize,
                    bytes[*received..].as_mut_ptr() as usize,
                    bytes.len() - *received,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result == -EAGAIN {
                return Ok(ExecErrorPipeState::Pending);
            }
            if result < 0 {
                return Err(ReleaseFailureReason::Io);
            }
            if result == 0 {
                return Ok(if *received == 0 {
                    ExecErrorPipeState::Eof
                } else {
                    ExecErrorPipeState::Partial
                });
            }
            let count = result as usize;
            if count > bytes.len() - *received {
                return Err(ReleaseFailureReason::Io);
            }
            *received += count;
            if *received == bytes.len() {
                let word = u32::from_ne_bytes(*bytes);
                return Ok(if word == CHILD_FD_DELEGATION_READY_WORD {
                    ExecErrorPipeState::DelegationRestrictionReady
                } else {
                    ExecErrorPipeState::Error(word as i32)
                });
            }
        }
    }

    struct LinuxExecStatusWait<'a> {
        status_fd: i32,
        reap: &'a ReapTicket,
    }

    impl ExecStatusWait for LinuxExecStatusWait<'_> {
        fn read_status_nonblocking(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    self.status_fd as usize,
                    buffer.as_mut_ptr() as usize,
                    buffer.len(),
                )
            };
            if result < 0 {
                Err(io::Error::from_raw_os_error((-result) as i32))
            } else {
                Ok(result as usize)
            }
        }

        fn wait_readable(&mut self, timeout: std::time::Duration) -> io::Result<bool> {
            const POLLIN: i16 = 0x001;
            const POLLERR: i16 = 0x008;
            const POLLHUP: i16 = 0x010;
            const POLLNVAL: i16 = 0x020;

            let whole_ms = timeout.as_millis();
            let fractional_ms = if timeout.subsec_nanos().is_multiple_of(1_000_000) {
                0
            } else {
                1
            };
            let timeout_ms = whole_ms
                .saturating_add(fractional_ms)
                .max(1)
                .min(i32::MAX as u128) as i32;
            let mut poll_fd = PollFd {
                fd: self.status_fd,
                events: POLLIN,
                revents: 0,
            };
            let result = unsafe {
                syscall3(
                    NR_POLL,
                    &mut poll_fd as *mut PollFd as usize,
                    1,
                    timeout_ms as usize,
                )
            };
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            if result == 0 {
                return Ok(false);
            }
            if poll_fd.revents & POLLNVAL != 0 {
                return Err(io::Error::from_raw_os_error(9));
            }
            Ok(poll_fd.revents & (POLLIN | POLLERR | POLLHUP) != 0)
        }

        fn child_exit_status(&mut self) -> io::Result<Option<ServiceExitStatus>> {
            reap_tracked_child(self.reap)
        }
    }

    impl LinuxChildRaw {
        fn wait_for_endpoint(
            &mut self,
            process_id: u32,
            started: std::time::Instant,
            budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<(), ReleaseFailureReason> {
            if cancel_check_interval.is_zero() {
                return Err(ReleaseFailureReason::Io);
            }
            let gate_fd = self.gate.as_ref().ok_or(ReleaseFailureReason::Io)?.raw();
            let error_fd = self
                .exec_error
                .as_ref()
                .ok_or(ReleaseFailureReason::Io)?
                .raw();
            let mut error_bytes = [0u8; 4];
            let mut error_bytes_read = 0usize;
            let mut delegation_restriction_ready = false;

            loop {
                if cancelled() {
                    return Err(ReleaseFailureReason::Cancelled);
                }
                if started.elapsed() >= budget {
                    return Err(ReleaseFailureReason::TimedOut);
                }
                if self.endpoint.is_none() {
                    match receive_endpoint_descriptor(gate_fd) {
                        Ok(endpoint) => {
                            let credentials = endpoint_peer_credentials(endpoint.as_raw_fd())
                                .map_err(|_| ReleaseFailureReason::Io)?;
                            if credentials.process_id <= 1
                                || credentials.process_id as u32 != process_id
                                || credentials.uid != self.execution_identity.uid
                                || credentials.gid != self.execution_identity.gid
                            {
                                return Err(ReleaseFailureReason::ProtocolViolation);
                            }
                            if cancelled() {
                                return Err(ReleaseFailureReason::Cancelled);
                            }
                            if started.elapsed() >= budget {
                                drop(endpoint);
                                return Err(ReleaseFailureReason::TimedOut);
                            }
                            self.endpoint = Some(endpoint);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
                        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                            return Err(ReleaseFailureReason::ProtocolViolation);
                        }
                        Err(_) => return Err(ReleaseFailureReason::Io),
                    }
                }

                match read_exec_error_pipe(error_fd, &mut error_bytes, &mut error_bytes_read)? {
                    ExecErrorPipeState::Pending => {}
                    ExecErrorPipeState::DelegationRestrictionReady => {
                        if delegation_restriction_ready {
                            return Err(ReleaseFailureReason::ProtocolViolation);
                        }
                        let identity = self
                            .spawned
                            .as_ref()
                            .ok_or(ReleaseFailureReason::Io)?
                            .identity();
                        if identity.process_id() != process_id {
                            return Err(ReleaseFailureReason::ProtocolViolation);
                        }
                        self.fd_delegation_attestation = Some(
                            FdDelegationRestrictionAttestation::from_verified_child(identity),
                        );
                        delegation_restriction_ready = true;
                        error_bytes = [0; 4];
                        error_bytes_read = 0;
                    }
                    ExecErrorPipeState::Error(errno) => {
                        return Err(ReleaseFailureReason::ExecError(errno));
                    }
                    ExecErrorPipeState::Partial => {
                        return Err(ReleaseFailureReason::PartialErrorReport);
                    }
                    ExecErrorPipeState::Eof => {
                        return match reap_tracked_child(&self.reap) {
                            Ok(Some(status)) => Err(ReleaseFailureReason::ChildExited(status)),
                            Ok(None) => Err(ReleaseFailureReason::ProtocolViolation),
                            Err(_) => Err(ReleaseFailureReason::Io),
                        };
                    }
                }

                if self.endpoint.is_some() && delegation_restriction_ready {
                    return Ok(());
                }

                match reap_tracked_child(&self.reap) {
                    Ok(Some(status)) => {
                        return match read_exec_error_pipe(
                            error_fd,
                            &mut error_bytes,
                            &mut error_bytes_read,
                        )? {
                            ExecErrorPipeState::Error(errno) => {
                                Err(ReleaseFailureReason::ExecError(errno))
                            }
                            ExecErrorPipeState::Partial => {
                                Err(ReleaseFailureReason::PartialErrorReport)
                            }
                            ExecErrorPipeState::DelegationRestrictionReady => {
                                Err(ReleaseFailureReason::ProtocolViolation)
                            }
                            ExecErrorPipeState::Pending | ExecErrorPipeState::Eof => {
                                Err(ReleaseFailureReason::ChildExited(status))
                            }
                        };
                    }
                    Ok(None) => {}
                    Err(_) => return Err(ReleaseFailureReason::Io),
                }

                if cancelled() {
                    return Err(ReleaseFailureReason::Cancelled);
                }
                let remaining = budget.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(ReleaseFailureReason::TimedOut);
                }
                let poll_budget = remaining.min(cancel_check_interval);
                let whole_ms = poll_budget.as_millis();
                let fractional_ms = if poll_budget.subsec_nanos().is_multiple_of(1_000_000) {
                    0
                } else {
                    1
                };
                let timeout_ms = whole_ms
                    .saturating_add(fractional_ms)
                    .max(1)
                    .min(i32::MAX as u128) as i32;
                let mut descriptors = [
                    PollFd {
                        fd: gate_fd,
                        events: POLLIN,
                        revents: 0,
                    },
                    PollFd {
                        fd: error_fd,
                        events: POLLIN,
                        revents: 0,
                    },
                ];
                let ready = unsafe {
                    syscall3(
                        NR_POLL,
                        descriptors.as_mut_ptr() as usize,
                        descriptors.len(),
                        timeout_ms as usize,
                    )
                };
                if ready == -(EINTR) {
                    continue;
                }
                if ready < 0 {
                    return Err(ReleaseFailureReason::Io);
                }
                if descriptors[0].revents & POLLNVAL != 0 {
                    return Err(ReleaseFailureReason::ProtocolViolation);
                }
                if descriptors[1].revents & POLLNVAL != 0 {
                    return Err(ReleaseFailureReason::Io);
                }
                if ready == 0 && started.elapsed() >= budget {
                    return Err(ReleaseFailureReason::TimedOut);
                }
                if descriptors[0].revents & POLLHUP != 0 && descriptors[0].revents & POLLIN == 0 {
                    continue;
                }
                if descriptors[0].revents & (POLLIN | POLLERR | POLLHUP) == 0
                    && descriptors[1].revents & (POLLIN | POLLERR | POLLHUP) == 0
                    && ready != 0
                {
                    return Err(ReleaseFailureReason::Io);
                }
            }
        }

        fn write_release_gate(
            &mut self,
            budget: std::time::Duration,
        ) -> Result<std::time::Instant, ReleaseFailureReason> {
            let started = std::time::Instant::now();
            if started.elapsed() >= budget {
                return Err(ReleaseFailureReason::TimedOut);
            }
            let gate = self.gate.as_ref().ok_or(ReleaseFailureReason::Io)?.raw();
            let byte = RELEASE_BYTE;
            // The parent end is nonblocking. Treat EINTR and a partial/error
            // result as a failed commit so the caller can clean the uncertain
            // child without holding the Supervisor lock for a retry loop.
            let result = unsafe {
                syscall6(
                    NR_SENDTO,
                    gate as usize,
                    &byte as *const u8 as usize,
                    1,
                    MSG_NOSIGNAL,
                    0,
                    0,
                )
            };
            if result != 1 {
                return Err(ReleaseFailureReason::Io);
            }
            Ok(started)
        }

        fn wait_exec_ack(
            &mut self,
            process_id: u32,
            started: std::time::Instant,
            budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<(), ReleaseFailureReason> {
            if self.endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel {
                if let Err(reason) = self.wait_for_endpoint(
                    process_id,
                    started,
                    budget,
                    cancel_check_interval,
                    cancelled,
                ) {
                    if matches!(reason, ReleaseFailureReason::ChildExited(_)) {
                        self.gate.take();
                        self.exec_error.take();
                        self.executable.take();
                    }
                    return Err(reason);
                }
            }
            let status_fd = self
                .exec_error
                .as_ref()
                .ok_or(ReleaseFailureReason::Io)?
                .raw();
            let mut wait = LinuxExecStatusWait {
                status_fd,
                reap: &self.reap,
            };
            let mut elapsed = || started.elapsed();
            match wait_for_exec_ack(
                &mut wait,
                budget,
                cancel_check_interval,
                &mut elapsed,
                cancelled,
            ) {
                Ok(()) => {
                    self.gate.take();
                    self.exec_error.take();
                    self.executable.take();
                    Ok(())
                }
                Err(reason @ ReleaseFailureReason::ChildExited(_)) => {
                    self.gate.take();
                    self.exec_error.take();
                    self.executable.take();
                    Err(reason)
                }
                Err(reason) => Err(reason),
            }
        }
    }

    pub(super) struct LinuxProcessOs;

    impl ProcessOs for LinuxProcessOs {
        type Raw = LinuxChildRaw;

        fn prepare(
            &mut self,
            executable: ExecutablePayload<'_>,
            arguments: &[&str],
            execution_identity: LinuxExecutionIdentity,
            endpoint_policy: ProcessEndpointPolicy,
            filesystem_policy: ProcessFilesystemPolicy,
        ) -> io::Result<ProcessHandle<Prepared, Self::Raw>> {
            if execution_identity.uid == 0 || execution_identity.gid == 0 {
                return process_prepare_stage(
                    "process.validate_non_root_identity",
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Linux process identity must not retain root credentials",
                    )),
                );
            }
            // Copy the exact PackageView executable slice into a private memfd and
            // seal it before creating the gated child. No pathname is resolved.
            let executable_fd = process_prepare_stage(
                "process.create_sealed_executable",
                create_sealed_executable(&mut LinuxSealedExecutableOps, executable.bytes()),
            )?;
            let task24_endpoint_mount = process_prepare_stage(
                "process.prepare_endpoint_mount_paths",
                task24_endpoint_mount_paths(arguments, endpoint_policy, execution_identity),
            )?;
            let task24_endpoint_source = task24_endpoint_mount
                .as_ref()
                .map_or(ptr::null(), |paths| paths.source.as_ptr());
            let task24_endpoint_target = task24_endpoint_mount
                .as_ref()
                .map_or(ptr::null(), |paths| paths.target.as_ptr());
            let app_filesystem =
                matches!(&filesystem_policy, ProcessFilesystemPolicy::Application(_));
            let private_filesystem =
                !matches!(&filesystem_policy, ProcessFilesystemPolicy::Inherited);
            let app_storage_mount = match &filesystem_policy {
                ProcessFilesystemPolicy::Application(storage) => {
                    process_prepare_stage(
                        "app_storage.validate_live_session",
                        storage.session_is_live().map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "app storage Session is no longer live",
                            )
                        }),
                    )?;
                    let stat = process_prepare_stage(
                        "app_storage.fstat_directory",
                        rustix::fs::fstat(storage.directory()).map_err(io::Error::from),
                    )?;
                    let file_type = stat.st_mode & 0o170000;
                    if file_type != 0o040000
                        || stat.st_uid != execution_identity.uid
                        || stat.st_gid != execution_identity.gid
                        || stat.st_mode & 0o777 != 0o700
                        || storage.app_id() == nagi_model::AppId(0)
                    {
                        return process_prepare_stage(
                            "app_storage.validate_private_owner_directory",
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "app storage root is not private and owned by the assigned app UID/GID",
                            )),
                        );
                    }
                    if LinuxExecutionIdentity::for_application(storage.app_id())
                        != Some(execution_identity)
                    {
                        return process_prepare_stage(
                            "app_storage.validate_application_uid",
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "app storage token does not match the assigned application UID",
                            )),
                        );
                    }
                    Some(prepare_app_storage_mount(storage.directory())?)
                }
                ProcessFilesystemPolicy::Inherited | ProcessFilesystemPolicy::PrivateTmpfs => None,
            };
            if endpoint_policy == ProcessEndpointPolicy::InheritedPeerChannel && private_filesystem
            {
                return process_prepare_stage(
                    "app_storage.validate_endpoint_policy",
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "system-service endpoint and app filesystem policies cannot be combined",
                    )),
                );
            }
            let argv_storage = process_argv_storage(arguments)?;
            let mut argv = argv_storage
                .iter()
                .map(|argument| argument.as_ptr())
                .collect::<Vec<_>>();
            argv.push(ptr::null());
            let envp = [ptr::null()];

            let null_fd = open_null()?;
            let mut gate_fds = [-1i32; 2];
            syscall_result(unsafe {
                syscall4(
                    NR_SOCKETPAIR,
                    AF_UNIX,
                    SOCK_STREAM | SOCK_CLOEXEC,
                    0,
                    gate_fds.as_mut_ptr() as usize,
                )
            })?;
            let gate_first = OwnedFd(gate_fds[0]);
            let gate_second = OwnedFd(gate_fds[1]);
            let gate_parent = promote_fd(gate_first)?;
            let gate_child = promote_fd(gate_second)?;
            set_nonblocking(gate_parent.raw())?;

            let mut error_fds = [-1i32; 2];
            syscall_result(unsafe {
                syscall2(NR_PIPE2, error_fds.as_mut_ptr() as usize, O_CLOEXEC)
            })?;
            let error_first = OwnedFd(error_fds[0]);
            let error_second = OwnedFd(error_fds[1]);
            let error_read = promote_fd(error_first)?;
            let error_write = promote_fd(error_second)?;
            set_nonblocking(error_read.raw())?;

            // Serialize child creation/registration with every consuming wait.
            // The child stays behind its gate until its ticket is published.
            let mut reaper = child_reaper()
                .lock()
                .map_err(|_| io::Error::other("PID-1 child reaper lock is poisoned"))?;
            let ticket_reservation: ReapTicketReservation = reaper.reserve_ticket()?;

            // clone3 atomically returns the exact pidfd. This is the only
            // production launch path; unsupported kernels fail closed without
            // a numeric-PID or pidfd_open fallback.
            match unsafe { clone3_pid1_service_child()? } {
                Clone3Pid1Child::Child => {
                    // Never unlock an inherited parent mutex in the child.
                    std::mem::forget(reaper);
                    unsafe {
                        child_wait_and_exec(
                            gate_parent.raw(),
                            gate_child.raw(),
                            error_read.raw(),
                            error_write.raw(),
                            null_fd.raw(),
                            executable_fd.raw(),
                            task24_endpoint_source,
                            task24_endpoint_target,
                            app_storage_mount.as_ref().map_or(-1, OwnedFd::raw),
                            private_filesystem,
                            app_filesystem,
                            argv.as_ptr(),
                            envp.as_ptr(),
                            execution_identity,
                            endpoint_policy,
                        )
                    }
                }
                Clone3Pid1Child::Parent(spawned) => {
                    let (ticket_spawn, owner_spawn, duplicate_error) =
                        match spawned.duplicate_for_authority() {
                            Ok(ticket_spawn) => (ticket_spawn, Some(spawned), None),
                            Err(error) => (spawned, None, Some(error)),
                        };
                    let ticket = ticket_reservation.bind_owned(ticket_spawn);
                    let mut gate_parent = gate_parent;
                    if let Err(error) = reaper.register(ticket.clone()) {
                        reaper.retain_failed_ticket(ticket.clone());
                        let launch_error =
                            fail_closed_child_before_release(&mut gate_parent, &ticket, &error);
                        drop(gate_child);
                        drop(error_read);
                        drop(error_write);
                        drop(null_fd);
                        drop(executable_fd);
                        drop(owner_spawn);
                        drop(reaper);
                        return Err(launch_error);
                    }
                    if let Some(error) = duplicate_error {
                        let launch_error =
                            fail_closed_child_before_release(&mut gate_parent, &ticket, &error);
                        drop(gate_child);
                        drop(error_read);
                        drop(error_write);
                        drop(null_fd);
                        drop(executable_fd);
                        drop(reaper);
                        return Err(launch_error);
                    }
                    let owner_spawn = owner_spawn.ok_or_else(|| {
                        io::Error::other("successful pidfd duplication lost its lifecycle owner")
                    })?;
                    let process_id = ticket.process_id();
                    drop(reaper);
                    drop(gate_child);
                    drop(error_write);
                    drop(null_fd);
                    if let Err(error) = await_child_setup_ready(
                        error_read.raw(),
                        std::time::Duration::from_secs(10),
                    ) {
                        let launch_error =
                            fail_closed_child_before_release(&mut gate_parent, &ticket, &error);
                        drop(error_read);
                        drop(executable_fd);
                        drop(owner_spawn);
                        return Err(launch_error);
                    }
                    Ok(ProcessHandle::new(
                        process_id,
                        LinuxChildRaw {
                            gate: Some(gate_parent),
                            exec_error: Some(error_read),
                            executable: Some(executable_fd),
                            spawned: Some(owner_spawn),
                            reap: ticket,
                            execution_identity,
                            endpoint_policy,
                            app_storage: match filesystem_policy {
                                ProcessFilesystemPolicy::Application(storage) => Some(storage),
                                ProcessFilesystemPolicy::Inherited
                                | ProcessFilesystemPolicy::PrivateTmpfs => None,
                            },
                            app_storage_revocation_sent: false,
                            endpoint: None,
                            fd_delegation_attestation: None,
                        },
                    ))
                }
                Clone3Pid1Child::ParentWithoutPidfd { process_id } => {
                    // The child exists but cannot be authenticated. Register
                    // a cleanup-only ticket before closing the gate so the
                    // single coordinator remains its only wait4 owner.
                    let ticket = ticket_reservation.bind_unverified(process_id);
                    let registration = reaper.register(ticket.clone());
                    let termination = terminate_without_pidfd(process_id);
                    drop(gate_parent);
                    drop(gate_child);
                    drop(error_read);
                    drop(error_write);
                    drop(null_fd);
                    drop(executable_fd);
                    drop(reaper);
                    registration?;
                    termination?;
                    wait_for_reap_ticket(&ticket)?;
                    Err(io::Error::other(
                        "clone3 created a child without its required atomic pidfd",
                    ))
                }
            }
        }

        fn take_handoff_resources(
            &mut self,
            child: &mut ProcessHandle<Running, Self::Raw>,
        ) -> io::Result<ProcessEndpointResources> {
            if child.raw.endpoint_policy != ProcessEndpointPolicy::InheritedPeerChannel {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "launch did not request an inherited peer channel",
                ));
            }
            if child.raw.endpoint.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "launch endpoint was already transferred or is unavailable",
                ));
            }
            let pidfd = child
                .raw
                .spawned
                .as_ref()
                .ok_or_else(|| io::Error::other("launch lost its clone3 pidfd"))?
                .duplicate_for_authority()?;
            let endpoint = child
                .raw
                .endpoint
                .take()
                .ok_or_else(|| io::Error::other("launch endpoint transfer raced"))?;
            Ok(ProcessEndpointResources { pidfd, endpoint })
        }

        fn duplicate_pidfd_for_prepared(
            &mut self,
            child: &ProcessHandle<Prepared, Self::Raw>,
        ) -> io::Result<SpawnedProcessPidfd> {
            child
                .raw
                .spawned
                .as_ref()
                .ok_or_else(|| io::Error::other("prepared child has no clone3 pidfd"))?
                .duplicate_for_authority()
        }

        fn duplicate_pidfd_for_running(
            &mut self,
            child: &mut ProcessHandle<Running, Self::Raw>,
        ) -> io::Result<SpawnedProcessPidfd> {
            child
                .raw
                .spawned
                .as_ref()
                .ok_or_else(|| io::Error::other("running child has no clone3 pidfd"))?
                .duplicate_for_authority()
        }

        fn take_fd_delegation_restriction_attestation(
            &mut self,
            child: &mut ProcessHandle<Running, Self::Raw>,
        ) -> io::Result<Option<FdDelegationRestrictionAttestation>> {
            Ok(child.raw.fd_delegation_attestation.take())
        }

        fn is_process_live(&self, child: &ProcessHandle<Running, Self::Raw>) -> io::Result<bool> {
            let pidfd = child
                .raw
                .spawned
                .as_ref()
                .ok_or_else(|| io::Error::other("running child has no clone3 pidfd"))?
                .as_raw_fd()?;
            let mut descriptor = PollFd {
                fd: pidfd,
                events: POLLIN,
                revents: 0,
            };
            let result =
                unsafe { syscall3(NR_POLL, &mut descriptor as *mut PollFd as usize, 1, 0) };
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            Ok(result == 0 && descriptor.revents == 0)
        }

        fn commit_release_gate<LaunchRecord>(
            &mut self,
            child: &mut ProcessHandle<Prepared, Self::Raw>,
            _registered: &RegisteredLaunch<LaunchRecord>,
            wait_budget: std::time::Duration,
        ) -> Result<std::time::Instant, ReleaseFailureReason> {
            child.raw.write_release_gate(wait_budget)
        }

        fn wait_exec_ack(
            &mut self,
            mut child: ProcessHandle<ReleaseUncertain, Self::Raw>,
            started: std::time::Instant,
            wait_budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<
            ProcessHandle<Running, Self::Raw>,
            ReleaseFailure<ProcessHandle<ReleaseUncertain, Self::Raw>>,
        > {
            match child.raw.wait_exec_ack(
                child.process_id,
                started,
                wait_budget,
                cancel_check_interval,
                cancelled,
            ) {
                Ok(()) => Ok(child.transition()),
                Err(reason) => {
                    let reaped = match reason {
                        ReleaseFailureReason::ChildExited(status) => Some(status),
                        _ => None,
                    };
                    Err(ReleaseFailure {
                        child: child.transition(),
                        reason,
                        reaped,
                    })
                }
            }
        }

        fn try_wait<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<Option<ServiceExitStatus>> {
            if let Some(status) = reap_tracked_child(&child.raw.reap)? {
                child.raw.app_storage.take();
                return Ok(Some(status));
            }
            if !child.raw.app_storage_revocation_sent
                && child
                    .raw
                    .app_storage
                    .as_ref()
                    .is_some_and(|storage| storage.session_is_live().is_err())
            {
                self.terminate(child)?;
                child.raw.app_storage_revocation_sent = true;
            }
            let status = reap_tracked_child(&child.raw.reap)?;
            if status.is_some() {
                child.raw.app_storage.take();
            }
            Ok(status)
        }

        fn terminate<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            let pidfd = child
                .raw
                .spawned
                .as_ref()
                .ok_or_else(|| io::Error::other("tracked child has no clone3 pidfd"))?
                .as_raw_fd()?;
            syscall_result(unsafe { syscall4(NR_PIDFD_SEND_SIGNAL, pidfd as usize, SIGKILL, 0, 0) })
                .map(|_| ())
        }
    }

    #[cfg(test)]
    mod inline_pidfd_retention_tests {
        use super::*;
        use std::time::{Duration, Instant};

        fn process_status_field<'a>(status: &'a str, name: &str) -> &'a str {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                .map(str::trim)
                .unwrap_or_else(|| panic!("/proc status is missing {name}"))
        }

        fn process_status_ids(status: &str, name: &str) -> Vec<u64> {
            process_status_field(status, name)
                .split_whitespace()
                .map(|value| value.parse().expect("proc status ID must be numeric"))
                .collect()
        }

        #[test]
        fn maintained_prepare_registry_owns_pidfd_after_owner_drop_until_exact_reap() {
            let executable = super::super::process_lifecycle_tests::synthetic_static_elf();
            let mut os = LinuxProcessOs;
            let child = os
                .prepare(
                    ExecutablePayload { bytes: &executable },
                    &[],
                    LinuxExecutionIdentity::DEV_SERVICE,
                    ProcessEndpointPolicy::Disabled,
                    ProcessFilesystemPolicy::Inherited,
                )
                .expect("maintained inline prepare must register the clone3 child");
            let mut handle = child;
            let proc_status =
                std::fs::read_to_string(format!("/proc/{}/status", handle.process_id))
                    .expect("setup-ready child must remain inspectable before gate release");
            let identity = handle.raw.execution_identity;
            assert_eq!(
                process_status_ids(&proc_status, "Uid"),
                vec![identity.uid as u64; 4],
                "all real/effective/saved/filesystem UIDs must match service identity"
            );
            assert_eq!(
                process_status_ids(&proc_status, "Gid"),
                vec![identity.gid as u64; 4],
                "all real/effective/saved/filesystem GIDs must match service identity"
            );
            assert_eq!(
                process_status_field(&proc_status, "Groups"),
                "",
                "service child must have no supplementary groups"
            );
            for name in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
                assert_eq!(
                    u64::from_str_radix(process_status_field(&proc_status, name), 16)
                        .expect("capability mask must be hexadecimal"),
                    0,
                    "{name} must be empty before release"
                );
            }
            assert_eq!(
                process_status_field(&proc_status, "NoNewPrivs"),
                "1",
                "no_new_privs must be active before release"
            );
            assert_eq!(
                process_status_field(&proc_status, "Seccomp"),
                "2",
                "namespace seccomp filter must be active before release"
            );
            assert_eq!(
                u64::from_str_radix(process_status_field(&proc_status, "SigCgt"), 16)
                    .expect("caught signal mask must be hexadecimal"),
                0,
                "clone3 must clear inherited caught handlers before child setup"
            );
            let blocked_signals =
                u64::from_str_radix(process_status_field(&proc_status, "SigBlk"), 16)
                    .expect("blocked signal mask must be hexadecimal");
            assert_eq!(
                blocked_signals & (1u64 << (SIGCHLD - 1)),
                0,
                "SIGCHLD must be unblocked only after child hardening and before setup-ready"
            );
            let observer = handle.raw.reap.observe_terminal_close();

            // Dropping the actual inline ProcessHandle closes the unreleased
            // child gate and its lifecycle pidfd. The registered ticket must
            // be the only strong owner until the exact exit is reaped.
            let reaper = child_reaper()
                .lock()
                .expect("global child reaper mutex must remain available");
            drop(handle);
            assert_eq!(observer.pidfd_owner_strong_count(), Some(1));
            assert!(observer.pidfd_owner_is_live());
            assert_eq!(observer.pidfd_owner_strong_count(), Some(1));
            drop(reaper);

            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = observer
                    .try_terminal_after_close()
                    .expect("registry ticket must survive through exact close")
                {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "idle reaper did not publish exact child status and close the registry pidfd"
                );
                reap_pending_child_exits().expect("maintained idle reaper scan must succeed");
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(status.exit_code, Some(125));
            assert_eq!(observer.pidfd_owner_strong_count(), Some(0));

            let no_new_privileges = unsafe { syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            assert_eq!(no_new_privileges, 0, "test thread must enable no_new_privs");
            install_service_namespace_seccomp()
                .expect("the same namespace filter must install in a service thread");
            // The pinned x86_64-musl Rust std path calls pthread_create, whose
            // bundled musl implementation uses __clone (NR_CLONE). This checks
            // that ordinary threads remain usable under the filter; it does
            // not test or prove a clone3-to-clone fallback.
            let thread = std::thread::Builder::new()
                .spawn(|| 0x53ec_c0de_u32)
                .expect("ordinary thread creation must work under the namespace filter");
            assert_eq!(
                thread.join().expect("service thread must complete"),
                0x53ec_c0de
            );
        }
    }
}

#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
fn backend_error_for_supervisor(failure: crate::supervisor::LaunchFailure) -> BackendError {
    match failure {
        crate::supervisor::LaunchFailure::ManifestsUnavailable => BackendError::Unavailable,
        crate::supervisor::LaunchFailure::Channel | crate::supervisor::LaunchFailure::Spawn => {
            BackendError::TransportFailure
        }
        _ => BackendError::IdentityRejected,
    }
}

#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
fn backend_error_for_grant(check: GrantCheck) -> BackendError {
    match check {
        GrantCheck::Granted => BackendError::ProtocolViolation,
        GrantCheck::NotLive => BackendError::Revoked,
        GrantCheck::NotDeclared => BackendError::IdentityRejected,
        // The Linux init adapter has no authenticated consent event path yet.
        GrantCheck::ConsentRequired => BackendError::Unavailable,
        GrantCheck::Denied => BackendError::ConsentDenied,
    }
}

#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
fn backend_error_for_package_provider(error: PackageProviderError) -> BackendError {
    use PackageProviderError as PackageError;
    match error {
        PackageError::TrustUnavailable | PackageError::InventoryUnavailable => {
            BackendError::Unavailable
        }
        PackageError::InvalidPackage(_)
        | PackageError::UnsignedPackage
        | PackageError::UntrustedSigner => BackendError::IdentityRejected,
        PackageError::InventoryCorrupt
        | PackageError::InventoryOutputTooSmall
        | PackageError::IdentityCollision => BackendError::ProtocolViolation,
    }
}

#[cfg(linux_dev_preview_smoke)]
fn backend_error_for_package_session_owner(error: PackageServiceSessionOwnerError) -> BackendError {
    use PackageServiceSessionOwnerError as OwnerError;
    match error {
        OwnerError::TrustUnavailable | OwnerError::Unavailable => BackendError::Unavailable,
        OwnerError::Package(_) | OwnerError::IdentityMismatch { .. } => {
            BackendError::IdentityRejected
        }
        OwnerError::Provider(error) => backend_error_for_package_provider(error),
        OwnerError::NotInstalled
        | OwnerError::ManifestChanged
        | OwnerError::StaleGeneration { .. }
        | OwnerError::OwnerMismatch => BackendError::Revoked,
        OwnerError::GenerationSaturated
        | OwnerError::PackageDigestUnavailable
        | OwnerError::DigestIndexUnavailable
        | OwnerError::DigestIndexFailed
        | OwnerError::PackageBlobUnavailable
        | OwnerError::PackageBlobFailed => BackendError::Unavailable,
        OwnerError::Quarantined => BackendError::Revoked,
        OwnerError::DigestInventoryFull
        | OwnerError::InstalledPackageBytesMismatch
        | OwnerError::DigestIndexOutcomeUnknown
        | OwnerError::DigestIndexCorrupt
        | OwnerError::DigestIndexReadbackMismatch
        | OwnerError::PackageBlobOutcomeUnknown
        | OwnerError::PackageBlobReadbackMismatch => BackendError::ProtocolViolation,
    }
}

/// Spawn a package through the existing Supervisor registry. The caller must
/// authenticate this exact package bytes using the production trust input and
/// confirmed installation inventory before calling. `PackageInspection` can
/// only be created by the package trust provider, so this adapter cannot accept
/// a raw `PackageView` or treat a missing package as installed. It consumes
/// only the caller's existing Session and service-required grants and never
/// creates a user decision. Normal `run()` remains fail closed while product
/// trust, session, consent, inventory, package bytes, and service transport
/// inputs are unavailable. The explicit `linux_dev_preview_smoke` build calls
/// this adapter only with its clearly labeled development fixtures.
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
fn spawn_with_supervisor<Os, ServiceId, Inventory>(
    lifecycle: &mut ProcessLifecycle<Os, ServiceId, LaunchRecord>,
    package_security: &PackageSecurityProvider<'_, Inventory>,
    package_bytes: &[u8],
    service: ServiceId,
    expected_app_id: AppId,
    required: bool,
    placement: LaunchPlacement,
    user: &Session,
    required_grants: &[&[u8]],
    application_storage: crate::profile_volume_notes_owner::AuthorizedAppStorageRoot,
    release_wait_budget: std::time::Duration,
    cancel_check_interval: std::time::Duration,
    cancelled: impl FnMut() -> bool,
) -> Result<u32, BackendError>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    Inventory: InstalledPackageInventory,
{
    let application_session_guard = application_storage.session_guard();
    let inspection = package_security.inspect(package_bytes).map_err(|error| {
        #[cfg(linux_dev_preview_smoke)]
        pid1_log!("nagi-init: dev preview package inspection failed: {error:?}");
        backend_error_for_package_provider(error)
    })?;
    let package = match inspection.installation {
        InstallationState::Installed { .. } => inspection.verified_package.view(),
        InstallationState::NotInstalled => {
            #[cfg(linux_dev_preview_smoke)]
            pid1_log!("nagi-init: dev preview package is not installed in the inspected inventory");
            return Err(BackendError::Unavailable);
        }
    };
    if user.token() == 0 || user.is_locked() {
        #[cfg(linux_dev_preview_smoke)]
        pid1_log!("nagi-init: dev preview Session validation failed");
        return Err(BackendError::ConsentDenied);
    }

    let signed_manifest = package.manifest();
    let manifest = AppManifest::from_declaration(signed_manifest.id(), signed_manifest.grants())
        .map_err(|error| {
            #[cfg(linux_dev_preview_smoke)]
            pid1_log!("nagi-init: dev preview signed manifest rejected: {error:?}");
            BackendError::IdentityRejected
        })?;
    let app_id = signed_manifest.app_id();
    if app_id != expected_app_id
        || manifest.app_id() != app_id
        || application_storage.app_id() != app_id
        || required_grants
            .iter()
            .any(|capability| !manifest.grants(capability))
    {
        #[cfg(linux_dev_preview_smoke)]
        pid1_log!(
            "nagi-init: dev preview app/grant validation failed (app_id_match={}, manifest_id_match={})",
            app_id == expected_app_id,
            manifest.app_id() == app_id
        );
        return Err(BackendError::IdentityRejected);
    }
    let execution_identity =
        LinuxExecutionIdentity::for_application(app_id).ok_or(BackendError::IdentityRejected)?;
    let executable =
        ExecutablePayload::from_package_executable(package.executable()).map_err(|error| {
            #[cfg(linux_dev_preview_smoke)]
            pid1_log!("nagi-init: dev preview executable payload rejected: {error:?}");
            BackendError::IdentityRejected
        })?;

    let result = lifecycle.spawn_authorized(
        ProcessLaunch {
            service,
            executable,
            arguments: &[],
            execution_identity,
            endpoint_policy: ProcessEndpointPolicy::Disabled,
            filesystem_policy: ProcessFilesystemPolicy::Application(application_storage),
            required,
            release_wait_budget,
            cancel_check_interval,
        },
        cancelled,
        |process_id| {
            crate::supervisor::register_or_match_manifest(manifest)
                .map_err(backend_error_for_supervisor)?;
            crate::supervisor::check_launch(app_id, placement)
                .map_err(backend_error_for_supervisor)?;
            crate::supervisor::record_launch(process_id, app_id, placement)
                .map_err(backend_error_for_supervisor)
        },
        |process_id, record, gate| {
            if record.process_id != process_id
                || record.app_id != app_id
                || record.app_session_id != placement.app_session_id
                || record.node_id != placement.node_id
                || record.workspace_id != placement.workspace_id
            {
                return Err(BackendError::ProtocolViolation);
            }
            application_session_guard
                .with_live_session_effect(|| {
                    crate::supervisor::authorize_launch_and_commit(record, required_grants, || {
                        gate.commit()
                    })
                })
                .map_err(|_| BackendError::Revoked)?
                .map_err(|failure| match failure {
                    crate::supervisor::LaunchCommitError::RegistryUnavailable => {
                        BackendError::Unavailable
                    }
                    crate::supervisor::LaunchCommitError::RecordMismatch => BackendError::Revoked,
                    crate::supervisor::LaunchCommitError::Grant(check) => {
                        backend_error_for_grant(check)
                    }
                    crate::supervisor::LaunchCommitError::Commit(error) => error,
                })
        },
        |process_id, record| {
            if crate::supervisor::record_exit_if_matches(process_id, record)
                || crate::supervisor::resolve(process_id).is_none()
            {
                Ok(())
            } else {
                Err(BackendError::Revoked)
            }
        },
    );
    #[cfg(linux_dev_preview_smoke)]
    if let Err(error) = result {
        pid1_log!("nagi-init: dev preview supervised process launch failed: {error:?}");
    }
    result
}

/// Start one fixed, development-only static ELF under an explicit empty-grant
/// Supervisor manifest. This is deliberately separate from package startup:
/// the raw ELF can only enter through the `linux_dev_preview_smoke` feature,
/// after the boot image builder has checked the exact candidate bytes.
#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn spawn_development_preview_elf(
    lifecycle: &mut LinuxDevPreviewLifecycle,
    elf_bytes: &[u8],
    manifest_identifier: &'static [u8],
    service: &'static str,
    arguments: &[&str],
    required: bool,
    placement: LaunchPlacement,
    execution_identity: LinuxExecutionIdentity,
) -> Result<u32, BackendError> {
    let manifest = AppManifest::from_declaration(manifest_identifier, core::iter::empty())
        .map_err(|error| {
            pid1_log!("nagi-init: dev preview process identity rejected: {error:?}");
            BackendError::IdentityRejected
        })?;
    let app_id = manifest.app_id();
    let executable =
        ExecutablePayload::from_development_preview_elf(elf_bytes).map_err(|error| {
            pid1_log!("nagi-init: dev preview static ELF rejected: {error:?}");
            BackendError::IdentityRejected
        })?;

    lifecycle.spawn_authorized(
        ProcessLaunch {
            service,
            executable,
            arguments,
            execution_identity,
            endpoint_policy: ProcessEndpointPolicy::Disabled,
            filesystem_policy: ProcessFilesystemPolicy::PrivateTmpfs,
            required,
            release_wait_budget: LinuxDevPreviewBackend::RELEASE_WAIT_BUDGET,
            cancel_check_interval: LinuxDevPreviewBackend::CANCEL_CHECK_INTERVAL,
        },
        || false,
        |process_id| {
            crate::supervisor::register_or_match_manifest(manifest)
                .map_err(backend_error_for_supervisor)?;
            crate::supervisor::check_launch(app_id, placement)
                .map_err(backend_error_for_supervisor)?;
            crate::supervisor::record_launch(process_id, app_id, placement)
                .map_err(backend_error_for_supervisor)
        },
        |process_id, record, gate| {
            if record.process_id != process_id
                || record.app_id != app_id
                || record.app_session_id != placement.app_session_id
                || record.node_id != placement.node_id
                || record.workspace_id != placement.workspace_id
            {
                return Err(BackendError::ProtocolViolation);
            }
            crate::supervisor::authorize_launch_and_commit(record, &[], || gate.commit()).map_err(
                |failure| match failure {
                    crate::supervisor::LaunchCommitError::RegistryUnavailable => {
                        BackendError::Unavailable
                    }
                    crate::supervisor::LaunchCommitError::RecordMismatch => BackendError::Revoked,
                    crate::supervisor::LaunchCommitError::Grant(check) => {
                        backend_error_for_grant(check)
                    }
                    crate::supervisor::LaunchCommitError::Commit(error) => error,
                },
            )
        },
        |process_id, record| {
            if crate::supervisor::record_exit_if_matches(process_id, record)
                || crate::supervisor::resolve(process_id).is_none()
            {
                Ok(())
            } else {
                Err(BackendError::Revoked)
            }
        },
    )
}

/// Explicitly development-only Linux integration backend. It verifies the
/// existing Developer Preview package trust/inventory path, then starts the
/// fixed task51 server and client through Supervisor and ProcessLifecycle.
/// Only a successful client exit after its real correlated service operation
/// may make this development target emit Ready; the normal build remains
/// fail-closed and does not select this fixture identity or route.
#[cfg(any(
    all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"),
    all(
        test,
        linux_dev_preview_smoke,
        target_os = "macos",
        target_arch = "x86_64"
    )
))]
struct LinuxDevPreviewBackend;

/// Validate the shared, development-only startup-input envelope without
/// choosing the candidate route's full schema. The server owner still owns
/// that schema; this boot boundary only requires a private `/run/nagi` path
/// and an explicit `mode=development` field before accepting an input.
#[cfg(linux_dev_preview_smoke)]
fn validate_development_input_envelope(
    path: &str,
    file_mode: u32,
    bytes: &[u8],
) -> Result<(), BackendError> {
    const INPUT_ROOT: &str = "/run/nagi/";
    const MAX_INPUT_BYTES: usize = 64 * 1024;

    let Some(relative) = path.strip_prefix(INPUT_ROOT) else {
        return Err(BackendError::ProtocolViolation);
    };
    if file_mode != 0o600
        || relative.is_empty()
        || relative.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.bytes().any(|byte| {
                    !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                })
        })
        || bytes.is_empty()
        || bytes.len() > MAX_INPUT_BYTES
    {
        return Err(BackendError::ProtocolViolation);
    }

    let text = std::str::from_utf8(bytes).map_err(|_| BackendError::ProtocolViolation)?;
    if text.contains('\r') || text.contains('\0') {
        return Err(BackendError::ProtocolViolation);
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.is_empty() || lines.iter().any(|line| line.is_empty()) {
        return Err(BackendError::ProtocolViolation);
    }

    let mut keys = std::collections::BTreeSet::new();
    let mut development_mode = false;
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            return Err(BackendError::ProtocolViolation);
        };
        if key.is_empty()
            || value.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || !keys.insert(key)
        {
            return Err(BackendError::ProtocolViolation);
        }
        if key == "mode" {
            if value != "development" {
                return Err(BackendError::ProtocolViolation);
            }
            development_mode = true;
        }
    }
    if development_mode {
        Ok(())
    } else {
        Err(BackendError::ProtocolViolation)
    }
}

#[cfg(linux_dev_preview_smoke)]
fn validate_development_action_input(
    path: &str,
    file_mode: u32,
    bytes: &[u8],
) -> Result<(), BackendError> {
    validate_development_input_envelope(path, file_mode, bytes)?;
    let text = std::str::from_utf8(bytes).map_err(|_| BackendError::ProtocolViolation)?;
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(BackendError::ProtocolViolation);
        };
        if fields.insert(key, value).is_some() {
            return Err(BackendError::ProtocolViolation);
        }
    }
    const REQUIRED_KEYS: [&str; 14] = [
        "format",
        "mode",
        "service_id",
        "service_version",
        "operation_id",
        "payload_type",
        "service_app_id",
        "peer_app_id",
        "launch_id",
        "token",
        "socket",
        "index_root",
        "intent",
        "query",
    ];
    if fields.len() != REQUIRED_KEYS.len()
        || REQUIRED_KEYS.iter().any(|key| !fields.contains_key(key))
    {
        return Err(BackendError::ProtocolViolation);
    }
    for (key, expected) in [
        ("format", "nagi-dev-service-input-v1"),
        ("mode", "development"),
        ("service_id", "dev.nagi.action"),
        ("service_version", "1.0"),
        ("operation_id", "execute"),
        ("payload_type", "nagi.action@1"),
        ("service_app_id", "org.nagi.dev.action-service"),
        ("peer_app_id", "org.nagi.dev.action-client"),
        ("socket", "/run/nagi/dev-action.sock"),
        ("index_root", "/run/nagi/dev-data"),
        ("intent", "find the live VFS fixture"),
        ("query", "nagi-m19-live-file.txt"),
    ] {
        if fields.get(key).copied() != Some(expected) {
            return Err(BackendError::ProtocolViolation);
        }
    }
    for (key, expected_len) in [("launch_id", 32usize), ("token", 64usize)] {
        let value = fields.get(key).ok_or(BackendError::ProtocolViolation)?;
        if value.len() != expected_len
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || value.bytes().all(|byte| byte == b'0')
        {
            return Err(BackendError::ProtocolViolation);
        }
    }
    Ok(())
}

const DEV_SERVICE_UID: u32 = 65_534;
const DEV_SERVICE_GID: u32 = 65_534;
const DEV_NOTES_DESKTOP_UID: u32 = 65_533;
const DEV_NOTES_DESKTOP_GID: u32 = 65_533;
const APP_FILES_UID: u32 = crate::profile_volume_notes_owner::USER_FILES_APP_UID;
const APP_FILES_GID: u32 = crate::profile_volume_notes_owner::USER_FILES_APP_GID;
const APP_NOTES_UID: u32 = crate::profile_volume_notes_owner::NOTES_APP_UID;
const APP_NOTES_GID: u32 = crate::profile_volume_notes_owner::NOTES_APP_GID;
#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
const DEV_INPUT_PATH: &str = "/run/nagi/dev-service.runtime";
#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
const DEV_SOCKET_PATH: &str = "/run/nagi/dev-action.sock";
#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
const DEV_INDEX_ROOT: &str = "/run/nagi/dev-data";
#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
const DEV_INDEX_FILE: &str = "/run/nagi/dev-data/nagi-m19-live-file.txt";
#[cfg(linux_desktop_dev_smoke)]
const NOTES_DESKTOP_INPUT_PATH: &str = "/run/nagi/notes-desktop.runtime";
#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const NOTES_DESKTOP_ELF_PATH: &str = "/nagi-notes-desktop";

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn chown_dev_path(path: &str) -> Result<(), BackendError> {
    chown_dev_path_as(path, DEV_SERVICE_UID, DEV_SERVICE_GID)
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn chown_dev_path_as(path: &str, uid: u32, gid: u32) -> Result<(), BackendError> {
    let path = CString::new(path).map_err(|_| BackendError::ProtocolViolation)?;
    // SAFETY: `path` is NUL-terminated and points to a fixed private initramfs
    // path; PID 1 owns this explicit development setup operation.
    let result = unsafe { chown(path.as_ptr(), uid, gid) };
    if result == 0 {
        Ok(())
    } else {
        Err(BackendError::Unavailable)
    }
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn change_dev_path_mode(path: &str, mode: u32) -> Result<(), BackendError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|_| BackendError::Unavailable)
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn require_root_owned_path(
    path: &str,
    expected_mode: u32,
    expected_type: fn(&std::fs::FileType) -> bool,
) -> Result<(), BackendError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|_| BackendError::Unavailable)?;
    if !expected_type(&metadata.file_type())
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.mode() & 0o7777 != expected_mode
    {
        return Err(BackendError::ProtocolViolation);
    }
    Ok(())
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn prepare_development_runtime_tree() -> Result<(), BackendError> {
    let directory_type = |file_type: &std::fs::FileType| file_type.is_dir();
    let regular_file_type = |file_type: &std::fs::FileType| file_type.is_file();
    require_root_owned_path("/run/nagi", 0o700, directory_type)?;
    require_root_owned_path(DEV_INDEX_ROOT, 0o700, directory_type)?;
    require_root_owned_path(DEV_INDEX_FILE, 0o600, regular_file_type)?;

    let mut root_entries = std::fs::read_dir("/run/nagi")
        .map_err(|_| BackendError::Unavailable)?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|_| BackendError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    root_entries.sort();
    #[cfg(linux_desktop_dev_smoke)]
    let expected_root_entries = vec!["dev-data".to_owned(), "notes-desktop.runtime".to_owned()];
    #[cfg(not(linux_desktop_dev_smoke))]
    let expected_root_entries = vec!["dev-data".to_owned()];
    if root_entries != expected_root_entries {
        return Err(BackendError::ProtocolViolation);
    }
    #[cfg(linux_desktop_dev_smoke)]
    {
        require_root_owned_path(NOTES_DESKTOP_INPUT_PATH, 0o600, regular_file_type)?;
        let input =
            std::fs::read(NOTES_DESKTOP_INPUT_PATH).map_err(|_| BackendError::Unavailable)?;
        validate_notes_desktop_input(0o600, &input)?;
    }

    let mut data_entries = std::fs::read_dir(DEV_INDEX_ROOT)
        .map_err(|_| BackendError::Unavailable)?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|_| BackendError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    data_entries.sort();
    if data_entries.len() != 1
        || data_entries[0] != "nagi-m19-live-file.txt"
        || std::fs::read(DEV_INDEX_FILE).map_err(|_| BackendError::Unavailable)?
            != b"nagi development search fixture\n"
    {
        return Err(BackendError::ProtocolViolation);
    }

    chown_dev_path("/run/nagi")?;
    chown_dev_path(DEV_INDEX_ROOT)?;
    chown_dev_path(DEV_INDEX_FILE)?;
    change_dev_path_mode("/run/nagi", 0o700)?;
    change_dev_path_mode(DEV_INDEX_ROOT, 0o700)?;
    change_dev_path_mode(DEV_INDEX_FILE, 0o600)?;

    if std::fs::symlink_metadata(DEV_SOCKET_PATH).is_ok()
        || std::fs::symlink_metadata(DEV_INPUT_PATH).is_ok()
    {
        return Err(BackendError::ProtocolViolation);
    }
    Ok(())
}

#[cfg(linux_desktop_dev_smoke)]
fn validate_notes_desktop_input(
    file_mode: u32,
    bytes: &[u8],
) -> Result<&'static str, BackendError> {
    validate_development_input_envelope(NOTES_DESKTOP_INPUT_PATH, file_mode, bytes)?;
    let text = std::str::from_utf8(bytes).map_err(|_| BackendError::ProtocolViolation)?;
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(BackendError::ProtocolViolation);
        };
        if fields.insert(key, value).is_some() {
            return Err(BackendError::ProtocolViolation);
        }
    }
    if fields.len() != 2 || fields.get("mode") != Some(&"development") {
        return Err(BackendError::ProtocolViolation);
    }
    match fields.get("phase").copied() {
        Some("save") => Ok("save"),
        Some("read") => Ok("read"),
        _ => Err(BackendError::ProtocolViolation),
    }
}

#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
const NOTES_INPUT_KEY_CODES: [u16; 5] = [15, 28, 30, 31, 57]; // Tab, Enter, A, S, Space.
#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
const NOTES_INPUT_KEY_MAX: usize = 0x2ff;
#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
const NOTES_INPUT_KEY_BITMAP_BYTES: usize = (NOTES_INPUT_KEY_MAX + 8) / 8;
#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
const NOTES_INPUT_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
const fn notes_event_ioctl_request(number: u32, size: usize) -> std::os::raw::c_ulong {
    ((2u64 << 30) | ((size as u64) << 16) | ((b'E' as u64) << 8) | number as u64) as _
}

#[cfg(all(
    any(
        linux_desktop_dev_smoke,
        all(
            any(linux_boot_owner_registration, linux_task24_owner_co_location),
            feature = "desktop-login"
        )
    ),
    target_os = "linux",
    target_arch = "x86_64"
))]
fn select_development_keyboard() -> Result<(String, std::fs::File), BackendError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

    const EVIOCGVERSION: std::os::raw::c_ulong =
        notes_event_ioctl_request(0x01, core::mem::size_of::<std::os::raw::c_int>());
    const EVIOCGBIT_KEY: std::os::raw::c_ulong =
        notes_event_ioctl_request(0x21, NOTES_INPUT_KEY_BITMAP_BYTES);

    unsafe extern "C" {
        fn ioctl(
            fd: std::os::raw::c_int,
            request: std::os::raw::c_ulong,
            ...
        ) -> std::os::raw::c_int;
    }

    fn supports_key(bitmap: &[u8; NOTES_INPUT_KEY_BITMAP_BYTES], key: u16) -> bool {
        let index = usize::from(key);
        bitmap[index / 8] & (1 << (index % 8)) != 0
    }

    let mut event_nodes = std::fs::read_dir("/dev/input")
        .map_err(|_| BackendError::Unavailable)?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|_| BackendError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|name| {
            name.strip_prefix("event").is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .collect::<Vec<_>>();
    event_nodes.sort();
    if event_nodes.is_empty() {
        return Err(BackendError::Unavailable);
    }

    let mut matches = Vec::new();
    for node_name in event_nodes {
        let path = format!("/dev/input/{node_name}");
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| BackendError::Unavailable)?;
        if metadata.uid() != 0 || metadata.gid() != 0 || !metadata.file_type().is_char_device() {
            return Err(BackendError::ProtocolViolation);
        }
        let sysfs_name =
            std::fs::read_to_string(format!("/sys/class/input/{node_name}/device/name"))
                .map_err(|_| BackendError::Unavailable)?;
        let sysfs_name = sysfs_name.trim().to_owned();
        if sysfs_name.is_empty() {
            return Err(BackendError::ProtocolViolation);
        }

        let mut options = std::fs::OpenOptions::new();
        options.read(true).custom_flags(NOTES_INPUT_O_NOFOLLOW);
        let device = options.open(&path).map_err(|_| BackendError::Unavailable)?;
        let opened = device.metadata().map_err(|_| BackendError::Unavailable)?;
        if !opened.file_type().is_char_device()
            || opened.uid() != 0
            || opened.gid() != 0
            || opened.rdev() != metadata.rdev()
        {
            return Err(BackendError::ProtocolViolation);
        }
        let mut version: std::os::raw::c_int = 0;
        let version_result = unsafe { ioctl(device.as_raw_fd(), EVIOCGVERSION, &mut version) };
        if version_result < 0 || version <= 0 {
            return Err(BackendError::Unavailable);
        }
        let mut bitmap = [0u8; NOTES_INPUT_KEY_BITMAP_BYTES];
        let bitmap_result =
            unsafe { ioctl(device.as_raw_fd(), EVIOCGBIT_KEY, bitmap.as_mut_ptr()) };
        if bitmap_result < 0 {
            return Err(BackendError::Unavailable);
        }

        let capabilities = NOTES_INPUT_KEY_CODES.map(|key| supports_key(&bitmap, key));
        pid1_log!(
            "nagi-init: evdev candidate path={path} sysfs_name={sysfs_name:?} version={version} keys(tab={},enter={},a={},s={},space={})",
            capabilities[0],
            capabilities[1],
            capabilities[2],
            capabilities[3],
            capabilities[4]
        );
        if capabilities.into_iter().all(|supported| supported) {
            matches.push((path, sysfs_name, device));
        }
    }

    match matches.len() {
        1 => {
            let (path, sysfs_name, device) = matches.pop().expect("one keyboard");
            pid1_log!(
                "nagi-init: selected evdev keyboard path={path} sysfs_name={sysfs_name:?} required_keys=PASS"
            );
            Ok((path, device))
        }
        0 => Err(BackendError::Unavailable),
        _ => {
            pid1_log!(
                "nagi-init: refusing ambiguous evdev keyboard selection count={}",
                matches.len()
            );
            Err(BackendError::ProtocolViolation)
        }
    }
}

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
fn load_notes_framebuffer_modules() -> Result<(), BackendError> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    const EXPECTED_RELEASE: &str = "6.8.0-71-generic";
    const MODULE_INIT_COMPRESSED_FILE: std::os::raw::c_int = 4;
    const NR_FINIT_MODULE: std::os::raw::c_long = 313;
    const EMPTY_MODULE_PARAMETERS: &[u8] = b"\0";

    #[repr(C)]
    struct LinuxUtsname {
        sysname: [std::os::raw::c_char; 65],
        nodename: [std::os::raw::c_char; 65],
        release: [std::os::raw::c_char; 65],
        version: [std::os::raw::c_char; 65],
        machine: [std::os::raw::c_char; 65],
        domainname: [std::os::raw::c_char; 65],
    }

    #[link(name = "c")]
    extern "C" {
        fn uname(name: *mut LinuxUtsname) -> std::os::raw::c_int;
        fn syscall(number: std::os::raw::c_long, ...) -> std::os::raw::c_long;
    }

    const MODULES: [(&str, &str); 4] = [
        ("ttm", "/nagi-dev-modules/ttm.ko.zst"),
        ("drm_ttm_helper", "/nagi-dev-modules/drm_ttm_helper.ko.zst"),
        (
            "drm_vram_helper",
            "/nagi-dev-modules/drm_vram_helper.ko.zst",
        ),
        ("bochs", "/nagi-dev-modules/bochs.ko.zst"),
    ];

    let mut uts = core::mem::MaybeUninit::<LinuxUtsname>::uninit();
    // SAFETY: `uname` writes one complete utsname into this uninitialized
    // output buffer before it is read below.
    if unsafe { uname(uts.as_mut_ptr()) } != 0 {
        pid1_log!(
            "nagi-init: cannot identify kernel before loading the development Notes framebuffer driver: {:?}",
            std::io::Error::last_os_error()
        );
        return Err(BackendError::Unavailable);
    }
    // SAFETY: uname succeeded and initialized the full structure.
    let uts = unsafe { uts.assume_init() };
    // SAFETY: uname's release field is NUL terminated by the Linux ABI.
    let release = unsafe { CStr::from_ptr(uts.release.as_ptr()) }
        .to_str()
        .map_err(|_| BackendError::ProtocolViolation)?;
    if release != EXPECTED_RELEASE {
        pid1_log!(
            "nagi-init: refusing pinned framebuffer modules for kernel_release={release} expected={EXPECTED_RELEASE}"
        );
        return Err(BackendError::IdentityRejected);
    }

    for (name, path) in MODULES {
        if std::path::Path::new("/sys/module").join(name).is_dir() {
            pid1_log!(
                "nagi-init: development Notes framebuffer module already present name={name}"
            );
            continue;
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            pid1_log!(
                "nagi-init: pinned framebuffer module unavailable name={name} error_kind={:?}",
                error.kind()
            );
            BackendError::Unavailable
        })?;
        if !metadata.file_type().is_file()
            || metadata.uid() != 0
            || metadata.gid() != 0
            || metadata.mode() & 0o7777 != 0o444
        {
            pid1_log!("nagi-init: pinned framebuffer module rejected name={name}");
            return Err(BackendError::ProtocolViolation);
        }
        let module = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(NOTES_INPUT_O_NOFOLLOW)
            .open(path)
            .map_err(|error| {
                pid1_log!(
                    "nagi-init: pinned framebuffer module open failed name={name} error_kind={:?}",
                    error.kind()
                );
                BackendError::Unavailable
            })?;
        // SAFETY: the fd refers to a root-owned, immutable, digest-pinned
        // compressed module included by the A-owned development CPIO builder.
        // The parameter pointer names a valid empty NUL-terminated string.
        // The syscall uses only the kernel-supported compressed-module flag;
        // it does not ignore vermagic, modversions, or signature checks.
        let loaded = unsafe {
            syscall(
                NR_FINIT_MODULE,
                module.as_raw_fd(),
                EMPTY_MODULE_PARAMETERS
                    .as_ptr()
                    .cast::<std::os::raw::c_char>(),
                MODULE_INIT_COMPRESSED_FILE,
            )
        };
        if loaded < 0 {
            let error = std::io::Error::last_os_error();
            pid1_log!(
                "nagi-init: pinned framebuffer module load failed name={name} error={error:?}"
            );
            return Err(BackendError::Unavailable);
        }
        pid1_log!("nagi-init: development Notes framebuffer module loaded name={name}");
    }
    Ok(())
}

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
fn prepare_notes_desktop_devices() -> Result<String, BackendError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    fn grant_to_notes_desktop(
        path: &str,
        is_expected_device: impl FnOnce(&std::fs::FileType) -> bool,
    ) -> Result<(), BackendError> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            pid1_log!(
                "nagi-init: Notes device grant failed path={path} operation=stat error_kind={:?}",
                error.kind()
            );
            BackendError::Unavailable
        })?;
        if metadata.uid() != 0 || metadata.gid() != 0 || !is_expected_device(&metadata.file_type())
        {
            pid1_log!(
                "nagi-init: Notes device grant rejected path={path} uid={} gid={} file_type={:?}",
                metadata.uid(),
                metadata.gid(),
                metadata.file_type()
            );
            return Err(BackendError::ProtocolViolation);
        }
        chown_dev_path_as(path, DEV_NOTES_DESKTOP_UID, DEV_NOTES_DESKTOP_GID).map_err(|error| {
            pid1_log!(
                "nagi-init: Notes device grant failed path={path} operation=chown error={error:?}"
            );
            error
        })?;
        change_dev_path_mode(path, 0o600).map_err(|error| {
            pid1_log!(
                "nagi-init: Notes device grant failed path={path} operation=chmod error={error:?}"
            );
            error
        })
    }

    load_notes_framebuffer_modules()?;
    let (keyboard_path, _keyboard) = select_development_keyboard()?;

    // Keep the Notes child isolated from the Nagi action service's UID. Grant
    // only its disk, uniquely capability-matched keyboard, and framebuffer;
    // reject symlinks or unexpected node types before changing devtmpfs ownership.
    grant_to_notes_desktop("/dev/vda", std::fs::FileType::is_block_device)?;
    grant_to_notes_desktop(&keyboard_path, std::fs::FileType::is_char_device)?;
    grant_to_notes_desktop("/dev/fb0", std::fs::FileType::is_char_device)?;
    Ok(keyboard_path)
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn append_hex(out: &mut String, bytes: &[u8]) {
    use std::fmt::Write;
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn write_development_action_input() -> Result<(), BackendError> {
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut entropy = [0u8; 48];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut entropy))
        .map_err(|_| BackendError::Unavailable)?;
    if entropy.iter().all(|byte| *byte == 0) {
        return Err(BackendError::ProtocolViolation);
    }

    let mut input = String::with_capacity(384);
    input.push_str(
        "format=nagi-dev-service-input-v1\nmode=development\nservice_id=dev.nagi.action\nservice_version=1.0\noperation_id=execute\npayload_type=nagi.action@1\nservice_app_id=org.nagi.dev.action-service\npeer_app_id=org.nagi.dev.action-client\nlaunch_id=",
    );
    append_hex(&mut input, &entropy[..16]);
    input.push_str("\ntoken=");
    append_hex(&mut input, &entropy[16..]);
    input.push_str(
        "\nsocket=/run/nagi/dev-action.sock\nindex_root=/run/nagi/dev-data\nintent=find the live VFS fixture\nquery=nagi-m19-live-file.txt\n",
    );
    let mut input = input.into_bytes();
    validate_development_action_input(DEV_INPUT_PATH, 0o600, &input)?;

    let write_result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(DEV_INPUT_PATH)
            .map_err(|_| BackendError::Unavailable)?;
        file.write_all(&input)
            .map_err(|_| BackendError::Unavailable)?;
        file.sync_all().map_err(|_| BackendError::Unavailable)?;
        drop(file);
        chown_dev_path(DEV_INPUT_PATH)?;
        change_dev_path_mode(DEV_INPUT_PATH, 0o600)?;
        let metadata =
            std::fs::symlink_metadata(DEV_INPUT_PATH).map_err(|_| BackendError::Unavailable)?;
        if !metadata.file_type().is_file()
            || metadata.uid() != DEV_SERVICE_UID
            || metadata.gid() != DEV_SERVICE_GID
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(BackendError::ProtocolViolation);
        }
        Ok(())
    })();
    input.fill(0);
    entropy.fill(0);
    write_result
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
type LinuxDevPreviewLifecycle =
    ProcessLifecycle<linux_x86_64_process::LinuxProcessOs, &'static str, LaunchRecord>;

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
fn revoke_dev_preview_record(process_id: u32, record: &LaunchRecord) -> Result<(), BackendError> {
    if crate::supervisor::record_exit_if_matches(process_id, record)
        || crate::supervisor::resolve(process_id).is_none()
    {
        Ok(())
    } else {
        Err(BackendError::Revoked)
    }
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
struct LinuxDevPreviewSession {
    lifecycle: LinuxDevPreviewLifecycle,
    _user: Session,
    package_owner: PackageServiceSessionOwner<'static>,
    package_inventory_snapshot: Option<OwnedPackageInventorySnapshot>,
    ready_pending: bool,
    #[cfg(linux_desktop_dev_smoke)]
    notes_desktop_started: bool,
    #[cfg(linux_desktop_dev_smoke)]
    notes_phase: Option<&'static str>,
}

#[cfg(any(
    all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"),
    all(
        test,
        linux_dev_preview_smoke,
        target_os = "macos",
        target_arch = "x86_64"
    )
))]
impl LinuxDevPreviewBackend {
    /// Test the same package identity boundary used by the Linux backend
    /// without constructing providers or crossing into process management.
    fn with_expected_package<T>(
        package_bytes: &[u8],
        expected_app_id: AppId,
        after_match: impl FnOnce(&PackageView<'_>) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let package = PackageView::parse(package_bytes).map_err(|error| {
            pid1_log!("nagi-init: dev preview XAPP parse failed: {error:?}");
            BackendError::IdentityRejected
        })?;
        let actual_app_id = package.manifest().app_id();
        linux_package_identity::with_expected_app_id(actual_app_id, expected_app_id, || {
            after_match(&package)
        })
        .map_err(|_| {
            pid1_log!("nagi-init: dev preview XAPP AppId did not match the pinned input");
            BackendError::IdentityRejected
        })?
    }
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
impl LinuxDevPreviewBackend {
    const PACKAGE_PATH: &'static str = "/nagi-dev-preview-linux.xapp";
    const ACTION_SERVICE_PATH: &'static str = "/nagi-dev-action-service";
    const ACTION_CLIENT_PATH: &'static str = "/nagi-dev-action-client";
    const EXPECTED_APP_ID: AppId = AppId(2_136_227_276_280_675_244);
    const FIXTURE_ACCOUNT: &'static [u8] = b"dev-prev-smoke";
    const FIXTURE_PASSWORD: &'static [u8] = b"nagi-dev-preview-only";
    const RELEASE_WAIT_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
    const CANCEL_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
    const SERVICE_START_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    const CLIENT_CALL_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

    const fn new() -> Self {
        Self
    }

    fn poll_dev_lifecycle(
        lifecycle: &mut LinuxDevPreviewLifecycle,
    ) -> Result<Option<LifecycleEvent<&'static str>>, BackendError> {
        lifecycle.poll(revoke_dev_preview_record)
    }

    fn wait_for_dev_socket(lifecycle: &mut LinuxDevPreviewLifecycle) -> Result<(), BackendError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};

        let started = std::time::Instant::now();
        loop {
            if let Some(event) = Self::poll_dev_lifecycle(lifecycle)? {
                return match event {
                    LifecycleEvent::ServiceExited { .. } => Err(BackendError::ServiceFailure),
                    _ => Err(BackendError::ProtocolViolation),
                };
            }
            match std::fs::symlink_metadata(DEV_SOCKET_PATH) {
                Ok(metadata) => {
                    if metadata.file_type().is_socket()
                        && metadata.uid() == DEV_SERVICE_UID
                        && metadata.gid() == DEV_SERVICE_GID
                        && metadata.mode() & 0o7777 == 0o600
                    {
                        return Ok(());
                    }
                    return Err(BackendError::ProtocolViolation);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(BackendError::Unavailable),
            }
            if started.elapsed() >= Self::SERVICE_START_BUDGET {
                return Err(BackendError::Unavailable);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn wait_for_successful_client(
        lifecycle: &mut LinuxDevPreviewLifecycle,
        expected_service: &'static str,
    ) -> Result<(), BackendError> {
        let started = std::time::Instant::now();
        loop {
            if let Some(event) = Self::poll_dev_lifecycle(lifecycle)? {
                return match event {
                    LifecycleEvent::ServiceExited {
                        service,
                        status,
                        required: false,
                    } if service == expected_service
                        && status.exit_code == Some(0)
                        && status.signal.is_none() =>
                    {
                        Ok(())
                    }
                    LifecycleEvent::ServiceExited { .. } => Err(BackendError::ServiceFailure),
                    _ => Err(BackendError::ProtocolViolation),
                };
            }
            if started.elapsed() >= Self::CLIENT_CALL_BUDGET {
                return Err(BackendError::TransportFailure);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn run_client_case(
        lifecycle: &mut LinuxDevPreviewLifecycle,
        user: &Session,
        case: &'static str,
        app_session_id: u64,
    ) -> Result<(), BackendError> {
        const CALL: &[&str] = &["--input", DEV_INPUT_PATH, "--case", "call"];
        const UNREGISTERED: &[&str] = &["--input", DEV_INPUT_PATH, "--case", "unregistered"];
        const UNKNOWN_PEER: &[&str] = &["--input", DEV_INPUT_PATH, "--case", "unknown-peer"];
        const BAD_TOKEN: &[&str] = &["--input", DEV_INPUT_PATH, "--case", "bad-token"];
        const MISMATCHED_CORRELATION: &[&str] = &[
            "--input",
            DEV_INPUT_PATH,
            "--case",
            "mismatched-correlation",
        ];
        const BAD_REQUEST_CORRELATION: &[&str] = &[
            "--input",
            DEV_INPUT_PATH,
            "--case",
            "bad-request-correlation",
        ];
        let arguments = match case {
            "call" => CALL,
            "unregistered" => UNREGISTERED,
            "unknown-peer" => UNKNOWN_PEER,
            "bad-token" => BAD_TOKEN,
            "mismatched-correlation" => MISMATCHED_CORRELATION,
            "bad-request-correlation" => BAD_REQUEST_CORRELATION,
            _ => return Err(BackendError::ProtocolViolation),
        };
        if user.token() == 0 || user.is_locked() {
            return Err(BackendError::ConsentDenied);
        }
        let placement = LaunchPlacement {
            app_session_id: AppSessionId(app_session_id),
            node_id: NodeId(1),
            workspace_id: None,
        };
        spawn_development_preview_elf(
            lifecycle,
            &std::fs::read(Self::ACTION_CLIENT_PATH).map_err(|_| BackendError::Unavailable)?,
            b"org.nagi.dev.action-client",
            "dev-action.client",
            &arguments,
            false,
            placement,
            LinuxExecutionIdentity::DEV_SERVICE,
        )?;
        Self::wait_for_successful_client(lifecycle, "dev-action.client")
    }

    fn start_action_service(
        lifecycle: &mut LinuxDevPreviewLifecycle,
        user: &Session,
    ) -> Result<(), BackendError> {
        use std::os::unix::fs::FileTypeExt;

        prepare_development_runtime_tree()?;
        write_development_action_input()?;
        let service_elf =
            std::fs::read(Self::ACTION_SERVICE_PATH).map_err(|_| BackendError::Unavailable)?;
        ExecutablePayload::from_development_preview_elf(&service_elf).map_err(|error| {
            pid1_log!("nagi-init: dev preview action service ELF rejected: {error:?}");
            BackendError::IdentityRejected
        })?;
        let placement = LaunchPlacement {
            app_session_id: AppSessionId(0x4e41_4749_0000_0002),
            node_id: NodeId(1),
            workspace_id: None,
        };
        spawn_development_preview_elf(
            lifecycle,
            &service_elf,
            b"org.nagi.dev.action-service",
            "dev-action.service",
            &["--input", DEV_INPUT_PATH],
            true,
            placement,
            LinuxExecutionIdentity::DEV_SERVICE,
        )?;
        Self::wait_for_dev_socket(lifecycle)?;

        for (case, app_session_id) in [
            ("call", 0x4e41_4749_0000_0101),
            ("unregistered", 0x4e41_4749_0000_0102),
            ("unknown-peer", 0x4e41_4749_0000_0103),
            ("bad-token", 0x4e41_4749_0000_0104),
            ("mismatched-correlation", 0x4e41_4749_0000_0105),
            ("bad-request-correlation", 0x4e41_4749_0000_0106),
        ] {
            Self::run_client_case(lifecycle, user, case, app_session_id)?;
        }
        if Self::poll_dev_lifecycle(lifecycle)?.is_some() {
            return Err(BackendError::ServiceFailure);
        }
        let socket =
            std::fs::symlink_metadata(DEV_SOCKET_PATH).map_err(|_| BackendError::Unavailable)?;
        if !socket.file_type().is_socket() {
            return Err(BackendError::ProtocolViolation);
        }
        Ok(())
    }

    #[cfg(linux_desktop_dev_smoke)]
    fn start_notes_desktop(session: &mut LinuxDevPreviewSession) -> Result<(), BackendError> {
        use std::os::unix::fs::MetadataExt;

        let keyboard_path = prepare_notes_desktop_devices()?;
        let metadata = std::fs::symlink_metadata(NOTES_DESKTOP_INPUT_PATH)
            .map_err(|_| BackendError::Unavailable)?;
        if metadata.uid() != 0 || metadata.gid() != 0 || metadata.mode() & 0o7777 != 0o600 {
            return Err(BackendError::ProtocolViolation);
        }
        let bytes =
            std::fs::read(NOTES_DESKTOP_INPUT_PATH).map_err(|_| BackendError::Unavailable)?;
        let phase = validate_notes_desktop_input(0o600, &bytes)?;
        let desktop =
            std::fs::read(NOTES_DESKTOP_ELF_PATH).map_err(|_| BackendError::Unavailable)?;
        ExecutablePayload::from_development_preview_elf(&desktop).map_err(|error| {
            pid1_log!("nagi-init: dev Notes desktop ELF rejected: {error:?}");
            BackendError::IdentityRejected
        })?;
        let placement = LaunchPlacement {
            app_session_id: AppSessionId(0x4e41_4749_0000_0107),
            node_id: NodeId(1),
            workspace_id: None,
        };
        let arguments = [phase, "/dev/vda", keyboard_path.as_str()];
        spawn_development_preview_elf(
            &mut session.lifecycle,
            &desktop,
            b"org.nagi.dev.notes-desktop",
            "dev-notes.desktop",
            &arguments,
            false,
            placement,
            LinuxExecutionIdentity::DEV_NOTES_DESKTOP,
        )?;
        session.notes_phase = Some(phase);
        pid1_log!("nagi-init: started development Notes desktop phase={phase}");
        Ok(())
    }

    fn initialize_verified_package(
        package_bytes: &[u8],
        package: &PackageView<'_>,
    ) -> Result<LinuxDevPreviewSession, BackendError> {
        // This account exists only in the explicit development smoke feature.
        // It is not a product identity source, profile binding, or consent.
        let mut accounts = AccountStore::new();
        accounts
            .add_account(
                Self::FIXTURE_ACCOUNT,
                Role::Standard,
                Self::FIXTURE_PASSWORD,
            )
            .map_err(|error| {
                pid1_log!("nagi-init: dev preview fixture account setup failed: {error:?}");
                BackendError::IdentityRejected
            })?;
        let user = accounts
            .authenticate(Self::FIXTURE_ACCOUNT, Self::FIXTURE_PASSWORD)
            .map_err(|error| {
                pid1_log!("nagi-init: dev preview fixture authentication failed: {error:?}");
                BackendError::IdentityRejected
            })?;

        if user.token() == 0 || user.is_locked() {
            return Err(BackendError::ConsentDenied);
        }

        // Construct the lifecycle session owner before the first install so
        // installation, snapshot validation, and later mutations all use the
        // exact PackageService instance retained for this session.
        let mut session = LinuxDevPreviewSession {
            lifecycle: ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs),
            _user: user,
            package_owner: PackageServiceSessionOwner::new(
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                PackageService::new(),
            ),
            package_inventory_snapshot: None,
            ready_pending: false,
            #[cfg(linux_desktop_dev_smoke)]
            notes_desktop_started: false,
            #[cfg(linux_desktop_dev_smoke)]
            notes_phase: None,
        };

        let install = session
            .package_owner
            .install(
                package_bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .map_err(|error| {
                pid1_log!("nagi-init: dev preview package installation failed: {error:?}");
                backend_error_for_package_session_owner(error)
            })?;
        if install.app_id != Self::EXPECTED_APP_ID || install.unsigned_warning {
            pid1_log!(
                "nagi-init: dev preview installation report did not match the expected signed package"
            );
            return Err(BackendError::IdentityRejected);
        }

        let package_snapshot = session
            .package_owner
            .capture(package_bytes, package.manifest().id())
            .map_err(|error| {
                pid1_log!("nagi-init: dev preview inventory snapshot failed: {error:?}");
                backend_error_for_package_session_owner(error)
            })?;
        if package_snapshot.app_id() != Self::EXPECTED_APP_ID {
            return Err(BackendError::IdentityRejected);
        }
        {
            let lease = session
                .package_owner
                .lease_for_dispatch(&package_snapshot)
                .map_err(|error| {
                    pid1_log!("nagi-init: dev preview inventory lease failed: {error:?}");
                    backend_error_for_package_session_owner(error)
                })?;
            let inspection =
                PackageSecurityProvider::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), lease.inventory())
                    .inspect(package_bytes)
                    .map_err(|error| {
                        pid1_log!("nagi-init: dev preview inventory inspection failed: {error:?}");
                        backend_error_for_package_provider(error)
                    })?;
            if !matches!(
                inspection.installation,
                InstallationState::Installed { generation }
                    if generation == package_snapshot.generation()
            ) {
                pid1_log!("nagi-init: dev preview package generation is no longer current");
                return Err(BackendError::Revoked);
            }
        }
        session.package_inventory_snapshot = Some(package_snapshot);

        match Self::start_action_service(&mut session.lifecycle, &session._user) {
            Ok(()) => {
                pid1_log!(
                    "nagi-init: development-only NIPC handshake and correlated file.search reply verified"
                );
                session.ready_pending = true;
                Ok(session)
            }
            Err(error) => {
                if session
                    .lifecycle
                    .shutdown(revoke_dev_preview_record)
                    .is_err()
                {
                    pid1_log!(
                        "nagi-init: development-only partial startup cleanup failed; no service is ready"
                    );
                    Err(BackendError::TransportFailure)
                } else {
                    Err(error)
                }
            }
        }
    }
}

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
impl InitBackend for LinuxDevPreviewBackend {
    #[cfg(feature = "linux-service-action-bridge")]
    type Action = String;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type Action = Infallible;
    #[cfg(feature = "linux-service-action-bridge")]
    type Reply = ActionResult;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type Reply = Infallible;
    type Session = LinuxDevPreviewSession;
    type ServiceHandle = ();
    type ServiceId = &'static str;

    fn initialize(&mut self) -> Result<Self::Session, BackendError> {
        let package_bytes =
            std::fs::read(Self::PACKAGE_PATH).map_err(|_| BackendError::Unavailable)?;
        Self::with_expected_package(&package_bytes, Self::EXPECTED_APP_ID, |package| {
            Self::initialize_verified_package(&package_bytes, package)
        })
    }

    fn call_service(
        &mut self,
        _session: &mut Self::Session,
        _service: &mut Self::ServiceHandle,
        request: ServiceRequest<Self::Action>,
        #[cfg(feature = "linux-service-action-bridge")] _cancellation: CancellationToken,
        #[cfg(feature = "linux-service-action-bridge")] _timeout: Duration,
    ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError> {
        #[cfg(feature = "linux-service-action-bridge")]
        {
            let _ = request;
            Err(ServiceIoError::Unavailable)
        }
        #[cfg(not(feature = "linux-service-action-bridge"))]
        match request.action {}
    }

    fn wait_event(
        &mut self,
        session: &mut Self::Session,
    ) -> Result<LifecycleEvent<Self::ServiceId>, BackendError> {
        if session.ready_pending {
            let snapshot = session
                .package_inventory_snapshot
                .as_ref()
                .ok_or(BackendError::Unavailable)?;
            let _lease = session
                .package_owner
                .lease_for_dispatch(snapshot)
                .map_err(backend_error_for_package_session_owner)?;
            session.ready_pending = false;
            return Ok(LifecycleEvent::Ready);
        }
        #[cfg(linux_desktop_dev_smoke)]
        if !session.notes_desktop_started {
            session.notes_desktop_started = true;
            Self::start_notes_desktop(session)?;
        }
        loop {
            if let Some(event) = session.lifecycle.poll(revoke_dev_preview_record)? {
                #[cfg(linux_desktop_dev_smoke)]
                if let LifecycleEvent::ServiceExited {
                    service: "dev-notes.desktop",
                    status,
                    ..
                } = &event
                {
                    pid1_log!(
                        "NAGI_NOTES_DESKTOP_CHILD phase={} exit={:?} signal={:?}",
                        session.notes_phase.unwrap_or("unknown"),
                        status.exit_code,
                        status.signal
                    );
                }
                return Ok(event);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn shutdown(
        &mut self,
        session: &mut Self::Session,
        _cause: ShutdownCause,
    ) -> Result<(), BackendError> {
        session.lifecycle.shutdown(revoke_dev_preview_record)
    }
}

/// A directory descriptor and the expected owner supplied by a boot/volume
/// owner. This pair is input to metadata validation only; it does not prove
/// where the descriptor came from or bind it to a selected product volume.
#[cfg(unix)]
struct PackageStoreDirectoryInput {
    directory: std::fs::File,
    expected_owner_uid: u32,
}

#[cfg(unix)]
impl PackageStoreDirectoryInput {
    fn new(directory: std::fs::File, expected_owner_uid: u32) -> Self {
        Self {
            directory,
            expected_owner_uid,
        }
    }
}

/// Adapter used while the Linux process backend is not connected. Its
/// optional package-store input is validated through the existing provider
/// boundary, then startup still fails closed until product trust/catalog and
/// authenticated transport are supplied.
struct UnconnectedBackend {
    #[cfg(unix)]
    package_store_directory: Option<PackageStoreDirectoryInput>,
    #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
    boot_catalog: Option<linux_boot_package_inputs::BootCatalogInputs>,
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        not(any(linux_dev_preview_smoke, nagi_release_build))
    ))]
    identity_data: Option<linux_mount_prerequisites::data_mount_owner::DevelopmentIdentityData>,
    #[cfg(test)]
    wait_calls: usize,
}

impl UnconnectedBackend {
    fn new() -> Self {
        Self::with_package_store_directory(None)
    }

    #[cfg(unix)]
    fn with_package_store_directory(input: Option<PackageStoreDirectoryInput>) -> Self {
        Self {
            package_store_directory: input,
            #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
            boot_catalog: None,
            #[cfg(all(
                target_os = "linux",
                target_arch = "x86_64",
                not(any(linux_dev_preview_smoke, nagi_release_build))
            ))]
            identity_data: None,
            #[cfg(test)]
            wait_calls: 0,
        }
    }

    #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
    fn with_verified_boot_inputs(inputs: linux_boot_package_inputs::BootPackageInputs) -> Self {
        let mut backend = Self::with_package_store_directory(Some(
            PackageStoreDirectoryInput::new(inputs.directory, 0),
        ));
        backend.boot_catalog = Some(inputs.catalogs);
        backend
    }

    #[cfg(not(unix))]
    fn with_package_store_directory(_input: Option<()>) -> Self {
        Self {
            #[cfg(test)]
            wait_calls: 0,
        }
    }
}

impl InitBackend for UnconnectedBackend {
    #[cfg(feature = "linux-service-action-bridge")]
    type Action = String;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type Action = Infallible;
    #[cfg(feature = "linux-service-action-bridge")]
    type Reply = ActionResult;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type Reply = Infallible;
    type Session = ();
    #[cfg(feature = "linux-service-action-bridge")]
    type ServiceHandle = Option<RuntimeActionServiceHandle>;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type ServiceHandle = ();
    #[cfg(feature = "linux-service-action-bridge")]
    type ServiceId = ServiceId;
    #[cfg(not(feature = "linux-service-action-bridge"))]
    type ServiceId = &'static str;

    fn initialize(&mut self) -> Result<Self::Session, BackendError> {
        #[cfg(unix)]
        {
            let Some(input) = self.package_store_directory.take() else {
                pid1_log!("nagi-init: boot package-store directory descriptor unavailable");
                return Err(BackendError::Unavailable);
            };
            #[cfg(all(
                target_os = "linux",
                target_arch = "x86_64",
                not(linux_dev_preview_smoke)
            ))]
            let package_store = input.directory.try_clone().map_err(|error| {
                pid1_log!("nagi-init: cannot retain boot package directory ({error})");
                BackendError::Unavailable
            })?;
            let persistence = LinuxInstalledPackagePersistence::from_trusted_directory(
                input.directory,
                input.expected_owner_uid,
            )
            .map_err(|error| {
                pid1_log!("nagi-init: package-store directory descriptor rejected ({error:?})");
                BackendError::Unavailable
            })?;

            #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
            let catalog = self.boot_catalog.take();
            #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
            let trusted_key = catalog
                .as_ref()
                .map(|inputs| inputs.catalog.package_signing_public_key());
            #[cfg(not(all(target_os = "linux", not(linux_dev_preview_smoke))))]
            let trusted_key = None;
            let trusted_key = trusted_key.as_ref();
            #[cfg(all(
                target_os = "linux",
                target_arch = "x86_64",
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                feature = "desktop-login",
                feature = "system-service-readiness",
                not(any(linux_dev_preview_smoke, nagi_release_build))
            ))]
            let trusted_key = trusted_key
                .map(linux_task24_endpoint_launch::identity_listener::pin_boot_package_key)
                .transpose()
                .map_err(|_| BackendError::IdentityRejected)?;
            let restore =
                nagi_init_package_provider::package_trust::PackageServiceSessionOwner::
                    restore_from_persistent_storage(
                        trusted_key,
                        nagi_package::InstallPolicy {
                            developer_mode: false,
                            require_signature: true,
                        },
                        &persistence,
                        &persistence,
                    );
            let package_owner = restore.map_err(|error| {
                pid1_log!("nagi-init: package-store restore/admission unavailable ({error:?})");
                BackendError::Unavailable
            })?;
            #[cfg(all(
                target_os = "linux",
                target_arch = "x86_64",
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                feature = "desktop-login",
                feature = "system-service-readiness",
                not(any(linux_dev_preview_smoke, nagi_release_build))
            ))]
            let package_owner = std::sync::Arc::new(package_owner);
            #[cfg(all(target_os = "linux", not(linux_dev_preview_smoke)))]
            if let Some(inputs) = catalog.as_ref() {
                use nagi_system_service_launch::ArtifactPreflight as _;

                #[cfg(all(
                    linux_task24_owner_co_location,
                    target_arch = "x86_64",
                    feature = "desktop-login",
                    not(nagi_release_build)
                ))]
                let _login_package = match linux_task24_endpoint_launch::VerifiedLoginChild::load(
                    &package_owner,
                    &persistence,
                )? {
                    Some(package) => {
                        pid1_log!(
                            "NAGI_DEV_TASK24_LOGIN_PACKAGE_VERIFIED generation={}",
                            package.package_generation()
                        );
                        Some(package)
                    }
                    None => None,
                };
                #[cfg(all(
                    linux_task24_owner_co_location,
                    target_arch = "x86_64",
                    feature = "desktop-login",
                    not(nagi_release_build)
                ))]
                let _login_run = if _login_package.is_some() {
                    let run = linux_task24_endpoint_launch::BootLoginRun::begin(inputs)?;
                    pid1_log!(
                        "NAGI_DEV_TASK24_RUN_RETAINED generation={}",
                        run.run_owner.generation()
                    );
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let run = std::sync::Arc::new(run);
                    Some(run)
                } else {
                    None
                };
                let catalog = &inputs.catalog;
                let descriptor = catalog.descriptor();
                let report = linux_boot_package_inputs::LinuxBootArtifactPreflight::new()
                    .and_then(|mut preflight| {
                        preflight.inspect(&descriptor.artifact_preflight_request())
                    })
                    .map_err(|error| {
                        pid1_log!("nagi-init: boot service artifact unavailable ({error:?})");
                        BackendError::Unavailable
                    })?;
                let report = nagi_system_service_launch::validate_artifact_preflight_report(
                    &report, descriptor,
                )
                .map_err(|error| {
                    pid1_log!("nagi-init: boot service artifact rejected ({error:?})");
                    BackendError::Unavailable
                })?;
                let bytes = package_owner
                    .read_installed_system_service_package(
                        descriptor.package_id().as_bytes(),
                        &persistence,
                    )
                    .map_err(|error| {
                        pid1_log!("nagi-init: boot service package read failed ({error:?})");
                        BackendError::Unavailable
                    })?;
                let package = nagi_system_service_launch::bind_installed_service_package(
                    descriptor,
                    &bytes,
                    &package_owner,
                )
                .map_err(|error| {
                    pid1_log!("nagi-init: boot service package binding failed ({error:?})");
                    BackendError::Unavailable
                })?;
                pid1_log!(
                    "nagi-init: boot service package verified generation={}",
                    package.package_generation()
                );
                #[cfg(target_arch = "x86_64")]
                {
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let mut pending_identity = _login_run
                        .as_ref()
                        .filter(|run| run.identity_ready.is_some())
                        .map(|run| {
                            linux_task24_endpoint_launch::identity_listener::PendingIdentityListener::new(
                                std::sync::Arc::clone(run),
                                std::sync::Arc::clone(&package_owner),
                                &package,
                                &bytes,
                            )
                        })
                        .transpose()
                        .map_err(|error| {
                            pid1_log!("nagi-init: Identity pending listener unavailable ({error})");
                            BackendError::Unavailable
                        })?;
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let identity_arguments = pending_identity
                        .as_ref()
                        .map(|pending| pending.child_arguments())
                        .transpose()
                        .map_err(|_| BackendError::ProtocolViolation)?;
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let arguments = identity_arguments
                        .as_ref()
                        .map(|argv| argv.iter().map(String::as_str).collect::<Vec<_>>())
                        .unwrap_or_default();
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let identity_listener = pending_identity.as_mut().map(|pending| {
                        linux_c3_boot_launch::IdentityListenerHandoff {
                            arguments: &arguments,
                            pending,
                        }
                    });
                    #[cfg(all(
                        feature = "system-service-readiness",
                        not(all(
                            target_os = "linux",
                            target_arch = "x86_64",
                            linux_task24_owner_co_location,
                            linux_task24_initial_login_runtime,
                            feature = "desktop-login",
                            feature = "system-service-readiness",
                            not(any(linux_dev_preview_smoke, nagi_release_build))
                        ))
                    ))]
                    let identity_listener = None;
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    let activating_identity = identity_listener.is_some();
                    #[cfg(not(nagi_release_build))]
                    let task24_data_root = if catalog.descriptor().package_id()
                        == linux_c3_boot_launch::TASK24_IDENTITY_PACKAGE_ID
                    {
                        let data = self.identity_data.as_ref().ok_or_else(|| {
                            pid1_log!(
                                "NAGI_DEV_TASK24_C3_DATA_JOURNAL_UNAVAILABLE reason=data_owner_missing"
                            );
                            BackendError::Unavailable
                        })?;
                        Some(data.input.selected_data_root().map_err(|error| {
                            pid1_log!(
                                "NAGI_DEV_TASK24_C3_DATA_JOURNAL_UNAVAILABLE reason=data_root_rejected ({error})"
                            );
                            BackendError::Unavailable
                        })?)
                    } else {
                        None
                    };
                    #[cfg(nagi_release_build)]
                    let task24_data_root = None;
                    let authenticated = linux_c3_boot_launch::authenticate_boot_service(
                        &package_store,
                        catalog,
                        &report,
                        package,
                        task24_data_root.as_ref(),
                        #[cfg(feature = "system-service-readiness")]
                        identity_listener,
                    )
                    .map_err(|error| {
                        pid1_log!(
                            "nagi-init: C3 boot service authentication unavailable ({error})"
                        );
                        BackendError::Unavailable
                    })?;
                    #[cfg(all(
                        target_os = "linux",
                        target_arch = "x86_64",
                        linux_task24_owner_co_location,
                        linux_task24_initial_login_runtime,
                        feature = "desktop-login",
                        feature = "system-service-readiness",
                        not(any(linux_dev_preview_smoke, nagi_release_build))
                    ))]
                    if activating_identity {
                        pid1_log!("NAGI_DEV_TASK24_IDENTITY_LISTENER_ACTIVATED");
                    }
                    let mapping = authenticated
                        .with_verified_system_mapping(&inputs.recovery_catalog, |mapping| {
                            mapping.mapping_generation()
                        });
                    #[cfg(all(linux_boot_identity_runtime, not(nagi_release_build)))]
                    if mapping.is_ok() {
                        if let Some(data) = self.identity_data.as_ref() {
                            match authenticated.recover_selected_identity_data(
                                &inputs.recovery_catalog,
                                &data.input,
                                &data.snapshots,
                                &data.accounts,
                            ) {
                                Ok(mut runtime) => {
                                    match runtime.verify_continuing_admission() {
                                        Ok(()) => {
                                            pid1_log!("NAGI_C1_BOOT_RECOVERY_AND_C3_CONTINUATION_VERIFIED");
                                            #[cfg(all(
                                            linux_task24_owner_co_location,
                                            linux_task24_initial_login_runtime,
                                            feature = "desktop-login"
                                        ))]
                                        let _login_route = _login_run.as_ref().and_then(|run| {
                                            match runtime.prepare_task24_route(run) {
                                                Ok(route) => {
                                                    pid1_log!("NAGI_DEV_TASK24_INITIAL_LOGIN_ROUTE_BOUND generation={}", run.run_owner.generation());
                                                    Some(route)
                                                }
                                                Err(error) => {
                                                    pid1_log!("NAGI_DEV_TASK24_INITIAL_LOGIN_ROUTE_UNAVAILABLE ({error:?})");
                                                    None
                                                }
                                            }
                                        });
                                            #[cfg(all(linux_boot_owner_registration, feature = "desktop-login"))]
                                        match linux_owner_registration_console::BootOwnerConsole::open() {
                                            Ok(mut console) => {
                                                #[cfg(all(
                                                    linux_task24_owner_co_location,
                                                    linux_task24_initial_login_runtime
                                                ))]
                                                if let (Some(run), Some(package), Some(route)) = (
                                                    _login_run.as_ref(),
                                                    _login_package.as_ref(),
                                                    _login_route,
                                                ) {
                                                    linux_task24_endpoint_launch::run_owner_login(
                                                        runtime,
                                                        package,
                                                        route,
                                                        run,
                                                        #[cfg(feature = "system-service-readiness")]
                                                        pending_identity.as_ref(),
                                                        &mut console,
                                                    );
                                                }
                                                match runtime.run_owner_registration(&mut console) {
                                                    Ok(()) => pid1_log!("NAGI_LINUX_OWNER_REGISTRATION_SAVED"),
                                                    Err(error) => pid1_log!("NAGI_LINUX_OWNER_REGISTRATION_UNAVAILABLE ({error})"),
                                                }
                                                wait_in_pid1(wait_for_pid1_os_event);
                                            }
                                            Err(error) => pid1_log!("NAGI_LINUX_OWNER_UI_UNAVAILABLE ({error})"),
                                        }
                                            wait_in_pid1(wait_for_pid1_os_event);
                                        }
                                        Err(error) => pid1_log!(
                                            "NAGI_C3_BOOT_CONTINUATION_UNAVAILABLE ({error:?})"
                                        ),
                                    }
                                }
                                Err(error) => {
                                    pid1_log!("NAGI_C1_BOOT_RECOVERY_UNAVAILABLE ({error:?})")
                                }
                            }
                        }
                    }
                    #[cfg(not(nagi_release_build))]
                    if mapping.is_ok() {
                        if let Some(data) = self.identity_data.as_ref() {
                            match data.input.selected_data_root() {
                                Ok(root) => {
                                    match root.prepare_account_verifier(&data.accounts) {
                                        Ok(_) => pid1_log!("NAGI_DEV_IDENTITY_ACCOUNTS_DIRECTORY_VALIDATED"),
                                        Err(error) => pid1_log!("NAGI_DEV_IDENTITY_ACCOUNTS_DIRECTORY_UNAVAILABLE ({error})"),
                                    }
                                    match root.retain_identity_snapshot_directory(&data.snapshots) {
                                        Ok(_) => pid1_log!("NAGI_DEV_IDENTITY_SNAPSHOTS_DIRECTORY_VALIDATED"),
                                        Err(error) => pid1_log!("NAGI_DEV_IDENTITY_SNAPSHOTS_DIRECTORY_UNAVAILABLE ({error})"),
                                    }
                                }
                                Err(error) => {
                                    pid1_log!("NAGI_DEV_IDENTITY_DATA_INPUTS_UNAVAILABLE ({error})")
                                }
                            }
                        }
                    }
                    authenticated.retire().map_err(|error| {
                        pid1_log!("nagi-init: C3 boot service retirement unavailable ({error})");
                        BackendError::Unavailable
                    })?;
                    let mapping_generation = mapping.map_err(|error| {
                        pid1_log!("nagi-init: signed C3 system identity unavailable ({error})");
                        BackendError::Unavailable
                    })?;
                    pid1_log!(
                        "nagi-init: signed C3 system identity verified mapping_generation={mapping_generation}"
                    );
                    pid1_log!(
                        "nagi-init: C3 authenticated and retired the boot service; C1 runtime unavailable"
                    );
                }
            }
            #[cfg(not(all(target_os = "linux", not(linux_dev_preview_smoke))))]
            drop(package_owner);
        }
        Err(BackendError::Unavailable)
    }

    fn call_service(
        &mut self,
        _session: &mut Self::Session,
        _service: &mut Self::ServiceHandle,
        request: ServiceRequest<Self::Action>,
        #[cfg(feature = "linux-service-action-bridge")] cancellation: CancellationToken,
        #[cfg(feature = "linux-service-action-bridge")] timeout: Duration,
    ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError> {
        #[cfg(feature = "linux-service-action-bridge")]
        {
            let Some(service) = _service.as_ref() else {
                return Err(ServiceIoError::Unavailable);
            };
            call_action_service(request, service, cancellation, timeout)
        }
        #[cfg(not(feature = "linux-service-action-bridge"))]
        {
            let _ = request;
            Err(ServiceIoError::Unavailable)
        }
    }

    fn wait_event(
        &mut self,
        _session: &mut Self::Session,
    ) -> Result<LifecycleEvent<Self::ServiceId>, BackendError> {
        #[cfg(test)]
        {
            self.wait_calls += 1;
        }
        Err(BackendError::Unavailable)
    }

    fn shutdown(
        &mut self,
        _session: &mut Self::Session,
        _cause: ShutdownCause,
    ) -> Result<(), BackendError> {
        Err(BackendError::Unavailable)
    }
}

#[cfg(test)]
#[path = "linux_init_process_owner_candidate.rs"]
mod linux_init_process_owner_candidate;

#[cfg(any(
    all(
        target_os = "linux",
        target_arch = "x86_64",
        linux_task24_owner_co_location,
        feature = "desktop-login",
        not(any(linux_dev_preview_smoke, nagi_release_build))
    ),
    all(test, target_os = "macos")
))]
#[path = "task24_login_cancellation.rs"]
mod task24_login_cancellation;

#[cfg(test)]
mod process_lifecycle_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::rc::Rc;

    pub(super) const fn synthetic_static_elf() -> [u8; 128] {
        let mut elf = [0u8; 128];
        elf[0] = 0x7f;
        elf[1] = b'E';
        elf[2] = b'L';
        elf[3] = b'F';
        elf[4] = 2; // ELFCLASS64
        elf[5] = 1; // little endian
        elf[6] = 1; // current ELF version
        elf[16] = 2; // ET_EXEC
        elf[18] = 62; // EM_X86_64
        elf[20] = 1;
        elf[24] = 0x78;
        elf[26] = 0x40; // entry 0x400078
        elf[32] = 64; // program table follows ELF header
        elf[52] = 64;
        elf[54] = 56;
        elf[56] = 1;
        elf[64] = 1; // PT_LOAD
        elf[68] = 5; // PF_R | PF_X
        elf[80] = 0;
        elf[81] = 0;
        elf[82] = 0x40; // virtual address 0x400000
        elf[96] = 128; // file size
        elf[104] = 128; // memory size
        let mut offset = 120;
        while offset < elf.len() {
            elf[offset] = 0x90;
            offset += 1;
        }
        elf
    }

    static TEST_EXECUTABLE: [u8; 128] = synthetic_static_elf();

    fn synthetic_static_pie(dynamic_tag: i64) -> Vec<u8> {
        const IMAGE_BYTES: usize = 208;
        const DYNAMIC_OFFSET: usize = 176;
        const DYNAMIC_ADDRESS: u64 = 0x4000b0;

        let mut elf = vec![0; IMAGE_BYTES];
        elf[..TEST_EXECUTABLE.len()].copy_from_slice(&TEST_EXECUTABLE);
        elf[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
        elf[56..58].copy_from_slice(&2u16.to_le_bytes()); // two program headers
        elf[96..104].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());
        elf[104..112].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());

        let dynamic_header = 120;
        elf[dynamic_header..dynamic_header + 4].copy_from_slice(&2u32.to_le_bytes());
        elf[dynamic_header + 4..dynamic_header + 8].copy_from_slice(&4u32.to_le_bytes());
        elf[dynamic_header + 8..dynamic_header + 16]
            .copy_from_slice(&(DYNAMIC_OFFSET as u64).to_le_bytes());
        elf[dynamic_header + 16..dynamic_header + 24]
            .copy_from_slice(&DYNAMIC_ADDRESS.to_le_bytes());
        elf[dynamic_header + 24..dynamic_header + 32]
            .copy_from_slice(&DYNAMIC_ADDRESS.to_le_bytes());
        elf[dynamic_header + 32..dynamic_header + 40].copy_from_slice(&32u64.to_le_bytes());
        elf[dynamic_header + 40..dynamic_header + 48].copy_from_slice(&32u64.to_le_bytes());
        elf[dynamic_header + 48..dynamic_header + 56].copy_from_slice(&8u64.to_le_bytes());

        elf[DYNAMIC_OFFSET..DYNAMIC_OFFSET + 8].copy_from_slice(&dynamic_tag.to_le_bytes());
        // The following all-zero entry is DT_NULL.
        elf
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum FakeMemfdEvent {
        Created,
        Write(usize),
        Sealed,
        SealFailed,
        MutationDenied,
        Closed,
    }

    #[derive(Debug, Default)]
    struct FakeMemfdFile {
        bytes: Vec<u8>,
        sealed: bool,
        closed: bool,
        fail_seal: bool,
        max_write: usize,
        events: Vec<FakeMemfdEvent>,
    }

    #[derive(Debug)]
    struct FakeMemfdControl {
        files: Vec<Rc<RefCell<FakeMemfdFile>>>,
        fail_next_seal: bool,
        max_write: usize,
    }

    impl Default for FakeMemfdControl {
        fn default() -> Self {
            Self {
                files: Vec::new(),
                fail_next_seal: false,
                max_write: usize::MAX,
            }
        }
    }

    #[derive(Debug)]
    struct FakeMemfdHandle(Rc<RefCell<FakeMemfdFile>>);

    impl FakeMemfdHandle {
        fn bytes(&self) -> Vec<u8> {
            self.0.borrow().bytes.clone()
        }

        fn is_sealed(&self) -> bool {
            self.0.borrow().sealed
        }

        fn attempt_mutation(&self, replacement: &[u8]) -> io::Result<()> {
            let mut file = self.0.borrow_mut();
            if file.sealed {
                file.events.push(FakeMemfdEvent::MutationDenied);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            file.bytes.clear();
            file.bytes.extend_from_slice(replacement);
            Ok(())
        }
    }

    impl Drop for FakeMemfdHandle {
        fn drop(&mut self) {
            let mut file = self.0.borrow_mut();
            if !file.closed {
                file.closed = true;
                file.events.push(FakeMemfdEvent::Closed);
            }
        }
    }

    struct FakeSealedExecutableOps {
        control: Rc<RefCell<FakeMemfdControl>>,
    }

    impl SealedExecutableOps for FakeSealedExecutableOps {
        type Handle = FakeMemfdHandle;

        fn create(&mut self) -> io::Result<Self::Handle> {
            let (fail_seal, max_write) = {
                let mut control = self.control.borrow_mut();
                let fail_seal = control.fail_next_seal;
                control.fail_next_seal = false;
                (fail_seal, control.max_write)
            };
            let file = Rc::new(RefCell::new(FakeMemfdFile {
                fail_seal,
                max_write,
                ..FakeMemfdFile::default()
            }));
            file.borrow_mut().events.push(FakeMemfdEvent::Created);
            self.control.borrow_mut().files.push(file.clone());
            Ok(FakeMemfdHandle(file))
        }

        fn write(&mut self, handle: &Self::Handle, bytes: &[u8]) -> io::Result<usize> {
            let mut file = handle.0.borrow_mut();
            if file.sealed {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let count = bytes.len().min(file.max_write);
            if count != 0 {
                file.bytes.extend_from_slice(&bytes[..count]);
            }
            file.events.push(FakeMemfdEvent::Write(count));
            Ok(count)
        }

        fn seal(&mut self, handle: &Self::Handle) -> io::Result<()> {
            let mut file = handle.0.borrow_mut();
            if file.fail_seal {
                file.events.push(FakeMemfdEvent::SealFailed);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            file.sealed = true;
            file.events.push(FakeMemfdEvent::Sealed);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeChildLifetime {
        dropped: bool,
    }

    #[derive(Debug)]
    struct FakeChild {
        process_id: u32,
        pending_status: Option<ServiceExitStatus>,
        executable: Option<FakeMemfdHandle>,
        lifetime: Rc<RefCell<FakeChildLifetime>>,
    }

    impl Drop for FakeChild {
        fn drop(&mut self) {
            self.lifetime.borrow_mut().dropped = true;
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum ProcessEvent {
        Prepared(u32),
        Registered(u32),
        RegistrationRejected(u32),
        ReleaseAttempt(u32),
        Released(u32),
        ReleaseFailed(u32),
        ReleaseTimedOut(u32),
        ReleaseCancelled(u32),
        ReleaseExecError(u32, i32),
        ReleasePartialError(u32),
        ReleaseChildExited(u32),
        Terminated(u32),
        Reaped(u32),
        Revoked(u32),
        RevokeFailed(u32),
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum ExecWaitEvent {
        ReadWouldBlock,
        ReadInterrupted,
        ReadBytes(usize),
        ReadEof,
        PollInterrupted,
        PollReady,
        PollTimeout,
        PollWait(std::time::Duration),
        ChildStatusChecked,
        ChildExited(u32),
    }

    enum FakeRead {
        WouldBlock,
        Interrupted,
        Bytes(Vec<u8>),
        Eof,
    }

    enum FakePoll {
        Interrupted,
        Ready,
        Timeout,
    }

    struct FakeExecScript {
        reads: VecDeque<FakeRead>,
        polls: VecDeque<FakePoll>,
        child_status: Option<ServiceExitStatus>,
    }

    impl Default for FakeExecScript {
        fn default() -> Self {
            Self {
                reads: [FakeRead::Eof].into(),
                polls: VecDeque::new(),
                child_status: None,
            }
        }
    }

    struct FakeExecWait {
        process_id: u32,
        script: FakeExecScript,
        events: Rc<RefCell<Vec<ExecWaitEvent>>>,
    }

    impl ExecStatusWait for FakeExecWait {
        fn read_status_nonblocking(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            match self
                .script
                .reads
                .pop_front()
                .unwrap_or(FakeRead::WouldBlock)
            {
                FakeRead::WouldBlock => {
                    self.events.borrow_mut().push(ExecWaitEvent::ReadWouldBlock);
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                }
                FakeRead::Interrupted => {
                    self.events
                        .borrow_mut()
                        .push(ExecWaitEvent::ReadInterrupted);
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                }
                FakeRead::Bytes(bytes) => {
                    let count = bytes.len().min(buffer.len());
                    buffer[..count].copy_from_slice(&bytes[..count]);
                    self.events
                        .borrow_mut()
                        .push(ExecWaitEvent::ReadBytes(count));
                    Ok(count)
                }
                FakeRead::Eof => {
                    self.events.borrow_mut().push(ExecWaitEvent::ReadEof);
                    Ok(0)
                }
            }
        }

        fn wait_readable(&mut self, timeout: std::time::Duration) -> io::Result<bool> {
            self.events
                .borrow_mut()
                .push(ExecWaitEvent::PollWait(timeout));
            match self.script.polls.pop_front().unwrap_or(FakePoll::Ready) {
                FakePoll::Interrupted => {
                    self.events
                        .borrow_mut()
                        .push(ExecWaitEvent::PollInterrupted);
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                }
                FakePoll::Ready => {
                    self.events.borrow_mut().push(ExecWaitEvent::PollReady);
                    Ok(true)
                }
                FakePoll::Timeout => {
                    self.events.borrow_mut().push(ExecWaitEvent::PollTimeout);
                    Ok(false)
                }
            }
        }

        fn child_exit_status(&mut self) -> io::Result<Option<ServiceExitStatus>> {
            self.events
                .borrow_mut()
                .push(ExecWaitEvent::ChildStatusChecked);
            let status = self.script.child_status.take();
            if status.is_some() {
                self.events
                    .borrow_mut()
                    .push(ExecWaitEvent::ChildExited(self.process_id));
            }
            Ok(status)
        }
    }

    #[derive(Default)]
    struct FakeOs {
        children: VecDeque<FakeChild>,
        events: Rc<RefCell<Vec<ProcessEvent>>>,
        exec_wait_events: Rc<RefCell<Vec<ExecWaitEvent>>>,
        exec_script: FakeExecScript,
        ack_start_age: std::time::Duration,
        prepared: usize,
        prepared_arguments: Vec<Vec<String>>,
        prepared_identities: Vec<LinuxExecutionIdentity>,
        gate_writes: Vec<u32>,
        terminated: Vec<u32>,
        try_wait_calls: Vec<u32>,
        fail_prepare: bool,
        fail_release: bool,
        fail_terminate: bool,
        fail_try_wait: bool,
        hold_running_after_terminate: bool,
        memfd_control: Rc<RefCell<FakeMemfdControl>>,
        executed_payloads: Vec<Vec<u8>>,
    }

    impl ProcessOs for FakeOs {
        type Raw = FakeChild;

        fn prepare(
            &mut self,
            executable: ExecutablePayload<'_>,
            arguments: &[&str],
            _execution_identity: LinuxExecutionIdentity,
            _endpoint_policy: ProcessEndpointPolicy,
            _filesystem_policy: ProcessFilesystemPolicy,
        ) -> io::Result<ProcessHandle<Prepared, Self::Raw>> {
            let argv = process_argv_storage(arguments)?
                .into_iter()
                .skip(1)
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            self.prepared_arguments.push(argv);
            self.prepared_identities.push(_execution_identity);
            self.prepared += 1;
            if self.fail_prepare {
                return Err(io::Error::other("injected prepare failure"));
            }
            let sealed = create_sealed_executable(
                &mut FakeSealedExecutableOps {
                    control: self.memfd_control.clone(),
                },
                executable.bytes(),
            )?;
            let mut child = self
                .children
                .pop_front()
                .ok_or_else(|| io::Error::other("missing fake child"))?;
            child.executable = Some(sealed);
            self.events
                .borrow_mut()
                .push(ProcessEvent::Prepared(child.process_id));
            Ok(ProcessHandle::new(child.process_id, child))
        }

        fn commit_release_gate<Record>(
            &mut self,
            child: &mut ProcessHandle<Prepared, Self::Raw>,
            _registered: &RegisteredLaunch<Record>,
            wait_budget: std::time::Duration,
        ) -> Result<std::time::Instant, ReleaseFailureReason> {
            let process_id = child.process_id;
            let started = std::time::Instant::now();
            self.events
                .borrow_mut()
                .push(ProcessEvent::ReleaseAttempt(process_id));
            if started.elapsed() >= wait_budget {
                self.events
                    .borrow_mut()
                    .push(ProcessEvent::ReleaseTimedOut(process_id));
                return Err(ReleaseFailureReason::TimedOut);
            }
            if self.fail_release {
                self.events
                    .borrow_mut()
                    .push(ProcessEvent::ReleaseFailed(process_id));
                return Err(ReleaseFailureReason::Io);
            }
            self.gate_writes.push(process_id);
            Ok(started)
        }

        fn wait_exec_ack(
            &mut self,
            mut child: ProcessHandle<ReleaseUncertain, Self::Raw>,
            started: std::time::Instant,
            wait_budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<
            ProcessHandle<Running, Self::Raw>,
            ReleaseFailure<ProcessHandle<ReleaseUncertain, Self::Raw>>,
        > {
            let process_id = child.process_id;

            let started = started.checked_sub(self.ack_start_age).unwrap_or(started);
            let mut wait = FakeExecWait {
                process_id,
                script: std::mem::take(&mut self.exec_script),
                events: self.exec_wait_events.clone(),
            };
            let mut elapsed = || started.elapsed();
            match wait_for_exec_ack(
                &mut wait,
                wait_budget,
                cancel_check_interval,
                &mut elapsed,
                cancelled,
            ) {
                Ok(()) => {
                    let image = child
                        .raw
                        .executable
                        .as_ref()
                        .filter(|image| image.is_sealed())
                        .map(FakeMemfdHandle::bytes);
                    let Some(image) = image else {
                        self.events
                            .borrow_mut()
                            .push(ProcessEvent::ReleaseFailed(process_id));
                        return Err(ReleaseFailure {
                            child: child.transition(),
                            reason: ReleaseFailureReason::Io,
                            reaped: None,
                        });
                    };
                    self.executed_payloads.push(image);
                    child.raw.executable.take();
                    self.events
                        .borrow_mut()
                        .push(ProcessEvent::Released(process_id));
                    Ok(child.transition())
                }
                Err(reason) => {
                    let reaped = match reason {
                        ReleaseFailureReason::ChildExited(status) => Some(status),
                        _ => None,
                    };
                    let event = match reason {
                        ReleaseFailureReason::Cancelled => {
                            ProcessEvent::ReleaseCancelled(process_id)
                        }
                        ReleaseFailureReason::TimedOut => ProcessEvent::ReleaseTimedOut(process_id),
                        ReleaseFailureReason::ExecError(errno) => {
                            ProcessEvent::ReleaseExecError(process_id, errno)
                        }
                        ReleaseFailureReason::ChildExited(_) => {
                            ProcessEvent::ReleaseChildExited(process_id)
                        }
                        ReleaseFailureReason::PartialErrorReport => {
                            ProcessEvent::ReleasePartialError(process_id)
                        }
                        ReleaseFailureReason::ProtocolViolation => {
                            ProcessEvent::ReleaseFailed(process_id)
                        }
                        ReleaseFailureReason::Io => ProcessEvent::ReleaseFailed(process_id),
                    };
                    self.events.borrow_mut().push(event);
                    Err(ReleaseFailure {
                        child: child.transition(),
                        reason,
                        reaped,
                    })
                }
            }
        }

        fn try_wait<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<Option<ServiceExitStatus>> {
            self.try_wait_calls.push(child.process_id);
            if self.fail_try_wait {
                return Err(io::Error::other("injected try_wait failure"));
            }
            let status = child.raw.pending_status.take();
            if status.is_some() {
                self.events
                    .borrow_mut()
                    .push(ProcessEvent::Reaped(child.process_id));
            }
            Ok(status)
        }

        fn terminate<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            if self.fail_terminate {
                return Err(io::Error::other("injected terminate failure"));
            }
            self.terminated.push(child.process_id);
            self.events
                .borrow_mut()
                .push(ProcessEvent::Terminated(child.process_id));
            if !self.hold_running_after_terminate {
                child.raw.pending_status = Some(ServiceExitStatus {
                    exit_code: None,
                    signal: Some(9),
                });
            }
            Ok(())
        }
    }

    fn child(process_id: u32, pending_status: Option<ServiceExitStatus>) -> FakeChild {
        FakeChild {
            process_id,
            pending_status,
            executable: None,
            lifetime: Rc::new(RefCell::new(FakeChildLifetime::default())),
        }
    }

    const TEST_RELEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
    const TEST_CANCEL_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

    fn launch(service: &'static str, required: bool) -> ProcessLaunch<'static, &'static str> {
        launch_with_budget(service, required, TEST_RELEASE_WAIT)
    }

    fn launch_with_budget(
        service: &'static str,
        required: bool,
        release_wait_budget: std::time::Duration,
    ) -> ProcessLaunch<'static, &'static str> {
        ProcessLaunch {
            service,
            executable: ExecutablePayload::from_package_executable(&TEST_EXECUTABLE).unwrap(),
            arguments: &[],
            execution_identity: LinuxExecutionIdentity::DEV_SERVICE,
            endpoint_policy: ProcessEndpointPolicy::Disabled,
            filesystem_policy: ProcessFilesystemPolicy::Inherited,
            required,
            release_wait_budget,
            cancel_check_interval: TEST_CANCEL_CHECK_INTERVAL,
        }
    }

    fn no_revoke(_: u32, _: &u32) -> Result<(), BackendError> {
        Ok(())
    }

    fn events(os: &FakeOs) -> Rc<RefCell<Vec<ProcessEvent>>> {
        os.events.clone()
    }

    fn wait_events(os: &FakeOs) -> Rc<RefCell<Vec<ExecWaitEvent>>> {
        os.exec_wait_events.clone()
    }

    type Table = ProcessLifecycle<FakeOs, &'static str, u32>;

    #[test]
    fn package_executable_shape_rejects_dynamic_and_wrong_arch_images() {
        let payload = ExecutablePayload::from_package_executable(&TEST_EXECUTABLE).unwrap();
        assert_eq!(payload.bytes(), TEST_EXECUTABLE);

        let mut wrong_arch = TEST_EXECUTABLE;
        wrong_arch[18] = 183;
        wrong_arch[19] = 0;
        assert_eq!(
            ExecutablePayload::from_package_executable(&wrong_arch),
            Err(ExecutableFormatError::WrongArchitecture)
        );

        let mut dynamic = TEST_EXECUTABLE.to_vec();
        dynamic.resize(176, 0);
        dynamic[56] = 2;
        dynamic[120..124].copy_from_slice(&2u32.to_le_bytes()); // PT_DYNAMIC
        assert_eq!(
            ExecutablePayload::from_package_executable(&dynamic),
            Err(ExecutableFormatError::DynamicSegment)
        );

        let mut interpreter = TEST_EXECUTABLE.to_vec();
        interpreter.resize(176, 0);
        interpreter[56] = 2;
        interpreter[120..124].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        assert_eq!(
            ExecutablePayload::from_package_executable(&interpreter),
            Err(ExecutableFormatError::DynamicSegment)
        );

        let mut dynamic_executable = TEST_EXECUTABLE;
        dynamic_executable[16] = 3; // ET_DYN
        assert_eq!(
            ExecutablePayload::from_package_executable(&dynamic_executable),
            Err(ExecutableFormatError::NotStaticExecutable)
        );

        let mut non_executable_load = TEST_EXECUTABLE;
        non_executable_load[68] = 4; // PF_R without PF_X
        assert_eq!(
            ExecutablePayload::from_package_executable(&non_executable_load),
            Err(ExecutableFormatError::MissingExecutableEntry)
        );

        assert_eq!(
            ExecutablePayload::from_package_executable(&TEST_EXECUTABLE[..100]),
            Err(ExecutableFormatError::Truncated)
        );
    }

    #[cfg(linux_dev_preview_smoke)]
    #[test]
    fn development_preview_accepts_static_pie_but_rejects_dynamic_dependencies() {
        let static_pie = synthetic_static_pie(21); // DT_DEBUG, no loader dependency.
        assert!(ExecutablePayload::from_development_preview_elf(&static_pie).is_ok());
        assert_eq!(
            ExecutablePayload::from_package_executable(&static_pie),
            Err(ExecutableFormatError::NotStaticExecutable)
        );

        let needs_library = synthetic_static_pie(1); // DT_NEEDED
        assert_eq!(
            ExecutablePayload::from_development_preview_elf(&needs_library),
            Err(ExecutableFormatError::DynamicSegment)
        );

        let mut interpreter = static_pie;
        interpreter[120..124].copy_from_slice(&3u32.to_le_bytes()); // PT_INTERP
        assert_eq!(
            ExecutablePayload::from_development_preview_elf(&interpreter),
            Err(ExecutableFormatError::DynamicSegment)
        );
    }

    #[test]
    fn sealed_payload_preserves_exact_bytes_and_rejects_replacement() {
        let control = Rc::new(RefCell::new(FakeMemfdControl::default()));
        control.borrow_mut().max_write = 11;
        let mut ops = FakeSealedExecutableOps {
            control: control.clone(),
        };
        let image = create_sealed_executable(&mut ops, &TEST_EXECUTABLE).unwrap();
        let file = control.borrow().files[0].clone();

        assert_eq!(image.bytes(), TEST_EXECUTABLE);
        assert!(image.is_sealed());
        assert_eq!(
            image.attempt_mutation(b"replacement").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(file.borrow().bytes, TEST_EXECUTABLE);
        assert!(
            file.borrow()
                .events
                .iter()
                .filter(|e| matches!(e, FakeMemfdEvent::Write(_)))
                .count()
                > 1
        );

        drop(image);
        assert!(file.borrow().closed);
        assert_eq!(file.borrow().events.last(), Some(&FakeMemfdEvent::Closed));
    }

    #[test]
    fn sealing_failure_closes_image_before_child_creation_or_registration() {
        let control = Rc::new(RefCell::new(FakeMemfdControl::default()));
        control.borrow_mut().fail_next_seal = true;
        let mut table = Table::new(FakeOs {
            children: [child(10, None)].into(),
            memfd_control: control.clone(),
            ..FakeOs::default()
        });
        let mut register_calls = 0;
        let result = table.spawn(
            launch("files", true),
            || false,
            |_| {
                register_calls += 1;
                Ok(10)
            },
            no_revoke,
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert_eq!(register_calls, 0);
        assert_eq!(table.os.children.len(), 1);
        assert!(table.os.events.borrow().is_empty());
        let file = control.borrow().files[0].clone();
        assert_eq!(file.borrow().bytes, TEST_EXECUTABLE);
        assert!(file.borrow().closed);
        assert!(file.borrow().events.contains(&FakeMemfdEvent::SealFailed));
        assert_eq!(file.borrow().events.last(), Some(&FakeMemfdEvent::Closed));
    }

    #[test]
    fn process_launch_forwards_arguments_and_releases_the_exact_sealed_payload() {
        let control = Rc::new(RefCell::new(FakeMemfdControl::default()));
        let mut table = Table::new(FakeOs {
            children: [child(10, None)].into(),
            memfd_control: control.clone(),
            ..FakeOs::default()
        });
        let arguments = ["--input", "/run/nagi/dev-preview/runtime.input"];
        let launch = ProcessLaunch {
            service: "files",
            executable: ExecutablePayload::from_package_executable(&TEST_EXECUTABLE).unwrap(),
            arguments: &arguments,
            execution_identity: LinuxExecutionIdentity::DEV_NOTES_DESKTOP,
            endpoint_policy: ProcessEndpointPolicy::Disabled,
            filesystem_policy: ProcessFilesystemPolicy::Inherited,
            required: true,
            release_wait_budget: TEST_RELEASE_WAIT,
            cancel_check_interval: TEST_CANCEL_CHECK_INTERVAL,
        };
        let result = table.spawn(launch, || false, Ok, no_revoke);

        assert_eq!(result, Ok(10));
        assert_eq!(table.os.executed_payloads, [TEST_EXECUTABLE.to_vec()]);
        assert_eq!(
            table.os.prepared_arguments,
            [vec![
                "--input".to_owned(),
                "/run/nagi/dev-preview/runtime.input".to_owned()
            ]]
        );
        assert_eq!(
            table.os.prepared_identities,
            [LinuxExecutionIdentity::DEV_NOTES_DESKTOP]
        );
        let file = control.borrow().files[0].clone();
        assert!(file.borrow().sealed);
        assert!(file.borrow().closed);
    }

    #[test]
    fn process_argv_rejects_embedded_nul() {
        assert!(process_argv_storage(&["--input\0bad"]).is_err());
    }

    #[test]
    fn child_console_output_is_limited_to_the_notes_development_identity() {
        assert_eq!(
            inherit_child_console_output(LinuxExecutionIdentity::DEV_NOTES_DESKTOP),
            cfg!(linux_desktop_dev_smoke)
        );
        assert!(!inherit_child_console_output(
            LinuxExecutionIdentity::DEV_SERVICE
        ));
    }

    #[test]
    fn prepare_failure_never_registers_or_releases() {
        let mut table = Table::new(FakeOs {
            fail_prepare: true,
            ..FakeOs::default()
        });
        let mut register_calls = 0;
        let result = table.spawn(
            launch("files", true),
            || false,
            |_| {
                register_calls += 1;
                Ok(11)
            },
            no_revoke,
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert_eq!(register_calls, 0);
        assert_eq!(table.active_len(), 0);
        assert!(table.os.events.borrow().is_empty());
    }

    #[test]
    fn cancellation_after_prepare_terminates_before_registration_and_release() {
        let mut table = Table::new(FakeOs {
            children: [child(21, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let mut cancel_checks = 0;
        let mut register_calls = 0;
        let result = table.spawn(
            launch("search", true),
            || {
                cancel_checks += 1;
                cancel_checks == 2
            },
            |_| {
                register_calls += 1;
                Ok(21)
            },
            no_revoke,
        );

        assert_eq!(result, Err(BackendError::Cancelled));
        assert_eq!(register_calls, 0);
        assert_eq!(table.os.terminated, [21]);
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(21),
                ProcessEvent::Terminated(21),
                ProcessEvent::Reaped(21)
            ]
        );
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn registration_failure_cleans_prepared_child_without_release() {
        let mut table = Table::new(FakeOs {
            children: [child(22, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let result = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                log.borrow_mut()
                    .push(ProcessEvent::RegistrationRejected(pid));
                Err(BackendError::IdentityRejected)
            },
            no_revoke,
        );

        assert_eq!(result, Err(BackendError::IdentityRejected));
        assert_eq!(table.os.terminated, [22]);
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(22),
                ProcessEvent::RegistrationRejected(22),
                ProcessEvent::Terminated(22),
                ProcessEvent::Reaped(22)
            ]
        );
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn registration_completes_before_release_is_attempted() {
        let mut table = Table::new(FakeOs {
            children: [child(23, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let result = table.spawn(
            launch("history", false),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            no_revoke,
        );

        assert_eq!(result, Ok(23));
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(23),
                ProcessEvent::Registered(23),
                ProcessEvent::ReleaseAttempt(23),
                ProcessEvent::Released(23)
            ]
        );
        assert!(table.live_record(23).is_some());
    }

    #[test]
    fn cancellation_after_registration_revokes_and_never_releases() {
        let mut table = Table::new(FakeOs {
            children: [child(24, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let mut checks = 0;
        let result = table.spawn(
            launch("files", true),
            || {
                checks += 1;
                checks == 3
            },
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::Cancelled));
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(24),
                ProcessEvent::Registered(24),
                ProcessEvent::Terminated(24),
                ProcessEvent::Reaped(24),
                ProcessEvent::Revoked(24)
            ]
        );
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(24).is_none());
    }

    #[test]
    fn grant_authorization_failure_reaps_revokes_and_never_releases() {
        let mut table = Table::new(FakeOs {
            children: [child(44, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let result = table.spawn_authorized(
            launch("files", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |_pid, _record, _gate| Err(BackendError::ConsentDenied),
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::ConsentDenied));
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(44),
                ProcessEvent::Registered(44),
                ProcessEvent::Terminated(44),
                ProcessEvent::Reaped(44),
                ProcessEvent::Revoked(44)
            ]
        );
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(44).is_none());
    }

    #[test]
    fn cancellation_after_registration_prevents_authorization_and_gate_commit() {
        let mut table = Table::new(FakeOs {
            children: [child(45, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let mut cancel_checks = 0;
        let result = table.spawn_authorized(
            launch("files", true),
            || {
                cancel_checks += 1;
                cancel_checks == 3
            },
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |_pid, _record, _gate| panic!("cancelled launch must not authorize"),
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::Cancelled));
        assert!(table.os.gate_writes.is_empty());
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(45),
                ProcessEvent::Registered(45),
                ProcessEvent::Terminated(45),
                ProcessEvent::Reaped(45),
                ProcessEvent::Revoked(45)
            ]
        );
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(45).is_none());
    }

    #[test]
    fn duplicate_gate_commit_is_rejected_and_cleans_the_uncertain_child() {
        let mut table = Table::new(FakeOs {
            children: [child(46, None)].into(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let result = table.spawn_authorized(
            launch("files", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |_pid, _record, gate| {
                gate.commit()?;
                gate.commit()
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::ProtocolViolation));
        assert_eq!(table.os.gate_writes, [46]);
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(46),
                ProcessEvent::Registered(46),
                ProcessEvent::ReleaseAttempt(46),
                ProcessEvent::Terminated(46),
                ProcessEvent::Reaped(46),
                ProcessEvent::Revoked(46),
            ]
        );
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(46).is_none());
    }

    #[test]
    fn release_failure_reaps_child_and_revokes_record() {
        let memfd_control = Rc::new(RefCell::new(FakeMemfdControl::default()));
        let mut table = Table::new(FakeOs {
            children: [child(25, None)].into(),
            fail_release: true,
            memfd_control: memfd_control.clone(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let result = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(table.os.gate_writes.is_empty());
        assert!(wait_events(&table.os).borrow().is_empty());
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(25),
                ProcessEvent::Registered(25),
                ProcessEvent::ReleaseAttempt(25),
                ProcessEvent::ReleaseFailed(25),
                ProcessEvent::Terminated(25),
                ProcessEvent::Reaped(25),
                ProcessEvent::Revoked(25)
            ]
        );
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(25).is_none());
        assert!(memfd_control.borrow().files[0].borrow().closed);
    }

    #[test]
    fn gate_failure_revokes_only_its_registered_record() {
        let live_records = RefCell::new(vec![700u32]);
        let mut table = Table::new(FakeOs {
            children: [child(47, None)].into(),
            fail_release: true,
            ..FakeOs::default()
        });

        let result = table.spawn_authorized(
            launch("files", true),
            || false,
            |pid| {
                live_records.borrow_mut().push(pid);
                Ok(pid)
            },
            |_pid, _record, gate| gate.commit(),
            |pid, record| {
                assert_eq!(pid, *record);
                live_records
                    .borrow_mut()
                    .retain(|current| current != record);
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert_eq!(*live_records.borrow(), [700]);
        assert!(table.os.gate_writes.is_empty());
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(47).is_none());
    }

    #[test]
    fn failed_release_cleanup_quarantines_record_and_blocks_pid_reuse() {
        let memfd_control = Rc::new(RefCell::new(FakeMemfdControl::default()));
        let mut table = Table::new(FakeOs {
            children: [child(26, None), child(26, None)].into(),
            fail_release: true,
            fail_terminate: true,
            memfd_control: memfd_control.clone(),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let mut register_calls = 0;
        let first = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                register_calls += 1;
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, _| {
                log.borrow_mut().push(ProcessEvent::RevokeFailed(pid));
                Err(BackendError::TransportFailure)
            },
        );
        let duplicate = table.spawn(
            launch("model", true),
            || false,
            |_| {
                register_calls += 1;
                Ok(99)
            },
            no_revoke,
        );
        let before_shutdown = log.borrow().clone();
        let exposed = table.live_record(26).is_some();
        let quarantined_image = memfd_control.borrow().files[0].clone();
        assert!(!quarantined_image.borrow().closed);

        table.os.fail_terminate = false;
        let shutdown = table.shutdown(|pid, record| {
            assert_eq!(pid, *record);
            log.borrow_mut().push(ProcessEvent::Revoked(pid));
            Ok(())
        });

        assert_eq!(first, Err(BackendError::TransportFailure));
        assert_eq!(duplicate, Err(BackendError::TransportFailure));
        assert_eq!(register_calls, 1);
        assert!(!exposed);
        assert_eq!(table.active_len(), 0);
        assert_eq!(shutdown, Ok(()));
        assert_eq!(
            before_shutdown
                .iter()
                .filter(|event| matches!(event, ProcessEvent::ReleaseAttempt(_)))
                .count(),
            1
        );
        assert!(before_shutdown
            .iter()
            .all(|event| !matches!(event, ProcessEvent::Released(_))));
        assert!(!before_shutdown.contains(&ProcessEvent::RevokeFailed(26)));
        assert_eq!(table.os.terminated, [26, 26]);
        assert!(log.borrow().contains(&ProcessEvent::Revoked(26)));
        assert!(quarantined_image.borrow().closed);
    }

    #[test]
    fn child_exit_reaps_before_revoking_the_record() {
        let mut table = Table::new(FakeOs {
            children: [child(
                27,
                Some(ServiceExitStatus {
                    exit_code: Some(4),
                    signal: None,
                }),
            )]
            .into(),
            ..FakeOs::default()
        });
        assert_eq!(
            table.spawn(launch("history", false), || false, Ok, no_revoke),
            Ok(27)
        );
        let mut revoked = Vec::new();
        let event = table
            .poll(|pid, record| {
                assert_eq!(pid, *record);
                revoked.push(pid);
                Ok(())
            })
            .unwrap();

        assert_eq!(revoked, [27]);
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(27).is_none());
        assert!(matches!(
            event,
            Some(LifecycleEvent::ServiceExited {
                service: "history",
                status: ServiceExitStatus {
                    exit_code: Some(4),
                    signal: None,
                },
                required: false,
            })
        ));
    }

    #[test]
    fn reused_pid_is_blocked_until_the_old_record_is_revoked() {
        let mut table = Table::new(FakeOs {
            children: [
                child(
                    28,
                    Some(ServiceExitStatus {
                        exit_code: Some(0),
                        signal: None,
                    }),
                ),
                child(28, None),
            ]
            .into(),
            ..FakeOs::default()
        });
        assert_eq!(
            table.spawn(launch("search", true), || false, Ok, no_revoke),
            Ok(28)
        );

        let mut revoke_calls = 0;
        assert!(matches!(
            table.poll(|_, _| {
                revoke_calls += 1;
                Err(BackendError::TransportFailure)
            }),
            Err(BackendError::TransportFailure)
        ));
        assert_eq!(revoke_calls, 1);
        assert_eq!(table.active_len(), 1);
        assert!(table.live_record(28).is_none());

        let mut register_calls = 0;
        assert_eq!(
            table.spawn(
                launch("search", true),
                || false,
                |_| {
                    register_calls += 1;
                    Ok(29)
                },
                no_revoke,
            ),
            Err(BackendError::ProtocolViolation)
        );
        assert_eq!(register_calls, 0);
        assert_eq!(table.os.terminated, [28]);
    }

    #[test]
    fn shutdown_terminates_reaps_and_revokes_each_child() {
        let mut table = Table::new(FakeOs {
            children: [child(31, None), child(32, None)].into(),
            ..FakeOs::default()
        });
        assert_eq!(
            table.spawn(launch("files", true), || false, Ok, no_revoke),
            Ok(31)
        );
        assert_eq!(
            table.spawn(launch("search", false), || false, Ok, no_revoke),
            Ok(32)
        );

        let mut revoked = Vec::new();
        table
            .shutdown(|pid, record| {
                assert_eq!(*record, pid);
                revoked.push(pid);
                Ok(())
            })
            .unwrap();

        assert_eq!(table.os.terminated, [31, 32]);
        assert_eq!(revoked, [31, 32]);
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn shutdown_quarantines_unreaped_child_until_poll_reaps_and_revokes() {
        let mut table = Table::new(FakeOs {
            children: [child(42, None)].into(),
            hold_running_after_terminate: true,
            ..FakeOs::default()
        });
        let lifetime = table.os.children.front().unwrap().lifetime.clone();
        assert_eq!(
            table.spawn(launch("files", true), || false, Ok, no_revoke),
            Ok(42)
        );
        assert!(table.live_record(42).is_some());

        let revoked = Rc::new(RefCell::new(Vec::new()));
        let revoked_calls = revoked.clone();
        let mut revoke = move |pid: u32, record: &u32| {
            assert_eq!(pid, *record);
            revoked_calls.borrow_mut().push(pid);
            Ok(())
        };
        assert_eq!(
            table.shutdown(&mut revoke),
            Err(BackendError::TransportFailure)
        );
        assert!(table.children.is_empty());
        assert_eq!(table.quarantine.len(), 1);
        assert_eq!(table.quarantine[0].process_id, 42);
        assert_eq!(table.quarantine[0].record, Some(42));
        assert!(table.live_record(42).is_none());
        assert!(revoked.borrow().is_empty());
        assert!(!lifetime.borrow().dropped);
        assert_eq!(table.os.try_wait_calls, [42, 42]);
        assert_eq!(table.os.terminated, [42]);

        assert!(matches!(
            table.poll(&mut revoke),
            Err(BackendError::TransportFailure)
        ));
        assert_eq!(table.quarantine.len(), 1);
        assert_eq!(table.quarantine[0].record, Some(42));
        assert!(revoked.borrow().is_empty());
        assert!(!lifetime.borrow().dropped);
        assert_eq!(table.os.try_wait_calls, [42, 42, 42, 42]);
        assert_eq!(table.os.terminated, [42, 42]);

        match &mut table.quarantine[0].child {
            PendingChild::Running(child) => {
                child.raw.pending_status = Some(ServiceExitStatus {
                    exit_code: None,
                    signal: Some(9),
                });
            }
            _ => panic!("running child must remain quarantined"),
        }
        assert!(matches!(table.poll(&mut revoke), Ok(None)));
        assert_eq!(*revoked.borrow(), [42]);
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(42).is_none());
        assert!(lifetime.borrow().dropped);
        assert_eq!(table.os.try_wait_calls, [42, 42, 42, 42, 42]);
        assert_eq!(table.os.terminated, [42, 42]);
    }

    #[test]
    fn failed_try_wait_quarantines_child_and_record() {
        let mut table = Table::new(FakeOs {
            children: [child(33, None)].into(),
            fail_try_wait: true,
            ..FakeOs::default()
        });
        assert_eq!(
            table.spawn(launch("model", true), || false, Ok, no_revoke),
            Ok(33)
        );

        assert_eq!(
            table.shutdown(|_, _| Ok(())),
            Err(BackendError::TransportFailure)
        );
        assert_eq!(table.active_len(), 1);
        assert!(table.live_record(33).is_none());
        assert_eq!(table.quarantine.len(), 1);
        assert_eq!(table.quarantine[0].record, Some(33));
    }

    #[test]
    fn failed_unregistered_terminate_is_quarantined_and_recovered() {
        let mut table = Table::new(FakeOs {
            children: [child(34, None), child(34, None)].into(),
            fail_terminate: true,
            ..FakeOs::default()
        });
        let first = table.spawn(
            launch("model", true),
            || false,
            |_| Err(BackendError::IdentityRejected),
            no_revoke,
        );
        assert_eq!(first, Err(BackendError::TransportFailure));
        assert_eq!(table.active_len(), 1);
        assert!(table.live_record(34).is_none());

        let duplicate = table.spawn(launch("model", true), || false, Ok, no_revoke);
        assert_eq!(duplicate, Err(BackendError::TransportFailure));
        assert_eq!(table.active_len(), 2);
        assert!(table.live_record(34).is_none());

        table.os.fail_terminate = false;
        assert_eq!(table.shutdown(no_revoke), Ok(()));
        assert_eq!(table.os.terminated, [34, 34]);
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn failed_unregistered_try_wait_keeps_handle_until_poll_reaps_it() {
        let mut table = Table::new(FakeOs {
            children: [child(35, None)].into(),
            fail_try_wait: true,
            ..FakeOs::default()
        });
        let result = table.spawn(
            launch("model", true),
            || false,
            |_| Err(BackendError::IdentityRejected),
            no_revoke,
        );
        assert_eq!(result, Err(BackendError::TransportFailure));
        assert_eq!(table.active_len(), 1);
        assert!(table.live_record(35).is_none());

        table.os.fail_try_wait = false;
        assert!(matches!(table.poll(no_revoke), Ok(None)));
        assert_eq!(table.active_len(), 0);
        assert_eq!(table.os.terminated, [35]);
    }

    #[test]
    fn expired_exec_ack_deadline_rejects_immediate_eof() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut wait = FakeExecWait {
            process_id: 43,
            script: FakeExecScript {
                reads: [FakeRead::Eof].into(),
                child_status: None,
                ..FakeExecScript::default()
            },
            events: events.clone(),
        };
        let started = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let mut elapsed = || started.elapsed();
        let mut cancelled = || false;

        let result = wait_for_exec_ack(
            &mut wait,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(1),
            &mut elapsed,
            &mut cancelled,
        );

        assert_eq!(result, Err(ReleaseFailureReason::TimedOut));
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn exec_ack_eof_rechecks_deadline_after_child_probe() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut wait = FakeExecWait {
            process_id: 46,
            script: FakeExecScript {
                reads: [FakeRead::Eof].into(),
                child_status: None,
                ..FakeExecScript::default()
            },
            events: events.clone(),
        };
        let mut elapsed_samples = [
            std::time::Duration::ZERO,
            std::time::Duration::from_millis(1),
        ]
        .into_iter();
        let mut elapsed = || {
            elapsed_samples
                .next()
                .unwrap_or(std::time::Duration::from_millis(1))
        };
        let mut cancelled = || false;

        let result = wait_for_exec_ack(
            &mut wait,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(1),
            &mut elapsed,
            &mut cancelled,
        );

        assert_eq!(result, Err(ReleaseFailureReason::TimedOut));
        assert_eq!(
            events.borrow().as_slice(),
            &[ExecWaitEvent::ReadEof, ExecWaitEvent::ChildStatusChecked]
        );
    }

    #[test]
    fn exec_ack_eof_rechecks_cancellation_after_child_probe() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut wait = FakeExecWait {
            process_id: 47,
            script: FakeExecScript {
                reads: [FakeRead::Eof].into(),
                child_status: None,
                ..FakeExecScript::default()
            },
            events: events.clone(),
        };
        let mut elapsed = || std::time::Duration::ZERO;
        let mut cancellation_checks = 0;
        let mut cancelled = || {
            cancellation_checks += 1;
            cancellation_checks == 3
        };

        let result = wait_for_exec_ack(
            &mut wait,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_millis(1),
            &mut elapsed,
            &mut cancelled,
        );

        assert_eq!(result, Err(ReleaseFailureReason::Cancelled));
        assert_eq!(
            events.borrow().as_slice(),
            &[ExecWaitEvent::ReadEof, ExecWaitEvent::ChildStatusChecked]
        );
    }

    #[test]
    fn expired_exec_ack_cleans_and_revokes_registered_record() {
        let mut table = Table::new(FakeOs {
            children: [child(44, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::Eof].into(),
                child_status: None,
                ..FakeExecScript::default()
            },
            // Model the caller-selected budget expiring after gate commit and
            // before the first nonblocking status read, without sleeping.
            ack_start_age: std::time::Duration::from_secs(1),
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch_with_budget("model", true, std::time::Duration::from_millis(1)),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(log.borrow().contains(&ProcessEvent::ReleaseTimedOut(44)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(44)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(44)));
        assert_eq!(table.os.gate_writes, [44]);
        assert!(wait_log.borrow().is_empty());
        assert_eq!(table.os.terminated, [44]);
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(44).is_none());
    }

    #[test]
    fn expired_release_budget_cleans_and_revokes_without_releasing() {
        let mut table = Table::new(FakeOs {
            children: [child(36, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::WouldBlock].into(),
                ..FakeExecScript::default()
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch_with_budget("model", true, std::time::Duration::ZERO),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(log.borrow().contains(&ProcessEvent::ReleaseTimedOut(36)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(36)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(36)));
        assert!(wait_log.borrow().is_empty());
        assert!(table.os.gate_writes.is_empty());
        assert_eq!(table.os.terminated, [36]);
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(36).is_none());
    }

    #[test]
    fn cancellation_while_waiting_cleans_and_revokes_without_release() {
        let mut table = Table::new(FakeOs {
            children: [child(37, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::WouldBlock].into(),
                polls: [FakePoll::Ready].into(),
                ..FakeExecScript::default()
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let cancel_wait_log = wait_log.clone();
        let result = table.spawn(
            launch("model", true),
            move || {
                cancel_wait_log
                    .borrow()
                    .contains(&ExecWaitEvent::PollWait(TEST_CANCEL_CHECK_INTERVAL))
            },
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::Cancelled));
        assert!(log.borrow().contains(&ProcessEvent::ReleaseCancelled(37)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(37)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(37)));
        assert_eq!(table.os.terminated, [37]);
        assert!(wait_log
            .borrow()
            .contains(&ExecWaitEvent::PollWait(TEST_CANCEL_CHECK_INTERVAL)));
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn repeated_eintr_during_read_and_poll_does_not_lose_release() {
        let mut table = Table::new(FakeOs {
            children: [child(38, None)].into(),
            exec_script: FakeExecScript {
                reads: [
                    FakeRead::Interrupted,
                    FakeRead::WouldBlock,
                    FakeRead::WouldBlock,
                    FakeRead::WouldBlock,
                    FakeRead::Eof,
                ]
                .into(),
                polls: [
                    FakePoll::Interrupted,
                    FakePoll::Interrupted,
                    FakePoll::Ready,
                ]
                .into(),
                child_status: None,
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch("files", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            no_revoke,
        );

        assert_eq!(result, Ok(38));
        assert_eq!(
            *log.borrow(),
            [
                ProcessEvent::Prepared(38),
                ProcessEvent::Registered(38),
                ProcessEvent::ReleaseAttempt(38),
                ProcessEvent::Released(38)
            ]
        );
        let trace = wait_log.borrow();
        assert_eq!(
            trace
                .iter()
                .filter(|event| **event == ExecWaitEvent::ReadInterrupted)
                .count(),
            1
        );
        assert_eq!(
            trace
                .iter()
                .filter(|event| **event == ExecWaitEvent::PollInterrupted)
                .count(),
            2
        );
        assert!(trace.contains(&ExecWaitEvent::ChildStatusChecked));
        assert!(table.live_record(38).is_some());
    }

    #[test]
    fn partial_error_report_then_eof_is_not_exec_success() {
        let mut table = Table::new(FakeOs {
            children: [child(39, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::Bytes(vec![5, 0]), FakeRead::Eof].into(),
                ..FakeExecScript::default()
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(log
            .borrow()
            .contains(&ProcessEvent::ReleasePartialError(39)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(39)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(39)));
        assert_eq!(table.os.gate_writes, [39]);
        assert_eq!(
            *wait_log.borrow(),
            [ExecWaitEvent::ReadBytes(2), ExecWaitEvent::ReadEof]
        );
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn split_exec_error_report_is_assembled_before_failure() {
        let mut table = Table::new(FakeOs {
            children: [child(40, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::Bytes(vec![5, 0]), FakeRead::Bytes(vec![0, 0])].into(),
                ..FakeExecScript::default()
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(log
            .borrow()
            .contains(&ProcessEvent::ReleaseExecError(40, 5)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(40)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(40)));
        assert_eq!(
            *wait_log.borrow(),
            [ExecWaitEvent::ReadBytes(2), ExecWaitEvent::ReadBytes(2)]
        );
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn eof_with_already_exited_child_is_reaped_not_marked_live() {
        let exit_status = ServiceExitStatus {
            exit_code: Some(127),
            signal: None,
        };
        let mut table = Table::new(FakeOs {
            children: [child(41, None)].into(),
            exec_script: FakeExecScript {
                reads: [FakeRead::Eof].into(),
                child_status: Some(exit_status),
                ..FakeExecScript::default()
            },
            ..FakeOs::default()
        });
        let log = events(&table.os);
        let wait_log = wait_events(&table.os);
        let result = table.spawn(
            launch("model", true),
            || false,
            |pid| {
                log.borrow_mut().push(ProcessEvent::Registered(pid));
                Ok(pid)
            },
            |pid, record| {
                assert_eq!(pid, *record);
                log.borrow_mut().push(ProcessEvent::Revoked(pid));
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(log.borrow().contains(&ProcessEvent::ReleaseChildExited(41)));
        assert!(!log.borrow().contains(&ProcessEvent::Released(41)));
        assert!(log.borrow().contains(&ProcessEvent::Revoked(41)));
        assert_eq!(
            *wait_log.borrow(),
            [
                ExecWaitEvent::ReadEof,
                ExecWaitEvent::ChildStatusChecked,
                ExecWaitEvent::ChildExited(41)
            ]
        );
        assert!(table.os.terminated.is_empty());
        assert_eq!(table.active_len(), 0);
        assert!(table.live_record(41).is_none());
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod linux_process_resource_tests {
    use super::*;
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;

    const TEST_RECORD: u32 = 0x4353_0001;
    const TEST_RELEASE_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);
    const TEST_CANCEL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

    fn endpoint_writer_elf() -> Vec<u8> {
        endpoint_writer_elf_with_exit_code(0)
    }

    fn endpoint_writer_elf_with_exit_code(exit_code: u8) -> Vec<u8> {
        let mut image = process_lifecycle_tests::synthetic_static_elf().to_vec();
        image.resize(256, 0);
        let image_len = image.len() as u64;
        image[96..104].copy_from_slice(&image_len.to_le_bytes());
        image[104..112].copy_from_slice(&image_len.to_le_bytes());
        image[112..120].copy_from_slice(&0x1000u64.to_le_bytes());
        let code = [
            0xb8, 0x01, 0x00, 0x00, 0x00, // mov eax, SYS_write
            0xbf, 0x03, 0x00, 0x00, 0x00, // mov edi, endpoint fd 3
            0x48, 0x8d, 0x35, 0x57, 0x00, 0x00, 0x00, // lea rsi, [rip + 87]
            0xba, 0x05, 0x00, 0x00, 0x00, // mov edx, 5
            0x0f, 0x05, // syscall
            0xb8, 0x00, 0x00, 0x00, 0x00, // mov eax, SYS_read
            0xbf, 0x03, 0x00, 0x00, 0x00, // mov edi, endpoint fd 3
            0x48, 0x89, 0xe6, // mov rsi, rsp
            0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
            0x0f, 0x05, // syscall
            0xb8, 0x01, 0x00, 0x00, 0x00, // mov eax, SYS_write
            0xbf, 0x03, 0x00, 0x00, 0x00, // mov edi, endpoint fd 3
            0x48, 0x8d, 0x35, 0x30, 0x00, 0x00, 0x00, // lea rsi, [rip + 48]
            0xba, 0x05, 0x00, 0x00, 0x00, // mov edx, 5
            0x0f, 0x05, // syscall
            0xb8, 0x00, 0x00, 0x00, 0x00, // mov eax, SYS_read
            0xbf, 0x03, 0x00, 0x00, 0x00, // mov edi, endpoint fd 3
            0x48, 0x89, 0xe6, // mov rsi, rsp
            0xba, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
            0x0f, 0x05, // syscall
            0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, SYS_exit
            0xbf, exit_code, 0x00, 0x00, 0x00, // mov edi, exit_code
            0x0f, 0x05, // syscall
        ];
        image[120..120 + code.len()].copy_from_slice(&code);
        image[224..229].copy_from_slice(b"HELLO");
        image[229..234].copy_from_slice(b"READY");
        image
    }

    pub(super) fn namespace_root_probe_elf() -> Vec<u8> {
        const IMAGE_BYTES: usize = 448;
        const PATH_OFFSET: usize = 320;
        const ISOLATED_OFFSET: usize = 352;
        const VISIBLE_OFFSET: usize = 360;

        let mut code = Vec::new();
        code.extend_from_slice(&[0xb8, 0x01, 0x01, 0x00, 0x00]); // mov eax, SYS_openat
        code.extend_from_slice(&[0xbf, 0x9c, 0xff, 0xff, 0xff]); // mov edi, AT_FDCWD
        code.push(0xbe); // mov esi, absolute path in this fixed-address ELF
        code.extend_from_slice(&((0x400000 + PATH_OFFSET) as u32).to_le_bytes());
        code.extend_from_slice(&[0x31, 0xd2, 0x0f, 0x05]); // xor edx, edx; syscall
        code.extend_from_slice(&[0x3d, 0xfe, 0xff, 0xff, 0xff]); // cmp eax, -ENOENT
        let visible_branch = code.len();
        code.extend_from_slice(&[0x75, 0]); // jne visible

        code.extend_from_slice(&[0xb8, 0x01, 0x00, 0x00, 0x00]); // mov eax, SYS_write
        code.extend_from_slice(&[0xbf, 0x03, 0x00, 0x00, 0x00]); // mov edi, fd 3
        code.push(0xbe);
        code.extend_from_slice(&((0x400000 + ISOLATED_OFFSET) as u32).to_le_bytes());
        code.extend_from_slice(&[0xba, 0x08, 0x00, 0x00, 0x00, 0x0f, 0x05]); // write 8
        code.extend_from_slice(&[0xb8, 0x00, 0x00, 0x00, 0x00]); // mov eax, SYS_read
        code.extend_from_slice(&[0xbf, 0x03, 0x00, 0x00, 0x00]); // mov edi, fd 3
        code.extend_from_slice(&[0x48, 0x89, 0xe6]); // mov rsi, rsp
        code.extend_from_slice(&[0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05]); // read 1
        code.extend_from_slice(&[0xb8, 0x3c, 0x00, 0x00, 0x00]); // mov eax, SYS_exit
        code.extend_from_slice(&[0x31, 0xff, 0x0f, 0x05]); // exit 0

        let visible_label = code.len();
        code.extend_from_slice(&[0xb8, 0x01, 0x00, 0x00, 0x00]); // mov eax, SYS_write
        code.extend_from_slice(&[0xbf, 0x03, 0x00, 0x00, 0x00]); // mov edi, fd 3
        code.push(0xbe);
        code.extend_from_slice(&((0x400000 + VISIBLE_OFFSET) as u32).to_le_bytes());
        code.extend_from_slice(&[0xba, 0x08, 0x00, 0x00, 0x00, 0x0f, 0x05]); // write 8
        code.extend_from_slice(&[0xb8, 0x00, 0x00, 0x00, 0x00]); // mov eax, SYS_read
        code.extend_from_slice(&[0xbf, 0x03, 0x00, 0x00, 0x00]); // mov edi, fd 3
        code.extend_from_slice(&[0x48, 0x89, 0xe6]); // mov rsi, rsp
        code.extend_from_slice(&[0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05]); // read 1
        code.extend_from_slice(&[0xb8, 0x3c, 0x00, 0x00, 0x00]); // mov eax, SYS_exit
        code.extend_from_slice(&[0xbf, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05]); // exit 1
        code[visible_branch + 1] = (visible_label - (visible_branch + 2)) as u8;
        assert!(120 + code.len() < PATH_OFFSET);

        let mut image = process_lifecycle_tests::synthetic_static_elf().to_vec();
        image.resize(IMAGE_BYTES, 0);
        image[96..104].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());
        image[104..112].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());
        image[120..120 + code.len()].copy_from_slice(&code);
        image[PATH_OFFSET..PATH_OFFSET + b"/nagi-c3-root-secret\0".len()]
            .copy_from_slice(b"/nagi-c3-root-secret\0");
        image[ISOLATED_OFFSET..ISOLATED_OFFSET + 8].copy_from_slice(b"ISOLATED");
        image[VISIBLE_OFFSET..VISIBLE_OFFSET + 8].copy_from_slice(b"VISIBLE!");
        image
    }

    fn app_storage_isolation_probe_elf(
        foreign_profile_path: &str,
        inherited_fds_to_close: [i32; 8],
    ) -> Vec<u8> {
        const IMAGE_BYTES: usize = 2048;
        // Keep the path markers clear of the probe's eight inherited-FD
        // checks and the identity/storage syscalls assembled below.
        const OWN_MARKER_OFFSET: usize = 1024;
        const FOREIGN_MARKER_OFFSET: usize = 1056;
        const FOREIGN_PROFILE_OFFSET: usize = 1088;

        let mut code = Vec::new();
        let mut failure_branches = Vec::new();
        let append_openat = |code: &mut Vec<u8>, path_offset: usize| {
            code.extend_from_slice(&[0xb8, 0x01, 0x01, 0x00, 0x00]); // mov eax, SYS_openat
            code.extend_from_slice(&[0xbf, 0x9c, 0xff, 0xff, 0xff]); // mov edi, AT_FDCWD
            code.push(0xbe); // mov esi, absolute path in this fixed-address ELF
            code.extend_from_slice(&((0x400000 + path_offset) as u32).to_le_bytes());
            code.extend_from_slice(&[0x31, 0xd2, 0x0f, 0x05]); // O_RDONLY; syscall
        };
        let mut append_failure_branch = |code: &mut Vec<u8>, condition: u8| {
            code.extend_from_slice(&[0x0f, condition]);
            failure_branches.push(code.len());
            code.extend_from_slice(&[0; 4]); // patched to the common failure exit
        };

        code.extend_from_slice(&[0xb8, 0x27, 0x00, 0x00, 0x00, 0x0f, 0x05]); // getpid()
        code.extend_from_slice(&[0x83, 0xf8, 0x01]); // child must be PID 1 in its PID namespace
        append_failure_branch(&mut code, 0x85); // jne fail

        for (syscall_number, expected_id) in [
            (102u32, LinuxExecutionIdentity::APP_FILES.uid), // getuid()
            (107u32, LinuxExecutionIdentity::APP_FILES.uid), // geteuid()
            (104u32, LinuxExecutionIdentity::APP_FILES.gid), // getgid()
            (108u32, LinuxExecutionIdentity::APP_FILES.gid), // getegid()
        ] {
            code.push(0xb8); // mov eax, syscall number
            code.extend_from_slice(&syscall_number.to_le_bytes());
            code.extend_from_slice(&[0x0f, 0x05]); // syscall
            code.push(0x3d); // cmp eax, expected app ID
            code.extend_from_slice(&expected_id.to_le_bytes());
            append_failure_branch(&mut code, 0x85); // jne fail
        }

        for inherited_fd in inherited_fds_to_close {
            code.extend_from_slice(&[0xb8, 72, 0x00, 0x00, 0x00]); // mov eax, SYS_fcntl
            code.push(0xbf); // mov edi, inherited descriptor that must be closed
            code.extend_from_slice(&inherited_fd.to_le_bytes());
            code.extend_from_slice(&[0xbe, libc::F_GETFD as u8, 0x00, 0x00, 0x00]); // mov esi, F_GETFD
            code.extend_from_slice(&[0x0f, 0x05]); // syscall
            code.push(0x3d); // cmp eax, -EBADF; the descriptor must be closed
            code.extend_from_slice(&(-libc::EBADF).to_le_bytes());
            append_failure_branch(&mut code, 0x85); // jne fail
        }

        append_openat(&mut code, OWN_MARKER_OFFSET);
        code.extend_from_slice(&[0x85, 0xc0]); // test eax, eax
        append_failure_branch(&mut code, 0x88); // js fail
        code.extend_from_slice(&[0x89, 0xc7]); // mov edi, eax (owned marker fd)
        code.extend_from_slice(&[0xb8, 0x00, 0x00, 0x00, 0x00]); // mov eax, SYS_read
        code.extend_from_slice(&[0x48, 0x89, 0xe6]); // mov rsi, rsp
        code.extend_from_slice(&[0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05]); // read 1
        code.extend_from_slice(&[0x83, 0xf8, 0x01]); // cmp eax, 1
        append_failure_branch(&mut code, 0x85); // jne fail
        code.extend_from_slice(&[0x80, 0x3c, 0x24, b'O']); // cmp byte [rsp], 'O'
        append_failure_branch(&mut code, 0x85); // jne fail

        append_openat(&mut code, FOREIGN_MARKER_OFFSET);
        code.extend_from_slice(&[0x83, 0xf8, 0xfe]); // cmp eax, -ENOENT
        append_failure_branch(&mut code, 0x85); // jne fail

        append_openat(&mut code, FOREIGN_PROFILE_OFFSET);
        code.extend_from_slice(&[0x83, 0xf8, 0xfe]); // cmp eax, -ENOENT
        append_failure_branch(&mut code, 0x85); // jne fail

        code.extend_from_slice(&[0xb8, 0x3c, 0x00, 0x00, 0x00]); // mov eax, SYS_exit
        code.extend_from_slice(&[0x31, 0xff, 0x0f, 0x05]); // exit 0
        let failure_offset = code.len();
        code.extend_from_slice(&[0xb8, 0x3c, 0x00, 0x00, 0x00]); // mov eax, SYS_exit
        code.extend_from_slice(&[0xbf, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05]); // exit 1
        for displacement_offset in failure_branches {
            let next_instruction = displacement_offset + 4;
            let displacement = i32::try_from(failure_offset)
                .unwrap()
                .checked_sub(i32::try_from(next_instruction).unwrap())
                .unwrap();
            code[displacement_offset..next_instruction]
                .copy_from_slice(&displacement.to_le_bytes());
        }

        let foreign_profile_bytes = foreign_profile_path.as_bytes();
        assert!(code.len() < OWN_MARKER_OFFSET - 120);
        assert!(OWN_MARKER_OFFSET + b"/data/own.marker\0".len() <= FOREIGN_MARKER_OFFSET);
        assert!(FOREIGN_MARKER_OFFSET + b"/data/foreign.marker\0".len() <= FOREIGN_PROFILE_OFFSET);
        assert!(FOREIGN_PROFILE_OFFSET + foreign_profile_bytes.len() < IMAGE_BYTES);

        let mut image = process_lifecycle_tests::synthetic_static_elf().to_vec();
        image.resize(IMAGE_BYTES, 0);
        image[96..104].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());
        image[104..112].copy_from_slice(&(IMAGE_BYTES as u64).to_le_bytes());
        image[120..120 + code.len()].copy_from_slice(&code);
        image[OWN_MARKER_OFFSET..OWN_MARKER_OFFSET + b"/data/own.marker\0".len()]
            .copy_from_slice(b"/data/own.marker\0");
        image[FOREIGN_MARKER_OFFSET..FOREIGN_MARKER_OFFSET + b"/data/foreign.marker\0".len()]
            .copy_from_slice(b"/data/foreign.marker\0");
        image[FOREIGN_PROFILE_OFFSET..FOREIGN_PROFILE_OFFSET + foreign_profile_bytes.len()]
            .copy_from_slice(foreign_profile_bytes);
        image[FOREIGN_PROFILE_OFFSET + foreign_profile_bytes.len()] = 0;
        image
    }

    fn untracked_child_exit_elf() -> Vec<u8> {
        let mut image = process_lifecycle_tests::synthetic_static_elf().to_vec();
        image.resize(160, 0);
        let image_len = image.len() as u64;
        image[96..104].copy_from_slice(&image_len.to_le_bytes());
        image[104..112].copy_from_slice(&image_len.to_le_bytes());
        image[120..132].copy_from_slice(&[
            0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, SYS_exit
            0xbf, 0x25, 0x00, 0x00, 0x00, // mov edi, 37
            0x0f, 0x05, // syscall
        ]);
        image
    }

    fn wrong_machine_elf() -> Vec<u8> {
        let mut image = endpoint_writer_elf();
        image[18..20].copy_from_slice(&3u16.to_le_bytes());
        image
    }

    fn launch_with_payload<'a>(
        executable: ExecutablePayload<'a>,
        endpoint_policy: ProcessEndpointPolicy,
    ) -> ProcessLaunch<'a, &'static str> {
        launch_with_service_payload("process-resource-test", executable, endpoint_policy)
    }

    fn launch_with_service_payload<'a>(
        service: &'static str,
        executable: ExecutablePayload<'a>,
        endpoint_policy: ProcessEndpointPolicy,
    ) -> ProcessLaunch<'a, &'static str> {
        ProcessLaunch {
            service,
            executable,
            arguments: &[],
            execution_identity: LinuxExecutionIdentity::DEV_SERVICE,
            endpoint_policy,
            filesystem_policy: ProcessFilesystemPolicy::Inherited,
            required: true,
            release_wait_budget: TEST_RELEASE_BUDGET,
            cancel_check_interval: TEST_CANCEL_INTERVAL,
        }
    }

    fn launch<'a>(
        bytes: &'a [u8],
        endpoint_policy: ProcessEndpointPolicy,
    ) -> ProcessLaunch<'a, &'static str> {
        launch_with_payload(
            ExecutablePayload::from_package_executable(bytes).unwrap(),
            endpoint_policy,
        )
    }

    fn launch_for_service<'a>(
        service: &'static str,
        bytes: &'a [u8],
        endpoint_policy: ProcessEndpointPolicy,
    ) -> ProcessLaunch<'a, &'static str> {
        launch_with_service_payload(
            service,
            ExecutablePayload::from_package_executable(bytes).unwrap(),
            endpoint_policy,
        )
    }

    fn no_revoke(_: u32, _: &u32) -> Result<(), BackendError> {
        Ok(())
    }

    fn wait_for_exit(
        table: &mut ProcessLifecycle<linux_x86_64_process::LinuxProcessOs, &'static str, u32>,
        process_id: u32,
    ) -> LifecycleEvent<&'static str> {
        let deadline = std::time::Instant::now() + TEST_RELEASE_BUDGET;
        loop {
            if let Some(event) = table.poll(no_revoke).unwrap() {
                return event;
            }
            assert!(std::time::Instant::now() < deadline, "child did not exit");
            std::thread::sleep(std::time::Duration::from_millis(1));
            assert_eq!(table.live_record(process_id).copied(), Some(TEST_RECORD));
        }
    }

    fn wait_for_next_event(
        table: &mut ProcessLifecycle<linux_x86_64_process::LinuxProcessOs, &'static str, u32>,
    ) -> LifecycleEvent<&'static str> {
        let deadline = std::time::Instant::now() + TEST_RELEASE_BUDGET;
        loop {
            if let Some(event) = table.poll(no_revoke).unwrap() {
                return event;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "ProcessLifecycle did not observe an exited child"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    struct IdleReaperGuard {
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl IdleReaperGuard {
        fn start() -> Self {
            linux_x86_64_process::initialize_reaper_wait_channel_for_test().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let worker = std::thread::spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    linux_x86_64_process::wait_for_os_event();
                }
            });
            Self {
                stop,
                worker: Some(worker),
            }
        }
    }

    impl Drop for IdleReaperGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    struct UntrackedChildFixture {
        process_id: u32,
        executable_path: std::path::PathBuf,
    }

    fn spawn_untracked_child_fixture() -> UntrackedChildFixture {
        use std::os::unix::fs::PermissionsExt;

        let executable_path = std::path::PathBuf::from(format!(
            "/c3-untracked-child-fixture-{}.elf",
            std::process::id()
        ));
        std::fs::write(&executable_path, untracked_child_exit_elf()).unwrap();
        let mut permissions = std::fs::metadata(&executable_path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable_path, permissions).unwrap();
        let child = Command::new(&executable_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn direct untracked-child ELF fixture");
        let process_id = child.id();
        // The shared reaper owns this direct child's wait status; retaining
        // the std::process::Child handle would introduce a second wait owner.
        std::mem::forget(child);
        UntrackedChildFixture {
            process_id,
            executable_path,
        }
    }

    fn wait_for_untracked_child_reaped(process_id: u32) {
        let deadline = std::time::Instant::now() + TEST_RELEASE_BUDGET;
        loop {
            if linux_x86_64_process::waitid_reports_untracked_child_reaped_for_test(process_id)
                .unwrap()
            {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "shared reaper did not reap the untracked child fixture"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[repr(C)]
    struct PeerCredentials {
        process_id: i32,
        uid: u32,
        gid: u32,
    }

    fn peer_credentials(fd: i32) -> io::Result<PeerCredentials> {
        use core::arch::asm;
        const NR_GETSOCKOPT: usize = 55;
        const SOL_SOCKET: usize = 1;
        const SO_PEERCRED: usize = 17;
        let mut credentials = PeerCredentials {
            process_id: -1,
            uid: u32::MAX,
            gid: u32::MAX,
        };
        let mut length = core::mem::size_of::<PeerCredentials>() as u32;
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") NR_GETSOCKOPT as isize => result,
                in("rdi") fd as usize,
                in("rsi") SOL_SOCKET,
                in("rdx") SO_PEERCRED,
                in("r10") &mut credentials as *mut PeerCredentials as usize,
                in("r8") &mut length as *mut u32 as usize,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
        if length as usize != core::mem::size_of::<PeerCredentials>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SO_PEERCRED returned an unexpected structure size",
            ));
        }
        Ok(credentials)
    }

    fn open_fd_count() -> usize {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .count()
    }

    #[test]
    fn clone3_endpoint_handoff_binds_peer_and_exact_record_once() {
        let image = endpoint_writer_elf();
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let process_id = table
            .spawn(
                launch(&image, ProcessEndpointPolicy::InheritedPeerChannel),
                || false,
                |_| Ok(TEST_RECORD),
                no_revoke,
            )
            .unwrap();

        let wrong_record = table.take_live_endpoint(process_id, TEST_RECORD + 1);
        assert_eq!(
            wrong_record.err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );

        let resources = table.take_live_endpoint(process_id, TEST_RECORD).unwrap();
        assert_eq!(resources.pidfd.process_id(), process_id);
        let transferred_endpoint = resources.endpoint;
        let duplicate_transfer = table.take_live_endpoint(process_id, TEST_RECORD);
        assert_eq!(
            duplicate_transfer.err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );

        let credentials = peer_credentials(transferred_endpoint.as_raw_fd()).unwrap();
        assert_eq!(credentials.process_id as u32, process_id);
        assert_eq!(credentials.uid, LinuxExecutionIdentity::DEV_SERVICE.uid);
        assert_eq!(credentials.gid, LinuxExecutionIdentity::DEV_SERVICE.gid);

        let mut endpoint = std::fs::File::from(transferred_endpoint);
        let mut marker = [0u8; 5];
        endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"HELLO");
        endpoint.write_all(b"R").unwrap();
        endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"READY");
        endpoint.write_all(b"Q").unwrap();
        let mut trailing = [0u8; 1];
        assert_eq!(endpoint.read(&mut trailing).unwrap(), 0);

        let event = wait_for_exit(&mut table, process_id);
        assert!(matches!(
            event,
            LifecycleEvent::ServiceExited {
                service: "process-resource-test",
                status: ServiceExitStatus {
                    exit_code: Some(0),
                    signal: None,
                },
                required: true,
            }
        ));
        assert_eq!(table.active_len(), 0);
        assert_eq!(resources.pidfd.process_id(), process_id);
    }

    #[test]
    fn endpoint_service_child_cannot_open_pid1_root_files_after_namespace_setup() {
        use std::os::unix::fs::OpenOptionsExt;

        let marker_path = "/nagi-c3-root-secret";
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(marker_path)
            .expect("create readable PID-1 root probe");
        marker.write_all(b"PID1 ROOT ONLY").unwrap();
        marker.sync_all().unwrap();
        drop(marker);

        let image = namespace_root_probe_elf();
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let process_id = table
            .spawn(
                launch(&image, ProcessEndpointPolicy::InheritedPeerChannel),
                || false,
                |_| Ok(TEST_RECORD),
                no_revoke,
            )
            .unwrap();
        let resources = table.take_live_endpoint(process_id, TEST_RECORD).unwrap();
        let mut endpoint = std::fs::File::from(resources.endpoint);
        let mut result = [0u8; 8];
        endpoint.read_exact(&mut result).unwrap();
        assert_eq!(&result, b"ISOLATED");
        endpoint.write_all(b"Q").unwrap();

        assert!(matches!(
            wait_for_exit(&mut table, process_id),
            LifecycleEvent::ServiceExited {
                status: ServiceExitStatus {
                    exit_code: Some(0),
                    signal: None,
                },
                ..
            }
        ));
        std::fs::remove_file(marker_path).unwrap();
    }

    #[test]
    fn app_process_tmpfs_root_exposes_only_its_authorized_profile_storage() {
        use nagi_profile_volume_notes_owner::{AppStorageKind, ProfileVolumeAppOwner};
        use std::os::fd::AsFd;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        fn mountinfo_fields(path: &std::path::Path) -> (Vec<String>, Vec<String>) {
            use std::os::fd::AsFd;

            let mountpoint = std::fs::File::open(path).expect("open exact fixture mountpoint");
            let mount_id = rustix::fs::statx(
                mountpoint.as_fd(),
                c"",
                rustix::fs::AtFlags::EMPTY_PATH,
                rustix::fs::StatxFlags::MNT_ID,
            )
            .expect("read exact fixture mount id")
            .stx_mnt_id;
            let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")
                .expect("read mount propagation state");
            let line = mountinfo
                .lines()
                .find(|line| {
                    line.split_whitespace()
                        .next()
                        .and_then(|field| field.parse::<u64>().ok())
                        == Some(mount_id)
                })
                .expect("find exact fixture mountpoint");
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let separator = fields
                .iter()
                .position(|field| *field == "-")
                .expect("mountinfo separator");
            let mount_options = fields[5].split(',').map(str::to_owned).collect::<Vec<_>>();
            let optional_fields = fields[6..separator]
                .iter()
                .map(|field| (*field).to_owned())
                .collect();
            (mount_options, optional_fields)
        }

        fn namespace_identity(path: &std::path::Path) -> io::Result<(u64, u64)> {
            let metadata = std::fs::metadata(path)?;
            Ok((metadata.dev(), metadata.ino()))
        }

        fn optional_namespace_identity(path: &std::path::Path) -> io::Result<Option<(u64, u64)>> {
            match namespace_identity(path) {
                Ok(identity) => Ok(Some(identity)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        struct AppIsolationFixture {
            root: std::path::PathBuf,
            mounted: bool,
        }

        impl Drop for AppIsolationFixture {
            fn drop(&mut self) {
                if self.mounted {
                    if let Ok(path) =
                        std::ffi::CString::new(self.root.as_os_str().as_encoded_bytes())
                    {
                        let _ = unsafe { libc::umount2(path.as_ptr(), libc::MNT_DETACH) };
                    }
                }
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        let root = std::path::PathBuf::from(format!(
            "/tmp/nagi-c3-l11-app-profile-isolation-{}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("create unique app isolation fixture root");
        let mut fixture = AppIsolationFixture {
            root: root.clone(),
            mounted: false,
        };
        let source = std::ffi::CString::new("tmpfs").unwrap();
        let target = std::ffi::CString::new(root.as_os_str().as_encoded_bytes()).unwrap();
        let filesystem = std::ffi::CString::new("tmpfs").unwrap();
        let options = std::ffi::CString::new("size=4m,mode=0755").unwrap();
        let mounted = unsafe {
            libc::mount(
                source.as_ptr(),
                target.as_ptr(),
                filesystem.as_ptr(),
                (libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC) as _,
                options.as_ptr().cast(),
            )
        };
        assert_eq!(
            mounted,
            0,
            "mount isolated tmpfs fixture: {}",
            io::Error::last_os_error()
        );
        fixture.mounted = true;

        // Exercise the exact production tmpfs options and verify the kernel
        // reports both configured resource ceilings.
        let bounded_tmpfs_path = root.join("bounded-tmpfs");
        std::fs::create_dir(&bounded_tmpfs_path).expect("create bounded tmpfs mountpoint");
        let bounded_tmpfs_target =
            std::ffi::CString::new(bounded_tmpfs_path.as_os_str().as_encoded_bytes()).unwrap();
        linux_x86_64_process::mount_service_tmpfs_for_test(&bounded_tmpfs_target)
            .expect("mount bounded service tmpfs options");
        assert!(mountinfo_fields(&bounded_tmpfs_path)
            .0
            .iter()
            .any(|option| option == "noexec"));
        let mut tmpfs_stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        assert_eq!(
            unsafe { libc::statvfs(bounded_tmpfs_target.as_ptr(), tmpfs_stats.as_mut_ptr()) },
            0,
            "read bounded service tmpfs limits: {}",
            io::Error::last_os_error()
        );
        let tmpfs_stats = unsafe { tmpfs_stats.assume_init() };
        let block_size = if tmpfs_stats.f_frsize == 0 {
            tmpfs_stats.f_bsize
        } else {
            tmpfs_stats.f_frsize
        } as u128;
        assert!(
            tmpfs_stats.f_blocks as u128 * block_size
                <= linux_x86_64_process::SERVICE_TMPFS_SIZE_BYTES as u128,
            "service tmpfs must not exceed its explicit byte ceiling"
        );
        assert!(
            tmpfs_stats.f_files <= linux_x86_64_process::SERVICE_TMPFS_MAX_INODES,
            "service tmpfs must not exceed its explicit inode ceiling"
        );
        assert_eq!(
            unsafe { libc::umount2(bounded_tmpfs_target.as_ptr(), libc::MNT_DETACH) },
            0,
            "unmount bounded tmpfs: {}",
            io::Error::last_os_error()
        );
        std::fs::remove_dir(&bounded_tmpfs_path).unwrap();

        // Start from a shared synthetic Data mount. Production first gives
        // the app child a recursively private root before attaching storage;
        // model that destination with a private bind-mounted target while
        // leaving the selected Data source shared.
        assert_eq!(
            unsafe {
                libc::mount(
                    std::ptr::null(),
                    target.as_ptr(),
                    std::ptr::null(),
                    libc::MS_REC | libc::MS_SHARED,
                    std::ptr::null(),
                )
            },
            0,
            "make only the fixture source mount shared: {}",
            io::Error::last_os_error()
        );
        let (root_mount_options, root_mount_optional_fields) = mountinfo_fields(&root);
        assert!(root_mount_options.iter().any(|option| option == "noexec"));
        assert!(root_mount_optional_fields
            .iter()
            .any(|field| field.starts_with("shared:")));
        let propagation_source = root.join("propagation-source");
        let propagation_target = root.join("propagation-target");
        std::fs::create_dir(&propagation_source).unwrap();
        std::fs::create_dir(&propagation_target).unwrap();
        let propagation_source_fd = std::fs::File::open(&propagation_source).unwrap();
        let propagation_target_c =
            std::ffi::CString::new(propagation_target.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(
            unsafe {
                libc::mount(
                    propagation_target_c.as_ptr(),
                    propagation_target_c.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND,
                    std::ptr::null(),
                )
            },
            0,
            "create a private destination mount like the app root: {}",
            io::Error::last_os_error()
        );
        assert_eq!(
            unsafe {
                libc::mount(
                    std::ptr::null(),
                    propagation_target_c.as_ptr(),
                    std::ptr::null(),
                    libc::MS_REC | libc::MS_PRIVATE,
                    std::ptr::null(),
                )
            },
            0,
            "make only the fixture destination mount private: {}",
            io::Error::last_os_error()
        );
        assert!(mountinfo_fields(&propagation_source)
            .1
            .iter()
            .any(|field| field.starts_with("shared:")));
        assert!(mountinfo_fields(&propagation_target)
            .0
            .iter()
            .any(|option| option == "noexec"));
        assert!(
            !mountinfo_fields(&propagation_target).1.iter().any(|field| {
                field.starts_with("shared:")
                    || field.starts_with("master:")
                    || field.starts_with("propagate_from:")
            })
        );
        linux_x86_64_process::attach_private_app_storage_mount_for_test(
            propagation_source_fd.as_fd(),
            &propagation_target_c,
        )
        .expect("attach private clone of shared source mount");
        let (app_storage_mount_options, app_storage_optional_fields) =
            mountinfo_fields(&propagation_target);
        assert!(app_storage_mount_options
            .iter()
            .any(|option| option == "noexec"));
        assert!(!app_storage_optional_fields.iter().any(|field| {
            field.starts_with("shared:")
                || field.starts_with("master:")
                || field.starts_with("propagate_from:")
        }));
        assert_eq!(
            unsafe { libc::umount2(propagation_target_c.as_ptr(), libc::MNT_DETACH) },
            0,
            "unmount private propagation fixture: {}",
            io::Error::last_os_error()
        );
        assert_eq!(
            unsafe { libc::umount2(propagation_target_c.as_ptr(), libc::MNT_DETACH) },
            0,
            "unmount private destination mount: {}",
            io::Error::last_os_error()
        );
        drop(propagation_source_fd);
        std::fs::remove_dir(&propagation_source).unwrap();
        std::fs::remove_dir(&propagation_target).unwrap();

        let data_root = std::fs::File::open(&root).expect("open selected Data root fixture");
        let data_root_fd = data_root.as_raw_fd();
        assert!(data_root_fd > 2);
        let data_root_flags = unsafe { libc::fcntl(data_root_fd, libc::F_GETFD) };
        assert!(data_root_flags >= 0);
        assert_eq!(
            unsafe {
                libc::fcntl(
                    data_root_fd,
                    libc::F_SETFD,
                    data_root_flags & !libc::FD_CLOEXEC,
                )
            },
            0,
            "make the PID-1 Data ancestor descriptor intentionally inheritable"
        );
        let mount_id = rustix::fs::statx(
            &data_root,
            c"",
            rustix::fs::AtFlags::EMPTY_PATH,
            rustix::fs::StatxFlags::MNT_ID,
        )
        .expect("read selected Data mount id")
        .stx_mnt_id;

        // A pre-existing non-empty root without an owner reservation is not
        // migrated to the app UID. Preserve it and fail before creating one.
        let legacy_namespace = root.join("profile-files-v1");
        std::fs::create_dir(&legacy_namespace).expect("create legacy namespace fixture");
        std::fs::set_permissions(&legacy_namespace, std::fs::Permissions::from_mode(0o700))
            .expect("make legacy namespace private");
        let legacy_profile = legacy_namespace.join("profile_legacy");
        std::fs::create_dir(&legacy_profile).expect("create legacy profile fixture");
        std::fs::set_permissions(&legacy_profile, std::fs::Permissions::from_mode(0o700))
            .expect("make legacy profile private");
        let legacy_root = legacy_profile.join("root");
        std::fs::create_dir(&legacy_root).expect("create legacy User Files root");
        std::fs::set_permissions(&legacy_root, std::fs::Permissions::from_mode(0o700))
            .expect("make legacy User Files root private");
        let legacy_marker = legacy_root.join("preserve.marker");
        std::fs::write(&legacy_marker, b"KEEP").expect("write legacy data marker");
        std::fs::set_permissions(&legacy_marker, std::fs::Permissions::from_mode(0o600))
            .expect("make legacy marker owner-only");
        let mut legacy_owner = ProfileVolumeAppOwner::pending_for(AppStorageKind::UserFiles);
        assert!(legacy_owner
            .attach_linux_profile(
                &data_root,
                mount_id,
                nagi_model::UserId(4),
                "profile_legacy",
                "principal_legacy",
            )
            .is_err());
        assert_eq!(std::fs::read(&legacy_marker).unwrap(), b"KEEP");
        assert_eq!(std::fs::metadata(&legacy_root).unwrap().uid(), 0);
        assert_eq!(std::fs::metadata(&legacy_root).unwrap().gid(), 0);
        assert!(!legacy_profile.join("reservation.v1").exists());

        let mut files_owner = ProfileVolumeAppOwner::pending_for(AppStorageKind::UserFiles);
        files_owner
            .attach_linux_profile(
                &data_root,
                mount_id,
                nagi_model::UserId(3),
                "profile_fixture",
                "principal_fixture",
            )
            .expect("provision User Files through the profile-volume owner");
        let files_root = files_owner
            .issue_files_root()
            .expect("issue owner-bound User Files root");
        let files_app = nagi_model::AppId::from_identifier(b"org.nagi.files");
        let own_directory = files_root
            .duplicate_directory_for_app_process(&files_app)
            .expect("duplicate the exact owner-issued Files root for its app process");
        let own_directory_fd = own_directory.as_raw_fd();
        let own_directory_flags = unsafe { libc::fcntl(own_directory_fd, libc::F_GETFD) };
        assert!(own_directory_flags >= 0);
        assert_eq!(
            unsafe {
                libc::fcntl(
                    own_directory_fd,
                    libc::F_SETFD,
                    own_directory_flags & !libc::FD_CLOEXEC,
                )
            },
            0,
            "make owner-issued directory descriptor intentionally inheritable"
        );
        let own_root = root.join("profile-files-v1/profile_fixture/root");
        let foreign_root = root.join("foreign-app-profile");
        std::fs::create_dir(&foreign_root).expect("create foreign app profile root");
        std::fs::set_permissions(&foreign_root, std::fs::Permissions::from_mode(0o700))
            .expect("make foreign profile root private");
        for (path, contents) in [
            (own_root.join("own.marker"), b"OWN".as_slice()),
            (foreign_root.join("foreign.marker"), b"FOREIGN".as_slice()),
        ] {
            std::fs::write(&path, contents).expect("write profile marker");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("make profile marker owner-only");
        }
        let pid1_only_path = root.join("pid1-only-inherited-fd.marker");
        std::fs::write(&pid1_only_path, b"PID1 ONLY").unwrap();
        let pid1_only_file = std::fs::OpenOptions::new()
            .read(true)
            .open(&pid1_only_path)
            .expect("open PID-1-only descriptor sentinel");
        let pid1_only_fd = pid1_only_file.as_raw_fd();
        assert!(pid1_only_fd > 2);
        let pid1_only_flags = unsafe { libc::fcntl(pid1_only_fd, libc::F_GETFD) };
        assert!(pid1_only_flags >= 0);
        assert_eq!(
            unsafe {
                libc::fcntl(
                    pid1_only_fd,
                    libc::F_SETFD,
                    pid1_only_flags & !libc::FD_CLOEXEC,
                )
            },
            0,
            "make sentinel descriptor intentionally inheritable"
        );

        let set_fixture_owner = |path: &std::path::Path, identity: LinuxExecutionIdentity| {
            let descriptor = std::fs::File::open(path).expect("open exact fixture inode");
            rustix::fs::fchown(
                &descriptor,
                Some(rustix::fs::Uid::from_raw(identity.uid)),
                Some(rustix::fs::Gid::from_raw(identity.gid)),
            )
            .expect("assign exact app owner to synthetic profile data");
        };
        set_fixture_owner(
            &own_root.join("own.marker"),
            LinuxExecutionIdentity::APP_FILES,
        );
        set_fixture_owner(&foreign_root, LinuxExecutionIdentity::DEV_NOTES_DESKTOP);
        set_fixture_owner(
            &foreign_root.join("foreign.marker"),
            LinuxExecutionIdentity::DEV_NOTES_DESKTOP,
        );
        let storage = crate::profile_volume_notes_owner::AuthorizedAppStorageRoot::for_test(
            nagi_model::AppId::from_identifier(b"org.nagi.files"),
            own_directory,
        );
        let executable = app_storage_isolation_probe_elf(
            foreign_root.join("foreign.marker").to_str().unwrap(),
            [
                own_directory_fd,
                data_root_fd,
                pid1_only_fd,
                linux_x86_64_process::APP_ISOLATION_TEST_FD_CANDIDATES[0],
                linux_x86_64_process::APP_ISOLATION_TEST_FD_CANDIDATES[1],
                linux_x86_64_process::APP_ISOLATION_TEST_FD_CANDIDATES[2],
                linux_x86_64_process::APP_ISOLATION_TEST_FD_CANDIDATES[3],
                linux_x86_64_process::APP_ISOLATION_TEST_FD_CANDIDATES[4],
            ],
        );
        let parent_pid_namespace = namespace_identity(std::path::Path::new("/proc/self/ns/pid"))
            .expect("read parent PID namespace identity");
        let parent_mount_namespace = namespace_identity(std::path::Path::new("/proc/self/ns/mnt"))
            .expect("read parent mount namespace identity");
        let parent_user_namespace =
            optional_namespace_identity(std::path::Path::new("/proc/self/ns/user"))
                .expect("read parent user namespace identity when supported");
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let process_id = table
            .spawn_authorized(
                ProcessLaunch {
                    service: "app-profile-isolation-test",
                    executable: ExecutablePayload::from_package_executable(&executable).unwrap(),
                    arguments: &[],
                    execution_identity: LinuxExecutionIdentity::APP_FILES,
                    endpoint_policy: ProcessEndpointPolicy::Disabled,
                    filesystem_policy: ProcessFilesystemPolicy::Application(storage),
                    required: true,
                    release_wait_budget: TEST_RELEASE_BUDGET,
                    cancel_check_interval: TEST_CANCEL_INTERVAL,
                },
                || false,
                |_| Ok(TEST_RECORD),
                |child_pid, _record, gate| {
                    // The child remains behind the authorization gate here,
                    // after PID/mount isolation and app credential setup but
                    // before its test ELF can run or exit. Compare namespace
                    // identities without relying on a post-reap /proc race.
                    let child_namespace_root =
                        std::path::PathBuf::from(format!("/proc/{child_pid}/ns"));
                    type NamespaceIdentity = (u64, u64);
                    type NamespaceSnapshot = (
                        NamespaceIdentity,
                        NamespaceIdentity,
                        Option<NamespaceIdentity>,
                    );
                    let namespaces = match (|| -> io::Result<NamespaceSnapshot> {
                        Ok((
                            namespace_identity(&child_namespace_root.join("pid"))?,
                            namespace_identity(&child_namespace_root.join("mnt"))?,
                            optional_namespace_identity(&child_namespace_root.join("user"))?,
                        ))
                    })() {
                        Ok(namespaces) => namespaces,
                        Err(error) => {
                            eprintln!("failed to read child namespaces before exec: {error}");
                            return Err(BackendError::ProtocolViolation);
                        }
                    };
                    if namespaces.0 == parent_pid_namespace
                        || namespaces.1 == parent_mount_namespace
                        || namespaces.2 != parent_user_namespace
                    {
                        eprintln!(
                            "unexpected app namespace identities before exec: child={namespaces:?}"
                        );
                        return Err(BackendError::ProtocolViolation);
                    }
                    gate.commit()
                },
                no_revoke,
            )
            .expect("spawn app behind its per-app filesystem root");

        assert!(matches!(
            wait_for_exit(&mut table, process_id),
            LifecycleEvent::ServiceExited {
                service: "app-profile-isolation-test",
                status: ServiceExitStatus {
                    exit_code: Some(0),
                    signal: None,
                },
                required: true,
            }
        ));
        assert_eq!(table.active_len(), 0);
        for inherited_fd in [data_root_fd, pid1_only_fd] {
            assert!(
                unsafe { libc::fcntl(inherited_fd, libc::F_GETFD) } >= 0,
                "closing child descriptors must leave the PID-1 descriptor table intact"
            );
        }
        drop(pid1_only_file);
        let own_root_metadata = std::fs::metadata(&own_root).unwrap();
        let own_marker_metadata = std::fs::metadata(own_root.join("own.marker")).unwrap();
        assert_eq!(
            own_root_metadata.uid(),
            LinuxExecutionIdentity::APP_FILES.uid
        );
        assert_eq!(
            own_root_metadata.gid(),
            LinuxExecutionIdentity::APP_FILES.gid
        );
        assert_eq!(own_root_metadata.mode() & 0o777, 0o700);
        assert_eq!(
            own_marker_metadata.uid(),
            LinuxExecutionIdentity::APP_FILES.uid
        );
        assert_eq!(
            own_marker_metadata.gid(),
            LinuxExecutionIdentity::APP_FILES.gid
        );
        assert_eq!(own_marker_metadata.mode() & 0o777, 0o600);
        drop((files_root, files_owner, data_root));
        let unmounted = unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) };
        assert_eq!(
            unmounted,
            0,
            "unmount isolated tmpfs fixture: {}",
            io::Error::last_os_error()
        );
        fixture.mounted = false;
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn shared_reaper_races_lifecycle_and_idle_wait_with_mixed_out_of_order_children() {
        let _idle_reaper = IdleReaperGuard::start();
        let first_image = endpoint_writer_elf_with_exit_code(17);
        let second_image = endpoint_writer_elf_with_exit_code(29);
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let first_pid = table
            .spawn(
                launch_for_service(
                    "tracked-first",
                    &first_image,
                    ProcessEndpointPolicy::InheritedPeerChannel,
                ),
                || false,
                |_| Ok(TEST_RECORD),
                no_revoke,
            )
            .unwrap();
        let second_pid = table
            .spawn(
                launch_for_service(
                    "tracked-second",
                    &second_image,
                    ProcessEndpointPolicy::InheritedPeerChannel,
                ),
                || false,
                |_| Ok(TEST_RECORD + 1),
                no_revoke,
            )
            .unwrap();

        let first_resources = table.take_live_endpoint(first_pid, TEST_RECORD).unwrap();
        let second_resources = table
            .take_live_endpoint(second_pid, TEST_RECORD + 1)
            .unwrap();
        let mut first_endpoint = std::fs::File::from(first_resources.endpoint);
        let mut second_endpoint = std::fs::File::from(second_resources.endpoint);
        let mut marker = [0u8; 5];
        first_endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"HELLO");
        second_endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"HELLO");

        let untracked_child = spawn_untracked_child_fixture();
        wait_for_untracked_child_reaped(untracked_child.process_id);
        std::fs::remove_file(untracked_child.executable_path).unwrap();
        assert_eq!(table.live_record(first_pid).copied(), Some(TEST_RECORD));
        assert_eq!(
            table.live_record(second_pid).copied(),
            Some(TEST_RECORD + 1)
        );

        // The second launch exits first while PID-1 idle reaping runs beside
        // ProcessLifecycle::poll. The ticket must receive its own exact status.
        second_endpoint.write_all(b"R").unwrap();
        second_endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"READY");
        second_endpoint.write_all(b"Q").unwrap();
        match wait_for_next_event(&mut table) {
            LifecycleEvent::ServiceExited {
                service: "tracked-second",
                status:
                    ServiceExitStatus {
                        exit_code: Some(29),
                        signal: None,
                    },
                required: true,
            } => {}
            _ => panic!("out-of-order tracked child status was misrouted"),
        }
        assert_eq!(table.live_record(first_pid).copied(), Some(TEST_RECORD));
        assert_eq!(table.active_len(), 1);

        first_endpoint.write_all(b"R").unwrap();
        first_endpoint.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"READY");
        first_endpoint.write_all(b"Q").unwrap();
        match wait_for_next_event(&mut table) {
            LifecycleEvent::ServiceExited {
                service: "tracked-first",
                status:
                    ServiceExitStatus {
                        exit_code: Some(17),
                        signal: None,
                    },
                required: true,
            } => {}
            _ => panic!("first tracked child status was misrouted"),
        }
        assert_eq!(table.active_len(), 0);
    }

    #[test]
    fn disabled_endpoint_policy_does_not_offer_handoff() {
        // With the endpoint disabled there is no peer read to hold the child.
        // Give PID 1 time to observe exec acknowledgement before normal exit.
        let mut image = process_lifecycle_tests::synthetic_static_elf().to_vec();
        image.resize(176, 0);
        let image_len = image.len() as u64;
        image[96..104].copy_from_slice(&image_len.to_le_bytes());
        image[104..112].copy_from_slice(&image_len.to_le_bytes());
        image[120..145].copy_from_slice(&[
            0xb8, 35, 0, 0, 0, // mov eax, SYS_nanosleep
            0x48, 0x8d, 0x3d, 28, 0, 0, 0, // lea rdi, [rip + 28] (timespec)
            0x31, 0xf6, 0x0f, 0x05, // xor esi, esi; syscall
            0xb8, 60, 0, 0, 0, // mov eax, SYS_exit
            0x31, 0xff, 0x0f, 0x05, // xor edi, edi; syscall
        ]);
        image[160..168].copy_from_slice(&1i64.to_le_bytes());
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let process_id = table
            .spawn(
                launch(&image, ProcessEndpointPolicy::Disabled),
                || false,
                |_| Ok(TEST_RECORD),
                no_revoke,
            )
            .unwrap();
        let handoff = table.take_live_endpoint(process_id, TEST_RECORD);
        assert_eq!(
            handoff.err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        let event = wait_for_exit(&mut table, process_id);
        assert!(matches!(
            event,
            LifecycleEvent::ServiceExited {
                status: ServiceExitStatus {
                    exit_code: Some(0),
                    signal: None,
                },
                ..
            }
        ));
    }

    #[test]
    fn exec_failure_reaps_child_and_closes_unhanded_pidfd_and_endpoint() {
        let _idle_reaper = IdleReaperGuard::start();
        let untracked_child = spawn_untracked_child_fixture();
        wait_for_untracked_child_reaped(untracked_child.process_id);
        std::fs::remove_file(untracked_child.executable_path).unwrap();
        let image = wrong_machine_elf();
        assert!(matches!(
            ExecutablePayload::from_package_executable(&image),
            Err(ExecutableFormatError::WrongArchitecture)
        ));
        // Bypass only the format constructor in this test so the real child
        // reaches execveat and exercises its error-report/cleanup path. Normal
        // launch construction always validates the package executable first.
        let unchecked_payload = ExecutablePayload { bytes: &image };
        let before = open_fd_count();
        let mut table = ProcessLifecycle::new(linux_x86_64_process::LinuxProcessOs);
        let registered_pid = std::cell::Cell::new(None);
        let mut revoked = 0usize;
        let mut revoke = |process_id, record: &u32| {
            assert_eq!(Some(process_id), registered_pid.get());
            assert_eq!(*record, TEST_RECORD);
            revoked += 1;
            Ok(())
        };
        let result = table.spawn(
            launch_with_payload(
                unchecked_payload,
                ProcessEndpointPolicy::InheritedPeerChannel,
            ),
            || false,
            |process_id| {
                registered_pid.set(Some(process_id));
                Ok(TEST_RECORD)
            },
            &mut revoke,
        );
        assert_eq!(result, Err(BackendError::TransportFailure));
        let pid = registered_pid
            .get()
            .expect("exec failure must follow registration");
        assert!(table.children.is_empty());
        assert!(table.live_record(pid).is_none());
        // A child can report exec failure before its exit is observable.
        // Drive the real reaper to completion rather than requiring that
        // asynchronous child exit has already happened when spawn returns.
        let deadline = std::time::Instant::now() + TEST_RELEASE_BUDGET;
        loop {
            match table.poll(&mut revoke) {
                Ok(None) => break,
                Err(BackendError::TransportFailure) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "child cleanup did not settle"
                    );
                    assert!(table.live_record(pid).is_none());
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                other => panic!("unexpected exec failure cleanup result: {other:?}"),
            }
        }
        assert!(table.quarantine.is_empty());
        assert_eq!(revoked, 1);
        assert_eq!(table.active_len(), 0);
        assert_eq!(open_fd_count(), before);
    }
}

#[cfg(all(test, feature = "linux-service-action-bridge"))]
mod service_action_bridge_tests {
    use super::*;

    #[test]
    fn missing_runtime_route_fails_closed_before_action_dispatch() {
        let mut backend = UnconnectedBackend::new();
        let mut session = ();
        let mut service = None;
        let request = ServiceRequest {
            request_id: 1,
            action: "preview-only request".to_owned(),
        };

        assert!(matches!(
            backend.call_service(
                &mut session,
                &mut service,
                request,
                CancellationToken::new(),
                Duration::from_secs(1),
            ),
            Err(ServiceIoError::Unavailable)
        ));
    }

    #[test]
    fn invalid_deadline_is_unavailable_not_a_transport_failure() {
        assert!(matches!(
            service_call_error_for_action(1, ActionCallError::Call(CallError::InvalidTimeout)),
            Err(ServiceIoError::Unavailable)
        ));
    }

    #[test]
    fn stale_route_registration_does_not_claim_grant_revocation() {
        assert!(matches!(
            service_response_for_contract_error(1, IpcErrorCode::StaleRegistration),
            Ok(ServiceResponse::Rejected {
                request_id: 1,
                reason: ServiceFailureCode::Failed,
            })
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_and_development_notes_use_distinct_process_identities() {
        let product_notes = nagi_model::AppId::from_identifier(b"org.nagi.notes");
        let development_notes = nagi_model::AppId::from_identifier(b"com.nagi.notes");
        assert_eq!(
            LinuxExecutionIdentity::for_application(product_notes),
            Some(LinuxExecutionIdentity::APP_NOTES)
        );
        assert_eq!(
            LinuxExecutionIdentity::for_application(development_notes),
            Some(LinuxExecutionIdentity::DEV_NOTES_DESKTOP)
        );
        assert_ne!(
            LinuxExecutionIdentity::APP_NOTES,
            LinuxExecutionIdentity::DEV_NOTES_DESKTOP
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn normal_pid1_notes_startup_returns_unavailable_without_verified_attachment() {
        let owner = crate::profile_volume_notes_owner::pending();
        assert!(matches!(
            normal_startup_profile_notes_bundle(&owner),
            Err(crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable)
        ));
    }
    #[cfg_attr(not(test), allow(unused_imports))]
    use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
    #[cfg_attr(not(test), allow(unused_imports))]
    use nagi_package::PackageService;
    #[cfg_attr(not(test), allow(unused_imports))]
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[cfg(linux_dev_preview_smoke)]
    #[test]
    fn development_input_envelope_requires_private_path_and_explicit_mode() {
        let valid = b"mode=development\nroute=owner-supplied\n";
        assert_eq!(
            validate_development_input_envelope("/run/nagi/launch.input", 0o600, valid),
            Ok(())
        );

        for (path, input) in [
            ("/tmp/launch.input", valid.as_slice()),
            ("/run/nagi/../launch.input", valid.as_slice()),
            (
                "/run/nagi/launch.input",
                b"route=owner-supplied\n".as_slice(),
            ),
            ("/run/nagi/launch.input", b"mode=production\n".as_slice()),
            (
                "/run/nagi/launch.input",
                b"mode=development\nmode=development\n".as_slice(),
            ),
            (
                "/run/nagi/launch.input",
                b"mode=development\n\nroute=owner-supplied\n".as_slice(),
            ),
            ("/run/nagi/launch.input", b"mode=development\r\n".as_slice()),
            (
                "/run/nagi/launch.input",
                b"mode=development\n\xff".as_slice(),
            ),
        ] {
            assert_eq!(
                validate_development_input_envelope(path, 0o600, input),
                Err(BackendError::ProtocolViolation),
                "rejected {path} with {} input bytes",
                input.len()
            );
        }
        assert_eq!(
            validate_development_input_envelope("/run/nagi/launch.input", 0o644, valid),
            Err(BackendError::ProtocolViolation)
        );
    }

    #[cfg(linux_dev_preview_smoke)]
    #[test]
    fn development_action_input_requires_exact_route_and_nonzero_secrets() {
        let valid = b"format=nagi-dev-service-input-v1\nmode=development\nservice_id=dev.nagi.action\nservice_version=1.0\noperation_id=execute\npayload_type=nagi.action@1\nservice_app_id=org.nagi.dev.action-service\npeer_app_id=org.nagi.dev.action-client\nlaunch_id=11111111111111111111111111111111\ntoken=2222222222222222222222222222222222222222222222222222222222222222\nsocket=/run/nagi/dev-action.sock\nindex_root=/run/nagi/dev-data\nintent=find the live VFS fixture\nquery=nagi-m19-live-file.txt\n";
        assert_eq!(
            validate_development_action_input("/run/nagi/dev-service.runtime", 0o600, valid),
            Ok(())
        );

        let valid_text = std::str::from_utf8(valid).unwrap();
        let mut zero_token = valid.to_vec();
        let token_start = zero_token
            .windows(b"token=".len())
            .position(|window| window == b"token=")
            .unwrap()
            + b"token=".len();
        zero_token[token_start..token_start + 64].fill(b'0');
        for invalid in [
            valid_text
                .replace("mode=development", "mode=production")
                .into_bytes(),
            valid_text
                .replace("service_id=dev.nagi.action", "service_id=dev.nagi.other")
                .into_bytes(),
            zero_token,
            [valid.as_slice(), b"extra=value\n"].concat(),
        ] {
            assert_eq!(
                validate_development_action_input("/run/nagi/dev-service.runtime", 0o600, &invalid),
                Err(BackendError::ProtocolViolation)
            );
        }
    }

    #[cfg(linux_desktop_dev_smoke)]
    #[test]
    fn notes_desktop_input_accepts_only_development_save_or_read_phase() {
        assert_eq!(
            validate_notes_desktop_input(0o600, b"mode=development\nphase=save\n"),
            Ok("save")
        );
        assert_eq!(
            validate_notes_desktop_input(0o600, b"mode=development\nphase=read\n"),
            Ok("read")
        );
        for (mode, bytes) in [
            (0o644, b"mode=development\nphase=save\n".as_slice()),
            (0o600, b"mode=production\nphase=save\n".as_slice()),
            (0o600, b"mode=development\nphase=other\n".as_slice()),
            (
                0o600,
                b"mode=development\nphase=save\nextra=value\n".as_slice(),
            ),
            (
                0o600,
                b"mode=development\nphase=save\nphase=read\n".as_slice(),
            ),
        ] {
            assert_eq!(
                validate_notes_desktop_input(mode, bytes),
                Err(BackendError::ProtocolViolation)
            );
        }
    }

    #[cfg(any(
        all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"),
        all(
            test,
            linux_dev_preview_smoke,
            target_os = "macos",
            target_arch = "x86_64"
        )
    ))]
    #[test]
    fn wrong_expected_app_id_skips_dev_package_install_inspect_provider_and_spawn() {
        use std::cell::Cell;

        let path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the signed Linux package input");
        let package_bytes = std::fs::read(path).expect("read signed Linux package input");
        let actual_app_id = PackageView::parse(&package_bytes)
            .expect("parse signed Linux package input")
            .manifest()
            .app_id();
        let wrong_expected_app_id = AppId(actual_app_id.0.wrapping_add(1));

        let installs = Cell::new(0);
        let inspections = Cell::new(0);
        let provider_constructions = Cell::new(0);
        let spawns = Cell::new(0);
        let result = LinuxDevPreviewBackend::with_expected_package(
            &package_bytes,
            wrong_expected_app_id,
            |_package| {
                installs.set(installs.get() + 1);
                inspections.set(inspections.get() + 1);
                provider_constructions.set(provider_constructions.get() + 1);
                spawns.set(spawns.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result, Err(BackendError::IdentityRejected));
        assert_eq!(installs.get(), 0);
        assert_eq!(inspections.get(), 0);
        assert_eq!(provider_constructions.get(), 0);
        assert_eq!(spawns.get(), 0);
    }

    struct FakeBackend {
        initialize_result: Result<(), BackendError>,
        events: VecDeque<LifecycleEvent<&'static str>>,
        shutdown_result: Result<(), BackendError>,
        shutdown_causes: Vec<ShutdownCause>,
        wait_calls: usize,
    }

    impl FakeBackend {
        fn new(
            initialize_result: Result<(), BackendError>,
            events: Vec<LifecycleEvent<&'static str>>,
            shutdown_result: Result<(), BackendError>,
        ) -> Self {
            Self {
                initialize_result,
                events: events.into(),
                shutdown_result,
                shutdown_causes: Vec::new(),
                wait_calls: 0,
            }
        }
    }

    impl InitBackend for FakeBackend {
        type Action = ();
        type Reply = ();
        type Session = ();
        type ServiceHandle = ();
        type ServiceId = &'static str;

        fn initialize(&mut self) -> Result<Self::Session, BackendError> {
            self.initialize_result
        }

        fn call_service(
            &mut self,
            _session: &mut Self::Session,
            _service: &mut Self::ServiceHandle,
            request: ServiceRequest<Self::Action>,
            #[cfg(feature = "linux-service-action-bridge")] _cancellation: CancellationToken,
            #[cfg(feature = "linux-service-action-bridge")] _timeout: Duration,
        ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError> {
            Ok(ServiceResponse::Completed {
                request_id: request.request_id,
                reply: (),
            })
        }

        fn wait_event(
            &mut self,
            _session: &mut Self::Session,
        ) -> Result<LifecycleEvent<Self::ServiceId>, BackendError> {
            self.wait_calls += 1;
            self.events.pop_front().ok_or(BackendError::Unavailable)
        }

        fn shutdown(
            &mut self,
            _session: &mut Self::Session,
            cause: ShutdownCause,
        ) -> Result<(), BackendError> {
            self.shutdown_causes.push(cause);
            self.shutdown_result
        }
    }

    fn failed_service(status: ServiceExitStatus) -> LifecycleEvent<&'static str> {
        LifecycleEvent::ServiceExited {
            service: "required.files",
            status,
            required: true,
        }
    }

    #[test]
    fn initialization_rejection_fails_closed_without_wait_or_shutdown() {
        let mut backend = FakeBackend::new(Err(BackendError::IdentityRejected), Vec::new(), Ok(()));
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::from(EX_NOPERM));
        assert_eq!(backend.wait_calls, 0);
        assert!(backend.shutdown_causes.is_empty());
    }

    #[cfg(unix)]
    struct PackageStoreTestDirectory(std::path::PathBuf);

    #[cfg(unix)]
    impl PackageStoreTestDirectory {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            use std::sync::atomic::{AtomicU64, Ordering};

            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "nagi-pid1-package-store-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create isolated package-store directory");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("set private package-store directory mode");
            Self(path)
        }

        fn open(&self) -> std::fs::File {
            std::fs::File::open(&self.0).expect("open package-store directory descriptor")
        }

        fn owner_uid(&self) -> u32 {
            use std::os::unix::fs::MetadataExt;

            std::fs::metadata(&self.0)
                .expect("read package-store directory metadata")
                .uid()
        }

        fn set_mode(&self, mode: u32) {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(mode))
                .expect("set package-store directory mode");
        }
    }

    #[cfg(unix)]
    impl Drop for PackageStoreTestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn assert_package_store_initialization_unavailable(input: Option<PackageStoreDirectoryInput>) {
        let mut backend = UnconnectedBackend::with_package_store_directory(input);
        assert_eq!(run_lifecycle(&mut backend), ExitCode::from(EX_UNAVAILABLE));
        assert_eq!(
            backend.wait_calls, 0,
            "no lifecycle event, including Ready, is read"
        );
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_rejects_a_missing_descriptor_without_ready() {
        assert_package_store_initialization_unavailable(None);
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_rejects_a_non_directory_descriptor_without_ready() {
        let directory = PackageStoreTestDirectory::new();
        let file_path = directory.0.join("regular-file");
        std::fs::write(&file_path, b"not a directory").expect("write regular-file fixture");
        let input = PackageStoreDirectoryInput::new(
            std::fs::File::open(file_path).expect("open regular-file descriptor"),
            directory.owner_uid(),
        );

        assert_package_store_initialization_unavailable(Some(input));
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_rejects_an_unexpected_owner_without_ready() {
        let directory = PackageStoreTestDirectory::new();
        let owner_uid = directory.owner_uid();
        let unexpected_uid = if owner_uid == 0 { 1 } else { 0 };
        let input = PackageStoreDirectoryInput::new(directory.open(), unexpected_uid);

        assert_package_store_initialization_unavailable(Some(input));
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_rejects_group_writable_directory_without_ready() {
        let directory = PackageStoreTestDirectory::new();
        directory.set_mode(0o720);
        let input = PackageStoreDirectoryInput::new(directory.open(), directory.owner_uid());

        assert_package_store_initialization_unavailable(Some(input));
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_rejects_other_writable_directory_without_ready() {
        let directory = PackageStoreTestDirectory::new();
        directory.set_mode(0o702);
        let input = PackageStoreDirectoryInput::new(directory.open(), directory.owner_uid());

        assert_package_store_initialization_unavailable(Some(input));
    }

    #[cfg(unix)]
    #[test]
    fn package_store_initialization_stays_unavailable_without_product_trust_or_catalog() {
        let directory = PackageStoreTestDirectory::new();
        let input = PackageStoreDirectoryInput::new(directory.open(), directory.owner_uid());

        assert_package_store_initialization_unavailable(Some(input));
    }

    #[test]
    fn pid1_non_unavailable_failure_enters_fail_closed_wait() {
        let mut backend = FakeBackend::new(Err(BackendError::IdentityRejected), Vec::new(), Ok(()));
        let wait_calls = std::cell::Cell::new(0);
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pid1_lifecycle(&mut backend, || {
                wait_calls.set(wait_calls.get() + 1);
                panic!("test stops after observing the first blocking-wait request");
            })
        }));

        assert!(held.is_err());
        assert_eq!(wait_calls.get(), 1);
    }

    #[test]
    fn pid1_required_service_exit_enters_fail_closed_wait_after_cleanup() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![failed_service(ServiceExitStatus {
                exit_code: Some(37),
                signal: None,
            })],
            Ok(()),
        );
        let wait_calls = std::cell::Cell::new(0);
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pid1_lifecycle(&mut backend, || {
                wait_calls.set(wait_calls.get() + 1);
                panic!("test stops after observing the first blocking-wait request");
            })
        }));

        assert!(held.is_err());
        assert_eq!(wait_calls.get(), 1);
        assert_eq!(
            backend.shutdown_causes,
            [ShutdownCause::RequiredServiceExited]
        );
    }

    #[test]
    fn pid1_unavailable_startup_stays_alive_without_ready() {
        let mut backend = UnconnectedBackend::new();
        let wait_calls = std::cell::Cell::new(0);
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pid1_lifecycle(&mut backend, || {
                wait_calls.set(wait_calls.get() + 1);
                panic!("test stops after observing the first blocking-wait request");
            })
        }));

        assert!(held.is_err());
        assert_eq!(wait_calls.get(), 1);
        assert_eq!(backend.wait_calls, 0);
    }

    #[test]
    fn duplicate_ready_is_a_protocol_failure_and_cleans_up() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![LifecycleEvent::Ready, LifecycleEvent::Ready],
            Ok(()),
        );
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::from(EX_PROTOCOL));
        assert_eq!(backend.shutdown_causes, [ShutdownCause::ProtocolViolation]);
    }

    #[test]
    fn ready_partial_write_failure_stays_unpublished_and_cleans_up() {
        struct BrokenReadyWriter(Vec<u8>);

        impl std::io::Write for BrokenReadyWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if !self.0.is_empty() {
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                let count = bytes.len().min(6);
                self.0.extend_from_slice(&bytes[..count]);
                Ok(count)
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut backend = FakeBackend::new(Ok(()), vec![LifecycleEvent::Ready], Ok(()));
        let mut writer = BrokenReadyWriter(Vec::new());
        let status = run_lifecycle_with_ready_publisher(&mut backend, |ready| {
            let result = publish_lifecycle_ready_to(ready, || Ok(&mut writer));
            assert!(!*ready);
            result
        });

        assert_eq!(status, ExitCode::from(EX_IOERR));
        assert_eq!(writer.0, b"nagi-i");
        assert_eq!(backend.shutdown_causes, [ShutdownCause::BackendFailure]);
    }

    #[test]
    fn ready_flush_failure_stays_unpublished() {
        struct BufferWithFailedFlush(Vec<u8>);

        impl std::io::Write for BufferWithFailedFlush {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }

        let mut ready = false;
        let mut writer = BufferWithFailedFlush(Vec::new());
        assert_eq!(
            publish_lifecycle_ready_to(&mut ready, || Ok(&mut writer)),
            Err(BackendError::TransportFailure)
        );
        assert!(!ready);
        assert_eq!(writer.0, b"nagi-init: state=ready\n");
    }

    #[test]
    fn ready_writer_open_failure_stays_unpublished() {
        let mut ready = false;
        let result = publish_lifecycle_ready_to(&mut ready, || {
            Err::<std::io::Sink, _>(std::io::ErrorKind::PermissionDenied.into())
        });

        assert_eq!(result, Err(BackendError::TransportFailure));
        assert!(!ready);
    }

    #[test]
    fn ready_fd_reports_os_write_failure() {
        use std::os::fd::AsFd as _;

        let read_only = std::fs::File::open("/dev/null").unwrap();
        let mut ready = false;

        assert_eq!(
            publish_lifecycle_ready_fd(&mut ready, read_only.as_fd()),
            Err(BackendError::TransportFailure)
        );
        assert!(!ready);
    }

    #[test]
    fn ready_fd_delivers_one_complete_notification() {
        use std::io::Read as _;
        use std::os::fd::AsFd as _;
        use std::os::unix::net::UnixStream;

        let (tx, mut rx) = UnixStream::pair().unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let mut ready = false;

        assert_eq!(publish_lifecycle_ready_fd(&mut ready, tx.as_fd()), Ok(()));
        assert!(ready);
        assert_eq!(
            publish_lifecycle_ready_fd(&mut ready, tx.as_fd()),
            Err(BackendError::ProtocolViolation)
        );

        drop(tx);
        let mut received = Vec::new();
        rx.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"nagi-init: state=ready\n");
    }

    #[test]
    fn ready_notification_is_written_once() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![LifecycleEvent::Ready, LifecycleEvent::Ready],
            Ok(()),
        );
        let mut writer = Vec::new();
        let status = run_lifecycle_with_ready_publisher(&mut backend, |ready| {
            publish_lifecycle_ready_to(ready, || Ok(&mut writer))
        });

        assert_eq!(status, ExitCode::from(EX_PROTOCOL));
        assert_eq!(writer, b"nagi-init: state=ready\n");
        assert_eq!(backend.shutdown_causes, [ShutdownCause::ProtocolViolation]);
    }

    #[test]
    fn notes_exit_reason_controls_normal_login_reconciliation() {
        use NotesConsoleExit::{ClosedByUser, ClosedWithoutSave, RequiredIdentityExited};
        assert!(NotesConsoleExit::allows_normal_login_reconciliation(
            Some(ClosedWithoutSave),
            true
        ));
        assert!(!NotesConsoleExit::allows_normal_login_reconciliation(
            Some(ClosedWithoutSave),
            false
        ));

        assert!(NotesConsoleExit::allows_normal_login_reconciliation(
            Some(ClosedByUser),
            true,
        ));
        assert!(!NotesConsoleExit::allows_normal_login_reconciliation(
            Some(RequiredIdentityExited),
            true,
        ));
        assert!(!NotesConsoleExit::allows_normal_login_reconciliation(
            Some(ClosedByUser),
            false,
        ));
        assert!(NotesConsoleExit::allows_normal_login_reconciliation(
            None, true,
        ));
    }

    #[test]
    fn notes_unknown_exit_blocks_normal_login_reconciliation() {
        assert!(!NotesConsoleExit::allows_normal_login_reconciliation(
            Some(NotesConsoleExit::StateUnknown),
            true,
        ));
    }

    #[test]
    fn notes_input_wait_polls_required_service_while_idle_and_on_input() {
        let mut input_polls = 0;
        let mut service_polls = 0;
        let input = wait_for_task24_input(
            |timeout| {
                assert_eq!(timeout, std::time::Duration::from_millis(100));
                input_polls += 1;
                Ok((input_polls == 4).then_some(42))
            },
            || {
                service_polls += 1;
                Ok(false)
            },
        )
        .unwrap();

        assert_eq!(input, Some(42));
        assert_eq!(input_polls, 4);
        assert_eq!(service_polls, 4);
    }

    #[test]
    fn notes_input_wait_prioritizes_required_exit_over_input() {
        let mut input_polls = 0;
        let mut service_polls = 0;
        let input = wait_for_task24_input(
            |_| {
                input_polls += 1;
                Ok((input_polls == 3).then_some(42))
            },
            || {
                service_polls += 1;
                Ok(service_polls == 3)
            },
        )
        .unwrap();

        assert_eq!(input, None);
        assert_eq!(input_polls, 3);
        assert_eq!(service_polls, 3);
    }

    #[test]
    fn notes_input_wait_propagates_required_service_poll_error() {
        let error = wait_for_task24_input(
            |_| Ok(Some(42)),
            || Err(std::io::ErrorKind::BrokenPipe.into()),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn task24_prelogin_monitor_has_priority_over_escape_enter_and_keyboard_error() {
        use std::io::ErrorKind;

        for key in [libnagi::INPUT_KEY_ESCAPE, libnagi::INPUT_KEY_ENTER] {
            assert!(matches!(
                poll_task24_input(|| Ok(Some(key)), || Ok(true)),
                Ok(Task24InputPoll::RequiredServiceExited)
            ));
            assert!(matches!(
                poll_task24_input(|| Ok(Some(key)), || Err(ErrorKind::BrokenPipe.into())),
                Err(error) if error.kind() == ErrorKind::BrokenPipe
            ));
            assert!(matches!(
                poll_task24_input(|| Ok(Some(key)), || Ok(false)),
                Ok(Task24InputPoll::Input(received)) if received == key
            ));
        }
        assert!(matches!(
            poll_task24_input::<()>(
                || Err(ErrorKind::InvalidData.into()),
                || Err(ErrorKind::BrokenPipe.into())
            ),
            Err(error) if error.kind() == ErrorKind::BrokenPipe
        ));
    }

    #[test]
    fn required_service_exit_is_fatal_and_cleans_up() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![
                LifecycleEvent::Ready,
                failed_service(ServiceExitStatus {
                    exit_code: None,
                    signal: Some(9),
                }),
            ],
            Ok(()),
        );
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::from(EX_SOFTWARE));
        assert_eq!(
            backend.shutdown_causes,
            [ShutdownCause::RequiredServiceExited]
        );
    }

    #[test]
    fn cancelled_initialization_returns_temporary_failure_without_ready() {
        let mut backend = FakeBackend::new(Err(BackendError::Cancelled), Vec::new(), Ok(()));
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::from(EX_TEMPFAIL));
        assert_eq!(backend.wait_calls, 0);
        assert!(backend.shutdown_causes.is_empty());
    }

    #[test]
    fn requested_termination_cleans_up_before_success() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![
                LifecycleEvent::Ready,
                LifecycleEvent::ShutdownRequested {
                    signal: ShutdownSignal::Terminate,
                },
            ],
            Ok(()),
        );
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(backend.shutdown_causes, [ShutdownCause::Requested]);
    }

    #[test]
    fn cleanup_failure_changes_failure_to_software_error() {
        let mut backend = FakeBackend::new(
            Ok(()),
            vec![
                LifecycleEvent::Ready,
                failed_service(ServiceExitStatus {
                    exit_code: Some(7),
                    signal: None,
                }),
            ],
            Err(BackendError::TransportFailure),
        );
        let status = run_lifecycle(&mut backend);

        assert_eq!(status, ExitCode::from(EX_SOFTWARE));
        assert_eq!(
            backend.shutdown_causes,
            [ShutdownCause::RequiredServiceExited]
        );
    }

    #[derive(Clone, Copy)]
    enum SessionOwnerLifecycleMode {
        Normal,
        BackendError,
        Panic,
    }

    struct SessionOwnerLifecycleSession {
        package_owner: Option<PackageServiceSessionOwner<'static>>,
        drop_count: Rc<Cell<usize>>,
    }

    impl Drop for SessionOwnerLifecycleSession {
        fn drop(&mut self) {
            {
                let _package_owner = self.package_owner.take();
            }
            self.drop_count.set(self.drop_count.get() + 1);
        }
    }

    struct SessionOwnerLifecycleBackend {
        mode: SessionOwnerLifecycleMode,
        wait_calls: usize,
        shutdown_calls: usize,
        session_drop_count: Rc<Cell<usize>>,
    }

    impl SessionOwnerLifecycleBackend {
        fn new(mode: SessionOwnerLifecycleMode) -> Self {
            Self {
                mode,
                wait_calls: 0,
                shutdown_calls: 0,
                session_drop_count: Rc::new(Cell::new(0)),
            }
        }
    }

    impl InitBackend for SessionOwnerLifecycleBackend {
        type Action = Infallible;
        type Reply = Infallible;
        type Session = SessionOwnerLifecycleSession;
        type ServiceHandle = ();
        type ServiceId = &'static str;

        fn initialize(&mut self) -> Result<Self::Session, BackendError> {
            Ok(SessionOwnerLifecycleSession {
                package_owner: Some(PackageServiceSessionOwner::new(None, PackageService::new())),
                drop_count: Rc::clone(&self.session_drop_count),
            })
        }

        fn call_service(
            &mut self,
            _session: &mut Self::Session,
            _service: &mut Self::ServiceHandle,
            _request: ServiceRequest<Self::Action>,
            #[cfg(feature = "linux-service-action-bridge")] _cancellation: CancellationToken,
            #[cfg(feature = "linux-service-action-bridge")] _timeout: Duration,
        ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError> {
            Err(ServiceIoError::Unavailable)
        }

        fn wait_event(
            &mut self,
            session: &mut Self::Session,
        ) -> Result<LifecycleEvent<Self::ServiceId>, BackendError> {
            assert!(session.package_owner.is_some());
            self.wait_calls += 1;
            match self.mode {
                SessionOwnerLifecycleMode::Normal if self.wait_calls == 1 => {
                    Ok(LifecycleEvent::Ready)
                }
                SessionOwnerLifecycleMode::Normal => Ok(LifecycleEvent::ShutdownRequested {
                    signal: ShutdownSignal::Terminate,
                }),
                SessionOwnerLifecycleMode::BackendError => Err(BackendError::TransportFailure),
                SessionOwnerLifecycleMode::Panic => {
                    panic!("injected lifecycle wait panic with package session owner live")
                }
            }
        }

        fn shutdown(
            &mut self,
            session: &mut Self::Session,
            _cause: ShutdownCause,
        ) -> Result<(), BackendError> {
            assert!(session.package_owner.is_some());
            self.shutdown_calls += 1;
            Ok(())
        }
    }

    #[test]
    fn lifecycle_drops_package_owner_on_normal_error_and_panic_paths() {
        let mut normal = SessionOwnerLifecycleBackend::new(SessionOwnerLifecycleMode::Normal);
        let status = run_lifecycle(&mut normal);
        assert_eq!(status, ExitCode::SUCCESS);
        assert_eq!(normal.shutdown_calls, 1);
        assert_eq!(normal.session_drop_count.get(), 1);

        let mut failed = SessionOwnerLifecycleBackend::new(SessionOwnerLifecycleMode::BackendError);
        let status = run_lifecycle(&mut failed);
        assert_eq!(status, ExitCode::from(EX_IOERR));
        assert_eq!(failed.shutdown_calls, 1);
        assert_eq!(failed.session_drop_count.get(), 1);

        let mut panicking = SessionOwnerLifecycleBackend::new(SessionOwnerLifecycleMode::Panic);
        let session_drop_count = Rc::clone(&panicking.session_drop_count);
        let wait_calls = Cell::new(0);
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pid1_lifecycle(&mut panicking, || {
                wait_calls.set(wait_calls.get() + 1);
                panic!("test stops after PID 1 enters its fail-closed wait");
            })
        }));

        assert!(held.is_err());
        assert_eq!(panicking.wait_calls, 1);
        assert_eq!(panicking.shutdown_calls, 0);
        assert_eq!(session_drop_count.get(), 1);
        assert_eq!(wait_calls.get(), 1);
    }
}
