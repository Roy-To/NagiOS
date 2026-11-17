//! Shell input, surface focus, and app lifecycle state for the Linux boot owner.
//!
//! Copy this module as a child of `user/nagi-init/src/linux_init.rs` only after
//! the caller can supply its one authorized evdev stream and the existing
//! authenticated Supervisor inputs. This module never opens a device, chooses
//! a grant, or spawns a development/raw ELF.

use core::fmt;
use linux_evdev_stream::{EvdevStream, LinuxEvdevTarget, StreamError, StreamItem};
use nagi_abi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE, INPUT_KEY_TAB};
use nagi_files::{render_three_pane, FilesApp, FilesErrorKind, Location, ViewLoadState};
use nagi_model::{AppId, AppSessionId, NodeId, WorkspaceId};
use nagi_notes::{NoteId, APP_ID as NOTES_APP_ID};
use std::io::Read;

/// The process identity recorded by the source Supervisor.
/// Linux builds use the real public Supervisor record; non-Linux host tests use
/// a shape-compatible local record only to exercise the pure state transitions.
#[cfg(target_os = "linux")]
pub type ProcessRecord = libnagi::launch::LaunchRecord;

#[cfg(not(target_os = "linux"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessRecord {
    pub process_id: u32,
    pub app_id: AppId,
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Shell,
    Files,
    Notes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    Unavailable,
    Denied,
    TransportFailure,
    ProtocolViolation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReapObservation {
    Pending,
    /// Returned only after the boot `ProcessLifecycle` has observed wait(2)
    /// completion and removed the matching child. A signal or nonzero status is
    /// still a real reap, but cannot produce the task-44 clean-exit event.
    Reaped {
        record: ProcessRecord,
        exit_code: Option<u8>,
        signal: Option<u8>,
    },
}

/// Only the parent boot module can implement this port. Its production
/// implementation must call the existing authenticated Supervisor path and
/// must use the same live session and grants; an absent supplier returns
/// `Unavailable` without trying the development preview launcher.
pub(super) trait SupervisorPort {
    fn launch_notes(&mut self, note_id: NoteId) -> Result<ProcessRecord, SupervisorError>;
    fn forward_input(
        &mut self,
        record: ProcessRecord,
        event: InputEvent,
    ) -> Result<(), SupervisorError>;
    fn poll_reap(&mut self, record: ProcessRecord) -> Result<ReapObservation, SupervisorError>;
}

/// Fail-closed adapter used while Stage20 authorization/runtime inputs are not
/// supplied by boot. It has no success path and cannot mint a launch record.
pub(super) struct UnavailableSupervisor;

impl SupervisorPort for UnavailableSupervisor {
    fn launch_notes(&mut self, _note_id: NoteId) -> Result<ProcessRecord, SupervisorError> {
        Err(SupervisorError::Unavailable)
    }

    fn forward_input(
        &mut self,
        _record: ProcessRecord,
        _event: InputEvent,
    ) -> Result<(), SupervisorError> {
        Err(SupervisorError::Unavailable)
    }

