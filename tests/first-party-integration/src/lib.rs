//! Real package-to-package adapters for the explicitly host-only first-party
//! integration preview. Nothing in this module is a Nagi target service.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nagi_files::{
    ActivityEvent as FilesActivityEvent, ActivityOutcome as FilesActivityOutcome,
    ActivitySink as FilesActivitySink, Actor as FilesActor, CapabilityAuthorizer, CapabilityGrant,
    CapabilityRight, CapabilitySet, CheckpointHook, CheckpointSnapshot, FilesSearchProvider,
    FilesService, FilesystemProvider, InMemoryProvider, Location, OperationResult, ResourceId,
    SearchRecord as FilesSearchRecord, WaybackCheckpointRequest,
};
use nagi_history::activity::{
    ActionKind, ActivityAccessPolicy, ActivityDraft, ActivityEvent, ActivityLedger, ActivityQuery,
    ActivitySink, Actor, ActorId, ActorKind, CheckpointId, EventResult, FailureCode, Reversibility,
    RevisionId, Timestamp, UserLocale,
};
use nagi_history::wayback::{
    CheckpointAccessPolicy, CheckpointDraft, CheckpointObject, CheckpointOrigin, CheckpointQuery,
    CheckpointReason, CheckpointScope, CheckpointStore, RevisionDraft, RevisionStore,
    SnapshotBackendRef,
};
use nagi_history::{ActivityContext, AppSessionId, NodeId, SurfaceId};
use nagi_home_search::actions::ActionAvailability;
use nagi_home_search::home::HomeData;
use nagi_home_search::registry::{
    AppAvailability, AppDescriptor, AppRegistry, IconMetadata, RegistrySnapshot,
};
use nagi_home_search::search::{
    ProviderDescriptor, ProviderError, ProviderId, SearchAction, SearchCandidate, SearchCategory,
    SearchContext, SearchCoordinator, SearchIdentity, SearchProvider, SearchQuery, SearchText,
};
use nagi_home_search::{
    CapabilityContext, CapabilityId, HomeController, HomeDataSource, HomeError, Locale,
    SearchResponse, TypedAction,
};
use nagi_model::{AppId, ObjectId, TransactionId, WorkspaceId};
use nagi_notes::{
    ActionPolicyError, ActionPrincipal, ActivityEvent as NotesActivityEvent,
    ActivityKind as NotesActivityKind, ActivityOrigin, ActivitySink as NotesActivitySink,
    NoteStore, NotesActionKind, NotesActionPolicy, NotesApp, NotesSearchProvider,
    SearchProvider as NotesSearchProviderContract,
};

pub const FILES_APP_ID: AppId = AppId::from_identifier(b"com.nagi.files");
pub const NOTES_METADATA_CAPABILITY: &str = "notes.metadata.read";
pub const NOTES_SEARCH_CAPABILITY: &str = "notes.search";
pub const NOTES_OPEN_CAPABILITY: &str = "notes.open";
pub const FILES_METADATA_CAPABILITY: &str = "files.metadata.read";
pub const FILES_SEARCH_CAPABILITY: &str = "files.search";
pub const FILES_OPEN_CAPABILITY: &str = "files.open";
pub const ACTIVITY_SEARCH_CAPABILITY: &str = "activity.search";
pub const ACTIVITY_READ_CAPABILITY: &str = "activity.read";
pub const WAYBACK_SEARCH_CAPABILITY: &str = "wayback.search";
pub const WAYBACK_READ_CAPABILITY: &str = "wayback.read";

const HOST_OBJECT_PREFIX: u64 = 0xf000_0000_0000_0000;
const PREVIEW_USER: Actor = Actor::new(ActorId(1), ActorKind::User);

/// A single host-preview allocator is shared by Notes and Files so their
/// canonical ObjectIds remain globally distinct inside this in-memory run.
/// The high-bit namespace is ephemeral and is not a target Object service.
pub struct PreviewObjectIdAllocator {
    next: AtomicU64,
}

impl PreviewObjectIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    fn next(&self) -> ObjectId {
        let value = self.next.fetch_add(1, Ordering::Relaxed);
        assert!(
            value < (1_u64 << 60),
            "host preview ObjectId space exhausted"
        );
        ObjectId(HOST_OBJECT_PREFIX | value)
    }
}

impl Default for PreviewObjectIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl nagi_notes::ObjectIdSource for PreviewObjectIdAllocator {
    fn next_object_id(&self) -> ObjectId {
        self.next()
    }
}

/// Maps the full Files ResourceId to a canonical host-preview ObjectId.
/// No narrowing cast or hash-based identity is used.
#[derive(Clone)]
pub struct FilesObjectResolver {
    allocator: Arc<PreviewObjectIdAllocator>,
    entries: Arc<Mutex<ResolverEntries>>,
}

#[derive(Default)]
struct ResolverEntries {
    by_resource: BTreeMap<ResourceId, ObjectId>,
    by_object: HashMap<u64, ResourceId>,
}

impl FilesObjectResolver {
    pub fn new(allocator: Arc<PreviewObjectIdAllocator>) -> Self {
        Self {
            allocator,
            entries: Arc::new(Mutex::new(ResolverEntries::default())),
        }
    }

