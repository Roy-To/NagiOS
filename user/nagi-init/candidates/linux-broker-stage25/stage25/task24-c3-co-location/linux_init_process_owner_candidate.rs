#![allow(dead_code, unused_imports, unexpected_cfgs)]
use std::ffi::CString;
use std::collections::HashSet;
use nagi_init_spawn_contract::{
    SpawnedProcessIdentity, SpawnedProcessPidfd,
};
pub(crate) use pid1_terminal_result::{ExactProcessTerminalResult, ExactTerminalState};
use pid1_terminal_result::{TerminalResultLedger, TerminalResultLookupError};
pub(crate) use pid1_owned_launch_lease::{
    BoundLaunchLease, ExactChildLaunchLease, LaunchGateDisposition, LeaseRetirementState,
};
use pid1_owned_launch_lease::{adopt_exact_child_lease, retire_exact_once};
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

const DEV_SERVICE_UID: u32 = 65_534;
const DEV_SERVICE_GID: u32 = 65_534;
const DEV_NOTES_DESKTOP_UID: u32 = 65_533;
const DEV_NOTES_DESKTOP_GID: u32 = 65_533;
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

struct ReleaseFailureInfo {
    reason: ReleaseFailureReason,
    reaped: Option<ServiceExitStatus>,
}

/// One-shot capability to write the exec gate after registry authorization.
/// It prevents a caller from writing the gate twice or reporting success
/// without committing it.
struct GateCommit<'a, Os: ProcessOs, LaunchRecord, LaunchLease> {
    os: &'a mut Os,
    child: &'a mut ProcessHandle<Prepared, Os::Raw>,
    registered: &'a RegisteredLaunch<LaunchRecord>,
    launch_lease: &'a mut Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_cleanup: &'a mut LeaseRetirementState,
    wait_budget: std::time::Duration,
    attempted: bool,
    duplicate_attempt: bool,
    started: Option<std::time::Instant>,
    failure: Option<ReleaseFailureReason>,
    spawn_handoff_complete: bool,
}

