//! Task-local contract for an owner's durable service-effect outcome record.
//!
//! Caller data and owner-validated request state are different types. A caller
//! can create [`UntrustedServiceEffectRequest`] or the task41-compatible
//! [`DurableServiceRequestIdentity`] claim, but only the crate's trusted owner
//! composition may mint [`OwnerValidatedServiceEffectRequest`] after it has
//! canonicalized the route and checked the live authority. The validated value
//! is still a snapshot: the owner must recheck it inside the effect transaction
//! before invoking a side effect.
//!
//! The stable key and payload/route binding survive process restarts. The
//! ephemeral authority epoch is deliberately separate and changes when its
//! process/session authority is renewed. A previous epoch cannot authorize a
//! retry after restart. If the new live authority or the prior durable outcome
//! cannot be reconstructed, the owner must return `Unavailable`,
//! `OutcomeUnknown`, or `Stale`; it must not synthesize `NotInvoked`.
//!
//! No production service-effect owner currently implements this contract.
//! This module is separate from Model Store `ReceiptStore` and task41's PID1
//! resolver.
//!
//! A model/package receipt is not an effect outcome:
//!
//! ```compile_fail
//! use nagi_model_manager::ReceiptEnvelope;
//! use nagi_task37_receipt_supplier::service_effect::ServiceEffectOutcomeRecord;
//!
//! fn promote_model_receipt(receipt: ReceiptEnvelope) -> ServiceEffectOutcomeRecord {
//!     receipt.into()
//! }
//! ```
//!
//! Callers cannot promote the shared request claim into an owner-validated binding:
//!
//! ```compile_fail
//! use nagi_task37_receipt_supplier::service_effect::{
//!     DurableServiceEffectOwner, DurableServiceRequestBinding,
//!     DurableServiceRequestIdentity, DurableServiceRequestKey,
//!     ServiceEffectOwnerId, UnavailableServiceEffectOwner,
//! };
//! let claim = DurableServiceRequestIdentity::new(
//!     DurableServiceRequestKey::new(ServiceEffectOwnerId::new([1; 16]), 7, [2; 16]),
//!     DurableServiceRequestBinding::new([3; 32], [4; 32], [5; 32]),
//! );
//! // The owner API rejects a public claim where an owner-validated value is required.
//! let _forged = UnavailableServiceEffectOwner.lookup_outcome(&claim);
//! ```

use core::num::NonZeroU64;

/// Stable identity of the canonical service owner namespace. This is a
/// database key component, not proof that the caller is authorized for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ServiceEffectOwnerId([u8; 16]);

impl ServiceEffectOwnerId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Durable idempotency key. The stable wire request ID is retained across
/// process restarts; ephemeral process/session authority is not part of this
/// key. This exact key type is shared with task41's adapter/resolver candidate.
/// The owner must compare the namespace to its own canonical identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DurableServiceRequestKey {
    owner: ServiceEffectOwnerId,
    app_id: u64,
    stable_request_id: [u8; 16],
}

impl DurableServiceRequestKey {
    pub const fn new(
        owner: ServiceEffectOwnerId,
        app_id: u64,
        stable_request_id: [u8; 16],
    ) -> Self {
        Self {
            owner,
            app_id,
            stable_request_id,
        }
    }

    pub const fn owner(self) -> ServiceEffectOwnerId {
        self.owner
    }

    pub const fn app_id(self) -> u64 {
        self.app_id
    }

    pub const fn stable_request_id(self) -> [u8; 16] {
        self.stable_request_id
    }
}

/// Untrusted request bytes supplied by a caller. The route is a request claim,
/// not a trusted route resolution; the owner must canonicalize it against its
/// live registry. Authority is intentionally not accepted as a caller-provided
/// fingerprint: the owner must obtain current session/consent state from its
/// own authority source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UntrustedServiceEffectRequest<'a> {
    key: DurableServiceRequestKey,
    payload: &'a [u8],
    requested_route: &'a [u8],
}

impl<'a> UntrustedServiceEffectRequest<'a> {
    pub const fn new(
        key: DurableServiceRequestKey,
        payload: &'a [u8],
        requested_route: &'a [u8],
    ) -> Self {
        Self {
            key,
            payload,
            requested_route,
        }
    }

    pub const fn key(self) -> DurableServiceRequestKey {
        self.key
    }

    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    pub const fn requested_route(self) -> &'a [u8] {
        self.requested_route
    }
}

