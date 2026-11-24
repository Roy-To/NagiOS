use std::fs::{self, File};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::localization::{self, Locale};
use crate::{
    render_three_pane, ActivityEvent, ActivityOutcome, ActivitySink, Actor, CancellationToken,
    CapabilityAuthorizer, CapabilityGrant, CapabilityRight, CapabilitySet, CheckpointHook,
    ContextPublisher, EntryKind, FileName, FilesActionApi, FilesActionCall, FilesActionResponse,
    FilesApp, FilesCanvas, FilesCanvasRect, FilesComponent, FilesContextSnapshot, FilesErrorKind,
    FilesSearchProvider, FilesService, FilesystemProvider, HookStatus, InMemoryProvider, Location,
    OperationKind, OperationOutcome, PermissionDecision, Reversibility, SandboxProvider, Tag,
    WaybackCheckpointRequest, WorkspaceReferenceSink,
};
use nagi_ui::{ColorRole, TypeRole};

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

#[derive(Default)]
struct FilesComponentCanvas {
    texts: Vec<String>,
    text_bounds: Vec<FilesCanvasRect>,
    selected_rows: usize,
    selected_row_bounds: Vec<FilesCanvasRect>,
    focused_row_bounds: Vec<FilesCanvasRect>,
}

impl FilesCanvas for FilesComponentCanvas {
    fn fill_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole) {
        if color == ColorRole::Selection {
            self.selected_rows += 1;
            self.selected_row_bounds.push(bounds);
        }
    }

    fn frame_rect(&mut self, bounds: FilesCanvasRect, color: ColorRole) {
        if color == ColorRole::Focus {
            self.focused_row_bounds.push(bounds);
        }
    }

    fn text(
        &mut self,
        bounds: FilesCanvasRect,
        value: &str,
        _typography: TypeRole,
        _color: ColorRole,
    ) {
        self.text_bounds.push(bounds);
        self.texts.push(value.to_owned());
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

fn assert_opened_resource(operation: &crate::OperationResult, resource_id: crate::ResourceId) {
    assert_eq!(operation.action_id, "files.open");
    assert_eq!(operation.affected, vec![resource_id]);
}

fn assert_file_bytes(path: impl AsRef<std::path::Path>, expected: &[u8]) {
    let path = path.as_ref();
    let actual = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert_eq!(actual, expected, "persisted bytes at {}", path.display());
}

fn dispatch_preview_input<P, A>(
    line: &str,
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
) -> Result<(), crate::FilesError>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    crate::preview_cli::dispatch_input_line(line, service, app, |_| {}, |_| {})
        .unwrap_or_else(|| panic!("preview input was not handled: {line}"))
}

fn dispatch_preview_input_recording<P, A>(
    line: &str,
    service: &mut FilesService<P, A>,
    app: &mut FilesApp,
    operations: &mut Vec<crate::OperationResult>,
    displayed_contents: &mut Vec<String>,
) -> Result<(), crate::FilesError>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    crate::preview_cli::dispatch_input_line(
        line,
        service,
        app,
        |operation| operations.push(operation.clone()),
        |contents| displayed_contents.push(contents.to_owned()),
    )
    .unwrap_or_else(|| panic!("preview input was not handled: {line}"))
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
fn files_component_lists_opens_and_draws_a_sandbox_provider_selection() {
    const CONTENTS: &[u8] = b"component preview\n";
    let temp = TempSandbox::new();
    fs::write(temp.path().join("component-preview.txt"), CONTENTS).unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());

    component.refresh().unwrap();
    let entry = component.state().entries.first().unwrap();
    let id = entry.id;
    component.select(id, false).unwrap();
    let (contents, operation) = component.open_selected(Actor::User).unwrap().unwrap();

    assert_eq!(contents, CONTENTS);
    assert_eq!(operation.action_id, "files.open");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert!(component.state().selected.contains(&id));
    assert_eq!(component.state().focused, Some(id));
    assert_eq!(
        component.state().layout().inspector.as_ref().map(|entry| entry.id),
        Some(id)
    );

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 1);
    assert!(canvas
        .texts
        .iter()
        .any(|text| text.contains("component-preview.txt")));
    assert!(canvas.texts.iter().any(|text| text == "Metadata"));
}

#[test]
fn files_component_selects_explicit_resource_id_after_refresh_before_open() {
    const CONTENTS: &[u8] = b"open the explicitly selected resource after refresh";
    let temp = TempSandbox::new();
    fs::write(temp.path().join("x-neighbor.txt"), b"neighbor").unwrap();
    fs::write(temp.path().join("z-target.txt"), CONTENTS).unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let target = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "z-target.txt")
        .unwrap()
        .clone();
    let old_row = component
        .state()
        .entries
        .iter()
        .position(|entry| entry.id == target.id)
        .unwrap();

    fs::write(temp.path().join("a-arrival.txt"), b"arrived before the old rows").unwrap();
    component.refresh().unwrap();
    let new_row = component
        .state()
        .entries
        .iter()
        .position(|entry| entry.id == target.id)
        .unwrap();
    assert_ne!(old_row, new_row, "refresh changes the target's displayed row");
    assert_ne!(
        component.state().entries[old_row].id,
        target.id,
        "the stale row index now refers to a different ResourceId"
    );

    component.select(target.id, false).unwrap();
    assert_eq!(component.state().focused, Some(target.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(target.id)
    );
    let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened, CONTENTS);
    assert_opened_resource(&operation, target.id);

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 1);
    assert!(canvas.texts.iter().any(|text| text == "F: z-target.txt"));
    assert!(canvas.texts.iter().any(|text| text == "z-target.txt"));
}

#[test]
fn files_component_displays_localized_open_failure_in_existing_status_area() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("private.txt"), b"must remain unread").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([CapabilityGrant::allow(
            Location::root(),
            CapabilityRight::Enumerate,
        )]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let private = component.state().entries[0].clone();
    component.select(private.id, false).unwrap();

    assert_eq!(
        component.open_selected(Actor::User).unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
    assert_eq!(component.state().focused, Some(private.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(private.id),
        "the denied open keeps its target visible in the Inspector"
    );

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 120),
        "This folder is empty.",
        Locale::JaJp,
    );
    assert!(
        canvas
            .texts
            .iter()
            .any(|text| text == "この場所にアクセスする権限がありません。"),
        "the existing Files status area must explain why Open did not complete"
    );
}

#[test]
fn files_component_highlights_selected_resource_id_for_equal_width_japanese_rows() {
    const SELECTED_NAME: &str = "記録.txt";
    const SIBLING_NAME: &str = "資料.txt";

    // Both names use two full-width Japanese characters and the same ASCII suffix.
    let temp = TempSandbox::new();
    fs::write(temp.path().join(SELECTED_NAME), b"selected contents").unwrap();
    fs::write(temp.path().join(SIBLING_NAME), b"sibling contents").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();

    let selected = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == SELECTED_NAME)
        .unwrap()
        .clone();
    let sibling = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == SIBLING_NAME)
        .unwrap()
        .clone();
    assert_ne!(selected.id, sibling.id);
    component.select(selected.id, false).unwrap();

    let layout = component.state().layout();
    assert_eq!(
        layout.selected.iter().copied().collect::<Vec<_>>(),
        vec![selected.id]
    );
    assert_eq!(component.state().focused, Some(selected.id));
    assert_eq!(
        layout.inspector.as_ref().map(|entry| entry.id),
        Some(selected.id)
    );
    assert!(layout
        .resources
        .iter()
        .any(|entry| entry.id == selected.id && entry.name.as_str() == SELECTED_NAME));

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 1);
    assert_eq!(canvas.selected_row_bounds.len(), 1);
    assert_eq!(canvas.focused_row_bounds.len(), 1);
    let selected_row = canvas.selected_row_bounds[0];
    assert_eq!(canvas.focused_row_bounds[0], selected_row);

    let rendered_selected_row = canvas
        .texts
        .iter()
        .zip(&canvas.text_bounds)
        .find(|(text, bounds)| bounds.y == selected_row.y && text.contains(SELECTED_NAME))
        .map(|(_, bounds)| *bounds)
        .expect("the selected resource name is rendered on its highlighted row");
    let rendered_sibling_row = canvas
        .texts
        .iter()
        .zip(&canvas.text_bounds)
        .find(|(text, bounds)| bounds.y != selected_row.y && text.contains(SIBLING_NAME))
        .map(|(_, bounds)| *bounds)
        .expect("the equal-width sibling is rendered on a separate row");
    assert_eq!(rendered_selected_row.y, selected_row.y);
    assert_ne!(rendered_sibling_row.y, selected_row.y);

    let mut state = component.state().clone();
    state.toggle_inspector();
    assert!(!state.inspector_visible);
    assert_eq!(
        state.selected.iter().copied().collect::<Vec<_>>(),
        vec![selected.id]
    );
    assert_eq!(state.focused, Some(selected.id));
    assert!(state.layout().inspector.is_none());

    state.toggle_inspector();
    assert!(state.inspector_visible);
    assert_eq!(
        state.selected.iter().copied().collect::<Vec<_>>(),
        vec![selected.id]
    );
    assert_eq!(state.focused, Some(selected.id));
    assert_eq!(
        state.layout().inspector.as_ref().map(|entry| entry.id),
        Some(selected.id)
    );
    assert_ne!(state.focused, Some(sibling.id));
}

#[test]
fn files_component_opens_selected_long_japanese_name_by_resource_id() {
    const LONG_NAME: &str =
        "これは表示幅を超える長い日本語ファイル名のテストデータですこれでも同じIDを開ける.txt";
    const CONTENTS: &[u8] = b"the clipped Japanese row still opens the selected ResourceId";
    let temp = TempSandbox::new();
    fs::write(temp.path().join("a-sibling.txt"), b"sibling").unwrap();
    fs::write(temp.path().join(LONG_NAME), CONTENTS).unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let selected = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == LONG_NAME)
        .unwrap()
        .clone();
    component.select(selected.id, false).unwrap();

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 120, 180),
        "This folder is empty.",
        Locale::JaJp,
    );
    let row_label = format!("F: {LONG_NAME}");
    let row_index = canvas
        .texts
        .iter()
        .position(|text| text == &row_label)
        .expect("the canvas receives the original name for its clipped row bounds");
    assert!(canvas.text_bounds[row_index].width < LONG_NAME.chars().count() as i32);
    assert_eq!(canvas.selected_rows, 1);
    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, selected.id);
    assert_eq!(inspector.name.as_str(), LONG_NAME);

    let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened, CONTENTS);
    assert_opened_resource(&operation, selected.id);
}

#[test]
fn files_component_displays_extensionless_file_and_folder_by_entry_kind() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("README"), b"an extensionless regular file").unwrap();
    fs::create_dir(temp.path().join("Archive")).unwrap();

    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([CapabilityGrant::allow(
            Location::root(),
            CapabilityRight::Enumerate,
        )]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();

    let file = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "README")
        .unwrap();
    let folder = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "Archive")
        .unwrap();
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(folder.kind, EntryKind::Folder);
    assert_ne!(file.id, folder.id);

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text == "F: README"));
    assert!(canvas.texts.iter().any(|text| text == "D: Archive"));
}

#[test]
fn files_component_orders_same_stem_extensions_in_listing() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("report.txt"), b"text document").unwrap();
    fs::write(temp.path().join("report.md"), b"markdown document").unwrap();

    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([CapabilityGrant::allow(
            Location::root(),
            CapabilityRight::Enumerate,
        )]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();

    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["report.md", "report.txt"]
    );
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    let file_rows = canvas
        .texts
        .iter()
        .filter(|text| text.starts_with("F: "))
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(file_rows, vec!["F: report.md", "F: report.txt"]);
}

