//! Startup-side wrapper for the shared opaque Notes attachment contract.
//!
//! PID 1 retains the selected Data root and invokes the Volume Manager
//! producer after C1 attaches the authenticated interactive Session.

use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

use nagi_linux_account_verifier::LinuxAccountVerifier;
use rustix::fs::{openat2, Mode, OFlags, ResolveFlags};

#[cfg(linux_task24_initial_login_runtime)]
use nagi_identity_session::{
    C1AuthenticatedInteractiveCaller, CallerContextId, PrincipalId, ProfileId,
    ProfileRootAuthorizer, ResolvedIdentity, SessionId, SessionKind, StorageRootFailure,
    TrustedCallerContext, UserId,
};

pub(crate) use nagi_profile_volume_notes_owner::{
    ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError, ProfileVolumeNotesOwnerStatus,
    NOTES_APP_GID, NOTES_APP_UID, USER_FILES_APP_GID, USER_FILES_APP_UID,
};

#[cfg(linux_task24_initial_login_runtime)]
use nagi_profile_volume_notes_owner::ProfileVolumeNotesAppProcessRoot;

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

    pub(crate) fn retain_identity_snapshot_directory(&self, relative: &Path) -> io::Result<File> {
        let directory = self.retain_profile_directory(relative)?;
        let metadata = directory.metadata()?;
        if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "identity snapshot directory is not private to the boot owner",
            ));
        }
        Ok(directory)
    }
}

/// Development device identity from the observed block-device serial and the
/// ext4 UUID accepted by the signed development selection. This is not a
/// product Nagi Volume ID and grants no access.
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
    identity: DevelopmentBootVolumeIdentity,
}

impl BootVolumeInput {
    pub(crate) fn from_development_data_mount(
        directory: File,
        identity: DevelopmentBootVolumeIdentity,
    ) -> Self {
        Self {
            directory,
            identity,
        }
    }

    /// Retain the volume already selected by the mount owner for boot-time
    /// identity inputs. The mount owner still controls its lifetime.
    pub(crate) fn selected_data_root(&self) -> io::Result<SelectedDataRoot> {
        SelectedDataRoot::from_selected_volume(self.directory.try_clone()?)
    }

    #[cfg(linux_task24_initial_login_runtime)]
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            directory: self.directory.try_clone()?,
            identity: self.identity.clone(),
        })
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
        let directory = root
            .retain_identity_snapshot_directory(snapshots)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("protected identity snapshot directory: {error}"),
                )
            })?;
        Ok((directory, verifier))
    }
}

/// Issued together from the verified Notes attachment and the exact
/// interactive Session. The factory's bundle and C1 guard enforce liveness
/// and descriptor ownership during each operation.
#[cfg(linux_task24_initial_login_runtime)]
pub(crate) struct AuthorizedNotesAttachment {
    pub(crate) bundle: ProfileVolumeNotesBundle,
    pub(crate) authorizer: NotesProfileRootAuthorizer,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(crate) struct NotesProfileRootAuthorizer {
    session: SessionId,
    caller: CallerContextId,
    principal: PrincipalId,
    profile: ProfileId,
    user: UserId,
}

#[cfg(linux_task24_initial_login_runtime)]
impl ProfileRootAuthorizer for NotesProfileRootAuthorizer {
    fn authorize_profile_root(
        &mut self,
        caller: &dyn TrustedCallerContext,
        identity: &ResolvedIdentity,
        requested_profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        if caller.principal_id() != &self.principal
            || caller.context_id() != &self.caller
            || identity.principal_id() != &self.principal
            || identity.caller_context_id() != &self.caller
        {
            return Err(StorageRootFailure::CallerMismatch);
        }
        if requested_profile != &self.profile
            || identity.profile_id() != &self.profile
            || identity.user_id() != Some(self.user)
        {
            return Err(StorageRootFailure::CrossProfile);
        }
        if identity.session_id() != &self.session || identity.kind() != SessionKind::LocalUser {
            return Err(StorageRootFailure::Denied);
        }
        Ok(())
    }
}

#[cfg(linux_task24_initial_login_runtime)]
pub(crate) use nagi_profile_volume_notes_owner::{
    ProfileSnapshotStorage, SingleObjectSnapshotStorageError, SnapshotStorage, SnapshotStorageError,
};

/// The consumer receives a typed view only while the attached C1 lease and
/// the matching Data bundle are both active.
#[cfg(linux_task24_initial_login_runtime)]
#[derive(Clone, Copy)]
pub(crate) enum ProfileSnapshotKind {
    Settings,
    Calendar,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(crate) struct AuthorizedProfileSnapshot {
    storage: ProfileSnapshotStorage,
    revalidation: std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
}

#[cfg(linux_task24_initial_login_runtime)]
impl AuthorizedProfileSnapshot {
    #[cfg(all(linux_task24_calendar_app, not(nagi_release_build)))]
    pub(crate) fn claim_calendar_initialization(
        &self,
    ) -> Result<bool, ProfileVolumeNotesOwnerError> {
        self.revalidation
            .with_live_session_effect(|| self.storage.claim_calendar_initialization())
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?
    }

