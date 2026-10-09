//! Lazy, single-model service for the first ordinary-session generative path.
//! Authentication is owned by the guest adapter. This service neither obtains
//! capabilities nor executes model output. Construction performs no artifact IO.

use core::cell::Cell;

use crate::{
    registry::resources_fit, ArtifactReadError, CancellationToken, CapabilityId,
    GenerativeProvider, IntegrityMetadata, LifecycleState, ModelArtifactReader, ModelBackend,
    ModelId, ModelManifest, ModelRequest, ModelResponse, ModelStoreRecord, ModelStreamResponse,
    ResidentModelRuntime, ResourceBudget, RoleId, RuntimeError, RuntimeResourceReport, StoreError,
    TextChunkSink,
};

/// Opens only the configured model-store artifact; no model-selected path is
/// accepted. Opening and integrity verification happen at the first request.
pub trait ModelArtifactSource {
    type Reader: ModelArtifactReader;
    fn open(&mut self, manifest: &ModelManifest) -> Result<Self::Reader, RuntimeError>;
}

/// A monotonic guest clock, with an explicit cooperative scheduling point.
/// There is no host-clock or spin-counter fallback.
pub trait ModelServiceClock {
    fn now_millis(&self) -> Option<u64>;
    fn cooperate(&self) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelServiceLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_output_tokens: u32,
    pub max_request_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelServiceError {
    Runtime(RuntimeError),
    Store(StoreError),
    ResourceLimit,
    ModelUnavailable,
    ClockUnavailable,
}

impl From<RuntimeError> for ModelServiceError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

/// Keeps one configured, local generative model resident across invocations.
/// The first slice deliberately does not advertise untested model switching.
pub struct LazyModelService<B: ModelBackend, A: ModelArtifactSource, C: ModelServiceClock> {
    model: ModelStoreRecord,
    runtime: ResidentModelRuntime<B>,
    artifacts: A,
    clock: C,
    resources: ResourceBudget,
    limits: ModelServiceLimits,
    state: LifecycleState,
}

impl<B: ModelBackend, A: ModelArtifactSource, C: ModelServiceClock> LazyModelService<B, A, C> {
    pub fn new(
        model: ModelStoreRecord,
        backend: B,
        artifacts: A,
        clock: C,
        resources: ResourceBudget,
        limits: ModelServiceLimits,
    ) -> Result<Self, ModelServiceError> {
        if limits.max_input_bytes == 0
            || limits.max_output_bytes == 0
            || limits.max_output_tokens == 0
            || limits.max_request_millis == 0
        {
            return Err(RuntimeError::InvalidRequest.into());
        }
        Ok(Self {
            model,
            runtime: ResidentModelRuntime::new(backend),
            artifacts,
            clock,
            resources,
            limits,
            state: LifecycleState::Unloaded,
        })
    }

    pub fn model_id(&self) -> &ModelId {
        self.model.model_id()
    }
    pub fn state(&self) -> LifecycleState {
        self.state
    }
    pub fn backend(&self) -> &B {
        self.runtime.backend()
    }
    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.runtime.resource_report()
    }
    pub fn terms_reference(&self) -> Option<&str> {
        self.model.license().terms_reference.as_deref()
    }

    /// The trusted caller records the user's acknowledgement of the exact
    /// configured terms. Inference input cannot acknowledge or replace them.
    pub fn acknowledge_terms(&mut self, reference: &str) -> Result<(), ModelServiceError> {
        self.model
            .acknowledge_terms(reference)
            .map_err(ModelServiceError::Store)
    }

    /// Select by capability/role, preserving the configured default. An
    /// explicit preference for another model fails; no silent substitution.
    pub fn select(
        &self,
        capability: &CapabilityId,
        role: Option<&RoleId>,
        preferred_model: Option<&ModelId>,
    ) -> Result<&ModelId, ModelServiceError> {
        let model = self.model.manifest();
        if !model.has_capability(capability)
            || !self
                .backend()
                .descriptor()
                .capabilities
                .contains(capability)
            || role.is_some_and(|role| !model.has_role(role))
            || preferred_model.is_some_and(|id| id != &model.model_id)
            || model.execution != crate::ExecutionScope::Local
        {
            return Err(ModelServiceError::ModelUnavailable);
        }
        Ok(&model.model_id)
    }

    /// The supervisor supplies a measured/reserved budget. Low memory releases
    /// the resident model before another request can be admitted.
    pub fn set_resource_budget(
        &mut self,
        resources: ResourceBudget,
    ) -> Result<(), ModelServiceError> {
        self.resources = resources;
        if !resources_fit(self.model.manifest(), resources) {
            self.unload()?;
        }
        Ok(())
    }

    /// Call on sign-out, lock, or resource pressure. A new request loads again.
    pub fn unload(&mut self) -> Result<(), ModelServiceError> {
        match self.runtime.unload() {
            Ok(()) => {
                self.state = LifecycleState::Unloaded;
                Ok(())
            }
            Err(error) => {
                self.state = LifecycleState::Disabled;
                Err(error.into())
            }
        }
    }

