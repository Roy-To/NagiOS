//! Profile-root Notes storage wired through C1's fixed, single-lease root API.
//!
//! The Notes store executes a complete NoteStore operation through one
//! `UserStorageRootAdapter::with_live_root_effect` callback. Its object I/O
//! delegates to the borrowed `ProfileRootEffect` and never reacquires C1.

#[cfg(target_os = "linux")]
use super::linux_profile_backend::LinuxProfileStorageRoot;
use super::{
    same_candidate, summary, validate_candidate, NoteStore, StoreError, MAX_STORED_NOTE_BYTES,
};
use crate::domain::{NoteDocument, NoteSummary};
use crate::identity::HostObjectIdSource;
use crate::markdown::{decode_stored, encode_stored_checked};
use nagi_identity_session::{
    AuthenticatedInteractiveCaller, CallerContextId, IdentityAccessContext, ProfileId,
    ProfileRootAuthorizer, ProfileRootEffect, ProfileStorageBackend, ResolvedIdentity,
    StorageRootEventSink, StorageRootFailure, StorageRootHandle, TrustedCallerContext,
    UserStorageRootAdapter,
};
use nagi_model::ObjectId;
#[cfg(target_os = "linux")]
use nagi_profile_volume_notes_owner::{
    AppStorageKind, ProfileVolumeNotesBundle, ProfileVolumeNotesOwnerError,
};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

const INDEX_MAGIC: &[u8] = b"NAGI-PROFILE-NOTES\0\x01";
const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_INDEX_RECORDS: usize = 16_384;

/// Consistency label for one resolved interactive identity and profile.
/// This value does not authenticate a caller or grant storage authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileRootNotesBinding {
    principal_id: nagi_identity_session::PrincipalId,
    caller_context_id: CallerContextId,
    session_id: nagi_identity_session::SessionId,
    profile_id: ProfileId,
    notes_object_id: ObjectId,
}

impl ProfileRootNotesBinding {
    fn from_claims(
        identity_principal: &nagi_identity_session::PrincipalId,
        identity_context: &CallerContextId,
        session_id: &nagi_identity_session::SessionId,
        identity_profile: &ProfileId,
        caller: &impl TrustedCallerContext,
        requested_profile: &ProfileId,
        notes_object_id: ObjectId,
    ) -> Result<Self, ProfileRootNotesBindingError> {
        let caller_principal = caller.principal_id().clone();
        let caller_context = caller.context_id().clone();
        if identity_principal != &caller_principal || identity_context != &caller_context {
            return Err(ProfileRootNotesBindingError::CallerMismatch);
        }
        if identity_profile != requested_profile {
            return Err(ProfileRootNotesBindingError::ProfileMismatch);
        }
        Ok(Self {
            principal_id: identity_principal.clone(),
            caller_context_id: identity_context.clone(),
            session_id: session_id.clone(),
            profile_id: identity_profile.clone(),
            notes_object_id,
        })
    }

    /// Build a consistency binding from C1's current identity and the same
    /// C2 trusted caller. Production callers must pass C2's sealed caller; the
    /// trait itself and this comparison do not create that seal.
    pub fn from_identity(
        identity: &ResolvedIdentity,
        caller: &impl TrustedCallerContext,
        requested_profile: &ProfileId,
        notes_object_id: ObjectId,
    ) -> Result<Self, ProfileRootNotesBindingError> {
        if !identity.is_live() {
            return Err(ProfileRootNotesBindingError::StaleSession);
        }
        let binding = Self::from_claims(
            identity.principal_id(),
            identity.caller_context_id(),
            identity.session_id(),
            identity.profile_id(),
            caller,
            requested_profile,
            notes_object_id,
        )?;
        // The caller implementation is owned by C2 and is expected to be an
        // immutable sealed value. Recheck the service-issued identity after
        // reading its external context so an ended lease is rejected here.
        if !identity.is_live() {
            return Err(ProfileRootNotesBindingError::StaleSession);
        }
        Ok(binding)
    }

    pub fn principal_id(&self) -> &nagi_identity_session::PrincipalId {
        &self.principal_id
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.caller_context_id
    }

    pub fn session_id(&self) -> &nagi_identity_session::SessionId {
        &self.session_id
    }

    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    /// Object selected by the profile-volume owner for this profile's Notes
    /// index. It must be reserved outside user request data.
    pub const fn notes_object_id(&self) -> ObjectId {
        self.notes_object_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileRootNotesBindingError {
    StaleSession,
    CallerMismatch,
    ProfileMismatch,
    ResourceMismatch,
}

impl fmt::Display for ProfileRootNotesBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StaleSession => "interactive Notes session is stale",
            Self::CallerMismatch => "interactive Notes caller does not match the C1 identity",
            Self::ProfileMismatch => "interactive Notes profile does not match the C1 identity",
            Self::ResourceMismatch => {
                "Notes root resources belong to a different caller or profile"
            }
        })
    }
}

impl std::error::Error for ProfileRootNotesBindingError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileRootNotesInputError {
    Binding(ProfileRootNotesBindingError),
    Root(StorageRootFailure),
    #[cfg(target_os = "linux")]
    Bundle(ProfileVolumeNotesOwnerError),
}

