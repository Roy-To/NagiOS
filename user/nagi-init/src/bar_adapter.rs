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
    #[cfg(all(test, nagi_desktop_host_test))]
    host_shared_session: bool,
}
impl Adapter {
    pub fn new(read_only_model_store: u64) -> Self {
        Self {
            service: SessionServices::new(read_only_model_store, admission_budget()),
            metadata: ModelManifest::parse_json(include_bytes!(
                "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
            ))
            .ok(),
            #[cfg(all(test, nagi_desktop_host_test))]
            host_shared_session: false,
        }
    }
    /// Host-only path that binds the real Shared state without starting a worker.
    #[cfg(all(test, nagi_desktop_host_test))]
    pub(crate) fn new_host_shared_session(read_only_model_store: u64) -> Self {
        let mut adapter = Self::new(read_only_model_store);
        adapter.host_shared_session = true;
        adapter
    }
    #[cfg(all(test, nagi_desktop_host_test))]
    pub(crate) fn host_shared_snapshot(&self) -> Option<crate::session_services::HostTestSnapshot> {
        self.service
            .as_ref()
            .ok()
            .map(crate::session_services::host_test_snapshot_for_test)
    }
    /// Complete a queued request through the real Shared state using a scripted host backend.
    #[cfg(all(test, nagi_desktop_host_test))]
    pub fn host_test_complete_request(&self) -> bool {
        match self.service.as_ref() {
            Ok(service) => crate::session_services::host_test_complete_queued_request(service),
            Err(_) => false,
        }
    }
    /// Display the pinned model name and the exact reference owned by the
    /// same SessionServices instance that will record the user's acceptance.
    pub fn terms(&self) -> Option<(&str, &str)> {
        let metadata = self.metadata.as_ref()?;
        let service = self.service.as_ref().ok()?;
        Some((&metadata.display_name, service.model_terms_reference()))
    }
    fn service(&mut self) -> Result<&mut SessionServices, SubmitError> {
        self.service.as_mut().map_err(|error| *error)
    }
}
impl Services for Adapter {
    fn on_signed_in(&mut self, s: &Session) -> Result<(), SubmitError> {
        #[cfg(all(test, nagi_desktop_host_test))]
        if self.host_shared_session {
            return crate::session_services::host_test_bind_for_test(self.service()?, s);
        }
        self.service()?.on_signed_in(s)
    }
    fn submit_text(&mut self, s: &Session, id: u64, text: &str) -> Result<(), SubmitError> {
        self.service()?.submit_text(s, id, text)
    }
    fn poll_reply(&mut self, s: &Session, id: u64) -> Result<Reply, SubmitError> {
        self.service()?.poll_reply(s, id)
    }
    fn submit_page_summary(
        &mut self,
        s: &Session,
        caller: nagi_ai::CallerIdentity,
        id: u64,
        request: nagi_ai::ContextRequest,
        authority: &dyn nagi_ai::ContextAuthority,
        browser: &mut dyn nagi_ai::PublicBrowserContextApi,
    ) -> Result<(), SubmitError> {
        self.service()?
            .submit_page_summary(s, caller, id, request, authority, browser)
    }
    fn poll_page_summary(
        &mut self,
        s: &Session,
        id: u64,
        current_request: &nagi_ai::ContextRequest,
        authority: &dyn nagi_ai::ContextAuthority,
        browser: &mut dyn nagi_ai::PublicBrowserContextApi,
    ) -> Result<Reply, SubmitError> {
        self.service()?
            .poll_page_summary(s, id, current_request, authority, browser)
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
