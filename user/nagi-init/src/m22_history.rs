use crate::m19_search::M19SearchActivity;
use alloc::{rc::Rc, string::String, vec, vec::Vec};
use core::cell::Cell;
use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_FILE_SIZE};
use nagi_ai::{
    execute_plan, validate_plan, ActionDescriptor, ActionHandler, ActionInvocation, ActionOutput,
    ActionPolicy, ActionRegistry, CallerIdentity, ContextAuthority, ContextRequest,
    ContextResolver, ExecutionError, ExecutionStatus, HandlerError, NagiPlan, ObjectAccess,
    ParameterKind, ParameterRule, PlanParseError, PolicyDenied, ResolvedContext, ValidationError,
};
use nagi_history::activity_ledger::{
    ActivityLedger, ActivityLedgerArchiveBackend, ActivityLedgerError, ActivityLedgerFileStore,
    ActivityOutcome, ActivityRecord, ActivityRecordInput, MAX_ACTIVITY_ARCHIVE_BYTES,
};
use nagi_history::guest::{
    ArchiveSlot, HistoryArchiveBackend, HistoryArchiveFileStore, HistoryArchiveStore,
    GUEST_ARCHIVE_FILE_BYTES, MAX_GUEST_ARCHIVE_BYTES,
};
use nagi_history::{
    ActivityContext, AppId, AppSessionId, HistoryError, HistoryService, MoveRecord, NodeId,
    ObjectId, Operation, SurfaceId, TransactionState, UndoOperation, WorkspaceId,
};
use nagi_model_manager::CapabilityId;

type GuestVolume = Vfs<SyscallBlockDevice>;

const CALLER: ActivityContext = ActivityContext {
    app_id: AppId(0x4d22),
    app_session_id: AppSessionId(0x2201),
    node_id: NodeId(0x2202),
    surface_id: Some(SurfaceId(0x2203)),
    workspace_id: Some(WorkspaceId(0x2204)),
};

type M22MoveFixture = (ObjectId, &'static [u8], &'static [u8], &'static [u8]);

const MOVES: [M22MoveFixture; 3] = [
    (ObjectId(0x2211), b"m22-a", b"m22-A", b"M22 fixture one"),
    (ObjectId(0x2212), b"m22-b", b"m22-B", b"M22 fixture two"),
    (ObjectId(0x2213), b"m22-c", b"m22-C", b"M22 fixture three"),
];

const M22_MOVE_ACTION: &str = "file.move";
const M22_MOVE_CAPABILITY: &str = "files.move";
const M22_MOVE_INTENT: &str = "move the three M22 fixture files";
const M22_MOVE_PLAN_SUMMARY: &str = "destinations=m22-A,m22-B,m22-C";
const DESTINATION_PARAMETERS: [&str; 3] = ["destination_a", "destination_b", "destination_c"];
const M22_COPY_OBJECT_ID: ObjectId = ObjectId(0x2221);
const M22_COPY_SOURCE_OBJECT_ID: ObjectId = ObjectId(0x2211);
const M22_COPY_DESTINATION: &[u8] = b"m22-copy";
const M22_COPY_ACTION: &str = "file.copy";
const M22_COPY_CAPABILITY: &str = "files.copy";
const M22_COPY_INTENT: &str = "copy the first M22 fixture file";
const M22_COPY_PLAN_SUMMARY: &str = "source=8721;destination=m22-copy";
const M22_COPY_MAX_BYTES: usize = 512;
const M19_SEARCH_ACTION: &str = "file.search";
const M22_MOVE_OBJECT_IDS: [ObjectId; 3] = [ObjectId(0x2211), ObjectId(0x2212), ObjectId(0x2213)];
const M22_COPY_OBJECT_IDS: [ObjectId; 1] = [M22_COPY_SOURCE_OBJECT_ID];

#[derive(Clone, Copy)]
struct M22FixtureGrant {
    caller: CallerIdentity,
    capability: &'static str,
}

#[derive(Clone, Copy)]
struct M22FileHandle {
    object_id: ObjectId,
    source_name: &'static [u8],
    destination_name: &'static [u8],
    contents: &'static [u8],
}

/// Authority for the private M22 acceptance fixture only. Production caller
/// identity and capabilities must come from the authenticated service
/// boundary, which is not available to guest applications yet.
#[derive(Clone)]
struct M22FixturePolicy {
    deny_modifications: bool,
    deny_copy: bool,
    copy_created: Rc<Cell<bool>>,
}

impl Default for M22FixturePolicy {
    fn default() -> Self {
        Self {
            deny_modifications: false,
            deny_copy: false,
            copy_created: Rc::new(Cell::new(false)),
        }
    }
}

impl M22FixturePolicy {
    fn caller_is_fixture(caller: CallerIdentity) -> bool {
        caller.app_id == CALLER.app_id
            && caller.app_session_id == CALLER.app_session_id
            && caller.node_id == CALLER.node_id
            && caller.workspace_id == CALLER.workspace_id
    }

    fn file_handle(object_id: ObjectId) -> Option<M22FileHandle> {
        MOVES
            .iter()
            .find(|(fixture_id, _, _, _)| *fixture_id == object_id)
            .map(
                |(object_id, source_name, destination_name, contents)| M22FileHandle {
                    object_id: *object_id,
                    source_name,
                    destination_name,
                    contents,
                },
            )
    }
}

impl ContextAuthority for M22FixturePolicy {
    fn can_read_object(&self, caller: CallerIdentity, object_id: ObjectId) -> bool {
        Self::caller_is_fixture(caller) && Self::file_handle(object_id).is_some()
    }

    fn can_read_workspace(&self, caller: CallerIdentity, workspace_id: WorkspaceId) -> bool {
        Self::caller_is_fixture(caller) && Some(workspace_id) == CALLER.workspace_id
    }
}

impl ActionPolicy for M22FixturePolicy {
    type CapabilityGrant = M22FixtureGrant;
    type ObjectHandle = M22FileHandle;

    fn check_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<(), PolicyDenied> {
        let permitted = capability.as_str() == M22_MOVE_CAPABILITY
            || (capability.as_str() == M22_COPY_CAPABILITY && !self.deny_copy);
        if Self::caller_is_fixture(caller) && permitted {
            Ok(())
        } else {
            Err(PolicyDenied::Capability)
        }
    }

    fn check_object_access(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<(), PolicyDenied> {
        let allowed_object = (matches!(access, ObjectAccess::Read | ObjectAccess::Modify)
            && Self::file_handle(object_id).is_some())
            || (access == ObjectAccess::Read
                && object_id == M22_COPY_OBJECT_ID
                && self.copy_created.get());
        if Self::caller_is_fixture(caller)
            && allowed_object
            && !(self.deny_modifications && access == ObjectAccess::Modify)
        {
            Ok(())
        } else {
            Err(PolicyDenied::Object)
        }
    }

    fn acquire_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<Self::CapabilityGrant, PolicyDenied> {
        self.check_capability(caller, capability)?;
        let capability = match capability.as_str() {
            M22_MOVE_CAPABILITY => M22_MOVE_CAPABILITY,
            M22_COPY_CAPABILITY => M22_COPY_CAPABILITY,
            _ => return Err(PolicyDenied::Capability),
        };
        Ok(M22FixtureGrant { caller, capability })
    }

    fn resolve_object(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<Self::ObjectHandle, PolicyDenied> {
        self.check_object_access(caller, object_id, access)?;
        Self::file_handle(object_id).ok_or(PolicyDenied::Object)
    }
}

struct M22MoveAction {
    backend: HistoryArchiveBackend<M22Files>,
    history: HistoryService,
    activity_ledger: ActivityLedger,
    search_activity: M19SearchActivity,
}

struct M22CopyAction {
    backend: HistoryArchiveBackend<M22Files>,
    history: HistoryService,
    activity_ledger: ActivityLedger,
    copy_created: Rc<Cell<bool>>,
}

/// Test-only action handler for the M21 guest orchestration checks. It can
/// succeed or fail deterministically and never performs product side effects.
struct M22ValidationProbeAction {
    execution_count: Rc<Cell<usize>>,
    succeed: bool,
}

impl ActionHandler<M22FixturePolicy> for M22ValidationProbeAction {
    fn execute(
        &mut self,
        _invocation: ActionInvocation<'_, M22FixturePolicy>,
    ) -> Result<ActionOutput, HandlerError> {
        self.execution_count
            .set(self.execution_count.get().saturating_add(1));
        if self.succeed {
            Ok(ActionOutput {
                summary: String::from("M21 orchestration probe completed"),
                object_ids: vec![],
            })
        } else {
            Err(HandlerError::Unavailable)
        }
    }
}

struct M22Files {
    volume: GuestVolume,
}

impl M22Files {
    fn path(slot: ArchiveSlot) -> &'static [u8] {
        match slot {
            ArchiveSlot::A => b"/m22-archive-a",
            ArchiveSlot::B => b"/m22-archive-b",
        }
    }

    fn activity_path(slot: ArchiveSlot) -> &'static [u8] {
        match slot {
            ArchiveSlot::A => b"/m22-ledger-a",
            ArchiveSlot::B => b"/m22-ledger-b",
        }
    }
}

impl HistoryArchiveFileStore for M22Files {
    fn read_file(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, HistoryError> {
        let handle = match self.volume.open_path(Self::path(slot)) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => return Ok(None),
            Err(_) => return Err(HistoryError::Storage),
        };
        self.volume
            .read(handle, buffer)
            .map(Some)
            .map_err(|_| HistoryError::Storage)
    }

    fn write_file(&mut self, slot: ArchiveSlot, bytes: &[u8]) -> Result<(), HistoryError> {
        if bytes.len() > MAX_FILE_SIZE {
            return Err(HistoryError::Capacity);
        }
        let path = Self::path(slot);
        let handle = match self.volume.open_path(path) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => self
                .volume
                .create_path(path)
                .map_err(|_| HistoryError::Storage)?,
            Err(_) => return Err(HistoryError::Storage),
        };
        self.volume
            .write(handle, bytes)
            .map_err(|_| HistoryError::Storage)
    }

    fn flush(&mut self) -> Result<(), HistoryError> {
        self.volume.flush().map_err(|_| HistoryError::Storage)
    }
}