    pub(crate) fn with_storage<R>(
        &mut self,
        operation: impl FnOnce(
            nagi_model::ObjectId,
            &mut dyn SnapshotStorage<Error = SingleObjectSnapshotStorageError<SnapshotStorageError>>,
        ) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        self.revalidation
            .with_live_session_effect(|| self.storage.with_active_snapshot(operation))
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?
    }
}

#[cfg(linux_task24_initial_login_runtime)]
pub(crate) struct AuthorizedFilesRoot {
    root: nagi_profile_volume_notes_owner::ProfileFilesRoot,
    revalidation: std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(crate) struct AuthorizedNotesAppProcessRoot {
    root: ProfileVolumeNotesAppProcessRoot,
    revalidation: std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
}

/// Single borrowed scope that couples one attached profile root to the exact
/// C1 handle used when that root was issued. The provider enters the C1 effect
/// from this value before invoking its owner-only root callback.
#[cfg(all(linux_task24_initial_login_runtime, feature = "linux-files-desktop"))]
pub(crate) struct OwnerOnlyFilesRootScope<'root> {
    owner: &'root AuthorizedFilesRoot,
}

#[cfg(all(linux_task24_initial_login_runtime, feature = "linux-files-desktop"))]
impl nagi_profile_volume_notes_owner::FilesRootScope for OwnerOnlyFilesRootScope<'_> {
    fn with_files_root<R>(
        &self,
        effect: impl for<'view> FnOnce(nagi_profile_volume_notes_owner::FilesRootView<'view>) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        <nagi_profile_volume_notes_owner::ProfileFilesRoot as
            nagi_profile_volume_notes_owner::FilesRootScope>::with_files_root(
            &self.owner.root,
            effect,
        )
    }
}

#[cfg(all(linux_task24_initial_login_runtime, feature = "linux-files-desktop"))]
impl nagi_files::C1SessionBoundFilesRootScope for OwnerOnlyFilesRootScope<'_> {
    fn c1_session_revalidation_handle(
        &self,
    ) -> &nagi_identity_session::SessionLeaseRevalidationHandle {
        self.owner.revalidation.as_ref()
    }
}

#[cfg(linux_task24_initial_login_runtime)]
impl AuthorizedFilesRoot {
    #[cfg(feature = "linux-files-desktop")]
    pub(crate) fn owner_only_scope(&self) -> OwnerOnlyFilesRootScope<'_> {
        OwnerOnlyFilesRootScope { owner: self }
    }

    /// Issue the exact attached User Files root to its matching signed app
    /// process. The C1 lease and volume owner remain retained until the app
    /// child is reaped; every PID-1 poll rechecks the lease and revokes a child
    /// whose Session is no longer live.
    pub(crate) fn into_process_storage(
        self,
        app_id: nagi_model::AppId,
    ) -> Result<AuthorizedAppStorageRoot, ProfileVolumeNotesOwnerError> {
        let files_app = nagi_model::AppId::from_identifier(b"org.nagi.files");
        if app_id != files_app {
            return Err(ProfileVolumeNotesOwnerError::ForeignAttachment);
        }
        let directory = self
            .revalidation
            .with_live_session_effect(|| self.root.duplicate_directory_for_app_process(&app_id))
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
        Ok(AuthorizedAppStorageRoot {
            app_id,
            directory,
            owner: Some(AuthorizedAppStorageOwner::Files(self)),
            #[cfg(test)]
            allow_unbound_session_guard_for_test: false,
        })
    }
}

