//! Task-25 PackageService snapshot adapter for the task-41 PID 1 candidate.
//!
//! This candidate borrows either the task25 owner or the distinct boot
//! DevPreview session owner by reference. Task25's public Stage13 adapter
//! accepts both lease types and keeps the exact PackageService borrow alive
//! through each synchronous callback. It does not create an owner or grant
//! session, consent, Model terms, durable service effect, or READY.

use super::{
    package_request_receipt_revoker_seal, service_effect_commit_seal, PackageRequestReceiptRevoker,
    PeerDenial, Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary, Pid1RunInstance,
    Pid1RunOwnerLease, ProcessInstanceKey, RegistrationId, ServiceRequestAdmission,
    ServiceRequestReceiptTransaction, TrustedInventory, TrustedManifest,
};
use nagi_dev_service_catalog_consumer::stage13_borrow_adapter_candidate::{
    with_stage13_borrowed_inventory, Stage13BorrowedInventoryConsumer, Stage13PackageServiceLease,
};
use nagi_dev_service_catalog_consumer::{
    GenerationBoundCommitError, GenerationBoundOperationRequest, OwnedPackageInventorySnapshot,
    PackageOwnerError, PackageServiceSessionOwner, SnapshotError,
};
#[cfg(test)]
use nagi_dev_service_catalog_consumer::{
    PackageInventorySnapshot, PackageInventorySnapshotSupplier,
};
use nagi_init_package_provider::package_trust as boot_package_trust;
use nagi_model::AppId;
use nagi_package::PackageService;
use nagi_package::PackageView;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::MutexGuard;
use std::sync::{Arc, Mutex};

/// A borrow of an already-owned PackageService session. Its generation lease
/// supplies same-instance inventory reads; a task25 receipt remains a local
/// receipt and not a service effect. A different process still has no IPC
/// lease/CAS and must remain unavailable.
pub(crate) struct SerializedPackageServiceAuthority<'owner, 'run, Owner: SessionPackageServiceOwner>
{
    owner: &'owner Owner,
    snapshot: Owner::Snapshot,
    trusted_manifest: TrustedManifest,
    run: Pid1RunInstance,
    run_owner_lease: &'run super::Pid1RunOwnerLease,
    request_state: Arc<PackageServiceRequestRevocationState>,
}

const MAX_PID1_PACKAGE_REQUESTS: usize = 32;

struct PackageServiceRequestState {
    admissions: Vec<(NonZeroU64, ServiceRequestAdmission)>,
    outcomes: Vec<PackageServiceReceiptOutcome>,
}

/// Local receipt table shared by the PackageService adapter and the PID1
/// lifecycle revocation hook. The exact registration/process pair is fixed at
/// capture; the directory owns only a Weak reference to this state.
struct PackageServiceRequestRevocationState {
    run: Pid1RunInstance,
    registration: Option<RegistrationId>,
    process: Option<ProcessInstanceKey>,
    revoked: AtomicBool,
    requests: Mutex<PackageServiceRequestState>,
}

impl PackageServiceRequestState {
    /// Release a known, terminal local-receipt slot when its admission is
    /// revoked. Unknown/in-progress outcomes remain as tombstones so no
    /// unresolved receipt can be retried merely to recover capacity.
    fn revoke_outcome(&mut self, request_id: NonZeroU64) {
        let Some(index) = self
            .outcomes
            .iter()
            .position(|outcome| outcome.request_id == request_id)
        else {
            return;
        };
        match self.outcomes[index].state {
            super::ServiceRequestOperationState::NotInvoked
            | super::ServiceRequestOperationState::ReceiptRecorded => {
                self.outcomes.remove(index);
            }
            super::ServiceRequestOperationState::Executing
            | super::ServiceRequestOperationState::OutcomeUnknown
            | super::ServiceRequestOperationState::Executed
            | super::ServiceRequestOperationState::Failed => {
                let outcome = &mut self.outcomes[index];
                outcome.state = super::ServiceRequestOperationState::OutcomeUnknown;
                outcome.receipt = None;
            }
        }
    }

    fn revoke_all(&mut self) {
        self.admissions.clear();
        self.outcomes.retain_mut(|outcome| match outcome.state {
            super::ServiceRequestOperationState::NotInvoked
            | super::ServiceRequestOperationState::ReceiptRecorded => false,
            super::ServiceRequestOperationState::Executing
            | super::ServiceRequestOperationState::OutcomeUnknown
            | super::ServiceRequestOperationState::Executed
            | super::ServiceRequestOperationState::Failed => {
                outcome.state = super::ServiceRequestOperationState::OutcomeUnknown;
                outcome.receipt = None;
                true
            }
        });
    }
}

impl package_request_receipt_revoker_seal::Sealed for PackageServiceRequestRevocationState {}

impl PackageRequestReceiptRevoker for PackageServiceRequestRevocationState {
    fn revoke_registration_under_gate(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
    ) -> Result<(), PeerDenial> {
        if !self.run.is_same_run(run) {
            return Err(PeerDenial::StaleRun);
        }
        if self.registration != Some(registration) || self.process != Some(process) {
            return Ok(());
        }
        // Latch denial before touching the table. If the table is busy or
        // poisoned, Supervisor retirement remains pending and can be retried;
        // no stale admission can be used in the meantime.
        self.revoked.store(true, Ordering::SeqCst);
        let mut state = self
            .requests
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let request_ids = state
            .admissions
            .iter()
            .filter(|(_, admission)| {
                admission.registration() == registration && admission.process() == process
            })
            .map(|(request_id, _)| *request_id)
            .collect::<Vec<_>>();
        state.admissions.retain(|(_, admission)| {
            admission.registration() != registration || admission.process() != process
        });
        for request_id in request_ids {
            state.revoke_outcome(request_id);
        }
        Ok(())
    }

    fn revoke_run_under_gate(&self, run: &Pid1RunInstance) -> Result<(), PeerDenial> {
        if !self.run.is_same_run(run) {
            return Err(PeerDenial::StaleRun);
        }
        self.revoked.store(true, Ordering::SeqCst);
        self.requests
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .revoke_all();
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct PackageServiceReceiptOutcome {
    request_id: NonZeroU64,
    state: super::ServiceRequestOperationState,
    receipt: Option<nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt>,
}

/// Shared adapter over the two distinct owners used by the task25 candidate
/// and the boot DevPreview session. The boot implementation borrows its
/// existing owner value; this trait never constructs or replaces one.
pub(crate) trait SessionPackageServiceOwner: Sync {
    type Snapshot: Copy + Send + Sync;
    type Lease<'owner>: Stage13PackageServiceLease
    where
        Self: 'owner;

    fn capture_snapshot(
        &self,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
    ) -> Result<Self::Snapshot, PeerDenial>;

    fn lease_for_dispatch<'owner>(
        &'owner self,
        snapshot: &Self::Snapshot,
    ) -> Result<Self::Lease<'owner>, PeerDenial>;

    fn snapshot_app_id(snapshot: Self::Snapshot) -> u64;
    fn snapshot_generation(snapshot: Self::Snapshot) -> u64;
    fn snapshot_manifest_id(snapshot: &Self::Snapshot) -> &[u8];

    fn commit_local_receipt(
        &self,
        _snapshot: Self::Snapshot,
        _record_id: u64,
    ) -> Result<nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }
}

