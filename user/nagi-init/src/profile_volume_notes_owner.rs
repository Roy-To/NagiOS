//! Startup-side wrapper for the shared opaque Notes attachment contract.
//!
//! The development Data mount may be retained as a typed boot-volume input,
//! but it is not a Profile identity, reservation, or authorization source.
//! The shared owner therefore remains Pending and every bundle request fails
//! closed until the real C1/C2 and Volume Manager producers are connected.

use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

use nagi_linux_account_verifier::LinuxAccountVerifier;
use rustix::fs::{openat2, Mode, OFlags, ResolveFlags};

pub(crate) use nagi_profile_volume_notes_owner::{
    ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError, ProfileVolumeNotesOwnerStatus,
};

/// Data root selected by the trusted volume owner. Opening a directory
/// through this root proves containment on the same mount. Profile
/// authorization and Notes ObjectId reservation are supplied separately.
pub(crate) struct SelectedDataRoot {
    directory: File,
}

impl SelectedDataRoot {
    pub(crate) fn from_selected_volume(directory: File) -> io::Result<Self> {
        Self::from_owner(directory, 0)
    }

    fn from_owner(directory: File, expected_uid: u32) -> io::Result<Self> {
        let metadata = directory.metadata()?;
        if !metadata.is_dir() || metadata.uid() != expected_uid || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "selected Data root is not an owner-controlled directory",
            ));
        }
        Ok(Self { directory })
    }

    /// Build the verifier once from the selected Data volume's protected
    /// account directory. A and C1 must retain this same verifier instance.
    pub(crate) fn prepare_account_verifier(
        &self,
        account_directory: &Path,
    ) -> io::Result<LinuxAccountVerifier> {
        LinuxAccountVerifier::from_protected_directory(
            self.retain_profile_directory(account_directory)?,
        )
        .map_err(|error| io::Error::other(format!("account directory rejected: {error:?}")))
    }

    /// Retain the owner-selected profile directory descriptor. NO_XDEV
    /// forbids a distinct filesystem or bind mount below the Data root.
    pub(crate) fn retain_profile_directory(&self, relative: &Path) -> io::Result<File> {
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a relative Data directory below the selected root",
            ));
        }
        Ok(File::from(openat2(
            &self.directory,
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )?))
    }
}

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
    directory: File,
    _identity: DevelopmentBootVolumeIdentity,
}

impl BootVolumeInput {
    pub(crate) fn from_development_data_mount(
        directory: File,
        identity: DevelopmentBootVolumeIdentity,
    ) -> Self {
        Self {
            directory,
            _identity: identity,
        }
    }

    /// Retain the volume already selected by the mount owner for boot-time
    /// identity inputs. The mount owner still controls its lifetime.
    pub(crate) fn selected_data_root(&self) -> io::Result<SelectedDataRoot> {
        SelectedDataRoot::from_selected_volume(self.directory.try_clone()?)
    }

    pub(crate) fn identity_inputs(
        &self,
        accounts: &Path,
        snapshots: &Path,
    ) -> io::Result<(File, LinuxAccountVerifier)> {
        let root = self.selected_data_root()?;
        let verifier = root.prepare_account_verifier(accounts).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("protected account directory: {error}"),
            )
        })?;
        let directory = root.retain_profile_directory(snapshots).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("protected identity snapshot directory: {error}"),
            )
        })?;
        let metadata = directory.metadata()?;
        if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "identity snapshot directory is not private to the boot owner",
            ));
        }
        Ok((directory, verifier))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn retains_account_and_profile_directories_under_the_same_data_root() {
        let path =
            std::env::temp_dir().join(format!("nagi-selected-data-root-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("accounts")).unwrap();
        fs::create_dir_all(path.join("profiles/owner")).unwrap();
        let directory = File::open(&path).unwrap();
        let uid = directory.metadata().unwrap().uid();
        let data = if uid == 0 {
            SelectedDataRoot::from_selected_volume(directory)
        } else {
            SelectedDataRoot::from_owner(directory, uid)
        }
        .unwrap();

        if uid == 0 {
            data.prepare_account_verifier(Path::new("accounts"))
                .unwrap();
        }
        let retained = data
            .retain_profile_directory(Path::new("profiles/owner"))
            .unwrap();
        let expected = fs::metadata(path.join("profiles/owner")).unwrap();
        let observed = retained.metadata().unwrap();
        assert_eq!(
            (observed.dev(), observed.ino()),
            (expected.dev(), expected.ino())
        );
        drop(retained);
        fs::remove_dir(path.join("profiles/owner")).unwrap();
        fs::remove_dir(path.join("profiles")).unwrap();
        fs::remove_dir(path.join("accounts")).unwrap();
        fs::remove_dir(path).unwrap();
    }
}

pub(crate) fn pending_with_boot_volume(input: BootVolumeInput) -> ProfileVolumeNotesOwner {
    ProfileVolumeNotesOwner {
        inner: nagi_profile_volume_notes_owner::ProfileVolumeNotesOwner::pending(),
        boot_volume: Some(input),
    }
}
