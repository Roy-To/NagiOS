use crate::{
    validate_artifact_preflight_report, validate_descriptor_json, ArtifactPreflightRequest,
    DescriptorError, PackageBindingError, PreflightReportError, SystemServiceLaunchMetadata,
    ValidatedSystemServiceDescriptor, VerifiedServicePackageBinding,
};
use nagi_capability::PrincipalKind;
use nagi_identity_session::{CallerContextId, PrincipalId};
use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_model::AppId;
use nagi_service_contract::ServiceId;
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::HashSet, mem, num::NonZeroU64};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchPortError {
    Unavailable,
    Rejected,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofValidationError {
    WrongVersion,
    WrongCatalogGeneration,
    WrongC3LaunchGeneration,
    WrongServiceBinding,
    WrongPackageBinding,
    MissingProcessIdentity,
    WrongProcessIdentity,
    WrongPeerCredentials,
    MissingEndpoint,
    NotLive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchError {
    Descriptor(DescriptorError),
    PackageBinding(PackageBindingError),
    Catalog(LaunchPortError),
    ArtifactPreflight(LaunchPortError),
    ArtifactReport(PreflightReportError),
    ProcessOwner(LaunchPortError),
    InvalidOwnerProof(ProofValidationError),
    OwnerGateUnavailable,
    Identity(LaunchPortError),
    IdentityOutcomeUnknown(SystemServiceRecoveryAdmission),
    IdentityRecoveryBlocked(LaunchPortError),
    IdentityQuarantineFailed(LaunchPortError, SystemServiceRecoveryAdmission),
    RecoveryReconciliationRequired(SystemServiceRecoveryAdmission),
    RecoveryQuarantineFailed {
        c1: Option<LaunchPortError>,
        c3: Option<LaunchPortError>,
        admission: SystemServiceRecoveryAdmission,
    },
    RecoveryReconciliation(LaunchPortError),
    RecoveryReconciliationPanicked,
    PackageQuarantineReleaseUnavailable(SystemServiceRecoveryAdmission),
    OwnerRevoke(LaunchPortError),
    OwnerCleanupPanicked,
    ProcessOwnerPanicked,
    RecoveryGateNotUsed,
    CatalogBindingMismatch,
}

/// Catalog-authenticated C1 identity mapping bound to one exact validated
/// service descriptor. Only a trusted catalog adapter should construct this
/// value; A rechecks the descriptor binding before creating a recovery token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSystemServiceCatalogBinding {
    descriptor: ValidatedSystemServiceDescriptor,
    principal_id: PrincipalId,
    caller_context_id: CallerContextId,
    mapping_generation: NonZeroU64,
}

impl AuthenticatedSystemServiceCatalogBinding {
    #[cfg(test)]
    pub(crate) fn from_trusted_catalog(
        descriptor: &ValidatedSystemServiceDescriptor,
        principal_id: PrincipalId,
        caller_context_id: CallerContextId,
        mapping_generation: NonZeroU64,
    ) -> Self {
        Self {
            descriptor: descriptor.clone(),
            principal_id,
            caller_context_id,
            mapping_generation,
        }
    }

    fn matches_descriptor(&self, descriptor: &ValidatedSystemServiceDescriptor) -> bool {
        self.descriptor == *descriptor
    }
}

/// Opaque A-issued C1 admission input. It joins catalog-selected canonical
/// C1 IDs, descriptor role/service generation, the installed package
/// generation, and the exact C3 launch generation. The one-shot C3 gate binds
/// the live proof to this token while each effect runs.
/// C1 must persist this scope in its recovery journal and must not derive IDs
/// from descriptor text or kernel credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemServiceRecoveryAdmission {
    catalog: AuthenticatedSystemServiceCatalogBinding,
    package_id: String,
    package_app_id: AppId,
    canonical_entry: String,
    executable_sha256: String,
    signed_package_sha256: [u8; 32],
    package_generation: u64,
    c3_launch_generation: NonZeroU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemServiceAdmissionError {
    PackageBindingMismatch,
    MappingGenerationMismatch,
    PackageGenerationMismatch,
    SignedPackageDigestMismatch,
}

impl SystemServiceRecoveryAdmission {
    fn new(
        catalog: AuthenticatedSystemServiceCatalogBinding,
        package: &VerifiedServicePackageBinding<'_>,
        lifecycle: &impl SystemServiceLaunchLifecycle,
    ) -> Self {
        Self {
            catalog,
            package_id: package.package_id().to_owned(),
            package_app_id: package.app_id(),
            canonical_entry: package.canonical_entry().to_owned(),
            executable_sha256: package.executable_sha256().to_owned(),
            signed_package_sha256: *package.signed_package_sha256(),
            package_generation: package.package_generation(),
            c3_launch_generation: lifecycle.c3_launch_generation(),
        }
    }

    /// Rebuild the typed input for restart reconciliation from the currently
    /// authenticated catalog mapping and installed package generation plus
    /// the C3 launch generation recorded by C1's durable intent.
    pub fn for_reconciliation(
        catalog: AuthenticatedSystemServiceCatalogBinding,
        package: &VerifiedServicePackageBinding<'_>,
        c3_launch_generation: NonZeroU64,
        expected_mapping_generation: NonZeroU64,
        expected_package_generation: u64,
        expected_signed_package_sha256: [u8; 32],
    ) -> Result<Self, SystemServiceAdmissionError> {
        if catalog.mapping_generation != expected_mapping_generation {
            return Err(SystemServiceAdmissionError::MappingGenerationMismatch);
        }
        if package.package_generation() != expected_package_generation {
            return Err(SystemServiceAdmissionError::PackageGenerationMismatch);
        }
        if *package.signed_package_sha256() != expected_signed_package_sha256 {
            return Err(SystemServiceAdmissionError::SignedPackageDigestMismatch);
        }
        if catalog.descriptor.package_id() != package.package_id()
            || catalog.descriptor.package_app_id() != package.app_id()
            || catalog.descriptor.canonical_entry() != package.canonical_entry()
            || catalog.descriptor.expected_sha256() != package.executable_sha256()
        {
            return Err(SystemServiceAdmissionError::PackageBindingMismatch);
        }
        Ok(Self {
            package_id: package.package_id().to_owned(),
            package_app_id: package.app_id(),
            canonical_entry: package.canonical_entry().to_owned(),
            executable_sha256: package.executable_sha256().to_owned(),
            signed_package_sha256: *package.signed_package_sha256(),
            package_generation: package.package_generation(),
            c3_launch_generation,
            catalog,
        })
    }

    pub fn descriptor(&self) -> &ValidatedSystemServiceDescriptor {
        &self.catalog.descriptor
    }

    pub fn service_id(&self) -> &ServiceId {
        self.catalog.descriptor.service_id()
    }

    pub fn role(&self) -> PrincipalKind {
        self.catalog.descriptor.role()
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.catalog.principal_id
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.catalog.caller_context_id
    }

    pub fn mapping_generation(&self) -> NonZeroU64 {
        self.catalog.mapping_generation
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_app_id(&self) -> AppId {
        self.package_app_id
    }

    pub fn canonical_entry(&self) -> &str {
        &self.canonical_entry
    }

    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }

    pub fn signed_package_sha256(&self) -> &[u8; 32] {
        &self.signed_package_sha256
    }

    pub fn package_generation(&self) -> u64 {
        self.package_generation
    }

    pub fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog.descriptor.generation()
    }

    pub fn c3_launch_generation(&self) -> NonZeroU64 {
        self.c3_launch_generation
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RecoveryKey {
    service_id: String,
    generation: u64,
}

fn recovery_key(descriptor: &ValidatedSystemServiceDescriptor) -> RecoveryKey {
    RecoveryKey {
        service_id: descriptor.service_id().as_str().to_owned(),
        generation: descriptor.generation().get(),
    }
}

fn blocked_recoveries() -> &'static Mutex<HashSet<RecoveryKey>> {
    static BLOCKED: OnceLock<Mutex<HashSet<RecoveryKey>>> = OnceLock::new();
    BLOCKED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_recovery_blocked(descriptor: &ValidatedSystemServiceDescriptor) -> bool {
    blocked_recoveries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&recovery_key(descriptor))
}

fn block_recovery(descriptor: &ValidatedSystemServiceDescriptor) {
    blocked_recoveries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(recovery_key(descriptor));
}

fn clear_recovery_block(descriptor: &ValidatedSystemServiceDescriptor) {
    blocked_recoveries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&recovery_key(descriptor));
}

/// A-owned authenticated catalog source for every field that affects role or
/// launch. The binding has no public constructor; an implementation must come
/// from the trusted catalog adapter inside this crate and compare the full
/// descriptor against its canonical record. Caller JSON, a package signature,
/// or an artifact digest alone cannot mint this binding. A currently has no
/// production catalog implementation, so no production launch can pass this
/// boundary yet.
pub trait TrustedSystemServiceCatalog {
    fn authenticate(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
    ) -> Result<AuthenticatedSystemServiceCatalogBinding, LaunchPortError>;
}

/// Artifact adapter must inspect the requested catalog-selected file and
/// return its bounded report. C3 does not launch from the inspected file: A
/// binds the report digest to the immutable executable snapshot copied from
/// the installed signed package, then passes that snapshot through the package
/// binding. This avoids relying on a mutable inode remaining unchanged after
/// preflight.
pub trait ArtifactPreflight {
    fn inspect(&mut self, request: &ArtifactPreflightRequest) -> Result<String, LaunchPortError>;
}

