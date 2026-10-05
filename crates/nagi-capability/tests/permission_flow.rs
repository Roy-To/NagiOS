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

/// ADR-0012: clipboard.read and clipboard.write are independent, unscoped,
/// and default-deny until a trusted host registers and grants them.
#[test]
fn clipboard_read_and_write_are_independent_default_deny_permissions() {
    let read = CapabilityId::new("clipboard.read").expect("clipboard.read");
    let write = CapabilityId::new("clipboard.write").expect("clipboard.write");
    let reader = PrincipalId::new("app:com.example.reader").expect("reader");
    let writer = PrincipalId::new("app:com.example.writer").expect("writer");
    let request = |principal: &PrincipalId, capability: &CapabilityId| {
        AccessRequest::new(
            principal.clone(),
            capability.clone(),
            CapabilityScope::Unscoped,
        )
    };
    let context = EvaluationContext::new(UnixTimestamp::from_unix_seconds(100), None);

    let mut unregistered =
        PermissionEvaluator::new(CapabilityRegistry::new(), InMemoryPolicyStore::new());
    assert_eq!(
        unregistered
            .check(&request(&writer, &write), &context)
            .reason(),
        DecisionReason::UnknownCapability
    );

    let mut registry = CapabilityRegistry::new();
    for (id, description) in [
        (&read, "Read clipboard formats and content"),
        (&write, "Replace or clear clipboard content"),
    ] {
        registry
            .register(CapabilityDefinition {
                id: id.clone(),
                description: description.into(),
            })
            .expect("register clipboard permission");
    }
    let mut evaluator = PermissionEvaluator::new(registry, InMemoryPolicyStore::new());
    for principal in [&reader, &writer] {
        for capability in [&read, &write] {
            assert_eq!(
                evaluator
                    .check(&request(principal, capability), &context)
                    .reason(),
                DecisionReason::NoGrant
            );
        }
    }

    for (principal, capability) in [(&reader, &read), (&writer, &write)] {
        evaluator
            .grant(
                &request(principal, capability),
                GrantLifetime::Persistent,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(90),
                None,
            )
            .expect("grant clipboard permission");
    }
    assert!(evaluator
        .authorize(&request(&reader, &read), &context)
        .is_allowed());
    assert!(evaluator
        .authorize(&request(&writer, &write), &context)
        .is_allowed());
    // Neither permission implies the other.
    assert_eq!(
        evaluator
            .check(&request(&reader, &write), &context)
            .reason(),
        DecisionReason::NoGrant
    );
    assert_eq!(
        evaluator.check(&request(&writer, &read), &context).reason(),
        DecisionReason::NoGrant
    );
    // There is no separate clear permission; it is part of clipboard.write.
    let clear = CapabilityId::new("clipboard.clear").expect("syntactically valid");
    assert_eq!(
        evaluator
            .check(&request(&writer, &clear), &context)
            .reason(),
        DecisionReason::UnknownCapability
    );
}
