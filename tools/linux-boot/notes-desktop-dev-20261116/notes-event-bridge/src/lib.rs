use linux_evdev_decoder::decode_evdev_record;
pub use linux_evdev_decoder::{
    DecodeError, LinuxEvdevTarget, EV_KEY, EV_SYN, SYN_DROPPED, SYN_REPORT,
};
pub use linux_evdev_stream::StreamError;
use linux_evdev_stream::{EvdevStream, StreamItem};
use nagi_abi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE, INPUT_KEY_SPACE};
use notes_files_codec_candidate::{
    AppError, NoteDocument, NoteSessionEffectGate, NoteStore, NotesUiController, ObjectId,
    SessionGatedNoteStore,
};
use std::sync::Arc;

/// Failure while turning one caller-framed Linux evdev record into a Notes action.
#[derive(Debug, PartialEq, Eq)]
pub enum EvdevRecordError {
    Decode(DecodeError),
    NeedsResync,
    Notes(AppError),
}

impl core::fmt::Display for EvdevRecordError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "evdev record decode: {error:?}"),
            Self::NeedsResync => formatter.write_str("evdev input state needs resynchronization"),
            Self::Notes(error) => write!(formatter, "Notes event: {error}"),
        }
    }
}

impl std::error::Error for EvdevRecordError {}

/// Outcome from one non-looping poll of the caller-owned event stream.
#[derive(Debug, PartialEq, Eq)]
pub enum NotesStreamPoll {
    Updated(NoteDocument),
    Unhandled,
    WouldBlock { buffered_bytes: usize },
    Eof,
    NeedsResync,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NotesStreamError {
    Stream(StreamError),
    Notes(AppError),
}

impl core::fmt::Display for NotesStreamError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stream(error) => write!(formatter, "evdev stream: {error:?}"),
            Self::Notes(error) => write!(formatter, "Notes stream action: {error}"),
        }
    }
}

impl std::error::Error for NotesStreamError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamTerminal {
    Eof,
    NeedsResync,
    Failed(StreamError),
}

/// Caller-supplied reader adapter for sequential Notes actions.
///
/// A poll calls the stream's `next_item` no more than once. Unsupported event types
/// and codes are consumed as unhandled records; malformed records, invalid key values,
/// and non-retryable I/O errors stop this adapter. `SYN_DROPPED` also latches it; the
/// caller must recover key state outside this crate, then construct a new adapter.
/// Passing `&mut R` preserves reader ownership with the caller for that operation.
pub struct NotesEventStream<R> {
    stream: EvdevStream<R>,
    terminal: Option<StreamTerminal>,
}

impl<R: std::io::Read> NotesEventStream<R> {
    pub fn new(reader: R, target: LinuxEvdevTarget) -> Self {
        Self {
            stream: EvdevStream::new(reader, target),
            terminal: None,
        }
    }

    pub fn poll(
        &mut self,
        bridge: &mut NotesEventBridge,
    ) -> Result<NotesStreamPoll, NotesStreamError> {
        if let Some(terminal) = self.terminal {
            return match terminal {
                StreamTerminal::Eof => Ok(NotesStreamPoll::Eof),
                StreamTerminal::NeedsResync => Ok(NotesStreamPoll::NeedsResync),
                StreamTerminal::Failed(error) => Err(NotesStreamError::Stream(error)),
            };
        }

        let item = match self.stream.next_item() {
            Ok(item) => item,
            Err(StreamError::Decode(DecodeError::UnsupportedEventType(_)))
            | Err(StreamError::Decode(DecodeError::UnsupportedCode { .. })) => {
                // One complete, framed event was consumed. It is outside this
                // Notes consumer's mapping, so leave the stream usable for the
                // next poll without turning this single poll into a read loop.
                return Ok(NotesStreamPoll::Unhandled);
            }
            Err(error) => {
                self.terminal = Some(StreamTerminal::Failed(error));
                return Err(NotesStreamError::Stream(error));
            }
        };
        match item {
            StreamItem::Event(event) => match bridge.handle_event(event) {
                Ok(Some(document)) => Ok(NotesStreamPoll::Updated(document)),
                Ok(None) => Ok(NotesStreamPoll::Unhandled),
                Err(error) => Err(NotesStreamError::Notes(error)),
            },
            StreamItem::WouldBlock { buffered_bytes } => {
                Ok(NotesStreamPoll::WouldBlock { buffered_bytes })
            }
            StreamItem::Eof => {
                self.terminal = Some(StreamTerminal::Eof);
                Ok(NotesStreamPoll::Eof)
            }
            StreamItem::NeedsResync => {
                self.terminal = Some(StreamTerminal::NeedsResync);
                Ok(NotesStreamPoll::NeedsResync)
            }
        }
    }
}