/// C3 implementation must load only `package.executable_bytes()`, binding
/// those immutable bytes to the exact reserved slot, Supervisor generation,
/// and connected endpoint. It must not reopen the descriptor's artifact
/// pathname or launch from a mutable FD. The returned proof must come from
/// the real pidfd-backed C3 process owner and kernel peer-credential check;
/// this API does not permit constructing one from descriptor data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationError {
    Unavailable,
    Rejected,
    Failed,
}

/// Exact C3 slot binding supplied by the maintained process owner. The C3
/// launch generation is independent of the catalog lifecycle generation.
pub trait SystemServiceLaunchLifecycle {
    fn service_id(&self) -> &ServiceId;
    fn role(&self) -> PrincipalKind;
    fn catalog_generation(&self) -> NonZeroU64;
    fn package_id(&self) -> &str;
    fn package_app_id(&self) -> AppId;
    fn canonical_entry(&self) -> &str;
    fn executable_sha256(&self) -> &str;
    fn signed_package_sha256(&self) -> &[u8; 32];
    fn package_generation(&self) -> u64;
    fn credentials(&self) -> (u32, u32);
    fn c3_launch_generation(&self) -> NonZeroU64;
    fn process_identity(&self) -> Option<SpawnedProcessIdentity>;
}

pub trait SystemServiceLaunchOwner {
    /// Exact reserved C3 slot identity. `reserve_launch` must not create a
    /// child; if it errors or unwinds, no process or Supervisor record may
    /// remain. The token is created before any operation that can create a
    /// child, so a later error or panic always has an exact cleanup key. The
    /// owner must enforce `metadata.transition()`: `Initial` requires an empty
    /// slot, while `Replace` requires the exact matching prior catalog
    /// generation under C3's serialized slot gate. A stale or mismatched
    /// replacement must fail before child creation.
    type Lifecycle: SystemServiceLaunchLifecycle;

    fn reserve_launch(
        &mut self,
        metadata: &SystemServiceLaunchMetadata,
        package: &VerifiedServicePackageBinding<'_>,
    ) -> Result<Self::Lifecycle, ReservationError>;

    fn launch_and_authenticate(
        &mut self,
        metadata: &SystemServiceLaunchMetadata,
        package: &VerifiedServicePackageBinding<'_>,
        lifecycle: &mut Self::Lifecycle,
    ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError>;

    /// Consume the exact reservation. `StoppedAndRevoked` means C3 has reaped
    /// the child and removed the matching slot. `Quarantined` means C3 has
    /// retained it in its own durable quarantine. `Unresolved` returns the
    /// token to A, which will keep the package generation fenced and leak the
    /// token rather than permit a retry. C3 owns all Supervisor mutation.
    fn finish_exact(
        &mut self,
        metadata: &SystemServiceLaunchMetadata,
        lifecycle: Self::Lifecycle,
        proof: Option<&SystemServiceEndpointOwnerProof>,
    ) -> LifecycleFinish<Self::Lifecycle>;

    /// Persist an exact C3 slot quarantine when `finish_exact` returns
    /// `Unresolved` or unwinds. The record must bind the service, catalog
    /// generation, and C3 launch generation in `admission`, and must keep that
    /// slot unavailable across restart. Repeating this operation for the same
    /// admission must be safe.
    fn persist_recovery_quarantine(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
    ) -> Result<(), LaunchPortError>;

    /// C1 calls this after acquiring its own durable recovery gate. C3 must
    /// hold the exact service/launch slot gate while `reconcile` atomically
    /// resolves the matching quarantine. The same API also serializes with any
    /// replacement or revoke of that slot.
    fn with_reconciliation_gate<R>(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, LaunchPortError>;
}

pub enum LifecycleFinish<L> {
    StoppedAndRevoked,
    Quarantined,
    Unresolved(L),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryGateError {
    AlreadyUsed,
    OwnerGateUnavailable,
    EffectPanicked,
}

/// One-shot bridge C1 uses to run its durable recovery effect while C3 holds
/// the exact proof generation gate. It exposes no proof handle, so C1 cannot
/// re-enter `is_live`, another authority callback, or revoke through the same
/// non-reentrant C3 mutex.
pub struct SystemServiceRecoveryGate<'proof> {
    proof: &'proof SystemServiceEndpointOwnerProof,
    descriptor: ValidatedSystemServiceDescriptor,
    used: bool,
    applied: bool,
    panicked: bool,
    protocol_violation: bool,
}

impl<'proof> SystemServiceRecoveryGate<'proof> {
    fn new(
        proof: &'proof SystemServiceEndpointOwnerProof,
        descriptor: &ValidatedSystemServiceDescriptor,
    ) -> Self {
        Self {
            proof,
            descriptor: descriptor.clone(),
            used: false,
            applied: false,
            panicked: false,
            protocol_violation: false,
        }
    }

    pub fn apply<R>(&mut self, effect: impl FnOnce() -> R) -> Result<R, RecoveryGateError> {
        if self.used {
            self.protocol_violation = true;
            return Err(RecoveryGateError::AlreadyUsed);
        }
        self.used = true;
        match run_recovery_effect(&self.descriptor, || {
            self.proof.with_live_authority_effect(effect)
        }) {
            Ok(result) => {
                self.applied = true;
                Ok(result)
            }
            Err(RecoveryGateError::EffectPanicked) => {
                self.panicked = true;
                Err(RecoveryGateError::EffectPanicked)
            }
            Err(error) => Err(error),
        }
    }

    pub fn was_applied(&self) -> bool {
        self.applied
    }

    pub fn effect_panicked(&self) -> bool {
        self.panicked
    }

    pub fn protocol_violation(&self) -> bool {
        self.protocol_violation
    }
}

fn run_recovery_effect<R>(
    descriptor: &ValidatedSystemServiceDescriptor,
    effect: impl FnOnce() -> std::io::Result<R>,
) -> Result<R, RecoveryGateError> {
    match catch_unwind(AssertUnwindSafe(effect)) {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => Err(RecoveryGateError::OwnerGateUnavailable),
        Err(_) => {
            block_recovery(descriptor);
            Err(RecoveryGateError::EffectPanicked)
        }
    }
}

/// One-shot typed reconciliation gate. C1 must acquire its durable recovery
/// lock before calling `run`; C3 then serializes the exact slot selected by the
/// same sealed admission while C1 decides and commits the durable terminal
/// state. Neither owner may independently reload or reconstruct the mapping
/// or launch generation during this operation.
pub struct SystemServiceReconciliationGate<'owner, O: SystemServiceLaunchOwner> {
    owner: &'owner mut O,
    admission: &'owner SystemServiceRecoveryAdmission,
    used: bool,
    successful: bool,
    protocol_violation: bool,
}

impl<O: SystemServiceLaunchOwner> SystemServiceReconciliationGate<'_, O> {
    pub fn run<R>(&mut self, reconcile: impl FnOnce() -> R) -> Result<R, LaunchPortError> {
        if self.used {
            self.protocol_violation = true;
            return Err(LaunchPortError::Rejected);
        }
        self.used = true;
        match self
            .owner
            .with_reconciliation_gate(self.admission, reconcile)
        {
            Ok(result) => {
                self.successful = true;
                Ok(result)
            }
            Err(error) => Err(error),
        }
    }

    pub fn was_used(&self) -> bool {
        self.used
    }

    pub fn was_successful(&self) -> bool {
        self.successful
    }

    pub fn protocol_violation(&self) -> bool {
        self.protocol_violation
    }
}

/// C1-owned private seal/recovery adapter. The implementation receives one
/// catalog-bound admission token and a one-shot gate. It must derive the C1
/// operation binding and C3 exact-generation gate from that same token, lock
/// the C1 recovery store and persist its exact intent before calling
/// `gate.apply`, then commit terminal state before returning success. The gate
/// exposes no C3 proof, preventing callback re-entry. This bootstrap operation
/// is independent of any user Session and must not perform user-session-
/// protected effects.
pub trait C1SystemServiceRecoveryClient {
    type Runtime;

    fn recover_system_service(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        package: &VerifiedServicePackageBinding<'_>,
        gate: &mut SystemServiceRecoveryGate<'_>,
    ) -> Result<Self::Runtime, C1RecoveryFailure>;

    /// Persist an unknown/partial outcome before returning from A. The C1
    /// implementation owns the durable fence and must fail closed if storage
    /// cannot be updated.
    fn quarantine_recovery(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
    ) -> Result<(), LaunchPortError>;

    /// Reconcile the durable C1 state after C3 has proved that the exact slot
    /// is stopped or otherwise isolated. The terminal action must invalidate
    /// any older handle on this live C1 service, or return a sealed proof that
    /// the old authority remains valid. The C3 gate must use this same sealed
    /// admission. A does not clear package or process fences without an
    /// operation-bound terminal receipt.
    fn reconcile_recovery<O: SystemServiceLaunchOwner>(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        gate: &mut SystemServiceReconciliationGate<'_, O>,
    ) -> Result<(), LaunchPortError>;
}

