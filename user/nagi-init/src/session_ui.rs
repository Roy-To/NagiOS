//! Bounded presentation state for ordinary authenticated Nagi Bar text requests.
//! Scripted transports test orchestration only; production uses SessionServices.
use crate::session_services::{
    ReplyError, SessionServices, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, MAX_OUTPUT_TOKENS,
};
pub use crate::session_services::{ReplyProgress as Reply, SubmitError};
use alloc::string::String;
use libnagi::security::Session;

pub const INPUT_CAPACITY: usize = MAX_INPUT_BYTES;
pub const OUTPUT_CAPACITY: usize = MAX_OUTPUT_BYTES;
pub const TERMS_NAME_CAPACITY: usize = 128;
pub const TERMS_REFERENCE_CAPACITY: usize = 256;

/// Calls only transfer bounded work. Native inference remains in the guest worker.
pub trait Services {
    fn on_signed_in(&mut self, session: &Session) -> Result<(), SubmitError>;
    fn submit_text(&mut self, session: &Session, id: u64, text: &str) -> Result<(), SubmitError>;
    fn poll_reply(&mut self, session: &Session, id: u64) -> Result<Reply, SubmitError>;
    fn cancel_request(&mut self, session: &Session, id: u64) -> Result<(), SubmitError>;
    fn on_lock_or_signout(&mut self, session: &Session);
    fn acknowledge_model_terms(
        &mut self,
        session: &Session,
        reference: &str,
    ) -> Result<(), SubmitError>;
}
impl Services for SessionServices {
    fn on_signed_in(&mut self, s: &Session) -> Result<(), SubmitError> {
        SessionServices::on_signed_in(self, s)
    }
    fn submit_text(&mut self, s: &Session, id: u64, text: &str) -> Result<(), SubmitError> {
        SessionServices::submit_text(self, s, id, text)
    }
    fn poll_reply(&mut self, s: &Session, id: u64) -> Result<Reply, SubmitError> {
        SessionServices::poll_reply(self, s, id)
    }
    fn cancel_request(&mut self, s: &Session, id: u64) -> Result<(), SubmitError> {
        SessionServices::cancel_request(self, s, id)
    }
    fn on_lock_or_signout(&mut self, s: &Session) {
        SessionServices::on_lock_or_signout(self, s)
    }
    fn acknowledge_model_terms(&mut self, s: &Session, reference: &str) -> Result<(), SubmitError> {
        SessionServices::acknowledge_model_terms(self, s, reference)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    SignedOut,
    Ready,
    Queued,
    Waiting,
    Complete,
    Cancelled,
    Unavailable,
    Busy,
    TermsRequired,
    Failed,
    InvalidReply,
    InputFull,
    EmptyInput,
    InvalidInput,
    TermsOffered,
    TermsPending,
    TermsAccepted,
    TermsFailed,
    Denied,
    WorkerUnavailable,
}
impl Status {
    pub const fn key(self) -> &'static str {
        match self {
            Self::SignedOut => "desktop.ai.status.signed_out",
            Self::Ready => "desktop.ai.status.ready",
            Self::Queued => "desktop.ai.status.queued",
            Self::Waiting => "desktop.ai.status.waiting",
            Self::Complete => "desktop.ai.status.complete",
            Self::Cancelled => "desktop.ai.status.cancelled",
            Self::Unavailable => "desktop.ai.status.unavailable",
            Self::Busy => "desktop.ai.status.busy",
            Self::TermsRequired => "desktop.ai.status.terms_required",
            Self::Failed => "desktop.ai.status.failed",
            Self::InvalidReply => "desktop.ai.status.invalid_reply",
            Self::InputFull => "desktop.ai.status.input_full",
            Self::EmptyInput => "desktop.ai.status.empty_input",
            Self::InvalidInput => "desktop.ai.status.invalid_input",
            Self::TermsOffered => "desktop.ai.status.terms_offered",
            Self::TermsPending => "desktop.ai.status.terms_pending",
            Self::TermsAccepted => "desktop.ai.status.terms_accepted",
            Self::TermsFailed => "desktop.ai.status.terms_failed",
            Self::Denied => "desktop.ai.status.denied",
            Self::WorkerUnavailable => "desktop.ai.status.worker_unavailable",
        }
    }
    fn submit_error(error: SubmitError) -> Self {
        match error {
            SubmitError::Denied => Self::Denied,
            SubmitError::Unavailable => Self::Unavailable,
            SubmitError::WorkerUnavailable => Self::WorkerUnavailable,
            SubmitError::InvalidInput => Self::InvalidInput,
            SubmitError::InputTooLarge => Self::InputFull,
            SubmitError::Busy => Self::Busy,
            SubmitError::TermsRequired => Self::TermsRequired,
            SubmitError::InvalidTerms => Self::TermsFailed,
            SubmitError::InvalidRequestId => Self::InvalidReply,
        }
    }
}