impl fmt::Display for ProfileRootNotesInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding(error) => fmt::Display::fmt(error, f),
            Self::Root(_) => f.write_str("C1 profile root could not be opened"),
            #[cfg(target_os = "linux")]
            Self::Bundle(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ProfileRootNotesInputError {}

/// Object-level operations exposed only while a single C1 root-effect lease
/// is active. The adapter must not re-enter `UserStorageRootAdapter` here.
pub trait ProfileRootNotesObjectIo {
    fn read_object(&mut self, object_id: ObjectId) -> Result<Vec<u8>, StoreError>;
    fn write_object(&mut self, object_id: ObjectId, bytes: &[u8]) -> Result<(), StoreError>;

    /// Cross the backend's durability boundary for a matching revision.
    /// The default applies to backends whose readable writes are already durable.
    fn confirm_existing_object(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Executes one complete Notes storage effect inside one C1 profile-root
/// lease. Implementations must call the closure exactly once while holding
/// the lease and must expose root I/O that does not acquire another lease.
/// Production backends must keep every synchronous I/O within an owner-defined
/// deadline, must not spawn or join work or re-enter C1 from the callback, and
/// must distinguish committed writes from writes still awaiting durability.
pub trait ProfileRootNotesEffectProvider {
    fn with_live_root_effect<R>(
        &mut self,
        effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
    ) -> Result<R, StoreError>;
}

/// One typed source for the binding and its root-effect provider. This keeps
/// store and gate selection out of app/request data.
pub trait ProfileRootNotesContext: ProfileRootNotesEffectProvider + Send {
    fn binding(&self) -> &ProfileRootNotesBinding;
}

/// A-owned holder for the exact C1 identity, C2 trusted caller, profile,
/// volume-selected Notes object, and opaque root handle. The single-effect
/// path consumes the C1 lane's `local-integration` API; builds use the current
/// checked-out C1 source at that path. Individual object methods below each
/// obtain their own C1 lease and must not be used inside an effect. A does not
/// copy the C1 implementation. Production backends must keep each synchronous
/// object operation within their deadline and must not wait for a worker or
/// re-enter C1 from this callback. The canonical Linux backend confirms an
/// existing revision only after syncing the object directory.
pub struct C1ProfileRootNotesInput<B, C, A, E = nagi_identity_session::NoopStorageRootEventSink>
where
    B: ProfileStorageBackend,
    C: TrustedCallerContext,
    A: ProfileRootAuthorizer,
    E: StorageRootEventSink,
{
    storage: UserStorageRootAdapter<B, E>,
    root: StorageRootHandle,
    identity: ResolvedIdentity,
    caller: C,
    profile_id: ProfileId,
    authorizer: A,
    binding: ProfileRootNotesBinding,
    #[cfg(target_os = "linux")]
    linux_attachment: Option<LinuxNotesAttachment>,
}

#[cfg(target_os = "linux")]
struct LinuxNotesAttachment {
    bundle: ProfileVolumeNotesBundle,
    root: std::sync::Arc<LinuxProfileStorageRoot>,
}

impl<B, C, A> C1ProfileRootNotesInput<B, C, A>
where
    B: ProfileStorageBackend,
    C: AuthenticatedInteractiveCaller,
    A: ProfileRootAuthorizer,
{
    pub fn open(
        mut storage: UserStorageRootAdapter<B>,
        identity: ResolvedIdentity,
        caller: C,
        requested_profile: ProfileId,
        notes_object_id: ObjectId,
        mut authorizer: A,
    ) -> Result<Self, ProfileRootNotesInputError> {
        let binding = ProfileRootNotesBinding::from_identity(
            &identity,
            &caller,
            &requested_profile,
            notes_object_id,
        )
        .map_err(ProfileRootNotesInputError::Binding)?;
        let access = IdentityAccessContext::new(&identity, &caller);
        let root = storage
            .resolve_root(&access, &requested_profile, &mut authorizer)
            .map_err(ProfileRootNotesInputError::Root)?;
        Ok(Self {
            storage,
            root,
            identity,
            caller,
            profile_id: requested_profile,
            authorizer,
            binding,
            #[cfg(target_os = "linux")]
            linux_attachment: None,
        })
    }

    pub fn binding(&self) -> &ProfileRootNotesBinding {
        &self.binding
    }

    /// One isolated root operation through the existing adapter. Do not call
    /// this from a NoteStore effect or from a C1 root-effect callback: it
    /// acquires a separate lease and would re-enter the session gate.
    pub fn read_object_one_lease(&mut self, object_id: ObjectId) -> Result<Vec<u8>, StoreError> {
        let mut operation = || {
            let access = IdentityAccessContext::new(&self.identity, &self.caller);
            self.storage
                .read_object(
                    &access,
                    &self.root,
                    &self.profile_id,
                    object_id,
                    &mut self.authorizer,
                )
                .map_err(map_root_failure)
        };
        #[cfg(target_os = "linux")]
        if let Some(attachment) = self.linux_attachment.as_ref() {
            return match attachment.bundle.with_active(operation) {
                Ok(result) => result,
                Err(error) => Err(map_bundle_store_failure(error)),
            };
        }
        operation()
    }

    /// One isolated root operation through the existing adapter. Do not call
    /// this from a NoteStore effect or from a C1 root-effect callback.
    pub fn write_object_one_lease(
        &mut self,
        object_id: ObjectId,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let mut operation = || {
            let access = IdentityAccessContext::new(&self.identity, &self.caller);
            self.storage
                .write_object(
                    &access,
                    &self.root,
                    &self.profile_id,
                    object_id,
                    bytes,
                    &mut self.authorizer,
                )
                .map_err(map_root_failure)
        };
        #[cfg(target_os = "linux")]
        if let Some(attachment) = self.linux_attachment.as_ref() {
            return match attachment.bundle.with_active(operation) {
                Ok(result) => result,
                Err(error) => Err(map_bundle_store_failure(error)),
            };
        }
        operation()
    }
}

#[cfg(target_os = "linux")]
impl<C, A> C1ProfileRootNotesInput<super::linux_profile_backend::LinuxProfileStorageBackend, C, A>
where
    C: AuthenticatedInteractiveCaller,
    A: ProfileRootAuthorizer,
{
    /// Open the A-owned Linux object backend from one opaque profile-volume
    /// bundle and the same live C1 identity/C2 caller used for every Notes
    /// effect. The bundle binds its retained directory descriptor and
    /// reserved Notes ObjectId to the same verified attachment. C1 remains
    /// the issuer of the scoped root handle and Session lease; C2 remains the
    /// sole source of the sealed interactive caller.
    pub fn open_linux(
        bundle: ProfileVolumeNotesBundle,
        identity: ResolvedIdentity,
        caller: C,
        authorizer: A,
        io_budget: std::time::Duration,
    ) -> Result<Self, ProfileRootNotesInputError> {
        bundle
            .validate_kind(AppStorageKind::Notes)
            .map_err(ProfileRootNotesInputError::Bundle)?;
        bundle
            .validate_scope(
                identity.principal_id().as_str(),
                identity.profile_id().as_str(),
            )
            .map_err(ProfileRootNotesInputError::Bundle)?;
        let requested_profile = ProfileId::new(bundle.profile_id().to_owned()).map_err(|_| {
            ProfileRootNotesInputError::Bundle(ProfileVolumeNotesOwnerError::ProfileMismatch)
        })?;
        let notes_object_id = bundle.notes_object_id();
        ProfileRootNotesBinding::from_identity(
            &identity,
            &caller,
            &requested_profile,
            notes_object_id,
        )
        .map_err(ProfileRootNotesInputError::Binding)?;

        let (mut input, linux_root) = bundle
            .with_borrowed_object_directory(|object_directory| {
                use super::linux_profile_backend::{
                    initialize_notes_index, LinuxProfileStorageBackend,
                };

                let startup_directory = rustix::io::fcntl_dupfd_cloexec(object_directory, 0)
                    .map_err(|_| {
                        ProfileRootNotesInputError::Root(StorageRootFailure::Unavailable)
                    })?;
                let backend_directory = rustix::io::fcntl_dupfd_cloexec(object_directory, 0)
                    .map_err(|_| {
                        ProfileRootNotesInputError::Root(StorageRootFailure::Unavailable)
                    })?;
                let backend = LinuxProfileStorageBackend::from_authenticated_identity_fd(
                    backend_directory,
                    &identity,
                    &caller,
                    &requested_profile,
                    io_budget,
                )
                .map_err(ProfileRootNotesInputError::Root)?;
                let linux_root = backend.root_for_durability();
                let mut input = Self::open(
                    UserStorageRootAdapter::new(backend),
                    identity,
                    caller,
                    requested_profile,
                    notes_object_id,
                    authorizer,
                )?;
                // The outer bundle callback already holds the attachment
                // guard. Install the retained bundle after this root effect.
                let empty = empty_profile_root_notes_object();
                input
                    .with_live_root_effect(|io| {
                        initialize_notes_index(
                            &startup_directory,
                            notes_object_id,
                            bundle.allows_initial_index(),
                            &empty,
                            io_budget,
                            || {
                                ProfileNotesIndex::read(io, notes_object_id)
                                    .map(|_| ())
                                    .map_err(|_| StorageRootFailure::Unavailable)
                            },
                        )
                        .map_err(map_root_failure)
                    })
                    .map_err(|_| {
                        ProfileRootNotesInputError::Root(StorageRootFailure::Unavailable)
                    })?;
                Ok((input, linux_root))
            })
            .map_err(ProfileRootNotesInputError::Bundle)??;
        input.linux_attachment = Some(LinuxNotesAttachment {
            bundle,
            root: linux_root,
        });
        Ok(input)
    }
}

impl<B, C, A, E> ProfileRootNotesEffectProvider for C1ProfileRootNotesInput<B, C, A, E>
where
    B: ProfileStorageBackend,
    C: AuthenticatedInteractiveCaller,
    A: ProfileRootAuthorizer,
    E: StorageRootEventSink,
{
    fn with_live_root_effect<R>(
        &mut self,
        effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let operation = || {
            let access = IdentityAccessContext::new(&self.identity, &self.caller);
            self.storage
                .with_live_root_effect(
                    &access,
                    &self.root,
                    &self.profile_id,
                    &mut self.authorizer,
                    |root_effect| {
                        let mut io = C1ProfileRootNotesObjectIo {
                            root_effect,
                            #[cfg(target_os = "linux")]
                            linux_root: self
                                .linux_attachment
                                .as_ref()
                                .map(|attachment| &*attachment.root),
                        };
                        Ok(effect(&mut io))
                    },
                )
                .map_err(map_root_failure)?
        };
        #[cfg(target_os = "linux")]
        if let Some(attachment) = self.linux_attachment.as_ref() {
            return match attachment.bundle.with_active(operation) {
                Ok(result) => result,
                Err(error) => Err(map_bundle_store_failure(error)),
            };
        }
        operation()
    }
}

struct C1ProfileRootNotesObjectIo<'effect, 'backend, B: ProfileStorageBackend> {
    root_effect: &'effect mut ProfileRootEffect<'backend, B>,
    #[cfg(target_os = "linux")]
    linux_root: Option<&'effect LinuxProfileStorageRoot>,
}

impl<B: ProfileStorageBackend> ProfileRootNotesObjectIo for C1ProfileRootNotesObjectIo<'_, '_, B> {
    fn read_object(&mut self, object_id: ObjectId) -> Result<Vec<u8>, StoreError> {
        self.root_effect
            .read_object(ObjectId(object_id.0))
            .map_err(map_root_failure)
    }

    fn write_object(&mut self, object_id: ObjectId, bytes: &[u8]) -> Result<(), StoreError> {
        let c1_object_id = ObjectId(object_id.0);
        #[cfg(target_os = "linux")]
        let requires_directory_sync = self.linux_root.is_some();
        #[cfg(not(target_os = "linux"))]
        let requires_directory_sync = false;
        let result = self.root_effect.write_object(c1_object_id, bytes);
        finish_root_write(result, requires_directory_sync, bytes, || {
            self.root_effect.read_object(c1_object_id)
        })
    }

    #[cfg(target_os = "linux")]
    fn confirm_existing_object(&mut self) -> Result<(), StoreError> {
        if let Some(root) = self.linux_root {
            root.confirm_directory_sync()
                .map_err(map_durable_write_failure)?;
        }
        Ok(())
    }
}

pub(super) fn finish_root_write(
    result: Result<(), StorageRootFailure>,
    requires_directory_sync: bool,
    expected: &[u8],
    readback: impl FnOnce() -> Result<Vec<u8>, StorageRootFailure>,
) -> Result<(), StoreError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if requires_directory_sync => Err(map_durable_write_failure(error)),
        Err(_) if readback().is_ok_and(|observed| observed == expected) => Ok(()),
        Err(error) => Err(map_root_failure(error)),
    }
}

pub(super) fn map_durable_write_failure(error: StorageRootFailure) -> StoreError {
    match error {
        StorageRootFailure::Unavailable => StoreError::ProfileRootWriteUnconfirmed,
        error => map_root_failure(error),
    }
}

impl<B, C, A, E> ProfileRootNotesContext for C1ProfileRootNotesInput<B, C, A, E>
where
    B: ProfileStorageBackend + Send,
    B::Root: Send + Sync,
    C: AuthenticatedInteractiveCaller + Send,
    A: ProfileRootAuthorizer + Send,
    E: StorageRootEventSink + Send,
{
    fn binding(&self) -> &ProfileRootNotesBinding {
        &self.binding
    }
}

fn map_root_failure(error: StorageRootFailure) -> StoreError {
    match error {
        StorageRootFailure::CallerMismatch
        | StorageRootFailure::CrossProfile
        | StorageRootFailure::InvalidHandle
        | StorageRootFailure::Denied => {
            StoreError::SandboxViolation("C1 profile-root binding rejected".to_owned())
        }
        StorageRootFailure::StaleSession => StoreError::SessionGateRejected,
        StorageRootFailure::CorruptState
        | StorageRootFailure::Unavailable
        | StorageRootFailure::HandleSpaceExhausted => StoreError::Unavailable,
    }
}

#[cfg(target_os = "linux")]
fn map_bundle_store_failure(error: ProfileVolumeNotesOwnerError) -> StoreError {
    match error {
        ProfileVolumeNotesOwnerError::Revoked | ProfileVolumeNotesOwnerError::Ended => {
            StoreError::SessionGateRejected
        }
        ProfileVolumeNotesOwnerError::CallerMismatch
        | ProfileVolumeNotesOwnerError::ProfileMismatch
        | ProfileVolumeNotesOwnerError::ForeignAttachment => {
            StoreError::SandboxViolation("profile-volume Notes bundle rejected".to_owned())
        }
        ProfileVolumeNotesOwnerError::Unavailable => StoreError::Unavailable,
    }
}

/// Canonical initial contents for the object ID reserved by the profile-volume
/// owner. An absent object is unavailable and must not be overwritten as if it
/// were a new profile.
pub fn empty_profile_root_notes_object() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(INDEX_MAGIC.len() + 4);
    bytes.extend_from_slice(INDEX_MAGIC);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes
}