impl ActivityLedgerFileStore for M22Files {
    fn read_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, ActivityLedgerError> {
        let handle = match self.volume.open_path(Self::activity_path(slot)) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => return Ok(None),
            Err(_) => return Err(ActivityLedgerError::Storage),
        };
        self.volume
            .read(handle, buffer)
            .map(Some)
            .map_err(|_| ActivityLedgerError::Storage)
    }

    fn write_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        bytes: &[u8],
    ) -> Result<(), ActivityLedgerError> {
        if bytes.len() > MAX_FILE_SIZE {
            return Err(ActivityLedgerError::Capacity);
        }
        let path = Self::activity_path(slot);
        let handle = match self.volume.open_path(path) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => self
                .volume
                .create_path(path)
                .map_err(|_| ActivityLedgerError::Storage)?,
            Err(_) => return Err(ActivityLedgerError::Storage),
        };
        self.volume
            .write(handle, bytes)
            .map_err(|_| ActivityLedgerError::Storage)
    }

    fn flush_activity_slots(&mut self) -> Result<(), ActivityLedgerError> {
        self.volume
            .flush()
            .map_err(|_| ActivityLedgerError::Storage)
    }
}

impl ActionHandler<M22FixturePolicy> for M22MoveAction {
    fn execute(
        &mut self,
        invocation: ActionInvocation<'_, M22FixturePolicy>,
    ) -> Result<ActionOutput, HandlerError> {
        let caller = invocation.caller();
        if !M22FixturePolicy::caller_is_fixture(caller)
            || invocation.plan_intent() != M22_MOVE_INTENT
            || invocation.action().action_id() != M22_MOVE_ACTION
            || invocation.action().object_access() != ObjectAccess::Modify
            || invocation.action().required_capabilities().len() != 1
            || invocation.action().required_capabilities()[0].as_str() != M22_MOVE_CAPABILITY
            || invocation.capability_grants().len() != 1
            || invocation.capability_grants()[0].caller != caller
            || invocation.capability_grants()[0].capability != M22_MOVE_CAPABILITY
            || invocation.object_ids().len() != MOVES.len()
            || invocation.object_handles().len() != MOVES.len()
            || invocation.parameters().len() != DESTINATION_PARAMETERS.len()
        {
            return Err(HandlerError::Failed);
        }

        for (index, (object_id, source_name, destination_name, contents)) in
            MOVES.iter().enumerate()
        {
            let Some(handle) = invocation.object_handles().get(index) else {
                return Err(HandlerError::Failed);
            };
            let Some(parameter_name) = DESTINATION_PARAMETERS.get(index) else {
                return Err(HandlerError::Failed);
            };
            let Some(requested_destination) = invocation
                .parameters()
                .get(*parameter_name)
                .and_then(|value| value.as_str())
            else {
                return Err(HandlerError::Failed);
            };
            if invocation.object_ids()[index] != *object_id
                || handle.object_id != *object_id
                || handle.source_name != *source_name
                || handle.destination_name != *destination_name
                || handle.contents != *contents
                || requested_destination.as_bytes() != handle.destination_name
                || !valid_fixture_basename(requested_destination.as_bytes())
            {
                return Err(HandlerError::Failed);
            }
        }

        {
            let volume = &mut self.backend.file_store_mut().volume;
            for handle in invocation.object_handles() {
                if !named_file_matches(volume, handle.source_name, handle.contents)
                    || !file_is_missing(volume, handle.destination_name)
                {
                    return Err(HandlerError::Failed);
                }
            }
        }

        let moves: [MoveRecord<'_>; 3] = core::array::from_fn(|index| {
            let handle = invocation.object_handles()[index];
            MoveRecord {
                object_id: handle.object_id,
                from_name: handle.source_name,
                to_name: handle.destination_name,
            }
        });
        let transaction_id = self
            .history
            .record_move_group(activity_context(caller), &moves)
            .map_err(|_| HandlerError::Failed)?;
        if !save_history(&mut self.backend, &self.history) {
            return Err(HandlerError::Failed);
        }

        if ensure_search_activity_record(&mut self.activity_ledger, self.search_activity).is_none()
        {
            return Err(HandlerError::Failed);
        }

        let activity_sequence = self
            .activity_ledger
            .record_action(ActivityRecordInput {
                occurred_at: libnagi::time_ticks(),
                context: activity_context(caller),
                transaction_id: Some(transaction_id),
                user_intent: invocation.plan_intent(),
                selected_model: None,
                action_id: M22_MOVE_ACTION,
                plan_summary: M22_MOVE_PLAN_SUMMARY,
                object_ids: invocation.object_ids(),
            })
            .map_err(|_| HandlerError::Failed)?;
        if !save_activity_ledger(self.backend.file_store_mut(), &self.activity_ledger) {
            return Err(HandlerError::Failed);
        }

        {
            let volume = &mut self.backend.file_store_mut().volume;
            for handle in invocation.object_handles() {
                if !apply_idempotent_move(
                    volume,
                    handle.source_name,
                    handle.destination_name,
                    handle.contents,
                ) {
                    return Err(HandlerError::Failed);
                }
            }
            if volume.flush().is_err() {
                return Err(HandlerError::Failed);
            }
        }

        self.history
            .commit_transaction(transaction_id, activity_context(caller))
            .map_err(|_| HandlerError::Failed)?;
        if !save_history(&mut self.backend, &self.history) {
            return Err(HandlerError::Failed);
        }
        self.activity_ledger
            .transition(activity_sequence, ActivityOutcome::Committed)
            .map_err(|_| HandlerError::Failed)?;
        if !save_activity_ledger(self.backend.file_store_mut(), &self.activity_ledger) {
            return Err(HandlerError::Failed);
        }

