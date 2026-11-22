//! Synthetic contract fixture for A's previous-launch adapter boundary.
//!
//! C3 v5 now persists a full prior scope, stable launch ID, PID, C1
//! correlation, and exact normal-exit terminal evidence. It does not
//! serialize the private process-identity nonce or the old in-memory
//! `SystemServiceLaunchRecord` handle. This fixture remains synthetic and does
//! not prove production C3 journal readback or owner-gate behavior.

use nagi_identity_session::C1PreviousLaunchTerminalGate;
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_system_service_recovery_contract::{
    canonical_recovery_binding_digest_from_scope, C1RecoveryOperationCorrelation,
    C3DurableRecoverySlotReceipt, C3RecoverySlotState, RecoveryClientError,
    RecoveryLifecycleTransition, SystemServiceRecoveryAdmissionScope,
};
use std::num::NonZeroU64;

#[derive(Clone)]
struct Scope {
    service_id: &'static str,
    principal_id: &'static str,
    caller_context_id: &'static str,
    mapping_generation: NonZeroU64,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    binding_digest: [u8; 32],
}

impl Scope {
    fn new(c3_launch_generation: u64) -> Self {
        let mut scope = Self {
            service_id: "service:fixture.identity",
            principal_id: "service:fixture.identity",
            caller_context_id: "caller_fixture_identity",
            mapping_generation: NonZeroU64::new(3).unwrap(),
            catalog_generation: NonZeroU64::new(9).unwrap(),
            c3_launch_generation: NonZeroU64::new(c3_launch_generation).unwrap(),
            binding_digest: [0; 32],
        };
        scope.binding_digest = canonical_recovery_binding_digest_from_scope(&scope);
        scope
    }

    fn refresh_digest(&mut self) {
        self.binding_digest = canonical_recovery_binding_digest_from_scope(self);
    }
}

impl SystemServiceRecoveryAdmissionScope for Scope {
    fn service_id(&self) -> &str {
        self.service_id
    }

    fn principal_id(&self) -> &str {
        self.principal_id
    }

    fn caller_context_id(&self) -> &str {
        self.caller_context_id
    }

    fn mapping_generation(&self) -> NonZeroU64 {
        self.mapping_generation
    }

    fn role(&self) -> &str {
        "system_service"
    }

    fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog_generation
    }

    fn transition(&self) -> RecoveryLifecycleTransition {
        RecoveryLifecycleTransition::Initial
    }

    fn package_id(&self) -> &str {
        "org.nagi.fixture"
    }

    fn package_app_id(&self) -> u64 {
        0x1122
    }

    fn canonical_entry(&self) -> &str {
        "fixture.elf"
    }

    fn signed_package_sha256(&self) -> &[u8; 32] {
        &[0x11; 32]
    }

    fn executable_sha256(&self) -> &[u8; 32] {
        &[0x22; 32]
    }

    fn package_generation(&self) -> u64 {
        17
    }

    fn c3_launch_generation(&self) -> NonZeroU64 {
        self.c3_launch_generation
    }

    fn credentials(&self) -> (u32, u32) {
        (1001, 1002)
    }

    fn binding_digest(&self) -> &[u8; 32] {
        &self.binding_digest
    }
}

#[derive(Clone)]
struct SyntheticOfficialPriorRecord {
    service_id: &'static str,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    process_identity: SpawnedProcessIdentity,
    owner_issued: bool,
}

impl SyntheticOfficialPriorRecord {
    fn for_scope(scope: &Scope) -> Self {
        Self {
            service_id: scope.service_id,
            catalog_generation: scope.catalog_generation,
            c3_launch_generation: scope.c3_launch_generation,
            process_identity: nagi_init_spawn_contract::test_identity(
                u32::try_from(1000 + scope.c3_launch_generation.get()).unwrap(),
            ),
            owner_issued: true,
        }
    }
}

#[derive(Clone)]
struct SyntheticTerminalReceipt {
    c1_operation_id: Option<NonZeroU64>,
    service_id: &'static str,
    catalog_generation: u64,
    c3_launch_generation: u64,
    binding_digest: [u8; 32],
    state: C3RecoverySlotState,
}

impl SyntheticTerminalReceipt {
    fn for_scope(scope: &Scope, operation_id: NonZeroU64) -> Self {
        Self {
            c1_operation_id: Some(operation_id),
            service_id: scope.service_id,
            catalog_generation: scope.catalog_generation.get(),
            c3_launch_generation: scope.c3_launch_generation.get(),
            binding_digest: scope.binding_digest,
            state: C3RecoverySlotState::ClearedAndAbsent,
        }
    }
}

