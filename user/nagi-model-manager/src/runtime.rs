use alloc::{string::String, vec::Vec};
use core::fmt;

use nagi_model::AppId;

use crate::{
    ArtifactId, ArtifactReference, BackendDescriptor, BackendId, CapabilityId, IntegrityMetadata,
    ManifestError, ModelId, ModelManifest, ProviderId,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelRequest<'a> {
    pub request_id: u64,
    pub caller: Option<AppId>,
    pub capability: &'a CapabilityId,
    pub input: &'a str,
    pub input_tokens: Option<u32>,
    pub max_output_tokens: u32,
    pub timeout_millis: Option<u64>,
}

pub trait CancellationToken {
    fn is_cancelled(&self) -> bool;
}

pub trait GenerativeProvider {
    fn model_id(&self) -> &ModelId;

    fn generate(
        &mut self,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<ModelResponse, RuntimeError>;
}

pub trait ModelBackend {
    type Session;

    fn descriptor(&self) -> &BackendDescriptor;

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

    pub fn load<'runtime>(
        &'runtime mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
        target_architecture: &str,
    ) -> Result<LoadedSession<'runtime, B>, RuntimeError> {
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
        if !manifest
            .supported_backends
            .iter()
            .any(|id| id == &self.backend.descriptor().backend_id)
            || !self
                .backend
                .descriptor()
                .runtime_api_versions
                .iter()
                .any(|version| version == &manifest.runtime_api_version)
            || !self
                .backend
                .descriptor()
                .artifact_formats
                .iter()
                .any(|format| format == &manifest.artifact.format)
            || (!self.backend.descriptor().architectures.is_empty()
                && !self
                    .backend
                    .descriptor()
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
            || (!manifest.compatibility.architectures.is_empty()
                && !manifest
                    .compatibility
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
        {
            return Err(RuntimeError::IncompatibleBackend);
        }
        let session = self.backend.load(manifest, artifact)?;
        Ok(LoadedSession {
            backend: &mut self.backend,
            session: Some(session),
            model_id: manifest.model_id.clone(),
            provider_id: manifest.provider.provider_id.clone(),
            capabilities: manifest.capabilities.clone(),
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
    max_input_tokens: u32,
    max_output_tokens: u32,
}

impl<B: ModelBackend> LoadedSession<'_, B> {
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
        if self.session.is_none() {
            return Err(RuntimeError::SessionClosed);
        }
        if request.max_output_tokens == 0 || request.max_output_tokens > self.max_output_tokens {
            return Err(RuntimeError::InvalidRequest);
        }
        if !self
            .capabilities
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
    use crate::{ExecutionScope, IntegrityMetadata, ModelManifest};
    use alloc::{format, vec};

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
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
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
            input: "hello",
            input_tokens: Some(1),
            max_output_tokens: 64,
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
}
