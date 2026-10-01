//! Deterministic orchestration test doubles. This module is available to unit
//! tests and to explicitly opted-in `test-support` consumers only.

use alloc::{format, string::String, vec, vec::Vec};

use crate::{
    ArtifactId, ArtifactReadError, ArtifactReference, BackendDescriptor, BackendHealth, BackendId,
    BackendResponse, CancellationToken, CapabilityId, FormatId, GenerationOptions,
    IntegrityMetadata, ModelArtifactReader, ModelBackend, ModelManifest, ModelRequest,
    RuntimeClassId, RuntimeError, RuntimeResourceReport, TextChunkSink, TokenUsage,
};

pub struct MemoryArtifactReader {
    artifact_id: ArtifactId,
    integrity: Option<IntegrityMetadata>,
    bytes: Vec<u8>,
}

impl MemoryArtifactReader {
    pub fn for_manifest(manifest: &ModelManifest, bytes: Vec<u8>) -> Self {
        let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
        Self {
            artifact_id: artifact_id.clone(),
            integrity: manifest.artifact.integrity.clone(),
            bytes,
        }
    }

    pub fn with_metadata(
        artifact_id: ArtifactId,
        integrity: Option<IntegrityMetadata>,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            artifact_id,
            integrity,
            bytes,
        }
    }
}

impl ModelArtifactReader for MemoryArtifactReader {
    fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.integrity.as_ref()
    }

    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        let offset = usize::try_from(offset).map_err(|_| ArtifactReadError::OutOfRange)?;
        if offset > self.bytes.len() {
            return Err(ArtifactReadError::OutOfRange);
        }
        let count = destination.len().min(self.bytes.len() - offset);
        destination[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        Ok(count)
    }
}

#[derive(Clone, Debug)]
pub struct FakeSession {
    model_id: String,
}

#[derive(Clone, Debug)]
pub struct FakeModelBackend {
    pub descriptor: BackendDescriptor,
    pub load_count: usize,
    pub infer_count: usize,
    pub stream_count: usize,
    pub unload_count: usize,
    pub response_output_tokens: u32,
    pub health: BackendHealth,
    pub active_sessions: u32,
    pub resident_memory_bytes: Option<u64>,
    pub max_concurrent_sessions: Option<u32>,
    pub load_error: Option<RuntimeError>,
    pub infer_error: Option<RuntimeError>,
    pub stream_error: Option<RuntimeError>,
    pub unload_error: Option<RuntimeError>,
    pub response_text: Option<String>,
    pub last_system_prompt: Option<String>,
    pub last_options: Option<GenerationOptions>,
}

impl FakeModelBackend {
    pub fn for_manifest(manifest: &ModelManifest) -> Self {
        Self {
            descriptor: BackendDescriptor {
                backend_id: manifest.supported_backends[0].clone(),
                artifact_formats: vec![manifest.artifact.format.clone()],
                runtime_api_versions: vec![manifest.runtime_api_version.clone()],
                architectures: Vec::new(),
                capabilities: manifest.capabilities.clone(),
                runtime_classes: manifest.runtime_class.iter().cloned().collect(),
            },
            load_count: 0,
            infer_count: 0,
            stream_count: 0,
            unload_count: 0,
            response_output_tokens: 1,
            health: BackendHealth::Healthy,
            active_sessions: 0,
            resident_memory_bytes: None,
            max_concurrent_sessions: Some(1),
            load_error: None,
            infer_error: None,
            stream_error: None,
            unload_error: None,
            response_text: None,
            last_system_prompt: None,
            last_options: None,
        }
    }
}

impl ModelBackend for FakeModelBackend {
    type Session = FakeSession;

    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn health(&self) -> BackendHealth {
        self.health
    }

    fn resource_report(&self) -> RuntimeResourceReport {
        RuntimeResourceReport {
            loaded_model_sessions: self.active_sessions,
            active_invocations: 0,
            max_concurrent_sessions: self.max_concurrent_sessions,
            resident_memory_bytes: self.resident_memory_bytes,
        }
    }

    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError> {
        if let Some(error) = self.load_error {
            return Err(error);
        }
        if artifact.is_empty() {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        let mut marker = [0; 1];
        if artifact
            .read_at(0, &mut marker)
            .map_err(|_| RuntimeError::ArtifactUnavailable)?
            == 0
        {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        self.load_count += 1;
        self.active_sessions += 1;
        Ok(FakeSession {
            model_id: manifest.model_id.as_str().into(),
        })
    }

    fn infer(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<BackendResponse, RuntimeError> {
        if let Some(error) = self.infer_error {
            return Err(error);
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        if request.timeout_millis == Some(0) {
            return Err(RuntimeError::Timeout);
        }
        self.infer_count += 1;
        self.last_system_prompt = request.system_prompt.map(String::from);
        self.last_options = Some(request.options);
        Ok(BackendResponse {
            text: self
                .response_text
                .clone()
                .unwrap_or_else(|| format!("fake:{}:{}", session.model_id, request.input)),
            usage: TokenUsage {
                input_tokens: request.input_tokens.unwrap_or(0),
                output_tokens: self.response_output_tokens,
            },
        })
    }

    fn infer_stream(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError> {
        if let Some(error) = self.stream_error {
            return Err(error);
        }
        if request.timeout_millis == Some(0) {
            return Err(RuntimeError::Timeout);
        }
        self.stream_count += 1;
        self.last_system_prompt = request.system_prompt.map(String::from);
        self.last_options = Some(request.options);
        let chunks = ["fake:", session.model_id.as_str(), ":", request.input];
        for chunk in chunks {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            sink.push_chunk(chunk)
                .map_err(RuntimeError::StreamConsumer)?;
        }
        Ok(TokenUsage {
            input_tokens: request.input_tokens.unwrap_or(0),
            output_tokens: self.response_output_tokens,
        })
    }

    fn unload(&mut self, _: Self::Session) -> Result<(), RuntimeError> {
        self.unload_count += 1;
        if let Some(error) = self.unload_error {
            return Err(error);
        }
        self.active_sessions = self.active_sessions.saturating_sub(1);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticCancellation(pub bool);

impl CancellationToken for StaticCancellation {
    fn is_cancelled(&self) -> bool {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NeverCancel;

impl CancellationToken for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub fn simple_backend(backend_id: &str, format: &str, runtime_api: &str) -> BackendDescriptor {
    BackendDescriptor {
        backend_id: BackendId::new(backend_id).expect("valid test backend id"),
        artifact_formats: vec![FormatId::new(format).expect("valid test format")],
        runtime_api_versions: vec![String::from(runtime_api)],
        architectures: Vec::new(),
        capabilities: vec![CapabilityId::new("text.generate").expect("valid test capability")],
        runtime_classes: vec![RuntimeClassId::new("generative_llm").expect("valid test class")],
    }
}