struct ProfileNotesIndex {
    histories: BTreeMap<u64, Vec<NoteDocument>>,
}

impl ProfileNotesIndex {
    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() > MAX_INDEX_BYTES || !bytes.starts_with(INDEX_MAGIC) {
            return Err(StoreError::Corrupt(
                "invalid profile Notes object header".to_owned(),
            ));
        }
        let mut cursor = INDEX_MAGIC.len();
        let record_count = read_u32(bytes, &mut cursor)? as usize;
        if record_count > MAX_INDEX_RECORDS {
            return Err(StoreError::Corrupt(
                "profile Notes object has too many revisions".to_owned(),
            ));
        }

        let mut histories = BTreeMap::<u64, Vec<NoteDocument>>::new();
        let mut previous = None;
        for _ in 0..record_count {
            let id = read_u64(bytes, &mut cursor)?;
            let revision = read_u64(bytes, &mut cursor)?;
            let payload_len = read_u32(bytes, &mut cursor)? as usize;
            if revision == 0 || payload_len > MAX_STORED_NOTE_BYTES {
                return Err(StoreError::Corrupt(
                    "invalid profile Notes revision record".to_owned(),
                ));
            }
            let key = (id, revision);
            if previous.is_some_and(|prior| prior >= key) {
                return Err(StoreError::Corrupt(
                    "profile Notes revisions are not canonical".to_owned(),
                ));
            }
            previous = Some(key);
            let payload = take(bytes, &mut cursor, payload_len)?;
            let stored = std::str::from_utf8(payload)
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            let note = decode_stored(stored, &HostObjectIdSource::new())
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            if note.id.0 != id || note.revision != revision {
                return Err(StoreError::Corrupt(
                    "profile Notes object key disagrees with document".to_owned(),
                ));
            }
            validate_candidate(&note, revision)?;
            let canonical = encode_stored_checked(&note).map_err(StoreError::InvalidDocument)?;
            if canonical.as_bytes() != payload {
                return Err(StoreError::Corrupt(
                    "profile Notes document is not canonical".to_owned(),
                ));
            }
            let history = histories.entry(id).or_default();
            let expected_revision = history
                .last()
                .and_then(|current| current.revision.checked_add(1))
                .unwrap_or(1);
            if revision != expected_revision {
                return Err(StoreError::Corrupt(
                    "profile Notes revision history has a gap".to_owned(),
                ));
            }
            history.push(note);
        }
        if cursor != bytes.len() {
            return Err(StoreError::Corrupt(
                "profile Notes object has trailing bytes".to_owned(),
            ));
        }
        Ok(Self { histories })
    }

    fn encode(&self) -> Result<Vec<u8>, StoreError> {
        let record_count = self
            .histories
            .values()
            .try_fold(0_usize, |count, history| count.checked_add(history.len()))
            .ok_or_else(|| {
                StoreError::Corrupt("profile Notes revision count overflow".to_owned())
            })?;
        if record_count > MAX_INDEX_RECORDS {
            return Err(StoreError::Unavailable);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve(INDEX_MAGIC.len() + 4)
            .map_err(|_| StoreError::Unavailable)?;
        bytes.extend_from_slice(INDEX_MAGIC);
        bytes.extend_from_slice(&(record_count as u32).to_be_bytes());
        for (id, history) in &self.histories {
            let mut expected_revision = 1_u64;
            for note in history {
                if note.id.0 != *id || note.revision != expected_revision {
                    return Err(StoreError::Corrupt(
                        "profile Notes history is not contiguous".to_owned(),
                    ));
                }
                let payload = encode_stored_checked(note).map_err(StoreError::InvalidDocument)?;
                if payload.len() > MAX_STORED_NOTE_BYTES {
                    return Err(StoreError::InvalidDocument("note exceeds 16 MiB"));
                }
                let payload_len = u32::try_from(payload.len())
                    .map_err(|_| StoreError::InvalidDocument("note exceeds 16 MiB"))?;
                let additional = 8_usize
                    .checked_add(8)
                    .and_then(|size| size.checked_add(4))
                    .and_then(|size| size.checked_add(payload.len()))
                    .ok_or(StoreError::Unavailable)?;
                let total = bytes
                    .len()
                    .checked_add(additional)
                    .ok_or(StoreError::Unavailable)?;
                if total > MAX_INDEX_BYTES {
                    return Err(StoreError::Unavailable);
                }
                bytes
                    .try_reserve(additional)
                    .map_err(|_| StoreError::Unavailable)?;
                bytes.extend_from_slice(&id.to_be_bytes());
                bytes.extend_from_slice(&note.revision.to_be_bytes());
                bytes.extend_from_slice(&payload_len.to_be_bytes());
                bytes.extend_from_slice(payload.as_bytes());
                expected_revision = expected_revision
                    .checked_add(1)
                    .ok_or(StoreError::RevisionOverflow)?;
            }
        }
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(StoreError::Unavailable);
        }
        Ok(bytes)
    }

    fn read(
        io: &mut dyn ProfileRootNotesObjectIo,
        object_id: ObjectId,
    ) -> Result<Self, StoreError> {
        let bytes = io.read_object(object_id)?;
        Self::decode(&bytes)
    }
}

