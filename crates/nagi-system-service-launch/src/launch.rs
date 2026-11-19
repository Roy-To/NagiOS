use crate::{
    validate_artifact_preflight_report, validate_descriptor_json, ArtifactPreflightRequest,
    DescriptorError, GenerationTransition, PackageBindingError, PreflightReportError,
    SystemServiceLaunchMetadata, ValidatedSystemServiceDescriptor, VerifiedServicePackageBinding,
};
use nagi_capability::PrincipalKind;
use nagi_identity_session::{CallerContextId, PrincipalId};
use nagi_init_package_provider::package_trust::{
    InstalledPackageBlobStore, PackageServiceSessionOwner,
};
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_model::AppId;
use nagi_service_contract::ServiceId;
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use nagi_system_service_recovery_contract::{
    canonical_recovery_binding_digest, canonical_recovery_binding_digest_from_scope,
    C1RecoveryOperationCorrelation, C1RecoveryReceiptClient, C3DurableRecoverySlotReceipt,
    C3RecoverySlotGate, RecoveryClientError, RecoveryLifecycleTransition, RecoveryReceiptMismatch,
    SystemServiceRecoveryAdmissionScope, SystemServiceRecoveryScopeInput,
};
use std::borrow::Cow;
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

impl From<RecoveryClientError> for LaunchPortError {
    fn from(error: RecoveryClientError) -> Self {
        match error {
            RecoveryClientError::Unavailable => Self::Unavailable,
            RecoveryClientError::Rejected => Self::Rejected,
            RecoveryClientError::Failed => Self::Failed,
            RecoveryClientError::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

fn recovery_client_error(error: LaunchPortError) -> RecoveryClientError {
    match error {
        LaunchPortError::Unavailable => RecoveryClientError::Unavailable,
        LaunchPortError::Rejected => RecoveryClientError::Rejected,
        LaunchPortError::Failed => RecoveryClientError::Failed,
        LaunchPortError::OutcomeUnknown => RecoveryClientError::OutcomeUnknown,
    }
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
    RecoveryTerminalReceiptUnavailable(SystemServiceRecoveryAdmission),
    RecoveryReceiptMismatch(RecoveryReceiptMismatch, SystemServiceRecoveryAdmission),
    RecoveryReceiptConsumptionFailed(LaunchPortError, SystemServiceRecoveryAdmission),
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

/// Canonical A/C3 map key for one service slot. The key deliberately excludes
/// catalog and process generations: a new generation replaces the same
/// service slot and cannot bypass a recovery fence for that service.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SystemServiceRuntimeSlotKey {
    service_id: ServiceId,
}

impl SystemServiceRuntimeSlotKey {
    pub fn new(service_id: &ServiceId) -> Self {
        Self {
            service_id: service_id.clone(),
        }
    }

    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }
}

/// Exact C3 reservation binding for one canonical service slot. This is
/// consistency metadata returned by the maintained C3 owner; it is not an
/// endpoint proof or authority. The proof still must bind the live pidfd,
/// peer credentials, and this unique C3 launch generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemServiceRuntimeSlotBinding {
    slot_key: SystemServiceRuntimeSlotKey,
    role: PrincipalKind,
    catalog_generation: NonZeroU64,
    transition: GenerationTransition,
    package_id: String,
    package_app_id: AppId,
    canonical_entry: String,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
    signed_package_bytes_len: u32,
    executable_bytes_len: u32,
    package_generation: u64,
    credentials: (u32, u32),
    c3_launch_generation: NonZeroU64,
}

impl SystemServiceRuntimeSlotBinding {
    /// Build the value returned by C3 only after it has atomically reserved
    /// the metadata's canonical service slot and allocated a unique process
    /// generation. The C3 adapter remains responsible for proving that the
    /// reservation is real and serialized with replacement/revocation.
    pub fn for_c3_reservation(
        metadata: &SystemServiceLaunchMetadata,
        package: &VerifiedServicePackageBinding<'_>,
        c3_launch_generation: NonZeroU64,
    ) -> Self {
        Self {
            slot_key: SystemServiceRuntimeSlotKey::new(metadata.service_id()),
            role: metadata.role(),
            catalog_generation: metadata.generation(),
            transition: metadata.transition(),
            package_id: package.package_id().to_owned(),
            package_app_id: package.app_id(),
            canonical_entry: package.canonical_entry().to_owned(),
            signed_package_sha256: *package.signed_package_sha256(),
            executable_sha256: *package.executable_sha256_bytes(),
            signed_package_bytes_len: package.signed_package_bytes_len(),
            executable_bytes_len: package.executable_bytes_len(),
            package_generation: package.package_generation(),
            credentials: metadata.credentials(),
            c3_launch_generation,
        }
    }

    pub fn slot_key(&self) -> &SystemServiceRuntimeSlotKey {
        &self.slot_key
    }

    pub fn service_id(&self) -> &ServiceId {
        self.slot_key.service_id()
    }

    pub fn role(&self) -> PrincipalKind {
        self.role
    }

    pub fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog_generation
    }

