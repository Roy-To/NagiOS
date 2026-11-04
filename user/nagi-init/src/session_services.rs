//! OS-owned ordinary-session model bridge. Only bounded data crosses into the
//! guest worker; the native model service and its raw pointers stay there.
//! Model input never establishes caller identity or durable terms consent.

use alloc::{string::String, sync::Arc};
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use libnagi::security::Session;
use nagi_model_manager::{
    ModelManifest, ModelResponse, ModelServiceError, ResourceBudget, RuntimeError,
};

#[cfg(any(target_os = "nagi", test))]
use nagi_model_manager::CancellationToken;

pub const MAX_INPUT_BYTES: usize = 24 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 256;
#[cfg(target_os = "nagi")]
const WORKER_STACK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    Context(nagi_ai::PageSummaryError),
    Denied,
    Unavailable,
    InvalidRequestId,
    InvalidInput,
    InputTooLarge,
    Busy,
    TermsRequired,
    InvalidTerms,
    WorkerUnavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyError {
    Context(nagi_ai::PageSummaryError),
    Denied,
    Model(ModelServiceError),
    WorkerUnavailable,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplyProgress {
    Pending,
    Ready(ModelResponse),
    Failed(ReplyError),
}

// Critical sections only transfer bounded ownership. No model operation, disk
// read, or explicit yield runs while holding this lock. Contending Nagi callers
// cooperate rather than spinning forever on the single bootstrap scheduler.
struct BridgeLock<T> {
    held: AtomicBool,
    value: UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for BridgeLock<T> {}
unsafe impl<T: Send> Send for BridgeLock<T> {}
impl<T> BridgeLock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
    fn lock(&self) -> Guard<'_, T> {
        while self
            .held
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            cooperate();
        }
        Guard(self)
    }
}
struct Guard<'a, T>(&'a BridgeLock<T>);
impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.0.value.get() }
    }
}
impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.0.value.get() }
    }
}
impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.0.held.store(false, Ordering::Release);
    }
}
fn cooperate() {
    #[cfg(target_os = "nagi")]
    {
        let _ = libnagi::thread_yield();
    }
    #[cfg(not(target_os = "nagi"))]
    core::hint::spin_loop();
}

#[cfg_attr(not(any(target_os = "nagi", test)), allow(dead_code))]
struct Job {
    user: Session,
    generation: u64,
    request_id: u64,
    #[cfg_attr(not(target_os = "nagi"), allow(dead_code))]
    input: String,
    summary: Option<nagi_ai::PageSummaryPrompt>,
    cancellation: Arc<AtomicBool>,
}
#[derive(Clone)]
struct SummarySnapshot {
    request: nagi_ai::ContextRequest,
    prompt: nagi_ai::PageSummaryPrompt,
}
struct Slot {
    summary: Option<SummarySnapshot>,
    request_id: u64,
    cancellation: Arc<AtomicBool>,
}
struct State {
    user: Option<Session>,
    last_request_id: u64,
    terms_confirmed: bool,
    queued: Option<Job>,
    active: Option<Slot>,
    reply: Option<Result<ModelResponse, ReplyError>>,
}
impl State {
    fn new() -> Self {
        Self {
            user: None,
            last_request_id: 0,
            terms_confirmed: false,
            queued: None,
            active: None,
            reply: None,
        }
    }
    fn allows(&self, user: &Session) -> bool {
        !user.is_locked()
            && user.token() != 0
            && self.user.is_some_and(|bound| bound.token() == user.token())
    }
    fn invalidate(&mut self) {
        if let Some(slot) = self.active.take() {
            slot.cancellation.store(true, Ordering::Release);
        }
        self.user = None;
        self.last_request_id = 0;
        self.terms_confirmed = false;
        self.queued = None;
        self.reply = None;
    }
}
struct Shared {
    state: BridgeLock<State>,
    generation: AtomicU64,
    closed: AtomicBool,
    cleanup_failed: AtomicBool,
    #[cfg(target_os = "nagi")]
    finished: AtomicBool,
}
impl Shared {
    fn new() -> Self {
        Self {
            state: BridgeLock::new(State::new()),
            generation: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
            #[cfg(target_os = "nagi")]
            finished: AtomicBool::new(false),
        }
    }
    fn bind(&self, user: &Session) -> Result<(), SubmitError> {
        if user.is_locked() {
            self.unbind(user);
            return Err(SubmitError::Denied);
        }
        let mut state = self.state.lock();
        if self.cleanup_failed.load(Ordering::Acquire) {
            // Keep the quarantined owner bound so it can consume the failure.
            // A different valid owner revokes that generation before failing.
            if user.token() != 0
                && state
                    .user
                    .is_some_and(|bound| bound.token() != user.token())
            {
                self.invalidate_and_advance(&mut state);
            }
            return Err(SubmitError::WorkerUnavailable);
        }
        if user.token() == 0 || self.closed.load(Ordering::Acquire) {
            return Err(SubmitError::Denied);
        }
        if state.allows(user) {
            return Ok(());
        }
        let generation = self
            .generation
            .load(Ordering::Acquire)
            .checked_add(1)
            .ok_or(SubmitError::Unavailable)?;
        state.invalidate();
        self.generation.store(generation, Ordering::Release);
        state.user = Some(*user);
        Ok(())
    }
    fn invalidate_and_advance(&self, state: &mut State) {
        state.invalidate();
        if self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.closed.store(true, Ordering::Release);
        }
    }
    fn unbind(&self, user: &Session) {
        let mut state = self.state.lock();
        if state
            .user
            .is_some_and(|bound| bound.token() == user.token())
        {
            self.invalidate_and_advance(&mut state);
        }
    }
    fn submit(&self, user: &Session, request_id: u64, input: &str) -> Result<(), SubmitError> {
        if user.is_locked() {
            self.unbind(user);
            return Err(SubmitError::Denied);
        }
        if input.len() > MAX_INPUT_BYTES {
            return Err(SubmitError::InputTooLarge);
        }
        if input.trim().is_empty() || input.contains('\0') {
            return Err(SubmitError::InvalidInput);
        }
        self.enqueue(user, request_id, String::from(input), None, None)
    }
    fn enqueue(
        &self,
        user: &Session,
        request_id: u64,
        input: String,
        summary: Option<SummarySnapshot>,
        expected_generation: Option<u64>,
    ) -> Result<(), SubmitError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut state = self.state.lock();
        if self.cleanup_failed.load(Ordering::Acquire) {
            return Err(SubmitError::WorkerUnavailable);
        }
        if !state.allows(user)
            || self.closed.load(Ordering::Acquire)
            || expected_generation
                .is_some_and(|generation| self.generation.load(Ordering::Acquire) != generation)
        {
            return Err(SubmitError::Denied);
        }
        if request_id == 0 || request_id <= state.last_request_id {
            return Err(SubmitError::InvalidRequestId);
        }
        if state.active.is_some() {
            return Err(SubmitError::Busy);
        }
        if !state.terms_confirmed {
            return Err(SubmitError::TermsRequired);
        }
        if summary
            .as_ref()
            .is_some_and(|snapshot| !snapshot.prompt.page().has_current_revision())
        {
            return Err(SubmitError::Context(nagi_ai::PageSummaryError::Context(
                nagi_ai::BrowserContextError::ApiUnavailable,
            )));
        }
        state.last_request_id = request_id;
        let prompt = summary.as_ref().map(|snapshot| snapshot.prompt.clone());
        state.active = Some(Slot {
            summary,
            request_id,
            cancellation: cancel.clone(),
        });
        state.queued = Some(Job {
            user: *user,
            generation: self.generation.load(Ordering::Acquire),
            request_id,
            input,
            summary: prompt,
            cancellation: cancel,
        });
        Ok(())
    }
    fn poll(&self, user: &Session, request_id: u64) -> Result<ReplyProgress, SubmitError> {
        self.poll_checked(user, request_id, None)
    }
    fn poll_checked(
        &self,
        user: &Session,
        request_id: u64,
        checked: Option<(u64, &nagi_ai::PageSummaryPrompt)>,
    ) -> Result<ReplyProgress, SubmitError> {
        if user.is_locked() {
            self.unbind(user);
            return Err(SubmitError::Denied);
        }
        let mut state = self.state.lock();
        if !state.allows(user) || self.closed.load(Ordering::Acquire) {
            return Err(SubmitError::Denied);
        }
        if state
            .active
            .as_ref()
            .is_none_or(|slot| slot.request_id != request_id)
        {
            return Err(SubmitError::InvalidRequestId);
        }
        // Plaintext polling can never release summary text. Its owner must
        // check current context; only cancelled failures may drain without it.
        let slot = state.active.as_ref().ok_or(SubmitError::InvalidRequestId)?;
        if let Some(snapshot) = slot.summary.as_ref() {
            if let Some((generation, prompt)) = checked {
                if generation != self.generation.load(Ordering::Acquire)
                    || prompt != &snapshot.prompt
                {
                    return Err(SubmitError::Denied);
                }
            } else if !slot.cancellation.load(Ordering::Acquire) {
                return Err(SubmitError::Denied);
            }
            if !snapshot.prompt.page().has_current_revision() {
                let cancelled_before = slot.cancellation.swap(true, Ordering::AcqRel);
                state.queued = None;
                if !matches!(state.reply, Some(Err(_))) {
                    state.reply = Some(Err(if cancelled_before {
                        cancelled()
                    } else {
                        ReplyError::Denied
                    }));
                }
            }
            // A cancelled summary may drain without a Browser only as a
            // cancellation failure. Even a broken completion cannot leak text.
            if checked.is_none() && state.reply.is_some() {
                state.reply = Some(Err(cancelled()));
            }
        } else if checked.is_some() {
            return Err(SubmitError::Denied);
        }
        let Some(reply) = state.reply.take() else {
            return Ok(ReplyProgress::Pending);
        };
        state.active = None;
        Ok(match reply {
            Ok(response) => ReplyProgress::Ready(response),
            Err(error) => ReplyProgress::Failed(error),
        })
    }
    fn cancel(&self, user: &Session, request_id: u64) -> Result<(), SubmitError> {
        if user.is_locked() {
            self.unbind(user);
            return Err(SubmitError::Denied);
        }
        let mut state = self.state.lock();
        if !state.allows(user) {
            return Err(SubmitError::Denied);
        }
        let slot = state
            .active
            .as_ref()
            .filter(|slot| slot.request_id == request_id)
            .ok_or(SubmitError::InvalidRequestId)?;
        slot.cancellation.store(true, Ordering::Release);
        // Even a completed result must be suppressed when the user cancels
        // before consuming it. Keep the slot occupied until cancellation is read.
        if state.reply.is_some() {
            state.reply = Some(Err(cancelled()));
        }
        Ok(())
    }
    #[cfg(any(target_os = "nagi", test))]
    fn complete(&self, job: &Job, mut result: Result<ModelResponse, ReplyError>) {
        let mut state = self.state.lock();
        if self.closed.load(Ordering::Acquire)
            || self.generation.load(Ordering::Acquire) != job.generation
            || !state.allows(&job.user)
            || state
                .active
                .as_ref()
                .is_none_or(|slot| slot.request_id != job.request_id)
        {
            return;
        }
        if job.cancellation.load(Ordering::Acquire) {
            result = Err(cancelled());
        }
        if result.as_ref().is_ok_and(|r| {
            r.request_id != job.request_id
                || r.text.trim().is_empty()
                || r.text.contains('\0')
                || r.usage.output_tokens > MAX_OUTPUT_TOKENS
                || r.text.len()
                    > if job.summary.is_some() {
                        nagi_ai::MAX_PAGE_SUMMARY_BYTES
                    } else {
                        MAX_OUTPUT_BYTES
                    }
        }) {
            result = Err(ReplyError::Model(
                RuntimeError::InvalidBackendResponse.into(),
            ));
        }
        state.reply = Some(result);
    }
}
fn cancelled() -> ReplyError {
    ReplyError::Model(RuntimeError::Cancelled.into())
}
#[cfg(any(target_os = "nagi", test))]
struct RequestCancellation<'a> {
    shared: &'a Shared,
    generation: u64,
    cancelled: &'a AtomicBool,
    summary: Option<&'a nagi_ai::PageSummaryPrompt>,
}
#[cfg(any(target_os = "nagi", test))]
impl CancellationToken for RequestCancellation<'_> {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .summary
                .is_some_and(|prompt| !prompt.page().has_current_revision())
            || self.shared.closed.load(Ordering::Acquire)
            || self.shared.generation.load(Ordering::Acquire) != self.generation
    }
}

