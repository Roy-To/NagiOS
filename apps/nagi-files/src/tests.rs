use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::localization::{self, Locale};
use crate::service::MAX_PENDING_CONFIRMATIONS;
use crate::{
    render_three_pane, ActivityEvent, ActivityOutcome, ActivitySink, Actor, CancellationToken,
    CapabilityAuthorizer, CapabilityGrant, CapabilityRight, CapabilitySet, CheckpointHook,
    ContextPublisher, EntryKind, FileName, FilesActionApi, FilesActionCall, FilesActionResponse,
    FilesApp, FilesContextSnapshot, FilesErrorKind, FilesSearchProvider, FilesService,
    FilesystemProvider, HookStatus, InMemoryProvider, Location, OperationKind, PermissionDecision,
    Reversibility, SandboxProvider, Tag, WaybackCheckpointRequest, WorkspaceReferenceSink,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct TempSandbox(PathBuf);

impl TempSandbox {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nagi-files-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated test sandbox");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn full_capabilities() -> CapabilitySet {
    CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Create),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Write),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Rename),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Move),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Delete),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Restore),
        CapabilityGrant::allow(Location::root(), CapabilityRight::PermanentDelete),
        CapabilityGrant::allow(Location::root(), CapabilityRight::SetMetadata),
        CapabilityGrant::allow(Location::root(), CapabilityRight::WorkspaceReference),
    ])
}

fn memory_service() -> FilesService<InMemoryProvider, CapabilitySet> {
    FilesService::new(InMemoryProvider::new(), full_capabilities())
}

#[derive(Clone, Default)]
struct ActivityRecorder(Arc<Mutex<Vec<ActivityEvent>>>);

impl ActivitySink for ActivityRecorder {
    fn record(&mut self, event: &ActivityEvent) -> Result<(), String> {
        self.0.lock().expect("activity mutex").push(event.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct CheckpointRecorder(Arc<Mutex<Vec<WaybackCheckpointRequest>>>);

impl CheckpointHook for CheckpointRecorder {
    fn checkpoint_before(&mut self, request: &WaybackCheckpointRequest) -> Result<String, String> {
        self.0
            .lock()
            .expect("checkpoint mutex")
            .push(request.clone());
        Ok(format!("checkpoint-{}", request.transaction_id.0))
    }
}

#[derive(Clone, Default)]
struct WorkspaceRecorder(Arc<Mutex<Vec<(String, crate::ResourceId)>>>);

impl WorkspaceReferenceSink for WorkspaceRecorder {
    fn add_reference(
        &mut self,
        workspace_id: &str,
        resource_id: crate::ResourceId,
    ) -> Result<(), String> {
        self.0
            .lock()
            .expect("workspace mutex")
            .push((workspace_id.to_owned(), resource_id));
        Ok(())
    }

    fn remove_reference(
        &mut self,
        workspace_id: &str,
        resource_id: crate::ResourceId,
    ) -> Result<(), String> {
        self.0
            .lock()
            .expect("workspace mutex")
            .retain(|reference| reference != &(workspace_id.to_owned(), resource_id));
        Ok(())
    }
}

#[derive(Default)]
struct ContextRecorder(Option<FilesContextSnapshot>);

impl ContextPublisher for ContextRecorder {
    fn publish(&mut self, context: &FilesContextSnapshot) -> Result<(), String> {
        self.0 = Some(context.clone());
        Ok(())
    }
}

#[test]
fn location_rejects_absolute_drive_and_parent_traversal_but_accepts_unicode() {
    for invalid in ["/outside", "../secret", "a/../secret", "C:/outside", "a\\b"] {
        assert!(Location::parse(invalid).is_err(), "must reject {invalid}");
    }
    assert_eq!(
        Location::parse("資料/日本語.txt").unwrap().as_str(),
        "資料/日本語.txt"
    );
    assert_eq!(Location::parse("a//b/").unwrap().as_str(), "a/b");
}

#[test]
fn filename_validation_keeps_unicode_and_rejects_separators() {
    assert_eq!(
        FileName::parse("報告書 2026.txt").unwrap().as_str(),
        "報告書 2026.txt"
    );
    for invalid in ["", ".", "..", "x/y", "x\\y", "C:secret"] {
        assert!(FileName::parse(invalid).is_err(), "must reject {invalid}");
    }
    for invalid in [
        "name:stream",
        "CON",
        "nul.txt",
        "LPT1",
        "trailing.",
        "trailing ",
    ] {
        assert!(
            FileName::parse(invalid).is_err(),
            "must reject host-special name {invalid}"
        );
    }
}

#[test]
fn memory_provider_enumerates_nested_navigation_and_empty_folders() {
    let mut provider = InMemoryProvider::new();
    let documents = provider
        .create_folder(&Location::root(), &FileName::parse("書類").unwrap())
        .unwrap();
    assert!(provider
        .list(&documents.child_location())
        .unwrap()
        .is_empty());
    let nested = documents
        .child_location()
        .join(&FileName::parse("下書き").unwrap());
    provider.insert_folder(&nested).unwrap();
    let file = nested.join(&FileName::parse("こんにちは.txt").unwrap());
    provider.insert_file(&file, b"hello").unwrap();
    assert_eq!(
        provider.list(&nested).unwrap()[0].name.as_str(),
        "こんにちは.txt"
    );
    assert_eq!(
        provider.read_file(&file, crate::MAX_PREVIEW_BYTES).unwrap(),
        b"hello"
    );
}

#[test]
fn memory_copy_move_and_rename_preserve_expected_identity() {
    let mut provider = InMemoryProvider::new();
    let folder = provider
        .create_folder(&Location::root(), &FileName::parse("source").unwrap())
        .unwrap();
    let source = folder
        .child_location()
        .join(&FileName::parse("note.txt").unwrap());
    let original = provider.insert_file(&source, b"contents").unwrap();
    let copied = provider
        .copy(
            &folder.child_location(),
            &Location::root(),
            &FileName::parse("source copy").unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_ne!(copied.id, folder.id);
    let copied_file = Location::parse("source copy/note.txt").unwrap();
    assert_eq!(
        provider
            .read_file(&copied_file, crate::MAX_PREVIEW_BYTES)
            .unwrap(),
        b"contents"
    );
    let renamed = provider
        .rename(&source, &FileName::parse("renamed.txt").unwrap())
        .unwrap();
    assert_eq!(renamed.id, original.id);
    let destination = provider
        .create_folder(&Location::root(), &FileName::parse("destination").unwrap())
        .unwrap();
    let moved = provider
        .move_item(
            &Location::parse("source/renamed.txt").unwrap(),
            &destination.child_location(),
            &FileName::parse("renamed.txt").unwrap(),
        )
        .unwrap();
    assert_eq!(moved.id, original.id);
    assert_eq!(
        provider
            .read_file(
                &Location::parse("destination/renamed.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"contents"
    );
}

#[test]
fn memory_copy_cancellation_leaves_no_partial_destination() {
    let mut provider = InMemoryProvider::new();
    provider
        .insert_file(&Location::parse("source.txt").unwrap(), b"data")
        .unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let error = provider
        .copy(
            &Location::parse("source.txt").unwrap(),
            &Location::root(),
            &FileName::parse("copy.txt").unwrap(),
            &token,
        )
        .unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::Cancelled);
    assert_eq!(provider.list(&Location::root()).unwrap().len(), 1);
}

#[test]
fn operation_service_enforces_permission_decisions_and_unavailable_provider() {
    let provider = InMemoryProvider::new();
    let denied = FilesService::new(provider, CapabilitySet::new());
    assert_eq!(
        denied.list(&Location::root()).unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );

    let asked = CapabilitySet::from_grants([CapabilityGrant::ask(
        Location::root(),
        CapabilityRight::Enumerate,
    )]);
    let service = FilesService::new(InMemoryProvider::new(), asked);
    assert_eq!(
        service.list(&Location::root()).unwrap_err().kind,
        FilesErrorKind::PermissionRequired
    );

    let unavailable = CapabilitySet::from_grants([CapabilityGrant::unavailable(
        Location::root(),
        CapabilityRight::Enumerate,
    )]);
    let service = FilesService::new(InMemoryProvider::new(), unavailable);
    assert_eq!(
        service.list(&Location::root()).unwrap_err().kind,
        FilesErrorKind::CapabilityUnavailable
    );

    let mut provider = InMemoryProvider::new();
    provider.set_unavailable("test provider offline");
    let service = FilesService::new(provider, full_capabilities());
    assert_eq!(
        service.list(&Location::root()).unwrap_err().kind,
        FilesErrorKind::ProviderUnavailable
    );
}

#[test]
fn more_specific_ask_or_deny_overrides_a_broad_capability_grant() {
    let path = Location::parse("private/key.txt").unwrap();
    let asked = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        CapabilityGrant::ask(Location::parse("private").unwrap(), CapabilityRight::Read),
    ]);
    assert_eq!(
        asked.decide(&crate::CapabilityRequest {
            right: CapabilityRight::Read,
            location: path.clone(),
        }),
        PermissionDecision::Ask
    );
    let denied = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        CapabilityGrant::deny(Location::parse("private").unwrap(), CapabilityRight::Read),
    ]);
    assert_eq!(
        denied.decide(&crate::CapabilityRequest {
            right: CapabilityRight::Read,
            location: path,
        }),
        PermissionDecision::Deny
    );
}

#[test]
fn operation_service_rejects_out_of_scope_and_forged_resource_handles() {
    let mut provider = InMemoryProvider::new();
    provider
        .insert_file(&Location::parse("inside.txt").unwrap(), b"ok")
        .unwrap();
    let capabilities = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(
            Location::parse("inside.txt").unwrap(),
            CapabilityRight::Read,
        ),
        CapabilityGrant::allow(
            Location::parse("inside.txt").unwrap(),
            CapabilityRight::Rename,
        ),
    ]);
    let mut service = FilesService::new(provider, capabilities);
    assert_eq!(
        service
            .metadata(&Location::parse("outside.txt").unwrap())
            .unwrap_err()
            .kind,
        FilesErrorKind::PermissionDenied
    );

    let mut forged = service
        .metadata(&Location::parse("inside.txt").unwrap())
        .unwrap();
    forged.id = crate::ResourceId(9999);
    assert_eq!(
        service
            .rename(
                &forged,
                &FileName::parse("renamed.txt").unwrap(),
                Actor::User
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::Conflict
    );
}