impl SessionPackageServiceOwner for PackageServiceSessionOwner<'_> {
    type Snapshot = OwnedPackageInventorySnapshot;
    type Lease<'owner>
        = nagi_dev_service_catalog_consumer::PackageServiceGenerationLease<'owner>
    where
        Self: 'owner;

    fn capture_snapshot(
        &self,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
    ) -> Result<Self::Snapshot, PeerDenial> {
        self.capture(package_bytes, expected_manifest_id)
            .map_err(map_package_owner_error)
    }

    fn lease_for_dispatch<'owner>(
        &'owner self,
        snapshot: &Self::Snapshot,
    ) -> Result<Self::Lease<'owner>, PeerDenial> {
        self.lease_for_dispatch(snapshot)
            .map_err(map_package_owner_error)
    }

    fn snapshot_app_id(snapshot: Self::Snapshot) -> u64 {
        snapshot.app_id().0
    }

    fn snapshot_generation(snapshot: Self::Snapshot) -> u64 {
        snapshot.generation()
    }

    fn snapshot_manifest_id(snapshot: &Self::Snapshot) -> &[u8] {
        snapshot.manifest_id()
    }

    fn commit_local_receipt(
        &self,
        snapshot: Self::Snapshot,
        record_id: u64,
    ) -> Result<nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt, PeerDenial> {
        self.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, record_id))
            .map_err(map_generation_bound_error)
    }
}

impl SessionPackageServiceOwner for boot_package_trust::PackageServiceSessionOwner<'_> {
    type Snapshot = boot_package_trust::OwnedPackageInventorySnapshot;
    type Lease<'owner>
        = boot_package_trust::PackageServiceGenerationLease<'owner>
    where
        Self: 'owner;

    fn capture_snapshot(
        &self,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
    ) -> Result<Self::Snapshot, PeerDenial> {
        self.capture(package_bytes, expected_manifest_id)
            .map_err(map_boot_package_owner_error)
    }

    fn lease_for_dispatch<'owner>(
        &'owner self,
        snapshot: &Self::Snapshot,
    ) -> Result<Self::Lease<'owner>, PeerDenial> {
        self.lease_for_dispatch(snapshot)
            .map_err(map_boot_package_owner_error)
    }

    fn snapshot_app_id(snapshot: Self::Snapshot) -> u64 {
        snapshot.app_id().0
    }

    fn snapshot_generation(snapshot: Self::Snapshot) -> u64 {
        snapshot.generation()
    }

    fn snapshot_manifest_id(snapshot: &Self::Snapshot) -> &[u8] {
        snapshot.manifest_id()
    }
}

struct Stage13InventoryConsumer<F, T> {
    app_id: u64,
    generation: u64,
    operation: Option<F>,
    result: Option<T>,
}

impl<F, T> Stage13BorrowedInventoryConsumer for Stage13InventoryConsumer<F, T>
where
    F: FnOnce() -> Result<T, PeerDenial>,
{
    type Error = PeerDenial;

    fn consume(&mut self, inventory: &PackageService) -> Result<(), Self::Error> {
        let installed = inventory
            .info(AppId(self.app_id))
            .map_err(|error| map_package_owner_error(PackageOwnerError::Package(error)))?;
        if installed.generation != self.generation {
            return Err(PeerDenial::StaleManifest);
        }
        self.result = Some(self.operation.take().ok_or(PeerDenial::Unavailable)?()?);
        Ok(())
    }
}

