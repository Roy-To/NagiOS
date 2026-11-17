#![forbid(unsafe_code)]

pub mod linux_clock;

use identity::{LiveSnapshot, LiveStatus, ResolvedIdentity, TrustedCallerContext, TryLiveError};
use std::sync::Arc;
use svc::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedCheckFailure {
    Cancelled,
    CallerMismatch,
    Stale(LiveStatus),
    Unavailable(TryLiveError),
}

/// Candidate shared observation of a REAL SDK resolver result. No raw-ID
/// constructor, serialization, C2 lease constructor, grant or authority mint.
/// The runtime supplies the original trusted caller and cancellation token;
/// an IPC payload must never supply either. Caller getters must themselves be
/// non-waiting. Sync is required of this actual caller, not inferred from IDs.
///
/// This does not change fixed C1's !Send context or expose its private lease.
/// Integration still requires C1's issuer factory to retain the same caller
/// and cancellation source. A different token is not a valid substitute.
pub struct SharedObservation<'caller> {
    identity: ResolvedIdentity,
    caller: SharedCaller<'caller>,
    cancellation: CancellationToken,
}

enum SharedCaller<'caller> {
    Borrowed(&'caller (dyn TrustedCallerContext + Sync)),
    Owned(Arc<dyn TrustedCallerContext + Send + Sync>),
}

impl SharedCaller<'_> {
    fn get(&self) -> &dyn TrustedCallerContext {
        match self {
            Self::Borrowed(caller) => *caller,
            Self::Owned(caller) => caller.as_ref(),
        }
    }
}

impl<'caller> SharedObservation<'caller> {
    pub fn from_resolved(
        identity: ResolvedIdentity,
        caller: &'caller (dyn TrustedCallerContext + Sync),
        cancellation: CancellationToken,
    ) -> Result<Self, SharedCheckFailure> {
        let observation = Self {
            identity,
            caller: SharedCaller::Borrowed(caller),
            cancellation,
        };
        observation.initially_live()
    }

    fn initially_live(self) -> Result<Self, SharedCheckFailure> {
        match self.check()?.status() {
            LiveStatus::Live => Ok(self),
            stale => Err(SharedCheckFailure::Stale(stale)),
        }
    }

    fn matches_caller(&self) -> bool {
        self.identity.principal_id() == self.caller.get().principal_id()
            && self.identity.caller_context_id() == self.caller.get().context_id()
    }

    /// Two caller-pair/cancellation checks bracket the actual SDK clock and
    /// revocation check. No work queue, blocking lock, spin/retry or saved bool.
    /// Concurrent revocation AFTER this observation is possible; operations
    /// still need their existing authorization/commit synchronization.
    pub fn check(&self) -> Result<LiveSnapshot, SharedCheckFailure> {
        if self.cancellation.is_cancelled() {
            return Err(SharedCheckFailure::Cancelled);
        }
        if !self.matches_caller() {
            return Err(SharedCheckFailure::CallerMismatch);
        }
        let result = self.identity.try_live_snapshot();
        if !self.matches_caller() {
            return Err(SharedCheckFailure::CallerMismatch);
        }
        if self.cancellation.is_cancelled() {
            return Err(SharedCheckFailure::Cancelled);
        }
        result.map_err(SharedCheckFailure::Unavailable)
    }

    /// Captured metadata, not fresh authority. Call check at admission.
    pub fn session_id(&self) -> &identity::SessionId {
        self.identity.session_id()
    }
    pub fn profile_id(&self) -> &identity::ProfileId {
        self.identity.profile_id()
    }
}

impl SharedObservation<'static> {
    /// Retain the SAME real runtime caller object for a shared authority whose
    /// storage must be 'static. No self-referential borrowed context, copied
    /// principal, unsafe cast or manufactured caller is needed. Issuer loss
    /// still revokes the observation even though the caller Arc stays alive.
    pub fn from_owned(
        identity: ResolvedIdentity,
        caller: Arc<dyn TrustedCallerContext + Send + Sync>,
        cancellation: CancellationToken,
    ) -> Result<Self, SharedCheckFailure> {
        Self {
            identity,
            caller: SharedCaller::Owned(caller),
            cancellation,
        }
        .initially_live()
    }
}
