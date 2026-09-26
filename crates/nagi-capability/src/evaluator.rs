use std::fmt;

use crate::ids::{CapabilityId, PrincipalId, SessionId};
use crate::model::{
    ActiveGrant, CapabilityScope, Grant, GrantEffect, GrantId, GrantLifetime, GrantSource,
    UnixTimestamp,
};
use crate::registry::CapabilityRegistry;
use crate::store::{PolicyDocument, PolicyStore, PolicyStoreError};

struct GrantParameters {
    effect: GrantEffect,
    lifetime: GrantLifetime,
    source: GrantSource,
    reason: Option<String>,
    granted_at: UnixTimestamp,
    expires_at: Option<UnixTimestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessRequest {
    pub principal: PrincipalId,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
}

impl AccessRequest {
    pub fn new(principal: PrincipalId, capability: CapabilityId, scope: CapabilityScope) -> Self {
        Self {
            principal,
            capability,
            scope,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationContext {
    pub now: UnixTimestamp,
    pub session_id: Option<SessionId>,
}

impl EvaluationContext {
    pub const fn new(now: UnixTimestamp, session_id: Option<SessionId>) -> Self {
        Self { now, session_id }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionEffect {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionReason {
    AuthorizedByGrant,
    ExplicitlyDenied,
    UnknownCapability,
    InvalidRequest,
    NoGrant,
    ScopeMismatch,
    Revoked,
    NotYetValid,
    Expired,
    WrongSession,
    OneShotConsumed,
    PolicyStoreUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decision {
    effect: DecisionEffect,
    reason: DecisionReason,
    grant_id: Option<GrantId>,
    explanation: String,
}

impl Decision {
    pub const fn effect(&self) -> DecisionEffect {
        self.effect
    }

    pub const fn reason(&self) -> DecisionReason {
        self.reason
    }

    pub const fn grant_id(&self) -> Option<GrantId> {
        self.grant_id
    }

    pub fn explanation(&self) -> &str {
        &self.explanation
    }

    pub const fn is_allowed(&self) -> bool {
        matches!(self.effect, DecisionEffect::Allow)
    }

    fn allow(grant: &Grant) -> Self {
        Self {
            effect: DecisionEffect::Allow,
            reason: DecisionReason::AuthorizedByGrant,
            grant_id: Some(grant.id),
            explanation: format!(
                "Allowed by active {:?} grant {} for capability {}.",
                grant.source, grant.id, grant.capability
            ),
        }
    }

    fn deny(reason: DecisionReason, explanation: impl Into<String>) -> Self {
        Self {
            effect: DecisionEffect::Deny,
            reason,
            grant_id: None,
            explanation: explanation.into(),
        }
    }

    fn deny_by_grant(grant: &Grant) -> Self {
        Self {
            effect: DecisionEffect::Deny,
            reason: DecisionReason::ExplicitlyDenied,
            grant_id: Some(grant.id),
            explanation: format!(
                "Denied by active {:?} policy grant {} for capability {}.",
                grant.source, grant.id, grant.capability
            ),
        }
    }
}

/// Runtime owners implement this contract at their service boundary after
/// the 0.2 integration gate. This crate intentionally provides no adapter.
pub trait CapabilityEnforcer {
    fn authorize(&mut self, request: &AccessRequest, context: &EvaluationContext) -> Decision;
}

pub struct PermissionEvaluator<S> {
    registry: CapabilityRegistry,
    store: S,
}

impl<S: PolicyStore> PermissionEvaluator<S> {
    pub fn new(registry: CapabilityRegistry, store: S) -> Self {
        Self { registry, store }
    }

    pub fn grant(
        &mut self,
        request: &AccessRequest,
        lifetime: GrantLifetime,
        source: GrantSource,
        reason: Option<String>,
        granted_at: UnixTimestamp,
        expires_at: Option<UnixTimestamp>,
    ) -> Result<GrantId, PolicyStoreError> {
        self.insert_grant(
            request,
            GrantParameters {
                effect: GrantEffect::Allow,
                lifetime,
                source,
                reason,
                granted_at,
                expires_at,
            },
        )
    }

    pub fn deny(
        &mut self,
        request: &AccessRequest,
        lifetime: GrantLifetime,
        source: GrantSource,
        reason: Option<String>,
        granted_at: UnixTimestamp,
        expires_at: Option<UnixTimestamp>,
    ) -> Result<GrantId, PolicyStoreError> {
        self.insert_grant(
            request,
            GrantParameters {
                effect: GrantEffect::Deny,
                lifetime,
                source,
                reason,
                granted_at,
                expires_at,
            },
        )
    }

    fn insert_grant(
        &mut self,
        request: &AccessRequest,
        parameters: GrantParameters,
    ) -> Result<GrantId, PolicyStoreError> {
        request
            .scope
            .validate()
            .map_err(|error| PolicyStoreError::Malformed(error.to_string()))?;
        if !self.registry.contains(&request.capability) {
            return Err(PolicyStoreError::Malformed(
                "cannot grant an unregistered capability".into(),
            ));
        }
        if parameters.effect == GrantEffect::Deny && parameters.lifetime == GrantLifetime::OneShot {
            return Err(PolicyStoreError::Malformed(
                "one-shot lifetime is reserved for allow grants".into(),
            ));
        }
        self.store.transact(|document| {
            let id = document.allocate_grant_id()?;
            let grant = Grant {
                id,
                principal: request.principal.clone(),
                capability: request.capability.clone(),
                effect: parameters.effect,
                scope: request.scope.clone(),
                lifetime: parameters.lifetime,
                source: parameters.source,
                reason: parameters.reason,
                granted_at: parameters.granted_at,
                expires_at: parameters.expires_at,
                revoked_at: None,
                consumed_at: None,
            };
            grant.validate().map_err(PolicyStoreError::InvalidGrant)?;
            document.push_grant(grant);
            Ok(id)
        })
    }

    pub fn revoke(
        &mut self,
        grant_id: GrantId,
        revoked_at: UnixTimestamp,
    ) -> Result<bool, PolicyStoreError> {
        self.store.transact(|document| {
            let Some(grant) = document.grant_mut(grant_id) else {
                return Ok(false);
            };
            if grant.revoked_at.is_some() || revoked_at < grant.granted_at {
                return Ok(false);
            }
            grant.revoked_at = Some(revoked_at);
            Ok(true)
        })
    }

    /// Checks access and atomically consumes an allowed one-shot grant.
    /// Storage errors are returned as a deny decision.
    pub fn check(&mut self, request: &AccessRequest, context: &EvaluationContext) -> Decision {
        if let Some(decision) = self.validate_request(request) {
            return decision;
        }
        let registry = &self.registry;
        let result = self.store.transact(|document| {
            document.validate()?;
            let (decision, consume) = evaluate(registry, document, request, context);
            if let Some(grant_id) = consume {
                document.consume_one_shot(grant_id, context.now)?;
            }
            Ok(decision)
        });
        result.unwrap_or_else(store_failure)
    }

    /// Explains the current policy result without consuming a one-shot grant.
    pub fn explain(&self, request: &AccessRequest, context: &EvaluationContext) -> Decision {
        if let Some(decision) = self.validate_request(request) {
            return decision;
        }
        let document = match self.store.snapshot() {
            Ok(document) => document,
            Err(error) => return store_failure(error),
        };
        if let Err(error) = document.validate() {
            return store_failure(error);
        }
        evaluate(&self.registry, &document, request, context).0
    }

    pub fn list_effective_grants(
        &self,
        principal: &PrincipalId,
        context: &EvaluationContext,
    ) -> Result<Vec<Grant>, PolicyStoreError> {
        let document = self.store.snapshot()?;
        document.validate()?;
        Ok(document
            .grants()
            .iter()
            .filter(|grant| {
                &grant.principal == principal
                    && self.registry.contains(&grant.capability)
                    && grant.is_active_at(context) == ActiveGrant::Active
            })
            .cloned()
            .collect())
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    fn validate_request(&self, request: &AccessRequest) -> Option<Decision> {
        if request.scope.validate().is_err() {
            return Some(Decision::deny(
                DecisionReason::InvalidRequest,
                "Denied because the requested resource scope is malformed.",
            ));
        }
        if !self.registry.contains(&request.capability) {
            return Some(Decision::deny(
                DecisionReason::UnknownCapability,
                format!(
                    "Denied because capability {} is not registered.",
                    request.capability
                ),
            ));
        }
        None
    }
}

impl<S: PolicyStore> CapabilityEnforcer for PermissionEvaluator<S> {
    fn authorize(&mut self, request: &AccessRequest, context: &EvaluationContext) -> Decision {
        self.check(request, context)
    }
}

fn evaluate(
    registry: &CapabilityRegistry,
    document: &PolicyDocument,
    request: &AccessRequest,
    context: &EvaluationContext,
) -> (Decision, Option<GrantId>) {
    if !registry.contains(&request.capability) {
        return (
            Decision::deny(
                DecisionReason::UnknownCapability,
                format!(
                    "Denied because capability {} is not registered.",
                    request.capability
                ),
            ),
            None,
        );
    }

    let relevant: Vec<&Grant> = document
        .grants()
        .iter()
        .filter(|grant| {
            grant.principal == request.principal && grant.capability == request.capability
        })
        .collect();
    if relevant.is_empty() {
        return (
            Decision::deny(
                DecisionReason::NoGrant,
                "Denied by default because this principal has no grant for the capability.",
            ),
            None,
        );
    }

    let scoped: Vec<&Grant> = relevant
        .iter()
        .copied()
        .filter(|grant| grant.scope.covers(&request.scope))
        .collect();
    if scoped.is_empty() {
        return (
            Decision::deny(
                DecisionReason::ScopeMismatch,
                "Denied because no grant covers the requested resource scope.",
            ),
            None,
        );
    }

    if let Some(denial) = scoped.iter().copied().find(|grant| {
        grant.effect == GrantEffect::Deny && grant.is_active_at(context) == ActiveGrant::Active
    }) {
        return (Decision::deny_by_grant(denial), None);
    }

    let mut allows: Vec<&Grant> = scoped
        .iter()
        .copied()
        .filter(|grant| {
            grant.effect == GrantEffect::Allow && grant.is_active_at(context) == ActiveGrant::Active
        })
        .collect();
    allows.sort_by_key(|grant| {
        (
            grant.scope.specificity(),
            lifetime_preference(&grant.lifetime),
            grant.id,
        )
    });
    if let Some(allow) = allows.first().copied() {
        let consume = (allow.lifetime == GrantLifetime::OneShot).then_some(allow.id);
        return (Decision::allow(allow), consume);
    }

    let inactive = scoped
        .iter()
        .copied()
        .filter(|grant| grant.effect == GrantEffect::Allow)
        .map(|grant| grant.is_active_at(context))
        .collect::<Vec<_>>();
    let reason = if inactive.contains(&ActiveGrant::Revoked) {
        DecisionReason::Revoked
    } else if inactive.contains(&ActiveGrant::NotYetValid) {
        DecisionReason::NotYetValid
    } else if inactive.contains(&ActiveGrant::Expired) {
        DecisionReason::Expired
    } else if inactive.contains(&ActiveGrant::WrongSession) {
        DecisionReason::WrongSession
    } else if inactive.contains(&ActiveGrant::Consumed) {
        DecisionReason::OneShotConsumed
    } else {
        DecisionReason::NoGrant
    };
    let explanation = match reason {
        DecisionReason::Revoked => "Denied because the matching grant was revoked.",
        DecisionReason::NotYetValid => "Denied because the matching grant is not valid yet.",
        DecisionReason::Expired => "Denied because the matching grant has expired.",
        DecisionReason::WrongSession => {
            "Denied because the session grant belongs to a different session."
        }
        DecisionReason::OneShotConsumed => {
            "Denied because the matching one-shot grant was already consumed."
        }
        _ => "Denied by default because no active allow grant applies.",
    };
    (Decision::deny(reason, explanation), None)
}

fn lifetime_preference(lifetime: &GrantLifetime) -> u8 {
    match lifetime {
        GrantLifetime::Persistent => 0,
        GrantLifetime::Session { .. } => 1,
        GrantLifetime::OneShot => 2,
    }
}

fn store_failure(_error: PolicyStoreError) -> Decision {
    Decision::deny(
        DecisionReason::PolicyStoreUnavailable,
        "Denied because the policy store could not be read safely; no permission was granted.",
    )
}

impl fmt::Display for DecisionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{FailingPolicyStore, InMemoryPolicyStore};

    fn cap(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("capability")
    }

    fn principal(value: &str) -> PrincipalId {
        PrincipalId::new(value).expect("principal")
    }

    fn request(principal_id: &str, path: &str) -> AccessRequest {
        AccessRequest::new(
            principal(principal_id),
            cap("filesystem.read"),
            CapabilityScope::Filesystem { path: path.into() },
        )
    }

    fn registry() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        registry
            .register(crate::CapabilityDefinition {
                id: cap("filesystem.read"),
                description: "Read selected filesystem resources".into(),
            })
            .expect("register");
        registry
    }

    fn context(now: u64) -> EvaluationContext {
        EvaluationContext::new(UnixTimestamp::from_unix_seconds(now), None)
    }

    fn evaluator() -> PermissionEvaluator<InMemoryPolicyStore> {
        PermissionEvaluator::new(registry(), InMemoryPolicyStore::new())
    }

    #[test]
    fn unknown_capabilities_and_unrequested_access_are_denied_by_default() {
        let evaluator = evaluator();
        let unknown = AccessRequest::new(
            principal("app:example.notes"),
            cap("camera.capture"),
            CapabilityScope::Unscoped,
        );
        assert_eq!(
            evaluator.explain(&unknown, &context(100)).reason(),
            DecisionReason::UnknownCapability
        );
        assert_eq!(
            evaluator
                .explain(&request("app:example.notes", "/data/notes"), &context(100))
                .reason(),
            DecisionReason::NoGrant
        );
    }

    #[test]
    fn a_grant_allows_only_its_principal_and_covered_scope() {
        let mut evaluator = evaluator();
        let read_docs = request("app:example.notes", "/data/docs");
        evaluator
            .grant(
                &read_docs,
                GrantLifetime::Persistent,
                GrantSource::User,
                Some("User selected the documents directory".into()),
                UnixTimestamp::from_unix_seconds(10),
                None,
            )
            .expect("grant");
        assert!(evaluator
            .check(
                &request("app:example.notes", "/data/docs/today.txt"),
                &context(11)
            )
            .is_allowed());
        assert_eq!(
            evaluator
                .check(
                    &request("app:example.notes", "/data/private.txt"),
                    &context(11)
                )
                .reason(),
            DecisionReason::ScopeMismatch
        );
        assert_eq!(
            evaluator
                .check(
                    &request("app:other.notes", "/data/docs/today.txt"),
                    &context(11)
                )
                .reason(),
            DecisionReason::NoGrant
        );
    }

    #[test]
    fn explicit_deny_takes_precedence_over_a_matching_allow() {
        let mut evaluator = evaluator();
        let access = request("app:example.notes", "/data/docs/secret.txt");
        evaluator
            .grant(
                &request("app:example.notes", "/data/docs"),
                GrantLifetime::Persistent,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                None,
            )
            .expect("allow");
        evaluator
            .deny(
                &access,
                GrantLifetime::Persistent,
                GrantSource::Policy,
                Some("Sensitive file".into()),
                UnixTimestamp::from_unix_seconds(2),
                None,
            )
            .expect("deny");
        let decision = evaluator.check(&access, &context(3));
        assert_eq!(decision.reason(), DecisionReason::ExplicitlyDenied);
        assert!(!decision.is_allowed());
    }

    #[test]
    fn revoke_removes_a_previously_effective_allow() {
        let mut evaluator = evaluator();
        let access = request("app:example.notes", "/data/docs");
        let id = evaluator
            .grant(
                &access,
                GrantLifetime::Persistent,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                None,
            )
            .expect("grant");
        assert!(evaluator.check(&access, &context(2)).is_allowed());
        assert!(evaluator
            .revoke(id, UnixTimestamp::from_unix_seconds(3))
            .expect("revoke"));
        assert_eq!(
            evaluator.check(&access, &context(4)).reason(),
            DecisionReason::Revoked
        );
    }

    #[test]
    fn one_shot_grant_is_consumed_once_and_explain_is_read_only() {
        let mut evaluator = evaluator();
        let access = request("app:example.notes", "/data/docs");
        evaluator
            .grant(
                &access,
                GrantLifetime::OneShot,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                None,
            )
            .expect("grant");
        assert!(evaluator.explain(&access, &context(2)).is_allowed());
        assert!(evaluator.check(&access, &context(2)).is_allowed());
        assert_eq!(
            evaluator.check(&access, &context(3)).reason(),
            DecisionReason::OneShotConsumed
        );
    }

    #[test]
    fn session_grant_is_bound_to_the_named_session_and_expiry_is_enforced() {
        let mut evaluator = evaluator();
        let access = request("app:example.notes", "/data/docs");
        evaluator
            .grant(
                &access,
                GrantLifetime::Session {
                    session_id: SessionId::new("session:one").expect("session"),
                },
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                Some(UnixTimestamp::from_unix_seconds(10)),
            )
            .expect("grant");
        let matching = EvaluationContext::new(
            UnixTimestamp::from_unix_seconds(2),
            Some(SessionId::new("session:one").expect("session")),
        );
        assert!(evaluator.check(&access, &matching).is_allowed());
        assert_eq!(
            evaluator.check(&access, &context(2)).reason(),
            DecisionReason::WrongSession
        );
        let expired = EvaluationContext::new(
            UnixTimestamp::from_unix_seconds(10),
            Some(SessionId::new("session:one").expect("session")),
        );
        assert_eq!(
            evaluator.check(&access, &expired).reason(),
            DecisionReason::Expired
        );
    }

    #[test]
    fn future_dated_grant_does_not_authorize_before_its_creation_time() {
        let mut evaluator = evaluator();
        let access = request("app:example.notes", "/data/docs");
        evaluator
            .grant(
                &access,
                GrantLifetime::Persistent,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(50),
                None,
            )
            .expect("grant");
        assert_eq!(
            evaluator.check(&access, &context(49)).reason(),
            DecisionReason::NotYetValid
        );
        assert!(evaluator.check(&access, &context(50)).is_allowed());
    }

    #[test]
    fn explanation_identifies_the_reason_for_default_deny() {
        let evaluator = evaluator();
        let decision = evaluator.explain(&request("app:example.notes", "/data/docs"), &context(10));
        assert_eq!(decision.reason(), DecisionReason::NoGrant);
        assert!(decision.explanation().contains("Denied by default"));
    }

    #[test]
    fn policy_store_failure_fails_closed() {
        let mut evaluator = PermissionEvaluator::new(registry(), FailingPolicyStore);
        let decision = evaluator.check(&request("app:example.notes", "/data/docs"), &context(10));
        assert_eq!(decision.reason(), DecisionReason::PolicyStoreUnavailable);
        assert!(!decision.is_allowed());
        assert!(!decision.explanation().contains("test storage failure"));
    }

    #[test]
    fn effective_grant_listing_is_principal_and_context_scoped() {
        let mut evaluator = evaluator();
        evaluator
            .grant(
                &request("app:example.notes", "/data/docs"),
                GrantLifetime::Persistent,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                None,
            )
            .expect("grant");
        assert_eq!(
            evaluator
                .list_effective_grants(&principal("app:example.notes"), &context(2))
                .expect("list")
                .len(),
            1
        );
        assert!(evaluator
            .list_effective_grants(&principal("app:other.notes"), &context(2))
            .expect("list")
            .is_empty());
    }

    #[test]
    fn effective_grant_listing_excludes_unregistered_capabilities() {
        let policy = br#"{
          "schema_version": 1,
          "next_grant_id": 2,
          "grants": [{
            "id": 1,
            "principal": "app:example.notes",
            "capability": "camera.capture",
            "effect": "allow",
            "scope": {"kind": "unscoped"},
            "lifetime": {"kind": "persistent"},
            "source": "user",
            "granted_at": 1
          }]
        }"#;
        let store = InMemoryPolicyStore::from_json(policy).expect("valid stored policy");
        let evaluator = PermissionEvaluator::new(registry(), store);
        assert!(evaluator
            .list_effective_grants(&principal("app:example.notes"), &context(2))
            .expect("list")
            .is_empty());
    }

    #[test]
    fn invalid_request_scope_fails_closed() {
        let evaluator = evaluator();
        let malformed = AccessRequest::new(
            principal("app:example.notes"),
            cap("filesystem.read"),
            CapabilityScope::Filesystem {
                path: "/data/../private".into(),
            },
        );
        assert_eq!(
            evaluator.explain(&malformed, &context(2)).reason(),
            DecisionReason::InvalidRequest
        );
    }
}
