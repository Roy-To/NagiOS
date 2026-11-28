//! Reusable signed-in Settings session for the Linux product desktop.
//!
//! The desktop supplies an owner-issued Settings persistence capability and
//! callbacks for the PID 1 locale mirror and C1 credential change. This module
//! never opens a path or derives authority from a libnagi Session.

use super::{
    panel::{SettingsAction, SettingsPanel, SettingsStatus},
    preferences::{Preferences, PreferencesError, ReadPreferences},
};
use nagi_localization::Locale;
use nagi_persistence_contract::Revision;
use nagi_ui::ThemeMode;

#[cfg(all(target_os = "linux", linux_task24_settings))]
use super::password::PasswordPanel;
#[cfg(all(target_os = "linux", linux_task24_settings))]
use libnagi::InputEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasswordResult {
    Changed,
    WrongCurrent,
    Wait,
    Same,
    Invalid,
    Unavailable,
    OutcomeUnknown,
}

pub(crate) trait PreferencesPersistence {
    fn reload(&mut self) -> Result<ReadPreferences, PreferencesError>;
    fn save(&mut self, preferences: Preferences) -> Result<Revision, PreferencesError>;
}

#[cfg(all(target_os = "linux", linux_task24_initial_login_runtime))]
impl PreferencesPersistence for super::preferences::SettingsStore {
    fn reload(&mut self) -> Result<ReadPreferences, PreferencesError> {
        super::preferences::SettingsStore::reload(self)
    }

    fn save(&mut self, preferences: Preferences) -> Result<Revision, PreferencesError> {
        super::preferences::SettingsStore::save(self, preferences)
    }
}

/// Callbacks supplied by the signed-in desktop's existing C1 and Settings
/// owner. In particular, `open` must return an already-authorized app root.
pub(crate) struct SettingsRuntime<'a, S: PreferencesPersistence> {
    pub(crate) pre_login_locale: Locale,
    pub(crate) open: &'a mut dyn FnMut() -> Result<S, PreferencesError>,
    pub(crate) mirror_locale: &'a mut dyn FnMut(Locale) -> std::io::Result<()>,
    pub(crate) change_password: &'a mut dyn FnMut(&[u8], &[u8]) -> PasswordResult,
}

pub(crate) struct SettingsSession<'a, S: PreferencesPersistence> {
    runtime: SettingsRuntime<'a, S>,
    preferences: Preferences,
    store: Option<S>,
    status: SettingsStatus,
}

impl<'a, S: PreferencesPersistence> SettingsSession<'a, S> {
    pub(crate) fn open(runtime: SettingsRuntime<'a, S>) -> Self {
        let preferences = Preferences {
            locale: runtime.pre_login_locale,
            theme: ThemeMode::Light,
        };
        let mut settings = Self {
            runtime,
            preferences,
            store: None,
            status: SettingsStatus::Idle,
        };
        match settings.reload_source(false) {
            Ok(ReadPreferences::Restored { .. }) => {}
            Ok(ReadPreferences::Missing) => {}
            Err(_) => {
                settings.preferences.locale = Locale::EnUs;
                settings.status = SettingsStatus::ReloadRequired;
            }
        }
        settings
    }

    pub(crate) fn preferences(&self) -> Preferences {
        self.preferences
    }

    pub(crate) fn status(&self) -> SettingsStatus {
        self.status
    }

    /// Passes the request to the caller's already-authorized C1 operation.
    /// This session does not verify credentials or create authentication state.
    pub(crate) fn change_password(&mut self, current: &[u8], next: &[u8]) -> PasswordResult {
        (self.runtime.change_password)(current, next)
    }

    pub(crate) fn panel(&self) -> SettingsPanel {
        let mut panel = SettingsPanel::new(self.preferences);
        panel.show_save_error(self.status);
        panel
    }

    #[cfg(all(target_os = "linux", linux_task24_settings))]
    pub(crate) fn password_key(&mut self, panel: &mut PasswordPanel, event: InputEvent) -> bool {
        panel.on_key(event, &mut *self.runtime.change_password)
    }

    #[cfg(all(target_os = "linux", linux_task24_settings))]
    pub(crate) fn render_password(
        &self,
        panel: &PasswordPanel,
        painter: &mut crate::ui::Painter<'_>,
    ) {
        panel.render(painter, self.preferences.locale, self.preferences.theme);
    }

    /// Applies only persistence actions. The caller owns Close, ChangePassword,
    /// shell theme application, and rendering through the returned state.
    pub(crate) fn update(&mut self, action: SettingsAction, panel: &mut SettingsPanel) {
        match action {
            SettingsAction::Save(preferences) => self.save(preferences),
            SettingsAction::Reload => {
                if self.reload_source(true).is_err() {
                    self.status = SettingsStatus::ReloadRequired;
                }
            }
            _ => return,
        }
        panel.show_preferences(self.preferences);
        panel.show_save_error(self.status);
    }