    pub fn resolve(&self, resource_id: ResourceId) -> Result<ObjectId, String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "Files Object resolver lock poisoned".to_owned())?;
        if let Some(object_id) = entries.by_resource.get(&resource_id) {
            return Ok(*object_id);
        }
        let object_id = self.allocator.next();
        entries.by_resource.insert(resource_id, object_id);
        entries.by_object.insert(object_id.0, resource_id);
        Ok(object_id)
    }

    pub fn resource_id(&self, object_id: ObjectId) -> Result<Option<ResourceId>, String> {
        self.entries
            .lock()
            .map(|entries| entries.by_object.get(&object_id.0).copied())
            .map_err(|_| "Files Object resolver lock poisoned".to_owned())
    }
}

#[derive(Clone)]
pub struct HostHistory {
    state: Arc<Mutex<HostHistoryState>>,
    note_store: Arc<dyn NoteStore>,
}

struct HostHistoryState {
    activity: ActivityLedger,
    checkpoints: CheckpointStore,
    revisions: RevisionStore,
    latest_note_revision: HashMap<u64, (u64, RevisionId)>,
    note_revision_ids: HashMap<(u64, u64), RevisionId>,
    note_snapshot_ids: HashMap<(u64, u64), SnapshotBackendRef>,
    note_snapshot_refs: HashMap<u64, (ObjectId, u64)>,
    latest_file_revision: HashMap<u128, RevisionId>,
    file_snapshot_groups: HashMap<u64, Vec<HostFileSnapshot>>,
    file_snapshot_transactions: HashMap<u64, SnapshotBackendRef>,
    file_checkpoint_transactions: HashMap<u64, CheckpointId>,
    next_snapshot_ref: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub struct HostFileSnapshot {
    pub resource_id: ResourceId,
    pub object_id: ObjectId,
    pub location: String,
    pub contents: Vec<u8>,
    pub modified_at_epoch_seconds: Option<i64>,
}

impl HostHistory {
    pub fn new(note_store: Arc<dyn NoteStore>) -> Self {
        Self {
            state: Arc::new(Mutex::new(HostHistoryState {
                activity: ActivityLedger::new(),
                checkpoints: CheckpointStore::new(),
                revisions: RevisionStore::new(),
                latest_note_revision: HashMap::new(),
                note_revision_ids: HashMap::new(),
                note_snapshot_ids: HashMap::new(),
                note_snapshot_refs: HashMap::new(),
                latest_file_revision: HashMap::new(),
                file_snapshot_groups: HashMap::new(),
                file_snapshot_transactions: HashMap::new(),
                file_checkpoint_transactions: HashMap::new(),
                next_snapshot_ref: 1,
            })),
            note_store,
        }
    }

    pub fn counts(&self) -> Result<(usize, usize, usize), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        Ok((
            state.activity.len(),
            state.checkpoints.len(),
            state.revisions.len(),
        ))
    }

    /// Resolve an opaque Wayback backend reference to the real persisted Notes
    /// revision held by this preview's NoteStore.
    pub fn load_note_snapshot(
        &self,
        backend_ref: SnapshotBackendRef,
    ) -> Result<Option<nagi_notes::NoteDocument>, String> {
        let version = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?
            .note_snapshot_refs
            .get(&backend_ref.0)
            .copied();
        let Some((object_id, revision)) = version else {
            return Ok(None);
        };
        self.note_store
            .load_revision(object_id, revision)
            .map_err(|error| error.to_string())
    }

