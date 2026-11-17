//! Private consumer; native descriptors never leave the storage backend.

use consumer::session_binding::{AdmissionFailure, SessionBinding};
use identity::{
    CurrentIdentityResolver, ProfileId, ProfileRootAuthorizer, ResolvedIdentity, SessionId,
    StorageRootFailure, TrustedCallerContext, UserStorageRootAdapter,
};
use model::ObjectId;
use svc::CancellationToken;

use super::DirectoryBackend;
use crate::{supply_current, SessionProfile, SupplyFailure};

struct ScopedDirectory<'caller> {
    supplied: SessionProfile<'caller, DirectoryBackend>,
    profile: ProfileId,
    session_id: SessionId,
}

impl<'caller> ScopedDirectory<'caller> {
    fn supply(
        resolver: &mut impl CurrentIdentityResolver,
        caller: &'caller dyn TrustedCallerContext,
        cancellation: CancellationToken,
        mut input: DirectoryBackend,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<Self, SupplyFailure> {
        // Wire exactly the same runtime token into admission and backend work.
        input.cancellation = Some(cancellation.clone());
        let supplied = supply_current(
            resolver,
            caller,
            cancellation,
            UserStorageRootAdapter::new(input),
            authorizer,
        )?;
        let identity = supplied
            .session()
            .identity()
            .map_err(SupplyFailure::Session)?;
        let profile = identity.profile_id().clone();
        // A cached session ID is used only for releasing this adapter's roots,
        // never as evidence or as an authorization input.
        let session_id = identity.session_id().clone();
        Ok(Self {
            supplied,
            profile,
            session_id,
        })
    }

    fn read(
        &mut self,
        object: ObjectId,
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<Vec<u8>, SupplyFailure> {
        let (session, handle, roots) = self.supplied.parts();
        let access = session.access_context().map_err(SupplyFailure::Session)?;
        let mut gate = AdmissionGate {
            session,
            inner: authorizer,
            aborted: None,
        };
        let result = roots.read_object(&access, handle, &self.profile, object, &mut gate);
        if let Some(reason) = gate.aborted {
            return Err(SupplyFailure::Session(reason));
        }
        result.map_err(SupplyFailure::Profile)
    }

    fn write(
        &mut self,
        object: ObjectId,
        data: &[u8],
        authorizer: &mut impl ProfileRootAuthorizer,
    ) -> Result<(), SupplyFailure> {
        let (session, handle, roots) = self.supplied.parts();
        let access = session.access_context().map_err(SupplyFailure::Session)?;
        let mut gate = AdmissionGate {
            session,
            inner: authorizer,
            aborted: None,
        };
        let result = roots.write_object(&access, handle, &self.profile, object, data, &mut gate);
        if let Some(reason) = gate.aborted {
            return Err(SupplyFailure::Session(reason));
        }
        result.map_err(SupplyFailure::Profile)
    }

    // Local request cancellation/root release, not authorized Identity logout.
    // In-flight work may finish; this is not rollback or forced fd revocation.
    fn revoke(&mut self) {
        self.supplied.session().cancel();
        let (_, _, roots) = self.supplied.parts();
        roots.revoke_session(&self.session_id);
    }
}

// Compose the existing policy with consumer admission; never grant on its behalf.
struct AdmissionGate<'operation, 'caller, A> {
    session: &'operation SessionBinding<'caller>,
    inner: &'operation mut A,
    aborted: Option<AdmissionFailure>,
}

impl<A> AdmissionGate<'_, '_, A> {
    fn check_admission(&mut self) -> Result<(), StorageRootFailure> {
        if let Err(reason) = self.session.check() {
            self.aborted = Some(reason);
            // Internal early-stop only. The consumer restores the actual
            // admission failure; this is not a new persisted policy decision.
            return Err(StorageRootFailure::Denied);
        }
        Ok(())
    }
}

impl<A: ProfileRootAuthorizer> ProfileRootAuthorizer for AdmissionGate<'_, '_, A> {
    fn authorize_profile_root(
        &mut self,
        caller: &dyn TrustedCallerContext,
        identity: &ResolvedIdentity,
        profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        self.check_admission()?;
        // Preserve original Denied/Unavailable results unchanged.
        self.inner
            .authorize_profile_root(caller, identity, profile)?;
        self.check_admission()
    }
}

#[cfg(test)]
mod tests;
