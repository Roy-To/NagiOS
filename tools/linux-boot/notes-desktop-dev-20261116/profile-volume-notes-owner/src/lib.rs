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
    principal_id: String,
    profile_id: String,
    object_directory: OwnedFd,
    notes_object_id: ObjectId,
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
        principal_id: impl Into<String>,
        profile_id: impl Into<String>,
        object_directory: OwnedFd,
        notes_object_id: ObjectId,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            profile_id: profile_id.into(),
            object_directory,
            notes_object_id,
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
pub struct ProfileVolumeNotesOwner {
    state: OwnerState,
}

impl ProfileVolumeNotesOwner {
    /// Start fail closed until the selected-profile-volume source is wired.
    pub const fn pending() -> Self {
        Self {
            state: OwnerState::Pending,
        }
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
        let (directory, object, allow_initial_index) =
            linux_data::reserve(data_root, mount_id, user, profile, principal)
                .map_err(|_| Unavailable)?;
        let mut attachment =
            VerifiedProfileVolumeAttachment::new(principal, profile, directory, object);
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
        Self {
            state: OwnerState::Verified(Arc::new(VerifiedProfileVolumeAttachment::new(
                principal_id,
                profile_id,
                object_directory,
                notes_object_id,
            ))),
        }
    }
}

impl Drop for ProfileVolumeNotesOwner {
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
    pub fn principal_id(&self) -> &str {
        &self.attachment.principal_id
    }

    pub fn profile_id(&self) -> &str {
        &self.attachment.profile_id
    }

    pub fn notes_object_id(&self) -> ObjectId {
        self.attachment.notes_object_id
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
