use super::super::NotesConsoleExit;
#[cfg(all(feature = "linux-files-desktop", not(nagi_release_build)))]
use super::files_console::FilesExit;
#[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
use crate::calendar_console::CalendarExit;
use crate::desktop_shell_model::{AppId, ShellAction, ShellModel};
#[cfg(test)]
use crate::ui::Painter;
use crate::ui_console::NagiConsole;
#[cfg(test)]
use libnagi::InputEvent;
use nagi_identity_session::ResolutionError;
use notes_files_codec_candidate::NoteDocument;
use std::{io, time::Duration};

#[cfg(linux_task24_settings)]
#[path = "linux_task24_desktop_shell/settings.rs"]
mod settings;
#[cfg(linux_task24_settings)]
pub(super) use settings::SettingsRuntime;

pub(super) trait ShellConsole: NagiConsole {}

impl<T: NagiConsole> ShellConsole for T {}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ShellExit {
    Logout,
    ClosedWithoutSaveAfterSessionEnded,
    RequiredIdentityExited,
    StateUnknown,
}

pub(super) enum AppExit {
    Notes(NotesConsoleExit, Option<NoteDocument>),
    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
    Calendar(CalendarExit),
    #[cfg(all(feature = "linux-files-desktop", not(nagi_release_build)))]
    Files(FilesExit),
}