/// C1 returns the same proof on a rejected recovery so A can revoke its exact
/// C3 record. The error is deliberately bounded to the launch port contract.
pub struct C1RecoveryFailure {
    error: LaunchPortError,
}

impl C1RecoveryFailure {
    pub fn new(error: LaunchPortError) -> Self {
        Self { error }
    }

    pub fn error(&self) -> LaunchPortError {
        self.error
    }
}

/// A successfully recovered C1 runtime paired with the retained C3 proof.
/// This type has no `READY` method; liveness snapshots are for monitoring only.
pub struct SystemServiceRuntime<'owner, O, I>
where
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    descriptor: ValidatedSystemServiceDescriptor,
    admission: SystemServiceRecoveryAdmission,
    identity_runtime: Option<I::Runtime>,
    proof: Option<Arc<SystemServiceEndpointOwnerProof>>,
    lifecycle: Option<O::Lifecycle>,
    owner: &'owner mut O,
    identity_client: &'owner mut I,
    package: Option<VerifiedServicePackageBinding<'owner>>,
    closed: bool,
}

impl<O, I> SystemServiceRuntime<'_, O, I>
where
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    pub fn service_id(&self) -> &nagi_service_contract::ServiceId {
        self.descriptor.service_id()
    }

    pub fn generation(&self) -> std::num::NonZeroU64 {
        self.descriptor.generation()
    }

    pub fn identity_runtime(&self) -> &I::Runtime {
        self.identity_runtime
            .as_ref()
            .expect("live service runtime retains its C1 owner")
    }

    pub fn package_binding(&self) -> &VerifiedServicePackageBinding<'_> {
        self.package
            .as_ref()
            .expect("live service runtime retains its package lease")
    }

    pub fn owner_liveness_snapshot(&self) -> std::io::Result<bool> {
        self.proof
            .as_ref()
            .expect("live service runtime retains its C3 proof")
            .is_live()
    }

    /// Stop this runtime and return its exact admission if any cleanup outcome
    /// is uncertain. Callers can use the returned admission to resume the
    /// operation-bound C1 reconciliation path. `Drop` remains a fail-closed
    /// fallback and persists both owner quarantines when this method is not
    /// used.
    pub fn shutdown(mut self) -> Result<(), LaunchError> {
        let identity_drop_unknown =
            catch_unwind(AssertUnwindSafe(|| drop(self.identity_runtime.take()))).is_err();
        if identity_drop_unknown {
            block_recovery(&self.descriptor);
            if let Some(package) = self.package.as_ref() {
                package.quarantine();
            }
        }

        let cleanup = match self.lifecycle.take() {
            Some(lifecycle) => finish_lifecycle(
                &mut *self.owner,
                &mut *self.identity_client,
                &self.descriptor,
                lifecycle,
                self.proof.as_deref(),
                &self.admission,
                self.package.take(),
            ),
            None => {
                block_recovery(&self.descriptor);
                if let Some(package) = self.package.take() {
                    package.quarantine();
                }
                Err(reconciliation_required(
                    &mut *self.owner,
                    &mut *self.identity_client,
                    &self.admission,
                    true,
                ))
            }
        };

        self.closed = true;
        match (identity_drop_unknown, cleanup) {
            (false, result) => result,
            (true, Ok(())) => {
                block_recovery(&self.descriptor);
                Err(reconciliation_required(
                    &mut *self.owner,
                    &mut *self.identity_client,
                    &self.admission,
                    false,
                ))
            }
            (true, Err(error)) => Err(error),
        }
    }
}

impl<O, I> Drop for SystemServiceRuntime<'_, O, I>
where
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let identity_runtime = self.identity_runtime.take();
        let identity_drop_unknown =
            catch_unwind(AssertUnwindSafe(|| drop(identity_runtime))).is_err();
        if identity_drop_unknown {
            block_recovery(&self.descriptor);
            if let Some(package) = self.package.as_ref() {
                package.quarantine();
            }
        }

        let Some(lifecycle) = self.lifecycle.take() else {
            if let Some(package) = self.package.take() {
                package.quarantine();
            }
            block_recovery(&self.descriptor);
            let _ = reconciliation_required(
                &mut *self.owner,
                &mut *self.identity_client,
                &self.admission,
                true,
            );
            self.closed = true;
            return;
        };
        let cleanup = finish_lifecycle(
            &mut *self.owner,
            &mut *self.identity_client,
            &self.descriptor,
            lifecycle,
            self.proof.as_deref(),
            &self.admission,
            self.package.take(),
        );
        if identity_drop_unknown && cleanup.is_ok() {
            let _ = reconciliation_required(
                &mut *self.owner,
                &mut *self.identity_client,
                &self.admission,
                false,
            );
        }
        if identity_drop_unknown || cleanup.is_err() {
            block_recovery(&self.descriptor);
        }
        self.closed = true;
    }
}

fn finish_lifecycle<O: SystemServiceLaunchOwner, I: C1SystemServiceRecoveryClient>(
    owner: &mut O,
    identity: &mut I,
    descriptor: &ValidatedSystemServiceDescriptor,
    lifecycle: O::Lifecycle,
    proof: Option<&SystemServiceEndpointOwnerProof>,
    admission: &SystemServiceRecoveryAdmission,
    package: Option<VerifiedServicePackageBinding<'_>>,
) -> Result<(), LaunchError> {
    let metadata = descriptor.launch_metadata();
    match catch_unwind(AssertUnwindSafe(|| {
        owner.finish_exact(&metadata, lifecycle, proof)
    })) {
        Ok(LifecycleFinish::StoppedAndRevoked) => {
            drop(package);
            Ok(())
        }
        Ok(LifecycleFinish::Quarantined) => {
            block_recovery(descriptor);
            if let Some(package) = package {
                package.quarantine();
            }
            Err(reconciliation_required(owner, identity, admission, false))
        }
        Ok(LifecycleFinish::Unresolved(lifecycle)) => {
            block_recovery(descriptor);
            if let Some(package) = package {
                package.quarantine();
            }
            let result = reconciliation_required(owner, identity, admission, true);
            mem::forget(lifecycle);
            Err(result)
        }
        Err(_) => {
            block_recovery(descriptor);
            if let Some(package) = package {
                package.quarantine();
            }
            Err(reconciliation_required(owner, identity, admission, true))
        }
    }
}

fn reconciliation_required<O: SystemServiceLaunchOwner, I: C1SystemServiceRecoveryClient>(
    owner: &mut O,
    identity: &mut I,
    admission: &SystemServiceRecoveryAdmission,
    quarantine_c3: bool,
) -> LaunchError {
    let c1 = catch_unwind(AssertUnwindSafe(|| identity.quarantine_recovery(admission)))
        .unwrap_or(Err(LaunchPortError::OutcomeUnknown))
        .err();
    let c3 = quarantine_c3
        .then(|| {
            catch_unwind(AssertUnwindSafe(|| {
                owner.persist_recovery_quarantine(admission)
            }))
            .unwrap_or(Err(LaunchPortError::OutcomeUnknown))
            .err()
        })
        .flatten();

    match (c1, c3) {
        (None, None) => LaunchError::RecoveryReconciliationRequired(admission.clone()),
        (c1, c3) => LaunchError::RecoveryQuarantineFailed {
            c1,
            c3,
            admission: admission.clone(),
        },
    }
}

enum RecoveryAttempt<R> {
    Recovered(R),
    Failed(C1RecoveryFailure),
    Panicked,
}

fn recover_with_panic_boundary<R>(
    descriptor: &ValidatedSystemServiceDescriptor,
    recover: impl FnOnce() -> Result<R, C1RecoveryFailure>,
) -> RecoveryAttempt<R> {
    match catch_unwind(AssertUnwindSafe(recover)) {
        Ok(Ok(runtime)) => RecoveryAttempt::Recovered(runtime),
        Ok(Err(failure)) => {
            if failure.error() == LaunchPortError::OutcomeUnknown {
                block_recovery(descriptor);
            }
            RecoveryAttempt::Failed(failure)
        }
        Err(_) => {
            block_recovery(descriptor);
            RecoveryAttempt::Panicked
        }
    }
}