    pub fn transition(&self) -> GenerationTransition {
        self.transition
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

    pub fn signed_package_sha256(&self) -> &[u8; 32] {
        &self.signed_package_sha256
    }

    pub fn executable_sha256_bytes(&self) -> &[u8; 32] {
        &self.executable_sha256
    }

    pub fn signed_package_bytes_len(&self) -> u32 {
        self.signed_package_bytes_len
    }

    pub fn executable_bytes_len(&self) -> u32 {
        self.executable_bytes_len
    }

    pub fn package_generation(&self) -> u64 {
        self.package_generation
    }

    pub fn credentials(&self) -> (u32, u32) {
        self.credentials
    }

    pub fn c3_launch_generation(&self) -> NonZeroU64 {
        self.c3_launch_generation
    }
}

/// Opaque A-issued C1 admission input. It joins catalog-selected canonical
/// C1 IDs, descriptor role/service generation, the installed package
/// generation, and the exact C3 launch generation. The one-shot C3 gate binds
/// the live proof to this token while each effect runs.
/// C1 must persist this scope in its recovery journal and must not derive IDs
/// from descriptor text or kernel credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemServiceRecoveryAdmission(Box<SystemServiceAdmissionData>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct SystemServiceAdmissionData {
    catalog: AuthenticatedSystemServiceCatalogBinding,
    runtime_slot: SystemServiceRuntimeSlotBinding,
    scope_binding_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemServiceAdmissionError {
    ArtifactDigestMismatch,
    PackageBindingMismatch,
    MappingGenerationMismatch,
    PackageGenerationMismatch,
    SignedPackageDigestMismatch,
}

impl SystemServiceRecoveryAdmission {
    fn new(
        catalog: AuthenticatedSystemServiceCatalogBinding,
        lifecycle: &impl SystemServiceLaunchLifecycle,
    ) -> Self {
        let runtime_slot = lifecycle.runtime_slot_binding().clone();
        Self::from_parts(catalog, runtime_slot)
    }

    fn from_parts(
        catalog: AuthenticatedSystemServiceCatalogBinding,
        runtime_slot: SystemServiceRuntimeSlotBinding,
    ) -> Self {
        let transition = match runtime_slot.transition() {
            GenerationTransition::Initial => RecoveryLifecycleTransition::Initial,
            GenerationTransition::Replace {
                previous_generation,
            } => RecoveryLifecycleTransition::Replace {
                previous_generation,
            },
        };
        let role = match runtime_slot.role() {
            PrincipalKind::FirstPartyApp => "first_party_app",
            PrincipalKind::ThirdPartyApp => "third_party_app",
            PrincipalKind::SystemService => "system_service",
            PrincipalKind::AiMediatedAction => "ai_mediated_action",
            PrincipalKind::Automation => "automation",
        };
        let (uid, gid) = runtime_slot.credentials();
        let scope_fields = SystemServiceRecoveryScopeInput {
            service_id: runtime_slot.service_id().as_str(),
            principal_id: catalog.principal_id.as_str(),
            caller_context_id: catalog.caller_context_id.as_str(),
            mapping_generation: catalog.mapping_generation,
            role,
            catalog_generation: runtime_slot.catalog_generation(),
            transition,
            package_id: runtime_slot.package_id(),
            package_app_id: runtime_slot.package_app_id().0,
            canonical_entry: runtime_slot.canonical_entry(),
            signed_package_sha256: runtime_slot.signed_package_sha256(),
            executable_sha256: runtime_slot.executable_sha256_bytes(),
            package_generation: runtime_slot.package_generation(),
            c3_launch_generation: runtime_slot.c3_launch_generation(),
            uid,
            gid,
        };
        let scope_binding_digest = canonical_recovery_binding_digest(&scope_fields);
        Self(Box::new(SystemServiceAdmissionData {
            catalog,
            runtime_slot,
            scope_binding_digest,
        }))
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
        Self::from_catalog_and_package(catalog, package, c3_launch_generation)
    }

    pub(crate) fn from_catalog_and_package(
        catalog: AuthenticatedSystemServiceCatalogBinding,
        package: &VerifiedServicePackageBinding<'_>,
        c3_launch_generation: NonZeroU64,
    ) -> Result<Self, SystemServiceAdmissionError> {
        if catalog.descriptor.package_id() != package.package_id()
            || catalog.descriptor.package_app_id() != package.app_id()
            || catalog.descriptor.canonical_entry() != package.canonical_entry()
            || catalog.descriptor.expected_sha256() != package.executable_sha256()
        {
            return Err(SystemServiceAdmissionError::PackageBindingMismatch);
        }
        let metadata = catalog.descriptor.launch_metadata();
        let runtime_slot = SystemServiceRuntimeSlotBinding::for_c3_reservation(
            &metadata,
            package,
            c3_launch_generation,
        );
        Ok(Self::from_parts(catalog, runtime_slot))
    }

    pub fn descriptor(&self) -> &ValidatedSystemServiceDescriptor {
        &self.0.catalog.descriptor
    }

    pub fn service_id(&self) -> &ServiceId {
        self.0.runtime_slot.service_id()
    }

    pub fn role(&self) -> PrincipalKind {
        self.0.runtime_slot.role()
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.0.catalog.principal_id
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.0.catalog.caller_context_id
    }

    pub fn mapping_generation(&self) -> NonZeroU64 {
        self.0.catalog.mapping_generation
    }

    pub fn package_id(&self) -> &str {
        self.0.runtime_slot.package_id()
    }

    pub fn package_app_id(&self) -> AppId {
        self.0.runtime_slot.package_app_id()
    }

    pub fn canonical_entry(&self) -> &str {
        self.0.runtime_slot.canonical_entry()
    }

    pub fn executable_sha256_bytes(&self) -> &[u8; 32] {
        self.0.runtime_slot.executable_sha256_bytes()
    }

    pub fn signed_package_sha256(&self) -> &[u8; 32] {
        self.0.runtime_slot.signed_package_sha256()
    }

    pub fn package_generation(&self) -> u64 {
        self.0.runtime_slot.package_generation()
    }

    pub fn catalog_generation(&self) -> NonZeroU64 {
        self.0.runtime_slot.catalog_generation()
    }

    pub fn c3_launch_generation(&self) -> NonZeroU64 {
        self.0.runtime_slot.c3_launch_generation()
    }

    /// The same canonical scope A passes to C1 and C3. Its digest binds the
    /// principal mapping, catalog transition, package generations, and exact
    /// C3 launch slot. The scope's address stays fixed when its owning
    /// admission moves into the runtime. Cloning the admission creates a
    /// different scope object for callers that require live object identity.
    pub fn scope(&self) -> &dyn SystemServiceRecoveryAdmissionScope {
        self.0.as_ref()
    }

