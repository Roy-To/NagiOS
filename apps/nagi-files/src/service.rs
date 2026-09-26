use crate::{
    Actor, CancellationToken, CapabilityAuthorizer, CapabilityRequest, CapabilityRight,
    ConfirmationChallenge, EntryKind, FileEntry, FileName, FilesError, FilesErrorKind,
    FilesystemProvider, Location, OperationIntent, OperationKind, PermissionDecision,
    ProviderAvailability, ResourceId, Reversibility, TransactionId, TrashEntry,
};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CHECKPOINT_CAPTURE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityEvent {
    pub transaction_id: TransactionId,
    pub actor: Actor,
    pub app_id: &'static str,
    pub action_id: &'static str,
    pub source: Option<Location>,
    pub destination: Option<Location>,
    pub target_resources: Vec<ResourceId>,
    pub timestamp_epoch_seconds: i64,
    pub result: ActivityOutcome,
    pub reversible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityOutcome {
    Started,
    Succeeded,
    Failed(FilesErrorKind),
    Denied,
}

pub trait ActivitySink: Send {
    fn record(&mut self, event: &ActivityEvent) -> Result<(), String>;
}

pub trait CheckpointHook: Send {
    fn checkpoint_before(&mut self, request: &WaybackCheckpointRequest) -> Result<String, String>;

    /// Optional snapshot-aware path. Older hooks remain source-compatible and
    /// receive the original metadata-only request through the default method.
    fn checkpoint_before_with_snapshots(
        &mut self,
        request: &WaybackCheckpointRequest,
        _snapshots: &[CheckpointSnapshot],
    ) -> Result<String, String> {
        self.checkpoint_before(request)
    }
}

pub trait WorkspaceReferenceSink: Send {
    fn add_reference(&mut self, workspace_id: &str, resource_id: ResourceId) -> Result<(), String>;

    fn remove_reference(
        &mut self,
        _workspace_id: &str,
        _resource_id: ResourceId,
    ) -> Result<(), String> {
        Err("workspace remove operation unavailable".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReferenceResult {
    pub operation: OperationResult,
    pub reference: HookStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WaybackCheckpointRequest {
    pub transaction_id: TransactionId,
    pub actor: Actor,
    pub action_id: &'static str,
    pub source: Option<Location>,
    pub destination: Option<Location>,
    pub affected: Vec<ResourceId>,
    pub reversibility: Reversibility,
}

/// A read-authorized pre-operation file version passed only to a trusted
/// checkpoint hook. Contents are private checkpoint payload and must never be
/// copied to Activity records, Search text, or logs.
#[derive(Clone, Eq, PartialEq)]
pub struct CheckpointSnapshot {
    pub resource_id: ResourceId,
    pub location: Location,
    pub contents: Vec<u8>,
    pub modified_at_epoch_seconds: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HookStatus {
    Recorded,
    Created(String),
    NotApplicable,
    Unavailable,
    Failed(String),
}

pub struct NoopActivitySink;

impl ActivitySink for NoopActivitySink {
    fn record(&mut self, _event: &ActivityEvent) -> Result<(), String> {
        Err("activity service unavailable".into())
    }
}

pub struct NoopCheckpointHook;

impl CheckpointHook for NoopCheckpointHook {
    fn checkpoint_before(&mut self, _request: &WaybackCheckpointRequest) -> Result<String, String> {
        Err("checkpoint service unavailable".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationOutcome {
    Applied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationResult {
    pub transaction_id: TransactionId,
    pub action_id: &'static str,
    pub outcome: OperationOutcome,
    pub affected: Vec<ResourceId>,
    pub reversibility: Reversibility,
    pub activity: HookStatus,
    pub checkpoint: HookStatus,
}

#[derive(Clone)]
struct PendingConfirmation {
    transaction_id: TransactionId,
    target: ResourceId,
    trash_id: ResourceId,
}

pub struct FilesService<P, A> {
    provider: P,
    authorizer: A,
    activity: Option<Box<dyn ActivitySink>>,
    checkpoint: Option<Box<dyn CheckpointHook>>,
    workspace: Option<Box<dyn WorkspaceReferenceSink>>,
    pending_confirmations: HashMap<u64, PendingConfirmation>,
    next_confirmation: u64,
}

impl<P, A> FilesService<P, A>
where
    P: FilesystemProvider,
    A: CapabilityAuthorizer,
{
    pub fn new(provider: P, authorizer: A) -> Self {
        Self {
            provider,
            authorizer,
            activity: None,
            checkpoint: None,
            workspace: None,
            pending_confirmations: HashMap::new(),
            next_confirmation: 1,
        }
    }

    pub fn with_activity_sink(mut self, sink: impl ActivitySink + 'static) -> Self {
        self.activity = Some(Box::new(sink));
        self
    }

    pub fn with_checkpoint_hook(mut self, hook: impl CheckpointHook + 'static) -> Self {
        self.checkpoint = Some(Box::new(hook));
        self
    }

    pub fn with_workspace_sink(mut self, sink: impl WorkspaceReferenceSink + 'static) -> Self {
        self.workspace = Some(Box::new(sink));
        self
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    pub fn provider_availability(&self) -> ProviderAvailability {
        self.provider.availability()
    }

    pub fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        self.ensure_available()?;
        self.authorize(CapabilityRight::Enumerate, location)?;
        self.provider.list(location)
    }

    pub fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError> {
        self.ensure_available()?;
        self.authorize(CapabilityRight::Read, location)?;
        self.provider.metadata(location)
    }

    pub fn read_file(&self, location: &Location) -> Result<Vec<u8>, FilesError> {
        self.ensure_available()?;
        self.authorize(CapabilityRight::Read, location)?;
        let contents = self
            .provider
            .read_file(location, crate::MAX_PREVIEW_BYTES)?;
        bounded_preview(contents, location)
    }

    pub fn open_file(
        &mut self,
        source: &FileEntry,
        actor: Actor,
    ) -> Result<(Vec<u8>, OperationResult), FilesError> {
        self.ensure_available()?;
        if source.kind != EntryKind::File {
            return Err(
                FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.child_location())
            );
        }
        let location = source.child_location();
        self.authorize(CapabilityRight::Read, &location)?;
        self.provider.verify_resource(source.id, &location)?;
        let mut intent = OperationIntent::new(OperationKind::Open, actor);
        intent.source = Some(location.clone());
        intent.affected.push(source.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::SourceExists);
        self.begin_agent_operation(&intent)?;
        match self.provider.read_file(&location, crate::MAX_PREVIEW_BYTES) {
            Ok(contents) if contents.len() <= crate::MAX_PREVIEW_BYTES => {
                let result = self.success(&intent, vec![source.id], HookStatus::NotApplicable);
                Ok((contents, result))
            }
            Ok(_) => {
                let error = FilesError::new(FilesErrorKind::FileTooLarge).at(location);
                self.failure(&intent, &error);
                Err(error)
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    /// Adds a resource reference to a workspace. The provider item stays at its
    /// original path; this API never implements workspace membership as move.
    pub fn add_to_workspace(
        &mut self,
        source: &FileEntry,
        workspace_id: &str,
        actor: Actor,
    ) -> Result<WorkspaceReferenceResult, FilesError> {
        self.ensure_available()?;
        if workspace_id.trim().is_empty() || workspace_id.len() > 128 {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        let location = source.child_location();
        self.authorize(CapabilityRight::Read, &location)?;
        self.authorize(CapabilityRight::WorkspaceReference, &location)?;
        self.provider.verify_resource(source.id, &location)?;
        let mut intent = OperationIntent::new(OperationKind::AddToWorkspace, actor);
        intent.source = Some(location);
        intent.affected.push(source.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::SourceExists);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        let Some(workspace) = self.workspace.as_mut() else {
            self.failure(
                &intent,
                &FilesError::new(FilesErrorKind::WorkspaceUnavailable),
            );
            return Err(FilesError::new(FilesErrorKind::WorkspaceUnavailable));
        };
        if workspace.add_reference(workspace_id, source.id).is_err() {
            let error = FilesError::new(FilesErrorKind::ProviderFailure);
            self.failure(&intent, &error);
            return Err(error);
        }
        let operation = self.success(&intent, vec![source.id], checkpoint);
        Ok(WorkspaceReferenceResult {
            operation,
            reference: HookStatus::Recorded,
        })
    }

    pub fn remove_from_workspace(
        &mut self,
        source: &FileEntry,
        workspace_id: &str,
        actor: Actor,
    ) -> Result<WorkspaceReferenceResult, FilesError> {
        self.ensure_available()?;
        if workspace_id.trim().is_empty() || workspace_id.len() > 128 {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        let location = source.child_location();
        self.authorize(CapabilityRight::Read, &location)?;
        self.authorize(CapabilityRight::WorkspaceReference, &location)?;
        self.provider.verify_resource(source.id, &location)?;
        let mut intent = OperationIntent::new(OperationKind::RemoveFromWorkspace, actor);
        intent.source = Some(location);
        intent.affected.push(source.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::SourceExists);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        let Some(workspace) = self.workspace.as_mut() else {
            self.failure(
                &intent,
                &FilesError::new(FilesErrorKind::WorkspaceUnavailable),
            );
            return Err(FilesError::new(FilesErrorKind::WorkspaceUnavailable));
        };
        if workspace.remove_reference(workspace_id, source.id).is_err() {
            let error = FilesError::new(FilesErrorKind::WorkspaceUnavailable);
            self.failure(&intent, &error);
            return Err(error);
        }
        let operation = self.success(&intent, vec![source.id], checkpoint);
        Ok(WorkspaceReferenceResult {
            operation,
            reference: HookStatus::Recorded,
        })
    }

    pub fn set_tags(
        &mut self,
        source: &FileEntry,
        tags: &[crate::Tag],
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        if tags.len() > 32 {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        let location = source.child_location();
        self.authorize(CapabilityRight::SetMetadata, &location)?;
        self.provider.verify_resource(source.id, &location)?;
        let mut normalized = tags
            .iter()
            .map(|tag| tag.as_str().to_owned())
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        let mut intent = OperationIntent::new(OperationKind::SetTags, actor);
        intent.source = Some(location.clone());
        intent.affected.push(source.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::SourceExists);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self.provider.set_tags(&location, &normalized) {
            Ok(entry) => {
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        self.ensure_available()?;
        let mut visible = Vec::new();
        for item in self.provider.list_trash()? {
            if self
                .authorize(CapabilityRight::Enumerate, &item.original_location)
                .is_ok()
            {
                visible.push(item);
            }
        }
        Ok(visible)
    }

    pub fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        self.authorize(CapabilityRight::Create, parent)?;
        let destination = parent.join(name);
        let mut intent = OperationIntent::new(OperationKind::CreateFolder, actor);
        intent.destination = Some(destination.clone());
        intent
            .preconditions
            .push(crate::OperationPrecondition::DestinationAbsent);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self.provider.create_folder(parent, name) {
            Ok(entry) => {
                intent.affected.push(entry.id);
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn rename(
        &mut self,
        source: &FileEntry,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        let source_location = source.child_location();
        self.authorize(CapabilityRight::Rename, &source_location)?;
        self.provider.verify_resource(source.id, &source_location)?;
        let destination = source.location.join(name);
        let mut intent = OperationIntent::new(OperationKind::Rename, actor);
        intent.source = Some(source_location.clone());
        intent.destination = Some(destination);
        intent.affected.push(source.id);
        intent.preconditions.extend([
            crate::OperationPrecondition::SourceExists,
            crate::OperationPrecondition::DestinationAbsent,
        ]);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self.provider.rename(&source_location, name) {
            Ok(entry) => {
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn copy(
        &mut self,
        source: &FileEntry,
        destination: &Location,
        name: &FileName,
        actor: Actor,
        cancellation: &CancellationToken,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.copy_with_kind(
            source,
            destination,
            name,
            OperationKind::Copy,
            actor,
            cancellation,
        )
    }

    fn copy_with_kind(
        &mut self,
        source: &FileEntry,
        destination: &Location,
        name: &FileName,
        kind: OperationKind,
        actor: Actor,
        cancellation: &CancellationToken,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        let source_location = source.child_location();
        self.authorize(CapabilityRight::Read, &source_location)?;
        self.provider.verify_resource(source.id, &source_location)?;
        self.authorize(CapabilityRight::Create, destination)?;
        let target = destination.join(name);
        let mut intent = OperationIntent::new(kind, actor);
        intent.source = Some(source_location.clone());
        intent.destination = Some(target);
        intent.affected.push(source.id);
        intent.preconditions.extend([
            crate::OperationPrecondition::SourceExists,
            crate::OperationPrecondition::DestinationAbsent,
        ]);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self
            .provider
            .copy(&source_location, destination, name, cancellation)
        {
            Ok(entry) => {
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn move_item(
        &mut self,
        source: &FileEntry,
        destination: &Location,
        name: &FileName,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        let source_location = source.child_location();
        self.authorize(CapabilityRight::Move, &source_location)?;
        self.provider.verify_resource(source.id, &source_location)?;
        self.authorize(CapabilityRight::Create, destination)?;
        let target = destination.join(name);
        let mut intent = OperationIntent::new(OperationKind::Move, actor);
        intent.source = Some(source_location.clone());
        intent.destination = Some(target);
        intent.affected.push(source.id);
        intent.preconditions.extend([
            crate::OperationPrecondition::SourceExists,
            crate::OperationPrecondition::DestinationAbsent,
        ]);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self.provider.move_item(&source_location, destination, name) {
            Ok(entry) => {
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn duplicate(
        &mut self,
        source: &FileEntry,
        actor: Actor,
        cancellation: &CancellationToken,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        let parent = source.location.clone();
        let entries = self.list(&parent)?;
        let duplicate_name = available_copy_name(&source.name, &entries)?;
        self.copy_with_kind(
            source,
            &parent,
            &duplicate_name,
            OperationKind::Duplicate,
            actor,
            cancellation,
        )
    }

    pub fn trash(
        &mut self,
        source: &FileEntry,
        actor: Actor,
    ) -> Result<(TrashEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        let source_location = source.child_location();
        if source.kind == EntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source_location));
        }
        self.authorize(CapabilityRight::Delete, &source_location)?;
        self.provider.verify_resource(source.id, &source_location)?;
        let mut intent = OperationIntent::new(OperationKind::Trash, actor);
        intent.source = Some(source_location);
        intent.affected.push(source.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::SourceExists);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self
            .provider
            .trash(intent.source.as_ref().expect("set above"))
        {
            Ok(trash_entry) => {
                let result = self.success(&intent, vec![trash_entry.id], checkpoint);
                Ok((trash_entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn restore(
        &mut self,
        trash_id: ResourceId,
        actor: Actor,
    ) -> Result<(FileEntry, OperationResult), FilesError> {
        self.ensure_available()?;
        let item = self
            .provider
            .list_trash()?
            .into_iter()
            .find(|entry| entry.id == trash_id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        self.authorize(CapabilityRight::Restore, &item.original_location)?;
        let mut intent = OperationIntent::new(OperationKind::Restore, actor);
        intent.source = Some(Location::parse(".nagi-files/trash")?);
        intent.destination = Some(item.original_location.clone());
        intent.affected.push(trash_id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::DestinationAbsent);
        self.begin_agent_operation(&intent)?;
        let checkpoint = self.checkpoint_before(&intent);
        match self.provider.restore(trash_id) {
            Ok(entry) => {
                let result = self.success(&intent, vec![entry.id], checkpoint);
                Ok((entry, result))
            }
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    pub fn request_permanent_delete(
        &mut self,
        trash_id: ResourceId,
    ) -> Result<ConfirmationChallenge, FilesError> {
        self.ensure_available()?;
        let item = self
            .provider
            .list_trash()?
            .into_iter()
            .find(|entry| entry.id == trash_id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        self.authorize(CapabilityRight::PermanentDelete, &item.original_location)?;
        let transaction_id = TransactionId::new();
        let nonce = self.next_confirmation;
        self.next_confirmation = self.next_confirmation.wrapping_add(1).max(1);
        self.pending_confirmations.insert(
            nonce,
            PendingConfirmation {
                transaction_id,
                target: item.id,
                trash_id,
            },
        );
        Ok(ConfirmationChallenge::new(
            nonce,
            transaction_id,
            item.id,
            item.original_location
                .file_name()
                .unwrap_or("item")
                .to_owned(),
        ))
    }

    pub fn confirm_permanent_delete(
        &mut self,
        challenge: ConfirmationChallenge,
        actor: Actor,
    ) -> Result<OperationResult, FilesError> {
        self.ensure_available()?;
        if actor != Actor::User {
            return Err(FilesError::new(FilesErrorKind::PermissionRequired));
        }
        let (nonce, transaction_id, target) = challenge.binding();
        let pending = self
            .pending_confirmations
            .remove(&nonce)
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidConfirmation))?;
        if pending.transaction_id != transaction_id || pending.target != target {
            return Err(FilesError::new(FilesErrorKind::InvalidConfirmation));
        }
        let item = self
            .provider
            .list_trash()?
            .into_iter()
            .find(|entry| entry.id == pending.trash_id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        self.authorize(CapabilityRight::PermanentDelete, &item.original_location)?;
        let mut intent = OperationIntent::new(OperationKind::PermanentDelete, actor);
        intent.transaction_id = transaction_id;
        intent.source = Some(item.original_location.clone());
        intent.affected.push(item.id);
        intent
            .preconditions
            .push(crate::OperationPrecondition::UserConfirmed);
        self.begin_agent_operation(&intent)?;
        let checkpoint = HookStatus::NotApplicable;
        match self.provider.permanently_delete(pending.trash_id) {
            Ok(()) => Ok(self.success(&intent, vec![item.id], checkpoint)),
            Err(error) => {
                self.failure(&intent, &error);
                Err(error)
            }
        }
    }

    fn ensure_available(&self) -> Result<(), FilesError> {
        match self.provider.availability() {
            ProviderAvailability::Available => Ok(()),
            ProviderAvailability::Unavailable(_) => {
                Err(FilesError::new(FilesErrorKind::ProviderUnavailable))
            }
        }
    }

    fn authorize(&self, right: CapabilityRight, location: &Location) -> Result<(), FilesError> {
        let request = CapabilityRequest {
            right,
            location: location.clone(),
        };
        match self.authorizer.decide(&request) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask => {
                Err(FilesError::new(FilesErrorKind::PermissionRequired).at(location.clone()))
            }
            PermissionDecision::Deny => {
                Err(FilesError::new(FilesErrorKind::PermissionDenied).at(location.clone()))
            }
            PermissionDecision::Unavailable => {
                Err(FilesError::new(FilesErrorKind::CapabilityUnavailable).at(location.clone()))
            }
        }
    }

    fn checkpoint_before(&mut self, intent: &OperationIntent) -> HookStatus {
        if intent.reversibility != Reversibility::Reversible {
            return HookStatus::NotApplicable;
        }
        if self.checkpoint.is_none() {
            return HookStatus::Unavailable;
        }
        let request = WaybackCheckpointRequest {
            transaction_id: intent.transaction_id,
            actor: intent.actor,
            action_id: intent.kind.action_id(),
            source: intent.source.clone(),
            destination: intent.destination.clone(),
            affected: intent.affected.clone(),
            reversibility: intent.reversibility,
        };
        let snapshots = match self.capture_checkpoint_snapshots(&request) {
            Ok(snapshots) => snapshots,
            Err(error) => return HookStatus::Failed(error),
        };
        let Some(hook) = self.checkpoint.as_mut() else {
            return HookStatus::Unavailable;
        };
        match hook.checkpoint_before_with_snapshots(&request, &snapshots) {
            Ok(id) => HookStatus::Created(id),
            Err(error) => HookStatus::Failed(error),
        }
    }

    fn capture_checkpoint_snapshots(
        &self,
        request: &WaybackCheckpointRequest,
    ) -> Result<Vec<CheckpointSnapshot>, String> {
        if request.affected.is_empty() {
            return Ok(Vec::new());
        }
        if request.affected.len() != 1 {
            return Err("Files host checkpoint capture supports one affected resource".to_owned());
        }
        let location = request
            .source
            .as_ref()
            .ok_or_else(|| "Files checkpoint has no pre-operation source location".to_owned())?;
        self.authorize(CapabilityRight::Read, location)
            .map_err(|error| format!("Files checkpoint read permission unavailable: {error:?}"))?;
        let entry = self
            .provider
            .verify_resource(request.affected[0], location)
            .map_err(|error| format!("Files checkpoint source verification failed: {error:?}"))?;
        if entry.kind != EntryKind::File {
            return Err(
                "Files host checkpoint capture currently supports regular files only".into(),
            );
        }
        let contents = self
            .provider
            .read_file(location, MAX_CHECKPOINT_CAPTURE_BYTES)
            .map_err(|error| format!("Files checkpoint source read failed: {error:?}"))?;
        if contents.len() > MAX_CHECKPOINT_CAPTURE_BYTES {
            return Err("Files checkpoint source exceeds the 16 MiB host capture limit".into());
        }
        Ok(vec![CheckpointSnapshot {
            resource_id: entry.id,
            location: location.clone(),
            contents,
            modified_at_epoch_seconds: entry.modified_at,
        }])
    }

    fn begin_agent_operation(&mut self, intent: &OperationIntent) -> Result<(), FilesError> {
        if intent.actor != Actor::Agent {
            return Ok(());
        }
        match self.record_activity(intent, ActivityOutcome::Started) {
            HookStatus::Recorded => Ok(()),
            HookStatus::Unavailable => Err(FilesError::new(FilesErrorKind::ActivityUnavailable)),
            HookStatus::Failed(_) => Err(FilesError::new(FilesErrorKind::ActivityFailure)),
            _ => Err(FilesError::new(FilesErrorKind::ActivityFailure)),
        }
    }

    fn success(
        &mut self,
        intent: &OperationIntent,
        affected: Vec<ResourceId>,
        checkpoint: HookStatus,
    ) -> OperationResult {
        let activity = self.record_activity(intent, ActivityOutcome::Succeeded);
        OperationResult {
            transaction_id: intent.transaction_id,
            action_id: intent.kind.action_id(),
            outcome: OperationOutcome::Applied,
            affected,
            reversibility: intent.reversibility,
            activity,
            checkpoint,
        }
    }

    fn failure(&mut self, intent: &OperationIntent, error: &FilesError) {
        let _ = self.record_activity(intent, ActivityOutcome::Failed(error.kind));
    }

    fn record_activity(
        &mut self,
        intent: &OperationIntent,
        outcome: ActivityOutcome,
    ) -> HookStatus {
        let Some(sink) = self.activity.as_mut() else {
            return HookStatus::Unavailable;
        };
        let event = ActivityEvent {
            transaction_id: intent.transaction_id,
            actor: intent.actor,
            app_id: "com.nagi.files",
            action_id: intent.kind.action_id(),
            source: intent.source.clone(),
            destination: intent.destination.clone(),
            target_resources: intent.affected.clone(),
            timestamp_epoch_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs() as i64),
            result: outcome,
            reversible: intent.reversibility == Reversibility::Reversible,
        };
        match sink.record(&event) {
            Ok(()) => HookStatus::Recorded,
            Err(error) => HookStatus::Failed(error),
        }
    }
}

fn bounded_preview(contents: Vec<u8>, location: &Location) -> Result<Vec<u8>, FilesError> {
    if contents.len() > crate::MAX_PREVIEW_BYTES {
        return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
    }
    Ok(contents)
}

fn available_copy_name(original: &FileName, entries: &[FileEntry]) -> Result<FileName, FilesError> {
    let stem = original.as_str();
    let (base, extension) = match stem.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() => (base, Some(extension)),
        _ => (stem, None),
    };
    let make_name = |suffix: &str| match extension {
        Some(extension) => FileName::parse(&format!("{base} {suffix}.{extension}")),
        None => FileName::parse(&format!("{base} {suffix}")),
    };
    for index in 1..=1000 {
        let suffix = if index == 1 {
            "copy".to_owned()
        } else {
            format!("copy {index}")
        };
        let name = make_name(&suffix)?;
        if !entries.iter().any(|entry| entry.name == name) {
            return Ok(name);
        }
    }
    Err(FilesError::new(FilesErrorKind::Conflict))
}
