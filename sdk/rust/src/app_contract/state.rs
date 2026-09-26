use crate::{AppId, AppSessionId};

use super::{AppError, ErrorCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppStateNamespace {
    pub app_id: AppId,
    /// `None` addresses durable app-wide state; `Some` addresses session state.
    pub session_id: Option<AppSessionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct AppStateVersion(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateLoad {
    Missing,
    Loaded {
        version: AppStateVersion,
        length: usize,
    },
    Corrupt,
}

/// Backend implementations own durable storage and must replace a state value
/// atomically. Reset is idempotent. This trait does not select a filesystem.
pub trait AppStateBackend {
    fn load(
        &mut self,
        namespace: AppStateNamespace,
        output: &mut [u8],
    ) -> Result<StateLoad, AppError>;

    fn save(
        &mut self,
        namespace: AppStateNamespace,
        version: AppStateVersion,
        bytes: &[u8],
    ) -> Result<(), AppError>;

    fn reset(&mut self, namespace: AppStateNamespace) -> Result<(), AppError>;
}

pub trait StateMigrator {
    fn migrate(
        &self,
        namespace: AppStateNamespace,
        from: AppStateVersion,
        to: AppStateVersion,
        source: &[u8],
        destination: &mut [u8],
    ) -> Result<usize, AppError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreKind {
    Missing,
    Restored,
    Migrated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreResult {
    pub kind: RestoreKind,
    pub version: AppStateVersion,
    pub length: usize,
}

/// Load and upgrade app state into `output`. Corrupt state is reported instead
/// of silently treated as missing. A successful migration is saved only after
/// the migrator has produced the complete destination bytes.
pub fn restore_state(
    backend: &mut impl AppStateBackend,
    migrator: &impl StateMigrator,
    namespace: AppStateNamespace,
    target: AppStateVersion,
    minimum_readable: AppStateVersion,
    scratch: &mut [u8],
    output: &mut [u8],
) -> Result<RestoreResult, AppError> {
    if target.0 == 0 || minimum_readable.0 == 0 || minimum_readable > target {
        return Err(AppError::new(ErrorCode::StateVersionIncompatible));
    }
    match backend.load(namespace, scratch)? {
        StateLoad::Missing => Ok(RestoreResult {
            kind: RestoreKind::Missing,
            version: target,
            length: 0,
        }),
        StateLoad::Corrupt => Err(AppError::new(ErrorCode::StateCorrupt)),
        StateLoad::Loaded { version, length } => {
            if version.0 == 0 || length > scratch.len() {
                return Err(AppError::new(ErrorCode::StateCorrupt));
            }
            if version > target {
                return Err(AppError::new(ErrorCode::StateVersionIncompatible));
            }
            if version < minimum_readable {
                return Err(AppError::new(ErrorCode::StateVersionIncompatible));
            }
            if version == target {
                if output.len() < length {
                    return Err(AppError::new(ErrorCode::BufferTooSmall));
                }
                output[..length].copy_from_slice(&scratch[..length]);
                return Ok(RestoreResult {
                    kind: RestoreKind::Restored,
                    version,
                    length,
                });
            }
            let length = migrator
                .migrate(namespace, version, target, &scratch[..length], output)
                .map_err(|_| AppError::new(ErrorCode::StateMigrationFailed))?;
            if length > output.len() {
                return Err(AppError::new(ErrorCode::BufferTooSmall));
            }
            backend.save(namespace, target, &output[..length])?;
            Ok(RestoreResult {
                kind: RestoreKind::Migrated,
                version: target,
                length,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        restore_state, AppStateBackend, AppStateNamespace, AppStateVersion, RestoreKind, StateLoad,
        StateMigrator,
    };
    use crate::app_contract::{AppError, ErrorCode};
    use crate::{AppId, AppSessionId};

    struct MemoryStore {
        version: Option<AppStateVersion>,
        bytes: [u8; 32],
        length: usize,
        corrupt: bool,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                version: None,
                bytes: [0; 32],
                length: 0,
                corrupt: false,
            }
        }
    }

    impl AppStateBackend for MemoryStore {
        fn load(
            &mut self,
            _namespace: AppStateNamespace,
            output: &mut [u8],
        ) -> Result<StateLoad, AppError> {
            if self.corrupt {
                return Ok(StateLoad::Corrupt);
            }
            let Some(version) = self.version else {
                return Ok(StateLoad::Missing);
            };
            if output.len() < self.length {
                return Err(AppError::new(ErrorCode::BufferTooSmall));
            }
            output[..self.length].copy_from_slice(&self.bytes[..self.length]);
            Ok(StateLoad::Loaded {
                version,
                length: self.length,
            })
        }

        fn save(
            &mut self,
            _namespace: AppStateNamespace,
            version: AppStateVersion,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            if bytes.len() > self.bytes.len() {
                return Err(AppError::new(ErrorCode::StateUnavailable));
            }
            self.bytes[..bytes.len()].copy_from_slice(bytes);
            self.length = bytes.len();
            self.version = Some(version);
            Ok(())
        }

        fn reset(&mut self, _namespace: AppStateNamespace) -> Result<(), AppError> {
            self.version = None;
            self.length = 0;
            self.bytes.fill(0);
            Ok(())
        }
    }

    struct AppendVersion;

    impl StateMigrator for AppendVersion {
        fn migrate(
            &self,
            _namespace: AppStateNamespace,
            _from: AppStateVersion,
            _to: AppStateVersion,
            source: &[u8],
            destination: &mut [u8],
        ) -> Result<usize, AppError> {
            if destination.len() < source.len() + 1 {
                return Err(AppError::new(ErrorCode::BufferTooSmall));
            }
            destination[..source.len()].copy_from_slice(source);
            destination[source.len()] = b'2';
            Ok(source.len() + 1)
        }
    }

    fn namespace() -> AppStateNamespace {
        AppStateNamespace {
            app_id: AppId(3),
            session_id: Some(AppSessionId(5)),
        }
    }

    #[test]
    fn missing_state_is_distinct_and_reset_is_idempotent() {
        let mut store = MemoryStore::new();
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        let result = restore_state(
            &mut store,
            &AppendVersion,
            namespace(),
            AppStateVersion(2),
            AppStateVersion(1),
            &mut scratch,
            &mut output,
        )
        .unwrap();
        assert_eq!(result.kind, RestoreKind::Missing);
        store.reset(namespace()).unwrap();
        store.reset(namespace()).unwrap();
        assert_eq!(store.version, None);
    }

    #[test]
    fn versioned_state_saves_and_restores() {
        let mut store = MemoryStore::new();
        store
            .save(namespace(), AppStateVersion(2), b"open-note")
            .unwrap();
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        let result = restore_state(
            &mut store,
            &AppendVersion,
            namespace(),
            AppStateVersion(2),
            AppStateVersion(1),
            &mut scratch,
            &mut output,
        )
        .unwrap();
        assert_eq!(result.kind, RestoreKind::Restored);
        assert_eq!(&output[..result.length], b"open-note");
    }

    #[test]
    fn corrupt_state_is_not_silently_reset() {
        let mut store = MemoryStore::new();
        store.corrupt = true;
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        assert_eq!(
            restore_state(
                &mut store,
                &AppendVersion,
                namespace(),
                AppStateVersion(2),
                AppStateVersion(1),
                &mut scratch,
                &mut output,
            ),
            Err(AppError::new(ErrorCode::StateCorrupt))
        );
        assert_eq!(store.version, None);
    }

    #[test]
    fn migration_hook_updates_persisted_version_only_after_success() {
        let mut store = MemoryStore::new();
        store
            .save(namespace(), AppStateVersion(1), b"note")
            .unwrap();
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        let result = restore_state(
            &mut store,
            &AppendVersion,
            namespace(),
            AppStateVersion(2),
            AppStateVersion(1),
            &mut scratch,
            &mut output,
        )
        .unwrap();
        assert_eq!(result.kind, RestoreKind::Migrated);
        assert_eq!(store.version, Some(AppStateVersion(2)));
        assert_eq!(&output[..result.length], b"note2");
    }

    #[test]
    fn future_state_version_is_not_downgraded() {
        let mut store = MemoryStore::new();
        store.save(namespace(), AppStateVersion(3), b"new").unwrap();
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        assert_eq!(
            restore_state(
                &mut store,
                &AppendVersion,
                namespace(),
                AppStateVersion(2),
                AppStateVersion(1),
                &mut scratch,
                &mut output,
            ),
            Err(AppError::new(ErrorCode::StateVersionIncompatible))
        );
        assert_eq!(store.version, Some(AppStateVersion(3)));
    }

    #[test]
    fn state_older_than_manifest_minimum_is_rejected() {
        let mut store = MemoryStore::new();
        store.save(namespace(), AppStateVersion(1), b"old").unwrap();
        let mut scratch = [0; 32];
        let mut output = [0; 32];
        assert_eq!(
            restore_state(
                &mut store,
                &AppendVersion,
                namespace(),
                AppStateVersion(3),
                AppStateVersion(2),
                &mut scratch,
                &mut output,
            ),
            Err(AppError::new(ErrorCode::StateVersionIncompatible))
        );
        assert_eq!(store.version, Some(AppStateVersion(1)));
    }
}
