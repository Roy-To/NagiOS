//! Ordinary-session Granite service. The desktop owns this adapter and supplies
//! its live authenticated Session; model input cannot establish caller authority.
//! This is an in-process bootstrap adapter, not an isolated AI process.

#[cfg(all(test, not(target_os = "nagi")))]
use self::boundary_tests::syscalls as libnagi;
use libnagi::security::Session;
use nagi_model_manager::{
    ArtifactReadError, ArtifactReference, CancellationToken, CapabilityId, Fat32ArtifactReader,
    LazyModelService, ModelArtifactSource, ModelId, ModelManifest, ModelRequest, ModelResponse,
    ModelServiceClock, ModelServiceError, ModelServiceLimits, ModelStoreRecord,
    ModelStoreSectorReader, ResourceBudget, RoleId, RuntimeError, RuntimeResourceReport,
    FAT32_SECTOR_SIZE,
};

#[path = "llama_backend.rs"]
pub(crate) mod llama_backend;
use llama_backend::LlamaCppBackend;
#[path = "model_session_access.rs"]
mod model_session_access;
use model_session_access::ModelSessionAccess;
#[path = "session_page_summary.rs"]
mod session_page_summary;
pub use session_page_summary::SessionPageSummaryError;

const MODEL_STORE_SECTORS: u64 = 67_108_864;
// These are service admission bounds, not model package metadata changes.
const LIMITS: ModelServiceLimits = ModelServiceLimits {
    max_input_bytes: 24 * 1024,
    max_output_bytes: 64 * 1024,
    max_output_tokens: 256,
    max_request_millis: 15 * 60 * 1000,
};

pub struct ReadOnlyModelStore {
    capability: u64,
    sectors_until_yield: u8,
}

impl ModelStoreSectorReader for ReadOnlyModelStore {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; FAT32_SECTOR_SIZE],
    ) -> Result<(), ArtifactReadError> {
        // FAT32 directory and cluster-chain walks can perform many sector
        // reads inside one artifact operation. Keep those walks cooperative
        // too, with the same 64 KiB scale as the outer artifact checkpoints.
        if self.sectors_until_yield == 0 {
            let _ = libnagi::thread_yield();
            self.sectors_until_yield = 128;
        }
        self.sectors_until_yield -= 1;
        libnagi::block_read(self.capability, sector, destination)
            .then_some(())
            .ok_or(ArtifactReadError::Unavailable)
    }
}

struct GuestArtifacts {
    model_store_capability: u64,
}

impl ModelArtifactSource for GuestArtifacts {
    type Reader = Fat32ArtifactReader<ReadOnlyModelStore>;
    fn open(&mut self, manifest: &ModelManifest) -> Result<Self::Reader, RuntimeError> {
        let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
        Fat32ArtifactReader::open(
            ReadOnlyModelStore {
                capability: self.model_store_capability,
                sectors_until_yield: 0,
            },
            MODEL_STORE_SECTORS,
            artifact_id.clone(),
        )
        .map_err(|_| RuntimeError::ArtifactUnavailable)
    }
}

struct GuestClock;