impl C3DurableRecoverySlotReceipt for SyntheticTerminalReceipt {
    fn c1_operation_id(&self) -> Option<NonZeroU64> {
        self.c1_operation_id
    }

    fn service_id(&self) -> &str {
        self.service_id
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct SyntheticRecoveryScopeSnapshot {
    service_id: String,
    principal_id: String,
    caller_context_id: String,
    mapping_generation: NonZeroU64,
    role: String,
    catalog_generation: NonZeroU64,
    transition: RecoveryLifecycleTransition,
    package_id: String,
    package_app_id: u64,
    canonical_entry: String,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
    package_generation: u64,
    c3_launch_generation: NonZeroU64,
    credentials: (u32, u32),
    binding_digest: [u8; 32],
}

impl SyntheticRecoveryScopeSnapshot {
    fn capture(scope: &dyn SystemServiceRecoveryAdmissionScope) -> Self {
        Self {
            service_id: scope.service_id().to_owned(),
            principal_id: scope.principal_id().to_owned(),
            caller_context_id: scope.caller_context_id().to_owned(),
            mapping_generation: scope.mapping_generation(),
            role: scope.role().to_owned(),
            catalog_generation: scope.catalog_generation(),
            transition: scope.transition(),
            package_id: scope.package_id().to_owned(),
            package_app_id: scope.package_app_id(),
            canonical_entry: scope.canonical_entry().to_owned(),
            signed_package_sha256: *scope.signed_package_sha256(),
            executable_sha256: *scope.executable_sha256(),
            package_generation: scope.package_generation(),
            c3_launch_generation: scope.c3_launch_generation(),
            credentials: scope.credentials(),
            binding_digest: *scope.binding_digest(),
        }
    }
}

#[derive(Clone, Copy)]
enum SyntheticChildOutcome {
    Exited { code: i32 },
    Signaled,
    Unknown,
}

#[derive(Clone)]
struct SyntheticPriorLaunchEvidence {
    record: Option<SyntheticOfficialPriorRecord>,
    journal_scope: Option<SyntheticRecoveryScopeSnapshot>,
    journal_process_identity: Option<SpawnedProcessIdentity>,
    receipt: Option<SyntheticTerminalReceipt>,
    child_outcome: SyntheticChildOutcome,
}

struct SyntheticC3PriorLaunchGate {
    evidence: SyntheticPriorLaunchEvidence,
    callbacks: usize,
}

fn exact_evidence_matches(
    current: &dyn SystemServiceRecoveryAdmissionScope,
    previous: &dyn SystemServiceRecoveryAdmissionScope,
    claim: C1RecoveryOperationCorrelation,
    evidence: &SyntheticPriorLaunchEvidence,
) -> bool {
    if canonical_recovery_binding_digest_from_scope(current) != *current.binding_digest()
        || canonical_recovery_binding_digest_from_scope(previous) != *previous.binding_digest()
        || current.role() != "system_service"
        || previous.role() != "system_service"
        || current.service_id() != previous.service_id()
        || current.principal_id() != previous.principal_id()
        || current.caller_context_id() != previous.caller_context_id()
        || current.mapping_generation() != previous.mapping_generation()
        || current.catalog_generation() != previous.catalog_generation()
        || previous.c3_launch_generation() >= current.c3_launch_generation()
        || previous.binding_digest() != claim.binding_digest()
    {
        return false;
    }

    let Some(record) = evidence.record.as_ref() else {
        return false;
    };
    let Some(journal_scope) = evidence.journal_scope.as_ref() else {
        return false;
    };
    if journal_scope != &SyntheticRecoveryScopeSnapshot::capture(previous) {
        return false;
    }
    let Some(journal_process_identity) = evidence.journal_process_identity.as_ref() else {
        return false;
    };
    if !record.owner_issued
        || !record
            .process_identity
            .same_instance(journal_process_identity)
        || record.service_id != previous.service_id()
        || record.catalog_generation != previous.catalog_generation()
        || record.c3_launch_generation != previous.c3_launch_generation()
    {
        return false;
    }

    let Some(receipt) = evidence.receipt.as_ref() else {
        return false;
    };
    if receipt.c1_operation_id() != Some(claim.operation_id())
        || receipt.service_id() != previous.service_id()
        || receipt.catalog_generation() != previous.catalog_generation().get()
        || receipt.c3_launch_generation() != previous.c3_launch_generation().get()
        || receipt.scope_binding_digest() != previous.binding_digest()
        || receipt.state() != C3RecoverySlotState::ClearedAndAbsent
    {
        return false;
    }

    matches!(
        evidence.child_outcome,
        SyntheticChildOutcome::Exited { code: 0 }
    )
}

impl C1PreviousLaunchTerminalGate for SyntheticC3PriorLaunchGate {
    type TerminalReceipt = SyntheticTerminalReceipt;

    fn with_previous_launch_reconciliation_gate(
        &mut self,
        current_scope: &dyn SystemServiceRecoveryAdmissionScope,
        previous_scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        revalidate_scopes: impl FnOnce() -> Result<(), RecoveryClientError>,
    ) -> Result<Self::TerminalReceipt, RecoveryClientError> {
        if !exact_evidence_matches(current_scope, previous_scope, claim, &self.evidence) {
            return Err(RecoveryClientError::OutcomeUnknown);
        }
        self.callbacks += 1;
        revalidate_scopes()?;
        Ok(self
            .evidence
            .receipt
            .as_ref()
            .expect("exact evidence includes a terminal receipt")
            .clone())
    }
}

fn evidence_for(scope: &Scope, operation_id: NonZeroU64) -> SyntheticPriorLaunchEvidence {
    let record = SyntheticOfficialPriorRecord::for_scope(scope);
    SyntheticPriorLaunchEvidence {
        journal_process_identity: Some(record.process_identity.clone()),
        journal_scope: Some(SyntheticRecoveryScopeSnapshot::capture(scope)),
        record: Some(record),
        receipt: Some(SyntheticTerminalReceipt::for_scope(scope, operation_id)),
        child_outcome: SyntheticChildOutcome::Exited { code: 0 },
    }
}

fn assert_unknown_without_callback(
    current: &dyn SystemServiceRecoveryAdmissionScope,
    previous: &dyn SystemServiceRecoveryAdmissionScope,
    claim: C1RecoveryOperationCorrelation,
    evidence: SyntheticPriorLaunchEvidence,
) {
    let mut gate = SyntheticC3PriorLaunchGate {
        evidence,
        callbacks: 0,
    };
    let mut callback_ran = false;
    let result = gate.with_previous_launch_reconciliation_gate(current, previous, claim, || {
        callback_ran = true;
        Ok(())
    });
    assert!(matches!(result, Err(RecoveryClientError::OutcomeUnknown)));
    assert_eq!(gate.callbacks, 0);
    assert!(!callback_ran);
}

#[test]
fn previous_launch_adapter_requires_complete_exact_terminal_evidence() {
    let current = Scope::new(22);
    let previous = Scope::new(21);
    let operation_id = NonZeroU64::new(73).unwrap();
    let claim = C1RecoveryOperationCorrelation::new(operation_id, previous.binding_digest);
    let complete = evidence_for(&previous, operation_id);

    let mut gate = SyntheticC3PriorLaunchGate {
        evidence: complete.clone(),
        callbacks: 0,
    };
    let mut callback_ran = false;
    let result = gate
        .with_previous_launch_reconciliation_gate(&current, &previous, claim, || {
            callback_ran = true;
            Ok(())
        })
        .unwrap();
    assert_eq!(result.c1_operation_id(), Some(operation_id));
    assert_eq!(result.service_id(), previous.service_id);
    assert_eq!(
        result.catalog_generation(),
        previous.catalog_generation.get()
    );
    assert_eq!(
        result.c3_launch_generation(),
        previous.c3_launch_generation.get()
    );
    assert_eq!(result.scope_binding_digest(), &previous.binding_digest);
    assert_eq!(result.state(), C3RecoverySlotState::ClearedAndAbsent);
    assert_eq!(gate.callbacks, 1);
    assert!(callback_ran);

    let mut wrong_operation = complete.clone();
    wrong_operation.receipt.as_mut().unwrap().c1_operation_id = Some(NonZeroU64::new(74).unwrap());
    assert_unknown_without_callback(&current, &previous, claim, wrong_operation);

    let mut wrong_digest = complete.clone();
    wrong_digest.receipt.as_mut().unwrap().binding_digest = [0x99; 32];
    assert_unknown_without_callback(&current, &previous, claim, wrong_digest);

    let mut wrong_service = complete.clone();
    wrong_service.receipt.as_mut().unwrap().service_id = "service:other";
    assert_unknown_without_callback(&current, &previous, claim, wrong_service);

    let mut wrong_catalog = complete.clone();
    wrong_catalog.receipt.as_mut().unwrap().catalog_generation = 10;
    assert_unknown_without_callback(&current, &previous, claim, wrong_catalog);

    let mut wrong_generation = complete.clone();
    wrong_generation
        .receipt
        .as_mut()
        .unwrap()
        .c3_launch_generation = 22;
    assert_unknown_without_callback(&current, &previous, claim, wrong_generation);

    let mut nonterminal = complete.clone();
    nonterminal.receipt.as_mut().unwrap().state = C3RecoverySlotState::Live;
    assert_unknown_without_callback(&current, &previous, claim, nonterminal);

    let mut no_record = complete.clone();
    no_record.record = None;
    assert_unknown_without_callback(&current, &previous, claim, no_record);

    let mut no_journal_scope = complete.clone();
    no_journal_scope.journal_scope = None;
    assert_unknown_without_callback(&current, &previous, claim, no_journal_scope);

    let mut mismatched_journal_scope = complete.clone();
    mismatched_journal_scope
        .journal_scope
        .as_mut()
        .unwrap()
        .c3_launch_generation = NonZeroU64::new(20).unwrap();
    assert_unknown_without_callback(&current, &previous, claim, mismatched_journal_scope);

    let mut untrusted_record = complete.clone();
    untrusted_record.record.as_mut().unwrap().owner_issued = false;
    assert_unknown_without_callback(&current, &previous, claim, untrusted_record);

    let mut missing_journal_process_identity = complete.clone();
    missing_journal_process_identity.journal_process_identity = None;
    assert_unknown_without_callback(&current, &previous, claim, missing_journal_process_identity);

    let mut mismatched_journal_process_identity = complete.clone();
    let record_process_identity = &complete.record.as_ref().unwrap().process_identity;
    mismatched_journal_process_identity.journal_process_identity = Some(
        nagi_init_spawn_contract::test_identity(record_process_identity.process_id()),
    );
    assert_unknown_without_callback(
        &current,
        &previous,
        claim,
        mismatched_journal_process_identity,
    );

    let mut wrong_record_service = complete.clone();
    wrong_record_service.record.as_mut().unwrap().service_id = "service:other";
    assert_unknown_without_callback(&current, &previous, claim, wrong_record_service);

    let mut wrong_record_catalog = complete.clone();
    wrong_record_catalog
        .record
        .as_mut()
        .unwrap()
        .catalog_generation = NonZeroU64::new(10).unwrap();
    assert_unknown_without_callback(&current, &previous, claim, wrong_record_catalog);

    let mut wrong_record_generation = complete.clone();
    wrong_record_generation
        .record
        .as_mut()
        .unwrap()
        .c3_launch_generation = NonZeroU64::new(20).unwrap();
    assert_unknown_without_callback(&current, &previous, claim, wrong_record_generation);

    let mut no_receipt = complete.clone();
    no_receipt.receipt = None;
    assert_unknown_without_callback(&current, &previous, claim, no_receipt);

    let mut unknown_exit = complete.clone();
    unknown_exit.child_outcome = SyntheticChildOutcome::Unknown;
    assert_unknown_without_callback(&current, &previous, claim, unknown_exit);

    let mut unsuccessful_exit = complete.clone();
    unsuccessful_exit.child_outcome = SyntheticChildOutcome::Exited { code: 1 };
    assert_unknown_without_callback(&current, &previous, claim, unsuccessful_exit);

    let mut signaled_exit = complete.clone();
    signaled_exit.child_outcome = SyntheticChildOutcome::Signaled;
    assert_unknown_without_callback(&current, &previous, claim, signaled_exit);

    let same_generation = Scope::new(22);
    let same_generation_claim =
        C1RecoveryOperationCorrelation::new(operation_id, same_generation.binding_digest);
    assert_unknown_without_callback(
        &current,
        &same_generation,
        same_generation_claim,
        evidence_for(&same_generation, operation_id),
    );

    let mut wrong_principal = Scope::new(21);
    wrong_principal.principal_id = "service:other.identity";
    wrong_principal.refresh_digest();
    let wrong_principal_claim =
        C1RecoveryOperationCorrelation::new(operation_id, wrong_principal.binding_digest);
    assert_unknown_without_callback(
        &current,
        &wrong_principal,
        wrong_principal_claim,
        evidence_for(&wrong_principal, operation_id),
    );

    let mut wrong_mapping = Scope::new(21);
    wrong_mapping.mapping_generation = NonZeroU64::new(4).unwrap();
    wrong_mapping.refresh_digest();
    let wrong_mapping_claim =
        C1RecoveryOperationCorrelation::new(operation_id, wrong_mapping.binding_digest);
    assert_unknown_without_callback(
        &current,
        &wrong_mapping,
        wrong_mapping_claim,
        evidence_for(&wrong_mapping, operation_id),
    );
}
