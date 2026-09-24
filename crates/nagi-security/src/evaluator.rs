use crate::{
    Actor, AuditEvent, DecisionReason, Evaluation, InvocationKind, PermissionDecision,
    PermissionStore, PolicyRequest, RiskClass,
};

pub trait AuditSink {
    fn record(&mut self, event: AuditEvent);
}

/// Deterministically evaluate one request against a read-only policy store.
/// The function performs no prompting, persistence, or side effects.
pub fn evaluate<S: PermissionStore + ?Sized>(request: &PolicyRequest, store: &S) -> Evaluation {
    if !actor_matches_invocation(request.actor, request.context.invocation) {
        return result(
            PermissionDecision::Deny,
            DecisionReason::ActorContextMismatch,
        );
    }
    let Some(risk) = request.capability.risk_class() else {
        return result(PermissionDecision::Deny, DecisionReason::UnknownCapability);
    };
    if request.context.invocation == InvocationKind::AiSuggestion {
        return result(
            PermissionDecision::Deny,
            DecisionReason::SuggestionCannotExecute,
        );
    }

    let policy = if request.context.invocation == InvocationKind::AiDelegated {
        evaluate_delegated(request, store)
    } else {
        evaluate_for_actor(request, request.actor, store)
    };
    if policy.decision != PermissionDecision::Allow {
        return policy;
    }

    if matches!(risk, RiskClass::Destructive | RiskClass::Privileged) {
        return result(
            PermissionDecision::Ask,
            DecisionReason::HighRiskConfirmationRequired,
        );
    }
    result(PermissionDecision::Allow, DecisionReason::AllowedByPolicy)
}

pub fn evaluate_and_record<S, A>(request: &PolicyRequest, store: &S, audit: &mut A) -> Evaluation
where
    S: PermissionStore + ?Sized,
    A: AuditSink + ?Sized,
{
    let evaluation = evaluate(request, store);
    audit.record(AuditEvent::from_evaluation(*request, evaluation));
    evaluation
}

fn evaluate_delegated<S: PermissionStore + ?Sized>(
    request: &PolicyRequest,
    store: &S,
) -> Evaluation {
    let Some(id) = request.context.delegation_id else {
        return result(PermissionDecision::Deny, DecisionReason::DelegationRequired);
    };
    let Some(delegation) = store.get_delegation(id) else {
        return result(PermissionDecision::Deny, DecisionReason::DelegationNotFound);
    };
    if delegation.agent != request.actor
        || delegation.capability != request.capability
        || !request.scope.is_within(delegation.scope)
    {
        return result(PermissionDecision::Deny, DecisionReason::DelegationMismatch);
    }
    if request.context.now >= delegation.expires_at {
        return result(PermissionDecision::Deny, DecisionReason::DelegationExpired);
    }
    if !request.context.foreground && !delegation.allow_background {
        return result(
            PermissionDecision::Deny,
            DecisionReason::DelegationDoesNotAllowBackground,
        );
    }

    let user_policy = evaluate_for_actor(request, Actor::User(delegation.user), store);
    if user_policy.decision == PermissionDecision::Allow {
        user_policy
    } else if user_policy.decision == PermissionDecision::Ask {
        result(
            PermissionDecision::Ask,
            DecisionReason::UserConfirmationRequired,
        )
    } else {
        result(
            PermissionDecision::Deny,
            DecisionReason::UserAuthorityNotGranted,
        )
    }
}

fn evaluate_for_actor<S: PermissionStore + ?Sized>(
    request: &PolicyRequest,
    actor: Actor,
    store: &S,
) -> Evaluation {
    let mut saw_capability = false;
    let mut saw_matching_scope = false;
    let mut blocked_by_background = false;
    let mut selected: Option<PermissionDecision> = None;

    for index in 0..store.permission_count() {
        let Some(grant) = store.permission_at(index) else {
            continue;
        };
        if grant.actor != actor || grant.capability != request.capability {
            continue;
        }
        saw_capability = true;
        if !request.scope.is_within(grant.scope) {
            continue;
        }
        saw_matching_scope = true;

        if grant.decision == PermissionDecision::Allow
            && !request.context.foreground
            && !grant.allow_background
        {
            blocked_by_background = true;
            continue;
        }
        selected = Some(match selected {
            Some(previous) => stricter(previous, grant.decision),
            None => grant.decision,
        });
    }

    match selected {
        Some(PermissionDecision::Deny) => {
            result(PermissionDecision::Deny, DecisionReason::ExplicitlyDenied)
        }
        Some(PermissionDecision::Ask) => result(
            PermissionDecision::Ask,
            DecisionReason::UserConfirmationRequired,
        ),
        Some(PermissionDecision::Allow) => {
            result(PermissionDecision::Allow, DecisionReason::AllowedByPolicy)
        }
        None if blocked_by_background => result(
            PermissionDecision::Deny,
            DecisionReason::BackgroundNotAllowed,
        ),
        None if saw_capability && !saw_matching_scope => {
            result(PermissionDecision::Deny, DecisionReason::ScopeNotGranted)
        }
        None => result(PermissionDecision::Deny, DecisionReason::NoMatchingPolicy),
    }
}

fn stricter(left: PermissionDecision, right: PermissionDecision) -> PermissionDecision {
    match (left, right) {
        (PermissionDecision::Deny, _) | (_, PermissionDecision::Deny) => PermissionDecision::Deny,
        (PermissionDecision::Ask, _) | (_, PermissionDecision::Ask) => PermissionDecision::Ask,
        _ => PermissionDecision::Allow,
    }
}

fn actor_matches_invocation(actor: Actor, invocation: InvocationKind) -> bool {
    matches!(
        (actor, invocation),
        (Actor::User(_), InvocationKind::UserDirect)
            | (Actor::SystemService(_), InvocationKind::SystemService)
            | (
                Actor::FirstPartyApp(_) | Actor::ThirdPartyApp(_),
                InvocationKind::Application
            )
            | (
                Actor::AiAgent(_),
                InvocationKind::AiSuggestion | InvocationKind::AiDelegated
            )
            | (Actor::BackgroundAutomation(_), InvocationKind::Automation)
    )
}

const fn result(decision: PermissionDecision, reason: DecisionReason) -> Evaluation {
    Evaluation { decision, reason }
}
