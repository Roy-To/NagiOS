//! Development boot's interactive Notes surface after a real Task24 Session.

use super::super::{linux_owner_registration_console::BootOwnerConsole, NotesConsoleExit};
use crate::profile_volume_notes_owner::AuthorizedNotesAttachment;
use crate::task24_notes_input::{is_escape_press, UnsavedCloseConfirmation};
#[cfg(all(linux_task24_owner_co_location, linux_dev_notes_performance))]
use crate::task24_notes_performance::{self, MemorySampler};
use libnagi::INPUT_KEY_ESCAPE;
use nagi_identity_session::{C1IssuedInteractiveSession, ResolutionError, ResolvedIdentity};
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
    mut check_session: impl FnMut() -> Result<(), nagi_identity_session::ResolutionError>,
    #[cfg(all(linux_dev_ws10, not(nagi_release_build)))] mut dev_action: impl FnMut() -> io::Result<
        Option<NotesConsoleExit>,
    >,
) -> io::Result<(NotesConsoleExit, Option<NoteDocument>)> {
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
            false,
        )?;
        #[cfg(all(linux_task24_owner_co_location, linux_dev_notes_performance))]
        let mut memory = MemorySampler::default();
        super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_INPUT_READY"));
        let mut has_unconfirmed_user_edit = false;
        let mut confirmation = UnsavedCloseConfirmation::default();
        loop {
            let Some(key) = super::super::wait_for_task24_input(
                |timeout| {
                    #[cfg(all(linux_task24_owner_co_location, linux_dev_notes_performance))]
                    if let Some(line) = memory.sample_if_due() {
                        super::super::log_pid1_stderr(format_args!("{line}"));
                    }
                    console.next_console_key_timeout(timeout)
                },
                &mut poll_required_identity_exit,
            )?
            else {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_NOTES_STOPPED_REQUIRED_IDENTITY_EXIT"
                ));
                return Ok((NotesConsoleExit::RequiredIdentityExited, None));
            };
            let event = key.event;
            let had_confirmation = confirmation.pending();
            if confirmation.on_input(event) {
                return close_unsaved(&bridge);
            }
            if had_confirmation && !confirmation.pending() {
                if let Some(draft) = bridge.current_note() {
                    present(console, &draft, false)?;
                }
            }
            #[cfg(all(linux_dev_ws10, not(nagi_release_build)))]
            if event.code == 66 && event.value == 1 {
                // Linux evdev KEY_F8. Only the explicitly selected dev mode acts.
                if let Some(exit) = dev_action()? {
                    return Ok((exit, None));
                }
                continue;
            }
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
                    if is_escape_press(event) {
                        let permanently_denied = matches!(
                            check_session(),
                            Err(ResolutionError::Ended | ResolutionError::Expired)
                        );
                        if confirmation.after_failed_escape(permanently_denied) {
                            return close_unsaved(&bridge);
                        }
                        // Do not arm a destructive confirmation if the
                        // operator could not see it.
                        if !bridge
                            .current_note()
                            .is_some_and(|draft| present(console, &draft, true).is_ok())
                        {
                            confirmation = UnsavedCloseConfirmation::default();
                            super::super::log_pid1_stderr(format_args!(
                                "NAGI_DEV_NOTES_CLOSE_CONFIRMATION_UNAVAILABLE"
                            ));
                        }
                    }
                    continue;
                }
                Err(error) => return Err(io::Error::other(error)),
            };
            if let Some(note) = result {
                present(console, &note, false)?;
                #[cfg(all(linux_task24_owner_co_location, linux_dev_notes_performance))]
                let presented_at = key
                    .monotonic_ns
                    .and_then(|_| task24_notes_performance::monotonic_ns());
                if crate::task24_notes_input::is_enter_code(event.code) {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_NOTES_SAVED object={:016x} revision={}",
                        note.id.0, note.revision
                    ));
                } else if event.code == INPUT_KEY_ESCAPE {
                    super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_CLOSED"));
                    return Ok((NotesConsoleExit::ClosedByUser, None));
                } else {
                    // For text input, Some means the bridge changed the document.
                    #[cfg(all(linux_task24_owner_co_location, linux_dev_notes_performance))]
                    if let Some(line) = task24_notes_performance::presentation_record(
                        key.monotonic_ns,
                        presented_at,
                    ) {
                        super::super::log_pid1_stderr(format_args!("{line}"));
                    }
                }
            }
        }
    })
}

fn close_unsaved(
    bridge: &ScopedNotesEventBridge<'_>,
) -> io::Result<(NotesConsoleExit, Option<NoteDocument>)> {
    super::super::log_pid1_stderr(format_args!("NAGI_DEV_NOTES_CLOSED_UNSAVED"));
    Ok((NotesConsoleExit::ClosedWithoutSave, bridge.current_note()))
}

fn present(
    console: &mut BootOwnerConsole,
    note: &NoteDocument,
    confirm_unsaved: bool,
) -> io::Result<()> {
    use crate::ui::{rgba, Rect};
    let ink = rgba(26, 32, 44);
    console.present(|painter| {
        painter.fill(Rect::new(0, 0, 320, 200), rgba(250, 250, 247));
        painter.text(18, 18, note.title.as_bytes(), ink);
        let mut y = 40;
        let body_bottom = if confirm_unsaved { 158 } else { 186 };
        for block in &note.blocks {
            if let BlockKind::Paragraph(text) = &block.kind {
                painter.text(18, y, text.as_bytes(), ink);
                y += 14;
                if y >= body_bottom {
                    break;
                }
            }
        }
        if confirm_unsaved {
            let locale = nagi_localization::Locale::EnUs;
            for (y, key) in [
                (162, "desktop.notes.close_save_failed"),
                (176, "desktop.notes.close_unsaved_hint"),
            ] {
                painter.text(18, y, nagi_localization::text(locale, key).as_bytes(), ink);
            }
        }
    })
}
