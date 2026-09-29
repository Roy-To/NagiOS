use crate::{
    DeterministicClock, DiagnosticKind, DiagnosticsRecorder, Failpoint, FailurePlan, HarnessError,
    PrincipalId, ResourceBudget, ResourceKind, Result, ServiceId,
};
use std::collections::{BTreeSet, VecDeque};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusFault {
    DropNext,
    DuplicateNext,
    ReorderNextToFront,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusMessage {
    pub sender: Option<PrincipalId>,
    pub receiver: ServiceId,
    pub correlation_id: String,
    pub protocol_version: u16,
    pub deadline: u64,
    pub payload: Vec<u8>,
}

impl BusMessage {
    pub fn new(
        sender: PrincipalId,
        receiver: ServiceId,
        correlation_id: impl Into<String>,
        deadline: u64,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            sender: Some(sender),
            receiver,
            correlation_id: correlation_id.into(),
            protocol_version: 1,
            deadline,
            payload,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BusOutcome {
    Queued { message_id: u64, copies: usize },
    Dropped,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct FakeServiceBus {
    inner: Arc<Mutex<BusState>>,
    allowed_routes: Arc<Mutex<BTreeSet<(PrincipalId, ServiceId)>>>,
    budget: ResourceBudget,
    failures: FailurePlan,
    clock: DeterministicClock,
    queue_capacity: usize,
    diagnostics: Option<DiagnosticsRecorder>,
}

struct BusState {
    next_message_id: u64,
    queue: VecDeque<QueuedMessage>,
    failed_providers: BTreeSet<ServiceId>,
    next_fault: Option<BusFault>,
}

struct QueuedMessage {
    message: BusMessage,
    _message_lease: crate::resources::ResourceLease,
    _bytes_lease: crate::resources::ResourceLease,
}

pub struct ReceivedMessage {
    message: BusMessage,
    _message_lease: crate::resources::ResourceLease,
    _bytes_lease: crate::resources::ResourceLease,
}

impl Deref for ReceivedMessage {
    type Target = BusMessage;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl FakeServiceBus {
    pub fn new(
        budget: ResourceBudget,
        failures: FailurePlan,
        clock: DeterministicClock,
        queue_capacity: usize,
        diagnostics: Option<DiagnosticsRecorder>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BusState {
                next_message_id: 1,
                queue: VecDeque::new(),
                failed_providers: BTreeSet::new(),
                next_fault: None,
            })),
            allowed_routes: Arc::new(Mutex::new(BTreeSet::new())),
            budget,
            failures,
            clock,
            queue_capacity,
            diagnostics,
        }
    }

    pub fn allow_route(&self, sender: PrincipalId, receiver: ServiceId) -> Result<()> {
        let mut routes = self
            .allowed_routes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let route = (sender, receiver);
        if !routes.contains(&route) && routes.len() >= 512 {
            return Err(HarnessError::QuotaExceeded(ResourceKind::Handles));
        }
        routes.insert(route);
        Ok(())
    }

    pub fn set_next_fault(&self, fault: BusFault) {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .next_fault = Some(fault);
    }

    pub fn fail_provider(&self, service: ServiceId) {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .failed_providers
            .insert(service);
    }

    pub fn recover_provider(&self, service: &ServiceId) {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .failed_providers
            .remove(service);
    }

    pub fn send(
        &self,
        message: BusMessage,
        cancellation: Option<&CancellationToken>,
    ) -> Result<BusOutcome> {
        if cancellation.is_some_and(CancellationToken::is_cancelled)
            || self.failures.trip(Failpoint::Cancellation)
        {
            self.record_failure(&message, "cancelled");
            return Err(HarnessError::Cancelled);
        }
        let Some(sender) = message.sender.as_ref() else {
            self.record_failure(&message, "missing-context");
            return Err(HarnessError::MissingContext);
        };
        if !self
            .allowed_routes
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(&(sender.clone(), message.receiver.clone()))
        {
            self.record_failure(&message, "route-denied");
            return Err(HarnessError::Unauthorized);
        }
        if message.protocol_version != 1
            || message.correlation_id.is_empty()
            || message.correlation_id.len() > 96
            || !message
                .correlation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || self.failures.trip(Failpoint::MalformedMessage)
        {
            self.record_failure(&message, "malformed");
            return Err(HarnessError::MalformedMessage);
        }
        if self.clock.now() >= message.deadline || self.failures.trip(Failpoint::Timeout) {
            self.record_failure(&message, "timeout");
            return Err(HarnessError::DeadlineExpired);
        }

        let (provider_failed, fault, current_len, first_id) = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let provider_failed = state.failed_providers.contains(&message.receiver);
            if !provider_failed {
                let fault = state.next_fault.take();
                (false, fault, state.queue.len(), state.next_message_id)
            } else {
                (true, None, state.queue.len(), state.next_message_id)
            }
        };
        if provider_failed || self.failures.trip(Failpoint::ProviderFailure) {
            self.record_failure(&message, "provider-failed");
            return Err(HarnessError::ProviderFailed);
        }
        if fault == Some(BusFault::DropNext) {
            self.record_failure(&message, "dropped-by-fault");
            return Ok(BusOutcome::Dropped);
        }

        let copies = if fault == Some(BusFault::DuplicateNext) {
            2
        } else {
            1
        };
        if current_len.saturating_add(copies) > self.queue_capacity {
            self.record_failure(&message, "queue-full");
            return Err(HarnessError::QueueFull);
        }
        let mut leases = Vec::with_capacity(copies);
        for _ in 0..copies {
            let message_lease = self.budget.acquire(ResourceKind::Messages, 1)?;
            let bytes_lease = self
                .budget
                .acquire(ResourceKind::Bytes, message.payload.len() as u64)?;
            leases.push((message_lease, bytes_lease));
        }

        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        // Recheck queue bounds after acquiring leases in case another test thread sent concurrently.
        if state.queue.len().saturating_add(copies) > self.queue_capacity {
            self.record_failure(&message, "queue-full");
            return Err(HarnessError::QueueFull);
        }
        let reorder = fault == Some(BusFault::ReorderNextToFront);
        let mut ids = Vec::with_capacity(copies);
        for (index, (message_lease, bytes_lease)) in leases.into_iter().enumerate() {
            let id = state.next_message_id;
            state.next_message_id = state.next_message_id.saturating_add(1);
            let queued = QueuedMessage {
                message: message.clone(),
                _message_lease: message_lease,
                _bytes_lease: bytes_lease,
            };
            if reorder {
                state.queue.push_front(queued);
            } else {
                state.queue.push_back(queued);
            }
            ids.push(id);
            let _ = index;
        }
        let first_id = ids.first().copied().unwrap_or(first_id);
        drop(state);
        self.record_success(&message, copies);
        Ok(BusOutcome::Queued {
            message_id: first_id,
            copies,
        })
    }

    pub fn receive(&self, receiver: &ServiceId) -> Option<ReceivedMessage> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let index = state
            .queue
            .iter()
            .position(|queued| &queued.message.receiver == receiver)?;
        state.queue.remove(index).map(|queued| {
            let QueuedMessage {
                message,
                _message_lease,
                _bytes_lease,
                ..
            } = queued;
            ReceivedMessage {
                message,
                _message_lease,
                _bytes_lease,
            }
        })
    }

    pub fn cancel_correlation(&self, correlation_id: &str) -> usize {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let before = state.queue.len();
        state
            .queue
            .retain(|queued| queued.message.correlation_id != correlation_id);
        before - state.queue.len()
    }

    pub fn queued(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .queue
            .len()
    }

    fn record_failure(&self, message: &BusMessage, reason: &str) {
        if let Some(diagnostics) = &self.diagnostics {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("receiver".to_owned(), message.receiver.to_string());
            fields.insert("reason".to_owned(), reason.to_owned());
            diagnostics.record(
                DiagnosticKind::IpcFailure,
                Some(message.correlation_id.clone()),
                "send",
                fields,
            );
        }
    }

    fn record_success(&self, message: &BusMessage, copies: usize) {
        if let Some(diagnostics) = &self.diagnostics {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("copies".to_owned(), copies.to_string());
            fields.insert("receiver".to_owned(), message.receiver.to_string());
            diagnostics.record(
                DiagnosticKind::IpcDelivered,
                Some(message.correlation_id.clone()),
                "send",
                fields,
            );
        }
    }
}
