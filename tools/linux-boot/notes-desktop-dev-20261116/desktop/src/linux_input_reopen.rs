//! Task-local Linux adapter for reopening only the input path already passed
//! to the Notes process by its existing grant setup.
//!
//! The caller supplies the existing `open_keyboard` routine as the opener so
//! that the product's character-device and key-capability checks remain in
//! force. This adapter never scans `/dev/input`, changes ownership or mode,
//! asks init for a new grant, or chooses a fallback path.

use crate::input_reconnect::{ReconnectCandidate, ReconnectError, ReconnectGate, ReconnectPhase};
use crate::input_recovery::{
    ReconnectProofError, RecoveryHost, RecoveryOwner, RecoveryPhase, RecoveryStep, RecoveryTicket,
};
use linux_evdev_stream::{EvdevStream, LinuxEvdevTarget};
use std::fmt::Display;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_int, c_ulong};
use std::path::{Path, PathBuf};

const UNIQUE_ID_BUFFER_BYTES: usize = 256;

/// Opaque identity produced only by this module's opened-FD EVIOCGUNIQ path
/// in normal builds. The tuple field and production constructor are private;
/// unit fixtures have a separate `cfg(test)` constructor below.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeviceIdentity(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeviceIdentityError {
    Empty,
    EventNodeToken,
}

impl DeviceIdentity {
    fn from_kernel_unique_id(value: String) -> Result<Self, DeviceIdentityError> {
        if value.trim().is_empty() {
            return Err(DeviceIdentityError::Empty);
        }
        let node_name = value.trim().rsplit('/').next().unwrap_or(value.trim());
        if node_name.strip_prefix("event").is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            return Err(DeviceIdentityError::EventNodeToken);
        }
        Ok(Self(value))
    }

    #[cfg(test)]
    pub(crate) fn from_test_identity(
        value: impl Into<String>,
    ) -> Result<Self, DeviceIdentityError> {
        Self::from_kernel_unique_id(value.into())
    }
}

const fn linux_ioc_read(kind: u32, number: u32, size: usize) -> c_ulong {
    ((2u64 << 30) | ((size as u64) << 16) | ((kind as u64) << 8) | number as u64) as c_ulong
}

const EVIOCGUNIQ: c_ulong = linux_ioc_read(b'E' as u32, 0x08, UNIQUE_ID_BUFFER_BYTES);

unsafe extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UniqueIdentityError {
    Io {
        raw_os_error: Option<i32>,
        message: String,
    },
    Missing,
    Truncated,
}

impl std::fmt::Display for UniqueIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { message, .. } => formatter.write_str(message),
            Self::Missing => formatter.write_str("evdev unique ID is empty"),
            Self::Truncated => formatter.write_str("evdev unique ID exceeds the query buffer"),
        }
    }
}

