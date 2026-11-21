use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Weak};

use nagi_model::ObjectId;

use crate::{
    CallerContextId, IdentityAccessContext, PrincipalId, ProfileId, ResolutionError,
    ResolvedIdentity, SessionId, TrustedCallerContext, UserId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageRootFailure {
    Denied,
    CallerMismatch,
    /// The Session was confirmed ended or expired.
    StaleSession,
    CrossProfile,
    /// The current committed C1 identity snapshot is corrupt and was rejected.
    /// This does not prove that the Session was ended; explicit C1 recovery is
    /// required before liveness can be established again.
    CorruptState,
    /// Session liveness could not be confirmed because a gate or dependency
    /// was unavailable. This may be transient and is not proof that the
    /// Session was revoked.
    Unavailable,
    InvalidHandle,
    HandleSpaceExhausted,
}

/// A separate authorization hook for opening and using a profile root. The
/// caller must derive its decision from trusted service context.
pub trait ProfileRootAuthorizer {
    fn authorize_profile_root(
        &mut self,
        caller: &dyn TrustedCallerContext,
        identity: &ResolvedIdentity,
        requested_profile: &ProfileId,
    ) -> Result<(), StorageRootFailure>;
}

pub trait StorageRootEventSink {
    fn failed(&mut self, profile_id: &ProfileId, reason: StorageRootFailure) -> bool;
}

#[derive(Default)]
pub struct NoopStorageRootEventSink;

impl StorageRootEventSink for NoopStorageRootEventSink {
    fn failed(&mut self, _profile_id: &ProfileId, _reason: StorageRootFailure) -> bool {
        true
    }
}

/// Backend roots are opaque provider handles, never paths exposed by this
/// adapter. A production VFS owner may implement this after the integration
/// gate; tests use an in-memory fake.
/// Handles are bound to both this exact adapter instance and the C1 issuer
/// that opened them. One adapter may retain roots for multiple issuer
/// instances, such as across service recovery; cleanup always selects the
/// exact issuer token. Each issuer registers this exact adapter in its C1
/// admission state, so an issuer cannot move to a second live adapter.
/// Backends normally resolve the logical User/Profile namespace shared by
/// those service instances. A backend that requires additional isolation must
/// partition it itself because `open_profile_root(user_id, profile_id)` does
/// not receive an issuer identity.
/// Synchronous profile storage invoked while C1 holds the checked Session
/// lease. Implementations must enforce an owner-specific deadline and return
/// `StorageRootFailure::Unavailable` when they cannot complete within it;
/// this adapter cannot interrupt a blocked backend call. A backend must not
/// re-enter C1 lease, storage, or lifecycle APIs, and must not wait for an
/// external lock whose holder may be waiting on the same C1 Session gate.
/// Dropping `Root` releases the adapter's handle to the provider root; it does
/// not itself end or revoke the C1 Session. A production backend must keep
/// root destruction bounded, nonblocking, non-reentrant, and non-panicking;
/// C1 cannot interrupt an arbitrary Rust destructor. Lifecycle cleanup drops
/// the root only after the exact Session ends or its issuer closes.
///
/// `write_object` must define whether an error guarantees that no commit
/// occurred. If a timeout, panic, or transport failure can happen after the
/// object may have committed, the storage owner must reconcile by an
/// idempotency key or read-after-write check before retrying. The C1 lease
/// protects Session revocation ordering; it does not roll back backend work.
pub trait ProfileStorageBackend {
    type Root;

    fn open_profile_root(
        &mut self,
        user_id: Option<UserId>,
        profile_id: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure>;

    fn read_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure>;

    fn write_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure>;
}

/// In-process capability-scoped reference. It cannot be serialized, cloned,
/// constructed, or inspected by callers. It contains no host path.
pub struct StorageRootHandle {
    token: u64,
    adapter_identity: Weak<()>,
    issuer_identity: Weak<AtomicBool>,
    session_id: SessionId,
    profile_id: ProfileId,
    user_id: Option<UserId>,
    caller_principal: PrincipalId,
    caller_context: CallerContextId,
}

/// A short-lived view of one already-authorized profile root. It can only be
/// created by `UserStorageRootAdapter::with_live_root_effect`, which holds the
/// matching C1 Session lease for the full callback. It exposes root-scoped I/O
/// without exposing the backend root or acquiring a nested C1 lease for each
/// operation. It is neither `Send` nor `Sync`, so this lease-scoped view cannot
/// be moved to or shared with a child thread.
///
/// ```compile_fail
/// use nagi_identity_session::{ProfileRootEffect, ProfileStorageBackend};
/// fn assert_send<T: Send>() {}
/// fn effect_view_is_local<B: ProfileStorageBackend + Send>() {
///     assert_send::<ProfileRootEffect<'static, B>>();
/// }
/// ```
pub struct ProfileRootEffect<'backend, B: ProfileStorageBackend> {
    backend: &'backend mut B,
    root: Arc<B::Root>,
    _local: PhantomData<Rc<()>>,
}

impl<B: ProfileStorageBackend> ProfileRootEffect<'_, B> {
    /// Read an object through the one profile root bound to the C1 caller,
    /// Session, and profile for this effect callback.
    pub fn read_object(&mut self, object_id: ObjectId) -> Result<Vec<u8>, StorageRootFailure> {
        self.backend.read_object(&self.root, object_id)
    }

    /// Write an object through the one profile root bound to the C1 caller,
    /// Session, and profile for this effect callback.
    pub fn write_object(
        &mut self,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.backend.write_object(&self.root, object_id, data)
    }
}

