//! C3's live-proof gate for an A-issued system-service recovery admission.
//!
//! This adapter consumes the shared recovery contract directly so C1 and C3
//! use the same trait identity. It checks the same admission object on every
//! call and binds the effect to C3's live pidfd, endpoint, service slot, and
//! serialized launch-generation gate. It does not authenticate the admission's
//! origin, persist a C3 slot receipt, or create a production service launch.

use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, C3RecoverySlotGate, RecoveryClientError,
    RecoveryLifecycleTransition, SystemServiceRecoveryAdmissionScope,
    SystemServiceRecoveryScopeInput, canonical_recovery_binding_digest,
};
use std::num::NonZeroU64;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// One-shot C3 callback gate retaining the exact admission reference passed by
/// the A/C1 owner. The proof is non-Clone and must come from the maintained C3
/// process/endpoint owner.
pub struct C3ProofRecoveryGate<'scope, 'proof> {
    scope: &'scope dyn SystemServiceRecoveryAdmissionScope,
    /// A consistency pin of the complete view exposed by the exact A-issued
    /// object at construction. It is never treated as an authority token.
    scope_snapshot: Option<RecoveryScopeSnapshot>,
    proof: &'proof SystemServiceEndpointOwnerProof,
    used: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecoveryScopeSnapshot {
    pub(super) service_id: String,
    pub(super) principal_id: String,
    pub(super) caller_context_id: String,
    pub(super) mapping_generation: NonZeroU64,
    pub(super) role: String,
    pub(super) catalog_generation: NonZeroU64,
    pub(super) transition: RecoveryLifecycleTransition,
    pub(super) package_id: String,
    pub(super) package_app_id: u64,
    pub(super) canonical_entry: String,
    pub(super) signed_package_sha256: [u8; 32],
    pub(super) executable_sha256: [u8; 32],
    pub(super) package_generation: u64,
    pub(super) c3_launch_generation: NonZeroU64,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) binding_digest: [u8; 32],
}

impl RecoveryScopeSnapshot {
    pub(super) fn capture(scope: &dyn SystemServiceRecoveryAdmissionScope) -> Self {
        let (uid, gid) = scope.credentials();
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
            uid,
            gid,
            binding_digest: *scope.binding_digest(),
        }
    }

    pub(super) fn canonical_digest(&self) -> [u8; 32] {
        canonical_recovery_binding_digest(&SystemServiceRecoveryScopeInput {
            service_id: &self.service_id,
            principal_id: &self.principal_id,
            caller_context_id: &self.caller_context_id,
            mapping_generation: self.mapping_generation,
            role: &self.role,
            catalog_generation: self.catalog_generation,
            transition: self.transition,
            package_id: &self.package_id,
            package_app_id: self.package_app_id,
            canonical_entry: &self.canonical_entry,
            signed_package_sha256: &self.signed_package_sha256,
            executable_sha256: &self.executable_sha256,
            package_generation: self.package_generation,
            c3_launch_generation: self.c3_launch_generation,
            uid: self.uid,
            gid: self.gid,
        })
    }
}

impl<'scope, 'proof> C3ProofRecoveryGate<'scope, 'proof> {
    pub fn new(
        scope: &'scope dyn SystemServiceRecoveryAdmissionScope,
        proof: &'proof SystemServiceEndpointOwnerProof,
    ) -> Self {
        Self {
            scope,
            scope_snapshot: catch_unwind(AssertUnwindSafe(|| {
                RecoveryScopeSnapshot::capture(scope)
            }))
            .ok(),
            proof,
            used: false,
        }
    }

    pub fn was_used(&self) -> bool {
        self.used
    }
}

impl C3RecoverySlotGate for C3ProofRecoveryGate<'_, '_> {
    fn with_reconciliation_gate<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError> {
        if self.used {
            return Err(RecoveryClientError::Rejected);
        }
        self.used = true;

        if !std::ptr::eq(scope, self.scope) {
            return Err(RecoveryClientError::Rejected);
        }
        if claim.binding_digest() != scope.binding_digest() {
            return Err(RecoveryClientError::Rejected);
        }

        let Some(snapshot) = self.scope_snapshot.as_ref() else {
            return Err(RecoveryClientError::Rejected);
        };
        let current = match catch_unwind(AssertUnwindSafe(|| RecoveryScopeSnapshot::capture(scope)))
        {
            Ok(snapshot) => snapshot,
            Err(_) => return Err(RecoveryClientError::Rejected),
        };
        if current != *snapshot {
            return Err(RecoveryClientError::Rejected);
        }
        validate_scope_snapshot_matches_proof(snapshot, self.proof)?;

        match catch_unwind(AssertUnwindSafe(|| {
            self.proof.with_live_authority_effect(reconcile)
        })) {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(RecoveryClientError::Rejected)
            }
            Ok(Err(_)) => Err(RecoveryClientError::Unavailable),
            // The callback may have crossed C1's durable terminal point before
            // unwinding. Preserve C1's recovery fence and report the outcome as
            // unknown; the poisoned C3 gate also fails closed on later use.
            Err(_) => Err(RecoveryClientError::OutcomeUnknown),
        }
    }
}

