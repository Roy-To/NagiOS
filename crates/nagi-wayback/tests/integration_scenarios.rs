use std::collections::BTreeMap;

use nagi_wayback_foundation::*;

fn actor(kind: ActorKind) -> ActorMetadata {
    ActorMetadata {
        kind,
        principal: Some(PrincipalReference {
            namespace: "nagi.user".into(),
            id: "user-17".into(),
        }),
        app_id: Some("org.nagi.notes".into()),
        delegated_for: None,
        model_id: (kind == ActorKind::Ai).then(|| "standard-role".into()),
    }
}

fn target(id: &str) -> TargetReference {
    TargetReference {
        resource_type: "document".into(),
        external_id: id.into(),
        app_id: Some("org.nagi.notes".into()),
    }
}

fn tx(id: &str, at_ms: i64) -> TransactionDraft {
    TransactionDraft {
        schema_version: SCHEMA_VERSION,
        id: TransactionId::new(id).unwrap(),
        started_at_ms: at_ms,
        actor: actor(ActorKind::User),
        summary: HumanSummary::Public("Update a note".into()),
        atomicity: AtomicityExpectation::BestEffortPartial,
    }
}

fn action(
    id: &str,
    at_ms: i64,
    kind: ActorKind,
    target_id: &str,
    transaction_id: Option<&str>,
    reversibility: Reversibility,
) -> ActivityEntryDraft {
    ActivityEntryDraft {
        schema_version: SCHEMA_VERSION,
        id: EntryId::new(id).unwrap(),
        timestamp_ms: at_ms,
        actor: actor(kind),
        app_id: Some("org.nagi.notes".into()),
        action_type: ActionType {
            namespace: "notes".into(),
            name: "document.update".into(),
        },
        target: target(target_id),
        transaction_id: transaction_id.map(|value| TransactionId::new(value).unwrap()),
        provenance: Provenance {
            initiating_event: None,
            parent_entry_id: None,
            cause_entry_id: None,
            correlation_id: None,
        },
        summary: HumanSummary::Public("Updated the note".into()),
        metadata: BTreeMap::new(),
        reversibility,
        snapshot_refs: Vec::new(),
        restore_point_refs: Vec::new(),
        payload: None,
    }
}

fn scope(target_id: &str) -> ScopeReference {
    ScopeReference {
        scope_type: "document".into(),
        external_id: target_id.into(),
        app_id: Some("org.nagi.notes".into()),
    }
}

fn manifest(id: &str, scope: ScopeReference, boundary: LedgerBoundary) -> SnapshotManifest {
    SnapshotManifest {
        schema_version: SCHEMA_VERSION,
        id: SnapshotId::new(id).unwrap(),
        scope,
        created_at_ms: 100,
        source_transaction: None,
        source_ledger_boundary: boundary,
        content_digest: "c".repeat(64),
        item_count: Some(1),
        byte_count: Some(256),
        backend_id: "fixture.metadata-only".into(),
        compatibility_version: "nagi-state-v1".into(),
        parent_snapshot: None,
    }
}