#[test]
fn operation_service_runs_typed_actions_and_reports_missing_hooks() {
    let mut service = memory_service();
    let (folder, result) = service
        .create_folder(
            &Location::root(),
            &FileName::parse("Notes").unwrap(),
            Actor::User,
        )
        .unwrap();
    assert_eq!(result.action_id, OperationKind::CreateFolder.action_id());
    assert_eq!(result.reversibility, Reversibility::Reversible);
    assert_eq!(result.activity, HookStatus::Unavailable);
    assert_eq!(result.checkpoint, HookStatus::Unavailable);
    assert_eq!(
        service.metadata(&folder.child_location()).unwrap().id,
        folder.id
    );
}

#[test]
fn activity_and_wayback_hooks_receive_transaction_and_truthful_reversibility() {
    let activity = ActivityRecorder::default();
    let checkpoints = CheckpointRecorder::default();
    let activity_records = activity.0.clone();
    let checkpoint_records = checkpoints.0.clone();
    let mut service = FilesService::new(InMemoryProvider::new(), full_capabilities())
        .with_activity_sink(activity)
        .with_checkpoint_hook(checkpoints);
    let (_, result) = service
        .create_folder(
            &Location::root(),
            &FileName::parse("folder").unwrap(),
            Actor::Agent,
        )
        .unwrap();
    assert_eq!(result.activity, HookStatus::Recorded);
    assert_eq!(
        result.checkpoint,
        HookStatus::Created(format!("checkpoint-{}", result.transaction_id.0))
    );
    let events = activity_records.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].action_id, "files.create_folder");
    assert_eq!(events[0].result, ActivityOutcome::Started);
    assert_eq!(events[1].result, ActivityOutcome::Succeeded);
    assert_eq!(events[0].transaction_id, result.transaction_id);
    assert_eq!(events[1].transaction_id, result.transaction_id);
    assert!(events[1].reversible);
    drop(events);
    let requests = checkpoint_records.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transaction_id, result.transaction_id);
    assert_eq!(requests[0].reversibility, Reversibility::Reversible);
}

