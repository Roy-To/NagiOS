//! E-side UI consumer for Task27's real Settings adapter, given an existing
//! provider handle. The caller must obtain that handle from its authorized
//! owner/broker; this adapter and module do not authorize, mount, or open it.

use nagi_storage_consumer_adapters::{
    SettingsSaveFailure, SettingsSaveOutcome, SettingsStorageAdapter,
};
use nagi_storage_provider::{StorageProvider, StorageProviderError};

/// Last provider result presented separately from the locale currently in use.
/// A later VFS read after `Unknown` is observational only and cannot promote it
/// to a durability acknowledgement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsCommitDisplay<L> {
    NotAttempted,
    Saved {
        selected: L,
    },
    Old {
        selected: L,
    },
    Unknown {
        selected: L,
    },
    Failed {
        selected: L,
        reported_outcome: SettingsSaveOutcome,
    },
}

/// UI snapshot keeps three facts separate: the locale the app is currently
/// running with, the locale the Settings selection should display from the
/// latest explicit read, and the result of the last save attempt. After an
/// Unknown save, explicit readback may update the Settings selection without
/// changing the running locale or claiming that the save was durable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettingsDisplay<L> {
    pub running_locale: Option<L>,
    pub settings_selection_locale: Option<L>,
    pub last_read_locale: Option<L>,
    pub last_readback: Option<SettingsReadback<L>>,
    pub commit: SettingsCommitDisplay<L>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsUiState {
    Unloaded,
    Loaded,
    Missing,
    Invalid,
    LoadUnavailable,
    Saved,
    SaveOld,
    SaveUnknown,
    SaveUnavailable,
}

/// What the last explicit Settings read observed. A read after an Unknown
/// save remains observational and is not a durability acknowledgment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsReadback<L> {
    Missing,
    Restored(L),
    ObservedAfterUnknown(L),
    Invalid,
    Unavailable,
}

#[derive(Debug)]
pub enum LocaleLoadResult<L> {
    Loaded(L),
    /// A live value observed after an Unknown save; it does not settle commit
    /// durability or change the locale already running in the UI.
    ObservedAfterUnknown(L),
    Missing,
    Invalid,
    Unavailable(StorageProviderError),
}

/// Single E-side consumer of Task27's actual adapter. It stores the adapter
/// supplied by the authorized provider owner; it cannot mount or clone a VFS.
pub struct SystemLanguageSettings<L> {
    adapter: SettingsStorageAdapter,
    active_locale: Option<L>,
    last_read_locale: Option<L>,
    last_readback: Option<SettingsReadback<L>>,
    state: SettingsUiState,
    commit_display: SettingsCommitDisplay<L>,
}

impl<L: Copy> SystemLanguageSettings<L> {
    pub fn new(adapter: SettingsStorageAdapter) -> Self {
        Self {
            adapter,
            active_locale: None,
            last_read_locale: None,
            last_readback: None,
            state: SettingsUiState::Unloaded,
            commit_display: SettingsCommitDisplay::NotAttempted,
        }
    }

    /// Broker-internal consumer wiring only. The supplied handle must remain
    /// inside the same trusted broker boundary as its StorageProviderOwner.
    /// This is not an application grant or an authenticated per-operation
    /// client and must never be put in a Notes child bootstrap. It does not
    /// authorize a profile, open a device, or mount a second provider.
    pub fn from_provider_handle(storage: StorageProvider) -> Self {
        Self::new(SettingsStorageAdapter::new(storage))
    }

    /// Read once through Task27's real adapter and parse its current locale.
    /// After an Unknown save, the result is explicitly observational: it may
    /// update `last_read_locale`, but cannot clear the warning or switch the
    /// running locale.
    pub fn load(&mut self, parse: impl FnOnce(&[u8]) -> Option<L>) -> LocaleLoadResult<L> {
        let preserve_unknown = matches!(
            self.commit_display,
            SettingsCommitDisplay::Unknown { .. }
                | SettingsCommitDisplay::Failed {
                    reported_outcome: SettingsSaveOutcome::Unknown,
                    ..
                }
        );

        match self.adapter.load_system_language() {
            Ok(Some(bytes)) => match parse(&bytes) {
                Some(locale) => {
                    self.last_read_locale = Some(locale);
                    if preserve_unknown {
                        self.last_readback = Some(SettingsReadback::ObservedAfterUnknown(locale));
                        LocaleLoadResult::ObservedAfterUnknown(locale)
                    } else {
                        self.last_readback = Some(SettingsReadback::Restored(locale));
                        self.active_locale = Some(locale);
                        self.state = SettingsUiState::Loaded;
                        LocaleLoadResult::Loaded(locale)
                    }
                }
                None => {
                    self.last_read_locale = None;
                    self.last_readback = Some(SettingsReadback::Invalid);
                    if !preserve_unknown {
                        self.state = SettingsUiState::Invalid;
                    }
                    LocaleLoadResult::Invalid
                }
            },
            Ok(None) => {
                self.last_read_locale = None;
                self.last_readback = Some(SettingsReadback::Missing);
                if !preserve_unknown {
                    self.state = SettingsUiState::Missing;
                }
                LocaleLoadResult::Missing
            }
            Err(error) => {
                self.last_read_locale = None;
                self.last_readback = Some(SettingsReadback::Unavailable);
                if !preserve_unknown {
                    self.state = SettingsUiState::LoadUnavailable;
                }
                LocaleLoadResult::Unavailable(error)
            }
        }
    }

