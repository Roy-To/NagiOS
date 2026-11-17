//! Source-connected Shell loop candidate. It reuses the task28 real
//! `EvdevStream` consumer/state machine and delegates launch/auth/reap to a
//! narrow Task41/Task24 adapter owned by the boot caller.

use crate::existing_shell_consumer as shell;
use crate::notes_bootstrap::{
    channel_pair, ChildNotesChannels, NotesBootstrap, ParentNotesChannels, SurfaceId,
};
use crate::notes_child::{
    read_frame, receive_close_commit, send_presented_ack, NotesChild, NotesChildError,
    NotesCloseCommitted, PixelFrame,
};
use crate::shell_surface::{
    DisplayError, FramebufferPresenter, PresentedFocus, ShellSurfaceOwner, Surface, SurfaceFrame,
    ValidatedDisplayLayout,
};
use crate::terminal_result::{
    ExactProcessProof, ExactTerminalResult, ExecGateDisposition, ExitStatus, TerminalDecision,
    TerminalIdentityError,
};
use nagi_init_spawn_contract::SpawnedProcessPidfd;
use nagi_model::ObjectId;
use nagi_notes::NoteRevision;
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

pub type ProcessRecord = shell::ProcessRecord;

/// The boot owner must launch through Task41's existing authorization and
/// retain Task24's opaque proof from that exact spawn. Close polling must map
/// `query_terminal_result(proof, record)`; PID-only and aggregate wait events
/// cannot implement this interface.
///
/// Production implementation belongs beside those crate-private PID1 owners,
/// not in this separate candidate crate. It must use the real current
/// `libnagi::security::Session`, package/inventory/grant suppliers, and the
/// exact Task24 proof/record from the same spawn. `ChildNotesChannels` are
/// currently local candidate endpoints: the test-only NGB1/SCM_RIGHTS helpers
/// are not a production bootstrap, and Task41's NIPC stream must not be used
/// as a raw descriptor frame. Until a source-owned Notes control/bootstrap
/// route carries this request and the channels, the production port stays
/// `UnavailableTaskAdapter`. Every launch and forwarded input must remain
/// bound to the actual interactive owner. The current NotesChild's direct
/// NoteStore-backed bridge is only a local candidate, not the C2 typed NIPC
/// client; StorageProvider/OwnedFd stays inside the broker and its adapters.
/// Each Notes operation must pass through the broker's live grant check.
/// The current C1 adapter is
/// `EndOwner<'caller, 'receiver, U>`: it privately binds an `EndDelivery` to
/// the exact recipient before SDK end and calls E only through `deliver`.
/// The C1 delivery path, not an E-local Session/Profile pair, rejects stale
/// recipient generations. E's delivery closure maps C1's transition to the
/// local UI kind; that kind alone is never authority. `P` may borrow its real
/// C1/C2 context for the required lifetime and does not require raw IDs, a
/// fixture authority, or `'static`. The boot adapter must use C2's existing
/// Stage24 SessionLease/PeerDenial and Send + Sync contract; E must not
/// redeclare or synthesize it. C1 delivery is in-process only: there is no
/// durable outbox, expiry/recovery owner, or crash-safe delivery guarantee.
pub(super) trait Task41Task24Adapter {
    type ProcessProof: ExactProcessProof;
    /// The E UI kind selected by the boot adapter; never a C1 authority token.
    type SessionEndUiKind;

    fn launch_notes_child(
        &mut self,
        request_id: NonZeroU64,
        note_id: ObjectId,
        surface_id: SurfaceId,
        channels: ChildNotesChannels,
    ) -> Result<(ProcessRecord, Self::ProcessProof), shell::SupervisorError>;

    fn query_terminal_result(
        &mut self,
        proof: &Self::ProcessProof,
        record: ProcessRecord,
    ) -> Result<ExactTerminalResult, shell::SupervisorError>;
}

/// Fail-closed port while the boot integration lacks Task41 launch inputs or
/// Task24 exact-record reap access.
pub(super) struct UnavailableTaskAdapter;

impl Task41Task24Adapter for UnavailableTaskAdapter {
    type ProcessProof = SpawnedProcessPidfd;
    type SessionEndUiKind = ();

    fn launch_notes_child(
        &mut self,
        _request_id: NonZeroU64,
        _note_id: ObjectId,
        _surface_id: SurfaceId,
        _channels: ChildNotesChannels,
    ) -> Result<(ProcessRecord, Self::ProcessProof), shell::SupervisorError> {
        Err(shell::SupervisorError::Unavailable)
    }

