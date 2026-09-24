use crate::*;
use nagi_model::{AppId, ObjectId, UserId};

fn capability(name: &str) -> CapabilityId {
    CapabilityId::parse(name.as_bytes()).expect("valid capability")
}

fn app() -> Actor {
    Actor::ThirdPartyApp(AppId(41))
}

fn request(actor: Actor, name: &str, scope: CapabilityScope) -> PolicyRequest {
    PolicyRequest {
        actor,
        capability: capability(name),
        scope,
        context: EvaluationContext {
            invocation: match actor {
                Actor::User(_) => InvocationKind::UserDirect,
                Actor::SystemService(_) => InvocationKind::SystemService,
                Actor::FirstPartyApp(_) | Actor::ThirdPartyApp(_) => InvocationKind::Application,
                Actor::AiAgent(_) => InvocationKind::AiDelegated,
                Actor::BackgroundAutomation(_) => InvocationKind::Automation,
            },
            foreground: true,
            user_initiated: true,
            now: 10,
            delegation_id: None,
            correlation_id: ActionCorrelationId(99),
        },
    }
}

fn grant(
    actor: Actor,
    name: &str,
    scope: CapabilityScope,
    decision: PermissionDecision,
    allow_background: bool,
) -> PermissionGrant {
    PermissionGrant {
        actor,
        capability: capability(name),
        scope,
        decision,
        allow_background,
    }
}

#[test]
fn missing_policy_defaults_to_deny_and_unknown_capabilities_fail_closed() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    let result = evaluate(&request(app(), "files.read", CapabilityScope::Any), &store);
    assert_eq!(result.decision, PermissionDecision::Deny);
    assert_eq!(result.reason, DecisionReason::NoMatchingPolicy);

    let unknown = CapabilityId::parse(b"vendor.experimental.read").expect("well-formed name");
    let mut request = request(app(), "files.read", CapabilityScope::Any);
    request.capability = unknown;
    store
        .set_permission(PermissionGrant {
            actor: app(),
            capability: unknown,
            scope: CapabilityScope::Any,
            decision: PermissionDecision::Allow,
            allow_background: true,
        })
        .expect("store explicit unknown row");
    assert_eq!(
        evaluate(&request, &store).reason,
        DecisionReason::UnknownCapability
    );
}

#[test]
fn explicit_allow_deny_and_ask_are_distinct() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    for (name, decision) in [
        ("files.read", PermissionDecision::Allow),
        ("files.write", PermissionDecision::Deny),
        ("microphone.capture", PermissionDecision::Ask),
    ] {
        store
            .set_permission(grant(app(), name, CapabilityScope::Any, decision, false))
            .expect("store decision");
    }
    assert_eq!(
        evaluate(&request(app(), "files.read", CapabilityScope::Any), &store).decision,
        PermissionDecision::Allow
    );
    assert_eq!(
        evaluate(&request(app(), "files.write", CapabilityScope::Any), &store).decision,
        PermissionDecision::Deny
    );
    assert_eq!(
        evaluate(
            &request(app(), "microphone.capture", CapabilityScope::Any),
            &store
        )
        .decision,
        PermissionDecision::Ask
    );
}

#[test]
fn conflicting_matching_policies_choose_the_most_restrictive_decision() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Ask,
            true,
        ))
        .expect("broad ask");
    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Object(ObjectId(4)),
            PermissionDecision::Allow,
            true,
        ))
        .expect("scoped allow");
    let object_request = request(app(), "files.read", CapabilityScope::Object(ObjectId(4)));
    assert_eq!(
        evaluate(&object_request, &store).decision,
        PermissionDecision::Ask
    );

    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Object(ObjectId(4)),
            PermissionDecision::Deny,
            true,
        ))
        .expect("scoped deny replaces scoped allow");
    assert_eq!(
        evaluate(&object_request, &store).decision,
        PermissionDecision::Deny
    );
}