    pub fn note_snapshot_ref(
        &self,
        object_id: ObjectId,
        revision: u64,
    ) -> Result<Option<SnapshotBackendRef>, String> {
        self.state
            .lock()
            .map(|state| {
                state
                    .note_snapshot_ids
                    .get(&(object_id.0, revision))
                    .copied()
            })
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())
    }

    pub fn file_snapshot_for_transaction(
        &self,
        transaction_id: u64,
    ) -> Result<Option<Vec<HostFileSnapshot>>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        let Some(backend_ref) = state.file_snapshot_transactions.get(&transaction_id) else {
            return Ok(None);
        };
        Ok(state.file_snapshot_groups.get(&backend_ref.0).cloned())
    }

    pub fn file_checkpoint_for_transaction(
        &self,
        transaction_id: u64,
    ) -> Result<Option<CheckpointId>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        Ok(state
            .activity
            .query_visible(
                ActivityQuery::default(),
                PREVIEW_USER,
                &PreviewActivityPolicy,
            )
            .find(|event| event.transaction_id() == Some(TransactionId(transaction_id)))
            .and_then(ActivityEvent::checkpoint_before))
    }

    fn record_note_event(&self, event: NotesActivityEvent) -> Result<(), String> {
        let actor = match event.origin {
            ActivityOrigin::User => PREVIEW_USER,
            ActivityOrigin::Agent(_) | ActivityOrigin::Mixed => {
                return Err(
                    "host preview requires delegated provenance for agent or mixed Notes events"
                        .to_owned(),
                );
            }
        };
        let timestamp = notes_timestamp(event.occurred_at.0)?;
        let context = preview_context(nagi_notes::APP_ID, event.workspace_id);
        let persisted = if event.kind == NotesActivityKind::Opened || event.revision == 0 {
            None
        } else {
            Some(
                self.note_store
                    .load_revision(event.object_id, event.revision)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| {
                        format!(
                            "Notes activity references unavailable persisted revision {}",
                            event.revision
                        )
                    })?,
            )
        };

        let mut state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        let mut draft =
            ActivityDraft::new(timestamp, actor, context, notes_action_kind(event.kind))
                .with_result(EventResult::Succeeded)
                .with_reversibility(Reversibility::Unknown)
                .with_target(event.object_id)
                .map_err(|error| format!("could not construct Notes Activity event: {error:?}"))?;

        let revision_key = (event.object_id.0, event.revision);
        let new_revision =
            persisted.is_some() && !state.note_revision_ids.contains_key(&revision_key);
        if new_revision {
            if !state.activity.can_append_events(2) || !state.checkpoints.can_create() {
                return Err("Activity or Wayback host preview capacity exhausted".to_owned());
            }
            if state.revisions.len() >= nagi_history::wayback::MAX_REVISIONS {
                return Err("Wayback revision host preview capacity exhausted".to_owned());
            }
            if state.next_snapshot_ref == 0 || state.next_snapshot_ref == u64::MAX {
                return Err("Wayback snapshot reference space exhausted".to_owned());
            }
            let backend_ref = SnapshotBackendRef(state.next_snapshot_ref);
            let prior = state.latest_note_revision.get(&event.object_id.0).copied();
            let parent = prior
                .filter(|(prior_revision, _)| *prior_revision < event.revision)
                .map(|(_, revision_id)| revision_id);
            let revision_id = state
                .revisions
                .append(RevisionDraft::new(
                    event.object_id,
                    parent,
                    timestamp,
                    actor,
                    context,
                    backend_ref,
                ))
                .map_err(|error| format!("could not append Notes Wayback revision: {error:?}"))?;
            state.next_snapshot_ref += 1;
            state
                .note_snapshot_refs
                .insert(backend_ref.0, (event.object_id, event.revision));
            state.note_snapshot_ids.insert(revision_key, backend_ref);
            state.note_revision_ids.insert(revision_key, revision_id);
            state
                .latest_note_revision
                .insert(event.object_id.0, (event.revision, revision_id));
            let checkpoint = CheckpointDraft::new(
                timestamp,
                actor,
                context,
                CheckpointScope::Document,
                CheckpointOrigin::User,
                CheckpointReason::AutomaticHistory,
                backend_ref,
            )
            .with_object(CheckpointObject::new(event.object_id, revision_id))
            .map_err(|error| format!("could not build Notes checkpoint: {error:?}"))?;

            state
                .activity
                .append(draft)
                .map_err(|error| format!("could not append Notes Activity event: {error:?}"))?;
            let HostHistoryState {
                checkpoints,
                activity,
                ..
            } = &mut *state;
            checkpoints
                .create(checkpoint, activity)
                .map_err(|error| format!("could not create Notes Wayback checkpoint: {error:?}"))?;
            return Ok(());
        }

        // Non-mutating events and later producer notifications for the same
        // saved revision still enter Activity, but never duplicate snapshots.
        draft = draft.with_result(EventResult::Succeeded);
        state
            .activity
            .append(draft)
            .map_err(|error| format!("could not append Notes Activity event: {error:?}"))?;
        Ok(())
    }

    fn record_files_event(
        &self,
        event: &FilesActivityEvent,
        resolver: &FilesObjectResolver,
    ) -> Result<(), String> {
        if event.actor != FilesActor::User {
            return Err(
                "host preview requires delegated provenance for non-user Files events".to_owned(),
            );
        }
        let timestamp = Timestamp::new(event.timestamp_epoch_seconds, 0)
            .map_err(|error| format!("invalid Files Activity timestamp: {error:?}"))?;
        let context = preview_context(FILES_APP_ID, None);
        let (action, result) = match &event.result {
            FilesActivityOutcome::Started => {
                (files_action_kind(event.action_id), EventResult::Pending)
            }
            FilesActivityOutcome::Succeeded => {
                (files_action_kind(event.action_id), EventResult::Succeeded)
            }
            FilesActivityOutcome::Failed(_) => (
                ActionKind::OperationFailed,
                EventResult::Failed(FailureCode::BackendFailure),
            ),
            FilesActivityOutcome::Denied => (
                ActionKind::OperationFailed,
                EventResult::Failed(FailureCode::PermissionDenied),
            ),
        };
        let reversibility = if event.action_id == "files.delete_permanently" {
            Reversibility::Irreversible
        } else {
            Reversibility::Unknown
        };
        let mut draft = ActivityDraft::new(timestamp, PREVIEW_USER, context, action)
            .with_result(result)
            .with_reversibility(reversibility)
            .with_transaction(TransactionId(event.transaction_id.0));
        if event.target_resources.len() > nagi_history::activity::MAX_ACTIVITY_OBJECTS {
            return Err("Files event exceeds shared Activity object-reference limit".to_owned());
        }
        for resource_id in &event.target_resources {
            let object_id = resolver.resolve(*resource_id)?;
            draft = draft
                .with_target(object_id)
                .map_err(|error| format!("could not add Files Activity target: {error:?}"))?;
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        if let Some(checkpoint_id) = state
            .file_checkpoint_transactions
            .get(&event.transaction_id.0)
            .copied()
        {
            draft = draft.with_checkpoint_before(checkpoint_id);
        }
        state
            .activity
            .append(draft)
            .map_err(|error| format!("could not append Files Activity event: {error:?}"))?;
        if !matches!(event.result, FilesActivityOutcome::Started) {
            state
                .file_checkpoint_transactions
                .remove(&event.transaction_id.0);
        }
        Ok(())
    }

    fn record_files_checkpoint(
        &self,
        request: &WaybackCheckpointRequest,
        snapshots: &[CheckpointSnapshot],
        resolver: &FilesObjectResolver,
    ) -> Result<String, String> {
        if request.actor != FilesActor::User {
            return Err("host Wayback preview accepts only authenticated user operations".into());
        }
        if snapshots.is_empty() {
            return Err("Files operation has no readable pre-operation file snapshot".into());
        }
        if snapshots.len() > nagi_history::activity::MAX_ACTIVITY_OBJECTS {
            return Err(
                "Files checkpoint exceeds the shared Activity object-reference limit".into(),
            );
        }
        let mut resolved = Vec::with_capacity(snapshots.len());
        for snapshot in snapshots {
            resolved.push((snapshot, resolver.resolve(snapshot.resource_id)?));
        }
        let timestamp = system_timestamp()?;
        let context = preview_context(FILES_APP_ID, None);
        let transaction = TransactionId(request.transaction_id.0);
        let mut state = self
            .state
            .lock()
            .map_err(|_| "host Activity/Wayback lock poisoned".to_owned())?;
        // CheckpointStore::create appends its own Activity event, and the
        // FilesService will append the operation result once the mutation
        // completes. Do not create an orphan checkpoint when only one ledger
        // slot remains.
        if !state.activity.can_append_events(2)
            || !state.checkpoints.can_create()
            || state.revisions.len() + resolved.len() > nagi_history::wayback::MAX_REVISIONS
        {
            return Err("Activity or Wayback host preview capacity exhausted".into());
        }
        if state.next_snapshot_ref == 0 || state.next_snapshot_ref == u64::MAX {
            return Err("Wayback snapshot reference space exhausted".into());
        }
        let backend_ref = SnapshotBackendRef(state.next_snapshot_ref);
        state.next_snapshot_ref += 1;
        let mut checkpoint = CheckpointDraft::new(
            timestamp,
            PREVIEW_USER,
            context,
            CheckpointScope::Transaction,
            CheckpointOrigin::User,
            CheckpointReason::TransactionBoundary,
            backend_ref,
        )
        .with_transaction(transaction);
        let mut stored = Vec::with_capacity(resolved.len());
        for (snapshot, object_id) in resolved {
            let parent = state
                .latest_file_revision
                .get(&snapshot.resource_id.0)
                .copied();
            let revision_id = state
                .revisions
                .append(
                    RevisionDraft::new(
                        object_id,
                        parent,
                        timestamp,
                        PREVIEW_USER,
                        context,
                        backend_ref,
                    )
                    .with_transaction(transaction),
                )
                .map_err(|error| format!("could not append Files Wayback revision: {error:?}"))?;
            state
                .latest_file_revision
                .insert(snapshot.resource_id.0, revision_id);
            checkpoint = checkpoint
                .with_object(CheckpointObject::new(object_id, revision_id))
                .map_err(|error| format!("could not build Files checkpoint: {error:?}"))?;
            stored.push(HostFileSnapshot {
                resource_id: snapshot.resource_id,
                object_id,
                location: snapshot.location.as_str().to_owned(),
                contents: snapshot.contents.clone(),
                modified_at_epoch_seconds: snapshot.modified_at_epoch_seconds,
            });
        }
        state.file_snapshot_groups.insert(backend_ref.0, stored);
        let (checkpoint_id, _) = {
            let HostHistoryState {
                checkpoints,
                activity,
                ..
            } = &mut *state;
            checkpoints
                .create(checkpoint, activity)
                .map_err(|error| format!("could not create Files Wayback checkpoint: {error:?}"))?
        };
        state
            .file_snapshot_transactions
            .insert(request.transaction_id.0, backend_ref);
        state
            .file_checkpoint_transactions
            .insert(request.transaction_id.0, checkpoint_id);
        Ok(format!("checkpoint:{}", checkpoint_id.0))
    }
}

