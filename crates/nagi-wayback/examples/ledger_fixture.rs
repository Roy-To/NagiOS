use std::collections::BTreeMap;

use nagi_wayback_foundation::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ledger = ActivityLedger::new(InMemoryLedgerStore::default())?;
    let transaction_id = TransactionId::new("schema-fixture-tx")?;
    ledger.start_transaction(TransactionDraft {
        schema_version: SCHEMA_VERSION,
        id: transaction_id.clone(),
        started_at_ms: 1_800_000_000_000,
        actor: ActorMetadata {
            kind: ActorKind::User,
            principal: Some(PrincipalReference {
                namespace: "nagi.user".into(),
                id: "fixture-user".into(),
            }),
            app_id: Some("org.nagi.notes".into()),
            delegated_for: None,
            model_id: None,
        },
        summary: HumanSummary::Public("Rename a note".into()),
        atomicity: AtomicityExpectation::AtomicLocal,
    })?;
    ledger.append_entry(ActivityEntryDraft {
        schema_version: SCHEMA_VERSION,
        id: EntryId::new("schema-fixture-entry")?,
        timestamp_ms: 1_800_000_000_001,
        actor: ActorMetadata {
            kind: ActorKind::Application,
            principal: Some(PrincipalReference {
                namespace: "nagi.app".into(),
                id: "org.nagi.notes".into(),
            }),
            app_id: Some("org.nagi.notes".into()),
            delegated_for: None,
            model_id: None,
        },
        app_id: Some("org.nagi.notes".into()),
        action_type: ActionType {
            namespace: "notes".into(),
            name: "document.rename".into(),
        },
        target: TargetReference {
            resource_type: "document".into(),
            external_id: "object:42".into(),
            app_id: Some("org.nagi.notes".into()),
        },
        transaction_id: Some(transaction_id.clone()),
        provenance: Provenance {
            initiating_event: Some("request:17".into()),
            parent_entry_id: None,
            cause_entry_id: None,
            correlation_id: Some(CorrelationId::new("correlation:17")?),
        },
        summary: HumanSummary::Public("Renamed a note".into()),
        metadata: BTreeMap::new(),
        reversibility: Reversibility::ExactReversible,
        snapshot_refs: Vec::new(),
        restore_point_refs: Vec::new(),
        payload: Some(PayloadMetadata {
            availability: PayloadAvailability::NotCaptured,
            snapshot_id: None,
            reason: Some("content is held outside the activity ledger".into()),
        }),
    })?;
    ledger.commit_transaction(&transaction_id, 1_800_000_000_002)?;
    println!("{}", String::from_utf8(ledger.serialize()?)?);
    Ok(())
}
