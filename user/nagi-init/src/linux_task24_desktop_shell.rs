use super::super::{linux_owner_registration_console::BootOwnerConsole, NotesConsoleExit};
use crate::desktop_shell_model::{AppId, ShellAction, ShellModel};
use crate::ui::Painter;
use libnagi::InputEvent;
use nagi_identity_session::ResolutionError;
use notes_files_codec_candidate::NoteDocument;
use std::{io, time::Duration};

#[cfg(linux_task24_settings)]
#[path = "linux_task24_desktop_shell/settings.rs"]
mod settings;
#[cfg(linux_task24_settings)]
pub(super) use settings::SettingsRuntime;

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
    mut launch: impl FnMut(
        &mut C,
        nagi_localization::Locale,
    ) -> io::Result<(NotesConsoleExit, Option<NoteDocument>)>,
    drafts: &mut Vec<NoteDocument>,
    #[cfg(linux_task24_settings)] settings_runtime: Option<SettingsRuntime<'_>>,
) -> io::Result<ShellExit> {
    use super::super::{log_pid1_stderr, poll_task24_input, Task24InputPoll};
    #[cfg(linux_task24_settings)]
    use crate::settings_store::panel::SettingsAction;

    if poll_identity()? {
        return Ok(ShellExit::RequiredIdentityExited);
    }
    if check_session().is_err() {
        return Ok(ShellExit::StateUnknown);
    }
    let mut model = ShellModel::new(nagi_localization::Locale::EnUs);
    #[cfg(linux_task24_settings)]
    let mut settings = settings_runtime.map(settings::DesktopSettings::open);
    #[cfg(linux_task24_settings)]
    let mut panel: Option<crate::settings_store::panel::SettingsPanel> = None;
    #[cfg(linux_task24_settings)]
    let mut password: Option<crate::settings_store::password::PasswordPanel> = None;
    #[cfg(linux_task24_settings)]
    if let Some(settings) = &settings {
        settings.apply(&mut model);
    }

    console.present(|painter| model.render(painter))?;
    log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_READY"));

    loop {
        let input = match poll_task24_input(
            || console.next_key_timeout(Duration::from_millis(100)),
            &mut poll_identity,
        ) {
            Ok(input) => input,
            #[cfg(linux_task24_settings)]
            Err(error)
                if error.get_ref().is_some_and(|cause| {
                    cause.is::<super::super::linux_owner_registration_console::KeyboardEventsLost>()
                }) =>
            {
                let (Some(settings), Some(form)) = (&settings, &mut password) else {
                    return Err(error);
                };
                if check_session().is_err() {
                    return Ok(ShellExit::StateUnknown);
                }
                form.input_lost();
                console.present(|painter| settings.render_password(form, painter))?;
                log_pid1_stderr(format_args!("NAGI_DEV_PASSWORD_PANEL_INPUT_LOST"));
                continue;
            }
            Err(error) => return Err(error),
        };
        if matches!(input, Task24InputPoll::RequiredServiceExited) {
            return Ok(ShellExit::RequiredIdentityExited);
        }
        if check_session().is_err() {
            return Ok(ShellExit::StateUnknown);
        }
        let Task24InputPoll::Input(event) = input else {
            continue;
        };
        #[cfg(linux_task24_settings)]
        if let (Some(settings), Some(active)) = (&mut settings, &mut panel) {
            if let Some(form) = &mut password {
                let appearance = form.appearance();
                if settings.password_key(form, event) {
                    password = None;
                    console.present(|painter| active.render(painter))?;
                    log_pid1_stderr(format_args!("NAGI_DEV_PASSWORD_PANEL_CLOSED"));
                } else if appearance != form.appearance() {
                    console.present(|painter| settings.render_password(form, painter))?;
                }
                continue;
            }
            match active.on_key(event) {
                SettingsAction::ChangePassword => {
                    let form = crate::settings_store::password::PasswordPanel::new();
                    console.present(|painter| settings.render_password(&form, painter))?;
                    password = Some(form);
                    log_pid1_stderr(format_args!("NAGI_DEV_PASSWORD_PANEL_OPENED"));
                }
                SettingsAction::Ignored => {}
                SettingsAction::Close => {
                    panel = None;
                    console.present(|painter| model.render(painter))?;
                    log_pid1_stderr(format_args!("NAGI_DEV_SETTINGS_CLOSED"));
                }
                action => {
                    settings.update(action, active);
                    settings.apply(&mut model);
                    console.present(|painter| active.render(painter))?;
                }
            }
            continue;
        }
        match model.on_key(event) {
            ShellAction::Ignored => {}
            ShellAction::Redraw => console.present(|painter| model.render(painter))?,
            ShellAction::Logout => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_LOGOUT_REQUESTED"));
                return Ok(ShellExit::Logout);
            }
            #[cfg(linux_task24_settings)]
            ShellAction::Launch(AppId::Settings) => {
                if let Some(settings) = &settings {
                    let active = settings.panel();
                    console.present(|painter| active.render(painter))?;
                    panel = Some(active);
                    log_pid1_stderr(format_args!("NAGI_DEV_SETTINGS_OPENED"));
                }
            }
            ShellAction::Launch(AppId::Notes) => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_APP_LAUNCH app=notes"));
                let (exit, draft) = launch(console, model.locale())?;
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
            |console, _| {
                frames_at_launch.push(console.frames);
                Ok((NotesConsoleExit::ClosedByUser, None))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None,
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
            |_, _| {
                ended.set(true);
                Ok((NotesConsoleExit::ClosedWithoutSave, None))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None,
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
            |_, _| panic!("no launch"),
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::RequiredIdentityExited);
        assert_eq!(console.frames, 0);

        console.keys.push_back(INPUT_KEY_ENTER);
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            |_, _| Ok((NotesConsoleExit::StateUnknown, None)),
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::StateUnknown);
        assert_eq!(console.frames, 1);
    }
}