fn validate_scope_snapshot_matches_proof(
    scope: &RecoveryScopeSnapshot,
    proof: &SystemServiceEndpointOwnerProof,
) -> Result<(), RecoveryClientError> {
    if scope.canonical_digest() != scope.binding_digest {
        return Err(RecoveryClientError::Rejected);
    }

    let slot = proof.service_slot_identity();
    let peer = proof.peer_credentials();
    if scope.role != "system_service"
        || scope.service_id != slot.service_id()
        || scope.catalog_generation != slot.catalog_generation()
        || scope.c3_launch_generation != proof.launch_generation()
        || (scope.uid, scope.gid) != (peer.uid, peer.gid)
        || proof.process_identity().process_id() != peer.process_id
    {
        return Err(RecoveryClientError::Rejected);
    }

    Ok(())
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::{C3ProofRecoveryGate, SystemServiceRecoveryAdmissionScope};
    use nagi_init_spawn_contract::test_pidfd_for_existing_process;
    use nagi_system_principal_contract::{
        SystemServiceEndpointOwnerProof, SystemServiceLaunchRecord,
    };
    use nagi_system_service_recovery_contract::{
        C1RecoveryOperationCorrelation, C3RecoverySlotGate, RecoveryClientError,
        RecoveryLifecycleTransition, SystemServiceRecoveryScopeInput,
        canonical_recovery_binding_digest,
    };
    use std::num::NonZeroU64;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FixtureScope {
        service_id: &'static str,
        catalog_generation: NonZeroU64,
        launch_generation: NonZeroU64,
        uid: u32,
        gid: u32,
        binding: [u8; 32],
    }

    fn fixture_scope(
        service_id: &'static str,
        catalog_generation: u64,
        launch_generation: u64,
        uid: u32,
        gid: u32,
    ) -> FixtureScope {
        let catalog_generation = NonZeroU64::new(catalog_generation).unwrap();
        let launch_generation = NonZeroU64::new(launch_generation).unwrap();
        let signed_package_sha256 = [0x11; 32];
        let executable_sha256 = [0x22; 32];
        let binding = canonical_recovery_binding_digest(&SystemServiceRecoveryScopeInput {
            service_id,
            principal_id: "service:fixture.identity",
            caller_context_id: "caller_fixture_identity",
            mapping_generation: NonZeroU64::new(3).unwrap(),
            role: "system_service",
            catalog_generation,
            transition: RecoveryLifecycleTransition::Initial,
            package_id: "org.nagi.fixture",
            package_app_id: 0x1122,
            canonical_entry: "fixture.elf",
            signed_package_sha256: &signed_package_sha256,
            executable_sha256: &executable_sha256,
            package_generation: 17,
            c3_launch_generation: launch_generation,
            uid,
            gid,
        });
        FixtureScope {
            service_id,
            catalog_generation,
            launch_generation,
            uid,
            gid,
            binding,
        }
    }

    impl SystemServiceRecoveryAdmissionScope for FixtureScope {
        fn service_id(&self) -> &str {
            self.service_id
        }

        fn principal_id(&self) -> &str {
            "service:fixture.identity"
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
            self.launch_generation
        }

        fn credentials(&self) -> (u32, u32) {
            (self.uid, self.gid)
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding
        }
    }

    struct SwitchingScope {
        first: FixtureScope,
        second: FixtureScope,
        show_second: std::sync::atomic::AtomicBool,
    }

    impl SwitchingScope {
        fn current(&self) -> &FixtureScope {
            if self.show_second.load(Ordering::SeqCst) {
                &self.second
            } else {
                &self.first
            }
        }
    }

    impl SystemServiceRecoveryAdmissionScope for SwitchingScope {
        fn service_id(&self) -> &str {
            self.current().service_id()
        }
        fn principal_id(&self) -> &str {
            self.current().principal_id()
        }
        fn caller_context_id(&self) -> &str {
            self.current().caller_context_id()
        }
        fn mapping_generation(&self) -> NonZeroU64 {
            self.current().mapping_generation()
        }
        fn role(&self) -> &str {
            self.current().role()
        }
        fn catalog_generation(&self) -> NonZeroU64 {
            self.current().catalog_generation()
        }
        fn transition(&self) -> RecoveryLifecycleTransition {
            self.current().transition()
        }
        fn package_id(&self) -> &str {
            self.current().package_id()
        }
        fn package_app_id(&self) -> u64 {
            self.current().package_app_id()
        }
        fn canonical_entry(&self) -> &str {
            self.current().canonical_entry()
        }
        fn signed_package_sha256(&self) -> &[u8; 32] {
            self.current().signed_package_sha256()
        }
        fn executable_sha256(&self) -> &[u8; 32] {
            self.current().executable_sha256()
        }
        fn package_generation(&self) -> u64 {
            self.current().package_generation()
        }
        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.current().c3_launch_generation()
        }
        fn credentials(&self) -> (u32, u32) {
            self.current().credentials()
        }
        fn binding_digest(&self) -> &[u8; 32] {
            self.current().binding_digest()
        }
    }

    fn current_uid_gid() -> (u32, u32) {
        extern "C" {
            fn getuid() -> u32;
            fn getgid() -> u32;
        }
        // SAFETY: getuid/getgid have no pointer arguments or side effects.
        unsafe { (getuid(), getgid()) }
    }

    fn proof_and_peer(
        service_id: &str,
        catalog_generation: u64,
        launch_generation: u64,
        uid: u32,
        gid: u32,
    ) -> (SystemServiceEndpointOwnerProof, UnixStream) {
        let process_id = std::process::id();
        let process = test_pidfd_for_existing_process(process_id).unwrap();
        // SAFETY: this test fixture binds these identities to the exact live
        // process opened above. No fixture value is used as production input.
        let record = unsafe {
            SystemServiceLaunchRecord::after_supervisor_system_service_slot_validation(
                &process,
                service_id,
                NonZeroU64::new(catalog_generation).unwrap(),
                NonZeroU64::new(launch_generation).unwrap(),
            )
        };
        let (endpoint, peer) = UnixStream::pair().unwrap();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            OwnedFd::from(endpoint),
            uid,
            gid,
        )
        .unwrap();
        (proof, peer)
    }

    fn run_gate<R>(
        gate: &mut C3ProofRecoveryGate<'_, '_>,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError> {
        let claim = C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(1).unwrap(),
            *scope.binding_digest(),
        );
        gate.with_reconciliation_gate(scope, claim, reconcile)
    }

    #[test]
    fn fixture_scope_uses_same_reference_and_live_exact_slot_gate() {
        let (uid, gid) = current_uid_gid();
        let scope = fixture_scope("fixture.identity", 9, 41, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let effects = AtomicUsize::new(0);
        let mut gate = C3ProofRecoveryGate::new(&scope, &proof);

        let result = run_gate(&mut gate, &scope, || effects.fetch_add(1, Ordering::SeqCst));

        assert_eq!(result, Ok(0));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert!(gate.was_used());
        assert_eq!(
            run_gate(&mut gate, &scope, || effects.fetch_add(1, Ordering::SeqCst)),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fixture_gate_rejects_distinct_scope_reference_and_wrong_slot_generation() {
        let (uid, gid) = current_uid_gid();
        let scope = fixture_scope("fixture.identity", 9, 41, uid, gid);
        let same_fields_new_object = fixture_scope("fixture.identity", 9, 41, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let effects = AtomicUsize::new(0);
        let mut gate = C3ProofRecoveryGate::new(&scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &same_fields_new_object, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let wrong_catalog_scope = fixture_scope("fixture.identity", 10, 41, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&wrong_catalog_scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &wrong_catalog_scope, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let wrong_service_scope = fixture_scope("fixture.other", 9, 41, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&wrong_service_scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &wrong_service_scope, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let wrong_launch_scope = fixture_scope("fixture.identity", 9, 42, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&wrong_launch_scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &wrong_launch_scope, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let wrong_credentials_scope =
            fixture_scope("fixture.identity", 9, 41, uid.wrapping_add(1), gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&wrong_credentials_scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &wrong_credentials_scope, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let mut tampered_scope = fixture_scope("fixture.identity", 9, 41, uid, gid);
        tampered_scope.binding[0] ^= 1;
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&tampered_scope, &proof);
        assert_eq!(
            run_gate(&mut gate, &tampered_scope, || {
                effects.fetch_add(1, Ordering::SeqCst)
            }),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fixture_gate_rejects_same_scope_object_that_switches_from_a_to_b() {
        let (uid, gid) = current_uid_gid();
        let scope = SwitchingScope {
            first: fixture_scope("fixture.identity", 9, 41, uid, gid),
            second: fixture_scope("fixture.other", 10, 42, uid.wrapping_add(1), gid),
            show_second: std::sync::atomic::AtomicBool::new(false),
        };
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let effects = AtomicUsize::new(0);
        let mut gate = C3ProofRecoveryGate::new(&scope, &proof);

        // C1 observes A. The same trait-object pointer then presents B to C3.
        scope.show_second.store(true, Ordering::SeqCst);
        assert_eq!(
            run_gate(&mut gate, &scope, || effects.fetch_add(1, Ordering::SeqCst)),
            Err(RecoveryClientError::Rejected)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn fixture_callback_panic_is_reported_as_unknown_and_poisoned_gate_rejects_retry() {
        let (uid, gid) = current_uid_gid();
        let scope = fixture_scope("fixture.identity", 9, 41, uid, gid);
        let (proof, _peer) = proof_and_peer("fixture.identity", 9, 41, uid, gid);
        let mut gate = C3ProofRecoveryGate::new(&scope, &proof);

        assert_eq!(
            run_gate(&mut gate, &scope, || panic!("fixture callback panic")),
            Err(RecoveryClientError::OutcomeUnknown)
        );
        assert_eq!(
            run_gate(&mut gate, &scope, || ()),
            Err(RecoveryClientError::Rejected)
        );
    }
}
