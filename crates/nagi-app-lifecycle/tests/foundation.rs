use nagi_app_lifecycle::{
    parse_app_package_manifest, AppVersion, LifecycleFailure, LifecycleFailureCategory,
    LifecycleReason, ManagedApplication, ManifestValidationErrorKind, APP_LIFECYCLE_EXTENSION_ID,
};
use nagi_sdk::app_contract::{
    AppIdentity, AppManifestContract, AppOrigin, AppStateVersion, CapabilityDecision,
    CapabilityRequest, ContractVersion, DisplayName, LifecycleEvent, LifecycleEventKind,
    LifecycleState, ResumeTarget, StateCompatibility, EN_US, SDK_CONTRACT_VERSION,
};
use nagi_sdk::{AppSessionId, NodeId};
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};
use std::str::FromStr;

fn base_manifest<'a>(capabilities: &'a [CapabilityRequest<'a>]) -> AppManifestContract<'a> {
    AppManifestContract {
        schema_version: 1,
        sdk_contract_version: SDK_CONTRACT_VERSION,
        identity: AppIdentity::new("com.example.notes", "1.2.0", AppOrigin::ThirdParty, None)
            .unwrap(),
        display_name: DisplayName {
            en_us: "Notes",
            ja_jp: None,
        },
        supported_locales: &[EN_US],
        icon: None,
        entrypoint: nagi_sdk::app_contract::AppEntrypoint {
            kind: nagi_sdk::app_contract::EntrypointKind::Native,
            target: "bin/notes.napp",
        },
        resources: &[],
        intents: &[],
        requested_capabilities: capabilities,
        background_services: &[],
        state: StateCompatibility {
            current: AppStateVersion(1),
            minimum_readable: AppStateVersion(1),
        },
    }
}

fn document(extension: Value) -> Value {
    json!({
        "extensions": {
            (APP_LIFECYCLE_EXTENSION_ID): extension
        }
    })
}

fn fixture(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

#[test]
fn application_versions_parse_and_use_semantic_precedence() {
    let release = AppVersion::from_str("1.0.0").unwrap();
    let prerelease = AppVersion::from_str("1.0.0-rc.2").unwrap();
    let later_prerelease = AppVersion::from_str("1.0.0-rc.10").unwrap();
    assert!(prerelease < later_prerelease);
    assert!(later_prerelease < release);
    let build_a = AppVersion::from_str("1.0.0+build.7").unwrap();
    let build_b = AppVersion::from_str("1.0.0+build.8").unwrap();
    assert_eq!(build_a, build_b);
    assert_eq!(build_a.cmp(&build_b), std::cmp::Ordering::Equal);
    let mut hash_a = std::collections::hash_map::DefaultHasher::new();
    let mut hash_b = std::collections::hash_map::DefaultHasher::new();
    build_a.hash(&mut hash_a);
    build_b.hash(&mut hash_b);
    assert_eq!(hash_a.finish(), hash_b.finish());
    assert!(AppVersion::from_str("01.0.0").is_err());
    assert!(AppVersion::from_str("1.0").is_err());
    assert!(AppVersion::from_str("1.0.0-01").is_err());
}

#[test]
fn package_manifest_accepts_optional_capabilities_services_and_runtime_floor() {
    let contract = base_manifest(&[CapabilityRequest {
        id: "storage.read",
        purpose_key: None,
    }]);
    let parsed = parse_app_package_manifest(
        &fixture(include_str!("../fixtures/manifest-rich.json")),
        contract,
    )
    .unwrap();

    assert_eq!(
        parsed.manifest.identity.app_id(),
        nagi_sdk::AppId::from_identifier(b"com.example.notes")
    );
    assert_eq!(
        parsed
            .lifecycle
            .minimum_nagi_version
            .as_ref()
            .unwrap()
            .to_string(),
        "0.2.0"
    );
    assert_eq!(parsed.lifecycle.optional_capabilities, ["network.client"]);
    assert_eq!(parsed.lifecycle.required_services.len(), 1);
    assert_eq!(parsed.version.to_string(), "1.2.0");
    assert_eq!(
        parsed.lifecycle.metadata["com.example.notes.channel"],
        "stable"
    );
    assert!(parsed
        .lifecycle
        .supports_runtime(&AppVersion::from_str("0.2.1").unwrap()));
    assert!(!parsed
        .lifecycle
        .supports_runtime(&AppVersion::from_str("0.1.9").unwrap()));
    assert!(parsed.lifecycle.required_services[0]
        .is_compatible_with(ContractVersion { major: 1, minor: 3 }));
    assert!(!parsed.lifecycle.required_services[0]
        .is_compatible_with(ContractVersion { major: 2, minor: 0 }));
}

#[test]
fn minimal_and_absent_extensions_are_accepted() {
    let minimal = parse_app_package_manifest(
        &fixture(include_str!("../fixtures/manifest-minimal.json")),
        base_manifest(&[]),
    )
    .unwrap();
    assert_eq!(minimal.lifecycle.schema_version, 1);
    assert!(minimal.lifecycle.required_services.is_empty());

    let no_extension = parse_app_package_manifest(&json!({}), base_manifest(&[])).unwrap();
    assert_eq!(no_extension.lifecycle.schema_version, 1);
    assert!(no_extension.lifecycle.optional_capabilities.is_empty());
}

#[test]
fn manifest_reports_missing_fields_and_invalid_versions_structurally() {
    let missing = parse_app_package_manifest(&document(json!({})), base_manifest(&[])).unwrap_err();
    assert_eq!(missing.kind, ManifestValidationErrorKind::MissingField);
    assert_eq!(missing.field, "schemaVersion");
    assert_eq!(missing.index, None);

    let invalid_runtime = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "minimumNagiVersion": "0.2"
        })),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        invalid_runtime.kind,
        ManifestValidationErrorKind::InvalidVersion
    );
    assert_eq!(invalid_runtime.field, "minimumNagiVersion");
    assert_eq!(invalid_runtime.kind.identifier(), "APP_LC_INVALID_VERSION");

    let mut invalid_entrypoint = base_manifest(&[]);
    invalid_entrypoint.entrypoint.target = "../notes.napp";
    let invalid_base = parse_app_package_manifest(
        &fixture(include_str!("../fixtures/manifest-minimal.json")),
        invalid_entrypoint,
    )
    .unwrap_err();
    assert_eq!(
        invalid_base.kind,
        ManifestValidationErrorKind::InvalidBaseManifest
    );
}