impl<'owner, 'run, Owner: SessionPackageServiceOwner>
    SerializedPackageServiceAuthority<'owner, 'run, Owner>
{
    /// Capture a generation through the same task-25 owner used later by the
    /// inventory lease and the effect commit transaction. Call before entering
    /// the PID1 gate because task-25 capture is setup work and may block.
    #[cfg(test)]
    pub(crate) fn capture(
        owner: &'owner Owner,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
        run_owner_lease: &'run super::Pid1RunOwnerLease,
    ) -> Result<Self, PeerDenial> {
        Self::capture_inner(
            owner,
            package_bytes,
            expected_manifest_id,
            run_owner_lease,
            None,
        )
    }

    /// Capture the package generation and register its receipt table against
    /// the exact live endpoint registration. Registration occurs under the
    /// same PID1 owner gate as Supervisor revoke and run termination.
    pub(crate) fn capture_for_registration<
        R: super::LaunchRecordView,
        Boundary: Pid1ReadinessAtomicBoundary,
    >(
        owner: &'owner Owner,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
        run_owner_lease: &'run Pid1RunOwnerLease,
        directory: &Pid1LivePeerDirectory<R>,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        readiness: &Boundary,
    ) -> Result<Self, PeerDenial> {
        let authority = Self::capture_inner(
            owner,
            package_bytes,
            expected_manifest_id,
            run_owner_lease,
            Some((registration, process)),
        )?;
        let revoker: Arc<dyn PackageRequestReceiptRevoker> = authority.request_state.clone();
        readiness.with_owner_gate(&authority.run, None, || {
            directory.register_package_request_receipt_revoker(
                &authority.run,
                registration,
                process,
                Owner::snapshot_app_id(authority.snapshot),
                Owner::snapshot_generation(authority.snapshot),
                &revoker,
            )
        })?;
        Ok(authority)
    }

    fn capture_inner(
        owner: &'owner Owner,
        package_bytes: &[u8],
        expected_manifest_id: &[u8],
        run_owner_lease: &'run Pid1RunOwnerLease,
        registration: Option<(RegistrationId, ProcessInstanceKey)>,
    ) -> Result<Self, PeerDenial> {
        if !run_owner_lease.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        let run = run_owner_lease.run().clone();
        let snapshot = owner.capture_snapshot(package_bytes, expected_manifest_id)?;
        let _lease = owner.lease_for_dispatch(&snapshot)?;
        let mut inventory_check = Stage13InventoryConsumer {
            app_id: Owner::snapshot_app_id(snapshot),
            generation: Owner::snapshot_generation(snapshot),
            operation: Some(|| Ok(())),
            result: None,
        };
        with_stage13_borrowed_inventory(&_lease, &mut inventory_check)?;
        let package = PackageView::parse(package_bytes)
            .map_err(|error| map_snapshot_error(SnapshotError::InvalidPackage(error)))?;
        let manifest = package.manifest();
        if manifest.app_id().0 != Owner::snapshot_app_id(snapshot)
            || manifest.id() != Owner::snapshot_manifest_id(&snapshot)
        {
            return Err(PeerDenial::StaleManifest);
        }
        let declared_capabilities = manifest.grants().map(<[u8]>::to_vec).collect::<Vec<_>>();
        let trusted_manifest = TrustedManifest::from_verified_installed_manifest(
            Owner::snapshot_app_id(snapshot),
            Owner::snapshot_generation(snapshot),
            declared_capabilities,
        )
        .ok_or(PeerDenial::StaleManifest)?;
        drop(_lease);
        let request_state = Arc::new(PackageServiceRequestRevocationState {
            run: run.clone(),
            registration: registration.map(|(registration, _)| registration),
            process: registration.map(|(_, process)| process),
            revoked: AtomicBool::new(false),
            requests: Mutex::new(PackageServiceRequestState {
                admissions: Vec::with_capacity(MAX_PID1_PACKAGE_REQUESTS),
                outcomes: Vec::with_capacity(MAX_PID1_PACKAGE_REQUESTS),
            }),
        });
        Ok(Self {
            owner,
            snapshot,
            trusted_manifest,
            run,
            run_owner_lease,
            request_state,
        })
    }

    fn ensure_run_current(&self) -> Result<(), PeerDenial> {
        if self.run_owner_lease.is_active()
            && self.run.is_same_run(self.run_owner_lease.run())
            && !self.request_state.revoked.load(Ordering::SeqCst)
        {
            Ok(())
        } else {
            if self.run_owner_lease.is_active() {
                Err(PeerDenial::StaleRegistration)
            } else {
                Err(PeerDenial::StaleRun)
            }
        }
    }

    fn with_current_inventory_lease<T>(
        &self,
        operation: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        self.ensure_run_current()?;
        let lease = self.owner.lease_for_dispatch(&self.snapshot)?;
        let mut consumer = Stage13InventoryConsumer {
            app_id: Owner::snapshot_app_id(self.snapshot),
            generation: Owner::snapshot_generation(self.snapshot),
            operation: Some(operation),
            result: None,
        };
        with_stage13_borrowed_inventory(&lease, &mut consumer)?;
        self.ensure_run_current()?;
        consumer.result.take().ok_or(PeerDenial::Unavailable)
    }

    /// Keep task25's exact borrow alive across the existing service-owner
    /// commit callback. Call this from inside `ServiceOwnerEffectTransaction`
    /// while the shared PID1 gate is held; it protects this PackageService
    /// generation through the callback but does not supply a service effect,
    /// consent, durable result, or cross-process lock.
    pub(crate) fn with_package_generation_lease<T>(
        &self,
        admission: &ServiceRequestAdmission,
        operation: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        self.validate_admission(admission)?;
        self.with_current_inventory_lease(operation)
    }

    /// Bind one opaque PID1 admission to this exact installed generation.
    /// Request state changes share the same PID1 gate as receipt commit and
    /// revocation. The PackageService owner remains a separate nested lock.
    pub(crate) fn bind_admission<B: Pid1ReadinessAtomicBoundary>(
        &self,
        admission: &ServiceRequestAdmission,
        readiness: &B,
    ) -> Result<(), PeerDenial> {
        self.validate_admission(admission)?;
        readiness.with_owner_gate(&self.run, None, || {
            self.ensure_run_current()?;
            self.with_current_inventory_lease(|| {
                let mut state = self
                    .request_state
                    .requests
                    .try_lock()
                    .map_err(|_| PeerDenial::Unavailable)?;
                if state.outcomes.iter().any(|outcome| {
                    outcome.request_id == admission.request_id()
                        && outcome.state != super::ServiceRequestOperationState::NotInvoked
                }) {
                    return Err(PeerDenial::DuplicateRequestCommit);
                }
                if let Some((_, existing)) = state
                    .admissions
                    .iter()
                    .find(|(request_id, _)| *request_id == admission.request_id())
                {
                    if existing != admission {
                        return Err(PeerDenial::StaleRegistration);
                    }
                } else {
                    if state.admissions.len() >= MAX_PID1_PACKAGE_REQUESTS {
                        return Err(PeerDenial::Capacity);
                    }
                    state
                        .admissions
                        .push((admission.request_id(), admission.clone()));
                }
                Ok(())
            })
        })
    }

    /// Revoke one previously bound request under the same PID1 gate. An
    /// owner-wide revoke can clear all admissions through this same lock order.
    pub(crate) fn revoke_admission<B: Pid1ReadinessAtomicBoundary>(
        &self,
        admission: &ServiceRequestAdmission,
        readiness: &B,
    ) -> Result<bool, PeerDenial> {
        self.ensure_run_current()?;
        readiness.with_owner_gate(&self.run, None, || {
            self.ensure_run_current()?;
            let mut state = self
                .request_state
                .requests
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            let Some(index) = state.admissions.iter().position(|(request_id, current)| {
                *request_id == admission.request_id() && current == admission
            }) else {
                return Ok(false);
            };
            state.admissions.remove(index);
            state.revoke_outcome(admission.request_id());
            Ok(true)
        })
    }

    pub(crate) fn revoke_all<B: Pid1ReadinessAtomicBoundary>(
        &self,
        readiness: &B,
    ) -> Result<(), PeerDenial> {
        self.ensure_run_current()?;
        readiness.with_owner_gate(&self.run, None, || {
            self.ensure_run_current()?;
            let mut state = self
                .request_state
                .requests
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            state.revoke_all();
            Ok(())
        })
    }

    fn validate_admission(&self, admission: &ServiceRequestAdmission) -> Result<(), PeerDenial> {
        self.ensure_run_current()?;
        if !admission.run().is_same_run(&self.run)
            || admission.app_id() != Owner::snapshot_app_id(self.snapshot)
            || admission.inventory_generation().get() != Owner::snapshot_generation(self.snapshot)
        {
            return Err(PeerDenial::StaleManifest);
        }
        if let Some((registration, process)) = self
            .request_state
            .registration
            .zip(self.request_state.process)
        {
            if admission.registration() != registration || admission.process() != process {
                return Err(PeerDenial::StaleRegistration);
            }
        }
        // This type records only the fixed task-25 package-generation receipt.
        // It does not authorize a service capability; the peer adapter checks
        // declared grants before it can issue a ServiceRequestAdmission.
        Ok(())
    }
}