#[test]
fn scope_matching_is_exact_or_narrower_and_never_widens_a_grant() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    let object = ObjectId(12);
    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Object(object),
            PermissionDecision::Allow,
            false,
        ))
        .expect("store scoped decision");

    assert_eq!(
        evaluate(
            &request(app(), "files.read", CapabilityScope::Object(object)),
            &store
        )
        .decision,
        PermissionDecision::Allow
    );
    assert_eq!(
        evaluate(
            &request(app(), "files.read", CapabilityScope::Object(ObjectId(13))),
            &store
        )
        .decision,
        PermissionDecision::Deny
    );
    assert_eq!(
        evaluate(
            &request(app(), "files.read", CapabilityScope::Object(ObjectId(13))),
            &store
        )
        .reason,
        DecisionReason::ScopeNotGranted
    );

    store
        .set_permission(grant(
            app(),
            "files.write",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            false,
        ))
        .expect("store broad decision");
    assert_eq!(
        evaluate(
            &request(app(), "files.write", CapabilityScope::Object(object)),
            &store
        )
        .decision,
        PermissionDecision::Allow
    );

    let root = ObjectId(40);
    store
        .set_permission(grant(
            app(),
            "contacts.read",
            CapabilityScope::Directory(root),
            PermissionDecision::Allow,
            false,
        ))
        .expect("directory grant");
    assert_eq!(
        evaluate(
            &request(
                app(),
                "contacts.read",
                CapabilityScope::ObjectWithinDirectory {
                    root_directory: root,
                    object: ObjectId(41),
                },
            ),
            &store,
        )
        .decision,
        PermissionDecision::Allow
    );
    assert_eq!(
        evaluate(
            &request(
                app(),
                "contacts.read",
                CapabilityScope::ObjectWithinDirectory {
                    root_directory: ObjectId(42),
                    object: ObjectId(41),
                },
            ),
            &store,
        )
        .decision,
        PermissionDecision::Deny
    );
}

#[test]
fn malformed_capability_names_are_rejected() {
    assert_eq!(CapabilityId::parse(b""), Err(IdentifierError::Empty));
    for value in [b"Files.read".as_slice(), b"files..read", b"files/../read"] {
        assert_eq!(CapabilityId::parse(value), Err(IdentifierError::Invalid));
    }
}

#[test]
fn revoking_a_permission_returns_evaluation_to_default_deny() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    let permission = grant(
        app(),
        "files.read",
        CapabilityScope::Any,
        PermissionDecision::Allow,
        false,
    );
    let key = permission.key();
    store.set_permission(permission).expect("store");
    assert_eq!(
        store.current_decision(&key),
        Some(PermissionDecision::Allow)
    );
    assert!(store.revoke_permission(&key));
    assert_eq!(store.permission_count(), 0);
    assert_eq!(store.permission_at(0), None);
    assert_eq!(
        evaluate(&request(app(), "files.read", CapabilityScope::Any), &store).decision,
        PermissionDecision::Deny
    );
}

#[test]
fn audit_event_carries_policy_result_time_and_correlation_id() {
    struct Capture(Option<AuditEvent>);
    impl AuditSink for Capture {
        fn record(&mut self, event: AuditEvent) {
            self.0 = Some(event);
        }
    }

    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            false,
        ))
        .expect("permission");
    let policy_request = request(app(), "files.read", CapabilityScope::Any);
    let mut sink = Capture(None);
    let decision = evaluate_and_record(&policy_request, &store, &mut sink);
    assert_eq!(decision.decision, PermissionDecision::Allow);
    let event = sink.0.expect("audit event");
    assert_eq!(event.actor, policy_request.actor);
    assert_eq!(event.capability, policy_request.capability);
    assert_eq!(event.requested_scope, policy_request.scope);
    assert_eq!(event.reason, DecisionReason::AllowedByPolicy);
    assert_eq!(event.timestamp, 10);
    assert_eq!(event.correlation_id, ActionCorrelationId(99));
}

#[test]
fn permission_grant_serialization_round_trips() {
    let domain = DomainName::parse(b"example.com").expect("domain");
    let original = grant(
        app(),
        "network.access",
        CapabilityScope::Domain(domain),
        PermissionDecision::Allow,
        true,
    );
    let mut bytes = [0; MAX_ENCODED_GRANT_BYTES];
    let length = encode_permission_grant(&original, &mut bytes).expect("encode");
    assert_eq!(
        decode_permission_grant(&bytes[..length]).expect("decode"),
        original
    );
}

#[test]
fn ai_requires_a_matching_explicit_delegation_and_user_grant() {
    let user = UserId(7);
    let agent = Actor::AiAgent(AppId(88));
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            Actor::User(user),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            false,
        ))
        .expect("user permission");
    store
        .set_delegation(DelegationGrant {
            id: DelegationId(5),
            user,
            agent,
            capability: capability("files.read"),
            scope: CapabilityScope::Any,
            expires_at: 100,
            allow_background: false,
        })
        .expect("delegation");

    let mut delegated = request(agent, "files.read", CapabilityScope::Object(ObjectId(3)));
    delegated.context.delegation_id = Some(DelegationId(5));
    assert_eq!(
        evaluate(&delegated, &store).decision,
        PermissionDecision::Allow
    );

    delegated.context.delegation_id = None;
    assert_eq!(
        evaluate(&delegated, &store).reason,
        DecisionReason::DelegationRequired
    );

    delegated.context.delegation_id = Some(DelegationId(5));
    let mut no_user_authority = InMemoryPermissionStore::<8, 2>::new();
    no_user_authority
        .set_delegation(DelegationGrant {
            id: DelegationId(5),
            user,
            agent,
            capability: capability("files.read"),
            scope: CapabilityScope::Any,
            expires_at: 100,
            allow_background: false,
        })
        .expect("delegation");
    assert_eq!(
        evaluate(&delegated, &no_user_authority).reason,
        DecisionReason::UserAuthorityNotGranted
    );
}