#[test]
fn files_component_keeps_nearly_named_file_and_folder_rows_distinct() {
    const FILE_CONTENTS: &[u8] = b"the file row opens this document";

    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("資料")).unwrap();
    fs::write(temp.path().join("資料.txt"), FILE_CONTENTS).unwrap();
    let folder_location = Location::parse("資料").unwrap();
    let file_location = Location::parse("資料.txt").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(folder_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location.clone(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();

    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["資料", "資料.txt"]
    );
    let folder = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "資料")
        .unwrap()
        .clone();
    let file = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "資料.txt")
        .unwrap()
        .clone();
    assert_eq!(folder.kind, EntryKind::Folder);
    assert_eq!(file.kind, EntryKind::File);
    assert_ne!(folder.id, file.id);

    component.select(file.id, false).unwrap();
    let file_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(file_inspector.id, file.id);
    assert_eq!(file_inspector.kind, EntryKind::File);
    assert_eq!(file_inspector.size_bytes, Some(FILE_CONTENTS.len() as u64));
    assert_eq!(
        component
            .state()
            .selected
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![file.id]
    );
    assert_eq!(component.state().focused, Some(file.id));
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    let selected_row = canvas.selected_row_bounds[0];
    assert!(canvas
        .texts
        .iter()
        .zip(&canvas.text_bounds)
        .any(|(text, bounds)| text.contains("F: 資料.txt") && bounds.y == selected_row.y));
    let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened, FILE_CONTENTS);
    assert_opened_resource(&operation, file.id);

    component.select(folder.id, false).unwrap();
    let folder_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(folder_inspector.id, folder.id);
    assert_eq!(folder_inspector.kind, EntryKind::Folder);
    assert_eq!(folder_inspector.size_bytes, None);
    assert_eq!(
        component
            .state()
            .selected
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![folder.id]
    );
    assert_eq!(component.state().focused, Some(folder.id));
    assert_eq!(
        component.open_selected(Actor::User).unwrap_err().kind,
        FilesErrorKind::UnsupportedEntry
    );
    let mut folder_canvas = FilesComponentCanvas::default();
    component.draw(
        &mut folder_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    let selected_folder_row = folder_canvas.selected_row_bounds[0];
    assert!(folder_canvas
        .texts
        .iter()
        .zip(&folder_canvas.text_bounds)
        .any(|(text, bounds)| text.contains("D: 資料") && bounds.y == selected_folder_row.y));
    component.navigate(folder.child_location()).unwrap();
    assert_eq!(component.state().location, folder_location);
    assert!(component.state().entries.is_empty());
    assert!(component.state().layout().inspector.is_none());
    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);

    let mut empty_folder_canvas = FilesComponentCanvas::default();
    component.draw(
        &mut empty_folder_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(empty_folder_canvas.selected_rows, 0);
    assert!(empty_folder_canvas
        .texts
        .iter()
        .any(|text| text == "This folder is empty."));

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let returned_file = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "資料.txt")
        .unwrap()
        .clone();
    component.select(returned_file.id, false).unwrap();
    let returned_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(returned_inspector.id, returned_file.id);
    assert_eq!(returned_inspector.kind, EntryKind::File);
    assert_eq!(
        returned_inspector.size_bytes,
        Some(FILE_CONTENTS.len() as u64)
    );
    assert_eq!(component.state().focused, Some(returned_file.id));
}

#[test]
fn files_component_inspector_text_stays_inside_a_short_canvas() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("small-view.txt"), b"short view").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let selected = component.state().entries[0].id;
    component.select(selected, false).unwrap();

    let bounds = FilesCanvasRect::new(0, 0, 320, 120);
    let mut canvas = FilesComponentCanvas::default();
    component.draw(&mut canvas, bounds, "empty", Locale::EnUs);

    assert!(
        canvas.text_bounds.iter().all(|text| {
            text.x >= bounds.x
                && text.y >= bounds.y
                && text.x + text.width <= bounds.x + bounds.width
                && text.y + text.height <= bounds.y + bounds.height
        }),
        "text bounds must stay inside the Files canvas: {:?}",
        canvas.text_bounds
    );
}

#[test]
fn files_component_empty_message_stays_inside_a_very_short_canvas() {
    let temp = TempSandbox::new();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([CapabilityGrant::allow(
            Location::root(),
            CapabilityRight::Enumerate,
        )]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    assert!(component.state().entries.is_empty());

    let bounds = FilesCanvasRect::new(0, 0, 320, 50);
    let mut canvas = FilesComponentCanvas::default();
    component.draw(&mut canvas, bounds, "empty folder", Locale::EnUs);

    assert!(
        canvas.text_bounds.iter().all(|text| {
            text.x >= bounds.x
                && text.y >= bounds.y
                && text.x + text.width <= bounds.x + bounds.width
                && text.y + text.height <= bounds.y + bounds.height
        }),
        "text bounds must stay inside the Files canvas: {:?}",
        canvas.text_bounds
    );
}

#[test]
fn files_component_refresh_shows_external_entry_and_keeps_selection_inspector() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("c-current.txt"), b"already listed").unwrap();
    fs::write(temp.path().join("a-first.txt"), b"selected most recently").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([CapabilityGrant::allow(
            Location::root(),
            CapabilityRight::Enumerate,
        )]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let first = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "a-first.txt")
        .unwrap()
        .clone();
    let current = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "c-current.txt")
        .unwrap()
        .clone();
    component.select(current.id, false).unwrap();
    component.select(first.id, true).unwrap();
    assert_eq!(component.state().focused, Some(first.id));

    fs::write(
        temp.path().join("b-arrival.txt"),
        b"appeared outside this view",
    )
    .unwrap();
    component.refresh().unwrap();

    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a-first.txt", "b-arrival.txt", "c-current.txt"]
    );
    assert!(component.state().selected.contains(&current.id));
    assert!(component.state().selected.contains(&first.id));
    assert_eq!(
        component
            .state()
            .selection_context()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![first.id, current.id],
        "multi-selection context follows the refreshed visible row order"
    );
    assert_eq!(component.state().focused, Some(first.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(first.id),
        "Inspector remains on the most recently selected resource"
    );

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 2);
    assert!(canvas.texts.iter().any(|text| text.contains("a-first.txt")));
    assert!(canvas
        .texts
        .iter()
        .any(|text| text.contains("b-arrival.txt")));
    assert!(canvas
        .texts
        .iter()
        .any(|text| text.contains("c-current.txt")));
    assert!(canvas.texts.iter().any(|text| text == "a-first.txt"));
}

#[test]
fn files_component_refresh_prunes_removed_selection_and_inspector() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("a-retained.txt"), b"still present").unwrap();
    fs::write(temp.path().join("b-removed.txt"), b"removed outside Files").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let retained = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "a-retained.txt")
        .unwrap()
        .clone();
    let removed = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "b-removed.txt")
        .unwrap()
        .clone();
    component.select(retained.id, false).unwrap();
    component.select(removed.id, true).unwrap();
    assert_eq!(component.state().focused, Some(removed.id));

    fs::remove_file(temp.path().join("b-removed.txt")).unwrap();
    component.refresh().unwrap();

    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a-retained.txt"]
    );
    assert_eq!(component.state().selected.len(), 1);
    assert!(component.state().selected.contains(&retained.id));
    assert!(!component.state().selected.contains(&removed.id));
    assert_eq!(component.state().focused, Some(retained.id));
    assert_eq!(
        component
            .state()
            .selection_context()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![retained.id]
    );
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(retained.id)
    );
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 1);
    assert!(canvas.texts.iter().any(|text| text == "a-retained.txt"));
    assert!(!canvas
        .texts
        .iter()
        .any(|text| text.contains("b-removed.txt")));

    fs::remove_file(temp.path().join("a-retained.txt")).unwrap();
    component.refresh().unwrap();

    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert!(component.state().entries.is_empty());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 0);
    assert!(!canvas
        .texts
        .iter()
        .any(|text| text.contains("a-retained.txt")));
}