impl<Owner: SessionPackageServiceOwner> TrustedInventory
    for SerializedPackageServiceAuthority<'_, '_, Owner>
{
    fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
        self.ensure_run_current()?;
        if app_id != Owner::snapshot_app_id(self.snapshot) {
            return Ok(None);
        }
        self.with_current_inventory_lease(|| Ok(()))?;
        Ok(Some(self.trusted_manifest.clone()))
    }

    fn generation_is_current(
        &self,
        app_id: u64,
        generation: NonZeroU64,
    ) -> Result<bool, PeerDenial> {
        self.ensure_run_current()?;
        if app_id != Owner::snapshot_app_id(self.snapshot)
            || generation.get() != Owner::snapshot_generation(self.snapshot)
        {
            return Ok(false);
        }
        self.with_current_inventory_lease(|| Ok(()))?;
        Ok(true)
    }

    fn with_current_manifests(
        &self,
        requested: &[(u64, NonZeroU64)],
        inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        self.ensure_run_current()?;
        if requested.is_empty() {
            return Err(PeerDenial::EmptyRequiredServiceSet);
        }
        for (app_id, generation) in requested {
            if *app_id != Owner::snapshot_app_id(self.snapshot) {
                return Err(PeerDenial::NotInstalled);
            }
            if generation.get() != Owner::snapshot_generation(self.snapshot) {
                return Err(PeerDenial::StaleManifest);
            }
        }
        let manifests = requested
            .iter()
            .map(|_| self.trusted_manifest.clone())
            .collect::<Vec<_>>();
        self.with_current_inventory_lease(|| inspect(&manifests))
    }
}

impl<Owner: SessionPackageServiceOwner> service_effect_commit_seal::Sealed
    for SerializedPackageServiceAuthority<'_, '_, Owner>
{
}

impl<Owner: SessionPackageServiceOwner> ServiceRequestReceiptTransaction
    for SerializedPackageServiceAuthority<'_, '_, Owner>
{
    fn resolve_receipt_state(
        &self,
        admission: &ServiceRequestAdmission,
    ) -> Result<super::ServiceRequestOperationState, PeerDenial> {
        self.validate_admission(admission)?;
        self.with_current_inventory_lease(|| {
            let state = self
                .request_state
                .requests
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if !state.admissions.iter().any(|(request_id, current)| {
                *request_id == admission.request_id() && current == admission
            }) {
                return Err(PeerDenial::StaleRegistration);
            }
            Ok(state
                .outcomes
                .iter()
                .find(|outcome| outcome.request_id == admission.request_id())
                .map(|outcome| match outcome.state {
                    super::ServiceRequestOperationState::Executing
                    | super::ServiceRequestOperationState::OutcomeUnknown => {
                        super::ServiceRequestOperationState::OutcomeUnknown
                    }
                    state => state,
                })
                .unwrap_or(super::ServiceRequestOperationState::NotInvoked))
        })
    }

    fn record_receipt_if_current(
        &self,
        admission: &ServiceRequestAdmission,
    ) -> Result<nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt, PeerDenial> {
        self.validate_admission(admission)?;
        // Check and borrow the current generation immediately before the fixed
        // receipt operation. Drop the read lease before the same owner writes;
        // its commit API revalidates the snapshot at its own atomic boundary.
        self.with_current_inventory_lease(|| Ok(()))?;
        let mut state = self
            .request_state
            .requests
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if !state.admissions.iter().any(|(request_id, current)| {
            *request_id == admission.request_id() && current == admission
        }) {
            return Err(PeerDenial::StaleRegistration);
        }
        if let Some(existing) = state
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.request_id == admission.request_id())
        {
            match existing.state {
                super::ServiceRequestOperationState::ReceiptRecorded => {
                    return Err(PeerDenial::DuplicateRequestCommit)
                }
                super::ServiceRequestOperationState::Executing
                | super::ServiceRequestOperationState::OutcomeUnknown => {
                    return Err(PeerDenial::RequestOutcomeUnknown)
                }
                super::ServiceRequestOperationState::NotInvoked => {
                    existing.state = super::ServiceRequestOperationState::Executing;
                    existing.receipt = None;
                }
                _ => return Err(PeerDenial::RequestIdentityConflict),
            }
        } else {
            if state.outcomes.len() >= MAX_PID1_PACKAGE_REQUESTS {
                return Err(PeerDenial::Capacity);
            }
            state.outcomes.push(PackageServiceReceiptOutcome {
                request_id: admission.request_id(),
                state: super::ServiceRequestOperationState::Executing,
                receipt: None,
            });
        }

        // The task25 fixed operation has a single receipt-table assignment and
        // no fallible work after publication. Pre-write errors are known
        // NotInvoked; duplicate IDs remain ambiguous because they do not bind
        // the existing receipt to this request's complete content.
        let result = self
            .owner
            .commit_local_receipt(self.snapshot, admission.request_id().get());
        let outcome = state
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.request_id == admission.request_id())
            .ok_or(PeerDenial::Unavailable)?;
        match result {
            Ok(receipt) => {
                outcome.state = super::ServiceRequestOperationState::ReceiptRecorded;
                outcome.receipt = Some(receipt);
                Ok(receipt)
            }
            Err(PeerDenial::RequestOutcomeUnknown | PeerDenial::DuplicateRequestCommit) => {
                outcome.state = super::ServiceRequestOperationState::OutcomeUnknown;
                Err(PeerDenial::RequestOutcomeUnknown)
            }
            Err(error) => {
                outcome.state = super::ServiceRequestOperationState::NotInvoked;
                Err(error)
            }
        }
    }

    fn resolve_receipt_record(
        &self,
        admission: &ServiceRequestAdmission,
    ) -> Result<Option<nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt>, PeerDenial>
    {
        self.validate_admission(admission)?;
        self.with_current_inventory_lease(|| {
            let state = self
                .request_state
                .requests
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if !state.admissions.iter().any(|(request_id, current)| {
                *request_id == admission.request_id() && current == admission
            }) {
                return Err(PeerDenial::StaleRegistration);
            }
            Ok(state
                .outcomes
                .iter()
                .find(|outcome| outcome.request_id == admission.request_id())
                .and_then(|outcome| outcome.receipt))
        })
    }
}

#[cfg(test)]
impl<Owner: SessionPackageServiceOwner> SerializedPackageServiceAuthority<'_, '_, Owner> {
    pub(super) fn test_set_outcome_state(
        &self,
        admission: &ServiceRequestAdmission,
        state_kind: super::ServiceRequestOperationState,
    ) -> Result<(), PeerDenial> {
        self.validate_admission(admission)?;
        let mut state = self
            .request_state
            .requests
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if !state
            .admissions
            .iter()
            .any(|(id, existing)| *id == admission.request_id() && existing == admission)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        if let Some(outcome) = state
            .outcomes
            .iter_mut()
            .find(|outcome| outcome.request_id == admission.request_id())
        {
            outcome.state = state_kind;
            if state_kind != super::ServiceRequestOperationState::ReceiptRecorded {
                outcome.receipt = None;
            }
        } else {
            if state.outcomes.len() >= MAX_PID1_PACKAGE_REQUESTS {
                return Err(PeerDenial::Capacity);
            }
            state.outcomes.push(PackageServiceReceiptOutcome {
                request_id: admission.request_id(),
                state: state_kind,
                receipt: None,
            });
        }
        Ok(())
    }

