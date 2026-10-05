use std::collections::BTreeMap;

use crate::*;

#[derive(Default)]
struct SnapshotMap(BTreeMap<SnapshotId, SnapshotManifest>);

impl SnapshotStore for SnapshotMap {
    fn get_manifest(&self, id: &SnapshotId) -> Result<Option<SnapshotManifest>, StoreError> {
        Ok(self.0.get(id).cloned())
    }
}

fn actor(kind: ActorKind) -> ActorMetadata {
    ActorMetadata {
        kind,
        principal: Some(PrincipalReference {
            namespace: "nagi.user".into(),
            id: "user-7".into(),
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

fn entry(
    id: &str,
    timestamp_ms: i64,
    transaction_id: Option<&str>,
    reversibility: Reversibility,
) -> ActivityEntryDraft {
    ActivityEntryDraft {
        schema_version: SCHEMA_VERSION,
        id: EntryId::new(id).unwrap(),
        timestamp_ms,
        actor: actor(ActorKind::Application),
        app_id: Some("org.nagi.notes".into()),
        action_type: ActionType {
            namespace: "notes".into(),
            name: "block.update".into(),
        },
        target: target("note-42"),
        transaction_id: transaction_id.map(|value| TransactionId::new(value).unwrap()),
        provenance: Provenance {
            initiating_event: None,
            parent_entry_id: None,
            cause_entry_id: None,
            correlation_id: None,
        },
        summary: HumanSummary::Public("Updated a note title".into()),
        metadata: BTreeMap::new(),
        reversibility,
        snapshot_refs: Vec::new(),
        restore_point_refs: Vec::new(),
        payload: None,
    }
}

fn transaction(id: &str, started_at_ms: i64) -> TransactionDraft {
    TransactionDraft {
        schema_version: SCHEMA_VERSION,
        id: TransactionId::new(id).unwrap(),
        started_at_ms,
        actor: actor(ActorKind::User),
        summary: HumanSummary::Public("Edit a note".into()),
        atomicity: AtomicityExpectation::AtomicLocal,
    }
}

fn snapshot(id: &str, scope: ScopeReference, boundary: LedgerBoundary) -> SnapshotManifest {
    SnapshotManifest {
        schema_version: SCHEMA_VERSION,
        id: SnapshotId::new(id).unwrap(),
        scope,
        created_at_ms: 10,
        source_transaction: None,
        source_ledger_boundary: boundary,
        content_digest: "a".repeat(64),
        item_count: Some(1),
        byte_count: Some(128),
        backend_id: "test.snapshot-store".into(),
        compatibility_version: "state-v1".into(),
        parent_snapshot: None,
    }
}

fn scope_for_note() -> ScopeReference {
    ScopeReference {
        scope_type: "document".into(),
        external_id: "note-42".into(),
        app_id: Some("org.nagi.notes".into()),
    }
}

fn ledger() -> ActivityLedger<InMemoryLedgerStore> {
    ActivityLedger::new(InMemoryLedgerStore::default()).unwrap()
}

#[test]
fn entry_and_transaction_ids_are_stable_and_order_is_append_order() {
    let mut ledger = ledger();
    let transaction_id = ledger.start_transaction(transaction("tx-1", 100)).unwrap();
    ledger
        .append_entry(entry(
            "entry-1",
            101,
            Some("tx-1"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger
        .append_entry(entry(
            "entry-2",
            102,
            Some("tx-1"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger.commit_transaction(&transaction_id, 103).unwrap();

    let transaction = ledger.transaction(&transaction_id).unwrap().unwrap();
    assert_eq!(transaction.entry_ids[0].as_str(), "entry-1");
    assert_eq!(transaction.entry_ids[1].as_str(), "entry-2");
    assert_eq!(transaction.schema_version, SCHEMA_VERSION);
    let entries = ledger.query(&ActivityQuery::default()).unwrap();
    assert_eq!(entries[0].id.as_str(), "entry-1");
    assert_eq!(entries[1].id.as_str(), "entry-2");
    assert_eq!(entries[0].integrity.sequence, 2);
    assert_eq!(entries[1].integrity.sequence, 3);
}

#[test]
fn committed_aborted_and_partial_transactions_have_distinct_states() {
    let mut ledger = ledger();
    let committed = ledger
        .start_transaction(transaction("committed", 10))
        .unwrap();
    ledger
        .append_entry(entry(
            "commit-entry",
            11,
            Some("committed"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger.commit_transaction(&committed, 12).unwrap();

    let aborted = ledger
        .start_transaction(transaction("aborted", 20))
        .unwrap();
    ledger
        .abort_transaction(
            &aborted,
            21,
            Some(HumanSummary::Public("Cancelled before any action".into())),
        )
        .unwrap();

    let partial = ledger
        .start_transaction(transaction("partial", 30))
        .unwrap();
    ledger
        .append_entry(entry(
            "partial-entry",
            31,
            Some("partial"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger
        .mark_partial_failure(
            &partial,
            32,
            HumanSummary::Public("Second update failed".into()),
        )
        .unwrap();

    assert!(matches!(
        ledger.transaction(&committed).unwrap().unwrap().state,
        TransactionState::Committed { at_ms: 12 }
    ));
    assert!(matches!(
        ledger.transaction(&aborted).unwrap().unwrap().state,
        TransactionState::Aborted { at_ms: 21, .. }
    ));
    assert!(matches!(
        ledger.transaction(&partial).unwrap().unwrap().state,
        TransactionState::PartiallyApplied { at_ms: 32, .. }
    ));
    assert!(matches!(
        ledger.plan_transaction_restore(&partial, &InMemorySnapshotStore::default(), None),
        Ok(RestorePlan {
            executable: false,
            ..
        })
    ));
}

#[test]
fn aborted_transactions_cannot_claim_recorded_mutations() {
    let mut ledger = ledger();
    let id = ledger.start_transaction(transaction("abort-1", 1)).unwrap();
    ledger
        .append_entry(entry(
            "abort-entry",
            2,
            Some("abort-1"),
            Reversibility::ExactReversible,
        ))
        .unwrap();
    assert!(matches!(
        ledger.abort_transaction(&id, 3, None),
        Err(LedgerError::InvalidTransition(_))
    ));
    ledger
        .mark_partial_failure(
            &id,
            3,
            HumanSummary::Public("Action may have applied".into()),
        )
        .unwrap();
}

#[test]
fn provenance_and_structured_filters_find_actor_target_and_cause() {
    let mut ledger = ledger();
    let correlation = CorrelationId::new("corr-ai-1").unwrap();
    let user_id = ledger
        .append_entry(ActivityEntryDraft {
            actor: actor(ActorKind::User),
            transaction_id: None,
            reversibility: Reversibility::ExactReversible,
            summary: HumanSummary::Public("Rename a note".into()),
            ..entry("request-1", 100, None, Reversibility::ExactReversible)
        })
        .unwrap()
        .id;
    let ai_id = ledger
        .append_entry(ActivityEntryDraft {
            actor: actor(ActorKind::Ai),
            provenance: Provenance {
                initiating_event: Some("request/user-17".into()),
                parent_entry_id: Some(user_id.clone()),
                cause_entry_id: Some(user_id.clone()),
                correlation_id: Some(correlation.clone()),
            },
            summary: HumanSummary::Public("Prepared a rename action".into()),
            ..entry("ai-plan-1", 101, None, Reversibility::ExactReversible)
        })
        .unwrap()
        .id;
    ledger
        .append_entry(ActivityEntryDraft {
            actor: actor(ActorKind::Application),
            provenance: Provenance {
                initiating_event: Some("tool/notes.rename".into()),
                parent_entry_id: Some(ai_id.clone()),
                cause_entry_id: Some(user_id.clone()),
                correlation_id: Some(correlation.clone()),
            },
            summary: HumanSummary::Public("Renamed the note".into()),
            ..entry("tool-result-1", 102, None, Reversibility::ExactReversible)
        })
        .unwrap();

    let chain = ledger
        .query(&ActivityQuery {
            correlation_id: Some(correlation),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].id, ai_id);
    assert_eq!(chain[1].provenance.cause_entry_id, Some(user_id.clone()));
    let by_cause = ledger
        .query(&ActivityQuery {
            causation_id: Some(user_id),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(by_cause.len(), 2);
    assert!(by_cause
        .iter()
        .all(|entry| entry.summary.public_text().is_some()));
}

#[test]
fn query_supports_time_actor_app_target_transaction_and_reversibility() {
    let mut ledger = ledger();
    let transaction_id = ledger
        .start_transaction(transaction("query-tx", 100))
        .unwrap();
    ledger
        .append_entry(entry(
            "query-entry",
            110,
            Some("query-tx"),
            Reversibility::SnapshotRequired { snapshot_id: None },
        ))
        .unwrap();

    let found = ledger
        .query(&ActivityQuery {
            time: Some(TimeRange {
                from_ms: Some(110),
                through_ms: Some(110),
            }),
            actor_kind: Some(ActorKind::Application),
            principal: Some(PrincipalReference {
                namespace: "nagi.user".into(),
                id: "user-7".into(),
            }),
            app_id: Some("org.nagi.notes".into()),
            target: Some(target("note-42")),
            action_type: Some(ActionType {
                namespace: "notes".into(),
                name: "block.update".into(),
            }),
            transaction_id: Some(transaction_id),
            reversibility: Some(ReversibilityClass::SnapshotRequired),
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id.as_str(), "query-entry");
    assert!(matches!(
        ledger.query(&ActivityQuery {
            time: Some(TimeRange {
                from_ms: Some(20),
                through_ms: Some(10),
            }),
            ..ActivityQuery::default()
        }),
        Err(LedgerError::InvalidQuery(_))
    ));
}

#[test]
fn sensitive_metadata_cannot_store_raw_values_and_payload_state_is_explicit() {
    let mut unsafe_entry = entry(
        "private-1",
        1,
        None,
        Reversibility::Unknown {
            reason: "not assessed".into(),
        },
    );
    unsafe_entry.metadata.insert(
        "clipboard.text".into(),
        MetadataField {
            sensitivity: FieldSensitivity::Secret,
            disposition: FieldDisposition::Recorded,
            value: Some(MetadataValue::Text("secret body".into())),
            reason: None,
        },
    );
    assert!(unsafe_entry.validate().is_err());

    let mut safe_entry = entry(
        "private-2",
        1,
        None,
        Reversibility::Unknown {
            reason: "not assessed".into(),
        },
    );
    safe_entry.metadata.insert(
        "clipboard.text".into(),
        MetadataField {
            sensitivity: FieldSensitivity::Secret,
            disposition: FieldDisposition::Redacted,
            value: None,
            reason: Some("secret fields are omitted".into()),
        },
    );
    safe_entry.payload = Some(PayloadMetadata {
        availability: PayloadAvailability::Unavailable,
        snapshot_id: None,
        reason: Some("no authorized snapshot reference".into()),
    });
    safe_entry.validate().unwrap();
    let mut ledger = ledger();
    ledger.append_entry(safe_entry).unwrap();
    let serialized = String::from_utf8(ledger.serialize().unwrap()).unwrap();
    assert!(!serialized.contains("secret body"));
    assert!(serialized.contains("redacted"));
    assert!(serialized.contains("unavailable"));
}

#[test]
fn snapshot_store_validates_manifest_digest_and_parent_link() {
    let mut snapshots = InMemorySnapshotStore::default();
    let parent = snapshot(
        "snap-parent",
        scope_for_note(),
        LedgerBoundary {
            sequence: 0,
            record_hash: None,
        },
    );
    snapshots.insert(parent.clone()).unwrap();
    let child = SnapshotManifest {
        id: SnapshotId::new("snap-child").unwrap(),
        parent_snapshot: Some(parent.id),
        ..parent
    };
    snapshots.insert(child).unwrap();
    assert_eq!(snapshots.len(), 2);

    let invalid = SnapshotManifest {
        id: SnapshotId::new("snap-invalid").unwrap(),
        content_digest: "not-a-digest".into(),
        parent_snapshot: None,
        ..snapshot(
            "snap-template",
            scope_for_note(),
            LedgerBoundary {
                sequence: 0,
                record_hash: None,
            },
        )
    };
    assert!(matches!(
        snapshots.insert(invalid),
        Err(StoreError::InvalidSnapshot(_))
    ));
}

#[test]
fn missing_or_incompatible_snapshot_never_produces_an_executable_plan() {
    let mut ledger = ledger();
    let transaction_id = ledger
        .start_transaction(transaction("snapshot-tx", 1))
        .unwrap();
    let snapshot_boundary = ledger.current_boundary().unwrap();
    let snapshot_id = SnapshotId::new("snapshot-missing").unwrap();
    let mut draft = entry(
        "snapshot-entry",
        2,
        Some("snapshot-tx"),
        Reversibility::SnapshotRequired {
            snapshot_id: Some(snapshot_id.clone()),
        },
    );
    draft.snapshot_refs.push(snapshot_id.clone());
    ledger.append_entry(draft).unwrap();
    ledger.commit_transaction(&transaction_id, 3).unwrap();

    let missing = ledger
        .plan_transaction_restore(
            &transaction_id,
            &InMemorySnapshotStore::default(),
            Some("state-v1"),
        )
        .unwrap();
    assert!(!missing.executable);
    assert!(missing.blockers.contains(&PlanBlocker::MissingSnapshot {
        snapshot_id: snapshot_id.clone()
    }));

    let mut snapshots = InMemorySnapshotStore::default();
    snapshots
        .insert(snapshot(
            "snapshot-missing",
            scope_for_note(),
            snapshot_boundary,
        ))
        .unwrap();
    let mismatch = ledger
        .plan_transaction_restore(&transaction_id, &snapshots, Some("state-v2"))
        .unwrap();
    assert!(!mismatch.executable);
    assert!(mismatch
        .blockers
        .iter()
        .any(|reason| matches!(reason, PlanBlocker::CompatibilityMismatch { .. })));
}

#[test]
fn snapshot_parent_dependencies_are_resolved_in_base_before_child_order() {
    let mut ledger = ledger();
    let transaction_id = ledger
        .start_transaction(transaction("snapshot-parent-tx", 1))
        .unwrap();
    let boundary = ledger.current_boundary().unwrap();
    let mut snapshots = InMemorySnapshotStore::default();
    let parent = snapshot("a-parent", scope_for_note(), boundary.clone());
    snapshots.insert(parent.clone()).unwrap();
    let child_id = SnapshotId::new("b-child").unwrap();
    snapshots
        .insert(SnapshotManifest {
            id: child_id.clone(),
            parent_snapshot: Some(parent.id.clone()),
            ..snapshot("b-child", scope_for_note(), boundary)
        })
        .unwrap();

    let mut draft = entry(
        "snapshot-parent-entry",
        2,
        Some("snapshot-parent-tx"),
        Reversibility::SnapshotRequired {
            snapshot_id: Some(child_id.clone()),
        },
    );
    draft.snapshot_refs.push(child_id);
    ledger.append_entry(draft).unwrap();
    ledger.commit_transaction(&transaction_id, 3).unwrap();

    let plan = ledger
        .plan_transaction_restore(&transaction_id, &snapshots, Some("state-v1"))
        .unwrap();
    assert!(plan.executable);
    assert_eq!(plan.required_snapshots.len(), 2);
    assert_eq!(plan.steps.len(), 2);
    assert!(matches!(
        &plan.steps[0],
        RestoreStep::SnapshotRestore { snapshot_id, .. } if snapshot_id.as_str() == "a-parent"
    ));
    assert!(matches!(
        &plan.steps[1],
        RestoreStep::SnapshotRestore { snapshot_id, .. } if snapshot_id.as_str() == "b-child"
    ));
}

#[test]
fn missing_or_cyclic_snapshot_dependencies_are_non_executable() {
    let mut ledger = ledger();
    let transaction_id = ledger
        .start_transaction(transaction("snapshot-cycle-tx", 1))
        .unwrap();
    let boundary = ledger.current_boundary().unwrap();
    let first_id = SnapshotId::new("cycle-first").unwrap();
    let second_id = SnapshotId::new("cycle-second").unwrap();
    let mut snapshots = SnapshotMap::default();
    snapshots.0.insert(
        first_id.clone(),
        SnapshotManifest {
            parent_snapshot: Some(second_id.clone()),
            ..snapshot("cycle-first", scope_for_note(), boundary.clone())
        },
    );
    snapshots.0.insert(
        second_id.clone(),
        SnapshotManifest {
            parent_snapshot: Some(first_id.clone()),
            ..snapshot("cycle-second", scope_for_note(), boundary)
        },
    );
    let mut draft = entry(
        "snapshot-cycle-entry",
        2,
        Some("snapshot-cycle-tx"),
        Reversibility::SnapshotRequired {
            snapshot_id: Some(first_id.clone()),
        },
    );
    draft.snapshot_refs.push(first_id);
    ledger.append_entry(draft).unwrap();
    ledger.commit_transaction(&transaction_id, 3).unwrap();

    let plan = ledger
        .plan_transaction_restore(&transaction_id, &snapshots, Some("state-v1"))
        .unwrap();
    assert!(!plan.executable);
    assert!(plan
        .blockers
        .iter()
        .any(|blocker| matches!(blocker, PlanBlocker::SnapshotDependencyCycle { .. })));
}

#[test]
fn transaction_reversibility_distinguishes_exact_compensation_and_irreversible() {
    let mut ledger = ledger();
    let id = ledger.start_transaction(transaction("classes", 1)).unwrap();
    for (entry_id, classification) in [
        ("exact", Reversibility::ExactReversible),
        (
            "compensate",
            Reversibility::CompensationRequired {
                contract_id: "mail.send-reversal-v1".into(),
            },
        ),
        (
            "irreversible",
            Reversibility::Irreversible {
                reason: "external recipient already received message".into(),
            },
        ),
    ] {
        ledger
            .append_entry(entry(entry_id, 2, Some("classes"), classification))
            .unwrap();
    }
    ledger.commit_transaction(&id, 3).unwrap();
    let plan = ledger
        .plan_transaction_restore(&id, &InMemorySnapshotStore::default(), None)
        .unwrap();
    assert!(!plan.executable);
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        RestoreStep::ExactUndo { entry_id } if entry_id.as_str() == "exact"
    )));
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        RestoreStep::Compensation { entry_id, contract_id }
            if entry_id.as_str() == "compensate" && contract_id == "mail.send-reversal-v1"
    )));
    assert!(plan.blockers.iter().any(|blocker| matches!(
        blocker,
        PlanBlocker::IrreversibleAction { entry_id, .. }
            if entry_id.as_str() == "irreversible"
    )));
}

#[test]
fn restore_point_rejects_missing_scopes_and_unmatched_ledger_boundary() {
    let mut ledger = ledger();
    let mut invalid = RestorePointDraft {
        schema_version: SCHEMA_VERSION,
        id: RestorePointId::new("bad-point").unwrap(),
        timestamp_ms: 10,
        label: HumanSummary::Public("Before edits".into()),
        snapshot_ids: vec![SnapshotId::new("snap-1").unwrap()],
        ledger_boundary: LedgerBoundary {
            sequence: 7,
            record_hash: Some("b".repeat(64)),
        },
        scopes: vec![scope_for_note()],
        compatibility_version: "state-v1".into(),
        source_transaction: None,
    };
    assert!(matches!(
        ledger.record_restore_point(invalid.clone()),
        Err(LedgerError::InvalidTransition(_))
    ));
    invalid.ledger_boundary = LedgerBoundary {
        sequence: 0,
        record_hash: None,
    };
    assert!(ledger.record_restore_point(invalid).is_ok());
}

#[test]
fn restore_point_plan_reverses_exact_changes_outside_snapshotted_scopes() {
    let mut ledger = ledger();
    let source_transaction = ledger
        .start_transaction(transaction("point-source", 1))
        .unwrap();
    ledger.commit_transaction(&source_transaction, 2).unwrap();
    let boundary = ledger.current_boundary().unwrap();
    let covered_scope = scope_for_note();
    let snapshot_id = SnapshotId::new("point-base-snapshot").unwrap();
    let mut snapshots = InMemorySnapshotStore::default();
    snapshots
        .insert(snapshot(
            snapshot_id.as_str(),
            covered_scope.clone(),
            boundary.clone(),
        ))
        .unwrap();
    let point_id = RestorePointId::new("point-base").unwrap();
    ledger
        .record_restore_point(RestorePointDraft {
            schema_version: SCHEMA_VERSION,
            id: point_id.clone(),
            timestamp_ms: 3,
            label: HumanSummary::Public("Before an unrelated note was created".into()),
            snapshot_ids: vec![snapshot_id],
            ledger_boundary: boundary,
            scopes: vec![covered_scope],
            compatibility_version: "state-v1".into(),
            source_transaction: Some(source_transaction),
        })
        .unwrap();
    let mut later_action = entry(
        "point-later-action",
        4,
        None,
        Reversibility::ExactReversible,
    );
    later_action.target.external_id = "note-created-later".into();
    ledger.append_entry(later_action).unwrap();

    let plan = ledger
        .plan_restore_point(&point_id, &snapshots, Some("state-v1"))
        .unwrap();
    assert!(plan.executable);
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        RestoreStep::ExactUndo { entry_id } if entry_id.as_str() == "point-later-action"
    )));
    assert!(plan
        .affected_scopes
        .iter()
        .any(|scope| { scope.external_id == "note-created-later" }));
}

#[test]
fn restore_point_plan_blocks_uncaptured_scope_and_uncommitted_transaction() {
    let mut ledger = ledger();
    let source_transaction = ledger
        .start_transaction(transaction("point-source-unsafe", 1))
        .unwrap();
    ledger.commit_transaction(&source_transaction, 2).unwrap();
    let boundary = ledger.current_boundary().unwrap();
    let covered_scope = scope_for_note();
    let snapshot_id = SnapshotId::new("point-unsafe-snapshot").unwrap();
    let mut snapshots = InMemorySnapshotStore::default();
    snapshots
        .insert(snapshot(
            snapshot_id.as_str(),
            covered_scope.clone(),
            boundary.clone(),
        ))
        .unwrap();
    let point_id = RestorePointId::new("point-unsafe").unwrap();
    ledger
        .record_restore_point(RestorePointDraft {
            schema_version: SCHEMA_VERSION,
            id: point_id.clone(),
            timestamp_ms: 3,
            label: HumanSummary::Public("Before an external action".into()),
            snapshot_ids: vec![snapshot_id],
            ledger_boundary: boundary,
            scopes: vec![covered_scope],
            compatibility_version: "state-v1".into(),
            source_transaction: Some(source_transaction),
        })
        .unwrap();
    let transaction_id = ledger
        .start_transaction(transaction("point-uncommitted", 4))
        .unwrap();
    let mut later_action = entry(
        "point-later-unsafe-action",
        5,
        Some("point-uncommitted"),
        Reversibility::SnapshotRequired { snapshot_id: None },
    );
    later_action.target.external_id = "uncaptured-resource".into();
    ledger.append_entry(later_action).unwrap();

    let plan = ledger
        .plan_restore_point(&point_id, &snapshots, Some("state-v1"))
        .unwrap();
    assert!(!plan.executable);
    assert!(plan.blockers.iter().any(|blocker| matches!(
        blocker,
        PlanBlocker::RestorePointScopeNotCaptured { entry_id, .. }
            if entry_id.as_str() == "point-later-unsafe-action"
    )));
    assert!(plan.blockers.iter().any(|blocker| matches!(
        blocker,
        PlanBlocker::TransactionNotCommitted { transaction_id: id }
            if id == &transaction_id
    )));
}

#[test]
fn serialization_roundtrip_and_unknown_versions_are_checked() {
    let mut ledger = ledger();
    ledger
        .append_entry(entry(
            "roundtrip-entry",
            42,
            None,
            Reversibility::ExactReversible,
        ))
        .unwrap();
    let bytes = ledger.serialize().unwrap();
    let decoded = ActivityLedger::from_json(&bytes).unwrap();
    assert_eq!(decoded.document().unwrap(), ledger.document().unwrap());

    let mut document = ledger.document().unwrap();
    document.schema_version = SCHEMA_VERSION + 1;
    let unknown = serde_json::to_vec(&document).unwrap();
    assert!(matches!(
        ActivityLedger::from_json(&unknown),
        Err(LedgerError::UnsupportedVersion {
            kind: "ledger document",
            ..
        })
    ));

    let mut entry_version = ledger.document().unwrap();
    if let LedgerPayload::ActivityEntry(entry) = &mut entry_version.records[0].payload {
        entry.schema_version = SCHEMA_VERSION + 1;
    }
    let entry_version = serde_json::to_vec(&entry_version).unwrap();
    assert!(ActivityLedger::from_json(&entry_version).is_err());

    let mut plan = RestorePlan {
        schema_version: SCHEMA_VERSION + 1,
        target: RestoreTarget::Transaction(TransactionId::new("future-tx").unwrap()),
        steps: Vec::new(),
        required_snapshots: Vec::new(),
        blockers: vec![PlanBlocker::NoRestorableActions],
        affected_scopes: Vec::new(),
        executable: false,
        execution_boundary: ExecutionBoundary::PlanOnlyFutureAuthorizedExecutorRequired,
    };
    assert!(plan.validate_version().is_err());
    plan.schema_version = SCHEMA_VERSION;
    let encoded = serde_json::to_vec(&plan).unwrap();
    let decoded: RestorePlan = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.schema_version, SCHEMA_VERSION);
}

#[test]
fn malformed_deserialized_identifiers_and_query_filters_are_rejected() {
    assert!(serde_json::from_str::<EntryId>("\"\"").is_err());

    let mut draft = entry("valid-entry", 10, None, Reversibility::ExactReversible);
    draft.id = EntryId(String::new());
    assert!(draft.validate().is_err());

    let mut draft = entry("valid-entry-2", 10, None, Reversibility::ExactReversible);
    draft.provenance.correlation_id = Some(CorrelationId("\n".into()));
    assert!(draft.validate().is_err());

    let ledger = ledger();
    let result = ledger.query(&ActivityQuery {
        transaction_id: Some(TransactionId(String::new())),
        ..ActivityQuery::default()
    });
    assert!(matches!(result, Err(LedgerError::Validation(_))));
}

#[test]
fn integrity_detects_record_edit_reorder_and_checkpoint_truncation() {
    let mut ledger = ledger();
    ledger
        .append_entry(entry(
            "integrity-1",
            1,
            None,
            Reversibility::ExactReversible,
        ))
        .unwrap();
    ledger
        .append_entry(entry(
            "integrity-2",
            2,
            None,
            Reversibility::ExactReversible,
        ))
        .unwrap();
    let original = ledger.document().unwrap();

    let mut edited = original.clone();
    if let LedgerPayload::ActivityEntry(entry) = &mut edited.records[0].payload {
        entry.summary = HumanSummary::Public("altered after append".into());
    }
    let bytes = serde_json::to_vec(&edited).unwrap();
    assert!(matches!(
        ActivityLedger::from_json(&bytes),
        Err(LedgerError::IntegrityMismatch { sequence: 1 })
    ));

    let mut reordered = original.clone();
    reordered.records.swap(0, 1);
    let bytes = serde_json::to_vec(&reordered).unwrap();
    assert!(ActivityLedger::from_json(&bytes).is_err());

    let mut truncated = original;
    truncated.records.pop();
    let bytes = serde_json::to_vec(&truncated).unwrap();
    assert!(matches!(
        ActivityLedger::from_json(&bytes),
        Err(LedgerError::CheckpointMismatch)
    ));
}

#[test]
fn append_order_is_stable_and_newest_first_is_an_explicit_query_choice() {
    let mut ledger = ledger();
    for (id, time) in [("first", 20), ("second", 10)] {
        ledger
            .append_entry(entry(
                id,
                time,
                None,
                Reversibility::Unknown {
                    reason: "not assessed".into(),
                },
            ))
            .unwrap();
    }
    let oldest = ledger.query(&ActivityQuery::default()).unwrap();
    let newest = ledger
        .query(&ActivityQuery {
            newest_first: true,
            ..ActivityQuery::default()
        })
        .unwrap();
    assert_eq!(oldest[0].id.as_str(), "first");
    assert_eq!(newest[0].id.as_str(), "second");
}

#[test]
fn recent_undo_candidates_are_newest_committed_first_and_keep_blockers() {
    let mut ledger = ledger();
    for (transaction_id, entry_id, reverse) in [
        ("old", "old-entry", Reversibility::ExactReversible),
        (
            "new",
            "new-entry",
            Reversibility::Unknown {
                reason: "provider does not report rollback support".into(),
            },
        ),
    ] {
        ledger
            .start_transaction(transaction(transaction_id, 1))
            .unwrap();
        ledger
            .append_entry(entry(entry_id, 2, Some(transaction_id), reverse))
            .unwrap();
        ledger
            .commit_transaction(&TransactionId::new(transaction_id).unwrap(), 3)
            .unwrap();
    }
    let candidates = ledger
        .undo_candidates(10, &InMemorySnapshotStore::default(), None)
        .unwrap();
    assert_eq!(candidates[0].transaction.id.as_str(), "new");
    assert!(!candidates[0].plan.executable);
    assert!(candidates[0]
        .plan
        .blockers
        .iter()
        .any(|blocker| matches!(blocker, PlanBlocker::UnknownReversibility { .. })));
    assert!(candidates[1].plan.executable);
}

#[test]
fn external_principal_references_remain_data_only() {
    let reference = PrincipalReference {
        namespace: "capability.identity".into(),
        id: "principal:42".into(),
    };
    let mut actor = actor(ActorKind::Ai);
    actor.principal = Some(reference.clone());
    actor.delegated_for = Some(PrincipalReference {
        namespace: "nagi.user".into(),
        id: "user-7".into(),
    });
    actor.validate().unwrap();
    let serialized = serde_json::to_string(&actor).unwrap();
    assert!(serialized.contains("capability.identity"));
    assert!(serialized.contains("principal:42"));
    assert!(!serialized.contains("authority"));
}