#[test]
fn unrelated_namespaced_extensions_are_left_for_their_owners() {
    let doc = json!({
        "extensions": {
            "com.example.vendor": {"schemaVersion": 99, "opaque": [1, 2, 3]}
        }
    });
    let parsed = parse_app_package_manifest(&doc, base_manifest(&[])).unwrap();
    assert!(parsed.lifecycle.optional_capabilities.is_empty());
}

#[test]
fn package_manifest_rejects_duplicate_and_contradictory_declarations() {
    let required = [CapabilityRequest {
        id: "storage.read",
        purpose_key: None,
    }];
    let duplicate_optional = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "optionalCapabilities": ["network.client", "network.client"]
        })),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_optional.kind,
        ManifestValidationErrorKind::DuplicateDeclaration
    );

    let conflict = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "optionalCapabilities": ["storage.read"]
        })),
        base_manifest(&required),
    )
    .unwrap_err();
    assert_eq!(
        conflict.kind,
        ManifestValidationErrorKind::ContradictoryDeclaration
    );

    let duplicate_service = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "requiredServices": [
                {"id": "org.nagi.files", "contractVersion": {"major": 1, "minor": 0}},
                {"id": "org.nagi.files", "contractVersion": {"major": 1, "minor": 1}}
            ]
        })),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_service.kind,
        ManifestValidationErrorKind::DuplicateDeclaration
    );
}

#[test]
fn package_manifest_rejects_unsupported_versions_unknown_fields_and_unsafe_ids() {
    let unsupported =
        parse_app_package_manifest(&document(json!({"schemaVersion": 2})), base_manifest(&[]))
            .unwrap_err();
    assert_eq!(
        unsupported.kind,
        ManifestValidationErrorKind::UnsupportedVersion
    );

    let unknown = parse_app_package_manifest(
        &document(json!({"schemaVersion": 1, "silentlyIgnored": true})),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(unknown.kind, ManifestValidationErrorKind::UnknownField);

    let unsafe_service = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "requiredServices": [{
                "id": "../files",
                "contractVersion": {"major": 1, "minor": 0}
            }]
        })),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        unsafe_service.kind,
        ManifestValidationErrorKind::InvalidValue
    );

    let invalid_capability = parse_app_package_manifest(
        &document(json!({"schemaVersion": 1, "optionalCapabilities": ["../files"]})),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        invalid_capability.kind,
        ManifestValidationErrorKind::InvalidValue
    );

    let invalid_service_version = parse_app_package_manifest(
        &document(json!({
            "schemaVersion": 1,
            "requiredServices": [{
                "id": "org.nagi.files",
                "contractVersion": {"major": 65_536, "minor": 0}
            }]
        })),
        base_manifest(&[]),
    )
    .unwrap_err();
    assert_eq!(
        invalid_service_version.kind,
        ManifestValidationErrorKind::InvalidValue
    );
}

