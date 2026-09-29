use crate::{DiagnosticKind, DiagnosticsRecorder, HarnessError};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OfflineRequest {
    pub destination: String,
    pub purpose: String,
}

#[derive(Clone, Default)]
pub struct OfflineNetworkPolicy {
    inner: Arc<Mutex<AttemptState>>,
    diagnostics: Option<DiagnosticsRecorder>,
}

#[derive(Default)]
struct AttemptState {
    attempts: Vec<OfflineRequest>,
    dropped: u64,
}

impl OfflineNetworkPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_diagnostics(mut self, diagnostics: DiagnosticsRecorder) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }

    /// Deliberately has no socket implementation: every attempted request is denied locally.
    pub fn request(&self, request: OfflineRequest) -> Result<(), HarnessError> {
        let request = OfflineRequest {
            destination: bounded_text(&crate::diagnostics::redact_value(&request.destination), 256),
            purpose: bounded_text(&crate::diagnostics::redact_value(&request.purpose), 128),
        };
        {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if state.attempts.len() < 256 {
                state.attempts.push(request.clone());
            } else {
                state.dropped = state.dropped.saturating_add(1);
            }
        }
        if let Some(diagnostics) = &self.diagnostics {
            let mut fields = BTreeMap::new();
            fields.insert("destination".to_owned(), request.destination);
            fields.insert("purpose".to_owned(), request.purpose);
            diagnostics.record(
                DiagnosticKind::OfflineNetworkDenied,
                None,
                "offline-policy",
                fields,
            );
        }
        Err(HarnessError::OfflineNetworkDenied)
    }

    pub fn attempts(&self) -> Vec<OfflineRequest> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .attempts
            .clone()
    }

    pub fn dropped_attempts(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .dropped
    }
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    value
        .char_indices()
        .take_while(|(index, character)| index + character.len_utf8() <= maximum_bytes)
        .map(|(_, character)| character)
        .collect()
}