    /// Invoke the real adapter exactly once. The returned `SettingsSaveFailure`
    /// is passed back intact so the caller can present its provider error and
    /// outcome. This consumer never retries an Unknown write.
    pub fn save(
        &mut self,
        selected: L,
        encode: impl FnOnce(L) -> Vec<u8>,
    ) -> Result<SettingsSaveOutcome, SettingsSaveFailure> {
        let bytes = encode(selected);
        self.last_readback = None;
        let result = self.adapter.save_system_language(&bytes);
        match &result {
            Ok(SettingsSaveOutcome::New) => {
                self.active_locale = Some(selected);
                self.last_read_locale = Some(selected);
                self.state = SettingsUiState::Saved;
                self.commit_display = SettingsCommitDisplay::Saved { selected };
            }
            Ok(SettingsSaveOutcome::Old) => {
                self.record_old(selected);
            }
            Ok(SettingsSaveOutcome::Unknown) => {
                self.record_unknown(selected);
            }
            Err(failure) => match failure.outcome {
                SettingsSaveOutcome::Old => self.record_old(selected),
                SettingsSaveOutcome::Unknown => self.record_unknown(selected),
                SettingsSaveOutcome::New => {
                    self.last_read_locale = None;
                    self.state = SettingsUiState::SaveUnavailable;
                    self.commit_display = SettingsCommitDisplay::Failed {
                        selected,
                        reported_outcome: SettingsSaveOutcome::New,
                    };
                }
            },
        }
        result
    }

    /// Consume the result of one real Task27 Desktop input dispatch. The caller
    /// must invoke this once for that returned value and must not also call
    /// `handle_event` for the same input. This method only records observations:
    /// it does not save, retry, read back, or dispatch another event. The caller
    /// retains the result, including any original provider error, and uses
    /// `handled` for redraw. Only `Ok(New)` is recorded as Saved; an `Err` stays
    /// Failed even if its embedded outcome says New.
    #[cfg(target_os = "linux")]
    pub fn observe_desktop_dispatch(
        &mut self,
        result: &nagi_linux_notes_desktop_dev::desktop::DesktopEventResult,
        map_locale: impl Fn(nagi_localization::Locale) -> L,
    ) {
        use nagi_linux_notes_desktop_dev::desktop::SettingsLocaleReadback;

        let running = map_locale(result.running_locale);
        self.active_locale = Some(running);

        if result.settings_save.is_some() {
            self.last_read_locale = None;
            self.last_readback = None;
        }

        match result.settings_save.as_ref() {
            Some(Ok(SettingsSaveOutcome::New)) => {
                self.state = SettingsUiState::Saved;
                self.commit_display = SettingsCommitDisplay::Saved { selected: running };
            }
            Some(Ok(SettingsSaveOutcome::Old)) => {
                self.state = SettingsUiState::SaveOld;
                self.commit_display = SettingsCommitDisplay::Old { selected: running };
            }
            Some(Ok(SettingsSaveOutcome::Unknown)) => {
                self.state = SettingsUiState::SaveUnknown;
                self.commit_display = SettingsCommitDisplay::Unknown { selected: running };
            }
            Some(Err(failure)) => {
                self.state = SettingsUiState::SaveUnavailable;
                self.commit_display = SettingsCommitDisplay::Failed {
                    selected: running,
                    reported_outcome: failure.outcome,
                };
            }
            None => {}
        }

        if let Some(readback) = result.locale_readback {
            let unknown_save = matches!(
                result.settings_save.as_ref(),
                Some(Ok(SettingsSaveOutcome::Unknown))
                    | Some(Err(SettingsSaveFailure {
                        outcome: SettingsSaveOutcome::Unknown,
                        ..
                    }))
            );
            match readback {
                SettingsLocaleReadback::Missing => {
                    self.last_read_locale = None;
                    self.last_readback = Some(SettingsReadback::Missing);
                }
                SettingsLocaleReadback::Restored(locale) => {
                    let observed = map_locale(locale);
                    self.last_read_locale = Some(observed);
                    self.last_readback = Some(if unknown_save {
                        SettingsReadback::ObservedAfterUnknown(observed)
                    } else {
                        SettingsReadback::Restored(observed)
                    });
                }
                SettingsLocaleReadback::Invalid => {
                    self.last_read_locale = None;
                    self.last_readback = Some(SettingsReadback::Invalid);
                }
                SettingsLocaleReadback::Unavailable => {
                    self.last_read_locale = None;
                    self.last_readback = Some(SettingsReadback::Unavailable);
                }
            }
        }
    }

