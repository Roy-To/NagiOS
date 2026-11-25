//! Opaque A-owned handoff from one verified profile-volume attachment to Notes.
//!
//! The Volume Manager owns profile directory selection and ObjectId
//! reservation. Consumers receive the resulting opaque attachment.

use nagi_model::ObjectId;
use rustix::io::fcntl_dupfd_cloexec;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(target_os = "linux")]
mod linux_data;

#[cfg(target_os = "linux")]
mod files_root;

#[cfg(target_os = "linux")]
pub use files_root::{
    FilesRootDirView, FilesRootEntry, FilesRootEntryKind, FilesRootFileView, FilesRootMetadata,
    FilesRootView,
};

mod snapshot;

pub use nagi_persistence_contract::{ConditionalWriteOutcome, Revision, Snapshot, SnapshotStorage};
pub use snapshot::{SingleObjectSnapshotStorageError, SnapshotStorageError};

/// The storage namespace selected by the profile-volume owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppStorageKind {
    Notes,
    Settings,
    Calendar,
    UserFiles,
}

/// Current terminal or live state of the startup attachment owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileVolumeNotesOwnerStatus {
    Pending,
    Verified,
    Revoked,
    Ended,
}

/// A fail-closed bundle or owner-state rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileVolumeNotesOwnerError {
    Unavailable,
    CallerMismatch,
    ProfileMismatch,
    ForeignAttachment,
    Revoked,
    Ended,
}

impl fmt::Display for ProfileVolumeNotesOwnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "verified profile-volume Notes attachment is unavailable",
            Self::CallerMismatch => "Notes caller does not match the live identity",
            Self::ProfileMismatch => "Notes profile does not match the verified attachment",
            Self::ForeignAttachment => "Notes bundle belongs to another attachment",
            Self::Revoked => "profile-volume Notes attachment was revoked",
            Self::Ended => "profile-volume Notes attachment has ended",
        })
    }
}

impl std::error::Error for ProfileVolumeNotesOwnerError {}

struct VerifiedProfileVolumeAttachment {
    kind: AppStorageKind,
    principal_id: String,
    profile_id: String,
    object_directory: OwnedFd,
    object_id: Option<ObjectId>,
    #[cfg(target_os = "linux")]
    allow_initial_index: bool,
    lifecycle: Mutex<AttachmentLifecycle>,
}

struct AttachmentLifecycle {
    status: AttachmentStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachmentStatus {
    Active,
    Revoked,
    Ended,
}

impl VerifiedProfileVolumeAttachment {
    #[cfg(any(test, target_os = "linux"))]
    fn new(
        kind: AppStorageKind,
        principal_id: impl Into<String>,
        profile_id: impl Into<String>,
        object_directory: OwnedFd,
        object_id: Option<ObjectId>,
    ) -> Self {
        Self {
            kind,
            principal_id: principal_id.into(),
            profile_id: profile_id.into(),
            object_directory,
            object_id,
            #[cfg(target_os = "linux")]
            allow_initial_index: false,
            lifecycle: Mutex::new(AttachmentLifecycle {
                status: AttachmentStatus::Active,
            }),
        }
    }

    fn lifecycle(
        &self,
    ) -> Result<MutexGuard<'_, AttachmentLifecycle>, ProfileVolumeNotesOwnerError> {
        self.lifecycle
            .lock()
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)
    }

    fn terminate(&self, status: AttachmentStatus) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.lifecycle()?.status = status;
        Ok(())
    }
}

enum OwnerState {
    Pending,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Verified(Arc<VerifiedProfileVolumeAttachment>),
    Revoked,
    Ended,
}

/// A-owned state for one selected profile-volume attachment.
///
/// This owner instance is also the storage namespace key: do not merge
/// attachments by PrincipalId/ProfileId alone. Reopening the same logical root
/// must reuse this owner, while another trust domain gets a separate owner.
pub struct ProfileVolumeAppOwner {
    kind: AppStorageKind,
    state: OwnerState,
}

pub type ProfileVolumeNotesOwner = ProfileVolumeAppOwner;

impl ProfileVolumeAppOwner {
    /// Start fail closed until the selected-profile-volume source is wired.
    pub const fn pending() -> Self {
        Self::pending_for(AppStorageKind::Notes)
    }

    pub const fn pending_for(kind: AppStorageKind) -> Self {
        Self {
            kind,
            state: OwnerState::Pending,
        }
    }

    pub const fn kind(&self) -> AppStorageKind {
        self.kind
    }

    /// Volume Manager entry point for the boot-selected Linux Data mount.
    /// PID 1 supplies claims from its live C1 Session while holding the
    /// Session effect guard. Directory names and ObjectId are selected here.
    #[cfg(target_os = "linux")]
    pub fn attach_linux_profile(
        &mut self,
        data_root: &impl std::os::fd::AsFd,
        mount_id: u64,
        user: nagi_model::UserId,
        profile: &str,
        principal: &str,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        use ProfileVolumeNotesOwnerError::{Ended, ForeignAttachment, Revoked, Unavailable};
        match self.state {
            OwnerState::Pending => {}
            OwnerState::Verified(_) => return Err(ForeignAttachment),
            OwnerState::Revoked => return Err(Revoked),
            OwnerState::Ended => return Err(Ended),
        }
        let (directory, object, allow_initial_index) = if self.kind == AppStorageKind::UserFiles {
            let directory =
                linux_data::reserve_files_root(data_root, mount_id, user, profile, principal)
                    .map_err(|_| Unavailable)?;
            (directory, None, false)
        } else {
            let (directory, object, allow_initial_index) =
                linux_data::reserve(data_root, mount_id, user, profile, principal, self.kind)
                    .map_err(|_| Unavailable)?;
            (directory, Some(object), allow_initial_index)
        };
        let mut attachment =
            VerifiedProfileVolumeAttachment::new(self.kind, principal, profile, directory, object);
        attachment.allow_initial_index = allow_initial_index;
        self.state = OwnerState::Verified(Arc::new(attachment));
        Ok(())
    }

    pub fn status(&self) -> ProfileVolumeNotesOwnerStatus {
        match self.state {
            OwnerState::Pending => ProfileVolumeNotesOwnerStatus::Pending,
            OwnerState::Verified(_) => ProfileVolumeNotesOwnerStatus::Verified,
            OwnerState::Revoked => ProfileVolumeNotesOwnerStatus::Revoked,
            OwnerState::Ended => ProfileVolumeNotesOwnerStatus::Ended,
        }
    }