#[derive(Clone)]
pub struct NotesActivityBridge(HostHistory);

impl NotesActivitySink for NotesActivityBridge {
    fn record(&self, event: NotesActivityEvent) -> Result<(), nagi_notes::ActivityError> {
        self.0
            .record_note_event(event)
            .map_err(nagi_notes::ActivityError)
    }
}

#[derive(Clone)]
pub struct FilesActivityBridge {
    history: HostHistory,
    resolver: FilesObjectResolver,
}

impl FilesActivityBridge {
    pub fn new(history: HostHistory, resolver: FilesObjectResolver) -> Self {
        Self { history, resolver }
    }
}

impl FilesActivitySink for FilesActivityBridge {
    fn record(&mut self, event: &FilesActivityEvent) -> Result<(), String> {
        self.history.record_files_event(event, &self.resolver)
    }
}

/// Stores read-authorized Files snapshots in the same host Wayback stores used
/// by Notes. The FilesService supplies bytes only for supported file-level
/// operations; unsupported cases fail without fabricating a checkpoint.
#[derive(Clone)]
pub struct SharedFilesWaybackHook {
    history: HostHistory,
    resolver: FilesObjectResolver,
}

impl SharedFilesWaybackHook {
    pub fn new(history: HostHistory, resolver: FilesObjectResolver) -> Self {
        Self { history, resolver }
    }
}

impl CheckpointHook for SharedFilesWaybackHook {
    fn checkpoint_before(&mut self, _request: &WaybackCheckpointRequest) -> Result<String, String> {
        Err("Files Wayback hook requires a read-authorized source snapshot".into())
    }

