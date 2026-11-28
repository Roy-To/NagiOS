use super::*;

const OWNER_KEY: ObjectId = ObjectId(9476);
const DARK_JAPANESE: Preferences = Preferences {
    locale: Locale::JaJp,
    theme: ThemeMode::Dark,
};

#[derive(Default)]
struct IssuedSnapshot {
    snapshot: Option<Snapshot>,
    writes: usize,
    publish_unknown: bool,
}

impl SnapshotStorage for IssuedSnapshot {
    type Error = ();

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error> {
        assert_eq!(key, OWNER_KEY);
        Ok(self.snapshot.clone())
    }

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<Self::Error> {
        assert_eq!(key, OWNER_KEY);
        self.writes += 1;
        let current = self.snapshot.as_ref().map(|snapshot| snapshot.revision);
        if current != expected {
            return ConditionalWriteOutcome::Conflict {
                current_revision: current,
            };
        }
        let revision = Revision::new(current.map_or(1, |revision| revision.get() + 1)).unwrap();
        self.snapshot = Some(Snapshot::new(revision, bytes.to_vec()));
        if self.publish_unknown {
            ConditionalWriteOutcome::OutcomeUnknown { error: () }
        } else {
            ConditionalWriteOutcome::Applied { revision }
        }
    }
}

#[derive(Default)]
struct IssuedSettingsScope {
    storage: IssuedSnapshot,
    callbacks: usize,
}

impl SettingsSnapshotScope for IssuedSettingsScope {
    type StorageError = ();
    type ScopeError = ();

    fn with_settings_storage<R, F>(&mut self, operation: F) -> Result<R, Self::ScopeError>
    where
        F: FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = Self::StorageError>) -> R,
    {
        self.callbacks += 1;
        Ok(operation(OWNER_KEY, &mut self.storage))
    }
}

#[test]
fn saves_and_reopens_the_issued_settings_object() {
    let mut storage = IssuedSnapshot::default();
    let mut settings = ObservedPreferences::default();
    assert_eq!(
        settings.reload(OWNER_KEY, &storage),
        Ok(ReadPreferences::Missing)
    );
    assert_eq!(
        settings.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Ok(Revision::new(1).unwrap())
    );

    let mut reopened = ObservedPreferences::default();
    assert_eq!(
        reopened.reload(OWNER_KEY, &storage),
        Ok(ReadPreferences::Restored {
            preferences: DARK_JAPANESE,
            invalid_locale: false,
            invalid_theme: false,
        })
    );
    assert_eq!(storage.writes, 1);
    assert_eq!(
        reopened.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Ok(Revision::new(2).unwrap())
    );
}

#[test]
fn settings_store_uses_only_the_owner_issued_scope_for_snapshot_io() {
    let mut settings = SettingsStore::new(IssuedSettingsScope::default());

    assert_eq!(settings.reload(), Ok(ReadPreferences::Missing));
    assert_eq!(settings.save(DARK_JAPANESE), Ok(Revision::new(1).unwrap()));
    assert_eq!(
        settings.reload(),
        Ok(ReadPreferences::Restored {
            preferences: DARK_JAPANESE,
            invalid_locale: false,
            invalid_theme: false,
        })
    );

    assert_eq!(settings.scope.callbacks, 3);
    assert_eq!(settings.scope.storage.writes, 1);
}

#[test]
fn conflict_requires_reload_before_another_write() {
    let mut storage = IssuedSnapshot::default();
    let mut first = ObservedPreferences::default();
    let mut stale = ObservedPreferences::default();
    first.reload(OWNER_KEY, &storage).unwrap();
    stale.reload(OWNER_KEY, &storage).unwrap();
    first.save(OWNER_KEY, &mut storage, DARK_JAPANESE).unwrap();
    assert_eq!(
        stale.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Err(PreferencesError::Conflict)
    );
    assert_eq!(
        stale.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Err(PreferencesError::ReloadRequired)
    );
    assert_eq!(storage.writes, 2);
}

#[test]
fn visible_bytes_do_not_turn_an_unknown_save_into_success() {
    let mut storage = IssuedSnapshot {
        publish_unknown: true,
        ..Default::default()
    };
    let mut settings = ObservedPreferences::default();
    settings.reload(OWNER_KEY, &storage).unwrap();
    assert_eq!(
        settings.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Err(PreferencesError::OutcomeUnknown)
    );
    assert_eq!(
        settings.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
        Err(PreferencesError::ReloadRequired)
    );
    assert_eq!(
        settings.reload(OWNER_KEY, &storage),
        Ok(ReadPreferences::Restored {
            preferences: DARK_JAPANESE,
            invalid_locale: false,
            invalid_theme: false,
        })
    );
    assert_eq!(storage.writes, 1);
}

#[test]
fn invalid_snapshot_cannot_be_replaced_as_if_missing() {
    let (_, bytes) = encode_replacement(None, Locale::JaJp, ThemeMode::Dark).unwrap();
    for (bytes, error) in [
        (
            b"invalid".to_vec(),
            PreferencesError::Invalid(DecodeError::Corrupt),
        ),
        (bytes, PreferencesError::RevisionMismatch),
    ] {
        let mut storage = IssuedSnapshot {
            snapshot: Some(Snapshot::new(Revision::new(2).unwrap(), bytes)),
            ..Default::default()
        };
        let mut settings = ObservedPreferences::default();
        assert_eq!(settings.reload(OWNER_KEY, &storage), Err(error));
        assert_eq!(
            settings.save(OWNER_KEY, &mut storage, DARK_JAPANESE),
            Err(PreferencesError::ReloadRequired)
        );
        assert_eq!(storage.writes, 0);
    }
}