/// Shared task41-compatible request-binding claim. This public data container
/// is untrusted and must never be treated as proof that the fingerprint was
/// computed from the payload or that the route is canonical. The effect owner
/// must independently validate the raw request and mint a distinct opaque
/// [`OwnerValidatedServiceEffectRequest`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableServiceRequestBinding {
    request_fingerprint: [u8; 32],
    route_fingerprint: [u8; 32],
    authority_generation_fingerprint: [u8; 32],
}

impl DurableServiceRequestBinding {
    /// Create an untrusted binding claim for adapter/resolver key matching.
    /// This constructor does not establish provenance or validate the request.
    pub const fn new(
        request_fingerprint: [u8; 32],
        route_fingerprint: [u8; 32],
        authority_generation_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            request_fingerprint,
            route_fingerprint,
            authority_generation_fingerprint,
        }
    }

    pub const fn request_fingerprint(self) -> [u8; 32] {
        self.request_fingerprint
    }

    pub const fn route_fingerprint(self) -> [u8; 32] {
        self.route_fingerprint
    }

    pub const fn authority_generation_fingerprint(self) -> [u8; 32] {
        self.authority_generation_fingerprint
    }
}

/// Shared task41-compatible identity claim. It may be used for request lookup
/// matching but it is not authenticated owner state and cannot authorize an
/// effect or `NotInvoked` retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableServiceRequestIdentity {
    key: DurableServiceRequestKey,
    binding: DurableServiceRequestBinding,
}

impl DurableServiceRequestIdentity {
    /// Create an untrusted identity claim. This constructor is data-only.
    pub const fn new(key: DurableServiceRequestKey, binding: DurableServiceRequestBinding) -> Self {
        Self { key, binding }
    }

    pub const fn key(self) -> DurableServiceRequestKey {
        self.key
    }

    pub const fn binding(self) -> DurableServiceRequestBinding {
        self.binding
    }
}

/// Stable binding privately derived by the owner from exact payload bytes and
/// its canonical route. Ephemeral authority is intentionally excluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StableServiceRequestBinding {
    payload_fingerprint: [u8; 32],
    canonical_route_fingerprint: [u8; 32],
}

impl StableServiceRequestBinding {
    const fn from_verified_owner(
        payload_fingerprint: [u8; 32],
        canonical_route_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            payload_fingerprint,
            canonical_route_fingerprint,
        }
    }
}

/// Ephemeral authority scope validated from the live owner-side session.
/// `owner_incarnation` changes across process/authority restart. The authority
/// fingerprint binds current process, session, inventory, and consent epochs;
/// it is not persisted as part of the stable request identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EphemeralServiceAuthorityBinding {
    owner_incarnation: NonZeroU64,
    authority_fingerprint: [u8; 32],
}

impl EphemeralServiceAuthorityBinding {
    /// Internal owner-composition constructor. Its arguments must come from
    /// the current live authority source, never from a request field.
    pub(crate) const fn from_live_owner(
        owner_incarnation: NonZeroU64,
        authority_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            owner_incarnation,
            authority_fingerprint,
        }
    }

    pub const fn owner_incarnation(self) -> NonZeroU64 {
        self.owner_incarnation
    }
}

/// Opaque request snapshot minted only by owner validation. It combines stable
/// identity with an ephemeral authority fence. It is not authorization by
/// itself: commit must revalidate that fence atomically before the effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerValidatedServiceEffectRequest {
    key: DurableServiceRequestKey,
    stable_binding: StableServiceRequestBinding,
    authority: EphemeralServiceAuthorityBinding,
}

impl OwnerValidatedServiceEffectRequest {
    const fn from_verified_owner(
        key: DurableServiceRequestKey,
        stable_binding: StableServiceRequestBinding,
        authority: EphemeralServiceAuthorityBinding,
    ) -> Self {
        Self {
            key,
            stable_binding,
            authority,
        }
    }

    pub const fn key(self) -> DurableServiceRequestKey {
        self.key
    }

    pub const fn authority_incarnation(self) -> NonZeroU64 {
        self.authority.owner_incarnation
    }
}

/// Reference to the durable result payload. The owner must retain the referenced
/// payload and verify its digest when returning it for replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceEffectResultReference {
    record_id: [u8; 16],
    digest: [u8; 32],
}

impl ServiceEffectResultReference {
    pub const fn new(record_id: [u8; 16], digest: [u8; 32]) -> Self {
        Self { record_id, digest }
    }