/// Validate, inspect, launch, authenticate the owner proof, then ask C1 to
/// recover. Failures stop before the next owner; after C3 launch, all failures
/// revoke the exact C3 slot. A calls C1 without holding the C3 slot gate; C1
/// must establish its durable recovery gate before invoking the one-shot C3
/// live-authority effect. C3 holds its generation gate only while that effect
/// runs. The captured `identity` port must be a bounded recovery client, not
/// an IdentityService owner. This path does not perform user-session-protected
/// effects; those require a separate adapter that takes the C1 Session gate
/// before C3/C2 owner gates, without capturing the issuing IdentityService
/// owner in the callback.
pub fn launch_identity_service<'owner, C, P, O, I>(
    descriptor_json: &str,
    package_bytes: &[u8],
    package_owner: &'owner PackageServiceSessionOwner<'_>,
    catalog: &mut C,
    preflight: &mut P,
    owner: &'owner mut O,
    identity: &'owner mut I,
) -> Result<SystemServiceRuntime<'owner, O, I>, LaunchError>
where
    C: TrustedSystemServiceCatalog,
    P: ArtifactPreflight,
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    let descriptor = validate_descriptor_json(descriptor_json).map_err(LaunchError::Descriptor)?;
    if is_recovery_blocked(&descriptor) {
        return Err(LaunchError::IdentityRecoveryBlocked(
            LaunchPortError::OutcomeUnknown,
        ));
    }
    let catalog_binding = catalog
        .authenticate(&descriptor)
        .map_err(LaunchError::Catalog)?;
    if !catalog_binding.matches_descriptor(&descriptor) {
        return Err(LaunchError::CatalogBindingMismatch);
    }

    let request = descriptor.artifact_preflight_request();
    let report_json = preflight
        .inspect(&request)
        .map_err(LaunchError::ArtifactPreflight)?;
    let report = validate_artifact_preflight_report(&report_json, &descriptor)
        .map_err(LaunchError::ArtifactReport)?;

    let package = crate::bind_installed_service_package(&descriptor, package_bytes, package_owner)
        .map_err(LaunchError::PackageBinding)?;
    if report.artifact_sha256() != package.executable_sha256() {
        return Err(LaunchError::ArtifactReport(
            PreflightReportError::DigestMismatch,
        ));
    }

    let launch_metadata = descriptor.launch_metadata();
    let mut lifecycle = match catch_unwind(AssertUnwindSafe(|| {
        owner.reserve_launch(&launch_metadata, &package)
    })) {
        Ok(Ok(lifecycle)) => lifecycle,
        Ok(Err(error)) => {
            return Err(LaunchError::ProcessOwner(match error {
                ReservationError::Unavailable => LaunchPortError::Unavailable,
                ReservationError::Rejected => LaunchPortError::Rejected,
                ReservationError::Failed => LaunchPortError::Failed,
            }));
        }
        Err(_) => {
            // The reservation API is failure-atomic and cannot create a child.
            return Err(LaunchError::ProcessOwnerPanicked);
        }
    };
    let admission = SystemServiceRecoveryAdmission::new(catalog_binding, &package, &lifecycle);
    if let Err(error) = validate_lifecycle_binding(&descriptor, &package, &lifecycle) {
        let cleanup = finish_lifecycle(
            owner,
            identity,
            &descriptor,
            lifecycle,
            None,
            &admission,
            Some(package),
        );
        return match cleanup {
            Err(cleanup_error) => Err(cleanup_error),
            Ok(()) => Err(LaunchError::InvalidOwnerProof(error)),
        };
    }

    let launch = catch_unwind(AssertUnwindSafe(|| {
        owner.launch_and_authenticate(&launch_metadata, &package, &mut lifecycle)
    }));
    let proof = match launch {
        Ok(Ok(proof)) => proof,
        Ok(Err(error)) => {
            let cleanup = finish_lifecycle(
                owner,
                identity,
                &descriptor,
                lifecycle,
                None,
                &admission,
                Some(package),
            );
            return match cleanup {
                Err(cleanup_error) => Err(cleanup_error),
                Ok(()) => Err(LaunchError::ProcessOwner(error)),
            };
        }
        Err(_) => {
            let cleanup = finish_lifecycle(
                owner,
                identity,
                &descriptor,
                lifecycle,
                None,
                &admission,
                Some(package),
            );
            return match cleanup {
                Err(cleanup_error) => Err(cleanup_error),
                Ok(()) => Err(LaunchError::ProcessOwnerPanicked),
            };
        }
    };
    let proof = Arc::new(proof);
    if let Err(error) = validate_owner_proof(&descriptor, &package, &lifecycle, &proof) {
        let cleanup = finish_lifecycle(
            owner,
            identity,
            &descriptor,
            lifecycle,
            Some(&proof),
            &admission,
            Some(package),
        );
        return match cleanup {
            Err(cleanup_error) => Err(cleanup_error),
            Ok(()) => Err(LaunchError::InvalidOwnerProof(error)),
        };
    }
    let mut recovery_gate = SystemServiceRecoveryGate::new(&proof, &descriptor);
    let recovery = recover_with_panic_boundary(&descriptor, || {
        identity.recover_system_service(&admission, &package, &mut recovery_gate)
    });
    let gate_panicked = recovery_gate.effect_panicked();
    let gate_applied = recovery_gate.was_applied();
    let gate_protocol_violation = recovery_gate.protocol_violation();
    match recovery {
        RecoveryAttempt::Failed(failure) => {
            let unknown = failure.error() == LaunchPortError::OutcomeUnknown
                || gate_panicked
                || gate_protocol_violation;
            let quarantine = if unknown {
                block_recovery(&descriptor);
                package.quarantine();
                catch_unwind(AssertUnwindSafe(|| {
                    identity.quarantine_recovery(&admission)
                }))
                .unwrap_or(Err(LaunchPortError::OutcomeUnknown))
            } else {
                Ok(())
            };
            let cleanup = finish_lifecycle(
                owner,
                identity,
                &descriptor,
                lifecycle,
                Some(&proof),
                &admission,
                Some(package),
            );
            if let Err(cleanup_error) = cleanup {
                return Err(cleanup_error);
            }
            if let Err(error) = quarantine {
                return Err(LaunchError::IdentityQuarantineFailed(error, admission));
            }
            if unknown {
                Err(LaunchError::IdentityOutcomeUnknown(admission))
            } else {
                Err(LaunchError::Identity(failure.error()))
            }
        }
        RecoveryAttempt::Panicked => {
            block_recovery(&descriptor);
            package.quarantine();
            let quarantine = catch_unwind(AssertUnwindSafe(|| {
                identity.quarantine_recovery(&admission)
            }))
            .unwrap_or(Err(LaunchPortError::OutcomeUnknown));
            let cleanup = finish_lifecycle(
                owner,
                identity,
                &descriptor,
                lifecycle,
                Some(&proof),
                &admission,
                Some(package),
            );
            if let Err(cleanup_error) = cleanup {
                return Err(cleanup_error);
            }
            if let Err(error) = quarantine {
                return Err(LaunchError::IdentityQuarantineFailed(error, admission));
            }
            Err(LaunchError::IdentityOutcomeUnknown(admission))
        }
        RecoveryAttempt::Recovered(identity_runtime) => {
            if !gate_applied || gate_panicked || gate_protocol_violation {
                block_recovery(&descriptor);
                package.quarantine();
                let quarantine = catch_unwind(AssertUnwindSafe(|| {
                    identity.quarantine_recovery(&admission)
                }))
                .unwrap_or(Err(LaunchPortError::OutcomeUnknown));
                let cleanup = finish_lifecycle(
                    owner,
                    identity,
                    &descriptor,
                    lifecycle,
                    Some(&proof),
                    &admission,
                    Some(package),
                );
                if let Err(cleanup_error) = cleanup {
                    return Err(cleanup_error);
                }
                return match quarantine {
                    Ok(()) => Err(LaunchError::RecoveryGateNotUsed),
                    Err(error) => Err(LaunchError::IdentityQuarantineFailed(error, admission)),
                };
            }
            Ok(SystemServiceRuntime {
                descriptor,
                admission,
                identity_runtime: Some(identity_runtime),
                proof: Some(proof),
                lifecycle: Some(lifecycle),
                owner,
                identity_client: identity,
                package: Some(package),
                closed: false,
            })
        }
    }
}

/// Attempt to reconcile both owners. This does not create a production
/// adapter: C1 must persist its operation-bound terminal state and C3 must
/// inspect the real Supervisor slot/process registry. The package owner has
/// no arbitrary clear callback, so a still-quarantined generation keeps the
/// local retry fence set and returns its exact admission to the caller.
pub fn reconcile_identity_service<O, I>(
    admission: &SystemServiceRecoveryAdmission,
    package_owner: &mut PackageServiceSessionOwner<'_>,
    owner: &mut O,
    identity: &mut I,
) -> Result<(), LaunchError>
where
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    let mut gate = SystemServiceReconciliationGate {
        owner,
        admission,
        used: false,
        successful: false,
        protocol_violation: false,
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        identity.reconcile_recovery(admission, &mut gate)
    }))
    .map_err(|_| {
        block_recovery(admission.descriptor());
        LaunchError::RecoveryReconciliationPanicked
    })?
    .map_err(|error| {
        block_recovery(admission.descriptor());
        LaunchError::RecoveryReconciliation(error)
    })?;
    let _ = result;
    if !gate.was_used() || !gate.was_successful() || gate.protocol_violation() {
        block_recovery(admission.descriptor());
        return Err(LaunchError::RecoveryGateNotUsed);
    }
    if package_owner.is_quarantined() {
        block_recovery(admission.descriptor());
        return Err(LaunchError::PackageQuarantineReleaseUnavailable(
            admission.clone(),
        ));
    }
    clear_recovery_block(admission.descriptor());
    Ok(())
}

fn validate_lifecycle_binding(
    descriptor: &ValidatedSystemServiceDescriptor,
    package: &VerifiedServicePackageBinding<'_>,
    lifecycle: &impl SystemServiceLaunchLifecycle,
) -> Result<(), ProofValidationError> {
    if lifecycle.service_id() != descriptor.service_id()
        || lifecycle.role() != descriptor.role()
        || lifecycle.catalog_generation() != descriptor.generation()
    {
        return Err(ProofValidationError::WrongServiceBinding);
    }
    let (uid, gid) = descriptor.credentials();
    if lifecycle.package_id() != package.package_id()
        || lifecycle.package_app_id() != package.app_id()
        || lifecycle.canonical_entry() != package.canonical_entry()
        || lifecycle.executable_sha256() != package.executable_sha256()
        || lifecycle.signed_package_sha256() != package.signed_package_sha256()
        || lifecycle.package_generation() != package.package_generation()
        || lifecycle.credentials() != (uid, gid)
    {
        return Err(ProofValidationError::WrongPackageBinding);
    }
    Ok(())
}

