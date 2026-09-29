use crate::{DeterministicClock, ResourceBudget, ResourceKind};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    TestStarted,
    TestEnded,
    TimeAdvanced,
    ServiceLifecycle,
    IpcDelivered,
    IpcFailure,
    AuthorizationDecision,
    FailpointActivated,
    Timeout,
    Cancelled,
    QuotaExceeded,
    OfflineNetworkDenied,
    ResourceLeak,
    Custom(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub test_id: String,
    pub correlation_id: Option<String>,
    pub virtual_time: u64,
    pub kind: DiagnosticKind,
    pub stage: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct DiagnosticsRecorder {
    inner: Arc<Mutex<RecorderState>>,
    clock: DeterministicClock,
    budget: ResourceBudget,
    test_id: String,
    capacity: usize,
}

struct RetainedEvent {
    event: DiagnosticEvent,
    _lease: crate::resources::ResourceLease,
}

struct RecorderState {
    next_sequence: u64,
    events: VecDeque<RetainedEvent>,
    dropped: u64,
    enabled: bool,
}

impl DiagnosticsRecorder {
    pub fn new(
        clock: DeterministicClock,
        budget: ResourceBudget,
        test_id: impl Into<String>,
        capacity: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecorderState {
                next_sequence: 1,
                events: VecDeque::new(),
                dropped: 0,
                enabled: true,
            })),
            clock,
            budget,
            test_id: test_id.into(),
            capacity,
        }
    }

    /// Capture is best effort: sink failure or quota exhaustion never changes the caller's result.
    pub fn record(
        &self,
        kind: DiagnosticKind,
        correlation_id: Option<String>,
        stage: impl Into<String>,
        fields: BTreeMap<String, String>,
    ) -> bool {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !state.enabled || state.events.len() >= self.capacity {
            state.dropped = state.dropped.saturating_add(1);
            return false;
        }
        let Ok(lease) = self.budget.acquire(ResourceKind::Events, 1) else {
            state.dropped = state.dropped.saturating_add(1);
            return false;
        };
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        let mut sanitized_fields = BTreeMap::new();
        let mut entries = fields.into_iter();
        for (key, value) in entries.by_ref().take(31) {
            let sensitive = is_sensitive_key(&key);
            let key = truncate(&key, 64);
            let redacted = if sensitive {
                "[REDACTED]".to_owned()
            } else {
                redact_value(&value)
            };
            sanitized_fields.insert(key, truncate(&redacted, 256));
        }
        if entries.next().is_some() {
            sanitized_fields.insert("_truncated".to_owned(), "true".to_owned());
        }
        state.events.push_back(RetainedEvent {
            event: DiagnosticEvent {
                sequence,
                test_id: truncate(&self.test_id, 96),
                correlation_id: correlation_id.map(|value| truncate(&redact_value(&value), 96)),
                virtual_time: self.clock.now(),
                kind,
                stage: truncate(&stage.into(), 96),
                fields: sanitized_fields,
            },
            _lease: lease,
        });
        true
    }

    pub fn events(&self) -> Vec<DiagnosticEvent> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .events
            .iter()
            .map(|entry| entry.event.clone())
            .collect()
    }

    pub fn dropped_events(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .dropped
    }

    pub fn truncated(&self) -> bool {
        self.dropped_events() > 0
    }

    pub fn set_sink_enabled(&self, enabled: bool) {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .enabled = enabled;
    }

    pub fn clear(&self) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.events.clear();
        state.dropped = 0;
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "credential",
        "authorization",
        "private_key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(crate) fn redact_value(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let markers = [
        "token=",
        "secret=",
        "password=",
        "credential=",
        "authorization=",
        "bearer ",
    ];
    match markers.iter().any(|marker| lower.contains(marker)) {
        true => "[REDACTED]".to_owned(),
        false => value.to_owned(),
    }
}

fn truncate(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    value
        .char_indices()
        .take_while(|(index, character)| index + character.len_utf8() <= maximum_bytes)
        .map(|(_, character)| character)
        .collect()
}