    pub const fn record_id(self) -> [u8; 16] {
        self.record_id
    }

    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

/// Persisted owner outcome. `ReceiptRecorded` is deliberately absent: a
/// package/model receipt write says nothing about whether an application
/// effect ran.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceEffectOutcome {
    /// Durable owner state proves the effect callback has not been entered.
    NotInvoked,
    /// The owner has claimed this request key; interruption is unresolved.
    Executing,
    /// The effect result and its durable result reference were committed.
    Executed(ServiceEffectResultReference),
    /// The owner can prove the effect was not applied and records a terminal
    /// failure. This does not implicitly authorize another attempt.
    FailedNoEffect { failure_code: u32 },
    /// The effect may have been applied but its result could not be committed
    /// or recovered. It must not be reset to `NotInvoked` by absence of data.
    OutcomeUnknown,
}

/// One revision of the canonical service owner's durable result record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceEffectOutcomeRecord {
    key: DurableServiceRequestKey,
    stable_binding: StableServiceRequestBinding,
    attempt_authority: EphemeralServiceAuthorityBinding,
    owner_generation: NonZeroU64,
    record_revision: NonZeroU64,
    outcome: ServiceEffectOutcome,
}

impl ServiceEffectOutcomeRecord {
    /// Internal owner-composition constructor. A durable row may be returned
    /// only after its complete commit marker and all fields are verified.
    pub(crate) const fn from_verified_owner(
        request: OwnerValidatedServiceEffectRequest,
        owner_generation: NonZeroU64,
        record_revision: NonZeroU64,
        outcome: ServiceEffectOutcome,
    ) -> Self {
        Self {
            key: request.key,
            stable_binding: request.stable_binding,
            attempt_authority: request.authority,
            owner_generation,
            record_revision,
            outcome,
        }
    }

    pub const fn key(self) -> DurableServiceRequestKey {
        self.key
    }

    pub const fn owner_generation(self) -> NonZeroU64 {
        self.owner_generation
    }

    pub const fn record_revision(self) -> NonZeroU64 {
        self.record_revision
    }

    pub const fn outcome(self) -> ServiceEffectOutcome {
        self.outcome
    }

    /// Compare stable identity for result lookup/replay. A match alone does not
    /// authorize a new effect attempt: use `authorize_not_invoked_retry_for`.
    pub fn validate_stable_identity_for(
        &self,
        expected: &OwnerValidatedServiceEffectRequest,
        current_owner_generation: NonZeroU64,
    ) -> Result<(), ServiceEffectOwnerError> {
        if self.key.owner != expected.key.owner {
            return Err(ServiceEffectOwnerError::OwnerMismatch);
        }
        if self.key != expected.key || self.stable_binding != expected.stable_binding {
            return Err(ServiceEffectOwnerError::RequestIdentityConflict);
        }
        if self.owner_generation != current_owner_generation {
            return Err(ServiceEffectOwnerError::Stale);
        }
        Ok(())
    }

