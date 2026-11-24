//! Calendar's consumer adapter for the S5 handle issued by PID 1.

use crate::calendar_session::CalendarSnapshotScope;
use crate::profile_volume_notes_owner::{AuthorizedProfileSnapshot, ProfileVolumeNotesOwnerError};
use nagi_calendar_core::ObjectId;
use nagi_persistence_contract::SnapshotStorage;
use nagi_profile_volume_notes_owner::{SingleObjectSnapshotStorageError, SnapshotStorageError};

impl CalendarSnapshotScope for AuthorizedProfileSnapshot {
    type ScopeError = ProfileVolumeNotesOwnerError;
    type StorageError = SingleObjectSnapshotStorageError<SnapshotStorageError>;

    fn claim_initialization(&mut self) -> Result<bool, Self::ScopeError> {
        self.claim_calendar_initialization()
    }

    fn with_storage<R>(
        &mut self,
        operation: impl FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = Self::StorageError>) -> R,
    ) -> Result<R, Self::ScopeError> {
        AuthorizedProfileSnapshot::with_storage(self, operation)
    }
}
