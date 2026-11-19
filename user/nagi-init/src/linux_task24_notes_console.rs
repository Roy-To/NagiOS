//! Development boot's interactive Notes surface after a real Task24 Session.

use super::super::linux_owner_registration_console::BootOwnerConsole;
use crate::profile_volume_notes_owner::AuthorizedNotesAttachment;
use e_profile_root_canonical_consumer::construct_notes_bridge_scoped;
use libnagi::{INPUT_KEY_ENTER, INPUT_KEY_ESCAPE};
use nagi_identity_session::{C1IssuedInteractiveSession, ResolvedIdentity};
use notes_files_codec_candidate::{BlockKind, NoteDocument};
use std::{io, sync::Arc, time::Duration};

pub(super) fn run(
    console: &mut BootOwnerConsole,
    (attachment, identity, issued): (
        AuthorizedNotesAttachment,
        ResolvedIdentity,
        Arc<C1IssuedInteractiveSession>,
    ),
) -> io::Result<()> {
    std::thread::scope(|scope| {
        let mut bridge = construct_notes_bridge_scoped(
            scope,
            attachment.bundle,
            identity,
            issued.caller(),
            attachment.authorizer,
            Duration::from_secs(5),
        )
        .map_err(io::Error::other)?;
        let locale = nagi_localization::Locale::EnUs;
        let label = nagi_localization::text(locale, "desktop.notes.title");
        if let Some(note) = bridge.list_notes().map_err(io::Error::other)?.first() {
            let readback = bridge.open_note(note.id).map_err(io::Error::other)?;
            super::super::log_pid1_stderr(format_args!(
                "NAGI_DEV_NOTES_REOPENED object={:016x} revision={}",
                readback.id.0, readback.revision
            ));
        } else {
            bridge.create_note(label).map_err(io::Error::other)?;
        }
        present(
            console,
            &bridge
                .current_note()
                .ok_or_else(|| io::Error::other("Notes document unavailable"))?,
        )?;
        super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_INPUT_READY"));
        loop {
            let event = console.next_key()?;
            let result = bridge.handle_event(event).map_err(io::Error::other)?;
            if let Some(note) = result {
                present(console, &note)?;
                if event.code == INPUT_KEY_ENTER {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_NOTES_SAVED object={:016x} revision={}",
                        note.id.0, note.revision
                    ));
                } else if event.code == INPUT_KEY_ESCAPE {
                    super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_CLOSED"));
                    return Ok(());
                }
            }
        }
    })
}

fn present(console: &mut BootOwnerConsole, note: &NoteDocument) -> io::Result<()> {
    use crate::ui::{rgba, Rect};
    let ink = rgba(26, 32, 44);
    console.present(|painter| {
        painter.fill(Rect::new(0, 0, 320, 200), rgba(250, 250, 247));
        painter.text(18, 18, note.title.as_bytes(), ink);
        let mut y = 40;
        for block in &note.blocks {
            if let BlockKind::Paragraph(text) = &block.kind {
                painter.text(18, y, text.as_bytes(), ink);
                y += 14;
                if y >= 186 {
                    break;
                }
            }
        }
    })
}