impl ModelServiceClock for GuestClock {
    fn now_millis(&self) -> Option<u64> {
        // The existing APIC monotonic timer contract is 10 ms per tick.
        libnagi::time_ticks().checked_mul(10)
    }
    fn cooperate(&self) {
        let _ = libnagi::thread_yield();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionModelError {
    Denied,
    Service(ModelServiceError),
}

type Service = LazyModelService<LlamaCppBackend, GuestArtifacts, GuestClock>;

pub struct SessionModelService {
    access: ModelSessionAccess,
    service: Service,
}

impl SessionModelService {
    /// Call after desktop readiness/sign-in. No disk read, model hash, or llama
    /// initialization occurs here. The budget is reserved by the caller using
    /// guest resources; this adapter does not invent an 8 GB free-memory report.
    pub fn new(
        user: &Session,
        model_store_capability: u64,
        budget: ResourceBudget,
    ) -> Result<Self, SessionModelError> {
        let access = ModelSessionAccess::bind(user).ok_or(SessionModelError::Denied)?;
        if model_store_capability == 0 {
            return Err(SessionModelError::Service(
                RuntimeError::ArtifactUnavailable.into(),
            ));
        }
        let manifest = ModelManifest::parse_json(include_bytes!(
            "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
        ))
        .map_err(|error| {
            SessionModelError::Service(RuntimeError::ManifestRejected(error).into())
        })?;
        let model = ModelStoreRecord::discovered(manifest)
            .map_err(|error| SessionModelError::Service(ModelServiceError::Store(error)))?;
        let service = Service::new(
            model,
            LlamaCppBackend::new(),
            GuestArtifacts {
                model_store_capability,
            },
            GuestClock,
            budget,
            LIMITS,
        )
        .map_err(SessionModelError::Service)?;
        Ok(Self { access, service })
    }

    fn authorize(&mut self, user: &Session) -> Result<(), SessionModelError> {
        if !self.access.allows(user, None) {
            let _ = self.service.unload();
            return Err(SessionModelError::Denied);
        }
        Ok(())
    }

    pub fn terms_reference(&self) -> Option<&str> {
        self.service.terms_reference()
    }

    /// Invoke only for an explicit user acknowledgement in OS-owned UI, never
    /// as a consequence of generated output or text in a model request.
    pub fn acknowledge_terms(
        &mut self,
        user: &Session,
        reference: &str,
    ) -> Result<(), SessionModelError> {
        self.authorize(user)?;
        self.service
            .acknowledge_terms(reference)
            .map_err(SessionModelError::Service)
    }

    /// Complete UTF-8 output with model/provider/backend identity and usage.
    /// Run on a guest worker thread so the desktop can poll input and cancel;
    /// the service yields during artifact reads and llama abort callbacks.
    pub fn generate(
        &mut self,
        user: &Session,
        role: Option<&RoleId>,
        preferred_model: Option<&ModelId>,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, SessionModelError> {
        self.authorize(user)?;
        // This first adapter serves the OS-owned Nagi Bar. External apps must
        // use a separately authenticated/granted IPC entry, which is not here.
        if !self.access.allows(user, request.caller) {
            return Err(SessionModelError::Denied);
        }
        self.service
            .select(request.capability, role, preferred_model)
            .map_err(SessionModelError::Service)?;
        self.service
            .generate(request, cancellation)
            .map_err(SessionModelError::Service)
    }

    /// Explicit OS-owned summary operation. `trusted_caller` is the requesting
    /// assistant identity supplied by OS-owned state, not the browser identity.
    /// This freely constructible value is a trusted-caller precondition, not
    /// proof of OS provenance. No ordinary transport supplies it yet. Browser
    /// policy still enforces disclosure; external AppIds remain unauthorized.
    #[allow(clippy::too_many_arguments)]
    pub fn summarize_page(
        &mut self,
        user: &Session,
        trusted_caller: nagi_ai::CallerIdentity,
        request_id: u64,
        request: nagi_ai::ContextRequest,
        authority: &impl nagi_ai::ContextAuthority,
        browser: &mut impl nagi_ai::PublicBrowserContextApi,
        cancellation: &dyn CancellationToken,
    ) -> Result<alloc::string::String, SessionPageSummaryError> {
        self.authorize(user)
            .map_err(|_| SessionPageSummaryError::Denied)?;
        let access = ModelSessionAccess::bind(user).ok_or(SessionPageSummaryError::Denied)?;
        let role = RoleId::new("standard").map_err(|_| SessionPageSummaryError::Denied)?;
        let capability =
            CapabilityId::new("text.generate").map_err(|_| SessionPageSummaryError::Denied)?;
        let model_id = self
            .selected_model(&capability, Some(&role))
            .map_err(|error| match error {
                SessionModelError::Denied => SessionPageSummaryError::Denied,
                SessionModelError::Service(error) => SessionPageSummaryError::Service(error),
            })?
            .clone();
        session_page_summary::summarize(
            &access,
            user,
            trusted_caller,
            request_id,
            request,
            authority,
            browser,
            model_id,
            LIMITS,
            |request, cancellation| {
                self.generate(user, Some(&role), None, request, cancellation)
                    .map_err(|error| match error {
                        SessionModelError::Denied => SessionPageSummaryError::Denied,
                        SessionModelError::Service(error) => {
                            SessionPageSummaryError::Service(error)
                        }
                    })
            },
            cancellation,
        )
    }

    /// Queue-owned summary snapshot, after public API extraction. Only the
    /// OS-owned SessionServices worker calls this; no external IPC is exposed.
    pub(super) fn generate_page_summary(
        &mut self,
        user: &Session,
        prompt: &nagi_ai::PageSummaryPrompt,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, SessionPageSummaryError> {
        self.authorize(user)
            .map_err(|_| SessionPageSummaryError::Denied)?;
        let role = RoleId::new("standard").map_err(|_| SessionPageSummaryError::Denied)?;
        let capability =
            CapabilityId::new("text.generate").map_err(|_| SessionPageSummaryError::Denied)?;
        let model_id = self
            .selected_model(&capability, Some(&role))
            .map_err(|error| match error {
                SessionModelError::Denied => SessionPageSummaryError::Denied,
                SessionModelError::Service(error) => SessionPageSummaryError::Service(error),
            })?
            .clone();
        let access = ModelSessionAccess::bind(user).ok_or(SessionPageSummaryError::Denied)?;
        session_page_summary::generate_prepared(
            &access,
            user,
            prompt,
            model_id,
            LIMITS,
            |request, token| {
                self.generate(user, Some(&role), None, request, token)
                    .map_err(|error| match error {
                        SessionModelError::Denied => SessionPageSummaryError::Denied,
                        SessionModelError::Service(error) => {
                            SessionPageSummaryError::Service(error)
                        }
                    })
            },
            cancellation,
        )
    }

    pub fn selected_model(
        &self,
        capability: &CapabilityId,
        role: Option<&RoleId>,
    ) -> Result<&ModelId, SessionModelError> {
        self.service
            .select(capability, role, None)
            .map_err(SessionModelError::Service)
    }

    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.service.resource_report()
    }

    pub fn set_resource_budget(&mut self, budget: ResourceBudget) -> Result<(), SessionModelError> {
        self.service
            .set_resource_budget(budget)
            .map_err(SessionModelError::Service)
    }

    /// Cancel the active worker first, then release on lock/sign-out or pressure.
    pub fn unload(&mut self) -> Result<(), SessionModelError> {
        self.service.unload().map_err(SessionModelError::Service)
    }
}

#[cfg(all(test, not(target_os = "nagi")))]
#[path = "../../../tests/model-session-service/src/model_service_boundary.rs"]
mod boundary_tests;
