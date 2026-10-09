//! These tests prove service orchestration only. They do not execute Granite.
use super::*;
use crate::testing::{FakeModelBackend, FakeSession, MemoryArtifactReader, NeverCancel};
use crate::{BackendDescriptor, BackendHealth, BackendResponse, GenerationOptions, TokenUsage};
use alloc::{format, rc::Rc, vec, vec::Vec};
use core::cell::Cell;
use sha2::{Digest, Sha256};

struct Artifacts {
    opens: Rc<Cell<usize>>,
    bytes: Vec<u8>,
    missing: bool,
}
impl ModelArtifactSource for Artifacts {
    type Reader = MemoryArtifactReader;
    fn open(&mut self, manifest: &ModelManifest) -> Result<Self::Reader, RuntimeError> {
        self.opens.set(self.opens.get() + 1);
        if self.missing {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        Ok(MemoryArtifactReader::for_manifest(
            manifest,
            self.bytes.clone(),
        ))
    }
}

struct TestClock {
    millis: Rc<Cell<u64>>,
    step: u64,
    available: bool,
}
impl ModelServiceClock for TestClock {
    fn now_millis(&self) -> Option<u64> {
        self.available.then(|| self.millis.get())
    }
    fn cooperate(&self) {
        self.millis.set(self.millis.get() + self.step);
    }
}

struct ProbeBackend {
    fake: FakeModelBackend,
    next_error: Cell<Option<RuntimeError>>,
    elapsed: Rc<Cell<u64>>,
    advance_on_infer: u64,
    cancelled: Rc<Cell<bool>>,
    cancel_on_infer: bool,
    unloads: Rc<Cell<usize>>,
}
impl ModelBackend for ProbeBackend {
    type Session = FakeSession;
    fn descriptor(&self) -> &BackendDescriptor {
        self.fake.descriptor()
    }
    fn health(&self) -> BackendHealth {
        self.fake.health()
    }
    fn resource_report(&self) -> RuntimeResourceReport {
        self.fake.resource_report()
    }
    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError> {
        self.fake.load(manifest, artifact)
    }
    fn infer(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<BackendResponse, RuntimeError> {
        if let Some(error) = self.next_error.take() {
            return Err(error);
        }
        let result = self.fake.infer(session, request, cancellation);
        self.elapsed.set(self.elapsed.get() + self.advance_on_infer);
        if self.cancel_on_infer {
            self.cancelled.set(true);
        }
        result
    }
    fn infer_stream(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError> {
        self.fake.infer_stream(session, request, cancellation, sink)
    }
    fn unload(&mut self, session: Self::Session) -> Result<(), RuntimeError> {
        self.unloads.set(self.unloads.get() + 1);
        self.fake.unload(session)
    }
}

impl CancellationToken for Cell<bool> {
    fn is_cancelled(&self) -> bool {
        self.get()
    }
}

type Service = LazyModelService<ProbeBackend, Artifacts, TestClock>;
fn budget() -> ResourceBudget {
    ResourceBudget {
        available_ram_bytes: 4 * 1024 * 1024 * 1024,
        available_storage_bytes: 4 * 1024 * 1024 * 1024,
        cpu_cores: 1,
        gpu_available: false,
    }
}
fn fixture(bytes: Vec<u8>) -> Service {
    fixture_backend(bytes, |_| {})
}
fn fixture_backend(bytes: Vec<u8>, configure: impl FnOnce(&mut ProbeBackend)) -> Service {
    let mut manifest =
        ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json")).unwrap();
    // Synthetic artifact bytes for the explicit orchestration backend.
    manifest.artifact.size_bytes = Some(bytes.len() as u64);
    manifest.artifact.integrity.as_mut().unwrap().digest = format!("{:x}", Sha256::digest(&bytes));
    let millis = Rc::new(Cell::new(0));
    let mut backend = ProbeBackend {
        fake: FakeModelBackend::for_manifest(&manifest),
        next_error: Cell::new(None),
        elapsed: millis.clone(),
        advance_on_infer: 0,
        cancelled: Rc::new(Cell::new(false)),
        cancel_on_infer: false,
        unloads: Rc::new(Cell::new(0)),
    };
    configure(&mut backend);
    let mut record = ModelStoreRecord::discovered(manifest).unwrap();
    record
        .acknowledge_terms(record.license().terms_reference.clone().unwrap().as_str())
        .unwrap();
    Service::new(
        record,
        backend,
        Artifacts {
            opens: Rc::new(Cell::new(0)),
            bytes,
            missing: false,
        },
        TestClock {
            millis,
            step: 0,
            available: true,
        },
        budget(),
        ModelServiceLimits {
            max_input_bytes: 4096,
            max_output_bytes: 8192,
            max_output_tokens: 256,
            max_request_millis: 1000,
        },
    )
    .unwrap()
}
fn service() -> Service {
    fixture(vec![1, 2, 3, 4])
}
fn request(capability: &CapabilityId) -> ModelRequest<'_> {
    ModelRequest {
        request_id: 1,
        caller: None,
        capability,
        system_prompt: Some("Be concise."),
        input: "通常セッションからこんにちは",
        input_tokens: None,
        max_output_tokens: 64,
        options: GenerationOptions::default(),
        timeout_millis: None,
        structured_output: None,
    }
}
fn capability() -> CapabilityId {
    CapabilityId::new("text.generate").unwrap()
}

#[test]
fn construction_and_selection_do_not_open_or_load_a_model() {
    let service = service();
    let capability = capability();
    let role = RoleId::new("standard").unwrap();
    assert_eq!(
        service
            .select(&capability, Some(&role), None)
            .unwrap()
            .as_str(),
        "ibm.granite-4.2-3b"
    );
    assert_eq!(service.state(), LifecycleState::Unloaded);
    assert_eq!(service.artifacts.opens.get(), 0);
    assert_eq!(service.backend().fake.load_count, 0);
    assert_eq!(service.resource_report().loaded_model_sessions, 0);
}

#[test]
fn ordinary_requests_use_their_actual_text_and_reuse_one_resident_session() {
    let mut service = service();
    let cap = capability();
    let first = service.generate(&request(&cap), &NeverCancel).unwrap();
    assert!(first.text.contains("通常セッションからこんにちは"));
    let second_request = ModelRequest {
        request_id: 2,
        input: "another independent request",
        ..request(&cap)
    };
    let second = service.generate(&second_request, &NeverCancel).unwrap();
    assert_eq!(second.request_id, 2);
    assert!(second.text.ends_with("another independent request"));
    assert_eq!(service.artifacts.opens.get(), 1);
    assert_eq!(service.backend().fake.load_count, 1);
    assert_eq!(service.backend().fake.infer_count, 2);
    assert_eq!(service.resource_report().loaded_model_sessions, 1);
    assert_eq!(service.state(), LifecycleState::Ready);
    service.unload().unwrap();
    assert_eq!(service.backend().fake.unload_count, 1);
    assert_eq!(service.resource_report().loaded_model_sessions, 0);
}

#[test]
fn terms_are_required_and_only_the_exact_reference_is_accepted() {
    let mut service = service();
    service.model = ModelStoreRecord::discovered(service.model.manifest().clone()).unwrap();
    let cap = capability();
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(ModelServiceError::Store(
            StoreError::LicenseAcknowledgementRequired
        ))
    );
    assert_eq!(service.artifacts.opens.get(), 0);
    assert_eq!(
        service.acknowledge_terms("a different reference"),
        Err(ModelServiceError::Store(
            StoreError::InvalidLicenseReference
        ))
    );
    let reference = alloc::string::String::from(service.terms_reference().unwrap());
    service.acknowledge_terms(&reference).unwrap();
    assert!(service.generate(&request(&cap), &NeverCancel).is_ok());
}

#[test]
fn missing_or_corrupted_artifacts_never_invoke_the_backend() {
    let cap = capability();
    let mut missing = service();
    missing.artifacts.missing = true;
    assert_eq!(
        missing.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::ArtifactUnavailable.into())
    );
    let mut corrupted = service();
    corrupted.artifacts.bytes[0] ^= 1;
    assert_eq!(
        corrupted.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::IntegrityMismatch.into())
    );
    assert_eq!(corrupted.backend().fake.load_count, 0);
    assert_eq!(corrupted.state(), LifecycleState::Failed);
    corrupted.artifacts.bytes[0] ^= 1;
    assert!(corrupted.generate(&request(&cap), &NeverCancel).is_ok());
}

