//! Owner-controlled end and reconciliation path for C1's real Session.

use crate::{
    bind_exact_child, is_known_pre_effect_failure, AttachmentGeneration, ChildBindingFailure,
    RouteError, SessionEndGate,
};
use nagi_identity_session::{
    AuthenticatedInteractiveCaller, Clock, EndSessionResult, GuestProfileCleaner,
    IdentityAuthorizer, IdentityEventSink, IdentityIdSource, IdentityService,
    IdentitySnapshotStore, InteractiveSessionContext, InteractiveSessionError,
};
use std::sync::Arc;

/// No raw constructor, serializer, or replacement-target setter. Only a
/// successful end of the captured C1 Session produces a delivery.
pub struct EndDelivery<T> {
    target: AttachmentGeneration,
    transition: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndFailure {
    Context(RouteError),
    WrongIssuer,
    Lifecycle(nagi_identity_session::IdentityError),
    SessionAlreadyEnded,
    AlreadyAttempted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndAttemptState {
    NotAttempted,
    RetryablePreEffect,
    OutcomeUnknown,
    Ended,
}

/// Callback completion is not UI acknowledgement or an OS cleanup receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryState {
    NotAttempted,
    Unconfirmed,
    CallbackReturned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupState {
    NotRequested,
    Requested,
    Unknown,
}

/// Observation only; process reaping is not full cleanup completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupProgress {
    Pending,
    Reaped,
    Unknown,
}

#[derive(Debug, Eq, PartialEq)]
pub enum CleanupFailure<E> {
    StillLive,
    NotRequested,
    AlreadyAttempted,
    Receiver(E),
    Observer(E),
}

/// Owns the live C1 context and pins one recipient by a unique mutable borrow.
/// It never creates a Session ID or models C3's process cleanup as complete.
pub struct EndOwner<'caller, 'receiver, U> {
    gate: SessionEndGate<'caller, 'receiver, U>,
    cleanup: CleanupState,
    cleanup_reason: Option<InteractiveSessionError>,
    end_attempt: EndAttemptState,
    delivery: DeliveryState,
}

impl<'caller, 'receiver, U> EndOwner<'caller, 'receiver, U> {
    pub fn attach(
        context: InteractiveSessionContext<'caller>,
        caller: &'caller dyn AuthenticatedInteractiveCaller,
        receiver: &'receiver mut U,
    ) -> Result<Self, RouteError> {
        Ok(Self {
            gate: SessionEndGate::attach(context, caller, receiver)?,
            cleanup: CleanupState::NotRequested,
            cleanup_reason: None,
            end_attempt: EndAttemptState::NotAttempted,
            delivery: DeliveryState::NotAttempted,
        })
    }

    pub fn bind_exact_child<E>(
        context: InteractiveSessionContext<'caller>,
        caller: &'caller dyn AuthenticatedInteractiveCaller,
        select: impl FnOnce(&InteractiveSessionContext<'caller>) -> Result<&'receiver mut U, E>,
    ) -> Result<Self, ChildBindingFailure<E>> {
        Ok(Self {
            gate: bind_exact_child(context, caller, select)?,
            cleanup: CleanupState::NotRequested,
            cleanup_reason: None,
            end_attempt: EndAttemptState::NotAttempted,
            delivery: DeliveryState::NotAttempted,
        })
    }

    pub fn receiver(&self) -> &U {
        self.gate.receiver()
    }

    pub fn into_receiver(self) -> &'receiver mut U {
        self.gate.into_receiver()
    }

    pub fn cleanup_state(&self) -> CleanupState {
        self.cleanup
    }

    pub fn end_attempt_state(&self) -> EndAttemptState {
        self.end_attempt
    }

    pub fn delivery_state(&self) -> DeliveryState {
        self.delivery
    }

    pub fn operations_stopped(&self) -> bool {
        self.gate.operations_stopped()
    }

    pub fn admit_new_operation(&mut self) -> Result<(), RouteError> {
        self.gate.admit_new_operation()
    }

    pub fn with_live_receiver<R>(
        &mut self,
        operation: impl FnOnce(&mut U, &InteractiveSessionContext<'caller>) -> R,
    ) -> Result<R, RouteError> {
        self.gate.with_live_receiver(operation)
    }

    /// End exactly the Session captured from the actual C1 observation. The
    /// service still authorizes and durably records the end. An `Ok` callback
    /// result is not a child cleanup receipt, and process failure can still
    /// lose the returned in-process delivery.
    pub fn end<S, I, C, A, G, E, T>(
        &mut self,
        service: &mut IdentityService<S, I, C, A, G, E>,
        transition: T,
    ) -> Result<EndDelivery<T>, EndFailure>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !self.gate.context.is_issued_by(service) {
            return Err(EndFailure::WrongIssuer);
        }
        match self.end_attempt {
            EndAttemptState::NotAttempted => self
                .gate
                .admit_new_operation()
                .map_err(EndFailure::Context)?,
            EndAttemptState::RetryablePreEffect => {
                if let Err(error) = self.gate.context.check() {
                    self.end_attempt = EndAttemptState::OutcomeUnknown;
                    self.gate.stopped = true;
                    return Err(EndFailure::Context(RouteError::Context(error)));
                }
            }
            EndAttemptState::OutcomeUnknown | EndAttemptState::Ended => {
                return Err(EndFailure::AlreadyAttempted);
            }
        }

        // Capture the Session before the first lifecycle callback; the stop
        // latch is set immediately before entering the effectful C1 method.
        let session_id = self
            .gate
            .context
            .identity()
            .map_err(|error| EndFailure::Context(RouteError::Context(error)))?
            .session_id()
            .clone();
        let target = self.gate.generation();
        self.gate.stopped = true;
        self.end_attempt = EndAttemptState::OutcomeUnknown;

        match service.end_session(self.gate.caller, &session_id) {
            Ok(EndSessionResult::Ended | EndSessionResult::GuestCleanupPending) => {
                self.end_attempt = EndAttemptState::Ended;
                Ok(EndDelivery { target, transition })
            }
            Ok(EndSessionResult::AlreadyEnded) => Err(EndFailure::SessionAlreadyEnded),
            Err(error) => {
                if is_known_pre_effect_failure(error)
                    && self.gate.context.check().is_ok()
                    && self.gate.caller.principal_id() == &self.gate.principal
                    && self.gate.caller.context_id() == &self.gate.caller_context
                {
                    self.end_attempt = EndAttemptState::RetryablePreEffect;
                }
                Err(EndFailure::Lifecycle(error))
            }
        }
    }