pub struct SessionServices {
    shared: Arc<Shared>,
    #[cfg(target_os = "nagi")]
    capability: u64,
    #[cfg(target_os = "nagi")]
    budget: ResourceBudget,
    terms_reference: String,
    #[cfg(target_os = "nagi")]
    worker: Option<WorkerHandle>,
}
impl SessionServices {
    /// Configuration only: no thread, artifact read, hash, or native context.
    /// The caller supplies an actual guest admission quota, not machine RAM.
    /// After dropping a bridge, yield in the UI idle/locked loop before retrying
    /// replacement: retired workers finish cooperatively, without a blocking join.
    pub fn new(model_store_capability: u64, budget: ResourceBudget) -> Result<Self, SubmitError> {
        if model_store_capability == 0 {
            return Err(SubmitError::Unavailable);
        }
        let manifest = ModelManifest::parse_json(include_bytes!(
            "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
        ))
        .map_err(|_| SubmitError::Unavailable)?;
        if budget.available_ram_bytes < manifest.resources.minimum_ram_bytes
            || budget.available_storage_bytes < manifest.resources.minimum_storage_bytes
            || budget.cpu_cores < manifest.resources.minimum_cpu_cores
            || (manifest.resources.gpu_required && !budget.gpu_available)
        {
            return Err(SubmitError::Unavailable);
        }
        let terms_reference = manifest
            .license
            .terms_reference
            .ok_or(SubmitError::Unavailable)?;
        #[cfg(target_os = "nagi")]
        {
            guest::claim_bridge()?;
            if let Err(error) = reap_retired() {
                guest::release_bridge();
                return Err(error);
            }
        }
        Ok(Self {
            shared: Arc::new(Shared::new()),
            #[cfg(target_os = "nagi")]
            capability: model_store_capability,
            #[cfg(target_os = "nagi")]
            budget,
            terms_reference,
            #[cfg(target_os = "nagi")]
            worker: None,
        })
    }
    /// Exact pinned terms reference for the Desktop consent UI to display and acknowledge.
    /// Reading it does not record consent.
    pub fn model_terms_reference(&self) -> &str {
        &self.terms_reference
    }

