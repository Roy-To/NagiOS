//! Ownership-contract tests only: real heap allocations and RAII are observed.
//! The busy release is caused by a live resource lease, not a pre-native error
//! switch. This is not Granite inference or a natural native free failure.

use crate::testing::{FakeModelBackend, FakeSession, MemoryArtifactReader, NeverCancel};
use crate::*;
use alloc::{boxed::Box, format, rc::Rc, vec};
use core::cell::{Cell, RefCell};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Events {
    allocations: Cell<usize>,
    releases: Cell<usize>,
    owner_drops: Cell<usize>,
    attempts: Cell<usize>,
    handle: Cell<usize>,
    attempted_handle: Cell<usize>,
    backend_drops: Cell<usize>,
    owners_at_backend_drop: Cell<usize>,
}

struct HeapResource {
    bytes: Box<[u8; 64]>,
    events: Rc<Events>,
}

impl Drop for HeapResource {
    fn drop(&mut self) {
        self.events.releases.set(self.events.releases.get() + 1);
    }
}

struct ResourceSession {
    metadata: FakeSession,
    resource: Option<Rc<HeapResource>>,
    events: Rc<Events>,
}

impl Drop for ResourceSession {
    fn drop(&mut self) {
        self.events
            .owner_drops
            .set(self.events.owner_drops.get() + 1);
    }
}

struct ResourceBackend {
    fake: FakeModelBackend,
    events: Rc<Events>,
    lease: Rc<RefCell<Option<Rc<HeapResource>>>>,
}

impl Drop for ResourceBackend {
    fn drop(&mut self) {
        self.events
            .backend_drops
            .set(self.events.backend_drops.get() + 1);
        self.events
            .owners_at_backend_drop
            .set(self.events.owner_drops.get());
    }
}

impl ModelBackend for ResourceBackend {
    type Session = ResourceSession;

    fn descriptor(&self) -> &BackendDescriptor {
        self.fake.descriptor()
    }

    fn health(&self) -> BackendHealth {
        self.fake.health()
    }

    fn resource_report(&self) -> RuntimeResourceReport {
        RuntimeResourceReport {
            loaded_model_sessions: (self.events.allocations.get() - self.events.releases.get())
                as u32,
            ..self.fake.resource_report()
        }
    }

    fn load(
        &mut self,
        manifest: &ModelManifest,
        artifact: &mut dyn ModelArtifactReader,
    ) -> Result<Self::Session, RuntimeError> {
        if self.events.allocations.get() != self.events.releases.get() {
            return Err(RuntimeError::BackendUnavailable);
        }
        let metadata = self.fake.load(manifest, artifact)?;
        let resource = Rc::new(HeapResource {
            bytes: Box::new([0x5a; 64]),
            events: self.events.clone(),
        });
        self.events
            .allocations
            .set(self.events.allocations.get() + 1);
        self.events.handle.set(resource.bytes.as_ptr() as usize);
        *self.lease.borrow_mut() = Some(resource.clone());
        Ok(ResourceSession {
            metadata,
            resource: Some(resource),
            events: self.events.clone(),
        })
    }

