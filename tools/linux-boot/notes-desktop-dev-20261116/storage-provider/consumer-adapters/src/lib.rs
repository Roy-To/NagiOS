mod files;
mod settings;

pub(crate) const SETTINGS_PENDING_NAME: &[u8] = b".nagi-settings-pending";

pub use files::FilesVfsProvider;
pub use settings::{SettingsSaveFailure, SettingsSaveOutcome, SettingsStorageAdapter};