    /// Call only after ordinary authenticated Desktop readiness. Repeated calls
    /// for the same live Session preserve request IDs and explicit confirmation.
    pub fn on_signed_in(&mut self, user: &Session) -> Result<(), SubmitError> {
        self.shared.bind(user)?;
        #[cfg(target_os = "nagi")]
        if self.worker.is_none() {
            match spawn_worker(
                self.shared.clone(),
                self.capability,
                self.budget,
                self.terms_reference.clone(),
            ) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    self.shared.unbind(user);
                    return Err(error);
                }
            }
        }
        #[cfg(not(target_os = "nagi"))]
        {
            self.shared.unbind(user);
            Err(SubmitError::WorkerUnavailable)
        }
        #[cfg(target_os = "nagi")]
        Ok(())
    }
    pub fn submit_text(
        &mut self,
        user: &Session,
        request_id: u64,
        input: &str,
    ) -> Result<(), SubmitError> {
        self.shared.submit(user, request_id, input)
    }
    /// OS-owned entry only: the caller is obtained from trusted desktop app
    /// state, never prompt text. A logical identity alone is not OS provenance.
    /// Extraction stays on the Browser owner thread; only bounded untrusted data
    /// crosses the worker queue. No terms acknowledgement occurs here.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_page_summary(
        &mut self,
        user: &Session,
        trusted_caller: nagi_ai::CallerIdentity,
        request_id: u64,
        request: nagi_ai::ContextRequest,
        authority: &(impl nagi_ai::ContextAuthority + ?Sized),
        browser: &mut (impl nagi_ai::PublicBrowserContextApi + ?Sized),
    ) -> Result<(), SubmitError> {
        if user.is_locked() {
            self.shared.unbind(user);
            return Err(SubmitError::Denied);
        }
        let generation = {
            let state = self.shared.state.lock();
            if !state.allows(user)
                || self.shared.closed.load(Ordering::Acquire)
                || trusted_caller.app_id != nagi_ai::NAGI_BAR_APP_ID
                || request.caller != trusted_caller
            {
                return Err(SubmitError::Denied);
            }
            if self.shared.cleanup_failed.load(Ordering::Acquire) {
                return Err(SubmitError::WorkerUnavailable);
            }
            if request_id == 0 || request_id <= state.last_request_id {
                return Err(SubmitError::InvalidRequestId);
            }
            if state.active.is_some() {
                return Err(SubmitError::Busy);
            }
            if !state.terms_confirmed {
                return Err(SubmitError::TermsRequired);
            }
            self.shared.generation.load(Ordering::Acquire)
        };
        let prompt = nagi_ai::prepare_page_summary(request_id, request.clone(), authority, browser)
            .map_err(SubmitError::Context)?;
        self.shared.enqueue(
            user,
            request_id,
            String::new(),
            Some(SummarySnapshot { request, prompt }),
            Some(generation),
        )
    }

    /// Recheck sharing, navigation, page text and Context Authority on every UI
    /// poll, including pending inference. Failure cancels and discards this slot.
    /// No Browser references or unsafe Send implementations cross the queue.
    pub fn poll_page_summary(
        &mut self,
        user: &Session,
        request_id: u64,
        current_request: &nagi_ai::ContextRequest,
        authority: &(impl nagi_ai::ContextAuthority + ?Sized),
        browser: &mut (impl nagi_ai::PublicBrowserContextApi + ?Sized),
    ) -> Result<ReplyProgress, SubmitError> {
        if user.is_locked() {
            self.shared.unbind(user);
            return Err(SubmitError::Denied);
        }
        let (generation, snapshot) = {
            let state = self.shared.state.lock();
            if !state.allows(user) || self.shared.closed.load(Ordering::Acquire) {
                return Err(SubmitError::Denied);
            }
            let slot = state
                .active
                .as_ref()
                .filter(|slot| slot.request_id == request_id)
                .ok_or(SubmitError::InvalidRequestId)?;
            let snapshot = slot.summary.clone().ok_or(SubmitError::Denied)?;
            (self.shared.generation.load(Ordering::Acquire), snapshot)
        };
        // Schedule the worker before checking current owner state, never after
        // a successful recheck and before consuming its reply on this UI step.
        cooperate();
        // Compare the current owner request too: replaying the old selection
        // through authority cannot establish that the user still selected it.
        let fresh = (snapshot.request == *current_request).then(|| {
            nagi_ai::prepare_page_summary(request_id, snapshot.request, authority, browser)
        });
        if !fresh.as_ref().is_some_and(|result| {
            result.as_ref().is_ok_and(|current| {
                current.page().has_current_revision() && current == &snapshot.prompt
            })
        }) {
            let mut state = self.shared.state.lock();
            if !state.allows(user) || self.shared.generation.load(Ordering::Acquire) != generation {
                return Err(SubmitError::Denied);
            }
            let slot = state
                .active
                .as_ref()
                .filter(|slot| slot.request_id == request_id)
                .ok_or(SubmitError::InvalidRequestId)?;
            let explicitly_cancelled = slot.cancellation.swap(true, Ordering::AcqRel);
            state.queued = None;
            state.reply = Some(Err(if explicitly_cancelled {
                cancelled()
            } else {
                match fresh {
                    Some(Err(error)) => ReplyError::Context(error),
                    _ => ReplyError::Denied,
                }
            }));
        }
        self.shared
            .poll_checked(user, request_id, Some((generation, &snapshot.prompt)))
    }

    pub fn poll_reply(
        &mut self,
        user: &Session,
        request_id: u64,
    ) -> Result<ReplyProgress, SubmitError> {
        if user.is_locked() {
            self.shared.unbind(user);
            return Err(SubmitError::Denied);
        }
        // Polling an idle UI iteration explicitly schedules the real guest
        // worker. No host inference worker exists in non-Nagi builds.
        cooperate();
        self.shared.poll(user, request_id)
    }
    pub fn cancel_request(&mut self, user: &Session, request_id: u64) -> Result<(), SubmitError> {
        self.shared.cancel(user, request_id)
    }
    pub fn on_lock_or_signout(&mut self, user: &Session) {
        self.shared.unbind(user);
    }
    /// Explicit OS UI confirmation only. This records ephemeral per-session
    /// consent; Store-owned durable persistence is separate. The worker applies
    /// the exact pinned reference to its own service before the first request.
    pub fn acknowledge_model_terms(
        &mut self,
        user: &Session,
        pinned_reference: &str,
    ) -> Result<(), SubmitError> {
        if user.is_locked() {
            self.shared.unbind(user);
            return Err(SubmitError::Denied);
        }
        let mut state = self.shared.state.lock();
        if !state.allows(user) {
            return Err(SubmitError::Denied);
        }
        if pinned_reference != self.terms_reference {
            return Err(SubmitError::InvalidTerms);
        }
        state.terms_confirmed = true;
        Ok(())
    }
}
impl Drop for SessionServices {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.state.lock().invalidate();
        #[cfg(target_os = "nagi")]
        if let Some(worker) = self.worker.take() {
            // Never unmap a running stack or block UI teardown on a model read.
            // One retired worker bounds retained memory; new() reaps it after
            // the worker has unloaded/exited, or rejects a premature replacement.
            let mut retired = RETIRED.lock();
            assert!(retired.is_none());
            *retired = Some(worker);
        }
        #[cfg(target_os = "nagi")]
        guest::release_bridge();
    }
}

#[cfg(any(target_os = "nagi", test))]
trait WorkerBackend: Sized {
    fn bind(user: &Session, capability: u64, budget: ResourceBudget) -> Result<Self, ReplyError>;
    fn acknowledge(&mut self, user: &Session, reference: &str) -> Result<(), ReplyError>;
    fn generate(
        &mut self,
        job: &Job,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, ReplyError>;
    fn unload(&mut self) -> Result<(), ReplyError>;
}
#[cfg(any(target_os = "nagi", test))]
struct WorkerMachine<B> {
    generation: u64,
    service: Option<B>,
    error: Option<ReplyError>,
    cleanup_error: Option<ReplyError>,
}
#[cfg(any(target_os = "nagi", test))]
impl<B: WorkerBackend> WorkerMachine<B> {
    fn new() -> Self {
        Self {
            generation: 0,
            service: None,
            error: None,
            cleanup_error: None,
        }
    }
    fn unload(&mut self, shared: &Shared) {
        if let Some(mut service) = self.service.take() {
            if let Err(error) = service.unload() {
                self.error = Some(error);
                self.cleanup_error = Some(error);
                shared.cleanup_failed.store(true, Ordering::Release);
            }
        }
    }
    fn step(
        &mut self,
        shared: &Shared,
        capability: u64,
        budget: ResourceBudget,
        reference: &str,
    ) -> bool {
        let (generation, user, job) = {
            let mut state = shared.state.lock();
            (
                shared.generation.load(Ordering::Acquire),
                state.user,
                state.queued.take(),
            )
        };
        if shared.closed.load(Ordering::Acquire) {
            self.unload(shared);
            return false;
        }
        if generation != self.generation {
            self.error = self.cleanup_error;
            self.unload(shared);
            self.generation = generation;
            if let Some(user) = user {
                if self.error.is_none() {
                    match B::bind(&user, capability, budget) {
                        Ok(service) => self.service = Some(service),
                        Err(error) => self.error = Some(error),
                    }
                }
            }
        }
        if let Some(job) = job {
            let cancellation = RequestCancellation {
                shared,
                generation: job.generation,
                cancelled: &job.cancellation,
                summary: job.summary.as_ref(),
            };
            let result = if cancellation.is_cancelled() {
                Err(cancelled())
            } else if let Some(error) = self.error {
                Err(error)
            } else if let Some(service) = self.service.as_mut() {
                service
                    .acknowledge(&job.user, reference)
                    .and_then(|()| service.generate(&job, &cancellation))
            } else {
                Err(ReplyError::WorkerUnavailable)
            };
            #[cfg(target_os = "nagi")]
            guest::report_backend_result(&result);
            shared.complete(&job, result);
        }
        true
    }
}

#[cfg(target_os = "nagi")]
mod guest {
    use super::*;
    use crate::model_service::{SessionModelError, SessionModelService};
    use alloc::boxed::Box;
    use nagi_model_manager::{CapabilityId, GenerationOptions, ModelRequest, RoleId};