    /// Conservative retry gate. Only a complete explicit `NotInvoked` row
    /// bound to the same stable request, current durable owner generation, and
    /// current ephemeral authority epoch can authorize another attempt.
    /// After a restart, an old epoch is stale. The owner may rebind a row only
    /// after it has durably recovered proof that no effect ran and has
    /// revalidated current authority in the same transaction. This candidate
    /// does not implement that recovery operation.
    pub fn authorize_not_invoked_retry_for(
        &self,
        expected: &OwnerValidatedServiceEffectRequest,
        current_owner_generation: NonZeroU64,
    ) -> Result<(), ServiceEffectOwnerError> {
        self.validate_stable_identity_for(expected, current_owner_generation)?;
        if !matches!(self.outcome, ServiceEffectOutcome::NotInvoked) {
            return Err(ServiceEffectOwnerError::RetryNotAuthorized);
        }
        if self.attempt_authority != expected.authority {
            return Err(ServiceEffectOwnerError::Stale);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceEffectExecutionFailure {
    /// The effect owner can prove no side effect was applied.
    DefinitelyNotApplied { failure_code: u32 },
    /// The effect may have occurred; keep the request unresolved.
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceEffectOwnerError {
    /// No authoritative row exists. Absence does not prove that the effect was
    /// never entered.
    RecordMissing,
    Unavailable,
    Busy,
    OwnerMismatch,
    RequestIdentityConflict,
    Stale,
    RetryNotAuthorized,
    /// The owner observed an incomplete/pending publish and cannot establish a
    /// complete outcome record.
    RecoveryRequired,
    Corrupt,
    OutcomeUnknown,
}

/// Owner boundary for a durable request-result table and the effect it records.
///
/// `validate_request` receives only caller-controlled bytes. It must resolve
/// the owner namespace, parse the exact payload, canonicalize the route, and
/// obtain current authority from the live owner; it must not accept a caller
/// fingerprint or authority-generation claim as proof. Failure to reconstruct
/// live authority after restart is `Unavailable` or `OutcomeUnknown`.
///
/// `lookup_outcome` may expose a durable outcome only for the stable key and
/// exact payload/canonical-route binding returned by owner validation. Its
/// result does not authorize retry. A `NotInvoked` row authorizes retry only
/// when `authorize_not_invoked_retry_for` succeeds for the current ephemeral
/// authority epoch. A restart changes that epoch; if the previous no-effect
/// state cannot be recovered and rebound atomically, retry remains forbidden.
///
/// `commit_effect_once` must compare the same identity and revalidate current
/// authority inside one owner transaction, serialize concurrent attempts and
/// revocation, claim the row by revision CAS, and persist the effect outcome and
/// result reference with the effect. If an external effect cannot join that
/// transaction, its downstream owner must deduplicate using this exact stable
/// key; otherwise an ambiguous result remains `OutcomeUnknown`. A model or
/// package receipt CAS is not this boundary. Implementing this trait is only an
/// API contract and does not authenticate an implementation or prove
/// durability.
pub trait DurableServiceEffectOwner: Send + Sync {
    fn validate_request(
        &self,
        request: &UntrustedServiceEffectRequest<'_>,
    ) -> Result<OwnerValidatedServiceEffectRequest, ServiceEffectOwnerError>;

    fn lookup_outcome(
        &self,
        request: &OwnerValidatedServiceEffectRequest,
    ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError>;

    fn commit_effect_once(
        &self,
        request: &OwnerValidatedServiceEffectRequest,
        effect: impl FnOnce() -> Result<ServiceEffectResultReference, ServiceEffectExecutionFailure>,
    ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError>;
}

/// Safe default until the canonical effect owner and durable outcome table are
/// connected. It neither validates caller data, looks up a synthetic result,
/// nor enters the effect.
pub struct UnavailableServiceEffectOwner;

impl DurableServiceEffectOwner for UnavailableServiceEffectOwner {
    fn validate_request(
        &self,
        _request: &UntrustedServiceEffectRequest<'_>,
    ) -> Result<OwnerValidatedServiceEffectRequest, ServiceEffectOwnerError> {
        Err(ServiceEffectOwnerError::Unavailable)
    }

    fn lookup_outcome(
        &self,
        _request: &OwnerValidatedServiceEffectRequest,
    ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError> {
        Err(ServiceEffectOwnerError::Unavailable)
    }

    fn commit_effect_once(
        &self,
        _request: &OwnerValidatedServiceEffectRequest,
        _effect: impl FnOnce() -> Result<ServiceEffectResultReference, ServiceEffectExecutionFailure>,
    ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError> {
        Err(ServiceEffectOwnerError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DurableServiceEffectOwner, DurableServiceRequestKey, EphemeralServiceAuthorityBinding,
        OwnerValidatedServiceEffectRequest, ServiceEffectOutcome, ServiceEffectOutcomeRecord,
        ServiceEffectOwnerError, ServiceEffectOwnerId, ServiceEffectResultReference,
        StableServiceRequestBinding, UnavailableServiceEffectOwner, UntrustedServiceEffectRequest,
    };
    use core::num::NonZeroU64;

    fn validated(
        payload_fingerprint: [u8; 32],
        route_fingerprint: [u8; 32],
        owner_incarnation: u64,
    ) -> OwnerValidatedServiceEffectRequest {
        let key = DurableServiceRequestKey::new(ServiceEffectOwnerId::new([1; 16]), 41, [2; 16]);
        let stable_binding = StableServiceRequestBinding::from_verified_owner(
            payload_fingerprint,
            route_fingerprint,
        );
        let authority = EphemeralServiceAuthorityBinding::from_live_owner(
            NonZeroU64::new(owner_incarnation).unwrap(),
            [5; 32],
        );
        OwnerValidatedServiceEffectRequest::from_verified_owner(key, stable_binding, authority)
    }

    fn untrusted<'a>(payload: &'a [u8]) -> UntrustedServiceEffectRequest<'a> {
        UntrustedServiceEffectRequest::new(
            DurableServiceRequestKey::new(ServiceEffectOwnerId::new([1; 16]), 41, [2; 16]),
            payload,
            b"untrusted route claim",
        )
    }

    #[test]
    fn same_stable_request_id_with_changed_payload_conflicts() {
        let original = validated([3; 32], [4; 32], 7);
        let changed_payload = validated([9; 32], [4; 32], 7);
        let record = ServiceEffectOutcomeRecord::from_verified_owner(
            original,
            NonZeroU64::new(11).unwrap(),
            NonZeroU64::new(2).unwrap(),
            ServiceEffectOutcome::Executed(ServiceEffectResultReference::new([6; 16], [7; 32])),
        );

        assert_eq!(
            record.validate_stable_identity_for(&changed_payload, NonZeroU64::new(11).unwrap()),
            Err(ServiceEffectOwnerError::RequestIdentityConflict)
        );
        assert_eq!(
            record.authorize_not_invoked_retry_for(&changed_payload, NonZeroU64::new(11).unwrap()),
            Err(ServiceEffectOwnerError::RequestIdentityConflict)
        );
    }

    #[test]
    fn restart_with_old_ephemeral_authority_never_authorizes_not_invoked_retry() {
        let old_process_request = validated([3; 32], [4; 32], 7);
        let after_restart = validated([3; 32], [4; 32], 8);
        let durable_not_invoked = ServiceEffectOutcomeRecord::from_verified_owner(
            old_process_request,
            NonZeroU64::new(11).unwrap(),
            NonZeroU64::new(3).unwrap(),
            ServiceEffectOutcome::NotInvoked,
        );

        // Stable identity still matches across restart; that does not make the
        // old process/session epoch current or retryable.
        assert_eq!(
            durable_not_invoked
                .validate_stable_identity_for(&after_restart, NonZeroU64::new(11).unwrap()),
            Ok(())
        );
        assert_eq!(
            durable_not_invoked
                .authorize_not_invoked_retry_for(&after_restart, NonZeroU64::new(11).unwrap()),
            Err(ServiceEffectOwnerError::Stale)
        );
    }

    struct FailedOwner(ServiceEffectOwnerError);

    impl DurableServiceEffectOwner for FailedOwner {
        fn validate_request(
            &self,
            _request: &UntrustedServiceEffectRequest<'_>,
        ) -> Result<OwnerValidatedServiceEffectRequest, ServiceEffectOwnerError> {
            Err(self.0)
        }

        fn lookup_outcome(
            &self,
            _request: &OwnerValidatedServiceEffectRequest,
        ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError> {
            Err(self.0)
        }

        fn commit_effect_once(
            &self,
            _request: &OwnerValidatedServiceEffectRequest,
            _effect: impl FnOnce() -> Result<
                ServiceEffectResultReference,
                super::ServiceEffectExecutionFailure,
            >,
        ) -> Result<ServiceEffectOutcomeRecord, ServiceEffectOwnerError> {
            Err(self.0)
        }
    }

    #[test]
    fn missing_stale_partial_and_unavailable_lookup_never_confirm_not_invoked() {
        let request = validated([3; 32], [4; 32], 7);
        let stale_record = ServiceEffectOutcomeRecord::from_verified_owner(
            validated([3; 32], [4; 32], 6),
            NonZeroU64::new(11).unwrap(),
            NonZeroU64::new(3).unwrap(),
            ServiceEffectOutcome::NotInvoked,
        );
        assert_eq!(
            stale_record.authorize_not_invoked_retry_for(&request, NonZeroU64::new(11).unwrap()),
            Err(ServiceEffectOwnerError::Stale)
        );

        for failure in [
            ServiceEffectOwnerError::RecordMissing,
            ServiceEffectOwnerError::RecoveryRequired,
            ServiceEffectOwnerError::Unavailable,
            ServiceEffectOwnerError::OutcomeUnknown,
        ] {
            let owner = FailedOwner(failure);
            assert_eq!(owner.lookup_outcome(&request), Err(failure));
        }
    }

    #[test]
    fn untrusted_bytes_cannot_be_validated_by_the_unavailable_owner() {
        let input = untrusted(
            br#"{"terms_accepted":true,"consent_event_id":1413897044,"terms_digest":"caller-claim","authority_epoch":7,"artifact_revision":6}"#,
        );
        let owner = UnavailableServiceEffectOwner;
        let local_outcome = ServiceEffectOutcome::OutcomeUnknown;
        let mut effect_calls = 0;

        assert_eq!(
            owner.validate_request(&input),
            Err(ServiceEffectOwnerError::Unavailable)
        );
        assert_eq!(local_outcome, ServiceEffectOutcome::OutcomeUnknown);

        // Even an explicit test-only token is not available at this public
        // boundary; production callers must first get one from the owner.
        let internal_test_request = validated([3; 32], [4; 32], 7);
        assert_eq!(
            owner.commit_effect_once(&internal_test_request, || {
                effect_calls += 1;
                Ok(ServiceEffectResultReference::new([6; 16], [7; 32]))
            }),
            Err(ServiceEffectOwnerError::Unavailable)
        );
        assert_eq!(local_outcome, ServiceEffectOutcome::OutcomeUnknown);
        assert_eq!(effect_calls, 0);
    }
}
