#![no_std]

extern crate alloc;

use core::num::NonZeroU64;
use sha2::{Digest, Sha256};

const RECOVERY_SCOPE_DOMAIN: &[u8] = b"NAGI-SYSTEM-SERVICE-RECOVERY-SCOPE\0\x01";

/// The exact catalog lifecycle transition bound into a recovery operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryLifecycleTransition {
    Initial,
    Replace { previous_generation: NonZeroU64 },
}

/// Inputs A copies from one authenticated catalog mapping, verified package,
/// and exact C3 reservation. This value is consistency metadata, not authority.
pub struct SystemServiceRecoveryScopeInput<'a> {
    pub service_id: &'a str,
    pub principal_id: &'a str,
    pub caller_context_id: &'a str,
    pub mapping_generation: NonZeroU64,
    pub role: &'a str,
    pub catalog_generation: NonZeroU64,
    pub transition: RecoveryLifecycleTransition,
    pub package_id: &'a str,
    pub package_app_id: u64,
    pub canonical_entry: &'a str,
    pub signed_package_sha256: &'a [u8; 32],
    pub executable_sha256: &'a [u8; 32],
    pub package_generation: u64,
    pub c3_launch_generation: NonZeroU64,
    pub uid: u32,
    pub gid: u32,
}

/// A-owned private admission type implements this shared view. C1/C3 receive
/// the same admission object through a trait reference; they must never create
/// a parallel token from public metadata.
pub trait SystemServiceRecoveryAdmissionScope {
    fn service_id(&self) -> &str;
    fn principal_id(&self) -> &str;
    fn caller_context_id(&self) -> &str;
    fn mapping_generation(&self) -> NonZeroU64;
    fn role(&self) -> &str;
    fn catalog_generation(&self) -> NonZeroU64;
    fn transition(&self) -> RecoveryLifecycleTransition;
    fn package_id(&self) -> &str;
    fn package_app_id(&self) -> u64;
    fn canonical_entry(&self) -> &str;
    fn signed_package_sha256(&self) -> &[u8; 32];
    fn executable_sha256(&self) -> &[u8; 32];
    fn package_generation(&self) -> u64;
    fn c3_launch_generation(&self) -> NonZeroU64;
    fn credentials(&self) -> (u32, u32);
    fn binding_digest(&self) -> &[u8; 32];
}

/// Canonical digest over all A-owned admission fields. This hashes a borrowed
/// construction view; the input itself is not the token passed to C1/C3.
pub fn canonical_recovery_binding_digest(input: &SystemServiceRecoveryScopeInput<'_>) -> [u8; 32] {
    let mut canonical = Sha256::new();
    canonical.update(RECOVERY_SCOPE_DOMAIN);
    hash_text(&mut canonical, input.service_id);
    hash_text(&mut canonical, input.principal_id);
    hash_text(&mut canonical, input.caller_context_id);
    canonical.update(input.mapping_generation.get().to_be_bytes());
    hash_text(&mut canonical, input.role);
    canonical.update(input.catalog_generation.get().to_be_bytes());
    match input.transition {
        RecoveryLifecycleTransition::Initial => canonical.update([0]),
        RecoveryLifecycleTransition::Replace {
            previous_generation,
        } => {
            canonical.update([1]);
            canonical.update(previous_generation.get().to_be_bytes());
        }
    }
    hash_text(&mut canonical, input.package_id);
    canonical.update(input.package_app_id.to_be_bytes());
    hash_text(&mut canonical, input.canonical_entry);
    canonical.update(input.signed_package_sha256);
    canonical.update(input.executable_sha256);
    canonical.update(input.package_generation.to_be_bytes());
    canonical.update(input.c3_launch_generation.get().to_be_bytes());
    canonical.update(input.uid.to_be_bytes());
    canonical.update(input.gid.to_be_bytes());
    canonical.finalize().into()
}