    /// Issue a consumer capability only from the owner's existing verified
    /// attachment. The attachment already binds the scope, retained FD, and
    /// reserved Notes ObjectId; this method accepts no path, FD, ID, or
    /// caller-supplied metadata. The Notes consumer separately binds the
    /// bundle to the live C1 identity and C2-sealed caller.
    pub fn issue_bundle(&self) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        if self.kind == AppStorageKind::UserFiles {
            return Err(ProfileVolumeNotesOwnerError::ForeignAttachment);
        }
        self.issue_active_attachment()
    }

    #[cfg(target_os = "linux")]
    pub fn issue_files_root(&self) -> Result<ProfileFilesRoot, ProfileVolumeNotesOwnerError> {
        if self.kind != AppStorageKind::UserFiles {
            return Err(ProfileVolumeNotesOwnerError::ForeignAttachment);
        }
        self.issue_active_attachment().map(ProfileFilesRoot)
    }

    fn issue_active_attachment(
        &self,
    ) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        let attachment = match &self.state {
            OwnerState::Pending => return Err(ProfileVolumeNotesOwnerError::Unavailable),
            OwnerState::Verified(attachment) => attachment,
            OwnerState::Revoked => return Err(ProfileVolumeNotesOwnerError::Revoked),
            OwnerState::Ended => return Err(ProfileVolumeNotesOwnerError::Ended),
        };
        let lifecycle = attachment.lifecycle()?;
        if lifecycle.status != AttachmentStatus::Active {
            return Err(attachment_status_error(lifecycle.status));
        }
        drop(lifecycle);
        Ok(ProfileVolumeNotesBundle {
            attachment: Arc::clone(attachment),
        })
    }

    /// Revoke the current attachment. The lifecycle mutex serializes this
    /// transition with each consumer effect, so once revoke returns no new
    /// Notes effect can enter through an outstanding bundle.
    pub fn revoke(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        let old = std::mem::replace(&mut self.state, OwnerState::Ended);
        self.state = match old {
            OwnerState::Pending => OwnerState::Revoked,
            OwnerState::Verified(attachment) => {
                attachment.terminate(AttachmentStatus::Revoked)?;
                OwnerState::Revoked
            }
            OwnerState::Revoked => OwnerState::Revoked,
            OwnerState::Ended => OwnerState::Ended,
        };
        Ok(())
    }

    /// End the owner lifetime and invalidate all outstanding bundles.
    pub fn end(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        let old = std::mem::replace(&mut self.state, OwnerState::Ended);
        if let OwnerState::Verified(attachment) = old {
            attachment.terminate(AttachmentStatus::Ended)?;
        }
        Ok(())
    }

    /// Confirm that a bundle belongs to this exact attachment before an
    /// attachment-scoped lifecycle event is applied.
    pub fn revoke_bundle(
        &mut self,
        bundle: &ProfileVolumeNotesBundle,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        match &self.state {
            OwnerState::Verified(attachment) if Arc::ptr_eq(attachment, &bundle.attachment) => {
                self.revoke()
            }
            OwnerState::Verified(_) => Err(ProfileVolumeNotesOwnerError::ForeignAttachment),
            OwnerState::Pending => Err(ProfileVolumeNotesOwnerError::Unavailable),
            OwnerState::Revoked => Err(ProfileVolumeNotesOwnerError::Revoked),
            OwnerState::Ended => Err(ProfileVolumeNotesOwnerError::Ended),
        }
    }

    #[cfg(test)]
    fn issue_for_claims(
        &self,
        principal_id: &str,
        profile_id: &str,
    ) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        let attachment = match &self.state {
            OwnerState::Pending => return Err(ProfileVolumeNotesOwnerError::Unavailable),
            OwnerState::Verified(attachment) => attachment,
            OwnerState::Revoked => return Err(ProfileVolumeNotesOwnerError::Revoked),
            OwnerState::Ended => return Err(ProfileVolumeNotesOwnerError::Ended),
        };
        let lifecycle = attachment.lifecycle()?;
        if lifecycle.status != AttachmentStatus::Active {
            return Err(attachment_status_error(lifecycle.status));
        }
        if principal_id != attachment.principal_id {
            return Err(ProfileVolumeNotesOwnerError::CallerMismatch);
        }
        if profile_id != attachment.profile_id {
            return Err(ProfileVolumeNotesOwnerError::ProfileMismatch);
        }
        drop(lifecycle);
        Ok(ProfileVolumeNotesBundle {
            attachment: Arc::clone(attachment),
        })
    }

    #[cfg(test)]
    fn verified_fixture(
        principal_id: impl Into<String>,
        profile_id: impl Into<String>,
        object_directory: OwnedFd,
        notes_object_id: ObjectId,
    ) -> Self {
        Self::verified_fixture_for(
            AppStorageKind::Notes,
            principal_id,
            profile_id,
            object_directory,
            notes_object_id,
        )
    }

    #[cfg(test)]
    fn verified_fixture_for(
        kind: AppStorageKind,
        principal_id: impl Into<String>,
        profile_id: impl Into<String>,
        object_directory: OwnedFd,
        notes_object_id: ObjectId,
    ) -> Self {
        Self {
            kind,
            state: OwnerState::Verified(Arc::new(VerifiedProfileVolumeAttachment::new(
                kind,
                principal_id,
                profile_id,
                object_directory,
                Some(notes_object_id),
            ))),
        }
    }
}

/// Storage attachments owned together for one interactive Session.
pub struct AppStorageOwners {
    notes: ProfileVolumeAppOwner,
    settings: ProfileVolumeAppOwner,
    calendar: ProfileVolumeAppOwner,
    user_files: ProfileVolumeAppOwner,
}

impl AppStorageOwners {
    pub fn pending() -> Self {
        Self {
            notes: ProfileVolumeAppOwner::pending_for(AppStorageKind::Notes),
            settings: ProfileVolumeAppOwner::pending_for(AppStorageKind::Settings),
            calendar: ProfileVolumeAppOwner::pending_for(AppStorageKind::Calendar),
            user_files: ProfileVolumeAppOwner::pending_for(AppStorageKind::UserFiles),
        }
    }

    pub fn notes(&self) -> &ProfileVolumeAppOwner {
        &self.notes
    }

    pub fn settings(&self) -> &ProfileVolumeAppOwner {
        &self.settings
    }

    pub fn calendar(&self) -> &ProfileVolumeAppOwner {
        &self.calendar
    }

    pub fn user_files(&mut self) -> &mut ProfileVolumeAppOwner {
        &mut self.user_files
    }