    /// Deliver only to the original attachment and only after the captured
    /// C1 Session has ended or expired. A callback return is not a render or
    /// cleanup acknowledgement. Panics leave delivery latched unconfirmed.
    pub fn deliver<T: Clone, N>(
        &mut self,
        delivery: &EndDelivery<T>,
        make_notice: impl FnOnce(T) -> N,
        receive: impl FnOnce(&mut U, N),
    ) -> Result<(), RouteError> {
        if !Arc::ptr_eq(&self.gate.generation.0, &delivery.target.0) {
            return self.gate.reject(RouteError::StaleGeneration);
        }
        if self.gate.caller.principal_id() != &self.gate.principal
            || self.gate.caller.context_id() != &self.gate.caller_context
        {
            self.gate.stopped = true;
            return self
                .gate
                .reject(RouteError::Context(InteractiveSessionError::CallerMismatch));
        }
        match self.gate.context.check() {
            Ok(()) => return self.gate.reject(RouteError::SessionStillLive),
            Err(InteractiveSessionError::Identity(
                nagi_identity_session::ResolutionError::Ended
                | nagi_identity_session::ResolutionError::Expired,
            )) => {}
            Err(error) => {
                self.gate.stopped = true;
                return self.gate.reject(RouteError::Context(error));
            }
        }
        self.gate.stopped = true;
        if self.gate.attempted {
            return self.gate.reject(RouteError::DuplicateNotice);
        }
        self.gate.attempted = true;
        self.delivery = DeliveryState::Unconfirmed;
        self.gate.last_error = Some(RouteError::DeliveryUnconfirmed);
        let notice = make_notice(delivery.transition.clone());
        receive(self.gate.receiver, notice);
        self.delivery = DeliveryState::CallbackReturned;
        self.gate.last_error = None;
        Ok(())
    }

    /// Make one effectful cleanup request against this exact retained
    /// receiver after the C1 context is stale or unavailable. The callback is
    /// never re-run; its return value does not establish cleanup completion.
    pub fn request_cleanup_once<E>(
        &mut self,
        request: impl FnOnce(&mut U, InteractiveSessionError) -> Result<(), E>,
    ) -> Result<(), CleanupFailure<E>> {
        let reason = match self.gate.context.check() {
            Ok(()) => return Err(CleanupFailure::StillLive),
            Err(error) => error,
        };
        self.gate.stopped = true;
        self.gate.last_error = Some(RouteError::Context(reason));
        if self.cleanup != CleanupState::NotRequested {
            return Err(CleanupFailure::AlreadyAttempted);
        }
        self.cleanup_reason = Some(reason);
        self.cleanup = CleanupState::Unknown;
        request(self.gate.receiver, reason).map_err(CleanupFailure::Receiver)?;
        self.cleanup = CleanupState::Requested;
        Ok(())
    }

    /// Re-observe the same retained receiver, including after an ambiguous
    /// request callback or an `Unknown` observation. `Pending` and `Reaped`
    /// are observations, not full completion. C1 cannot manufacture C3's
    /// sealed exact-terminal receipt from a generic callback.
    pub fn poll_cleanup<E>(
        &mut self,
        observe: impl FnOnce(&mut U, InteractiveSessionError) -> Result<CleanupProgress, E>,
    ) -> Result<CleanupProgress, CleanupFailure<E>> {
        match self.cleanup {
            CleanupState::NotRequested => return Err(CleanupFailure::NotRequested),
            CleanupState::Requested | CleanupState::Unknown => {}
        }
        let reason = self.cleanup_reason.ok_or(CleanupFailure::NotRequested)?;
        match observe(self.gate.receiver, reason) {
            Ok(CleanupProgress::Unknown) => {
                self.cleanup = CleanupState::Unknown;
                Ok(CleanupProgress::Unknown)
            }
            Ok(progress @ (CleanupProgress::Pending | CleanupProgress::Reaped)) => {
                self.cleanup = CleanupState::Requested;
                Ok(progress)
            }
            Err(error) => Err(CleanupFailure::Observer(error)),
        }
    }
}