fn validate_owner_proof(
    descriptor: &ValidatedSystemServiceDescriptor,
    package: &VerifiedServicePackageBinding<'_>,
    lifecycle: &impl SystemServiceLaunchLifecycle,
    proof: &SystemServiceEndpointOwnerProof,
) -> Result<(), ProofValidationError> {
    if proof.version()
        != nagi_system_principal_contract::SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
    {
        return Err(ProofValidationError::WrongVersion);
    }
    validate_lifecycle_binding(descriptor, package, lifecycle)?;
    if proof.launch_generation() != lifecycle.c3_launch_generation() {
        return Err(ProofValidationError::WrongC3LaunchGeneration);
    }
    let Some(expected_process) = lifecycle.process_identity() else {
        return Err(ProofValidationError::MissingProcessIdentity);
    };
    if !proof.process_identity().same_instance(&expected_process) {
        return Err(ProofValidationError::WrongProcessIdentity);
    }
    let (expected_uid, expected_gid) = descriptor.credentials();
    let peer = proof.peer_credentials();
    if peer.uid != expected_uid || peer.gid != expected_gid {
        return Err(ProofValidationError::WrongPeerCredentials);
    }
    if proof.endpoint().is_none() {
        return Err(ProofValidationError::MissingEndpoint);
    }
    if !proof.is_live().map_err(|_| ProofValidationError::NotLive)? {
        return Err(ProofValidationError::NotLive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bind_installed_service_package, GenerationTransition, SYSTEM_SERVICE_TARGET};
    use ed25519_dalek::{Signer, SigningKey};
    use nagi_capability::PrincipalKind;
    use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
    use nagi_package::{
        build_xapp, InstallPolicy, PackageService, MAX_PACKAGE_BYTES, SIGNATURE_BYTES,
        TRUSTED_SIGNING_PUBLIC_KEY,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ARTIFACT_TEST_PATH: AtomicUsize = AtomicUsize::new(0);
    static RECOVERY_STATE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn recovery_state_test_guard() -> std::sync::MutexGuard<'static, ()> {
        RECOVERY_STATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TEST_SIGNING_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const FIXTURE_MANIFEST: &[u8] =
        b"id=org.nagi.fixture\nname=Fixture\nversion=1.0.0\nentry=fixture.elf\nsurfaces=compact\n";

    fn valid_descriptor() -> Value {
        json!({
            "schema_version": 1,
            "service_id": "fixture.identity",
            "role": "system_service",
            "package": { "id": "org.nagi.fixture", "canonical_entry": "fixture.elf" },
            "executable": {
                "artifact_path": "/fixture/identity.elf",
                "expected_sha256": DIGEST,
                "target": SYSTEM_SERVICE_TARGET
            },
            "credentials": { "uid": 1001, "gid": 1002 },
            "generation": 9,
            "lifecycle": { "mode": "initial" },
            "endpoint": {
                "transport": "unix_stream",
                "owner": "c3_pidfd_supervisor_generation",
                "handoff": "connected_owned_fd",
                "generation_policy": "unique_per_connection",
                "expected_peer_uid": 1001,
                "expected_peer_gid": 1002
            }
        })
    }

    fn valid_report() -> String {
        report_for_digest(DIGEST)
    }

    fn report_for_digest(digest: &str) -> String {
        json!({
            "status": "artifact_preflight_passed",
            "schema_version": 1,
            "target": SYSTEM_SERVICE_TARGET,
            "artifact_sha256": digest,
            "elf_type": "ET_DYN",
            "program_header_count": 10,
            "checks": {
                "regular_file": true,
                "no_symlink_path_components": true,
                "sha256_matches_expected": true,
                "elf64_little_endian": true,
                "x86_64": true,
                "executable_elf": true,
                "no_pt_interp": true
            },
            "claim_scope": "artifact_preflight_only",
            "not_proven": [
                "signature", "canonical_package_identity", "service_role",
                "user_authentication", "policy_authorization", "installation",
                "runtime_ready"
            ]
        })
        .to_string()
    }

    fn digest_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn fixture_package(executable: &[u8]) -> Vec<u8> {
        let key = SigningKey::from_bytes(&TEST_SIGNING_SEED);
        assert_eq!(key.verifying_key().to_bytes(), TRUSTED_SIGNING_PUBLIC_KEY);
        let placeholder = [0; SIGNATURE_BYTES];
        let mut bytes = [0; MAX_PACKAGE_BYTES];
        let length = build_xapp(
            FIXTURE_MANIFEST,
            executable,
            b"",
            b"",
            b"",
            &placeholder,
            &mut bytes,
        )
        .expect("build package fixture");
        let signature = key.sign(&bytes[..length - SIGNATURE_BYTES]);
        bytes[length - SIGNATURE_BYTES..length].copy_from_slice(&signature.to_bytes());
        bytes[..length].to_vec()
    }

    fn installed_fixture_owner(package_bytes: &[u8]) -> PackageServiceSessionOwner<'static> {
        let mut owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install test package under the fixture key");
        owner
    }

    fn empty_package_owner() -> PackageServiceSessionOwner<'static> {
        PackageServiceSessionOwner::new(None, PackageService::new())
    }

    fn descriptor_with_digest(digest: &str) -> String {
        let mut descriptor = valid_descriptor();
        descriptor["executable"]["expected_sha256"] = json!(digest);
        descriptor.to_string()
    }

    #[test]
    fn accepts_only_explicit_system_service_descriptor_shape() {
        let parsed = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        assert_eq!(parsed.role(), PrincipalKind::SystemService);
        assert_eq!(parsed.service_id().as_str(), "fixture.identity");
        assert_eq!(parsed.package_id(), "org.nagi.fixture");
        assert_eq!(parsed.canonical_entry(), "fixture.elf");
        assert_eq!(
            parsed.package_app_id(),
            nagi_model::AppId::from_identifier(b"org.nagi.fixture")
        );
        assert_eq!(parsed.credentials(), (1001, 1002));
        assert_eq!(parsed.generation().get(), 9);
        assert_eq!(parsed.transition(), GenerationTransition::Initial);
        assert_eq!(
            parsed.launch_metadata().transition(),
            GenerationTransition::Initial
        );
        let request: Value =
            serde_json::from_str(&parsed.artifact_preflight_request().to_json().unwrap()).unwrap();
        assert_eq!(request["schema_version"], 1);
        assert_eq!(request["artifact_path"], "/fixture/identity.elf");
        assert_eq!(request["expected_sha256"], DIGEST);
        assert_eq!(request["target"], SYSTEM_SERVICE_TARGET);
    }

    #[test]
    fn rejects_missing_unknown_duplicate_and_malformed_fields() {
        let mut missing = valid_descriptor();
        missing.as_object_mut().unwrap().remove("endpoint");
        assert_eq!(
            validate_descriptor_json(&missing.to_string()),
            Err(DescriptorError::InvalidJson)
        );

        let mut unknown = valid_descriptor();
        unknown["unexpected"] = json!(true);
        assert_eq!(
            validate_descriptor_json(&unknown.to_string()),
            Err(DescriptorError::InvalidJson)
        );

        let duplicate = valid_descriptor().to_string().replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        );
        assert_eq!(
            validate_descriptor_json(&duplicate),
            Err(DescriptorError::InvalidJson)
        );
        assert_eq!(
            validate_descriptor_json(" "),
            Err(DescriptorError::InvalidJson)
        );
    }

    #[test]
    fn rejects_role_id_digest_target_and_path_mismatches() {
        let cases = [
            ("role", json!("first_party_app"), DescriptorError::WrongRole),
            (
                "service_id",
                json!("Identity"),
                DescriptorError::InvalidServiceId,
            ),
            (
                "package.id",
                json!("Org.Nagi.Fixture"),
                DescriptorError::InvalidPackageId,
            ),
            (
                "package.canonical_entry",
                json!("../identity.elf"),
                DescriptorError::InvalidCanonicalEntry,
            ),
            (
                "executable.expected_sha256",
                json!(DIGEST.to_uppercase()),
                DescriptorError::InvalidDigest,
            ),
            (
                "executable.target",
                json!("aarch64-unknown-linux-musl"),
                DescriptorError::UnsupportedTarget,
            ),
            (
                "executable.artifact_path",
                json!("/fixture/../identity.elf"),
                DescriptorError::InvalidArtifactPath,
            ),
        ];
        for (path, value, expected) in cases {
            let mut descriptor = valid_descriptor();
            set_path(&mut descriptor, path, value);
            assert_eq!(
                validate_descriptor_json(&descriptor.to_string()),
                Err(expected),
                "path: {path}"
            );
        }
    }

    #[test]
    fn rejects_zero_and_stale_generation_transitions() {
        let mut zero = valid_descriptor();
        zero["generation"] = json!(0);
        assert_eq!(
            validate_descriptor_json(&zero.to_string()),
            Err(DescriptorError::ZeroGeneration)
        );

        for previous in [0, 9, 10] {
            let mut stale = valid_descriptor();
            stale["lifecycle"] = json!({ "mode": "replace", "previous_generation": previous });
            assert_eq!(
                validate_descriptor_json(&stale.to_string()),
                Err(DescriptorError::InvalidGenerationTransition),
                "previous generation: {previous}"
            );
        }
        let mut replacement = valid_descriptor();
        replacement["lifecycle"] = json!({ "mode": "replace", "previous_generation": 8 });
        let replacement = validate_descriptor_json(&replacement.to_string()).unwrap();
        assert!(matches!(
            replacement.transition(),
            GenerationTransition::Replace { previous_generation } if previous_generation.get() == 8
        ));
        assert!(matches!(
            replacement.launch_metadata().transition(),
            GenerationTransition::Replace { previous_generation } if previous_generation.get() == 8
        ));
    }

    #[test]
    fn rejects_endpoint_policy_and_credential_contradictions() {
        for (field, value) in [
            ("transport", json!("unix_datagram")),
            ("owner", json!("caller")),
            ("handoff", json!("path_reopen")),
            ("generation_policy", json!("shared")),
        ] {
            let mut descriptor = valid_descriptor();
            descriptor["endpoint"][field] = value;
            assert_eq!(
                validate_descriptor_json(&descriptor.to_string()),
                Err(DescriptorError::UnsupportedEndpoint),
                "endpoint field: {field}"
            );
        }
        let mut peer_uid = valid_descriptor();
        peer_uid["endpoint"]["expected_peer_uid"] = json!(1003);
        assert_eq!(
            validate_descriptor_json(&peer_uid.to_string()),
            Err(DescriptorError::PeerCredentialsMismatch)
        );
        let mut peer_gid = valid_descriptor();
        peer_gid["endpoint"]["expected_peer_gid"] = json!(1003);
        assert_eq!(
            validate_descriptor_json(&peer_gid.to_string()),
            Err(DescriptorError::PeerCredentialsMismatch)
        );
    }

    #[test]
    fn validates_preflight_report_without_promoting_its_claim_scope() {
        let descriptor = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        let report = validate_artifact_preflight_report(&valid_report(), &descriptor).unwrap();
        assert_eq!(report.artifact_sha256(), DIGEST);
        assert_eq!(report.elf_type(), "ET_DYN");
        assert_eq!(report.program_header_count(), 10);

        let mut mismatch: Value = serde_json::from_str(&valid_report()).unwrap();
        mismatch["artifact_sha256"] = json!("b".repeat(64));
        assert_eq!(
            validate_artifact_preflight_report(&mismatch.to_string(), &descriptor),
            Err(PreflightReportError::DigestMismatch)
        );
        let mut interp: Value = serde_json::from_str(&valid_report()).unwrap();
        interp["checks"]["no_pt_interp"] = json!(false);
        assert_eq!(
            validate_artifact_preflight_report(&interp.to_string(), &descriptor),
            Err(PreflightReportError::CheckFailed)
        );
        let mut scope: Value = serde_json::from_str(&valid_report()).unwrap();
        scope["not_proven"] = json!(["runtime_ready"]);
        assert_eq!(
            validate_artifact_preflight_report(&scope.to_string(), &descriptor),
            Err(PreflightReportError::ClaimScopeMismatch)
        );
    }

    #[test]
    fn package_binding_checks_catalog_id_entry_digest_and_holds_installed_generation() {
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        let binding = bind_installed_service_package(&descriptor, &package_bytes, &package_owner)
            .expect("signed fixture matches catalog-selected package values");
        assert_eq!(binding.package_id(), "org.nagi.fixture");
        assert_eq!(binding.canonical_entry(), "fixture.elf");
        assert_eq!(binding.executable_sha256(), digest);
        assert_eq!(binding.app_id(), descriptor.package_app_id());
        assert_eq!(binding.package_generation(), 1);
        drop(binding);

        let mut wrong_entry: Value =
            serde_json::from_str(&descriptor_with_digest(&digest)).unwrap();
        wrong_entry["package"]["canonical_entry"] = json!("other.elf");
        let wrong_entry = validate_descriptor_json(&wrong_entry.to_string()).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_entry, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::CanonicalEntryMismatch)
        );

        let wrong_digest =
            validate_descriptor_json(&descriptor_with_digest(&"b".repeat(64))).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_digest, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::ExecutableDigestMismatch)
        );

        let mut wrong_id: Value = serde_json::from_str(&descriptor_with_digest(&digest)).unwrap();
        wrong_id["package"]["id"] = json!("org.nagi.other");
        let wrong_id = validate_descriptor_json(&wrong_id.to_string()).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_id, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::ManifestIdMismatch)
        );
    }

    #[test]
    fn same_manifest_different_executable_cannot_bind_the_installed_generation() {
        let executable_a = b"fixture executable A";
        let executable_b = b"fixture executable B";
        let package_a = fixture_package(executable_a);
        let package_b = fixture_package(executable_b);
        let package_owner = installed_fixture_owner(&package_a);
        let digest_a = digest_hex(executable_a);
        let digest_b = digest_hex(executable_b);
        assert_ne!(digest_a, digest_b);

        let descriptor_a = validate_descriptor_json(&descriptor_with_digest(&digest_a)).unwrap();
        let binding_a = bind_installed_service_package(&descriptor_a, &package_a, &package_owner)
            .expect("the installed exact package bytes bind at generation 1");
        assert_eq!(binding_a.package_generation(), 1);
        drop(binding_a);

        let descriptor_b = validate_descriptor_json(&descriptor_with_digest(&digest_b)).unwrap();
        assert_eq!(
            bind_installed_service_package(&descriptor_b, &package_b, &package_owner).err(),
            Some(PackageBindingError::Owner(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    InstalledPackageBytesMismatch
            ))
        );
        assert_eq!(
            package_owner
                .capture(&package_a, b"org.nagi.fixture")
                .unwrap()
                .generation(),
            1,
            "rejected bytes must leave installed generation unchanged"
        );
    }

    #[test]
    fn c3_lifecycle_binding_separates_catalog_and_process_generations() {
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let (mut _catalog, _preflight, mut owner, _identity) = launch_ports();
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();

        assert_eq!(lifecycle.catalog_generation(), descriptor.generation());
        assert_ne!(lifecycle.c3_launch_generation(), descriptor.generation());
        assert_eq!(
            validate_lifecycle_binding(&descriptor, &binding, &lifecycle),
            Ok(())
        );
    }

    #[test]
    fn recovery_admission_binds_canonical_mapping_and_exact_package_generations() {
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut catalog = Catalog {
            error: None,
            calls: AtomicUsize::new(0),
        };
        let catalog_binding = catalog.authenticate(&descriptor).unwrap();
        let signed_package_sha256 = *binding.signed_package_sha256();
        let admission = SystemServiceRecoveryAdmission::for_reconciliation(
            catalog_binding.clone(),
            &binding,
            NonZeroU64::new(33).unwrap(),
            NonZeroU64::new(3).unwrap(),
            1,
            signed_package_sha256,
        )
        .unwrap();

        assert_eq!(admission.service_id().as_str(), "fixture.identity");
        assert_eq!(admission.role(), PrincipalKind::SystemService);
        assert_eq!(
            admission.principal_id().to_string(),
            "service:fixture.identity"
        );
        assert_eq!(
            admission.caller_context_id().as_str(),
            "caller_fixture_identity"
        );
        assert_eq!(admission.mapping_generation().get(), 3);
        assert_eq!(admission.catalog_generation().get(), 9);
        assert_eq!(admission.c3_launch_generation().get(), 33);
        assert_eq!(admission.package_generation(), 1);

        assert_eq!(
            SystemServiceRecoveryAdmission::for_reconciliation(
                catalog_binding.clone(),
                &binding,
                NonZeroU64::new(33).unwrap(),
                NonZeroU64::new(4).unwrap(),
                1,
                signed_package_sha256,
            )
            .err(),
            Some(SystemServiceAdmissionError::MappingGenerationMismatch)
        );
        assert_eq!(
            SystemServiceRecoveryAdmission::for_reconciliation(
                catalog_binding.clone(),
                &binding,
                NonZeroU64::new(33).unwrap(),
                NonZeroU64::new(3).unwrap(),
                2,
                signed_package_sha256,
            )
            .err(),
            Some(SystemServiceAdmissionError::PackageGenerationMismatch)
        );
        assert_eq!(
            SystemServiceRecoveryAdmission::for_reconciliation(
                catalog_binding,
                &binding,
                NonZeroU64::new(33).unwrap(),
                NonZeroU64::new(3).unwrap(),
                1,
                [0; 32],
            )
            .err(),
            Some(SystemServiceAdmissionError::SignedPackageDigestMismatch)
        );
    }

    struct Catalog {
        error: Option<LaunchPortError>,
        calls: AtomicUsize,
    }

    impl TrustedSystemServiceCatalog for Catalog {
        fn authenticate(
            &mut self,
            descriptor: &ValidatedSystemServiceDescriptor,
        ) -> Result<AuthenticatedSystemServiceCatalogBinding, LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(
                AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
                    descriptor,
                    PrincipalId::new("service:fixture.identity").unwrap(),
                    CallerContextId::new("caller_fixture_identity").unwrap(),
                    NonZeroU64::new(3).unwrap(),
                ),
            )
        }
    }

    struct Preflight {
        error: Option<LaunchPortError>,
        report: String,
        calls: AtomicUsize,
    }

    impl ArtifactPreflight for Preflight {
        fn inspect(
            &mut self,
            _request: &ArtifactPreflightRequest,
        ) -> Result<String, LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(self.report.clone())
        }
    }

    struct Owner {
        error: Option<LaunchPortError>,
        calls: AtomicUsize,
        revocations: AtomicUsize,
        quarantines: AtomicUsize,
        finish: FinishBehavior,
        mutate_artifact_before_launch: Option<(PathBuf, Vec<u8>)>,
        launched_executable: Option<Vec<u8>>,
    }

    struct TestLifecycle {
        service_id: ServiceId,
        role: PrincipalKind,
        catalog_generation: NonZeroU64,
        package_id: String,
        package_app_id: AppId,
        canonical_entry: String,
        executable_sha256: String,
        signed_package_sha256: [u8; 32],
        package_generation: u64,
        credentials: (u32, u32),
        c3_launch_generation: NonZeroU64,
        process_identity: Option<SpawnedProcessIdentity>,
    }

    impl SystemServiceLaunchLifecycle for TestLifecycle {
        fn service_id(&self) -> &ServiceId {
            &self.service_id
        }

        fn role(&self) -> PrincipalKind {
            self.role
        }

        fn catalog_generation(&self) -> NonZeroU64 {
            self.catalog_generation
        }

        fn package_id(&self) -> &str {
            &self.package_id
        }

        fn package_app_id(&self) -> AppId {
            self.package_app_id
        }

        fn canonical_entry(&self) -> &str {
            &self.canonical_entry
        }

        fn executable_sha256(&self) -> &str {
            &self.executable_sha256
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            &self.signed_package_sha256
        }

        fn package_generation(&self) -> u64 {
            self.package_generation
        }

        fn credentials(&self) -> (u32, u32) {
            self.credentials
        }

        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.c3_launch_generation
        }

        fn process_identity(&self) -> Option<SpawnedProcessIdentity> {
            self.process_identity.clone()
        }
    }

    #[derive(Clone, Copy)]
    enum FinishBehavior {
        Stopped,
        Quarantined,
        Unresolved,
        Panicked,
    }

    impl SystemServiceLaunchOwner for Owner {
        type Lifecycle = TestLifecycle;

        fn reserve_launch(
            &mut self,
            metadata: &SystemServiceLaunchMetadata,
            package: &VerifiedServicePackageBinding<'_>,
        ) -> Result<Self::Lifecycle, ReservationError> {
            Ok(TestLifecycle {
                service_id: metadata.service_id().clone(),
                role: metadata.role(),
                catalog_generation: metadata.generation(),
                package_id: package.package_id().to_owned(),
                package_app_id: package.app_id(),
                canonical_entry: package.canonical_entry().to_owned(),
                executable_sha256: package.executable_sha256().to_owned(),
                signed_package_sha256: *package.signed_package_sha256(),
                package_generation: package.package_generation(),
                credentials: metadata.credentials(),
                c3_launch_generation: NonZeroU64::new(33).unwrap(),
                process_identity: None,
            })
        }

        fn launch_and_authenticate(
            &mut self,
            metadata: &SystemServiceLaunchMetadata,
            package: &VerifiedServicePackageBinding<'_>,
            _lifecycle: &mut Self::Lifecycle,
        ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(package.package_id(), metadata.package_id());
            assert_eq!(package.canonical_entry(), metadata.canonical_entry());
            assert_eq!(package.executable_sha256(), metadata.expected_sha256());
            if let Some((path, replacement)) = self.mutate_artifact_before_launch.take() {
                let mut writer = OpenOptions::new()
                    .write(true)
                    .open(path)
                    .map_err(|_| LaunchPortError::Failed)?;
                writer
                    .write_all(&replacement)
                    .map_err(|_| LaunchPortError::Failed)?;
                writer.sync_all().map_err(|_| LaunchPortError::Failed)?;
            }
            self.launched_executable = Some(package.executable_bytes().to_vec());
            Err(self.error.unwrap_or(LaunchPortError::Unavailable))
        }

        fn finish_exact(
            &mut self,
            _metadata: &SystemServiceLaunchMetadata,
            lifecycle: Self::Lifecycle,
            _proof: Option<&SystemServiceEndpointOwnerProof>,
        ) -> LifecycleFinish<Self::Lifecycle> {
            self.revocations.fetch_add(1, Ordering::SeqCst);
            match self.finish {
                FinishBehavior::Stopped => LifecycleFinish::StoppedAndRevoked,
                FinishBehavior::Quarantined => LifecycleFinish::Quarantined,
                FinishBehavior::Unresolved => LifecycleFinish::Unresolved(lifecycle),
                FinishBehavior::Panicked => panic!("simulated exact C3 cleanup panic"),
            }
        }

        fn persist_recovery_quarantine(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
        ) -> Result<(), LaunchPortError> {
            self.quarantines.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn with_reconciliation_gate<R>(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            reconcile: impl FnOnce() -> R,
        ) -> Result<R, LaunchPortError> {
            Ok(reconcile())
        }
    }

    struct TestRuntime {
        panic_on_drop: bool,
    }

    impl Drop for TestRuntime {
        fn drop(&mut self) {
            if self.panic_on_drop {
                panic!("simulated C1 runtime drop panic after partial effect");
            }
        }
    }

    struct Identity {
        calls: AtomicUsize,
        quarantines: AtomicUsize,
    }

    impl C1SystemServiceRecoveryClient for Identity {
        type Runtime = TestRuntime;

        fn recover_system_service(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            _package: &VerifiedServicePackageBinding<'_>,
            _gate: &mut SystemServiceRecoveryGate<'_>,
        ) -> Result<Self::Runtime, C1RecoveryFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            unreachable!("host negative paths never receive a fabricated C3 proof")
        }

        fn quarantine_recovery(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
        ) -> Result<(), LaunchPortError> {
            self.quarantines.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn reconcile_recovery<O: SystemServiceLaunchOwner>(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            gate: &mut SystemServiceReconciliationGate<'_, O>,
        ) -> Result<(), LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            gate.run(|| ())?;
            Ok(())
        }
    }

    fn launch_ports() -> (Catalog, Preflight, Owner, Identity) {
        (
            Catalog {
                error: None,
                calls: AtomicUsize::new(0),
            },
            Preflight {
                error: None,
                report: valid_report(),
                calls: AtomicUsize::new(0),
            },
            Owner {
                error: Some(LaunchPortError::Unavailable),
                calls: AtomicUsize::new(0),
                revocations: AtomicUsize::new(0),
                quarantines: AtomicUsize::new(0),
                finish: FinishBehavior::Stopped,
                mutate_artifact_before_launch: None,
                launched_executable: None,
            },
            Identity {
                calls: AtomicUsize::new(0),
                quarantines: AtomicUsize::new(0),
            },
        )
    }

    #[test]
    fn descriptor_rejection_stops_before_catalog_preflight_c3_and_c1() {
        let _guard = recovery_state_test_guard();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            "{}",
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::Descriptor(DescriptorError::InvalidJson))
        );
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
        assert_eq!(preflight.calls.load(Ordering::SeqCst), 0);
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn catalog_rejection_stops_before_preflight_c3_and_c1() {
        let _guard = recovery_state_test_guard();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        catalog.error = Some(LaunchPortError::Rejected);
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::Catalog(LaunchPortError::Rejected))
        );
        assert_eq!(preflight.calls.load(Ordering::SeqCst), 0);
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn preflight_failure_or_report_mismatch_stops_before_c3_and_c1() {
        let _guard = recovery_state_test_guard();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        preflight.error = Some(LaunchPortError::Unavailable);
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ArtifactPreflight(LaunchPortError::Unavailable))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);

        preflight.error = None;
        preflight.report = "{}".to_owned();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ArtifactReport(
                PreflightReportError::InvalidJson
            ))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn c3_launch_failure_stops_before_c1_recovery() {
        let _guard = recovery_state_test_guard();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        owner.error = Some(LaunchPortError::Failed);
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        preflight.report = report_for_digest(&digest);
        let result = launch_identity_service(
            &descriptor_with_digest(&digest),
            &package_bytes,
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ProcessOwner(LaunchPortError::Failed))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn c3_unknown_with_quarantine_blocks_retry_until_both_owners_reconcile() {
        let _guard = recovery_state_test_guard();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        owner.error = Some(LaunchPortError::OutcomeUnknown);
        owner.finish = FinishBehavior::Quarantined;
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let mut package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let descriptor_json = descriptor_with_digest(&digest);
        let descriptor = validate_descriptor_json(&descriptor_json).unwrap();
        clear_recovery_block(&descriptor);
        preflight.report = report_for_digest(&digest);

        let admission = {
            let first = launch_identity_service(
                &descriptor_json,
                &package_bytes,
                &package_owner,
                &mut catalog,
                &mut preflight,
                &mut owner,
                &mut identity,
            );
            match first {
                Err(LaunchError::RecoveryReconciliationRequired(admission)) => admission,
                Err(error) => panic!("expected exact reconciliation token, got {error:?}"),
                Ok(runtime) => {
                    drop(runtime);
                    panic!("unexpected successful launch")
                }
            }
        };
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert!(is_recovery_blocked(&descriptor));
        assert_eq!(
            package_owner.remove(descriptor.package_app_id()),
            Err(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    Quarantined
            )
        );

        {
            let second = launch_identity_service(
                &descriptor_json,
                &package_bytes,
                &package_owner,
                &mut catalog,
                &mut preflight,
                &mut owner,
                &mut identity,
            );
            match second {
                Err(error) => assert_eq!(
                    error,
                    LaunchError::IdentityRecoveryBlocked(LaunchPortError::OutcomeUnknown)
                ),
                Ok(runtime) => {
                    drop(runtime);
                    panic!("blocked recovery unexpectedly launched")
                }
            }
        }
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);

        assert_eq!(
            reconcile_identity_service(&admission, &mut package_owner, &mut owner, &mut identity),
            Err(LaunchError::PackageQuarantineReleaseUnavailable(
                admission.clone()
            ))
        );
        assert!(is_recovery_blocked(&descriptor));
        assert!(package_owner.is_quarantined());
        assert_eq!(identity.calls.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 0);
        assert_eq!(
            package_owner.capture(&package_bytes, b"org.nagi.fixture"),
            Err(nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::Quarantined)
        );
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn c1_panic_is_converted_to_unknown_and_fences_the_service_key() {
        let _guard = recovery_state_test_guard();
        let descriptor = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        clear_recovery_block(&descriptor);

        let result: RecoveryAttempt<()> = recover_with_panic_boundary(&descriptor, || {
            panic!("simulated panic after a partial C1 write")
        });
        assert!(matches!(result, RecoveryAttempt::Panicked));
        assert!(is_recovery_blocked(&descriptor));

        assert_eq!(
            run_recovery_effect(&descriptor, || {
                panic!("simulated panic inside the C3 gated effect")
            }),
            Err::<(), _>(RecoveryGateError::EffectPanicked)
        );
        assert!(is_recovery_blocked(&descriptor));

        clear_recovery_block(&descriptor);
    }

    #[test]
    fn runtime_drop_runs_exact_cleanup_and_fences_package_after_c1_drop_panic() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let mut package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Stopped,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };

        let runtime = SystemServiceRuntime {
            descriptor: descriptor.clone(),
            admission,
            identity_runtime: Some(TestRuntime {
                panic_on_drop: true,
            }),
            proof: None,
            lifecycle: Some(lifecycle),
            owner: &mut owner,
            identity_client: &mut identity,
            package: Some(binding),
            closed: false,
        };
        drop(runtime);

        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 0);
        assert!(is_recovery_blocked(&descriptor));
        assert_eq!(
            package_owner.remove(descriptor.package_app_id()),
            Err(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    Quarantined
            )
        );
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn explicit_shutdown_returns_admission_after_c1_runtime_drop_panic() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let mut package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Stopped,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };
        let runtime = SystemServiceRuntime {
            descriptor: descriptor.clone(),
            admission: admission.clone(),
            identity_runtime: Some(TestRuntime {
                panic_on_drop: true,
            }),
            proof: None,
            lifecycle: Some(lifecycle),
            owner: &mut owner,
            identity_client: &mut identity,
            package: Some(binding),
            closed: false,
        };

        assert_eq!(
            runtime.shutdown(),
            Err(LaunchError::RecoveryReconciliationRequired(admission))
        );
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 0);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert!(is_recovery_blocked(&descriptor));
        assert_eq!(
            package_owner.remove(descriptor.package_app_id()),
            Err(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    Quarantined
            )
        );
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn explicit_shutdown_returns_admission_when_c3_cleanup_panics() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Panicked,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };
        let runtime = SystemServiceRuntime {
            descriptor: descriptor.clone(),
            admission: admission.clone(),
            identity_runtime: Some(TestRuntime {
                panic_on_drop: false,
            }),
            proof: None,
            lifecycle: Some(lifecycle),
            owner: &mut owner,
            identity_client: &mut identity,
            package: Some(binding),
            closed: false,
        };

        assert_eq!(
            runtime.shutdown(),
            Err(LaunchError::RecoveryReconciliationRequired(admission))
        );
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 1);
        assert!(package_owner.is_quarantined());
        assert!(is_recovery_blocked(&descriptor));
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn runtime_drop_persists_both_quarantines_when_c3_cleanup_is_unresolved() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Unresolved,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };
        let runtime = SystemServiceRuntime {
            descriptor: descriptor.clone(),
            admission,
            identity_runtime: Some(TestRuntime {
                panic_on_drop: false,
            }),
            proof: None,
            lifecycle: Some(lifecycle),
            owner: &mut owner,
            identity_client: &mut identity,
            package: Some(binding),
            closed: false,
        };
        drop(runtime);

        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 1);
        assert!(package_owner.is_quarantined());
        assert!(is_recovery_blocked(&descriptor));
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn exact_cleanup_panic_returns_reconciliation_token_and_keeps_package_fenced() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let mut package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Panicked,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };

        let result = finish_lifecycle(
            &mut owner,
            &mut identity,
            &descriptor,
            lifecycle,
            None,
            &admission,
            Some(binding),
        );
        assert_eq!(
            result,
            Err(LaunchError::RecoveryReconciliationRequired(
                admission.clone()
            ))
        );
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(
            package_owner.remove(descriptor.package_app_id()),
            Err(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    Quarantined
            )
        );
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn unresolved_exact_cleanup_keeps_retry_fenced() {
        let _guard = recovery_state_test_guard();
        let mut owner = Owner {
            error: None,
            calls: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
            finish: FinishBehavior::Unresolved,
            mutate_artifact_before_launch: None,
            launched_executable: None,
        };
        let package_bytes = fixture_package(b"fixture system service executable");
        let digest = digest_hex(b"fixture system service executable");
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        let package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let lifecycle = owner
            .reserve_launch(&descriptor.launch_metadata(), &binding)
            .unwrap();
        let catalog = AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &descriptor,
            PrincipalId::new("service:fixture.identity").unwrap(),
            CallerContextId::new("caller_fixture_identity").unwrap(),
            NonZeroU64::new(3).unwrap(),
        );
        let admission = SystemServiceRecoveryAdmission::new(catalog, &binding, &lifecycle);
        let mut identity = Identity {
            calls: AtomicUsize::new(0),
            quarantines: AtomicUsize::new(0),
        };

        let result = finish_lifecycle(
            &mut owner,
            &mut identity,
            &descriptor,
            lifecycle,
            None,
            &admission,
            Some(binding),
        );
        assert_eq!(
            result,
            Err(LaunchError::RecoveryReconciliationRequired(
                admission.clone()
            ))
        );
        assert_eq!(owner.revocations.load(Ordering::SeqCst), 1);
        assert_eq!(identity.quarantines.load(Ordering::SeqCst), 1);
        assert_eq!(owner.quarantines.load(Ordering::SeqCst), 1);
        assert!(is_recovery_blocked(&descriptor));

        clear_recovery_block(&descriptor);
    }

    #[test]
    fn post_hash_in_place_mutation_does_not_change_launch_snapshot() {
        let _guard = recovery_state_test_guard();
        use std::os::unix::fs::MetadataExt;

        let executable = b"fixture system service executable";
        let replacement = b"fixture system service EXECUTABLE";
        assert_eq!(executable.len(), replacement.len());

        let sequence = NEXT_ARTIFACT_TEST_PATH.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "nagi-service-launch-artifact-race-{}-{sequence}.elf",
            std::process::id()
        ));
        std::fs::write(&path, executable).unwrap();
        let before = std::fs::metadata(&path).unwrap();

        let digest = digest_hex(executable);
        let mut descriptor_value = valid_descriptor();
        descriptor_value["executable"]["artifact_path"] = json!(path.to_string_lossy().to_string());
        descriptor_value["executable"]["expected_sha256"] = json!(digest);
        let descriptor_json = descriptor_value.to_string();
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        preflight.report = report_for_digest(&digest);
        owner.mutate_artifact_before_launch = Some((path.clone(), replacement.to_vec()));

        let result = launch_identity_service(
            &descriptor_json,
            &package_bytes,
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );

        assert!(matches!(
            &result,
            Err(LaunchError::ProcessOwner(LaunchPortError::Unavailable))
        ));
        drop(result);
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        assert_eq!(std::fs::read(&path).unwrap(), replacement);
        assert_eq!(
            owner.launched_executable.as_deref(),
            Some(executable.as_slice())
        );
        assert_eq!(
            digest_hex(owner.launched_executable.as_deref().unwrap()),
            digest
        );
        assert_ne!(
            digest_hex(&std::fs::read(&path).unwrap()),
            digest_hex(owner.launched_executable.as_deref().unwrap())
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);

        std::fs::remove_file(path).unwrap();
    }

    fn set_path(value: &mut Value, path: &str, replacement: Value) {
        let mut segments = path.split('.').peekable();
        let mut current = value;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                current[segment] = replacement;
                return;
            }
            current = &mut current[segment];
        }
    }
}