    pub fn runtime_slot_binding(&self) -> &SystemServiceRuntimeSlotBinding {
        &self.0.runtime_slot
    }
}

impl SystemServiceRecoveryAdmissionScope for SystemServiceAdmissionData {
    fn service_id(&self) -> &str {
        self.runtime_slot.service_id().as_str()
    }

    fn principal_id(&self) -> &str {
        self.catalog.principal_id.as_str()
    }

    fn caller_context_id(&self) -> &str {
        self.catalog.caller_context_id.as_str()
    }

    fn mapping_generation(&self) -> NonZeroU64 {
        self.catalog.mapping_generation
    }

    fn role(&self) -> &str {
        match self.runtime_slot.role() {
            PrincipalKind::FirstPartyApp => "first_party_app",
            PrincipalKind::ThirdPartyApp => "third_party_app",
            PrincipalKind::SystemService => "system_service",
            PrincipalKind::AiMediatedAction => "ai_mediated_action",
            PrincipalKind::Automation => "automation",
        }
    }

    fn catalog_generation(&self) -> NonZeroU64 {
        self.runtime_slot.catalog_generation()
    }

    fn transition(&self) -> RecoveryLifecycleTransition {
        match self.runtime_slot.transition() {
            GenerationTransition::Initial => RecoveryLifecycleTransition::Initial,
            GenerationTransition::Replace {
                previous_generation,
            } => RecoveryLifecycleTransition::Replace {
                previous_generation,
            },
        }
    }

    fn package_id(&self) -> &str {
        self.runtime_slot.package_id()
    }

    fn package_app_id(&self) -> u64 {
        self.runtime_slot.package_app_id().0
    }

    fn canonical_entry(&self) -> &str {
        self.runtime_slot.canonical_entry()
    }

    fn signed_package_sha256(&self) -> &[u8; 32] {
        self.runtime_slot.signed_package_sha256()
    }

    fn executable_sha256(&self) -> &[u8; 32] {
        self.runtime_slot.executable_sha256_bytes()
    }

    fn package_generation(&self) -> u64 {
        self.runtime_slot.package_generation()
    }

    fn c3_launch_generation(&self) -> NonZeroU64 {
        self.runtime_slot.c3_launch_generation()
    }

    fn credentials(&self) -> (u32, u32) {
        self.runtime_slot.credentials()
    }

