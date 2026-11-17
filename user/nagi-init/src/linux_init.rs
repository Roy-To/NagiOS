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
#[cfg(all(target_arch = "x86_64", feature = "m10-desktop"))]
#[path = "linux_package_identity.rs"]
mod linux_package_identity;

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

pub(crate) fn run() -> ExitCode {
    if std::process::id() != 1 {
        pid1_log!("nagi-init: refusing Linux init entry outside PID 1");
        return ExitCode::from(EX_USAGE);
    }

    #[cfg(all(linux_pid1_safety_smoke, target_os = "linux", target_arch = "x86_64"))]
    return linux_x86_64_process::run_pid1_safety_smoke();

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

    #[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
    let mut backend = LinuxDevPreviewBackend::new();
    #[cfg(not(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64")))]
    let mut backend = UnconnectedBackend::new();
    run_pid1_lifecycle(&mut backend, wait_for_pid1_os_event)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn wait_for_pid1_os_event() {
    linux_x86_64_process::wait_for_os_event();
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn wait_for_pid1_os_event() {
    std::thread::park_timeout(std::time::Duration::from_secs(3600));
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

/// Lifecycle core after the production PID 1 guard. Unit tests exercise only
/// this orchestration contract with a fake backend; they do not start processes.
///
/// This loop owns readiness, required-service exit, backend failure, and
/// shutdown behavior. A backend cannot make init report ready by merely
/// starting a process; it must deliver the explicit authenticated Ready
/// event after all required handshakes have completed.
pub(crate) fn run_lifecycle<B: InitBackend>(backend: &mut B) -> ExitCode {
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
            LifecycleEvent::Ready if ready => {
                return stop_after_failure(
                    backend,
                    &mut session,
                    ShutdownCause::ProtocolViolation,
                    BackendError::ProtocolViolation,
                );
            }
            LifecycleEvent::Ready => {
                ready = true;
                pid1_log!("nagi-init: state=ready");
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
    Io,
}

struct ReleaseFailure<Child> {
    child: Child,
    reason: ReleaseFailureReason,
    reaped: Option<ServiceExitStatus>,
}

/// One-shot capability to write the exec gate after registry authorization.
/// It prevents a caller from writing the gate twice or reporting success
/// without committing it.
struct GateCommit<'a, Os: ProcessOs, LaunchRecord> {
    os: &'a mut Os,
    child: &'a mut ProcessHandle<Prepared, Os::Raw>,
    registered: &'a RegisteredLaunch<LaunchRecord>,
    wait_budget: std::time::Duration,
    attempted: bool,
    duplicate_attempt: bool,
    started: Option<std::time::Instant>,
    failure: Option<ReleaseFailureReason>,
}

impl<'a, Os: ProcessOs, LaunchRecord> GateCommit<'a, Os, LaunchRecord> {
    fn new(
        os: &'a mut Os,
        child: &'a mut ProcessHandle<Prepared, Os::Raw>,
        registered: &'a RegisteredLaunch<LaunchRecord>,
        wait_budget: std::time::Duration,
    ) -> Self {
        Self {
            os,
            child,
            registered,
            wait_budget,
            attempted: false,
            duplicate_attempt: false,
            started: None,
            failure: None,
        }
    }

    fn commit(&mut self) -> Result<(), BackendError> {
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
                self.started = Some(started);
                Ok(())
            }
            Err(reason) => {
                self.failure = Some(reason);
                Err(release_failure_backend_error(reason))
            }
        }
    }
}

fn release_failure_backend_error(reason: ReleaseFailureReason) -> BackendError {
    if reason == ReleaseFailureReason::Cancelled {
        BackendError::Cancelled
    } else {
        BackendError::TransportFailure
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
    ) -> std::io::Result<ProcessHandle<Prepared, Self::Raw>>;

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
    ) -> Result<
        ProcessHandle<Running, Self::Raw>,
        ReleaseFailure<ProcessHandle<ReleaseUncertain, Self::Raw>>,
    >;

    fn try_wait<Stage: ProcessStage>(
        &mut self,
        child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<Option<ServiceExitStatus>>;

    fn terminate<Stage: ProcessStage>(
        &mut self,
        child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<()>;
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
    required: bool,
    /// Caller-selected release deadline. There is intentionally no product default.
    release_wait_budget: std::time::Duration,
    /// Caller-selected maximum poll slice for cancellation checks.
    cancel_check_interval: std::time::Duration,
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

struct ManagedProcess<OsChild, ServiceId, LaunchRecord> {
    process_id: u32,
    service: ServiceId,
    required: bool,
    child: OsChild,
    record: LaunchRecord,
    reaped: Option<ServiceExitStatus>,
}

enum PendingChild<Raw> {
    Prepared(ProcessHandle<Prepared, Raw>),
    ReleaseUncertain(ProcessHandle<ReleaseUncertain, Raw>),
    Running(ProcessHandle<Running, Raw>),
}

struct QuarantinedProcess<Raw, LaunchRecord> {
    process_id: u32,
    child: PendingChild<Raw>,
    record: Option<LaunchRecord>,
    reaped: Option<ServiceExitStatus>,
}

struct ProcessLifecycle<Os: ProcessOs, ServiceId, LaunchRecord> {
    os: Os,
    children: Vec<ManagedProcess<ProcessHandle<Running, Os::Raw>, ServiceId, LaunchRecord>>,
    /// Handles awaiting cleanup/revocation are hidden from live_record and block
    /// PID reuse until both obligations have completed.
    quarantine: Vec<QuarantinedProcess<Os::Raw, LaunchRecord>>,
}

impl<Os, ServiceId, LaunchRecord> ProcessLifecycle<Os, ServiceId, LaunchRecord>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy,
{
    fn new(os: Os) -> Self {
        Self {
            os,
            children: Vec::new(),
            quarantine: Vec::new(),
        }
    }

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
            &mut GateCommit<'_, Os, LaunchRecord>,
        ) -> Result<(), BackendError>,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
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
            )
            .map_err(|_| BackendError::TransportFailure)?;
        let process_id = prepared.process_id;
        if process_id <= 1 {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                &mut revoke,
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
                &mut revoke,
            )?;
            return Err(BackendError::ProtocolViolation);
        }
        if cancelled() {
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                &mut revoke,
            )?;
            return Err(BackendError::Cancelled);
        }

        let record = match register(process_id) {
            Ok(record) => record,
            Err(error) => {
                self.discard_pending(
                    process_id,
                    PendingChild::Prepared(prepared),
                    None,
                    None,
                    &mut revoke,
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
                &mut revoke,
            )?;
            return Err(BackendError::Cancelled);
        }

        let registered = RegisteredLaunch { record };
        let mut gate_commit = GateCommit::new(
            &mut self.os,
            &mut prepared,
            &registered,
            launch.release_wait_budget,
        );
        let authorization = authorize(process_id, &record, &mut gate_commit);
        let gate_attempted = gate_commit.attempted;
        let duplicate_gate_attempt = gate_commit.duplicate_attempt;
        let release_started = gate_commit.started;
        let gate_failure = gate_commit.failure;
        drop(gate_commit);

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
                &mut revoke,
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
            Ok(child) => {
                let record = registered.into_record();
                self.children.push(ManagedProcess {
                    process_id,
                    service: launch.service,
                    required: launch.required,
                    child,
                    record,
                    reaped: None,
                });
                Ok(process_id)
            }
            Err(failure) => {
                // The gate byte was committed. A cancellation, timeout, or
                // status-channel failure leaves execution uncertain, so reap
                // and revoke before reporting failure.
                let was_cancelled = matches!(failure.reason, ReleaseFailureReason::Cancelled);
                self.discard_pending(
                    process_id,
                    PendingChild::ReleaseUncertain(failure.child),
                    Some(registered.into_record()),
                    failure.reaped,
                    &mut revoke,
                )?;
                if was_cancelled {
                    Err(BackendError::Cancelled)
                } else {
                    Err(BackendError::TransportFailure)
                }
            }
        }
    }

    /// Retry cleanup/revocation for quarantined launches before reporting a
    /// managed service exit. Quarantine never creates an IPC-visible record.
    fn poll(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
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
            if child_clean && record_clean {
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
            return Ok(Some(LifecycleEvent::ServiceExited {
                service: child.service,
                status,
                required: child.required,
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

    fn active_len(&self) -> usize {
        self.children.len() + self.quarantine.len()
    }

    fn shutdown(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
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
            if child_clean && record_clean {
                self.quarantine.remove(index);
            } else {
                failed = true;
                index += 1;
            }
        }

        // Any registered child that is still alive after this bounded cleanup
        // pass stops being IPC-visible and moves intact into quarantine. A later
        // poll/shutdown retries nonblocking reap while retaining its record/handle.
        let mut index = 0;
        while index < self.children.len() {
            if self.children[index].reaped.is_none() {
                let entry = self.children.remove(index);
                self.quarantine.push(QuarantinedProcess {
                    process_id: entry.process_id,
                    child: PendingChild::Running(entry.child),
                    record: Some(entry.record),
                    reaped: entry.reaped,
                });
                failed = true;
            } else {
                index += 1;
            }
        }

        let mut index = 0;
        while index < self.children.len() {
            let process_id = self.children[index].process_id;
            let record = &self.children[index].record;
            if revoke(process_id, record).is_ok() {
                self.children.remove(index);
                continue;
            }
            failed = true;
            index += 1;
        }

        if failed || !self.children.is_empty() || !self.quarantine.is_empty() {
            Err(BackendError::TransportFailure)
        } else {
            Ok(())
        }
    }

    fn discard_pending(
        &mut self,
        process_id: u32,
        mut child: PendingChild<Os::Raw>,
        mut record: Option<LaunchRecord>,
        already_reaped: Option<ServiceExitStatus>,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let mut reaped = already_reaped;
        let child_clean = Self::attempt_cleanup(&mut self.os, &mut child, &mut reaped);
        let record_clean = if child_clean {
            Self::retry_record_revocation(process_id, &mut record, revoke)
        } else {
            false
        };
        if child_clean && record_clean {
            return Ok(());
        }

        // Keep both the OS handle and any record whose revocation failed.
        self.quarantine.push(QuarantinedProcess {
            process_id,
            child,
            record,
            reaped,
        });
        Err(BackendError::TransportFailure)
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux_x86_64_process {
    use super::*;
    use core::arch::asm;
    use std::io;
    use std::ptr;

    const NR_READ: usize = 0;
    const NR_WRITE: usize = 1;
    const NR_CLOSE: usize = 3;
    const NR_DUP2: usize = 33;
    const NR_SOCKETPAIR: usize = 53;
    const NR_CLONE: usize = 56;
    const NR_EXECVEAT: usize = 322;
    const NR_MEMFD_CREATE: usize = 319;
    const NR_EXIT_GROUP: usize = 231;
    const NR_WAIT4: usize = 61;
    const NR_KILL: usize = 62;
    const NR_SETGROUPS: usize = 116;
    const NR_SETRESUID: usize = 117;
    const NR_SETRESGID: usize = 119;
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
    const NR_CLOSE_RANGE: usize = 436;
    const NR_POLL: usize = 7;
    const NR_RT_SIGPROCMASK: usize = 14;
    const NR_SIGNALFD4: usize = 289;
    #[cfg(linux_pid1_safety_smoke)]
    const NR_NANOSLEEP: usize = 35;

    const EINTR: isize = 4;
    const ESRCH: isize = 3;
    const ECHILD: isize = 10;
    const EAGAIN: isize = 11;
    #[cfg(linux_pid1_safety_smoke)]
    const EBUSY: i32 = 16;
    const SIGCHLD: usize = 17;
    const SIGKILL: usize = 9;
    const SIG_BLOCK: usize = 0;
    const SIG_SETMASK: usize = 2;
    const CLONE_NEWPID: usize = 0x2000_0000;
    const WNOHANG: usize = 1;
    const SFD_CLOEXEC: usize = 0x80000;
    const LINUX_SIGNAL_INFO_BYTES: usize = 128;
    const AF_UNIX: usize = 1;
    const SOCK_STREAM: usize = 1;
    const SOCK_CLOEXEC: usize = 0x80000;
    const O_RDWR: usize = 2;
    const O_CLOEXEC: usize = 0x80000;
    const AT_FDCWD: usize = (-100isize) as usize;
    const F_DUPFD_CLOEXEC: usize = 1030;
    const F_SETFL: usize = 4;
    const O_NONBLOCK: usize = 0x800;
    const MSG_NOSIGNAL: usize = 0x4000;
    const CLOSE_RANGE_CLOEXEC: usize = 1 << 2;
    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const SERVICE_SANDBOX_ID: usize = 65_534;
    const RELEASE_BYTE: u8 = 1;

    // Cache only a successfully created descriptor. Transient setup failures
    // (including fd exhaustion) must remain retryable on the next timed wait.
    static PID1_REAPER: std::sync::OnceLock<OwnedFd> = std::sync::OnceLock::new();
    static PID1_REAPER_WARNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    pub(super) fn wait_for_os_event() {
        if let Err(error) = wait_for_any_child_event() {
            if !PID1_REAPER_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                super::log_pid1_stderr(format_args!(
                    "nagi-init: child reaper wait failed ({error}); retrying with timed reap"
                ));
            }
            std::thread::park_timeout(std::time::Duration::from_millis(250));
        }
    }

    fn pid1_sigchld_fd() -> io::Result<&'static OwnedFd> {
        if let Some(fd) = PID1_REAPER.get() {
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
                SFD_CLOEXEC,
            )
        };
        let fd = OwnedFd(syscall_result(fd)? as i32);
        if let Err(unused_fd) = PID1_REAPER.set(fd) {
            drop(unused_fd);
        }
        PID1_REAPER.get().ok_or_else(|| {
            io::Error::other("SIGCHLD descriptor initialization did not publish a descriptor")
        })
    }

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

    /// Reap all exited children known to PID 1. The bool is true when wait4
    /// observed at least one child that has not exited yet.
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
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "signalfd returned a short SIGCHLD record",
            ));
        }
    }

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

                let mut process_os = LinuxProcessOs;
                let mut service = ProcessHandle::<Running, LinuxChildRaw>::new(
                    service_pid,
                    LinuxChildRaw {
                        gate: None,
                        exec_error: None,
                        executable: None,
                    },
                );
                process_os.terminate(&mut service)?;
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
        let mut process_os = LinuxProcessOs;
        let mut child = ProcessHandle::<Running, LinuxChildRaw>::new(
            process_id,
            LinuxChildRaw {
                gate: None,
                exec_error: None,
                executable: None,
            },
        );
        process_os.terminate(&mut child)?;
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
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "wait4 returned a different child PID",
            ));
        }
        Ok(Some(decode_wait_status(status)))
    }

    fn send_child_error(error_fd: i32, errno: usize) -> ! {
        let bytes = (errno as u32).to_ne_bytes();
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

    unsafe fn child_wait_and_exec(
        gate_parent: i32,
        gate_child: i32,
        error_read: i32,
        error_write: i32,
        null_fd: i32,
        executable_fd: i32,
        argv: *const *const i8,
        envp: *const *const i8,
        execution_identity: LinuxExecutionIdentity,
    ) -> ! {
        unsafe {
            syscall1(NR_CLOSE, gate_parent as usize);
            syscall1(NR_CLOSE, error_read as usize);
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
            syscall1(NR_CLOSE, gate_child as usize);
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
                    send_child_error(error_write, (-result) as usize);
                }
            }
            syscall1(NR_CLOSE, null_fd as usize);
            let closed = syscall3(NR_CLOSE_RANGE, 3, u32::MAX as usize, CLOSE_RANGE_CLOEXEC);
            if closed < 0 {
                send_child_error(error_write, (-closed) as usize);
            }
            // PID 1 blocks SIGCHLD for signalfd-based reaping. Services inherit
            // that signal mask across clone, so restore an empty mask before
            // exec in the private service PID namespace.
            let unblocked_signals = 0u64;
            let signals_unblocked = syscall4(
                NR_RT_SIGPROCMASK,
                SIG_SETMASK,
                &unblocked_signals as *const u64 as usize,
                0,
                core::mem::size_of::<u64>(),
            );
            if signals_unblocked < 0 {
                send_child_error(error_write, (-signals_unblocked) as usize);
            }
            // PID 1 owns Linux-side setup, but service ELFs never inherit its
            // host root identity. These numeric credentials are an OS
            // sandbox detail only; they do not create Nagi identity or grant
            // consent. Drop supplementary groups before the primary IDs, then
            // prohibit privilege-gaining exec transitions.
            let groups_dropped = syscall2(NR_SETGROUPS, 0, 0);
            if groups_dropped < 0 {
                send_child_error(error_write, (-groups_dropped) as usize);
            }
            let group_ids_dropped = syscall3(
                NR_SETRESGID,
                execution_identity.gid as usize,
                execution_identity.gid as usize,
                execution_identity.gid as usize,
            );
            if group_ids_dropped < 0 {
                send_child_error(error_write, (-group_ids_dropped) as usize);
            }
            let user_ids_dropped = syscall3(
                NR_SETRESUID,
                execution_identity.uid as usize,
                execution_identity.uid as usize,
                execution_identity.uid as usize,
            );
            if user_ids_dropped < 0 {
                send_child_error(error_write, (-user_ids_dropped) as usize);
            }
            let no_new_privileges = syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            if no_new_privileges < 0 {
                send_child_error(error_write, (-no_new_privileges) as usize);
            }
            let empty_path = b"\0";
            let executed = syscall5(
                NR_EXECVEAT,
                executable_fd as usize,
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

    pub(super) struct LinuxChildRaw {
        gate: Option<OwnedFd>,
        exec_error: Option<OwnedFd>,
        executable: Option<OwnedFd>,
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    struct LinuxExecStatusWait {
        status_fd: i32,
        process_id: u32,
    }

    impl ExecStatusWait for LinuxExecStatusWait {
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
            let fractional_ms = if timeout.subsec_nanos() % 1_000_000 == 0 {
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
            wait_child_nonblocking(self.process_id)
        }
    }

    impl LinuxChildRaw {
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
            let status_fd = self
                .exec_error
                .as_ref()
                .ok_or(ReleaseFailureReason::Io)?
                .raw();
            let mut wait = LinuxExecStatusWait {
                status_fd,
                process_id,
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
        ) -> io::Result<ProcessHandle<Prepared, Self::Raw>> {
            if execution_identity.uid == 0 || execution_identity.gid == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Linux process identity must not retain root credentials",
                ));
            }
            // Copy the exact PackageView executable slice into a private memfd and
            // seal it before creating the gated child. No pathname is resolved.
            let executable_fd =
                create_sealed_executable(&mut LinuxSealedExecutableOps, executable.bytes())?;
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

            // Each launched child becomes PID 1 in a child namespace. If it exits,
            // Linux terminates every remaining namespace member, letting
            // shutdown target this tracked PID without signaling helpers.
            let cloned = unsafe { syscall5(NR_CLONE, SIGCHLD | CLONE_NEWPID, 0, 0, 0, 0) };
            if cloned == 0 {
                unsafe {
                    child_wait_and_exec(
                        gate_parent.raw(),
                        gate_child.raw(),
                        error_read.raw(),
                        error_write.raw(),
                        null_fd.raw(),
                        executable_fd.raw(),
                        argv.as_ptr(),
                        envp.as_ptr(),
                        execution_identity,
                    )
                }
            }
            let process_id = syscall_result(cloned)? as u32;

            drop(gate_child);
            drop(error_write);
            drop(null_fd);
            Ok(ProcessHandle::new(
                process_id,
                LinuxChildRaw {
                    gate: Some(gate_parent),
                    exec_error: Some(error_read),
                    executable: Some(executable_fd),
                },
            ))
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
            wait_child_nonblocking(child.process_id)
        }

        fn terminate<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            syscall_result(unsafe { syscall2(NR_KILL, child.process_id as usize, SIGKILL) })
                .map(|_| ())
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
        OwnerError::GenerationSaturated => BackendError::Unavailable,
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
    release_wait_budget: std::time::Duration,
    cancel_check_interval: std::time::Duration,
    cancelled: impl FnMut() -> bool,
) -> Result<u32, BackendError>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    Inventory: InstalledPackageInventory,
{
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
            execution_identity: LinuxExecutionIdentity::DEV_SERVICE,
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
            crate::supervisor::authorize_launch_and_commit(record, required_grants, || {
                gate.commit()
            })
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

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const NOTES_INPUT_KEY_CODES: [u16; 5] = [15, 28, 30, 31, 57]; // Tab, Enter, A, S, Space.
#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const NOTES_INPUT_KEY_MAX: usize = 0x2ff;
#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const NOTES_INPUT_KEY_BITMAP_BYTES: usize = (NOTES_INPUT_KEY_MAX + 8) / 8;
#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const NOTES_INPUT_O_NOFOLLOW: i32 = 0x0002_0000;

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
fn mount_notes_sysfs_read_only() -> Result<(), BackendError> {
    const MS_RDONLY: std::os::raw::c_ulong = 1;
    const MS_NOSUID: std::os::raw::c_ulong = 2;
    const MS_NODEV: std::os::raw::c_ulong = 4;
    const MS_NOEXEC: std::os::raw::c_ulong = 8;
    let source = CString::new("sysfs").map_err(|_| BackendError::ProtocolViolation)?;
    let target = CString::new("/sys").map_err(|_| BackendError::ProtocolViolation)?;
    let filesystem = CString::new("sysfs").map_err(|_| BackendError::ProtocolViolation)?;
    // Do not infer an already-mounted filesystem from EBUSY. The private CPIO
    // starts with an empty /sys mountpoint, so any mount failure is fatal here.
    let result = unsafe {
        mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC,
            core::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        pid1_log!(
            "nagi-init: read-only sysfs mount failed ({})",
            std::io::Error::last_os_error()
        );
        Err(BackendError::Unavailable)
    }
}

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
const fn notes_event_ioctl_request(number: u32, size: usize) -> std::os::raw::c_ulong {
    ((2u64 << 30) | ((size as u64) << 16) | ((b'E' as u64) << 8) | number as u64) as _
}

#[cfg(all(linux_desktop_dev_smoke, target_os = "linux", target_arch = "x86_64"))]
fn select_notes_keyboard_device() -> Result<String, BackendError> {
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
            matches.push((path, sysfs_name));
        }
    }

    match matches.as_slice() {
        [(path, sysfs_name)] => {
            pid1_log!(
                "nagi-init: selected evdev keyboard path={path} sysfs_name={sysfs_name:?} required_keys=PASS"
            );
            Ok(path.clone())
        }
        [] => Err(BackendError::Unavailable),
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
fn prepare_notes_desktop_devices() -> Result<String, BackendError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    fn grant_to_notes_desktop(
        path: &str,
        is_expected_device: impl FnOnce(&std::fs::FileType) -> bool,
    ) -> Result<(), BackendError> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| BackendError::Unavailable)?;
        if metadata.uid() != 0 || metadata.gid() != 0 || !is_expected_device(&metadata.file_type())
        {
            return Err(BackendError::ProtocolViolation);
        }
        chown_dev_path_as(path, DEV_NOTES_DESKTOP_UID, DEV_NOTES_DESKTOP_GID)?;
        change_dev_path_mode(path, 0o600)
    }

    mount_notes_sysfs_read_only()?;
    let keyboard_path = select_notes_keyboard_device()?;

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

/// Adapter used while the Linux process backend is not connected. Its optional
/// typed action route is empty in `run()` until a runtime supplier has provided
/// authenticated transport, live registration, and the canonical route.
struct UnconnectedBackend;

impl UnconnectedBackend {
    fn new() -> Self {
        Self
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
        Err(BackendError::Unavailable)
    }

    fn call_service(
        &mut self,
        _session: &mut Self::Session,
        service: &mut Self::ServiceHandle,
        request: ServiceRequest<Self::Action>,
        #[cfg(feature = "linux-service-action-bridge")] cancellation: CancellationToken,
        #[cfg(feature = "linux-service-action-bridge")] timeout: Duration,
    ) -> Result<ServiceResponse<Self::Reply>, ServiceIoError> {
        #[cfg(feature = "linux-service-action-bridge")]
        {
            let Some(service) = service.as_ref() else {
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
mod process_lifecycle_tests {
    use super::*;
    use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
    use nagi_package::PackageService;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io;
    use std::rc::Rc;

    const fn synthetic_static_elf() -> [u8; 128] {
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
    use std::collections::VecDeque;

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
        let mut backend = UnconnectedBackend;
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
            drop(self.package_owner.take());
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