    pub(super) fn test_outcome_snapshot(
        &self,
    ) -> Result<Vec<(NonZeroU64, super::ServiceRequestOperationState, bool)>, PeerDenial> {
        let state = self
            .request_state
            .requests
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        Ok(state
            .outcomes
            .iter()
            .map(|outcome| (outcome.request_id, outcome.state, outcome.receipt.is_some()))
            .collect())
    }

    pub(super) fn test_admission_count(&self) -> Result<usize, PeerDenial> {
        self.request_state
            .requests
            .lock()
            .map(|state| state.admissions.len())
            .map_err(|_| PeerDenial::Unavailable)
    }
}

impl<R: super::LaunchRecordView> super::AuthorizedPeer<'_, R> {
    /// Route a service effect through task25's exact borrowed PackageService
    /// lease. The service owner transaction is entered first; this lease then
    /// remains live around its callback and the side effect. It is local to
    /// this owner process and does not make the task25 receipt a durable effect.
    pub(crate) fn commit_effect_with_package_generation_lease<
        T,
        EffectOwner: super::ServiceOwnerEffectTransaction,
        Boundary: Pid1ReadinessAtomicBoundary,
        Owner: SessionPackageServiceOwner,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness: &Boundary,
        effect_owner: &EffectOwner,
        package_owner: &SerializedPackageServiceAuthority<'_, '_, Owner>,
        effect: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        self.commit_effect(admission, readiness, effect_owner, || {
            package_owner.with_package_generation_lease(admission, effect)
        })
    }

    pub(crate) fn commit_consented_effect_with_package_generation_lease<
        T,
        Consent: super::SessionOwnedConsentAuthority,
        EffectOwner: super::ServiceOwnerEffectTransaction,
        Boundary: Pid1ReadinessAtomicBoundary,
        Owner: SessionPackageServiceOwner,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        consent: &Consent,
        readiness: &Boundary,
        effect_owner: &EffectOwner,
        package_owner: &SerializedPackageServiceAuthority<'_, '_, Owner>,
        effect: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        self.commit_consented_effect(admission, consent, readiness, effect_owner, || {
            package_owner.with_package_generation_lease(admission, effect)
        })
    }
}

fn map_generation_bound_error(error: GenerationBoundCommitError) -> PeerDenial {
    match error {
        GenerationBoundCommitError::Unavailable => PeerDenial::InventoryUnavailable,
        GenerationBoundCommitError::Busy => PeerDenial::InventoryUnavailable,
        GenerationBoundCommitError::OwnerMismatch => PeerDenial::StaleManifest,
        GenerationBoundCommitError::Snapshot(error) => map_snapshot_error(error),
        GenerationBoundCommitError::InvalidRecordId => PeerDenial::Unavailable,
        GenerationBoundCommitError::DuplicateRecordId(_) => PeerDenial::RequestOutcomeUnknown,
        GenerationBoundCommitError::RecordCapacity => PeerDenial::Capacity,
    }
}

fn map_package_owner_error(error: PackageOwnerError) -> PeerDenial {
    match error {
        PackageOwnerError::Unavailable | PackageOwnerError::Busy => {
            PeerDenial::InventoryUnavailable
        }
        PackageOwnerError::OwnerMismatch => PeerDenial::StaleManifest,
        PackageOwnerError::Package(error) => {
            map_snapshot_error(SnapshotError::InvalidPackage(error))
        }
        PackageOwnerError::Snapshot(error) => map_snapshot_error(error),
    }
}

fn map_boot_package_owner_error(
    error: boot_package_trust::PackageServiceSessionOwnerError,
) -> PeerDenial {
    use boot_package_trust::PackageServiceSessionOwnerError as Error;
    match error {
        Error::TrustUnavailable | Error::Unavailable => PeerDenial::InventoryUnavailable,
        Error::Package(error) => map_snapshot_error(SnapshotError::InvalidPackage(error)),
        Error::Provider(_) => PeerDenial::InventoryUnavailable,
        Error::IdentityMismatch { .. } => PeerDenial::ManifestMismatch,
        Error::NotInstalled
        | Error::ManifestChanged
        | Error::StaleGeneration { .. }
        | Error::GenerationSaturated
        | Error::OwnerMismatch => PeerDenial::StaleManifest,
    }
}

/// Test-only compatibility adapter for the earlier borrowed-Mutex candidate.
/// Production code must use `SerializedPackageServiceAuthority`, which borrows
/// the retained task-25 owner and its incarnation-bound lease.
#[cfg(test)]
pub(crate) struct PackageServiceTrustedInventory<'a> {
    service: &'a Mutex<PackageService>,
    trusted_key: Option<&'a [u8; 32]>,
    snapshot: PackageInventorySnapshot,
    package: PackageView<'a>,
    trusted_manifest: TrustedManifest,
}

#[cfg(test)]
impl<'a> PackageServiceTrustedInventory<'a> {
    /// Capture the signed package and require a fresh whole-inventory
    /// `validate_current()` before returning the authority adapter.
    pub(crate) fn capture(
        service: &'a Mutex<PackageService>,
        trusted_key: Option<&'a [u8; 32]>,
        package_bytes: &'a [u8],
        expected_manifest_id: &[u8],
    ) -> Result<Self, PeerDenial> {
        let package = PackageView::parse(package_bytes)
            .map_err(|error| map_snapshot_error(SnapshotError::InvalidPackage(error)))?;
        let service_guard = try_lock(service)?;
        let snapshot_supplier = PackageInventorySnapshotSupplier::new(trusted_key, &service_guard);
        let snapshot = snapshot_supplier
            .capture(package_bytes, expected_manifest_id)
            .map_err(map_snapshot_error)?;

        // Capture can authenticate the input and return its generation before
        // all installed manifest fields have been compared. Validate now,
        // immediately, before using the package's signed grants.
        snapshot_supplier
            .validate_current(&snapshot)
            .map_err(map_snapshot_error)?;
        let manifest = package.manifest();
        if manifest.app_id() != snapshot.app_id() || manifest.id() != snapshot.manifest_id() {
            return Err(PeerDenial::StaleManifest);
        }
        let declared_capabilities = manifest.grants().map(<[u8]>::to_vec).collect::<Vec<_>>();
        let trusted_manifest = TrustedManifest::from_verified_installed_manifest(
            snapshot.app_id().0,
            snapshot.generation(),
            declared_capabilities,
        )
        .ok_or(PeerDenial::StaleManifest)?;
        drop(snapshot_supplier);
        drop(service_guard);

        Ok(Self {
            service,
            trusted_key,
            snapshot,
            package,
            trusted_manifest,
        })
    }

