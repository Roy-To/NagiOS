use crate::{
    CapabilityDeclaration, CapabilityId, CapabilityScope, DeclarationError, PolicyDocument,
    PolicyStoreError, Principal, PrincipalId, PrincipalKind,
};

#[test]
fn capability_identifiers_require_a_lowercase_namespace() {
    for invalid in [
        "",
        "filesystem",
        "Filesystem.read",
        "filesystem.",
        ".read",
        "fs.read/write",
    ] {
        assert!(CapabilityId::new(invalid).is_err(), "accepted {invalid}");
    }
    assert!(CapabilityId::new("filesystem.read").is_ok());
}

#[test]
fn principal_ids_reject_whitespace_and_control_characters() {
    assert!(PrincipalId::new("app:com.example.notes").is_ok());
    assert!(PrincipalId::new("app:com example.notes").is_err());
    assert!(PrincipalId::new("app:com.example\nnotes").is_err());
}

#[test]
fn declaration_parse_rejects_invalid_identifier_and_unknown_fields() {
    let malformed_id =
        br#"{"schema_version":1,"requested_capabilities":[{"capability":"Camera.capture"}]}"#;
    assert!(CapabilityDeclaration::parse_json(malformed_id).is_err());
    let unknown_field = br#"{"schema_version":1,"requested_capabilities":[],"grant_all":true}"#;
    assert!(matches!(
        CapabilityDeclaration::parse_json(unknown_field),
        Err(DeclarationError::Malformed(_))
    ));
}

#[test]
fn declaration_schema_version_mismatch_is_reported() {
    let input = br#"{"schema_version":2,"requested_capabilities":[]}"#;
    assert_eq!(
        CapabilityDeclaration::parse_json(input),
        Err(DeclarationError::UnsupportedVersion(2))
    );
}

#[test]
fn manifest_declaration_does_not_choose_its_trusted_principal() {
    let declaration = CapabilityDeclaration::parse_json(
        br#"{"schema_version":1,"requested_capabilities":[{"capability":"filesystem.read","scope":{"kind":"filesystem","path":"/Documents"},"purpose":"Open a user-selected document"}]}"#,
    )
    .expect("declaration");
    let trusted_principal = PrincipalId::new("app:com.example.notes").expect("principal");
    let bound = crate::bind_declaration(&declaration, &trusted_principal).expect("host binding");
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].principal, trusted_principal);
    assert_eq!(
        bound[0].scope,
        CapabilityScope::Filesystem {
            path: "/Documents".into()
        }
    );
}

#[test]
fn principal_supports_ai_and_other_common_actor_kinds() {
    let principal = Principal {
        id: PrincipalId::new("agent:albert").expect("principal"),
        kind: PrincipalKind::AiMediatedAction,
        publisher_id: Some("nagi".into()),
        package_id: Some("org.nagi.albert".into()),
        display_name: Some("Albert".into()),
    };
    assert!(principal.validate().is_ok());
}

#[test]
fn malformed_persisted_policy_is_rejected_fail_closed() {
    assert!(matches!(
        PolicyDocument::from_json(b"{"),
        Err(PolicyStoreError::Malformed(_))
    ));

    let invalid_grant = br#"{
      "schema_version": 1,
      "next_grant_id": 2,
      "grants": [{
        "id": 1,
        "principal": "app:example.notes",
        "capability": "filesystem.read",
        "effect": "allow",
        "scope": {"kind": "filesystem", "path": "/data/../private"},
        "lifetime": {"kind": "persistent"},
        "source": "user",
        "reason": null,
        "granted_at": 10,
        "expires_at": null,
        "revoked_at": null,
        "consumed_at": null
      }]
    }"#;
    assert!(matches!(
        PolicyDocument::from_json(invalid_grant),
        Err(PolicyStoreError::InvalidGrant(_))
    ));

    let zero_grant_id = br#"{
      "schema_version": 1,
      "next_grant_id": 1,
      "grants": [{
        "id": 0,
        "principal": "app:example.notes",
        "capability": "filesystem.read",
        "effect": "allow",
        "scope": {"kind": "unscoped"},
        "lifetime": {"kind": "persistent"},
        "source": "user",
        "granted_at": 1
      }]
    }"#;
    assert!(matches!(
        PolicyDocument::from_json(zero_grant_id),
        Err(PolicyStoreError::Malformed(_))
    ));
}

#[test]
fn persisted_policy_schema_version_mismatch_is_rejected() {
    let input = br#"{"schema_version":2,"next_grant_id":1,"grants":[]}"#;
    assert_eq!(
        PolicyDocument::from_json(input),
        Err(PolicyStoreError::UnsupportedSchemaVersion(2))
    );
}

#[test]
fn persisted_policy_rejects_unknown_fields_and_duplicate_grant_ids() {
    let unknown_field = br#"{"schema_version":1,"next_grant_id":1,"grants":[],"allow_all":true}"#;
    assert!(matches!(
        PolicyDocument::from_json(unknown_field),
        Err(PolicyStoreError::Malformed(_))
    ));
    let duplicate_ids = br#"{
      "schema_version": 1,
      "next_grant_id": 2,
      "grants": [
        {"id":1,"principal":"app:a","capability":"filesystem.read","effect":"allow","scope":{"kind":"unscoped"},"lifetime":{"kind":"persistent"},"source":"user","granted_at":1},
        {"id":1,"principal":"app:b","capability":"filesystem.read","effect":"allow","scope":{"kind":"unscoped"},"lifetime":{"kind":"persistent"},"source":"user","granted_at":1}
      ]
    }"#;
    assert!(matches!(
        PolicyDocument::from_json(duplicate_ids),
        Err(PolicyStoreError::Malformed(_))
    ));
}

#[test]
fn optional_declaration_fields_are_omitted_from_serialized_output() {
    let declaration = CapabilityDeclaration {
        schema_version: 1,
        requested_capabilities: vec![crate::DeclaredCapability {
            capability: CapabilityId::new("clipboard.read").expect("capability"),
            scope: None,
            purpose: None,
        }],
    };
    let json = declaration.to_json().expect("serialize");
    assert!(!json.contains("scope"));
    assert!(!json.contains("purpose"));
}