    fn query_terminal_result(
        &mut self,
        _proof: &Self::ProcessProof,
        _record: ProcessRecord,
    ) -> Result<ExactTerminalResult, shell::SupervisorError> {
        Err(shell::SupervisorError::Unavailable)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActiveNotes {
    pub(super) request_id: NonZeroU64,
    pub(super) note_id: ObjectId,
    pub(super) surface_id: SurfaceId,
    pub(super) record: ProcessRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ShellLoopObservation {
    Idle,
    SessionEndNoticeApplied,
    SessionEndNoticeIgnored,
    ActionIgnoredAfterSessionEnd,
    NotesLaunchRequested {
        request_id: u64,
        note_id: ObjectId,
    },
    NotesLaunchUnavailable {
        request_id: u64,
        error: shell::SupervisorError,
    },
    ActionRejected(shell::ControlError),
    NotesStarted {
        request_id: u64,
        record: ProcessRecord,
    },
    NotesInputForwarded {
        request_id: u64,
    },
    FocusCommitted(PresentedFocus),
    NotesFramePresented {
        request_id: u64,
        record: ProcessRecord,
        note_id: ObjectId,
        revision: u64,
        surface_id: SurfaceId,
        first_frame: bool,
    },
    NotesCloseCommitted(NotesCloseCommitted),
    NotesCloseAwaitingFrame {
        request_id: u64,
        expected_revision: NoteRevision,
        observed_revision: Option<NoteRevision>,
    },
    WaitingForExactTerminal {
        record: ProcessRecord,
        gate_disposition: Option<ExecGateDisposition>,
    },
    ReapedAwaitingCleanup {
        record: ProcessRecord,
        status: ExitStatus,
        gate_disposition: Option<ExecGateDisposition>,
    },
    TerminalStateUnavailable {
        record: ProcessRecord,
        error: shell::SupervisorError,
    },
    TerminalIdentityRejected {
        record: ProcessRecord,
        error: TerminalIdentityError,
    },
    TerminalStateUnknown {
        record: ProcessRecord,
        gate_disposition: Option<ExecGateDisposition>,
    },
    TerminalExecGateUncertain {
        record: ProcessRecord,
        gate_disposition: Option<ExecGateDisposition>,
    },
    NotesReaped {
        request_id: u64,
        record: ProcessRecord,
        exit_code: Option<i32>,
        signal: Option<i32>,
        gate_disposition: ExecGateDisposition,
    },
    InputUnavailable(shell::InputLost),
    DisplayUnavailable(DisplayError),
    FilesProviderUnavailable,
}

/// Synchronous local result. This does not mean C3 reclaimed child permissions
/// or that an acknowledgment was durably stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalSessionEndReceipt {
    Applied,
    AlreadyApplied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildRevocationState {
    NotNeeded,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ShellLoopError {
    Io(std::io::ErrorKind),
    Display(DisplayError),
    Child(NotesChildError),
    Supervisor(shell::SupervisorError),
    Control(shell::ControlError),
    Protocol,
    RequestSequenceExhausted,
}

impl From<std::io::Error> for ShellLoopError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.kind())
    }
}

impl From<DisplayError> for ShellLoopError {
    fn from(error: DisplayError) -> Self {
        Self::Display(error)
    }
}

impl From<NotesChildError> for ShellLoopError {
    fn from(error: NotesChildError) -> Self {
        Self::Child(error)
    }
}

impl From<shell::ControlError> for ShellLoopError {
    fn from(error: shell::ControlError) -> Self {
        Self::Control(error)
    }
}

struct ParentChild<Proof> {
    channels: ParentNotesChannels,
    active: ActiveNotes,
    proof: Proof,
    latest_frame: Option<PixelFrame>,
    pending_close: Option<NotesCloseCommitted>,
    closing: bool,
    first_presentation_acked: bool,
}

/// A one-shot bridge from a Task24 exact, fully-cleaned terminal result into
/// the legacy ShellController state transition. It is constructed only after
/// `query_terminal_result(proof, record)` has passed opaque-instance, full
/// LaunchRecord, Complete, and Released-gate checks. It is never a PID poll.
struct ExactOneShotReap {
    expected: ProcessRecord,
    status: ExitStatus,
    consumed: bool,
}

impl shell::SupervisorPort for ExactOneShotReap {
    fn launch_notes(
        &mut self,
        _note_id: nagi_notes::NoteId,
    ) -> Result<ProcessRecord, shell::SupervisorError> {
        Err(shell::SupervisorError::Unavailable)
    }

    fn forward_input(
        &mut self,
        _record: ProcessRecord,
        _event: nagi_abi::InputEvent,
    ) -> Result<(), shell::SupervisorError> {
        Err(shell::SupervisorError::Unavailable)
    }