#[test]
fn ai_delegation_is_scope_bound_time_bound_and_separately_background_bound() {
    let user = UserId(7);
    let agent = Actor::AiAgent(AppId(88));
    let requested_object = ObjectId(3);
    let granted_object = ObjectId(4);
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            Actor::User(user),
            "files.read",
            CapabilityScope::Object(requested_object),
            PermissionDecision::Allow,
            true,
        ))
        .expect("user permission");
    store
        .set_delegation(DelegationGrant {
            id: DelegationId(5),
            user,
            agent,
            capability: capability("files.read"),
            scope: CapabilityScope::Object(requested_object),
            expires_at: 100,
            allow_background: false,
        })
        .expect("delegation");
    let mut delegated = request(
        agent,
        "files.read",
        CapabilityScope::Object(requested_object),
    );
    delegated.context.delegation_id = Some(DelegationId(5));
    assert_eq!(
        evaluate(&delegated, &store).decision,
        PermissionDecision::Allow
    );

    delegated.scope = CapabilityScope::Object(granted_object);
    assert_eq!(
        evaluate(&delegated, &store).reason,
        DecisionReason::DelegationMismatch
    );
    delegated.scope = CapabilityScope::Object(requested_object);
    delegated.context.now = 100;
    assert_eq!(
        evaluate(&delegated, &store).reason,
        DecisionReason::DelegationExpired
    );
    delegated.context.now = 10;
    delegated.context.foreground = false;
    assert_eq!(
        evaluate(&delegated, &store).reason,
        DecisionReason::DelegationDoesNotAllowBackground
    );

    store
        .set_delegation(DelegationGrant {
            id: DelegationId(5),
            user,
            agent,
            capability: capability("files.read"),
            scope: CapabilityScope::Object(requested_object),
            expires_at: 100,
            allow_background: true,
        })
        .expect("replace delegation");
    assert_eq!(
        evaluate(&delegated, &store).decision,
        PermissionDecision::Allow
    );
    assert!(store.revoke_delegation(DelegationId(5)));
    assert_eq!(
        evaluate(&delegated, &store).reason,
        DecisionReason::DelegationNotFound
    );
}

#[test]
fn background_actions_require_an_explicit_background_grant() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            false,
        ))
        .expect("foreground permission");
    let mut background = request(app(), "files.read", CapabilityScope::Any);
    background.context.foreground = false;
    assert_eq!(
        evaluate(&background, &store).reason,
        DecisionReason::BackgroundNotAllowed
    );

    store
        .set_permission(grant(
            app(),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            true,
        ))
        .expect("background permission");
    assert_eq!(
        evaluate(&background, &store).decision,
        PermissionDecision::Allow
    );
}

#[test]
fn ai_suggestion_cannot_execute_an_action() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    let agent = Actor::AiAgent(AppId(88));
    store
        .set_permission(grant(
            agent,
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            true,
        ))
        .expect("agent row");
    let mut suggestion = request(agent, "files.read", CapabilityScope::Any);
    suggestion.context.invocation = InvocationKind::AiSuggestion;
    assert_eq!(
        evaluate(&suggestion, &store).reason,
        DecisionReason::SuggestionCannotExecute
    );
}

#[test]
fn destructive_and_privileged_capabilities_require_action_confirmation() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    for name in ["files.delete", "system.administration"] {
        store
            .set_permission(grant(
                app(),
                name,
                CapabilityScope::Any,
                PermissionDecision::Allow,
                false,
            ))
            .expect("permission");
        let result = evaluate(&request(app(), name, CapabilityScope::Any), &store);
        assert_eq!(result.decision, PermissionDecision::Ask);
        assert_eq!(result.reason, DecisionReason::HighRiskConfirmationRequired);
    }
}

#[test]
fn system_service_and_application_permissions_are_separate() {
    let mut store = InMemoryPermissionStore::<8, 2>::new();
    store
        .set_permission(grant(
            Actor::SystemService(SystemServiceId(2)),
            "files.read",
            CapabilityScope::Any,
            PermissionDecision::Allow,
            false,
        ))
        .expect("system permission");
    assert_eq!(
        evaluate(
            &request(
                Actor::SystemService(SystemServiceId(2)),
                "files.read",
                CapabilityScope::Any
            ),
            &store
        )
        .decision,
        PermissionDecision::Allow
    );
    assert_eq!(
        evaluate(&request(app(), "files.read", CapabilityScope::Any), &store).decision,
        PermissionDecision::Deny
    );
}