#[cfg(unix)]
#[test]
fn files_component_refresh_replaced_same_name_prunes_old_selection_before_open() {
    const FILE_NAME: &str = "report.txt";
    const OLD_CONTENTS: &[u8] = b"the selected file before replacement\n";
    const NEW_CONTENTS: &[u8] = b"the replacement file with the same name\n";

    let temp = TempSandbox::new();
    let visible_path = temp.path().join(FILE_NAME);
    let staged_path = temp.path().join("replacement-staged.txt");
    fs::write(&visible_path, OLD_CONTENTS).unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let original = component.state().entries[0].clone();
    assert_eq!(original.name.as_str(), FILE_NAME);
    component.select(original.id, false).unwrap();
    let (old_contents, old_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(old_operation.affected, vec![original.id]);
    assert_eq!(old_contents, OLD_CONTENTS);

    fs::write(&staged_path, NEW_CONTENTS).unwrap();
    fs::rename(&staged_path, &visible_path).unwrap();
    component.refresh().unwrap();

    assert_eq!(component.state().entries.len(), 1);
    let replacement = component.state().entries[0].clone();
    assert_eq!(replacement.name.as_str(), FILE_NAME);
    assert_ne!(replacement.id, original.id);
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    assert_eq!(
        component.select(original.id, false).unwrap_err().kind,
        FilesErrorKind::NotFound,
        "a stale same-name ResourceId cannot select the replacement row"
    );
    assert!(component.open_selected(Actor::User).unwrap().is_none());

    component.select(replacement.id, false).unwrap();
    assert_eq!(component.state().focused, Some(replacement.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(replacement.id)
    );
    let (new_contents, new_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_opened_resource(&new_operation, replacement.id);
    assert_eq!(new_contents, NEW_CONTENTS);
}

#[test]
fn files_component_clear_selection_disables_open_and_save() {
    const ORIGINAL: &[u8] = b"selection cleared before any write\n";
    const UNSELECTED_SAVE: &[u8] = b"must not replace the unselected file\n";

    let temp = TempSandbox::new();
    let file_path = temp.path().join("note.txt");
    fs::write(&file_path, ORIGINAL).unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let note = component.state().entries[0].clone();
    component.select(note.id, false).unwrap();
    assert_eq!(component.state().focused, Some(note.id));

    component.clear_selection();

    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 0);
    assert!(canvas.focused_row_bounds.is_empty());
    assert!(component.open_selected(Actor::User).unwrap().is_none());
    assert_eq!(
        component
            .save_selected(UNSELECTED_SAVE, Actor::User)
            .unwrap_err()
            .kind,
        FilesErrorKind::NotFound
    );
    assert_file_bytes(file_path, ORIGINAL);
}

#[cfg(unix)]
#[test]
fn authorized_root_fd_lists_saves_and_reopens_existing_file() {
    const ORIGINAL: &[u8] = b"before authorized-root save\n";
    const SAVED: &[u8] = b"after authorized-root save\n";
    let temp = TempSandbox::new();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let file_location = Location::parse("existing.txt").unwrap();
    fs::write(temp.path().join(file_location.as_str()), ORIGINAL).unwrap();

    let provider = SandboxProvider::from_authorized_root(File::open(temp.path()).unwrap()).unwrap();
    assert_eq!(provider.sandbox_root(), None);
    let service = FilesService::new(
        provider,
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Write),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    assert_eq!(component.state().entries.len(), 1);
    let original = component.state().entries[0].clone();
    assert_eq!(original.name.as_str(), "existing.txt");
    component.select(original.id, false).unwrap();

    let (saved, operation) = component.save_selected(SAVED, Actor::User).unwrap();
    assert_eq!(operation.action_id, "files.write");
    assert_eq!(saved.id, original.id);
    drop(component);

    // Reopen the same fixture profile by receiving a fresh directory FD. This
    // is one-host close/reopen coverage, not reboot durability or stable
    // logical ResourceId acceptance.
    let provider = SandboxProvider::from_authorized_root(File::open(temp.path()).unwrap()).unwrap();
    assert_eq!(provider.sandbox_root(), None);
    let service = FilesService::new(
        provider,
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut reopened = FilesComponent::new(service, Location::root());
    reopened.refresh().unwrap();
    assert!(reopened.state().selected.is_empty());
    assert_eq!(reopened.state().focused, None);
    assert!(reopened.state().layout().inspector.is_none());
    let entry = reopened.state().entries.first().unwrap().clone();
    assert_eq!(entry.name.as_str(), "existing.txt");
    assert_eq!(
        entry.id, original.id,
        "the same saved item keeps its host-local resource identity after reopen"
    );
    assert_eq!(entry.size_bytes, Some(SAVED.len() as u64));
    reopened.select(entry.id, false).unwrap();
    assert_eq!(reopened.state().selected.len(), 1);
    assert!(reopened.state().selected.contains(&original.id));
    assert_eq!(reopened.state().focused, Some(original.id));
    let inspector = reopened.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, original.id);
    assert_eq!(inspector.name.as_str(), "existing.txt");
    assert_eq!(inspector.size_bytes, Some(SAVED.len() as u64));
    let (contents, operation) = reopened.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(operation.action_id, "files.open");
    assert_eq!(contents, SAVED);
}

#[cfg(unix)]
#[test]
fn authorized_root_fd_starts_at_the_callers_initial_location() {
    const WORKSPACE_CONTENTS: &[u8] = b"selected development workspace file\n";

    let temp = TempSandbox::new();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(temp.path().join("workspace")).unwrap();
    fs::write(temp.path().join("root-only.txt"), b"root listing only\n").unwrap();
    fs::write(temp.path().join("workspace/note.txt"), WORKSPACE_CONTENTS).unwrap();

    let initial_location = Location::parse("workspace").unwrap();
    let workspace_file = Location::parse("workspace/note.txt").unwrap();
    let authorizer = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(initial_location.clone(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(workspace_file, CapabilityRight::Read),
    ]);
    let mut component = FilesComponent::from_authorized_root(
        File::open(temp.path()).unwrap(),
        authorizer,
        initial_location.clone(),
    )
    .unwrap();

    assert_eq!(component.state().location, initial_location);
    component.refresh().unwrap();
    assert_eq!(component.state().entries.len(), 1);
    let note = component.state().entries[0].clone();
    assert_eq!(note.name.as_str(), "note.txt");

    let mut initial_canvas = FilesComponentCanvas::default();
    component.draw(
        &mut initial_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(initial_canvas
        .texts
        .iter()
        .any(|text| text == "Home - workspace"));
    assert!(initial_canvas
        .texts
        .iter()
        .any(|text| text == "F: note.txt"));
    assert!(!initial_canvas
        .texts
        .iter()
        .any(|text| text.contains("root-only.txt")));

    component.select(note.id, false).unwrap();
    let mut selected_canvas = FilesComponentCanvas::default();
    component.draw(
        &mut selected_canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(selected_canvas.selected_rows, 1);
    assert!(selected_canvas
        .texts
        .iter()
        .any(|text| text == "F: note.txt"));
    let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_opened_resource(&operation, note.id);
    assert_eq!(opened, WORKSPACE_CONTENTS);

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    assert!(component
        .state()
        .entries
        .iter()
        .any(|entry| entry.name.as_str() == "root-only.txt"));
}

#[cfg(unix)]
#[test]
fn authorized_root_fd_does_not_fall_back_when_initial_location_is_denied() {
    let temp = TempSandbox::new();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(temp.path().join("workspace")).unwrap();
    fs::write(temp.path().join("root-only.txt"), b"must not be listed\n").unwrap();

    let initial_location = Location::parse("workspace").unwrap();
    let authorizer = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::deny(initial_location.clone(), CapabilityRight::Enumerate),
    ]);
    let mut component = FilesComponent::from_authorized_root(
        File::open(temp.path()).unwrap(),
        authorizer,
        initial_location.clone(),
    )
    .unwrap();

    assert_eq!(component.state().location, initial_location);
    assert_eq!(
        component.refresh().unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
    assert_eq!(component.state().location, initial_location);
    assert!(component.state().entries.is_empty());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
}

#[cfg(unix)]
#[test]
fn authorized_root_fd_rejects_lexical_escape_and_symlink_traversal() {
    let root = TempSandbox::new();
    let outside = TempSandbox::new();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.path().join("inside")).unwrap();
    fs::write(root.path().join("inside/secret.txt"), b"inside").unwrap();
    std::os::unix::fs::symlink(root.path().join("inside"), root.path().join("inside-link"))
        .unwrap();
    fs::write(outside.path().join("secret.txt"), b"outside").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        root.path().join("escape.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape-dir")).unwrap();

    let provider = SandboxProvider::from_authorized_root(File::open(root.path()).unwrap()).unwrap();
    let internal_symlink = provider
        .list(&Location::parse("inside-link").unwrap())
        .unwrap_err();
    assert_eq!(internal_symlink.kind, FilesErrorKind::SymlinkNotAllowed);
    let lexical_escape = Location::parse("../secret.txt").unwrap_err();
    assert_eq!(lexical_escape.kind, FilesErrorKind::SandboxEscape);
    let symlink_file = provider
        .read_file(&Location::parse("escape.txt").unwrap(), 64)
        .unwrap_err();
    assert_eq!(symlink_file.kind, FilesErrorKind::SymlinkNotAllowed);
    let symlink_directory = provider
        .read_file(&Location::parse("escape-dir/secret.txt").unwrap(), 64)
        .unwrap_err();
    assert_eq!(symlink_directory.kind, FilesErrorKind::SymlinkNotAllowed);
    assert_file_bytes(outside.path().join("secret.txt"), b"outside");

    let non_directory = File::open(outside.path().join("secret.txt")).unwrap();
    match SandboxProvider::from_authorized_root(non_directory) {
        Err(error) => assert_eq!(error.kind, FilesErrorKind::InvalidLocation),
        Ok(_) => panic!("a regular file FD is not an authorized Files root"),
    }

    let shared_root = TempSandbox::new();
    fs::set_permissions(shared_root.path(), fs::Permissions::from_mode(0o750)).unwrap();
    match SandboxProvider::from_authorized_root(File::open(shared_root.path()).unwrap()) {
        Err(error) => assert_eq!(error.kind, FilesErrorKind::PermissionDenied),
        Ok(_) => panic!("a group-accessible directory is not a private Files root"),
    }
}

#[cfg(unix)]
#[test]
fn authorized_root_fd_is_marked_close_on_exec() {
    let temp = TempSandbox::new();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = File::open(temp.path()).unwrap();
    let flags = rustix::io::fcntl_getfd(&root).unwrap();
    rustix::io::fcntl_setfd(&root, flags & !rustix::io::FdFlags::CLOEXEC).unwrap();
    assert!(
        !rustix::io::fcntl_getfd(&root)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );

    crate::sandbox::ensure_close_on_exec(&root).unwrap();

    assert!(
        rustix::io::fcntl_getfd(&root)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
}

#[test]
fn files_component_renames_selected_resource_through_service() {
    const CONTENTS: &[u8] = b"rename component fixture";
    const SIBLING_CONTENTS: &[u8] = b"the middle row remains unchanged";
    let temp = TempSandbox::new();
    fs::write(temp.path().join("before.txt"), CONTENTS).unwrap();
    fs::write(temp.path().join("middle.txt"), SIBLING_CONTENTS).unwrap();
    let original_location = Location::parse("before.txt").unwrap();
    let expected_tags = vec!["rename-check".to_owned(), "日本語".to_owned()];
    let mut seed_provider = SandboxProvider::new(temp.path()).unwrap();
    let tagged_source = seed_provider
        .set_tags(&original_location, &expected_tags)
        .unwrap();
    drop(seed_provider);
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("before.txt").unwrap(),
                CapabilityRight::Rename,
            ),
            CapabilityGrant::allow(
                Location::parse("z_after.txt").unwrap(),
                CapabilityRight::Read,
            ),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    assert_eq!(component.state().entries[0].name.as_str(), "before.txt");
    assert_eq!(component.state().entries[1].name.as_str(), "middle.txt");
    let source_id = component.state().entries[0].id;
    assert_eq!(source_id, tagged_source.id);
    assert_eq!(component.state().entries[0].tags, expected_tags);
    component.select(source_id, false).unwrap();

    let destination = FileName::parse("z_after.txt").unwrap();
    let (renamed, operation) = component
        .rename_selected(&destination, Actor::User)
        .unwrap();

    assert_eq!(renamed.id, source_id);
    assert_eq!(renamed.name.as_str(), "z_after.txt");
    assert_eq!(renamed.tags, expected_tags);
    assert_eq!(operation.action_id, "files.rename");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(component.state().entries.len(), 2);
    assert_eq!(component.state().entries[0].name.as_str(), "middle.txt");
    assert_eq!(component.state().entries[1].name.as_str(), "z_after.txt");
    assert!(component.state().selected.contains(&renamed.id));
    assert_eq!(component.state().focused, Some(renamed.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(source_id)
    );
    let reordered_inspector = component
        .state()
        .layout()
        .inspector
        .expect("Inspector follows the renamed resource after its row moves");
    assert_eq!(reordered_inspector.id, source_id);
    assert_eq!(reordered_inspector.name.as_str(), "z_after.txt");
    assert_file_bytes(temp.path().join("z_after.txt"), CONTENTS);
    assert!(!temp.path().join("before.txt").exists());
    assert_file_bytes(temp.path().join("middle.txt"), SIBLING_CONTENTS);
    let (reopened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(reopened, CONTENTS);
    assert_opened_resource(&operation, source_id);

    drop(component);
    let fresh_service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("z_after.txt").unwrap(),
                CapabilityRight::Read,
            ),
        ]),
    );
    let mut fresh_component = FilesComponent::new(fresh_service, Location::root());
    fresh_component.refresh().unwrap();
    let fresh_entry = fresh_component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "z_after.txt")
        .unwrap()
        .clone();
    assert_eq!(fresh_entry.id, source_id);
    assert_eq!(fresh_entry.tags, expected_tags);
    assert_eq!(fresh_entry.size_bytes, Some(CONTENTS.len() as u64));
    assert!(!fresh_component
        .state()
        .entries
        .iter()
        .any(|entry| entry.name.as_str() == "before.txt"));
    fresh_component.select(fresh_entry.id, false).unwrap();
    let fresh_inspector = fresh_component.state().layout().inspector.unwrap();
    assert_eq!(fresh_inspector.id, source_id);
    assert_eq!(fresh_inspector.name.as_str(), "z_after.txt");
    assert_eq!(fresh_inspector.tags, expected_tags);
    let (fresh_contents, fresh_operation) = fresh_component
        .open_selected(Actor::User)
        .unwrap()
        .unwrap();
    assert_eq!(fresh_contents, CONTENTS);
    assert_opened_resource(&fresh_operation, source_id);
}

