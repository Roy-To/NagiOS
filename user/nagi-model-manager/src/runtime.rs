use alloc::{string::String, vec::Vec};
use core::fmt;

use nagi_model::AppId;

use crate::{
    ArtifactId, ArtifactReference, BackendDescriptor, BackendId, CapabilityId, IntegrityMetadata,
    ManifestError, ModelId, ModelManifest, ProviderId,
};

pub const STREAMING_CAPABILITY_ID: &str = "text.stream";

pub trait ModelArtifactReader {
    fn artifact_id(&self) -> &ArtifactId;
    fn verified_integrity(&self) -> Option<&IntegrityMetadata>;
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactReadError {
    Unavailable,
    OutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResponse {
    pub text: String,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResponse {
    pub request_id: u64,
    pub model_id: ModelId,
    pub provider_id: ProviderId,
    pub backend_id: BackendId,
    pub text: String,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelStreamResponse {
    pub request_id: u64,
    pub model_id: ModelId,
    pub provider_id: ProviderId,
    pub backend_id: BackendId,
    pub usage: TokenUsage,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GenerationOptions {
    /// Temperature expressed in thousandths; supported values are 0 through 2000.
    pub temperature_milli: Option<u16>,
    /// Top-p expressed in thousandths; supported values are 1 through 1000.
    pub top_p_milli: Option<u16>,
    pub seed: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelRequest<'a> {
    pub request_id: u64,
    pub caller: Option<AppId>,
    pub capability: &'a CapabilityId,
    /// Optional system instructions. `input_tokens` includes this field when supplied.
    pub system_prompt: Option<&'a str>,
    pub input: &'a str,
    pub input_tokens: Option<u32>,
    pub max_output_tokens: u32,
    pub options: GenerationOptions,
    pub timeout_millis: Option<u64>,
}

pub trait CancellationToken {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamConsumerError {
    Closed,
    Failed,
}

pub trait TextChunkSink {
    fn push_chunk(&mut self, chunk: &str) -> Result<(), StreamConsumerError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendHealth {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeResourceReport {
    pub loaded_model_sessions: u32,
    pub active_invocations: u32,
    pub max_concurrent_sessions: Option<u32>,
    pub resident_memory_bytes: Option<u64>,
}

pub trait GenerativeProvider {
    fn model_id(&self) -> &ModelId;

    fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, RuntimeError>;

    fn generate_stream(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<ModelStreamResponse, RuntimeError>;
}

pub trait ModelBackend {
    type Session;

    fn descriptor(&self) -> &BackendDescriptor;

    fn health(&self) -> BackendHealth;

    fn resource_report(&self) -> RuntimeResourceReport;

    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError>;

    fn infer(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<BackendResponse, RuntimeError>;

    fn infer_stream(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError>;

    fn unload(&mut self, session: Self::Session) -> Result<(), RuntimeError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    ManifestRejected(ManifestError),
    IncompatibleBackend,
    RemoteProviderUnavailable,
    ArtifactUnavailable,
    IntegrityRequired,
    ArtifactMismatch,
    IntegrityMismatch,
    InvalidRequest,
    UnsupportedCapability,
    StreamConsumer(StreamConsumerError),
    ContextLimit,
    InvalidBackendResponse,
    Cancelled,
    Timeout,
    BackendUnavailable,
    LoadFailed,
    InferenceFailed,
    UnloadFailed,
    SessionClosed,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::ManifestRejected(error) => {
                return write!(formatter, "manifest rejected: {error}")
            }
            Self::IncompatibleBackend => "backend is incompatible with model",
            Self::RemoteProviderUnavailable => {
                "remote provider is not available in the local runtime"
            }
            Self::ArtifactUnavailable => "model artifact is unavailable",
            Self::IntegrityRequired => "model artifact requires verified integrity metadata",
            Self::ArtifactMismatch => "artifact identity or size does not match the manifest",
            Self::IntegrityMismatch => "artifact integrity does not match the manifest",
            Self::InvalidRequest => "model request is invalid",
            Self::UnsupportedCapability => "model does not support the requested capability",
            Self::StreamConsumer(StreamConsumerError::Closed) => {
                "model output stream consumer closed"
            }
            Self::StreamConsumer(StreamConsumerError::Failed) => {
                "model output stream consumer failed"
            }
            Self::ContextLimit => "request exceeds the model context limits",
            Self::InvalidBackendResponse => "backend response exceeds the declared request bounds",
            Self::Cancelled => "model request was cancelled",
            Self::Timeout => "model request timed out",
            Self::BackendUnavailable => "model backend is unavailable",
            Self::LoadFailed => "model load failed",
            Self::InferenceFailed => "model inference failed",
            Self::UnloadFailed => "model unload failed",
            Self::SessionClosed => "model session is closed",
        };
        formatter.write_str(description)
    }
}

pub struct ModelRuntime<B: ModelBackend> {
    backend: B,
}

impl<B: ModelBackend> ModelRuntime<B> {
    pub const fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn health(&self) -> BackendHealth {
        self.backend.health()
    }

    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.backend.resource_report()
    }

    pub fn validate(
        &self,
        manifest: &ModelManifest,
        artifact: &dyn ModelArtifactReader,
        target_architecture: &str,
    ) -> Result<(), RuntimeError> {
        manifest
            .validate()
            .map_err(RuntimeError::ManifestRejected)?;
        if manifest.execution != crate::ExecutionScope::Local {
            return Err(RuntimeError::RemoteProviderUnavailable);
        }
        if manifest.artifact.integrity.is_none() {
            return Err(RuntimeError::IntegrityRequired);
        }
        if artifact.is_empty() {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
        if artifact.artifact_id() != artifact_id
            || manifest
                .artifact
                .size_bytes
                .is_some_and(|size| size != artifact.len())
        {
            return Err(RuntimeError::ArtifactMismatch);
        }
        if manifest
            .artifact
            .integrity
            .as_ref()
            .is_some_and(|expected| artifact.verified_integrity() != Some(expected))
        {
            return Err(RuntimeError::IntegrityMismatch);
        }
        let descriptor = self.backend.descriptor();
        if !manifest
            .supported_backends
            .iter()
            .any(|id| id == &descriptor.backend_id)
            || !descriptor
                .runtime_api_versions
                .iter()
                .any(|version| version == &manifest.runtime_api_version)
            || !descriptor
                .artifact_formats
                .iter()
                .any(|format| format == &manifest.artifact.format)
            || (!descriptor.architectures.is_empty()
                && !descriptor
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
            || (!manifest.compatibility.architectures.is_empty()
                && !manifest
                    .compatibility
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
            || manifest
                .runtime_class
                .as_ref()
                .is_some_and(|runtime_class| {
                    !descriptor
                        .runtime_classes
                        .iter()
                        .any(|supported| supported == runtime_class)
                })
            || !manifest
                .capabilities
                .iter()
                .any(|capability| descriptor.capabilities.contains(capability))
        {
            return Err(RuntimeError::IncompatibleBackend);
        }
        if self.backend.health() == BackendHealth::Unavailable {
            return Err(RuntimeError::BackendUnavailable);
        }
        Ok(())
    }

    pub fn load<'runtime>(
        &'runtime mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
        target_architecture: &str,
    ) -> Result<LoadedSession<'runtime, B>, RuntimeError> {
        self.validate(manifest, artifact, target_architecture)?;
        let backend_capabilities = self.backend.descriptor().capabilities.clone();
        let session = self.backend.load(manifest, artifact)?;
        Ok(LoadedSession {
            backend: &mut self.backend,
            session: Some(session),
            model_id: manifest.model_id.clone(),
            provider_id: manifest.provider.provider_id.clone(),
            capabilities: manifest.capabilities.clone(),
            backend_capabilities,
            max_input_tokens: manifest.context.max_input_tokens,
            max_output_tokens: manifest.context.max_output_tokens,
        })
    }
}

pub struct LoadedSession<'runtime, B: ModelBackend> {
    backend: &'runtime mut B,
    session: Option<B::Session>,
    model_id: ModelId,
    provider_id: ProviderId,
    capabilities: Vec<CapabilityId>,
    backend_capabilities: Vec<CapabilityId>,
    max_input_tokens: u32,
    max_output_tokens: u32,
}

impl<B: ModelBackend> LoadedSession<'_, B> {
    pub fn health(&self) -> BackendHealth {
        self.backend.health()
    }

    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.backend.resource_report()
    }

    fn validate_request(
        &self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<(), RuntimeError> {
        if self.session.is_none() {
            return Err(RuntimeError::SessionClosed);
        }
        if request.max_output_tokens == 0
            || request.max_output_tokens > self.max_output_tokens
            || request
                .options
                .temperature_milli
                .is_some_and(|temperature| temperature > 2000)
            || request
                .options
                .top_p_milli
                .is_some_and(|top_p| top_p == 0 || top_p > 1000)
        {
            return Err(RuntimeError::InvalidRequest);
        }
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == request.capability)
            || !self
                .backend_capabilities
                .iter()
                .any(|capability| capability == request.capability)
        {
            return Err(RuntimeError::UnsupportedCapability);
        }
        if request
            .input_tokens
            .is_some_and(|tokens| tokens > self.max_input_tokens)
        {
            return Err(RuntimeError::ContextLimit);
        }
        if request.timeout_millis == Some(0) {
            return Err(RuntimeError::Timeout);
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        if self.backend.health() == BackendHealth::Unavailable {
            return Err(RuntimeError::BackendUnavailable);
        }
        Ok(())
    }

    fn has_streaming_capability(&self) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability.as_str() == STREAMING_CAPABILITY_ID)
            && self
                .backend_capabilities
                .iter()
                .any(|capability| capability.as_str() == STREAMING_CAPABILITY_ID)
    }

    pub fn unload(mut self) -> Result<(), RuntimeError> {
        let session = self.session.take().ok_or(RuntimeError::SessionClosed)?;
        self.backend.unload(session)
    }
}

impl<B: ModelBackend> GenerativeProvider for LoadedSession<'_, B> {
    fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, RuntimeError> {
        self.validate_request(request, cancellation)?;
        let backend_id = self.backend.descriptor().backend_id.clone();
        let session = self.session.as_mut().ok_or(RuntimeError::SessionClosed)?;
        let response = self.backend.infer(session, request, cancellation)?;
        if response.usage.output_tokens > request.max_output_tokens
            || response.usage.input_tokens > self.max_input_tokens
        {
            return Err(RuntimeError::InvalidBackendResponse);
        }
        Ok(ModelResponse {
            request_id: request.request_id,
            model_id: self.model_id.clone(),
            provider_id: self.provider_id.clone(),
            backend_id,
            text: response.text,
            usage: response.usage,
        })
    }

    fn generate_stream(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<ModelStreamResponse, RuntimeError> {
        self.validate_request(request, cancellation)?;
        if !self.has_streaming_capability() {
            return Err(RuntimeError::UnsupportedCapability);
        }
        let backend_id = self.backend.descriptor().backend_id.clone();
        let session = self.session.as_mut().ok_or(RuntimeError::SessionClosed)?;
        let usage = self
            .backend
            .infer_stream(session, request, cancellation, sink)?;
        if usage.output_tokens > request.max_output_tokens
            || usage.input_tokens > self.max_input_tokens
        {
            return Err(RuntimeError::InvalidBackendResponse);
        }
        Ok(ModelStreamResponse {
            request_id: request.request_id,
            model_id: self.model_id.clone(),
            provider_id: self.provider_id.clone(),
            backend_id,
            usage,
        })
    }
}

impl<B: ModelBackend> Drop for LoadedSession<'_, B> {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = self.backend.unload(session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionScope, IntegrityMetadata, ModelManifest, RuntimeClassId};
    use alloc::{format, string::String, vec};
    use core::cell::Cell;

    use crate::testing::{FakeModelBackend, MemoryArtifactReader, NeverCancel, StaticCancellation};

    fn manifest() -> ModelManifest {
        let mut manifest = ModelManifest::parse_json(
            include_str!("../tests/fixtures/granite-4.2-3b.json").as_bytes(),
        )
        .unwrap();
        // Synthetic metadata used only by this in-memory orchestration test.
        manifest.artifact.size_bytes = Some(1);
        manifest.artifact.integrity = Some(IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: "a".repeat(64),
        });
        manifest
    }

    #[test]
    fn fake_backend_loads_generates_and_unloads_a_model_session() {
        let model = manifest();
        let backend = FakeModelBackend::for_manifest(&model);
        let mut runtime = ModelRuntime::new(backend);
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let capability = CapabilityId::new("text.generate").unwrap();
        let request = ModelRequest {
            request_id: 7,
            caller: Some(AppId(9)),
            capability: &capability,
            system_prompt: Some("Be concise."),
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
            options: GenerationOptions::default(),
            timeout_millis: Some(1000),
        };
        let mut session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        assert_eq!(session.model_id().as_str(), "ibm.granite-4.2-3b");
        let response = session.generate(&request, &NeverCancel).unwrap();
        assert_eq!(response.request_id, 7);
        assert_eq!(
            response.text,
            format!("fake:{}:hello", model.model_id.as_str())
        );
        assert_eq!(response.backend_id.as_str(), "llama_cpp");
        session.unload().unwrap();
        assert_eq!(runtime.backend().unload_count, 1);
        assert_eq!(
            runtime.backend().last_system_prompt.as_deref(),
            Some("Be concise.")
        );
    }

    #[test]
    fn cancellation_timeout_and_bounds_are_structured_errors() {
        let model = manifest();
        let mut backend = FakeModelBackend::for_manifest(&model);
        backend.response_output_tokens = 65;
        let mut runtime = ModelRuntime::new(backend);
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let capability = CapabilityId::new("text.generate").unwrap();
        let mut session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        let mut request = ModelRequest {
            request_id: 8,
            caller: None,
            capability: &capability,
            system_prompt: None,
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
            options: GenerationOptions::default(),
            timeout_millis: Some(10),
        };
        assert_eq!(
            session.generate(&request, &StaticCancellation(true)),
            Err(RuntimeError::Cancelled)
        );
        request.timeout_millis = Some(0);
        assert_eq!(
            session.generate(&request, &NeverCancel),
            Err(RuntimeError::Timeout)
        );
        request.timeout_millis = Some(10);
        request.input_tokens = Some(model.context.max_input_tokens + 1);
        assert_eq!(
            session.generate(&request, &NeverCancel),
            Err(RuntimeError::ContextLimit)
        );
        request.input_tokens = Some(1);
        assert_eq!(
            session.generate(&request, &NeverCancel),
            Err(RuntimeError::InvalidBackendResponse)
        );
        request.options.top_p_milli = Some(1001);
        assert_eq!(
            session.generate(&request, &NeverCancel),
            Err(RuntimeError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_remote_models_and_incompatible_backends() {
        let mut model = manifest();
        model.execution = ExecutionScope::Remote;
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&manifest()));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        assert_eq!(
            runtime.load(&model, &mut artifact, "x86_64").err(),
            Some(RuntimeError::RemoteProviderUnavailable)
        );

        let model = manifest();
        let mut incompatible = FakeModelBackend::for_manifest(&model);
        incompatible.descriptor.backend_id = BackendId::new("other_backend").unwrap();
        let mut runtime = ModelRuntime::new(incompatible);
        assert_eq!(
            runtime.load(&model, &mut artifact, "x86_64").err(),
            Some(RuntimeError::IncompatibleBackend)
        );
    }

    #[test]
    fn rejects_artifact_identity_and_integrity_mismatches() {
        let model = manifest();
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        let wrong_artifact = MemoryArtifactReader::with_metadata(
            ArtifactId::new("other.model").unwrap(),
            None,
            vec![1],
        );
        let mut wrong_artifact = wrong_artifact;
        assert_eq!(
            runtime.load(&model, &mut wrong_artifact, "x86_64").err(),
            Some(RuntimeError::ArtifactMismatch)
        );
        let mut wrong_size = MemoryArtifactReader::with_metadata(
            ArtifactId::new("ibm.granite-4.2-3b").unwrap(),
            None,
            vec![1, 2],
        );
        assert_eq!(
            runtime.load(&model, &mut wrong_size, "x86_64").err(),
            Some(RuntimeError::ArtifactMismatch)
        );

        let mut integrity_model = manifest();
        let expected = IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: "a".repeat(64),
        };
        integrity_model.artifact.integrity = Some(expected.clone());
        let mut wrong_integrity = MemoryArtifactReader::with_metadata(
            ArtifactId::new("ibm.granite-4.2-3b").unwrap(),
            Some(IntegrityMetadata {
                algorithm: String::from("sha256"),
                digest: "b".repeat(64),
            }),
            vec![1],
        );
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&integrity_model));
        assert_eq!(
            runtime
                .load(&integrity_model, &mut wrong_integrity, "x86_64")
                .err(),
            Some(RuntimeError::IntegrityMismatch)
        );
        let mut matching_integrity = MemoryArtifactReader::with_metadata(
            ArtifactId::new("ibm.granite-4.2-3b").unwrap(),
            Some(expected),
            vec![1],
        );
        let session = runtime
            .load(&integrity_model, &mut matching_integrity, "x86_64")
            .unwrap();
        session.unload().unwrap();
    }

    #[test]
    fn model_store_artifact_reader_supports_bounded_random_access() {
        let model = manifest();
        assert!(matches!(
            model.artifact.reference,
            ArtifactReference::ModelStore { .. }
        ));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1, 2, 3]);
        let mut output = [0; 2];
        assert_eq!(artifact.read_at(1, &mut output), Ok(2));
        assert_eq!(output, [2, 3]);
        assert_eq!(
            artifact.read_at(4, &mut output),
            Err(ArtifactReadError::OutOfRange)
        );
    }

    #[test]
    fn refuses_to_load_an_artifact_without_integrity_metadata() {
        let model = ModelManifest::parse_json(
            include_str!("../tests/fixtures/granite-4.2-3b.json").as_bytes(),
        )
        .unwrap();
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        assert_eq!(
            runtime.load(&model, &mut artifact, "x86_64").err(),
            Some(RuntimeError::IntegrityRequired)
        );
    }

    #[derive(Default)]
    struct CollectChunks(String);

    impl TextChunkSink for CollectChunks {
        fn push_chunk(&mut self, chunk: &str) -> Result<(), StreamConsumerError> {
            self.0.push_str(chunk);
            Ok(())
        }
    }

    struct CancelAfterChecks(Cell<u32>);

    impl CancellationToken for CancelAfterChecks {
        fn is_cancelled(&self) -> bool {
            let checks = self.0.get();
            self.0.set(checks + 1);
            checks >= 2
        }
    }

    #[test]
    fn streams_provider_neutral_text_chunks_and_usage() {
        let model = manifest();
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let capability = CapabilityId::new("text.generate").unwrap();
        let request = ModelRequest {
            request_id: 12,
            caller: None,
            capability: &capability,
            system_prompt: Some("Be concise."),
            input: "hello",
            input_tokens: Some(3),
            max_output_tokens: 64,
            options: GenerationOptions {
                temperature_milli: Some(500),
                top_p_milli: Some(900),
                seed: Some(17),
            },
            timeout_millis: Some(1000),
        };
        let mut session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        let mut chunks = CollectChunks::default();
        let response = session
            .generate_stream(&request, &NeverCancel, &mut chunks)
            .unwrap();

        assert_eq!(chunks.0, "fake:ibm.granite-4.2-3b:hello");
        assert_eq!(response.request_id, request.request_id);
        assert_eq!(response.model_id, model.model_id);
        assert_eq!(response.backend_id.as_str(), "llama_cpp");
        assert_eq!(response.usage.input_tokens, 3);
        assert_eq!(session.health(), BackendHealth::Healthy);
        session.unload().unwrap();
        assert_eq!(runtime.backend().stream_count, 1);
        assert_eq!(
            runtime.backend().last_system_prompt.as_deref(),
            Some("Be concise.")
        );
        assert_eq!(runtime.backend().last_options, Some(request.options));
    }

    #[test]
    fn cancellation_during_streaming_returns_cancelled_after_emitted_chunks() {
        let model = manifest();
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let capability = CapabilityId::new("text.generate").unwrap();
        let request = ModelRequest {
            request_id: 13,
            caller: None,
            capability: &capability,
            system_prompt: None,
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
            options: GenerationOptions::default(),
            timeout_millis: Some(1000),
        };
        let mut session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        let mut chunks = CollectChunks::default();
        assert_eq!(
            session.generate_stream(&request, &CancelAfterChecks(Cell::new(0)), &mut chunks),
            Err(RuntimeError::Cancelled)
        );
        assert_eq!(chunks.0, "fake:");
    }

    #[test]
    fn model_without_stream_capability_rejects_streaming() {
        let mut model =
            ModelManifest::parse_json(include_str!("../tests/fixtures/gemma-3-1b.json").as_bytes())
                .unwrap();
        model.artifact.size_bytes = Some(1);
        model.artifact.integrity = Some(IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: "a".repeat(64),
        });
        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let capability = CapabilityId::new("text.generate").unwrap();
        let request = ModelRequest {
            request_id: 14,
            caller: None,
            capability: &capability,
            system_prompt: None,
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
            options: GenerationOptions::default(),
            timeout_millis: Some(1000),
        };
        let mut session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        assert_eq!(
            session.generate_stream(&request, &NeverCancel, &mut CollectChunks::default()),
            Err(RuntimeError::UnsupportedCapability)
        );
    }

    #[test]
    fn exposes_backend_health_and_resource_reporting_and_handles_unavailable_backend() {
        let model = manifest();
        let mut backend = FakeModelBackend::for_manifest(&model);
        backend.resident_memory_bytes = Some(4096);
        let mut runtime = ModelRuntime::new(backend);
        assert_eq!(runtime.health(), BackendHealth::Healthy);
        assert_eq!(
            runtime.resource_report(),
            RuntimeResourceReport {
                loaded_model_sessions: 0,
                active_invocations: 0,
                max_concurrent_sessions: Some(1),
                resident_memory_bytes: Some(4096),
            }
        );
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        let session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
        assert_eq!(session.resource_report().loaded_model_sessions, 1);
        session.unload().unwrap();
        assert_eq!(runtime.resource_report().loaded_model_sessions, 0);

        let mut unavailable = FakeModelBackend::for_manifest(&model);
        unavailable.health = BackendHealth::Unavailable;
        let mut runtime = ModelRuntime::new(unavailable);
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        assert_eq!(
            runtime.load(&model, &mut artifact, "x86_64").err(),
            Some(RuntimeError::BackendUnavailable)
        );
    }

    #[test]
    fn reports_backend_load_failure_and_repeated_load_unload_cycles() {
        let model = manifest();
        let mut failed_backend = FakeModelBackend::for_manifest(&model);
        failed_backend.load_error = Some(RuntimeError::LoadFailed);
        let mut failed_runtime = ModelRuntime::new(failed_backend);
        let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        assert_eq!(
            failed_runtime.load(&model, &mut artifact, "x86_64").err(),
            Some(RuntimeError::LoadFailed)
        );
        assert_eq!(failed_runtime.backend().load_count, 0);

        let mut runtime = ModelRuntime::new(FakeModelBackend::for_manifest(&model));
        for _ in 0..3 {
            let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
            runtime
                .load(&model, &mut artifact, "x86_64")
                .unwrap()
                .unload()
                .unwrap();
        }
        assert_eq!(runtime.backend().load_count, 3);
        assert_eq!(runtime.backend().unload_count, 3);
        assert_eq!(runtime.resource_report().loaded_model_sessions, 0);
    }

    #[test]
    fn runtime_class_support_is_extensible_without_model_specific_selection() {
        let mut model = manifest();
        let system_one = RuntimeClassId::new("system_one").unwrap();
        let decision = CapabilityId::new("decision.boolean").unwrap();
        model.runtime_class = Some(system_one.clone());
        model.capabilities = vec![decision.clone()];
        let backend = FakeModelBackend::for_manifest(&model);
        assert!(backend.descriptor.runtime_classes.contains(&system_one));
        let runtime = ModelRuntime::new(backend);
        let artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
        assert!(runtime.validate(&model, &artifact, "x86_64").is_ok());

        let mut unsupported = FakeModelBackend::for_manifest(&model);
        unsupported.descriptor.runtime_classes.clear();
        let unsupported_runtime = ModelRuntime::new(unsupported);
        assert_eq!(
            unsupported_runtime.validate(&model, &artifact, "x86_64"),
            Err(RuntimeError::IncompatibleBackend)
        );
    }
}
