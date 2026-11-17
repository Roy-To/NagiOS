use libnagi::storage::Vfs;
use nagi_storage_provider::{
    LinuxBlockDevice, StorageError, StorageProvider, StorageProviderError,
};
use std::fmt;

const SYSTEM_LANGUAGE_PATH: &[u8] = b"system-language";
use super::SETTINGS_PENDING_NAME;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsSaveOutcome {
    Old,
    New,
    Unknown,
}

#[derive(Debug)]
pub struct SettingsSaveFailure {
    pub outcome: SettingsSaveOutcome,
    pub error: StorageProviderError,
}

impl fmt::Display for SettingsSaveFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "settings save outcome {:?}: {}",
            self.outcome, self.error
        )
    }
}

impl std::error::Error for SettingsSaveFailure {}

impl From<StorageError> for SettingsSaveFailure {
    fn from(error: StorageError) -> Self {
        Self {
            outcome: SettingsSaveOutcome::Unknown,
            error: StorageProviderError::from(error),
        }
    }
}

/// Adapter for the desktop Settings locale preference, preserving the current
/// root-level file format and `en-US` / `ja-JP` payload bytes.
#[derive(Clone)]
pub struct SettingsStorageAdapter {
    storage: StorageProvider,
}

impl SettingsStorageAdapter {
    pub fn new(storage: StorageProvider) -> Self {
        Self { storage }
    }

    pub fn load_system_language(&self) -> Result<Option<Vec<u8>>, StorageProviderError> {
        match self.storage.read_file_snapshot(SYSTEM_LANGUAGE_PATH, 64) {
            Ok((_metadata, bytes)) if bytes.is_empty() => Ok(None),
            Ok((_metadata, bytes)) => Ok(Some(bytes)),
            Err(StorageProviderError::Vfs(StorageError::NotFound)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Publishes through a prepared root-level replacement file. The VFS has
    /// no filesystem-wide transaction: errors before replace are reported Old
    /// when the prior value remains readable, while replace/commit-flush
    /// failures are Unknown and must not be shown as Saved or retried silently.
    pub fn save_system_language(
        &self,
        code: &[u8],
    ) -> Result<SettingsSaveOutcome, SettingsSaveFailure> {
        if code.is_empty() || code.len() > 64 {
            return Err(SettingsSaveFailure {
                outcome: SettingsSaveOutcome::Old,
                error: StorageProviderError::Vfs(StorageError::FileTooLarge),
            });
        }

        self.storage.with_volume(|volume| {
            let previous = read_system_language(volume).map_err(|error| SettingsSaveFailure {
                outcome: SettingsSaveOutcome::Unknown,
                error: StorageProviderError::Vfs(error),
            })?;

            match volume.metadata_path(SYSTEM_LANGUAGE_PATH) {
                Ok(metadata) if metadata.mode & 0xf000 == 0x8000 => {}
                Ok(_) => {
                    return Err(prepublish_failure(
                        volume,
                        &previous,
                        StorageError::IsDirectory,
                    ));
                }
                Err(StorageError::NotFound) => {
                    if let Err(error) = volume.create_path(SYSTEM_LANGUAGE_PATH) {
                        return Err(prepublish_failure(volume, &previous, error));
                    }
                }
                Err(error) => return Err(prepublish_failure(volume, &previous, error)),
            }

            match volume.metadata_path(SETTINGS_PENDING_NAME) {
                Ok(metadata) if metadata.mode & 0xf000 == 0x8000 => {
                    if let Err(error) = volume.remove_path(SETTINGS_PENDING_NAME) {
                        return Err(prepublish_failure(volume, &previous, error));
                    }
                }
                Ok(_) => {
                    return Err(prepublish_failure(
                        volume,
                        &previous,
                        StorageError::IsDirectory,
                    ));
                }
                Err(StorageError::NotFound) => {}
                Err(error) => return Err(prepublish_failure(volume, &previous, error)),
            }

            let pending = match volume.create_path(SETTINGS_PENDING_NAME) {
                Ok(handle) => handle,
                Err(error) => return Err(prepublish_failure(volume, &previous, error)),
            };
            if let Err(error) = volume.write(pending, code) {
                return Err(prepublish_failure(volume, &previous, error));
            }
            if let Err(error) = volume.flush() {
                return Err(prepublish_failure(volume, &previous, error));
            }

            if let Err(error) = volume.replace(SETTINGS_PENDING_NAME, SYSTEM_LANGUAGE_PATH) {
                return Err(prepublish_failure(volume, &previous, error));
            }
            if let Err(error) = volume.flush() {
                return Err(SettingsSaveFailure {
                    outcome: SettingsSaveOutcome::Unknown,
                    error: StorageProviderError::Vfs(error),
                });
            }
            Ok(SettingsSaveOutcome::New)
        })
    }
}

fn read_system_language(
    volume: &mut Vfs<LinuxBlockDevice>,
) -> Result<Option<Vec<u8>>, StorageError> {
    let metadata = match volume.metadata_path(SYSTEM_LANGUAGE_PATH) {
        Ok(metadata) => metadata,
        Err(StorageError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.mode & 0xf000 != 0x8000 {
        return Err(StorageError::IsDirectory);
    }
    let length = usize::try_from(metadata.size).map_err(|_| StorageError::FileTooLarge)?;
    if length == 0 {
        return Ok(None);
    }
    if length > 64 {
        return Err(StorageError::FileTooLarge);
    }
    let handle = volume.open_path(SYSTEM_LANGUAGE_PATH)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| StorageError::FileTooLarge)?;
    bytes.resize(length, 0);
    let read = volume.read(handle, &mut bytes)?;
    if read != length {
        return Err(StorageError::Block);
    }
    Ok(Some(bytes))
}

fn prepublish_failure(
    volume: &mut Vfs<LinuxBlockDevice>,
    previous: &Option<Vec<u8>>,
    error: StorageError,
) -> SettingsSaveFailure {
    let outcome = match read_system_language(volume) {
        Ok(current) if &current == previous => SettingsSaveOutcome::Old,
        _ => SettingsSaveOutcome::Unknown,
    };
    SettingsSaveFailure {
        outcome,
        error: StorageProviderError::Vfs(error),
    }
}