#[test]
fn scenario_a_reversible_app_action_produces_an_executable_undo_plan() {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default()).unwrap();
    let id = ledger.start_transaction(tx("scenario-a", 100)).unwrap();
    ledger
        .append_entry(action(
            "scenario-a-action",
            101,
            ActorKind::Application,
            "note-a",
            Some("scenario-a"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger.commit_transaction(&id, 102).unwrap();

    let queried = ledger
        .query(&ActivityQuery {
            transaction_id: Some(id.clone()),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(queried.len(), 1);
    let candidates = ledger
        .undo_candidates(10, &InMemorySnapshotStore::default(), None)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].plan.executable);
    assert_eq!(candidates[0].plan.steps.len(), 1);
    assert!(matches!(
        candidates[0].plan.steps[0],
        RestoreStep::ExactUndo { .. }
    ));
}

#[test]
fn scenario_b_ai_tool_and_state_change_chain_is_queryable_by_cause_and_correlation() {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default()).unwrap();
    let correlation = CorrelationId::new("ai-chain-55").unwrap();
    let request = ledger
        .append_entry(action(
            "scenario-b-request",
            200,
            ActorKind::User,
            "note-b",
            None,
            Reversibility::ExactReversible,
        ))
        .unwrap()
        .id;
    let transaction_id = ledger.start_transaction(tx("scenario-b", 201)).unwrap();
    let mut ai_action = action(
        "scenario-b-ai",
        202,
        ActorKind::Ai,
        "note-b",
        Some("scenario-b"),
        Reversibility::ExactReversible,
    );
    ai_action.provenance = Provenance {
        initiating_event: Some("user-request/note-title".into()),
        parent_entry_id: Some(request.clone()),
        cause_entry_id: Some(request.clone()),
        correlation_id: Some(correlation.clone()),
    };
    let ai_id = ledger.append_entry(ai_action).unwrap().id;
    let mut tool_action = action(
        "scenario-b-tool",
        203,
        ActorKind::Application,
        "note-b",
        Some("scenario-b"),
        Reversibility::ExactReversible,
    );
    tool_action.provenance = Provenance {
        initiating_event: Some("tool/notes.update".into()),
        parent_entry_id: Some(ai_id.clone()),
        cause_entry_id: Some(request.clone()),
        correlation_id: Some(correlation.clone()),
    };
    let tool_id = ledger.append_entry(tool_action).unwrap().id;
    let mut state_change = action(
        "scenario-b-state",
        204,
        ActorKind::Application,
        "note-b",
        Some("scenario-b"),
        Reversibility::ExactReversible,
    );
    state_change.provenance = Provenance {
        initiating_event: Some("state-change/object-revision-19".into()),
        parent_entry_id: Some(tool_id.clone()),
        cause_entry_id: Some(ai_id),
        correlation_id: Some(correlation.clone()),
    };
    ledger.append_entry(state_change).unwrap();
    ledger.commit_transaction(&transaction_id, 205).unwrap();

    let chain = ledger
        .query(&ActivityQuery {
            correlation_id: Some(correlation),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(chain.len(), 3);
    assert_eq!(
        chain[0]
            .provenance
            .parent_entry_id
            .as_ref()
            .unwrap()
            .as_str(),
        "scenario-b-request"
    );
    assert_eq!(chain[2].provenance.parent_entry_id, Some(tool_id));
    let caused_by_request = ledger
        .query(&ActivityQuery {
            causation_id: Some(request),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(caused_by_request.len(), 2);
    assert_eq!(caused_by_request[0].actor.kind, ActorKind::Ai);
}

#[test]
fn scenario_c_snapshot_manifest_and_restore_point_resolve_a_safe_plan() {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default()).unwrap();
    let transaction_id = ledger.start_transaction(tx("scenario-c", 300)).unwrap();
    let boundary = ledger.current_boundary().unwrap();
    let snapshot_id = SnapshotId::new("scenario-c-snapshot").unwrap();
    let mut snapshots = InMemorySnapshotStore::default();
    snapshots
        .insert(manifest(
            snapshot_id.as_str(),
            scope("note-c"),
            boundary.clone(),
        ))
        .unwrap();
    let point_id = RestorePointId::new("scenario-c-point").unwrap();
    ledger
        .record_restore_point(RestorePointDraft {
            schema_version: SCHEMA_VERSION,
            id: point_id.clone(),
            timestamp_ms: 301,
            label: HumanSummary::Public("Before the AI edit".into()),
            snapshot_ids: vec![snapshot_id.clone()],
            ledger_boundary: boundary,
            scopes: vec![scope("note-c")],
            compatibility_version: "nagi-state-v1".into(),
            source_transaction: Some(transaction_id.clone()),
        })
        .unwrap();
    let mut destructive = action(
        "scenario-c-delete",
        302,
        ActorKind::Ai,
        "note-c",
        Some("scenario-c"),
        Reversibility::SnapshotRequired {
            snapshot_id: Some(snapshot_id.clone()),
        },
    );
    destructive.snapshot_refs.push(snapshot_id);
    destructive.restore_point_refs.push(point_id.clone());
    ledger.append_entry(destructive).unwrap();
    ledger.commit_transaction(&transaction_id, 303).unwrap();

    let plan = ledger
        .plan_restore_point(&point_id, &snapshots, Some("nagi-state-v1"))
        .unwrap();
    assert!(plan.executable);
    assert_eq!(plan.required_snapshots.len(), 1);
    assert!(matches!(plan.steps[0], RestoreStep::SnapshotRestore { .. }));
    assert_eq!(
        plan.execution_boundary,
        ExecutionBoundary::PlanOnlyFutureAuthorizedExecutorRequired
    );
}

#[test]
fn scenario_d_irreversible_unknown_and_missing_snapshot_actions_are_blocked() {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default()).unwrap();
    for (tx_id, entry_id, reversibility) in [
        (
            "scenario-d-irreversible",
            "scenario-d-action-irreversible",
            Reversibility::Irreversible {
                reason: "external message delivery cannot be rolled back".into(),
            },
        ),
        (
            "scenario-d-unknown",
            "scenario-d-action-unknown",
            Reversibility::Unknown {
                reason: "provider has not declared reversibility".into(),
            },
        ),
        (
            "scenario-d-missing-ref",
            "scenario-d-action-missing-ref",
            Reversibility::SnapshotRequired { snapshot_id: None },
        ),
    ] {
        let transaction_id = ledger.start_transaction(tx(tx_id, 400)).unwrap();
        ledger
            .append_entry(action(
                entry_id,
                401,
                ActorKind::Application,
                "note-d",
                Some(tx_id),
                reversibility,
            ))
            .unwrap();
        ledger.commit_transaction(&transaction_id, 402).unwrap();
    }
    let snapshots = InMemorySnapshotStore::default();
    let irreversible = ledger
        .plan_transaction_restore(
            &TransactionId::new("scenario-d-irreversible").unwrap(),
            &snapshots,
            None,
        )
        .unwrap();
    assert!(!irreversible.executable);
    assert!(irreversible
        .blockers
        .iter()
        .any(|blocker| matches!(blocker, PlanBlocker::IrreversibleAction { .. })));

    let unknown = ledger
        .plan_transaction_restore(
            &TransactionId::new("scenario-d-unknown").unwrap(),
            &snapshots,
            None,
        )
        .unwrap();
    assert!(!unknown.executable);
    assert!(unknown
        .blockers
        .iter()
        .any(|blocker| matches!(blocker, PlanBlocker::UnknownReversibility { .. })));

    let missing = ledger
        .plan_transaction_restore(
            &TransactionId::new("scenario-d-missing-ref").unwrap(),
            &snapshots,
            None,
        )
        .unwrap();
    assert!(!missing.executable);
    assert!(missing
        .blockers
        .iter()
        .any(|blocker| matches!(blocker, PlanBlocker::MissingSnapshotReference { .. })));
}