    fn poll_reap(&mut self, _record: ProcessRecord) -> Result<ReapObservation, SupervisorError> {
        Err(SupervisorError::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildState {
    None,
    Launching {
        request_id: LaunchRequestId,
        note_id: NoteId,
    },
    Running(ProcessRecord),
    Closing(ProcessRecord),
    UnexpectedIdentity(ProcessRecord),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DispatchOutcome {
    NotesLaunchRejected(SupervisorError),
    NotesStarted(ProcessRecord),
    /// This transition must be called only from the Notes UI close handler
    /// after `NoteSession::close`/durable flush succeeds. A Shell Escape key
    /// alone does not produce this observation.
    NotesCloseRequested,
    NotesInputForwarded,
    AwaitingReap,
    NotesReaped {
        record: ProcessRecord,
        exit_code: Option<u8>,
        signal: Option<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShellAction {
    Ignored,
    /// Ordinary Shell Enter request. The boot owner can emit
    /// `notes_launch_requested` before calling the Supervisor.
    LaunchNotes {
        request_id: LaunchRequestId,
        note_id: NoteId,
    },
    /// The event must travel through a Supervisor-owned app-control channel;
    /// never pass the evdev reader or framebuffer handle to the child.
    ForwardNotesInput {
        record: ProcessRecord,
        event: InputEvent,
    },
    /// Logical focus only; not proof of a Files process or visible framebuffer.
    FocusChanged(Surface),
    FilesUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ControlError {
    Supervisor(SupervisorError),
    NotesLaunchPending,
    LaunchRequestMismatch,
    LaunchSequenceExhausted,
    NotesAlreadyLive,
    NotesClosing,
    NotesNotLive,
    ReapIdentityMismatch,
}

/// Opaque, monotonically increasing correlation ID for one launch request.
/// A completion from an earlier request cannot satisfy a later launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LaunchRequestId(u64);

impl From<SupervisorError> for ControlError {
    fn from(error: SupervisorError) -> Self {
        Self::Supervisor(error)
    }
}

/// One controller owns the logical focus state. It never gives apps the evdev
/// descriptor or direct framebuffer ownership. Physical display acquisition
/// and process-to-surface transfer are still unconnected in the Linux boot.
pub(super) struct ShellController {
    note_id: NoteId,
    focus: Surface,
    child: ChildState,
    next_launch_request_id: u64,
}

impl ShellController {
    pub(super) fn new(note_id: NoteId) -> Self {
        Self {
            note_id,
            focus: Surface::Shell,
            child: ChildState::None,
            next_launch_request_id: 0,
        }
    }

    pub(super) fn focus(&self) -> Surface {
        self.focus
    }

    pub(super) fn running_record(&self) -> Option<ProcessRecord> {
        match self.child {
            ChildState::Running(record)
            | ChildState::Closing(record)
            | ChildState::UnexpectedIdentity(record) => Some(record),
            ChildState::None | ChildState::Launching { .. } => None,
        }
    }

    pub(super) fn dispatch(&mut self, event: InputEvent) -> Result<ShellAction, ControlError> {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            // Releases and autorepeat cannot trigger launch or close actions.
            return Ok(ShellAction::Ignored);
        }

        if event.code == INPUT_KEY_TAB {
            self.cycle_focus();
            return Ok(ShellAction::FocusChanged(self.focus));
        }

        match event.code {
            INPUT_KEY_ENTER if self.focus == Surface::Shell => self.launch_notes(),
            INPUT_KEY_ENTER if self.focus == Surface::Files => Ok(ShellAction::FilesUnavailable),
            _ if self.focus == Surface::Notes => self.forward_to_notes(event),
            _ => Ok(ShellAction::Ignored),
        }
    }

    /// Apply the actual result from the existing boot Supervisor launch call.
    /// This is separate from `dispatch` so the Shell request event can be
    /// emitted before launch, while app-start evidence is emitted only here
    /// after a real process record has been returned.
    pub(super) fn complete_launch(
        &mut self,
        request_id: LaunchRequestId,
        result: Result<ProcessRecord, SupervisorError>,
    ) -> Result<DispatchOutcome, ControlError> {
        match self.child {
            ChildState::Launching {
                request_id: expected,
                ..
            } if expected == request_id => {}
            ChildState::Launching { .. } => return Err(ControlError::LaunchRequestMismatch),
            ChildState::Closing(_) => return Err(ControlError::NotesClosing),
            ChildState::None => return Err(ControlError::NotesNotLive),
            ChildState::Running(_) | ChildState::UnexpectedIdentity(_) => {
                return Err(ControlError::NotesAlreadyLive)
            }
        }
        self.child = ChildState::None;
        let record = match result {
            Ok(record) => record,
            Err(error) => return Ok(DispatchOutcome::NotesLaunchRejected(error)),
        };
        if record.app_id != NOTES_APP_ID {
            self.child = ChildState::UnexpectedIdentity(record);
            self.focus = Surface::Shell;
            return Ok(DispatchOutcome::NotesLaunchRejected(
                SupervisorError::ProtocolViolation,
            ));
        }
        self.child = ChildState::Running(record);
        self.focus = Surface::Notes;
        Ok(DispatchOutcome::NotesStarted(record))
    }

    /// Observe a close request from the Notes UI only after its own durable
    /// close path succeeds. Shell input never calls this directly.
    pub(super) fn notes_close_requested(
        &mut self,
        record: ProcessRecord,
    ) -> Result<DispatchOutcome, ControlError> {
        match self.child {
            ChildState::Running(expected) if expected == record => {
                self.child = ChildState::Closing(record);
                self.focus = Surface::Files;
                Ok(DispatchOutcome::NotesCloseRequested)
            }
            ChildState::Running(_) | ChildState::UnexpectedIdentity(_) => {
                Err(ControlError::ReapIdentityMismatch)
            }
            ChildState::Closing(_) => Err(ControlError::NotesClosing),
            ChildState::Launching { .. } => Err(ControlError::NotesLaunchPending),
            ChildState::None => Err(ControlError::NotesNotLive),
        }
    }

    pub(super) fn poll_close<P: SupervisorPort>(
        &mut self,
        supervisor: &mut P,
    ) -> Result<DispatchOutcome, ControlError> {
        let ChildState::Closing(expected) = self.child else {
            return Err(ControlError::NotesNotLive);
        };
        match supervisor.poll_reap(expected)? {
            ReapObservation::Pending => Ok(DispatchOutcome::AwaitingReap),
            ReapObservation::Reaped {
                record: actual,
                exit_code,
                signal,
            } if actual == expected => {
                self.child = ChildState::None;
                self.focus = Surface::Shell;
                Ok(DispatchOutcome::NotesReaped {
                    record: expected,
                    exit_code,
                    signal,
                })
            }
            ReapObservation::Reaped { .. } => Err(ControlError::ReapIdentityMismatch),
        }
    }

    fn cycle_focus(&mut self) {
        self.focus = match (self.focus, self.child) {
            (Surface::Shell, ChildState::Running(_)) => Surface::Notes,
            (Surface::Notes, ChildState::Running(_)) => Surface::Files,
            (Surface::Files, ChildState::Running(_)) => Surface::Shell,
            (Surface::Shell, ChildState::Launching { .. }) => Surface::Files,
            (Surface::Files, ChildState::Launching { .. }) => Surface::Shell,
            (Surface::Notes, ChildState::Launching { .. }) => Surface::Shell,
            (Surface::Shell, ChildState::Closing(_) | ChildState::UnexpectedIdentity(_)) => {
                Surface::Files
            }
            (Surface::Notes, ChildState::Closing(_) | ChildState::UnexpectedIdentity(_)) => {
                Surface::Files
            }
            (Surface::Files, ChildState::Closing(_) | ChildState::UnexpectedIdentity(_)) => {
                Surface::Shell
            }
            (Surface::Shell, ChildState::None) => Surface::Files,
            (Surface::Files, ChildState::None) => Surface::Shell,
            (Surface::Notes, ChildState::None) => Surface::Shell,
        };
    }

    fn launch_notes(&mut self) -> Result<ShellAction, ControlError> {
        match self.child {
            ChildState::Running(_) | ChildState::UnexpectedIdentity(_) => {
                return Err(ControlError::NotesAlreadyLive)
            }
            ChildState::Closing(_) => return Err(ControlError::NotesClosing),
            ChildState::Launching { .. } => return Err(ControlError::NotesLaunchPending),
            ChildState::None => {}
        }
        let request_id = self
            .next_launch_request_id
            .checked_add(1)
            .map(LaunchRequestId)
            .ok_or(ControlError::LaunchSequenceExhausted)?;
        self.next_launch_request_id = request_id.0;
        self.child = ChildState::Launching {
            request_id,
            note_id: self.note_id,
        };
        Ok(ShellAction::LaunchNotes {
            request_id,
            note_id: self.note_id,
        })
    }

    fn forward_to_notes(&mut self, event: InputEvent) -> Result<ShellAction, ControlError> {
        match self.child {
            ChildState::Running(record) => Ok(ShellAction::ForwardNotesInput { record, event }),
            ChildState::Closing(_) => Err(ControlError::NotesClosing),
            ChildState::Launching { .. } => Err(ControlError::NotesLaunchPending),
            ChildState::None | ChildState::UnexpectedIdentity(_) => Err(ControlError::NotesNotLive),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputPoll {
    Idle,
    Event(InputEvent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InputLost {
    Eof,
    NeedsResync,
    Stream(StreamError),
}

/// The caller moves its already-open, already-authorized input source here.
/// The parser and reader are non-cloneable, so the control plane has one input
/// owner. This adapter never opens `/dev/input` or infers a grant.
pub(super) struct ShellInput<R> {
    stream: EvdevStream<R>,
}

impl<R: Read> ShellInput<R> {
    pub(super) fn new(reader: R, target: LinuxEvdevTarget) -> Self {
        Self {
            stream: EvdevStream::new(reader, target),
        }
    }

    pub(super) fn poll(&mut self) -> Result<InputPoll, InputLost> {
        match self.stream.next_item() {
            Ok(StreamItem::Event(event)) => Ok(InputPoll::Event(event)),
            Ok(StreamItem::WouldBlock { .. }) => Ok(InputPoll::Idle),
            Ok(StreamItem::Eof) => Err(InputLost::Eof),
            Ok(StreamItem::NeedsResync) => Err(InputLost::NeedsResync),
            Err(error) => Err(InputLost::Stream(error)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeStep {
    Idle,
    Input(Result<ShellAction, ControlError>),
}

/// Owns the one moved input parser and Shell controller for its lifetime. On
/// stream failure callers receive a typed error while retaining this value, so
/// the controller is not implicitly reset by an input read error. The boot
/// caller still has to supply the exclusive display owner and compositor path.
pub(super) struct ShellRuntime<R> {
    input: ShellInput<R>,
    controller: ShellController,
    files: FilesSurface,
}

impl<R: Read> ShellRuntime<R> {
    pub(super) fn new(reader: R, target: LinuxEvdevTarget, note_id: NoteId) -> Self {
        Self {
            input: ShellInput::new(reader, target),
            controller: ShellController::new(note_id),
            files: FilesSurface::unavailable(),
        }
    }

    pub(super) fn step(&mut self) -> Result<RuntimeStep, InputLost> {
        match self.input.poll()? {
            InputPoll::Idle => Ok(RuntimeStep::Idle),
            InputPoll::Event(event) => Ok(RuntimeStep::Input(self.controller.dispatch(event))),
        }
    }

    pub(super) fn controller(&self) -> &ShellController {
        &self.controller
    }

    pub(super) fn complete_launch(
        &mut self,
        request_id: LaunchRequestId,
        result: Result<ProcessRecord, SupervisorError>,
    ) -> Result<DispatchOutcome, ControlError> {
        self.controller.complete_launch(request_id, result)
    }

    pub(super) fn notes_close_requested(
        &mut self,
        record: ProcessRecord,
    ) -> Result<DispatchOutcome, ControlError> {
        self.controller.notes_close_requested(record)
    }

    pub(super) fn poll_close<P: SupervisorPort>(
        &mut self,
        supervisor: &mut P,
    ) -> Result<DispatchOutcome, ControlError> {
        self.controller.poll_close(supervisor)
    }

    pub(super) fn files_frame(&self) -> String {
        self.files.frame()
    }
}

/// Uses the actual Files application model and renderer. The normal provider is
/// not supplied yet, so the view explicitly renders ProviderUnavailable and
/// never substitutes the host sandbox or an in-memory Files provider.
pub(super) struct FilesSurface {
    app: FilesApp,
    locale: nagi_files::localization::Locale,
}

impl FilesSurface {
    pub(super) fn unavailable() -> Self {
        let mut app = FilesApp::new(Location::root());
        app.state.load_state = ViewLoadState::Error(FilesErrorKind::ProviderUnavailable);
        Self {
            app,
            locale: nagi_files::localization::Locale::EnUs,
        }
    }

    pub(super) fn frame(&self) -> String {
        render_three_pane(
            &self.app.state.layout(),
            "Filesystem provider unavailable",
            self.locale,
        )
    }
}

/// Create the actual Linux Notes Desktop controller only from an already
/// authorized NoteStore bridge supplied by its caller. No host-preview or
/// replacement storage is constructed here.
#[cfg(target_os = "linux")]
pub(super) fn attach_notes_surface(
    locale: nagi_localization::Locale,
    bridge: notes_event_bridge_candidate::NotesEventBridge,
    note_id: NoteId,
) -> Result<nagi_linux_notes_desktop_dev::desktop::Desktop, impl std::error::Error + 'static> {
    nagi_linux_notes_desktop_dev::desktop::Desktop::new_with_notes(locale, bridge, note_id)
}

impl fmt::Display for InputLost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Shell input lost: {self:?}")
    }
}

impl std::error::Error for InputLost {}