#[test]
fn files_component_saves_selected_resource_through_service() {
    const ORIGINAL: &[u8] = b"before save";
    const SAVED: &[u8] = b"after save, then navigate back";
    const CHILD_CONTENTS: &[u8] = b"child entry";
    const CHILD_SAVED: &[u8] = b"child saved in the active folder\n";
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("browse")).unwrap();
    fs::write(temp.path().join("browse/inside.txt"), CHILD_CONTENTS).unwrap();
    fs::write(temp.path().join("draft.txt"), ORIGINAL).unwrap();
    let file_location = Location::parse("draft.txt").unwrap();
    let browse_location = Location::parse("browse").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(browse_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("browse/inside.txt").unwrap(),
                CapabilityRight::Write,
            ),
            CapabilityGrant::allow(
                Location::parse("browse/inside.txt").unwrap(),
                CapabilityRight::Read,
            ),
            CapabilityGrant::allow(file_location.clone(), CapabilityRight::Write),
            CapabilityGrant::allow(file_location, CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let selected_id = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "draft.txt")
        .unwrap()
        .id;
    component.select(selected_id, false).unwrap();

    let (saved, operation) = component.save_selected(SAVED, Actor::User).unwrap();

    assert_eq!(saved.name.as_str(), "draft.txt");
    assert_eq!(operation.action_id, "files.write");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(operation.affected, vec![selected_id]);
    assert_file_bytes(temp.path().join("draft.txt"), SAVED);
    assert_eq!(saved.id, selected_id);
    assert_eq!(saved.name.as_str(), "draft.txt");
    assert_eq!(saved.size_bytes, Some(SAVED.len() as u64));
    assert!(component.state().entries.contains(&saved));
    assert!(component.state().selected.contains(&saved.id));
    assert_eq!(component.state().focused, Some(saved.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(saved.id)
    );

    component.navigate(browse_location).unwrap();
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let child_entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "inside.txt")
        .unwrap()
        .clone();
    component.select(child_entry.id, false).unwrap();
    let child_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(child_inspector.id, child_entry.id);
    assert_eq!(child_inspector.name.as_str(), "inside.txt");
    assert_eq!(
        child_inspector.size_bytes,
        Some(CHILD_CONTENTS.len() as u64)
    );
    let (saved_child, save_operation) = component.save_selected(CHILD_SAVED, Actor::User).unwrap();
    assert_eq!(save_operation.action_id, "files.write");
    assert_eq!(save_operation.outcome, OperationOutcome::Applied);
    assert_eq!(save_operation.affected, vec![child_entry.id]);
    assert_eq!(saved_child.id, child_entry.id);
    assert_eq!(saved_child.size_bytes, Some(CHILD_SAVED.len() as u64));
    assert_file_bytes(temp.path().join("browse/inside.txt"), CHILD_SAVED);
    assert!(component.state().selected.contains(&child_entry.id));
    assert_eq!(component.state().focused, Some(child_entry.id));
    let saved_child_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(saved_child_inspector.id, child_entry.id);
    assert_eq!(saved_child_inspector.name.as_str(), "inside.txt");
    assert_eq!(
        saved_child_inspector.size_bytes,
        Some(CHILD_SAVED.len() as u64)
    );
    let (child_contents, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(child_contents, CHILD_SAVED);
    assert_opened_resource(&operation, child_entry.id);

    assert!(component.navigate_parent().unwrap());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());

    let returned = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "draft.txt")
        .unwrap()
        .clone();
    assert_eq!(returned.id, saved.id);
    assert_eq!(returned.size_bytes, Some(SAVED.len() as u64));
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == saved.id)
            .count(),
        1
    );
    component.select(returned.id, false).unwrap();
    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, saved.id);
    assert_eq!(inspector.name.as_str(), "draft.txt");
    assert_eq!(inspector.size_bytes, Some(SAVED.len() as u64));

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text == "F: draft.txt"));
    assert!(canvas.texts.iter().any(|text| text == "draft.txt"));

    let (reopened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(reopened, SAVED);
    assert_eq!(operation.action_id, "files.open");
}

#[test]
fn files_component_trashes_selected_resource_and_restores_it_into_visible_listing() {
    const CONTENTS: &[u8] = b"trash restore component fixture";
    let temp = TempSandbox::new();
    fs::write(temp.path().join("recover.txt"), CONTENTS).unwrap();
    let file_location = Location::parse("recover.txt").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(file_location.clone(), CapabilityRight::Read),
            CapabilityGrant::allow(file_location.clone(), CapabilityRight::Delete),
            CapabilityGrant::allow(file_location, CapabilityRight::Restore),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    assert!(!temp.path().join(".nagi-files").exists());
    component.refresh().unwrap();
    assert!(!temp.path().join(".nagi-files").exists());
    let original = component.state().entries[0].clone();
    component.select(original.id, false).unwrap();

    let (trashed, trash_operation) = component.trash_selected(Actor::User).unwrap();

    assert!(temp.path().join(".nagi-files/trash").is_dir());
    assert_eq!(trashed.id, original.id);
    assert_eq!(trash_operation.action_id, "files.delete");
    assert_eq!(trash_operation.outcome, OperationOutcome::Applied);
    assert!(component.state().entries.is_empty());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert_eq!(component.state().layout().inspector, None);
    assert!(!temp.path().join("recover.txt").exists());
    assert_eq!(component.list_trash().unwrap(), vec![trashed.clone()]);

    let (restored, restore_operation) = component
        .restore_trash_item(trashed.id, Actor::User)
        .unwrap();

    assert_eq!(restored.id, original.id);
    assert_eq!(restored.name, original.name);
    assert_eq!(restored.location, original.location);
    assert_eq!(restore_operation.action_id, "files.restore");
    assert_eq!(restore_operation.outcome, OperationOutcome::Applied);
    assert_eq!(component.state().entries, vec![restored.clone()]);
    assert!(component.state().selected.contains(&restored.id));
    assert_eq!(component.state().focused, Some(restored.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(restored.id)
    );
    assert_eq!(component.list_trash().unwrap(), Vec::new());

    let (reopened, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(reopened, CONTENTS);
    assert_eq!(open_operation.action_id, "files.open");
    assert_file_bytes(temp.path().join("recover.txt"), CONTENTS);
}

#[test]
fn files_component_restore_then_parent_navigation_presents_item_once() {
    const CONTENTS: &[u8] = b"restored after navigating away";
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("browse")).unwrap();
    fs::write(temp.path().join("recover.txt"), CONTENTS).unwrap();
    fs::write(temp.path().join("browse/nested.txt"), b"nested").unwrap();

    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let original = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "recover.txt")
        .unwrap()
        .clone();
    component.select(original.id, false).unwrap();

    let (trashed, _) = component.trash_selected(Actor::User).unwrap();
    assert_eq!(trashed.id, original.id);
    component
        .navigate(Location::parse("browse").unwrap())
        .unwrap();

    let (restored, operation) = component
        .restore_trash_item(trashed.id, Actor::User)
        .unwrap();

    assert_eq!(restored.id, original.id);
    assert_eq!(restored.location, Location::root());
    assert_eq!(operation.action_id, "files.restore");
    assert_eq!(
        component.state().location,
        Location::parse("browse").unwrap()
    );
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == restored.id)
            .count(),
        0,
        "restoring outside the current view must not append a presented row"
    );

    assert!(component.navigate_parent().unwrap());

    let restored_rows = component
        .state()
        .entries
        .iter()
        .filter(|entry| entry.id == restored.id)
        .count();
    assert_eq!(restored_rows, 1);
    assert_eq!(component.state().entries.len(), 2);
    assert_eq!(component.state().selected.len(), 0);
    assert_eq!(component.state().focused, None);
    assert_eq!(component.list_trash().unwrap(), Vec::new());

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(
        canvas
            .texts
            .iter()
            .filter(|text| text.contains("recover.txt"))
            .count(),
        1,
        "the restored item must be presented once after returning to its parent"
    );
    assert_eq!(canvas.selected_rows, 0);
    assert_file_bytes(temp.path().join("recover.txt"), CONTENTS);
}

#[test]
fn files_component_parent_navigation_refreshes_external_parent_listing() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("browse")).unwrap();
    fs::write(temp.path().join("before.txt"), b"before parent navigation").unwrap();
    fs::write(
        temp.path().join("retained.txt"),
        b"unchanged while in the empty child folder",
    )
    .unwrap();

    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    assert_eq!(component.state().location, Location::root());
    assert_eq!(component.state().breadcrumbs.len(), 1);
    assert_eq!(component.state().breadcrumbs[0].label, "Home");
    assert_eq!(component.state().breadcrumbs[0].location, Location::root());
    let original_folder = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "browse")
        .unwrap()
        .clone();
    let before_entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "before.txt")
        .unwrap()
        .clone();
    assert!(component
        .state()
        .entries
        .iter()
        .any(|entry| entry.name.as_str() == "before.txt"));
    component.select(before_entry.id, false).unwrap();
    assert_eq!(component.state().focused, Some(before_entry.id));

    component
        .navigate(original_folder.child_location())
        .unwrap();
    assert_eq!(component.state().location, original_folder.child_location());
    assert_eq!(component.state().breadcrumbs.len(), 2);
    assert_eq!(component.state().breadcrumbs[0].label, "Home");
    assert_eq!(component.state().breadcrumbs[0].location, Location::root());
    assert_eq!(component.state().breadcrumbs[1].label, "browse");
    assert_eq!(
        component.state().breadcrumbs[1].location,
        original_folder.child_location()
    );
    assert!(component.state().entries.is_empty());
    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());

    fs::rename(
        temp.path().join("before.txt"),
        temp.path().join("after.txt"),
    )
    .unwrap();
    fs::write(
        temp.path().join("arrived.txt"),
        b"arrived while in child folder",
    )
    .unwrap();

    assert!(component.navigate_parent().unwrap());

    assert_eq!(component.state().location, Location::root());
    assert_eq!(component.state().breadcrumbs.len(), 1);
    assert_eq!(component.state().breadcrumbs[0].label, "Home");
    assert_eq!(component.state().breadcrumbs[0].location, Location::root());
    assert_eq!(component.state().load_state, crate::ViewLoadState::Ready);
    assert_eq!(component.state().entries.len(), 4);
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    assert!(!component
        .state()
        .entries
        .iter()
        .any(|entry| entry.name.as_str() == "before.txt"));
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.name.as_str() == "browse")
            .count(),
        1
    );
    for name in ["after.txt", "arrived.txt", "retained.txt"] {
        assert_eq!(
            component
                .state()
                .entries
                .iter()
                .filter(|entry| entry.name.as_str() == name)
                .count(),
            1,
            "parent refresh should present {name} exactly once"
        );
    }
    let retained = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "retained.txt")
        .unwrap();
    assert_eq!(
        retained.size_bytes,
        Some(b"unchanged while in the empty child folder".len() as u64)
    );
    assert_file_bytes(
        temp.path().join("retained.txt"),
        b"unchanged while in the empty child folder",
    );

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(!canvas.texts.iter().any(|text| text.contains("before.txt")));
    for name in ["browse", "after.txt", "arrived.txt", "retained.txt"] {
        assert_eq!(
            canvas
                .texts
                .iter()
                .filter(|text| text.contains(name))
                .count(),
            1,
            "the refreshed parent view should draw {name} exactly once"
        );
    }
}

