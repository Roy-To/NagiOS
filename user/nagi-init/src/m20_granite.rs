use alloc::{
    ffi::CString,
    string::{String, ToString},
    vec,
};
use core::ffi::{c_char, c_void};

use nagi_model_manager::{
    ArtifactId, BackendDescriptor, BackendHealth, BackendId, BackendResponse, CancellationToken,
    CapabilityId, Fat32ArtifactReader, FormatId, GenerationOptions, GenerativeProvider,
    ModelArtifactReader, ModelBackend, ModelManifest, ModelRequest, ModelRuntime, RuntimeClassId,
    RuntimeError, RuntimeResourceReport, StructuredOutputSchema, TokenUsage,
    STRUCTURED_GENERATION_CAPABILITY_ID,
};

use super::SyscallModelStoreReader;

const MODEL_STORE_SECTORS: u64 = 67_108_864;
const GRANITE_MODEL_ID: &str = "ibm.granite-4.2-3b";
const GRANITE_ARTIFACT_ID: &str = "ibm.granite-4.2-3b";
const GRANITE_MODEL_BYTES: u64 = 2_244_011_552;
const GRANITE_CONTEXT_TOKENS: u32 = 4096;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const STRING_ANSWER_SCHEMA: &[u8] = br#"{"type":"object","properties":{"answer":{"type":"string","minLength":1,"maxLength":256}},"required":["answer"],"additionalProperties":false}"#;
const STRING_ANSWER_CANONICAL_SCHEMA: &str = "{\"additionalProperties\":false,\"properties\":{\"answer\":{\"maxLength\":256,\"minLength\":1,\"type\":\"string\"}},\"required\":[\"answer\"],\"type\":\"object\"}";
const STRING_ANSWER_GRAMMAR: &[u8] = b"root ::= \"{\\\"answer\\\":\" json-string \"}\"\njson-string ::= \"\\\"\" json-char{1,256} \"\\\"\"\njson-char ::= [^\"\\\\\\x7F\\x00-\\x1F] | \"\\\\\" ([\"\\\\bfnrt] | \"u\" [0-9a-fA-F]{4})\n\0";

unsafe extern "C" {
    fn nagi_m20_llama_backend_initialize() -> i32;
    fn nagi_m20_llama_load_from_fd(fd: i32, context_tokens: u32, thread_count: i32) -> *mut c_void;
    fn nagi_m20_llama_set_cancel_callback(
        handle: *mut c_void,
        callback: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
        callback_context: *mut c_void,
    );
    fn nagi_m20_llama_generate(
        handle: *mut c_void,
        system_prompt: *const c_char,
        input: *const c_char,
        grammar: *const c_char,
        max_output_tokens: i32,
        temperature: f32,
        top_p: f32,
        seed: u32,
        output: *mut c_char,
        output_capacity: usize,
        output_size: *mut usize,
        input_tokens: *mut u32,
        output_tokens: *mut u32,
    ) -> i32;
    fn nagi_m20_llama_free(handle: *mut c_void);
}

struct ModelStoreReadContext<'a> {
    artifact: &'a mut dyn ModelArtifactReader,
    failed: bool,
}