    fn checkpoint_before_with_snapshots(
        &mut self,
        request: &WaybackCheckpointRequest,
        snapshots: &[CheckpointSnapshot],
    ) -> Result<String, String> {
        self.history
            .record_files_checkpoint(request, snapshots, &self.resolver)
    }
}

pub struct HostUserNotesPolicy;

impl NotesActionPolicy for HostUserNotesPolicy {
    fn authorize(
        &self,
        principal: ActionPrincipal,
        action: NotesActionKind,
        _object_id: Option<ObjectId>,
    ) -> Result<(), ActionPolicyError> {
        let _ = action;
        if principal == ActionPrincipal::User {
            Ok(())
        } else {
            Err(ActionPolicyError::Denied)
        }
    }
}

pub struct NotesSearchAdapter {
    provider: NotesSearchProvider,
    policy: Arc<dyn NotesActionPolicy>,
}

impl NotesSearchAdapter {
    pub fn new(app: Arc<NotesApp>, policy: Arc<dyn NotesActionPolicy>) -> Self {
        Self {
            provider: NotesSearchProvider::for_app(app),
            policy,
        }
    }
}

impl SearchProvider for NotesSearchAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: provider_id("notes"),
            priority: 20,
            required_capability: Some(capability(NOTES_SEARCH_CAPABILITY)),
        }
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let hits = self
            .provider
            .search(&query.raw_query)
            .map_err(|_| ProviderError::Failed)?;
        let mut results = Vec::new();
        for hit in hits {
            match self.policy.authorize(
                ActionPrincipal::User,
                NotesActionKind::Get,
                Some(hit.object_id),
            ) {
                Ok(()) => {}
                Err(ActionPolicyError::Denied) => continue,
                Err(ActionPolicyError::ProviderUnavailable(_)) => {
                    return Err(ProviderError::Unavailable);
                }
            }
            let open_capability = capability(NOTES_OPEN_CAPABILITY);
            let availability = if query.capabilities.allows(Some(&open_capability)) {
                ActionAvailability::HostPreviewOnly
            } else {
                ActionAvailability::PermissionRequired {
                    capability: open_capability.clone(),
                }
            };
            results.push(SearchCandidate {
                identity: SearchIdentity::Object(hit.object_id),
                category: SearchCategory::Notes,
                text: SearchText {
                    title: hit.title,
                    subtitle: hit.tags.first().cloned(),
                    content: Some(hit.snippet.clone()),
                    tags: hit.tags,
                },
                modified_at_unix_seconds: Some(hit.modified_at.0 / 1_000),
                workspace_ids: hit.workspace_ids,
                visibility_capability: Some(capability(NOTES_METADATA_CAPABILITY)),
                action: Some(SearchAction {
                    action: TypedAction::OpenObject {
                        object_id: hit.object_id,
                        app_id: nagi_notes::APP_ID,
                    },
                    required_capability: Some(open_capability),
                    availability,
                }),
                preview: None,
                is_fixture: false,
            });
        }
        Ok(results)
    }
}

pub struct FilesSearchAdapter<P, A> {
    service: Arc<Mutex<FilesService<P, A>>>,
    resolver: FilesObjectResolver,
    start: Location,
    provider: FilesSearchProvider,
}

impl<P, A> FilesSearchAdapter<P, A> {
    pub fn new(
        service: Arc<Mutex<FilesService<P, A>>>,
        resolver: FilesObjectResolver,
        start: Location,
    ) -> Self {
        Self {
            service,
            resolver,
            start,
            provider: FilesSearchProvider::default(),
        }
    }
}

impl<P, A> SearchProvider for FilesSearchAdapter<P, A>
where
    P: FilesystemProvider + Send,
    A: CapabilityAuthorizer + Send,
{
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: provider_id("files"),
            priority: 15,
            required_capability: Some(capability(FILES_SEARCH_CAPABILITY)),
        }
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let service = self
            .service
            .lock()
            .map_err(|_| ProviderError::Unavailable)?;
        let records = self
            .provider
            .search(&*service, &self.start, &query.raw_query, 64)
            .map_err(|error| match error.kind {
                nagi_files::FilesErrorKind::PermissionDenied
                | nagi_files::FilesErrorKind::PermissionRequired
                | nagi_files::FilesErrorKind::CapabilityUnavailable => {
                    ProviderError::PermissionDenied
                }
                nagi_files::FilesErrorKind::ProviderUnavailable => ProviderError::Unavailable,
                _ => ProviderError::Failed,
            })?;
        records
            .into_iter()
            .map(|record| self.candidate(query, record))
            .collect()
    }
}

impl<P, A> FilesSearchAdapter<P, A> {
    fn candidate(
        &self,
        query: &SearchQuery,
        record: FilesSearchRecord,
    ) -> Result<SearchCandidate, ProviderError> {
        let object_id = self
            .resolver
            .resolve(record.resource_id)
            .map_err(|_| ProviderError::Unavailable)?;
        let open_capability = capability(FILES_OPEN_CAPABILITY);
        let availability = if query.capabilities.allows(Some(&open_capability)) {
            ActionAvailability::HostPreviewOnly
        } else {
            ActionAvailability::PermissionRequired {
                capability: open_capability.clone(),
            }
        };
        Ok(SearchCandidate {
            identity: SearchIdentity::Object(object_id),
            category: SearchCategory::Files,
            text: SearchText {
                title: record.title,
                subtitle: Some(record.location.to_string()),
                content: None,
                tags: record.tags,
            },
            modified_at_unix_seconds: record
                .modified_at
                .and_then(|timestamp| u64::try_from(timestamp).ok()),
            workspace_ids: Vec::new(),
            visibility_capability: Some(capability(FILES_METADATA_CAPABILITY)),
            action: Some(SearchAction {
                action: TypedAction::OpenObject {
                    object_id,
                    app_id: FILES_APP_ID,
                },
                required_capability: Some(open_capability),
                availability,
            }),
            preview: None,
            is_fixture: false,
        })
    }
}