    fn poll_reap(
        &mut self,
        record: ProcessRecord,
    ) -> Result<shell::ReapObservation, shell::SupervisorError> {
        if self.consumed || record != self.expected {
            return Err(shell::SupervisorError::ProtocolViolation);
        }
        self.consumed = true;
        // The legacy transition type has u8 status fields. Preserve the
        // original i32 status in the Task24 result/observation; this narrow
        // projection is consumed only to clear the legacy Closing state.
        Ok(shell::ReapObservation::Reaped {
            record: self.expected,
            exit_code: self
                .status
                .exit_code
                .and_then(|value| u8::try_from(value).ok()),
            signal: self
                .status
                .signal
                .and_then(|value| u8::try_from(value).ok()),
        })
    }
}

/// One normal-UI owner. It owns the actual evdev parser, the only physical
/// framebuffer writer, logical focus, Files surface state, and the current
/// child channels/LaunchRecord. `R` is moved in from the already-authorized
/// boot input opener; no device is opened or grant inferred here.
pub(super) struct ShellLoop<R, P: Task41Task24Adapter> {
    input_fd: i32,
    runtime: shell::ShellRuntime<R>,
    surface_owner: ShellSurfaceOwner,
    supervisor: P,
    note_id: ObjectId,
    shell_surface_id: SurfaceId,
    notes_surface_id: SurfaceId,
    next_wire_request: u64,
    active: Option<ParentChild<P::ProcessProof>>,
    input_lost: Option<shell::InputLost>,
    display_failed: Option<DisplayError>,
    awaiting_initial_frame: bool,
    session_ended: bool,
    child_revocation: ChildRevocationState,
}

impl<R, P> ShellLoop<R, P>
where
    R: Read + AsRawFd,
    P: Task41Task24Adapter,
{
    pub(super) fn new(
        input: R,
        input_target: linux_evdev_stream::LinuxEvdevTarget,
        note_id: ObjectId,
        display: FramebufferPresenter,
        shell_surface_id: SurfaceId,
        notes_surface_id: SurfaceId,
        files_surface_id: SurfaceId,
        supervisor: P,
    ) -> Result<(Self, PresentedFocus), ShellLoopError> {
        let input_fd = input.as_raw_fd();
        set_nonblocking(input_fd)?;
        let (surface_owner, initial) = ShellSurfaceOwner::new(
            display,
            shell_surface_id,
            notes_surface_id,
            files_surface_id,
        )?;
        Ok((
            Self {
                input_fd,
                runtime: shell::ShellRuntime::new(input, input_target, note_id),
                surface_owner,
                supervisor,
                note_id,
                shell_surface_id,
                notes_surface_id,
                next_wire_request: 0,
                active: None,
                input_lost: None,
                display_failed: None,
                awaiting_initial_frame: false,
                session_ended: false,
                child_revocation: ChildRevocationState::NotNeeded,
            },
            initial,
        ))
    }

    pub(super) fn active_notes(&self) -> Option<&ActiveNotes> {
        self.active.as_ref().map(|active| &active.active)
    }

    pub(super) fn focused_surface(&self) -> Surface {
        self.surface_owner.focused()
    }

    /// Apply E's mapped kind only from C1 `EndOwner::deliver`'s validated
    /// receiver closure. A kind alone is never proof. This local gate hides the
    /// child surface and stops input/actions, but does not reclaim child
    /// authority. C3 must select the exact child from the real LaunchRecord and
    /// confirm revocation/reap. Dirty Notes is neither flushed, discarded, nor
    /// replayed here.
    pub(super) fn apply_delivered_session_end(
        &mut self,
        _kind: P::SessionEndUiKind,
    ) -> (LocalSessionEndReceipt, Vec<ShellLoopObservation>) {
        let mut observations = Vec::new();
        if self.session_ended {
            observations.push(ShellLoopObservation::SessionEndNoticeIgnored);
            return (LocalSessionEndReceipt::AlreadyApplied, observations);
        }

        self.session_ended = true;
        self.child_revocation = if self.active.is_some() {
            ChildRevocationState::Unconfirmed
        } else {
            ChildRevocationState::NotNeeded
        };
        observations.push(ShellLoopObservation::SessionEndNoticeApplied);
        // Hide any application surface after invalidation. This does not
        // claim a complete lock/login UI; it only restores the Shell surface.
        let _ = self.try_present_focus(Surface::Shell, None, &mut observations);
        (LocalSessionEndReceipt::Applied, observations)
    }

    /// E delivery does not prove child authority or resources were reclaimed.
    /// An attached child remains explicitly unconfirmed until C3 returns a
    /// real cleanup acknowledgment.
    pub(super) fn child_revocation_state(&self) -> ChildRevocationState {
        self.child_revocation
    }

    /// One bounded Linux `poll(2)` turn. When Closing, it invokes the inherited
    /// exact-record reap adapter on every iteration, including turns with no
    /// keyboard input. Input loss disables only that FD; it does not drop the
    /// Shell owner or current Notes channel/draft.
    pub(super) fn run_once(
        &mut self,
        timeout_ms: i32,
    ) -> Result<Vec<ShellLoopObservation>, ShellLoopError> {
        let mut observations = Vec::new();
        let mut pollfds = Vec::with_capacity(3);
        let input_index = if !self.session_ended
            && self.input_lost.is_none()
            && self.display_failed.is_none()
            && !self.awaiting_initial_frame
            && !self
                .active
                .as_ref()
                .is_some_and(|active| active.pending_close.is_some() || active.closing)
        {
            let index = pollfds.len();
            pollfds.push(libc::pollfd {
                fd: self.input_fd,
                events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                revents: 0,
            });
            Some(index)
        } else {
            None
        };
        let child_input_index = self.active.as_ref().map(|child| {
            let index = pollfds.len();
            pollfds.push(libc::pollfd {
                fd: child.channels.input.as_raw_fd(),
                events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                revents: 0,
            });
            index
        });
        let child_display_index = if self.session_ended {
            None
        } else {
            self.active.as_ref().map(|child| {
                let index = pollfds.len();
                pollfds.push(libc::pollfd {
                    fd: child.channels.display.as_raw_fd(),
                    events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
                    revents: 0,
                });
                index
            })
        };
        let ready = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as _, timeout_ms) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }

        if child_display_index.is_some_and(|index| pollfds[index].revents & libc::POLLIN != 0) {
            self.receive_notes_frame(&mut observations)?;
        }
        if child_input_index.is_some_and(|index| pollfds[index].revents & libc::POLLIN != 0) {
            self.receive_child_control(&mut observations)?;
        }
        let child_close_in_progress = self
            .active
            .as_ref()
            .is_some_and(|active| active.pending_close.is_some() || active.closing);
        if let Some(index) = input_index.filter(|_| !child_close_in_progress) {
            let revents = pollfds[index].revents;
            if revents & libc::POLLIN != 0 {
                match self.runtime.step() {
                    Ok(shell::RuntimeStep::Idle) => {}
                    Ok(shell::RuntimeStep::Input(Ok(action))) => {
                        self.handle_shell_action(action, &mut observations)?;
                    }
                    Ok(shell::RuntimeStep::Input(Err(error))) => {
                        observations.push(ShellLoopObservation::ActionRejected(error));
                    }
                    Err(input_lost) => {
                        self.input_lost = Some(input_lost);
                        observations.push(ShellLoopObservation::InputUnavailable(input_lost));
                    }
                }
            } else if revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                match self.runtime.step() {
                    Err(input_lost) => {
                        self.input_lost = Some(input_lost);
                        observations.push(ShellLoopObservation::InputUnavailable(input_lost));
                    }
                    Ok(_) => {
                        let input_lost = shell::InputLost::Eof;
                        self.input_lost = Some(input_lost);
                        observations.push(ShellLoopObservation::InputUnavailable(input_lost));
                    }
                }
            }
        }

        if self.active.as_ref().is_some_and(|active| active.closing) {
            let record = self
                .active
                .as_ref()
                .expect("closing implies child")
                .active
                .record;
            let terminal_result = {
                let active = self.active.as_ref().expect("closing implies child");
                self.supervisor
                    .query_terminal_result(&active.proof, active.active.record)
            };
            match terminal_result {
                Err(error) => {
                    observations
                        .push(ShellLoopObservation::TerminalStateUnavailable { record, error });
                }
                Ok(terminal) => {
                    let gate_disposition = terminal.gate_disposition;
                    let decision = {
                        let active = self.active.as_ref().expect("closing implies child");
                        terminal.classify(&active.proof, record)
                    };
                    match decision {
                        Err(error) => {
                            observations.push(ShellLoopObservation::TerminalIdentityRejected {
                                record,
                                error,
                            });
                        }
                        Ok(TerminalDecision::Pending) => {
                            observations.push(ShellLoopObservation::WaitingForExactTerminal {
                                record,
                                gate_disposition,
                            });
                        }
                        Ok(TerminalDecision::ReapedCleanupPending(status)) => {
                            observations.push(ShellLoopObservation::ReapedAwaitingCleanup {
                                record,
                                status,
                                gate_disposition,
                            });
                        }
                        Ok(TerminalDecision::Unknown) => {
                            observations.push(ShellLoopObservation::TerminalStateUnknown {
                                record,
                                gate_disposition,
                            });
                        }
                        Ok(TerminalDecision::ExecGateNotReleased) => {
                            observations.push(ShellLoopObservation::TerminalExecGateUncertain {
                                record,
                                gate_disposition,
                            });
                        }
                        Ok(TerminalDecision::Complete(status)) => {
                            let mut exact_reap = ExactOneShotReap {
                                expected: record,
                                status,
                                consumed: false,
                            };
                            match self.runtime.poll_close(&mut exact_reap)? {
                                shell::DispatchOutcome::NotesReaped {
                                    record: completed, ..
                                } if completed == record => {}
                                _ => return Err(ShellLoopError::Protocol),
                            }
                            if !exact_reap.consumed {
                                return Err(ShellLoopError::Protocol);
                            }
                            let active =
                                self.active.take().expect("terminal complete implies child");
                            observations.push(ShellLoopObservation::NotesReaped {
                                request_id: active.active.request_id.get(),
                                record,
                                exit_code: status.exit_code,
                                signal: status.signal,
                                gate_disposition: ExecGateDisposition::Released,
                            });
                            let _ = self.try_present_focus(Surface::Shell, None, &mut observations);
                        }
                    }
                }
            }
        }

        if observations.is_empty() {
            observations.push(ShellLoopObservation::Idle);
        }
        Ok(observations)
    }

    fn handle_shell_action(
        &mut self,
        action: shell::ShellAction,
        observations: &mut Vec<ShellLoopObservation>,
    ) -> Result<(), ShellLoopError> {
        if self.session_ended {
            observations.push(ShellLoopObservation::ActionIgnoredAfterSessionEnd);
            return Ok(());
        }
        match action {
            shell::ShellAction::Ignored => {}
            shell::ShellAction::FilesUnavailable => {
                observations.push(ShellLoopObservation::FilesProviderUnavailable);
            }
            shell::ShellAction::LaunchNotes {
                request_id: inner_id,
                note_id,
            } => {
                let request_value = self
                    .next_wire_request
                    .checked_add(1)
                    .ok_or(ShellLoopError::RequestSequenceExhausted)?;
                let request_id = NonZeroU64::new(request_value)
                    .ok_or(ShellLoopError::RequestSequenceExhausted)?;
                self.next_wire_request = request_value;
                observations.push(ShellLoopObservation::NotesLaunchRequested {
                    request_id: request_value,
                    note_id,
                });
                let (parent, child) = channel_pair()?;
                let launch = self.supervisor.launch_notes_child(
                    request_id,
                    note_id,
                    self.notes_surface_id,
                    child,
                );
                match launch {
                    Ok((record, proof)) => match self.runtime.complete_launch(inner_id, Ok(record))
                    {
                        Ok(shell::DispatchOutcome::NotesStarted(verified))
                            if verified == record =>
                        {
                            self.active = Some(ParentChild {
                                channels: parent,
                                active: ActiveNotes {
                                    request_id,
                                    note_id,
                                    surface_id: self.notes_surface_id,
                                    record,
                                },
                                proof,
                                latest_frame: None,
                                pending_close: None,
                                closing: false,
                                first_presentation_acked: false,
                            });
                            self.awaiting_initial_frame = true;
                            observations.push(ShellLoopObservation::NotesStarted {
                                request_id: request_value,
                                record,
                            });
                        }
                        Ok(shell::DispatchOutcome::NotesLaunchRejected(error)) => {
                            let _ = parent;
                            observations.push(ShellLoopObservation::NotesLaunchUnavailable {
                                request_id: request_value,
                                error,
                            });
                        }
                        _ => return Err(ShellLoopError::Protocol),
                    },
                    Err(error) => {
                        drop(parent);
                        self.runtime
                            .complete_launch(inner_id, Err(error))
                            .map_err(|_| ShellLoopError::Protocol)?;
                        observations.push(ShellLoopObservation::NotesLaunchUnavailable {
                            request_id: request_value,
                            error,
                        });
                    }
                }
            }
            shell::ShellAction::ForwardNotesInput { record, event } => {
                let Some(active) = self.active.as_ref() else {
                    return Err(ShellLoopError::Protocol);
                };
                if active.active.record != record
                    || active.closing
                    || self.surface_owner.focused() != Surface::Notes
                    || self.surface_owner.input_owner() != active.active.surface_id
                {
                    return Err(ShellLoopError::Protocol);
                }
                let bytes = crate::notes_child::encode_input(event);
                if active.channels.input.send(&bytes)? != bytes.len() {
                    return Err(ShellLoopError::Protocol);
                }
                observations.push(ShellLoopObservation::NotesInputForwarded {
                    request_id: active.active.request_id.get(),
                });
            }
            shell::ShellAction::FocusChanged(focus) => {
                let requested = match focus {
                    shell::Surface::Shell => Surface::Shell,
                    shell::Surface::Notes => Surface::Notes,
                    shell::Surface::Files => Surface::Files,
                };
                let notes_frame = self
                    .active
                    .as_ref()
                    .and_then(|active| active.latest_frame.as_ref())
                    .map(|frame| SurfaceFrame {
                        surface_id: frame.surface_id,
                        pixels: frame.pixels.clone(),
                    });
                let Some(committed) =
                    self.try_present_focus(requested, notes_frame.as_ref(), observations)
                else {
                    return Ok(());
                };
                if requested == Surface::Notes {
                    if let Some(active) = self.active.as_mut() {
                        if !active.first_presentation_acked {
                            send_presented_ack(
                                &mut active.channels.display,
                                active.active.request_id,
                                active.active.surface_id,
                                active
                                    .latest_frame
                                    .as_ref()
                                    .ok_or(ShellLoopError::Protocol)?
                                    .revision,
                            )?;
                            active.first_presentation_acked = true;
                        }
                    }
                }
                observations.push(ShellLoopObservation::FocusCommitted(committed));
            }
        }
        Ok(())
    }

    fn receive_notes_frame(
        &mut self,
        observations: &mut Vec<ShellLoopObservation>,
    ) -> Result<(), ShellLoopError> {
        if self.session_ended {
            return Ok(());
        }
        let should_present = self.display_failed.is_none()
            && self.runtime.controller().focus() == shell::Surface::Notes;
        let (frame, request_id, record, note_id, surface_id, first_frame) = {
            let Some(active) = self.active.as_mut() else {
                return Err(ShellLoopError::Protocol);
            };
            let frame = read_frame(&mut active.channels.display, active.active.surface_id)?;
            if frame.note_id != active.active.note_id {
                return Err(ShellLoopError::Protocol);
            }
            active.latest_frame = Some(frame.clone());
            (
                frame,
                active.active.request_id,
                active.active.record,
                active.active.note_id,
                active.active.surface_id,
                !active.first_presentation_acked,
            )
        };
        if should_present {
            let surface_frame = SurfaceFrame {
                surface_id: frame.surface_id,
                pixels: frame.pixels.clone(),
            };
            if let Some(committed) =
                self.try_present_focus(Surface::Notes, Some(&surface_frame), observations)
            {
                if first_frame {
                    let active = self.active.as_mut().ok_or(ShellLoopError::Protocol)?;
                    send_presented_ack(
                        &mut active.channels.display,
                        request_id,
                        surface_id,
                        frame.revision,
                    )?;
                    active.first_presentation_acked = true;
                    self.awaiting_initial_frame = false;
                }
                observations.push(ShellLoopObservation::NotesFramePresented {
                    request_id: request_id.get(),
                    record,
                    note_id,
                    revision: frame.revision,
                    surface_id,
                    first_frame,
                });
                observations.push(ShellLoopObservation::FocusCommitted(committed));
            }
        }
        self.commit_pending_close_when_frame_matches(observations)?;
        Ok(())
    }

    fn receive_child_control(
        &mut self,
        observations: &mut Vec<ShellLoopObservation>,
    ) -> Result<(), ShellLoopError> {
        let Some(active) = self.active.as_mut() else {
            return Err(ShellLoopError::Protocol);
        };
        let close = receive_close_commit(&active.channels.input)?;
        if close.request_id != active.active.request_id
            || close.surface_id != active.active.surface_id
            || close.note_id != active.active.note_id
            || close.revision == 0
        {
            return Err(ShellLoopError::Protocol);
        }
        if active
            .pending_close
            .as_ref()
            .is_some_and(|pending| pending != &close)
        {
            return Err(ShellLoopError::Protocol);
        }
        active.pending_close = Some(close);
        self.commit_pending_close_when_frame_matches(observations)
    }

    /// A Notes close result only becomes an OS close request after Shell has
    /// received the same canonical NoteId and revision in that surface's last
    /// frame. A save result by itself never releases the child owner.
    fn commit_pending_close_when_frame_matches(
        &mut self,
        observations: &mut Vec<ShellLoopObservation>,
    ) -> Result<(), ShellLoopError> {
        let Some(active) = self.active.as_ref() else {
            return Err(ShellLoopError::Protocol);
        };
        let Some(close) = active.pending_close.clone() else {
            return Ok(());
        };
        let matching_revision = active.latest_frame.as_ref().is_some_and(|frame| {
            frame.surface_id == close.surface_id
                && frame.note_id == close.note_id
                && frame.revision == close.revision
        });
        if !matching_revision {
            observations.push(ShellLoopObservation::NotesCloseAwaitingFrame {
                request_id: close.request_id.get(),
                expected_revision: close.revision,
                observed_revision: active.latest_frame.as_ref().map(|frame| frame.revision),
            });
            return Ok(());
        }

        let record = active.active.record;
        match self.runtime.notes_close_requested(record) {
            Ok(shell::DispatchOutcome::NotesCloseRequested) => {
                let active = self.active.as_mut().ok_or(ShellLoopError::Protocol)?;
                active.pending_close = None;
                active.closing = true;
                observations.push(ShellLoopObservation::NotesCloseCommitted(close));
                if !self.session_ended {
                    if let Some(focused) =
                        self.try_present_focus(Surface::Files, None, observations)
                    {
                        observations.push(ShellLoopObservation::FocusCommitted(focused));
                    }
                }
            }
            _ => return Err(ShellLoopError::Protocol),
        }
        Ok(())
    }

    fn try_present_focus(
        &mut self,
        requested: Surface,
        notes: Option<&SurfaceFrame>,
        observations: &mut Vec<ShellLoopObservation>,
    ) -> Option<PresentedFocus> {
        if self.display_failed.is_some() {
            return None;
        }
        match self.surface_owner.present_focus(requested, notes) {
            Ok(committed) => Some(committed),
            Err(error) => {
                self.display_failed = Some(error);
                observations.push(ShellLoopObservation::DisplayUnavailable(error));
                None
            }
        }
    }
}