#[test]
fn lifecycle_observation_includes_app_identity_and_structured_failure() {
    let identity = base_manifest(&[]).identity;
    let mut app = ManagedApplication::new(identity).unwrap();
    let launch = app
        .apply(nagi_sdk::app_contract::LifecycleEvent {
            kind: nagi_sdk::app_contract::LifecycleEventKind::LaunchRequested,
            session_id: AppSessionId(7),
            node_id: Some(NodeId(3)),
            sequence: Some(1),
        })
        .unwrap();
    assert_eq!(launch.app_id, identity.app_id());
    assert_eq!(launch.reason, LifecycleReason::LaunchRequested);
    assert_eq!(
        launch.previous_state,
        nagi_sdk::app_contract::LifecycleState::Registered
    );
    assert_eq!(
        launch.new_state,
        nagi_sdk::app_contract::LifecycleState::Launching
    );
    assert_eq!(launch.failure, None);

    let crash = app
        .apply(nagi_sdk::app_contract::LifecycleEvent {
            kind: nagi_sdk::app_contract::LifecycleEventKind::AbnormalTermination {
                reason_code: 42,
            },
            session_id: AppSessionId(7),
            node_id: Some(NodeId(3)),
            sequence: Some(2),
        })
        .unwrap();
    assert_eq!(crash.reason, LifecycleReason::AbnormalTermination);
    assert_eq!(
        crash.failure,
        Some(LifecycleFailure {
            category: LifecycleFailureCategory::AbnormalTermination,
            reason_code: 42,
        })
    );
    assert_eq!(crash.session_id, AppSessionId(7));
    assert_eq!(crash.sequence, Some(2));
}

#[test]
fn lifecycle_launch_suspend_resume_termination_flow_is_observable() {
    let identity = base_manifest(&[]).identity;
    let mut app = ManagedApplication::new(identity).unwrap();
    let mut sequence = 1;
    let mut apply = |kind| {
        let observation = app
            .apply(LifecycleEvent {
                kind,
                session_id: AppSessionId(11),
                node_id: Some(NodeId(4)),
                sequence: Some(sequence),
            })
            .unwrap();
        sequence += 1;
        observation
    };

    assert_eq!(
        apply(LifecycleEventKind::LaunchRequested).new_state,
        LifecycleState::Launching
    );
    assert_eq!(
        apply(LifecycleEventKind::CapabilitiesResolved(
            CapabilityDecision::NotRequested
        ))
        .new_state,
        LifecycleState::Launching
    );
    assert_eq!(
        apply(LifecycleEventKind::Ready).new_state,
        LifecycleState::Ready
    );
    assert_eq!(
        apply(LifecycleEventKind::Activate).new_state,
        LifecycleState::Foreground
    );
    assert_eq!(
        apply(LifecycleEventKind::Suspended {
            checkpoint: Some(8)
        })
        .new_state,
        LifecycleState::Suspended
    );
    assert_eq!(
        apply(LifecycleEventKind::Resumed {
            target: ResumeTarget::Background,
            checkpoint: Some(8),
        })
        .new_state,
        LifecycleState::Background
    );
    assert_eq!(
        apply(LifecycleEventKind::TerminationRequested).new_state,
        LifecycleState::Terminating
    );
    assert_eq!(
        apply(LifecycleEventKind::ShutdownHookCompleted).new_state,
        LifecycleState::Terminating
    );
    let terminated = apply(LifecycleEventKind::Terminated);
    assert_eq!(terminated.previous_state, LifecycleState::Terminating);
    assert_eq!(terminated.new_state, LifecycleState::Terminated);
    assert_eq!(terminated.app_id, identity.app_id());
}

#[test]
fn repeated_lifecycle_requests_are_rejected_without_mutating_state() {
    let mut app = ManagedApplication::new(base_manifest(&[]).identity).unwrap();
    let launch = |sequence| LifecycleEvent {
        kind: LifecycleEventKind::LaunchRequested,
        session_id: AppSessionId(15),
        node_id: None,
        sequence: Some(sequence),
    };
    app.apply(launch(1)).unwrap();
    let error = app.apply(launch(2)).unwrap_err();
    assert_eq!(error.code.identifier(), "APP_INVALID_LIFECYCLE_TRANSITION");
    assert_eq!(app.state(), LifecycleState::Launching);
}

#[test]
fn invalid_lifecycle_events_remain_rejected_by_the_canonical_sdk_machine() {
    let mut app = ManagedApplication::new(base_manifest(&[]).identity).unwrap();
    let error = app
        .apply(nagi_sdk::app_contract::LifecycleEvent {
            kind: nagi_sdk::app_contract::LifecycleEventKind::Ready,
            session_id: AppSessionId(9),
            node_id: None,
            sequence: None,
        })
        .unwrap_err();
    assert_eq!(
        error.code,
        nagi_sdk::app_contract::ErrorCode::InvalidLifecycleTransition
    );
    assert_eq!(
        app.state(),
        nagi_sdk::app_contract::LifecycleState::Registered
    );
    assert_eq!(error.code.identifier(), "APP_INVALID_LIFECYCLE_TRANSITION");
}