/// Recompute the binding digest from the exact A-issued scope view. C1 and C3
/// can use this helper to detect inconsistent implementations without minting
/// or altering an admission object.
pub fn canonical_recovery_binding_digest_from_scope(
    scope: &dyn SystemServiceRecoveryAdmissionScope,
) -> [u8; 32] {
    let (uid, gid) = scope.credentials();
    canonical_recovery_binding_digest(&SystemServiceRecoveryScopeInput {
        service_id: scope.service_id(),
        principal_id: scope.principal_id(),
        caller_context_id: scope.caller_context_id(),
        mapping_generation: scope.mapping_generation(),
        role: scope.role(),
        catalog_generation: scope.catalog_generation(),
        transition: scope.transition(),
        package_id: scope.package_id(),
        package_app_id: scope.package_app_id(),
        canonical_entry: scope.canonical_entry(),
        signed_package_sha256: scope.signed_package_sha256(),
        executable_sha256: scope.executable_sha256(),
        package_generation: scope.package_generation(),
        c3_launch_generation: scope.c3_launch_generation(),
        uid,
        gid,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3RecoverySlotState {
    ClearedAndAbsent,
    Live,
    Quarantined,
    Unknown,
}

/// Implemented by the maintained C3 owner only after durable readback of the
/// exact service/catalog/C3 generation slot under its serialized slot gate.
pub trait C3DurableRecoverySlotReceipt {
    fn service_id(&self) -> &str;
    fn catalog_generation(&self) -> u64;
    fn c3_launch_generation(&self) -> u64;
    fn scope_binding_digest(&self) -> &[u8; 32];
    fn state(&self) -> C3RecoverySlotState;
}

/// Implemented for C1's private-constructor receipt by the C1 integration
/// adapter. A must then pass the original receipt by value to C1's durable
/// receipt consumer so the exact operation/binding/outcome is re-read.
pub trait C1RecoveryTerminalReceipt {
    fn operation_id(&self) -> u64;
    fn binding_digest(&self) -> &[u8; 32];
    fn outcome_digest(&self) -> [u8; 32];
}

/// Bounded error vocabulary for the cycle-free C1 receipt client. A durable
/// write whose outcome cannot be read back must return `OutcomeUnknown` and
/// leave the recovery fence in place.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryClientError {
    Unavailable,
    Rejected,
    Failed,
    OutcomeUnknown,
}

/// A's one-shot gate for C1 to reconcile the exact C3 service slot while C1
/// holds its own serialized recovery journal lock. A binds the gate to one
/// admission object; C1 must use that same object to derive its operation
/// permit and must establish their full binding equality before crossing an
/// irreversible terminal/fence-closure point. If a gate rejection or later
/// reconciliation error occurs after C1 closes its fence, C1 must keep that
/// fence closed; dropping the writer must never reopen the raw reader path.
/// A retains the C3 durable-slot receipt separately and compares it before
/// clearing its own fence.
pub trait C3RecoverySlotGate {
    fn with_reconciliation_gate<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError>;
}

/// Cycle-free C1 receipt API. Before any irreversible terminal/fence closure,
/// the adapter must compare the exact A-owned `scope` against the operation
/// permit and reconciliation identity it will persist and pass through the
/// gate. It must not defer this comparison until after closing the fence. If a
/// later check, write, readback, or gate call fails after closure, the adapter
/// keeps the fence closed even when its writer is dropped. The receipt type
/// should have private fields and be issued only after C1 commits a terminal
/// journal row bound to `scope.binding_digest()` and the operation ID.
/// `consume` must re-read that durable row for the exact receipt and reject
/// stale, mismatched, or replayed receipts. A clears no fence until this call
/// succeeds and its separately retained C3 slot receipt matches the same
/// scope.
pub trait C1RecoveryReceiptClient {
    type Receipt: C1RecoveryTerminalReceipt;

    fn reconcile_terminal<G: C3RecoverySlotGate>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        gate: &mut G,
    ) -> Result<Self::Receipt, RecoveryClientError>;

    fn consume_terminal_receipt(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        receipt: Self::Receipt,
    ) -> Result<(), RecoveryClientError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryReceiptMismatch {
    NonCanonicalScopeBinding,
    ZeroOperationId,
    WrongC1Binding,
    WrongC3Binding,
    WrongService,
    WrongCatalogGeneration,
    WrongC3LaunchGeneration,
    C3SlotNotCleared,
}

/// Validate consistency fields only. C1 must still consume the original
/// opaque receipt against its durable journal, and C3 must source its typed
/// receipt from a durable exact-slot readback. This function cannot clear a
/// fence or mint authority.
pub fn validate_recovery_receipts<C1, C3>(
    scope: &dyn SystemServiceRecoveryAdmissionScope,
    c1: &C1,
    c3: &C3,
) -> Result<(), RecoveryReceiptMismatch>
where
    C1: C1RecoveryTerminalReceipt,
    C3: C3DurableRecoverySlotReceipt,
{
    if canonical_recovery_binding_digest_from_scope(scope) != *scope.binding_digest() {
        return Err(RecoveryReceiptMismatch::NonCanonicalScopeBinding);
    }
    if c1.operation_id() == 0 {
        return Err(RecoveryReceiptMismatch::ZeroOperationId);
    }
    if c1.binding_digest() != scope.binding_digest() {
        return Err(RecoveryReceiptMismatch::WrongC1Binding);
    }
    if c3.scope_binding_digest() != scope.binding_digest() {
        return Err(RecoveryReceiptMismatch::WrongC3Binding);
    }
    if c3.service_id() != scope.service_id() {
        return Err(RecoveryReceiptMismatch::WrongService);
    }
    if c3.catalog_generation() != scope.catalog_generation().get() {
        return Err(RecoveryReceiptMismatch::WrongCatalogGeneration);
    }
    if c3.c3_launch_generation() != scope.c3_launch_generation().get() {
        return Err(RecoveryReceiptMismatch::WrongC3LaunchGeneration);
    }
    if c3.state() != C3RecoverySlotState::ClearedAndAbsent {
        return Err(RecoveryReceiptMismatch::C3SlotNotCleared);
    }
    let _outcome_digest = c1.outcome_digest();
    Ok(())
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_recovery_binding_digest, validate_recovery_receipts, C1RecoveryTerminalReceipt,
        C3DurableRecoverySlotReceipt, C3RecoverySlotState, RecoveryLifecycleTransition,
        RecoveryReceiptMismatch, SystemServiceRecoveryAdmissionScope,
        SystemServiceRecoveryScopeInput,
    };
    use core::num::NonZeroU64;

    fn scope_input(
        principal_id: &'static str,
        c3_generation: u64,
    ) -> SystemServiceRecoveryScopeInput<'static> {
        SystemServiceRecoveryScopeInput {
            service_id: "fixture.identity",
            principal_id,
            caller_context_id: "caller_fixture_identity",
            mapping_generation: NonZeroU64::new(3).unwrap(),
            role: "system_service",
            catalog_generation: NonZeroU64::new(9).unwrap(),
            transition: RecoveryLifecycleTransition::Initial,
            package_id: "org.nagi.fixture",
            package_app_id: 0x1122,
            canonical_entry: "fixture.elf",
            signed_package_sha256: &[0x11; 32],
            executable_sha256: &[0x22; 32],
            package_generation: 17,
            c3_launch_generation: NonZeroU64::new(c3_generation).unwrap(),
            uid: 1001,
            gid: 1002,
        }
    }

    struct FixtureScope {
        principal_id: &'static str,
        c3_generation: NonZeroU64,
        binding: [u8; 32],
    }

    fn scope(principal_id: &'static str, c3_generation: u64) -> FixtureScope {
        FixtureScope {
            principal_id,
            c3_generation: NonZeroU64::new(c3_generation).unwrap(),
            binding: canonical_recovery_binding_digest(&scope_input(principal_id, c3_generation)),
        }
    }

    impl SystemServiceRecoveryAdmissionScope for FixtureScope {
        fn service_id(&self) -> &str {
            "fixture.identity"
        }

        fn principal_id(&self) -> &str {
            self.principal_id
        }

        fn caller_context_id(&self) -> &str {
            "caller_fixture_identity"
        }

        fn mapping_generation(&self) -> NonZeroU64 {
            NonZeroU64::new(3).unwrap()
        }

        fn role(&self) -> &str {
            "system_service"
        }

        fn catalog_generation(&self) -> NonZeroU64 {
            NonZeroU64::new(9).unwrap()
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
            self.c3_generation
        }

        fn credentials(&self) -> (u32, u32) {
            (1001, 1002)
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding
        }
    }

    struct Receipt {
        operation_id: u64,
        binding: [u8; 32],
        outcome: [u8; 32],
    }

    impl C1RecoveryTerminalReceipt for Receipt {
        fn operation_id(&self) -> u64 {
            self.operation_id
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding
        }

        fn outcome_digest(&self) -> [u8; 32] {
            self.outcome
        }
    }

    struct SlotReceipt<'a> {
        service_id: &'a str,
        catalog_generation: u64,
        c3_generation: u64,
        binding: [u8; 32],
        state: C3RecoverySlotState,
    }

    impl C3DurableRecoverySlotReceipt for SlotReceipt<'_> {
        fn service_id(&self) -> &str {
            self.service_id
        }

        fn catalog_generation(&self) -> u64 {
            self.catalog_generation
        }

        fn c3_launch_generation(&self) -> u64 {
            self.c3_generation
        }

        fn scope_binding_digest(&self) -> &[u8; 32] {
            &self.binding
        }

        fn state(&self) -> C3RecoverySlotState {
            self.state
        }
    }

    fn c1(scope: &dyn SystemServiceRecoveryAdmissionScope, operation_id: u64) -> Receipt {
        Receipt {
            operation_id,
            binding: *scope.binding_digest(),
            outcome: [0x33; 32],
        }
    }

    fn c3(scope: &dyn SystemServiceRecoveryAdmissionScope) -> SlotReceipt<'_> {
        SlotReceipt {
            service_id: scope.service_id(),
            catalog_generation: scope.catalog_generation().get(),
            c3_generation: scope.c3_launch_generation().get(),
            binding: *scope.binding_digest(),
            state: C3RecoverySlotState::ClearedAndAbsent,
        }
    }

    #[test]
    fn canonical_scope_digest_changes_with_identity_and_c3_generation() {
        let original =
            canonical_recovery_binding_digest(&scope_input("service:fixture.identity", 33));
        assert_ne!(
            original,
            canonical_recovery_binding_digest(&scope_input("service:other.identity", 33))
        );
        assert_ne!(
            original,
            canonical_recovery_binding_digest(&scope_input("service:fixture.identity", 34))
        );
    }

    #[test]
    fn receipt_validation_rejects_missing_operation_and_wrong_c1_binding() {
        let scope = scope("service:fixture.identity", 33);
        let slot = c3(&scope);
        assert_eq!(
            validate_recovery_receipts(&scope, &c1(&scope, 0), &slot),
            Err(RecoveryReceiptMismatch::ZeroOperationId)
        );
        let mut wrong_binding = c1(&scope, 1);
        wrong_binding.binding[0] ^= 1;
        assert_eq!(
            validate_recovery_receipts(&scope, &wrong_binding, &slot),
            Err(RecoveryReceiptMismatch::WrongC1Binding)
        );
    }

    #[test]
    fn receipt_validation_rejects_noncanonical_scope_binding() {
        let mut scope = scope("service:fixture.identity", 33);
        scope.binding[0] ^= 1;
        let receipt = c1(&scope, 1);
        let slot = c3(&scope);
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::NonCanonicalScopeBinding)
        );
    }

    #[test]
    fn receipt_validation_rejects_unmatched_or_uncleared_c3_slot() {
        let scope = scope("service:fixture.identity", 33);
        let receipt = c1(&scope, 1);
        let mut slot = c3(&scope);
        slot.service_id = "other.service";
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::WrongService)
        );
        let mut slot = c3(&scope);
        slot.catalog_generation += 1;
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::WrongCatalogGeneration)
        );
        let mut slot = c3(&scope);
        slot.c3_generation += 1;
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::WrongC3LaunchGeneration)
        );
        let mut slot = c3(&scope);
        slot.binding[0] ^= 1;
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::WrongC3Binding)
        );
        let mut slot = c3(&scope);
        slot.state = C3RecoverySlotState::Quarantined;
        assert_eq!(
            validate_recovery_receipts(&scope, &receipt, &slot),
            Err(RecoveryReceiptMismatch::C3SlotNotCleared)
        );
    }
}