    // Fixed diagnostic codes only: never log prompts, generated text, Session
    // identities, request IDs or model paths. A backend result is not evidence
    // that the current UI generation accepted it; Shared still fences replies.
    pub(super) fn report_backend_result(result: &Result<ModelResponse, ReplyError>) {
        let code: &[u8] = match result {
            Ok(_) => b"ready",
            Err(ReplyError::Context(_)) => b"context-denied",
            Err(ReplyError::Denied) => b"denied",
            Err(ReplyError::WorkerUnavailable) => b"worker-unavailable",
            Err(ReplyError::Model(ModelServiceError::ResourceLimit)) => b"resource-limit",
            Err(ReplyError::Model(ModelServiceError::ModelUnavailable)) => b"model-unavailable",
            Err(ReplyError::Model(ModelServiceError::ClockUnavailable)) => b"clock-unavailable",
            Err(ReplyError::Model(ModelServiceError::Store(_))) => b"model-store",
            Err(ReplyError::Model(ModelServiceError::Runtime(error))) => match error {
                RuntimeError::ArtifactUnavailable => b"artifact-unavailable",
                RuntimeError::IntegrityRequired => b"integrity-required",
                RuntimeError::ArtifactMismatch => b"artifact-mismatch",
                RuntimeError::IntegrityMismatch => b"integrity-mismatch",
                RuntimeError::Cancelled => b"cancelled",
                RuntimeError::Timeout => b"timeout",
                RuntimeError::InvalidRequest => b"invalid-request",
                RuntimeError::BackendUnavailable => b"backend-unavailable",
                RuntimeError::LoadFailed => b"load-failed",
                RuntimeError::InferenceFailed => b"inference-failed",
                RuntimeError::UnloadFailed => b"unload-failed",
                RuntimeError::SessionClosed => b"session-closed",
                _ => b"runtime-error",
            },
        };
        let _ = libnagi::console_write(b"Nagi session worker backend-result=");
        let _ = libnagi::console_write(code);
        let _ = libnagi::console_write(b"\r\n");
    }
    impl From<SessionModelError> for ReplyError {
        fn from(value: SessionModelError) -> Self {
            match value {
                SessionModelError::Denied => Self::Denied,
                SessionModelError::Service(error) => Self::Model(error),
            }
        }
    }
    impl WorkerBackend for SessionModelService {
        fn bind(
            user: &Session,
            capability: u64,
            budget: ResourceBudget,
        ) -> Result<Self, ReplyError> {
            Self::new(user, capability, budget).map_err(Into::into)
        }
        fn acknowledge(&mut self, user: &Session, reference: &str) -> Result<(), ReplyError> {
            self.acknowledge_terms(user, reference).map_err(Into::into)
        }
        fn generate(
            &mut self,
            job: &Job,
            cancellation: &dyn CancellationToken,
        ) -> Result<ModelResponse, ReplyError> {
            if let Some(prompt) = job.summary.as_ref() {
                return self
                    .generate_page_summary(&job.user, prompt, cancellation)
                    .map_err(|error| match error {
                        crate::model_service::SessionPageSummaryError::Denied => ReplyError::Denied,
                        crate::model_service::SessionPageSummaryError::Cancelled => cancelled(),
                        crate::model_service::SessionPageSummaryError::Service(error) => {
                            ReplyError::Model(error)
                        }
                        crate::model_service::SessionPageSummaryError::Summary(error) => {
                            ReplyError::Context(error)
                        }
                    });
            }
            let capability =
                CapabilityId::new("text.generate").map_err(|_| ReplyError::WorkerUnavailable)?;
            let role = RoleId::new("standard").map_err(|_| ReplyError::WorkerUnavailable)?;
            let request = ModelRequest {
                request_id: job.request_id,
                caller: None,
                capability: &capability,
                system_prompt: None,
                input: &job.input,
                input_tokens: None,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                options: GenerationOptions::default(),
                timeout_millis: Some(15 * 60 * 1000),
                structured_output: None,
            };
            SessionModelService::generate(
                self,
                &job.user,
                Some(&role),
                None,
                &request,
                cancellation,
            )
            .map_err(Into::into)
        }
        fn unload(&mut self) -> Result<(), ReplyError> {
            SessionModelService::unload(self).map_err(Into::into)
        }
    }
    pub(super) struct WorkerHandle {
        thread: u64,
        joined: bool,
        stack: usize,
        shared: Arc<Shared>,
    }
    static BRIDGE_OWNED: AtomicBool = AtomicBool::new(false);
    // An unload error cannot establish that all native resources were freed.
    // Reap the owned thread/stack, then reject replacement for this process.
    static BRIDGE_QUARANTINED: AtomicBool = AtomicBool::new(false);
    pub(super) fn claim_bridge() -> Result<(), SubmitError> {
        BRIDGE_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| SubmitError::WorkerUnavailable)
    }
    pub(super) fn release_bridge() {
        BRIDGE_OWNED.store(false, Ordering::Release);
    }
    pub(super) static RETIRED: BridgeLock<Option<WorkerHandle>> = BridgeLock::new(None);
    struct Start {
        shared: Arc<Shared>,
        capability: u64,
        budget: ResourceBudget,
        reference: String,
    }
    #[unsafe(no_mangle)]
    extern "C" fn nagi_session_services_worker_entry(argument: usize) {
        // Unique Box ownership transfers only after successful native creation.
        let start = unsafe { Box::from_raw(argument as *mut Start) };
        let _ = libnagi::console_write(b"Nagi session worker started\r\n");
        let mut machine = WorkerMachine::<SessionModelService>::new();
        while machine.step(
            &start.shared,
            start.capability,
            start.budget,
            &start.reference,
        ) {
            cooperate();
        }
        machine.unload(&start.shared);
        let shared = start.shared.clone();
        if shared.cleanup_failed.load(Ordering::Acquire) {
            BRIDGE_QUARANTINED.store(true, Ordering::Release);
        }
        drop(start);
        drop(machine);
        shared.finished.store(true, Ordering::Release);
        drop(shared);
        libnagi::thread_exit(0);
    }
    pub(super) fn spawn_worker(
        shared: Arc<Shared>,
        capability: u64,
        budget: ResourceBudget,
        reference: String,
    ) -> Result<WorkerHandle, SubmitError> {
        let stack =
            libnagi::mmap_anonymous(WORKER_STACK_BYTES, libnagi::PROT_READ | libnagi::PROT_WRITE)
                .ok_or(SubmitError::WorkerUnavailable)?;
        let argument = Box::into_raw(Box::new(Start {
            shared: shared.clone(),
            capability,
            budget,
            reference,
        }));
        if let Some(thread) = libnagi::thread_create(
            nagi_session_services_worker_entry as usize,
            argument as usize,
            stack,
            WORKER_STACK_BYTES,
        ) {
            Ok(WorkerHandle {
                thread,
                joined: false,
                stack: stack as usize,
                shared,
            })
        } else {
            unsafe {
                drop(Box::from_raw(argument));
            }
            // This range is exclusively owned and has not been transferred to
            // a thread. The kernel's valid-owned-range unmap invariant applies.
            let _ = libnagi::munmap(stack, WORKER_STACK_BYTES);
            Err(SubmitError::WorkerUnavailable)
        }
    }
    pub(super) fn reap_retired() -> Result<(), SubmitError> {
        let worker = {
            let mut retired = RETIRED.lock();
            if retired
                .as_ref()
                .is_some_and(|w| !w.shared.finished.load(Ordering::Acquire))
            {
                return Err(SubmitError::WorkerUnavailable);
            }
            retired.take()
        };
        if let Some(mut worker) = worker {
            if !worker.joined {
                if libnagi::thread_join(worker.thread).is_none() {
                    *RETIRED.lock() = Some(worker);
                    return Err(SubmitError::WorkerUnavailable);
                }
                worker.joined = true;
            }
            if !libnagi::munmap(worker.stack as *mut u8, WORKER_STACK_BYTES) {
                *RETIRED.lock() = Some(worker);
                return Err(SubmitError::WorkerUnavailable);
            }
        }
        if BRIDGE_QUARANTINED.load(Ordering::Acquire) {
            Err(SubmitError::WorkerUnavailable)
        } else {
            Ok(())
        }
    }
}
#[cfg(target_os = "nagi")]
use guest::{reap_retired, spawn_worker, WorkerHandle, RETIRED};