    fn binding_digest(&self) -> &[u8; 32] {
        &self.scope_binding_digest
    }
}

type RecoveryKey = SystemServiceRuntimeSlotKey;

fn recovery_key(descriptor: &ValidatedSystemServiceDescriptor) -> RecoveryKey {
    SystemServiceRuntimeSlotKey::new(descriptor.service_id())
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
/// or an artifact digest alone cannot mint this binding.
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
    fn runtime_slot_binding(&self) -> &SystemServiceRuntimeSlotBinding;
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
    /// Exact C3 durable-slot result obtained under the reconciliation gate.
    type RecoveryReceipt: C3DurableRecoverySlotReceipt;

    fn reserve_launch(
        &mut self,
        metadata: &SystemServiceLaunchMetadata,
        package: &VerifiedServicePackageBinding<'_>,
    ) -> Result<Self::Lifecycle, ReservationError>;

    /// Spawn from the exact A admission built for this reservation. Use
    /// the same scope object for C3 spawn and endpoint proof issuance.
    fn launch_and_authenticate(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
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
        admission: &SystemServiceRecoveryAdmission,
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
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<(R, Self::RecoveryReceipt), LaunchPortError>;
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
    receipt: Option<O::RecoveryReceipt>,
}

impl<O: SystemServiceLaunchOwner> SystemServiceReconciliationGate<'_, O> {
    pub fn run<R>(
        &mut self,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, LaunchPortError> {
        if self.used || claim.binding_digest() != self.admission.scope().binding_digest() {
            self.protocol_violation = true;
            return Err(LaunchPortError::Rejected);
        }
        self.used = true;
        match self
            .owner
            .with_reconciliation_gate(self.admission, claim, reconcile)
        {
            Ok((result, receipt)) => {
                if receipt.c1_operation_id() != Some(claim.operation_id()) {
                    self.protocol_violation = true;
                    return Err(LaunchPortError::OutcomeUnknown);
                }
                self.successful = true;
                self.receipt = Some(receipt);
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

    fn take_receipt(&mut self) -> Option<O::RecoveryReceipt> {
        self.receipt.take()
    }
}

impl<O: SystemServiceLaunchOwner> C3RecoverySlotGate for SystemServiceReconciliationGate<'_, O> {
    fn with_reconciliation_gate<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError> {
        if !core::ptr::eq(scope, self.admission.scope()) {
            self.protocol_violation = true;
            return Err(RecoveryClientError::Rejected);
        }
        self.run(claim, reconcile).map_err(recovery_client_error)
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
pub trait C1SystemServiceRecoveryClient: C1RecoveryReceiptClient {
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
    match catch_unwind(AssertUnwindSafe(|| {
        owner.finish_exact(admission, lifecycle, proof)
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
    launch_identity_service_with_package(
        descriptor_json,
        |_| Ok(Cow::Borrowed(package_bytes)),
        package_owner,
        catalog,
        preflight,
        owner,
        identity,
    )
}

/// Launch using the signed package from the boot-selected durable store.
/// The caller supplies the authenticated catalog and the same package owner
/// used to restore that store's inventory.
pub fn launch_identity_service_from_store<'owner, C, P, O, I>(
    descriptor_json: &str,
    blobs: &dyn InstalledPackageBlobStore,
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
    launch_identity_service_with_package(
        descriptor_json,
        |descriptor| {
            package_owner
                .read_installed_system_service_package(descriptor.package_id().as_bytes(), blobs)
                .map(Cow::Owned)
                .map_err(PackageBindingError::Owner)
        },
        package_owner,
        catalog,
        preflight,
        owner,
        identity,
    )
}

fn launch_identity_service_with_package<'owner, 'package, C, P, O, I>(
    descriptor_json: &str,
    read_package: impl FnOnce(
        &ValidatedSystemServiceDescriptor,
    ) -> Result<Cow<'package, [u8]>, PackageBindingError>,
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

    let package_bytes = read_package(&descriptor).map_err(LaunchError::PackageBinding)?;
    let package = crate::bind_installed_service_package(&descriptor, &package_bytes, package_owner)
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
    let admission = SystemServiceRecoveryAdmission::new(catalog_binding, &lifecycle);
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
        owner.launch_and_authenticate(&admission, &package, &mut lifecycle)
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
    let scope = admission.scope();
    if canonical_recovery_binding_digest_from_scope(scope) != *scope.binding_digest() {
        block_recovery(admission.descriptor());
        return Err(LaunchError::RecoveryReceiptMismatch(
            RecoveryReceiptMismatch::NonCanonicalScopeBinding,
            admission.clone(),
        ));
    }
    let mut gate = SystemServiceReconciliationGate {
        owner,
        admission,
        used: false,
        successful: false,
        protocol_violation: false,
        receipt: None,
    };
    let terminal_receipt = catch_unwind(AssertUnwindSafe(|| {
        identity.reconcile_terminal(scope, &mut gate)
    }))
    .map_err(|_| {
        block_recovery(admission.descriptor());
        LaunchError::RecoveryReconciliationPanicked
    })?
    .map_err(|error| {
        block_recovery(admission.descriptor());
        LaunchError::RecoveryReconciliation(error.into())
    })?;
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
    let Some(slot_receipt) = gate.take_receipt() else {
        block_recovery(admission.descriptor());
        return Err(LaunchError::RecoveryTerminalReceiptUnavailable(
            admission.clone(),
        ));
    };
    if let Err(mismatch) = nagi_system_service_recovery_contract::validate_recovery_receipts(
        scope,
        &terminal_receipt,
        &slot_receipt,
    ) {
        block_recovery(admission.descriptor());
        return Err(LaunchError::RecoveryReceiptMismatch(
            mismatch,
            admission.clone(),
        ));
    }
    if let Err(error) = identity.consume_terminal_receipt(scope, terminal_receipt) {
        block_recovery(admission.descriptor());
        return Err(LaunchError::RecoveryReceiptConsumptionFailed(
            error.into(),
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
    let runtime = lifecycle.runtime_slot_binding();
    if runtime.slot_key() != &SystemServiceRuntimeSlotKey::new(descriptor.service_id())
        || runtime.role() != descriptor.role()
        || runtime.catalog_generation() != descriptor.generation()
        || runtime.transition() != descriptor.transition()
    {
        return Err(ProofValidationError::WrongServiceBinding);
    }
    let (uid, gid) = descriptor.credentials();
    if runtime.package_id() != package.package_id()
        || runtime.package_app_id() != package.app_id()
        || runtime.canonical_entry() != package.canonical_entry()
        || runtime.executable_sha256_bytes() != package.executable_sha256_bytes()
        || runtime.signed_package_sha256() != package.signed_package_sha256()
        || runtime.signed_package_bytes_len() != package.signed_package_bytes_len()
        || runtime.executable_bytes_len() != package.executable_bytes_len()
        || runtime.package_generation() != package.package_generation()
        || runtime.credentials() != (uid, gid)
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
    let slot = proof.service_slot_identity();
    if slot.service_id() != descriptor.service_id().as_str()
        || slot.catalog_generation() != descriptor.generation()
    {
        return Err(ProofValidationError::WrongServiceBinding);
    }
    if proof.launch_generation() != lifecycle.runtime_slot_binding().c3_launch_generation() {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::{bind_installed_service_package, GenerationTransition, SYSTEM_SERVICE_TARGET};
    use ed25519_dalek::{Signer, SigningKey};
    use nagi_capability::PrincipalKind;
    use nagi_init_package_provider::{
        linux_package_persistence::LinuxInstalledPackagePersistence,
        package_trust::{InstalledPackageDigestIndex, PackageServiceSessionOwner},
    };
    use nagi_package::{
        build_xapp, InstallPolicy, PackageService, PackageView, MAX_PACKAGE_BYTES, SIGNATURE_BYTES,
        TRUSTED_SIGNING_PUBLIC_KEY,
    };
    use nagi_system_service_recovery_contract::{
        C1RecoveryReceiptClient, C1RecoveryTerminalReceipt, C3RecoverySlotGate,
        C3RecoverySlotState, RecoveryClientError,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fs::{self, File, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_claim(
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        operation_id: u64,
    ) -> C1RecoveryOperationCorrelation {
        C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(operation_id).unwrap(),
            *scope.binding_digest(),
        )
    }

    static NEXT_ARTIFACT_TEST_PATH: AtomicUsize = AtomicUsize::new(0);
    static NEXT_PACKAGE_STORE_PATH: AtomicUsize = AtomicUsize::new(0);
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
    static ALTERNATE_SCOPE_DIGEST: [u8; 32] = [0xa5; 32];

    struct AlternatingScope {
        admission: SystemServiceRecoveryAdmission,
        binding_digest_reads: AtomicUsize,
    }

    impl AlternatingScope {
        fn base(&self) -> &dyn SystemServiceRecoveryAdmissionScope {
            self.admission.scope()
        }
    }

    impl SystemServiceRecoveryAdmissionScope for AlternatingScope {
        fn service_id(&self) -> &str {
            self.base().service_id()
        }

        fn principal_id(&self) -> &str {
            self.base().principal_id()
        }

        fn caller_context_id(&self) -> &str {
            self.base().caller_context_id()
        }

        fn mapping_generation(&self) -> NonZeroU64 {
            self.base().mapping_generation()
        }

        fn role(&self) -> &str {
            self.base().role()
        }

        fn catalog_generation(&self) -> NonZeroU64 {
            self.base().catalog_generation()
        }

        fn transition(&self) -> RecoveryLifecycleTransition {
            self.base().transition()
        }

        fn package_id(&self) -> &str {
            self.base().package_id()
        }

        fn package_app_id(&self) -> u64 {
            self.base().package_app_id()
        }

        fn canonical_entry(&self) -> &str {
            self.base().canonical_entry()
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            self.base().signed_package_sha256()
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            self.base().executable_sha256()
        }

        fn package_generation(&self) -> u64 {
            self.base().package_generation()
        }

        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.base().c3_launch_generation()
        }

        fn credentials(&self) -> (u32, u32) {
            self.base().credentials()
        }

        fn binding_digest(&self) -> &[u8; 32] {
            if self.binding_digest_reads.fetch_add(1, Ordering::SeqCst) % 2 == 0 {
                self.base().binding_digest()
            } else {
                &ALTERNATE_SCOPE_DIGEST
            }
        }
    }

    struct TestPackagePersistence {
        path: PathBuf,
        index: Option<LinuxInstalledPackagePersistence>,
    }

    impl TestPackagePersistence {
        fn new() -> Self {
            let serial = NEXT_PACKAGE_STORE_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nagi-package-store-test-{}-{serial}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create isolated package store directory");
            let root = File::open(&path).expect("open isolated package store directory");
            let owner_uid = root
                .metadata()
                .expect("read isolated package store metadata")
                .uid();
            let index = LinuxInstalledPackagePersistence::from_trusted_directory(root, owner_uid)
                .expect("adopt isolated package store directory");
            Self {
                path,
                index: Some(index),
            }
        }

        fn index(&self) -> &LinuxInstalledPackagePersistence {
            self.index.as_ref().expect("package store is live")
        }

        fn index_mut(&mut self) -> &mut LinuxInstalledPackagePersistence {
            self.index.as_mut().expect("package store is live")
        }
    }

    impl Drop for TestPackagePersistence {
        fn drop(&mut self) {
            drop(self.index.take());
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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
        let mut persistence = TestPackagePersistence::new();
        let mut owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install_with_digest_index(
                package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                persistence.index_mut(),
            )
            .expect("install and read back the fixture digest index");
        owner
    }

    #[test]
    fn linux_package_persistence_commits_and_reads_empty_snapshot() {
        let mut persistence = TestPackagePersistence::new();
        let commit = persistence
            .index_mut()
            .replace_snapshot(&[])
            .expect("commit empty package snapshot");
        drop(commit);
        let mut records = [None; nagi_package::MAX_SLOTS];
        let readback = persistence
            .index()
            .read_snapshot(&mut records)
            .expect("read empty package snapshot");
        drop(readback);
        assert!(records.iter().all(Option::is_none));
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
    fn service_binding_rejects_superseded_generation_and_missing_restored_digest() {
        let executable_a = b"fixture executable generation A";
        let executable_b = b"fixture executable generation B";
        let package_a = fixture_package(executable_a);
        let package_b = fixture_package(executable_b);
        let digest_a = digest_hex(executable_a);
        let descriptor_a = validate_descriptor_json(&descriptor_with_digest(&digest_a)).unwrap();

        let mut index = TestPackagePersistence::new();
        let mut owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install_with_digest_index(
                &package_a,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                index.index_mut(),
            )
            .unwrap();
        owner
            .install_with_digest_index(
                &package_b,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                index.index_mut(),
            )
            .unwrap();
        assert_eq!(
            bind_installed_service_package(&descriptor_a, &package_a, &owner).err(),
            Some(PackageBindingError::Owner(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    InstalledPackageBytesMismatch
            ))
        );

        let mut reconstructed_store = PackageService::new();
        let parsed = PackageView::parse(&package_a).unwrap();
        reconstructed_store
            .install_with_trusted_key(
                &parsed,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                &TRUSTED_SIGNING_PUBLIC_KEY,
            )
            .unwrap();
        let reconstructed_owner =
            PackageServiceSessionOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), reconstructed_store);
        assert_eq!(
            bind_installed_service_package(&descriptor_a, &package_a, &reconstructed_owner).err(),
            Some(PackageBindingError::Owner(
                nagi_init_package_provider::package_trust::PackageServiceSessionOwnerError::
                    PackageDigestUnavailable
            ))
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

        let runtime_slot = lifecycle.runtime_slot_binding();
        assert_eq!(runtime_slot.service_id(), descriptor.service_id());
        assert_eq!(runtime_slot.catalog_generation(), descriptor.generation());
        assert_eq!(runtime_slot.transition(), descriptor.transition());
        assert_ne!(runtime_slot.c3_launch_generation(), descriptor.generation());
        assert_eq!(
            runtime_slot.package_generation(),
            binding.package_generation()
        );
        assert_eq!(
            runtime_slot.executable_sha256_bytes(),
            binding.executable_sha256_bytes()
        );
        assert_eq!(
            validate_lifecycle_binding(&descriptor, &binding, &lifecycle),
            Ok(())
        );
    }

    #[test]
    fn lifecycle_binding_rejects_initial_replace_transition_mismatch_both_ways() {
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let initial = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        let mut replace_json: Value =
            serde_json::from_str(&descriptor_with_digest(&digest)).unwrap();
        replace_json["lifecycle"] = json!({ "mode": "replace", "previous_generation": 8 });
        let replace = validate_descriptor_json(&replace_json.to_string()).unwrap();
        let binding =
            bind_installed_service_package(&initial, &package_bytes, &package_owner).unwrap();
        let (mut _catalog, _preflight, mut owner, _identity) = launch_ports();

        let replace_reservation = TestLifecycle {
            runtime_slot_binding: SystemServiceRuntimeSlotBinding::for_c3_reservation(
                &replace.launch_metadata(),
                &binding,
                NonZeroU64::new(33).unwrap(),
            ),
            process_identity: None,
        };
        assert_eq!(
            validate_lifecycle_binding(&initial, &binding, &replace_reservation),
            Err(ProofValidationError::WrongServiceBinding)
        );

        let initial_reservation = owner
            .reserve_launch(&initial.launch_metadata(), &binding)
            .unwrap();
        assert_eq!(
            validate_lifecycle_binding(&replace, &binding, &initial_reservation),
            Err(ProofValidationError::WrongServiceBinding)
        );
    }

    #[test]
    fn recovery_fence_covers_the_canonical_service_slot_across_catalog_generations() {
        let _guard = recovery_state_test_guard();
        let first = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        let mut replacement = valid_descriptor();
        replacement["generation"] = json!(10);
        replacement["lifecycle"] = json!({ "mode": "replace", "previous_generation": 9 });
        let replacement = validate_descriptor_json(&replacement.to_string()).unwrap();

        block_recovery(&first);
        assert!(is_recovery_blocked(&replacement));
        clear_recovery_block(&first);
        assert!(!is_recovery_blocked(&replacement));
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

        let launch_scope_address =
            admission.scope() as *const dyn SystemServiceRecoveryAdmissionScope as *const ();
        let runtime_storage = Box::new(admission);
        let admission = *runtime_storage;
        assert_eq!(
            admission.scope() as *const dyn SystemServiceRecoveryAdmissionScope as *const (),
            launch_scope_address
        );
        let cloned_admission = admission.clone();
        let mut c3_owner = launch_ports().2;
        let mut gate = SystemServiceReconciliationGate {
            owner: &mut c3_owner,
            admission: &admission,
            used: false,
            successful: false,
            protocol_violation: false,
            receipt: None,
        };
        assert_eq!(
            gate.with_reconciliation_gate(
                cloned_admission.scope(),
                test_claim(cloned_admission.scope(), 1),
                || (),
            ),
            Err(RecoveryClientError::Rejected)
        );
        assert!(!gate.was_used());
        assert!(gate.protocol_violation());

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

    #[test]
    fn a_b_a_trait_scope_is_rejected_before_c3_gate_observes_it() {
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
        let admission = SystemServiceRecoveryAdmission::for_reconciliation(
            catalog.authenticate(&descriptor).unwrap(),
            &binding,
            NonZeroU64::new(33).unwrap(),
            NonZeroU64::new(3).unwrap(),
            1,
            *binding.signed_package_sha256(),
        )
        .unwrap();
        let scope = AlternatingScope {
            admission: admission.clone(),
            binding_digest_reads: AtomicUsize::new(0),
        };
        let original_digest = *admission.scope().binding_digest();

        // The alternate trait view visibly changes A -> B -> A on reread.
        assert_eq!(*scope.binding_digest(), original_digest);
        assert_eq!(*scope.binding_digest(), ALTERNATE_SCOPE_DIGEST);
        assert_eq!(*scope.binding_digest(), original_digest);

        let mut c3_owner = launch_ports().2;
        let mut gate = SystemServiceReconciliationGate {
            owner: &mut c3_owner,
            admission: &admission,
            used: false,
            successful: false,
            protocol_violation: false,
            receipt: None,
        };
        let mut callback_called = false;
        assert_eq!(
            gate.with_reconciliation_gate(&scope, test_claim(admission.scope(), 1), || {
                callback_called = true
            },),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(scope.binding_digest_reads.load(Ordering::SeqCst), 3);
        assert!(!callback_called);
        assert!(!gate.was_used());
        assert!(gate.protocol_violation());
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

    struct TestC3RecoveryReceipt {
        c1_operation_id: NonZeroU64,
        service_id: String,
        catalog_generation: u64,
        c3_launch_generation: u64,
        binding_digest: [u8; 32],
        state: C3RecoverySlotState,
    }

    impl C3DurableRecoverySlotReceipt for TestC3RecoveryReceipt {
        fn c1_operation_id(&self) -> Option<NonZeroU64> {
            Some(self.c1_operation_id)
        }

        fn service_id(&self) -> &str {
            &self.service_id
        }

        fn catalog_generation(&self) -> u64 {
            self.catalog_generation
        }

        fn c3_launch_generation(&self) -> u64 {
            self.c3_launch_generation
        }

        fn scope_binding_digest(&self) -> &[u8; 32] {
            &self.binding_digest
        }

        fn state(&self) -> C3RecoverySlotState {
            self.state
        }
    }

    struct TestLifecycle {
        runtime_slot_binding: SystemServiceRuntimeSlotBinding,
        process_identity: Option<SpawnedProcessIdentity>,
    }

    impl SystemServiceLaunchLifecycle for TestLifecycle {
        fn runtime_slot_binding(&self) -> &SystemServiceRuntimeSlotBinding {
            &self.runtime_slot_binding
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
        type RecoveryReceipt = TestC3RecoveryReceipt;

        fn reserve_launch(
            &mut self,
            metadata: &SystemServiceLaunchMetadata,
            package: &VerifiedServicePackageBinding<'_>,
        ) -> Result<Self::Lifecycle, ReservationError> {
            Ok(TestLifecycle {
                runtime_slot_binding: SystemServiceRuntimeSlotBinding::for_c3_reservation(
                    metadata,
                    package,
                    NonZeroU64::new(33).unwrap(),
                ),
                process_identity: None,
            })
        }

        fn launch_and_authenticate(
            &mut self,
            admission: &SystemServiceRecoveryAdmission,
            package: &VerifiedServicePackageBinding<'_>,
            _lifecycle: &mut Self::Lifecycle,
        ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let metadata = admission.descriptor().launch_metadata();
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
            _admission: &SystemServiceRecoveryAdmission,
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
            admission: &SystemServiceRecoveryAdmission,
            claim: C1RecoveryOperationCorrelation,
            reconcile: impl FnOnce() -> R,
        ) -> Result<(R, Self::RecoveryReceipt), LaunchPortError> {
            let scope = admission.scope();
            Ok((
                reconcile(),
                TestC3RecoveryReceipt {
                    c1_operation_id: claim.operation_id(),
                    service_id: scope.service_id().to_owned(),
                    catalog_generation: scope.catalog_generation().get(),
                    c3_launch_generation: scope.c3_launch_generation().get(),
                    binding_digest: *scope.binding_digest(),
                    state: C3RecoverySlotState::ClearedAndAbsent,
                },
            ))
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

    struct TestC1TerminalReceipt {
        operation_id: u64,
        binding_digest: [u8; 32],
        outcome_digest: [u8; 32],
    }

    impl C1RecoveryTerminalReceipt for TestC1TerminalReceipt {
        fn operation_id(&self) -> u64 {
            self.operation_id
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding_digest
        }

        fn outcome_digest(&self) -> [u8; 32] {
            self.outcome_digest
        }
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
    }

    impl C1RecoveryReceiptClient for Identity {
        type Receipt = TestC1TerminalReceipt;

        fn reconcile_terminal<G: C3RecoverySlotGate>(
            &mut self,
            scope: &dyn SystemServiceRecoveryAdmissionScope,
            gate: &mut G,
        ) -> Result<Self::Receipt, RecoveryClientError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            gate.with_reconciliation_gate(scope, test_claim(scope, 1), || ())?;
            Ok(TestC1TerminalReceipt {
                operation_id: 0,
                binding_digest: *scope.binding_digest(),
                outcome_digest: [0x31; 32],
            })
        }

        fn consume_terminal_receipt(
            &mut self,
            _scope: &dyn SystemServiceRecoveryAdmissionScope,
            _receipt: Self::Receipt,
        ) -> Result<(), RecoveryClientError> {
            Err(RecoveryClientError::Rejected)
        }
    }

    struct WrongOperationIdentity;

    impl C1SystemServiceRecoveryClient for WrongOperationIdentity {
        type Runtime = TestRuntime;

        fn recover_system_service(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            _package: &VerifiedServicePackageBinding<'_>,
            _gate: &mut SystemServiceRecoveryGate<'_>,
        ) -> Result<Self::Runtime, C1RecoveryFailure> {
            unreachable!("reconciliation-only test client")
        }

        fn quarantine_recovery(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
        ) -> Result<(), LaunchPortError> {
            Ok(())
        }
    }

    impl C1RecoveryReceiptClient for WrongOperationIdentity {
        type Receipt = TestC1TerminalReceipt;

        fn reconcile_terminal<G: C3RecoverySlotGate>(
            &mut self,
            scope: &dyn SystemServiceRecoveryAdmissionScope,
            gate: &mut G,
        ) -> Result<Self::Receipt, RecoveryClientError> {
            gate.with_reconciliation_gate(scope, test_claim(scope, 2), || ())?;
            Ok(TestC1TerminalReceipt {
                operation_id: 2,
                binding_digest: *scope.binding_digest(),
                outcome_digest: [0x52; 32],
            })
        }

        fn consume_terminal_receipt(
            &mut self,
            _scope: &dyn SystemServiceRecoveryAdmissionScope,
            receipt: Self::Receipt,
        ) -> Result<(), RecoveryClientError> {
            if receipt.operation_id == 1 {
                Ok(())
            } else {
                Err(RecoveryClientError::Rejected)
            }
        }
    }

    struct ConsumingIdentity {
        consumed: AtomicUsize,
    }

    struct PostClosureUnknownIdentity {
        fenced: bool,
    }

    impl C1SystemServiceRecoveryClient for ConsumingIdentity {
        type Runtime = TestRuntime;

        fn recover_system_service(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            _package: &VerifiedServicePackageBinding<'_>,
            _gate: &mut SystemServiceRecoveryGate<'_>,
        ) -> Result<Self::Runtime, C1RecoveryFailure> {
            unreachable!("reconciliation-only test client")
        }

        fn quarantine_recovery(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
        ) -> Result<(), LaunchPortError> {
            Ok(())
        }
    }

    impl C1RecoveryReceiptClient for ConsumingIdentity {
        type Receipt = TestC1TerminalReceipt;

        fn reconcile_terminal<G: C3RecoverySlotGate>(
            &mut self,
            scope: &dyn SystemServiceRecoveryAdmissionScope,
            gate: &mut G,
        ) -> Result<Self::Receipt, RecoveryClientError> {
            gate.with_reconciliation_gate(scope, test_claim(scope, 1), || ())?;
            Ok(TestC1TerminalReceipt {
                operation_id: 1,
                binding_digest: *scope.binding_digest(),
                outcome_digest: [0x61; 32],
            })
        }

        fn consume_terminal_receipt(
            &mut self,
            scope: &dyn SystemServiceRecoveryAdmissionScope,
            receipt: Self::Receipt,
        ) -> Result<(), RecoveryClientError> {
            if receipt.operation_id == 1
                && receipt.binding_digest == *scope.binding_digest()
                && receipt.outcome_digest == [0x61; 32]
            {
                self.consumed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            } else {
                Err(RecoveryClientError::Rejected)
            }
        }
    }

    impl C1SystemServiceRecoveryClient for PostClosureUnknownIdentity {
        type Runtime = TestRuntime;

        fn recover_system_service(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
            _package: &VerifiedServicePackageBinding<'_>,
            _gate: &mut SystemServiceRecoveryGate<'_>,
        ) -> Result<Self::Runtime, C1RecoveryFailure> {
            unreachable!("reconciliation-only test client")
        }

        fn quarantine_recovery(
            &mut self,
            _admission: &SystemServiceRecoveryAdmission,
        ) -> Result<(), LaunchPortError> {
            self.fenced = true;
            Ok(())
        }
    }

    impl C1RecoveryReceiptClient for PostClosureUnknownIdentity {
        type Receipt = TestC1TerminalReceipt;

        fn reconcile_terminal<G: C3RecoverySlotGate>(
            &mut self,
            scope: &dyn SystemServiceRecoveryAdmissionScope,
            gate: &mut G,
        ) -> Result<Self::Receipt, RecoveryClientError> {
            gate.with_reconciliation_gate(scope, test_claim(scope, 1), || ())?;
            // Model a C1 terminal write whose durable result cannot be read
            // back. The C1-side fence remains closed and no receipt escapes.
            self.fenced = true;
            Err(RecoveryClientError::OutcomeUnknown)
        }

        fn consume_terminal_receipt(
            &mut self,
            _scope: &dyn SystemServiceRecoveryAdmissionScope,
            _receipt: Self::Receipt,
        ) -> Result<(), RecoveryClientError> {
            panic!("unknown terminal outcome must not produce a receipt")
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
    fn boot_launch_uses_the_installed_package_from_the_reopened_store() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let bytes = fixture_package(executable);
        let mut persistence = TestPackagePersistence::new();
        let root = File::open(&persistence.path).unwrap();
        let owner_uid = root.metadata().unwrap().uid();
        let mut blobs =
            LinuxInstalledPackagePersistence::from_trusted_directory(root, owner_uid).unwrap();
        let policy = InstallPolicy {
            developer_mode: false,
            require_signature: true,
        };
        PackageServiceSessionOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), PackageService::new())
            .install_with_persistent_storage(&bytes, policy, &mut blobs, persistence.index_mut())
            .unwrap();
        drop(bytes);
        drop(blobs);

        let blobs = LinuxInstalledPackagePersistence::from_trusted_directory(
            File::open(&persistence.path).unwrap(),
            owner_uid,
        )
        .unwrap();
        let package_owner = PackageServiceSessionOwner::restore_from_persistent_storage(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            policy,
            &blobs,
            persistence.index(),
        )
        .unwrap();
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        let digest = digest_hex(executable);
        preflight.report = report_for_digest(&digest);
        let result = launch_identity_service_from_store(
            &descriptor_with_digest(&digest),
            &blobs,
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ProcessOwner(LaunchPortError::Unavailable))
        );
        assert_eq!(owner.launched_executable.as_deref(), Some(&executable[..]));
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

        // A reconstructed owner starts unquarantined even though its
        // inventory and digest index can be restored. The C1 callback and C3
        // callback success above are still insufficient to clear A's fence:
        // no operation-bound terminal receipt is connected to this port.
        let mut restored_index = TestPackagePersistence::new();
        let mut index_seed_owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        index_seed_owner
            .install_with_digest_index(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
                restored_index.index_mut(),
            )
            .unwrap();
        let restored_inventory = PackageService::restore_with_trusted_key(
            &[(&package_bytes, 1)],
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
            &TRUSTED_SIGNING_PUBLIC_KEY,
        )
        .unwrap();
        let mut reconstructed_owner =
            PackageServiceSessionOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), restored_inventory);
        reconstructed_owner
            .restore_digest_index(restored_index.index())
            .unwrap();
        assert!(!reconstructed_owner.is_quarantined());
        let mut wrong_operation_identity = WrongOperationIdentity;
        assert_eq!(
            reconcile_identity_service(
                &admission,
                &mut reconstructed_owner,
                &mut owner,
                &mut wrong_operation_identity,
            ),
            Err(LaunchError::RecoveryReceiptConsumptionFailed(
                LaunchPortError::Rejected,
                admission.clone(),
            ))
        );
        assert!(is_recovery_blocked(&descriptor));
        assert!(!reconstructed_owner.is_quarantined());
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn reconciliation_consumes_exact_terminal_and_c3_receipts_before_clearing_fence() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        block_recovery(&descriptor);

        let mut package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut catalog = Catalog {
            error: None,
            calls: AtomicUsize::new(0),
        };
        let admission = SystemServiceRecoveryAdmission::for_reconciliation(
            catalog.authenticate(&descriptor).unwrap(),
            &binding,
            NonZeroU64::new(33).unwrap(),
            NonZeroU64::new(3).unwrap(),
            1,
            *binding.signed_package_sha256(),
        )
        .unwrap();
        let mut owner = launch_ports().2;
        let mut identity = ConsumingIdentity {
            consumed: AtomicUsize::new(0),
        };

        assert_eq!(
            reconcile_identity_service(&admission, &mut package_owner, &mut owner, &mut identity),
            Ok(())
        );
        assert_eq!(identity.consumed.load(Ordering::SeqCst), 1);
        assert!(!is_recovery_blocked(&descriptor));
        assert!(package_owner
            .capture_for_system_service(&package_bytes, b"org.nagi.fixture")
            .is_ok());
        clear_recovery_block(&descriptor);
    }

    #[test]
    fn post_closure_unknown_keeps_c1_and_a_recovery_fences() {
        let _guard = recovery_state_test_guard();
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        clear_recovery_block(&descriptor);
        block_recovery(&descriptor);

        let mut package_owner = installed_fixture_owner(&package_bytes);
        let binding =
            bind_installed_service_package(&descriptor, &package_bytes, &package_owner).unwrap();
        let mut catalog = Catalog {
            error: None,
            calls: AtomicUsize::new(0),
        };
        let admission = SystemServiceRecoveryAdmission::for_reconciliation(
            catalog.authenticate(&descriptor).unwrap(),
            &binding,
            NonZeroU64::new(33).unwrap(),
            NonZeroU64::new(3).unwrap(),
            1,
            *binding.signed_package_sha256(),
        )
        .unwrap();
        let mut owner = launch_ports().2;
        let mut identity = PostClosureUnknownIdentity { fenced: false };

        assert_eq!(
            reconcile_identity_service(&admission, &mut package_owner, &mut owner, &mut identity),
            Err(LaunchError::RecoveryReconciliation(
                LaunchPortError::OutcomeUnknown
            ))
        );
        assert!(identity.fenced);
        assert!(is_recovery_blocked(&descriptor));
        assert!(!package_owner.is_quarantined());
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
        let admission = SystemServiceRecoveryAdmission::new(catalog, &lifecycle);
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