#[test]
fn files_component_parent_return_clears_inspector_and_reloads_saved_tags() {
    const CONTENTS: &[u8] = b"saved metadata survives leaving and returning to the parent";
    const CHILD_CONTENTS: &[u8] = b"child focus must not leak into its parent";

    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("browse")).unwrap();
    fs::write(temp.path().join("report.txt"), CONTENTS).unwrap();
    fs::write(temp.path().join("browse/inside.txt"), CHILD_CONTENTS).unwrap();
    let file_location = Location::parse("report.txt").unwrap();
    let saved_tags = vec!["archive".to_owned(), "日本語".to_owned()];
    let mut seed_provider = SandboxProvider::new(temp.path()).unwrap();
    let tagged = seed_provider.set_tags(&file_location, &saved_tags).unwrap();
    drop(seed_provider);

    let browse_location = Location::parse("browse").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(browse_location.clone(), CapabilityRight::Enumerate),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let before_navigation = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "report.txt")
        .unwrap()
        .clone();
    assert_eq!(before_navigation.id, tagged.id);
    assert_eq!(before_navigation.tags, saved_tags);
    assert_eq!(before_navigation.size_bytes, Some(CONTENTS.len() as u64));
    component.select(before_navigation.id, false).unwrap();
    let selected_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(selected_inspector.id, tagged.id);
    assert_eq!(selected_inspector.tags, saved_tags);
    assert_eq!(selected_inspector.size_bytes, Some(CONTENTS.len() as u64));

    component.navigate(browse_location).unwrap();
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let child_entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "inside.txt")
        .unwrap()
        .clone();
    assert_ne!(child_entry.id, tagged.id);
    assert_eq!(child_entry.size_bytes, Some(CHILD_CONTENTS.len() as u64));
    component.select(child_entry.id, false).unwrap();
    let child_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(child_inspector.id, child_entry.id);
    assert_eq!(child_inspector.name.as_str(), "inside.txt");
    assert_eq!(
        child_inspector.size_bytes,
        Some(CHILD_CONTENTS.len() as u64)
    );

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    let returned_entry = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "report.txt")
        .unwrap()
        .clone();
    assert!(!component
        .state()
        .entries
        .iter()
        .any(|entry| entry.id == child_entry.id));
    assert_eq!(returned_entry.id, tagged.id);
    assert_eq!(returned_entry.tags, saved_tags);
    assert_eq!(returned_entry.size_bytes, Some(CONTENTS.len() as u64));

    component.select(returned_entry.id, false).unwrap();
    let returned_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(returned_inspector.id, tagged.id);
    assert_eq!(returned_inspector.tags, saved_tags);
    assert_eq!(returned_inspector.size_bytes, Some(CONTENTS.len() as u64));
}

#[cfg(unix)]
#[test]
fn files_component_without_delete_right_does_not_create_trash_storage() {
    let temp = TempSandbox::new();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(temp.path().join("visible.txt"), b"read only").unwrap();
    let metadata_root = temp.path().join(".nagi-files");
    let trash_root = metadata_root.join("trash");

    let mut no_grants = FilesComponent::from_authorized_root(
        File::open(temp.path()).unwrap(),
        CapabilitySet::new(),
        Location::root(),
    )
    .unwrap();
    assert!(!metadata_root.exists());
    assert_eq!(
        no_grants.refresh().unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
    assert!(!trash_root.exists());
    drop(no_grants);

    let read_only = CapabilitySet::from_grants([
        CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
    ]);
    let mut component = FilesComponent::from_authorized_root(
        File::open(temp.path()).unwrap(),
        read_only,
        Location::root(),
    )
    .unwrap();
    assert!(!metadata_root.exists());
    component.refresh().unwrap();
    assert_eq!(component.state().entries.len(), 1);
    assert!(!trash_root.exists());
    let selected = component.state().entries[0].id;
    component.select(selected, false).unwrap();
    assert_eq!(
        component.trash_selected(Actor::User).unwrap_err().kind,
        FilesErrorKind::PermissionDenied
    );
    assert!(!trash_root.exists());
    assert_file_bytes(temp.path().join("visible.txt"), b"read only");
}

#[test]
fn files_component_copies_selected_resource_to_another_folder_and_reads_it() {
    const CONTENTS: &[u8] = b"copy component fixture";
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::create_dir(temp.path().join("destination")).unwrap();
    fs::write(temp.path().join("source/original.txt"), CONTENTS).unwrap();
    let source_location = Location::parse("source").unwrap();
    let destination = Location::parse("destination").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(source_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("source/original.txt").unwrap(),
                CapabilityRight::Read,
            ),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Create),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("destination/copied.txt").unwrap(),
                CapabilityRight::Read,
            ),
        ]),
    );
    let mut component = FilesComponent::new(service, source_location.clone());
    component.refresh().unwrap();
    let source = component.state().entries[0].clone();
    component.select(source.id, false).unwrap();

    let (copied, operation) = component
        .copy_selected(
            &destination,
            &FileName::parse("copied.txt").unwrap(),
            Actor::User,
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(copied.location, destination);
    assert_eq!(copied.name.as_str(), "copied.txt");
    assert_eq!(operation.action_id, "files.copy");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(operation.affected, vec![copied.id]);
    assert_eq!(component.state().location, source_location);
    assert_eq!(component.state().entries, vec![source.clone()]);
    assert!(component.state().selected.contains(&source.id));
    assert_eq!(component.state().focused, Some(source.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(source.id)
    );

    component.navigate(destination.clone()).unwrap();
    assert_eq!(component.state().entries, vec![copied.clone()]);
    let listed_copy = &component.state().entries[0];
    assert_eq!(listed_copy.name.as_str(), "copied.txt");
    assert_eq!(listed_copy.size_bytes, Some(CONTENTS.len() as u64));
    component.select(copied.id, false).unwrap();
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text.contains("copied.txt")));
    assert!(canvas
        .texts
        .iter()
        .any(|text| text == &format!("{} B", CONTENTS.len())));
    let (contents, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(contents, CONTENTS);
    assert_eq!(open_operation.action_id, "files.open");
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(copied.id)
    );
    assert_file_bytes(temp.path().join("destination/copied.txt"), CONTENTS);

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    for folder in ["source", "destination"] {
        assert_eq!(
            component
                .state()
                .entries
                .iter()
                .filter(|entry| {
                    entry.kind == crate::EntryKind::Folder && entry.name.as_str() == folder
                })
                .count(),
            1,
            "parent listing contains {folder} once after returning from the destination"
        );
    }
    assert!(component
        .state()
        .entries
        .iter()
        .all(|entry| entry.name.as_str() != "copied.txt"));

    component.navigate(source_location.clone()).unwrap();
    assert_eq!(component.state().entries.len(), 1);
    let original_after_return = &component.state().entries[0];
    assert_eq!(original_after_return.id, source.id);
    assert_eq!(original_after_return.name.as_str(), "original.txt");
    assert_eq!(
        original_after_return.size_bytes,
        Some(CONTENTS.len() as u64)
    );
    assert_file_bytes(temp.path().join("source/original.txt"), CONTENTS);
    component.select(original_after_return.id, false).unwrap();
    let (original_contents, original_open) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(original_open.action_id, "files.open");
    assert_eq!(original_contents, CONTENTS);
}

#[test]
fn files_component_keeps_tags_and_contents_with_same_name_in_distinct_folders() {
    const FILE_NAME: &str = "report.txt";
    const LEFT_CONTENTS: &[u8] = b"left report body";
    const RIGHT_CONTENTS: &[u8] = b"right report body";
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("left")).unwrap();
    fs::create_dir(temp.path().join("right")).unwrap();
    fs::write(temp.path().join("left").join(FILE_NAME), LEFT_CONTENTS).unwrap();
    fs::write(temp.path().join("right").join(FILE_NAME), RIGHT_CONTENTS).unwrap();

    let left_location = Location::parse("left/report.txt").unwrap();
    let right_location = Location::parse("right/report.txt").unwrap();
    let left_tags = vec!["left-only".to_owned(), "日本語".to_owned()];
    let mut seed_provider = SandboxProvider::new(temp.path()).unwrap();
    let tagged_left = seed_provider
        .set_tags(&left_location, &left_tags)
        .unwrap();
    let expected_right = seed_provider.metadata(&right_location).unwrap();
    assert_ne!(tagged_left.id, expected_right.id);
    drop(seed_provider);

    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        full_capabilities(),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();

    component.navigate(Location::parse("left").unwrap()).unwrap();
    let left = component.state().entries[0].clone();
    assert_eq!(left.id, tagged_left.id);
    assert_eq!(left.name.as_str(), FILE_NAME);
    assert_eq!(left.tags, left_tags);
    component.select(left.id, false).unwrap();
    let left_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(left_inspector.id, tagged_left.id);
    assert_eq!(left_inspector.tags, left_tags);
    let (opened_left, left_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened_left, LEFT_CONTENTS);
    assert_eq!(left_operation.affected, vec![tagged_left.id]);

    component.navigate(Location::parse("right").unwrap()).unwrap();
    let right = component.state().entries[0].clone();
    assert_eq!(right.id, expected_right.id);
    assert_eq!(right.name.as_str(), FILE_NAME);
    assert_ne!(right.id, tagged_left.id);
    assert!(right.tags.is_empty());
    component.select(right.id, false).unwrap();
    let right_inspector = component.state().layout().inspector.unwrap();
    assert_eq!(right_inspector.id, right.id);
    assert!(right_inspector.tags.is_empty());
    let (opened_right, right_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(opened_right, RIGHT_CONTENTS);
    assert_eq!(right_operation.affected, vec![right.id]);
}

