//! Run with `cargo run -p nagi-history --features sandbox --example activity_wayback_preview`.
//! This is a host-only fixture preview; it does not call a Nagi target service.

use nagi_history::activity::{
    ActionGroupId, ActionKind, ActivityAccessPolicy, ActivityDraft, ActivityLedger, ActivityQuery,
    Actor, ActorId, ActorKind, CorrelationId, EventResult, InverseKind, MetadataKey, MetadataValue,
    PrivacyClass, Provenance, Reversibility, RevisionId, Timestamp, TransactionId, UndoDescriptor,
    UserLocale,
};
use nagi_history::view::{
    render_checkpoint_timeline, render_diff_summary, render_restore_outcome, render_timeline,
    EpochTimeFormatter,
};
use nagi_history::wayback::sandbox::InMemoryRestoreSandbox;
use nagi_history::wayback::{
    compare_with_policy, execute_restore, prepare_restore_plan, CheckpointAccessPolicy,
    CheckpointDraft, CheckpointObject, CheckpointOrigin, CheckpointQuery, CheckpointReadStore,
    CheckpointReason, CheckpointScope, CheckpointStore, CurrentObjectRevision, DiffAccessPolicy,
    RestoreBackendAvailability, RestoreMode, RestorePlanId, RestorePolicy, SnapshotBackend,
};
use nagi_history::{ActivityContext, AppId, AppSessionId, NodeId, ObjectId, WorkspaceId};

const USER: Actor = Actor::new(ActorId(1), ActorKind::User);
const CONTEXT: ActivityContext = ActivityContext {
    app_id: AppId(1),
    app_session_id: AppSessionId(1),
    node_id: NodeId(1),
    surface_id: None,
    workspace_id: Some(WorkspaceId(1)),
};

struct Allow;

impl ActivityAccessPolicy for Allow {
    fn can_read(&self, _viewer: Actor, _event: &nagi_history::activity::ActivityEvent) -> bool {
        true
    }
}

impl CheckpointAccessPolicy for Allow {
    fn can_read(
        &self,
        _viewer: Actor,
        _checkpoint: &nagi_history::wayback::CheckpointRecord,
    ) -> bool {
        true
    }
}

impl DiffAccessPolicy for Allow {
    fn can_compare(&self, _viewer: Actor, _object: ObjectId) -> bool {
        true
    }
}

impl RestorePolicy for Allow {
    fn authorize(
        &self,
        _actor: Actor,
        _plan: &nagi_history::wayback::RestorePlan,
    ) -> Result<(), nagi_history::activity::FailureCode> {
        Ok(())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Activity + Wayback preview failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), &'static str> {
    let locale = UserLocale::JaJp;
    let occurred_at = Timestamp::new(1_790_000_000, 0).map_err(|_| "invalid timestamp")?;
    let target_object = ObjectId(42);
    let transaction = TransactionId(17);
    let action_group = ActionGroupId(9);
    let mut activity = ActivityLedger::new();
    let undo = UndoDescriptor::new(
        InverseKind::RestoreObjectRevision,
        RevisionId(2),
        RevisionId(1),
    );
    let mut sandbox = InMemoryRestoreSandbox::new();
    sandbox
        .register_object(target_object, RevisionId(1), b"before")
        .map_err(|_| "could not seed host fixture")?;
    let checkpoint_object = CheckpointObject::new(target_object, RevisionId(1));
    let backend_ref = sandbox
        .capture(CheckpointScope::Document, &[checkpoint_object])
        .map_err(|_| "could not capture host fixture")?;
    let mut checkpoints = CheckpointStore::new();
    let checkpoint = checkpoints
        .create(
            CheckpointDraft::new(
                occurred_at,
                USER,
                CONTEXT,
                CheckpointScope::Document,
                CheckpointOrigin::User,
                CheckpointReason::UserRequested,
                backend_ref,
            )
            .with_object(checkpoint_object)
            .map_err(|_| "could not describe checkpoint object")?,
            &mut activity,
        )
        .map_err(|_| "could not record host checkpoint")?
        .0;