pub struct ActivitySearchAdapter {
    history: HostHistory,
}

impl ActivitySearchAdapter {
    pub fn new(history: HostHistory) -> Self {
        Self { history }
    }
}

impl SearchProvider for ActivitySearchAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: provider_id("activity"),
            priority: 10,
            required_capability: Some(capability(ACTIVITY_SEARCH_CAPABILITY)),
        }
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let state = self
            .history
            .state
            .lock()
            .map_err(|_| ProviderError::Unavailable)?;
        let results_capability = capability(ACTIVITY_READ_CAPABILITY);
        let action_availability = if query.capabilities.allows(Some(&results_capability)) {
            ActionAvailability::HostPreviewOnly
        } else {
            ActionAvailability::PermissionRequired {
                capability: results_capability.clone(),
            }
        };
        Ok(state
            .activity
            .query_visible(
                ActivityQuery::default(),
                PREVIEW_USER,
                &PreviewActivityPolicy,
            )
            .map(|event| {
                activity_candidate(
                    event,
                    query.locale,
                    &results_capability,
                    &action_availability,
                )
            })
            .collect())
    }
}

pub struct WaybackSearchAdapter {
    history: HostHistory,
}

impl WaybackSearchAdapter {
    pub fn new(history: HostHistory) -> Self {
        Self { history }
    }
}

impl SearchProvider for WaybackSearchAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: provider_id("wayback"),
            priority: 8,
            required_capability: Some(capability(WAYBACK_SEARCH_CAPABILITY)),
        }
    }

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
        if query.cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let state = self
            .history
            .state
            .lock()
            .map_err(|_| ProviderError::Unavailable)?;
        let read_capability = capability(WAYBACK_READ_CAPABILITY);
        let action_availability = if query.capabilities.allows(Some(&read_capability)) {
            ActionAvailability::HostPreviewOnly
        } else {
            ActionAvailability::PermissionRequired {
                capability: read_capability.clone(),
            }
        };
        Ok(state
            .checkpoints
            .query_visible(
                CheckpointQuery::default(),
                PREVIEW_USER,
                &PreviewCheckpointPolicy,
            )
            .map(|checkpoint| {
                let title = match query.locale {
                    Locale::EnUs => "Automatic history checkpoint",
                    Locale::JaJp => "自動履歴チェックポイント",
                };
                let object_ids = checkpoint
                    .objects()
                    .map(|object| object.object_id())
                    .collect::<Vec<_>>();
                SearchCandidate {
                    identity: SearchIdentity::Checkpoint(checkpoint.id()),
                    category: SearchCategory::Activity,
                    text: SearchText {
                        title: title.to_owned(),
                        subtitle: Some(format!("{}", checkpoint.id().0)),
                        content: None,
                        tags: vec!["wayback".to_owned(), "checkpoint".to_owned()],
                    },
                    modified_at_unix_seconds: u64::try_from(checkpoint.created_at().seconds()).ok(),
                    workspace_ids: checkpoint.context().workspace_id.into_iter().collect(),
                    visibility_capability: Some(read_capability.clone()),
                    action: Some(SearchAction {
                        action: TypedAction::OpenCheckpoint {
                            checkpoint_id: checkpoint.id(),
                        },
                        required_capability: Some(read_capability.clone()),
                        availability: action_availability.clone(),
                    }),
                    preview: (!object_ids.is_empty())
                        .then(|| format!("{} object revision(s)", object_ids.len())),
                    is_fixture: false,
                }
            })
            .collect())
    }
}

pub struct PreviewActivityPolicy;

impl ActivityAccessPolicy for PreviewActivityPolicy {
    fn can_read(&self, viewer: Actor, event: &ActivityEvent) -> bool {
        viewer == PREVIEW_USER && event.actor() == PREVIEW_USER
    }
}

pub struct PreviewCheckpointPolicy;

impl CheckpointAccessPolicy for PreviewCheckpointPolicy {
    fn can_read(
        &self,
        viewer: Actor,
        checkpoint: &nagi_history::wayback::CheckpointRecord,
    ) -> bool {
        viewer == PREVIEW_USER && checkpoint.actor() == PREVIEW_USER
    }
}

pub struct IntegratedHost {
    pub history: HostHistory,
    pub notes: Arc<NotesApp>,
    pub note_store: Arc<dyn NoteStore>,
    pub files: Arc<Mutex<FilesService<InMemoryProvider, CapabilitySet>>>,
    pub files_objects: FilesObjectResolver,
    pub home: HomeController,
}

