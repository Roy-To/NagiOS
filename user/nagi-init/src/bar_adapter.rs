//! Desktop configuration only. No model operations or terms acceptance at boot.
use super::session_ui::{Reply, Services, SubmitError};
use crate::session_services::SessionServices;
use libnagi::security::Session;
use nagi_model_manager::{ModelManifest, ResourceBudget};

/// Product admission policy, within m20-llama-memory's 3584 MiB native heap.
/// 512 MiB is excluded for non-model native allocations. This is neither VM
/// total RAM nor a measurement of free RAM; allocator isolation is still open.
pub const fn admission_budget() -> ResourceBudget {
    ResourceBudget {
        available_ram_bytes: 3072 * 1024 * 1024,
        available_storage_bytes: 2560 * 1024 * 1024,
        cpu_cores: 1,
        gpu_available: false,
    }
}

pub struct Adapter {
    service: Result<SessionServices, SubmitError>,
    metadata: Option<ModelManifest>,
}
impl Adapter {
    pub fn new(read_only_model_store: u64) -> Self {
        Self {
            service: SessionServices::new(read_only_model_store, admission_budget()),
            metadata: ModelManifest::parse_json(include_bytes!(
                "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
            ))
            .ok(),
        }
    }
    /// Display the same pinned package metadata consumed by SessionServices.
    /// This is OS configuration, never inferred from generated text.
    pub fn terms(&self) -> Option<(&str, &str)> {
        let m = self.metadata.as_ref()?;
        Some((&m.display_name, m.license.terms_reference.as_deref()?))
    }
    fn service(&mut self) -> Result<&mut SessionServices, SubmitError> {
        self.service.as_mut().map_err(|error| *error)
    }
}
impl Services for Adapter {
    fn on_signed_in(&mut self, s: &Session) -> Result<(), SubmitError> {
        self.service()?.on_signed_in(s)
    }
    fn submit_text(&mut self, s: &Session, id: u64, text: &str) -> Result<(), SubmitError> {
        self.service()?.submit_text(s, id, text)
    }
    fn poll_reply(&mut self, s: &Session, id: u64) -> Result<Reply, SubmitError> {
        self.service()?.poll_reply(s, id)
    }
    fn cancel_request(&mut self, s: &Session, id: u64) -> Result<(), SubmitError> {
        self.service()?.cancel_request(s, id)
    }
    fn on_lock_or_signout(&mut self, s: &Session) {
        if let Ok(service) = self.service() {
            service.on_lock_or_signout(s);
        }
    }
    fn acknowledge_model_terms(&mut self, s: &Session, reference: &str) -> Result<(), SubmitError> {
        self.service()?.acknowledge_model_terms(s, reference)
    }
}