    sandbox
        .update_object(target_object, RevisionId(2), b"after")
        .map_err(|_| "could not update host fixture")?;
    activity
        .append(
            ActivityDraft::new(
                Timestamp::new(1_790_000_001, 0).map_err(|_| "invalid timestamp")?,
                USER,
                CONTEXT,
                ActionKind::ObjectChanged,
            )
            .with_result(EventResult::Succeeded)
            .with_target(target_object)
            .map_err(|_| "could not add Activity target")?
            .with_action_group(action_group)
            .with_transaction(transaction)
            .with_correlation(CorrelationId(33))
            .with_reversibility(Reversibility::Reversible(undo))
            .with_metadata(
                MetadataKey::DocumentContent,
                MetadataValue::Code(0),
                PrivacyClass::Content,
            )
            .map_err(|_| "could not add redacted Activity metadata")?,
        )
        .map_err(|_| "could not append Activity")?;
    let diff = compare_with_policy(
        &sandbox,
        &Allow,
        USER,
        backend_ref,
        target_object,
        Some(RevisionId(2)),
        RevisionId(1),
    )
    .map_err(|_| "could not compare host fixture")?;
    print_view("Wayback compare", |output| {
        render_diff_summary(target_object, diff, locale, output)
    })?;

    let plan = prepare_restore_plan(
        RestorePlanId(1),
        CheckpointReadStore::get_checkpoint_visible(&checkpoints, checkpoint, USER, &Allow)
            .ok_or("checkpoint disappeared")?,
        USER,
        Provenance::Direct {
            originating_intent: None,
        },
        CONTEXT,
        Timestamp::new(1_790_000_002, 0).map_err(|_| "invalid timestamp")?,
        RestoreMode::InPlace,
        &[target_object],
        &[CurrentObjectRevision::new(target_object, RevisionId(2))],
        CorrelationId(34),
        &mut activity,
    )
    .map_err(|_| "could not prepare restore preview")?;
    print_view("Wayback restore plan", |output| {
        nagi_history::wayback::render_restore_plan(
            &plan,
            locale,
            RestoreBackendAvailability::HostInMemorySandbox,
            output,
        )
    })?;

    let confirmation = plan
        .confirm(USER, true)
        .map_err(|_| "restore confirmation was not accepted")?;
    let outcome = execute_restore(
        confirmation,
        Timestamp::new(1_790_000_003, 0).map_err(|_| "invalid timestamp")?,
        &Allow,
        &mut checkpoints,
        &mut sandbox,
        &mut activity,
    )
    .map_err(|_| "host sandbox restore failed")?;
    print_view("Restore result", |output| {
        render_restore_outcome(outcome, locale, output)
    })?;
    let bytes = sandbox
        .object_bytes(target_object)
        .ok_or("restored host object is missing")?;
    println!(
        "Host sandbox object bytes after restore: {}",
        String::from_utf8_lossy(bytes)
    );

    print_view("Activity timeline", |output| {
        render_timeline(
            &activity,
            ActivityQuery::default(),
            USER,
            &Allow,
            &EpochTimeFormatter,
            locale,
            output,
        )
    })?;
    print_view("Wayback checkpoints", |output| {
        render_checkpoint_timeline(
            &checkpoints,
            CheckpointQuery::default().workspace(WorkspaceId(1)),
            USER,
            &Allow,
            &EpochTimeFormatter,
            locale,
            output,
        )
    })?;
    Ok(())
}

fn print_view<E: core::fmt::Debug>(
    label: &str,
    render: impl FnOnce(&mut [u8]) -> Result<usize, E>,
) -> Result<(), &'static str> {
    let mut output = [0; 2048];
    let length = render(&mut output).map_err(|_| "view rendering failed")?;
    let text = core::str::from_utf8(&output[..length]).map_err(|_| "view was not UTF-8")?;
    println!("--- {label} (host preview) ---\n{text}");
    Ok(())
}