#[test]
fn incompatible_capability_role_and_manual_model_selection_fail_closed() {
    let service = service();
    assert_eq!(
        service.select(&CapabilityId::new("speech.stt").unwrap(), None, None),
        Err(ModelServiceError::ModelUnavailable)
    );
    assert_eq!(
        service.select(&capability(), Some(&RoleId::new("lite").unwrap()), None),
        Err(ModelServiceError::ModelUnavailable)
    );
    assert_eq!(
        service.select(
            &capability(),
            None,
            Some(&ModelId::new("qwen.qwen3-4b").unwrap())
        ),
        Err(ModelServiceError::ModelUnavailable)
    );
    assert_eq!(service.artifacts.opens.get(), 0);
}

#[test]
fn low_resource_budget_unloads_and_prevents_reload_until_resources_recover() {
    let mut service = service();
    let cap = capability();
    service.generate(&request(&cap), &NeverCancel).unwrap();
    service
        .set_resource_budget(ResourceBudget {
            available_ram_bytes: 1,
            ..budget()
        })
        .unwrap();
    assert_eq!(service.resource_report().loaded_model_sessions, 0);
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(ModelServiceError::ResourceLimit)
    );
    assert_eq!(service.artifacts.opens.get(), 1);
    service.set_resource_budget(budget()).unwrap();
    service.generate(&request(&cap), &NeverCancel).unwrap();
    assert_eq!(service.backend().fake.load_count, 2);
}

