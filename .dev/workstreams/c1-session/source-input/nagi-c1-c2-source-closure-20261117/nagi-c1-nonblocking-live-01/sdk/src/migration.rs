use crate::{IdentitySnapshot, SnapshotError, IDENTITY_SCHEMA_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationFailure {
    UnsupportedVersion,
    InvalidLegacyState,
}

/// Owner-supplied migration seam. This crate has no deployed prior identity
/// format, so it provides no production legacy migrator.
pub trait IdentitySnapshotMigrator {
    fn migrate_to_current(
        &self,
        source_version: u32,
        source: &[u8],
    ) -> Result<Vec<u8>, MigrationFailure>;
}

#[derive(Default)]
pub struct NoIdentitySnapshotMigrator;

impl IdentitySnapshotMigrator for NoIdentitySnapshotMigrator {
    fn migrate_to_current(
        &self,
        _source_version: u32,
        _source: &[u8],
    ) -> Result<Vec<u8>, MigrationFailure> {
        Err(MigrationFailure::UnsupportedVersion)
    }
}

pub fn decode_snapshot_with_migration(
    source: &[u8],
    migrator: &impl IdentitySnapshotMigrator,
) -> Result<IdentitySnapshot, SnapshotError> {
    let value: serde_json::Value =
        serde_json::from_slice(source).map_err(|_| SnapshotError::Malformed)?;
    let source_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(SnapshotError::Malformed)?;

    if source_version == IDENTITY_SCHEMA_VERSION {
        return IdentitySnapshot::from_json(source);
    }
    if source_version > IDENTITY_SCHEMA_VERSION {
        return Err(SnapshotError::UnsupportedVersion(source_version));
    }

    let migrated = migrator
        .migrate_to_current(source_version, source)
        .map_err(|_| SnapshotError::MigrationFailed)?;
    IdentitySnapshot::from_json(&migrated).map_err(|_| SnapshotError::MigratedStateInvalid)
}