pub struct Terms {
    name: String,
    reference: String,
    acknowledged: bool,
}
impl Terms {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn reference(&self) -> &str {
        &self.reference
    }
    pub const fn acknowledged(&self) -> bool {
        self.acknowledged
    }
}
#[derive(Clone, Copy)]
struct Request {
    session: Session,
    id: u64,
}
pub struct Controller {
    session: Option<Session>,
    bound: bool,
    input: String,
    output: String,
    last_request_id: u64,
    queued: Option<Request>,
    in_flight: Option<Request>,
    discarding: bool,
    terms: Option<Terms>,
    accept_queued: bool,
    status: Status,
}
impl Controller {
    pub const fn new() -> Self {
        Self {
            session: None,
            bound: false,
            input: String::new(),
            output: String::new(),
            last_request_id: 0,
            queued: None,
            in_flight: None,
            discarding: false,
            terms: None,
            accept_queued: false,
            status: Status::SignedOut,
        }
    }
    pub const fn session(&self) -> Option<Session> {
        self.session
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub const fn status(&self) -> Status {
        self.status
    }
    pub const fn status_key(&self) -> &'static str {
        self.status.key()
    }
    pub fn terms(&self) -> Option<&Terms> {
        self.terms.as_ref()
    }
    pub fn is_busy(&self) -> bool {
        self.queued.is_some() || self.in_flight.is_some() || self.accept_queued
    }
    pub fn request_id(&self) -> Option<u64> {
        self.in_flight.or(self.queued).map(|r| r.id)
    }
    /// Desktop calls this only with readiness-confirmed live sessions. Failed
    /// binding is surfaced once and requires a fresh session to retry.
    pub fn sign_in(&mut self, session: &Session, services: &mut dyn Services) {
        if session.is_locked() || session.token() == 0 {
            self.lock_or_signout(services);
        } else if self.session != Some(*session) {
            self.lock_or_signout(services);
            self.session = Some(*session);
            match services.on_signed_in(session) {
                Ok(()) => {
                    self.bound = true;
                    self.status = Status::Ready;
                }
                Err(error) => {
                    self.bound = false;
                    self.status = Status::submit_error(error);
                }
            }
        }
    }
    /// Called before Desktop locks or erases its Session. Cancellation and
    /// invalidation precede replacement; IDs survive signout and are never reused.
    pub fn lock_or_signout(&mut self, services: &mut dyn Services) {
        if let Some(r) = self.in_flight.take() {
            let _ = services.cancel_request(&r.session, r.id);
        }
        self.queued = None;
        self.accept_queued = false;
        self.discarding = false;
        self.input = String::new();
        self.output = String::new();
        self.terms = None;
        self.bound = false;
        self.status = Status::SignedOut;
        if let Some(session) = self.session.take() {
            services.on_lock_or_signout(&session);
        }
    }
    fn can_edit(&mut self) -> bool {
        if self.session.is_none() {
            self.status = Status::SignedOut;
            false
        } else if !self.bound {
            false
        } else if self.is_busy() {
            self.status = Status::Busy;
            false
        } else {
            true
        }
    }
    pub fn set_input(&mut self, text: &str) -> bool {
        if !self.can_edit() {
            return false;
        }
        if text.len() > INPUT_CAPACITY {
            self.status = Status::InputFull;
            return false;
        }
        if text.contains('\0') {
            self.status = Status::InvalidInput;
            return false;
        }
        self.input.clear();
        self.input.push_str(text);
        self.status = Status::Ready;
        true
    }
    pub fn push_text(&mut self, text: &str) -> bool {
        if !self.can_edit() {
            return false;
        }
        if text.len() > INPUT_CAPACITY - self.input.len() {
            self.status = Status::InputFull;
            return false;
        }
        if text.contains('\0') {
            self.status = Status::InvalidInput;
            return false;
        }
        self.input.push_str(text);
        self.status = Status::Ready;
        true
    }
    pub fn backspace(&mut self) -> bool {
        if !self.can_edit() || self.input.is_empty() {
            return false;
        }
        self.input.pop();
        self.status = Status::Ready;
        true
    }
    pub fn request_submit(&mut self) -> bool {
        if !self.can_edit() {
            return false;
        }
        if self.input.trim().is_empty() {
            self.status = Status::EmptyInput;
            return false;
        }
        if self.terms.as_ref().is_some_and(|t| !t.acknowledged) {
            self.status = Status::TermsRequired;
            return false;
        }
        let Some(id) = self.last_request_id.checked_add(1) else {
            self.status = Status::Failed;
            return false;
        };
        let Some(session) = self.session else {
            return false;
        };
        self.last_request_id = id;
        self.queued = Some(Request { session, id });
        self.output.clear();
        self.status = Status::Queued;
        true
    }
    pub fn cancel(&mut self, services: &mut dyn Services) {
        if let Some(r) = self.in_flight {
            let _ = services.cancel_request(&r.session, r.id);
            self.discarding = true;
        }
        // PR43 retains the occupied slot until its cancellation result is polled.
        // Drain without publishing, even if the transport produces a late Ready.
        self.queued = None;
        self.accept_queued = false;
        self.output.clear();
        self.status = if self.session.is_some() {
            Status::Cancelled
        } else {
            Status::SignedOut
        };
    }
    /// Exact pinned OS metadata only. Offering never acknowledges or submits.
    pub fn offer_terms(&mut self, name: &str, reference: &str) -> bool {
        if !self.can_edit() {
            return false;
        }
        if name.trim().is_empty()
            || reference.trim().is_empty()
            || name.len() > TERMS_NAME_CAPACITY
            || reference.len() > TERMS_REFERENCE_CAPACITY
        {
            self.terms = None;
            self.status = Status::TermsFailed;
            return false;
        }
        self.terms = Some(Terms {
            name: name.into(),
            reference: reference.into(),
            acknowledged: false,
        });
        self.status = Status::TermsOffered;
        true
    }
    /// Explicit acceptance is ephemeral for this session. No persistence claim.
    pub fn accept_request(&mut self) -> bool {
        if !self.can_edit() || self.terms.as_ref().is_none_or(|t| t.acknowledged) {
            return false;
        }
        self.accept_queued = true;
        self.status = Status::TermsPending;
        true
    }
    /// At most one submit/ack/poll per step; sign-in is readiness-gated by Desktop.
    pub fn tick(&mut self, live: Option<&Session>, services: &mut dyn Services) {
        match live {
            Some(s) => self.sign_in(s, services),
            None => self.lock_or_signout(services),
        }
        let Some(session) = self.session.filter(|_| self.bound) else {
            return;
        };
        if self.accept_queued {
            self.accept_queued = false;
            if let Some(terms) = self.terms.as_mut() {
                terms.acknowledged = services
                    .acknowledge_model_terms(&session, &terms.reference)
                    .is_ok();
                self.status = if terms.acknowledged {
                    Status::TermsAccepted
                } else {
                    Status::TermsFailed
                };
            }
        } else if let Some(r) = self.queued.take() {
            match services.submit_text(&session, r.id, &self.input) {
                Ok(()) => {
                    self.in_flight = Some(r);
                    self.discarding = false;
                    self.status = Status::Waiting;
                }
                Err(e) => {
                    self.status = Status::submit_error(e);
                    if e == SubmitError::TermsRequired {
                        if let Some(t) = self.terms.as_mut() {
                            t.acknowledged = false;
                        }
                    }
                }
            }
        } else if let Some(r) = self.in_flight {
            let reply = services.poll_reply(&session, r.id);
            if matches!(reply, Ok(Reply::Pending)) {
                if !self.discarding {
                    self.status = Status::Waiting;
                }
                return;
            }
            self.in_flight = None;
            self.output.clear();
            if self.discarding {
                self.discarding = false;
                self.status = Status::Cancelled;
                return;
            }
            match reply {
                Ok(Reply::Ready(response)) => {
                    if r.session != session
                        || response.request_id != r.id
                        || response.text.trim().is_empty()
                        || response.text.contains('\0')
                        || response.text.len() > OUTPUT_CAPACITY
                        || response.usage.output_tokens > MAX_OUTPUT_TOKENS
                    {
                        let _ = services.cancel_request(&session, r.id);
                        self.status = Status::InvalidReply;
                    } else {
                        self.output = response.text;
                        self.status = Status::Complete;
                    }
                }
                Ok(Reply::Failed(ReplyError::Denied)) => self.status = Status::Denied,
                Ok(Reply::Failed(ReplyError::WorkerUnavailable)) => {
                    self.status = Status::WorkerUnavailable
                }
                Ok(Reply::Failed(ReplyError::Model(
                    nagi_model_manager::ModelServiceError::Store(
                        nagi_model_manager::StoreError::LicenseAcknowledgementRequired,
                    ),
                ))) => {
                    if let Some(terms) = self.terms.as_mut() {
                        terms.acknowledged = false;
                    }
                    self.status = Status::TermsRequired;
                }
                Ok(Reply::Failed(ReplyError::Model(_))) => self.status = Status::Failed,
                Err(e) => self.status = Status::submit_error(e),
                Ok(Reply::Pending) => unreachable!(),
            }
        }
    }
}
impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

/// One bounded desktop step always yields, including failed readiness, no input,
/// lock and no pending request. The production closure calls thread_yield.
pub fn idle_step(
    ui: &mut Controller,
    live: Option<&Session>,
    ready: bool,
    services: &mut dyn Services,
    yield_now: impl FnOnce(),
) {
    ui.tick(if ready { live } else { None }, services);
    yield_now();
}