    pub fn owners_mut(&mut self) -> [&mut ProfileVolumeAppOwner; 3] {
        [&mut self.notes, &mut self.settings, &mut self.calendar]
    }

    pub fn revoke(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.apply_to_all(ProfileVolumeAppOwner::revoke)
    }

    pub fn end(&mut self) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.apply_to_all(ProfileVolumeAppOwner::end)
    }

    fn apply_to_all(
        &mut self,
        transition: fn(&mut ProfileVolumeAppOwner) -> Result<(), ProfileVolumeNotesOwnerError>,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        let mut result = Ok(());
        for owner in self.owners_mut() {
            // Attempt every transition even when one owner fails closed.
            result = result.and(transition(owner));
        }
        result.and(transition(&mut self.user_files))
    }
}

impl Drop for ProfileVolumeAppOwner {
    fn drop(&mut self) {
        let old = std::mem::replace(&mut self.state, OwnerState::Ended);
        if let OwnerState::Verified(attachment) = old {
            let _ = attachment.terminate(AttachmentStatus::Ended);
        }
    }
}

/// Opaque consumer capability tying one retained FD and reserved Notes
/// ObjectId to the same verified profile-volume attachment.
///
/// There is intentionally no public constructor or deserializer. The hidden
/// FD duplication method exists only so the Linux Notes codec can install its
/// descriptor-backed C1 storage backend while retaining this lifecycle token.
pub struct ProfileVolumeNotesBundle {
    attachment: Arc<VerifiedProfileVolumeAttachment>,
}

impl ProfileVolumeNotesBundle {
    pub fn kind(&self) -> AppStorageKind {
        self.attachment.kind
    }

    pub fn validate_kind(
        &self,
        expected: AppStorageKind,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        if self.kind() != expected {
            return Err(ProfileVolumeNotesOwnerError::ForeignAttachment);
        }
        Ok(())
    }

    pub fn principal_id(&self) -> &str {
        &self.attachment.principal_id
    }

    pub fn profile_id(&self) -> &str {
        &self.attachment.profile_id
    }

    pub fn notes_object_id(&self) -> ObjectId {
        self.object_id()
    }

    pub fn object_id(&self) -> ObjectId {
        self.attachment
            .object_id
            .expect("object bundle has a reserved ObjectId")
    }

    /// A new reservation permits the codec to create its initial index.
    /// The codec also checks its durable initialization marker.
    #[cfg(target_os = "linux")]
    pub fn allows_initial_index(&self) -> bool {
        self.attachment.allow_initial_index
    }

    /// Compare the bundle's stored scope labels with values obtained from the
    /// canonical C1 identity. This comparison does not authenticate caller
    /// input; the Notes consumer must also validate that identity and its C2
    /// caller before opening or using the bundle.
    pub fn validate_scope(
        &self,
        principal_id: &str,
        profile_id: &str,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        if principal_id != self.attachment.principal_id {
            return Err(ProfileVolumeNotesOwnerError::CallerMismatch);
        }
        if profile_id != self.attachment.profile_id {
            return Err(ProfileVolumeNotesOwnerError::ProfileMismatch);
        }
        self.with_active(|| ())?;
        Ok(())
    }

    /// Check that the profile directory and the retained Data root are on
    /// the same Linux mount. The Volume Manager still supplies the profile
    /// mapping and reservation from its own authority.
    #[cfg(target_os = "linux")]
    pub fn validate_data_mount(
        &self,
        data_root: impl std::os::fd::AsFd,
        selected_mount_id: u64,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.with_active(|| {
            use ProfileVolumeNotesOwnerError::ForeignAttachment;

            linux_data::require_same_mount(&data_root, selected_mount_id)
                .map_err(|_| ForeignAttachment)?;
            linux_data::require_same_mount(&self.attachment.object_directory, selected_mount_id)
                .map_err(|_| ForeignAttachment)?;
            Ok(())
        })?
    }

    /// Run an entire consumer operation while serialized against revoke/end.
    /// This does not replace C1's live Session effect lease.
    pub fn with_active<R>(
        &self,
        operation: impl FnOnce() -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        let lifecycle = self.attachment.lifecycle()?;
        if lifecycle.status != AttachmentStatus::Active {
            return Err(attachment_status_error(lifecycle.status));
        }
        Ok(operation())
    }

    /// Duplicate the owner's retained descriptor for the C1 backend. The
    /// caller must retain this bundle and gate every effect through
    /// `with_active`; the returned descriptor by itself carries no owner state.
    #[doc(hidden)]
    pub fn duplicate_object_directory(&self) -> Result<OwnedFd, ProfileVolumeNotesOwnerError> {
        self.with_duplicated_object_directory(|directory| directory)
    }

    /// Keep the attachment lifecycle lock held while the Linux consumer opens
    /// its C1 root from a duplicate of the retained descriptor.
    #[doc(hidden)]
    pub fn with_duplicated_object_directory<R>(
        &self,
        operation: impl FnOnce(OwnedFd) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        let lifecycle = self.attachment.lifecycle()?;
        if lifecycle.status != AttachmentStatus::Active {
            return Err(attachment_status_error(lifecycle.status));
        }
        let directory = fcntl_dupfd_cloexec(&self.attachment.object_directory, 0)
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
        Ok(operation(directory))
    }

    #[cfg(test)]
    fn same_attachment(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.attachment, &other.attachment)
    }
}

/// A private User Files directory, with no object reservation.
#[cfg(target_os = "linux")]
pub struct ProfileFilesRoot(ProfileVolumeNotesBundle);

#[cfg(target_os = "linux")]
impl ProfileFilesRoot {
    pub fn validate_scope(
        &self,
        principal: &str,
        profile: &str,
        selected_data: impl std::os::fd::AsFd,
        mount_id: u64,
    ) -> Result<(), ProfileVolumeNotesOwnerError> {
        self.0.validate_scope(principal, profile)?;
        self.0.validate_data_mount(selected_data, mount_id)
    }

    /// The trusted consumer invokes this only inside the matching C1 lease.
    /// The higher-ranked callback keeps the view and any opened file borrows
    /// from escaping this owner-checked effect.
    pub fn with_active_root<R>(
        &self,
        effect: impl for<'view> FnOnce(FilesRootView<'view>) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        self.with_files_root(effect)
    }
}

