use nagi_model_manager::{
    ArtifactId, CancellationToken, CapabilityId, Fat32ArtifactReader, GenerationOptions,
    GenerativeProvider, ModelArtifactReader, ModelManifest, ModelRequest, ModelRuntime,
    RuntimeError, StructuredOutputSchema, STRUCTURED_GENERATION_CAPABILITY_ID,
};

#[cfg(not(feature = "m20-model-service"))]
#[path = "llama_backend.rs"]
mod llama_backend;
use super::SyscallModelStoreReader;
#[cfg(feature = "m20-model-service")]
use crate::model_service::llama_backend::LlamaCppBackend;
#[cfg(not(feature = "m20-model-service"))]
use llama_backend::LlamaCppBackend;

const MODEL_STORE_SECTORS: u64 = 67_108_864;
const GRANITE_ARTIFACT_ID: &str = "ibm.granite-4.2-3b";
const GRANITE_MODEL_BYTES: u64 = 2_244_011_552;
const STRING_ANSWER_SCHEMA: &[u8] = br#"{"type":"object","properties":{"answer":{"type":"string","minLength":1,"maxLength":256}},"required":["answer"],"additionalProperties":false}"#;

/// Report which acceptance stage failed so a FAIL is diagnosable from the
/// serial log alone.
fn stage_failed(stage: &str, detail: Option<RuntimeError>) -> bool {
    libnagi::console_write(b"Nagi M20 Granite stage FAIL: ");
    libnagi::console_write(stage.as_bytes());
    if let Some(error) = detail {
        libnagi::console_write(alloc::format!(" ({error:?})").as_bytes());
    }
    libnagi::console_write(b"\r\n");
    false
}

pub fn run(model_store_capability: u64) -> bool {
    if model_store_capability == 0 {
        return false;
    }
    let artifact_id = match ArtifactId::new(GRANITE_ARTIFACT_ID) {
        Ok(artifact_id) => artifact_id,
        Err(_) => return false,
    };
    let mut artifact = match Fat32ArtifactReader::open(
        SyscallModelStoreReader(model_store_capability),
        MODEL_STORE_SECTORS,
        artifact_id,
    ) {
        Ok(artifact) if artifact.len() == GRANITE_MODEL_BYTES => artifact,
        _ => return stage_failed("model store artifact", None),
    };
    let manifest = match ModelManifest::parse_json(include_bytes!(
        "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
    )) {
        Ok(manifest) => manifest,
        Err(_) => return stage_failed("manifest", None),
    };
    let schema = match StructuredOutputSchema::parse_json(STRING_ANSWER_SCHEMA) {
        Ok(schema) => schema,
        Err(_) => return false,
    };
    let capability = match CapabilityId::new(STRUCTURED_GENERATION_CAPABILITY_ID) {
        Ok(capability) => capability,
        Err(_) => return false,
    };
    let mut backend = LlamaCppBackend::new();
    if !backend.initialize() {
        return stage_failed("backend initialization", None);
    }
    libnagi::console_write(b"Nagi M20 trace: llama backend initialized\r\n");
    let mut runtime = ModelRuntime::new(backend);
    let mut session = match runtime.load(&manifest, &mut artifact, "x86_64") {
        Ok(session) => session,
        Err(error) => return stage_failed("model load", Some(error)),
    };
    libnagi::console_write(b"Nagi M20 trace: Granite model loaded\r\n");
    let request = ModelRequest {
        request_id: 0x4e41_4749_0020_0001,
        caller: None,
        capability: &capability,
        system_prompt: Some(
            "Answer the user's request concisely. Return only a JSON object with one property named answer whose value is a nonempty string of at most 256 characters.",
        ),
        input: "Say hello in Japanese.",
        input_tokens: None,
        max_output_tokens: 96,
        options: GenerationOptions {
            temperature_milli: Some(0),
            top_p_milli: None,
            seed: Some(20),
        },
        timeout_millis: None,
        structured_output: Some(&schema),
    };
    let response = match session.generate(&request, &NeverCancel) {
        Ok(response) => response,
        Err(error) => return stage_failed("generation", Some(error)),
    };
    if response.text.is_empty() {
        return stage_failed("empty response", None);
    }
    if let Err(error) = session.unload() {
        return stage_failed("unload", Some(error));
    }
    libnagi::console_write(b"Nagi M20 Granite structured response: ");
    libnagi::console_write(response.text.as_bytes());
    libnagi::console_write(b"\r\n");
    true
}

struct NeverCancel;

impl CancellationToken for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}