pub(super) fn run<C: ShellConsole>(
    console: &mut C,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), ResolutionError>,
    mut finish_logout: impl FnMut() -> io::Result<()>,
    mut launch: impl FnMut(AppId, &mut C, nagi_localization::Locale) -> io::Result<AppExit>,
    drafts: &mut Vec<NoteDocument>,
    #[cfg(linux_task24_settings)] settings_runtime: Option<
        SettingsRuntime<'_, impl crate::settings_store::preferences::SettingsSnapshotScope>,
    >,
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
    #[cfg(linux_task24_calendar_app)]
    log_pid1_stderr(format_args!("NAGI_DEV_LAUNCHER_LAYOUT calendar=enabled"));
    log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_READY"));

    loop {
        let input = poll_task24_input(
            || console.next_key_timeout(Duration::from_millis(100)),
            &mut poll_identity,
        );
        #[cfg(linux_task24_settings)]
        let input = match input {
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
            input => input?,
        };
        #[cfg(not(linux_task24_settings))]
        let input = input?;
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
                // A successful result is the C2 handoff point: the caller must
                // invalidate the exact C1 Session and every app-storage owner
                // before returning to unlock. Release wiring must also attach
                // the C3 app owner and complete exact child retirement before
                // it can return to unlock; this Task24 path has no such owner.
                finish_logout()?;
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
            #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
            ShellAction::Launch(AppId::Calendar) => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_APP_LAUNCH app=calendar"));
                match launch(AppId::Calendar, console, model.locale())? {
                    AppExit::Calendar(CalendarExit::ClosedByUser) => {
                        if poll_identity()? {
                            return Ok(ShellExit::RequiredIdentityExited);
                        }
                        match check_session() {
                            Ok(()) => {}
                            Err(ResolutionError::Ended | ResolutionError::Expired) => {
                                log_pid1_stderr(format_args!(
                                    "NAGI_DEV_DESKTOP_APP_CLOSED app=calendar exit=SessionEnded"
                                ));
                                return Ok(ShellExit::ClosedWithoutSaveAfterSessionEnded);
                            }
                            Err(_) => return Ok(ShellExit::StateUnknown),
                        }
                        console.present(|painter| model.render(painter))?;
                        log_pid1_stderr(format_args!(
                            "NAGI_DEV_DESKTOP_APP_CLOSED app=calendar exit=ClosedByUser"
                        ));
                    }
                    AppExit::Calendar(CalendarExit::SessionEnded) => {
                        log_pid1_stderr(format_args!(
                            "NAGI_DEV_DESKTOP_APP_CLOSED app=calendar exit=SessionEnded"
                        ));
                        return Ok(ShellExit::ClosedWithoutSaveAfterSessionEnded);
                    }
                    AppExit::Calendar(CalendarExit::RequiredIdentityExited) => {
                        return Ok(ShellExit::RequiredIdentityExited);
                    }
                    AppExit::Calendar(CalendarExit::StateUnknown) | AppExit::Notes(..) => {
                        return Ok(ShellExit::StateUnknown);
                    }
                    #[cfg(all(feature = "linux-files-desktop", not(nagi_release_build)))]
                    AppExit::Files(_) => return Ok(ShellExit::StateUnknown),
                }
            }
            #[cfg(all(feature = "linux-files-desktop", not(nagi_release_build)))]
            ShellAction::Launch(AppId::Files) => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_APP_LAUNCH app=files"));
                match launch(AppId::Files, console, model.locale())? {
                    AppExit::Files(FilesExit::Closed) => {
                        if poll_identity()? {
                            return Ok(ShellExit::RequiredIdentityExited);
                        }
                        match check_session() {
                            Ok(()) => {}
                            Err(ResolutionError::Ended | ResolutionError::Expired) => {
                                log_pid1_stderr(format_args!(
                                    "NAGI_DEV_DESKTOP_APP_CLOSED app=files exit=SessionEnded"
                                ));
                                return Ok(ShellExit::ClosedWithoutSaveAfterSessionEnded);
                            }
                            Err(_) => return Ok(ShellExit::StateUnknown),
                        }
                        console.present(|painter| model.render(painter))?;
                        log_pid1_stderr(format_args!(
                            "NAGI_DEV_DESKTOP_APP_CLOSED app=files exit=ClosedByUser"
                        ));
                    }
                    AppExit::Files(FilesExit::SessionEnded) => {
                        log_pid1_stderr(format_args!(
                            "NAGI_DEV_DESKTOP_APP_CLOSED app=files exit=SessionEnded"
                        ));
                        return Ok(ShellExit::ClosedWithoutSaveAfterSessionEnded);
                    }
                    AppExit::Files(FilesExit::RequiredIdentityExited) => {
                        return Ok(ShellExit::RequiredIdentityExited);
                    }
                    AppExit::Files(FilesExit::StateUnknown) => {
                        return Ok(ShellExit::StateUnknown);
                    }
                    AppExit::Notes(..) => return Ok(ShellExit::StateUnknown),
                    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
                    AppExit::Calendar(_) => return Ok(ShellExit::StateUnknown),
                }
            }
            ShellAction::Launch(AppId::Notes) => {
                log_pid1_stderr(format_args!("NAGI_DEV_DESKTOP_APP_LAUNCH app=notes"));
                let (exit, draft) = match launch(AppId::Notes, console, model.locale())? {
                    AppExit::Notes(exit, draft) => (exit, draft),
                    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
                    AppExit::Calendar(_) => return Ok(ShellExit::StateUnknown),
                    #[cfg(all(feature = "linux-files-desktop", not(nagi_release_build)))]
                    AppExit::Files(_) => return Ok(ShellExit::StateUnknown),
                };
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

    #[cfg(linux_task24_settings)]
    type NoSettingsRuntime =
        SettingsRuntime<'static, crate::profile_volume_notes_owner::AuthorizedProfileSnapshot>;

    #[derive(Default)]
    struct FakeConsole {
        keys: VecDeque<u16>,
        frames: usize,
    }

    impl NagiConsole for FakeConsole {
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

    fn key(code: u16) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value: 1,
        }
    }

    fn keys_to_logout(model: &mut ShellModel) -> Vec<u16> {
        let mut keys = Vec::new();
        loop {
            assert_eq!(model.on_key(key(INPUT_KEY_TAB)), ShellAction::Redraw);
            keys.push(INPUT_KEY_TAB);
            let action = model.on_key(key(INPUT_KEY_ENTER));
            if action == ShellAction::Logout {
                keys.push(INPUT_KEY_ENTER);
                break keys;
            }
            assert!(matches!(action, ShellAction::Launch(_)));
        }
    }

    fn logout_keys() -> Vec<u16> {
        keys_to_logout(&mut ShellModel::new(nagi_localization::Locale::EnUs))
    }

    fn logout_tab_count() -> usize {
        logout_keys().len() - 1
    }

    #[test]
    fn starts_notes_twice_before_logout() {
        let mut console = FakeConsole {
            keys: [INPUT_KEY_ENTER, INPUT_KEY_ENTER]
                .into_iter()
                .chain(logout_keys())
                .collect(),
            ..Default::default()
        };
        let mut frames_at_launch = Vec::new();
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            || Ok(()),
            |app, console, _locale| {
                assert_eq!(app, AppId::Notes);
                frames_at_launch.push(console.frames);
                Ok(AppExit::Notes(NotesConsoleExit::ClosedByUser, None))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::Logout);
        assert_eq!(frames_at_launch, [1, 2]);
        assert_eq!(console.frames, 3 + logout_tab_count());
    }

    #[test]
    fn normal_logout_runs_c1_and_storage_cleanup_before_returning() {
        use std::cell::Cell;

        let cleanup_calls = Cell::new(0);
        let session_checked = Cell::new(false);
        let app_closed = Cell::new(false);
        let mut console = FakeConsole {
            keys: std::iter::once(INPUT_KEY_ENTER)
                .chain(logout_keys())
                .collect(),
            ..Default::default()
        };
        let exit = run(
            &mut console,
            || Ok(false),
            || {
                session_checked.set(true);
                Ok(())
            },
            || {
                assert!(session_checked.get());
                assert!(app_closed.get());
                cleanup_calls.set(cleanup_calls.get() + 1);
                Ok(())
            },
            |app, _, _locale| {
                assert_eq!(app, AppId::Notes);
                app_closed.set(true);
                Ok(AppExit::Notes(NotesConsoleExit::ClosedByUser, None))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();

        assert_eq!(exit, ShellExit::Logout);
        assert_eq!(cleanup_calls.get(), 1);
        assert!(app_closed.get());
    }

    #[test]
    fn failed_normal_logout_cleanup_never_returns_logout_success() {
        use std::cell::Cell;

        let cleanup_calls = Cell::new(0);
        let mut console = FakeConsole {
            keys: logout_keys().into(),
            ..Default::default()
        };
        let result = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            || {
                cleanup_calls.set(cleanup_calls.get() + 1);
                Err(io::Error::other("cleanup not confirmed"))
            },
            |_, _, _locale| panic!("failed logout must not launch another app"),
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        );

        assert!(result.is_err());
        assert_eq!(cleanup_calls.get(), 1);
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
            || Ok(()),
            |_, _, _locale| {
                ended.set(true);
                Ok(AppExit::Notes(NotesConsoleExit::ClosedWithoutSave, None))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
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
            || panic!("logout must not run after required Identity exit"),
            |_, _, _locale| panic!("no launch"),
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::RequiredIdentityExited);
        assert_eq!(console.frames, 0);

        console.keys.push_back(INPUT_KEY_ENTER);
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            || panic!("unknown app exit must not run logout cleanup"),
            |_, _, _locale| Ok(AppExit::Notes(NotesConsoleExit::StateUnknown, None)),
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::StateUnknown);
        assert_eq!(console.frames, 1);
    }

    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
    #[test]
    fn calendar_close_returns_to_the_launcher_and_logout_stays_available() {
        let mut model = ShellModel::new(nagi_localization::Locale::EnUs);
        assert_eq!(model.on_key(key(INPUT_KEY_TAB)), ShellAction::Redraw);
        assert_eq!(
            model.on_key(key(INPUT_KEY_ENTER)),
            ShellAction::Launch(AppId::Calendar)
        );
        let logout_keys = keys_to_logout(&mut model);
        let mut console = FakeConsole {
            keys: [INPUT_KEY_TAB, INPUT_KEY_ENTER]
                .into_iter()
                .chain(logout_keys.iter().copied())
                .collect(),
            ..Default::default()
        };
        let mut launched = Vec::new();
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            || Ok(()),
            |app, _, _locale| {
                launched.push(app);
                Ok(AppExit::Calendar(CalendarExit::ClosedByUser))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();
        assert_eq!(launched, [AppId::Calendar]);
        assert_eq!(exit, ShellExit::Logout);
        assert_eq!(console.frames, 3 + logout_keys.len() - 1);
    }

    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
    #[test]
    fn ended_calendar_session_exits_without_resuming_the_launcher() {
        let mut console = FakeConsole {
            keys: [INPUT_KEY_TAB, INPUT_KEY_ENTER].into(),
            ..Default::default()
        };
        let exit = run(
            &mut console,
            || Ok(false),
            || Ok(()),
            || panic!("session-ended app exit must not run normal logout cleanup"),
            |app, _, _locale| {
                assert_eq!(app, AppId::Calendar);
                Ok(AppExit::Calendar(CalendarExit::SessionEnded))
            },
            &mut Vec::new(),
            #[cfg(linux_task24_settings)]
            None::<NoSettingsRuntime>,
        )
        .unwrap();
        assert_eq!(exit, ShellExit::ClosedWithoutSaveAfterSessionEnded);
        assert_eq!(console.frames, 2);
    }
}
