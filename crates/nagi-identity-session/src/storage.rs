use std::collections::HashMap;
use std::sync::Arc;

use nagi_model::ObjectId;

use crate::{
    CallerContextId, IdentityAccessContext, PrincipalId, ProfileId, ResolvedIdentity, SessionId,
    TrustedCallerContext, UserId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageRootFailure {
    Denied,
    CallerMismatch,
    StaleSession,
    CrossProfile,
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
    session_id: SessionId,
    profile_id: ProfileId,
    user_id: Option<UserId>,
    caller_principal: PrincipalId,
    caller_context: CallerContextId,
}

impl std::fmt::Debug for StorageRootHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StorageRootHandle(<opaque>)")
    }
}

struct BoundRoot<R> {
    session_id: SessionId,
    profile_id: ProfileId,
    user_id: Option<UserId>,
    caller_principal: PrincipalId,
    caller_context: CallerContextId,
    root: Arc<R>,
}

pub struct UserStorageRootAdapter<B: ProfileStorageBackend, E = NoopStorageRootEventSink> {
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
            .ok_or(StorageRootFailure::StaleSession)?;
        let token = self.next_token;
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or(StorageRootFailure::HandleSpaceExhausted)?;
        let root = self
            .backend
            .open_profile_root(access.identity.user_id(), requested_profile)?;
        self.roots.insert(
            token,
            BoundRoot {
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
        self.authorize_scope(access, requested_profile, authorizer)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .ok_or(StorageRootFailure::StaleSession)?;
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

    fn write_object_inner(
        &mut self,
        access: &IdentityAccessContext<'_>,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
        object_id: ObjectId,
        data: &[u8],
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<(), StorageRootFailure> {
        self.authorize_scope(access, requested_profile, authorizer)?;
        let _lease = access
            .identity
            .acquire_live_lease()
            .ok_or(StorageRootFailure::StaleSession)?;
        let root = Arc::clone(
            &self
                .bound_root(access.identity, handle, requested_profile)?
                .root,
        );
        self.backend.write_object(&root, object_id, data)
    }

    pub fn revoke_session(&mut self, session_id: &SessionId) {
        self.roots.retain(|_, root| &root.session_id != session_id);
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
        if !access.identity.is_live() {
            return Err(StorageRootFailure::StaleSession);
        }
        if access.identity.principal_id() != access.caller.principal_id()
            || access.identity.caller_context_id() != access.caller.context_id()
        {
            return Err(StorageRootFailure::CallerMismatch);
        }
        if access.identity.profile_id() != requested_profile {
            return Err(StorageRootFailure::CrossProfile);
        }
        authorizer.authorize_profile_root(access.caller, access.identity, requested_profile)
    }

    fn bound_root(
        &self,
        identity: &ResolvedIdentity,
        handle: &StorageRootHandle,
        requested_profile: &ProfileId,
    ) -> Result<&BoundRoot<B::Root>, StorageRootFailure> {
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

impl<B: ProfileStorageBackend> UserStorageRootAdapter<B, NoopStorageRootEventSink> {
    pub fn into_backend(self) -> B {
        self.backend
    }
}