        Ok(ActionOutput {
            summary: String::from("Moved three M22 fixture files as one recoverable transaction"),
            object_ids: invocation.object_ids().to_vec(),
        })
    }
}

impl ActionHandler<M22FixturePolicy> for M22CopyAction {
    fn execute(
        &mut self,
        invocation: ActionInvocation<'_, M22FixturePolicy>,
    ) -> Result<ActionOutput, HandlerError> {
        let caller = invocation.caller();
        if !M22FixturePolicy::caller_is_fixture(caller)
            || invocation.plan_intent() != M22_COPY_INTENT
            || invocation.action().action_id() != M22_COPY_ACTION
            || invocation.action().object_access() != ObjectAccess::Read
            || invocation.action().required_capabilities().len() != 1
            || invocation.action().required_capabilities()[0].as_str() != M22_COPY_CAPABILITY
            || invocation.capability_grants().len() != 1
            || invocation.capability_grants()[0].caller != caller
            || invocation.capability_grants()[0].capability != M22_COPY_CAPABILITY
            || invocation.object_ids() != [M22_COPY_SOURCE_OBJECT_ID]
            || invocation.object_handles().len() != 1
            || invocation.parameters().len() != 1
        {
            return Err(HandlerError::Failed);
        }

        let handle = invocation.object_handles()[0];
        let Some((object_id, source_name, moved_name, contents)) = MOVES
            .iter()
            .find(|(object_id, _, _, _)| *object_id == M22_COPY_SOURCE_OBJECT_ID)
        else {
            return Err(HandlerError::Failed);
        };
        let Some(destination_name) = invocation.string_parameter("destination_name") else {
            return Err(HandlerError::Failed);
        };
        if handle.object_id != *object_id
            || handle.source_name != *source_name
            || handle.destination_name != *moved_name
            || handle.contents != *contents
            || destination_name.as_bytes() != M22_COPY_DESTINATION
            || !valid_fixture_basename(destination_name.as_bytes())
        {
            return Err(HandlerError::Failed);
        }

        let Some(move_record) = self.history.record_at(0) else {
            return Err(HandlerError::Failed);
        };
        if !verify_context_and_group(&self.history, move_record.transaction_id.0)
            || !verify_transaction_state(
                &self.history,
                move_record.transaction_id.0,
                TransactionState::Committed,
            )
            || !verify_names(&mut self.backend.file_store_mut().volume, false)
        {
            return Err(HandlerError::Failed);
        }

        let mut content = [0; MAX_FILE_SIZE];
        let content_length = {
            let volume = &mut self.backend.file_store_mut().volume;
            if !named_file_matches(volume, handle.destination_name, handle.contents)
                || !file_is_missing(volume, destination_name.as_bytes())
            {
                return Err(HandlerError::Failed);
            }
            let source = volume
                .open(handle.destination_name)
                .map_err(|_| HandlerError::Failed)?;
            let length = volume
                .read(source, &mut content)
                .map_err(|_| HandlerError::Failed)?;
            if length > M22_COPY_MAX_BYTES || &content[..length] != handle.contents {
                return Err(HandlerError::Failed);
            }
            length
        };

        let transaction_id = self
            .history
            .record_create_transaction(
                activity_context(caller),
                M22_COPY_OBJECT_ID,
                destination_name.as_bytes(),
                &content[..content_length],
            )
            .map_err(|_| HandlerError::Failed)?;
        if !save_history(&mut self.backend, &self.history) {
            return Err(HandlerError::Failed);
        }

        let activity_sequence = self
            .activity_ledger
            .record_action(ActivityRecordInput {
                occurred_at: libnagi::time_ticks(),
                context: activity_context(caller),
                transaction_id: Some(transaction_id),
                user_intent: invocation.plan_intent(),
                selected_model: None,
                action_id: M22_COPY_ACTION,
                plan_summary: M22_COPY_PLAN_SUMMARY,
                object_ids: invocation.object_ids(),
            })
            .map_err(|_| HandlerError::Failed)?;
        if !save_activity_ledger(self.backend.file_store_mut(), &self.activity_ledger) {
            return Err(HandlerError::Failed);
        }

        {
            let volume = &mut self.backend.file_store_mut().volume;
            let destination = volume
                .create(destination_name.as_bytes())
                .map_err(|_| HandlerError::Failed)?;
            volume
                .write(destination, &content[..content_length])
                .map_err(|_| HandlerError::Failed)?;
            volume.flush().map_err(|_| HandlerError::Failed)?;
        }

        self.history
            .commit_transaction(transaction_id, activity_context(caller))
            .map_err(|_| HandlerError::Failed)?;
        if !save_history(&mut self.backend, &self.history) {
            return Err(HandlerError::Failed);
        }
        self.activity_ledger
            .transition(activity_sequence, ActivityOutcome::Committed)
            .map_err(|_| HandlerError::Failed)?;
        if !save_activity_ledger(self.backend.file_store_mut(), &self.activity_ledger) {
            return Err(HandlerError::Failed);
        }

        if content_length != contents.len()
            || self
                .activity_ledger
                .record_for_transaction(transaction_id)
                .is_none()
        {
            return Err(HandlerError::Failed);
        }
        self.copy_created.set(true);
        Ok(ActionOutput {
            summary: String::from("Copied one M22 fixture file as a recoverable transaction"),
            object_ids: alloc::vec![M22_COPY_OBJECT_ID],
        })
    }
}

