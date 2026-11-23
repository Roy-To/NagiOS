//! Development boot's interactive Notes surface after a real Task24 Session.

use super::super::{linux_owner_registration_console::BootOwnerConsole, NotesConsoleExit};
use crate::profile_volume_notes_owner::AuthorizedNotesAttachment;
use libnagi::INPUT_KEY_ESCAPE;
use nagi_identity_session::{C1IssuedInteractiveSession, ResolvedIdentity};
use notes_event_bridge_candidate::ScopedNotesEventBridge;
use notes_files_codec_candidate::{
    BlockKind, C1ProfileRootNotesInput, NoteDocument, ProfileRootNotesStore,
    ScopedNotesUiController,
};
use std::{io, sync::Arc, time::Duration};

pub(super) fn run(
    console: &mut BootOwnerConsole,
    (attachment, identity, issued): (
        AuthorizedNotesAttachment,
        ResolvedIdentity,
        Arc<C1IssuedInteractiveSession>,
    ),
    mut poll_required_identity_exit: impl FnMut() -> io::Result<bool>,
) -> io::Result<NotesConsoleExit> {
    std::thread::scope(|scope| {
        let input = C1ProfileRootNotesInput::open_linux(
            attachment.bundle,
            identity,
            issued.caller(),
            attachment.authorizer,
            Duration::from_secs(5),
        )
        .map_err(io::Error::other)?;
        let store = Arc::new(ProfileRootNotesStore::new(input));
        let controller = ScopedNotesUiController::new_scoped(Arc::clone(&store), scope);
        let mut bridge = ScopedNotesEventBridge::from_controller(controller);
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
        let mut has_unconfirmed_user_edit = false;
        loop {
            let Some(event) = super::super::wait_for_task24_input(
                |timeout| console.next_key_timeout(timeout),
                &mut poll_required_identity_exit,
            )?
            else {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_NOTES_STOPPED_REQUIRED_IDENTITY_EXIT"
                ));
                return Ok(NotesConsoleExit::RequiredIdentityExited);
            };
            let result = crate::task24_notes_input::handle_event(
                &mut bridge,
                store.as_ref(),
                event,
                &mut has_unconfirmed_user_edit,
            );
            let result = match result {
                Ok(result) => result,
                Err(error)
                    if crate::task24_notes_input::is_enter_code(event.code)
                        || event.code == INPUT_KEY_ESCAPE =>
                {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_NOTES_SAVE_FAILED event={} error={error}",
                        event.code
                    ));
                    continue;
                }
                Err(error) => return Err(io::Error::other(error)),
            };
            if let Some(note) = result {
                present(console, &note)?;
                if crate::task24_notes_input::is_enter_code(event.code) {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_NOTES_SAVED object={:016x} revision={}",
                        note.id.0, note.revision
                    ));
                } else if event.code == INPUT_KEY_ESCAPE {
                    super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_CLOSED"));
                    return Ok(NotesConsoleExit::ClosedByUser);
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
