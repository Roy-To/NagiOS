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

#[cfg(all(target_os = "linux", linux_task24_initial_login_runtime))]
use crate::profile_volume_notes_owner::AuthorizedProfileSnapshot;

/// Keeps the issued session capability and the last observed preference
/// revision together. The profile key comes from the capability on every I/O.
#[cfg(all(target_os = "linux", linux_task24_initial_login_runtime))]
pub(crate) struct SettingsStore {
    authorized: AuthorizedProfileSnapshot,
    observed: ObservedPreferences,
}

#[cfg(all(target_os = "linux", linux_task24_initial_login_runtime))]
impl SettingsStore {
    pub(crate) fn new(authorized: AuthorizedProfileSnapshot) -> Self {
        Self {
            authorized,
            observed: ObservedPreferences::default(),
        }
    }

    pub(crate) fn reload(&mut self) -> Result<ReadPreferences, PreferencesError> {
        self.observed.current = None;
        self.authorized
            .with_storage(|key, storage| self.observed.reload(key, storage))
            .map_err(|_| PreferencesError::Unavailable)?
    }

    /// A conflict requires a fresh observation. An unknown publication also
    /// fences the underlying S4 writer, so reacquire a capability before the
    /// next save and use reload only to observe the canonical value.
    pub(crate) fn save(&mut self, preferences: Preferences) -> Result<Revision, PreferencesError> {
        self.authorized
            .with_storage(|key, storage| self.observed.save(key, storage, preferences))
            .map_err(|_| PreferencesError::Unavailable)?
    }
}

#[cfg(test)]
mod tests;