#[cfg(target_os = "linux")]
/// One owner callback for a complete Files service operation. The callback
/// should include the operation authorizer and all provider work. Calls on the
/// supplied view are scoped primitives and must not re-enter this scope.
///
/// The HRTB keeps the root view inside that one callback:
///
/// ```compile_fail
/// use nagi_profile_volume_notes_owner::{FilesRootScope, FilesRootView};
///
/// fn escape(scope: &impl FilesRootScope) -> FilesRootView<'static> {
///     scope.with_files_root(|view| view).unwrap()
/// }
/// ```
///
/// The consumer receives no raw descriptor:
///
/// ```compile_fail
/// use nagi_profile_volume_notes_owner::FilesRootScope;
///
/// fn keep_fd(scope: &impl FilesRootScope) {
///     scope.with_files_root(|view| {
///         let _fd = view.as_fd().try_clone_to_owned().unwrap();
///     }).unwrap();
/// }
/// ```
///
/// Open-file and open-directory views are also callback-scoped:
///
/// ```compile_fail
/// use nagi_profile_volume_notes_owner::{FilesRootDirView, FilesRootScope};
/// use std::path::Path;
///
/// fn escape_dir(scope: &impl FilesRootScope) -> FilesRootDirView<'static> {
///     scope.with_files_root(|view| {
///         view.with_open_dir(Path::new("."), |directory| Ok(directory)).unwrap()
///     }).unwrap()
/// }
/// ```
pub trait FilesRootScope {
    fn with_files_root<R>(
        &self,
        effect: impl for<'view> FnOnce(FilesRootView<'view>) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError>;
}

#[cfg(target_os = "linux")]
impl FilesRootScope for ProfileFilesRoot {
    fn with_files_root<R>(
        &self,
        effect: impl for<'view> FnOnce(FilesRootView<'view>) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        self.0.with_active(|| {
            let directory = &self.0.attachment.object_directory;
            let mount_id = linux_data::mount_id(directory)
                .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
            linux_data::require_same_mount(directory, mount_id)
                .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
            Ok(effect(FilesRootView::new(directory)))
        })?
    }
}

/// A typed, single-object Data provider kept behind its owner bundle.
pub struct ProfileSnapshotStorage {
    bundle: ProfileVolumeNotesBundle,
    provider: snapshot::DirectoryFdSnapshotStorage,
}

impl ProfileSnapshotStorage {
    pub fn open_settings(
        bundle: ProfileVolumeNotesBundle,
    ) -> Result<Self, ProfileVolumeNotesOwnerError> {
        bundle.validate_kind(AppStorageKind::Settings)?;
        Self::open(bundle)
    }

    pub fn open_calendar(
        bundle: ProfileVolumeNotesBundle,
    ) -> Result<Self, ProfileVolumeNotesOwnerError> {
        bundle.validate_kind(AppStorageKind::Calendar)?;
        Self::open(bundle)
    }

    /// Claim the one permitted initialization of the reserved Calendar object.
    /// A claim survives an uncertain first write, so missing data stays fenced.
    #[cfg(target_os = "linux")]
    pub fn claim_calendar_initialization(&self) -> Result<bool, ProfileVolumeNotesOwnerError> {
        use rustix::fs::{openat, Mode, OFlags};
        use rustix::io::Errno;
        use std::fs::File;

        self.bundle.validate_kind(AppStorageKind::Calendar)?;
        if !self.bundle.allows_initial_index() {
            return Ok(false);
        }
        self.bundle.with_duplicated_object_directory(|directory| {
            let directory = File::from(directory);
            let marker = match openat(
                &directory,
                ".calendar-initialization-v1",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(marker) => File::from(marker),
                Err(Errno::EXIST) => return Ok(false),
                Err(_) => return Err(ProfileVolumeNotesOwnerError::Unavailable),
            };
            marker
                .sync_all()
                .and_then(|()| directory.sync_all())
                .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
            Ok(true)
        })?
    }

    fn open(bundle: ProfileVolumeNotesBundle) -> Result<Self, ProfileVolumeNotesOwnerError> {
        let provider = bundle
            .with_duplicated_object_directory(|directory| {
                snapshot::DirectoryFdSnapshotStorage::new(directory.into())
            })?
            .map_err(|_| ProfileVolumeNotesOwnerError::Unavailable)?;
        Ok(Self { bundle, provider })
    }

    /// The composing caller holds its exact live Session throughout this callback.
    pub fn with_active_snapshot<R>(
        &mut self,
        operation: impl FnOnce(
            ObjectId,
            &mut dyn SnapshotStorage<Error = SingleObjectSnapshotStorageError<SnapshotStorageError>>,
        ) -> R,
    ) -> Result<R, ProfileVolumeNotesOwnerError> {
        let object = self.bundle.object_id();
        self.bundle.with_active(|| {
            snapshot::with_single_object_snapshot_storage(&mut self.provider, object, |view| {
                operation(object, view)
            })
        })
    }
}