#[test]
fn checkpoint_failure_is_visible_and_does_not_fake_checkpoint_success() {
    struct BrokenCheckpoint;
    impl CheckpointHook for BrokenCheckpoint {
        fn checkpoint_before(&mut self, _: &WaybackCheckpointRequest) -> Result<String, String> {
            Err("snapshot service down".into())
        }
    }
    let mut service = FilesService::new(InMemoryProvider::new(), full_capabilities())
        .with_checkpoint_hook(BrokenCheckpoint);
    let (folder, result) = service
        .create_folder(
            &Location::root(),
            &FileName::parse("folder").unwrap(),
            Actor::User,
        )
        .unwrap();
    assert_eq!(
        result.checkpoint,
        HookStatus::Failed("snapshot service down".into())
    );
    assert_eq!(
        service.metadata(&folder.child_location()).unwrap().id,
        folder.id
    );
}

#[test]
fn agent_mutation_fails_closed_when_activity_is_unavailable() {
    let mut service = memory_service();
    let error = service
        .create_folder(
            &Location::root(),
            &FileName::parse("agent-folder").unwrap(),
            Actor::Agent,
        )
        .unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::ActivityUnavailable);
    assert!(service.list(&Location::root()).unwrap().is_empty());
}

#[test]
fn duplicate_action_has_its_own_action_id_and_preserves_the_source() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("notes.txt").unwrap(), b"contents")
        .unwrap();
    let (copy, result) = service
        .duplicate(&source, Actor::User, &CancellationToken::new())
        .unwrap();
    assert_eq!(result.action_id, "files.duplicate");
    assert_ne!(copy.id, source.id);
    assert_eq!(
        service
            .read_file(&Location::parse("notes.txt").unwrap())
            .unwrap(),
        b"contents"
    );
    assert_eq!(
        service.read_file(&copy.child_location()).unwrap(),
        b"contents"
    );
}

#[test]
fn tags_are_resource_metadata_with_permission_checks_and_copy_identity_rules() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("tagged.txt").unwrap(), b"contents")
        .unwrap();
    let tags = [
        Tag::parse(" research ").unwrap(),
        Tag::parse("日本語").unwrap(),
    ];
    let (tagged, result) = service.set_tags(&source, &tags, Actor::User).unwrap();
    assert_eq!(result.action_id, "files.set_tags");
    assert_eq!(tagged.tags, vec!["research", "日本語"]);
    let (_, duplicate) = service
        .duplicate(&tagged, Actor::User, &CancellationToken::new())
        .unwrap();
    let copied = service
        .provider()
        .list(&Location::root())
        .unwrap()
        .into_iter()
        .find(|entry| entry.name.as_str() == "tagged copy.txt")
        .unwrap();
    assert_eq!(copied.tags, Vec::<String>::new());
    assert_eq!(duplicate.action_id, "files.duplicate");
    assert_eq!(
        Tag::parse("\n").unwrap_err().kind,
        FilesErrorKind::InvalidName
    );
}