/// Query the Linux evdev unique-id ioctl from this exact opened handle.
/// Empty IDs, failed ioctls, and IDs too long for the fixed buffer fail closed.
/// Device name, event-node path, bus/vendor/product, and physical path are not
/// used as identity fallbacks.
pub(crate) fn identity_from_open_evdev(
    device: &File,
) -> Result<DeviceIdentity, UniqueIdentityError> {
    let mut bytes = [0u8; UNIQUE_ID_BUFFER_BYTES];
    // EVIOCGUNIQ is defined by the Linux input UAPI as _IOC(_IOC_READ, 'E',
    // 0x08, len). Its request is issued on the same file descriptor whose
    // identity is being captured or re-opened.
    let result = unsafe {
        ioctl(
            device.as_raw_fd(),
            EVIOCGUNIQ,
            bytes.as_mut_ptr().cast::<std::ffi::c_void>(),
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        return Err(UniqueIdentityError::Io {
            raw_os_error: error.raw_os_error(),
            message: error.to_string(),
        });
    }
    if result == 0 {
        return Err(UniqueIdentityError::Missing);
    }
    let Some(length) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(UniqueIdentityError::Truncated);
    };
    if length == 0 {
        return Err(UniqueIdentityError::Missing);
    }

    // Hex encoding preserves arbitrary kernel-returned bytes without lossy
    // UTF-8 conversion and yields a non-path opaque token for the gate.
    let token = bytes[..length]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    DeviceIdentity::from_kernel_unique_id(format!("evdev-uniq:{token}"))
        .map_err(|_| UniqueIdentityError::Missing)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReopenAttemptError {
    InvalidRecoveryPhase(RecoveryPhase),
    Unavailable {
        reason: String,
        notification_error: Option<String>,
    },
    InvalidReconnectProof(ReconnectProofError),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReconnectedInputLossError {
    InvalidRecoveryPhase(RecoveryPhase),
    IdentityGateUnavailable,
    IdentityGateNotConnected(ReconnectPhase),
    IdentityGateTransition(ReconnectError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconnectedInputLossAction {
    RetryOriginalGrant,
}

pub(crate) struct ReopenedGrantedInput {
    pub(crate) stream: EvdevStream<File>,
    pub(crate) path: PathBuf,
    pub(crate) parser_epoch: u64,
    pub(crate) discarded_partial_record_bytes: usize,
    pub(crate) recovery_step: RecoveryStep,
}

/// Evidence minted only after this module has reopened the original grant,
/// queried the opened handle's real EVIOCGUNIQ value, and received an identity
/// match from `ReconnectGate`. It is bound to one recovery ticket and cannot
/// be cloned or constructed outside this module in ordinary builds.
pub(crate) struct VerifiedReconnectProof {
    owner_ticket: RecoveryTicket,
    parser_epoch: u64,
    consumed: bool,
}

impl VerifiedReconnectProof {
    fn after_gate_match(owner_ticket: RecoveryTicket, parser_epoch: u64) -> Self {
        Self {
            owner_ticket,
            parser_epoch,
            consumed: false,
        }
    }

    pub(crate) fn consume_for(
        &mut self,
        owner_ticket: RecoveryTicket,
        expected_parser_epoch: u64,
    ) -> Result<u64, ReconnectProofError> {
        if self.owner_ticket != owner_ticket {
            return Err(ReconnectProofError::WrongOwner);
        }
        if self.consumed {
            return Err(ReconnectProofError::AlreadyConsumed);
        }
        if self.parser_epoch != expected_parser_epoch {
            return Err(ReconnectProofError::WrongParserEpoch {
                expected: expected_parser_epoch,
                actual: self.parser_epoch,
            });
        }
        self.consumed = true;
        Ok(self.parser_epoch)
    }

    #[cfg(test)]
    pub(crate) fn test_only(owner_ticket: RecoveryTicket, parser_epoch: u64) -> Self {
        Self::after_gate_match(owner_ticket, parser_epoch)
    }
}

/// The grant and identity captured from the initially opened, validated
/// device. Construct this while the original descriptor is still live, then
/// move it into recovery after input loss. Re-querying the old descriptor only
/// after disconnect is too late: the kernel may already have invalidated it.
pub(crate) struct LinuxInputGrant {
    path: PathBuf,
    identity: Result<DeviceIdentity, String>,
}

impl LinuxInputGrant {
    pub(crate) fn capture(path: impl Into<PathBuf>, initial_device: &File) -> Self {
        let identity = identity_from_open_evdev(initial_device)
            .map_err(|error| format!("stable evdev unique ID unavailable: {error}"));
        Self {
            path: path.into(),
            identity,
        }
    }

    /// Fixture constructor. Product callers must use `capture` on the
    /// already-opened descriptor; this does not establish physical identity.
    #[cfg(test)]
    pub(crate) fn from_test_identity(path: impl Into<PathBuf>, identity: DeviceIdentity) -> Self {
        Self {
            path: path.into(),
            identity: Ok(identity),
        }
    }

    pub(crate) fn into_recovery<T: Send + 'static>(
        self,
        partial_record_bytes: usize,
        owner: RecoveryOwner<T>,
    ) -> LinuxGrantedReopen<T> {
        let (gate, initial_identity_error) = match self.identity {
            Ok(identity) => match ReconnectGate::authorize(Some(identity)) {
                Ok(mut gate) => match gate.disconnect(partial_record_bytes) {
                    Ok(()) => (Some(gate), None),
                    Err(error) => (
                        None,
                        Some(format!("input state could not be parked: {error:?}")),
                    ),
                },
                Err(error) => (
                    None,
                    Some(format!("initial input identity was rejected: {error:?}")),
                ),
            },
            Err(error) => (None, Some(error)),
        };
        LinuxGrantedReopen {
            path: self.path,
            owner,
            gate,
            initial_identity_error,
        }
    }
}

/// Holds the dirty recovery owner and its original identity gate while
/// attempting explicit reopens of the original granted path. A failed open or
/// identity check reports the reason and leaves the owner/draft here for a
/// later attempt. Dropping this adapter still uses `RecoveryOwner`'s
/// process-local RAM parking contract.
pub(crate) struct LinuxGrantedReopen<T: Send + 'static> {
    path: PathBuf,
    owner: RecoveryOwner<T>,
    gate: Option<ReconnectGate>,
    initial_identity_error: Option<String>,
}

impl<T: Send + 'static> LinuxGrantedReopen<T> {
    pub(crate) fn owner(&self) -> &RecoveryOwner<T> {
        &self.owner
    }

    pub(crate) fn owner_mut(&mut self) -> &mut RecoveryOwner<T> {
        &mut self.owner
    }

    /// Re-arm the retained identity gate after a verified replacement stream
    /// is lost. The owner keeps its draft and last parser epoch; only a fresh
    /// identity match can advance it again.
    pub(crate) fn handle_reconnected_input_loss<H: RecoveryHost<T>>(
        &mut self,
        partial_record_bytes: usize,
        reason: impl Into<String>,
        host: &mut H,
    ) -> Result<(ReconnectedInputLossAction, Option<String>), ReconnectedInputLossError> {
        let phase = self.owner.phase();
        if !matches!(
            phase,
            RecoveryPhase::AwaitingUserChoice
                | RecoveryPhase::NotificationUnavailable
                | RecoveryPhase::ConfirmingDiscard
        ) {
            return Err(ReconnectedInputLossError::InvalidRecoveryPhase(phase));
        }

        let gate = self
            .gate
            .as_mut()
            .ok_or(ReconnectedInputLossError::IdentityGateUnavailable)?;
        let gate_phase = gate.phase();
        if gate_phase != ReconnectPhase::Connected {
            return Err(ReconnectedInputLossError::IdentityGateNotConnected(
                gate_phase,
            ));
        }
        gate.disconnect(partial_record_bytes)
            .map_err(ReconnectedInputLossError::IdentityGateTransition)?;
        self.owner
            .wait_for_input_after_reconnected_loss()
            .map_err(ReconnectedInputLossError::InvalidRecoveryPhase)?;

        let notification_error = self
            .owner
            .recovery()
            .report_reconnect_unavailable(self.path.to_string_lossy().into_owned(), reason, host)
            .map_err(ReconnectedInputLossError::InvalidRecoveryPhase)?;
        Ok((
            ReconnectedInputLossAction::RetryOriginalGrant,
            notification_error,
        ))
    }

    /// Make one reopen attempt. The supplied opener must be the existing
    /// keyboard opener (including its no-follow, device-type, and capability
    /// checks). It receives only the original granted path. This method does
    /// not retry automatically or authorize another path.
    pub(crate) fn try_reopen_once<H, O, E>(
        &mut self,
        open_existing_grant: O,
        host: &mut H,
    ) -> Result<ReopenedGrantedInput, ReopenAttemptError>
    where
        H: RecoveryHost<T>,
        O: FnOnce(&Path) -> Result<File, E>,
        E: Display,
    {
        self.try_reopen_once_with_identity_inner(
            open_existing_grant,
            identity_from_open_evdev,
            host,
        )
    }

    /// Test-only fixture seam for injecting an identity result after the real
    /// opener has returned its handle. This method does not exist in ordinary
    /// builds, so product code cannot bypass the real ioctl check through this
    /// adapter API. Linux syscall negatives use `try_reopen_once`, which always
    /// calls `identity_from_open_evdev`.
    #[cfg(test)]
    pub(crate) fn try_reopen_once_with_identity<H, O, E, I, IE>(
        &mut self,
        open_existing_grant: O,
        identify_opened_handle: I,
        host: &mut H,
    ) -> Result<ReopenedGrantedInput, ReopenAttemptError>
    where
        H: RecoveryHost<T>,
        O: FnOnce(&Path) -> Result<File, E>,
        E: Display,
        I: FnOnce(&File) -> Result<DeviceIdentity, IE>,
        IE: Display,
    {
        self.try_reopen_once_with_identity_inner(open_existing_grant, identify_opened_handle, host)
    }

    fn try_reopen_once_with_identity_inner<H, O, E, I, IE>(
        &mut self,
        open_existing_grant: O,
        identify_opened_handle: I,
        host: &mut H,
    ) -> Result<ReopenedGrantedInput, ReopenAttemptError>
    where
        H: RecoveryHost<T>,
        O: FnOnce(&Path) -> Result<File, E>,
        E: Display,
        I: FnOnce(&File) -> Result<DeviceIdentity, IE>,
        IE: Display,
    {
        if self.owner.phase() != RecoveryPhase::WaitingForInput {
            return Err(ReopenAttemptError::InvalidRecoveryPhase(self.owner.phase()));
        }
        if let Some(reason) = self.initial_identity_error.clone() {
            return Err(self.report_unavailable(reason, host));
        }

        let device = match open_existing_grant(&self.path) {
            Ok(device) => device,
            Err(error) => {
                return Err(self.report_unavailable(
                    format!("existing input grant could not reopen its original path: {error}"),
                    host,
                ));
            }
        };
        let identity = match identify_opened_handle(&device) {
            Ok(identity) => identity,
            Err(error) => {
                return Err(self.report_unavailable(
                    format!("reopened handle has no usable evdev unique ID: {error}"),
                    host,
                ));
            }
        };

        let Some(gate) = self.gate.as_mut() else {
            return Err(self.report_unavailable(
                "the initial evdev identity gate is unavailable".to_owned(),
                host,
            ));
        };
        let candidate = ReconnectCandidate::new(
            self.path.to_string_lossy().into_owned(),
            Some(identity),
            device,
        );
        let accepted = match gate.reconnect(vec![candidate]) {
            Ok(accepted) => accepted,
            Err(ReconnectError::NoMatchingDevice) => {
                return Err(self.report_unavailable(
                    "reopened handle does not match the original evdev unique ID".to_owned(),
                    host,
                ));
            }
            Err(error) => {
                return Err(self.report_unavailable(
                    format!("reconnect gate rejected the reopened handle: {error:?}"),
                    host,
                ));
            }
        };
        let accepted = accepted.start_fresh_stream(LinuxEvdevTarget::X86_64);
        let mut proof =
            VerifiedReconnectProof::after_gate_match(self.owner.ticket(), accepted.parser_epoch);
        let recovery_step = match self.owner.accept_verified_reconnect(&mut proof, host) {
            Ok(step) => step,
            Err(error) => {
                return Err(ReopenAttemptError::InvalidReconnectProof(error));
            }
        };
        Ok(ReopenedGrantedInput {
            stream: accepted.stream,
            path: PathBuf::from(accepted.node_path),
            parser_epoch: accepted.parser_epoch,
            discarded_partial_record_bytes: accepted.discarded_partial_record_bytes,
            recovery_step,
        })
    }

    fn report_unavailable<H: RecoveryHost<T>>(
        &mut self,
        reason: String,
        host: &mut H,
    ) -> ReopenAttemptError {
        match self.owner.recovery().report_reconnect_unavailable(
            self.path.to_string_lossy().into_owned(),
            reason.clone(),
            host,
        ) {
            Ok(notification_error) => ReopenAttemptError::Unavailable {
                reason,
                notification_error,
            },
            Err(phase) => ReopenAttemptError::InvalidRecoveryPhase(phase),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_self_owned_fd_tests {
    use super::{
        identity_from_open_evdev, LinuxGrantedReopen, LinuxInputGrant, ReopenAttemptError,
        UniqueIdentityError, VerifiedReconnectProof,
    };
    use crate::input_failure_guard::InputFailureAction;
    use crate::input_reconnect::DeviceIdentity;
    use crate::input_recovery::{
        RecoveryHost, RecoveryNotice, RecoveryOwner, RecoveryPhase, RecoveryStart,
    };
    use crate::recovery_ui_commands::{RecoveryUiCommand, RecoveryUiCommandAdapter};
    use linux_evdev_stream::{EvdevStream, LinuxEvdevTarget, StreamError, StreamItem};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::fd::FromRawFd;
    use std::os::raw::c_int;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct NoticeHost {
        notices: usize,
    }

    impl RecoveryHost<String> for NoticeHost {
        type SaveError = &'static str;
        type NotificationError = &'static str;

        fn retry_save(&mut self, _draft: &mut String) -> Result<(), Self::SaveError> {
            Ok(())
        }

        fn notify(&mut self, _notice: RecoveryNotice) -> Result<(), Self::NotificationError> {
            self.notices += 1;
            Ok(())
        }
    }

    fn dirty_owner() -> RecoveryOwner<String> {
        let RecoveryStart::Held(owner) =
            RecoveryOwner::from_input_failure(InputFailureAction::HoldUnsavedDraft {
                owner: "linux syscall fixture draft".to_owned(),
                input_error: "fixture input closed",
                save_error: "fixture final flush failed".to_owned(),
                notification_error: None,
            })
        else {
            panic!("fixture must retain its unresolved draft");
        };
        owner
    }

    fn explicitly_discard(adapter: &mut LinuxGrantedReopen<String>, host: &mut NoticeHost) {
        assert_eq!(adapter.owner().phase(), RecoveryPhase::WaitingForInput);
        let mut proof = VerifiedReconnectProof::test_only(adapter.owner().ticket(), 1);
        adapter
            .owner_mut()
            .accept_verified_reconnect(&mut proof, host)
            .expect("test-only proof advances this fixture owner");
        let mut ui = RecoveryUiCommandAdapter::borrow(adapter.owner_mut());
        ui.apply(RecoveryUiCommand::BeginDiscard, host)
            .expect("explicit begin-discard command");
        ui.apply(RecoveryUiCommand::ConfirmDiscard, host)
            .expect("explicit confirmation releases the fixture draft");
        drop(ui);
        assert!(adapter.owner().is_resolved());
    }

    static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

    fn owned_temp_file() -> (PathBuf, std::fs::File) {
        let path = std::env::temp_dir().join(format!(
            "nagi-task28-owned-fd-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .expect("create this test's private regular file");
        (path, file)
    }

    fn assert_linux_enotty(file: &std::fs::File, kind: &str) {
        match identity_from_open_evdev(file) {
            Err(UniqueIdentityError::Io {
                raw_os_error: Some(25),
                ..
            }) => eprintln!("TASK28_LINUX_IOCTL_NEGATIVE kind={kind} errno=25 expected=ENOTTY"),
            other => panic!("expected Linux ENOTTY from self-owned {kind}, got {other:?}"),
        }
    }

    #[test]
    fn linux_ev_iocguniq_on_self_owned_regular_file_returns_enotty() {
        let (path, file) = owned_temp_file();
        fs::remove_file(path).expect("unlink only this test's file");
        assert_linux_enotty(&file, "regular-file");
    }

    #[test]
    fn linux_ev_iocguniq_on_self_owned_pipe_returns_enotty() {
        unsafe extern "C" {
            fn pipe(fds: *mut c_int) -> c_int;
        }
        let mut descriptors = [-1; 2];
        let result = unsafe { pipe(descriptors.as_mut_ptr()) };
        assert_eq!(result, 0, "create a private pipe for this test");
        let reader = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
        let writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        assert_linux_enotty(&reader, "pipe");
        drop(writer);
    }

    #[test]
    fn linux_adapter_opens_only_self_owned_regular_fd_then_actual_ioctl_fails_enotty() {
        let (path, _initial_file) = owned_temp_file();
        let identity = DeviceIdentity::from_test_identity("fixture:startup-id-only")
            .expect("fixture startup identity is opaque");
        let grant = LinuxInputGrant::from_test_identity(path.clone(), identity);
        let mut adapter = grant.into_recovery(0, dirty_owner());
        let mut opener_paths = Vec::new();
        let mut host = NoticeHost { notices: 0 };

        let error = match adapter.try_reopen_once(
            |original_path| {
                opener_paths.push(original_path.to_path_buf());
                std::fs::File::open(original_path)
            },
            &mut host,
        ) {
            Err(error) => error,
            Ok(_) => panic!("a self-owned regular file cannot pass evdev identity ioctl"),
        };

        assert_eq!(opener_paths, [path.clone()]);
        assert!(matches!(
            error,
            ReopenAttemptError::Unavailable { ref reason, .. }
                if reason.contains("reopened handle has no usable evdev unique ID")
                    && reason.contains("os error 25")
        ));
        assert_eq!(host.notices, 1);
        assert_eq!(
            adapter.owner().draft().map(String::as_str),
            Some("linux syscall fixture draft")
        );
        eprintln!("TASK28_LINUX_ACTUAL_REOPEN opener_calls=1 fd=self-owned-regular-file ioctl_errno=25 outcome=held");
        explicitly_discard(&mut adapter, &mut host);
        drop(adapter);
        std::fs::remove_file(path).expect("remove only this test's private file");
    }

    #[test]
    fn linux_self_owned_pipe_eof_and_short_record_are_reported_without_device_claims() {
        unsafe extern "C" {
            fn pipe(fds: *mut c_int) -> c_int;
        }
        let mut descriptors = [-1; 2];
        let result = unsafe { pipe(descriptors.as_mut_ptr()) };
        assert_eq!(result, 0, "create a private pipe for this test");
        let reader = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
        let mut writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        let short_record = [0u8; 7];
        writer
            .write_all(&short_record)
            .expect("write to this test's pipe");
        drop(writer);

        let mut stream = EvdevStream::new(reader, LinuxEvdevTarget::X86_64);
        assert!(matches!(
            stream.next_item(),
            Err(StreamError::TruncatedRecord {
                expected: 24,
                received: 7
            })
        ));
        eprintln!("TASK28_LINUX_PIPE_NEGATIVE kind=short-record expected=24 received=7");

        let mut eof_descriptors = [-1; 2];
        let eof_pipe_result = unsafe { pipe(eof_descriptors.as_mut_ptr()) };
        assert_eq!(
            eof_pipe_result, 0,
            "create a private EOF pipe for this test"
        );
        let eof_reader = unsafe { std::fs::File::from_raw_fd(eof_descriptors[0]) };
        let eof_writer = unsafe { std::fs::File::from_raw_fd(eof_descriptors[1]) };
        drop(eof_writer);
        let mut eof_stream = EvdevStream::new(eof_reader, LinuxEvdevTarget::X86_64);
        assert_eq!(eof_stream.next_item(), Ok(StreamItem::Eof));
        eprintln!("TASK28_LINUX_PIPE_NEGATIVE kind=closed-writer-eof expected=StreamItem::Eof");
    }

    #[test]
    fn linux_reopen_of_removed_self_owned_path_fails_without_device_access() {
        let (path, file) = owned_temp_file();
        fs::remove_file(&path).expect("unlink only this test's file");
        drop(file);
        let result = std::fs::File::open(path);
        assert!(matches!(
            result,
            Err(ref error) if error.kind() == std::io::ErrorKind::NotFound
        ));
        eprintln!("TASK28_LINUX_REOPEN_NEGATIVE kind=removed-self-owned-path errno=ENOENT");
    }
}