// This follows the existing public US-layout scancode mapping as
// `libnagi::login::key_char`, with space accepted as Notes text. It is kept
// local so this candidate can link the exact ABI event type without importing
// libnagi's separate `nagi-model` path instance.
const fn key_char(code: u16) -> Option<u8> {
    const LETTERS: [(u16, u8); 26] = [
        (16, b'q'),
        (17, b'w'),
        (18, b'e'),
        (19, b'r'),
        (20, b't'),
        (21, b'y'),
        (22, b'u'),
        (23, b'i'),
        (24, b'o'),
        (25, b'p'),
        (30, b'a'),
        (31, b's'),
        (32, b'd'),
        (33, b'f'),
        (34, b'g'),
        (35, b'h'),
        (36, b'j'),
        (37, b'k'),
        (38, b'l'),
        (44, b'z'),
        (45, b'x'),
        (46, b'c'),
        (47, b'v'),
        (48, b'b'),
        (49, b'n'),
        (50, b'm'),
    ];
    match code {
        2..=10 => Some(b'1' + (code - 2) as u8),
        11 => Some(b'0'),
        12 => Some(b'-'),
        INPUT_KEY_SPACE => Some(b' '),
        _ => {
            let mut index = 0;
            while index < LETTERS.len() {
                if LETTERS[index].0 == code {
                    return Some(LETTERS[index].1);
                }
                index += 1;
            }
            None
        }
    }
}

/// Routes the real Nagi input ABI through the public Notes controller.
/// Callers pass only events already routed to the Notes window.
pub struct NotesEventBridge {
    controller: NotesUiController,
}

impl NotesEventBridge {
    /// Construct an explicitly unauthenticated bridge for developer previews
    /// and fixtures. Production launchers must use `new_with_session_gate`.
    pub fn new<S: NoteStore + 'static>(store: Arc<S>) -> Self {
        Self::from_controller(NotesUiController::new(store))
    }

    /// Construct a bridge whose Notes reads and saves all run under a required
    /// live-session effect gate. The supplied gate must bind the authenticated
    /// caller to C1's canonical Session and hold its effect admission through
    /// completion; a standalone `check_live` is insufficient.
    pub fn new_with_session_gate<S, G>(store: Arc<S>, gate: G) -> Self
    where
        S: NoteStore + 'static,
        G: NoteSessionEffectGate + 'static,
    {
        let gated = Arc::new(SessionGatedNoteStore::new(store, gate));
        Self::new(gated)
    }

    pub fn from_controller(controller: NotesUiController) -> Self {
        Self { controller }
    }

    /// Open a selected persisted note and return its real document model for rendering.
    pub fn open_note(&mut self, id: ObjectId) -> Result<NoteDocument, AppError> {
        self.controller.open_note(id)?;
        self.controller.current_note().ok_or(AppError::NotFound(id))
    }

    /// Process one real `libnagi::InputEvent`; return the changed/saved/closed Notes model.
    /// Character input follows the existing public US-layout `key_char` mapping.
    /// Escape uses the existing close path, which flushes dirty content rather than discarding it.
    pub fn handle_event(&mut self, event: InputEvent) -> Result<Option<NoteDocument>, AppError> {
        if event.event_type != INPUT_EVENT_KEY || event.value == 0 {
            return Ok(None);
        }

        match event.code {
            INPUT_KEY_ENTER => self.controller.save_active(),
            INPUT_KEY_ESCAPE => self.controller.close_active(),
            code => {
                let Some(byte) = key_char(code) else {
                    return Ok(None);
                };
                let text = char::from(byte).to_string();
                if self.controller.input_text(&text)? {
                    Ok(self.controller.current_note())
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Decode one explicitly framed x86 evdev record, then route its existing ABI event
    /// through this bridge. This does not open a device, authorize input, or generate text.
    pub fn handle_evdev_record(
        &mut self,
        target: LinuxEvdevTarget,
        record: &[u8],
    ) -> Result<Option<NoteDocument>, EvdevRecordError> {
        let event = decode_evdev_record(target, record).map_err(EvdevRecordError::Decode)?;
        if event.event_type == EV_SYN && event.code == SYN_DROPPED {
            return Err(EvdevRecordError::NeedsResync);
        }
        self.handle_event(event).map_err(EvdevRecordError::Notes)
    }

    /// Mirror the existing Notes title action for callers that provide title editing.
    pub fn set_title(&self, title: impl Into<String>) -> Result<bool, AppError> {
        self.controller.set_title(title)
    }

    pub fn current_note(&self) -> Option<NoteDocument> {
        self.controller.current_note()
    }

    /// Flush the current session once when its owner is preparing to preserve
    /// or release the desktop after input loss.
    pub fn flush_active(&self) -> Result<Option<NoteDocument>, AppError> {
        self.controller.save_active()
    }

    /// Retry the same active Notes session after an explicit recovery action.
    pub fn retry_active(&self) -> Result<Option<NoteDocument>, AppError> {
        self.controller.retry_active()
    }
}