#[test]
fn workspace_action_adds_a_reference_without_moving_the_file() {
    let workspace = WorkspaceRecorder::default();
    let references = workspace.0.clone();
    let mut service = FilesService::new(InMemoryProvider::new(), full_capabilities())
        .with_workspace_sink(workspace);
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("project.txt").unwrap(), b"content")
        .unwrap();
    let result = service
        .add_to_workspace(&source, "workspace-42", Actor::User)
        .unwrap();
    assert_eq!(result.operation.action_id, "files.add_to_workspace");
    assert_eq!(result.reference, HookStatus::Recorded);
    assert_eq!(
        service
            .metadata(&Location::parse("project.txt").unwrap())
            .unwrap()
            .id,
        source.id
    );
    assert_eq!(
        service
            .provider()
            .read_file(
                &Location::parse("project.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"content"
    );
    assert_eq!(
        *references.lock().unwrap(),
        vec![("workspace-42".to_owned(), source.id)]
    );
    let removed = service
        .remove_from_workspace(&source, "workspace-42", Actor::User)
        .unwrap();
    assert_eq!(removed.operation.action_id, "files.remove_from_workspace");
    assert!(references.lock().unwrap().is_empty());
    assert!(service
        .metadata(&Location::parse("project.txt").unwrap())
        .is_ok());
}

#[test]
fn workspace_action_reports_missing_provider_without_changing_resource() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("project.txt").unwrap(), b"content")
        .unwrap();
    assert_eq!(
        service
            .add_to_workspace(&source, "workspace-42", Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::WorkspaceUnavailable
    );
    assert_eq!(
        service
            .provider()
            .read_file(
                &Location::parse("project.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"content"
    );
}

#[test]
fn selected_context_snapshot_is_published_through_a_typed_boundary() {
    let mut service = memory_service();
    service
        .provider_mut()
        .insert_file(&Location::parse("context.txt").unwrap(), b"x")
        .unwrap();
    let mut app = FilesApp::new(Location::root());
    app.refresh(&service).unwrap();
    let id = app.state.entries[0].id;
    app.state.select(id, false).unwrap();
    let mut publisher = ContextRecorder::default();
    app.publish_context(&mut publisher).unwrap();
    let published = publisher.0.unwrap();
    assert_eq!(published.current_location, Location::root());
    assert_eq!(published.selected_resources, vec![id]);
    assert_eq!(published.focused_resource, Some(id));
}

#[test]
fn sandbox_tags_survive_restart_move_and_trash_restore_without_sidecars() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("destination")).unwrap();
    fs::write(temp.path().join("report.txt"), b"report").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let entry = provider
        .metadata(&Location::parse("report.txt").unwrap())
        .unwrap();
    let tagged = provider
        .set_tags(
            &Location::parse("report.txt").unwrap(),
            &["finance".into(), "東京".into()],
        )
        .unwrap();
    assert_eq!(tagged.tags, vec!["finance", "東京"]);
    let moved = provider
        .move_item(
            &Location::parse("report.txt").unwrap(),
            &Location::parse("destination").unwrap(),
            &FileName::parse("report.txt").unwrap(),
        )
        .unwrap();
    assert_eq!(moved.id, entry.id);
    assert_eq!(moved.tags, vec!["finance", "東京"]);
    drop(provider);
    provider = SandboxProvider::new(temp.path()).unwrap();
    assert_eq!(
        provider
            .metadata(&Location::parse("destination/report.txt").unwrap())
            .unwrap()
            .tags,
        vec!["finance", "東京"]
    );
    let trashed = provider
        .trash(&Location::parse("destination/report.txt").unwrap())
        .unwrap();
    provider.restore(trashed.id).unwrap();
    assert_eq!(
        provider
            .metadata(&Location::parse("destination/report.txt").unwrap())
            .unwrap()
            .tags,
        vec!["finance", "東京"]
    );
    let internal_files = fs::read_dir(temp.path().join(".nagi-files"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert!(internal_files.iter().any(|name| name == "tags-v1"));
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[test]
fn permanent_delete_requires_a_bound_one_use_confirmation() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("old.txt").unwrap(), b"old")
        .unwrap();
    let (trash, _) = service.trash(&source, Actor::User).unwrap();
    let challenge = service.request_permanent_delete(trash.id).unwrap();
    assert_eq!(service.list_trash().unwrap().len(), 1);
    assert_eq!(
        service
            .confirm_permanent_delete(challenge.clone(), Actor::Agent)
            .unwrap_err()
            .kind,
        FilesErrorKind::PermissionRequired
    );
    assert_eq!(service.list_trash().unwrap().len(), 1);
    let replay = challenge.clone();
    let result = service
        .confirm_permanent_delete(challenge, Actor::User)
        .unwrap();
    assert_eq!(result.reversibility, Reversibility::Irreversible);
    assert_eq!(result.checkpoint, HookStatus::NotApplicable);
    assert!(service.list_trash().unwrap().is_empty());
    assert_eq!(
        service
            .confirm_permanent_delete(replay, Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::InvalidConfirmation
    );
}

#[test]
fn pending_permanent_delete_confirmations_are_bounded_and_release_capacity() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("old.txt").unwrap(), b"old")
        .unwrap();
    let (trash, _) = service.trash(&source, Actor::User).unwrap();
    let mut challenges = Vec::new();
    for _ in 0..MAX_PENDING_CONFIRMATIONS {
        challenges.push(service.request_permanent_delete(trash.id).unwrap());
    }

    assert_eq!(
        service.request_permanent_delete(trash.id).unwrap_err().kind,
        FilesErrorKind::ConfirmationLimitReached
    );

    service
        .confirm_permanent_delete(challenges.remove(0), Actor::User)
        .unwrap();
    let another = service
        .provider_mut()
        .insert_file(&Location::parse("another.txt").unwrap(), b"another")
        .unwrap();
    let (another_trash, _) = service.trash(&another, Actor::User).unwrap();
    assert!(service.request_permanent_delete(another_trash.id).is_ok());
}

#[test]
fn canceling_permanent_delete_confirmation_releases_its_one_use_slot() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("old.txt").unwrap(), b"old")
        .unwrap();
    let (trash, _) = service.trash(&source, Actor::User).unwrap();
    let challenge = service.request_permanent_delete(trash.id).unwrap();

    assert_eq!(
        service
            .cancel_permanent_delete_confirmation(challenge.clone(), Actor::Agent)
            .unwrap_err()
            .kind,
        FilesErrorKind::PermissionRequired
    );
    service
        .cancel_permanent_delete_confirmation(challenge.clone(), Actor::User)
        .unwrap();
    assert_eq!(
        service
            .confirm_permanent_delete(challenge.clone(), Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::InvalidConfirmation
    );
    assert!(service.request_permanent_delete(trash.id).is_ok());
}