    fn reload_source(
        &mut self,
        retry_locale_mirror: bool,
    ) -> Result<ReadPreferences, PreferencesError> {
        self.store = None;
        let mut store = (self.runtime.open)()?;
        let read = store.reload()?;
        self.store = Some(store);
        self.status = SettingsStatus::Idle;
        match read {
            ReadPreferences::Missing => {
                self.preferences = Preferences {
                    locale: self.runtime.pre_login_locale,
                    theme: ThemeMode::Light,
                };
            }
            ReadPreferences::Restored { preferences, .. } => {
                self.preferences = preferences;
                if retry_locale_mirror || preferences.locale != self.runtime.pre_login_locale {
                    self.mirror_locale();
                }
            }
        }
        Ok(read)
    }

    fn save(&mut self, preferences: Preferences) {
        let Some(store) = self.store.as_mut() else {
            self.status = SettingsStatus::ReloadRequired;
            return;
        };
        match store.save(preferences) {
            Ok(_) => {
                self.preferences = preferences;
                self.status = SettingsStatus::Idle;
                // Settings storage is canonical. Mirror on every successful
                // save, even when the locale itself did not change.
                self.mirror_locale();
            }
            Err(error) => {
                self.status = if matches!(
                    error,
                    PreferencesError::Conflict
                        | PreferencesError::OutcomeUnknown
                        | PreferencesError::ReloadRequired
                        | PreferencesError::RevisionMismatch
                ) {
                    SettingsStatus::ReloadRequired
                } else {
                    SettingsStatus::SaveFailed
                };
            }
        }
    }

    fn mirror_locale(&mut self) {
        if (self.runtime.mirror_locale)(self.preferences.locale).is_err() {
            self.status = SettingsStatus::LocaleSyncFailed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
    use nagi_persistence_contract::Revision;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[derive(Default)]
    struct Persisted {
        preferences: Option<Preferences>,
        writes: usize,
        fail_next_save: bool,
    }

    struct TestStore(Rc<RefCell<Persisted>>);

    impl PreferencesPersistence for TestStore {
        fn reload(&mut self) -> Result<ReadPreferences, PreferencesError> {
            Ok(match self.0.borrow().preferences {
                Some(preferences) => ReadPreferences::Restored {
                    preferences,
                    invalid_locale: false,
                    invalid_theme: false,
                },
                None => ReadPreferences::Missing,
            })
        }

        fn save(&mut self, preferences: Preferences) -> Result<Revision, PreferencesError> {
            let mut persisted = self.0.borrow_mut();
            persisted.writes += 1;
            if persisted.fail_next_save {
                persisted.fail_next_save = false;
                return Err(PreferencesError::Unavailable);
            }
            persisted.preferences = Some(preferences);
            Ok(Revision::new(persisted.writes as u64).unwrap())
        }
    }

    fn key(code: u16) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value: 1,
        }
    }

