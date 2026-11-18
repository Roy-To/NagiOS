//! Startup-side wrapper for the shared opaque Notes attachment contract.
//!
//! The development Data mount may be retained as a typed boot-volume input,
//! but it is not a Profile identity, reservation, or authorization source.
//! The shared owner therefore remains Pending and every bundle request fails
//! closed until the real C1/C2 and Volume Manager producers are connected.

use std::fs::File;

pub(crate) use nagi_profile_volume_notes_owner::{
    ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError, ProfileVolumeNotesOwnerStatus,
};

/// Development device identity from the explicit QEMU serial and ext4 UUID.
/// This is not a product Nagi Volume ID and grants no access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DevelopmentBootVolumeIdentity {
    device_serial: String,
    filesystem_uuid: String,
    mount_id: u64,
}

impl DevelopmentBootVolumeIdentity {
    pub(crate) fn new(device_serial: String, filesystem_uuid: String, mount_id: u64) -> Self {
        Self {
            device_serial,
            filesystem_uuid,
            mount_id,
        }
    }
}

/// Typed, retained directory input for the private Profile Notes owner.
///
/// Construction is crate-private. Holding this FD does not authenticate a
/// Profile, establish a same-volume Notes ObjectId reservation, or issue a
/// Verified attachment.
pub(crate) struct BootVolumeInput {
    _directory: File,
    _identity: DevelopmentBootVolumeIdentity,
}

impl BootVolumeInput {
    pub(crate) fn from_development_data_mount(
        directory: File,
        identity: DevelopmentBootVolumeIdentity,
    ) -> Self {
        Self {
            _directory: directory,
            _identity: identity,
        }
    }
}

pub(crate) struct ProfileVolumeNotesOwner {
    inner: nagi_profile_volume_notes_owner::ProfileVolumeNotesOwner,
    boot_volume: Option<BootVolumeInput>,
}

impl ProfileVolumeNotesOwner {
    pub(crate) fn status(&self) -> ProfileVolumeNotesOwnerStatus {
        self.inner.status()
    }

    pub(crate) fn issue_bundle(
        &self,
    ) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        self.inner.issue_bundle()
    }

    pub(crate) fn has_boot_volume_input(&self) -> bool {
        self.boot_volume.is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.inner.revoke()
    }

    #[allow(dead_code)]
    pub(crate) fn revoke_bundle(
        &mut self,
        bundle: &ProfileVolumeNotesBundle,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.inner.revoke_bundle(bundle)
    }

    pub(crate) fn end(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        let result = self.inner.end();
        self.boot_volume.take();
        result
    }
}

pub(crate) fn pending() -> ProfileVolumeNotesOwner {
    ProfileVolumeNotesOwner {
        inner: nagi_profile_volume_notes_owner::ProfileVolumeNotesOwner::pending(),
        boot_volume: None,
    }
}

pub(crate) fn pending_with_boot_volume(input: BootVolumeInput) -> ProfileVolumeNotesOwner {
    ProfileVolumeNotesOwner {
        inner: nagi_profile_volume_notes_owner::ProfileVolumeNotesOwner::pending(),
        boot_volume: Some(input),
    }
}
