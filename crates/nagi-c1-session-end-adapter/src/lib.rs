//! C1-owned routing for an external UI/session-end notice.
//!
//! This adapter holds the actual service-issued C1 context and one uniquely
//! borrowed receiver. Its opaque delivery is bound to that receiver before
//! `IdentityService::end_session` runs. It is neither a durable outbox nor an
//! OS-child cleanup acknowledgement; E supplies its own transition type and
//! C3 must supply its real trusted child/launch-record integration.
#![forbid(unsafe_code)]

use nagi_identity_session::{
    AuthenticatedInteractiveCaller, CallerContextId, IdentityError, InteractiveSessionContext,
    InteractiveSessionError, PrincipalId,
};
use std::sync::Arc;

pub mod end_owner;
pub mod owner_proxy;

/// Opaque, in-process recipient attachment generation. Cloning retains this
/// exact target; replacing a UI or child requires a fresh attachment. It is
/// not an ID, issuer epoch, launch generation, or authorization proof.
#[derive(Clone)]
pub struct AttachmentGeneration(Arc<()>);

/// Compatibility spelling for E-side callers; this remains a local token.
pub type UiGeneration = AttachmentGeneration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteError {
    Context(InteractiveSessionError),
    StaleGeneration,
    SessionStillLive,
    DuplicateNotice,
    OperationsStopped,
    DeliveryUnconfirmed,
}

/// Owns the real C1 context and uniquely borrows one recipient. It does not
/// own the UI's dirty state or grant authority to a callback.
pub struct SessionEndGate<'caller, 'receiver, U> {
    context: InteractiveSessionContext<'caller>,
    caller: &'caller dyn AuthenticatedInteractiveCaller,
    principal: PrincipalId,
    caller_context: CallerContextId,
    generation: AttachmentGeneration,
    stopped: bool,
    attempted: bool,
    last_error: Option<RouteError>,
    receiver: &'receiver mut U,
}

impl<'caller, 'receiver, U> SessionEndGate<'caller, 'receiver, U> {
    pub fn attach(
        context: InteractiveSessionContext<'caller>,
        caller: &'caller dyn AuthenticatedInteractiveCaller,
        receiver: &'receiver mut U,
    ) -> Result<Self, RouteError> {
        if context.principal_id() != caller.principal_id()
            || context.caller_context_id() != caller.context_id()
        {
            return Err(RouteError::Context(InteractiveSessionError::CallerMismatch));
        }
        context.check().map_err(RouteError::Context)?;
        Ok(Self {
            principal: context.principal_id().clone(),
            caller_context: context.caller_context_id().clone(),
            context,
            caller,
            generation: AttachmentGeneration(Arc::new(())),
            stopped: false,
            attempted: false,
            last_error: None,
            receiver,
        })
    }

    pub fn generation(&self) -> AttachmentGeneration {
        self.generation.clone()
    }

    pub fn operations_stopped(&self) -> bool {
        self.stopped
    }

    pub fn last_error(&self) -> Option<RouteError> {
        self.last_error
    }

    pub fn receiver(&self) -> &U {
        self.receiver
    }

    pub fn into_receiver(self) -> &'receiver mut U {
        self.receiver
    }

    /// Call before each new interactive operation. The operation still needs
    /// its own authorization at its C1 boundary.
    pub fn admit_new_operation(&mut self) -> Result<(), RouteError> {
        if self.stopped {
            return self.reject(RouteError::OperationsStopped);
        }
        if let Err(error) = self.context.check() {
            self.stopped = true;
            return self.reject(RouteError::Context(error));
        }
        Ok(())
    }

    pub fn with_live_receiver<R>(
        &mut self,
        operation: impl FnOnce(&mut U, &InteractiveSessionContext<'caller>) -> R,
    ) -> Result<R, RouteError> {
        self.admit_new_operation()?;
        Ok(operation(self.receiver, &self.context))
    }

    fn reject<T>(&mut self, error: RouteError) -> Result<T, RouteError> {
        self.last_error = Some(error);
        Err(error)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ChildBindingFailure<E> {
    Context(RouteError),
    Supplier(E),
}

/// The C3 selector must validate its actual trusted launch record and return
/// that exact child receiver. The closure must not spawn or mutate a child and
/// must not select by untrusted IDs. This adapter does not invent a C3 API.
pub fn bind_exact_child<'caller, 'child, C, E>(
    context: InteractiveSessionContext<'caller>,
    caller: &'caller dyn AuthenticatedInteractiveCaller,
    select: impl FnOnce(&InteractiveSessionContext<'caller>) -> Result<&'child mut C, E>,
) -> Result<SessionEndGate<'caller, 'child, C>, ChildBindingFailure<E>> {
    if context.principal_id() != caller.principal_id()
        || context.caller_context_id() != caller.context_id()
    {
        return Err(ChildBindingFailure::Context(RouteError::Context(
            InteractiveSessionError::CallerMismatch,
        )));
    }
    // Avoid invoking even a read-only selector for an already stale context.
    context
        .check()
        .map_err(|error| ChildBindingFailure::Context(RouteError::Context(error)))?;
    let child = select(&context).map_err(ChildBindingFailure::Supplier)?;
    SessionEndGate::attach(context, caller, child).map_err(ChildBindingFailure::Context)
}

/// Known no-effect end failures may be retried only by `EndOwner` while the
/// same context and trusted caller still pass their live check.
pub(crate) fn is_known_pre_effect_failure(error: IdentityError) -> bool {
    matches!(
        error,
        IdentityError::AuthorizationDenied
            | IdentityError::AuthorizationUnavailable
            | IdentityError::SessionNotFound
            | IdentityError::SessionNotCurrent
    )
}