#[test]
fn files_component_inspector_sizes_match_opened_kib_sized_files() {
    const FIRST_SIZE: usize = 1_536;
    const SECOND_SIZE: usize = 4_097;

    let temp = TempSandbox::new();
    let first_contents = vec![b'A'; FIRST_SIZE];
    let second_contents = vec![b'B'; SECOND_SIZE];
    fs::write(temp.path().join("first.bin"), &first_contents).unwrap();
    fs::write(temp.path().join("second.bin"), &second_contents).unwrap();
    let first_location = Location::parse("first.bin").unwrap();
    let second_location = Location::parse("second.bin").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(Location::root(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let first = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.location.join(&entry.name) == first_location)
        .unwrap()
        .clone();
    let second = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.location.join(&entry.name) == second_location)
        .unwrap()
        .clone();

    for (entry, expected_contents) in [(&first, &first_contents), (&second, &second_contents)] {
        component.select(entry.id, false).unwrap();
        assert_eq!(component.state().selected.len(), 1);
        assert!(component.state().selected.contains(&entry.id));
        assert_eq!(component.state().focused, Some(entry.id));
        let inspector = component.state().layout().inspector.unwrap();
        assert_eq!(inspector.id, entry.id);
        assert_eq!(inspector.name, entry.name);
        assert_eq!(inspector.size_bytes, Some(expected_contents.len() as u64));

        let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
        assert_opened_resource(&operation, entry.id);
        assert_eq!(opened.as_slice(), expected_contents);

        let inspector_after_open = component.state().layout().inspector.unwrap();
        assert_eq!(inspector_after_open.id, entry.id);
        assert_eq!(inspector_after_open.name, entry.name);
        assert_eq!(
            inspector_after_open.size_bytes,
            Some(expected_contents.len() as u64)
        );
        let mut canvas = FilesComponentCanvas::default();
        component.draw(
            &mut canvas,
            FilesCanvasRect::new(0, 0, 640, 360),
            "This folder is empty.",
            Locale::EnUs,
        );
        assert!(canvas.texts.iter().any(|text| text == entry.name.as_str()));
        assert!(canvas
            .texts
            .iter()
            .any(|text| text == &format!("{} B", expected_contents.len())));
    }

    fs::rename(
        temp.path().join("first.bin"),
        temp.path().join("z-first.bin"),
    )
    .unwrap();
    fs::rename(
        temp.path().join("second.bin"),
        temp.path().join("a-second.bin"),
    )
    .unwrap();
    component.refresh().unwrap();
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["a-second.bin", "z-first.bin"],
        "the display order changes before the reopened reads"
    );
    let second_after_reorder = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "a-second.bin")
        .unwrap()
        .clone();
    let first_after_reorder = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "z-first.bin")
        .unwrap()
        .clone();
    #[cfg(unix)]
    {
        assert!(component
            .state()
            .selected
            .contains(&second_after_reorder.id));
        assert_eq!(component.state().focused, Some(second_after_reorder.id));
        let inspector = component.state().layout().inspector.unwrap();
        assert_eq!(inspector.id, second_after_reorder.id);
        assert_eq!(inspector.size_bytes, Some(SECOND_SIZE as u64));
    }

    for (entry, expected_contents) in [
        (&second_after_reorder, second_contents.as_slice()),
        (&first_after_reorder, first_contents.as_slice()),
    ] {
        component.select(entry.id, false).unwrap();
        assert_eq!(component.state().selected.len(), 1);
        assert!(component.state().selected.contains(&entry.id));
        assert_eq!(component.state().focused, Some(entry.id));
        let inspector = component.state().layout().inspector.unwrap();
        assert_eq!(inspector.id, entry.id);
        assert_eq!(inspector.size_bytes, Some(expected_contents.len() as u64));

        let mut canvas = FilesComponentCanvas::default();
        component.draw(
            &mut canvas,
            FilesCanvasRect::new(0, 0, 640, 360),
            "This folder is empty.",
            Locale::EnUs,
        );
        assert!(canvas
            .texts
            .iter()
            .any(|text| text == &format!("{} B", expected_contents.len())));

        let (opened, operation) = component.open_selected(Actor::User).unwrap().unwrap();
        assert_opened_resource(&operation, entry.id);
        assert_eq!(opened.as_slice(), expected_contents);
    }
    assert_eq!(component.state().focused, Some(first_after_reorder.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| (entry.id, entry.size_bytes)),
        Some((first_after_reorder.id, Some(FIRST_SIZE as u64)))
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        fn allocated_bytes(path: &Path) -> std::io::Result<u64> {
            let metadata = fs::symlink_metadata(path)?;
            let own = metadata.blocks().saturating_mul(512);
            if metadata.is_dir() {
                fs::read_dir(path)?.try_fold(own, |total, entry| {
                    allocated_bytes(&entry?.path()).map(|size| total.saturating_add(size))
                })
            } else {
                Ok(own)
            }
        }

        println!(
            "NAGI_FILES_INSPECTOR_REORDER_SCRATCH path={} allocated_bytes={}",
            temp.path().display(),
            allocated_bytes(temp.path()).expect("measure task-owned scratch before cleanup")
        );
    }
}

#[test]
fn files_component_copies_selected_resource_within_visible_folder_and_selects_copy() {
    const CONTENTS: &[u8] = b"same-folder copy keeps its source independent";

    let temp = TempSandbox::new();
    let source_path = temp.path().join("source.txt");
    let copy_path = temp.path().join("copy.txt");
    fs::write(&source_path, CONTENTS).unwrap();
    let source_location = Location::parse("source.txt").unwrap();
    let copy_location = Location::parse("copy.txt").unwrap();
    let root = Location::root();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(root.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(root.clone(), CapabilityRight::Create),
            CapabilityGrant::allow(source_location.clone(), CapabilityRight::Read),
            CapabilityGrant::allow(copy_location.clone(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, root.clone());
    component.refresh().unwrap();
    let source = component.state().entries[0].clone();
    component.select(source.id, false).unwrap();

    let (copied, operation) = component
        .copy_selected(
            &root,
            &FileName::parse("copy.txt").unwrap(),
            Actor::User,
            &CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(operation.action_id, "files.copy");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(operation.affected, vec![copied.id]);
    assert_ne!(copied.id, source.id);
    assert_eq!(copied.location, root);
    assert_eq!(copied.name.as_str(), "copy.txt");
    assert_eq!(component.state().entries.len(), 2);
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == source.id)
            .count(),
        1
    );
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == copied.id)
            .count(),
        1
    );
    assert_file_bytes(&source_path, CONTENTS);
    assert_file_bytes(&copy_path, CONTENTS);
    assert_eq!(component.state().selected.len(), 2);
    assert!(component.state().selected.contains(&source.id));
    assert!(component.state().selected.contains(&copied.id));
    assert_eq!(component.state().focused, Some(copied.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(copied.id)
    );

    component.refresh().unwrap();

    assert_eq!(component.state().location, root);
    assert_eq!(component.state().entries.len(), 2);
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == source.id)
            .count(),
        1
    );
    assert_eq!(
        component
            .state()
            .entries
            .iter()
            .filter(|entry| entry.id == copied.id && entry.name.as_str() == "copy.txt")
            .count(),
        1
    );
    assert!(component.state().selected.contains(&source.id));
    assert!(component.state().selected.contains(&copied.id));
    assert_eq!(component.state().focused, Some(copied.id));
    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, copied.id);
    assert_eq!(inspector.name.as_str(), "copy.txt");
    assert_eq!(inspector.size_bytes, Some(CONTENTS.len() as u64));

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text == "F: source.txt"));
    assert!(canvas.texts.iter().any(|text| text == "F: copy.txt"));
    assert!(canvas.texts.iter().any(|text| text == "copy.txt"));
    assert_eq!(canvas.selected_rows, 2);

    let (opened, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(open_operation.action_id, "files.open");
    assert_eq!(opened, CONTENTS);
}

#[test]
fn files_component_moves_selected_resource_within_visible_folder_and_updates_row() {
    const CONTENTS: &[u8] = b"same-folder move preserves the resource";

    let temp = TempSandbox::new();
    let source_path = temp.path().join("before.txt");
    let moved_path = temp.path().join("after.txt");
    fs::write(&source_path, CONTENTS).unwrap();
    let source_location = Location::parse("before.txt").unwrap();
    let moved_location = Location::parse("after.txt").unwrap();
    let root = Location::root();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(root.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(root.clone(), CapabilityRight::Create),
            CapabilityGrant::allow(source_location.clone(), CapabilityRight::Move),
            CapabilityGrant::allow(moved_location.clone(), CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, root.clone());
    component.refresh().unwrap();
    let source = component.state().entries[0].clone();
    component.select(source.id, false).unwrap();

    let (moved, operation) = component
        .move_selected(&root, &FileName::parse("after.txt").unwrap(), Actor::User)
        .unwrap();

    assert_eq!(operation.action_id, "files.move");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(operation.affected, vec![moved.id]);
    assert_eq!(moved.id, source.id);
    assert_eq!(moved.location, root);
    assert_eq!(moved.name.as_str(), "after.txt");
    assert_eq!(component.state().entries.len(), 1);
    assert_eq!(component.state().entries[0].id, source.id);
    assert_eq!(component.state().entries[0].name.as_str(), "after.txt");
    assert!(!component
        .state()
        .entries
        .iter()
        .any(|entry| entry.name.as_str() == "before.txt"));
    assert!(!source_path.exists());
    assert_file_bytes(&moved_path, CONTENTS);
    assert_eq!(component.state().selected.len(), 1);
    assert!(component.state().selected.contains(&source.id));
    assert_eq!(component.state().focused, Some(source.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(source.id)
    );

    component.refresh().unwrap();

    assert_eq!(component.state().location, root);
    assert_eq!(component.state().entries.len(), 1);
    assert_eq!(component.state().entries[0].id, source.id);
    assert_eq!(component.state().entries[0].name.as_str(), "after.txt");
    assert_eq!(component.state().selected.len(), 1);
    assert!(component.state().selected.contains(&source.id));
    assert_eq!(component.state().focused, Some(source.id));
    let inspector = component.state().layout().inspector.unwrap();
    assert_eq!(inspector.id, source.id);
    assert_eq!(inspector.name.as_str(), "after.txt");
    assert_eq!(inspector.size_bytes, Some(CONTENTS.len() as u64));

    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert!(canvas.texts.iter().any(|text| text == "F: after.txt"));
    assert!(!canvas.texts.iter().any(|text| text == "F: before.txt"));
    assert!(canvas.texts.iter().any(|text| text == "after.txt"));
    assert_eq!(canvas.selected_rows, 1);

    let (opened, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(open_operation.action_id, "files.open");
    assert_eq!(opened, CONTENTS);
}

#[test]
fn files_component_moves_selected_resource_to_another_folder_and_reads_it() {
    const CONTENTS: &[u8] = b"move component fixture";
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::create_dir(temp.path().join("destination")).unwrap();
    fs::write(temp.path().join("source/original.txt"), CONTENTS).unwrap();
    let source_location = Location::parse("source").unwrap();
    let destination = Location::parse("destination").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(source_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("source/original.txt").unwrap(),
                CapabilityRight::Move,
            ),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Create),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(
                Location::parse("destination/moved.txt").unwrap(),
                CapabilityRight::Read,
            ),
        ]),
    );
    let mut component = FilesComponent::new(service, source_location.clone());
    component.refresh().unwrap();
    let source = component.state().entries[0].clone();
    component.select(source.id, false).unwrap();

    let (moved, operation) = component
        .move_selected(
            &destination,
            &FileName::parse("moved.txt").unwrap(),
            Actor::User,
        )
        .unwrap();

    assert_eq!(moved.location, destination);
    assert_eq!(moved.name.as_str(), "moved.txt");
    assert_eq!(operation.action_id, "files.move");
    assert_eq!(operation.outcome, OperationOutcome::Applied);
    assert_eq!(operation.affected, vec![moved.id]);
    assert!(component.state().entries.is_empty());
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert_eq!(component.state().layout().inspector, None);
    assert!(!temp.path().join("source/original.txt").exists());

    component.navigate(destination.clone()).unwrap();
    assert_eq!(component.state().entries, vec![moved.clone()]);
    component.select(moved.id, false).unwrap();
    let (contents, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(contents, CONTENTS);
    assert_eq!(open_operation.action_id, "files.open");
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(moved.id)
    );
    assert_file_bytes(temp.path().join("destination/moved.txt"), CONTENTS);

    assert!(component.navigate_parent().unwrap());
    assert_eq!(component.state().location, Location::root());
    for folder in ["source", "destination"] {
        assert_eq!(
            component
                .state()
                .entries
                .iter()
                .filter(|entry| {
                    entry.kind == crate::EntryKind::Folder && entry.name.as_str() == folder
                })
                .count(),
            1,
            "parent listing contains {folder} once after returning from the destination"
        );
    }

    component.navigate(source_location.clone()).unwrap();
    assert!(component.state().entries.is_empty());
    assert!(!temp.path().join("source/original.txt").exists());
    assert_file_bytes(temp.path().join("destination/moved.txt"), CONTENTS);
}