    /// Revalidate the snapshot and hold the shared PackageService lock while
    /// the caller copies this exact signed package's executable bytes into the
    /// PID 1-owned executable descriptor. This limits generation changes in
    /// the spawn preparation window; it is not a proof of stored-byte identity.
    pub(crate) fn lock_current_for_spawn(
        &self,
    ) -> Result<PackageServiceLaunchLease<'_, 'a>, PeerDenial> {
        let guard = try_lock(self.service)?;
        self.validate_current_locked(&guard)?;
        Ok(PackageServiceLaunchLease {
            _service_guard: guard,
            package: &self.package,
            generation: self.snapshot.generation(),
        })
    }

    fn validate_current(&self) -> Result<(), PeerDenial> {
        let guard = try_lock(self.service)?;
        self.validate_current_locked(&guard)
    }

    fn validate_current_locked(&self, service: &PackageService) -> Result<(), PeerDenial> {
        PackageInventorySnapshotSupplier::new(self.trusted_key, service)
            .validate_current(&self.snapshot)
            .map_err(map_snapshot_error)
    }
}

#[cfg(test)]
impl TrustedInventory for PackageServiceTrustedInventory<'_> {
    fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
        if app_id != self.snapshot.app_id().0 {
            return Ok(None);
        }
        self.validate_current()?;
        Ok(Some(self.trusted_manifest.clone()))
    }

    fn generation_is_current(
        &self,
        app_id: u64,
        generation: std::num::NonZeroU64,
    ) -> Result<bool, PeerDenial> {
        if app_id != self.snapshot.app_id().0 || generation.get() != self.snapshot.generation() {
            return Ok(false);
        }
        self.validate_current()?;
        Ok(true)
    }

    fn with_current_manifests(
        &self,
        requested: &[(u64, std::num::NonZeroU64)],
        inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        // Keep this exact PackageService instance immutably borrowed from
        // generation validation through all readiness checks. Safe local
        // install/remove calls require mutable access and cannot proceed until
        // the callback returns. This mutex is process-local; it is not a
        // cross-process PackageService transaction.
        let guard = try_lock(self.service)?;
        self.validate_current_locked(&guard)?;
        if requested.is_empty() {
            return Err(PeerDenial::EmptyRequiredServiceSet);
        }
        let mut manifests = Vec::with_capacity(requested.len());
        for (app_id, generation) in requested {
            if *app_id != self.snapshot.app_id().0 {
                return Err(PeerDenial::NotInstalled);
            }
            if generation.get() != self.snapshot.generation() {
                return Err(PeerDenial::StaleManifest);
            }
            manifests.push(self.trusted_manifest.clone());
        }
        inspect(&manifests)
    }
}

/// Holds the same inventory lock across the immediate revalidation and the
/// caller's executable copy step. The caller must drop this before Supervisor
/// registry work to avoid lock inversion.
#[cfg(test)]
pub(crate) struct PackageServiceLaunchLease<'guard, 'package> {
    _service_guard: MutexGuard<'guard, PackageService>,
    package: &'guard PackageView<'package>,
    generation: u64,
}

