use crate::app_manifest::load_manifest;
use nagi_sdk::app_contract::{
    invoke_graceful_shutdown, resolve_capabilities, restore_state, validate_intent, AppError,
    AppIdentity, AppRegistry, AppStateBackend, AppStateNamespace, AppStateVersion,
    CapabilityDecision, CapabilityRequest, CapabilityResolver, CorrelationId, DeepLink, ErrorCode,
    ErrorEnvelope, GracefulShutdownHook, Intent, IntentDeclaration, IpcEnvelope, LifecycleEvent,
    LifecycleEventKind, LifecycleMachine, LifecycleState, MessageKind, OrderingMetadata, RequestId,
    RestoreKind, ResumeTarget, StateLoad, StateMigrator, IPC_PROTOCOL_VERSION,
};
use nagi_sdk::{AppId, AppSessionId, NodeId};
use std::path::PathBuf;

struct GrantAllDeclared;

impl CapabilityResolver for GrantAllDeclared {
    fn resolve(
        &mut self,
        _app_id: AppId,
        requested: &[CapabilityRequest<'_>],
    ) -> Result<CapabilityDecision, AppError> {
        assert!(!requested.is_empty());
        Ok(CapabilityDecision::Resolved)
    }
}

#[derive(Default)]
struct TestState {
    version: Option<AppStateVersion>,
    bytes: Vec<u8>,
}

impl AppStateBackend for TestState {
    fn load(
        &mut self,
        _namespace: AppStateNamespace,
        output: &mut [u8],
    ) -> Result<StateLoad, AppError> {
        let Some(version) = self.version else {
            return Ok(StateLoad::Missing);
        };
        if self.bytes.len() > output.len() {
            return Err(AppError::new(ErrorCode::BufferTooSmall));
        }
        output[..self.bytes.len()].copy_from_slice(&self.bytes);
        Ok(StateLoad::Loaded {
            version,
            length: self.bytes.len(),
        })
    }

    fn save(
        &mut self,
        _namespace: AppStateNamespace,
        version: AppStateVersion,
        bytes: &[u8],
    ) -> Result<(), AppError> {
        self.bytes = bytes.to_vec();
        self.version = Some(version);
        Ok(())
    }