#[test]
fn invalid_input_and_limits_are_rejected_before_artifact_io() {
    let mut service = service();
    let cap = capability();
    for invalid in [
        ModelRequest {
            input: "",
            ..request(&cap)
        },
        ModelRequest {
            input: "hello\0world",
            ..request(&cap)
        },
        ModelRequest {
            request_id: 0,
            ..request(&cap)
        },
        ModelRequest {
            max_output_tokens: 257,
            ..request(&cap)
        },
        ModelRequest {
            options: GenerationOptions {
                top_p_milli: Some(0),
                ..Default::default()
            },
            ..request(&cap)
        },
    ] {
        assert_eq!(
            service.generate(&invalid, &NeverCancel),
            Err(RuntimeError::InvalidRequest.into())
        );
    }
    assert_eq!(
        service.generate(
            &ModelRequest {
                timeout_millis: Some(0),
                ..request(&cap)
            },
            &NeverCancel
        ),
        Err(RuntimeError::Timeout.into())
    );
    assert_eq!(service.artifacts.opens.get(), 0);
}

#[test]
fn pre_cancelled_request_does_not_load_and_hashing_can_be_cancelled() {
    let mut service = fixture(vec![1; 20000]);
    let cap = capability();
    assert_eq!(
        service.generate(&request(&cap), &Cell::new(true)),
        Err(RuntimeError::Cancelled.into())
    );
    assert_eq!(service.artifacts.opens.get(), 0);
    struct CancelDuringRead(Cell<usize>);
    impl CancellationToken for CancelDuringRead {
        fn is_cancelled(&self) -> bool {
            self.0.set(self.0.get() + 1);
            self.0.get() >= 3
        }
    }
    assert_eq!(
        service.generate(&request(&cap), &CancelDuringRead(Cell::new(0))),
        Err(RuntimeError::Cancelled.into())
    );
    assert_eq!(service.backend().fake.load_count, 0);
}

#[test]
fn deadline_covers_hashing_and_does_not_use_a_host_clock() {
    let cap = capability();
    let mut service = fixture(vec![1; 20000]);
    service.clock.available = false;
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(ModelServiceError::ClockUnavailable)
    );
    assert_eq!(service.artifacts.opens.get(), 0);
    service.clock.available = true;
    service.clock.step = 1;
    assert_eq!(
        service.generate(
            &ModelRequest {
                timeout_millis: Some(3),
                ..request(&cap)
            },
            &NeverCancel
        ),
        Err(RuntimeError::Timeout.into())
    );
    assert_eq!(service.backend().fake.load_count, 0);
}