fn run_file_move_action(
    block_capability: u64,
    backend: HistoryArchiveBackend<M22Files>,
    history: HistoryService,
    activity_ledger: ActivityLedger,
    search_activity: M19SearchActivity,
) -> bool {
    let policy = M22FixturePolicy::default();
    let caller = CallerIdentity {
        app_id: CALLER.app_id,
        app_session_id: CALLER.app_session_id,
        node_id: CALLER.node_id,
        workspace_id: CALLER.workspace_id,
    };
    let candidate_objects: Vec<_> = MOVES
        .iter()
        .map(|(object_id, _, _, _)| *object_id)
        .collect();
    let Ok(context) = ContextResolver.resolve(
        ContextRequest {
            caller,
            selected_object: None,
            candidate_objects,
        },
        &policy,
    ) else {
        return false;
    };
    if MOVES
        .iter()
        .any(|(object_id, _, _, _)| !context.contains_object(*object_id))
    {
        return false;
    }

    // This deny check is part of the deterministic guest fixture. Production
    // authority requires an authenticated capability provider.
    let foreign_caller = CallerIdentity {
        app_id: AppId(CALLER.app_id.0.wrapping_add(1)),
        ..caller
    };
    let Ok(capability) = CapabilityId::new(M22_MOVE_CAPABILITY) else {
        return false;
    };
    if policy.check_capability(foreign_caller, &capability).is_ok()
        || policy
            .check_object_access(foreign_caller, MOVES[0].0, ObjectAccess::Modify)
            .is_ok()
    {
        return false;
    }

    if !verify_m21_plan_rejections(&context) {
        return false;
    }
    libnagi::console_write(b"Nagi M21 plan rejection validation PASS\r\n");
    if !verify_m21_partial_execution(&context, &policy) {
        return false;
    }
    libnagi::console_write(b"Nagi M21 partial execution failure validation PASS\r\n");

    let plan = r#"{"plan_version":1,"intent":"move the three M22 fixture files","steps":[{"action":"file.move","object_ids":[8721,8722,8723],"parameters":{"destination_a":"m22-A","destination_b":"m22-B","destination_c":"m22-C"}}]}"#;
    let Ok(plan) = NagiPlan::parse_complete(plan) else {
        return false;
    };
    let Ok(descriptor) = ActionDescriptor::new(
        M22_MOVE_ACTION,
        vec![capability],
        ObjectAccess::Modify,
        MOVES.len(),
        MOVES.len(),
        DESTINATION_PARAMETERS
            .iter()
            .map(|name| ParameterRule::new(*name, ParameterKind::String { max_bytes: 32 }, true))
            .collect(),
    ) else {
        return false;
    };
    let mut registry = ActionRegistry::new();
    if registry
        .register(
            descriptor,
            M22MoveAction {
                backend,
                history,
                activity_ledger,
                search_activity,
            },
        )
        .is_err()
    {
        return false;
    }
    let Ok(validated) = validate_plan(plan, &context, &registry, &policy) else {
        return false;
    };
    let report = execute_plan(validated, &mut registry, &policy);
    let action_succeeded = report.status == ExecutionStatus::Succeeded
        && report.completed.len() == 1
        && report.completed[0].action_id == M22_MOVE_ACTION
        && report.completed[0].object_ids.as_slice()
            == MOVES.map(|(object_id, _, _, _)| object_id).as_slice();
    if !action_succeeded {
        return false;
    }
    drop(registry);

    let Ok((mut volume, _)) = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
    else {
        return false;
    };
    if !verify_names(&mut volume, false) {
        return false;
    }
    let mut persisted = HistoryArchiveBackend::new(M22Files { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(Some(length)) = persisted.load_archive(&mut archive) else {
        return false;
    };
    let Ok(history) = HistoryService::restore_recoverable(&archive[..length]) else {
        return false;
    };
    let Some(first) = history.record_at(0) else {
        return false;
    };
    if !verify_context_and_group(&history, first.transaction_id.0)
        || !verify_transaction_state(
            &history,
            first.transaction_id.0,
            TransactionState::Committed,
        )
    {
        return false;
    }
    let Ok((ledger, _)) = load_activity_ledger(persisted.file_store_mut()) else {
        return false;
    };
    let move_object_ids = MOVES.map(|(object_id, _, _, _)| object_id);
    if !verify_search_activity_record(&ledger, search_activity)
        || !verify_activity_record(
            &ledger,
            first.transaction_id,
            M22_MOVE_ACTION,
            M22_MOVE_INTENT,
            M22_MOVE_PLAN_SUMMARY,
            &move_object_ids,
            ActivityOutcome::Committed,
        )
    {
        return false;
    }

    libnagi::console_write(b"Nagi M22 file.search Activity Ledger PASS\r\n");
    libnagi::console_write(b"Nagi M22 AI Activity Ledger committed PASS\r\n");
    libnagi::console_write(b"Nagi M21 file.move Plan Validate Execute PASS\r\n");
    libnagi::console_write(b"Nagi M22 move group persisted in guest VFS PASS\r\n");
    true
}

fn run_file_copy_action(block_capability: u64) -> bool {
    let Ok((volume, _)) = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability)) else {
        return false;
    };
    let mut backend = HistoryArchiveBackend::new(M22Files { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(Some(length)) = backend.load_archive(&mut archive) else {
        return false;
    };
    let Ok(history) = HistoryService::restore_recoverable(&archive[..length]) else {
        return false;
    };
    let Ok((activity_ledger, true)) = load_activity_ledger(backend.file_store_mut()) else {
        return false;
    };
    let Some(move_record) = history.record_at(0) else {
        return false;
    };
    if !verify_context_and_group(&history, move_record.transaction_id.0)
        || !verify_transaction_state(
            &history,
            move_record.transaction_id.0,
            TransactionState::Committed,
        )
        || !verify_names(&mut backend.file_store_mut().volume, false)
        || !file_is_missing(&mut backend.file_store_mut().volume, M22_COPY_DESTINATION)
    {
        return false;
    }

    let copy_created = Rc::new(Cell::new(false));
    let policy = M22FixturePolicy {
        copy_created: Rc::clone(&copy_created),
        ..M22FixturePolicy::default()
    };
    let caller = CallerIdentity {
        app_id: CALLER.app_id,
        app_session_id: CALLER.app_session_id,
        node_id: CALLER.node_id,
        workspace_id: CALLER.workspace_id,
    };
    let Ok(context) = ContextResolver.resolve(
        ContextRequest {
            caller,
            selected_object: None,
            candidate_objects: alloc::vec![M22_COPY_SOURCE_OBJECT_ID],
        },
        &policy,
    ) else {
        return false;
    };
    if !context.contains_object(M22_COPY_SOURCE_OBJECT_ID) {
        return false;
    }

    let Ok(copy_capability) = CapabilityId::new(M22_COPY_CAPABILITY) else {
        return false;
    };
    let Ok(descriptor) = ActionDescriptor::new(
        M22_COPY_ACTION,
        alloc::vec![copy_capability],
        ObjectAccess::Read,
        1,
        1,
        alloc::vec![ParameterRule::new(
            "destination_name",
            ParameterKind::String {
                max_bytes: nagi_history::MAX_NAME_BYTES,
            },
            true,
        )],
    ) else {
        return false;
    };
    let mut registry = ActionRegistry::new();
    if registry
        .register(
            descriptor,
            M22CopyAction {
                backend,
                history,
                activity_ledger,
                copy_created: Rc::clone(&copy_created),
            },
        )
        .is_err()
    {
        return false;
    }

    let copy_plan = r#"{"plan_version":1,"intent":"copy the first M22 fixture file","steps":[{"action":"file.copy","object_ids":[8721],"parameters":{"destination_name":"m22-copy"}}]}"#;
    let denied_policy = M22FixturePolicy {
        deny_copy: true,
        copy_created: Rc::clone(&copy_created),
        ..M22FixturePolicy::default()
    };
    let Ok(denied_plan) = NagiPlan::parse_complete(copy_plan) else {
        return false;
    };
    if !matches!(
        validate_plan(denied_plan, &context, &registry, &denied_policy),
        Err(ValidationError::CapabilityDenied)
    ) || copy_created.get()
    {
        return false;
    }

    let bad_name_plan = r#"{"plan_version":1,"intent":"copy the first M22 fixture file","steps":[{"action":"file.copy","object_ids":[8721],"parameters":{"destination_name":"../outside"}}]}"#;
    let Ok(bad_name_plan) = NagiPlan::parse_complete(bad_name_plan) else {
        return false;
    };
    let Ok(validated_bad_name) = validate_plan(bad_name_plan, &context, &registry, &policy) else {
        return false;
    };
    let bad_name_report = execute_plan(validated_bad_name, &mut registry, &policy);
    if bad_name_report.status != ExecutionStatus::Failed
        || bad_name_report.error != Some(ExecutionError::Handler(HandlerError::Failed))
        || copy_created.get()
    {
        return false;
    }

    let Ok(plan) = NagiPlan::parse_complete(copy_plan) else {
        return false;
    };
    let Ok(validated) = validate_plan(plan, &context, &registry, &policy) else {
        return false;
    };
    let report = execute_plan(validated, &mut registry, &policy);
    let action_succeeded = report.status == ExecutionStatus::Succeeded
        && report.completed.len() == 1
        && report.completed[0].action_id == M22_COPY_ACTION
        && report.completed[0].object_ids == [M22_COPY_OBJECT_ID]
        && copy_created.get();
    if !action_succeeded {
        return false;
    }
    drop(registry);

    let Ok((mut volume, _)) = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
    else {
        return false;
    };
    let Some((_, _, _, expected_contents)) = MOVES
        .iter()
        .find(|(object_id, _, _, _)| *object_id == M22_COPY_SOURCE_OBJECT_ID)
    else {
        return false;
    };
    if !named_file_matches(&mut volume, M22_COPY_DESTINATION, expected_contents)
        || !named_file_matches(&mut volume, MOVES[0].2, expected_contents)
    {
        return false;
    }
    let mut persisted = HistoryArchiveBackend::new(M22Files { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(Some(length)) = persisted.load_archive(&mut archive) else {
        return false;
    };
    let Ok(history) = HistoryService::restore_recoverable(&archive[..length]) else {
        return false;
    };
    if !verify_full_history(&history)
        || !verify_transaction_state(
            &history,
            history
                .record_at(MOVES.len())
                .map_or(0, |record| record.transaction_id.0),
            TransactionState::Committed,
        )
    {
        return false;
    }
    let Ok((ledger, _)) = load_activity_ledger(persisted.file_store_mut()) else {
        return false;
    };
    let Some(copy_record) = history.record_at(MOVES.len()) else {
        return false;
    };
    let copy_object_ids = [M22_COPY_SOURCE_OBJECT_ID];
    if !verify_activity_record(
        &ledger,
        copy_record.transaction_id,
        M22_COPY_ACTION,
        M22_COPY_INTENT,
        M22_COPY_PLAN_SUMMARY,
        &copy_object_ids,
        ActivityOutcome::Committed,
    ) {
        return false;
    }

    libnagi::console_write(b"Nagi M21 file.copy Plan Validate Execute PASS\r\n");
    libnagi::console_write(b"Nagi M22 file.copy prepared transaction persisted PASS\r\n");
    true
}

fn verify_m21_plan_rejections(context: &ResolvedContext) -> bool {
    if !matches!(
        NagiPlan::parse_complete(
            r#"{"plan_version":1,"intent":"incomplete","steps":[{"action":"file.move"}"#
        ),
        Err(PlanParseError::InvalidJson)
    ) {
        return false;
    }

    let Ok(move_capability) = CapabilityId::new(M22_MOVE_CAPABILITY) else {
        return false;
    };
    let Ok(move_descriptor) = ActionDescriptor::new(
        M22_MOVE_ACTION,
        vec![move_capability],
        ObjectAccess::Modify,
        MOVES.len(),
        MOVES.len(),
        DESTINATION_PARAMETERS
            .iter()
            .map(|name| ParameterRule::new(*name, ParameterKind::String { max_bytes: 32 }, true))
            .collect(),
    ) else {
        return false;
    };
    let Ok(delete_capability) = CapabilityId::new("files.delete") else {
        return false;
    };
    let Ok(capability_probe_descriptor) = ActionDescriptor::new(
        "test.capability_probe",
        vec![delete_capability],
        ObjectAccess::Modify,
        0,
        0,
        vec![],
    ) else {
        return false;
    };

    let execution_count = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    if registry
        .register(
            move_descriptor,
            M22ValidationProbeAction {
                execution_count: Rc::clone(&execution_count),
                succeed: false,
            },
        )
        .is_err()
        || registry
            .register(
                capability_probe_descriptor,
                M22ValidationProbeAction {
                    execution_count: Rc::clone(&execution_count),
                    succeed: false,
                },
            )
            .is_err()
    {
        return false;
    }

    let rejected_plans = [
        (
            r#"{"plan_version":2,"intent":"unsupported version","steps":[{"action":"file.move","object_ids":[8721,8722,8723],"parameters":{"destination_a":"m22-A","destination_b":"m22-B","destination_c":"m22-C"}}]}"#,
            ValidationError::UnsupportedVersion,
        ),
        (
            r#"{"plan_version":1,"intent":"unregistered action","steps":[{"action":"file.unregistered","object_ids":[8721,8722,8723],"parameters":{"destination_a":"m22-A","destination_b":"m22-B","destination_c":"m22-C"}}]}"#,
            ValidationError::UnsupportedAction,
        ),
        (
            r#"{"plan_version":1,"intent":"outside fixture context","steps":[{"action":"file.move","object_ids":[8721,8722,8724],"parameters":{"destination_a":"m22-A","destination_b":"m22-B","destination_c":"m22-C"}}]}"#,
            ValidationError::ObjectOutsideContext,
        ),
        (
            r#"{"plan_version":1,"intent":"denied object modification","steps":[{"action":"file.move","object_ids":[8721,8722,8723],"parameters":{"destination_a":"m22-A","destination_b":"m22-B","destination_c":"m22-C"}}]}"#,
            ValidationError::ObjectDenied,
        ),
        (
            r#"{"plan_version":1,"intent":"denied capability","steps":[{"action":"test.capability_probe","object_ids":[],"parameters":{}}]}"#,
            ValidationError::CapabilityDenied,
        ),
    ];
    for (json, expected) in rejected_plans {
        let denial_policy = M22FixturePolicy {
            deny_modifications: expected == ValidationError::ObjectDenied,
            ..M22FixturePolicy::default()
        };
        if !m21_plan_rejected_as(json, context, &registry, &denial_policy, expected) {
            return false;
        }
    }

    execution_count.get() == 0
}

fn m21_plan_rejected_as(
    json: &str,
    context: &ResolvedContext,
    registry: &ActionRegistry<M22FixturePolicy>,
    policy: &M22FixturePolicy,
    expected: ValidationError,
) -> bool {
    let Ok(plan) = NagiPlan::parse_complete(json) else {
        return false;
    };
    matches!(
        validate_plan(plan, context, registry, policy),
        Err(error) if error == expected
    )
}

fn verify_m21_partial_execution(context: &ResolvedContext, policy: &M22FixturePolicy) -> bool {
    let Ok(capability) = CapabilityId::new(M22_MOVE_CAPABILITY) else {
        return false;
    };
    let Ok(first_descriptor) = ActionDescriptor::new(
        "test.partial_success",
        vec![capability.clone()],
        ObjectAccess::Read,
        0,
        0,
        vec![],
    ) else {
        return false;
    };
    let Ok(failing_descriptor) = ActionDescriptor::new(
        "test.partial_failure",
        vec![capability],
        ObjectAccess::Read,
        0,
        0,
        vec![],
    ) else {
        return false;
    };

    let execution_count = Rc::new(Cell::new(0));
    let mut registry = ActionRegistry::new();
    if registry
        .register(
            first_descriptor,
            M22ValidationProbeAction {
                execution_count: Rc::clone(&execution_count),
                succeed: true,
            },
        )
        .is_err()
        || registry
            .register(
                failing_descriptor,
                M22ValidationProbeAction {
                    execution_count: Rc::clone(&execution_count),
                    succeed: false,
                },
            )
            .is_err()
    {
        return false;
    }

    let json = r#"{"plan_version":1,"intent":"exercise partial executor failure","steps":[{"action":"test.partial_success","object_ids":[],"parameters":{}},{"action":"test.partial_failure","object_ids":[],"parameters":{}}]}"#;
    let Ok(plan) = NagiPlan::parse_complete(json) else {
        return false;
    };
    let Ok(validated) = validate_plan(plan, context, &registry, policy) else {
        return false;
    };
    let report = execute_plan(validated, &mut registry, policy);
    report.status == ExecutionStatus::Partial
        && report.completed.len() == 1
        && report.completed[0].action_id == "test.partial_success"
        && report.failed_step == Some(1)
        && report.error == Some(ExecutionError::Handler(HandlerError::Unavailable))
        && execution_count.get() == 2
}

fn activity_context(caller: CallerIdentity) -> ActivityContext {
    ActivityContext {
        app_id: caller.app_id,
        app_session_id: caller.app_session_id,
        node_id: caller.node_id,
        // The fixture policy only admits this one caller, whose presentation
        // surface is retained in the durable NH16 activity record.
        surface_id: CALLER.surface_id,
        workspace_id: caller.workspace_id,
    }
}

fn valid_fixture_basename(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= nagi_history::MAX_NAME_BYTES
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Exercises the NH16 archive over the persistent guest VFS. This fixture
/// validates storage and restart recovery only; it is not a production AI
/// service or authenticated production policy. Its initial grouped mutation
/// is deliberately exercised through the M21 Executor fixture boundary.
pub fn run(block_capability: u64, search_activity: M19SearchActivity) -> bool {
    let Ok((volume, _)) = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability)) else {
        return false;
    };
    let mut backend = HistoryArchiveBackend::new(M22Files { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(loaded) = backend.load_archive(&mut archive) else {
        return false;
    };

    let (mut history, is_new) = match loaded {
        Some(length) => match HistoryService::restore_recoverable(&archive[..length]) {
            Ok(history) => (history, false),
            Err(_) => return false,
        },
        None => (HistoryService::new(), true),
    };
    let (mut activity_ledger, ledger_was_present) =
        match load_activity_ledger(backend.file_store_mut()) {
            Ok(loaded) => loaded,
            Err(_) => return false,
        };

    if is_new {
        if ledger_was_present || !ensure_original_fixture(&mut backend.file_store_mut().volume) {
            return false;
        }
        return run_file_move_action(
            block_capability,
            backend,
            history,
            activity_ledger,
            search_activity,
        ) && run_file_copy_action(block_capability);
    }
    resume_m22_history(
        &mut backend,
        &mut history,
        &mut activity_ledger,
        search_activity,
    )
}

fn resume_m22_history(
    backend: &mut HistoryArchiveBackend<M22Files>,
    history: &mut HistoryService,
    activity_ledger: &mut ActivityLedger,
    search_activity: M19SearchActivity,
) -> bool {
    if !verify_full_history(history) {
        return false;
    }
    if ensure_search_activity_record(activity_ledger, search_activity).is_none()
        || !save_activity_ledger(backend.file_store_mut(), activity_ledger)
        || !verify_search_activity_record(activity_ledger, search_activity)
    {
        return false;
    }
    libnagi::console_write(b"Nagi M22 file.search Activity Ledger PASS\r\n");
    let Some(move_record) = history.record_at(0) else {
        return false;
    };
    let Some(copy_record) = history.record_at(MOVES.len()) else {
        return false;
    };

    for transaction_id in [move_record.transaction_id, copy_record.transaction_id] {
        let Some(first) = (0..history.len())
            .filter_map(|index| history.record_at(index))
            .find(|record| record.transaction_id == transaction_id)
        else {
            return false;
        };
        let Some(activity_sequence) =
            ensure_activity_record(history, activity_ledger, transaction_id)
        else {
            return false;
        };
        if !save_activity_ledger(backend.file_store_mut(), activity_ledger) {
            return false;
        }

        match first.transaction_state {
            TransactionState::Prepared => {
                let applied = if first.operation == Operation::Move {
                    apply_forward_group(&mut backend.file_store_mut().volume)
                } else if first.operation == Operation::Create {
                    let Ok(create) = history.prepared_create(transaction_id, CALLER) else {
                        return false;
                    };
                    if !prepared_create_matches_fixture(create) {
                        return false;
                    }
                    apply_idempotent_create(&mut backend.file_store_mut().volume, create)
                        && backend.file_store_mut().volume.flush().is_ok()
                } else {
                    false
                };
                if !applied
                    || history.commit_transaction(transaction_id, CALLER).is_err()
                    || !save_history(backend, history)
                    || !verify_transaction_files(
                        &mut backend.file_store_mut().volume,
                        history,
                        transaction_id,
                        false,
                    )
                {
                    return false;
                }
                if !set_activity_outcome(
                    activity_ledger,
                    activity_sequence,
                    ActivityOutcome::Committed,
                ) || !save_activity_ledger(backend.file_store_mut(), activity_ledger)
                {
                    return false;
                }
                libnagi::console_write(b"Nagi M22 recovered prepared file transaction PASS\r\n");
            }
            TransactionState::Committed | TransactionState::UndoPending => {
                if first.transaction_state == TransactionState::Committed
                    && !verify_transaction_files(
                        &mut backend.file_store_mut().volume,
                        history,
                        transaction_id,
                        false,
                    )
                {
                    return false;
                }
                let Ok(batch) = history.prepare_undo_transaction(transaction_id, CALLER) else {
                    return false;
                };
                if !save_history(backend, history)
                    || !set_activity_outcome(
                        activity_ledger,
                        activity_sequence,
                        ActivityOutcome::UndoPending,
                    )
                    || !save_activity_ledger(backend.file_store_mut(), activity_ledger)
                {
                    return false;
                }
                for action in batch.actions() {
                    let (from_name, from_length) = action.from_name.bytes();
                    let (to_name, to_length) = action.to_name.bytes();
                    let volume = &mut backend.file_store_mut().volume;
                    let applied = match action.operation {
                        UndoOperation::MoveBack if transaction_id == move_record.transaction_id => {
                            let Some((_, _, _, contents)) = MOVES
                                .iter()
                                .find(|(object_id, _, _, _)| *object_id == action.object_id)
                            else {
                                return false;
                            };
                            apply_idempotent_move(
                                volume,
                                &from_name[..from_length],
                                &to_name[..to_length],
                                contents,
                            )
                        }
                        UndoOperation::Delete
                            if transaction_id == copy_record.transaction_id
                                && action.object_id == M22_COPY_OBJECT_ID =>
                        {
                            let Some((_, _, _, expected_contents)) =
                                MOVES.iter().find(|(object_id, _, _, _)| {
                                    *object_id == M22_COPY_SOURCE_OBJECT_ID
                                })
                            else {
                                return false;
                            };
                            &from_name[..from_length] == M22_COPY_DESTINATION
                                && &to_name[..to_length] == M22_COPY_DESTINATION
                                && action.content_length == expected_contents.len()
                                && &action.content[..action.content_length] == *expected_contents
                                && apply_idempotent_delete(
                                    volume,
                                    M22_COPY_DESTINATION,
                                    expected_contents,
                                )
                        }
                        _ => false,
                    };
                    if !applied {
                        return false;
                    }
                }
                if backend.file_store_mut().volume.flush().is_err()
                    || history
                        .complete_undo_transaction(transaction_id, CALLER)
                        .is_err()
                    || !save_history(backend, history)
                    || !verify_transaction_files(
                        &mut backend.file_store_mut().volume,
                        history,
                        transaction_id,
                        true,
                    )
                {
                    return false;
                }
                if !set_activity_outcome(
                    activity_ledger,
                    activity_sequence,
                    ActivityOutcome::Undone,
                ) || !save_activity_ledger(backend.file_store_mut(), activity_ledger)
                {
                    return false;
                }
                libnagi::console_write(b"Nagi M22 AI Activity Ledger undo result PASS\r\n");
                libnagi::console_write(b"Nagi M22 composite undo applied and persisted PASS\r\n");
            }
            TransactionState::Undone => {
                if !verify_transaction_files(
                    &mut backend.file_store_mut().volume,
                    history,
                    transaction_id,
                    true,
                ) {
                    return false;
                }
                libnagi::console_write(b"Nagi M22 AI Activity Ledger undo result PASS\r\n");
                libnagi::console_write(b"Nagi M22 archive restart and restored files PASS\r\n");
            }
        }
    }

    let Some(move_record) = history.record_at(0) else {
        return false;
    };
    let Some(copy_record) = history.record_at(MOVES.len()) else {
        return false;
    };
    let move_object_ids = MOVES.map(|(object_id, _, _, _)| object_id);
    let copy_object_ids = [M22_COPY_SOURCE_OBJECT_ID];
    if !verify_names(&mut backend.file_store_mut().volume, true)
        || !file_is_missing(&mut backend.file_store_mut().volume, M22_COPY_DESTINATION)
        || !verify_activity_record(
            activity_ledger,
            move_record.transaction_id,
            M22_MOVE_ACTION,
            M22_MOVE_INTENT,
            M22_MOVE_PLAN_SUMMARY,
            &move_object_ids,
            ActivityOutcome::Undone,
        )
        || !verify_activity_record(
            activity_ledger,
            copy_record.transaction_id,
            M22_COPY_ACTION,
            M22_COPY_INTENT,
            M22_COPY_PLAN_SUMMARY,
            &copy_object_ids,
            ActivityOutcome::Undone,
        )
    {
        return false;
    }
    true
}

fn save_history<F: HistoryArchiveFileStore>(
    backend: &mut HistoryArchiveBackend<F>,
    history: &HistoryService,
) -> bool {
    let mut bytes = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(length) = history.serialize_recoverable(&mut bytes) else {
        return false;
    };
    backend.write_archive(&bytes[..length]).is_ok()
}

fn load_activity_ledger<F: ActivityLedgerFileStore>(
    files: &mut F,
) -> Result<(ActivityLedger, bool), ActivityLedgerError> {
    let mut backend = ActivityLedgerArchiveBackend::new(files);
    let mut archive = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
    match backend.load_archive(&mut archive)? {
        Some(length) => Ok((
            ActivityLedger::restore_recoverable(&archive[..length])?,
            true,
        )),
        None => Ok((ActivityLedger::new(), false)),
    }
}

fn save_activity_ledger<F: ActivityLedgerFileStore>(
    files: &mut F,
    ledger: &ActivityLedger,
) -> bool {
    let mut archive = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
    let Ok(length) = ledger.serialize_recoverable(&mut archive) else {
        return false;
    };
    ActivityLedgerArchiveBackend::new(files)
        .write_archive(&archive[..length])
        .is_ok()
}

fn ensure_search_activity_record(
    ledger: &mut ActivityLedger,
    activity: M19SearchActivity,
) -> Option<u64> {
    let mut existing_sequence = None;
    for index in 0..ledger.len() {
        let record = ledger.record_at(index)?;
        if record.action_id() == M19_SEARCH_ACTION {
            if record.transaction_id().is_some()
                || existing_sequence.is_some()
                || !search_activity_record_matches(record, activity)
            {
                return None;
            }
            existing_sequence = Some(record.sequence());
        }
    }

    let sequence = match existing_sequence {
        Some(sequence) => sequence,
        None => {
            let object_ids = [activity.object_id];
            ledger
                .record_action(ActivityRecordInput {
                    occurred_at: activity.occurred_at,
                    context: activity.context,
                    transaction_id: None,
                    user_intent: activity.user_intent,
                    selected_model: None,
                    action_id: M19_SEARCH_ACTION,
                    plan_summary: activity.plan_summary,
                    object_ids: &object_ids,
                })
                .ok()?
        }
    };

    if !set_activity_outcome(ledger, sequence, ActivityOutcome::Committed)
        || !verify_search_activity_record(ledger, activity)
    {
        return None;
    }
    Some(sequence)
}

fn search_activity_record_matches(record: &ActivityRecord, activity: M19SearchActivity) -> bool {
    record.context() == activity.context
        && record.transaction_id().is_none()
        && record.user_intent() == activity.user_intent
        && record.selected_model().is_none()
        && record.action_id() == M19_SEARCH_ACTION
        && record.plan_summary() == activity.plan_summary
        && record.object_ids() == [activity.object_id]
}

fn verify_search_activity_record(ledger: &ActivityLedger, activity: M19SearchActivity) -> bool {
    let mut search_record = None;
    for index in 0..ledger.len() {
        let Some(record) = ledger.record_at(index) else {
            return false;
        };
        if record.action_id() == M19_SEARCH_ACTION {
            if record.transaction_id().is_some() {
                return false;
            }
            if search_record.is_some() {
                return false;
            }
            search_record = Some(record);
        }
    }
    let Some(record) = search_record else {
        return false;
    };
    search_activity_record_matches(record, activity)
        && record.current_outcome() == ActivityOutcome::Committed
        && record.outcomes() == [ActivityOutcome::Prepared, ActivityOutcome::Committed]
}

fn ensure_activity_record(
    history: &HistoryService,
    ledger: &mut ActivityLedger,
    transaction_id: nagi_history::TransactionId,
) -> Option<u64> {
    let move_record = history.record_at(0)?;
    let copy_record = history.record_at(MOVES.len())?;
    let first = (0..history.len())
        .filter_map(|index| history.record_at(index))
        .find(|record| record.transaction_id == transaction_id)?;
    let (action_id, intent, plan_summary, object_ids) =
        if transaction_id == move_record.transaction_id {
            (
                M22_MOVE_ACTION,
                M22_MOVE_INTENT,
                M22_MOVE_PLAN_SUMMARY,
                &M22_MOVE_OBJECT_IDS[..],
            )
        } else if transaction_id == copy_record.transaction_id {
            (
                M22_COPY_ACTION,
                M22_COPY_INTENT,
                M22_COPY_PLAN_SUMMARY,
                &M22_COPY_OBJECT_IDS[..],
            )
        } else {
            return None;
        };
    let sequence = match ledger.record_for_transaction(transaction_id) {
        Some(record)
            if activity_record_matches(
                record,
                transaction_id,
                action_id,
                intent,
                plan_summary,
                object_ids,
            ) =>
        {
            record.sequence()
        }
        Some(_) => return None,
        None => ledger
            .record_action(ActivityRecordInput {
                occurred_at: libnagi::time_ticks(),
                context: CALLER,
                transaction_id: Some(transaction_id),
                user_intent: intent,
                selected_model: None,
                action_id,
                plan_summary,
                object_ids,
            })
            .ok()?,
    };
    let target = match first.transaction_state {
        TransactionState::Prepared => ActivityOutcome::Prepared,
        TransactionState::Committed => ActivityOutcome::Committed,
        TransactionState::UndoPending => ActivityOutcome::UndoPending,
        TransactionState::Undone => ActivityOutcome::Undone,
    };
    set_activity_outcome(ledger, sequence, target).then_some(sequence)
}

fn activity_record_matches(
    record: &ActivityRecord,
    transaction_id: nagi_history::TransactionId,
    action_id: &str,
    intent: &str,
    plan_summary: &str,
    object_ids: &[ObjectId],
) -> bool {
    record.context() == CALLER
        && record.transaction_id() == Some(transaction_id)
        && record.user_intent() == intent
        && record.selected_model().is_none()
        && record.action_id() == action_id
        && record.plan_summary() == plan_summary
        && record.object_ids() == object_ids
}

fn set_activity_outcome(
    ledger: &mut ActivityLedger,
    sequence: u64,
    target: ActivityOutcome,
) -> bool {
    let Some(record) = (0..ledger.len())
        .filter_map(|index| ledger.record_at(index))
        .find(|record| record.sequence() == sequence)
    else {
        return false;
    };
    let current = record.current_outcome();
    let path = [
        ActivityOutcome::Prepared,
        ActivityOutcome::Committed,
        ActivityOutcome::UndoPending,
        ActivityOutcome::Undone,
    ];
    let Some(current_index) = path.iter().position(|candidate| *candidate == current) else {
        return false;
    };
    let Some(target_index) = path.iter().position(|candidate| *candidate == target) else {
        return false;
    };
    if current_index > target_index {
        return false;
    }
    for next in path.iter().take(target_index + 1).skip(current_index + 1) {
        if ledger.transition(sequence, *next).is_err() {
            return false;
        }
    }
    true
}

fn verify_activity_record(
    ledger: &ActivityLedger,
    transaction_id: nagi_history::TransactionId,
    action_id: &str,
    intent: &str,
    plan_summary: &str,
    object_ids: &[ObjectId],
    expected_outcome: ActivityOutcome,
) -> bool {
    let Some(record) = ledger.record_for_transaction(transaction_id) else {
        return false;
    };
    if !activity_record_matches(
        record,
        transaction_id,
        action_id,
        intent,
        plan_summary,
        object_ids,
    ) || record.current_outcome() != expected_outcome
    {
        return false;
    }
    match expected_outcome {
        ActivityOutcome::Committed => {
            record.outcomes() == [ActivityOutcome::Prepared, ActivityOutcome::Committed]
        }
        ActivityOutcome::Undone => {
            record.outcomes()
                == [
                    ActivityOutcome::Prepared,
                    ActivityOutcome::Committed,
                    ActivityOutcome::UndoPending,
                    ActivityOutcome::Undone,
                ]
        }
        _ => false,
    }
}

fn ensure_original_fixture(volume: &mut GuestVolume) -> bool {
    if !file_is_missing(volume, M22_COPY_DESTINATION) {
        return false;
    }
    let mut all_empty = true;
    for (_, source, destination, contents) in MOVES {
        if file_is_missing(volume, source) {
            if file_is_missing(volume, destination) {
                continue;
            }
            return false;
        }
        all_empty = false;
        if !named_file_matches(volume, source, contents) || !file_is_missing(volume, destination) {
            return false;
        }
    }
    if all_empty {
        for (_, source, _, contents) in MOVES {
            if !write_named_file(volume, source, contents) {
                return false;
            }
        }
        volume.flush().is_ok()
    } else {
        for (_, source, destination, contents) in MOVES {
            if !file_is_missing(volume, destination) {
                return false;
            }
            if file_is_missing(volume, source) {
                if !write_named_file(volume, source, contents) {
                    return false;
                }
            } else if !named_file_matches(volume, source, contents) {
                return false;
            }
        }
        volume.flush().is_ok()
    }
}

fn apply_forward_group(volume: &mut GuestVolume) -> bool {
    MOVES.iter().all(|(_, source, destination, contents)| {
        apply_idempotent_move(volume, source, destination, contents)
    }) && volume.flush().is_ok()
}

fn apply_idempotent_move(
    volume: &mut GuestVolume,
    source: &[u8],
    destination: &[u8],
    contents: &[u8],
) -> bool {
    let source_missing = file_is_missing(volume, source);
    let destination_missing = file_is_missing(volume, destination);
    match (source_missing, destination_missing) {
        (false, true) if named_file_matches(volume, source, contents) => {
            volume.rename(source, destination).is_ok()
        }
        (true, false) if named_file_matches(volume, destination, contents) => true,
        _ => false,
    }
}

fn apply_idempotent_create(volume: &mut GuestVolume, create: nagi_history::PreparedCreate) -> bool {
    let (name, name_length) = create.name().bytes();
    let name = &name[..name_length];
    if name.is_empty() || create.content().len() > MAX_FILE_SIZE {
        return false;
    }
    match volume.open(name) {
        Ok(handle) => {
            let mut content = [0; MAX_FILE_SIZE];
            let Ok(length) = volume.read(handle, &mut content) else {
                return false;
            };
            if &content[..length] == create.content() {
                true
            } else if length <= create.content().len()
                && create.content().starts_with(&content[..length])
                && volume.remove(name).is_ok()
            {
                volume
                    .create(name)
                    .and_then(|handle| volume.write(handle, create.content()))
                    .is_ok()
            } else {
                false
            }
        }
        Err(StorageError::NotFound) => volume
            .create(name)
            .and_then(|handle| volume.write(handle, create.content()))
            .is_ok(),
        Err(_) => false,
    }
}

fn prepared_create_matches_fixture(create: nagi_history::PreparedCreate) -> bool {
    let (name, name_length) = create.name().bytes();
    let Some((_, _, _, expected_contents)) = MOVES
        .iter()
        .find(|(object_id, _, _, _)| *object_id == M22_COPY_SOURCE_OBJECT_ID)
    else {
        return false;
    };
    create.object_id() == M22_COPY_OBJECT_ID
        && &name[..name_length] == M22_COPY_DESTINATION
        && create.content() == *expected_contents
}

fn apply_idempotent_delete(volume: &mut GuestVolume, name: &[u8], expected: &[u8]) -> bool {
    match volume.open(name) {
        Err(StorageError::NotFound) => true,
        Ok(handle) => {
            let mut content = [0; MAX_FILE_SIZE];
            let Ok(length) = volume.read(handle, &mut content) else {
                return false;
            };
            length == expected.len()
                && &content[..length] == expected
                && volume.remove(name).is_ok()
        }
        Err(_) => false,
    }
}

fn verify_names(volume: &mut GuestVolume, restored: bool) -> bool {
    MOVES.iter().all(|(_, source, destination, contents)| {
        let (expected_name, absent_name) = if restored {
            (*source, *destination)
        } else {
            (*destination, *source)
        };
        named_file_matches(volume, expected_name, contents) && file_is_missing(volume, absent_name)
    })
}

fn verify_context_and_group(history: &HistoryService, transaction_id: u64) -> bool {
    if history.len() < MOVES.len() {
        return false;
    }
    MOVES
        .iter()
        .enumerate()
        .all(|(index, (object_id, _, _, _))| {
            history.record_at(index).is_some_and(|record| {
                record.transaction_id.0 == transaction_id
                    && record.context == CALLER
                    && record.object_id == *object_id
                    && record.operation == nagi_history::Operation::Move
                    && record.sequence == index as u64 + 1
            })
        })
}

fn verify_full_history(history: &HistoryService) -> bool {
    if history.len() != MOVES.len() + 1 {
        return false;
    }
    let Some(first) = history.record_at(0) else {
        return false;
    };
    let Some(copy) = history.record_at(MOVES.len()) else {
        return false;
    };
    copy.transaction_id != first.transaction_id
        && copy.context == CALLER
        && copy.object_id == M22_COPY_OBJECT_ID
        && copy.operation == Operation::Create
        && copy.sequence == MOVES.len() as u64 + 1
        && copy.before_length == 0
        && usize::from(copy.after_length) <= M22_COPY_MAX_BYTES
        && verify_context_and_group(history, first.transaction_id.0)
        && verify_transaction_state(history, first.transaction_id.0, first.transaction_state)
        && verify_transaction_state(history, copy.transaction_id.0, copy.transaction_state)
}

fn verify_transaction_state(
    history: &HistoryService,
    transaction_id: u64,
    expected_state: TransactionState,
) -> bool {
    let mut found = false;
    for index in 0..history.len() {
        if let Some(record) = history.record_at(index) {
            if record.transaction_id.0 == transaction_id {
                found = true;
                if record.transaction_state != expected_state {
                    return false;
                }
            }
        }
    }
    found
}

fn verify_transaction_files(
    volume: &mut GuestVolume,
    history: &HistoryService,
    transaction_id: nagi_history::TransactionId,
    restored: bool,
) -> bool {
    let Some(move_record) = history.record_at(0) else {
        return false;
    };
    if transaction_id == move_record.transaction_id {
        return verify_names(volume, restored);
    }
    let Some(copy_record) = history.record_at(MOVES.len()) else {
        return false;
    };
    if transaction_id != copy_record.transaction_id {
        return false;
    }
    if restored {
        file_is_missing(volume, M22_COPY_DESTINATION)
    } else {
        let Some((_, _, _, contents)) = MOVES
            .iter()
            .find(|(object_id, _, _, _)| *object_id == M22_COPY_SOURCE_OBJECT_ID)
        else {
            return false;
        };
        named_file_matches(volume, M22_COPY_DESTINATION, contents)
    }
}

fn write_named_file(volume: &mut GuestVolume, name: &[u8], contents: &[u8]) -> bool {
    if contents.len() > MAX_FILE_SIZE {
        return false;
    }
    let mut path = [0; nagi_history::MAX_NAME_BYTES + 1];
    path[0] = b'/';
    path[1..1 + name.len()].copy_from_slice(name);
    let path = &path[..name.len() + 1];
    let Ok(handle) = volume.create_path(path) else {
        return false;
    };
    volume.write(handle, contents).is_ok()
}

fn named_file_matches(volume: &mut GuestVolume, name: &[u8], expected: &[u8]) -> bool {
    let mut path = [0; nagi_history::MAX_NAME_BYTES + 1];
    path[0] = b'/';
    path[1..1 + name.len()].copy_from_slice(name);
    let Ok(handle) = volume.open_path(&path[..name.len() + 1]) else {
        return false;
    };
    let mut contents = [0; MAX_FILE_SIZE];
    let Ok(length) = volume.read(handle, &mut contents) else {
        return false;
    };
    length == expected.len() && &contents[..length] == expected
}

fn file_is_missing(volume: &mut GuestVolume, name: &[u8]) -> bool {
    let mut path = [0; nagi_history::MAX_NAME_BYTES + 1];
    path[0] = b'/';
    path[1..1 + name.len()].copy_from_slice(name);
    matches!(
        volume.open_path(&path[..name.len() + 1]),
        Err(StorageError::NotFound)
    )
}