/// NoteStore implementation whose operation is wholly enclosed by its typed
/// root-effect input. All reads/writes in one commit occur inside the same C1
/// lease; it must not be wrapped again with `SessionGatedNoteStore`.
pub struct ProfileRootNotesStore<I: ProfileRootNotesContext> {
    input: Mutex<I>,
    binding: ProfileRootNotesBinding,
}

impl<I: ProfileRootNotesContext> ProfileRootNotesStore<I> {
    pub fn new(input: I) -> Self {
        let binding = input.binding().clone();
        Self {
            input: Mutex::new(input),
            binding,
        }
    }

    pub fn binding(&self) -> &ProfileRootNotesBinding {
        &self.binding
    }

    pub fn ensure_binding(
        &self,
        expected: &ProfileRootNotesBinding,
    ) -> Result<(), ProfileRootNotesBindingError> {
        if &self.binding == expected {
            Ok(())
        } else {
            Err(ProfileRootNotesBindingError::ResourceMismatch)
        }
    }

    fn with_effect<R>(
        &self,
        effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        let mut input = self.input.lock().map_err(|_| StoreError::Unavailable)?;
        input.with_live_root_effect(effect)
    }
}

impl<I: ProfileRootNotesContext> NoteStore for ProfileRootNotesStore<I> {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        self.with_effect(|io| {
            let index = ProfileNotesIndex::read(io, self.binding.notes_object_id)?;
            let mut notes = index
                .histories
                .values()
                .filter_map(|history| history.last())
                .filter(|note| include_deleted || !note.deleted)
                .map(summary)
                .collect::<Vec<_>>();
            notes.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.0.cmp(&right.id.0))
            });
            Ok(notes)
        })
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.with_effect(|io| {
            Ok(ProfileNotesIndex::read(io, self.binding.notes_object_id)?
                .histories
                .remove(&id.0)
                .and_then(|history| history.last().cloned()))
        })
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.with_effect(|io| {
            Ok(ProfileNotesIndex::read(io, self.binding.notes_object_id)?
                .histories
                .remove(&id.0)
                .and_then(|history| history.into_iter().find(|note| note.revision == revision)))
        })
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        self.with_effect(|io| {
            validate_candidate(note, expected_revision)?;
            let mut index = ProfileNotesIndex::read(io, self.binding.notes_object_id)?;
            let history = index.histories.entry(note.id.0).or_default();
            let actual = history.last().map(|current| current.revision).unwrap_or(0);
            if actual != expected_revision {
                if actual == expected_revision.saturating_add(1)
                    && history
                        .last()
                        .is_some_and(|current| same_candidate(current, note))
                {
                    io.confirm_existing_object()?;
                    return Ok(history.last().expect("checked above").clone());
                }
                return Err(StoreError::RevisionConflict {
                    expected: expected_revision,
                    actual,
                });
            }
            let revision = expected_revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            let mut saved = note.clone();
            saved.revision = revision;
            let encoded = encode_stored_checked(&saved).map_err(StoreError::InvalidDocument)?;
            if encoded.len() > MAX_STORED_NOTE_BYTES {
                return Err(StoreError::InvalidDocument("note exceeds 16 MiB"));
            }
            history.push(saved.clone());
            let bytes = index.encode()?;
            io.write_object(self.binding.notes_object_id, &bytes)?;
            Ok(saved)
        })
    }
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], StoreError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| StoreError::Corrupt("profile Notes object length overflow".to_owned()))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| StoreError::Corrupt("truncated profile Notes object".to_owned()))?;
    *cursor = end;
    Ok(value)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, StoreError> {
    let bytes = take(bytes, cursor, 4)?;
    Ok(u32::from_be_bytes(
        bytes.try_into().expect("checked length"),
    ))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, StoreError> {
    let bytes = take(bytes, cursor, 8)?;
    Ok(u64::from_be_bytes(
        bytes.try_into().expect("checked length"),
    ))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::domain::{Block, BlockKind, Timestamp};
    use nagi_identity_session::{CallerContextId, PrincipalId, SessionId};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TestCaller {
        principal: PrincipalId,
        context: CallerContextId,
    }

    impl TrustedCallerContext for TestCaller {
        fn principal_id(&self) -> &PrincipalId {
            &self.principal
        }

        fn context_id(&self) -> &CallerContextId {
            &self.context
        }
    }

    fn binding(
        identity_principal: &str,
        identity_context: &str,
        identity_profile: &str,
        caller_principal: &str,
        caller_context: &str,
        requested_profile: &str,
        notes_object_id: ObjectId,
    ) -> Result<ProfileRootNotesBinding, ProfileRootNotesBindingError> {
        let identity_principal = PrincipalId::new(identity_principal).unwrap();
        let identity_context = CallerContextId::new(identity_context).unwrap();
        let identity_profile = ProfileId::new(identity_profile).unwrap();
        let session = SessionId::new("session_notes_binding_test").unwrap();
        let caller = TestCaller {
            principal: PrincipalId::new(caller_principal).unwrap(),
            context: CallerContextId::new(caller_context).unwrap(),
        };
        ProfileRootNotesBinding::from_claims(
            &identity_principal,
            &identity_context,
            &session,
            &identity_profile,
            &caller,
            &ProfileId::new(requested_profile).unwrap(),
            notes_object_id,
        )
    }

    pub(in crate::store) const RETRY_INDEX_ID: ObjectId = ObjectId(0xfeed_0001);

    fn retry_binding() -> ProfileRootNotesBinding {
        binding(
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            RETRY_INDEX_ID,
        )
        .unwrap()
    }

    pub(in crate::store) fn check_durable_retry(
        io: impl ProfileRootNotesObjectIo + Send + 'static,
        fail_next_sync: impl Fn(),
        write_count: &AtomicUsize,
    ) {
        use crate::{
            ActivityError, ActivityEvent, ActivityKind, ActivitySink, NotesApp, SaveStatus,
            SystemClock,
        };
        use std::time::Duration;

        // Supply only the admitted effect; the tests provide its object I/O.
        struct Input<I> {
            binding: ProfileRootNotesBinding,
            io: I,
        }
        impl<I: ProfileRootNotesObjectIo + Send> ProfileRootNotesContext for Input<I> {
            fn binding(&self) -> &ProfileRootNotesBinding {
                &self.binding
            }
        }
        impl<I: ProfileRootNotesObjectIo> ProfileRootNotesEffectProvider for Input<I> {
            fn with_live_root_effect<R>(
                &mut self,
                effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
            ) -> Result<R, StoreError> {
                effect(&mut self.io)
            }
        }

        #[derive(Default)]
        struct SavedEvents(Mutex<Vec<(ObjectId, u64)>>);
        impl ActivitySink for SavedEvents {
            fn record(&self, event: ActivityEvent) -> Result<(), ActivityError> {
                if event.kind == ActivityKind::Saved {
                    self.0
                        .lock()
                        .unwrap()
                        .push((event.object_id, event.revision));
                }
                Ok(())
            }
        }

        let store = Arc::new(ProfileRootNotesStore::new(Input {
            binding: retry_binding(),
            io,
        }));
        let id = ObjectId(0x1234);
        let initial = store
            .commit(&NoteDocument::new(id, "Parent sync", Timestamp(1)), 0)
            .unwrap();
        let activity = Arc::new(SavedEvents::default());
        let app = NotesApp::with_providers(
            store.clone(),
            Arc::new(HostObjectIdSource::new()),
            Arc::new(SystemClock),
            activity.clone(),
            Duration::from_secs(3600),
        );
        let session = app.open_note(id).unwrap();
        session
            .append_block(Block::new(
                ObjectId(0x5678),
                BlockKind::Paragraph("retain this draft".to_owned()),
            ))
            .unwrap();

        fail_next_sync();
        assert!(session.flush().is_err());
        let published = store.load(id).unwrap().unwrap();
        assert_eq!(published.id, id);
        assert_eq!(published.revision, initial.revision + 1);
        assert_eq!(published.search_text(), "retain this draft");
        assert!(session.is_dirty());
        assert!(matches!(session.status(), SaveStatus::Failed(ref error)
            if error.source == StoreError::ProfileRootWriteUnconfirmed));
        assert!(app.close_note(id).is_err());
        let draft = session.note();
        assert_eq!(draft.id, id);
        assert_eq!(draft.revision, initial.revision);
        assert_eq!(draft.search_text(), published.search_text());
        assert!(activity.0.lock().unwrap().is_empty());

        fail_next_sync();
        assert!(session.retry().is_err());
        assert!(session.is_dirty());
        assert_eq!(session.note(), draft);
        assert!(activity.0.lock().unwrap().is_empty());
        assert_eq!(
            write_count.load(Ordering::SeqCst),
            2,
            "seed and original publish"
        );

        let saved = session.retry().unwrap();
        assert_eq!(saved, published);
        assert!(!session.is_dirty());
        assert_eq!(session.status(), SaveStatus::Saved);
        assert_eq!(app.close_note(id).unwrap(), saved);
        assert_eq!(app.open_note(id).unwrap().note(), saved);
        assert_eq!(*activity.0.lock().unwrap(), [(id, saved.revision)]);
        assert_eq!(write_count.load(Ordering::SeqCst), 2);
        assert_eq!(store.load_revision(id, saved.revision + 1).unwrap(), None);
    }

    // Portable check of the Notes state machine. Linux additionally runs the
    // same assertions with the real object directory and fsync syscall.
    #[test]
    fn profile_root_save_waits_for_durable_retry() {
        use std::sync::atomic::AtomicBool;

        struct MemoryDurabilityIo {
            bytes: Vec<u8>,
            fail_next_sync: Arc<AtomicBool>,
            writes: Arc<AtomicUsize>,
        }
        impl MemoryDurabilityIo {
            fn sync_directory(&self) -> Result<(), StorageRootFailure> {
                if self.fail_next_sync.swap(false, Ordering::SeqCst) {
                    Err(StorageRootFailure::Unavailable)
                } else {
                    Ok(())
                }
            }
        }
        impl ProfileRootNotesObjectIo for MemoryDurabilityIo {
            fn read_object(&mut self, _: ObjectId) -> Result<Vec<u8>, StoreError> {
                Ok(self.bytes.clone())
            }

            fn write_object(&mut self, _: ObjectId, bytes: &[u8]) -> Result<(), StoreError> {
                self.writes.fetch_add(1, Ordering::SeqCst);
                self.bytes = bytes.to_vec();
                finish_root_write(self.sync_directory(), true, bytes, || {
                    Ok(self.bytes.clone())
                })
            }

            fn confirm_existing_object(&mut self) -> Result<(), StoreError> {
                self.sync_directory().map_err(map_durable_write_failure)
            }
        }

        let fail_next_sync = Arc::new(AtomicBool::new(false));
        let writes = Arc::new(AtomicUsize::new(0));
        check_durable_retry(
            MemoryDurabilityIo {
                bytes: empty_profile_root_notes_object(),
                fail_next_sync: fail_next_sync.clone(),
                writes: writes.clone(),
            },
            || fail_next_sync.store(true, Ordering::SeqCst),
            &writes,
        );
    }

    struct MemoryInput {
        binding: ProfileRootNotesBinding,
        objects: HashMap<ObjectId, Vec<u8>>,
        effect_calls: Arc<AtomicUsize>,
    }

    impl ProfileRootNotesEffectProvider for MemoryInput {
        fn with_live_root_effect<R>(
            &mut self,
            effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
        ) -> Result<R, StoreError> {
            self.effect_calls.fetch_add(1, Ordering::SeqCst);
            let mut io = MemoryIo {
                objects: &mut self.objects,
            };
            effect(&mut io)
        }
    }

    impl ProfileRootNotesContext for MemoryInput {
        fn binding(&self) -> &ProfileRootNotesBinding {
            &self.binding
        }
    }

    struct MemoryIo<'a> {
        objects: &'a mut HashMap<ObjectId, Vec<u8>>,
    }

    impl ProfileRootNotesObjectIo for MemoryIo<'_> {
        fn read_object(&mut self, object_id: ObjectId) -> Result<Vec<u8>, StoreError> {
            self.objects
                .get(&object_id)
                .cloned()
                .ok_or(StoreError::Unavailable)
        }

        fn write_object(&mut self, object_id: ObjectId, bytes: &[u8]) -> Result<(), StoreError> {
            self.objects.insert(object_id, bytes.to_vec());
            Ok(())
        }
    }

    fn memory_input(binding: ProfileRootNotesBinding) -> (MemoryInput, Arc<AtomicUsize>) {
        let notes_object_id = binding.notes_object_id();
        let mut objects = HashMap::new();
        objects.insert(notes_object_id, empty_profile_root_notes_object());
        let effect_calls = Arc::new(AtomicUsize::new(0));
        (
            MemoryInput {
                binding,
                objects,
                effect_calls: Arc::clone(&effect_calls),
            },
            effect_calls,
        )
    }

    #[test]
    fn caller_and_profile_mismatches_are_rejected_before_root_creation() {
        assert!(matches!(
            binding(
                "principal_notes_a",
                "caller_notes_a",
                "profile_notes_a",
                "principal_notes_b",
                "caller_notes_a",
                "profile_notes_a",
                ObjectId(0xfeed_0001)
            ),
            Err(ProfileRootNotesBindingError::CallerMismatch)
        ));
        assert!(matches!(
            binding(
                "principal_notes_a",
                "caller_notes_a",
                "profile_notes_a",
                "principal_notes_a",
                "caller_notes_b",
                "profile_notes_a",
                ObjectId(0xfeed_0001)
            ),
            Err(ProfileRootNotesBindingError::CallerMismatch)
        ));
        assert!(matches!(
            binding(
                "principal_notes_a",
                "caller_notes_a",
                "profile_notes_a",
                "principal_notes_a",
                "caller_notes_a",
                "profile_notes_b",
                ObjectId(0xfeed_0001)
            ),
            Err(ProfileRootNotesBindingError::ProfileMismatch)
        ));
    }

    #[test]
    fn root_note_store_roundtrips_codec_history_in_one_effect_lease_per_call() {
        let binding = binding(
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            ObjectId(0xfeed_0001),
        )
        .unwrap();
        let (input, effect_calls) = memory_input(binding);
        let store = ProfileRootNotesStore::new(input);
        let mut note = NoteDocument::new(ObjectId(0x1234), "Root note", Timestamp(10));
        note.blocks.push(Block::new(
            ObjectId(0x5678),
            BlockKind::Paragraph("stored through the existing Notes codec".to_owned()),
        ));

        let saved = store.commit(&note, 0).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(store.load(note.id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(note.id, 1).unwrap(), Some(saved));
        assert_eq!(store.list(false).unwrap().len(), 1);
        assert_eq!(effect_calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn root_note_store_rejects_another_profile_binding_before_effect() {
        let binding_a = binding(
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            ObjectId(0xfeed_0001),
        )
        .unwrap();
        let binding_b = binding(
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            ObjectId(0xfeed_0001),
        )
        .unwrap();
        let mut binding_b = binding_b;
        binding_b.profile_id = ProfileId::new("profile_notes_b").unwrap();
        let (input, effect_calls) = memory_input(binding_a);
        let store = ProfileRootNotesStore::new(input);

        assert_eq!(
            store.ensure_binding(&binding_b),
            Err(ProfileRootNotesBindingError::ResourceMismatch)
        );
        let binding_other_object = binding(
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            "principal_notes_a",
            "caller_notes_a",
            "profile_notes_a",
            ObjectId(0xfeed_0002),
        )
        .unwrap();
        assert_eq!(
            store.ensure_binding(&binding_other_object),
            Err(ProfileRootNotesBindingError::ResourceMismatch)
        );
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn missing_single_lease_provider_fails_closed_without_running_effect() {
        let mut provider = UnavailableProvider;
        let called = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&called);
        let result = provider.with_live_root_effect(|_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(result, Err(StoreError::Unavailable));
        assert_eq!(called.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn uncertain_c1_state_is_not_reported_as_a_revoked_session() {
        assert_eq!(
            map_root_failure(StorageRootFailure::CorruptState),
            StoreError::Unavailable
        );
        assert_eq!(
            map_root_failure(StorageRootFailure::Unavailable),
            StoreError::Unavailable
        );
        assert_eq!(
            map_root_failure(StorageRootFailure::StaleSession),
            StoreError::SessionGateRejected
        );
    }

    #[test]
    fn durable_write_failure_cannot_be_cleared_by_matching_readback() {
        let intended = b"canonical Notes index bytes";
        let failure = Err(StorageRootFailure::Unavailable);
        assert_eq!(
            finish_root_write(failure, true, intended, || panic!(
                "a read cannot confirm fsync"
            )),
            Err(StoreError::ProfileRootWriteUnconfirmed)
        );
        assert_eq!(
            finish_root_write(failure, false, intended, || Ok(intended.to_vec())),
            Ok(())
        );
        assert_eq!(
            finish_root_write(failure, false, intended, || Ok(
                b"previous index bytes".to_vec()
            )),
            Err(StoreError::Unavailable)
        );
    }

    struct UnavailableProvider;

    impl ProfileRootNotesEffectProvider for UnavailableProvider {
        fn with_live_root_effect<R>(
            &mut self,
            _effect: impl FnOnce(&mut dyn ProfileRootNotesObjectIo) -> Result<R, StoreError>,
        ) -> Result<R, StoreError> {
            Err(StoreError::Unavailable)
        }
    }

    #[test]
    fn malformed_root_notes_object_is_rejected() {
        let mut objects = HashMap::new();
        objects.insert(
            ObjectId(0xfeed_0001),
            [empty_profile_root_notes_object(), vec![1]].concat(),
        );
        let mut io = MemoryIo {
            objects: &mut objects,
        };
        assert!(matches!(
            ProfileNotesIndex::read(&mut io, ObjectId(0xfeed_0001)),
            Err(StoreError::Corrupt(_))
        ));
    }
}