#[cfg(linux_task24_initial_login_runtime)]
impl AuthorizedNotesAppProcessRoot {
    /// Convert this exact session-bound Notes process root into the generic
    /// process launch capability. The app ID is independently checked by the
    /// profile-volume owner before a descriptor is duplicated.
    pub(crate) fn into_process_storage(
        self,
        app_id: nagi_model::AppId,
    ) -> Result<AuthorizedAppStorageRoot, ProfileVolumeNotesOwnerError> {
        let notes_app = nagi_model::AppId::from_identifier(b"org.nagi.notes");
        if app_id != notes_app {
            return Err(ProfileVolumeNotesOwnerError::ForeignAttachment);
        }
        let directory = self
            .revalidation
            .with_live_session_effect(|| self.root.duplicate_directory_for_app_process(&app_id))
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
        Ok(AuthorizedAppStorageRoot {
            app_id,
            directory,
            owner: Some(AuthorizedAppStorageOwner::Notes(self)),
            #[cfg(test)]
            allow_unbound_session_guard_for_test: false,
        })
    }
}

#[cfg(linux_task24_initial_login_runtime)]
enum AuthorizedAppStorageOwner {
    Files(AuthorizedFilesRoot),
    Notes(AuthorizedNotesAppProcessRoot),
}

#[cfg(linux_task24_initial_login_runtime)]
impl AuthorizedAppStorageOwner {
    fn revalidation(
        &self,
    ) -> &std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle> {
        match self {
            Self::Files(owner) => &owner.revalidation,
            Self::Notes(owner) => &owner.revalidation,
        }
    }
}

/// Non-forgeable, Session-bound root passed to one exact app launch. Only the
/// profile-volume owner can construct a production value; LinuxProcessOs keeps
/// it alive until exact child reap and checks the C1 lease while polling.
pub(crate) struct AuthorizedAppStorageRoot {
    app_id: nagi_model::AppId,
    directory: std::os::fd::OwnedFd,
    #[cfg(linux_task24_initial_login_runtime)]
    owner: Option<AuthorizedAppStorageOwner>,
    #[cfg(test)]
    allow_unbound_session_guard_for_test: bool,
}

#[derive(Clone)]
pub(crate) struct AuthorizedAppStorageSessionGuard {
    #[cfg(linux_task24_initial_login_runtime)]
    revalidation: Option<std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>>,
    #[cfg(test)]
    allow_unbound_for_test: bool,
}

impl AuthorizedAppStorageSessionGuard {
    pub(crate) fn with_live_session_effect<R>(
        &self,
        effect: impl FnOnce() -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        #[cfg(linux_task24_initial_login_runtime)]
        if let Some(revalidation) = self.revalidation.as_ref() {
            return revalidation
                .with_live_session_effect(effect)
                .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable);
        }
        #[cfg(test)]
        if self.allow_unbound_for_test {
            return Ok(effect());
        }
        let _ = effect;
        Err(ProfileVolumeNotesOwnerError::Unavailable)
    }
}

impl AuthorizedAppStorageRoot {
    pub(crate) fn app_id(&self) -> nagi_model::AppId {
        self.app_id
    }