impl std::fmt::Debug for StorageRootHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StorageRootHandle(<opaque>)")
    }
}

struct BoundRoot<R> {
    issuer_identity: Weak<AtomicBool>,
    session_id: SessionId,
    profile_id: ProfileId,
    user_id: Option<UserId>,
    caller_principal: PrincipalId,
    caller_context: CallerContextId,
    root: Arc<R>,
}

pub struct UserStorageRootAdapter<B: ProfileStorageBackend, E = NoopStorageRootEventSink> {
    adapter_identity: Arc<()>,
    backend: B,
    next_token: u64,
    roots: HashMap<u64, BoundRoot<B::Root>>,
    events: E,
}

impl<B: ProfileStorageBackend> UserStorageRootAdapter<B, NoopStorageRootEventSink> {
    pub fn new(backend: B) -> Self {
        Self::with_event_sink(backend, NoopStorageRootEventSink)
    }
}

impl<B: ProfileStorageBackend, E: StorageRootEventSink> UserStorageRootAdapter<B, E> {
    pub fn with_event_sink(backend: B, events: E) -> Self {
        Self {
            adapter_identity: Arc::new(()),
            backend,
            next_token: 1,
            roots: HashMap::new(),
            events,
        }
    }

    pub fn resolve_root(
        &mut self,
        access: &IdentityAccessContext<'_>,
        requested_profile: &ProfileId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<StorageRootHandle, StorageRootFailure> {
        let result = self.resolve_root_inner(access, requested_profile, authorizer);
        if let Err(reason) = &result {
            let _ = self.events.failed(requested_profile, *reason);
        }
        result
    }

    fn resolve_root_inner(
        &mut self,
        access: &IdentityAccessContext<'_>,
        requested_profile: &ProfileId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<StorageRootHandle, StorageRootFailure> {
        self.authorize_scope(access, requested_profile, authorizer)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .map_err(map_resolution_error)?;
        access
            .identity
            .bind_storage_adapter(&self.adapter_identity)
            .map_err(map_resolution_error)?;
        let token = self.next_token;
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or(StorageRootFailure::HandleSpaceExhausted)?;
        let root = self
            .backend
            .open_profile_root(access.identity.user_id(), requested_profile)?;
        let issuer_identity = access.identity.issuer_identity();
        self.roots.insert(
            token,
            BoundRoot {
                issuer_identity: issuer_identity.clone(),
                session_id: access.identity.session_id().clone(),
                profile_id: requested_profile.clone(),
                user_id: access.identity.user_id(),
                caller_principal: access.identity.principal_id().clone(),
                caller_context: access.identity.caller_context_id().clone(),
                root: Arc::new(root),
            },
        );
        Ok(StorageRootHandle {
            token,
            adapter_identity: Arc::downgrade(&self.adapter_identity),
            issuer_identity,
            session_id: access.identity.session_id().clone(),
            profile_id: requested_profile.clone(),
            user_id: access.identity.user_id(),
            caller_principal: access.identity.principal_id().clone(),
            caller_context: access.identity.caller_context_id().clone(),
        })
    }

    pub fn read_object(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        object_id: ObjectId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        let result =
            self.read_object_inner(access, handle, requested_profile, object_id, authorizer);
        if let Err(reason) = &result {
            let _ = self.events.failed(requested_profile, *reason);
        }
        result
    }

    fn read_object_inner(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        object_id: ObjectId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.validate_handle_adapter(handle)?;
        self.validate_handle_issuer(access.identity, handle)?;
        self.authorize_scope(access, requested_profile, authorizer)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .map_err(map_resolution_error)?;
        let root = Arc::clone(
            &self
                .bound_root(access.identity, handle, requested_profile)?
                .root,
        );
        self.backend.read_object(&root, object_id)
    }

    pub fn write_object(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        object_id: ObjectId,
        data: &[u8],
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<(), StorageRootFailure> {
        let result = self.write_object_inner(
            access,
            handle,
            requested_profile,
            object_id,
            data,
            authorizer,
        );
        if let Err(reason) = &result {
            let _ = self.events.failed(requested_profile, *reason);
        }
        result
    }

    /// Run a bounded synchronous effect with root-backed reads or writes under
    /// one checked C1 Session lease. The adapter validates the same caller,
    /// Session, and profile pair used to open `handle`, checks the handle while
    /// holding the lease, and only then lends root-scoped I/O to `effect`.
    ///
    /// This is the composition point for an effect that needs both the C1
    /// revocation gate and profile storage. Do not wrap this callback in
    /// `SessionLeaseRevalidationHandle::with_live_session_effect`, and do not
    /// re-enter C1 storage or lifecycle APIs from the callback; that would
    /// re-acquire the same Session lease. The callback must remain synchronous
    /// on this thread, must not spawn or join C1-using work, and must respect
    /// the backend deadline and reconciliation contract. The borrowed view
    /// cannot escape the callback, and its private fields prevent changing the
    /// selected root.
    pub fn with_live_root_effect<R>(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        authorizer: &mut impl ProfileRootAuthorizer,
        effect: impl FnOnce(&mut ProfileRootEffect<'_, B>) -> Result<R, StorageRootFailure>,
    ) -> Result<R, StorageRootFailure> {
        let result =
            self.with_live_root_effect_inner(access, handle, requested_profile, authorizer, effect);
        if let Err(reason) = &result {
            let _ = self.events.failed(requested_profile, *reason);
        }
        result
    }

    fn with_live_root_effect_inner<R>(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        authorizer: &mut impl ProfileRootAuthorizer,
        effect: impl FnOnce(&mut ProfileRootEffect<'_, B>) -> Result<R, StorageRootFailure>,
    ) -> Result<R, StorageRootFailure> {
        self.validate_handle_adapter(handle)?;
        self.validate_handle_issuer(access.identity, handle)?;
        self.authorize_scope(access, requested_profile, authorizer)?;
        // Authorization may call an external policy owner. Revalidate the
        // exact pair after it returns, then take the lease before checking the
        // opaque handle or entering the effect.
        validate_access_scope(access, requested_profile)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .map_err(map_resolution_error)?;
        let root = Arc::clone(
            &self
                .bound_root(access.identity, handle, requested_profile)?
                .root,
        );
        let mut root_effect = ProfileRootEffect {
            backend: &mut self.backend,
            root,
            _local: PhantomData,
        };
        effect(&mut root_effect)
    }

    fn write_object_inner(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        object_id: ObjectId,
        data: &[u8],
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<(), StorageRootFailure> {
        self.validate_handle_adapter(handle)?;
        self.validate_handle_issuer(access.identity, handle)?;
        self.authorize_scope(access, requested_profile, authorizer)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .map_err(map_resolution_error)?;
        let root = Arc::clone(
            &self
                .bound_root(access.identity, handle, requested_profile)?
                .root,
        );
        self.backend.write_object(&root, object_id, data)
    }

    /// Drop roots opened by this exact resolved Session. This invalidates its
    /// storage handles but does not end the C1 Session. A numeric Session ID
    /// alone is not an ownership key: separate issuers may contain identical
    /// persisted IDs. Cleanup is synchronous and does not wait for C1 leases;
    /// lifecycle callers must first complete the relevant transition.
    pub fn revoke_session(&mut self, identity: &ResolvedIdentity) -> usize {
        let issuer_identity = identity.issuer_identity();
        self.revoke_session_for_identity(&issuer_identity, identity.session_id())
    }

    /// Remove roots owned by one exact Session issuer. This is crate-private so
    /// lifecycle code can clean an issuer without reconstructing an
    /// InteractiveSessionContext after revocation.
    pub(crate) fn revoke_session_for_issuer(
        &mut self,
        issuer_identity: &Arc<AtomicBool>,
        session_id: &SessionId,
    ) -> usize {
        self.revoke_session_for_identity(&Arc::downgrade(issuer_identity), session_id)
    }

    pub(crate) fn adapter_identity(&self) -> &Arc<()> {
        &self.adapter_identity
    }

    /// Remove every root belonging to one issuer after that issuer has
    /// revoked its Sessions or closed admission. Roots for any other issuer
    /// are retained even when their persisted Session IDs are equal.
    pub(crate) fn revoke_issuer(&mut self, issuer_identity: &Arc<AtomicBool>) -> usize {
        let issuer_identity = Arc::downgrade(issuer_identity);
        let before = self.roots.len();
        self.roots
            .retain(|_, root| !Weak::ptr_eq(&root.issuer_identity, &issuer_identity));
        before - self.roots.len()
    }

    fn revoke_session_for_identity(
        &mut self,
        issuer_identity: &Weak<AtomicBool>,
        session_id: &SessionId,
    ) -> usize {
        let before = self.roots.len();
        self.roots.retain(|_, root| {
            !(Weak::ptr_eq(&root.issuer_identity, issuer_identity)
                && &root.session_id == session_id)
        });
        before - self.roots.len()
    }

    fn validate_handle_adapter(
        &self,
        handle: &StorageRootHandle,
    ) -> Result<(), StorageRootFailure> {
        if Weak::ptr_eq(
            &handle.adapter_identity,
            &Arc::downgrade(&self.adapter_identity),
        ) {
            Ok(())
        } else {
            Err(StorageRootFailure::InvalidHandle)
        }
    }

    fn validate_handle_issuer(
        &self,
        identity: &ResolvedIdentity,
        handle: &StorageRootHandle,
    ) -> Result<(), StorageRootFailure> {
        if Weak::ptr_eq(&handle.issuer_identity, &identity.issuer_identity()) {
            Ok(())
        } else {
            Err(StorageRootFailure::InvalidHandle)
        }
    }

    pub fn into_parts(self) -> (B, E) {
        (self.backend, self.events)
    }

    fn authorize_scope(
        &mut self,
        access: &IdentityAccessContext<'_>,
        requested_profile: &ProfileId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<(), StorageRootFailure> {
        if !access
            .identity
            .accepts_storage_adapter(&self.adapter_identity)
            .map_err(map_resolution_error)?
        {
            return Err(StorageRootFailure::InvalidHandle);
        }
        validate_access_scope(access, requested_profile)?;
        authorizer.authorize_profile_root(access.caller, access.identity, requested_profile)
    }

    fn bound_root(
        &self,
        identity: &ResolvedIdentity,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
    ) -> Result<&BoundRoot<B::Root>, StorageRootFailure> {
        self.validate_handle_adapter(handle)?;
        self.validate_handle_issuer(identity, handle)?;
        let issuer_identity = identity.issuer_identity();
        if handle.session_id != *identity.session_id()
            || handle.profile_id != *requested_profile
            || handle.profile_id != *identity.profile_id()
            || handle.user_id != identity.user_id()
            || handle.caller_principal != *identity.principal_id()
            || handle.caller_context != *identity.caller_context_id()
        {
            return Err(StorageRootFailure::CrossProfile);
        }
        let root = self
            .roots
            .get(&handle.token)
            .ok_or(StorageRootFailure::InvalidHandle)?;
        if !Weak::ptr_eq(&root.issuer_identity, &issuer_identity) {
            return Err(StorageRootFailure::InvalidHandle);
        }
        if root.session_id != *identity.session_id()
            || root.profile_id != *requested_profile
            || root.user_id != identity.user_id()
            || root.caller_principal != *identity.principal_id()
            || root.caller_context != *identity.caller_context_id()
        {
            return Err(StorageRootFailure::CrossProfile);
        }
        Ok(root)
    }
}

fn validate_access_scope(
    access: &IdentityAccessContext<'_>,
    requested_profile: &ProfileId,
) -> Result<(), StorageRootFailure> {
    access.identity.check_live().map_err(map_resolution_error)?;
    if access.identity.principal_id() != access.caller.principal_id()
        || access.identity.caller_context_id() != access.caller.context_id()
    {
        return Err(StorageRootFailure::CallerMismatch);
    }
    if access.identity.profile_id() != requested_profile {
        return Err(StorageRootFailure::CrossProfile);
    }
    Ok(())
}

fn map_resolution_error(error: ResolutionError) -> StorageRootFailure {
    match error {
        ResolutionError::NoSession | ResolutionError::Ended | ResolutionError::Expired => {
            StorageRootFailure::StaleSession
        }
        ResolutionError::CorruptState => StorageRootFailure::CorruptState,
        ResolutionError::Denied => StorageRootFailure::Denied,
        ResolutionError::Unavailable => StorageRootFailure::Unavailable,
    }
}

impl<B: ProfileStorageBackend> UserStorageRootAdapter<B, NoopStorageRootEventSink> {
    pub fn into_backend(self) -> B {
        self.backend
    }
}
