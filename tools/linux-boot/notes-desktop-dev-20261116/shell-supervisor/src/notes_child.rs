//! Notes child consumer built around the current public Notes event bridge.
//! Physical input/display devices remain with Shell; the child owns only its
//! typed channels and the real opened Notes session.

use crate::notes_bootstrap::{ChildNotesChannels, NotesBootstrap, SurfaceId};
use nagi_abi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ESCAPE};
use notes_event_bridge_candidate::NotesEventBridge;
use notes_files_codec_candidate::{NoteDocument, NoteId, NoteRevision};
use std::io::{self, Read, Write};
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixDatagram, UnixStream};

pub const FRAME_WIDTH: usize = 320;
pub const FRAME_HEIGHT: usize = 200;
pub const PIXEL_COUNT: usize = FRAME_WIDTH * FRAME_HEIGHT;
pub const FRAME_HEADER_BYTES: usize = 40;
const INPUT_BYTES: usize = 8;
const CLOSE_BYTES: usize = 40;
const ACK_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PixelFrame {
    pub surface_id: SurfaceId,
    pub note_id: NoteId,
    pub revision: NoteRevision,
    pub pixels: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotesCloseCommitted {
    pub request_id: NonZeroU64,
    pub surface_id: SurfaceId,
    pub note_id: NoteId,
    pub revision: NoteRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotesChildError {
    Notes(String),
    Io(io::ErrorKind),
    MalformedInput,
    MalformedFrame,
    WrongSurface,
    WrongDocument,
    NotOpen,
}

impl From<io::Error> for NotesChildError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotesChildOutcome {
    Ignored,
    Updated(NoteDocument),
    CloseCommitted(NotesCloseCommitted),
    CloseBlocked {
        document: Option<NoteDocument>,
        reason: String,
    },
}

/// Local Notes UI candidate using an in-process NoteStore-backed event bridge.
/// It is not Task41's typed NIPC client and does not enforce Stage24 live
/// Session, grant, or revocation checks per operation. Do not wire this
/// implementation into production until the source-owned broker supplies the
/// actual typed client and Notes operation schema; never replace that seam with a
/// StorageProvider handle or FD in the child bootstrap.
pub struct NotesChild {
    request_id: NonZeroU64,
    note_id: NoteId,
    surface_id: SurfaceId,
    channels: ChildNotesChannels,
    bridge: NotesEventBridge,
    document: NoteDocument,
    status_message: Option<String>,
    close_committed: bool,
}

impl NotesChild {
    pub fn open(
        bootstrap: NotesBootstrap,
        mut bridge: NotesEventBridge,
    ) -> Result<Self, NotesChildError> {
        let document = bridge
            .open_note(bootstrap.note_id)
            .map_err(|error| NotesChildError::Notes(error.to_string()))?;
        if document.id.0 != bootstrap.note_id.0 {
            return Err(NotesChildError::WrongDocument);
        }
        Ok(Self {
            request_id: bootstrap.request_id,
            note_id: bootstrap.note_id,
            surface_id: bootstrap.surface_id,
            channels: bootstrap.channels,
            bridge,
            document,
            status_message: None,
            close_committed: false,
        })
    }

    pub fn initial_frame(&self) -> PixelFrame {
        render_document(
            self.surface_id,
            &self.document,
            self.status_message.as_deref(),
        )
    }

    /// Called only after Shell's physical display owner has written this frame
    /// and successfully flushed the framebuffer. This ACK is not process or
    /// Supervisor identity evidence.
    pub fn wait_for_presented_ack(&mut self) -> Result<(), NotesChildError> {
        let mut ack = [0u8; ACK_BYTES];
        self.channels.display.read_exact(&mut ack)?;
        if &ack[0..4] != b"NDA1"
            || u64::from_le_bytes(ack[4..12].try_into().expect("fixed slice"))
                != self.request_id.get()
            || u64::from_le_bytes(ack[12..20].try_into().expect("fixed slice"))
                != self.surface_id.get()
            || u64::from_le_bytes(ack[20..28].try_into().expect("fixed slice"))
                != self.document.revision
            || ack[28..32] != [0; 4]
        {
            return Err(NotesChildError::MalformedFrame);
        }
        Ok(())
    }

    pub fn send_current_frame(&mut self) -> Result<(), NotesChildError> {
        let frame = self.initial_frame();
        write_frame(&mut self.channels.display, &frame)
    }

    /// Own the Notes child loop after Task41 bootstrap. The parent must
    /// acknowledge the initially opened surface before input is accepted.
    /// Successful edits are rendered over the transferred display channel;
    /// Escape returns only after the real bridge close/flush succeeds. A
    /// failed close keeps the same bridge/session alive and renders an
    /// explicit unsaved status while awaiting another input event.
    pub fn run_until_close(&mut self) -> Result<NotesCloseCommitted, NotesChildError> {
        self.send_current_frame()?;
        self.wait_for_presented_ack()?;
        loop {
            wait_readable(self.channels.input.as_raw_fd())?;
            match self.poll_input_once()? {
                NotesChildOutcome::Ignored => {}
                NotesChildOutcome::Updated(_) => self.send_current_frame()?,
                NotesChildOutcome::CloseCommitted(committed) => {
                    // Present the exact post-flush document revision before
                    // telling the parent it may begin close/reap. The parent
                    // still compares NoteId and revision; this frame is not
                    // itself a persistence or process-exit acknowledgment.
                    self.send_current_frame()?;
                    return Ok(committed);
                }
                NotesChildOutcome::CloseBlocked { reason, .. } => {
                    self.status_message = Some(format!("UNSAVED - SAVE FAILED: {reason}"));
                    self.send_current_frame()?;
                }
            }
        }
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    /// Read and dispatch one atomic parent-routed input datagram. On a close
    /// key the bridge's `close_active` must return a successfully flushed
    /// document before a close-committed response is sent.
    pub fn poll_input_once(&mut self) -> Result<NotesChildOutcome, NotesChildError> {
        if self.close_committed {
            return Err(NotesChildError::NotOpen);
        }
        let mut bytes = [0u8; INPUT_BYTES + 1];
        let count = self.channels.input.recv(&mut bytes)?;
        if count != INPUT_BYTES {
            return Err(NotesChildError::MalformedInput);
        }
        let event = decode_input(
            bytes[..INPUT_BYTES]
                .try_into()
                .expect("input frame length checked"),
        );
        self.handle_event(event)
    }

    pub fn handle_event(
        &mut self,
        event: InputEvent,
    ) -> Result<NotesChildOutcome, NotesChildError> {
        if self.close_committed {
            return Err(NotesChildError::NotOpen);
        }
        match self.bridge.handle_event(event) {
            Ok(Some(document)) => {
                if document.id != self.note_id {
                    return Err(NotesChildError::WrongDocument);
                }
                self.document = document.clone();
                if event.event_type == INPUT_EVENT_KEY
                    && event.code == INPUT_KEY_ESCAPE
                    && event.value != 0
                    && self.bridge.current_note().is_none()
                {
                    self.status_message = None;
                    let committed = NotesCloseCommitted {
                        request_id: self.request_id,
                        surface_id: self.surface_id,
                        note_id: self.note_id,
                        revision: self.document.revision,
                    };
                    send_close_commit(&self.channels.input, &committed)?;
                    self.close_committed = true;
                    Ok(NotesChildOutcome::CloseCommitted(committed))
                } else {
                    Ok(NotesChildOutcome::Updated(document))
                }
            }
            Ok(None) => Ok(NotesChildOutcome::Ignored),
            Err(error) => {
                let document = self.bridge.current_note();
                if let Some(current) = &document {
                    if current.id == self.note_id {
                        self.document = current.clone();
                    }
                }
                let reason = error.to_string();
                if event.event_type == INPUT_EVENT_KEY
                    && event.code == INPUT_KEY_ESCAPE
                    && event.value != 0
                {
                    self.status_message = Some(format!("UNSAVED - SAVE FAILED: {reason}"));
                    Ok(NotesChildOutcome::CloseBlocked { document, reason })
                } else {
                    Err(NotesChildError::Notes(reason))
                }
            }
        }
    }

    pub fn current_document(&self) -> &NoteDocument {
        &self.document
    }

    pub fn input_channel(&self) -> &UnixDatagram {
        &self.channels.input
    }

    pub fn display_channel(&self) -> &UnixStream {
        &self.channels.display
    }
}

pub fn read_frame(
    stream: &mut UnixStream,
    expected_surface: SurfaceId,
) -> Result<PixelFrame, NotesChildError> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    stream.read_exact(&mut header)?;
    if &header[0..4] != b"NDF1"
        || u16::from_le_bytes([header[4], header[5]]) != 1
        || u16::from_le_bytes([header[6], header[7]]) as usize != FRAME_HEADER_BYTES
        || u16::from_le_bytes([header[8], header[9]]) as usize != FRAME_WIDTH
        || u16::from_le_bytes([header[10], header[11]]) as usize != FRAME_HEIGHT
    {
        return Err(NotesChildError::MalformedFrame);
    }
    let surface_id = SurfaceId::new(u64::from_le_bytes(
        header[12..20].try_into().expect("fixed slice"),
    ))
    .ok_or(NotesChildError::MalformedFrame)?;
    if surface_id != expected_surface {
        return Err(NotesChildError::WrongSurface);
    }
    let note_id = nagi_model::ObjectId(u64::from_le_bytes(
        header[20..28].try_into().expect("fixed slice"),
    ));
    let revision = u64::from_le_bytes(header[28..36].try_into().expect("fixed slice"));
    let pixel_bytes = u32::from_le_bytes(header[36..40].try_into().expect("fixed slice")) as usize;
    if note_id.0 == 0 || revision == 0 || pixel_bytes != PIXEL_COUNT * 4 {
        return Err(NotesChildError::MalformedFrame);
    }
    let mut raw = vec![0u8; pixel_bytes];
    stream.read_exact(&mut raw)?;
    let pixels = raw
        .chunks_exact(4)
        .map(|pixel| u32::from_le_bytes(pixel.try_into().expect("four-byte pixel")))
        .collect();
    Ok(PixelFrame {
        surface_id,
        note_id,
        revision,
        pixels,
    })
}

pub fn write_frame(stream: &mut UnixStream, frame: &PixelFrame) -> Result<(), NotesChildError> {
    if frame.pixels.len() != PIXEL_COUNT || frame.note_id.0 == 0 || frame.revision == 0 {
        return Err(NotesChildError::MalformedFrame);
    }
    let mut header = [0u8; FRAME_HEADER_BYTES];
    header[0..4].copy_from_slice(b"NDF1");
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[6..8].copy_from_slice(&(FRAME_HEADER_BYTES as u16).to_le_bytes());
    header[8..10].copy_from_slice(&(FRAME_WIDTH as u16).to_le_bytes());
    header[10..12].copy_from_slice(&(FRAME_HEIGHT as u16).to_le_bytes());
    header[12..20].copy_from_slice(&frame.surface_id.get().to_le_bytes());
    header[20..28].copy_from_slice(&frame.note_id.0.to_le_bytes());
    header[28..36].copy_from_slice(&frame.revision.to_le_bytes());
    header[36..40].copy_from_slice(&((PIXEL_COUNT * 4) as u32).to_le_bytes());
    stream.write_all(&header)?;
    for pixel in &frame.pixels {
        stream.write_all(&pixel.to_le_bytes())?;
    }
    Ok(())
}

pub fn send_presented_ack(
    stream: &mut UnixStream,
    request_id: NonZeroU64,
    surface_id: SurfaceId,
    revision: NoteRevision,
) -> Result<(), NotesChildError> {
    let mut ack = [0u8; ACK_BYTES];
    ack[0..4].copy_from_slice(b"NDA1");
    ack[4..12].copy_from_slice(&request_id.get().to_le_bytes());
    ack[12..20].copy_from_slice(&surface_id.get().to_le_bytes());
    ack[20..28].copy_from_slice(&revision.to_le_bytes());
    stream.write_all(&ack)?;
    Ok(())
}

pub fn receive_close_commit(input: &UnixDatagram) -> Result<NotesCloseCommitted, NotesChildError> {
    let mut bytes = [0u8; CLOSE_BYTES + 1];
    let count = input.recv(&mut bytes)?;
    if count != CLOSE_BYTES || &bytes[0..4] != b"NCC1" || bytes[36..40] != [0; 4] {
        return Err(NotesChildError::MalformedInput);
    }
    let bytes = &bytes[..CLOSE_BYTES];
    Ok(NotesCloseCommitted {
        request_id: NonZeroU64::new(u64::from_le_bytes(
            bytes[4..12].try_into().expect("fixed slice"),
        ))
        .ok_or(NotesChildError::MalformedInput)?,
        surface_id: SurfaceId::new(u64::from_le_bytes(
            bytes[12..20].try_into().expect("fixed slice"),
        ))
        .ok_or(NotesChildError::MalformedInput)?,
        note_id: nagi_model::ObjectId(u64::from_le_bytes(
            bytes[20..28].try_into().expect("fixed slice"),
        )),
        revision: u64::from_le_bytes(bytes[28..36].try_into().expect("fixed slice")),
    })
}

pub fn encode_input(event: InputEvent) -> [u8; INPUT_BYTES] {
    let mut bytes = [0; INPUT_BYTES];
    bytes[0..2].copy_from_slice(&event.event_type.to_le_bytes());
    bytes[2..4].copy_from_slice(&event.code.to_le_bytes());
    bytes[4..8].copy_from_slice(&event.value.to_le_bytes());
    bytes
}

fn decode_input(bytes: &[u8; INPUT_BYTES]) -> InputEvent {
    InputEvent {
        event_type: u16::from_le_bytes([bytes[0], bytes[1]]),
        code: u16::from_le_bytes([bytes[2], bytes[3]]),
        value: i32::from_le_bytes(bytes[4..8].try_into().expect("fixed slice")),
    }
}

fn wait_readable(fd: i32) -> Result<(), NotesChildError> {
    loop {
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, -1) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if descriptor.revents & libc::POLLIN != 0 {
            return Ok(());
        }
        return Err(NotesChildError::Io(io::ErrorKind::BrokenPipe));
    }
}

fn send_close_commit(
    input: &UnixDatagram,
    committed: &NotesCloseCommitted,
) -> Result<(), NotesChildError> {
    let mut bytes = [0u8; CLOSE_BYTES];
    bytes[0..4].copy_from_slice(b"NCC1");
    bytes[4..12].copy_from_slice(&committed.request_id.get().to_le_bytes());
    bytes[12..20].copy_from_slice(&committed.surface_id.get().to_le_bytes());
    bytes[20..28].copy_from_slice(&committed.note_id.0.to_le_bytes());
    bytes[28..36].copy_from_slice(&committed.revision.to_le_bytes());
    if input.send(&bytes)? != CLOSE_BYTES {
        return Err(NotesChildError::MalformedInput);
    }
    Ok(())
}

/// Render the actual Notes title/body data into a bounded 320×200 child frame.
/// This compact candidate painter is not the production Notes Desktop skin.
fn render_document(
    surface_id: SurfaceId,
    document: &NoteDocument,
    status: Option<&str>,
) -> PixelFrame {
    let pixels = render_text_pixels(&format!(
        "NOTE {} / REVISION {}\n{}\n{}\n{}",
        document.id.0,
        document.revision,
        status.unwrap_or(""),
        document.title,
        document.search_text()
    ));
    PixelFrame {
        surface_id,
        note_id: document.id,
        revision: document.revision,
        pixels,
    }
}

pub(crate) fn render_text_pixels(text: &str) -> Vec<u32> {
    let mut pixels = vec![0x0018_2028; PIXEL_COUNT];
    draw_text(&mut pixels, 8, 8, text, 0x00f0_f2f5);
    pixels
}

fn draw_text(pixels: &mut [u32], x: usize, y: usize, text: &str, color: u32) {
    let mut cursor_x = x;
    let mut cursor_y = y;
    for character in text.chars() {
        if character == '\n' || cursor_x + 6 >= FRAME_WIDTH {
            cursor_x = x;
            cursor_y += 9;
            if character == '\n' || cursor_y + 7 >= FRAME_HEIGHT {
                continue;
            }
        }
        if cursor_y + 7 >= FRAME_HEIGHT {
            break;
        }
        let glyph = glyph(character.to_ascii_uppercase());
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    let at = (cursor_y + row) * FRAME_WIDTH + cursor_x + column;
                    pixels[at] = color;
                }
            }
        }
        cursor_x += 6;
    }
}

fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0; 7],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes_bootstrap::{channel_pair, NotesBootstrap, ParentNotesChannels};
    use notes_files_codec_candidate::{
        HostObjectIdSource, InMemoryNoteStore, NoopActivitySink, NoteDocument, NoteStore, NoteSummary,
        NotesApp, NotesUiController, StoreError, SystemClock, Timestamp,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn setup() -> (NotesChild, Arc<InMemoryNoteStore>, ParentNotesChannels) {
        let store = Arc::new(InMemoryNoteStore::new());
        let note = NoteDocument::new(nagi_model::ObjectId(0x55), "existing", Timestamp(1));
        store.commit(&note, 0).unwrap();
        let (parent, child) = channel_pair().unwrap();
        let bootstrap = NotesBootstrap {
            request_id: NonZeroU64::new(9).unwrap(),
            note_id: note.id,
            surface_id: SurfaceId::new(12).unwrap(),
            channels: child,
        };
        let bridge = NotesEventBridge::new(Arc::clone(&store));
        (NotesChild::open(bootstrap, bridge).unwrap(), store, parent)
    }

    #[test]
    fn real_bridge_edit_updates_frame_and_only_successful_escape_closes() {
        let (mut child, store, parent) = setup();
        assert_eq!(child.current_document().revision, 1);
        assert_eq!(child.current_document().search_text(), "");
        let changed = child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: 30,
                value: 1,
            })
            .unwrap();
        assert!(matches!(changed, NotesChildOutcome::Updated(_)));
        assert_eq!(child.current_document().search_text(), "a");
        let frame = child.initial_frame();
        assert!(frame.pixels.iter().any(|pixel| *pixel == 0x00c8_d4e0));
        let close = child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: INPUT_KEY_ESCAPE,
                value: 1,
            })
            .unwrap();
        let NotesChildOutcome::CloseCommitted(commit) = close else {
            panic!("successful close expected")
        };
        assert_eq!(commit.request_id.get(), 9);
        assert_eq!(commit.revision, 2);
        assert_eq!(
            store
                .load_revision(nagi_model::ObjectId(0x55), 2)
                .unwrap()
                .unwrap()
                .search_text(),
            "a"
        );
        assert_eq!(receive_close_commit(&parent.input).unwrap(), commit);
        assert_eq!(child.poll_input_once(), Err(NotesChildError::NotOpen));
    }

    #[test]
    fn frame_wire_round_trip_checks_surface_and_revision() {
        let (mut child, _store, _parent) = setup();
        let (mut sender, mut receiver) = UnixStream::pair().unwrap();
        let expected = child.initial_frame();
        let clone = expected.clone();
        let writer = std::thread::spawn(move || write_frame(&mut sender, &clone).unwrap());
        let observed = read_frame(&mut receiver, SurfaceId::new(12).unwrap()).unwrap();
        writer.join().unwrap();
        assert_eq!(observed, expected);
        assert_eq!(
            read_frame(&mut receiver, SurfaceId::new(99).unwrap()),
            Err(NotesChildError::Io(io::ErrorKind::UnexpectedEof))
        );
    }

    struct CommitFailureStore {
        inner: InMemoryNoteStore,
        fail_commits: AtomicBool,
    }

    impl NoteStore for CommitFailureStore {
        fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
            self.inner.list(include_deleted)
        }

        fn load(&self, id: nagi_model::ObjectId) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load(id)
        }

        fn load_revision(
            &self,
            id: nagi_model::ObjectId,
            revision: u64,
        ) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load_revision(id, revision)
        }

        fn commit(
            &self,
            note: &NoteDocument,
            expected_revision: u64,
        ) -> Result<NoteDocument, StoreError> {
            if self.fail_commits.load(Ordering::SeqCst) {
                return Err(StoreError::Io("injected original-store failure".to_owned()));
            }
            self.inner.commit(note, expected_revision)
        }
    }

    #[test]
    fn failed_close_keeps_dirty_document_and_sends_no_close_commit() {
        let id = nagi_model::ObjectId(0x88);
        let inner = InMemoryNoteStore::new();
        inner
            .commit(&NoteDocument::new(id, "retained", Timestamp(1)), 0)
            .unwrap();
        let store = Arc::new(CommitFailureStore {
            inner,
            fail_commits: AtomicBool::new(false),
        });
        let (parent, child_channels) = channel_pair().unwrap();
        let bootstrap = NotesBootstrap {
            request_id: NonZeroU64::new(10).unwrap(),
            note_id: id,
            surface_id: SurfaceId::new(13).unwrap(),
            channels: child_channels,
        };
        let mut child =
            NotesChild::open(bootstrap, NotesEventBridge::new(Arc::clone(&store))).unwrap();
        assert!(matches!(
            child
                .handle_event(InputEvent {
                    event_type: INPUT_EVENT_KEY,
                    code: 30,
                    value: 1,
                })
                .unwrap(),
            NotesChildOutcome::Updated(_)
        ));
        store.fail_commits.store(true, Ordering::SeqCst);
        let blocked = child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: INPUT_KEY_ESCAPE,
                value: 1,
            })
            .unwrap();
        assert!(matches!(
            blocked,
            NotesChildOutcome::CloseBlocked {
                document: Some(_),
                ..
            }
        ));
        assert!(!child.close_committed);
        assert_eq!(child.current_document().search_text(), "a");
        assert_eq!(child.current_document().revision, 1);
        assert!(child
            .status_message()
            .unwrap()
            .contains("UNSAVED - SAVE FAILED"));
        assert_eq!(
            store.inner.load(id).unwrap().unwrap().revision,
            1,
            "persisted revision remains unchanged on close failure"
        );
        parent.input.set_nonblocking(true).unwrap();
        let mut bytes = [0u8; CLOSE_BYTES];
        assert_eq!(
            parent.input.recv(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "no close-committed response is sent after a failed flush"
        );
    }

    struct PublishThenLoseAckStore {
        inner: InMemoryNoteStore,
        commit_calls: AtomicUsize,
    }

    impl NoteStore for PublishThenLoseAckStore {
        fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
            self.inner.list(include_deleted)
        }

        fn load(&self, id: nagi_model::ObjectId) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load(id)
        }

        fn load_revision(
            &self,
            id: nagi_model::ObjectId,
            revision: u64,
        ) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load_revision(id, revision)
        }

        fn commit(
            &self,
            note: &NoteDocument,
            expected_revision: u64,
        ) -> Result<NoteDocument, StoreError> {
            let attempt = self.commit_calls.fetch_add(1, Ordering::SeqCst);
            let published = self.inner.commit(note, expected_revision)?;
            if attempt == 0 {
                // Model a StoreError::Io after publication: the caller does not
                // receive the durable acknowledgment and must not resend it
                // automatically. The in-memory fixture is not durable storage.
                Err(StoreError::Io("injected lost commit acknowledgment".into()))
            } else {
                Ok(published)
            }
        }
    }

    #[test]
    fn ambiguous_post_publish_error_keeps_dirty_open_and_is_not_resent() {
        let id = nagi_model::ObjectId(0x99);
        let inner = InMemoryNoteStore::new();
        inner
            .commit(&NoteDocument::new(id, "retained", Timestamp(1)), 0)
            .unwrap();
        let store = Arc::new(PublishThenLoseAckStore {
            inner,
            commit_calls: AtomicUsize::new(0),
        });
        let app = NotesApp::with_providers(
            store.clone(),
            Arc::new(HostObjectIdSource::new()),
            Arc::new(SystemClock),
            Arc::new(NoopActivitySink),
            Duration::from_secs(60),
        );
        let bridge = NotesEventBridge::from_controller(NotesUiController::from_app(app));
        let (parent, child_channels) = channel_pair().unwrap();
        let bootstrap = NotesBootstrap {
            request_id: NonZeroU64::new(11).unwrap(),
            note_id: id,
            surface_id: SurfaceId::new(14).unwrap(),
            channels: child_channels,
        };
        let mut child = NotesChild::open(bootstrap, bridge).unwrap();

        child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: 30,
                value: 1,
            })
            .unwrap();
        let first_close = child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: INPUT_KEY_ESCAPE,
                value: 1,
            })
            .unwrap();
        assert!(matches!(
            first_close,
            NotesChildOutcome::CloseBlocked {
                document: Some(_),
                ..
            }
        ));
        assert!(!child.close_committed);
        assert_eq!(child.current_document().id, id);
        assert_eq!(child.current_document().revision, 1);
        assert_eq!(child.current_document().search_text(), "a");
        let published = store.inner.load(id).unwrap().unwrap();
        assert_eq!(published.revision, 2);
        assert_eq!(published.search_text(), "a");
        assert_eq!(store.commit_calls.load(Ordering::SeqCst), 1);

        // A second close attempt observes the same failed NoteSession. It does
        // not call NoteSession::retry and cannot auto-replay the uncertain save.
        let second_close = child
            .handle_event(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code: INPUT_KEY_ESCAPE,
                value: 1,
            })
            .unwrap();
        assert!(matches!(
            second_close,
            NotesChildOutcome::CloseBlocked {
                document: Some(_),
                ..
            }
        ));
        assert!(!child.close_committed);
        assert_eq!(store.commit_calls.load(Ordering::SeqCst), 1);

        parent.input.set_nonblocking(true).unwrap();
        let mut bytes = [0u8; CLOSE_BYTES];
        assert_eq!(
            parent.input.recv(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "ambiguous save failure did not emit a close commit"
        );
    }
}
