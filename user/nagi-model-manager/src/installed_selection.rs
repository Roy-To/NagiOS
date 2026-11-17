//! Private in-process installed selection. Trusted adapters must be supplied by
//! the canonical Session and durable receipt owners; registry metadata is not a grant.
use crate::{
    registry::resources_fit, verify_model_artifact_integrity, ArtifactId, ArtifactReadError,
    ArtifactReference, AuthenticatedImmutableArtifact, BackendHealth, CancellationToken,
    ExecutionScope, InstalledReceipt, IntegrityMetadata, LiveAuthority, ModelArtifactReader,
    ModelBackend, ModelId, ModelManifest, ModelStoreArtifact, PrivateInstalledStage,
    PrivateInstalledStageObserver, ReceiptError, ResidentModelRuntime, RuntimeError,
    RuntimeResourceReport, SelectionRequest,
};
use core::cell::Cell;

/// Construction is an explicit trusted-owner boundary, not install verification.
/// The reader owns an immutable artifact/revision lease retained until load ends.
pub struct InstalledSelectionCandidate<R> {
    receipt: InstalledReceipt,
    reader: R,
}
impl<R> InstalledSelectionCandidate<R> {
    pub fn from_trusted_owner(receipt: InstalledReceipt, reader: R) -> Self {
        Self { receipt, reader }
    }
    pub fn receipt(&self) -> &InstalledReceipt {
        &self.receipt
    }
    pub fn manifest(&self) -> &ModelManifest {
        self.receipt.manifest()
    }
    pub fn reader(&self) -> &R {
        &self.reader
    }
}