fn attachment_status_error(status: AttachmentStatus) -> ProfileVolumeNotesOwnerError {
    match status {
        AttachmentStatus::Active => unreachable!("active status passed to terminal mapper"),
        AttachmentStatus::Revoked => ProfileVolumeNotesOwnerError::Revoked,
        AttachmentStatus::Ended => ProfileVolumeNotesOwnerError::Ended,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    fn fixture_owner(profile_id: &str) -> (ProfileVolumeNotesOwner, UnixStream) {
        let (directory, peer) = UnixStream::pair().unwrap();
        let directory: OwnedFd = directory.into();
        let owner = ProfileVolumeNotesOwner::verified_fixture(
            "principal_notes_owner_test",
            profile_id,
            directory,
            ObjectId(0x4e4f_5445),
        );
        (owner, peer)
    }

    fn issue_fixture(
        owner: &ProfileVolumeNotesOwner,
        profile_id: &str,
    ) -> Result<ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError> {
        owner.issue_for_claims("principal_notes_owner_test", profile_id)
    }

    #[test]
    fn pending_owner_fails_closed_without_a_bundle() {
        let owner = ProfileVolumeNotesOwner::pending();
        assert_eq!(owner.status(), ProfileVolumeNotesOwnerStatus::Pending);
        assert!(matches!(
            owner.issue_bundle(),
            Err(ProfileVolumeNotesOwnerError::Unavailable)
        ));
    }

    #[test]
    fn wrong_profile_is_rejected_before_bundle_issuance() {
        let (owner, _peer) = fixture_owner("profile_notes_owner_test");
        assert!(matches!(
            issue_fixture(&owner, "profile_notes_other"),
            Err(ProfileVolumeNotesOwnerError::ProfileMismatch)
        ));
    }

    #[test]
    fn consumer_scope_comparison_rejects_foreign_principal_and_profile() {
        let (owner, _peer) = fixture_owner("profile_notes_owner_test");
        let bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();

        assert!(matches!(
            bundle.validate_scope("principal_other", "profile_notes_owner_test"),
            Err(ProfileVolumeNotesOwnerError::CallerMismatch)
        ));
        assert!(matches!(
            bundle.validate_scope("principal_notes_owner_test", "profile_notes_other"),
            Err(ProfileVolumeNotesOwnerError::ProfileMismatch)
        ));
    }

    #[test]
    fn bundle_rejects_a_different_storage_kind() {
        let (directory, _peer) = UnixStream::pair().unwrap();
        let owner = ProfileVolumeAppOwner::verified_fixture_for(
            AppStorageKind::Calendar,
            "principal_notes_owner_test",
            "profile_notes_owner_test",
            directory.into(),
            ObjectId(0x4e4f_5445),
        );
        let bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        assert_eq!(bundle.kind(), AppStorageKind::Calendar);
        assert_eq!(bundle.object_id(), ObjectId(0x4e4f_5445));
        assert_eq!(bundle.validate_kind(AppStorageKind::Calendar), Ok(()));
        assert_eq!(
            bundle.validate_kind(AppStorageKind::Notes),
            Err(ProfileVolumeNotesOwnerError::ForeignAttachment)
        );
    }

    #[test]
    fn foreign_attachment_cannot_revoke_or_impersonate_another_owner() {
        let (mut first_owner, _first_peer) = fixture_owner("profile_notes_owner_test");
        let (second_owner, _second_peer) = fixture_owner("profile_notes_owner_test");
        let first_bundle = issue_fixture(&first_owner, "profile_notes_owner_test").unwrap();
        let second_bundle = issue_fixture(&second_owner, "profile_notes_owner_test").unwrap();

        // ObjectIds are namespace-local; an identical label cannot join two
        // independently retained roots into one attachment.
        assert_eq!(
            first_bundle.notes_object_id(),
            second_bundle.notes_object_id()
        );
        assert!(!first_bundle.same_attachment(&second_bundle));
        assert!(matches!(
            first_owner.revoke_bundle(&second_bundle),
            Err(ProfileVolumeNotesOwnerError::ForeignAttachment)
        ));
        assert_eq!(
            first_owner.status(),
            ProfileVolumeNotesOwnerStatus::Verified
        );
        assert_eq!(
            second_owner.status(),
            ProfileVolumeNotesOwnerStatus::Verified
        );
    }

    #[test]
    fn same_user_profile_distinct_attachments_have_distinct_roots() {
        let (first_owner, _first_peer) = fixture_owner("profile_notes_owner_test");
        let (second_owner, _second_peer) = fixture_owner("profile_notes_owner_test");
        let first_bundle = issue_fixture(&first_owner, "profile_notes_owner_test").unwrap();
        let second_bundle = issue_fixture(&second_owner, "profile_notes_owner_test").unwrap();
        let first_fd = first_bundle.duplicate_object_directory().unwrap();
        let second_fd = second_bundle.duplicate_object_directory().unwrap();
        let first_stat = rustix::fs::fstat(&first_fd).unwrap();
        let second_stat = rustix::fs::fstat(&second_fd).unwrap();

        assert_eq!(first_bundle.principal_id(), second_bundle.principal_id());
        assert_eq!(first_bundle.profile_id(), second_bundle.profile_id());
        assert!(!first_bundle.same_attachment(&second_bundle));
        assert_ne!(
            (first_stat.st_dev, first_stat.st_ino),
            (second_stat.st_dev, second_stat.st_ino)
        );
    }

    #[test]
    fn reissued_bundle_reuses_reserved_notes_id_and_retained_directory() {
        let (owner, _peer) = fixture_owner("profile_notes_owner_test");
        let first = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        let reopened = issue_fixture(&owner, "profile_notes_owner_test").unwrap();

        assert!(first.same_attachment(&reopened));
        assert_eq!(first.notes_object_id(), reopened.notes_object_id());

        let first_directory = first.duplicate_object_directory().unwrap();
        let reopened_directory = reopened.duplicate_object_directory().unwrap();
        let first_stat = rustix::fs::fstat(&first_directory).unwrap();
        let reopened_stat = rustix::fs::fstat(&reopened_directory).unwrap();
        assert_eq!(
            (first_stat.st_dev, first_stat.st_ino),
            (reopened_stat.st_dev, reopened_stat.st_ino)
        );
    }

    #[test]
    fn dropping_one_open_bundle_keeps_other_open_and_owner_fd_alive() {
        let (owner, mut peer) = fixture_owner("profile_notes_owner_test");
        let first_bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        let second_bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        let duplicate = second_bundle.duplicate_object_directory().unwrap();
        peer.set_nonblocking(true).unwrap();

        assert!(first_bundle.same_attachment(&second_bundle));
        drop(first_bundle);
        assert!(second_bundle.with_active(|| ()).is_ok());
        assert!(second_bundle.duplicate_object_directory().is_ok());
        let mut byte = [0_u8; 1];
        assert!(
            matches!(peer.read(&mut byte), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );

        drop(second_bundle);
        assert!(
            matches!(peer.read(&mut byte), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        drop(duplicate);
        drop(owner);
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn settings_snapshot_reopens_and_retained_provider_is_denied_after_revocation() {
        use std::fs::{self, File};
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "nagi-settings-snapshot-owner-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let object = ObjectId(42);
        let attach = |kind| {
            ProfileVolumeAppOwner::verified_fixture_for(
                kind,
                "principal",
                "profile",
                File::open(&path).unwrap().into(),
                object,
            )
        };
        let notes = attach(AppStorageKind::Notes);
        assert!(matches!(
            ProfileSnapshotStorage::open_settings(notes.issue_bundle().unwrap()),
            Err(ProfileVolumeNotesOwnerError::ForeignAttachment)
        ));
        let mut settings = attach(AppStorageKind::Settings);
        let open = |owner: &ProfileVolumeAppOwner| {
            ProfileSnapshotStorage::open_settings(owner.issue_bundle().unwrap()).unwrap()
        };
        let mut first = open(&settings);
        let saved = first
            .with_active_snapshot(|key, storage| {
                assert_eq!(key, object);
                assert_eq!(storage.read(key), Ok(None));
                assert_eq!(
                    storage.read(ObjectId(77)),
                    Err(SingleObjectSnapshotStorageError::ObjectIdMismatch)
                );
                storage.compare_and_replace(key, None, b"settings payload")
            })
            .unwrap();
        let ConditionalWriteOutcome::Applied { revision } = saved else {
            panic!("initial snapshot was not saved");
        };
        drop(first);

        let mut reopened = open(&settings);
        assert_eq!(
            reopened.with_active_snapshot(|key, storage| storage.read(key)),
            Ok(Ok(Some(Snapshot::new(
                revision,
                b"settings payload".to_vec()
            ))))
        );
        settings.revoke().unwrap();
        assert_eq!(
            reopened.with_active_snapshot(|_, _| -> () { panic!("revoked provider invoked") }),
            Err(ProfileVolumeNotesOwnerError::Revoked)
        );
        drop(reopened);
        drop(settings);
        drop(notes);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn calendar_and_settings_snapshots_stay_separate_across_reopen_and_logout() {
        use std::fs::{self, File};
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "nagi-calendar-snapshot-owner-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let attach = |kind, directory| {
            let root = path.join(directory);
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            ProfileVolumeAppOwner::verified_fixture_for(
                kind,
                "principal",
                "profile",
                File::open(&root).unwrap().into(),
                ObjectId(42),
            )
        };
        let mut owners = AppStorageOwners::pending();
        owners.settings = attach(AppStorageKind::Settings, "settings");
        owners.calendar = attach(AppStorageKind::Calendar, "calendar");
        assert!(matches!(
            ProfileSnapshotStorage::open_calendar(owners.settings().issue_bundle().unwrap()),
            Err(ProfileVolumeNotesOwnerError::ForeignAttachment)
        ));
        let mut settings =
            ProfileSnapshotStorage::open_settings(owners.settings().issue_bundle().unwrap())
                .unwrap();
        let first = |storage: &mut ProfileSnapshotStorage, payload: &[u8]| {
            let result = storage
                .with_active_snapshot(|key, provider| {
                    assert_eq!(provider.read(key), Ok(None));
                    provider.compare_and_replace(key, None, payload)
                })
                .unwrap();
            assert!(matches!(result, ConditionalWriteOutcome::Applied { .. }));
        };
        first(&mut settings, b"settings");
        let open_calendar = |owners: &AppStorageOwners| {
            ProfileSnapshotStorage::open_calendar(owners.calendar().issue_bundle().unwrap())
                .unwrap()
        };
        let mut calendar = open_calendar(&owners);
        first(&mut calendar, b"calendar");
        drop(calendar);
        let mut calendar = open_calendar(&owners);
        for (storage, expected) in [
            (&mut settings, &b"settings"[..]),
            (&mut calendar, &b"calendar"[..]),
        ] {
            let snapshot = storage
                .with_active_snapshot(|key, provider| provider.read(key))
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.bytes, expected);
        }
        owners.end().unwrap();
        for storage in [&mut settings, &mut calendar] {
            assert_eq!(
                storage.with_active_snapshot(|_, _| -> () { panic!("ended provider invoked") }),
                Err(ProfileVolumeNotesOwnerError::Ended),
            );
        }
        drop((settings, calendar, owners));
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_files_root_scopes_normal_io_and_stops_after_end_or_revoke() {
        use rustix::fs::{Mode, OFlags};
        use std::fs::{self, File};
        use std::io::{Read, Write};
        use std::os::unix::fs::{symlink, DirBuilderExt, MetadataExt};
        use std::path::Path;

        let directory =
            std::env::temp_dir().join(format!("nagi-files-owner-{}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let root = directory.join("root");
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::write(directory.join("outside"), b"outside").unwrap();
        symlink("../outside", root.join("escape")).unwrap();

        let mut group = AppStorageOwners::pending();
        group.user_files.state =
            OwnerState::Verified(Arc::new(VerifiedProfileVolumeAttachment::new(
                AppStorageKind::UserFiles,
                "principal",
                "profile",
                File::open(&root).unwrap().into(),
                None,
            )));
        assert!(matches!(
            group.user_files().issue_bundle(),
            Err(ProfileVolumeNotesOwnerError::ForeignAttachment)
        ));
        let bundle = group.user_files().issue_files_root().unwrap();
        bundle
            .with_active_root(|view| {
                for invalid in ["../outside", "/etc/passwd", "escape"] {
                    assert!(view
                        .with_open_file(Path::new(invalid), OFlags::RDONLY, Mode::empty(), |_| Ok(
                            ()
                        ))
                        .is_err());
                }
                view.sync_parent(Path::new("owned")).unwrap();
                assert!(view
                    .create_file_new(Path::new("../outside"), b"must not escape")
                    .is_err());
                view.with_open_file(
                    Path::new("owned"),
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                    Mode::RUSR | Mode::WUSR,
                    |file| {
                        file.write_all(b"files payload")?;
                        file.sync_all()
                    },
                )
                .unwrap();
                view.sync_root().unwrap();
                let before = view.symlink_metadata(Path::new("owned")).unwrap();
                assert_eq!(before.kind, FilesRootEntryKind::RegularFile);
                let host_metadata = fs::symlink_metadata(directory.join("root/owned")).unwrap();
                assert_eq!(before.device, host_metadata.dev());
                assert_eq!(before.inode, host_metadata.ino());
                view.create_file_new(Path::new("index"), b"old index")
                    .unwrap();
                view.write_temp_and_replace(
                    Path::new(".index.tmp"),
                    Path::new("index"),
                    b"new index",
                )
                .unwrap();
                let index_bytes = view
                    .with_open_file(Path::new("index"), OFlags::RDONLY, Mode::empty(), |file| {
                        let mut bytes = Vec::new();
                        file.read_to_end(&mut bytes)?;
                        Ok(bytes)
                    })
                    .unwrap();
                assert_eq!(index_bytes, b"new index");
                assert!(view.symlink_metadata(Path::new(".index.tmp")).is_err());
                assert!(view
                    .create_file_new(Path::new("owned"), b"must not replace")
                    .is_err());
                view.write_existing_file(Path::new("owned"), b"updated payload")
                    .unwrap();
                assert_eq!(
                    view.symlink_metadata(Path::new("owned"))
                        .unwrap()
                        .size_bytes,
                    u64::from(b"updated payload".len() as u32)
                );
                view.create_directory(Path::new("folder")).unwrap();
                view.create_file_new(Path::new("folder/source"), b"copy payload")
                    .unwrap();
                let listed = view.read_directory(Path::new("folder")).unwrap();
                assert_eq!(listed.len(), 1);
                assert_eq!(listed[0].name, "source");
                assert_eq!(listed[0].metadata.kind, FilesRootEntryKind::RegularFile);
                let source_identity = view.symlink_metadata(Path::new("folder/source")).unwrap();
                view.create_file_new(Path::new("folder/existing"), b"retain destination")
                    .unwrap();
                assert!(view
                    .rename_without_replacement(
                        Path::new("folder/source"),
                        Path::new("folder/existing"),
                    )
                    .is_err());
                assert_eq!(
                    view.symlink_metadata(Path::new("folder/source"))
                        .unwrap()
                        .inode,
                    source_identity.inode
                );
                let destination_bytes = view
                    .with_open_file(
                        Path::new("folder/existing"),
                        OFlags::RDONLY,
                        Mode::empty(),
                        |file| {
                            let mut bytes = Vec::new();
                            file.read_to_end(&mut bytes)?;
                            Ok(bytes)
                        },
                    )
                    .unwrap();
                assert_eq!(destination_bytes, b"retain destination");
                let renamed = view
                    .rename_without_replacement(
                        Path::new("folder/source"),
                        Path::new("folder/renamed"),
                    )
                    .unwrap();
                assert_eq!(renamed.inode, source_identity.inode);
                assert!(view
                    .rename_without_replacement(
                        Path::new("folder/renamed"),
                        Path::new("folder/renamed"),
                    )
                    .is_err());
                view.create_file_new(Path::new("unlink-me"), b"short lived")
                    .unwrap();
                view.unlink(Path::new("unlink-me")).unwrap();
                view.create_directory(Path::new("empty-directory")).unwrap();
                view.rmdir(Path::new("empty-directory")).unwrap();
                assert!(view
                    .copy_path(
                        Path::new("folder"),
                        Path::new("folder/copied-inside-source"),
                        || false,
                    )
                    .is_err());
                assert!(view
                    .symlink_metadata(Path::new("folder/copied-inside-source"))
                    .is_err());
                assert!(view
                    .copy_path(Path::new("folder"), Path::new("cancelled-copy"), || true,)
                    .is_err());
                assert!(view.symlink_metadata(Path::new("cancelled-copy")).is_err());
                view.create_file_new(Path::new("large-source"), &vec![0x5a; 130 * 1024])
                    .unwrap();
                view.create_file_new(Path::new("preserve-destination"), b"keep existing")
                    .unwrap();
                assert!(view
                    .copy_path(
                        Path::new("large-source"),
                        Path::new("preserve-destination"),
                        || false,
                    )
                    .is_err());
                let preserved = view
                    .with_open_file(
                        Path::new("preserve-destination"),
                        OFlags::RDONLY,
                        Mode::empty(),
                        |file| {
                            let mut bytes = Vec::new();
                            file.read_to_end(&mut bytes)?;
                            Ok(bytes)
                        },
                    )
                    .unwrap();
                assert_eq!(preserved, b"keep existing");
                let mut cancellation_checks = 0;
                let cancelled =
                    view.copy_path(Path::new("large-source"), Path::new("partial-copy"), || {
                        cancellation_checks += 1;
                        cancellation_checks >= 3
                    });
                assert_eq!(
                    cancelled.unwrap_err().kind(),
                    std::io::ErrorKind::Interrupted
                );
                assert!(view.symlink_metadata(Path::new("partial-copy")).is_err());
                assert!(view
                    .copy_path(Path::new("escape"), Path::new("copied-link"), || false)
                    .is_err());
                assert!(view.symlink_metadata(Path::new("copied-link")).is_err());
                view.copy_path(Path::new("folder"), Path::new("copied-folder"), || false)
                    .unwrap();
                assert_eq!(
                    view.symlink_metadata(Path::new("copied-folder/renamed"))
                        .unwrap()
                        .kind,
                    FilesRootEntryKind::RegularFile
                );
                view.remove_path(Path::new("copied-folder")).unwrap();
                assert!(view.symlink_metadata(Path::new("copied-folder")).is_err());
                symlink("../outside", root.join("folder/nested-link")).unwrap();
                assert_eq!(
                    view.validate_tree_mounts(Path::new("folder"))
                        .unwrap_err()
                        .kind(),
                    std::io::ErrorKind::PermissionDenied
                );
                assert!(view
                    .rename_without_replacement(Path::new("folder"), Path::new("moved-folder"))
                    .is_err());
                assert_eq!(
                    view.symlink_metadata(Path::new("folder")).unwrap().kind,
                    FilesRootEntryKind::Directory
                );
                assert!(view
                    .copy_path(Path::new("folder"), Path::new("copied-folder"), || false)
                    .is_err());
                assert!(view.symlink_metadata(Path::new("copied-folder")).is_err());
                view.unlink(Path::new("folder/nested-link")).unwrap();
                view.validate_tree_mounts(Path::new("folder")).unwrap();
                let root_entries = view.read_directory(Path::new(".")).unwrap();
                assert!(root_entries.iter().any(|entry| entry.name == "escape"));
                assert_eq!(
                    view.symlink_metadata(Path::new("escape")).unwrap().kind,
                    FilesRootEntryKind::Symlink
                );
                view.create_directory(Path::new(".nagi-files")).unwrap();
                symlink("../../outside", root.join(".nagi-files/trash-index-v1.tmp")).unwrap();
                view.write_temp_and_replace(
                    Path::new(".nagi-files/trash-index-v1.tmp"),
                    Path::new(".nagi-files/trash-index-v1"),
                    b"trash-index",
                )
                .unwrap();
                view.write_temp_and_replace(
                    Path::new(".nagi-files/tags-v1.tmp"),
                    Path::new(".nagi-files/tags-v1"),
                    b"tags-index",
                )
                .unwrap();
                view.write_temp_and_replace(
                    Path::new(".nagi-files/trash-index-v1.tmp"),
                    Path::new(".nagi-files/trash-index-v1"),
                    b"trash-index-v2",
                )
                .unwrap();
                view.write_temp_and_replace(
                    Path::new(".nagi-files/tags-v1.tmp"),
                    Path::new(".nagi-files/tags-v1"),
                    b"tags-index-v2",
                )
                .unwrap();
                for (path, expected) in [
                    (
                        Path::new(".nagi-files/trash-index-v1"),
                        &b"trash-index-v2"[..],
                    ),
                    (Path::new(".nagi-files/tags-v1"), &b"tags-index-v2"[..]),
                ] {
                    let contents = view
                        .with_open_file(path, OFlags::RDONLY, Mode::empty(), |file| {
                            let mut contents = Vec::new();
                            file.read_to_end(&mut contents)?;
                            Ok(contents)
                        })
                        .unwrap();
                    assert_eq!(contents, expected);
                }
            })
            .unwrap();

        let readback = group.user_files().issue_files_root().unwrap();
        let bytes = readback
            .with_active_root(|view| {
                view.with_open_file(Path::new("owned"), OFlags::RDONLY, Mode::empty(), |file| {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
            })
            .unwrap()
            .unwrap();
        assert_eq!(bytes, b"updated payload");
        assert_eq!(fs::read(directory.join("outside")).unwrap(), b"outside");
        group.end().unwrap();
        assert_eq!(
            readback.with_active_root(|_| -> () { panic!("ended Files root invoked") }),
            Err(ProfileVolumeNotesOwnerError::Ended),
        );
        assert!(matches!(
            readback.with_active_root(|view| {
                view.with_open_file(
                    Path::new("owned"),
                    OFlags::RDONLY,
                    Mode::empty(),
                    |_| Ok(()),
                )
            }),
            Err(ProfileVolumeNotesOwnerError::Ended),
        ));
        assert!(matches!(
            readback.with_active_root(|view| view.read_directory(Path::new("."))),
            Err(ProfileVolumeNotesOwnerError::Ended),
        ));
        drop((group, bundle, readback));

        let mut revoked_group = AppStorageOwners::pending();
        revoked_group.user_files.state =
            OwnerState::Verified(Arc::new(VerifiedProfileVolumeAttachment::new(
                AppStorageKind::UserFiles,
                "principal",
                "profile",
                File::open(&root).unwrap().into(),
                None,
            )));
        let revoked_root = revoked_group.user_files().issue_files_root().unwrap();
        let revoked_readback = revoked_root
            .with_active_root(|view| {
                view.with_open_file(Path::new("owned"), OFlags::RDONLY, Mode::empty(), |file| {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
            })
            .unwrap()
            .unwrap();
        assert_eq!(revoked_readback, b"updated payload");
        revoked_group.revoke().unwrap();
        assert!(matches!(
            revoked_root.with_active_root(|view| {
                view.with_open_file(
                    Path::new("owned"),
                    OFlags::RDONLY,
                    Mode::empty(),
                    |_| Ok(()),
                )
            }),
            Err(ProfileVolumeNotesOwnerError::Revoked),
        ));
        assert!(matches!(
            revoked_root.with_active_root(|view| view.create_directory(Path::new("after-revoke"))),
            Err(ProfileVolumeNotesOwnerError::Revoked),
        ));
        drop((revoked_group, revoked_root));
        fs::remove_dir_all(directory).unwrap();
    }

    fn app_storage_fixture() -> (AppStorageOwners, [ProfileVolumeNotesBundle; 3]) {
        let mut owners = AppStorageOwners::pending();
        let bundles = owners.owners_mut().map(|owner| {
            let (directory, _peer) = UnixStream::pair().unwrap();
            *owner = ProfileVolumeAppOwner::verified_fixture_for(
                owner.kind(),
                "principal",
                "profile",
                directory.into(),
                ObjectId(42),
            );
            let bundle = owner.issue_bundle().unwrap();
            assert_eq!(bundle.with_active(|| 42), Ok(42));
            bundle
        });
        (owners, bundles)
    }

    #[test]
    fn failed_logout_still_ends_other_storage_bundles() {
        let (mut owners, [notes, settings, calendar]) = app_storage_fixture();
        let poison = std::panic::catch_unwind(|| {
            notes.with_active(|| panic!("fixture storage effect panicked"))
        });
        assert!(poison.is_err());

        assert_eq!(owners.end(), Err(ProfileVolumeNotesOwnerError::Unavailable));
        assert_eq!(
            notes.with_active(|| ()),
            Err(ProfileVolumeNotesOwnerError::Unavailable)
        );
        for bundle in [settings, calendar] {
            assert_eq!(
                bundle.with_active(|| ()),
                Err(ProfileVolumeNotesOwnerError::Ended)
            );
        }
    }

    #[test]
    fn logout_and_revocation_invalidate_every_app_storage_bundle() {
        for terminal in [
            ProfileVolumeNotesOwnerError::Ended,
            ProfileVolumeNotesOwnerError::Revoked,
        ] {
            let (mut owners, bundles) = app_storage_fixture();
            assert_eq!(
                bundles.each_ref().map(ProfileVolumeNotesBundle::kind),
                [
                    AppStorageKind::Notes,
                    AppStorageKind::Settings,
                    AppStorageKind::Calendar
                ]
            );
            if terminal == ProfileVolumeNotesOwnerError::Ended {
                owners.end().unwrap();
            } else {
                owners.revoke().unwrap();
                owners.end().unwrap();
            }
            for bundle in bundles {
                assert_eq!(
                    bundle.with_active(|| -> () { panic!("terminal storage effect") }),
                    Err(terminal)
                );
            }
        }
    }

    #[test]
    fn revoked_owner_rejects_bundle_and_closes_descriptor_after_last_reference() {
        let (mut owner, mut peer) = fixture_owner("profile_notes_owner_test");
        let bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        owner.revoke().unwrap();

        assert_eq!(owner.status(), ProfileVolumeNotesOwnerStatus::Revoked);
        assert!(matches!(
            bundle.duplicate_object_directory(),
            Err(ProfileVolumeNotesOwnerError::Revoked)
        ));
        assert!(matches!(
            bundle.with_active(|| ()),
            Err(ProfileVolumeNotesOwnerError::Revoked)
        ));
        drop(owner);
        drop(bundle);
        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn ended_owner_rejects_bundle_and_closes_descriptor_after_last_reference() {
        let (mut owner, mut peer) = fixture_owner("profile_notes_owner_test");
        let bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        owner.end().unwrap();

        assert_eq!(owner.status(), ProfileVolumeNotesOwnerStatus::Ended);
        assert!(matches!(
            bundle.duplicate_object_directory(),
            Err(ProfileVolumeNotesOwnerError::Ended)
        ));
        assert!(matches!(
            owner.issue_bundle(),
            Err(ProfileVolumeNotesOwnerError::Ended)
        ));
        drop(owner);
        drop(bundle);
        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn bundle_drop_releases_the_retained_descriptor() {
        let (owner, mut peer) = fixture_owner("profile_notes_owner_test");
        let bundle = issue_fixture(&owner, "profile_notes_owner_test").unwrap();
        drop(owner);
        drop(bundle);
        let mut byte = [0_u8; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }
}
