use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use nagi_model::{ObjectId, WorkspaceId};

use crate::{codec, MetadataRecord, ModelError, Relation, Workspace};

pub const CURRENT_STORE_VERSION: u16 = 1;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OBJECT_RECORDS: usize = 65_536;
pub const MAX_RELATIONS: usize = 262_144;
pub const MAX_WORKSPACES: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    Io,
    SnapshotTooLarge,
    /// A bounded snapshot buffer could not reserve memory; retry may succeed.
    Capacity,
    /// The commit may be visible. Stop mutations until a successful reload.
    RecoveryRequired,
}

/// The backend persists opaque versioned snapshots; it does not define object
/// identity, query semantics, or authorization. Implementations must replace
/// a complete snapshot or return an error without claiming success. A write
/// error that cannot rule out publication MUST be RecoveryRequired. Other
/// write errors mean the candidate was not published. Consumers must latch
/// RecoveryRequired and reload before constructing another mutation.
pub trait SnapshotBackend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError>;
    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataStoreError {
    Backend(BackendError),
    CorruptSnapshot,
    UnsupportedVersion(u16),
    Capacity,
    InvalidRecord(ModelError),
    ObjectNotFound,
    WorkspaceNotFound,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreState {
    pub records: BTreeMap<ObjectId, MetadataRecord>,
    pub relations: BTreeSet<Relation>,
    pub workspaces: BTreeMap<WorkspaceId, Workspace>,
}

pub(crate) struct MetadataStore<B> {
    backend: B,
    state: StoreState,
    requires_reload: bool,
}

impl<B: SnapshotBackend> MetadataStore<B> {
    pub(crate) fn open(mut backend: B) -> Result<Self, MetadataStoreError> {
        let state = match backend
            .load_snapshot()
            .map_err(MetadataStoreError::Backend)?
        {
            Some(snapshot) => codec::decode(&snapshot)?,
            None => StoreState::default(),
        };
        Ok(Self {
            backend,
            state,
            requires_reload: false,
        })
    }

    pub(crate) fn requires_reload(&self) -> bool {
        self.requires_reload
    }

    /// Install only a completely loaded and decoded state. Failure leaves the
    /// last confirmed read view intact and keeps every mutation blocked.
    pub(crate) fn reload(&mut self) -> Result<(), MetadataStoreError> {
        self.requires_reload = true;
        let state = match self
            .backend
            .load_snapshot()
            .map_err(MetadataStoreError::Backend)?
        {
            Some(snapshot) => codec::decode(&snapshot)?,
            None => StoreState::default(),
        };
        self.state = state;
        self.requires_reload = false;
        Ok(())
    }

    fn ensure_writable(&self) -> Result<(), MetadataStoreError> {
        if self.requires_reload {
            Err(MetadataStoreError::Backend(BackendError::RecoveryRequired))
        } else {
            Ok(())
        }
    }

    pub(crate) fn record(&self, id: ObjectId) -> Option<&MetadataRecord> {
        self.state.records.get(&id)
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &MetadataRecord> {
        self.state.records.values()
    }

    pub(crate) fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.state.workspaces.get(&id)
    }

    pub(crate) fn workspaces(&self) -> impl Iterator<Item = &Workspace> {
        self.state.workspaces.values()
    }

    pub(crate) fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.state.relations.iter()
    }

    pub(crate) fn upsert_record(
        &mut self,
        mut record: MetadataRecord,
    ) -> Result<(), MetadataStoreError> {
        self.ensure_writable()?;
        record
            .validate()
            .map_err(MetadataStoreError::InvalidRecord)?;
        record.tombstoned_at = None;
        let mut next = self.state.clone();
        if !next.records.contains_key(&record.object_id) && next.records.len() >= MAX_OBJECT_RECORDS
        {
            return Err(MetadataStoreError::Capacity);
        }
        next.records.insert(record.object_id, record);
        self.commit(next)
    }

    /// Delete leaves a durable tombstone, removes incident relations, and
    /// removes Workspace memberships. Re-upserting the same ObjectId revives
    /// it and begins with no implicit relationships or memberships.
    pub(crate) fn remove_record(
        &mut self,
        id: ObjectId,
        deleted_at: i64,
    ) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        let Some(existing) = self.state.records.get(&id) else {
            return Ok(false);
        };
        if existing.tombstoned_at.is_some() {
            return Ok(false);
        }
        let mut next = self.state.clone();
        if let Some(record) = next.records.get_mut(&id) {
            record.tombstoned_at = Some(deleted_at);
        }
        next.relations
            .retain(|relation| relation.source != id && relation.target != id);
        for workspace in next.workspaces.values_mut() {
            workspace.objects.retain(|object_id| *object_id != id);
        }
        self.commit(next)?;
        Ok(true)
    }

    pub(crate) fn upsert_workspace(
        &mut self,
        mut workspace: Workspace,
    ) -> Result<(), MetadataStoreError> {
        self.ensure_writable()?;
        workspace
            .validate()
            .map_err(MetadataStoreError::InvalidRecord)?;
        workspace.sessions.sort();
        workspace.sessions.dedup();
        workspace.objects.sort();
        workspace.objects.dedup();
        for object_id in &workspace.objects {
            if self
                .state
                .records
                .get(object_id)
                .is_none_or(|record| record.tombstoned_at.is_some())
            {
                return Err(MetadataStoreError::ObjectNotFound);
            }
        }
        let mut next = self.state.clone();
        if !next.workspaces.contains_key(&workspace.workspace_id)
            && next.workspaces.len() >= MAX_WORKSPACES
        {
            return Err(MetadataStoreError::Capacity);
        }
        next.workspaces.insert(workspace.workspace_id, workspace);
        self.commit(next)
    }

    pub(crate) fn remove_workspace(&mut self, id: WorkspaceId) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        if !self.state.workspaces.contains_key(&id) {
            return Ok(false);
        }
        let mut next = self.state.clone();
        next.workspaces.remove(&id);
        self.commit(next)?;
        Ok(true)
    }

    pub(crate) fn add_workspace_object(
        &mut self,
        workspace_id: WorkspaceId,
        object_id: ObjectId,
    ) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        if self
            .state
            .records
            .get(&object_id)
            .is_none_or(|record| record.tombstoned_at.is_some())
        {
            return Err(MetadataStoreError::ObjectNotFound);
        }
        let mut next = self.state.clone();
        let workspace = next
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(MetadataStoreError::WorkspaceNotFound)?;
        match workspace.objects.binary_search(&object_id) {
            Ok(_) => Ok(false),
            Err(index) => {
                if workspace.objects.len() >= crate::model::MAX_WORKSPACE_OBJECTS {
                    return Err(MetadataStoreError::Capacity);
                }
                workspace.objects.insert(index, object_id);
                self.commit(next)?;
                Ok(true)
            }
        }
    }

    pub(crate) fn remove_workspace_object(
        &mut self,
        workspace_id: WorkspaceId,
        object_id: ObjectId,
    ) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        let mut next = self.state.clone();
        let workspace = next
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(MetadataStoreError::WorkspaceNotFound)?;
        match workspace.objects.binary_search(&object_id) {
            Ok(index) => {
                workspace.objects.remove(index);
                self.commit(next)?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    pub(crate) fn link_session(
        &mut self,
        workspace_id: WorkspaceId,
        session: crate::WorkspaceSession,
    ) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        let mut next = self.state.clone();
        let workspace = next
            .workspaces
            .get_mut(&workspace_id)
            .ok_or(MetadataStoreError::WorkspaceNotFound)?;
        match workspace.sessions.binary_search(&session) {
            Ok(_) => Ok(false),
            Err(index) => {
                if workspace.sessions.len() >= crate::model::MAX_WORKSPACE_SESSIONS {
                    return Err(MetadataStoreError::Capacity);
                }
                workspace.sessions.insert(index, session);
                self.commit(next)?;
                Ok(true)
            }
        }
    }

    pub(crate) fn add_relation(&mut self, relation: Relation) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        for id in [relation.source, relation.target] {
            if self
                .state
                .records
                .get(&id)
                .is_none_or(|record| record.tombstoned_at.is_some())
            {
                return Err(MetadataStoreError::ObjectNotFound);
            }
        }
        if self.state.relations.contains(&relation) {
            return Ok(false);
        }
        if self.state.relations.len() >= MAX_RELATIONS {
            return Err(MetadataStoreError::Capacity);
        }
        let mut next = self.state.clone();
        next.relations.insert(relation);
        self.commit(next)?;
        Ok(true)
    }

    pub(crate) fn remove_relation(
        &mut self,
        relation: Relation,
    ) -> Result<bool, MetadataStoreError> {
        self.ensure_writable()?;
        if !self.state.relations.contains(&relation) {
            return Ok(false);
        }
        let mut next = self.state.clone();
        next.relations.remove(&relation);
        self.commit(next)?;
        Ok(true)
    }

    fn commit(&mut self, next: StoreState) -> Result<(), MetadataStoreError> {
        self.ensure_writable()?;
        let bytes = codec::encode(&next)?;
        if let Err(error) = self.backend.write_snapshot(&bytes) {
            self.requires_reload |= error == BackendError::RecoveryRequired;
            return Err(MetadataStoreError::Backend(error));
        }
        self.state = next;
        Ok(())
    }
}
