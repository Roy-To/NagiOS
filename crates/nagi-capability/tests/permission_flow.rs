use nagi_capability::{
    bind_declaration, AccessRequest, CapabilityDefinition, CapabilityEnforcer, CapabilityId,
    CapabilityRegistry, CapabilityScope, DecisionReason, EvaluationContext, GrantLifetime,
    GrantSource, InMemoryPolicyStore, PermissionEvaluator, PrincipalId, UnixTimestamp,
};

#[test]
fn manifest_to_grant_check_revoke_flow_and_principal_isolation() {
    let manifest = br#"{
      "schema_version": 1,
      "requested_capabilities": [{
        "capability": "filesystem.read",
        "scope": {"kind": "filesystem", "path": "/Documents"},
        "purpose": "Open a user-selected document"
      }]
    }"#;
    let declaration = nagi_capability::CapabilityDeclaration::parse_json(manifest)
        .expect("manifest declaration validation");
    let app = PrincipalId::new("app:com.example.notes").expect("app ID");
    let requests = bind_declaration(&declaration, &app).expect("bind trusted package identity");
    let request = AccessRequest::new(
        requests[0].principal.clone(),
        requests[0].capability.clone(),
        requests[0].scope.clone(),
    );

    let mut registry = CapabilityRegistry::new();
    registry
        .register(CapabilityDefinition {
            id: request.capability.clone(),
            description: "Read a filesystem resource".into(),
        })
        .expect("register capability");
    let mut evaluator = PermissionEvaluator::new(registry, InMemoryPolicyStore::new());
    let other_app_access = AccessRequest::new(
        PrincipalId::new("app:com.example.reader").expect("other app ID"),
        request.capability.clone(),
        request.scope.clone(),
    );
    let context = EvaluationContext::new(UnixTimestamp::from_unix_seconds(100), None);

    assert_eq!(
        evaluator.check(&request, &context).reason(),
        DecisionReason::NoGrant
    );
    let grant_id = evaluator
        .grant(
            &request,
            GrantLifetime::Persistent,
            GrantSource::User,
            Some("User approved this app's document access".into()),
            UnixTimestamp::from_unix_seconds(90),
            None,
        )
        .expect("explicit user grant");
    assert!(evaluator.authorize(&request, &context).is_allowed());
    assert_eq!(
        evaluator.check(&other_app_access, &context).reason(),
        DecisionReason::NoGrant
    );
    assert!(evaluator
        .revoke(grant_id, UnixTimestamp::from_unix_seconds(101))
        .expect("revoke"));
    assert_eq!(
        evaluator.check(&request, &context).reason(),
        DecisionReason::Revoked
    );
}

#[test]
fn persisted_policy_round_trip_preserves_versioned_grants() {
    let capability = CapabilityId::new("network.client").expect("capability");
    let mut registry = CapabilityRegistry::new();
    registry
        .register(CapabilityDefinition {
            id: capability.clone(),
            description: "Connect to a declared network origin".into(),
        })
        .expect("register");
    let mut evaluator = PermissionEvaluator::new(registry, InMemoryPolicyStore::new());
    let request = AccessRequest::new(
        PrincipalId::new("service:sync").expect("principal"),
        capability,
        CapabilityScope::NetworkOrigin {
            origin: "https://sync.example".into(),
        },
    );
    evaluator
        .grant(
            &request,
            GrantLifetime::Persistent,
            GrantSource::User,
            None,
            UnixTimestamp::from_unix_seconds(10),
            None,
        )
        .expect("grant");
    let serialized = evaluator.store().to_json().expect("serialize");
    let restored = InMemoryPolicyStore::from_json(serialized.as_bytes()).expect("restore");
    assert_eq!(restored.to_json().expect("serialize"), serialized);
}