    fn record_old(&mut self, selected: L) {
        self.last_read_locale = None;
        self.state = SettingsUiState::SaveOld;
        self.commit_display = SettingsCommitDisplay::Old { selected };
    }

    fn record_unknown(&mut self, selected: L) {
        self.last_read_locale = None;
        self.state = SettingsUiState::SaveUnknown;
        self.commit_display = SettingsCommitDisplay::Unknown { selected };
    }

    pub fn active_locale(&self) -> Option<L> {
        self.active_locale
    }

    pub fn state(&self) -> SettingsUiState {
        self.state
    }

    pub fn display(&self) -> SettingsDisplay<L> {
        SettingsDisplay {
            running_locale: self.active_locale,
            settings_selection_locale: self.last_read_locale.or(self.active_locale),
            last_read_locale: self.last_read_locale,
            last_readback: self.last_readback,
            commit: self.commit_display,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_storage_provider::{
        fail_flush_after_for_test, StorageProvider, StorageProviderOwner, MIN_VFS_BYTES,
    };
    use std::fs::OpenOptions;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Locale {
        En,
        Ja,
    }

    struct TempDisk(PathBuf);

    impl TempDisk {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "nagi-task28-settings-adapter-{}-{stamp}.img",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDisk {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn actual_adapter_fixture() -> (TempDisk, StorageProviderOwner, StorageProvider) {
        let disk = TempDisk::new();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&disk.0)
            .expect("create task-owned disposable VFS image");
        file.set_len(MIN_VFS_BYTES)
            .expect("size task-owned disposable VFS image");
        file.sync_all().expect("flush blank test image");
        let owner = StorageProviderOwner::format_blank_file(file)
            .expect("format task-owned blank VFS image");
        let storage = owner.consumer_handle();
        (disk, owner, storage)
    }

    fn parse_locale(value: &[u8]) -> Option<Locale> {
        match value {
            b"en-US" => Some(Locale::En),
            b"ja-JP" => Some(Locale::Ja),
            _ => None,
        }
    }

    fn encode_locale(locale: Locale) -> Vec<u8> {
        match locale {
            Locale::En => b"en-US".to_vec(),
            Locale::Ja => b"ja-JP".to_vec(),
        }
    }

    #[test]
    fn actual_adapter_failure_keeps_unknown_readback_separate_from_running_locale() {
        let (_disk, _owner, storage) = actual_adapter_fixture();
        let mut settings = SystemLanguageSettings::from_provider_handle(storage);

        assert!(matches!(
            settings.load(parse_locale),
            LocaleLoadResult::Missing
        ));
        assert!(matches!(
            settings.save(Locale::En, encode_locale),
            Ok(SettingsSaveOutcome::New)
        ));
        assert_eq!(settings.active_locale(), Some(Locale::En));
        assert_eq!(
            settings.display().settings_selection_locale,
            Some(Locale::En)
        );

        // Actual SettingsStorageAdapter injection: fail its prepared-file
        // flush, which the provider classifies as Old and keeps at en-US.
        fail_flush_after_for_test(0);
        let old = settings
            .save(Locale::Ja, encode_locale)
            .expect_err("prepublish flush failure must remain a provider error");
        assert_eq!(old.outcome, SettingsSaveOutcome::Old);
        assert_eq!(settings.state(), SettingsUiState::SaveOld);
        assert_eq!(settings.active_locale(), Some(Locale::En));
        assert_eq!(
            settings.display().settings_selection_locale,
            Some(Locale::En)
        );

        // Fail the final flush after publish. The real adapter returns
        // Err(Unknown), while the replacement is visible through its read API.
        fail_flush_after_for_test(1);
        let unknown = settings
            .save(Locale::Ja, encode_locale)
            .expect_err("post-publish flush failure must not report Saved");
        assert_eq!(unknown.outcome, SettingsSaveOutcome::Unknown);
        assert_eq!(settings.state(), SettingsUiState::SaveUnknown);
        assert_eq!(
            settings.display(),
            SettingsDisplay {
                running_locale: Some(Locale::En),
                settings_selection_locale: Some(Locale::En),
                last_read_locale: None,
                last_readback: None,
                commit: SettingsCommitDisplay::Unknown {
                    selected: Locale::Ja
                }
            }
        );

        assert!(matches!(
            settings.load(parse_locale),
            LocaleLoadResult::ObservedAfterUnknown(Locale::Ja)
        ));
        assert_eq!(settings.active_locale(), Some(Locale::En));
        assert_eq!(settings.state(), SettingsUiState::SaveUnknown);
        assert_eq!(
            settings.display(),
            SettingsDisplay {
                running_locale: Some(Locale::En),
                settings_selection_locale: Some(Locale::Ja),
                last_read_locale: Some(Locale::Ja),
                last_readback: Some(SettingsReadback::ObservedAfterUnknown(Locale::Ja)),
                commit: SettingsCommitDisplay::Unknown {
                    selected: Locale::Ja
                }
            }
        );
    }

    #[test]
    fn actual_adapter_load_distinguishes_missing_and_invalid_values() {
        let (_disk, _owner, storage) = actual_adapter_fixture();
        let mut settings = SystemLanguageSettings::from_provider_handle(storage.clone());
        assert!(matches!(
            settings.load(parse_locale),
            LocaleLoadResult::Missing
        ));

        let handle = storage
            .create_path(b"system-language")
            .expect("create task-owned invalid preference");
        storage
            .write(handle, b"unsupported")
            .expect("write task-owned invalid preference");
        storage
            .flush()
            .expect("flush task-owned invalid preference");
        assert!(matches!(
            settings.load(parse_locale),
            LocaleLoadResult::Invalid
        ));
        assert_eq!(settings.state(), SettingsUiState::Invalid);
    }

    /// Drives the existing Task27 Desktop Settings focus with the actual
    /// `InputEvent` path and consumes its exact one-dispatch result. This does
    /// not test the outer Shell controller or a live Session/grant.
    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_settings_input_result_is_consumed_without_second_dispatch() {
        use nagi_linux_notes_desktop_dev::desktop::Desktop;

        fn key(code: u16) -> libnagi::InputEvent {
            libnagi::InputEvent {
                event_type: libnagi::INPUT_EVENT_KEY,
                code,
                value: 1,
            }
        }

        fn map_locale(locale: nagi_localization::Locale) -> Locale {
            match locale {
                nagi_localization::Locale::EnUs => Locale::En,
                nagi_localization::Locale::JaJp => Locale::Ja,
            }
        }

        fn dispatch(
            desktop: &mut Desktop,
            event: libnagi::InputEvent,
            volume: &mut nagi_linux_notes_desktop_dev::desktop::UserDataVolume,
            settings: &mut SystemLanguageSettings<Locale>,
        ) -> nagi_linux_notes_desktop_dev::desktop::DesktopEventResult {
            let result = desktop.handle_event_with_settings_result(event, volume);
            settings.observe_desktop_dispatch(&result, map_locale);
            result
        }

        let (_disk, _owner, storage) = actual_adapter_fixture();
        let mut settings = SystemLanguageSettings::from_provider_handle(storage.clone());
        assert!(matches!(
            settings.load(parse_locale),
            LocaleLoadResult::Missing
        ));

        let mut desktop_volume = storage.clone();
        let mut desktop = Desktop::new(nagi_localization::Locale::EnUs);
        assert!(dispatch(
            &mut desktop,
            key(libnagi::INPUT_KEY_TAB),
            &mut desktop_volume,
            &mut settings
        )
        .handled);
        assert!(dispatch(
            &mut desktop,
            key(libnagi::INPUT_KEY_ENTER),
            &mut desktop_volume,
            &mut settings
        )
        .handled);
        assert!(dispatch(
            &mut desktop,
            key(libnagi::INPUT_KEY_TAB),
            &mut desktop_volume,
            &mut settings
        )
        .handled);

        let mut before = vec![0; libnagi::SURFACE_BYTES / core::mem::size_of::<u32>()];
        desktop.render(&mut before);
        let result = dispatch(
            &mut desktop,
            key(libnagi::INPUT_KEY_ENTER),
            &mut desktop_volume,
            &mut settings,
        );
        assert!(result.handled);
        assert_eq!(
            result.settings_save.as_ref().unwrap().as_ref().unwrap(),
            &SettingsSaveOutcome::New
        );
        assert_eq!(result.running_locale, nagi_localization::Locale::JaJp);
        let mut after = vec![0; before.len()];
        desktop.render(&mut after);
        assert_ne!(
            before, after,
            "the real Desktop renderer reflects selection"
        );
        assert_eq!(
            settings.display().settings_selection_locale,
            Some(Locale::Ja)
        );
        assert_eq!(
            settings.display().commit,
            SettingsCommitDisplay::Saved {
                selected: Locale::Ja
            }
        );
        assert_eq!(settings.display().running_locale, Some(Locale::Ja));
        assert_eq!(
            settings.display().last_readback,
            None,
            "the successful input result is not followed by an implicit extra read"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_unknown_failure_keeps_readback_observational_and_never_saved() {
        use nagi_linux_notes_desktop_dev::desktop::{Desktop, SettingsLocaleReadback};

        fn key(code: u16) -> libnagi::InputEvent {
            libnagi::InputEvent {
                event_type: libnagi::INPUT_EVENT_KEY,
                code,
                value: 1,
            }
        }

        fn map_locale(locale: nagi_localization::Locale) -> Locale {
            match locale {
                nagi_localization::Locale::EnUs => Locale::En,
                nagi_localization::Locale::JaJp => Locale::Ja,
            }
        }

        let (_disk, _owner, storage) = actual_adapter_fixture();
        let mut settings = SystemLanguageSettings::from_provider_handle(storage.clone());
        let mut desktop_volume = storage;
        let mut desktop = Desktop::new(nagi_localization::Locale::EnUs);
        for event in [
            key(libnagi::INPUT_KEY_TAB),
            key(libnagi::INPUT_KEY_ENTER),
            key(libnagi::INPUT_KEY_TAB),
        ] {
            let result = desktop.handle_event_with_settings_result(event, &mut desktop_volume);
            settings.observe_desktop_dispatch(&result, map_locale);
            assert!(result.settings_save.is_none());
        }

        // One successful prepared-file flush followed by the actual adapter's
        // publish-barrier failure. Desktop performs only its existing explicit
        // Unknown readback; this consumer does not send a second save or read.
        fail_flush_after_for_test(1);
        let result = desktop.handle_event_with_settings_result(
            key(libnagi::INPUT_KEY_ENTER),
            &mut desktop_volume,
        );
        assert!(matches!(
            result.settings_save,
            Some(Err(SettingsSaveFailure {
                outcome: SettingsSaveOutcome::Unknown,
                ..
            }))
        ));
        assert!(matches!(
            result.locale_readback,
            Some(SettingsLocaleReadback::Restored(nagi_localization::Locale::JaJp))
        ));
        settings.observe_desktop_dispatch(&result, map_locale);
        assert_eq!(settings.state(), SettingsUiState::SaveUnavailable);
        assert_eq!(
            settings.display(),
            SettingsDisplay {
                running_locale: Some(Locale::Ja),
                settings_selection_locale: Some(Locale::Ja),
                last_read_locale: Some(Locale::Ja),
                last_readback: Some(SettingsReadback::ObservedAfterUnknown(Locale::Ja)),
                commit: SettingsCommitDisplay::Failed {
                    selected: Locale::Ja,
                    reported_outcome: SettingsSaveOutcome::Unknown,
                },
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_error_with_new_outcome_is_not_a_save_ack() {
        use nagi_linux_notes_desktop_dev::desktop::DesktopEventResult;
        use nagi_storage_provider::{StorageError, StorageProviderError};

        let (_disk, _owner, storage) = actual_adapter_fixture();
        let mut settings = SystemLanguageSettings::from_provider_handle(storage);
        let result = DesktopEventResult {
            handled: true,
            settings_save: Some(Err(SettingsSaveFailure {
                outcome: SettingsSaveOutcome::New,
                error: StorageProviderError::Vfs(StorageError::Block),
            })),
            locale_readback: None,
            running_locale: nagi_localization::Locale::EnUs,
        };

        settings.observe_desktop_dispatch(&result, |locale| match locale {
            nagi_localization::Locale::EnUs => Locale::En,
            nagi_localization::Locale::JaJp => Locale::Ja,
        });
        assert_eq!(settings.state(), SettingsUiState::SaveUnavailable);
        assert_eq!(
            settings.display().commit,
            SettingsCommitDisplay::Failed {
                selected: Locale::En,
                reported_outcome: SettingsSaveOutcome::New,
            }
        );
    }
}
