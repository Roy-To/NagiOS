//! Deterministic orchestration test doubles. This module is available to unit
//! tests and to explicitly opted-in `test-support` consumers only.

use alloc::{format, string::String, vec, vec::Vec};

use crate::{
    ArtifactId, ArtifactReadError, ArtifactReference, BackendDescriptor, BackendId,
    BackendResponse, CancellationToken, FormatId, IntegrityMetadata, ModelArtifactReader,
    ModelBackend, ModelManifest, ModelRequest, RuntimeError, TokenUsage,
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
    pub unload_count: usize,
    pub response_output_tokens: u32,
}

impl FakeModelBackend {
    pub fn for_manifest(manifest: &ModelManifest) -> Self {
        Self {
            descriptor: BackendDescriptor {
                backend_id: manifest.supported_backends[0].clone(),
                artifact_formats: vec![manifest.artifact.format.clone()],
                runtime_api_versions: vec![manifest.runtime_api_version.clone()],
                architectures: Vec::new(),
            },
            load_count: 0,
            infer_count: 0,
            unload_count: 0,
            response_output_tokens: 1,
        }
    }
}

impl ModelBackend for FakeModelBackend {
    type Session = FakeSession;

    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError> {
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
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        if request.timeout_millis == Some(0) {
            return Err(RuntimeError::Timeout);
        }
        self.infer_count += 1;
        Ok(BackendResponse {
            text: format!("fake:{}:{}", session.model_id, request.input),
            usage: TokenUsage {
                input_tokens: request.input_tokens.unwrap_or(0),
                output_tokens: self.response_output_tokens,
            },
        })
    }

    fn unload(&mut self, _: Self::Session) -> Result<(), RuntimeError> {
        self.unload_count += 1;
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
    }
}