/// Must use actual durable installed receipts and recheck exact current manifest,
/// terms, artifact identity and revision. Lifecycle/verified metadata alone fail.
pub trait TrustedInstalledModels {
    type Lease: ModelStoreArtifact;
    fn capture_installed(
        &mut self,
        id: &ModelId,
    ) -> Result<InstalledSelectionCandidate<Self::Lease>, InstalledSelectionError>;
    /// Trusted privacy/sensitivity and latency policy, with a measured resource
    /// reservation. Re-evaluate for this request; inference input is not a grant.
    fn admit_selection(
        &self,
        candidate: &InstalledSelectionCandidate<Self::Lease>,
        request: &SelectionRequest<'_>,
    ) -> Result<(), InstalledSelectionError>;
    fn revalidate_installed(
        &self,
        candidate: &InstalledSelectionCandidate<Self::Lease>,
    ) -> Result<(), InstalledSelectionError>;
    /// Proof route only. Unknown continuing load/privacy budget must deny.
    /// Ordinary legacy constructors do not call this additive owner boundary.
    fn revalidate_load(
        &self,
        _candidate: &InstalledSelectionCandidate<Self::Lease>,
        _phase: InstalledLoadPhase,
    ) -> Result<(), InstalledSelectionError> {
        Err(InstalledSelectionError::Unavailable)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledLoadPhase {
    BeforeLoad,
    Loading,
    Loaded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledSelectionError {
    Owner(ReceiptError),
    Unavailable,
    AdmissionRejected,
    Runtime(RuntimeError),
    Quarantined,
}

pub enum InstalledSelectionMode<'a> {
    Manual(&'a ModelId),
    Automatic(&'a [ModelId]),
}

pub struct InstalledModelSelector<B: ModelBackend> {
    runtime: ResidentModelRuntime<B>,
    quarantined: bool,
    receipt: Option<InstalledReceipt>,
    observer: Option<PrivateInstalledStageObserver>,
}

type CandidateLoader<B, A, S> = fn(
    &mut InstalledModelSelector<B>,
    &ModelId,
    &SelectionRequest<'_>,
    &mut S,
    &SelectionControl<'_, A>,
) -> Result<ModelId, InstalledSelectionError>;

impl<B: ModelBackend> InstalledModelSelector<B> {
    pub const fn new(backend: B) -> Self {
        Self {
            runtime: ResidentModelRuntime::new(backend),
            quarantined: false,
            receipt: None,
            observer: None,
        }
    }
    pub fn set_private_stage_observer(&mut self, observer: Option<PrivateInstalledStageObserver>) {
        self.observer = observer;
    }
    pub fn model_id(&self) -> Option<&ModelId> {
        if self.quarantined {
            None
        } else {
            self.runtime.model_id()
        }
    }
    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }
    pub fn backend(&self) -> &B {
        self.runtime.backend()
    }
    pub fn resource_report(&self) -> RuntimeResourceReport {
        self.runtime.resource_report()
    }

    pub(crate) fn quarantine_for_owner(&mut self) {
        self.quarantined = true;
    }
    pub(crate) fn retire_for_owner(&mut self) -> Result<(), InstalledSelectionError> {
        match self.runtime.unload() {
            Ok(()) => {
                self.receipt = None;
                Ok(())
            }
            Err(error) => {
                self.quarantined = true;
                Err(InstalledSelectionError::Runtime(error))
            }
        }
    }
    pub(crate) fn invoke_for_owner<T>(
        &mut self,
        call: impl FnOnce(
            &mut ResidentModelRuntime<B>,
            &InstalledReceipt,
        ) -> Result<T, InstalledSelectionError>,
    ) -> Result<T, InstalledSelectionError> {
        if self.quarantined {
            return Err(InstalledSelectionError::Quarantined);
        }
        let receipt = self
            .receipt
            .as_ref()
            .ok_or(InstalledSelectionError::Unavailable)?;
        call(&mut self.runtime, receipt)
    }

    pub fn select<A: LiveAuthority, S: TrustedInstalledModels>(
        &mut self,
        mode: InstalledSelectionMode<'_>,
        request: &SelectionRequest<'_>,
        authority: &A,
        live: &A::Session,
        cancel: &dyn CancellationToken,
        installed: &mut S,
    ) -> Result<ModelId, InstalledSelectionError> {
        self.select_with(
            mode,
            request,
            authority,
            live,
            cancel,
            installed,
            Self::try_candidate::<A, S>,
        )
    }
    pub fn select_authenticated<A: LiveAuthority, S: TrustedInstalledModels>(
        &mut self,
        mode: InstalledSelectionMode<'_>,
        request: &SelectionRequest<'_>,
        authority: &A,
        live: &A::Session,
        cancel: &dyn CancellationToken,
        installed: &mut S,
    ) -> Result<ModelId, InstalledSelectionError>
    where
        S::Lease: AuthenticatedImmutableArtifact,
    {
        self.select_with(
            mode,
            request,
            authority,
            live,
            cancel,
            installed,
            Self::try_authenticated_candidate::<A, S>,
        )
    }
    // This private helper keeps the authority/session/cancellation/install
    // inputs explicit; grouping them would obscure which values are rechecked
    // for each candidate.
    #[allow(clippy::too_many_arguments)]
    fn select_with<A: LiveAuthority, S: TrustedInstalledModels>(
        &mut self,
        mode: InstalledSelectionMode<'_>,
        request: &SelectionRequest<'_>,
        authority: &A,
        live: &A::Session,
        cancel: &dyn CancellationToken,
        installed: &mut S,
        candidate_loader: CandidateLoader<B, A, S>,
    ) -> Result<ModelId, InstalledSelectionError> {
        if self.quarantined {
            return Err(InstalledSelectionError::Quarantined);
        }
        if request
            .preferred_model_id
            .is_some_and(|preferred| match mode {
                InstalledSelectionMode::Manual(id) => id != preferred,
                InstalledSelectionMode::Automatic(_) => true,
            })
        {
            return Err(InstalledSelectionError::AdmissionRejected);
        }
        let stamp = authority
            .check(live)
            .map_err(InstalledSelectionError::Owner)?;
        let control = SelectionControl {
            authority,
            live,
            cancel,
            stamp: &stamp,
            stopped: Cell::new(None),
        };
        control.check()?;
        let ids = match mode {
            InstalledSelectionMode::Manual(id) => core::slice::from_ref(id),
            InstalledSelectionMode::Automatic(ids) => ids,
        };
        let automatic = matches!(mode, InstalledSelectionMode::Automatic(_));
        let mut last = InstalledSelectionError::Unavailable;
        for id in ids {
            control.check()?;
            let result = candidate_loader(self, id, request, installed, &control);
            if let Err(error) = control.check() {
                if result.is_ok() && self.runtime.unload().is_err() {
                    self.quarantined = true;
                }
                if result.is_ok() && !self.quarantined {
                    self.receipt = None;
                }
                return Err(error);
            }
            match result {
                Ok(id) => return Ok(id),
                Err(error) => {
                    if self.quarantined || !automatic || !may_fallback(error) {
                        return Err(error);
                    }
                    last = error;
                }
            }
        }
        Err(last)
    }

    fn try_candidate<A: LiveAuthority, S: TrustedInstalledModels>(
        &mut self,
        id: &ModelId,
        request: &SelectionRequest<'_>,
        installed: &mut S,
        control: &SelectionControl<'_, A>,
    ) -> Result<ModelId, InstalledSelectionError> {
        control.check()?;
        let captured = installed.capture_installed(id);
        control.check()?;
        let mut candidate = captured?;
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        installed.admit_selection(&candidate, request)?;
        control.check()?;
        self.preflight(id, request, &candidate)?;
        if self.observer.is_some() {
            installed.revalidate_installed(&candidate)?;
            control.check()?;
            crate::private_stages::observe(
                self.observer,
                PrivateInstalledStage::SelectionVerifyBegin,
            );
            installed.revalidate_installed(&candidate)?;
            installed.admit_selection(&candidate, request)?;
            control.check()?;
        }
        let expected = candidate
            .receipt
            .manifest()
            .artifact
            .integrity
            .as_ref()
            .ok_or(InstalledSelectionError::AdmissionRejected)?;
        let verified = verify_model_artifact_integrity(
            &mut SelectionReader {
                reader: &mut candidate.reader,
                revision: candidate.receipt.artifact_revision(),
                store_identity: candidate.receipt.store_identity(),
                control,
            },
            expected,
        );
        control.check()?;
        verified.map_err(InstalledSelectionError::Runtime)?;
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        if self.observer.is_some() {
            crate::private_stages::observe(
                self.observer,
                PrivateInstalledStage::SelectionVerifyDone,
            );
            installed.revalidate_installed(&candidate)?;
            installed.admit_selection(&candidate, request)?;
            control.check()?;
        }
        // All admission gates run before retirement; failures above retain old.
        if let Err(error) = self.runtime.unload() {
            self.quarantined = true;
            return Err(InstalledSelectionError::Runtime(error));
        }
        self.receipt = None;
        control.check()?;
        if self.runtime.resource_report().loaded_model_sessions != 0 {
            self.quarantined = true;
            return Err(InstalledSelectionError::Quarantined);
        }
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        installed.admit_selection(&candidate, request)?;
        control.check()?;
        let observation_failure = Cell::new(None);
        let loaded = if self.observer.is_some() {
            self.runtime.load_observed_installed(
                LegacyLoadReader {
                    candidate: &mut candidate,
                    control,
                    installed,
                    request,
                    observer: self.observer,
                    failure: &observation_failure,
                },
                request.target_architecture,
            )
        } else {
            self.runtime.load(
                candidate.receipt.manifest(),
                &mut SelectionReader {
                    reader: &mut candidate.reader,
                    revision: candidate.receipt.artifact_revision(),
                    store_identity: candidate.receipt.store_identity(),
                    control,
                },
                request.target_architecture,
            )
        };
        // Always clean a newly loaded session when final publication checks fail.
        let final_gate = observation_failure
            .get()
            .map_or(Ok(()), Err)
            .and_then(|()| control.check())
            .and_then(|()| installed.revalidate_installed(&candidate))
            .and_then(|()| control.check());
        if let Err(error) = final_gate {
            if self.runtime.unload().is_err() {
                self.quarantined = true;
            }
            return Err(error);
        }
        loaded.map_err(InstalledSelectionError::Runtime)?;
        if self.runtime.resource_report().loaded_model_sessions != 1 {
            self.quarantined = true;
            let _ = self.runtime.unload();
            return Err(InstalledSelectionError::Quarantined);
        }
        let id = candidate.receipt.manifest().model_id.clone();
        self.receipt = Some(candidate.receipt);
        Ok(id)
    }

    fn try_authenticated_candidate<A: LiveAuthority, S: TrustedInstalledModels>(
        &mut self,
        id: &ModelId,
        request: &SelectionRequest<'_>,
        installed: &mut S,
        control: &SelectionControl<'_, A>,
    ) -> Result<ModelId, InstalledSelectionError>
    where
        S::Lease: AuthenticatedImmutableArtifact,
    {
        control.check()?;
        let captured = installed.capture_installed(id);
        control.check()?;
        let mut candidate = captured?;
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        installed.admit_selection(&candidate, request)?;
        control.check()?;
        installed.revalidate_load(&candidate, InstalledLoadPhase::BeforeLoad)?;
        control.check()?;
        self.preflight(id, request, &candidate)?;
        crate::private_stages::observe(self.observer, PrivateInstalledStage::SelectionVerifyBegin);
        control.check()?;
        installed.revalidate_installed(&candidate)?;
        installed.admit_selection(&candidate, request)?;
        control.check()?;
        let expected = candidate
            .receipt
            .manifest()
            .artifact
            .integrity
            .as_ref()
            .ok_or(InstalledSelectionError::AdmissionRejected)?;
        let verified = verify_model_artifact_integrity(
            &mut SelectionReader {
                reader: &mut candidate.reader,
                revision: candidate.receipt.artifact_revision(),
                store_identity: candidate.receipt.store_identity(),
                control,
            },
            expected,
        );
        control.check()?;
        verified.map_err(InstalledSelectionError::Runtime)?;
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        crate::private_stages::observe(self.observer, PrivateInstalledStage::SelectionVerifyDone);
        control.check()?;
        installed.revalidate_installed(&candidate)?;
        installed.admit_selection(&candidate, request)?;
        control.check()?;
        installed.revalidate_load(&candidate, InstalledLoadPhase::BeforeLoad)?;
        control.check()?;
        installed.revalidate_installed(&candidate)?;
        control.check()?;
        // The sole mint site owns the exact authenticated candidate just hashed.
        let mut proof = VerifiedInstalledCandidate {
            verified_manifest: candidate.receipt.manifest().clone(),
            candidate,
            permit: Some(()),
            failure: Cell::new(None),
        };
        if let Err(error) = self.runtime.unload() {
            self.quarantined = true;
            return Err(InstalledSelectionError::Runtime(error));
        }
        self.receipt = None;
        control.check()?;
        if self.runtime.resource_report().loaded_model_sessions != 0 {
            self.quarantined = true;
            return Err(InstalledSelectionError::Quarantined);
        }
        installed.revalidate_installed(&proof.candidate)?;
        control.check()?;
        installed.admit_selection(&proof.candidate, request)?;
        control.check()?;
        installed.revalidate_load(&proof.candidate, InstalledLoadPhase::BeforeLoad)?;
        control.check()?;
        let loaded = self.runtime.load_verified_installed(
            proof.begin_load(control, installed, self.observer)?,
            request.target_architecture,
        );
        let final_gate = proof
            .failure
            .get()
            .map_or(Ok(()), Err)
            .and_then(|()| control.check())
            .and_then(|()| installed.revalidate_installed(&proof.candidate))
            .and_then(|()| installed.revalidate_load(&proof.candidate, InstalledLoadPhase::Loaded))
            .and_then(|()| control.check());
        if let Err(error) = final_gate {
            if self.runtime.unload().is_err() {
                self.quarantined = true;
            }
            return Err(error);
        }
        loaded.map_err(InstalledSelectionError::Runtime)?;
        if self.runtime.resource_report().loaded_model_sessions != 1 {
            self.quarantined = true;
            let _ = self.runtime.unload();
            return Err(InstalledSelectionError::Quarantined);
        }
        let id = proof.candidate.receipt.manifest().model_id.clone();
        self.receipt = Some(proof.candidate.receipt);
        Ok(id)
    }

    fn preflight<R: ModelStoreArtifact>(
        &self,
        id: &ModelId,
        request: &SelectionRequest<'_>,
        candidate: &InstalledSelectionCandidate<R>,
    ) -> Result<(), InstalledSelectionError> {
        let m = candidate.receipt.manifest();
        let b = self.runtime.backend().descriptor();
        m.validate()
            .map_err(|e| InstalledSelectionError::Runtime(RuntimeError::ManifestRejected(e)))?;
        candidate
            .reader
            .validate_lease()
            .map_err(InstalledSelectionError::Owner)?;
        if candidate.reader.store_identity() != candidate.receipt.store_identity()
            || candidate.reader.revision() != candidate.receipt.artifact_revision()
        {
            return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
        }
        let ArtifactReference::ModelStore { artifact_id } = &m.artifact.reference;
        if &m.model_id != id
            || m.execution != ExecutionScope::Local
            || (m.license.acknowledgement_required
                && candidate.receipt.acknowledged_terms() != m.license.terms_reference.as_deref())
            || !m.has_capability(request.capability)
            || !b.capabilities.contains(request.capability)
            || request.role.is_some_and(|r| !m.has_role(r))
            || m.context.max_input_tokens < request.minimum_context_tokens
            || !resources_fit(m, request.resources)
            || request.target_architecture.is_empty()
            || (!m.compatibility.architectures.is_empty()
                && !m
                    .compatibility
                    .architectures
                    .iter()
                    .any(|a| a == request.target_architecture))
            || (!b.architectures.is_empty()
                && !b
                    .architectures
                    .iter()
                    .any(|a| a == request.target_architecture))
            || !m.supported_backends.contains(&b.backend_id)
            || !b.runtime_api_versions.contains(&m.runtime_api_version)
            || !b.artifact_formats.contains(&m.artifact.format)
            || m.runtime_class
                .as_ref()
                .is_some_and(|c| !b.runtime_classes.contains(c))
            || self.runtime.resource_report().loaded_model_sessions > 1
            || self.runtime.resource_report().active_invocations != 0
            || self.runtime.health() == BackendHealth::Unavailable
            || candidate.reader.artifact_id() != artifact_id
            || candidate.reader.is_empty()
            || m.artifact.size_bytes != Some(candidate.reader.len())
            || m.artifact.integrity.is_none()
            || candidate
                .reader
                .verified_integrity()
                .is_some_and(|v| Some(v) != m.artifact.integrity.as_ref())
        {
            return Err(InstalledSelectionError::AdmissionRejected);
        }
        Ok(())
    }
}
fn may_fallback(error: InstalledSelectionError) -> bool {
    matches!(
        error,
        InstalledSelectionError::Unavailable
            | InstalledSelectionError::AdmissionRejected
            | InstalledSelectionError::Runtime(
                RuntimeError::LoadFailed
                    | RuntimeError::BackendUnavailable
                    | RuntimeError::ArtifactUnavailable
                    | RuntimeError::IntegrityMismatch
                    | RuntimeError::ArtifactMismatch
                    | RuntimeError::IntegrityRequired
                    | RuntimeError::IncompatibleBackend
                    | RuntimeError::ManifestRejected(_)
            )
    )
}
pub(crate) struct LegacyLoadReader<
    'read,
    'owner,
    'request,
    R: ModelStoreArtifact,
    A: LiveAuthority,
    S: TrustedInstalledModels<Lease = R>,
> {
    candidate: &'read mut InstalledSelectionCandidate<R>,
    control: &'read SelectionControl<'owner, A>,
    installed: &'read S,
    request: &'read SelectionRequest<'request>,
    observer: Option<PrivateInstalledStageObserver>,
    failure: &'read Cell<Option<InstalledSelectionError>>,
}
impl<R: ModelStoreArtifact, A: LiveAuthority, S: TrustedInstalledModels<Lease = R>>
    LegacyLoadReader<'_, '_, '_, R, A, S>
{
    pub(crate) fn manifest(&self) -> &ModelManifest {
        self.candidate.manifest()
    }
    pub(crate) fn checkpoint(&self) -> Result<(), RuntimeError> {
        if self.failure.get().is_some() {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        let check = self
            .control
            .check()
            .and_then(|()| self.installed.revalidate_installed(self.candidate))
            .and_then(|()| self.control.check());
        if let Err(e) = check {
            self.failure.set(Some(e));
            return Err(RuntimeError::ArtifactUnavailable);
        }
        Ok(())
    }
    pub(crate) fn stage(
        &self,
        stage: PrivateInstalledStage,
        phase: InstalledLoadPhase,
    ) -> Result<(), RuntimeError> {
        self.checkpoint()?;
        let gate = || {
            self.installed
                .revalidate_load(self.candidate, phase)
                .and_then(|()| {
                    if phase == InstalledLoadPhase::BeforeLoad {
                        self.installed.admit_selection(self.candidate, self.request)
                    } else {
                        Ok(())
                    }
                })
        };
        if let Err(e) = gate() {
            self.failure.set(Some(e));
            return Err(RuntimeError::ArtifactUnavailable);
        }
        crate::private_stages::observe(self.observer, stage);
        self.checkpoint()?;
        if let Err(e) = gate() {
            self.failure.set(Some(e));
            return Err(RuntimeError::ArtifactUnavailable);
        }
        self.checkpoint()
    }
}
impl<R: ModelStoreArtifact, A: LiveAuthority, S: TrustedInstalledModels<Lease = R>>
    ModelArtifactReader for LegacyLoadReader<'_, '_, '_, R, A, S>
{
    fn artifact_id(&self) -> &ArtifactId {
        self.candidate.reader.artifact_id()
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.candidate.reader.verified_integrity()
    }
    fn len(&self) -> u64 {
        self.candidate.reader.len()
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        destination.fill(0);
        if self.checkpoint().is_err() {
            return Err(ArtifactReadError::Unavailable);
        }
        let result = self.candidate.reader.read_at(offset, destination);
        if self.checkpoint().is_err() || result.is_err() {
            destination.fill(0);
            return Err(ArtifactReadError::Unavailable);
        }
        result
    }
}
struct VerifiedInstalledCandidate<R: AuthenticatedImmutableArtifact> {
    candidate: InstalledSelectionCandidate<R>,
    verified_manifest: ModelManifest,
    permit: Option<()>,
    failure: Cell<Option<InstalledSelectionError>>,
}
impl<R: AuthenticatedImmutableArtifact> VerifiedInstalledCandidate<R> {
    fn begin_load<'read, 'owner, A: LiveAuthority, S: TrustedInstalledModels<Lease = R>>(
        &'read mut self,
        control: &'read SelectionControl<'owner, A>,
        installed: &'read S,
        observer: Option<PrivateInstalledStageObserver>,
    ) -> Result<VerifiedLoadReader<'read, 'owner, R, A, S>, InstalledSelectionError> {
        self.permit.take().ok_or(InstalledSelectionError::Runtime(
            RuntimeError::IntegrityRequired,
        ))?;
        let reader = VerifiedLoadReader {
            proof: self,
            control,
            installed,
            observer,
        };
        if let Err(error) = reader.check() {
            return Err(reader
                .proof
                .failure
                .get()
                .unwrap_or(InstalledSelectionError::Runtime(error)));
        }
        Ok(reader)
    }
}
pub(crate) struct VerifiedLoadReader<
    'read,
    'owner,
    R: AuthenticatedImmutableArtifact,
    A: LiveAuthority,
    S: TrustedInstalledModels<Lease = R>,
> {
    proof: &'read mut VerifiedInstalledCandidate<R>,
    control: &'read SelectionControl<'owner, A>,
    installed: &'read S,
    observer: Option<PrivateInstalledStageObserver>,
}
impl<R: AuthenticatedImmutableArtifact, A: LiveAuthority, S: TrustedInstalledModels<Lease = R>>
    VerifiedLoadReader<'_, '_, R, A, S>
{
    pub(crate) fn manifest(&self) -> &ModelManifest {
        &self.proof.verified_manifest
    }
    fn check(&self) -> Result<(), RuntimeError> {
        if self.proof.failure.get().is_some() {
            return Err(RuntimeError::ArtifactUnavailable);
        }
        let checked = (|| {
            self.control.check()?;
            self.proof
                .candidate
                .reader
                .validate_lease()
                .map_err(InstalledSelectionError::Owner)?;
            if self.proof.candidate.receipt.manifest() != &self.proof.verified_manifest
                || self.proof.candidate.reader.store_identity()
                    != self.proof.candidate.receipt.store_identity()
                || self.proof.candidate.reader.revision()
                    != self.proof.candidate.receipt.artifact_revision()
            {
                return Err(InstalledSelectionError::Owner(ReceiptError::Stale));
            }
            self.control.check()
        })();
        if let Err(error) = checked {
            self.proof.failure.set(Some(error));
            return Err(RuntimeError::ArtifactUnavailable);
        }
        Ok(())
    }
    pub(crate) fn stage(
        &self,
        stage: PrivateInstalledStage,
        phase: InstalledLoadPhase,
    ) -> Result<(), RuntimeError> {
        self.check()?;
        self.owner_check(phase)?;
        crate::private_stages::observe(self.observer, stage);
        self.check()?;
        self.owner_check(phase)?;
        self.check()
    }
    fn owner_check(&self, phase: InstalledLoadPhase) -> Result<(), RuntimeError> {
        if let Err(error) = self.installed.revalidate_load(&self.proof.candidate, phase) {
            self.proof.failure.set(Some(error));
            return Err(RuntimeError::ArtifactUnavailable);
        }
        self.check()
    }
    pub(crate) fn checkpoint(&self) -> Result<(), RuntimeError> {
        self.check()
    }
}
impl<R: AuthenticatedImmutableArtifact, A: LiveAuthority, S: TrustedInstalledModels<Lease = R>>
    ModelArtifactReader for VerifiedLoadReader<'_, '_, R, A, S>
{
    fn artifact_id(&self) -> &ArtifactId {
        self.proof.candidate.reader.artifact_id()
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.proof.candidate.reader.verified_integrity()
    }
    fn len(&self) -> u64 {
        self.proof.candidate.reader.len()
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        destination.fill(0);
        if self.check().is_err() {
            return Err(ArtifactReadError::Unavailable);
        }
        let result = self.proof.candidate.reader.read_at(offset, destination);
        if self.check().is_err() || result.is_err() {
            destination.fill(0);
            return Err(ArtifactReadError::Unavailable);
        }
        result
    }
}

struct SelectionControl<'a, A: LiveAuthority> {
    authority: &'a A,
    live: &'a A::Session,
    cancel: &'a dyn CancellationToken,
    stamp: &'a A::Stamp,
    stopped: Cell<Option<ReceiptError>>,
}
impl<A: LiveAuthority> SelectionControl<'_, A> {
    fn check(&self) -> Result<(), InstalledSelectionError> {
        if let Some(error) = self.stopped.get() {
            return Err(InstalledSelectionError::Owner(error));
        }
        let result = if self.cancel.is_cancelled() {
            Err(ReceiptError::Cancelled)
        } else {
            self.authority.check(self.live).and_then(|stamp| {
                if stamp == *self.stamp {
                    Ok(())
                } else {
                    Err(ReceiptError::Stale)
                }
            })
        };
        if let Err(error) = result {
            self.stopped.set(Some(error));
        }
        result.map_err(InstalledSelectionError::Owner)
    }
}
impl<A: LiveAuthority> CancellationToken for SelectionControl<'_, A> {
    fn is_cancelled(&self) -> bool {
        self.check().is_err()
    }
}
struct SelectionReader<'a, 'b, R, A: LiveAuthority> {
    reader: &'a mut R,
    revision: u64,
    store_identity: crate::ModelStoreIdentity,
    control: &'a SelectionControl<'b, A>,
}
impl<R: ModelStoreArtifact, A: LiveAuthority> ModelArtifactReader
    for SelectionReader<'_, '_, R, A>
{
    fn artifact_id(&self) -> &ArtifactId {
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
        if let Err(error) = self.reader.validate_lease().and_then(|()| {
            if self.reader.revision() == self.revision
                && self.reader.store_identity() == self.store_identity
            {
                Ok(())
            } else {
                Err(ReceiptError::Stale)
            }
        }) {
            self.control.stopped.set(Some(error));
            return Err(ArtifactReadError::Unavailable);
        }
        let result = self.reader.read_at(offset, destination);
        if let Err(error) = self.reader.validate_lease().and_then(|()| {
            if self.reader.revision() == self.revision
                && self.reader.store_identity() == self.store_identity
            {
                Ok(())
            } else {
                Err(ReceiptError::Stale)
            }
        }) {
            self.control.stopped.set(Some(error));
            return Err(ArtifactReadError::Unavailable);
        }
        self.control
            .check()
            .map_err(|_| ArtifactReadError::Unavailable)?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::{FakeModelBackend, FakeSession, MemoryArtifactReader},
        BackendDescriptor, BackendResponse, CommitFence, ModelRequest, ModelStoreArtifacts,
        ReceiptEnvelope, ReceiptManager, ReceiptStore, ResourceBudget, TextChunkSink, TokenUsage,
    };
    use alloc::{format, rc::Rc, vec, vec::Vec};
    use sha2::{Digest, Sha256};
    struct NeverCancel;
    impl CancellationToken for NeverCancel {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    #[derive(Default)]
    struct Authority {
        calls: Cell<usize>,
        deny_at: Cell<Option<usize>>,
        stale_at: Cell<Option<usize>>,
    }
    impl LiveAuthority for Authority {
        type Session = u64;
        type Stamp = u64;
        fn check(&self, live: &u64) -> Result<u64, ReceiptError> {
            let n = self.calls.get() + 1;
            self.calls.set(n);
            if self.deny_at.get().is_some_and(|at| n >= at) {
                return Err(ReceiptError::Denied);
            }
            Ok(if self.stale_at.get().is_some_and(|at| n >= at) {
                live + 1
            } else {
                *live
            })
        }
    }
    struct Snapshot {
        identity: crate::ModelStoreIdentity,
        reader: MemoryArtifactReader,
        stale: bool,
        revoke_on_read: bool,
    }
    impl ModelArtifactReader for Snapshot {
        fn artifact_id(&self) -> &ArtifactId {
            self.reader.artifact_id()
        }
        fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
            self.reader.verified_integrity()
        }
        fn len(&self) -> u64 {
            self.reader.len()
        }
        fn read_at(&mut self, o: u64, d: &mut [u8]) -> Result<usize, ArtifactReadError> {
            let result = self.reader.read_at(o, d);
            if self.revoke_on_read {
                self.stale = true;
            }
            result
        }
    }
    impl ModelStoreArtifact for Snapshot {
        fn store_identity(&self) -> crate::ModelStoreIdentity {
            self.identity
        }
        fn revision(&self) -> u64 {
            3
        }
        fn validate_lease(&self) -> Result<(), ReceiptError> {
            if self.stale {
                Err(ReceiptError::Stale)
            } else {
                Ok(())
            }
        }
    }
    struct Sources;
    impl ModelStoreArtifacts for Sources {
        type Snapshot = Snapshot;
        fn capture(&mut self, m: &ModelManifest, _: u64) -> Result<Snapshot, ReceiptError> {
            Ok(Snapshot {
                identity: crate::ModelStoreIdentity(1),
                reader: MemoryArtifactReader::for_manifest(m, vec![1]),
                stale: false,
                revoke_on_read: false,
            })
        }
    }
    struct Store(ReceiptEnvelope);
    impl ReceiptStore for Store {
        fn store_identity(&self) -> crate::ModelStoreIdentity {
            crate::ModelStoreIdentity(1)
        }
        fn revision(&self) -> Result<u64, ReceiptError> {
            Ok(1)
        }
        fn commit<S: ModelStoreArtifact>(
            &mut self,
            _: u64,
            _: &ReceiptEnvelope,
            _: &S,
            _: &mut dyn CommitFence,
        ) -> Result<u64, ReceiptError> {
            Err(ReceiptError::Unavailable)
        }
        fn read(&mut self, _: &ModelId) -> Result<Option<ReceiptEnvelope>, ReceiptError> {
            Ok(Some(self.0.clone()))
        }
    }
    fn receipt(m: &ModelManifest) -> InstalledReceipt {
        let terms = m.license.terms_reference.clone();
        let mut manager = ReceiptManager::new(
            Authority::default(),
            Sources,
            Store(ReceiptEnvelope {
                manifest: m.clone(),
                store_identity: crate::ModelStoreIdentity(1),
                artifact_revision: 3,
                committed_revision: 1,
                acknowledged_terms: terms,
            }),
        );
        manager.reopen_receipts(&1, m, 1, &NeverCancel).unwrap()
    }
    struct Models {
        lease_identity: crate::ModelStoreIdentity,
        manifests: Vec<ModelManifest>,
        opens: Vec<ModelId>,
        bad: Option<ModelId>,
        stale: bool,
        policy_denied: bool,
        revoke_on_read: bool,
    }
    impl TrustedInstalledModels for Models {
        type Lease = Snapshot;
        fn capture_installed(
            &mut self,
            id: &ModelId,
        ) -> Result<InstalledSelectionCandidate<Snapshot>, InstalledSelectionError> {
            self.opens.push(id.clone());
            let m = self
                .manifests
                .iter()
                .find(|m| &m.model_id == id)
                .ok_or(InstalledSelectionError::Unavailable)?;
            let bytes = if self.bad.as_ref() == Some(id) {
                vec![2]
            } else {
                vec![1]
            };
            Ok(InstalledSelectionCandidate::from_trusted_owner(
                receipt(m),
                Snapshot {
                    identity: self.lease_identity,
                    reader: MemoryArtifactReader::for_manifest(m, bytes),
                    stale: self.stale,
                    revoke_on_read: self.revoke_on_read,
                },
            ))
        }
        fn admit_selection(
            &self,
            _: &InstalledSelectionCandidate<Snapshot>,
            _: &SelectionRequest<'_>,
        ) -> Result<(), InstalledSelectionError> {
            if self.policy_denied {
                Err(InstalledSelectionError::Owner(ReceiptError::Denied))
            } else {
                Ok(())
            }
        }
        fn revalidate_installed(
            &self,
            c: &InstalledSelectionCandidate<Snapshot>,
        ) -> Result<(), InstalledSelectionError> {
            c.reader
                .validate_lease()
                .map_err(InstalledSelectionError::Owner)
        }
    }
    struct TrackedSession {
        inner: FakeSession,
        live: Rc<Cell<u32>>,
        owns_resource: bool,
    }
    impl Drop for TrackedSession {
        fn drop(&mut self) {
            if self.owns_resource {
                self.live.set(self.live.get() - 1);
            }
        }
    }
    struct Backend {
        inner: FakeModelBackend,
        fail_load: Rc<Cell<bool>>,
        fail_unload: Rc<Cell<bool>>,
        peak: u32,
        live: Rc<Cell<u32>>,
    }
    impl ModelBackend for Backend {
        type Session = TrackedSession;
        fn descriptor(&self) -> &BackendDescriptor {
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
            r: &mut dyn ModelArtifactReader,
        ) -> Result<TrackedSession, RuntimeError> {
            if self.fail_load.replace(false) {
                return Err(RuntimeError::LoadFailed);
            }
            let inner = self.inner.load(m, r)?;
            self.live.set(self.live.get() + 1);
            self.peak = self.peak.max(self.live.get());
            Ok(TrackedSession {
                inner,
                live: self.live.clone(),
                owns_resource: true,
            })
        }
        fn infer(
            &mut self,
            s: &mut TrackedSession,
            r: &ModelRequest<'_>,
            c: &dyn CancellationToken,
        ) -> Result<BackendResponse, RuntimeError> {
            self.inner.infer(&mut s.inner, r, c)
        }
        fn infer_stream(
            &mut self,
            s: &mut TrackedSession,
            r: &ModelRequest<'_>,
            c: &dyn CancellationToken,
            k: &mut dyn TextChunkSink,
        ) -> Result<TokenUsage, RuntimeError> {
            self.inner.infer_stream(&mut s.inner, r, c, k)
        }
        fn unload(&mut self, mut s: TrackedSession) -> Result<(), RuntimeError> {
            self.unload_resident(&mut s)
        }
        fn unload_resident(&mut self, s: &mut TrackedSession) -> Result<(), RuntimeError> {
            if self.fail_unload.get() {
                return Err(RuntimeError::UnloadFailed);
            }
            self.inner.unload_resident(&mut s.inner)?;
            if s.owns_resource {
                s.live.set(s.live.get() - 1);
                s.owns_resource = false;
            }
            Ok(())
        }
    }
    fn fixture() -> (
        InstalledModelSelector<Backend>,
        Models,
        Authority,
        CapabilityId,
    ) {
        let mut a = ModelManifest::parse_json(
            include_str!("../tests/fixtures/granite-4.2-3b.json").as_bytes(),
        )
        .unwrap();
        a.artifact.size_bytes = Some(1);
        a.artifact.integrity = Some(IntegrityMetadata {
            algorithm: "sha256".into(),
            digest: format!("{:x}", Sha256::digest([1u8])),
        });
        let mut b = a.clone();
        b.model_id = ModelId::new("synthetic.second").unwrap();
        let backend = Backend {
            inner: FakeModelBackend::for_manifest(&a),
            fail_load: Rc::new(Cell::new(false)),
            fail_unload: Rc::new(Cell::new(false)),
            peak: 0,
            live: Rc::new(Cell::new(0)),
        };
        (
            InstalledModelSelector::new(backend),
            Models {
                lease_identity: crate::ModelStoreIdentity(1),
                manifests: vec![a, b],
                opens: vec![],
                bad: None,
                stale: false,
                policy_denied: false,
                revoke_on_read: false,
            },
            Authority::default(),
            crate::CapabilityId::new("text.generate").unwrap(),
        )
    }
    fn request(cap: &crate::CapabilityId) -> SelectionRequest<'_> {
        SelectionRequest {
            capability: cap,
            role: None,
            minimum_context_tokens: 1,
            resources: ResourceBudget {
                available_ram_bytes: u64::MAX,
                available_storage_bytes: u64::MAX,
                cpu_cores: 255,
                gpu_available: true,
            },
            target_architecture: "x86_64",
            offline_only: true,
            preferred_model_id: None,
            role_default_model_id: None,
        }
    }
    use crate::CapabilityId;
    fn pick(
        s: &mut InstalledModelSelector<Backend>,
        m: &mut Models,
        a: &Authority,
        cap: &CapabilityId,
        id: &ModelId,
    ) -> Result<ModelId, InstalledSelectionError> {
        s.select(
            InstalledSelectionMode::Manual(id),
            &request(cap),
            a,
            &1,
            &NeverCancel,
            m,
        )
    }
    #[test]
    fn switch_retires_before_loading_and_never_exceeds_one_resident() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        pick(&mut s, &mut m, &a, &cap, &ids[0]).unwrap();
        pick(&mut s, &mut m, &a, &cap, &ids[1]).unwrap();
        assert_eq!(s.model_id(), Some(&ids[1]));
        assert_eq!(s.backend().peak, 1);
        assert_eq!(s.backend().live.get(), 1);
        assert_eq!(s.backend().inner.unload_count, 1);
    }
    #[test]
    fn invalid_bytes_and_resource_admission_keep_old_resident() {
        let (mut s, mut m, a, cap) = fixture();
        let old = m.manifests[0].model_id.clone();
        let new = m.manifests[1].model_id.clone();
        pick(&mut s, &mut m, &a, &cap, &old).unwrap();
        m.bad = Some(new.clone());
        assert_eq!(
            pick(&mut s, &mut m, &a, &cap, &new),
            Err(InstalledSelectionError::Runtime(
                RuntimeError::IntegrityMismatch
            ))
        );
        assert_eq!(s.model_id(), Some(&old));
        m.bad = None;
        let mut r = request(&cap);
        r.resources.available_ram_bytes = 0;
        assert_eq!(
            s.select(
                InstalledSelectionMode::Manual(&new),
                &r,
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::AdmissionRejected)
        );
        assert_eq!(s.model_id(), Some(&old));
        assert_eq!(s.backend().inner.load_count, 1);
    }
    #[test]
    fn retirement_failure_quarantines_without_loading_or_fallback() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        pick(&mut s, &mut m, &a, &cap, &ids[0]).unwrap();
        s.backend().fail_unload.set(true);
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&[ids[1].clone(), ids[0].clone()]),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Runtime(RuntimeError::UnloadFailed))
        );
        assert!(s.is_quarantined());
        assert_eq!(s.model_id(), None);
        assert_eq!(s.resource_report().loaded_model_sessions, 1);
        assert_eq!(m.opens.len(), 2);
        assert_eq!(
            pick(&mut s, &mut m, &a, &cap, &ids[0]),
            Err(InstalledSelectionError::Quarantined)
        );
        let retained = s.backend().live.clone();
        assert_eq!(retained.get(), 1);
        drop(s);
        assert_eq!(retained.get(), 0);
    }
    #[test]
    fn manual_load_failure_has_no_substitution_automatic_uses_installed_only() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        s.backend().fail_load.set(true);
        assert_eq!(
            pick(&mut s, &mut m, &a, &cap, &ids[0]),
            Err(InstalledSelectionError::Runtime(RuntimeError::LoadFailed))
        );
        assert_eq!(m.opens.len(), 1);
        s.backend().fail_load.set(true);
        let missing = ModelId::new("synthetic.missing").unwrap();
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&[missing, ids[0].clone(), ids[1].clone()]),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Ok(ids[1].clone())
        );
        assert_eq!(s.backend().inner.load_count, 1);
    }
    #[test]
    fn live_denial_stale_and_cancel_stop_without_fallback() {
        for stale in [false, true] {
            let (mut s, mut m, a, cap) = fixture();
            let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
            if stale {
                a.stale_at.set(Some(5));
            } else {
                a.deny_at.set(Some(5));
            }
            let error = if stale {
                ReceiptError::Stale
            } else {
                ReceiptError::Denied
            };
            assert_eq!(
                s.select(
                    InstalledSelectionMode::Automatic(&ids),
                    &request(&cap),
                    &a,
                    &1,
                    &NeverCancel,
                    &mut m
                ),
                Err(InstalledSelectionError::Owner(error))
            );
            assert_eq!(m.opens.len(), 1);
            assert_eq!(s.backend().inner.load_count, 0);
        }
        struct Cancel;
        impl CancellationToken for Cancel {
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        let (mut s, mut m, a, cap) = fixture();
        let id = m.manifests[0].model_id.clone();
        assert_eq!(
            s.select(
                InstalledSelectionMode::Manual(&id),
                &request(&cap),
                &a,
                &1,
                &Cancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Cancelled))
        );
        assert!(m.opens.is_empty());
    }
    #[test]
    fn missing_authority_and_stale_lease_fail_closed() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        a.deny_at.set(Some(1));
        assert_eq!(
            pick(&mut s, &mut m, &a, &cap, &ids[0]),
            Err(InstalledSelectionError::Owner(ReceiptError::Denied))
        );
        assert!(m.opens.is_empty());
        let a = Authority::default();
        m.stale = true;
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&ids),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Stale))
        );
        assert_eq!(m.opens.len(), 1);
    }
    #[test]
    fn last_authority_gate_cleans_new_resident_and_cleanup_failure_quarantines() {
        let (mut baseline, mut models, authority, cap) = fixture();
        let id = models.manifests[0].model_id.clone();
        pick(&mut baseline, &mut models, &authority, &cap, &id).unwrap();
        let final_call = authority.calls.get();
        for cleanup_fails in [false, true] {
            let (mut s, mut m, a, cap) = fixture();
            a.deny_at.set(Some(final_call));
            s.backend().fail_unload.set(cleanup_fails);
            assert_eq!(
                pick(&mut s, &mut m, &a, &cap, &id),
                Err(InstalledSelectionError::Owner(ReceiptError::Denied))
            );
            assert_eq!(s.backend().inner.load_count, 1);
            assert_eq!(s.model_id(), None);
            assert_eq!(s.is_quarantined(), cleanup_fails);
            assert_eq!(
                s.resource_report().loaded_model_sessions,
                u32::from(cleanup_fails)
            );
        }
    }
    #[test]
    fn policy_denial_and_lease_revoke_during_read_preserve_old_and_stop_fallback() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        pick(&mut s, &mut m, &a, &cap, &ids[0]).unwrap();
        m.policy_denied = true;
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&ids[1..]),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Denied))
        );
        assert_eq!(s.model_id(), Some(&ids[0]));
        m.policy_denied = false;
        m.revoke_on_read = true;
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&[ids[1].clone(), ids[0].clone()]),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Stale))
        );
        assert_eq!(s.model_id(), Some(&ids[0]));
        assert_eq!(m.opens.len(), 3);
        assert_eq!(s.backend().inner.load_count, 1);
    }
    #[test]
    fn role_architecture_capability_context_and_explicit_preference_are_preflight_gates() {
        let (mut s, mut m, a, cap) = fixture();
        let old = m.manifests[0].model_id.clone();
        let new = m.manifests[1].model_id.clone();
        pick(&mut s, &mut m, &a, &cap, &old).unwrap();
        let bad_role = crate::RoleId::new("synthetic.missing").unwrap();
        let bad_cap = CapabilityId::new("synthetic.missing").unwrap();
        for variant in 0..5 {
            let mut r = request(&cap);
            match variant {
                0 => r.role = Some(&bad_role),
                1 => r.target_architecture = "aarch64",
                2 => r.capability = &bad_cap,
                3 => r.minimum_context_tokens = u32::MAX,
                _ => r.preferred_model_id = Some(&old),
            }
            assert_eq!(
                s.select(
                    InstalledSelectionMode::Manual(&new),
                    &r,
                    &a,
                    &1,
                    &NeverCancel,
                    &mut m
                ),
                Err(InstalledSelectionError::AdmissionRejected)
            );
            assert_eq!(s.model_id(), Some(&old));
        }
        assert_eq!(s.backend().inner.load_count, 1);
    }
    #[test]
    fn unavailable_live_owner_never_reads_installed_store() {
        struct Missing;
        impl LiveAuthority for Missing {
            type Session = u64;
            type Stamp = u64;
            fn check(&self, _: &u64) -> Result<u64, ReceiptError> {
                Err(ReceiptError::Unavailable)
            }
        }
        let (mut s, mut m, _, cap) = fixture();
        let id = m.manifests[0].model_id.clone();
        assert_eq!(
            s.select(
                InstalledSelectionMode::Manual(&id),
                &request(&cap),
                &Missing,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Unavailable))
        );
        assert!(m.opens.is_empty());
        assert_eq!(s.backend().live.get(), 0);
    }
    #[test]
    fn cancellation_during_bounded_hash_read_stops_fallback_and_keeps_old() {
        struct CancelAfter(Cell<usize>);
        impl CancellationToken for CancelAfter {
            fn is_cancelled(&self) -> bool {
                let n = self.0.get();
                self.0.set(n + 1);
                n >= 7
            }
        }
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        pick(&mut s, &mut m, &a, &cap, &ids[0]).unwrap();
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&[ids[1].clone(), ids[0].clone()]),
                &request(&cap),
                &a,
                &1,
                &CancelAfter(Cell::new(0)),
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Cancelled))
        );
        assert_eq!(s.model_id(), Some(&ids[0]));
        assert_eq!(m.opens.len(), 2);
        assert_eq!(s.backend().inner.load_count, 1);
    }
    #[test]
    fn artifact_from_another_store_cannot_bind_an_installed_receipt() {
        let (mut s, mut m, a, cap) = fixture();
        let ids: Vec<_> = m.manifests.iter().map(|m| m.model_id.clone()).collect();
        m.lease_identity = crate::ModelStoreIdentity(2);
        assert_eq!(
            s.select(
                InstalledSelectionMode::Automatic(&ids),
                &request(&cap),
                &a,
                &1,
                &NeverCancel,
                &mut m
            ),
            Err(InstalledSelectionError::Owner(ReceiptError::Stale))
        );
        assert_eq!(m.opens.len(), 1);
        assert_eq!(s.backend().live.get(), 0);
    }
    mod runtime_bridge {
        use super::*;
        use crate::{
            InstalledModelService, LiveRequestAuthority, ModelServiceClock, ModelServiceLimits,
            TrustedResidentModels,
        };
        struct State {
            authority_checks: Cell<u64>,
            rotate_at: Cell<u64>,
            locked: Cell<bool>,
            incarnation: Cell<u64>,
            epoch: Cell<u64>,
            cancel: Cell<bool>,
            stale_receipt: Cell<bool>,
            policy: Cell<bool>,
            response_race: Cell<bool>,
            final_admit_revoke: Cell<bool>,
            final_admit_receipt: Cell<bool>,
            revoke_next_admit: Cell<bool>,
            polls: Cell<u64>,
            race_at: Cell<u64>,
            race_mode: Cell<u8>,
            clock_step: Cell<u64>,
            clock_failed: Cell<bool>,
            late_response: Cell<bool>,
            deadline_revalidate: Cell<u8>,
            clock_jump_to: Cell<u64>,
        }
        struct BridgeAuthority(Rc<State>);
        impl LiveAuthority for BridgeAuthority {
            type Session = u64;
            type Stamp = (u64, u64);
            fn check(&self, live: &u64) -> Result<Self::Stamp, ReceiptError> {
                let n = self.0.authority_checks.get() + 1;
                self.0.authority_checks.set(n);
                if n == self.0.rotate_at.get() {
                    self.0.incarnation.set(self.0.incarnation.get() + 1);
                }
                if *live != 1 || self.0.locked.get() {
                    return Err(ReceiptError::Denied);
                }
                Ok((*live, self.0.incarnation.get()))
            }
        }
        impl LiveRequestAuthority for BridgeAuthority {
            fn check_request_epoch(&self, _: &u64, epoch: u64) -> Result<(), ReceiptError> {
                if epoch == self.0.epoch.get() {
                    Ok(())
                } else {
                    Err(ReceiptError::Stale)
                }
            }
        }
        struct BridgeModels {
            inner: Models,
            state: Rc<State>,
        }
        impl TrustedInstalledModels for BridgeModels {
            type Lease = Snapshot;
            fn capture_installed(
                &mut self,
                id: &ModelId,
            ) -> Result<InstalledSelectionCandidate<Snapshot>, InstalledSelectionError>
            {
                self.inner.capture_installed(id)
            }
            fn revalidate_installed(
                &self,
                c: &InstalledSelectionCandidate<Snapshot>,
            ) -> Result<(), InstalledSelectionError> {
                self.inner.revalidate_installed(c)
            }
            fn admit_selection(
                &self,
                c: &InstalledSelectionCandidate<Snapshot>,
                r: &SelectionRequest<'_>,
            ) -> Result<(), InstalledSelectionError> {
                if self.state.policy.get() {
                    return Err(InstalledSelectionError::Owner(ReceiptError::Denied));
                }
                self.inner.admit_selection(c, r)
            }
        }
        impl TrustedResidentModels for BridgeModels {
            fn revalidate_resident(
                &self,
                r: &InstalledReceipt,
            ) -> Result<(), InstalledSelectionError> {
                let n = self.state.deadline_revalidate.get();
                if n > 0 {
                    self.state.deadline_revalidate.set(n - 1);
                    if n == 1 {
                        self.state.clock_jump_to.set(2000);
                    }
                }
                if self.state.stale_receipt.get()
                    || r.store_identity() != crate::ModelStoreIdentity(1)
                    || r.artifact_revision() != 3
                    || !self.inner.manifests.iter().any(|m| m == r.manifest())
                {
                    Err(InstalledSelectionError::Owner(ReceiptError::Stale))
                } else {
                    Ok(())
                }
            }
            fn admit_inference(
                &self,
                _: &InstalledReceipt,
                _: &ModelRequest<'_>,
            ) -> Result<(), InstalledSelectionError> {
                if self.state.revoke_next_admit.replace(false) {
                    if self.state.final_admit_receipt.get() {
                        self.state.stale_receipt.set(true);
                    } else {
                        self.state.locked.set(true);
                    }
                }
                if self.state.policy.get() {
                    Err(InstalledSelectionError::Owner(ReceiptError::Denied))
                } else {
                    Ok(())
                }
            }
            fn admit_response(
                &self,
                _: &InstalledReceipt,
                _: &crate::ModelResponse,
            ) -> Result<(), InstalledSelectionError> {
                if self.state.late_response.get() {
                    self.state.deadline_revalidate.set(2);
                }
                if self.state.final_admit_revoke.get() {
                    self.state.revoke_next_admit.set(true);
                }
                if self.state.response_race.get() {
                    self.state.epoch.set(self.state.epoch.get() + 1);
                }
                Ok(())
            }
        }
        struct Clock {
            state: Rc<State>,
            now: Cell<u64>,
        }
        impl ModelServiceClock for Clock {
            fn now_millis(&self) -> Option<u64> {
                if self.state.clock_failed.get() {
                    return None;
                }
                let n = self.now.get().max(self.state.clock_jump_to.get());
                self.now.set(n + self.state.clock_step.get());
                Some(n)
            }
            fn cooperate(&self) {
                let n = self.state.polls.get() + 1;
                self.state.polls.set(n);
                if n == self.state.race_at.get() {
                    match self.state.race_mode.get() {
                        0 => self.state.locked.set(true),
                        1 => self.state.epoch.set(99),
                        2 => self.state.stale_receipt.set(true),
                        _ => self.state.cancel.set(true),
                    }
                }
            }
        }
        struct Cancel(Rc<State>);
        impl CancellationToken for Cancel {
            fn is_cancelled(&self) -> bool {
                self.0.cancel.get()
            }
        }
        type Service = InstalledModelService<Backend, BridgeAuthority, BridgeModels, Clock>;
        fn setup() -> (Service, Rc<State>, CapabilityId, ModelId) {
            let (_unused, models, _a, cap) = fixture();
            let id = models.manifests[0].model_id.clone();
            let backend = Backend {
                inner: FakeModelBackend::for_manifest(&models.manifests[0]),
                fail_load: Rc::new(Cell::new(false)),
                fail_unload: Rc::new(Cell::new(false)),
                peak: 0,
                live: Rc::new(Cell::new(0)),
            };
            let state = Rc::new(State {
                authority_checks: Cell::new(0),
                rotate_at: Cell::new(0),
                locked: Cell::new(false),
                incarnation: Cell::new(1),
                epoch: Cell::new(1),
                cancel: Cell::new(false),
                stale_receipt: Cell::new(false),
                policy: Cell::new(false),
                response_race: Cell::new(false),
                final_admit_revoke: Cell::new(false),
                final_admit_receipt: Cell::new(false),
                revoke_next_admit: Cell::new(false),
                polls: Cell::new(0),
                race_at: Cell::new(0),
                race_mode: Cell::new(0),
                clock_step: Cell::new(0),
                clock_failed: Cell::new(false),
                late_response: Cell::new(false),
                deadline_revalidate: Cell::new(0),
                clock_jump_to: Cell::new(0),
            });
            let service = Service::new(
                backend,
                BridgeAuthority(state.clone()),
                BridgeModels {
                    inner: models,
                    state: state.clone(),
                },
                Clock {
                    state: state.clone(),
                    now: Cell::new(0),
                },
                ModelServiceLimits {
                    max_input_bytes: 256,
                    max_output_bytes: 256,
                    max_output_tokens: 16,
                    max_request_millis: 1000,
                },
            )
            .unwrap();
            (service, state, cap, id)
        }
        fn selected(service: &mut Service, cap: &CapabilityId, id: &ModelId) {
            service
                .select(
                    InstalledSelectionMode::Manual(id),
                    &request(cap),
                    &1,
                    &NeverCancel,
                )
                .unwrap();
        }
        fn generation(cap: &CapabilityId, id: u64) -> ModelRequest<'_> {
            ModelRequest {
                request_id: id,
                caller: None,
                capability: cap,
                system_prompt: None,
                input: "synthetic host prompt",
                input_tokens: Some(1),
                max_output_tokens: 8,
                options: crate::GenerationOptions::default(),
                timeout_millis: Some(1000),
                structured_output: None,
            }
        }
        #[test]
        fn two_finite_requests_reuse_one_actual_counted_resident() {
            let (mut service, state, cap, id) = setup();
            assert_eq!(service.backend().inner.load_count, 0);
            selected(&mut service, &cap, &id);
            let response = service
                .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                .unwrap();
            assert_eq!(response.model_id, id);
            state.epoch.set(2);
            service
                .generate(2, &1, &generation(&cap, 2), &NeverCancel)
                .unwrap();
            assert_eq!(service.backend().inner.load_count, 1);
            assert_eq!(service.backend().inner.infer_count, 2);
            assert_eq!(service.backend().peak, 1);
            assert_eq!(service.backend().live.get(), 1);
        }
        #[test]
        fn old_epoch_receipt_and_live_session_fail_without_inference() {
            for mode in 0..4 {
                let (mut service, state, cap, id) = setup();
                selected(&mut service, &cap, &id);
                match mode {
                    0 => state.epoch.set(2),
                    1 => state.stale_receipt.set(true),
                    2 => state.locked.set(true),
                    _ => state.incarnation.set(2),
                };
                assert!(service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .is_err());
                assert_eq!(service.backend().inner.infer_count, 0);
                assert_eq!(service.backend().live.get(), 0);
            }
        }
        #[test]
        fn canonical_revoke_cancel_epoch_and_receipt_changes_during_checkpoint_discard() {
            for phase in [3, 5] {
                for mode in 0..4 {
                    let (mut service, state, cap, id) = setup();
                    selected(&mut service, &cap, &id);
                    state.race_at.set(phase);
                    state.race_mode.set(mode);
                    assert!(service
                        .generate(1, &1, &generation(&cap, 1), &Cancel(state.clone()))
                        .is_err());
                    assert_eq!(service.backend().live.get(), 0);
                    assert!(service.model_id().is_none());
                }
            }
        }
        #[test]
        fn response_admission_epoch_race_never_exposes_output() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.response_race.set(true);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Stale))
            );
            assert_eq!(service.backend().inner.infer_count, 1);
            assert_eq!(service.backend().live.get(), 0);
        }
        #[test]
        fn failure_cleanup_quarantines_retained_owner_no_new_load_or_fallback() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            service.backend().fail_unload.set(true);
            state.response_race.set(true);
            assert!(service
                .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                .is_err());
            assert!(service.is_quarantined());
            assert_eq!(service.backend().live.get(), 1);
            assert!(service
                .select(
                    InstalledSelectionMode::Manual(&id),
                    &request(&cap),
                    &1,
                    &NeverCancel
                )
                .is_err());
            assert_eq!(service.backend().inner.load_count, 1);
            let live = service.backend().live.clone();
            drop(service);
            assert_eq!(live.get(), 0);
        }
        #[test]
        fn limits_policy_and_external_caller_are_checked_before_backend() {
            for mode in 0..4 {
                let (mut service, state, cap, id) = setup();
                selected(&mut service, &cap, &id);
                let mut req = generation(&cap, 1);
                match mode {
                    0 => req.max_output_tokens = 17,
                    1 => req.timeout_millis = None,
                    2 => state.policy.set(true),
                    _ => req.caller = Some(nagi_model::AppId(9)),
                };
                assert!(service.generate(1, &1, &req, &NeverCancel).is_err());
                assert_eq!(service.backend().inner.infer_count, 0);
            }
        }
        #[test]
        fn monotonic_deadline_and_unavailable_clock_never_fake_success() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.clock_step.set(600);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Runtime(RuntimeError::Timeout))
            );
            assert_eq!(service.backend().live.get(), 0);
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.clock_failed.set(true);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Unavailable))
            );
            assert_eq!(service.backend().inner.infer_count, 0);
            // A clock failure after a valid selection does not silently destroy
            // an existing resident; the owner may explicitly unload it.
            assert_eq!(service.model_id(), Some(&id));
            assert_eq!(service.backend().inner.unload_count, 0);
            assert_eq!(service.backend().live.get(), 1);
        }
        #[test]
        fn unavailable_clock_rejects_selection_before_load_and_preserves_old_resident() {
            let (mut service, state, cap, id) = setup();
            state.clock_failed.set(true);
            assert_eq!(
                service.select(
                    InstalledSelectionMode::Manual(&id),
                    &request(&cap),
                    &1,
                    &NeverCancel
                ),
                Err(InstalledSelectionError::Owner(ReceiptError::Unavailable))
            );
            assert!(service.model_id().is_none());
            assert_eq!(service.backend().inner.load_count, 0);
            assert_eq!(service.backend().inner.infer_count, 0);
            assert_eq!(service.backend().inner.unload_count, 0);
            assert_eq!(service.backend().live.get(), 0);

            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.clock_failed.set(true);
            assert_eq!(
                service.select(
                    InstalledSelectionMode::Manual(&id),
                    &request(&cap),
                    &1,
                    &NeverCancel
                ),
                Err(InstalledSelectionError::Owner(ReceiptError::Unavailable))
            );
            assert_eq!(service.model_id(), Some(&id));
            assert_eq!(service.backend().inner.load_count, 1);
            assert_eq!(service.backend().inner.infer_count, 0);
            assert_eq!(service.backend().inner.unload_count, 0);
            assert_eq!(service.backend().live.get(), 1);
        }
        #[test]
        fn replay_and_explicit_lock_sync_cannot_publish_or_reuse_after_retirement() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            service
                .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                .unwrap();
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Stale))
            );
            assert_eq!(service.backend().inner.infer_count, 1);
            state.locked.set(true);
            assert!(service.sync_session(&1, &NeverCancel).is_err());
            assert_eq!(service.backend().live.get(), 0);
        }
        #[test]
        fn final_owner_admission_revoke_discards_completed_response() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.final_admit_revoke.set(true);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Denied))
            );
            assert_eq!(service.backend().inner.infer_count, 1);
            assert_eq!(service.backend().live.get(), 0);
        }
        #[test]
        fn candidate_policy_denial_preserves_old_when_actual_session_is_unchanged() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.policy.set(true);
            let second = ModelId::new("synthetic.second").unwrap();
            assert_eq!(
                service
                    .select(
                        InstalledSelectionMode::Manual(&second),
                        &request(&cap),
                        &1,
                        &NeverCancel
                    )
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Denied))
            );
            assert_eq!(service.model_id(), Some(&id));
            assert_eq!(service.backend().inner.load_count, 1);
            assert_eq!(service.backend().live.get(), 1);
            state.policy.set(false);
            service
                .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                .unwrap();
        }
        #[test]
        fn select_stamp_rotation_after_sync_retires_old_even_if_candidate_denied() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.policy.set(true);
            state.rotate_at.set(state.authority_checks.get() + 2);
            let second = ModelId::new("synthetic.second").unwrap();
            assert_eq!(
                service
                    .select(
                        InstalledSelectionMode::Manual(&second),
                        &request(&cap),
                        &1,
                        &NeverCancel
                    )
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Stale))
            );
            assert_eq!(service.backend().live.get(), 0);
        }

        #[test]
        fn final_owner_callback_receipt_revocation_discards_completed_response() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.final_admit_revoke.set(true);
            state.final_admit_receipt.set(true);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Owner(ReceiptError::Stale))
            );
            assert_eq!(service.backend().inner.infer_count, 1);
            assert_eq!(service.backend().live.get(), 0);
        }
        #[test]
        fn final_receipt_callback_crossing_deadline_discards_completed_response() {
            let (mut service, state, cap, id) = setup();
            selected(&mut service, &cap, &id);
            state.late_response.set(true);
            assert_eq!(
                service
                    .generate(1, &1, &generation(&cap, 1), &NeverCancel)
                    .err(),
                Some(InstalledSelectionError::Runtime(RuntimeError::Timeout))
            );
            assert_eq!(service.backend().inner.infer_count, 1);
            assert_eq!(service.backend().live.get(), 0);
        }
    }
}

#[cfg(test)]
#[path = "installed_proof_tests.rs"]
mod proof_tests;
