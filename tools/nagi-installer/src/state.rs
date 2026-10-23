use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::InstallerError;

pub const INVENTORY_SCHEMA_VERSION: u32 = 1;
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn new_transaction_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "{timestamp:032x}-{:08x}-{sequence:016x}",
        std::process::id()
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOperation {
    FreshInstall,
    Upgrade,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilityDelta {
    pub unchanged: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl CapabilityDelta {
    pub(crate) fn between(previous: &[String], requested: &[String]) -> Self {
        let previous = previous.iter().collect::<std::collections::BTreeSet<_>>();
        let requested = requested.iter().collect::<std::collections::BTreeSet<_>>();
        Self {
            unchanged: previous
                .intersection(&requested)
                .map(|value| (*value).clone())
                .collect(),
            added: requested
                .difference(&previous)
                .map(|value| (*value).clone())
                .collect(),
            removed: previous
                .difference(&requested)
                .map(|value| (*value).clone())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledPackage {
    pub app_id: String,
    pub version: String,
    /// Store-relative path; absolute host paths are never persisted.
    pub active_location: String,
    pub transaction_id: String,
    pub required_capabilities: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventorySnapshot {
    pub generation: u64,
    packages: BTreeMap<String, InstalledPackage>,
}

impl InventorySnapshot {
    pub fn get(&self, app_id: &str) -> Option<&InstalledPackage> {
        self.packages.get(app_id)
    }

    pub fn packages(&self) -> impl Iterator<Item = &InstalledPackage> {
        self.packages.values()
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionPhase {
    Created,
    Validating,
    Staged,
    ReadyToCommit,
    Committing,
    Committed,
    RollingBack,
    RolledBack,
    Failed,
}

impl TransactionPhase {
    pub const fn allows_transition_to(self, next: Self) -> bool {
        use TransactionPhase::*;
        matches!(
            (self, next),
            (Created, Validating | Failed)
                | (Validating, Staged | Failed)
                | (Staged, ReadyToCommit | Failed)
                | (ReadyToCommit, Committing | Failed)
                | (Committing, Committed | RollingBack | Failed)
                | (Failed, RollingBack)
                | (RollingBack, RolledBack | Failed)
        )
    }

    pub(crate) fn is_committed(self) -> bool {
        self == Self::Committed
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalOperation {
    Install,
    Uninstall,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InventoryFile {
    pub schema_version: u32,
    pub generation: u64,
    pub packages: BTreeMap<String, InstalledPackage>,
}

impl Default for InventoryFile {
    fn default() -> Self {
        Self {
            schema_version: INVENTORY_SCHEMA_VERSION,
            generation: 0,
            packages: BTreeMap::new(),
        }
    }
}

impl InventoryFile {
    pub(crate) fn snapshot(&self) -> InventorySnapshot {
        InventorySnapshot {
            generation: self.generation,
            packages: self.packages.clone(),
        }
    }

    pub(crate) fn update(
        &mut self,
        app_id: &str,
        package: Option<InstalledPackage>,
    ) -> Result<(), InstallerError> {
        self.generation = self.generation.checked_add(1).ok_or_else(|| {
            InstallerError::InventoryFailure("inventory generation overflow".into())
        })?;
        match package {
            Some(package) => {
                self.packages.insert(app_id.to_owned(), package);
            }
            None => {
                self.packages.remove(app_id);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransactionJournal {
    pub schema_version: u32,
    pub transaction_id: String,
    pub operation: JournalOperation,
    pub app_id: String,
    pub prior: Option<InstalledPackage>,
    pub target: Option<InstalledPackage>,
    pub staging_location: Option<String>,
    pub phase: TransactionPhase,
    pub failure: Option<String>,
}

impl TransactionJournal {
    pub(crate) fn transition(&mut self, next: TransactionPhase) -> Result<(), InstallerError> {
        if !self.phase.allows_transition_to(next) {
            return Err(InstallerError::InvalidTransition {
                from: format!("{:?}", self.phase),
                to: format!("{next:?}"),
            });
        }
        self.phase = next;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UninstallOutcome {
    pub app_id: String,
    pub removed_version: String,
    /// False means cleanup is still pending and will be retried by recovery.
    pub package_files_removed: bool,
}
