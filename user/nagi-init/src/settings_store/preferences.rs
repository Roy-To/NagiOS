//! The Settings consumer of the owner-issued profile snapshot.

use super::codec::{decode, encode_replacement, DecodeError};
use nagi_localization::Locale;
use nagi_model::ObjectId;
use nagi_persistence_contract::{ConditionalWriteOutcome, Revision, Snapshot, SnapshotStorage};
use nagi_ui::ThemeMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Preferences {
    pub(crate) locale: Locale,
    pub(crate) theme: ThemeMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadPreferences {
    Missing,
    Restored {
        preferences: Preferences,
        invalid_locale: bool,
        invalid_theme: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreferencesError {
    Unavailable,
    Invalid(DecodeError),
    RevisionMismatch,
    ReloadRequired,
    Conflict,
    OutcomeUnknown,
}

#[derive(Default)]
struct ObservedPreferences {
    current: Option<(ObjectId, Option<Snapshot>)>,
}

impl ObservedPreferences {
    fn reload<E>(
        &mut self,
        key: ObjectId,
        storage: &dyn SnapshotStorage<Error = E>,
    ) -> Result<ReadPreferences, PreferencesError> {
        self.current = None;
        let snapshot = storage
            .read(key)
            .map_err(|_| PreferencesError::Unavailable)?;
        let result = match &snapshot {
            Some(snapshot) => {
                let decoded = decode(&snapshot.bytes).map_err(PreferencesError::Invalid)?;
                if decoded.revision.get() != snapshot.revision.get() {
                    return Err(PreferencesError::RevisionMismatch);
                }
                ReadPreferences::Restored {
                    preferences: Preferences {
                        locale: decoded.preferences.locale,
                        theme: decoded.preferences.theme,
                    },
                    invalid_locale: decoded.invalid_locale,
                    invalid_theme: decoded.invalid_theme,
                }
            }
            None => ReadPreferences::Missing,
        };
        self.current = Some((key, snapshot));
        Ok(result)
    }

    fn save<E>(
        &mut self,
        key: ObjectId,
        storage: &mut dyn SnapshotStorage<Error = E>,
        preferences: Preferences,
    ) -> Result<Revision, PreferencesError> {
        let (observed_key, snapshot) = self
            .current
            .as_ref()
            .ok_or(PreferencesError::ReloadRequired)?;
        if *observed_key != key {
            return Err(PreferencesError::ReloadRequired);
        }
        let (next, bytes) = encode_replacement(
            snapshot.as_ref().map(|snapshot| snapshot.bytes.as_slice()),
            preferences.locale,
            preferences.theme,
        )
        .map_err(PreferencesError::Invalid)?;
        let expected = snapshot.as_ref().map(|snapshot| snapshot.revision);
        match storage.compare_and_replace(key, expected, &bytes) {
            ConditionalWriteOutcome::Applied { revision } if revision.get() == next.get() => {
                self.current = Some((key, Some(Snapshot::new(revision, bytes))));
                Ok(revision)
            }
            ConditionalWriteOutcome::Conflict { .. } => {
                self.current = None;
                Err(PreferencesError::Conflict)
            }
            ConditionalWriteOutcome::Applied { .. }
            | ConditionalWriteOutcome::OutcomeUnknown { .. } => {
                self.current = None;
                Err(PreferencesError::OutcomeUnknown)
            }
            ConditionalWriteOutcome::NotApplied { .. } => Err(PreferencesError::Unavailable),
        }
    }
}

/// An owner-issued, session-scoped view of the Settings snapshot.
///
/// The owner supplies the object key and lends its typed S4 storage only for
/// this callback. Settings does not choose a profile, open a path, or retain a
/// borrowed storage view after the owner callback returns. Implementations
/// must keep the selected owner, profile/root, and Settings object key fixed
/// for the lifetime of one `SettingsStore`. If selection changes, the caller
/// must discard that store and create a new one before reloading or saving;
/// otherwise a cached revision could be reused against a different profile
/// that happens to use the same object key.
pub(crate) trait SettingsSnapshotScope {
    type StorageError;
    type ScopeError;

    fn with_settings_storage<R, F>(&mut self, operation: F) -> Result<R, Self::ScopeError>
    where
        F: FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = Self::StorageError>) -> R;
}

/// Keeps one owner-issued Settings scope and its last observed preference
/// revision together. The scope must remain bound to the same owner/profile
/// until this store is discarded, and the key comes from that scope on every
/// I/O.
pub(crate) struct SettingsStore<S: SettingsSnapshotScope> {
    scope: S,
    observed: ObservedPreferences,
}

impl<S: SettingsSnapshotScope> SettingsStore<S> {
    pub(crate) fn new(scope: S) -> Self {
        Self {
            scope,
            observed: ObservedPreferences::default(),
        }
    }

    pub(crate) fn reload(&mut self) -> Result<ReadPreferences, PreferencesError> {
        self.observed.current = None;
        let observed = &mut self.observed;
        self.scope
            .with_settings_storage(|key, storage| observed.reload(key, storage))
            .map_err(|_| PreferencesError::Unavailable)?
    }

    /// A conflict requires a fresh observation. An unknown publication also
    /// fences the underlying S4 writer, so reacquire a capability before the
    /// next save and use reload only to observe the canonical value.
    pub(crate) fn save(&mut self, preferences: Preferences) -> Result<Revision, PreferencesError> {
        let observed = &mut self.observed;
        self.scope
            .with_settings_storage(|key, storage| observed.save(key, storage, preferences))
            .map_err(|_| PreferencesError::Unavailable)?
    }
}

#[cfg(all(target_os = "linux", linux_task24_initial_login_runtime))]
impl SettingsSnapshotScope for crate::profile_volume_notes_owner::AuthorizedProfileSnapshot {
    type StorageError = crate::profile_volume_notes_owner::SingleObjectSnapshotStorageError<
        crate::profile_volume_notes_owner::SnapshotStorageError,
    >;
    type ScopeError = ();

    fn with_settings_storage<R, F>(&mut self, operation: F) -> Result<R, Self::ScopeError>
    where
        F: FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = Self::StorageError>) -> R,
    {
        Self::with_storage(self, operation).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests;