#[test]
fn deadline_and_cancellation_after_inference_discard_output_and_unload() {
    let cap = capability();
    let mut timed = fixture_backend(vec![1], |backend| backend.advance_on_infer = 1000);
    assert_eq!(
        timed.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::Timeout.into())
    );
    assert_eq!(timed.resource_report().loaded_model_sessions, 0);
    assert_eq!(timed.backend().fake.infer_count, 1);
    let mut cancelled = fixture_backend(vec![1], |backend| backend.cancel_on_infer = true);
    let signal = cancelled.backend().cancelled.clone();
    assert_eq!(
        cancelled.generate(&request(&cap), signal.as_ref()),
        Err(RuntimeError::Cancelled.into())
    );
    assert_eq!(cancelled.resource_report().loaded_model_sessions, 0);
}

#[test]
fn inference_failure_releases_context_and_retry_loads_a_fresh_session() {
    let mut service = service();
    service
        .backend()
        .next_error
        .set(Some(RuntimeError::InferenceFailed));
    let cap = capability();
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::InferenceFailed.into())
    );
    assert_eq!(service.state(), LifecycleState::Failed);
    assert_eq!(service.resource_report().loaded_model_sessions, 0);
    service.generate(&request(&cap), &NeverCancel).unwrap();
    assert_eq!(service.backend().fake.load_count, 2);
    assert_eq!(service.backend().fake.unload_count, 1);
    assert_eq!(service.state(), LifecycleState::Ready);
}

#[test]
fn blank_oversized_and_out_of_token_bound_responses_are_never_presented() {
    let cap = capability();
    for bad_text in [alloc::string::String::from("   "), "x".repeat(8193)] {
        let mut service = fixture_backend(vec![1], |backend| {
            backend.fake.response_text = Some(bad_text)
        });
        assert_eq!(
            service.generate(&request(&cap), &NeverCancel),
            Err(RuntimeError::InvalidBackendResponse.into())
        );
        assert_eq!(service.resource_report().loaded_model_sessions, 0);
    }
    let mut service = fixture_backend(vec![1], |backend| backend.fake.response_output_tokens = 65);
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::InvalidBackendResponse.into())
    );
}

#[test]
fn load_failure_leaves_no_resident_session() {
    let mut service = fixture_backend(vec![1], |backend| {
        backend.fake.load_error = Some(RuntimeError::LoadFailed)
    });
    let cap = capability();
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::LoadFailed.into())
    );
    assert_eq!(service.state(), LifecycleState::Failed);
    assert_eq!(service.resource_report().loaded_model_sessions, 0);
}

#[test]
fn unload_failure_disables_the_service_instead_of_loading_another_context() {
    let mut service = fixture_backend(vec![1], |backend| {
        backend.fake.unload_error = Some(RuntimeError::UnloadFailed)
    });
    let cap = capability();
    service.generate(&request(&cap), &NeverCancel).unwrap();
    assert_eq!(service.unload(), Err(RuntimeError::UnloadFailed.into()));
    assert_eq!(service.state(), LifecycleState::Disabled);
    assert_eq!(
        service.generate(&request(&cap), &NeverCancel),
        Err(RuntimeError::BackendUnavailable.into())
    );
    assert_eq!(service.backend().fake.load_count, 1);
}

#[test]
fn dropping_the_service_unloads_once_and_unload_is_idempotent() {
    let mut service = service();
    let unloads = service.backend().unloads.clone();
    let cap = capability();
    service.generate(&request(&cap), &NeverCancel).unwrap();
    service.unload().unwrap();
    service.unload().unwrap();
    assert_eq!(unloads.get(), 1);
    service.generate(&request(&cap), &NeverCancel).unwrap();
    drop(service);
    assert_eq!(unloads.get(), 2);
}

#[test]
fn streaming_unavailable_does_not_load_and_provider_view_uses_checked_service() {
    struct Sink;
    impl TextChunkSink for Sink {
        fn push_chunk(&mut self, _: &str) -> Result<(), crate::StreamConsumerError> {
            Ok(())
        }
    }
    let mut service = service();
    let cap = capability();
    assert_eq!(
        GenerativeProvider::generate_stream(&mut service, &request(&cap), &NeverCancel, &mut Sink),
        Err(RuntimeError::UnsupportedCapability)
    );
    assert_eq!(service.artifacts.opens.get(), 0);
    let result = GenerativeProvider::generate(&mut service, &request(&cap), &NeverCancel).unwrap();
    assert_eq!(result.model_id, *GenerativeProvider::model_id(&service));
}