    fn reset(&mut self, _namespace: AppStateNamespace) -> Result<(), AppError> {
        self.bytes.clear();
        self.version = None;
        Ok(())
    }
}

struct NoMigration;

impl StateMigrator for NoMigration {
    fn migrate(
        &self,
        _namespace: AppStateNamespace,
        _from: AppStateVersion,
        _to: AppStateVersion,
        _source: &[u8],
        _destination: &mut [u8],
    ) -> Result<usize, AppError> {
        Err(AppError::new(ErrorCode::StateMigrationFailed))
    }
}

fn event(kind: LifecycleEventKind) -> LifecycleEvent {
    LifecycleEvent {
        kind,
        session_id: AppSessionId(12),
        node_id: Some(NodeId(4)),
        sequence: None,
    }
}

#[test]
fn public_contract_runs_manifest_to_intent_state_and_shutdown_flow() {
    let notes = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/rust/fixtures/manifests/notes.json");
    let manifest = load_manifest(&notes).expect("notes contract manifest");

    let mut registry_slots = [None; 4];
    let mut registry = AppRegistry::new(&mut registry_slots);
    let identity = AppIdentity::new(
        &manifest.identifier,
        &manifest.version,
        manifest.origin,
        manifest.publisher_id.as_deref(),
    )
    .expect("validated app identity");
    registry
        .register(identity)
        .expect("fixture app registration");
    assert_eq!(registry.lookup(manifest.app_id), Some(identity));
    assert_eq!(registry.len(), 1);

    let capabilities = [CapabilityRequest {
        id: "storage.read",
        purpose_key: Some("notes.read_purpose"),
    }];
    let decision = resolve_capabilities(&mut GrantAllDeclared, manifest.app_id, &capabilities)
        .expect("external resolver decision");
    assert_eq!(decision, CapabilityDecision::Resolved);

    let mut lifecycle = LifecycleMachine::new();
    assert_eq!(lifecycle.state(), LifecycleState::Registered);
    lifecycle
        .apply(event(LifecycleEventKind::LaunchRequested))
        .unwrap();
    lifecycle
        .apply(event(LifecycleEventKind::CapabilitiesResolved(decision)))
        .unwrap();
    lifecycle.apply(event(LifecycleEventKind::Ready)).unwrap();
    lifecycle
        .apply(event(LifecycleEventKind::Activate))
        .unwrap();
    assert_eq!(lifecycle.state(), LifecycleState::Foreground);

    let declaration = IntentDeclaration {
        id: "notes.open-note",
        version: 1,
        payload_type: "nagi.note-reference@1",
        route_id: Some("open-note"),
    };
    let intent = Intent {
        id: declaration.id,
        version: declaration.version,
        target_app: Some(manifest.app_id),
        source_app: AppId::from_identifier(b"com.nagi.sdk-fixture.activity"),
        correlation_id: CorrelationId([3; 16]),
        payload_type: declaration.payload_type,
        payload: b"opaque-note-object-id",
        deep_link: Some(DeepLink::parse("nagi://com.nagi.sdk-fixture.notes/open-note").unwrap()),
    };
    validate_intent(&intent, &manifest.identifier, &[declaration]).unwrap();

    let namespace = AppStateNamespace {
        app_id: manifest.app_id,
        session_id: Some(AppSessionId(12)),
    };
    let mut state = TestState::default();
    let mut scratch = [0_u8; 128];
    let mut restored = [0_u8; 128];
    let missing = restore_state(
        &mut state,
        &NoMigration,
        namespace,
        AppStateVersion(2),
        AppStateVersion(1),
        &mut scratch,
        &mut restored,
    )
    .unwrap();
    assert_eq!(missing.kind, RestoreKind::Missing);
    state
        .save(namespace, AppStateVersion(2), b"last-open-note")
        .unwrap();
    lifecycle
        .apply(event(LifecycleEventKind::Suspended {
            checkpoint: Some(8),
        }))
        .unwrap();
    lifecycle
        .apply(event(LifecycleEventKind::Resumed {
            target: ResumeTarget::Foreground,
            checkpoint: Some(8),
        }))
        .unwrap();
    let restored_state = restore_state(
        &mut state,
        &NoMigration,
        namespace,
        AppStateVersion(2),
        AppStateVersion(1),
        &mut scratch,
        &mut restored,
    )
    .unwrap();
    assert_eq!(restored_state.kind, RestoreKind::Restored);
    assert_eq!(&restored[..restored_state.length], b"last-open-note");
    lifecycle
        .apply(event(LifecycleEventKind::TerminationRequested))
        .unwrap();
    struct ShutdownHook(bool);
    impl GracefulShutdownHook for ShutdownHook {
        fn on_termination_requested(&mut self, shutdown: LifecycleEvent) -> Result<(), AppError> {
            assert_eq!(shutdown.session_id, AppSessionId(12));
            self.0 = true;
            Ok(())
        }
    }
    let mut shutdown_hook = ShutdownHook(false);
    invoke_graceful_shutdown(
        &mut shutdown_hook,
        event(LifecycleEventKind::TerminationRequested),
    )
    .unwrap();
    assert!(shutdown_hook.0);
    lifecycle
        .apply(event(LifecycleEventKind::ShutdownHookCompleted))
        .unwrap();
    lifecycle
        .apply(event(LifecycleEventKind::Terminated))
        .unwrap();
    assert_eq!(lifecycle.state(), LifecycleState::Terminated);
}

#[test]
fn public_ipc_error_and_denial_contracts_are_usable_by_a_host_adapter() {
    let envelope = IpcEnvelope {
        protocol_version: IPC_PROTOCOL_VERSION,
        source: AppId(21),
        destination: Some(AppId(22)),
        kind: MessageKind::Request,
        correlation_id: CorrelationId([9; 16]),
        request_id: Some(RequestId([7; 16])),
        ordering: OrderingMetadata {
            timestamp_micros: Some(100),
            sequence: Some(4),
        },
        payload_type: "nagi.resource-reference@1",
        payload: b"object-id",
    };
    let mut wire = [0; 256];
    let length = envelope.encode(&mut wire).unwrap();
    assert_eq!(IpcEnvelope::decode(&wire[..length]).unwrap(), envelope);

    let error = ErrorEnvelope {
        correlation_id: envelope.correlation_id,
        code: ErrorCode::PermissionDenied,
        retryable: false,
        message_key: Some("errors.permission_denied"),
    };
    let mut error_wire = [0; 128];
    let length = error.encode(&mut error_wire).unwrap();
    assert_eq!(ErrorEnvelope::decode(&error_wire[..length]).unwrap(), error);

    struct Reject;
    impl CapabilityResolver for Reject {
        fn resolve(
            &mut self,
            _app_id: AppId,
            _requested: &[CapabilityRequest<'_>],
        ) -> Result<CapabilityDecision, AppError> {
            Ok(CapabilityDecision::Denied)
        }
    }
    assert_eq!(
        resolve_capabilities(
            &mut Reject,
            AppId(22),
            &[CapabilityRequest {
                id: "storage.write",
                purpose_key: None,
            }]
        ),
        Err(AppError::new(ErrorCode::PermissionDenied))
    );
}