#[test]
fn mismatched_cross_service_confirmations_do_not_consume_pending_challenges() {
    let mut issuer = memory_service();
    let issued_file = issuer
        .provider_mut()
        .insert_file(&Location::parse("issued.txt").unwrap(), b"issued")
        .unwrap();
    let (issued_trash, _) = issuer.trash(&issued_file, Actor::User).unwrap();
    let issued_challenge = issuer.request_permanent_delete(issued_trash.id).unwrap();

    let mut cancel_target = memory_service();
    let cancel_file = cancel_target
        .provider_mut()
        .insert_file(&Location::parse("cancel-target.txt").unwrap(), b"target")
        .unwrap();
    let (cancel_trash, _) = cancel_target.trash(&cancel_file, Actor::User).unwrap();
    let cancel_target_challenge = cancel_target
        .request_permanent_delete(cancel_trash.id)
        .unwrap();
    assert_eq!(
        cancel_target
            .cancel_permanent_delete_confirmation(issued_challenge.clone(), Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::InvalidConfirmation
    );
    cancel_target
        .confirm_permanent_delete(cancel_target_challenge, Actor::User)
        .unwrap();

    let mut confirm_target = memory_service();
    let confirm_file = confirm_target
        .provider_mut()
        .insert_file(&Location::parse("confirm-target.txt").unwrap(), b"target")
        .unwrap();
    let (confirm_trash, _) = confirm_target.trash(&confirm_file, Actor::User).unwrap();
    let confirm_target_challenge = confirm_target
        .request_permanent_delete(confirm_trash.id)
        .unwrap();
    assert_eq!(
        confirm_target
            .confirm_permanent_delete(issued_challenge, Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::InvalidConfirmation
    );
    confirm_target
        .cancel_permanent_delete_confirmation(confirm_target_challenge, Actor::User)
        .unwrap();
}

#[test]
fn files_ui_tracks_breadcrumbs_multi_selection_and_empty_loading_error_states() {
    let mut service = memory_service();
    let folder = service
        .provider_mut()
        .create_folder(&Location::root(), &FileName::parse("parent").unwrap())
        .unwrap();
    let nested = folder
        .child_location()
        .join(&FileName::parse("child").unwrap());
    service.provider_mut().insert_folder(&nested).unwrap();
    let first = service
        .provider_mut()
        .insert_file(&Location::parse("parent/first.txt").unwrap(), b"1")
        .unwrap();
    let second = service
        .provider_mut()
        .insert_file(&Location::parse("parent/second.txt").unwrap(), b"2")
        .unwrap();

    let mut app = FilesApp::new(Location::root());
    app.refresh(&service).unwrap();
    assert_eq!(app.state.load_state, crate::ViewLoadState::Ready);
    app.navigate(&service, Location::parse("parent").unwrap())
        .unwrap();
    assert_eq!(app.state.breadcrumbs.len(), 2);
    app.state.select(first.id, false).unwrap();
    app.state.select(second.id, true).unwrap();
    assert_eq!(app.state.selection_context().len(), 2);
    let layout = app.state.layout();
    let rendered = render_three_pane(&layout, "empty", Locale::JaJp);
    assert!(rendered.contains("場所"));
    assert!(rendered.contains("2 *[F] first.txt"));
    assert!(rendered.contains("3 *[F] second.txt"));
    assert!(rendered.contains("*"));

    app.navigate(&service, nested).unwrap();
    assert!(app.state.entries.is_empty());
    let rendered = render_three_pane(&app.state.layout(), "empty-state", Locale::EnUs);
    assert!(rendered.contains("empty-state"));
    assert!(app
        .navigate(&service, Location::parse("does-not-exist").unwrap())
        .is_err());
    assert!(matches!(
        app.state.load_state,
        crate::ViewLoadState::Error(FilesErrorKind::NotFound)
    ));
}

#[test]
fn selection_model_handles_a_large_directory_without_losing_identity() {
    let mut service = memory_service();
    for index in 0..2_000 {
        let location = Location::parse(&format!("item-{index:04}.txt")).unwrap();
        service.provider_mut().insert_file(&location, b"").unwrap();
    }
    let mut app = FilesApp::new(Location::root());
    app.refresh(&service).unwrap();
    assert_eq!(app.state.entries.len(), 2_000);
    let first = app.state.entries[17].id;
    let last = app.state.entries[1_997].id;
    app.state.select(first, false).unwrap();
    app.state.select(last, true).unwrap();
    assert_eq!(app.state.selected.len(), 2);
    assert!(app
        .state
        .select(crate::ResourceId(u128::MAX), true)
        .is_err());
}

#[test]
fn search_filters_inaccessible_resources_and_keeps_metadata_only_results() {
    let mut provider = InMemoryProvider::new();
    provider
        .insert_file(&Location::parse("secret-report.txt").unwrap(), b"secret")
        .unwrap();
    let capabilities = CapabilitySet::from_grants([CapabilityGrant::allow(
        Location::root(),
        CapabilityRight::Enumerate,
    )]);
    let service = FilesService::new(provider, capabilities);
    let results = FilesSearchProvider::default()
        .search(&service, &Location::root(), "secret", 10)
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_finds_filename_and_location_metadata_without_reading_content() {
    let mut provider = InMemoryProvider::new();
    let folder = provider
        .create_folder(&Location::root(), &FileName::parse("Reports").unwrap())
        .unwrap();
    provider
        .insert_file(
            &folder
                .child_location()
                .join(&FileName::parse("Annual.txt").unwrap()),
            b"body",
        )
        .unwrap();
    let service = FilesService::new(provider, full_capabilities());
    let results = FilesSearchProvider::default()
        .search(&service, &Location::root(), "annual", 10)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Annual.txt");
    assert_eq!(results[0].location.as_str(), "Reports/Annual.txt");
}

#[test]
fn search_matches_tags_without_indexing_file_contents() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("data.bin").unwrap(), b"secret body")
        .unwrap();
    let tagged = service
        .set_tags(&source, &[Tag::parse("quarterly").unwrap()], Actor::User)
        .unwrap()
        .0;
    let results = FilesSearchProvider::default()
        .search(&service, &Location::root(), "quarterly", 10)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].resource_id, tagged.id);
    assert_eq!(results[0].tags, vec!["quarterly"]);
    assert!(FilesSearchProvider::default()
        .search(&service, &Location::root(), "secret body", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn localization_has_matching_nonempty_english_and_japanese_keys_and_fallback() {
    let english = localization::keys(localization::EN_US);
    let japanese = localization::keys(localization::JA_JP);
    assert_eq!(english, japanese);
    assert!(english.iter().all(|key| {
        localization::text(Locale::EnUs, key).is_some_and(|value| !value.is_empty())
            && localization::text(Locale::JaJp, key).is_some_and(|value| !value.is_empty())
    }));
    assert_eq!(localization::Locale::parse("ja-JP"), Locale::JaJp);
    assert_eq!(localization::Locale::parse("ja"), Locale::JaJp);
    assert_eq!(localization::Locale::parse("fr-FR"), Locale::EnUs);
    assert_eq!(localization::text(Locale::JaJp, "missing.key"), None);
}

#[test]
fn sandbox_enumeration_navigation_unicode_empty_folder_and_metadata() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("empty")).unwrap();
    fs::create_dir(temp.path().join("資料")).unwrap();
    fs::write(temp.path().join("資料/日本語.txt"), "Nagi Files").unwrap();
    let provider = SandboxProvider::new(temp.path()).unwrap();
    assert!(provider
        .list(&Location::parse("empty").unwrap())
        .unwrap()
        .is_empty());
    let entries = provider.list(&Location::parse("資料").unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name.as_str(), "日本語.txt");
    assert_eq!(entries[0].kind, EntryKind::File);
    assert_eq!(entries[0].size_bytes, Some(10));
    assert!(entries[0].modified_at.is_some());
    assert_eq!(
        provider
            .read_file(
                &Location::parse("資料/日本語.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"Nagi Files"
    );
}

#[test]
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn sandbox_case_insensitive_path_alias_keeps_scoped_deny_effective() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("Private")).unwrap();
    fs::write(temp.path().join("Private/secret.txt"), b"classified").unwrap();
    assert!(temp.path().join("private/secret.txt").exists());

    let authorizer = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        CapabilityGrant::deny(Location::parse("Private").unwrap(), CapabilityRight::Read),
    ]);
    let service = FilesService::new(SandboxProvider::new(temp.path()).unwrap(), authorizer);
    let alias = Location::parse("private/secret.txt").unwrap();

    assert_eq!(
        service.read_file(&alias).unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
}