impl IntegratedHost {
    pub fn new() -> Result<Self, String> {
        let object_ids = Arc::new(PreviewObjectIdAllocator::new());
        let store = Arc::new(nagi_notes::InMemoryNoteStore::new());
        let note_store: Arc<dyn NoteStore> = store.clone();
        let history = HostHistory::new(Arc::clone(&note_store));
        let notes_activity: Arc<dyn NotesActivitySink> =
            Arc::new(NotesActivityBridge(history.clone()));
        let notes = Arc::new(NotesApp::with_providers(
            Arc::clone(&note_store),
            object_ids.clone(),
            Arc::new(nagi_notes::SystemClock),
            notes_activity,
            Duration::from_millis(20),
        ));

        let files_objects = FilesObjectResolver::new(object_ids);
        let mut grants = Vec::new();
        for right in [
            CapabilityRight::Enumerate,
            CapabilityRight::Read,
            CapabilityRight::Create,
            CapabilityRight::Rename,
            CapabilityRight::Write,
            CapabilityRight::Move,
            CapabilityRight::Delete,
            CapabilityRight::Restore,
            CapabilityRight::PermanentDelete,
        ] {
            grants.push(CapabilityGrant::allow(Location::root(), right));
        }
        let file_capabilities = CapabilitySet::from_grants(grants);
        let files_activity = FilesActivityBridge::new(history.clone(), files_objects.clone());
        let files = Arc::new(Mutex::new(
            FilesService::new(InMemoryProvider::new(), file_capabilities)
                .with_activity_sink(files_activity)
                .with_checkpoint_hook(SharedFilesWaybackHook::new(
                    history.clone(),
                    files_objects.clone(),
                )),
        ));

        let notes_provider: Arc<dyn SearchProvider> = Arc::new(NotesSearchAdapter::new(
            Arc::clone(&notes),
            Arc::new(HostUserNotesPolicy),
        ));
        let files_provider: Arc<dyn SearchProvider> = Arc::new(FilesSearchAdapter::new(
            Arc::clone(&files),
            files_objects.clone(),
            Location::root(),
        ));
        let activity_provider: Arc<dyn SearchProvider> =
            Arc::new(ActivitySearchAdapter::new(history.clone()));
        let wayback_provider: Arc<dyn SearchProvider> =
            Arc::new(WaybackSearchAdapter::new(history.clone()));
        let providers = vec![
            notes_provider,
            files_provider,
            activity_provider,
            wayback_provider,
        ];
        let registry: Arc<dyn AppRegistry> = Arc::new(
            first_party_registry()
                .map_err(|error| format!("invalid first-party registry: {error:?}"))?,
        );
        let home = HomeController::new(
            registry,
            Arc::new(EmptyHomeSource),
            Arc::new(SearchCoordinator::new(Duration::from_millis(500), 100)),
            providers,
            false,
        );
        Ok(Self {
            history,
            notes,
            note_store,
            files,
            files_objects,
            home,
        })
    }

    pub fn search(
        &self,
        text: impl Into<String>,
        locale: Locale,
    ) -> Result<SearchResponse, String> {
        let query = SearchQuery::new(
            text,
            self.home.allocate_search_request_id(),
            SearchContext::default(),
            host_preview_capabilities(),
            locale,
        );
        self.home
            .search(query)
            .map_err(|error| format!("Search failed: {error:?}"))
    }

    pub fn home_apps(&self, locale: Locale) -> Result<Vec<String>, HomeError> {
        let snapshot = self.home.snapshot(locale, host_preview_capabilities())?;
        Ok(snapshot
            .apps
            .iter()
            .map(|app| {
                app.descriptor
                    .display_name(&nagi_home_search::LocalizationCatalog, locale)
            })
            .collect())
    }
}

struct EmptyHomeSource;

impl HomeDataSource for EmptyHomeSource {
    fn snapshot(&self, _capabilities: &CapabilityContext) -> Result<HomeData, HomeError> {
        Ok(HomeData::default())
    }
}

pub fn first_party_registry() -> Result<RegistrySnapshot, nagi_home_search::RegistryError> {
    let entries = [
        app_descriptor(
            nagi_notes::APP_ID,
            "app.notes",
            "notes",
            'N',
            [63, 117, 191],
            10,
            "#/notes",
        ),
        app_descriptor(
            FILES_APP_ID,
            "app.files",
            "files",
            'F',
            [48, 145, 109],
            20,
            "#/files",
        ),
    ];
    RegistrySnapshot::new(entries.to_vec())
}

fn app_descriptor(
    app_id: AppId,
    key: &str,
    icon: &str,
    glyph: char,
    accent_rgb: [u8; 3],
    launcher_order: u16,
    route: &str,
) -> AppDescriptor {
    AppDescriptor {
        app_id,
        localization_name_key: key.to_owned(),
        localization_description_key: format!("{key}.description"),
        manifest_name_fallback: None,
        manifest_description_fallback: None,
        icon: IconMetadata {
            asset_key: format!("icons/{icon}"),
            fallback_glyph: glyph,
            accent_rgb,
        },
        launcher_order,
        availability: AppAvailability::HostPreview,
        launch_capability: Some(capability("apps.launch")),
        preview_route: Some(route.to_owned()),
        launch_action: TypedAction::LaunchApp { app_id },
    }
}

pub fn host_preview_capabilities() -> CapabilityContext {
    CapabilityContext::from_visible_grants(
        [
            "apps.launch",
            NOTES_SEARCH_CAPABILITY,
            NOTES_METADATA_CAPABILITY,
            NOTES_OPEN_CAPABILITY,
            FILES_SEARCH_CAPABILITY,
            FILES_METADATA_CAPABILITY,
            FILES_OPEN_CAPABILITY,
            ACTIVITY_SEARCH_CAPABILITY,
            ACTIVITY_READ_CAPABILITY,
            WAYBACK_SEARCH_CAPABILITY,
            WAYBACK_READ_CAPABILITY,
        ]
        .into_iter()
        .map(capability),
    )
}