#[cfg(test)]
impl PackageServiceLaunchLease<'_, '_> {
    pub(crate) fn executable_bytes(&self) -> &[u8] {
        self.package.executable()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
fn try_lock(service: &Mutex<PackageService>) -> Result<MutexGuard<'_, PackageService>, PeerDenial> {
    service
        .try_lock()
        .map_err(|_| PeerDenial::InventoryUnavailable)
}

fn map_snapshot_error(error: SnapshotError) -> PeerDenial {
    match error {
        SnapshotError::NotInstalled
        | SnapshotError::ManifestChanged
        | SnapshotError::StaleGeneration { .. }
        | SnapshotError::GenerationSaturated => PeerDenial::StaleManifest,
        other => PeerDenial::PackageSnapshot(std::sync::Arc::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::super as supplier_root;
    use super::*;
    use nagi_package::{InstallPolicy, PackageView, TRUSTED_SIGNING_PUBLIC_KEY};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Mutex;

    const EXPECTED_ID: &[u8] = b"org.nagi.acceptance.isolated-app";

    fn signed_package() -> Vec<u8> {
        let path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the task-25 signed XAPP");
        std::fs::read(path).expect("read task-25 signed XAPP")
    }

    fn seeded_service(bytes: &[u8]) -> Mutex<PackageService> {
        let package = PackageView::parse(bytes).expect("parse signed package");
        let mut service = PackageService::new();
        service
            .install_with_trusted_key(
                &package,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                &TRUSTED_SIGNING_PUBLIC_KEY,
            )
            .expect("seed PackageService with signed package");
        Mutex::new(service)
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct OwnerRecord;

    impl supplier_root::LaunchRecordView for OwnerRecord {
        fn process_id(self) -> u32 {
            42
        }

        fn app_id(self) -> u64 {
            7
        }

        fn app_session_id(self) -> u64 {
            9
        }
    }

    struct OwnerTestBoundary {
        run: supplier_root::Pid1RunInstance,
        domain: supplier_root::Pid1AuthorityDomain,
        gate: Mutex<()>,
        active: AtomicBool,
    }

    impl supplier_root::readiness_boundary_seal::Sealed for OwnerTestBoundary {}

    impl supplier_root::Pid1ReadinessAtomicBoundary for OwnerTestBoundary {
        fn authority_domain(&self) -> &supplier_root::Pid1AuthorityDomain {
            &self.domain
        }

        fn with_owner_gate<T>(
            &self,
            run: &supplier_root::Pid1RunInstance,
            _required: Option<&supplier_root::RequiredServiceSet>,
            operation: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !self.run.is_same_run(run) {
                return Err(PeerDenial::StaleRun);
            }
            let _guard = self.gate.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            self.active.store(true, Ordering::SeqCst);
            struct Reset<'a>(&'a AtomicBool);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = Reset(&self.active);
            operation()
        }
    }

    fn owner_test_admission(
        run: &supplier_root::Pid1RunInstance,
        app_id: u64,
        generation: NonZeroU64,
        capability: Vec<u8>,
    ) -> ServiceRequestAdmission {
        let route = supplier_root::TrustedRouteKey::from_canonical_route(91, 1, 1, 0, 1)
            .expect("test route");
        let endpoint = supplier_root::TrustedServiceEndpoint::from_service_registry(route, 1, 1)
            .expect("test endpoint");
        let capability_name = if capability.is_empty() {
            "package.receipt".to_owned()
        } else {
            String::from_utf8(capability).unwrap()
        };
        static TEST_ENDPOINT_RECIPIENT: std::sync::OnceLock<
            supplier_root::EndpointCheckoutRecipient,
        > = std::sync::OnceLock::new();
        ServiceRequestAdmission {
            run: run.clone(),
            request_id: supplier_root::next_service_request_id().expect("test request id"),
            wire_request_id: supplier_root::StableServiceRequestId::from_protocol_decoder([1; 16]),
            fingerprint: supplier_root::ServiceRequestFingerprint::for_test(b"task25 receipt"),
            operation_state: Arc::new(AtomicU8::new(
                supplier_root::ServiceRequestOperationState::NotInvoked as u8,
            )),
            owner_identity: supplier_root::ServiceEffectOwnerId::new([0x41; 16]),
            canonical_request_bytes: None,
            consent_event: None,
            consent_requirement: supplier_root::RouteConsentRequirement::NotRequired,
            registration: supplier_root::RegistrationId(NonZeroU64::new(1).unwrap()),
            process: supplier_root::ProcessInstanceKey::from_pid1_spawn(42, 1).unwrap(),
            task24_identity: nagi_init_spawn_contract::test_identity(42),
            app_id,
            app_session_id: 9,
            inventory_generation: generation,
            session_generation: NonZeroU64::new(1).unwrap(),
            route,
            endpoint,
            endpoint_epoch: endpoint.endpoint_generation,
            endpoint_recipient: TEST_ENDPOINT_RECIPIENT
                .get_or_init(|| {
                    supplier_root::EndpointCheckoutRecipient::from_registered_owner(endpoint)
                })
                .clone(),
            service_id: nagi_service_contract::ServiceId::new("package.receipt").unwrap(),
            contract_version: nagi_service_contract::ContractVersion::new(1, 0).unwrap(),
            operation_id: nagi_service_contract::OperationId::new("record_receipt").unwrap(),
            capability_id: nagi_service_contract::CapabilityId::new(capability_name).unwrap(),
        }
    }

    fn install_signed(owner: &PackageServiceSessionOwner<'_>, bytes: &[u8]) {
        owner
            .install(
                bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install signed package through task-25 owner");
    }

    #[test]
    fn package_service_adapter_revalidates_full_manifest_and_signed_grants() {
        let bytes = signed_package();
        let service = seeded_service(&bytes);
        let adapter = PackageServiceTrustedInventory::capture(
            &service,
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            &bytes,
            EXPECTED_ID,
        )
        .unwrap();
        let app_id = PackageView::parse(&bytes).unwrap().manifest().app_id();
        let manifest = adapter.installed_manifest(app_id.0).unwrap().unwrap();
        assert_eq!(manifest.app_id, app_id.0);
        assert_eq!(manifest.inventory_generation.get(), 1);
        assert!(adapter
            .generation_is_current(app_id.0, manifest.inventory_generation)
            .unwrap());
        let lease = adapter.lock_current_for_spawn().unwrap();
        assert_eq!(lease.generation(), 1);
        assert!(!lease.executable_bytes().is_empty());
    }

    #[test]
    fn package_service_adapter_rejects_generation_change_after_capture() {
        let bytes = signed_package();
        let service = seeded_service(&bytes);
        let adapter = PackageServiceTrustedInventory::capture(
            &service,
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            &bytes,
            EXPECTED_ID,
        )
        .unwrap();
        let app_id = PackageView::parse(&bytes).unwrap().manifest().app_id();
        {
            let mut service = service.lock().unwrap();
            service.remove(app_id).unwrap();
            let package = PackageView::parse(&bytes).unwrap();
            service
                .install_with_trusted_key(
                    &package,
                    InstallPolicy {
                        developer_mode: false,
                        require_signature: true,
                    },
                    &TRUSTED_SIGNING_PUBLIC_KEY,
                )
                .unwrap();
        }
        assert!(matches!(
            adapter.installed_manifest(app_id.0),
            Err(PeerDenial::StaleManifest)
        ));
        assert!(matches!(
            adapter.lock_current_for_spawn(),
            Err(PeerDenial::StaleManifest)
        ));
    }

    #[test]
    fn package_service_adapter_uses_nonblocking_catalog_reads() {
        let bytes = signed_package();
        let service = seeded_service(&bytes);
        let adapter = PackageServiceTrustedInventory::capture(
            &service,
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            &bytes,
            EXPECTED_ID,
        )
        .unwrap();
        let app_id = PackageView::parse(&bytes).unwrap().manifest().app_id().0;
        let _held = service.lock().unwrap();
        assert!(matches!(
            adapter.installed_manifest(app_id),
            Err(PeerDenial::InventoryUnavailable)
        ));
    }

    #[test]
    fn package_service_read_lease_blocks_same_instance_mutation_through_callback() {
        let bytes = signed_package();
        let service = seeded_service(&bytes);
        let adapter = PackageServiceTrustedInventory::capture(
            &service,
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            &bytes,
            EXPECTED_ID,
        )
        .unwrap();
        let app_id = PackageView::parse(&bytes).unwrap().manifest().app_id().0;
        let generation = std::num::NonZeroU64::new(1).unwrap();
        adapter
            .with_current_manifests(&[(app_id, generation)], &mut |manifests| {
                assert_eq!(manifests.len(), 1);
                assert_eq!(manifests[0].inventory_generation, generation);
                assert!(matches!(
                    service.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                Ok(())
            })
            .unwrap();
        assert!(service.try_lock().is_ok());
    }

    #[test]
    fn package_service_owner_poison_fails_closed() {
        let bytes = signed_package();
        let service = seeded_service(&bytes);
        let adapter = PackageServiceTrustedInventory::capture(
            &service,
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            &bytes,
            EXPECTED_ID,
        )
        .unwrap();
        let app_id = PackageView::parse(&bytes).unwrap().manifest().app_id().0;
        let generation = std::num::NonZeroU64::new(1).unwrap();
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = service.lock().unwrap();
            panic!("simulate unavailable local PackageService owner");
        }));
        assert!(poisoned.is_err());
        assert!(matches!(
            adapter.with_current_manifests(&[(app_id, generation)], &mut |_| Ok(())),
            Err(PeerDenial::InventoryUnavailable)
        ));
    }

    #[test]
    fn serialized_package_owner_records_one_shot_generation_receipt_only() {
        let bytes = signed_package();
        let owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        install_signed(&owner, &bytes);
        let directory = supplier_root::Pid1LivePeerDirectory::<OwnerRecord>::new();
        let run_owner_lease = directory
            .begin_run_owner_lease()
            .expect("test PID1 owner lease");
        let run = run_owner_lease.run().clone();
        let adapter = SerializedPackageServiceAuthority::capture(
            &owner,
            &bytes,
            EXPECTED_ID,
            &run_owner_lease,
        )
        .unwrap();
        let package = PackageView::parse(&bytes).unwrap();
        let manifest = package.manifest();
        let app_id = adapter.snapshot.app_id().0;
        let generation = NonZeroU64::new(adapter.snapshot.generation()).unwrap();
        let capability = manifest
            .grants()
            .next()
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let boundary = OwnerTestBoundary {
            run: run.clone(),
            domain: supplier_root::Pid1AuthorityDomain::new(),
            gate: Mutex::new(()),
            active: AtomicBool::new(false),
        };
        let first = owner_test_admission(&run, app_id, generation, capability.clone());
        adapter.bind_admission(&first, &boundary).unwrap();
        let receipt = boundary
            .with_owner_gate(&run, None, || adapter.record_receipt_if_current(&first))
            .unwrap();
        assert_eq!(receipt.record_id(), first.request_id().get());
        assert_eq!(receipt.app_id().0, app_id);
        assert_eq!(receipt.generation(), generation.get());
        assert_eq!(
            adapter.resolve_receipt_state(&first).unwrap(),
            supplier_root::ServiceRequestOperationState::ReceiptRecorded
        );
        assert_eq!(
            adapter.resolve_receipt_record(&first).unwrap(),
            Some(receipt)
        );
        assert!(matches!(
            boundary.with_owner_gate(&run, None, || {
                adapter.record_receipt_if_current(&first.clone())
            }),
            Err(PeerDenial::DuplicateRequestCommit)
        ));
        adapter.revoke_all(&boundary).unwrap();
        assert!(matches!(
            adapter.resolve_receipt_record(&first),
            Err(PeerDenial::StaleRegistration)
        ));
        let current = adapter.installed_manifest(app_id).unwrap().unwrap();
        assert_eq!(current.app_id, app_id);
        assert_eq!(current.inventory_generation, generation);
        assert!(adapter.generation_is_current(app_id, generation).unwrap());
        adapter
            .with_current_manifests(&[(app_id, generation)], &mut |manifests| {
                assert_eq!(manifests, &[current.clone()]);
                assert!(matches!(
                    owner.remove(adapter.snapshot.app_id()),
                    Err(PackageOwnerError::Busy)
                ));
                Ok(())
            })
            .unwrap();

        let revoked = owner_test_admission(&run, app_id, generation, capability.clone());
        adapter.bind_admission(&revoked, &boundary).unwrap();
        assert!(adapter.revoke_admission(&revoked, &boundary).unwrap());
        assert!(matches!(
            boundary.with_owner_gate(&run, None, || {
                adapter.record_receipt_if_current(&revoked)
            }),
            Err(PeerDenial::StaleRegistration)
        ));

        let stale = owner_test_admission(&run, app_id, generation, capability);
        adapter.bind_admission(&stale, &boundary).unwrap();
        owner
            .remove(adapter.snapshot.app_id())
            .expect("remove signed package through task-25 owner");
        install_signed(&owner, &bytes);
        match boundary.with_owner_gate(&run, None, || adapter.record_receipt_if_current(&stale)) {
            Err(PeerDenial::StaleManifest) => {}
            other => panic!("expected stale PackageService generation, got {other:?}"),
        }

        directory
            .end_run_owner_lease(&run_owner_lease, &boundary)
            .unwrap();
        assert!(matches!(
            adapter.installed_manifest(app_id),
            Err(PeerDenial::StaleRun)
        ));
        assert!(matches!(
            adapter.resolve_receipt_record(&first),
            Err(PeerDenial::StaleRun)
        ));
    }

    #[test]
    fn boot_devpreview_owner_is_borrowed_and_generation_lease_covers_commit_callback() {
        let bytes = signed_package();
        let mut owner = boot_package_trust::PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install through the existing boot owner instance");
        let directory = supplier_root::Pid1LivePeerDirectory::<OwnerRecord>::new();
        let run_owner_lease = directory
            .begin_run_owner_lease()
            .expect("test PID1 owner lease");
        let run = run_owner_lease.run().clone();
        let adapter = SerializedPackageServiceAuthority::capture(
            &owner,
            &bytes,
            EXPECTED_ID,
            &run_owner_lease,
        )
        .expect("borrow the boot provider's exact session owner type");
        let app_id = adapter.snapshot.app_id().0;
        let generation = NonZeroU64::new(adapter.snapshot.generation()).unwrap();
        let admission = owner_test_admission(&run, app_id, generation, b"files.read".to_vec());
        let boundary = OwnerTestBoundary {
            run: run.clone(),
            domain: supplier_root::Pid1AuthorityDomain::new(),
            gate: Mutex::new(()),
            active: AtomicBool::new(false),
        };
        adapter.bind_admission(&admission, &boundary).unwrap();

        let calls = AtomicU8::new(0);
        boundary
            .with_owner_gate(&run, None, || {
                adapter.with_package_generation_lease(&admission, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            adapter.generation_is_current(app_id, generation).unwrap(),
            true
        );
    }

    #[test]
    fn revoking_32_resolved_receipts_releases_capacity_for_a_new_request() {
        let mut state = PackageServiceRequestState {
            admissions: Vec::new(),
            outcomes: Vec::with_capacity(MAX_PID1_PACKAGE_REQUESTS),
        };

        for raw_id in 1..=MAX_PID1_PACKAGE_REQUESTS as u64 {
            let request_id = NonZeroU64::new(raw_id).unwrap();
            state.outcomes.push(PackageServiceReceiptOutcome {
                request_id,
                state: supplier_root::ServiceRequestOperationState::ReceiptRecorded,
                receipt: None,
            });
            assert_eq!(state.outcomes.len(), 1);
            state.revoke_outcome(request_id);
            assert!(state.outcomes.is_empty());
        }

        assert!(state.outcomes.len() < MAX_PID1_PACKAGE_REQUESTS);
        state.outcomes.push(PackageServiceReceiptOutcome {
            request_id: NonZeroU64::new(1000).unwrap(),
            state: supplier_root::ServiceRequestOperationState::Executing,
            receipt: None,
        });
        assert_eq!(state.outcomes.len(), 1);
    }

    #[test]
    fn revoke_all_reclaims_known_outcomes_but_keeps_unknown_fail_closed() {
        let mut state = PackageServiceRequestState {
            admissions: Vec::new(),
            outcomes: (1..MAX_PID1_PACKAGE_REQUESTS as u64)
                .map(|raw_id| PackageServiceReceiptOutcome {
                    request_id: NonZeroU64::new(raw_id).unwrap(),
                    state: supplier_root::ServiceRequestOperationState::ReceiptRecorded,
                    receipt: None,
                })
                .collect(),
        };
        state.outcomes.push(PackageServiceReceiptOutcome {
            request_id: NonZeroU64::new(MAX_PID1_PACKAGE_REQUESTS as u64).unwrap(),
            state: supplier_root::ServiceRequestOperationState::OutcomeUnknown,
            receipt: None,
        });

        state.revoke_all();

        assert_eq!(state.outcomes.len(), 1);
        assert_eq!(
            state.outcomes[0].state,
            supplier_root::ServiceRequestOperationState::OutcomeUnknown
        );
        assert!(state.outcomes[0].receipt.is_none());
    }
}