    pub(crate) fn directory(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.directory)
    }

    pub(crate) fn session_is_live(&self) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.session_guard()
            .with_live_session_effect(|| ())
            .map(|_| ())
    }

    pub(crate) fn session_guard(&self) -> AuthorizedAppStorageSessionGuard {
        #[cfg(linux_task24_initial_login_runtime)]
        let revalidation = self
            .owner
            .as_ref()
            .map(|owner| std::sync::Arc::clone(owner.revalidation()));
        AuthorizedAppStorageSessionGuard {
            #[cfg(linux_task24_initial_login_runtime)]
            revalidation,
            #[cfg(test)]
            allow_unbound_for_test: self.allow_unbound_session_guard_for_test,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(app_id: nagi_model::AppId, directory: std::os::fd::OwnedFd) -> Self {
        Self {
            app_id,
            directory,
            #[cfg(linux_task24_initial_login_runtime)]
            owner: None,
            #[cfg(test)]
            allow_unbound_session_guard_for_test: true,
        }
    }
}

pub(crate) struct AppStorageOwners {
    inner: nagi_profile_volume_notes_owner::AppStorageOwners,
    boot_volume: Option<BootVolumeInput>,
}

impl AppStorageOwners {
    pub(crate) fn status(&self) -> ProfileVolumeNotesOwnerStatus {
        self.inner.notes().status()
    }

    pub(crate) fn issue_bundle(
        &self,
    ) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        self.inner.notes().issue_bundle()
    }

    /// Called under the exact C1 Session effect guard after C2 attachment.
    /// The Volume Manager owns persistent profile and object selection.
    #[cfg(linux_task24_initial_login_runtime)]
    pub(crate) fn issue_for_attached_session(
        &mut self,
        caller: &C1AuthenticatedInteractiveCaller,
        identity: &ResolvedIdentity,
    ) -> Result<AuthorizedNotesAttachment, ProfileVolumeNotesOwnerError> {
        let user = self.attach_session_profile(caller, identity)?;
        let bundle = self.issue_bundle()?;
        Ok(AuthorizedNotesAttachment {
            bundle,
            authorizer: NotesProfileRootAuthorizer {
                session: identity.session_id().clone(),
                caller: caller.context_id().clone(),
                principal: caller.principal_id().clone(),
                profile: identity.profile_id().clone(),
                user,
            },
        })
    }

    #[cfg(linux_task24_initial_login_runtime)]
    pub(crate) fn issue_snapshot_for_attached_session(
        &mut self,
        caller: &C1AuthenticatedInteractiveCaller,
        identity: &ResolvedIdentity,
        revalidation: &std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
        kind: ProfileSnapshotKind,
    ) -> Result<AuthorizedProfileSnapshot, ProfileVolumeNotesOwnerError> {
        self.attach_session_profile(caller, identity)?;
        let storage = match kind {
            ProfileSnapshotKind::Settings => {
                ProfileSnapshotStorage::open_settings(self.inner.settings().issue_bundle()?)
            }
            ProfileSnapshotKind::Calendar => {
                ProfileSnapshotStorage::open_calendar(self.inner.calendar().issue_bundle()?)
            }
        }?;
        Ok(AuthorizedProfileSnapshot {
            storage,
            revalidation: std::sync::Arc::clone(revalidation),
        })
    }

    #[cfg(linux_task24_initial_login_runtime)]
    pub(crate) fn issue_files_root_for_attached_session(
        &mut self,
        caller: &C1AuthenticatedInteractiveCaller,
        identity: &ResolvedIdentity,
        revalidation: &std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
    ) -> Result<AuthorizedFilesRoot, ProfileVolumeNotesOwnerError> {
        use ProfileVolumeNotesOwnerError::Unavailable;

        let user = self.attach_session_profile(caller, identity)?;
        let boot_volume = self.boot_volume.as_ref().ok_or(Unavailable)?;
        let owner = self.inner.user_files();
        if owner.status() == ProfileVolumeNotesOwnerStatus::Pending {
            owner.attach_linux_profile(
                &boot_volume.directory,
                boot_volume.identity.mount_id,
                user.to_model(),
                identity.profile_id().as_str(),
                identity.principal_id().as_str(),
            )?;
        }
        let root = owner.issue_files_root()?;
        root.validate_scope(
            identity.principal_id().as_str(),
            identity.profile_id().as_str(),
            &boot_volume.directory,
            boot_volume.identity.mount_id,
        )?;
        Ok(AuthorizedFilesRoot {
            root,
            revalidation: std::sync::Arc::clone(revalidation),
        })
    }

    /// Issue the Notes app-specific process root from the exact C1-attached
    /// profile and retain the live Session authority through child reap.
    #[cfg(linux_task24_initial_login_runtime)]
    pub(crate) fn issue_notes_app_process_root_for_attached_session(
        &mut self,
        caller: &C1AuthenticatedInteractiveCaller,
        identity: &ResolvedIdentity,
        revalidation: &std::sync::Arc<nagi_identity_session::SessionLeaseRevalidationHandle>,
    ) -> Result<AuthorizedNotesAppProcessRoot, ProfileVolumeNotesOwnerError> {
        use ProfileVolumeNotesOwnerError::Unavailable;

        self.attach_session_profile(caller, identity)?;
        let boot_volume = self.boot_volume.as_ref().ok_or(Unavailable)?;
        let root = self.inner.notes().issue_notes_app_process_root()?;
        root.validate_scope(
            identity.principal_id().as_str(),
            identity.profile_id().as_str(),
            &boot_volume.directory,
            boot_volume.identity.mount_id,
        )?;
        Ok(AuthorizedNotesAppProcessRoot {
            root,
            revalidation: std::sync::Arc::clone(revalidation),
        })
    }

    #[cfg(linux_task24_initial_login_runtime)]
    fn attach_session_profile(
        &mut self,
        caller: &C1AuthenticatedInteractiveCaller,
        identity: &ResolvedIdentity,
    ) -> Result<UserId, ProfileVolumeNotesOwnerError> {
        use ProfileVolumeNotesOwnerError::{CallerMismatch, Unavailable};

        if caller.principal_id() != identity.principal_id()
            || caller.context_id() != identity.caller_context_id()
        {
            return Err(CallerMismatch);
        }
        let boot_volume = self.boot_volume.as_ref().ok_or(Unavailable)?;
        if identity.kind() != SessionKind::LocalUser {
            return Err(Unavailable);
        }
        let user = identity.user_id().ok_or(Unavailable)?;
        for owner in self.inner.owners_mut() {
            if owner.status() == ProfileVolumeNotesOwnerStatus::Pending {
                owner.attach_linux_profile(
                    &boot_volume.directory,
                    boot_volume.identity.mount_id,
                    user.to_model(),
                    identity.profile_id().as_str(),
                    identity.principal_id().as_str(),
                )?;
            }
            let bundle = owner.issue_bundle()?;
            bundle.validate_scope(
                identity.principal_id().as_str(),
                identity.profile_id().as_str(),
            )?;
            bundle.validate_data_mount(&boot_volume.directory, boot_volume.identity.mount_id)?;
        }
        Ok(user)
    }

    pub(crate) fn has_boot_volume_input(&self) -> bool {
        self.boot_volume.is_some()
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.inner.revoke()
    }

    pub(crate) fn end(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        let result = self.inner.end();
        self.boot_volume.take();
        result
    }
}

pub(crate) fn pending() -> AppStorageOwners {
    AppStorageOwners {
        inner: nagi_profile_volume_notes_owner::AppStorageOwners::pending(),
        boot_volume: None,
    }
}

pub(crate) fn pending_with_boot_volume(input: BootVolumeInput) -> AppStorageOwners {
    AppStorageOwners {
        inner: nagi_profile_volume_notes_owner::AppStorageOwners::pending(),
        boot_volume: Some(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_app_session_lease_fails_closed_without_test_fixture_override() {
        let guard = AuthorizedAppStorageSessionGuard {
            #[cfg(linux_task24_initial_login_runtime)]
            revalidation: None,
            allow_unbound_for_test: false,
        };
        let effect_called = std::cell::Cell::new(false);
        assert!(guard
            .with_live_session_effect(|| effect_called.set(true))
            .is_err());
        assert!(!effect_called.get());
    }

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