fn activity_candidate(
    event: &ActivityEvent,
    locale: Locale,
    read_capability: &CapabilityId,
    action_availability: &ActionAvailability,
) -> SearchCandidate {
    let mut buffer = [0; 128];
    let user_locale = match locale {
        Locale::EnUs => UserLocale::EnUs,
        Locale::JaJp => UserLocale::JaJp,
    };
    let length =
        nagi_history::activity::render_summary(event, user_locale, &mut buffer).unwrap_or_default();
    let title = String::from_utf8_lossy(&buffer[..length]).into_owned();
    let result = match (locale, event.result()) {
        (_, EventResult::Pending) => "pending",
        (_, EventResult::Succeeded) => "succeeded",
        (_, EventResult::Failed(_)) => "failed",
        (_, EventResult::Partial { .. }) => "partial",
    };
    let object_ids = event.targets().collect::<Vec<_>>();
    SearchCandidate {
        identity: SearchIdentity::Activity(event.id()),
        category: SearchCategory::Activity,
        text: SearchText {
            title,
            subtitle: Some(match locale {
                Locale::EnUs => result.to_owned(),
                Locale::JaJp => match result {
                    "pending" => "進行中".to_owned(),
                    "succeeded" => "完了".to_owned(),
                    "failed" => "失敗".to_owned(),
                    _ => "一部完了".to_owned(),
                },
            }),
            content: None,
            tags: Vec::new(),
        },
        modified_at_unix_seconds: u64::try_from(event.occurred_at().seconds()).ok(),
        workspace_ids: event.context().workspace_id.into_iter().collect(),
        visibility_capability: Some(read_capability.clone()),
        action: Some(SearchAction {
            action: TypedAction::OpenActivityEvent {
                event_id: event.id(),
            },
            required_capability: Some(read_capability.clone()),
            availability: action_availability.clone(),
        }),
        preview: (!object_ids.is_empty())
            .then(|| format!("{} object reference(s)", object_ids.len())),
        is_fixture: false,
    }
}

fn notes_action_kind(kind: NotesActivityKind) -> ActionKind {
    match kind {
        NotesActivityKind::Created => ActionKind::ObjectCreated,
        NotesActivityKind::Opened => ActionKind::ObjectAccessed,
        NotesActivityKind::Edited | NotesActivityKind::Saved | NotesActivityKind::Renamed => {
            ActionKind::ObjectChanged
        }
        NotesActivityKind::Deleted => ActionKind::ObjectDeleted,
        NotesActivityKind::Restored => ActionKind::ObjectRestored,
    }
}

fn files_action_kind(action_id: &str) -> ActionKind {
    match action_id {
        "files.create_folder" | "files.copy" | "files.duplicate" => ActionKind::ObjectCreated,
        "files.open" => ActionKind::ObjectAccessed,
        "files.rename" | "files.move" => ActionKind::ObjectMoved,
        "files.delete" | "files.delete_permanently" => ActionKind::ObjectDeleted,
        "files.restore" => ActionKind::ObjectRestored,
        "files.add_to_workspace" | "files.remove_from_workspace" | "files.set_tags" => {
            ActionKind::ObjectChanged
        }
        _ => ActionKind::Custom(1),
    }
}

fn preview_context(app_id: AppId, workspace_id: Option<WorkspaceId>) -> ActivityContext {
    ActivityContext {
        app_id,
        app_session_id: AppSessionId(1),
        node_id: NodeId(1),
        surface_id: Some(SurfaceId(1)),
        workspace_id,
    }
}

fn notes_timestamp(milliseconds: u64) -> Result<Timestamp, String> {
    let seconds = i64::try_from(milliseconds / 1_000)
        .map_err(|_| "Notes timestamp is outside the shared Activity range".to_owned())?;
    let nanos = ((milliseconds % 1_000) * 1_000_000) as u32;
    Timestamp::new(seconds, nanos)
        .map_err(|error| format!("invalid Notes Activity timestamp: {error:?}"))
}

fn system_timestamp() -> Result<Timestamp, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("host preview clock precedes Unix epoch: {error}"))?;
    let seconds = i64::try_from(now.as_secs())
        .map_err(|_| "host preview timestamp is outside the Activity range".to_owned())?;
    Timestamp::new(seconds, now.subsec_nanos())
        .map_err(|error| format!("invalid host preview timestamp: {error:?}"))
}

fn provider_id(value: &str) -> ProviderId {
    ProviderId::new(value).expect("static provider IDs are valid")
}

fn capability(value: &str) -> CapabilityId {
    CapabilityId::new(value).expect("static capability IDs are valid")
}

pub fn insert_preview_file(
    provider: &mut InMemoryProvider,
    path: &str,
    bytes: &[u8],
) -> Result<ResourceId, String> {
    let location =
        Location::parse(path).map_err(|error| format!("invalid preview file path: {error:?}"))?;
    provider
        .insert_file(&location, bytes)
        .map(|entry| entry.id)
        .map_err(|error| format!("could not seed in-memory Files provider: {error:?}"))
}

pub fn operation_completed(result: &OperationResult) -> bool {
    result.activity == nagi_files::HookStatus::Recorded
}

#[cfg(test)]
mod tests;
