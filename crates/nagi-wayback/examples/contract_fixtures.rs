use std::collections::BTreeMap;

use nagi_wayback_foundation::*;

fn actor(kind: ActorKind) -> ActorMetadata {
    ActorMetadata {
        kind,
        principal: Some(PrincipalReference {
            namespace: "nagi.user".into(),
            id: "fixture-user".into(),
        }),
        app_id: Some("org.nagi.notes".into()),
        delegated_for: None,
        model_id: None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default())?;
    let transaction_id = ledger.start_transaction(TransactionDraft {
        schema_version: SCHEMA_VERSION,
        id: TransactionId::new("contract-fixture-transaction")?,
        started_at_ms: 1_800_000_000_000,
        actor: actor(ActorKind::User),
        summary: HumanSummary::Public("Restore a note to a checkpoint".into()),
        atomicity: AtomicityExpectation::AtomicLocal,
    })?;
    let boundary = ledger.current_boundary()?;
    let scope = ScopeReference {
        scope_type: "document".into(),
        external_id: "object:fixture".into(),
        app_id: Some("org.nagi.notes".into()),
    };
    let snapshot = SnapshotManifest {
        schema_version: SCHEMA_VERSION,
        id: SnapshotId::new("contract-fixture-snapshot")?,
        scope: scope.clone(),
        created_at_ms: 1_800_000_000_000,
        source_transaction: Some(transaction_id.clone()),
        source_ledger_boundary: boundary.clone(),
        content_digest: "f".repeat(64),
        item_count: Some(1),
        byte_count: Some(512),
        backend_id: "fixture.metadata-only".into(),
        compatibility_version: "state-v1".into(),
        parent_snapshot: None,
    };
    let mut snapshot_store = InMemorySnapshotStore::default();
    snapshot_store.insert(snapshot.clone())?;
    let restore_point_id = RestorePointId::new("contract-fixture-point")?;
    ledger.record_restore_point(RestorePointDraft {
        schema_version: SCHEMA_VERSION,
        id: restore_point_id.clone(),
        timestamp_ms: 1_800_000_000_001,
        label: HumanSummary::Public("Before the title change".into()),
        snapshot_ids: vec![snapshot.id.clone()],
        ledger_boundary: boundary,
        scopes: vec![scope],
        compatibility_version: "state-v1".into(),
        source_transaction: Some(transaction_id.clone()),
    })?;
    let snapshot_id = snapshot.id.clone();
    let entry_id = EntryId::new("contract-fixture-entry")?;
    ledger.append_entry(ActivityEntryDraft {
        schema_version: SCHEMA_VERSION,
        id: entry_id.clone(),
        timestamp_ms: 1_800_000_000_002,
        actor: actor(ActorKind::Application),
        app_id: Some("org.nagi.notes".into()),
        action_type: ActionType {
            namespace: "notes".into(),
            name: "document.delete-content".into(),
        },
        target: TargetReference {
            resource_type: "document".into(),
            external_id: "object:fixture".into(),
            app_id: Some("org.nagi.notes".into()),
        },
        transaction_id: Some(transaction_id.clone()),
        provenance: Provenance {
            initiating_event: Some("request:fixture".into()),
            parent_entry_id: None,
            cause_entry_id: None,
            correlation_id: Some(CorrelationId::new("correlation:fixture")?),
        },
        summary: HumanSummary::Public("Removed note content".into()),
        metadata: BTreeMap::new(),
        reversibility: Reversibility::SnapshotRequired {
            snapshot_id: Some(snapshot_id.clone()),
        },
        snapshot_refs: vec![snapshot_id],
        restore_point_refs: vec![restore_point_id.clone()],
        payload: Some(PayloadMetadata {
            availability: PayloadAvailability::SnapshotReference,
            snapshot_id: Some(SnapshotId::new("contract-fixture-snapshot")?),
            reason: None,
        }),
    })?;
    ledger.commit_transaction(&transaction_id, 1_800_000_000_003)?;
    let entry = ledger
        .query(&ActivityQuery {
            transaction_id: Some(transaction_id.clone()),
            ..ActivityQuery::default()
        })?
        .into_iter()
        .find(|entry| entry.id == entry_id)
        .expect("fixture entry was appended");
    let transaction = ledger
        .transaction(&transaction_id)?
        .expect("fixture transaction was started");
    let restore_point = ledger
        .restore_point(&restore_point_id)?
        .expect("fixture restore point was appended");
    let restore_plan =
        ledger.plan_restore_point(&restore_point_id, &snapshot_store, Some("state-v1"))?;

    let fixtures = serde_json::json!({
        "ledger": ledger.document()?,
        "activity_entry": entry,
        "transaction": transaction,
        "snapshot_manifest": snapshot,
        "restore_point": restore_point,
        "restore_plan": restore_plan,
    });
    println!("{}", serde_json::to_string_pretty(&fixtures)?);
    Ok(())
}
