//! Development Files screen backed by the selected profile's authorized root.

use super::desktop_shell::ShellConsole;
use crate::linux_task24_files_keys::{FilesKeyAction, FilesKeys};
use crate::profile_volume_notes_owner::AuthorizedFilesRoot;
use crate::ui::{Painter, Rect};
use nagi_files::localization::{error_message, text, Locale as FilesLocale};
use nagi_files::{
    Actor, CapabilityAuthorizer, CapabilityRequest, CapabilityRight, FilesCanvas, FilesCanvasRect,
    FilesError, FilesErrorKind, FilesystemProvider, PermissionDecision,
};
use nagi_files_c1_session_consumer::{C1FilesScreenConsumer, FilesSessionEffectError};
use nagi_identity_session::ResolutionError;
use nagi_ui::{color, ColorRole, ThemeMode, TypeRole};
use std::{io, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FilesExit {
    Closed,
    SessionEnded,
    RequiredIdentityExited,
    StateUnknown,
}

/// PID 1's existing per-operation user-files policy. The owner scope limits
/// the provider to the selected profile root; FilesService asks this policy
/// separately for every Files right. This is intentionally not an app-wide
/// manifest grant and does not authorize writes or permanent deletion.
struct LinuxUserFilesAuthorizer;

impl CapabilityAuthorizer for LinuxUserFilesAuthorizer {
    fn decide(&self, request: &CapabilityRequest) -> PermissionDecision {
        use CapabilityRight::*;
        match request.right {
            Enumerate | Read | Create | Rename | Move | Delete | Restore => {
                PermissionDecision::Allow
            }
            Write | PermanentDelete | WorkspaceReference | SetMetadata => PermissionDecision::Deny,
        }
    }
}

struct PainterCanvas<'p, 'surface>(&'p mut Painter<'surface>);

impl FilesCanvas for PainterCanvas<'_, '_> {
    fn fill_rect(&mut self, bounds: FilesCanvasRect, role: ColorRole) {
        self.0.fill(painter_rect(bounds), pixel(role));
    }

    fn frame_rect(&mut self, bounds: FilesCanvasRect, role: ColorRole) {
        if bounds.width > 0 && bounds.height > 0 {
            self.0.frame(painter_rect(bounds), pixel(role));
        }
    }

    fn text(
        &mut self,
        bounds: FilesCanvasRect,
        value: &str,
        _typography: TypeRole,
        role: ColorRole,
    ) {
        if bounds.height < 7 || bounds.width < 7 {
            return;
        }
        let mut width = 0;
        let mut end = 0;
        for (offset, ch) in value.char_indices() {
            width += if ch.is_ascii() { 7 } else { 8 };
            if width > bounds.width {
                break;
            }
            end = offset + ch.len_utf8();
        }
        self.0
            .text(bounds.x, bounds.y, &value.as_bytes()[..end], pixel(role));
    }
}

fn painter_rect(bounds: FilesCanvasRect) -> Rect {
    Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

fn pixel(role: ColorRole) -> u32 {
    color(ThemeMode::Light, role).to_pixel()
}

pub(super) fn run<C: ShellConsole>(
    console: &mut C,
    owner: &AuthorizedFilesRoot,
    locale: nagi_localization::Locale,
    poll_identity: impl FnMut() -> io::Result<bool>,
    check_session: impl FnMut() -> Result<(), ResolutionError>,
) -> io::Result<FilesExit> {
    let files_locale = match locale {
        nagi_localization::Locale::EnUs => FilesLocale::EnUs,
        nagi_localization::Locale::JaJp => FilesLocale::JaJp,
    };
    // This consumer owns the sole Files operation boundary: it borrows the
    // already-authorized root scope and runs C1 revalidation per complete
    // FilesService operation. Do not wrap calls below in another C1 gate.
    let mut files = C1FilesScreenConsumer::from_authorized_root_scope(
        owner,
        LinuxUserFilesAuthorizer,
        nagi_files::Location::root(),
    );
    run_screen(
        console,
        &mut files,
        files_locale,
        poll_identity,
        check_session,
        |notice| {
            use super::super::log_pid1_stderr;
            match notice {
                Notice::Ready => log_pid1_stderr(format_args!("NAGI_DEV_FILES_READY")),
                Notice::Operation(op, result) => {
                    log_pid1_stderr(format_args!("NAGI_DEV_FILES_OP op={op} result={result}"))
                }
                Notice::Closed => log_pid1_stderr(format_args!("NAGI_DEV_FILES_CLOSED")),
            }
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Notice {
    Ready,
    Operation(&'static str, &'static str),
    Closed,
}

#[derive(Clone, Debug)]
struct FilePreview {
    name: String,
    lines: Vec<String>,
}

fn run_screen<C, P, A>(
    console: &mut C,
    files: &mut C1FilesScreenConsumer<'_, P, A>,
    locale: FilesLocale,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), ResolutionError>,
    mut notice: impl FnMut(Notice),
) -> io::Result<FilesExit>
where
    C: ShellConsole,
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    if poll_identity()? {
        return Ok(FilesExit::RequiredIdentityExited);
    }
    if let Some(exit) = session_exit(check_session()) {
        return Ok(exit);
    }
    if let Err(error) = files.refresh() {
        return Ok(effect_exit(error));
    }
    let mut keys = FilesKeys::default();
    let mut trash = Vec::new();
    let mut preview = None;
    let mut status = String::new();
    present(console, files, &keys, &trash, None, &status, locale)?;
    notice(Notice::Ready);

    loop {
        let input = console.next_key_timeout(Duration::from_millis(100))?;
        if poll_identity()? {
            return Ok(FilesExit::RequiredIdentityExited);
        }
        if let Some(exit) = session_exit(check_session()) {
            return Ok(exit);
        }
        let Some(event) = input else {
            continue;
        };

        if preview.is_some() {
            if event.event_type == libnagi::INPUT_EVENT_KEY && event.code == 1 && event.value == 1 {
                preview = None;
                status.clear();
                present(console, files, &keys, &trash, None, &status, locale)?;
            }
            continue;
        }

        let action = keys.on_event(event, files.state(), &trash);
        if matches!(&action, FilesKeyAction::Ignored) {
            continue;
        }
        if matches!(&action, FilesKeyAction::Close) {
            notice(Notice::Closed);
            return Ok(FilesExit::Closed);
        }

        let mut operation = None;
        let result: Result<(), FilesSessionEffectError> = match action {
            FilesKeyAction::Ignored | FilesKeyAction::Close => unreachable!(),
            FilesKeyAction::Redraw => Ok(()),
            FilesKeyAction::Select(id) => files
                .select(id, false)
                .map_err(FilesSessionEffectError::Files),
            FilesKeyAction::OpenFolder(location) => files.navigate(location),
            FilesKeyAction::OpenFile => {
                operation = Some("open");
                match files.open_selected(Actor::User) {
                    Ok(Some((bytes, _record))) => {
                        let name = files
                            .state()
                            .focused
                            .and_then(|id| {
                                files.state().entries.iter().find(|entry| entry.id == id)
                            })
                            .map(|entry| entry.name.as_str().to_owned())
                            .unwrap_or_default();
                        preview = Some(file_preview(name, &bytes));
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            FilesKeyAction::RejectSymlink => Err(FilesSessionEffectError::Files(FilesError::new(
                FilesErrorKind::SymlinkNotAllowed,
            ))),
            FilesKeyAction::Parent => files
                .state()
                .location
                .parent()
                .map_or(Ok(()), |parent| files.navigate(parent)),
            FilesKeyAction::CreateFolder(name) => {
                operation = Some("create_folder");
                files.create_folder(&name, Actor::User).map(|(entry, _)| {
                    if files.state().entries.iter().any(|row| row.id == entry.id) {
                        let _ = files.select(entry.id, false);
                    }
                })
            }
            FilesKeyAction::Rename(name) => {
                operation = Some("rename");
                files.rename_selected(&name, Actor::User).map(|_| ())
            }
            FilesKeyAction::Trash => {
                operation = Some("trash");
                files.trash_selected(Actor::User).map(|_| ())
            }
            FilesKeyAction::ShowTrash => {
                operation = Some("list_trash");
                files.list_trash().map(|rows| trash = rows)
            }
            FilesKeyAction::Restore(id) => {
                operation = Some("restore");
                restore(files, &mut keys, &trash, id)
            }
            FilesKeyAction::RestoreAs(id, parent, name) => {
                operation = Some("restore");
                files
                    .restore_trash_item_as(id, &parent, &name, Actor::User)
                    .map(|_| {
                        trash.retain(|entry| entry.id != id);
                    })
            }
        };

        status = match result {
            Ok(()) => {
                if let Some(operation) = operation {
                    notice(Notice::Operation(operation, "applied"));
                }
                String::new()
            }
            Err(FilesSessionEffectError::Session(error)) => {
                return Ok(match error {
                    ResolutionError::Ended | ResolutionError::Expired => FilesExit::SessionEnded,
                    _ => FilesExit::StateUnknown,
                });
            }
            Err(FilesSessionEffectError::Files(error)) => {
                if let Some(operation) = operation {
                    notice(Notice::Operation(operation, failure_label(error.kind)));
                }
                error_message(locale, &error)
            }
        };
        present(
            console,
            files,
            &keys,
            &trash,
            preview.as_ref(),
            &status,
            locale,
        )?;
    }
}

fn restore<P: FilesystemProvider, A: CapabilityAuthorizer>(
    files: &mut C1FilesScreenConsumer<'_, P, A>,
    keys: &mut FilesKeys,
    trash: &[nagi_files::TrashEntry],
    id: nagi_files::ResourceId,
) -> Result<(), FilesSessionEffectError> {
    let result = files.restore_trash_item(id, Actor::User).map(|_| ());
    if matches!(&result, Err(FilesSessionEffectError::Files(error)) if is_conflict(error.kind)) {
        if let Some(path) = trash
            .iter()
            .find(|row| row.id == id)
            .map(|row| &row.original_location)
        {
            if let Some((parent, name)) = path.parent().zip(
                path.file_name()
                    .and_then(|name| nagi_files::FileName::parse(name).ok()),
            ) {
                keys.begin_restore_as(id, parent, &name);
            }
        }
    }
    result
}

fn session_exit(result: Result<(), ResolutionError>) -> Option<FilesExit> {
    match result {
        Ok(()) => None,
        Err(ResolutionError::Ended | ResolutionError::Expired) => Some(FilesExit::SessionEnded),
        Err(_) => Some(FilesExit::StateUnknown),
    }
}

fn effect_exit(error: FilesSessionEffectError) -> FilesExit {
    match error {
        FilesSessionEffectError::Session(ResolutionError::Ended | ResolutionError::Expired) => {
            FilesExit::SessionEnded
        }
        FilesSessionEffectError::Session(_) | FilesSessionEffectError::Files(_) => {
            FilesExit::StateUnknown
        }
    }
}

fn is_conflict(kind: FilesErrorKind) -> bool {
    matches!(
        kind,
        FilesErrorKind::Conflict | FilesErrorKind::AlreadyExists
    )
}

fn failure_label(kind: FilesErrorKind) -> &'static str {
    use FilesErrorKind::*;
    match kind {
        Conflict | AlreadyExists => "conflict",
        PermissionDenied | PermissionRequired | SandboxEscape | SymlinkNotAllowed | ReadOnly => {
            "denied"
        }
        PartialFailure => "partial",
        _ => "failed",
    }
}

fn file_preview(name: String, bytes: &[u8]) -> FilePreview {
    let lines = std::str::from_utf8(bytes)
        .ok()
        .map(|text| {
            text.lines()
                .take(12)
                .map(|line| {
                    line.chars()
                        .filter(|character| !character.is_control())
                        .take(40)
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    FilePreview { name, lines }
}

fn present<C, P, A>(
    console: &mut C,
    files: &C1FilesScreenConsumer<'_, P, A>,
    keys: &FilesKeys,
    trash: &[nagi_files::TrashEntry],
    preview: Option<&FilePreview>,
    status: &str,
    locale: FilesLocale,
) -> io::Result<()>
where
    C: ShellConsole,
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    console.present(|painter| {
        let mut canvas = PainterCanvas(painter);
        canvas.fill_rect(FilesCanvasRect::new(0, 0, 320, 200), ColorRole::Canvas);
        if let Some(preview) = preview {
            draw_preview(&mut canvas, preview, locale);
        } else if keys.in_trash() {
            draw_trash(&mut canvas, trash, locale, keys);
        } else {
            files.draw(
                &mut canvas,
                FilesCanvasRect::new(0, 0, 320, 154),
                localized(locale, "files.empty"),
                locale,
            );
        }
        if let Some((name, invalid)) = keys.name_input() {
            line(&mut canvas, 160, &format!("Name: {name}"));
            if invalid {
                line(&mut canvas, 172, localized(locale, "files.error.invalid_name"));
            }
        } else if !status.is_empty() {
            line(&mut canvas, 172, status);
        }
        if preview.is_some() {
            line(&mut canvas, 188, "Esc close preview");
        } else if keys.in_trash() {
            line(&mut canvas, 188, "Esc return  Up/Down select  Enter restore");
        } else {
            line(
                &mut canvas,
                188,
                "Esc close  Up/Down select  Enter open  Backspace parent  N new  R rename  Del trash  T trash",
            );
        }
    })
}

fn draw_trash(
    canvas: &mut impl FilesCanvas,
    trash: &[nagi_files::TrashEntry],
    locale: FilesLocale,
    keys: &FilesKeys,
) {
    line(canvas, 5, localized(locale, "files.sidebar.trash"));
    if trash.is_empty() {
        line(canvas, 20, localized(locale, "files.empty"));
    }
    for (index, row) in trash.iter().take(10).enumerate() {
        let y = 20 + index as i32 * 15;
        if Some(row.id) == keys.trash_selection(trash) {
            canvas.frame_rect(FilesCanvasRect::new(5, y - 2, 310, 12), ColorRole::Focus);
        }
        line(canvas, y, &row.original_location.to_string());
    }
}

fn draw_preview(canvas: &mut impl FilesCanvas, preview: &FilePreview, locale: FilesLocale) {
    canvas.fill_rect(FilesCanvasRect::new(12, 10, 296, 174), ColorRole::Surface);
    canvas.frame_rect(FilesCanvasRect::new(12, 10, 296, 174), ColorRole::Border);
    line(canvas, 17, &preview.name);
    if preview.lines.is_empty() {
        line(canvas, 34, localized(locale, "files.kind.file"));
        return;
    }
    for (index, text) in preview.lines.iter().enumerate() {
        line(canvas, 34 + index as i32 * 11, text);
    }
}

fn localized(locale: FilesLocale, key: &'static str) -> &'static str {
    text(locale, key).unwrap_or(key)
}

fn line(canvas: &mut impl FilesCanvas, y: i32, value: &str) {
    canvas.text(
        FilesCanvasRect::new(8, y, 304, 9),
        value,
        TypeRole::Body,
        ColorRole::TextPrimary,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_files_policy_checks_each_right_and_denies_unlisted_effects() {
        let authorizer = LinuxUserFilesAuthorizer;
        let nested = nagi_files::Location::parse("folder/item.txt").unwrap();
        for right in [
            CapabilityRight::Enumerate,
            CapabilityRight::Read,
            CapabilityRight::Create,
            CapabilityRight::Rename,
            CapabilityRight::Move,
            CapabilityRight::Delete,
            CapabilityRight::Restore,
        ] {
            assert_eq!(
                authorizer.decide(&CapabilityRequest {
                    right,
                    location: nested.clone(),
                }),
                PermissionDecision::Allow,
                "expected PID 1 user-files policy to allow {right:?}"
            );
        }
        for right in [
            CapabilityRight::Write,
            CapabilityRight::PermanentDelete,
            CapabilityRight::WorkspaceReference,
            CapabilityRight::SetMetadata,
        ] {
            assert_eq!(
                authorizer.decide(&CapabilityRequest {
                    right,
                    location: nested.clone(),
                }),
                PermissionDecision::Deny,
                "expected PID 1 user-files policy to deny {right:?}"
            );
        }
    }

    #[test]
    fn preview_removes_control_characters_and_bounds_visible_lines() {
        let preview = file_preview(
            "note.txt".to_owned(),
            b"first\x00line\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\neighth\nninth\ntenth\neleventh\ntwelfth\nthirteenth",
        );
        assert_eq!(preview.lines.len(), 12);
        assert_eq!(preview.lines[0], "firstline");
        assert_eq!(preview.lines[11], "twelfth");
    }
}