#[test]
fn sandbox_rejects_hidden_internal_metadata_and_symlink_path_traversal() {
    let temp = TempSandbox::new();
    let outside = TempSandbox::new();
    fs::write(outside.path().join("secret.txt"), b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), temp.path().join("escape")).unwrap();
    let provider = SandboxProvider::new(temp.path()).unwrap();
    #[cfg(unix)]
    assert_eq!(
        provider.list(&Location::root()).unwrap()[0].kind,
        EntryKind::Symlink
    );
    assert_eq!(
        provider
            .list(&Location::parse(".nagi-files/trash").unwrap())
            .unwrap_err()
            .kind,
        FilesErrorKind::SandboxEscape
    );
    assert_eq!(
        provider
            .list(&Location::parse(".NAGI-FILES/trash").unwrap())
            .unwrap_err()
            .kind,
        FilesErrorKind::SandboxEscape
    );
    #[cfg(unix)]
    assert_eq!(
        provider
            .list(&Location::parse("escape").unwrap())
            .unwrap_err()
            .kind,
        FilesErrorKind::SymlinkNotAllowed
    );
    assert_eq!(
        fs::read(outside.path().join("secret.txt")).unwrap(),
        b"outside"
    );
    #[cfg(unix)]
    {
        let root_alias = outside.path().join("sandbox-root-link");
        std::os::unix::fs::symlink(temp.path(), &root_alias).unwrap();
        assert!(SandboxProvider::new(&root_alias).is_err());
        fs::remove_file(root_alias).unwrap();
    }
}

#[test]
#[cfg(unix)]
fn sandbox_symlinks_are_presented_but_never_followed_or_copied() {
    let temp = TempSandbox::new();
    let outside = TempSandbox::new();
    fs::write(outside.path().join("private.txt"), b"private").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("private.txt"),
        temp.path().join("link.txt"),
    )
    .unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    #[cfg(unix)]
    {
        let link = service.list(&Location::root()).unwrap().remove(0);
        assert_eq!(link.kind, EntryKind::Symlink);
        assert_eq!(
            service.open_file(&link, Actor::User).unwrap_err().kind,
            FilesErrorKind::UnsupportedEntry
        );
        assert_eq!(
            service
                .copy(
                    &link,
                    &Location::root(),
                    &FileName::parse("copy.txt").unwrap(),
                    Actor::User,
                    &CancellationToken::new()
                )
                .unwrap_err()
                .kind,
            FilesErrorKind::SymlinkNotAllowed
        );
    }
    assert_eq!(
        fs::read(outside.path().join("private.txt")).unwrap(),
        b"private"
    );
}

#[test]
fn sandbox_copy_nested_rename_move_identity_conflicts_and_descendant_guard() {
    let temp = TempSandbox::new();
    fs::create_dir_all(temp.path().join("source/nested")).unwrap();
    fs::create_dir(temp.path().join("destination")).unwrap();
    fs::write(temp.path().join("source/nested/file.txt"), b"payload").unwrap();
    fs::write(temp.path().join("conflict.txt"), b"keep").unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let source = service
        .list(&Location::root())
        .unwrap()
        .into_iter()
        .find(|item| item.name.as_str() == "source")
        .unwrap();
    let (copy, _) = service
        .copy(
            &source,
            &Location::root(),
            &FileName::parse("source-copy").unwrap(),
            Actor::User,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_ne!(copy.id, source.id);
    assert_eq!(
        fs::read(temp.path().join("source-copy/nested/file.txt")).unwrap(),
        b"payload"
    );
    assert_eq!(
        service
            .copy(
                &source,
                &Location::root(),
                &FileName::parse("source-copy").unwrap(),
                Actor::User,
                &CancellationToken::new()
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::Conflict
    );
    let same_directory_conflict = service
        .rename(
            &source,
            &FileName::parse("conflict.txt").unwrap(),
            Actor::User,
        )
        .unwrap_err();
    assert_eq!(same_directory_conflict.kind, FilesErrorKind::Conflict);
    let (renamed, _) = service
        .rename(&copy, &FileName::parse("renamed").unwrap(), Actor::User)
        .unwrap();
    assert_eq!(renamed.id, copy.id);
    let destination = Location::parse("destination").unwrap();
    let (moved, _) = service
        .move_item(
            &renamed,
            &destination,
            &FileName::parse("renamed").unwrap(),
            Actor::User,
        )
        .unwrap();
    assert_eq!(moved.id, copy.id);
    assert_eq!(
        fs::read(temp.path().join("destination/renamed/nested/file.txt")).unwrap(),
        b"payload"
    );
    let error = service
        .move_item(
            &source,
            &Location::parse("source/nested").unwrap(),
            &FileName::parse("inside").unwrap(),
            Actor::User,
        )
        .unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::DestinationInsideSource);
    assert_eq!(fs::read(temp.path().join("conflict.txt")).unwrap(), b"keep");
}

#[test]
fn sandbox_missing_source_and_invalid_destination_fail_without_side_effects() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("file.txt"), b"file").unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let stale = service.list(&Location::root()).unwrap().remove(0);
    fs::remove_file(temp.path().join("file.txt")).unwrap();
    assert_eq!(
        service
            .rename(
                &stale,
                &FileName::parse("renamed.txt").unwrap(),
                Actor::User
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::NotFound
    );
    fs::write(temp.path().join("file.txt"), b"file").unwrap();
    let fresh = service
        .metadata(&Location::parse("file.txt").unwrap())
        .unwrap();
    assert_eq!(
        service
            .copy(
                &fresh,
                &Location::parse("missing").unwrap(),
                &FileName::parse("copy.txt").unwrap(),
                Actor::User,
                &CancellationToken::new()
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::NotFound
    );
    assert!(!temp.path().join("missing/copy.txt").exists());
}

#[test]
#[cfg(unix)]
fn sandbox_partial_copy_failure_rolls_back_new_destination() {
    let temp = TempSandbox::new();
    let outside = TempSandbox::new();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::write(temp.path().join("source/a.txt"), b"a").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), temp.path().join("source/escape")).unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let source = service.list(&Location::root()).unwrap().remove(0);
    assert_eq!(
        service
            .copy(
                &source,
                &Location::root(),
                &FileName::parse("partial-copy").unwrap(),
                Actor::User,
                &CancellationToken::new()
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::SymlinkNotAllowed
    );
    assert!(!temp.path().join("partial-copy").exists());
}

