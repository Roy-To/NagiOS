use super::super::{NotesConsoleExit, linux_owner_registration_console::BootOwnerConsole};
use crate::desktop_shell_model::{AppId, ShellAction, ShellModel};
use crate::ui::Painter;
use libnagi::InputEvent;
use nagi_identity_session::ResolutionError;
use notes_files_codec_candidate::NoteDocument;
use std::{io, time::Duration};

pub(super) trait ShellConsole {
    fn present(&mut self, render: impl FnOnce(&mut Painter<'_>)) -> io::Result<()>;
    fn next_key_timeout(&mut self, timeout: Duration) -> io::Result<Option<InputEvent>>;
}

impl ShellConsole for BootOwnerConsole {
    fn present(&mut self, render: impl FnOnce(&mut Painter<'_>)) -> io::Result<()> {
        BootOwnerConsole::present(self, render)
    }

    fn next_key_timeout(&mut self, timeout: Duration) -> io::Result<Option<InputEvent>> {
        Ok(self.next_console_key_timeout(timeout)?.map(|key| key.event))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ShellExit {
    Logout,
    ClosedWithoutSaveAfterSessionEnded,
    RequiredIdentityExited,
    StateUnknown,
}

pub(super) fn run<C: ShellConsole>(
    console: &mut C,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), ResolutionError>,
    mut launch: impl FnMut(&mut C) -> io::Result<(NotesConsoleExit, Option<NoteDocument>)>,
    drafts: &mut Vec<NoteDocument>,
) -> io::Result<ShellExit> {
    use super::super::{Task24InputPoll, log_pid1_stderr, poll_task24_input};

    if poll_identity()? {
        return Ok(ShellExit::RequiredIdentityExited);
    }
    if check_session().is_err() {
        return Ok(ShellExit::StateUnknown);
    }
    let mut model = ShellModel::new(nagi_localization::Locale::EnUs);
    console.present(|painter| model.render(painter))?;
    log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_READY"));

    loop {
        let input = poll_task24_input(
            || console.next_key_timeout(Duration::from_millis(100)),
            &mut poll_identity,
        )?;
        if matches!(input, Task24InputPoll::RequiredServiceExited) {
            return Ok(ShellExit::RequiredIdentityExited);
        }
        if check_session().is_err() {
            return Ok(ShellExit::StateUnknown);
        }
        let Task24InputPoll::Input(event) = input else {
            continue;
        };
        match model.on_key(event) {
            ShellAction::Ignored => {}
            ShellAction::Redraw => console.present(|painter| model.render(painter))?,
            ShellAction::Logout => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_LOGOUT_REQUESTED"));
                return Ok(ShellExit::Logout);
            }
            ShellAction::Launch(AppId::Notes) => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_APP_LAUNCH app=notes"));
                let (exit, draft) = launch(console)?;
                if let Some(draft) = draft {
                    drafts.push(draft);
                }
                match exit {
                    NotesConsoleExit::ClosedByUser | NotesConsoleExit::ClosedWithoutSave => {
                        if poll_identity()? {
                            return Ok(ShellExit::RequiredIdentityExited);
                        }
                        match check_session() {
                            Ok(()) => {}
                            Err(ResolutionError::Ended | ResolutionError::Expired)
                                if matches!(exit, NotesConsoleExit::ClosedWithoutSave) =>
                            {
                                return Ok(ShellExit::ClosedWithoutSaveAfterSessionEnded);
                            }
                            Err(_) => return Ok(ShellExit::StateUnknown),
                        }
                        console.present(|painter| model.render(painter))?;
                        let reason = match exit {
                            NotesConsoleExit::ClosedByUser => "ClosedByUser",
                            _ => "ClosedWithoutSave",
                        };
                        log_pid1_stderr(format_args!(
                            "NAGI_DEV_DESKTOP_APP_CLOSED app=notes exit={reason}"
                        ));
                    }
                    NotesConsoleExit::RequiredIdentityExited => {
                        return Ok(ShellExit::RequiredIdentityExited);
                    }
                    NotesConsoleExit::StateUnknown => return Ok(ShellExit::StateUnknown),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::{INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
    use std::collections::VecDeque;

    #[derive(Default)]
    struct FakeConsole {
        keys: VecDeque<u16>,
        frames: usize,
    }

    impl ShellConsole for FakeConsole {
        fn present(&mut self, render: impl FnOnce(&mut Painter<'_>)) -> io::Result<()> {
            render(&mut Painter::new(&mut vec![0; 320 * 200]));
            self.frames += 1;
            Ok(())
        }

        fn next_key_timeout(&mut self, _: Duration) -> io::Result<Option<InputEvent>> {
            let code = self.keys.pop_front().expect("test key sequence exhausted");
            Ok(Some(InputEvent {
                event_type: INPUT_EVENT_KEY,
                code,
                value: 1,
            }))
        }
    }

    #[test]
    fn starts_notes_twice_before_logout() {
        let mut console = FakeConsole {
            keys: [
                INPUT_KEY_ENTER,
                INPUT_KEY_ENTER,
                INPUT_KEY_TAB,
                INPUT_KEY_ENTER,
            ]
            .into(),
            ..Default::default()
        };
        let mut frames_at_launch = Vec::new();
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            |console| {
                frames_at_launch.push(console.frames);
                Ok((NotesConsoleExit::ClosedByUser, None))
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(exit, ShellExit::Logout);
        assert_eq!(frames_at_launch, [1, 2]);
        assert_eq!(console.frames, 4);
    }

    #[test]
    fn permits_existing_cleanup_after_unsaved_close_of_ended_session() {
        use std::cell::Cell;

        let ended = Cell::new(false);
        let mut console = FakeConsole {
            keys: [INPUT_KEY_ENTER].into(),
            ..Default::default()
        };
        let exit = run(
            &mut console,
            || Ok(false),
            || {
                if ended.get() {
                    Err(ResolutionError::Ended)
                } else {
                    Ok(())
                }
            },
            |_| {
                ended.set(true);
                Ok((NotesConsoleExit::ClosedWithoutSave, None))
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(exit, ShellExit::ClosedWithoutSaveAfterSessionEnded);
        assert_eq!(console.frames, 1);
    }

    #[test]
    fn service_exit_and_unknown_notes_exit_stop_the_shell() {
        let mut console = FakeConsole::default();
        let exit = run(
            &mut console,
            || Ok(true),
            || Ok(()),
            |_| panic!("no launch"),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(exit, ShellExit::RequiredIdentityExited);
        assert_eq!(console.frames, 0);

        console.keys.push_back(INPUT_KEY_ENTER);
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            |_| Ok((NotesConsoleExit::StateUnknown, None)),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(exit, ShellExit::StateUnknown);
        assert_eq!(console.frames, 1);
    }
}