#[test]
fn files_component_moves_folder_and_reselects_it_through_sandbox_provider() {
    const CONTENTS: &[u8] = b"moved folder remains readable after reselect\n";

    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("destination")).unwrap();
    fs::create_dir(temp.path().join("work-items")).unwrap();
    fs::write(temp.path().join("work-items/readme.txt"), CONTENTS).unwrap();

    let destination = Location::parse("destination").unwrap();
    let source_location = Location::parse("work-items").unwrap();
    let moved_folder_location = Location::parse("destination/work-items").unwrap();
    let document_location = Location::parse("destination/work-items/readme.txt").unwrap();
    let service = FilesService::new(
        SandboxProvider::new(temp.path()).unwrap(),
        CapabilitySet::from_grants([
            CapabilityGrant::allow(Location::root(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(source_location, CapabilityRight::Move),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Create),
            CapabilityGrant::allow(destination.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(moved_folder_location.clone(), CapabilityRight::Enumerate),
            CapabilityGrant::allow(document_location, CapabilityRight::Read),
        ]),
    );
    let mut component = FilesComponent::new(service, Location::root());
    component.refresh().unwrap();
    let source = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "work-items")
        .unwrap()
        .clone();
    assert_eq!(source.kind, crate::EntryKind::Folder);
    component.select(source.id, false).unwrap();

    let (moved, operation) = component
        .move_selected(
            &destination,
            &FileName::parse("work-items").unwrap(),
            Actor::User,
        )
        .unwrap();

    assert_eq!(operation.action_id, "files.move");
    assert_eq!(moved.kind, crate::EntryKind::Folder);
    assert_eq!(moved.id, source.id);
    assert_eq!(moved.location, destination);
    assert!(component
        .state()
        .entries
        .iter()
        .all(|entry| entry.name.as_str() != "work-items"));
    assert!(component.state().selected.is_empty());
    assert_eq!(component.state().focused, None);
    assert!(component.state().layout().inspector.is_none());
    assert!(!temp.path().join("work-items/readme.txt").exists());

    component.navigate(destination).unwrap();
    let moved_after_refresh = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "work-items")
        .unwrap()
        .clone();
    assert_eq!(moved_after_refresh.id, source.id);
    component.select(moved_after_refresh.id, false).unwrap();
    assert_eq!(component.state().selected.len(), 1);
    assert!(component.state().selected.contains(&moved.id));
    assert_eq!(component.state().focused, Some(moved.id));
    assert_eq!(
        component
            .state()
            .layout()
            .inspector
            .as_ref()
            .map(|entry| entry.id),
        Some(moved.id)
    );
    let mut canvas = FilesComponentCanvas::default();
    component.draw(
        &mut canvas,
        FilesCanvasRect::new(0, 0, 640, 360),
        "This folder is empty.",
        Locale::EnUs,
    );
    assert_eq!(canvas.selected_rows, 1);
    assert!(canvas.texts.iter().any(|text| text.contains("work-items")));

    component.navigate(moved_folder_location).unwrap();
    let document = component
        .state()
        .entries
        .iter()
        .find(|entry| entry.name.as_str() == "readme.txt")
        .unwrap()
        .clone();
    component.select(document.id, false).unwrap();
    let (contents, open_operation) = component.open_selected(Actor::User).unwrap().unwrap();
    assert_eq!(open_operation.action_id, "files.open");
    assert_eq!(contents, CONTENTS);
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
fn preview_input_navigation_selects_folder_in_three_pane_view() {
    let mut service = memory_service();
    let mut app = FilesApp::new(Location::root());

    dispatch_preview_input("ls", &mut service, &mut app).unwrap();
    assert!(app.state.entries.is_empty());
    dispatch_preview_input("mkdir LinuxFilesSmoke", &mut service, &mut app).unwrap();

    dispatch_preview_input("cd LinuxFilesSmoke", &mut service, &mut app).unwrap();
    assert_eq!(
        app.state.location,
        Location::parse("LinuxFilesSmoke").unwrap()
    );
    assert_eq!(
        app.state.breadcrumbs.last().unwrap().label,
        "LinuxFilesSmoke"
    );
    dispatch_preview_input("ls", &mut service, &mut app).unwrap();
    assert!(app.state.entries.is_empty());

    dispatch_preview_input("up", &mut service, &mut app).unwrap();
    assert_eq!(app.state.location, Location::root());
    dispatch_preview_input("select 1", &mut service, &mut app).unwrap();

    let layout = app.state.layout();
    let folder = layout.resources.first().unwrap();
    assert_eq!(folder.kind, EntryKind::Folder);
    assert!(layout.selected.contains(&folder.id));
    assert_eq!(layout.inspector.as_ref().unwrap().id, folder.id);
    let rendered = render_three_pane(&layout, "empty", Locale::EnUs);
    assert!(rendered.contains("1 *[D] LinuxFilesSmoke"));
    assert!(rendered.contains("Location: /"));
}

#[test]
fn preview_input_open_reads_and_displays_file_with_operation_result() {
    const EXPECTED_CONTENTS: &str = "Nagi Files open fixture contents\n";
    const BINARY_CONTENTS: &[u8] = &[0x00, 0xff, 0x80, 0x7f];

    let mut service = memory_service();
    let text_entry = service
        .provider_mut()
        .insert_file(
            &Location::parse("LinuxFilesOpenSmoke.txt").unwrap(),
            EXPECTED_CONTENTS.as_bytes(),
        )
        .unwrap();
    let binary_entry = service
        .provider_mut()
        .insert_file(
            &Location::parse("LinuxFilesOpenSmoke.bin").unwrap(),
            BINARY_CONTENTS,
        )
        .unwrap();
    let mut app = FilesApp::new(Location::root());
    let mut operations = Vec::new();
    let mut displayed_contents = Vec::new();

    dispatch_preview_input_recording(
        "ls",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    let text_index = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == text_entry.id)
        .unwrap()
        + 1;
    dispatch_preview_input_recording(
        &format!("select {text_index}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    dispatch_preview_input_recording(
        "open",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    let binary_index = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == binary_entry.id)
        .unwrap()
        + 1;
    dispatch_preview_input_recording(
        &format!("select {binary_index}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    dispatch_preview_input_recording(
        "open",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();

    assert_eq!(
        displayed_contents,
        [
            EXPECTED_CONTENTS.to_owned(),
            format!("Binary file ({} bytes).", BINARY_CONTENTS.len())
        ]
    );
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].action_id, "files.open");
    assert_eq!(operations[0].outcome, crate::OperationOutcome::Applied);
    assert_eq!(
        operations[0].reversibility,
        crate::Reversibility::NoMutation
    );
    assert_eq!(operations[0].affected, vec![text_entry.id]);
    assert_eq!(operations[1].action_id, "files.open");
    assert_eq!(operations[1].outcome, crate::OperationOutcome::Applied);
    assert_eq!(
        operations[1].reversibility,
        crate::Reversibility::NoMutation
    );
    assert_eq!(operations[1].affected, vec![binary_entry.id]);
}

#[test]
fn preview_only_adjacent_selection_uses_current_resource_ids() {
    let mut service = memory_service();
    service
        .provider_mut()
        .insert_file(&Location::parse("alpha.txt").unwrap(), b"alpha")
        .unwrap();
    service
        .provider_mut()
        .insert_file(&Location::parse("bravo.txt").unwrap(), b"bravo")
        .unwrap();
    service
        .provider_mut()
        .insert_file(&Location::parse("charlie.txt").unwrap(), b"charlie")
        .unwrap();
    let mut app = FilesApp::new(Location::root());
    dispatch_preview_input("ls", &mut service, &mut app).unwrap();
    let ids = app
        .state
        .entries
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 3);

    assert!(
        crate::preview_cli::dispatch_input_line("next", &mut service, &mut app, |_| {}, |_| {})
            .is_none(),
        "adjacent-selection commands are preview-only"
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("next", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[0]]
    );
    assert_eq!(app.state.focused, Some(ids[0]));

    assert!(
        crate::preview_cli::dispatch_preview_only_line("next", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[1]]
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("previous", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[0]]
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("previous", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[0]]
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("next", &mut app)
            .unwrap()
            .is_ok()
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("next", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[2]]
    );
    assert!(
        crate::preview_cli::dispatch_preview_only_line("next", &mut app)
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![ids[2]]
    );
}