    pub fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, ModelServiceError> {
        if self.state == LifecycleState::Disabled {
            return Err(RuntimeError::BackendUnavailable.into());
        }
        self.select(request.capability, None, None)?;
        if !self.model.license_acknowledged() {
            return Err(ModelServiceError::Store(
                StoreError::LicenseAcknowledgementRequired,
            ));
        }
        let manifest = self.model.manifest();
        if !resources_fit(manifest, self.resources) {
            self.unload()?;
            return Err(ModelServiceError::ResourceLimit);
        }
        let bytes = request
            .input
            .len()
            .checked_add(request.system_prompt.map_or(0, str::len));
        if request.request_id == 0
            || request.input.trim().is_empty()
            || bytes.is_none_or(|bytes| bytes > self.limits.max_input_bytes)
            || request.input.contains('\0')
            || request
                .system_prompt
                .is_some_and(|prompt| prompt.contains('\0'))
            || request.max_output_tokens == 0
            || request.max_output_tokens > self.limits.max_output_tokens
            || request.max_output_tokens > manifest.context.max_output_tokens
            || request
                .input_tokens
                .is_some_and(|tokens| tokens > manifest.context.max_input_tokens)
            || request
                .options
                .temperature_milli
                .is_some_and(|value| value > 2000)
            || request
                .options
                .top_p_milli
                .is_some_and(|value| value == 0 || value > 1000)
            || (request.capability.as_str() == crate::STRUCTURED_GENERATION_CAPABILITY_ID)
                != request.structured_output.is_some()
        {
            return Err(RuntimeError::InvalidRequest.into());
        }
        let duration = request
            .timeout_millis
            .unwrap_or(self.limits.max_request_millis)
            .min(self.limits.max_request_millis);
        let now = self
            .clock
            .now_millis()
            .ok_or(ModelServiceError::ClockUnavailable)?;
        let deadline = now
            .checked_add(duration)
            .ok_or(ModelServiceError::ClockUnavailable)?;
        let control = RequestControl {
            cancellation,
            clock: &self.clock,
            deadline,
            stopped: Cell::new(None),
        };
        control.check()?;
        if self.runtime.model_id().is_none() {
            self.state = LifecycleState::Loading;
            let loaded = self.artifacts.open(manifest).and_then(|mut reader| {
                let mut reader = ControlledReader {
                    reader: &mut reader,
                    control: &control,
                };
                self.runtime.load(manifest, &mut reader, "x86_64")
            });
            if let Err(error) = loaded {
                self.state = LifecycleState::Failed;
                return Err(control.check().err().unwrap_or(error).into());
            }
        }
        self.state = LifecycleState::Busy;
        let result = control
            .check()
            .and_then(|()| self.runtime.generate(request, &control))
            .and_then(|response| {
                control.check()?;
                if response.text.trim().is_empty()
                    || response.text.len() > self.limits.max_output_bytes
                {
                    return Err(RuntimeError::InvalidBackendResponse);
                }
                Ok(response)
            });
        let result = match control.check() {
            Err(error) => Err(error),
            Ok(()) => result,
        };
        if result.is_err() {
            // Discard failed/cancelled invocation state; never present partial
            // text, and do not reuse a possibly damaged backend context.
            self.unload()?;
            self.state = LifecycleState::Failed;
        } else {
            self.state = LifecycleState::Ready;
        }
        result.map_err(ModelServiceError::Runtime)
    }
}

struct RequestControl<'a, C> {
    cancellation: &'a dyn CancellationToken,
    clock: &'a C,
    deadline: u64,
    stopped: Cell<Option<RuntimeError>>,
}

impl<C: ModelServiceClock> RequestControl<'_, C> {
    fn check(&self) -> Result<(), RuntimeError> {
        if let Some(error) = self.stopped.get() {
            return Err(error);
        }
        self.clock.cooperate();
        let error = if self.cancellation.is_cancelled() {
            Some(RuntimeError::Cancelled)
        } else if self
            .clock
            .now_millis()
            .is_none_or(|now| now >= self.deadline)
        {
            Some(RuntimeError::Timeout)
        } else {
            None
        };
        self.stopped.set(error);
        error.map_or(Ok(()), Err)
    }
}

impl<C: ModelServiceClock> CancellationToken for RequestControl<'_, C> {
    fn is_cancelled(&self) -> bool {
        self.check().is_err()
    }
}

struct ControlledReader<'a, C> {
    reader: &'a mut dyn ModelArtifactReader,
    control: &'a RequestControl<'a, C>,
}

impl<C: ModelServiceClock> ModelArtifactReader for ControlledReader<'_, C> {
    fn artifact_id(&self) -> &crate::ArtifactId {
        self.reader.artifact_id()
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.reader.verified_integrity()
    }
    fn len(&self) -> u64 {
        self.reader.len()
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        self.control
            .check()
            .map_err(|_| ArtifactReadError::Unavailable)?;
        self.reader.read_at(offset, destination)
    }
}

/// Binds the selected local service to consumers such as the existing Planner
/// and page-summary API. Authentication must happen before obtaining this view.
impl<B: ModelBackend, A: ModelArtifactSource, C: ModelServiceClock> GenerativeProvider
    for LazyModelService<B, A, C>
{
    fn model_id(&self) -> &ModelId {
        self.model_id()
    }

    fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, RuntimeError> {
        LazyModelService::generate(self, request, cancellation).map_err(|error| match error {
            ModelServiceError::Runtime(error) => error,
            ModelServiceError::ModelUnavailable => RuntimeError::UnsupportedCapability,
            _ => RuntimeError::BackendUnavailable,
        })
    }

    fn generate_stream(
        &mut self,
        _request: &ModelRequest<'_>,
        _cancellation: &dyn CancellationToken,
        _sink: &mut dyn TextChunkSink,
    ) -> Result<ModelStreamResponse, RuntimeError> {
        // The proven target backend returns complete bounded text only.
        Err(RuntimeError::UnsupportedCapability)
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
