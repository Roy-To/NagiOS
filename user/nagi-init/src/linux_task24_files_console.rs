//! Development-only adapter for the shared Files screen.

use super::super::files_app::{self, FilesAppConsole, Notice};
use super::desktop_shell::ShellConsole;
use crate::profile_volume_notes_owner::AuthorizedFilesRoot;
use std::{io, time::Duration};

pub(super) use super::super::files_app::FilesExit;

struct DevelopmentShellAdapter<'console, C>(&'console mut C);

impl<C: ShellConsole> FilesAppConsole for DevelopmentShellAdapter<'_, C> {
    fn present(&mut self, render: impl FnOnce(&mut crate::ui::Painter<'_>)) -> io::Result<()> {
        self.0.present(render)
    }

    fn next_key_timeout(&mut self, timeout: Duration) -> io::Result<Option<libnagi::InputEvent>> {
        self.0.next_key_timeout(timeout)
    }
}

pub(super) fn run<C: ShellConsole>(
    console: &mut C,
    owner: &AuthorizedFilesRoot,
    locale: nagi_localization::Locale,
    poll_identity: impl FnMut() -> io::Result<bool>,
    check_session: impl FnMut() -> Result<(), nagi_identity_session::ResolutionError>,
) -> io::Result<FilesExit> {
    let mut console = DevelopmentShellAdapter(console);
    files_app::run_with_console(
        &mut console,
        owner,
        locale,
        poll_identity,
        check_session,
        |notice| match notice {
            Notice::Ready => {
                crate::linux_init::log_pid1_stderr(format_args!("NAGI_DEV_FILES_READY"))
            }
            Notice::Operation(operation, result) => crate::linux_init::log_pid1_stderr(
                format_args!("NAGI_DEV_FILES_OP op={operation} result={result}"),
            ),
            Notice::Closed => {
                crate::linux_init::log_pid1_stderr(format_args!("NAGI_DEV_FILES_CLOSED"))
            }
        },
    )
}