unsafe extern "C" fn read_artifact_at(
    context: *mut c_void,
    offset: u64,
    destination: *mut u8,
    length: usize,
) -> isize {
    if context.is_null() || (destination.is_null() && length != 0) {
        return -1;
    }
    let context = unsafe { &mut *context.cast::<ModelStoreReadContext<'_>>() };
    if context.failed || length == 0 {
        return 0;
    }
    if offset >= ModelArtifactReader::len(context.artifact) {
        return 0;
    }
    let destination = unsafe { core::slice::from_raw_parts_mut(destination, length) };
    match ModelArtifactReader::read_at(context.artifact, offset, destination) {
        Ok(read) if read != 0 && read <= length => read as isize,
        _ => {
            context.failed = true;
            -1
        }
    }
}

struct LlamaSession {
    handle: *mut c_void,
}

struct LlamaCppBackend {
    descriptor: BackendDescriptor,
    initialized: bool,
    loaded_sessions: u32,
    active_invocations: u32,
}

impl LlamaCppBackend {
    fn new() -> Self {
        Self {
            descriptor: BackendDescriptor {
                backend_id: BackendId::new("llama_cpp").expect("static backend ID"),
                artifact_formats: vec![FormatId::new("gguf").expect("static format ID")],
                runtime_api_versions: vec![String::from("nagi.ai/1")],
                architectures: vec![String::from("x86_64")],
                capabilities: vec![
                    CapabilityId::new("text.generate").expect("static capability ID"),
                    CapabilityId::new(STRUCTURED_GENERATION_CAPABILITY_ID)
                        .expect("static capability ID"),
                ],
                runtime_classes: vec![
                    RuntimeClassId::new("generative_llm").expect("static runtime class ID")
                ],
            },
            initialized: unsafe { nagi_m20_llama_backend_initialize() != 0 },
            loaded_sessions: 0,
            active_invocations: 0,
        }
    }
}

impl ModelBackend for LlamaCppBackend {
    type Session = LlamaSession;

    fn descriptor(&self) -> &BackendDescriptor {
        &self.descriptor
    }

    fn health(&self) -> BackendHealth {
        if self.initialized {
            BackendHealth::Healthy
        } else {
            BackendHealth::Unavailable
        }
    }

    fn resource_report(&self) -> RuntimeResourceReport {
        RuntimeResourceReport {
            loaded_model_sessions: self.loaded_sessions,
            active_invocations: self.active_invocations,
            max_concurrent_sessions: Some(1),
            resident_memory_bytes: None,
        }
    }

    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError> {
        if !self.initialized || self.loaded_sessions != 0 {
            return Err(RuntimeError::BackendUnavailable);
        }
        if manifest.model_id.as_str() != GRANITE_MODEL_ID
            || artifact.artifact_id().as_str() != GRANITE_ARTIFACT_ID
            || artifact.len() != GRANITE_MODEL_BYTES
        {
            return Err(RuntimeError::ArtifactMismatch);
        }

        let length = artifact.len();
        let mut reader = ModelStoreReadContext {
            artifact,
            failed: false,
        };
        let fd = unsafe {
            nagi_posix::open_readonly_callback(
                (&mut reader as *mut ModelStoreReadContext<'_>).cast(),
                length,
                read_artifact_at,
            )
        }
        .map_err(|_| RuntimeError::ArtifactUnavailable)?;
        let handle = unsafe { nagi_m20_llama_load_from_fd(fd, GRANITE_CONTEXT_TOKENS, 2) };
        if handle.is_null() || reader.failed {
            if !handle.is_null() {
                unsafe { nagi_m20_llama_free(handle) };
            }
            return Err(RuntimeError::LoadFailed);
        }
        self.loaded_sessions = 1;
        Ok(LlamaSession { handle })
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
        if request.max_output_tokens > i32::MAX as u32 {
            return Err(RuntimeError::InvalidRequest);
        }
        let input =
            CString::new(request.input.as_bytes()).map_err(|_| RuntimeError::InvalidRequest)?;
        let system_prompt = request
            .system_prompt
            .map(|prompt| CString::new(prompt.as_bytes()))
            .transpose()
            .map_err(|_| RuntimeError::InvalidRequest)?;
        let grammar = match request.structured_output {
            Some(schema) if schema.document().to_string() == STRING_ANSWER_CANONICAL_SCHEMA => {
                STRING_ANSWER_GRAMMAR.as_ptr().cast::<c_char>()
            }
            Some(_) => return Err(RuntimeError::UnsupportedCapability),
            None => core::ptr::null(),
        };
        let temperature = request
            .options
            .temperature_milli
            .map_or(0.0, |value| value as f32 / 1000.0);
        let top_p = request
            .options
            .top_p_milli
            .map_or(1.0, |value| value as f32 / 1000.0);
        let seed = request.options.seed.unwrap_or(0) as u32;
        let mut output = [0u8; MAX_RESPONSE_BYTES];
        let mut output_size = 0usize;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        let mut cancel_context = CancellationContext { cancellation };
        unsafe {
            nagi_m20_llama_set_cancel_callback(
                session.handle,
                Some(check_cancelled),
                (&mut cancel_context as *mut CancellationContext<'_>).cast(),
            );
        }
        self.active_invocations = 1;
        let status = unsafe {
            nagi_m20_llama_generate(
                session.handle,
                system_prompt
                    .as_ref()
                    .map_or(core::ptr::null(), |prompt| prompt.as_ptr()),
                input.as_ptr(),
                grammar,
                request.max_output_tokens as i32,
                temperature,
                top_p,
                seed,
                output.as_mut_ptr().cast(),
                output.len(),
                &mut output_size,
                &mut input_tokens,
                &mut output_tokens,
            )
        };
        unsafe {
            nagi_m20_llama_set_cancel_callback(session.handle, None, core::ptr::null_mut());
        }
        self.active_invocations = 0;
        match status {
            0 => {}
            2 => return Err(RuntimeError::ContextLimit),
            3 if cancellation.is_cancelled() => return Err(RuntimeError::Cancelled),
            3 => return Err(RuntimeError::InferenceFailed),
            4 => return Err(RuntimeError::UnsupportedCapability),
            5 => return Err(RuntimeError::InvalidBackendResponse),
            _ => return Err(RuntimeError::InferenceFailed),
        }
        if output_size == 0 || output_size > output.len() {
            return Err(RuntimeError::InvalidBackendResponse);
        }
        let text = core::str::from_utf8(&output[..output_size])
            .map_err(|_| RuntimeError::InvalidBackendResponse)?;
        Ok(BackendResponse {
            text: String::from(text),
            usage: TokenUsage {
                input_tokens,
                output_tokens,
            },
        })
    }

    fn infer_stream(
        &mut self,
        _session: &mut Self::Session,
        _request: &ModelRequest<'_>,
        _cancellation: &dyn CancellationToken,
        _sink: &mut dyn nagi_model_manager::TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError> {
        Err(RuntimeError::UnsupportedCapability)
    }

    fn unload(&mut self, session: Self::Session) -> Result<(), RuntimeError> {
        if session.handle.is_null() || self.loaded_sessions == 0 {
            return Err(RuntimeError::UnloadFailed);
        }
        unsafe { nagi_m20_llama_free(session.handle) };
        self.loaded_sessions = 0;
        Ok(())
    }
}

struct CancellationContext<'a> {
    cancellation: &'a dyn CancellationToken,
}

unsafe extern "C" fn check_cancelled(context: *mut c_void) -> bool {
    if context.is_null() {
        return true;
    }
    let context = unsafe { &*context.cast::<CancellationContext<'_>>() };
    context.cancellation.is_cancelled()
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
        _ => return false,
    };
    let manifest = match ModelManifest::parse_json(include_bytes!(
        "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
    )) {
        Ok(manifest) => manifest,
        Err(_) => return false,
    };
    let schema = match StructuredOutputSchema::parse_json(STRING_ANSWER_SCHEMA) {
        Ok(schema) => schema,
        Err(_) => return false,
    };
    let capability = match CapabilityId::new(STRUCTURED_GENERATION_CAPABILITY_ID) {
        Ok(capability) => capability,
        Err(_) => return false,
    };
    let backend = LlamaCppBackend::new();
    let mut runtime = ModelRuntime::new(backend);
    let mut session = match runtime.load(&manifest, &mut artifact, "x86_64") {
        Ok(session) => session,
        Err(_) => return false,
    };
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
        Err(_) => return false,
    };
    if response.text.is_empty() || session.unload().is_err() {
        return false;
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