static BOOT_MODEL_STORE: AtomicU64 = AtomicU64::new(0);
pub fn configure_boot(model_store_capability: u64) {
    BOOT_MODEL_STORE.store(model_store_capability, Ordering::Release);
}
pub fn boot_model_store_capability() -> u64 {
    BOOT_MODEL_STORE.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use core::sync::atomic::AtomicUsize;
    use libnagi::security::{AccountStore, Role};
    use nagi_model_manager::{BackendId, ModelId, ProviderId, TokenUsage};
    // Explicit orchestration-only backend. These tests never run host inference
    // or claim a synthetic text response is an ordinary guest result.
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static BINDS: AtomicUsize = AtomicUsize::new(0);
    static GENERATES: AtomicUsize = AtomicUsize::new(0);
    static UNLOADS: AtomicUsize = AtomicUsize::new(0);
    static MODE: AtomicUsize = AtomicUsize::new(0);
    struct FixtureBackend;
    impl WorkerBackend for FixtureBackend {
        fn bind(_: &Session, _: u64, _: ResourceBudget) -> Result<Self, ReplyError> {
            BINDS.fetch_add(1, Ordering::Relaxed);
            Ok(Self)
        }
        fn acknowledge(&mut self, _: &Session, _: &str) -> Result<(), ReplyError> {
            Ok(())
        }
        fn generate(
            &mut self,
            job: &Job,
            _: &dyn CancellationToken,
        ) -> Result<ModelResponse, ReplyError> {
            GENERATES.fetch_add(1, Ordering::Relaxed);
            match MODE.load(Ordering::Relaxed) {
                1 => Err(ReplyError::Model(RuntimeError::ArtifactUnavailable.into())),
                2 => Ok(response(job.request_id + 1, "orchestration fixture")),
                3 => Ok(response(job.request_id, &"x".repeat(MAX_OUTPUT_BYTES + 1))),
                5 => Err(ReplyError::Model(RuntimeError::ArtifactUnavailable.into())),
                6 => Ok(response(job.request_id, "fixture\0reply")),
                7 => {
                    let mut reply = response(job.request_id, "fixture");
                    reply.usage.output_tokens = MAX_OUTPUT_TOKENS + 1;
                    Ok(reply)
                }
                _ => Ok(response(
                    job.request_id,
                    if job.summary.is_some() {
                        "summary orchestration fixture"
                    } else {
                        &job.input
                    },
                )),
            }
        }
        fn unload(&mut self) -> Result<(), ReplyError> {
            UNLOADS.fetch_add(1, Ordering::Relaxed);
            if matches!(MODE.load(Ordering::Relaxed), 4 | 5) {
                Err(ReplyError::WorkerUnavailable)
            } else {
                Ok(())
            }
        }
    }
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        let g = TEST_SERIAL.lock().unwrap();
        BINDS.store(0, Ordering::Relaxed);
        GENERATES.store(0, Ordering::Relaxed);
        UNLOADS.store(0, Ordering::Relaxed);
        MODE.store(0, Ordering::Relaxed);
        g
    }
    /// Returns two independently authenticated session tokens for the same owner account.
    fn same_owner_sessions() -> (Session, Session) {
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"owner", Role::Owner, b"test only password")
            .unwrap();
        (
            accounts
                .authenticate(b"owner", b"test only password")
                .unwrap(),
            accounts
                .authenticate(b"owner", b"test only password")
                .unwrap(),
        )
    }
    fn budget() -> ResourceBudget {
        ResourceBudget {
            available_ram_bytes: 3 * 1024 * 1024 * 1024,
            available_storage_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 1,
            gpu_available: false,
        }
    }
    fn response(id: u64, text: &str) -> ModelResponse {
        ModelResponse {
            request_id: id,
            model_id: ModelId::new("fixture.model").unwrap(),
            provider_id: ProviderId::new("fixture.provider").unwrap(),
            backend_id: BackendId::new("fixture.backend").unwrap(),
            text: text.to_string(),
            usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
            },
        }
    }
    fn bridge(owner_session: &Session) -> SessionServices {
        let bridge = SessionServices::new(42, budget()).unwrap();
        bridge.shared.bind(owner_session).unwrap();
        bridge
    }
    fn confirm(bridge: &mut SessionServices, owner_session: &Session) {
        let reference = bridge.model_terms_reference().to_string();
        bridge
            .acknowledge_model_terms(owner_session, &reference)
            .unwrap();
    }
    fn step(worker: &mut WorkerMachine<FixtureBackend>, bridge: &SessionServices) -> bool {
        worker.step(&bridge.shared, 42, budget(), &bridge.terms_reference)
    }

    struct ContextOwner {
        denied: bool,
        body: &'static str,
        calls: usize,
        rebind: Option<(Arc<Shared>, Session)>,
        epoch: Option<nagi_ai::BrowserContextEpoch>,
    }
    impl nagi_ai::ContextAuthority for ContextOwner {
        fn can_read_object(&self, _: nagi_ai::CallerIdentity, _: nagi_model::ObjectId) -> bool {
            false
        }
    }
    impl nagi_ai::PublicBrowserContextApi for ContextOwner {
        fn current_page_context(
            &mut self,
            request: nagi_ai::BrowserContextRequest,
        ) -> Result<Option<nagi_ai::BrowserPageSnapshot>, nagi_ai::BrowserContextApiError> {
            self.calls += 1;
            assert_eq!(request.caller().app_id, nagi_ai::NAGI_BAR_APP_ID);
            if let Some((shared, user)) = self.rebind.take() {
                shared.unbind(&user);
                shared.bind(&user).unwrap();
            }
            if self.denied {
                return Err(nagi_ai::BrowserContextApiError::Denied);
            }
            let snapshot =
                nagi_ai::BrowserPageSnapshot::new(7, None, None, None, Some(self.body.into()));
            Ok(Some(match &self.epoch {
                Some(epoch) => snapshot.with_revision(epoch.revision()),
                None => snapshot,
            }))
        }
    }
    struct NoObjects;
    impl nagi_ai::ContextAuthority for NoObjects {
        fn can_read_object(&self, _: nagi_ai::CallerIdentity, _: nagi_model::ObjectId) -> bool {
            false
        }
    }
    fn summary_context() -> (nagi_ai::ContextRequest, ContextOwner) {
        (
            nagi_ai::ContextRequest {
                caller: nagi_ai::CallerIdentity {
                    app_id: nagi_ai::NAGI_BAR_APP_ID,
                    app_session_id: nagi_model::AppSessionId(8),
                    node_id: nagi_model::NodeId(9),
                    workspace_id: None,
                },
                selected_object: None,
                candidate_objects: alloc::vec::Vec::new(),
            },
            ContextOwner {
                denied: false,
                body: "untrusted page fixture",
                calls: 0,
                rebind: None,
                epoch: Some(nagi_ai::BrowserContextEpoch::new()),
            },
        )
    }
    #[test]
    fn unversioned_browser_cannot_queue_async_summary_or_consume_request_id() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        browser.epoch = None;
        assert_eq!(
            b.submit_page_summary(
                &user,
                request.caller,
                1,
                request.clone(),
                &NoObjects,
                &mut browser
            ),
            Err(SubmitError::Context(nagi_ai::PageSummaryError::Context(
                nagi_ai::BrowserContextError::ApiUnavailable,
            )))
        );
        assert!(b.shared.state.lock().active.is_none());
        assert!(b.shared.state.lock().queued.is_none());
        browser.epoch = Some(nagi_ai::BrowserContextEpoch::new());
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
    }

    #[test]
    fn page_and_sharing_aba_fence_queued_running_and_completed_replies() {
        for phase in 0..3 {
            for sharing in [false, true] {
                for explicit_cancel in [false, true] {
                    let (user, _) = same_owner_sessions();
                    let mut b = bridge(&user);
                    confirm(&mut b, &user);
                    let (request, mut browser) = summary_context();
                    b.submit_page_summary(
                        &user,
                        request.caller,
                        1,
                        request.clone(),
                        &NoObjects,
                        &mut browser,
                    )
                    .unwrap();
                    let old = if phase > 0 {
                        b.shared.state.lock().queued.take()
                    } else {
                        None
                    };
                    if phase == 2 {
                        b.shared
                            .complete(old.as_ref().unwrap(), Ok(response(1, "old page reply")));
                    }
                    // Both transitions occur between UI polls. All public
                    // fields have their original values at the next read.
                    browser.epoch.as_mut().unwrap().invalidate();
                    if sharing {
                        browser.denied = true;
                    } else {
                        browser.body = "B";
                    }
                    browser.epoch.as_mut().unwrap().invalidate();
                    browser.denied = false;
                    browser.body = "untrusted page fixture";
                    if explicit_cancel {
                        b.cancel_request(&user, 1).unwrap();
                    }
                    assert_eq!(
                        b.poll_page_summary(&user, 1, &request, &NoObjects, &mut browser),
                        Ok(ReplyProgress::Failed(if explicit_cancel {
                            cancelled()
                        } else {
                            ReplyError::Denied
                        }))
                    );
                    if let Some(old) = old.as_ref() {
                        assert!(old.cancellation.load(Ordering::Acquire));
                    }
                    b.submit_text(&user, 2, "new request").unwrap();
                    if let Some(old) = old.as_ref() {
                        b.shared.complete(old, Ok(response(1, "late old page")));
                    }
                    assert_eq!(b.poll_reply(&user, 2), Ok(ReplyProgress::Pending));
                }
            }
        }
    }

    #[test]
    fn revoked_epoch_reaches_worker_cancellation_before_next_ui_poll() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        let prompt = b
            .shared
            .state
            .lock()
            .active
            .as_ref()
            .unwrap()
            .summary
            .as_ref()
            .unwrap()
            .prompt
            .clone();
        let cancel = AtomicBool::new(false);
        let cancellation = RequestCancellation {
            shared: &b.shared,
            generation: b.shared.generation.load(Ordering::Acquire),
            cancelled: &cancel,
            summary: Some(&prompt),
        };
        assert!(!cancellation.is_cancelled());
        browser.epoch.as_mut().unwrap().invalidate();
        assert!(cancellation.is_cancelled());
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        assert!(step(&mut worker, &b));
        assert_eq!(GENERATES.load(Ordering::Relaxed), 0);
        assert_eq!(
            b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser),
            Ok(ReplyProgress::Failed(ReplyError::Denied))
        );
    }

    #[test]
    fn revoked_revision_cannot_release_success_through_checked_queue() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        b.shared.complete(&old, Ok(response(1, "revoked reply")));
        let prompt = old.summary.as_ref().unwrap();
        let generation = b.shared.generation.load(Ordering::Acquire);
        // Destroying the owner also revokes all retained snapshot clones.
        browser.epoch = None;
        assert_eq!(
            b.shared.poll_checked(&user, 1, Some((generation, prompt))),
            Ok(ReplyProgress::Failed(ReplyError::Denied))
        );
        assert!(old.cancellation.load(Ordering::Acquire));
    }

    #[test]
    fn summary_requires_terms_and_full_bar_identity_before_context_extraction() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        let (request, mut browser) = summary_context();
        assert_eq!(
            b.submit_page_summary(
                &user,
                request.caller,
                1,
                request.clone(),
                &NoObjects,
                &mut browser
            ),
            Err(SubmitError::TermsRequired)
        );
        confirm(&mut b, &user);
        let mut wrong = request.caller;
        wrong.app_session_id = nagi_model::AppSessionId(99);
        assert_eq!(
            b.submit_page_summary(&user, wrong, 1, request.clone(), &NoObjects, &mut browser),
            Err(SubmitError::Denied)
        );
        let mut wrong_app = request.clone();
        wrong_app.caller.app_id = nagi_model::AppId(999);
        assert_eq!(
            b.submit_page_summary(
                &user,
                wrong_app.caller,
                1,
                wrong_app,
                &NoObjects,
                &mut browser
            ),
            Err(SubmitError::Denied)
        );
        assert_eq!(browser.calls, 0);
        browser.denied = true;
        assert!(matches!(
            b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser),
            Err(SubmitError::Context(_))
        ));
        assert!(b.shared.state.lock().active.is_none());
    }
    #[test]
    fn summary_uses_real_queue_and_requires_fresh_context_even_after_completion() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        assert_eq!(b.poll_reply(&user, 1), Err(SubmitError::Denied));
        assert_eq!(
            b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser),
            Ok(ReplyProgress::Pending)
        );
        step(&mut WorkerMachine::<FixtureBackend>::new(), &b);
        assert_eq!(b.poll_reply(&user, 1), Err(SubmitError::Denied));
        assert!(
            matches!(b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser), Ok(ReplyProgress::Ready(reply)) if reply.request_id == 1 && reply.text == "summary orchestration fixture")
        );
        assert_eq!(browser.calls, 3);
    }
    #[test]
    fn summary_navigation_and_sharing_changes_cancel_pending_and_completed_results() {
        let _g = serial();
        for complete in [false, true] {
            for denied in [false, true] {
                let (user, _) = same_owner_sessions();
                let mut b = bridge(&user);
                confirm(&mut b, &user);
                let (request, mut browser) = summary_context();
                b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
                    .unwrap();
                let old = b.shared.state.lock().queued.take().unwrap();
                if complete {
                    b.shared.complete(&old, Ok(response(1, "late fixture")));
                }
                browser.denied = denied;
                browser.body = "new navigation fixture";
                assert!(matches!(
                    b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser),
                    Ok(ReplyProgress::Failed(_))
                ));
                assert!(old.cancellation.load(Ordering::Acquire));
                b.submit_text(&user, 2, "next fixture").unwrap();
                b.shared.complete(&old, Ok(response(1, "late fixture")));
                assert_eq!(b.poll_reply(&user, 2), Ok(ReplyProgress::Pending));
            }
        }
    }
    #[test]
    fn explicit_summary_cancel_precedes_context_failure() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        b.cancel_request(&user, 1).unwrap();
        browser.denied = true;
        assert_eq!(
            b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser),
            Ok(ReplyProgress::Failed(cancelled()))
        );
    }

    #[test]
    fn cancelled_summary_plaintext_drain_cannot_release_even_injected_success() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        assert_eq!(b.poll_reply(&user, 1), Err(SubmitError::Denied));
        b.cancel_request(&user, 1).unwrap();
        assert_eq!(b.poll_reply(&user, 1), Ok(ReplyProgress::Pending));
        // Deliberately bypass complete() in this host orchestration probe.
        b.shared.state.lock().reply = Some(Ok(response(1, "must never escape")));
        assert_eq!(
            b.poll_reply(&user, 1),
            Ok(ReplyProgress::Failed(cancelled()))
        );
    }

    #[test]
    fn summary_context_extraction_cannot_survive_generation_change() {
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        browser.rebind = Some((b.shared.clone(), user));
        assert_eq!(
            b.submit_page_summary(&user, request.caller, 1, request, &NoObjects, &mut browser),
            Err(SubmitError::Denied)
        );
        assert!(!b.shared.state.lock().terms_confirmed);
        assert!(b.shared.state.lock().active.is_none());
    }
    #[test]
    fn summary_lock_and_same_owner_new_session_fence_reused_request_id() {
        let _g = serial();
        let (user, other) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let (request, mut browser) = summary_context();
        b.submit_page_summary(
            &user,
            request.caller,
            1,
            request.clone(),
            &NoObjects,
            &mut browser,
        )
        .unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        b.on_lock_or_signout(&user);
        assert!(old.cancellation.load(Ordering::Acquire));
        b.shared.bind(&other).unwrap();
        confirm(&mut b, &other);
        b.submit_page_summary(&other, request.caller, 1, request, &NoObjects, &mut browser)
            .unwrap();
        b.shared
            .complete(&old, Ok(response(1, "old session fixture")));
        assert_eq!(
            b.poll_page_summary(&user, 1, &summary_context().0, &NoObjects, &mut browser),
            Err(SubmitError::Denied)
        );
        assert_eq!(
            b.poll_page_summary(&other, 1, &summary_context().0, &NoObjects, &mut browser),
            Ok(ReplyProgress::Pending)
        );
        step(&mut WorkerMachine::<FixtureBackend>::new(), &b);
        assert!(
            matches!(b.poll_page_summary(&other, 1, &summary_context().0, &NoObjects, &mut browser), Ok(ReplyProgress::Ready(reply)) if reply.text == "summary orchestration fixture")
        );
    }
    #[test]
    fn configuration_rejects_absent_store_and_insufficient_guest_budget() {
        assert!(matches!(
            SessionServices::new(0, budget()),
            Err(SubmitError::Unavailable)
        ));
        let mut low = budget();
        low.available_ram_bytes = 128 * 1024 * 1024;
        assert!(matches!(
            SessionServices::new(42, low),
            Err(SubmitError::Unavailable)
        ));
        let mut low = budget();
        low.available_storage_bytes = 1;
        assert!(matches!(
            SessionServices::new(42, low),
            Err(SubmitError::Unavailable)
        ));
    }
    #[test]
    fn no_host_worker_or_authority_is_created_by_configuration() {
        let (user, _) = same_owner_sessions();
        let mut b = SessionServices::new(42, budget()).unwrap();
        assert_eq!(b.submit_text(&user, 1, "test"), Err(SubmitError::Denied));
        assert_eq!(b.on_signed_in(&user), Err(SubmitError::WorkerUnavailable));
        assert_eq!(b.submit_text(&user, 1, "test"), Err(SubmitError::Denied));
    }
    #[test]
    fn terms_and_request_bounds_require_explicit_bound_session() {
        let (user, other) = same_owner_sessions();
        let mut b = bridge(&user);
        assert_eq!(
            b.submit_text(&user, 1, "test"),
            Err(SubmitError::TermsRequired)
        );
        assert_eq!(
            b.acknowledge_model_terms(&other, &b.terms_reference.clone()),
            Err(SubmitError::Denied)
        );
        assert_eq!(
            b.acknowledge_model_terms(&user, "terms in generated input"),
            Err(SubmitError::InvalidTerms)
        );
        confirm(&mut b, &user);
        assert_eq!(
            b.submit_text(&user, 0, "test"),
            Err(SubmitError::InvalidRequestId)
        );
        assert_eq!(
            b.submit_text(&user, 1, "\0"),
            Err(SubmitError::InvalidInput)
        );
        assert_eq!(
            b.submit_text(&user, 1, &"x".repeat(MAX_INPUT_BYTES + 1)),
            Err(SubmitError::InputTooLarge)
        );
        assert_eq!(b.submit_text(&user, 1, " 日本語 "), Ok(()));
        assert_eq!(b.submit_text(&user, 2, "second"), Err(SubmitError::Busy));
    }
    #[test]
    fn one_slot_remains_occupied_until_completion_is_consumed() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        b.submit_text(&user, 1, "bounded orchestration fixture")
            .unwrap();
        assert!(step(&mut worker, &b));
        assert_eq!(b.submit_text(&user, 2, "second"), Err(SubmitError::Busy));
        assert!(matches!(
            b.poll_reply(&user, 1),
            Ok(ReplyProgress::Ready(_))
        ));
        assert_eq!(
            b.submit_text(&user, 1, "replay"),
            Err(SubmitError::InvalidRequestId)
        );
        assert_eq!(b.submit_text(&user, 2, "second"), Ok(()));
    }
    #[test]
    fn artifact_unavailable_is_an_error_not_a_reply() {
        let _g = serial();
        MODE.store(1, Ordering::Relaxed);
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        b.submit_text(&user, 1, "fixture request").unwrap();
        step(&mut WorkerMachine::<FixtureBackend>::new(), &b);
        assert_eq!(
            b.poll_reply(&user, 1),
            Ok(ReplyProgress::Failed(ReplyError::Model(
                RuntimeError::ArtifactUnavailable.into()
            )))
        );
    }
    #[test]
    fn wrong_request_and_oversized_responses_are_rejected() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        for mode in [2, 3, 6, 7] {
            MODE.store(mode, Ordering::Relaxed);
            b.submit_text(&user, mode as u64, "fixture request")
                .unwrap();
            step(&mut worker, &b);
            assert_eq!(
                b.poll_reply(&user, mode as u64),
                Ok(ReplyProgress::Failed(ReplyError::Model(
                    RuntimeError::InvalidBackendResponse.into()
                )))
            );
        }
    }
    #[test]
    fn cancel_before_start_skips_generation_and_completed_cancel_hides_reply() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        b.submit_text(&user, 1, "first").unwrap();
        b.cancel_request(&user, 1).unwrap();
        step(&mut worker, &b);
        assert_eq!(GENERATES.load(Ordering::Relaxed), 0);
        assert_eq!(
            b.poll_reply(&user, 1),
            Ok(ReplyProgress::Failed(cancelled()))
        );
        b.submit_text(&user, 2, "second").unwrap();
        step(&mut worker, &b);
        b.cancel_request(&user, 2).unwrap();
        assert_eq!(
            b.poll_reply(&user, 2),
            Ok(ReplyProgress::Failed(cancelled()))
        );
    }
    #[test]
    fn session_change_cancels_old_job_and_late_output_cannot_replace_new_reply() {
        let _g = serial();
        let (user, other) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        b.submit_text(&user, 1, "old").unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        let shared = b.shared.clone();
        let cancellation = RequestCancellation {
            shared: &shared,
            generation: old.generation,
            cancelled: &old.cancellation,
            summary: old.summary.as_ref(),
        };
        b.shared.bind(&other).unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            b.submit_text(&other, 1, "new"),
            Err(SubmitError::TermsRequired)
        );
        confirm(&mut b, &other);
        b.submit_text(&other, 1, "new").unwrap();
        step(&mut worker, &b);
        assert!(cancellation.is_cancelled());
        b.shared.complete(&old, Ok(response(1, "late old")));
        assert_eq!(b.poll_reply(&user, 1), Err(SubmitError::Denied));
        assert!(matches!(b.poll_reply(&other,1),Ok(ReplyProgress::Ready(r)) if r.text=="new"));
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 1);
        assert_eq!(BINDS.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn lock_and_close_cancel_then_unload_without_publishing_output() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        b.submit_text(&user, 1, "old").unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        b.on_lock_or_signout(&user);
        step(&mut worker, &b);
        b.shared.complete(&old, Ok(response(1, "late")));
        assert_eq!(b.poll_reply(&user, 1), Err(SubmitError::Denied));
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 1);
        b.shared.bind(&user).unwrap();
        step(&mut worker, &b);
        b.shared.closed.store(true, Ordering::Release);
        assert!(!step(&mut worker, &b));
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn locked_submit_revokes_worker_and_suppresses_inflight_reply() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        b.submit_text(&user, 1, "old in flight").unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        let generation = b.shared.generation.load(Ordering::Acquire);
        let mut locked = user;
        locked.lock();

        assert_eq!(
            b.submit_text(&locked, 2, "must not enter queue"),
            Err(SubmitError::Denied)
        );
        assert!(old.cancellation.load(Ordering::Acquire));
        assert_eq!(b.shared.generation.load(Ordering::Acquire), generation + 1);
        assert!(b.shared.state.lock().user.is_none());
        step(&mut worker, &b);
        b.shared.complete(&old, Ok(response(1, "late")));
        assert!(b.shared.state.lock().reply.is_none());
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn locked_cancel_revokes_worker_and_suppresses_inflight_reply() {
        let _g = serial();
        let (user, _) = same_owner_sessions();
        let mut b = bridge(&user);
        confirm(&mut b, &user);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        b.submit_text(&user, 1, "old in flight").unwrap();
        let old = b.shared.state.lock().queued.take().unwrap();
        let generation = b.shared.generation.load(Ordering::Acquire);
        let mut locked = user;
        locked.lock();

        assert_eq!(b.cancel_request(&locked, 1), Err(SubmitError::Denied));
        assert!(old.cancellation.load(Ordering::Acquire));
        assert_eq!(b.shared.generation.load(Ordering::Acquire), generation + 1);
        assert!(b.shared.state.lock().user.is_none());
        step(&mut worker, &b);
        b.shared.complete(&old, Ok(response(1, "late")));
        assert!(b.shared.state.lock().reply.is_none());
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_submitters_cannot_expand_the_single_request_slot() {
        let (user, _) = same_owner_sessions();
        let shared = Arc::new(Shared::new());
        shared.bind(&user).unwrap();
        shared.state.lock().terms_confirmed = true;
        let accepted = std::thread::scope(|scope| {
            let mut threads = alloc::vec::Vec::new();
            for id in 1..=8 {
                let shared = shared.clone();
                threads.push(
                    scope.spawn(move || shared.submit(&user, id, "bounded test data").is_ok()),
                );
            }
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|accepted| *accepted)
                .count()
        });
        assert_eq!(accepted, 1);
        let state = shared.state.lock();
        assert!(state.active.is_some());
        assert!(state.queued.is_some());
        assert!(state.reply.is_none());
    }
    #[test]
    fn generate_failure_then_unload_failure_quarantines_new_generation() {
        let _g = serial();
        let (initial_session, alternate_session) = same_owner_sessions();
        let mut b = bridge(&initial_session);
        confirm(&mut b, &initial_session);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        MODE.store(5, Ordering::Relaxed);
        b.submit_text(&initial_session, 1, "generation fails")
            .unwrap();
        step(&mut worker, &b);
        assert!(!b.shared.cleanup_failed.load(Ordering::Acquire));
        assert_eq!(
            b.poll_reply(&initial_session, 1),
            Ok(ReplyProgress::Failed(ReplyError::Model(
                RuntimeError::ArtifactUnavailable.into()
            )))
        );

        b.shared.bind(&alternate_session).unwrap();
        confirm(&mut b, &alternate_session);
        b.submit_text(&alternate_session, 1, "new generation")
            .unwrap();
        step(&mut worker, &b);
        assert_eq!(BINDS.load(Ordering::Relaxed), 1);
        assert_eq!(UNLOADS.load(Ordering::Relaxed), 1);
        assert!(b.shared.cleanup_failed.load(Ordering::Acquire));
        assert_eq!(worker.cleanup_error, Some(ReplyError::WorkerUnavailable));

        let generation_before_locked_bind = b.shared.generation.load(Ordering::Acquire);
        let mut locked_session = alternate_session;
        locked_session.lock();
        assert_eq!(b.on_signed_in(&locked_session), Err(SubmitError::Denied));
        assert_eq!(
            b.shared.generation.load(Ordering::Acquire),
            generation_before_locked_bind + 1
        );
        {
            let state = b.shared.state.lock();
            assert!(state.user.is_none());
            assert!(!state.terms_confirmed);
            assert!(state.active.is_none());
            assert!(state.queued.is_none());
            assert!(state.reply.is_none());
        }
        assert!(b.shared.cleanup_failed.load(Ordering::Acquire));
        assert_eq!(
            b.poll_reply(&alternate_session, 1),
            Err(SubmitError::Denied)
        );
        assert_eq!(
            b.submit_text(&alternate_session, 2, "quarantined"),
            Err(SubmitError::WorkerUnavailable)
        );
    }

    fn fail_unload_after_alternate_session_switch() -> (SessionServices, Session, Session, Job) {
        let (initial_session, alternate_session) = same_owner_sessions();
        let mut b = bridge(&initial_session);
        confirm(&mut b, &initial_session);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);

        b.shared.bind(&alternate_session).unwrap();
        confirm(&mut b, &alternate_session);
        b.submit_text(&alternate_session, 1, "cleanup failure")
            .unwrap();
        let late = {
            let state = b.shared.state.lock();
            let job = state.queued.as_ref().unwrap();
            Job {
                user: job.user,
                generation: job.generation,
                request_id: job.request_id,
                input: job.input.clone(),
                summary: job.summary.clone(),
                cancellation: job.cancellation.clone(),
            }
        };
        MODE.store(4, Ordering::Relaxed);
        step(&mut worker, &b);
        assert!(b.shared.cleanup_failed.load(Ordering::Acquire));
        assert_eq!(worker.cleanup_error, Some(ReplyError::WorkerUnavailable));
        // Return the quarantined bound session before its alternate session token.
        (b, alternate_session, initial_session, late)
    }

    #[test]
    fn same_owner_quarantine_keeps_failure_reply_pollable() {
        let _g = serial();
        let (mut b, quarantined_session, _, _) = fail_unload_after_alternate_session_switch();
        let generation = b.shared.generation.load(Ordering::Acquire);
        assert_eq!(
            b.on_signed_in(&quarantined_session),
            Err(SubmitError::WorkerUnavailable)
        );
        assert_eq!(b.shared.generation.load(Ordering::Acquire), generation);
        assert!(b
            .shared
            .state
            .lock()
            .user
            .is_some_and(|bound| bound.token() == quarantined_session.token()));
        assert_eq!(
            b.poll_reply(&quarantined_session, 1),
            Ok(ReplyProgress::Failed(ReplyError::WorkerUnavailable))
        );
    }

    #[test]
    fn quarantine_session_switch_revokes_old_access_and_fences_late_completion() {
        let _g = serial();
        let (mut b, quarantined_session, alternate_session, late) =
            fail_unload_after_alternate_session_switch();
        assert_eq!(
            b.on_signed_in(&quarantined_session),
            Err(SubmitError::WorkerUnavailable)
        );
        let old_generation = b.shared.generation.load(Ordering::Acquire);
        {
            let state = b.shared.state.lock();
            assert_eq!(
                state.user.map(|bound| bound.token()),
                Some(quarantined_session.token())
            );
            assert!(state.terms_confirmed);
            assert!(matches!(
                state.reply.as_ref(),
                Some(Err(ReplyError::WorkerUnavailable))
            ));
            assert!(state.active.is_some());
        }

        assert_eq!(
            b.on_signed_in(&alternate_session),
            Err(SubmitError::WorkerUnavailable)
        );
        assert_eq!(
            b.shared.generation.load(Ordering::Acquire),
            old_generation + 1
        );
        assert!(late.cancellation.load(Ordering::Acquire));
        {
            let state = b.shared.state.lock();
            assert!(state.user.is_none());
            assert!(!state.terms_confirmed);
            assert!(state.active.is_none());
            assert!(state.queued.is_none());
            assert!(state.reply.is_none());
        }
        assert_eq!(
            b.poll_reply(&quarantined_session, 1),
            Err(SubmitError::Denied)
        );
        assert_eq!(
            b.cancel_request(&quarantined_session, 1),
            Err(SubmitError::Denied)
        );
        let reference = b.model_terms_reference().to_string();
        assert_eq!(
            b.acknowledge_model_terms(&quarantined_session, &reference),
            Err(SubmitError::Denied)
        );
        b.shared
            .complete(&late, Ok(response(1, "late quarantined reply")));
        assert!(b.shared.state.lock().reply.is_none());
        assert_eq!(
            b.submit_text(&alternate_session, 2, "still quarantined"),
            Err(SubmitError::WorkerUnavailable)
        );
    }

    #[test]
    fn unload_failure_quarantines_later_generations_and_new_submissions() {
        let _g = serial();
        let (initial_session, alternate_session) = same_owner_sessions();
        let mut b = bridge(&initial_session);
        confirm(&mut b, &initial_session);
        let mut worker = WorkerMachine::<FixtureBackend>::new();
        step(&mut worker, &b);
        MODE.store(4, Ordering::Relaxed);
        b.shared.bind(&alternate_session).unwrap();
        confirm(&mut b, &alternate_session);
        b.submit_text(&alternate_session, 1, "new").unwrap();
        step(&mut worker, &b);
        assert_eq!(BINDS.load(Ordering::Relaxed), 1);
        assert_eq!(
            b.poll_reply(&alternate_session, 1),
            Ok(ReplyProgress::Failed(ReplyError::WorkerUnavailable))
        );
        MODE.store(0, Ordering::Relaxed);
        b.on_lock_or_signout(&alternate_session);
        assert_eq!(
            b.shared.bind(&initial_session),
            Err(SubmitError::WorkerUnavailable)
        );
        step(&mut worker, &b);
        assert_eq!(BINDS.load(Ordering::Relaxed), 1);
        assert!(b.shared.cleanup_failed.load(Ordering::Acquire));
        assert_eq!(worker.cleanup_error, Some(ReplyError::WorkerUnavailable));
        assert_eq!(
            b.submit_text(&alternate_session, 2, "quarantined"),
            Err(SubmitError::WorkerUnavailable)
        );
    }
}