#[test]
fn preview_input_open_without_index_uses_focused_selection_after_refresh_and_multi_selection() {
    const SELECTED_CONTENTS: &str = "originally selected file remains selected";
    const FOCUSED_CONTENTS: &str = "most recently selected file is opened";
    const SELECTED_NAME: &str = "資料.txt";
    const FOCUSED_NAME: &str = "alpha.txt";

    let mut service = memory_service();
    let folder_location = Location::parse("browse").unwrap();
    let folder = service
        .provider_mut()
        .insert_folder(&folder_location)
        .unwrap();
    let alpha = service
        .provider_mut()
        .insert_file(
            &Location::parse(FOCUSED_NAME).unwrap(),
            FOCUSED_CONTENTS.as_bytes(),
        )
        .unwrap();
    let selected = service
        .provider_mut()
        .insert_file(
            &Location::parse(SELECTED_NAME).unwrap(),
            SELECTED_CONTENTS.as_bytes(),
        )
        .unwrap();
    let mut app = FilesApp::new(Location::root());
    let mut operations = Vec::new();
    let mut displayed_contents = Vec::new();

    dispatch_preview_input_recording(
        "ls",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    let selected_index = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == selected.id)
        .unwrap()
        + 1;
    dispatch_preview_input_recording(
        &format!("select {selected_index}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    let alpha_index = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == alpha.id)
        .unwrap()
        + 1;
    dispatch_preview_input_recording(
        &format!("select-add {alpha_index}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    assert_eq!(app.state.selected.len(), 2);
    assert!(app.state.selected.contains(&selected.id));
    assert!(app.state.selected.contains(&alpha.id));
    assert_eq!(app.state.focused, Some(alpha.id));

    service
        .provider_mut()
        .insert_file(
            &Location::parse("0_arrived.txt").unwrap(),
            b"an earlier row inserted after selection",
        )
        .unwrap();
    dispatch_preview_input_recording(
        "ls",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    let selected_index_after_refresh = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == selected.id)
        .unwrap()
        + 1;
    assert_ne!(selected_index_after_refresh, selected_index);
    assert_ne!(app.state.entries[selected_index - 1].id, selected.id);
    let alpha_index_after_refresh = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == alpha.id)
        .unwrap()
        + 1;
    assert_ne!(alpha_index_after_refresh, alpha_index);
    assert_eq!(app.state.selected.len(), 2);
    assert!(app.state.selected.contains(&selected.id));
    assert!(app.state.selected.contains(&alpha.id));
    assert_eq!(app.state.focused, Some(alpha.id));
    assert_eq!(
        app.state.layout().inspector.as_ref().map(|entry| entry.id),
        Some(alpha.id)
    );
    dispatch_preview_input_recording(
        &format!("open {selected_index_after_refresh}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    assert_eq!(displayed_contents, [SELECTED_CONTENTS]);
    assert_eq!(operations.len(), 1);
    assert_opened_resource(&operations[0], selected.id);
    assert_eq!(app.state.focused, Some(alpha.id));
    assert!(app.state.selected.contains(&selected.id));
    assert!(app.state.selected.contains(&alpha.id));

    dispatch_preview_input_recording(
        "open",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();

    assert_eq!(displayed_contents, [SELECTED_CONTENTS, FOCUSED_CONTENTS]);
    assert_eq!(operations.len(), 2);
    assert_opened_resource(&operations[1], alpha.id);
    assert_eq!(app.state.location, Location::root());
    assert_eq!(app.state.focused, Some(alpha.id));
    assert!(app.state.selected.contains(&selected.id));
    assert!(app.state.selected.contains(&alpha.id));

    let folder_index = app
        .state
        .entries
        .iter()
        .position(|entry| entry.id == folder.id)
        .unwrap()
        + 1;
    dispatch_preview_input_recording(
        &format!("select {folder_index}"),
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    dispatch_preview_input_recording(
        "open",
        &mut service,
        &mut app,
        &mut operations,
        &mut displayed_contents,
    )
    .unwrap();
    assert_eq!(app.state.location, folder_location);
    assert!(app.state.entries.is_empty());
    assert!(app.state.selected.is_empty());
    assert_eq!(app.state.focused, None);
    assert_eq!(folder.kind, EntryKind::Folder);

    dispatch_preview_input("up", &mut service, &mut app).unwrap();
    assert_eq!(app.state.location, Location::root());
    let parent_ids = app
        .state
        .entries
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert_eq!(parent_ids.len(), 4);
    dispatch_preview_input("up", &mut service, &mut app).unwrap();
    assert_eq!(app.state.location, Location::root());
    assert_eq!(
        app.state
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        parent_ids,
        "Back at the Files root must not move outside the provider root"
    );
    assert_eq!(
        crate::preview_cli::resolve_location(&Location::root(), "..")
            .unwrap_err()
            .kind,
        crate::FilesErrorKind::SandboxEscape
    );
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
fn search_matches_changed_tags_on_the_selected_refreshed_row_without_indexing_file_contents() {
    let mut service = memory_service();
    let source = service
        .provider_mut()
        .insert_file(&Location::parse("data.bin").unwrap(), b"secret body")
        .unwrap();
    let other = service
        .provider_mut()
        .insert_file(&Location::parse("other.bin").unwrap(), b"other body")
        .unwrap();
    let mut app = FilesApp::new(Location::root());
    app.refresh(&service).unwrap();
    app.state.select(source.id, false).unwrap();
    assert_eq!(app.state.focused, Some(source.id));
    assert_eq!(
        app.state.layout().inspector.as_ref().map(|entry| entry.id),
        Some(source.id)
    );

    let tagged = service
        .set_tags(&source, &[Tag::parse("quarterly").unwrap()], Actor::User)
        .unwrap()
        .0;
    app.refresh(&service).unwrap();
    let selected_row = app
        .state
        .entries
        .iter()
        .find(|entry| entry.id == source.id)
        .expect("the tagged file remains in the refreshed list");
    assert_eq!(selected_row.tags, vec!["quarterly"]);
    assert_eq!(
        app.state.selected.iter().copied().collect::<Vec<_>>(),
        vec![source.id]
    );
    assert_eq!(app.state.focused, Some(source.id));
    let inspector = app.state.layout().inspector.unwrap();
    assert_eq!(inspector.id, source.id);
    assert_eq!(inspector.tags, vec!["quarterly"]);

    let results = FilesSearchProvider::default()
        .search(&service, &Location::root(), "quarterly", 10)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].resource_id, tagged.id);
    assert_eq!(results[0].resource_id, source.id);
    assert_eq!(results[0].tags, vec!["quarterly"]);
    assert!(!app
        .state
        .entries
        .iter()
        .any(|entry| entry.id == other.id && !entry.tags.is_empty()));
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
    assert_file_bytes(outside.path().join("secret.txt"), b"outside");
    #[cfg(unix)]
    {
        let root_alias = outside.path().join("sandbox-root-link");
        std::os::unix::fs::symlink(temp.path(), &root_alias).unwrap();
        assert!(SandboxProvider::new(&root_alias).is_err());
        fs::remove_file(root_alias).unwrap();
    }
}

#[test]
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
    assert_file_bytes(outside.path().join("private.txt"), b"private");
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
    assert_file_bytes(temp.path().join("source-copy/nested/file.txt"), b"payload");
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
    assert_file_bytes(
        temp.path().join("destination/renamed/nested/file.txt"),
        b"payload",
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
    assert_file_bytes(temp.path().join("conflict.txt"), b"keep");
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
    assert_file_bytes(temp.path().join("source/file.txt"), b"contents");
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
fn sandbox_trash_persists_across_process_reopen_and_restores_resource_identity() {
    const CHILD_ROOT: &str = "NAGI_FILES_TRASH_REOPEN_CHILD_ROOT";
    const CHILD_ID: &str = "NAGI_FILES_TRASH_REOPEN_CHILD_ID";
    const PARENT_PID: &str = "NAGI_FILES_TRASH_REOPEN_PARENT_PID";
    const CHILD_EXECUTABLE: &str = "NAGI_FILES_TRASH_REOPEN_CHILD_EXECUTABLE";
    const HOLD_OPEN: &str = "NAGI_FILES_TRASH_REOPEN_HOLD_OPEN";
    const TEST_NAME: &str =
        "tests::sandbox_trash_persists_across_process_reopen_and_restores_resource_identity";
    const CONTENTS: &[u8] = b"cross-process restore";

    let location = Location::parse("restore.txt").unwrap();
    let test_executable = std::env::var_os(CHILD_EXECUTABLE)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap());
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let expected_id = std::env::var(CHILD_ID).expect("child ResourceId");
        let parent_pid = std::env::var(PARENT_PID)
            .expect("parent process id")
            .parse::<u32>()
            .expect("valid parent process id");
        assert_ne!(std::process::id(), parent_pid, "child must be a new process");

        let mut provider = SandboxProvider::new(PathBuf::from(root)).unwrap();
        let trash = provider.list_trash().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id.to_string(), expected_id);

        let restored = provider.restore(trash[0].id).unwrap();
        assert_eq!(restored.id.to_string(), expected_id);
        assert_eq!(provider.read_file(&location, crate::MAX_PREVIEW_BYTES).unwrap(), CONTENTS);
        return;
    }

    let temp = TempSandbox::new();
    fs::write(temp.path().join("restore.txt"), CONTENTS).unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let original = provider.metadata(&location).unwrap();
    let trashed = provider.trash(&location).unwrap();
    assert_eq!(trashed.id, original.id);
    drop(provider);

    let child = std::process::Command::new(&test_executable)
        .arg("--exact")
        .arg(TEST_NAME)
        .arg("--nocapture")
        .env(CHILD_ROOT, temp.path())
        .env(CHILD_ID, original.id.to_string())
        .env(PARENT_PID, std::process::id().to_string())
        .env(CHILD_EXECUTABLE, &test_executable)
        .output()
        .expect("launch child test process");
    assert!(
        child.status.success(),
        "child test process failed: status={}\nstdout:\n{}\nstderr:\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );
    assert!(
        String::from_utf8_lossy(&child.stdout).contains("1 passed"),
        "child test harness did not execute the selected test: {}",
        String::from_utf8_lossy(&child.stdout),
    );

    let provider = SandboxProvider::new(temp.path()).unwrap();
    assert!(provider.list_trash().unwrap().is_empty());
    assert_eq!(provider.metadata(&location).unwrap().id, original.id);
    assert_eq!(provider.read_file(&location, crate::MAX_PREVIEW_BYTES).unwrap(), CONTENTS);

    if std::env::var_os(HOLD_OPEN).is_some() {
        println!(
            "NAGI_FILES_CROSS_PROCESS_TRASH_PASS child_reopened=true resource_id={} bytes_match=true trash_empty=true pid1_held=true",
            original.id,
        );
        loop {
            std::thread::sleep(std::time::Duration::from_secs(300));
        }
    }
}

#[test]
fn sandbox_reloads_unindexed_trash_and_restores_the_payload() {
    let temp = TempSandbox::new();
    fs::write(temp.path().join("recover.txt"), b"recover me").unwrap();
    let internal = temp.path().join(".nagi-files");
    fs::create_dir_all(internal.join("trash")).unwrap();
    let blocked_index = internal.join("trash-index-v1.tmp");
    fs::create_dir(&blocked_index).unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let error = provider.trash(&Location::parse("recover.txt").unwrap()).unwrap_err();
    assert_eq!(error.kind, FilesErrorKind::PartialFailure);
    drop(provider);
    fs::remove_dir(blocked_index).unwrap();
    let index = internal.join("trash-index-v1");

    let mut reopened = SandboxProvider::new(temp.path()).unwrap();
    let entries = reopened.list_trash().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!index.exists(), "opening the provider must remain read-only");
    let restored = reopened.restore(entries[0].id).unwrap();
    assert!(restored.location.is_root());
    assert_file_bytes(temp.path().join(restored.name.as_str()), b"recover me");
    assert!(reopened.list_trash().unwrap().is_empty());
}

#[test]
fn sandbox_root_fallback_requires_restore_permission_at_the_destination() {
    let temp = TempSandbox::new();
    fs::create_dir(temp.path().join("gone")).unwrap();
    fs::write(temp.path().join("gone/item.txt"), b"payload").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let original = Location::parse("gone/item.txt").unwrap();
    let trashed = provider.trash(&original).unwrap();
    fs::remove_dir(temp.path().join("gone")).unwrap();

    let mut denied = FilesService::new(provider, CapabilitySet::from_grants([
        CapabilityGrant::allow(original, CapabilityRight::Restore),
    ]));
    assert_eq!(
        denied.restore(trashed.id, Actor::User).unwrap_err().kind,
        FilesErrorKind::PermissionDenied,
    );
    assert!(!temp.path().join("item.txt").exists());
    let mut allowed = FilesService::new(SandboxProvider::new(temp.path()).unwrap(), full_capabilities());
    let (entry, _) = allowed.restore(trashed.id, Actor::User).unwrap();
    assert!(entry.location.is_root());
    assert_file_bytes(temp.path().join("item.txt"), b"payload");
}

#[test]
fn sandbox_restore_as_preserves_an_occupied_original_name() {
    let temp = TempSandbox::new();
    let original = Location::parse("item.txt").unwrap();
    fs::write(temp.path().join("item.txt"), b"trashed").unwrap();
    let mut provider = SandboxProvider::new(temp.path()).unwrap();
    let trashed = provider.trash(&original).unwrap();
    fs::write(temp.path().join("item.txt"), b"current").unwrap();
    let mut service = FilesService::new(provider, full_capabilities());
    let alternate = FileName::parse("item (restored).txt").unwrap();
    service
        .restore_as(trashed.id, &Location::root(), &alternate, Actor::User)
        .unwrap();
    assert_file_bytes(temp.path().join("item.txt"), b"current");
    assert_file_bytes(temp.path().join(alternate.as_str()), b"trashed");
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
    assert_file_bytes(temp.path().join("item.txt"), b"new");
    fs::remove_file(temp.path().join("item.txt")).unwrap();
    provider.restore(trashed.id).unwrap();
    assert_file_bytes(temp.path().join("item.txt"), b"old");
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
    assert_eq!(crate::FILES_ACTIONS.len(), 16);
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
    assert_eq!(
        crate::find_action("files.write").unwrap().required_rights,
        &[CapabilityRight::Write]
    );
    assert!(crate::find_action("files.write").unwrap().reversible);
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

    let file_location = Location::parse("reports/draft.txt").unwrap();
    service
        .provider_mut()
        .insert_file(&file_location, b"before save")
        .unwrap();
    let draft = service.provider().metadata(&file_location).unwrap();
    let written = FilesActionApi::execute(
        &mut service,
        FilesActionCall::Write {
            resource: draft,
            contents: b"saved through the typed action".to_vec(),
        },
        Actor::User,
    )
    .unwrap();
    let FilesActionResponse::ResourceChanged {
        resource: saved,
        operation,
    } = written
    else {
        panic!("write action must return the updated resource");
    };
    assert_eq!(operation.action_id, "files.write");
    assert_eq!(
        service.read_file(&saved.child_location()).unwrap(),
        b"saved through the typed action"
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