fn set_nonblocking(fd: i32) -> Result<(), ShellLoopError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes_bootstrap::ParentNotesChannels;
    use crate::notes_child::{encode_input, NotesChild};
    use crate::notes_child::{FRAME_HEIGHT, FRAME_WIDTH, PIXEL_COUNT};
    use crate::shell_surface::{PixelFormat, ValidatedDisplayLayout};
    use crate::terminal_result::{ExactTerminalState, ExecGateDisposition, ExitStatus};
    use nagi_abi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ESCAPE, INPUT_KEY_TAB};
    use nagi_init_spawn_contract::SpawnedProcessIdentity;
    use nagi_model::{AppSessionId, NodeId};
    use nagi_notes::APP_ID;
    use notes_event_bridge_candidate::NotesEventBridge;
    use notes_files_codec_candidate::{InMemoryNoteStore, NoteDocument, NoteStore, Timestamp};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct SharedLaunch(Mutex<Option<(NonZeroU64, ObjectId, SurfaceId, ChildNotesChannels)>>);

    struct FakeTaskAdapter {
        shared: Arc<SharedLaunch>,
        pending_terminal: bool,
        poll_count: Arc<Mutex<usize>>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestSessionEndUiKind {
        Lock,
        Logout,
    }

    struct FakeProof(SpawnedProcessIdentity);

    impl ExactProcessProof for FakeProof {
        fn process_identity(&self) -> SpawnedProcessIdentity {
            self.0.clone()
        }
    }

    impl Task41Task24Adapter for FakeTaskAdapter {
        type ProcessProof = FakeProof;
        type SessionEndUiKind = TestSessionEndUiKind;

        fn launch_notes_child(
            &mut self,
            request_id: NonZeroU64,
            note_id: ObjectId,
            surface_id: SurfaceId,
            channels: ChildNotesChannels,
        ) -> Result<(ProcessRecord, Self::ProcessProof), shell::SupervisorError> {
            let record = host_record();
            *self.shared.0.lock().unwrap() = Some((request_id, note_id, surface_id, channels));
            Ok((
                record,
                FakeProof(nagi_init_spawn_contract::test_identity(record.process_id)),
            ))
        }

        fn query_terminal_result(
            &mut self,
            proof: &Self::ProcessProof,
            record: ProcessRecord,
        ) -> Result<ExactTerminalResult, shell::SupervisorError> {
            *self.poll_count.lock().unwrap() += 1;
            let state = if self.pending_terminal {
                self.pending_terminal = false;
                ExactTerminalState::Pending
            } else {
                ExactTerminalState::Complete(ExitStatus {
                    exit_code: Some(0),
                    signal: None,
                })
            };
            Ok(ExactTerminalResult {
                process_instance: proof.process_identity(),
                record,
                gate_disposition: Some(ExecGateDisposition::Released),
                state,
            })
        }
    }

    fn host_record() -> ProcessRecord {
        ProcessRecord {
            process_id: 4001,
            app_id: APP_ID,
            app_session_id: AppSessionId(7),
            node_id: NodeId(2),
            workspace_id: None,
        }
    }

    fn framebuffer_file() -> (File, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "nagi-task28-loop-fb-{}-{}.raw",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (file, path)
    }

    fn evdev_key(code: u16, value: i32) -> [u8; 24] {
        let mut event = [0u8; 24];
        event[16..18].copy_from_slice(&1u16.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        event
    }

    #[test]
    fn verified_terminal_bridge_is_record_bound_and_one_shot() {
        let record = host_record();
        let mut bridge = ExactOneShotReap {
            expected: record,
            status: ExitStatus {
                exit_code: Some(0),
                signal: None,
            },
            consumed: false,
        };
        let mut wrong_record = record;
        wrong_record.process_id += 1;
        assert_eq!(
            shell::SupervisorPort::poll_reap(&mut bridge, wrong_record),
            Err(shell::SupervisorError::ProtocolViolation)
        );
        assert!(!bridge.consumed);
        assert_eq!(
            shell::SupervisorPort::poll_reap(&mut bridge, record),
            Ok(shell::ReapObservation::Reaped {
                record,
                exit_code: Some(0),
                signal: None,
            })
        );
        assert_eq!(
            shell::SupervisorPort::poll_reap(&mut bridge, record),
            Err(shell::SupervisorError::ProtocolViolation)
        );
    }

    #[test]
    fn source_loop_opens_real_notes_bridge_routes_input_and_polls_exact_reap() {
        let note_id = ObjectId(0x77);
        let store = Arc::new(InMemoryNoteStore::new());
        store
            .commit(&NoteDocument::new(note_id, "retained", Timestamp(1)), 0)
            .unwrap();
        let shared = Arc::new(SharedLaunch::default());
        let poll_count = Arc::new(Mutex::new(0));
        let adapter = FakeTaskAdapter {
            shared: Arc::clone(&shared),
            pending_terminal: true,
            poll_count: Arc::clone(&poll_count),
        };
        let (input, mut event_writer) = UnixStream::pair().unwrap();
        let (display, fb_path) = framebuffer_file();
        let fb_layout = ValidatedDisplayLayout {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            stride_bytes: FRAME_WIDTH * 4,
            mapped_bytes: PIXEL_COUNT * 4,
            format: PixelFormat::Rgba8888,
        };
        let (mut loop_owner, _) = ShellLoop::new(
            input,
            linux_evdev_stream::LinuxEvdevTarget::X86_64,
            note_id,
            FramebufferPresenter::new(display, fb_layout).unwrap(),
            SurfaceId::new(1).unwrap(),
            SurfaceId::new(2).unwrap(),
            SurfaceId::new(3).unwrap(),
            adapter,
        )
        .unwrap();
        event_writer.write_all(&evdev_key(28, 1)).unwrap();
        let launched = loop_owner.run_once(50).unwrap();
        assert!(launched.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesLaunchRequested { request_id: 1, .. }
        )));
        let request = shared
            .0
            .lock()
            .unwrap()
            .take()
            .expect("child launch channels");
        let (request_id, child_note_id, surface_id, child_channels) = request;
        let bootstrap = NotesBootstrap {
            request_id,
            note_id: child_note_id,
            surface_id,
            channels: child_channels,
        };
        let mut child =
            NotesChild::open(bootstrap, NotesEventBridge::new(Arc::clone(&store))).unwrap();
        let child_sender = std::thread::spawn(move || {
            child.send_current_frame().unwrap();
            child
        });
        let presented = loop_owner.run_once(250).unwrap();
        assert!(presented.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesFramePresented {
                revision: 1,
                first_frame: true,
                ..
            }
        )));
        let mut child = child_sender.join().unwrap();
        child.wait_for_presented_ack().unwrap();

        // Tab moves to the actual Files renderer; the Shell owner commits the
        // framebuffer before transferring input routing.
        event_writer
            .write_all(&evdev_key(INPUT_KEY_TAB, 1))
            .unwrap();
        let focus = loop_owner.run_once(50).unwrap();
        assert!(focus.iter().any(|event| matches!(event, ShellLoopObservation::FocusCommitted(commit) if commit.surface_id.get() == 3 && commit.focus_committed)));
        assert_eq!(loop_owner.focused_surface(), Surface::Files);

        // Tab back to Shell, then to Notes; only the active parent routes Escape.
        event_writer
            .write_all(&evdev_key(INPUT_KEY_TAB, 1))
            .unwrap();
        loop_owner.run_once(50).unwrap();
        event_writer
            .write_all(&evdev_key(INPUT_KEY_TAB, 1))
            .unwrap();
        loop_owner.run_once(50).unwrap();

        // Edit through Shell's existing Notes input route. A later close
        // result must wait until the parent reads this document's post-flush
        // revision, not merely any nonzero revision.
        event_writer.write_all(&evdev_key(30, 1)).unwrap();
        let edited = loop_owner.run_once(50).unwrap();
        assert!(edited.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesInputForwarded { .. }
        )));
        assert!(matches!(
            child.poll_input_once().unwrap(),
            crate::notes_child::NotesChildOutcome::Updated(_)
        ));
        event_writer
            .write_all(&evdev_key(INPUT_KEY_ESCAPE, 1))
            .unwrap();
        let routed = loop_owner.run_once(50).unwrap();
        assert!(routed
            .iter()
            .any(|event| matches!(event, ShellLoopObservation::NotesInputForwarded { .. })));
        // Queue a dirty revision-1 frame ahead of the successful revision-2
        // close frame to force the parent through the mismatch/hold state.
        child.send_current_frame().unwrap();
        assert!(matches!(
            child.poll_input_once().unwrap(),
            crate::notes_child::NotesChildOutcome::CloseCommitted(_)
        ));
        child.send_current_frame().unwrap();
        let awaiting = loop_owner.run_once(50).unwrap();
        assert!(awaiting.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesCloseAwaitingFrame {
                expected_revision: 2,
                observed_revision: Some(1),
                ..
            }
        )));
        assert!(!awaiting.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesCloseCommitted(_)
                | ShellLoopObservation::NotesReaped { .. }
        )));
        assert!(loop_owner.active_notes().is_some());

        let close = loop_owner.run_once(50).unwrap();
        assert!(close
            .iter()
            .any(|event| matches!(event, ShellLoopObservation::NotesCloseCommitted(_))));
        assert!(close.iter().any(|event| matches!(
            event,
            ShellLoopObservation::WaitingForExactTerminal { .. }
        )));
        assert!(loop_owner.active_notes().is_some());
        let reaped = loop_owner.run_once(50).unwrap();
        assert!(reaped.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesReaped {
                exit_code: Some(0),
                signal: None,
                gate_disposition: ExecGateDisposition::Released,
                ..
            }
        )));
        assert!(*poll_count.lock().unwrap() >= 2);
        assert!(loop_owner.active_notes().is_none());
        std::fs::remove_file(fb_path).unwrap();
    }

    #[test]
    fn delivered_session_end_hides_but_does_not_release_dirty_child() {
        let note_id = ObjectId(0x78);
        let store = Arc::new(InMemoryNoteStore::new());
        store
            .commit(&NoteDocument::new(note_id, "retained", Timestamp(1)), 0)
            .unwrap();
        let shared = Arc::new(SharedLaunch::default());
        let adapter = FakeTaskAdapter {
            shared: Arc::clone(&shared),
            pending_terminal: false,
            poll_count: Arc::new(Mutex::new(0)),
        };
        let (input, mut event_writer) = UnixStream::pair().unwrap();
        let (display, fb_path) = framebuffer_file();
        let (mut loop_owner, _) = ShellLoop::new(
            input,
            linux_evdev_stream::LinuxEvdevTarget::X86_64,
            note_id,
            FramebufferPresenter::new(
                display,
                ValidatedDisplayLayout {
                    width: FRAME_WIDTH,
                    height: FRAME_HEIGHT,
                    stride_bytes: FRAME_WIDTH * 4,
                    mapped_bytes: PIXEL_COUNT * 4,
                    format: PixelFormat::Bgra8888,
                },
            )
            .unwrap(),
            SurfaceId::new(1).unwrap(),
            SurfaceId::new(2).unwrap(),
            SurfaceId::new(3).unwrap(),
            adapter,
        )
        .unwrap();

        event_writer.write_all(&evdev_key(28, 1)).unwrap();
        assert!(loop_owner
            .run_once(50)
            .unwrap()
            .iter()
            .any(|event| matches!(event, ShellLoopObservation::NotesStarted { .. })));
        let (request_id, child_note_id, surface_id, child_channels) =
            shared.0.lock().unwrap().take().unwrap();
        let bootstrap = NotesBootstrap {
            request_id,
            note_id: child_note_id,
            surface_id,
            channels: child_channels,
        };
        let mut child =
            NotesChild::open(bootstrap, NotesEventBridge::new(Arc::clone(&store))).unwrap();
        child.send_current_frame().unwrap();
        loop_owner.run_once(50).unwrap();
        child.wait_for_presented_ack().unwrap();
        assert!(loop_owner.active_notes().is_some());

        // Dirty the in-memory child document without closing/flushing it.
        assert!(matches!(
            child
                .handle_event(InputEvent {
                    event_type: INPUT_EVENT_KEY,
                    code: 30,
                    value: 1,
                })
                .unwrap(),
            crate::notes_child::NotesChildOutcome::Updated(_)
        ));
        assert_eq!(child.current_document().search_text(), "a");
        assert_eq!(
            store
                .load_revision(note_id, 1)
                .unwrap()
                .unwrap()
                .search_text(),
            ""
        );

        // A normal Shell focus transition occurs before the C1 deliver
        // callback. This test starts at the E receiver edge; it does not test
        // C1's private recipient/generation validation.
        event_writer
            .write_all(&evdev_key(INPUT_KEY_TAB, 1))
            .unwrap();
        loop_owner.run_once(50).unwrap();
        assert_eq!(loop_owner.focused_surface(), Surface::Files);

        let (receipt, observations) =
            loop_owner.apply_delivered_session_end(TestSessionEndUiKind::Logout);
        assert_eq!(receipt, LocalSessionEndReceipt::Applied);
        assert!(observations.contains(&ShellLoopObservation::SessionEndNoticeApplied));
        assert_eq!(loop_owner.focused_surface(), Surface::Shell);
        assert!(loop_owner.active_notes().is_some());
        assert_eq!(
            loop_owner.child_revocation_state(),
            ChildRevocationState::Unconfirmed
        );
        assert_eq!(child.current_document().search_text(), "a");
        assert_eq!(
            store
                .load_revision(note_id, 1)
                .unwrap()
                .unwrap()
                .search_text(),
            ""
        );
        let (receipt, observations) =
            loop_owner.apply_delivered_session_end(TestSessionEndUiKind::Lock);
        assert_eq!(receipt, LocalSessionEndReceipt::AlreadyApplied);
        assert_eq!(
            observations,
            vec![ShellLoopObservation::SessionEndNoticeIgnored]
        );

        // The former physical input source can no longer change focus or
        // launch/close a child; no dirty document is flushed or discarded.
        event_writer
            .write_all(&evdev_key(INPUT_KEY_TAB, 1))
            .unwrap();
        let after_end = loop_owner.run_once(5).unwrap();
        assert_eq!(loop_owner.focused_surface(), Surface::Shell);
        assert!(loop_owner.active_notes().is_some());
        assert!(!after_end.iter().any(|event| matches!(
            event,
            ShellLoopObservation::NotesLaunchRequested { .. }
                | ShellLoopObservation::NotesCloseCommitted(_)
        )));
        std::fs::remove_file(fb_path).unwrap();
    }

    #[test]
    fn input_eof_is_reported_once_and_does_not_drop_active_owner() {
        let note_id = ObjectId(9);
        let shared = Arc::new(SharedLaunch::default());
        let poll_count = Arc::new(Mutex::new(0));
        let adapter = FakeTaskAdapter {
            shared: Arc::clone(&shared),
            pending_terminal: false,
            poll_count,
        };
        let (input, peer) = UnixStream::pair().unwrap();
        drop(peer);
        let (display, path) = framebuffer_file();
        let (mut owner, _) = ShellLoop::new(
            input,
            linux_evdev_stream::LinuxEvdevTarget::X86_64,
            note_id,
            FramebufferPresenter::new(
                display,
                ValidatedDisplayLayout {
                    width: FRAME_WIDTH,
                    height: FRAME_HEIGHT,
                    stride_bytes: FRAME_WIDTH * 4,
                    mapped_bytes: PIXEL_COUNT * 4,
                    format: PixelFormat::Bgra8888,
                },
            )
            .unwrap(),
            SurfaceId::new(1).unwrap(),
            SurfaceId::new(2).unwrap(),
            SurfaceId::new(3).unwrap(),
            adapter,
        )
        .unwrap();
        let lost = owner.run_once(50).unwrap();
        assert!(lost
            .iter()
            .any(|event| matches!(event, ShellLoopObservation::InputUnavailable(_))));
        assert!(owner
            .run_once(10)
            .unwrap()
            .iter()
            .all(|event| !matches!(event, ShellLoopObservation::InputUnavailable(_))));
        assert!(owner.active_notes().is_none());
        std::fs::remove_file(path).unwrap();
    }
}
