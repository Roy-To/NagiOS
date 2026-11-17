//! Synthetic owners/bytes/backend only. No native or guest evidence.
extern crate std;
use super::*;
use crate::{
    testing::{FakeModelBackend, FakeSession, NeverCancel},
    AuthenticatedArtifactLease, AuthenticatedModelStoreSnapshot, AuthenticatedSnapshotAuthority,
    BackendDescriptor, BackendResponse, CommitFence, LiveRequestAuthority, ModelRequest,
    ModelStoreArtifacts, ReceiptEnvelope, ReceiptManager, ReceiptStore, ResourceBudget,
    RuntimeResourceReport, TokenUsage,
};
use alloc::{format, rc::Rc, vec::Vec};
use core::cell::{Cell, RefCell};
#[derive(Default)]
struct State {
    epoch: Cell<u64>,
    locked: Cell<bool>,
    cancelled: Cell<bool>,
    terms: Cell<bool>,
    identity: Cell<u128>,
    revision: Cell<u64>,
    policy: Cell<bool>,
    load_denied: Cell<bool>,
    reads: Cell<usize>,
    opens: Cell<usize>,
    bytes: RefCell<Vec<u8>>,
    metadata_mutation: Cell<Option<usize>>,
}
#[derive(Clone)]
struct Authority(Rc<State>);
impl LiveAuthority for Authority {
    type Session = u64;
    type Stamp = u64;
    fn check(&self, live: &u64) -> Result<u64, ReceiptError> {
        if *live != 1 || self.0.locked.get() {
            Err(ReceiptError::Denied)
        } else {
            Ok(self.0.epoch.get())
        }
    }
}
impl LiveRequestAuthority for Authority {
    fn check_request_epoch(&self, live: &u64, epoch: u64) -> Result<(), ReceiptError> {
        self.check(live)?;
        if self.0.cancelled.get() {
            Err(ReceiptError::Cancelled)
        } else if epoch != self.0.epoch.get() {
            Err(ReceiptError::Stale)
        } else {
            Ok(())
        }
    }
}
impl AuthenticatedSnapshotAuthority for Authority {
    fn check_os_terms(&self, live: &u64, _: &str) -> Result<(), ReceiptError> {
        self.check(live)?;
        if self.0.terms.get() {
            Ok(())
        } else {
            Err(ReceiptError::TermsRequired)
        }
    }
    fn current_request_epoch(&self, live: &u64) -> Result<u64, ReceiptError> {
        self.check(live)
    }
}
struct Source {
    state: Rc<State>,
    id: ArtifactId,
}
impl ModelArtifactReader for Source {
    fn artifact_id(&self) -> &ArtifactId {
        &self.id
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        None
    }
    fn len(&self) -> u64 {
        self.state.bytes.borrow().len() as u64
    }
    fn read_at(&mut self, o: u64, d: &mut [u8]) -> Result<usize, ArtifactReadError> {
        self.state.reads.set(self.state.reads.get() + 1);
        let b = self.state.bytes.borrow();
        let start = o as usize;
        if start > b.len() {
            return Err(ArtifactReadError::OutOfRange);
        }
        let n = d.len().min(b.len() - start);
        d[..n].copy_from_slice(&b[start..start + n]);
        Ok(n)
    }
}
struct Lease {
    state: Rc<State>,
    manifest: ModelManifest,
}
impl AuthenticatedArtifactLease for Lease {
    fn store_identity(&self) -> crate::ModelStoreIdentity {
        crate::ModelStoreIdentity(17)
    }
    fn revision(&self) -> u64 {
        1
    }
    fn check(&self, m: &ModelManifest) -> Result<(), ReceiptError> {
        if self.state.identity.get() != 17 || self.state.revision.get() != 1 || m != &self.manifest
        {
            Err(ReceiptError::Stale)
        } else {
            Ok(())
        }
    }
}
type View = AuthenticatedModelStoreSnapshot<Source, Lease, Authority, NeverCancel>;
struct Artifacts {
    state: Rc<State>,
}
impl ModelStoreArtifacts for Artifacts {
    type Snapshot = View;
    fn capture(&mut self, m: &ModelManifest, _: u64) -> Result<View, ReceiptError> {
        self.state.opens.set(self.state.opens.get() + 1);
        let ArtifactReference::ModelStore { artifact_id } = &m.artifact.reference;
        View::capture_verified(
            Source {
                state: self.state.clone(),
                id: artifact_id.clone(),
            },
            Lease {
                state: self.state.clone(),
                manifest: m.clone(),
            },
            Authority(self.state.clone()),
            &1,
            NeverCancel,
            m,
        )
    }
}
struct Store {
    state: Rc<State>,
    receipt: Option<ReceiptEnvelope>,
}
impl ReceiptStore for Store {
    fn store_identity(&self) -> crate::ModelStoreIdentity {
        crate::ModelStoreIdentity(self.state.identity.get())
    }
    fn revision(&self) -> Result<u64, ReceiptError> {
        Ok(if self.receipt.is_some() { 1 } else { 0 })
    }
    fn read(&mut self, _: &ModelId) -> Result<Option<ReceiptEnvelope>, ReceiptError> {
        Ok(self.receipt.clone())
    }
    fn commit<T: ModelStoreArtifact>(
        &mut self,
        rev: u64,
        e: &ReceiptEnvelope,
        lease: &T,
        f: &mut dyn CommitFence,
    ) -> Result<u64, ReceiptError> {
        assert_eq!(rev, 0);
        lease.validate_lease()?;
        f.check()?;
        self.receipt = Some(e.clone());
        Ok(1)
    }
}
struct Models {
    state: Rc<State>,
    manifest: ModelManifest,
    retained: Option<(InstalledReceipt, View)>,
}
impl TrustedInstalledModels for Models {
    type Lease = View;
    fn capture_installed(
        &mut self,
        id: &ModelId,
    ) -> Result<InstalledSelectionCandidate<View>, InstalledSelectionError> {
        if id != &self.manifest.model_id {
            return Err(InstalledSelectionError::Unavailable);
        }
        let (r, v) = self
            .retained
            .take()
            .ok_or(InstalledSelectionError::Unavailable)?;
        Ok(InstalledSelectionCandidate::from_trusted_owner(r, v))
    }
    fn revalidate_installed(
        &self,
        c: &InstalledSelectionCandidate<View>,
    ) -> Result<(), InstalledSelectionError> {
        c.reader
            .validate_lease()
            .map_err(InstalledSelectionError::Owner)?;
        if self.state.identity.get() != c.receipt.store_identity().0
            || self.state.revision.get() != c.receipt.artifact_revision()
            || c.manifest() != &self.manifest
        {
            return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
        }
        Ok(())
    }
    fn admit_selection(
        &self,
        _: &InstalledSelectionCandidate<View>,
        _: &SelectionRequest<'_>,
    ) -> Result<(), InstalledSelectionError> {
        if self.state.policy.get() {
            Err(InstalledSelectionError::AdmissionRejected)
        } else {
            Ok(())
        }
    }
    fn revalidate_load(
        &self,
        c: &InstalledSelectionCandidate<View>,
        _: InstalledLoadPhase,
    ) -> Result<(), InstalledSelectionError> {
        self.revalidate_installed(c)?;
        if self.state.load_denied.get() {
            Err(InstalledSelectionError::AdmissionRejected)
        } else {
            Ok(())
        }
    }
}
impl crate::TrustedResidentModels for Models {
    fn revalidate_resident(&self, r: &InstalledReceipt) -> Result<(), InstalledSelectionError> {
        if r.manifest() != &self.manifest || r.store_identity().0 != self.state.identity.get() {
            return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
        }
        Authority(self.state.clone())
            .check_request_epoch(&1, self.state.epoch.get())
            .map_err(InstalledSelectionError::Owner)
    }
    fn admit_inference(
        &self,
        r: &InstalledReceipt,
        _: &ModelRequest<'_>,
    ) -> Result<(), InstalledSelectionError> {
        self.revalidate_resident(r)
    }
    fn admit_response(
        &self,
        r: &InstalledReceipt,
        _: &crate::ModelResponse,
    ) -> Result<(), InstalledSelectionError> {
        self.revalidate_resident(r)
    }
}
struct Clock;
impl crate::ModelServiceClock for Clock {
    fn now_millis(&self) -> Option<u64> {
        Some(1)
    }
}
struct Backend {
    inner: FakeModelBackend,
    zero_failure: Rc<Cell<bool>>,
    state: Rc<State>,
}
impl ModelBackend for Backend {
    type Session = FakeSession;
    fn descriptor(&self) -> &BackendDescriptor {
        if let Some(kind) = self.state.metadata_mutation.take() {
            match kind {
                0 => self.state.epoch.set(self.state.epoch.get() + 1),
                1 => self.state.terms.set(false),
                _ => self.state.identity.set(19),
            }
        }
        self.inner.descriptor()
    }
    fn health(&self) -> BackendHealth {
        self.inner.health()
    }
    fn resource_report(&self) -> RuntimeResourceReport {
        self.inner.resource_report()
    }
    fn load(
        &mut self,
        m: &ModelManifest,
        a: &mut dyn ModelArtifactReader,
    ) -> Result<FakeSession, RuntimeError> {
        let mut out = [0xcc; 64];
        if a.read_at(0, &mut out).is_err() {
            self.zero_failure.set(out == [0; 64]);
            return Err(RuntimeError::ArtifactUnavailable);
        }
        self.inner.load(m, a)
    }
    fn unload(&mut self, s: FakeSession) -> Result<(), RuntimeError> {
        self.inner.unload(s)
    }
    fn unload_resident(&mut self, s: &mut FakeSession) -> Result<(), RuntimeError> {
        self.inner.unload_resident(s)
    }
    fn infer_stream(
        &mut self,
        s: &mut FakeSession,
        r: &ModelRequest<'_>,
        c: &dyn CancellationToken,
        sink: &mut dyn crate::TextChunkSink,
    ) -> Result<TokenUsage, RuntimeError> {
        self.inner.infer_stream(s, r, c, sink)
    }
    fn infer(
        &mut self,
        s: &mut FakeSession,
        r: &ModelRequest<'_>,
        c: &dyn CancellationToken,
    ) -> Result<BackendResponse, RuntimeError> {
        self.inner.infer(s, r, c)
    }
}
fn installed(state: Rc<State>, m: ModelManifest) -> Models {
    let mut manager = ReceiptManager::new(
        Authority(state.clone()),
        Artifacts {
            state: state.clone(),
        },
        Store {
            state: state.clone(),
            receipt: None,
        },
    );
    let c = manager
        .capture_artifact(
            &1,
            &m,
            m.license.terms_reference.as_deref(),
            0,
            &NeverCancel,
        )
        .unwrap();
    let pair = manager
        .commit_verified_install_with_snapshot(&1, &m, c, &NeverCancel)
        .unwrap();
    Models {
        state,
        manifest: m,
        retained: Some(pair),
    }
}
fn fixture() -> (
    InstalledModelSelector<Backend>,
    Models,
    Authority,
    crate::CapabilityId,
) {
    let state = Rc::new(State::default());
    state.epoch.set(1);
    state.terms.set(true);
    state.identity.set(17);
    state.revision.set(1);
    *state.bytes.borrow_mut() = (0..(crate::SEALED_CHUNK_BYTES * 2 + 13))
        .map(|n| (n % 241) as u8)
        .collect();
    let mut m =
        ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json")).unwrap();
    m.artifact.size_bytes = Some(state.bytes.borrow().len() as u64);
    m.artifact.integrity.as_mut().unwrap().digest =
        format!("{:x}", sha2::Sha256::digest(&*state.bytes.borrow()));
    let backend = Backend {
        inner: FakeModelBackend::for_manifest(&m),
        zero_failure: Rc::new(Cell::new(false)),
        state: state.clone(),
    };
    let models = installed(state.clone(), m);
    (
        InstalledModelSelector::new(backend),
        models,
        Authority(state),
        crate::CapabilityId::new("text.generate").unwrap(),
    )
}
fn request(c: &crate::CapabilityId) -> SelectionRequest<'_> {
    SelectionRequest {
        capability: c,
        role: None,
        minimum_context_tokens: 1,
        resources: ResourceBudget {
            available_ram_bytes: u64::MAX,
            available_storage_bytes: u64::MAX,
            cpu_cores: 1,
            gpu_available: false,
        },
        target_architecture: "x86_64",
        offline_only: true,
        preferred_model_id: None,
        role_default_model_id: None,
    }
}
use sha2::Digest;
#[test]
fn proof_selection_hashes_same_view_once_and_backend_reads_authenticate() {
    let (mut s, mut m, a, cap) = fixture();
    let baseline = a.0.reads.get();
    let id = m.manifest.model_id.clone();
    s.select_authenticated(
        InstalledSelectionMode::Manual(&id),
        &request(&cap),
        &a,
        &1,
        &NeverCancel,
        &mut m,
    )
    .unwrap();
    assert_eq!(a.0.reads.get() - baseline, 25);
    assert_eq!(s.backend().inner.active_sessions, 1);
    assert_eq!(s.backend().inner.load_count, 1);
}
#[test]
fn legacy_selector_still_fullhashes_twice_with_same_authenticated_view() {
    let (mut s, mut m, a, cap) = fixture();
    let baseline = a.0.reads.get();
    let id = m.manifest.model_id.clone();
    s.select(
        InstalledSelectionMode::Manual(&id),
        &request(&cap),
        &a,
        &1,
        &NeverCancel,
        &mut m,
    )
    .unwrap();
    assert_eq!(a.0.reads.get() - baseline, 42);
    assert_eq!(s.backend().inner.active_sessions, 1);
}
#[test]
fn spent_proof_cannot_replay_or_substitute_a_reader() {
    let (_, mut m, a, cap) = fixture();
    let id = m.manifest.model_id.clone();
    let mut candidate = m.capture_installed(&id).unwrap();
    let stamp = a.check(&1).unwrap();
    let control = SelectionControl {
        authority: &a,
        live: &1,
        cancel: &NeverCancel,
        stamp: &stamp,
        stopped: Cell::new(None),
    };
    verify_model_artifact_integrity(
        &mut SelectionReader {
            reader: &mut candidate.reader,
            revision: 1,
            store_identity: crate::ModelStoreIdentity(17),
            control: &control,
        },
        candidate
            .receipt
            .manifest()
            .artifact
            .integrity
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let mut proof = VerifiedInstalledCandidate {
        verified_manifest: candidate.manifest().clone(),
        candidate,
        permit: Some(()),
        failure: Cell::new(None),
    };
    {
        let _reader = proof.begin_load(&control, &m, None).unwrap();
    }
    assert!(matches!(
        proof.begin_load(&control, &m, None),
        Err(InstalledSelectionError::Runtime(
            RuntimeError::IntegrityRequired
        ))
    ));
    assert_eq!(m.manifest.model_id, id);
    let _ = cap;
}
#[test]
fn failed_retirement_quarantines_and_never_loads_a_new_verified_candidate() {
    let (mut s, mut m, a, c) = fixture();
    let old = m.manifest.model_id.clone();
    s.select_authenticated(
        InstalledSelectionMode::Manual(&old),
        &request(&c),
        &a,
        &1,
        &NeverCancel,
        &mut m,
    )
    .unwrap();
    let mut next = m.manifest.clone();
    next.model_id = ModelId::new("synthetic.next").unwrap();
    let mut next = installed(a.0.clone(), next);
    let id = next.manifest.model_id.clone();
    s.runtime.backend_mut_for_proof_test().inner.unload_error = Some(RuntimeError::LoadFailed);
    assert!(s
        .select_authenticated(
            InstalledSelectionMode::Manual(&id),
            &request(&c),
            &a,
            &1,
            &NeverCancel,
            &mut next
        )
        .is_err());
    assert!(s.is_quarantined());
    assert_eq!(s.backend().inner.load_count, 1);
    assert_eq!(s.backend().inner.active_sessions, 1);
}
#[test]
fn typed_service_constructor_dispatches_proof_and_default_constructor_keeps_fullhash() {
    for proof_route in [false, true] {
        let (mut selector, models, a, cap) = fixture();
        let baseline = a.0.reads.get();
        let id = models.manifest.model_id.clone();
        let backend = selector.runtime.backend_mut_for_proof_test();
        let owned = Backend {
            inner: backend.inner.clone(),
            zero_failure: backend.zero_failure.clone(),
            state: a.0.clone(),
        };
        let limits = crate::ModelServiceLimits {
            max_input_bytes: 1024,
            max_output_bytes: 1024,
            max_output_tokens: 16,
            max_request_millis: 100,
        };
        let mut service = if proof_route {
            crate::InstalledModelService::new_authenticated_immutable_views(
                owned,
                a.clone(),
                models,
                Clock,
                limits,
                None,
            )
        } else {
            crate::InstalledModelService::new(owned, a.clone(), models, Clock, limits)
        }
        .unwrap();
        service
            .select(
                InstalledSelectionMode::Manual(&id),
                &request(&cap),
                &1,
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(
            a.0.reads.get() - baseline,
            if proof_route { 25 } else { 42 }
        );
    }
}
#[cfg(feature = "private-installed-acceptance-diagnostics")]
mod observer_tests {
    use super::*;
    std::thread_local! {static ACTION:RefCell<Option<(Rc<State>,PrivateInstalledStage,usize)>>=RefCell::new(None);}
    fn observe(stage: PrivateInstalledStage) {
        ACTION.with(|a| {
            let action = a.borrow().clone();
            if let Some((s, at, kind)) = action {
                if stage == at {
                    match kind {
                        0 => s.epoch.set(s.epoch.get() + 1),
                        1 => s.locked.set(true),
                        2 => s.cancelled.set(true),
                        3 => s.terms.set(false),
                        4 => s.identity.set(19),
                        5 => s.load_denied.set(true),
                        6 => s.bytes.borrow_mut()[0] ^= 1,
                        n => s.metadata_mutation.set(Some(n - 7)),
                    }
                }
            }
        });
    }
    fn arm(s: Rc<State>, at: PrivateInstalledStage, kind: usize) {
        ACTION.with(|a| *a.borrow_mut() = Some((s, at, kind)));
    }
    fn reset() {
        ACTION.with(|a| *a.borrow_mut() = None);
    }
    #[test]
    fn every_new_boundary_refences_actual_live_epoch_terms_namespace_cancel_and_budget() {
        for stage in [
            PrivateInstalledStage::SelectionVerifyBegin,
            PrivateInstalledStage::SelectionVerifyDone,
            PrivateInstalledStage::RuntimeProofBegin,
            PrivateInstalledStage::RuntimeProofDone,
            PrivateInstalledStage::BackendLoadBegin,
            PrivateInstalledStage::BackendLoadDone,
        ] {
            for kind in 0..6 {
                let (mut s, mut m, a, c) = fixture();
                arm(a.0.clone(), stage, kind);
                s.set_private_stage_observer(Some(observe));
                let id = m.manifest.model_id.clone();
                assert!(
                    s.select_authenticated(
                        InstalledSelectionMode::Manual(&id),
                        &request(&c),
                        &a,
                        &1,
                        &NeverCancel,
                        &mut m
                    )
                    .is_err(),
                    "{stage:?} kind{kind}"
                );
                assert_eq!(s.model_id(), None);
                assert_eq!(s.backend().inner.active_sessions, 0);
                reset();
            }
        }
    }
    #[test]
    fn final_load_admission_rejection_before_retirement_retains_old_resident() {
        let (mut s, mut m, a, c) = fixture();
        let old = m.manifest.model_id.clone();
        s.select_authenticated(
            InstalledSelectionMode::Manual(&old),
            &request(&c),
            &a,
            &1,
            &NeverCancel,
            &mut m,
        )
        .unwrap();
        let mut next = m.manifest.clone();
        next.model_id = ModelId::new("synthetic.next").unwrap();
        let mut next = installed(a.0.clone(), next);
        let id = next.manifest.model_id.clone();
        arm(a.0.clone(), PrivateInstalledStage::SelectionVerifyDone, 5);
        s.set_private_stage_observer(Some(observe));
        assert_eq!(
            s.select_authenticated(
                InstalledSelectionMode::Manual(&id),
                &request(&c),
                &a,
                &1,
                &NeverCancel,
                &mut next
            ),
            Err(InstalledSelectionError::AdmissionRejected)
        );
        assert_eq!(s.model_id(), Some(&old));
        assert_eq!(s.backend().inner.active_sessions, 1);
        assert_eq!(s.backend().inner.unload_count, 0);
        assert_eq!(s.backend().inner.load_count, 1);
        reset();
    }
    #[test]
    fn uncached_physical_mutation_after_proof_fails_backend_read_and_zeroes_output() {
        let (mut s, mut m, a, c) = fixture();
        arm(a.0.clone(), PrivateInstalledStage::BackendLoadBegin, 6);
        s.set_private_stage_observer(Some(observe));
        let id = m.manifest.model_id.clone();
        assert!(s
            .select_authenticated(
                InstalledSelectionMode::Manual(&id),
                &request(&c),
                &a,
                &1,
                &NeverCancel,
                &mut m
            )
            .is_err());
        assert!(s.backend().zero_failure.get());
        assert_eq!(s.backend().inner.active_sessions, 0);
        reset();
    }
    #[test]
    fn postload_callback_cleanup_failure_quarantines_and_retains_failed_session() {
        let (mut s, mut m, a, c) = fixture();
        s.runtime.backend_mut_for_proof_test().inner.unload_error = Some(RuntimeError::LoadFailed);
        arm(a.0.clone(), PrivateInstalledStage::BackendLoadDone, 2);
        s.set_private_stage_observer(Some(observe));
        let id = m.manifest.model_id.clone();
        assert!(s
            .select_authenticated(
                InstalledSelectionMode::Manual(&id),
                &request(&c),
                &a,
                &1,
                &NeverCancel,
                &mut m
            )
            .is_err());
        assert!(s.is_quarantined());
        assert_eq!(s.backend().inner.active_sessions, 1);
        reset();
    }
    #[test]
    fn runtime_metadata_callbacks_after_proof_consume_refence_epoch_terms_and_namespace() {
        for kind in 7..10 {
            let (mut s, mut m, a, c) = fixture();
            arm(a.0.clone(), PrivateInstalledStage::RuntimeProofBegin, kind);
            s.set_private_stage_observer(Some(observe));
            let id = m.manifest.model_id.clone();
            assert!(s
                .select_authenticated(
                    InstalledSelectionMode::Manual(&id),
                    &request(&c),
                    &a,
                    &1,
                    &NeverCancel,
                    &mut m
                )
                .is_err());
            assert_eq!(s.backend().inner.load_count, 0);
            assert_eq!(s.backend().inner.active_sessions, 0);
            reset();
        }
    }
    #[test]
    fn observed_legacy_runtime_verify_boundaries_still_hash_and_refence() {
        for at in [
            PrivateInstalledStage::RuntimeVerifyBegin,
            PrivateInstalledStage::RuntimeVerifyDone,
        ] {
            for kind in 0..6 {
                let (mut s, mut m, a, c) = fixture();
                arm(a.0.clone(), at, kind);
                s.set_private_stage_observer(Some(observe));
                let id = m.manifest.model_id.clone();
                assert!(s
                    .select(
                        InstalledSelectionMode::Manual(&id),
                        &request(&c),
                        &a,
                        &1,
                        &NeverCancel,
                        &mut m
                    )
                    .is_err());
                assert_eq!(s.backend().inner.load_count, 0);
                reset();
            }
        }
    }
    #[test]
    fn observer_feature_does_not_enable_proof_on_ordinary_constructor() {
        let (mut s, mut m, a, c) = fixture();
        let baseline = a.0.reads.get();
        s.set_private_stage_observer(Some(observe));
        let id = m.manifest.model_id.clone();
        s.select(
            InstalledSelectionMode::Manual(&id),
            &request(&c),
            &a,
            &1,
            &NeverCancel,
            &mut m,
        )
        .unwrap();
        assert_eq!(a.0.reads.get() - baseline, 42);
        assert_eq!(s.backend().inner.load_count, 1);
        reset();
    }
}