    fn runtime<'a>(
        locale: Locale,
        open: &'a mut dyn FnMut() -> Result<TestStore, PreferencesError>,
        mirror: &'a mut dyn FnMut(Locale) -> std::io::Result<()>,
        change: &'a mut dyn FnMut(&[u8], &[u8]) -> PasswordResult,
    ) -> SettingsRuntime<'a, TestStore> {
        SettingsRuntime {
            pre_login_locale: locale,
            open,
            mirror_locale: mirror,
            change_password: change,
        }
    }

    fn unavailable_password(_current: &[u8], _next: &[u8]) -> PasswordResult {
        PasswordResult::Unavailable
    }

    #[test]
    fn forwards_password_change_result_to_the_owner_callback() {
        let persisted = Rc::new(RefCell::new(Persisted::default()));
        let open_state = persisted.clone();
        let mut open = move || Ok(TestStore(open_state.clone()));
        let mut mirror = |_| Ok(());
        let returned = Cell::new(PasswordResult::Unavailable);
        let mut change = |current: &[u8], next: &[u8]| {
            assert_eq!(current, b"current");
            assert_eq!(next, b"next");
            returned.get()
        };
        let mut settings =
            SettingsSession::open(runtime(Locale::EnUs, &mut open, &mut mirror, &mut change));

        for outcome in [
            PasswordResult::Changed,
            PasswordResult::WrongCurrent,
            PasswordResult::Wait,
            PasswordResult::Same,
            PasswordResult::Invalid,
            PasswordResult::Unavailable,
            PasswordResult::OutcomeUnknown,
        ] {
            returned.set(outcome);
            assert_eq!(settings.change_password(b"current", b"next"), outcome);
        }
    }

    #[test]
    fn saves_locale_and_dark_theme_through_the_owner_store_and_reopens_them() {
        let persisted = Rc::new(RefCell::new(Persisted::default()));
        let mirrored = Rc::new(RefCell::new(Vec::new()));
        let fail_mirror = Rc::new(RefCell::new(false));
        let open_state = persisted.clone();
        let mut open = move || Ok(TestStore(open_state.clone()));
        let mirror_state = mirrored.clone();
        let mirror_failure = fail_mirror.clone();
        let mut mirror = move |locale| {
            mirror_state.borrow_mut().push(locale);
            if *mirror_failure.borrow() {
                Err(std::io::Error::other("fixture mirror failure"))
            } else {
                Ok(())
            }
        };
        let mut change = unavailable_password;
        let mut settings =
            SettingsSession::open(runtime(Locale::EnUs, &mut open, &mut mirror, &mut change));
        let mut panel = settings.panel();

        settings.update(panel.on_key(key(INPUT_KEY_TAB)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_ENTER)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_TAB)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_TAB)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_ENTER)), &mut panel);

        let expected = Preferences {
            locale: Locale::JaJp,
            theme: ThemeMode::Dark,
        };
        assert_eq!(settings.preferences(), expected);
        assert_eq!(persisted.borrow().preferences, Some(expected));
        assert_eq!(persisted.borrow().writes, 2);
        assert_eq!(*mirrored.borrow(), [Locale::JaJp, Locale::JaJp]);
        drop(settings);

        let open_state = persisted.clone();
        let mut open = move || Ok(TestStore(open_state.clone()));
        let mirror_state = mirrored.clone();
        let mut mirror = move |locale| {
            mirror_state.borrow_mut().push(locale);
            Ok(())
        };
        let mut change = unavailable_password;
        let reopened =
            SettingsSession::open(runtime(Locale::EnUs, &mut open, &mut mirror, &mut change));
        assert_eq!(reopened.preferences(), expected);
        assert_eq!(reopened.status(), SettingsStatus::Idle);
        assert_eq!(mirrored.borrow().last(), Some(&Locale::JaJp));
    }

    #[test]
    fn failed_save_does_not_update_the_pid1_locale_mirror() {
        let persisted = Rc::new(RefCell::new(Persisted {
            fail_next_save: true,
            ..Persisted::default()
        }));
        let mirrored = Rc::new(RefCell::new(Vec::new()));
        let open_state = persisted.clone();
        let mut open = move || Ok(TestStore(open_state.clone()));
        let mirror_state = mirrored.clone();
        let mut mirror = move |locale| {
            mirror_state.borrow_mut().push(locale);
            Ok(())
        };
        let mut change = unavailable_password;
        let mut settings =
            SettingsSession::open(runtime(Locale::EnUs, &mut open, &mut mirror, &mut change));
        let mut panel = settings.panel();
        settings.update(panel.on_key(key(INPUT_KEY_TAB)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_ENTER)), &mut panel);

        assert_eq!(persisted.borrow().preferences, None);
        assert_eq!(persisted.borrow().writes, 1);
        assert!(mirrored.borrow().is_empty());
        assert_eq!(settings.status(), SettingsStatus::SaveFailed);
        assert_eq!(settings.preferences().locale, Locale::EnUs);
    }

    #[test]
    fn locale_mirror_failure_is_visible_and_reload_retries_from_canonical_settings() {
        let persisted = Rc::new(RefCell::new(Persisted::default()));
        let mirrored = Rc::new(RefCell::new(Vec::new()));
        let fail_mirror = Rc::new(RefCell::new(true));
        let open_state = persisted.clone();
        let mut open = move || Ok(TestStore(open_state.clone()));
        let mirror_state = mirrored.clone();
        let mirror_failure = fail_mirror.clone();
        let mut mirror = move |locale| {
            mirror_state.borrow_mut().push(locale);
            if *mirror_failure.borrow() {
                Err(std::io::Error::other("fixture mirror failure"))
            } else {
                Ok(())
            }
        };
        let mut change = unavailable_password;
        let mut settings =
            SettingsSession::open(runtime(Locale::EnUs, &mut open, &mut mirror, &mut change));
        let mut panel = settings.panel();
        settings.update(panel.on_key(key(INPUT_KEY_TAB)), &mut panel);
        settings.update(panel.on_key(key(INPUT_KEY_ENTER)), &mut panel);
        assert_eq!(settings.status(), SettingsStatus::LocaleSyncFailed);
        assert_eq!(settings.preferences().locale, Locale::JaJp);

        *fail_mirror.borrow_mut() = false;
        for _ in 0..4 {
            panel.on_key(key(INPUT_KEY_TAB));
        }
        settings.update(panel.on_key(key(INPUT_KEY_ENTER)), &mut panel);

        assert_eq!(settings.status(), SettingsStatus::Idle);
        assert_eq!(persisted.borrow().writes, 1);
        assert_eq!(mirrored.borrow().as_slice(), [Locale::JaJp, Locale::JaJp]);
    }
}