#[test]
fn sandbox_copy_cancellation_creates_no_destination() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::write(temp.path().join("source/file.txt"), b"contents").unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let source = service.list(&Location::root()).unwrap().remove(0);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        service
            .copy(
                &source,
                &Location::root(),
                &FileName::parse("cancelled-copy").unwrap(),
                Actor::User,
                &cancellation,
            )
            .unwrap_err()
            .kind,
        FilesErrorKind::Cancelled
    );
    assert!(!temp.path().join("cancelled-copy").exists());
    assert_eq!(
        fs::read(temp.path().join("source/file.txt")).unwrap(),
        b"contents"
    );
}

#[test]
fn sandbox_trash_persists_and_restore_preserves_resource_identity() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("restore.txt"), b"restore me").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let original = provider
        .metadata(&Location::parse("restore.txt").unwrap())
        .unwrap();
    let trashed = provider
        .trash(&Location::parse("restore.txt").unwrap())
        .unwrap();
    assert_eq!(trashed.id, original.id);
    assert!(!temp.path().join("restore.txt").exists());
    drop(provider);

    provider = SandboxProvider::new(temp.path()).unwrap();
    assert_eq!(provider.list_trash().unwrap().len(), 1);
    let restored = provider.restore(trashed.id).unwrap();
    assert_eq!(restored.id, original.id);
    assert_eq!(
        provider
            .read_file(
                &Location::parse("restore.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"restore me"
    );
    assert!(provider.list_trash().unwrap().is_empty());
}

#[test]
fn sandbox_restore_conflict_preserves_trash_for_later_restore() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("item.txt"), b"old").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let trashed = provider
        .trash(&Location::parse("item.txt").unwrap())
        .unwrap();
    fs::write(temp.path().join("item.txt"), b"new").unwrap();
    assert_eq!(
        provider.restore(trashed.id).unwrap_err().kind,
        FilesErrorKind::Conflict
    );
    assert_eq!(provider.list_trash().unwrap().len(), 1);
    assert_eq!(fs::read(temp.path().join("item.txt")).unwrap(), b"new");
    fs::remove_file(temp.path().join("item.txt")).unwrap();
    provider.restore(trashed.id).unwrap();
    assert_eq!(fs::read(temp.path().join("item.txt")).unwrap(), b"old");
}

#[test]
fn sandbox_permanent_delete_requires_confirmation_and_removes_trash_data() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("erase.txt"), b"erase").unwrap();
    let mut service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let source = service.list(&Location::root()).unwrap().remove(0);
    let (trash, _) = service.trash(&source, Actor::User).unwrap();
    let challenge = service.request_permanent_delete(trash.id).unwrap();
    assert_eq!(service.list_trash().unwrap().len(), 1);
    let result = service
        .confirm_permanent_delete(challenge, Actor::User)
        .unwrap();
    assert_eq!(result.reversibility, Reversibility::Irreversible);
    assert!(service.list_trash().unwrap().is_empty());
    assert!(temp
        .path()
        .join(".nagi-files/trash")
        .read_dir()
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn localization_resources_include_navigation_error_selection_and_conflict_strings() {
    for required in [
        "files.sidebar.home",
        "files.sidebar.trash",
        "files.empty",
        "files.loading",
        "files.selection.count",
        "files.error.permission",
        "files.error.conflict",
        "files.confirm.permanent_delete",
    ] {
        assert!(
            localization::text(Locale::EnUs, required).is_some(),
            "missing en-US {required}"
        );
        assert!(
            localization::text(Locale::JaJp, required).is_some(),
            "missing ja-JP {required}"
        );
    }
}

#[test]
fn memory_trash_and_restore_keep_identity_and_conflict_state() {
    let mut provider = InMemoryProvider::new();
    let source = provider
        .insert_file(&Location::parse("item.txt").unwrap(), b"data")
        .unwrap();
    let trash = provider
        .trash(&Location::parse("item.txt").unwrap())
        .unwrap();
    assert_eq!(trash.id, source.id);
    let restored = provider.restore(trash.id).unwrap();
    assert_eq!(restored.id, source.id);
    assert_eq!(
        provider
            .read_file(
                &Location::parse("item.txt").unwrap(),
                crate::MAX_PREVIEW_BYTES
            )
            .unwrap(),
        b"data"
    );
}

#[test]
fn operation_preconditions_distinguish_reversible_and_irreversible_actions() {
    assert_eq!(
        OperationKind::Trash.reversibility(),
        Reversibility::Reversible
    );
    assert_eq!(
        OperationKind::PermanentDelete.reversibility(),
        Reversibility::Irreversible
    );
    assert_eq!(
        OperationKind::Open.reversibility(),
        Reversibility::NoMutation
    );
    assert_eq!(
        CapabilitySet::from_grants([CapabilityGrant::deny(
            Location::root(),
            CapabilityRight::Delete
        )])
        .decide(&crate::CapabilityRequest {
            right: CapabilityRight::Delete,
            location: Location::parse("x").unwrap()
        }),
        PermissionDecision::Deny
    );
}

#[test]
fn files_action_catalog_has_stable_ids_rights_and_destructive_risk() {
    assert_eq!(crate::FILES_ACTIONS.len(), 15);
    assert!(crate::FILES_ACTIONS.iter().all(|action| {
        action.id.starts_with("files.")
            && action.input_schema.ends_with(".v1")
            && !action.required_rights.is_empty()
    }));
    assert_eq!(
        crate::find_action("files.delete_permanently").unwrap().risk,
        crate::ActionRisk::Destructive
    );
    assert!(
        !crate::find_action("files.delete_permanently")
            .unwrap()
            .reversible
    );
    assert!(crate::find_action("files.unknown").is_none());
}

#[test]
fn typed_action_api_dispatches_calls_through_files_service() {
    let mut service = memory_service();
    let created = FilesActionApi::execute(
        &mut service,
        FilesActionCall::CreateFolder {
            parent: Location::root(),
            name: FileName::parse("reports").unwrap(),
        },
        Actor::User,
    )
    .unwrap();
    let FilesActionResponse::ResourceChanged {
        resource: folder,
        operation,
    } = created
    else {
        panic!("create-folder action must return the created resource");
    };
    assert_eq!(operation.action_id, "files.create_folder");

    let listed = FilesActionApi::execute(
        &mut service,
        FilesActionCall::List {
            location: Location::root(),
        },
        Actor::User,
    )
    .unwrap();
    assert!(
        matches!(listed, FilesActionResponse::Listed(entries) if entries == vec![folder.clone()])
    );

    let search = FilesActionApi::execute(
        &mut service,
        FilesActionCall::Search {
            start: Location::root(),
            query: "reports".to_owned(),
            limit: 10,
        },
        Actor::User,
    )
    .unwrap();
    assert!(
        matches!(search, FilesActionResponse::SearchResults(records) if records.len() == 1 && records[0].resource_id == folder.id)
    );
}

#[test]
fn typed_action_api_keeps_agent_mutations_fail_closed_without_activity() {
    let mut service = memory_service();
    let error = FilesActionApi::execute(
        &mut service,
        FilesActionCall::CreateFolder {
            parent: Location::root(),
            name: FileName::parse("agent-folder").unwrap(),
        },
        Actor::Agent,
    )
    .unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::ActivityUnavailable);
    assert!(service.list(&Location::root()).unwrap().is_empty());
}

