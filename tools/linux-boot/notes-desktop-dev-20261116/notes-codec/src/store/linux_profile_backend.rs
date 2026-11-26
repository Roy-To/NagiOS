//! C1 adapter for the profile-volume owner's scoped Notes object operations.

use nagi_identity_session::{
    AuthenticatedInteractiveCaller, ProfileId, ProfileStorageBackend, ResolvedIdentity,
    StorageRootFailure, UserId,
};
use nagi_model::ObjectId;
use nagi_profile_volume_notes_owner::{NotesObject, NotesObjectError, ProfileVolumeNotesBundle};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[doc(hidden)]
pub struct LinuxProfileStorageRoot {
    user_id: Option<UserId>,
    profile_id: ProfileId,
    bundle: ProfileVolumeNotesBundle,
    io_budget: Duration,
}

impl LinuxProfileStorageRoot {
    pub(super) fn with_object<R>(
        &self,
        operation: impl for<'owner> FnOnce(NotesObject<'owner>) -> R,
    ) -> Result<R, nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerError> {
        self.bundle.with_notes_object(self.io_budget, operation)
    }

    pub(super) fn object_id(&self) -> ObjectId {
        self.bundle.notes_object_id()
    }
}

/// Storage for a single Notes object selected and retained by the volume owner.
pub struct LinuxProfileStorageBackend {
    root: Arc<LinuxProfileStorageRoot>,
    root_opened: bool,
}

impl LinuxProfileStorageBackend {
    pub(super) fn from_authenticated_attachment(
        bundle: ProfileVolumeNotesBundle,
        identity: &ResolvedIdentity,
        caller: &impl AuthenticatedInteractiveCaller,
        requested_profile: &ProfileId,
        io_budget: Duration,
    ) -> Result<Self, StorageRootFailure> {
        if !identity.is_live() {
            return Err(StorageRootFailure::StaleSession);
        }
        if identity.principal_id() != caller.principal_id()
            || identity.caller_context_id() != caller.context_id()
        {
            return Err(StorageRootFailure::CallerMismatch);
        }
        if identity.profile_id() != requested_profile {
            return Err(StorageRootFailure::CrossProfile);
        }
        if io_budget.is_zero() || Instant::now().checked_add(io_budget).is_none() {
            return Err(StorageRootFailure::Unavailable);
        }
        Ok(Self {
            root: Arc::new(LinuxProfileStorageRoot {
                user_id: identity.user_id(),
                profile_id: requested_profile.clone(),
                bundle,
                io_budget,
            }),
            root_opened: false,
        })
    }

    pub(super) fn root_for_owner_effect(&self) -> Arc<LinuxProfileStorageRoot> {
        Arc::clone(&self.root)
    }

    fn check_root_and_object(
        &self,
        root: &Arc<LinuxProfileStorageRoot>,
        object_id: ObjectId,
    ) -> Result<(), StorageRootFailure> {
        if !Arc::ptr_eq(root, &self.root) || object_id != root.object_id() {
            return Err(StorageRootFailure::CrossProfile);
        }
        Ok(())
    }
}

impl ProfileStorageBackend for LinuxProfileStorageBackend {
    type Root = Arc<LinuxProfileStorageRoot>;

    fn open_profile_root(
        &mut self,
        user_id: Option<UserId>,
        profile_id: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        if user_id != self.root.user_id || profile_id != &self.root.profile_id {
            return Err(StorageRootFailure::CrossProfile);
        }
        if self.root_opened {
            return Err(StorageRootFailure::InvalidHandle);
        }
        self.root_opened = true;
        Ok(Arc::clone(&self.root))
    }

    fn read_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.check_root_and_object(root, object_id)?;
        root.with_object(|object| object.read())
            .map_err(map_owner_failure)?
            .map_err(map_object_failure)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.check_root_and_object(root, object_id)?;
        root.with_object(|mut object| object.write(data))
            .map_err(map_owner_failure)?
            .map_err(map_object_failure)
    }
}

pub(super) fn map_object_failure(error: NotesObjectError) -> StorageRootFailure {
    match error {
        NotesObjectError::Denied => StorageRootFailure::Denied,
        NotesObjectError::Unavailable => StorageRootFailure::Unavailable,
    }
}

fn map_owner_failure(
    error: nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
) -> StorageRootFailure {
    use nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerError;
    match error {
        ProfileVolumeNotesOwnerError::Revoked | ProfileVolumeNotesOwnerError::Ended => {
            StorageRootFailure::StaleSession
        }
        ProfileVolumeNotesOwnerError::CallerMismatch => StorageRootFailure::CallerMismatch,
        ProfileVolumeNotesOwnerError::ProfileMismatch => StorageRootFailure::CrossProfile,
        ProfileVolumeNotesOwnerError::ForeignAttachment => StorageRootFailure::Denied,
        ProfileVolumeNotesOwnerError::Unavailable => StorageRootFailure::Unavailable,
    }
}