    fn infer(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<BackendResponse, RuntimeError> {
        self.fake
            .infer(&mut session.metadata, request, cancellation)
    }

    fn infer_stream(
        &mut self,
        session: &mut Self::Session,
        request: &ModelRequest<'_>,
        cancellation: &dyn CancellationToken,
        sink: &mut dyn TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError> {
        self.fake
            .infer_stream(&mut session.metadata, request, cancellation, sink)
    }

    fn unload(&mut self, mut session: Self::Session) -> Result<(), RuntimeError> {
        self.unload_resident(&mut session)
    }

    fn unload_resident(&mut self, session: &mut Self::Session) -> Result<(), RuntimeError> {
        self.events.attempts.set(self.events.attempts.get() + 1);
        let resource = session
            .resource
            .as_ref()
            .ok_or(RuntimeError::SessionClosed)?;
        self.events
            .attempted_handle
            .set(resource.bytes.as_ptr() as usize);
        // Releasing a shared resource would invalidate a still-active lease.
        if Rc::strong_count(resource) != 1 {
            return Err(RuntimeError::UnloadFailed);
        }
        self.fake.unload_resident(&mut session.metadata)?;
        drop(session.resource.take());
        Ok(())
    }
}

struct Fixture {
    model: ModelManifest,
    backend: ResourceBackend,
    events: Rc<Events>,
    lease: Rc<RefCell<Option<Rc<HeapResource>>>>,
}

fn fixture() -> Fixture {
    let mut manifest =
        ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json")).unwrap();
    // Tiny, explicitly test-only artifact; no weights or native inference.
    manifest.artifact.size_bytes = Some(1);
    manifest.artifact.integrity.as_mut().unwrap().digest = format!("{:x}", Sha256::digest([1]));
    let events = Rc::new(Events::default());
    let lease = Rc::new(RefCell::new(None));
    let backend = ResourceBackend {
        fake: FakeModelBackend::for_manifest(&manifest),
        events: events.clone(),
        lease: lease.clone(),
    };
    Fixture {
        model: manifest,
        backend,
        events,
        lease,
    }
}

fn request(capability: &CapabilityId) -> ModelRequest<'_> {
    ModelRequest {
        request_id: 1,
        caller: None,
        capability,
        system_prompt: None,
        input: "ownership probe",
        input_tokens: None,
        max_output_tokens: 8,
        options: GenerationOptions::default(),
        timeout_millis: None,
        structured_output: None,
    }
}

struct RejectSink;
impl TextChunkSink for RejectSink {
    fn push_chunk(&mut self, _: &str) -> Result<(), StreamConsumerError> {
        panic!("quarantined session exposed output")
    }
}

#[test]
fn resident_failed_unload_keeps_the_actual_heap_owner_until_retry() {
    let Fixture {
        model,
        backend,
        events,
        lease,
    } = fixture();
    let mut runtime = ResidentModelRuntime::new(backend);
    let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
    runtime.load(&model, &mut artifact, "x86_64").unwrap();
    let handle = events.handle.get();
    assert_ne!(handle, 0);

    assert_eq!(runtime.unload(), Err(RuntimeError::UnloadFailed));
    assert_eq!(events.attempted_handle.get(), handle);
    assert_eq!(events.releases.get(), 0);
    assert_eq!(
        events.owner_drops.get(),
        0,
        "unload Err destroyed the session owner"
    );
    assert_eq!(runtime.model_id(), Some(&model.model_id));
    assert_eq!(lease.borrow().as_ref().unwrap().bytes[0], 0x5a);
    assert_eq!(runtime.health(), BackendHealth::Unavailable);
    assert_eq!(
        runtime.load(&model, &mut artifact, "x86_64"),
        Err(RuntimeError::BackendUnavailable)
    );
    let capability = CapabilityId::new("text.generate").unwrap();
    assert_eq!(
        runtime.generate(&request(&capability), &NeverCancel),
        Err(RuntimeError::BackendUnavailable)
    );
    assert_eq!(
        runtime.generate_stream(&request(&capability), &NeverCancel, &mut RejectSink),
        Err(RuntimeError::BackendUnavailable)
    );
    assert_eq!(runtime.backend().fake.infer_count, 0);

    drop(lease.borrow_mut().take());
    assert_eq!(
        events.releases.get(),
        0,
        "runtime must still own the same allocation"
    );
    runtime.unload().unwrap();
    assert_eq!(events.attempted_handle.get(), handle);
    assert_eq!(events.releases.get(), 1);
    assert_eq!(events.owner_drops.get(), 1);
    assert_eq!(events.attempts.get(), 2);
    assert_eq!(runtime.resource_report().loaded_model_sessions, 0);
    assert_eq!(runtime.health(), BackendHealth::Unavailable);
    assert_eq!(
        runtime.load(&model, &mut artifact, "x86_64"),
        Err(RuntimeError::BackendUnavailable)
    );
    runtime.unload().unwrap();
    assert_eq!(events.releases.get(), 1);
    assert_eq!(events.attempts.get(), 2);
}

#[test]
fn legacy_loaded_session_preserves_consuming_unload_contract() {
    let Fixture {
        model,
        backend,
        events,
        lease,
    } = fixture();
    let mut runtime = ModelRuntime::new(backend);
    let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
    let session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
    assert_eq!(session.unload(), Err(RuntimeError::UnloadFailed));
    assert_eq!(events.owner_drops.get(), 1);
    assert_eq!(events.attempts.get(), 1);
    drop(lease.borrow_mut().take());
    assert_eq!(events.releases.get(), 1);
    let second = runtime.load(&model, &mut artifact, "x86_64").unwrap();
    drop(lease.borrow_mut().take());
    second.unload().unwrap();
    assert_eq!(events.allocations.get(), 2);
    assert_eq!(events.releases.get(), 2);
    assert_eq!(events.attempts.get(), 2);
}

#[test]
fn legacy_loaded_session_drop_uses_consuming_unload_once() {
    let Fixture {
        model,
        backend,
        events,
        lease,
    } = fixture();
    let mut runtime = ModelRuntime::new(backend);
    let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
    let session = runtime.load(&model, &mut artifact, "x86_64").unwrap();
    drop(session);
    assert_eq!(events.attempts.get(), 1);
    assert_eq!(events.owner_drops.get(), 1);
    assert_eq!(events.releases.get(), 0);
    drop(lease.borrow_mut().take());
    assert_eq!(events.releases.get(), 1);
    drop(runtime);
    assert_eq!(events.releases.get(), 1);
    assert_eq!(events.owners_at_backend_drop.get(), 1);
}

#[test]
fn resident_drop_retries_and_drops_the_owner_before_the_backend() {
    let Fixture {
        model,
        backend,
        events,
        lease,
    } = fixture();
    let mut runtime = ResidentModelRuntime::new(backend);
    let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
    runtime.load(&model, &mut artifact, "x86_64").unwrap();
    assert_eq!(runtime.unload(), Err(RuntimeError::UnloadFailed));
    let handle = events.handle.get();
    drop(runtime);
    assert_eq!(events.attempts.get(), 2);
    assert_eq!(events.attempted_handle.get(), handle);
    assert_eq!(events.owner_drops.get(), 1);
    assert_eq!(events.backend_drops.get(), 1);
    assert_eq!(events.owners_at_backend_drop.get(), 1);
    assert_eq!(events.releases.get(), 0);
    drop(lease.borrow_mut().take());
    assert_eq!(events.releases.get(), 1);
}

#[test]
fn dropping_an_inert_owner_does_not_release_a_new_resource() {
    let Fixture {
        model,
        mut backend,
        events,
        lease,
    } = fixture();
    let mut artifact = MemoryArtifactReader::for_manifest(&model, vec![1]);
    let mut first = backend.load(&model, &mut artifact).unwrap();
    drop(lease.borrow_mut().take());
    backend.unload_resident(&mut first).unwrap();
    let mut second = backend.load(&model, &mut artifact).unwrap();
    drop(first);
    assert_eq!(backend.resource_report().loaded_model_sessions, 1);
    assert_eq!(events.releases.get(), 1);
    drop(lease.borrow_mut().take());
    backend.unload_resident(&mut second).unwrap();
    drop(second);
    assert_eq!(events.releases.get(), 2);
    assert_eq!(backend.resource_report().loaded_model_sessions, 0);
}

struct Artifacts;
impl ModelArtifactSource for Artifacts {
    type Reader = MemoryArtifactReader;
    fn open(&mut self, model: &ModelManifest) -> Result<Self::Reader, RuntimeError> {
        Ok(MemoryArtifactReader::for_manifest(model, vec![1]))
    }
}
struct Clock;
impl ModelServiceClock for Clock {
    fn now_millis(&self) -> Option<u64> {
        Some(0)
    }
}

#[test]
fn service_cleanup_retry_releases_the_resource_but_stays_disabled() {
    let Fixture {
        model,
        backend,
        events,
        lease,
    } = fixture();
    let mut record = ModelStoreRecord::discovered(model).unwrap();
    let terms = record.license().terms_reference.clone().unwrap();
    record.acknowledge_terms(&terms).unwrap();
    let mut service = LazyModelService::new(
        record,
        backend,
        Artifacts,
        Clock,
        ResourceBudget {
            available_ram_bytes: u64::MAX,
            available_storage_bytes: u64::MAX,
            cpu_cores: 4,
            gpu_available: false,
        },
        ModelServiceLimits {
            max_input_bytes: 256,
            max_output_bytes: 256,
            max_output_tokens: 64,
            max_request_millis: 1000,
        },
    )
    .unwrap();
    let capability = CapabilityId::new("text.generate").unwrap();
    service
        .generate(&request(&capability), &NeverCancel)
        .unwrap();
    assert_eq!(service.unload(), Err(RuntimeError::UnloadFailed.into()));
    assert_eq!(events.owner_drops.get(), 0);
    drop(lease.borrow_mut().take());
    service.unload().unwrap();
    assert_eq!(events.releases.get(), 1);
    assert_eq!(service.state(), LifecycleState::Disabled);
    assert_eq!(
        service.generate(&request(&capability), &NeverCancel),
        Err(RuntimeError::BackendUnavailable.into())
    );
    drop(service);
    assert_eq!(events.releases.get(), 1);
}