#[test]
#[cfg(unix)]
fn sandbox_symlink_at_internal_storage_boundary_fails_closed() {
    let temp = TempSandbox::new();
    let outside = TempSandbox::new();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".nagi-files")).unwrap();
        assert!(matches!(
            SandboxProvider::new(temp.path()),
            Err(error) if error.kind == FilesErrorKind::SymlinkNotAllowed
        ));
    }
}

#[test]
#[cfg(unix)]
fn sandbox_trash_handle_fails_closed_if_internal_store_is_moved_outside_root() {
    let temp = TempSandbox::new();
    let outside = TempSandbox::new();
    fs::write(temp.path().join("discard.txt"), b"discard me").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let trashed = provider
        .trash(&Location::parse("discard.txt").unwrap())
        .unwrap();
    let moved_store = outside.path().join("moved-store");
    fs::rename(temp.path().join(".nagi-files"), &moved_store).unwrap();
    fs::create_dir(temp.path().join(".nagi-files")).unwrap();
    fs::create_dir(temp.path().join(".nagi-files/trash")).unwrap();

    assert_eq!(
        provider.permanently_delete(trashed.id).unwrap_err().kind,
        FilesErrorKind::SandboxEscape
    );
    assert!(moved_store
        .join("trash")
        .read_dir()
        .unwrap()
        .next()
        .is_some());
}

#[test]
fn sandbox_rejects_corrupt_trash_manifest_paths() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join(".nagi-files")).unwrap();
    fs::create_dir(temp.path().join(".nagi-files/trash")).unwrap();
    fs::write(
        temp.path().join(".nagi-files/trash-index-v1"),
        "00000000000000000000000000000001\t0\tf\t-\t7365637265742e747874\t2e2e\n",
    )
    .unwrap();
    assert!(matches!(
        SandboxProvider::new(temp.path()),
        Err(error) if error.kind == FilesErrorKind::CorruptMetadata
    ));
}

#[test]
fn sandbox_copy_accepts_japanese_filename_without_normalizing_text() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("日本語.txt"), "東京").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let entry = provider.list(&Location::root()).unwrap().remove(0);
    assert_eq!(entry.name.as_str(), "日本語.txt");
    let copy = provider
        .copy(
            &entry.child_location(),
            &Location::root(),
            &FileName::parse("複製.txt").unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(copy.name.as_str(), "複製.txt");
    assert_eq!(
        fs::read_to_string(temp.path().join("複製.txt")).unwrap(),
        "東京"
    );
}

#[test]
fn sandbox_metadata_contains_path_identity_kind_and_availability() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("metadata.txt"), "12345").unwrap();
    let provider = SandboxProvider::new(temp.path()).unwrap();
    let entry = provider
        .metadata(&Location::parse("metadata.txt").unwrap())
        .unwrap();
    assert_eq!(entry.location, Location::root());
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(entry.size_bytes, Some(5));
    assert!(entry.modified_at.is_some());
    assert_eq!(entry.availability, crate::EntryAvailability::Available);
}

#[test]
fn open_preview_has_a_bounded_read_size() {
    let mut provider = InMemoryProvider::new();
    let source = provider
        .insert_file(
            &Location::parse("large.txt").unwrap(),
            &vec![b'x'; crate::MAX_PREVIEW_BYTES + 1],
        )
        .unwrap();
    let mut service = FilesService::new(provider, full_capabilities());
    assert_eq!(
        service.open_file(&source, Actor::User).unwrap_err().kind,
        FilesErrorKind::FileTooLarge
    );
    assert_eq!(
        service
            .read_file(&source.child_location())
            .unwrap_err()
            .kind,
        FilesErrorKind::FileTooLarge
    );
}

#[test]
fn sandbox_preview_reader_enforces_the_requested_byte_limit() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("bounded.txt"), vec![b'x'; 64]).unwrap();
    let provider = SandboxProvider::new(temp.path()).unwrap();
    let error = provider
        .read_file(&Location::parse("bounded.txt").unwrap(), 16)
        .unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::FileTooLarge);
}