impl<'a, Os: ProcessOs, LaunchRecord: Copy + Eq, LaunchLease>
    GateCommit<'a, Os, LaunchRecord, LaunchLease>
{
    fn new(
        os: &'a mut Os,
        child: &'a mut ProcessHandle<Prepared, Os::Raw>,
        registered: &'a RegisteredLaunch<LaunchRecord>,
        launch_lease: &'a mut Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
        lease_cleanup: &'a mut LeaseRetirementState,
        wait_budget: std::time::Duration,
    ) -> Self {
        Self {
            os,
            child,
            registered,
            launch_lease,
            lease_cleanup,
            wait_budget,
            attempted: false,
            duplicate_attempt: false,
            started: None,
            failure: None,
            spawn_handoff_complete: false,
        }
    }

    /// Obtain the sealed child proof, install/register its listener, then move
    /// the resulting non-Clone lease into this exact lifecycle slot before
    /// returning to the authorization callback. The lease remains here if
    /// later gate commit fails or the callback unwinds.
    fn install_and_retain_spawn_lease(
        &mut self,
        install: impl FnOnce(SpawnedProcessPidfd, &LaunchRecord) -> Result<LaunchLease, BackendError>,
    ) -> Result<(), BackendError>
    where
        LaunchLease: ExactChildLaunchLease<LaunchRecord>,
    {
        if self.attempted || self.spawn_handoff_complete || self.launch_lease.is_some() {
            return Err(BackendError::ProtocolViolation);
        }
        let proof = self
            .os
            .pidfd_handoff(self.child)
            .map_err(|_| BackendError::Unavailable)?;
        let process_instance = proof.identity();
        let launch_record = self.registered.record;
        let lease = install(proof, &launch_record)?;
        // Adopt before returning control to the authorization callback. Even
        // a mismatched value must remain owner-held: dropping a registry lease
        // here could revoke only part of a successful external registration.
        adopt_exact_child_lease(
            self.launch_lease,
            self.lease_cleanup,
            process_instance,
            launch_record,
            lease,
        )?;
        self.spawn_handoff_complete = true;
        Ok(())
    }

    fn commit(&mut self) -> Result<(), BackendError> {
        if self.attempted {
            self.duplicate_attempt = true;
            return Err(BackendError::ProtocolViolation);
        }
        if self.os.requires_spawn_proof_handoff() && !self.spawn_handoff_complete {
            return Err(BackendError::ProtocolViolation);
        }
        if self.os.requires_launch_lease() && self.launch_lease.is_none() {
            return Err(BackendError::ProtocolViolation);
        }
        self.attempted = true;
        match self
            .os
            .commit_release_gate(self.child, self.registered, self.wait_budget)
        {
            Ok(started) => {
                if let Some(lease) = self.launch_lease.as_mut() {
                    lease.gate_disposition = LaunchGateDisposition::Released;
                }
                self.started = Some(started);
                Ok(())
            }
            Err(reason) => {
                if let Some(lease) = self.launch_lease.as_mut() {
                    // A failed gate write may be partial. Preserve uncertainty
                    // for the later exact-reap retirement path.
                    lease.gate_disposition = LaunchGateDisposition::Unknown;
                }
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

    /// Duplicate the already-owned atomic clone3 pidfd for the Task-41
    /// process-instance authority. The original remains in `Raw` for the one
    /// ProcessLifecycle terminate/reap/close owner.
    fn pidfd_handoff(
        &self,
        child: &ProcessHandle<Prepared, Self::Raw>,
    ) -> std::io::Result<SpawnedProcessPidfd>;

    /// Duplicate the process-instance proof retained by the existing raw child
    /// owner. Unsupported backends cannot manufacture a C3 cleanup binding.
    fn duplicate_live_spawn_proof<Stage: ProcessStage>(
        &self,
        _child: &ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<SpawnedProcessPidfd> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "live spawn proof duplication is unavailable",
        ))
    }

    /// Real Linux ProcessOs requires a successful identity bridge before the
    /// child can pass its exec gate. Pure host adapters can leave it disabled.
    fn requires_spawn_proof_handoff(&self) -> bool {
        false
    }

    fn requires_launch_lease(&self) -> bool {
        false
    }

    fn same_process_instance<Stage: ProcessStage>(
        &self,
        _child: &ProcessHandle<Stage, Self::Raw>,
        _identity: &SpawnedProcessIdentity,
    ) -> bool {
        false
    }

    fn process_instance_identity<Stage: ProcessStage>(
        &self,
        _child: &ProcessHandle<Stage, Self::Raw>,
    ) -> Option<SpawnedProcessIdentity> {
        None
    }

    fn termination_audit_state<Stage: ProcessStage>(
        &self,
        _child: &ProcessHandle<Stage, Self::Raw>,
    ) -> TerminationAuditState {
        TerminationAuditState::NotAttempted
    }

    fn prepare(
        &mut self,
        executable: ExecutablePayload<'_>,
        arguments: &[&str],
        execution_identity: LinuxExecutionIdentity,
    ) -> std::io::Result<ProcessHandle<Prepared, Self::Raw>>;

    /// Store a child in the lifecycle owner's persistent slot before the
    /// acquisition primitive returns. Existing host adapters inherit this
    /// compatibility default; LinuxProcessOs overrides it at clone success.
    fn prepare_into(
        &mut self,
        executable: ExecutablePayload<'_>,
        arguments: &[&str],
        execution_identity: LinuxExecutionIdentity,
        owner: &mut Option<ProcessHandle<Prepared, Self::Raw>>,
    ) -> std::io::Result<()> {
        if owner.is_some() {
            return Err(std::io::Error::other("process acquisition slot is occupied"));
        }
        *owner = Some(self.prepare(executable, arguments, execution_identity)?);
        Ok(())
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
        child: &mut ProcessHandle<ReleaseUncertain, Self::Raw>,
        started: std::time::Instant,
        wait_budget: std::time::Duration,
        cancel_check_interval: std::time::Duration,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<(), ReleaseFailureInfo>;

    fn try_wait<Stage: ProcessStage>(
        &mut self,
        child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<Option<ServiceExitStatus>>;

    /// Explicitly close child-owned descriptors after reaping; production
    /// adapters must preserve close failure as Unknown rather than Drop success.
    fn close_child<Stage: ProcessStage>(
        &mut self,
        _child: &mut ProcessHandle<Stage, Self::Raw>,
    ) -> std::io::Result<()> {
        Ok(())
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanupAttempt {
    NotAttempted,
    Attempted,
    Complete,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminationAuditState {
    NotAttempted,
    Acknowledged,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminationAuditSummary {
    pub(crate) not_attempted: usize,
    pub(crate) acknowledged: usize,
    pub(crate) unknown: usize,
}

impl TerminationAuditSummary {
    fn record(&mut self, state: TerminationAuditState) {
        match state {
            TerminationAuditState::NotAttempted => {
                self.not_attempted = self.not_attempted.saturating_add(1)
            }
            TerminationAuditState::Acknowledged => {
                self.acknowledged = self.acknowledged.saturating_add(1)
            }
            TerminationAuditState::Unknown => {
                self.unknown = self.unknown.saturating_add(1)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceDisposition {
    AllTrackedResourcesDisposed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CleanupReport {
    pub(crate) resources: ResourceDisposition,
    pub(crate) termination_audit: TerminationAuditSummary,
}

struct ManagedProcess<OsChild, ServiceId, LaunchRecord, LaunchLease> {
    process_id: u32,
    service: ServiceId,
    required: bool,
    child: OsChild,
    record: LaunchRecord,
    record_cleanup: CleanupAttempt,
    launch_lease: Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_cleanup: LeaseRetirementState,
    reaped: Option<ServiceExitStatus>,
    /// A C3-selected exact cleanup request. Lifecycle poll performs the one
    /// termination attempt and continues observation through the sole reaper.
    cleanup_requested: bool,
}

enum PendingChild<Raw> {
    Prepared(ProcessHandle<Prepared, Raw>),
    ReleaseUncertain(ProcessHandle<ReleaseUncertain, Raw>),
    Running(ProcessHandle<Running, Raw>),
}

struct QuarantinedProcess<Raw, LaunchRecord, LaunchLease> {
    process_id: u32,
    child: PendingChild<Raw>,
    record: Option<LaunchRecord>,
    record_cleanup: CleanupAttempt,
    launch_lease: Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    lease_cleanup: LeaseRetirementState,
    reaped: Option<ServiceExitStatus>,
}

struct ProcessLifecycle<Os: ProcessOs, ServiceId, LaunchRecord, LaunchLease = ()> {
    os: Os,
    children: Vec<ManagedProcess<ProcessHandle<Running, Os::Raw>, ServiceId, LaunchRecord, LaunchLease>>,
    /// Handles awaiting cleanup/revocation are hidden from live_record and block
    /// PID reuse until both obligations have completed.
    quarantine: Vec<QuarantinedProcess<Os::Raw, LaunchRecord, LaunchLease>>,
    /// Reserved before Linux clone and retained if prepare unwinds before handoff.
    acquiring: Option<ProcessHandle<Prepared, Os::Raw>>,
    /// Handle remains owner-held while the exec acknowledgement is awaited.
    waiting: Option<ProcessHandle<ReleaseUncertain, Os::Raw>>,
    /// Registry record becomes owner-held before authorization callbacks run.
    acquiring_record: Option<LaunchRecord>,
    /// Non-Clone endpoint/record leases stay in the same owner as the child.
    acquiring_lease: Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    acquiring_lease_cleanup: LeaseRetirementState,
    /// Consumable exact-instance results; this reports prior reaping and never
    /// performs another wait.
    terminal_results: TerminalResultLedger<LaunchRecord>,
}

impl<Os, ServiceId, LaunchRecord, LaunchLease>
    ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy + Eq,
{
    fn new(os: Os) -> Self {
        Self {
            os,
            children: Vec::new(),
            quarantine: Vec::new(),
            acquiring: None,
            waiting: None,
            acquiring_record: None,
            acquiring_lease: None,
            acquiring_lease_cleanup: LeaseRetirementState::NotAttempted,
            terminal_results: TerminalResultLedger::default(),
        }
    }

    /// Duplicate only the proof retained for a unique, live child whose exact
    /// LaunchRecord and non-Clone endpoint lease still agree.
    pub(crate) fn duplicate_exact_child_proof_for_record(
        &self,
        record: LaunchRecord,
    ) -> Result<SpawnedProcessPidfd, BackendError>
    where
        LaunchLease: ExactChildLaunchLease<LaunchRecord>,
    {
        let mut matching = self.children.iter().filter(|entry| {
            entry.record == record
                && entry.reaped.is_none()
                && !entry.cleanup_requested
                && entry.launch_lease.as_ref().is_some_and(|bound| {
                    bound.record == record
                        && bound.gate_disposition == LaunchGateDisposition::Released
                        && self.os.same_process_instance(&entry.child, &bound.process_instance)
                        && bound
                            .lease
                            .matches_spawn(&bound.process_instance, &record)
                })
        });
        let entry = matching.next().ok_or(BackendError::IdentityRejected)?;
        if matching.next().is_some() {
            return Err(BackendError::IdentityRejected);
        }
        let bound = entry
            .launch_lease
            .as_ref()
            .ok_or(BackendError::IdentityRejected)?;
        let current = self
            .os
            .process_instance_identity(&entry.child)
            .ok_or(BackendError::IdentityRejected)?;
        if !current.same_instance(&bound.process_instance) {
            return Err(BackendError::IdentityRejected);
        }
        self.os
            .duplicate_live_spawn_proof(&entry.child)
            .map_err(|_| BackendError::Unavailable)
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
            &mut GateCommit<'_, Os, LaunchRecord, LaunchLease>,
        ) -> Result<(), BackendError>,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<u32, BackendError> {
        if cancelled() {
            return Err(BackendError::Cancelled);
        }

        if self.acquiring.is_some()
            || self.waiting.is_some()
            || self.acquiring_record.is_some()
            || self.acquiring_lease.is_some()
            || self.acquiring_lease_cleanup != LeaseRetirementState::NotAttempted
        {
            return Err(BackendError::TransportFailure);
        }
        if self
            .os
            .prepare_into(
                launch.executable,
                launch.arguments,
                launch.execution_identity,
                &mut self.acquiring,
            )
            .is_err()
        {
            if let Some(prepared) = self.acquiring.take() {
                let process_id = prepared.process_id;
                self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                &mut revoke,
                )?;
            }
            return Err(BackendError::TransportFailure);
        }
        let process_id = self
            .acquiring
            .as_ref()
            .map(|child| child.process_id)
            .ok_or(BackendError::TransportFailure)?;
        if process_id <= 1 {
            let prepared = self.acquiring.take().expect("prepared child retained");
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
            let prepared = self.acquiring.take().expect("prepared child retained");
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
            let prepared = self.acquiring.take().expect("prepared child retained");
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                None,
                None,
                &mut revoke,
            )?;
            return Err(BackendError::Cancelled);
        }

        // The process handle remains in `self.acquiring` across register and
        // authorize callbacks. If either unwinds, the backend-level owner can
        // still find it after the outer catch.
        let record = match register(process_id) {
            Ok(record) => {
                self.acquiring_record = Some(record);
                record
            }
            Err(error) => {
                let prepared = self.acquiring.take().expect("prepared child retained");
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
        if cancelled() {
            let prepared = self.acquiring.take().expect("prepared child retained");
            let owned_record = self.acquiring_record.take();
            self.discard_pending(
                process_id,
                PendingChild::Prepared(prepared),
                owned_record,
                None,
                &mut revoke,
            )?;
            return Err(BackendError::Cancelled);
        }

        let registered = RegisteredLaunch { record };
        let mut gate_commit = {
            let prepared = self.acquiring.as_mut().expect("prepared child retained");
            GateCommit::new(
                &mut self.os,
                prepared,
                &registered,
                &mut self.acquiring_lease,
                &mut self.acquiring_lease_cleanup,
                launch.release_wait_budget,
            )
        };
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
            let prepared = self.acquiring.take().expect("prepared child retained");
            let pending = if gate_attempted {
                PendingChild::ReleaseUncertain(prepared.transition())
            } else {
                PendingChild::Prepared(prepared)
            };
            let owned_record = self.acquiring_record.take();
            self.discard_pending(
                process_id,
                pending,
                owned_record,
                None,
                &mut revoke,
            )?;
            return Err(error);
        }

        let started = release_started.expect("successful gate commit has a start time");
        let prepared = self.acquiring.take().expect("prepared child retained");
        self.waiting = Some(prepared.transition());
        let mut cancellation_panicked = false;
        let mut safe_cancelled = || {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cancelled())) {
                Ok(value) => value,
                Err(_) => {
                    cancellation_panicked = true;
                    true
                }
            }
        };
        let ack_result = {
            let child = self.waiting.as_mut().expect("exec-wait child retained");
            self.os.wait_exec_ack(
                child,
                started,
                launch.release_wait_budget,
                launch.cancel_check_interval,
                &mut safe_cancelled,
            )
        };
        match ack_result {
            Ok(()) if !cancellation_panicked => {
                self.children.reserve(1);
                let child = self.waiting.take().expect("exec-wait child retained").transition();
                let record = self.acquiring_record.take().expect("registered record retained");
                self.children.push(ManagedProcess {
                    process_id,
                    service: launch.service,
                    required: launch.required,
                    child,
                    record,
                    record_cleanup: CleanupAttempt::NotAttempted,
                    launch_lease: self.acquiring_lease.take(),
                    lease_cleanup: std::mem::replace(
                        &mut self.acquiring_lease_cleanup,
                        LeaseRetirementState::NotAttempted,
                    ),
                    reaped: None,
                    cleanup_requested: false,
                });
                Ok(process_id)
            }
            Ok(()) => {
                let child = self.waiting.take().expect("exec-wait child retained");
                let record = self.acquiring_record.take();
                self.discard_pending(
                    process_id,
                    PendingChild::ReleaseUncertain(child),
                    record,
                    None,
                    &mut revoke,
                )?;
                Err(BackendError::TransportFailure)
            }
            Err(failure) => {
                let child = self.waiting.take().expect("exec-wait child retained");
                let was_cancelled = cancellation_panicked
                    || matches!(failure.reason, ReleaseFailureReason::Cancelled);
                let record = self.acquiring_record.take();
                self.discard_pending(
                    process_id,
                    PendingChild::ReleaseUncertain(child),
                    record,
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
        mut retire_lease: impl FnMut(
            &mut BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<Option<LifecycleEvent<ServiceId>>, BackendError> {
        let mut index = 0;
        let mut cleanup_failed = false;
        while index < self.quarantine.len() {
            self.terminal_results
                .reserve_completion_slot()
                .map_err(|_| BackendError::TransportFailure)?;
            let entry = &mut self.quarantine[index];
            let completed_record = entry.record;
            let child_clean =
                Self::attempt_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped);
            let record_clean = if child_clean {
                Self::retry_record_revocation(
                    entry.process_id, &mut entry.record, &mut entry.record_cleanup, &mut revoke,
                )
            } else {
                false
            };
            let process_instance = Self::pending_process_instance(&self.os, &entry.child);
            let lease_record = entry
                .launch_lease
                .as_ref()
                .map(|lease| lease.record)
                .or(completed_record);
            let gate_disposition = entry
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let lease_clean = if child_clean && record_clean {
                match (process_instance.as_ref(), lease_record, entry.reaped) {
                    (Some(identity), Some(record), Some(status)) => Self::retire_bound_launch_lease(
                        &mut entry.launch_lease,
                        &mut entry.lease_cleanup,
                        identity,
                        record,
                        status,
                        &mut retire_lease,
                    ),
                    (_, _, _) if entry.launch_lease.is_none() => true,
                    _ => false,
                }
            } else {
                false
            };
            if child_clean && record_clean && lease_clean {
                if let (Some(record), Some(status), Some(process_instance)) = (
                    lease_record,
                    entry.reaped,
                    process_instance,
                ) {
                    self.terminal_results
                        .push_complete(process_instance, record, status, gate_disposition);
                }
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
            if self.children[index].cleanup_requested && self.children[index].reaped.is_none() {
                self.terminal_results
                    .reserve_completion_slot()
                    .map_err(|_| BackendError::TransportFailure)?;
                let entry = &mut self.children[index];
                if !Self::attempt_running_cleanup(
                    &mut self.os,
                    &mut entry.child,
                    &mut entry.reaped,
                ) {
                    continue;
                }
            } else if self.children[index].reaped.is_none() {
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
            self.terminal_results
                .reserve_completion_slot()
                .map_err(|_| BackendError::TransportFailure)?;
            if self
                .os
                .close_child(&mut self.children[index].child)
                .is_err()
            {
                return Err(BackendError::TransportFailure);
            }
            let process_id = self.children[index].process_id;
            let record_clean = {
                let entry = &mut self.children[index];
                Self::attempt_record_revocation(
                    process_id, &entry.record, &mut entry.record_cleanup, &mut revoke,
                )
            };
            if !record_clean {
                return Err(BackendError::TransportFailure);
            }
            let process_instance = self.os.process_instance_identity(&self.children[index].child);
            let record = self.children[index].record;
            let gate_disposition = self.children[index]
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let lease_clean = {
                let child = &mut self.children[index];
                match process_instance.as_ref() {
                    Some(identity) => Self::retire_bound_launch_lease(
                        &mut child.launch_lease,
                        &mut child.lease_cleanup,
                        identity,
                        record,
                        status,
                        &mut retire_lease,
                    ),
                    None if child.launch_lease.is_none() => true,
                    None => false,
                }
            };
            if !lease_clean {
                return Err(BackendError::TransportFailure);
            }
            let child = self.children.remove(index);
            if let Some(process_instance) = process_instance {
                self.terminal_results
                    .push_complete(process_instance, child.record, status, gate_disposition);
            }
            return Ok(Some(LifecycleEvent::ServiceExited {
                service: child.service,
                status,
                required: child.required,
            }));
        }
        Ok(None)
    }

    /// Query one exact child/record pair. The only successful terminal status
    /// comes from this lifecycle's pidfd-verified, single-owner wait path.
    /// `Unknown` remains explicit and is never converted to completion.
    pub(crate) fn query_terminal_result(
        &mut self,
        proof: &SpawnedProcessPidfd,
        record: LaunchRecord,
    ) -> Result<ExactProcessTerminalResult<LaunchRecord>, BackendError> {
        let identity = proof.identity();
        match self.terminal_results.take_complete(&identity, record) {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            Err(TerminalResultLookupError::RecordMismatch) => {
                return Err(BackendError::IdentityRejected)
            }
        }

        let process_id = proof.process_id();
        if let Some(child) = self.children.iter_mut().find(|child| child.process_id == process_id) {
            if !self.os.same_process_instance(&child.child, &identity) || child.record != record {
                return Err(BackendError::IdentityRejected);
            }
            let gate_disposition = child
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let state = match child.reaped {
                Some(status) => ExactTerminalState::Reaped(status),
                None => match self.os.try_wait(&mut child.child) {
                    Ok(Some(status)) => {
                        child.reaped = Some(status);
                        ExactTerminalState::Reaped(status)
                    }
                    Ok(None) => ExactTerminalState::Pending,
                    Err(_) => ExactTerminalState::Unknown,
                },
            };
            return Ok(ExactProcessTerminalResult {
                process_instance: identity,
                record,
                gate_disposition,
                state,
            });
        }

        if let Some(entry) = self.quarantine.iter_mut().find(|entry| entry.process_id == process_id) {
            let gate_disposition = entry
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let owned_record = entry
                .record
                .or_else(|| entry.launch_lease.as_ref().map(|lease| lease.record));
            let same_instance = match &entry.child {
                PendingChild::Prepared(child) => self.os.same_process_instance(child, &identity),
                PendingChild::ReleaseUncertain(child) => {
                    self.os.same_process_instance(child, &identity)
                }
                PendingChild::Running(child) => self.os.same_process_instance(child, &identity),
            };
            if !same_instance || owned_record != Some(record) {
                return Err(BackendError::IdentityRejected);
            }
            let state = if let Some(status) = entry.reaped {
                ExactTerminalState::Reaped(status)
            } else {
                let waited = match &mut entry.child {
                    PendingChild::Prepared(child) => self.os.try_wait(child),
                    PendingChild::ReleaseUncertain(child) => self.os.try_wait(child),
                    PendingChild::Running(child) => self.os.try_wait(child),
                };
                match waited {
                    Ok(Some(status)) => {
                        entry.reaped = Some(status);
                        ExactTerminalState::Reaped(status)
                    }
                    Ok(None) => ExactTerminalState::Pending,
                    Err(_) => ExactTerminalState::Unknown,
                }
            };
            return Ok(ExactProcessTerminalResult {
                process_instance: identity,
                record,
                gate_disposition,
                state,
            });
        }

        Ok(ExactProcessTerminalResult {
            process_instance: identity,
            record,
            gate_disposition: None,
            state: ExactTerminalState::Unknown,
        })
    }

    fn pending_process_instance(
        os: &Os,
        child: &PendingChild<Os::Raw>,
    ) -> Option<SpawnedProcessIdentity> {
        match child {
            PendingChild::Prepared(child) => os.process_instance_identity(child),
            PendingChild::ReleaseUncertain(child) => os.process_instance_identity(child),
            PendingChild::Running(child) => os.process_instance_identity(child),
        }
    }

    fn retire_bound_launch_lease(
        lease: &mut Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
        state: &mut LeaseRetirementState,
        expected_instance: &SpawnedProcessIdentity,
        expected_record: LaunchRecord,
        status: ServiceExitStatus,
        retire: &mut impl FnMut(
            &mut BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> bool {
        retire_exact_once(
            lease,
            state,
            expected_instance,
            expected_record,
            status,
            retire,
        )
    }

    /// A PID is not an identity. Only a released, unreaped process with a live
    /// registered record can be resolved here.
    fn live_record(&self, process_id: u32) -> Option<&LaunchRecord> {
        self.children
            .iter()
            .find(|entry| {
                entry.process_id == process_id
                    && entry.reaped.is_none()
                    && !entry.cleanup_requested
            })
            .map(|entry| &entry.record)
    }

    /// Queue cleanup for exactly one clone3-backed child and let the existing
    /// ProcessLifecycle/Pid1ChildReaper finish it. The attempt is recorded
    /// before the syscall; an error never permits a second request.
    pub(crate) fn request_cleanup_exact(
        &mut self,
        proof: &SpawnedProcessPidfd,
        record: LaunchRecord,
    ) -> Result<(), BackendError> {
        let identity = proof.identity();
        let process_id = proof.process_id();
        if let Some(entry) = self.children.iter_mut().find(|entry| entry.process_id == process_id) {
            if !self.os.same_process_instance(&entry.child, &identity)
                || entry.record != record
                || entry.cleanup_requested
            {
                return Err(BackendError::IdentityRejected);
            }
            let Some(bound) = entry.launch_lease.as_ref() else {
                return Err(BackendError::IdentityRejected);
            };
            if !bound.process_instance.same_instance(&identity)
                || bound.record != record
                || !bound.lease.matches_spawn(&identity, &record)
            {
                return Err(BackendError::IdentityRejected);
            }
            entry.cleanup_requested = true;
            return self
                .os
                .terminate(&mut entry.child)
                .map_err(|_| BackendError::TransportFailure);
        }
        if let Some(entry) = self.quarantine.iter().find(|entry| entry.process_id == process_id) {
            let same_instance = match &entry.child {
                PendingChild::Prepared(child) => self.os.same_process_instance(child, &identity),
                PendingChild::ReleaseUncertain(child) => {
                    self.os.same_process_instance(child, &identity)
                }
                PendingChild::Running(child) => self.os.same_process_instance(child, &identity),
            };
            let owned_record = entry
                .record
                .or_else(|| entry.launch_lease.as_ref().map(|lease| lease.record));
            if same_instance && owned_record == Some(record) {
                return Ok(());
            }
        }
        Err(BackendError::IdentityRejected)
    }

    fn active_len(&self) -> usize {
        self.children.len()
            + self.quarantine.len()
            + (self.acquiring.is_some() as usize)
            + (self.waiting.is_some() as usize)
            + (self.acquiring_record.is_some() as usize)
            + (self.acquiring_lease.is_some() as usize)
    }

    fn shutdown(
        &mut self,
        mut revoke: impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
        mut retire_lease: impl FnMut(
            &mut BoundLaunchLease<LaunchLease, LaunchRecord>,
            ServiceExitStatus,
        ) -> Result<(), BackendError>,
    ) -> Result<CleanupReport, BackendError> {
        // Each Linux service is the init process of its own PID namespace.
        // Killing a tracked service therefore terminates its remaining namespace
        // members. Never signal untracked processes in PID 1's namespace.
        let mut failed = false;
        let mut termination_audit = TerminationAuditSummary::default();
        let mut audited_process_ids = HashSet::new();
        if let Some(child) = self.waiting.take() {
            let process_id = child.process_id;
            self.quarantine.push(QuarantinedProcess {
                process_id,
                child: PendingChild::ReleaseUncertain(child),
                record: self.acquiring_record.take(),
                record_cleanup: CleanupAttempt::NotAttempted,
                launch_lease: self.acquiring_lease.take(),
                lease_cleanup: std::mem::replace(
                    &mut self.acquiring_lease_cleanup,
                    LeaseRetirementState::NotAttempted,
                ),
                reaped: None,
            });
        }
        if let Some(child) = self.acquiring.take() {
            let process_id = child.process_id;
            self.quarantine.push(QuarantinedProcess {
                process_id,
                child: PendingChild::Prepared(child),
                record: self.acquiring_record.take(),
                record_cleanup: CleanupAttempt::NotAttempted,
                launch_lease: self.acquiring_lease.take(),
                lease_cleanup: std::mem::replace(
                    &mut self.acquiring_lease_cleanup,
                    LeaseRetirementState::NotAttempted,
                ),
                reaped: None,
            });
        }
        for entry in &mut self.children {
            self.terminal_results
                .reserve_completion_slot()
                .map_err(|_| BackendError::TransportFailure)?;
            if !Self::attempt_running_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped) {
                failed = true;
            }
            if audited_process_ids.insert(entry.process_id) {
                termination_audit.record(self.os.termination_audit_state(&entry.child));
            }
        }

        let mut index = 0;
        while index < self.quarantine.len() {
            self.terminal_results
                .reserve_completion_slot()
                .map_err(|_| BackendError::TransportFailure)?;
            let entry = &mut self.quarantine[index];
            let child_clean =
                Self::attempt_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped);
            if audited_process_ids.insert(entry.process_id) {
                termination_audit.record(Self::pending_termination_audit(&self.os, &entry.child));
            }
            let record_clean = if child_clean {
                Self::retry_record_revocation(
                    entry.process_id, &mut entry.record, &mut entry.record_cleanup, &mut revoke,
                )
            } else {
                false
            };
            let process_instance = Self::pending_process_instance(&self.os, &entry.child);
            let terminal_record = entry
                .launch_lease
                .as_ref()
                .map(|lease| lease.record)
                .or(entry.record);
            let gate_disposition = entry
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let lease_clean = if child_clean && record_clean {
                match (process_instance.as_ref(), terminal_record, entry.reaped) {
                    (Some(identity), Some(record), Some(status)) => Self::retire_bound_launch_lease(
                        &mut entry.launch_lease,
                        &mut entry.lease_cleanup,
                        identity,
                        record,
                        status,
                        &mut retire_lease,
                    ),
                    (_, _, _) if entry.launch_lease.is_none() => true,
                    _ => false,
                }
            } else {
                false
            };
            if child_clean && record_clean && lease_clean {
                if let (Some(record), Some(status), Some(process_instance)) = (
                    terminal_record,
                    entry.reaped,
                    process_instance,
                ) {
                    self.terminal_results
                        .push_complete(process_instance, record, status, gate_disposition);
                }
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
                    record_cleanup: entry.record_cleanup,
                    launch_lease: entry.launch_lease,
                    lease_cleanup: entry.lease_cleanup,
                    reaped: entry.reaped,
                });
                failed = true;
            } else {
                index += 1;
            }
        }

        let mut index = 0;
        while index < self.children.len() {
            let child_clean = {
                let entry = &mut self.children[index];
                Self::attempt_running_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped)
            };
            if audited_process_ids.insert(self.children[index].process_id) {
                termination_audit.record(
                    self.os
                        .termination_audit_state(&self.children[index].child),
                );
            }
            if !child_clean {
                failed = true;
                index += 1;
                continue;
            }
            let process_id = self.children[index].process_id;
            let record_clean = {
                let entry = &mut self.children[index];
                Self::attempt_record_revocation(
                    process_id, &entry.record, &mut entry.record_cleanup, &mut revoke,
                )
            };
            let status = self.children[index].reaped;
            let process_instance = self.os.process_instance_identity(&self.children[index].child);
            let gate_disposition = self.children[index]
                .launch_lease
                .as_ref()
                .map(|lease| lease.gate_disposition);
            let lease_clean = if record_clean {
                let child = &mut self.children[index];
                match (process_instance.as_ref(), status) {
                    (Some(identity), Some(status)) => Self::retire_bound_launch_lease(
                        &mut child.launch_lease,
                        &mut child.lease_cleanup,
                        identity,
                        child.record,
                        status,
                        &mut retire_lease,
                    ),
                    (_, _) if child.launch_lease.is_none() => true,
                    _ => false,
                }
            } else {
                false
            };
            if record_clean && lease_clean {
                let child = self.children.remove(index);
                if let (Some(process_instance), Some(status)) =
                    (process_instance, child.reaped)
                {
                    self.terminal_results.push_complete(
                        process_instance,
                        child.record,
                        status,
                        gate_disposition,
                    );
                }
                continue;
            }
            failed = true;
            index += 1;
        }

        if failed
            || self.acquiring.is_some()
            || self.waiting.is_some()
            || self.acquiring_record.is_some()
            || self.acquiring_lease.is_some()
            || !self.children.is_empty()
            || !self.quarantine.is_empty()
        {
            Err(BackendError::TransportFailure)
        } else {
            Ok(CleanupReport {
                resources: ResourceDisposition::AllTrackedResourcesDisposed,
                termination_audit,
            })
        }
    }

    fn pending_termination_audit(
        os: &Os,
        child: &PendingChild<Os::Raw>,
    ) -> TerminationAuditState {
        match child {
            PendingChild::Prepared(child) => os.termination_audit_state(child),
            PendingChild::ReleaseUncertain(child) => os.termination_audit_state(child),
            PendingChild::Running(child) => os.termination_audit_state(child),
        }
    }

    fn discard_pending(
        &mut self,
        process_id: u32,
        child: PendingChild<Os::Raw>,
        record: Option<LaunchRecord>,
        already_reaped: Option<ServiceExitStatus>,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        // Publish the real child/record pair before terminate, reap, or revoke.
        // If a callback unwinds, the outer lifecycle owner still retains it.
        let index = self.quarantine.len();
        self.quarantine.push(QuarantinedProcess {
            process_id,
            child,
            record,
            record_cleanup: CleanupAttempt::NotAttempted,
            launch_lease: self.acquiring_lease.take(),
            lease_cleanup: std::mem::replace(
                &mut self.acquiring_lease_cleanup,
                LeaseRetirementState::NotAttempted,
            ),
            reaped: already_reaped,
        });
        let child_clean = {
            let entry = &mut self.quarantine[index];
            Self::attempt_cleanup(&mut self.os, &mut entry.child, &mut entry.reaped)
        };
        let record_clean = if child_clean {
            let entry = &mut self.quarantine[index];
            Self::retry_record_revocation(
                process_id, &mut entry.record, &mut entry.record_cleanup, revoke,
            )
        } else {
            false
        };
        if child_clean && record_clean && self.quarantine[index].launch_lease.is_none() {
            self.quarantine.remove(index);
            Ok(())
        } else {
            Err(BackendError::TransportFailure)
        }
    }

    fn attempt_record_revocation(
        process_id: u32,
        record: &LaunchRecord,
        state: &mut CleanupAttempt,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> bool {
        match *state {
            CleanupAttempt::Complete => true,
            CleanupAttempt::Attempted | CleanupAttempt::Unknown => false,
            CleanupAttempt::NotAttempted => {
                *state = CleanupAttempt::Attempted;
                if revoke(process_id, record).is_ok() {
                    *state = CleanupAttempt::Complete;
                    true
                } else {
                    *state = CleanupAttempt::Unknown;
                    false
                }
            }
        }
    }

    fn retry_record_revocation(
        process_id: u32,
        record: &mut Option<LaunchRecord>,
        state: &mut CleanupAttempt,
        revoke: &mut impl FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    ) -> bool {
        let Some(current) = record.as_ref() else {
            *state = CleanupAttempt::Complete;
            return true;
        };
        if Self::attempt_record_revocation(process_id, current, state, revoke) {
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
            return os.close_child(child).is_ok();
        }

        match os.try_wait(child) {
            Ok(Some(status)) => {
                *reaped = Some(status);
                return os.close_child(child).is_ok();
            }
            Ok(None) => {}
            // An unknown/failed wait cannot safely be followed by kill(pid):
            // the PID may already have been consumed and reused.
            Err(_) => return false,
        }

        // SIGKILL is asynchronous and attempted once by the raw-handle owner.
        // A later SIGCHLD pass observes/reaps the exact PID.
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
                os.close_child(child).is_ok()
            }
            Ok(None) | Err(_) => false,
        }
    }
}

#[path = "pid1_child_reaper.rs"]
mod pid1_child_reaper;

#[path = "pid1_terminal_result.rs"]
mod pid1_terminal_result;

#[path = "pid1_owned_launch_lease.rs"]
mod pid1_owned_launch_lease;
#[path = "reconcile.rs"]
mod reconcile;
#[path = "process_lifecycle_exact_child_source.rs"]
mod process_lifecycle_exact_child_source;
#[path = "task41_exact_child_launch_lease.rs"]
mod task41_exact_child_launch_lease;

include!("linux_x86_64_process.rs");
