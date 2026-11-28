use crate::desktop_shell_model::ShellModel;
use crate::settings_store::{
    panel::{SettingsAction, SettingsPanel, SettingsStatus},
    password::{PasswordPanel, PasswordResult},
    preferences::{
        Preferences, PreferencesError, ReadPreferences, SettingsSnapshotScope, SettingsStore,
    },
};
use nagi_localization::Locale;
use nagi_ui::ThemeMode;

pub(in crate::linux_init) struct SettingsRuntime<'a, S: SettingsSnapshotScope> {
    pub(in crate::linux_init) pre_login_locale: Locale,
    pub(in crate::linux_init) open:
        &'a mut dyn FnMut() -> Result<SettingsStore<S>, PreferencesError>,
    pub(in crate::linux_init) mirror: &'a mut dyn FnMut(Locale) -> std::io::Result<()>,
    pub(in crate::linux_init) change_password: &'a mut dyn FnMut(&[u8], &[u8]) -> PasswordResult,
}

pub(super) struct DesktopSettings<'a, S: SettingsSnapshotScope> {
    runtime: SettingsRuntime<'a, S>,
    preferences: Preferences,
    store: Option<SettingsStore<S>>,
    status: SettingsStatus,
}

impl<'a, S: SettingsSnapshotScope> DesktopSettings<'a, S> {
    pub(super) fn open(runtime: SettingsRuntime<'a, S>) -> Self {
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
        match settings.reload() {
            Ok(Some(false)) if settings.preferences.locale != settings.runtime.pre_login_locale => {
                settings.mirror();
            }
            Err(_) => {
                settings.preferences.locale = Locale::EnUs;
                settings.status = SettingsStatus::ReloadRequired;
            }
            _ => {}
        }
        settings
    }

    pub(super) fn panel(&self) -> SettingsPanel {
        let mut panel = SettingsPanel::new(self.preferences);
        panel.show_save_error(self.status);
        panel
    }

    pub(super) fn apply(&self, shell: &mut ShellModel) {
        shell.apply_preferences(self.preferences);
    }

    pub(super) fn password_key(
        &mut self,
        panel: &mut PasswordPanel,
        event: libnagi::InputEvent,
    ) -> bool {
        panel.on_key(event, &mut *self.runtime.change_password)
    }

    pub(super) fn render_password(
        &self,
        panel: &PasswordPanel,
        painter: &mut crate::ui::Painter<'_>,
    ) {
        panel.render(painter, self.preferences.locale, self.preferences.theme);
    }

    fn reload(&mut self) -> Result<Option<bool>, PreferencesError> {
        self.store = None;
        let mut store = (self.runtime.open)()?;
        let result = store.reload()?;
        self.store = Some(store);
        self.status = SettingsStatus::Idle;
        match result {
            ReadPreferences::Missing => {
                self.preferences = Preferences {
                    locale: self.runtime.pre_login_locale,
                    theme: ThemeMode::Light,
                };
                Ok(None)
            }
            ReadPreferences::Restored {
                preferences,
                invalid_locale,
                invalid_theme,
            } => {
                self.preferences = preferences;
                super::super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_SETTINGS_RESTORED locale={} theme={} invalid_locale={invalid_locale} invalid_theme={invalid_theme}",
                    preferences.locale.code(),
                    theme_name(preferences.theme),
                ));
                Ok(Some(invalid_locale))
            }
        }
    }

    fn mirror(&mut self) {
        if (self.runtime.mirror)(self.preferences.locale).is_err() {
            super::super::super::log_pid1_stderr(format_args!(
                "NAGI_DEV_SYSTEM_LOCALE_MIRROR_FAILED"
            ));
        }
    }

    pub(super) fn update(&mut self, action: SettingsAction, panel: &mut SettingsPanel) {
        let result = match action {
            SettingsAction::Save(preferences) => self.save(preferences),
            SettingsAction::Reload => self.reload().map(|_| ()),
            _ => return,
        };
        match result {
            Ok(()) => panel.show_preferences(self.preferences),
            Err(error) => {
                self.status = if matches!(action, SettingsAction::Reload)
                    || matches!(
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
                panel.show_save_error(self.status);
            }
        }
    }

    fn save(&mut self, preferences: Preferences) -> Result<(), PreferencesError> {
        let revision = self
            .store
            .as_mut()
            .ok_or(PreferencesError::ReloadRequired)?
            .save(preferences)?;
        self.preferences = preferences;
        self.status = SettingsStatus::Idle;
        self.mirror();
        super::super::super::log_pid1_stderr(format_args!(
            "NAGI_DEV_SETTINGS_SAVED revision={} locale={} theme={}",
            revision.get(),
            preferences.locale.code(),
            theme_name(preferences.theme),
        ));
        Ok(())
    }
}

fn theme_name(theme: ThemeMode) -> &'static str {
    match theme {
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    }
}
