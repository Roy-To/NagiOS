#![no_std]

#[cfg(test)]
extern crate std;

pub mod activity;
pub mod transaction;
pub mod view;
pub mod wayback;

pub const MAX_RECORDS: usize = 16;
pub const MAX_SNAPSHOT_BYTES: usize = 1024;
pub const MAX_NAME_BYTES: usize = 32;

pub use nagi_model::{
    AppId, AppSessionId, NodeId, ObjectId, SurfaceId, TransactionId, WorkspaceId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityContext {
    pub app_id: AppId,
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub surface_id: Option<SurfaceId>,
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Create,
    Edit,
    Move,
    Delete,
    Restore,
}

impl Operation {
    pub const fn code(self) -> u8 {
        match self {
            Self::Create => 1,
            Self::Edit => 2,
            Self::Move => 3,
            Self::Delete => 4,
            Self::Restore => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoryError {
    Capacity,
    SnapshotTooLarge,
    NameTooLong,
    EmptyName,
    NoHistory,
    InvalidRecord,
    BufferTooSmall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Name {
    bytes: [u8; MAX_NAME_BYTES],
    length: u8,
}

impl Name {
    pub fn new(bytes: &[u8]) -> Result<Self, HistoryError> {
        if bytes.is_empty() {
            return Err(HistoryError::EmptyName);
        }
        if bytes.len() > MAX_NAME_BYTES {
            return Err(HistoryError::NameTooLong);
        }
        let mut name = Self {
            bytes: [0; MAX_NAME_BYTES],
            length: bytes.len() as u8,
        };
        name.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(name)
    }

    pub const fn bytes(self) -> ([u8; MAX_NAME_BYTES], usize) {
        (self.bytes, self.length as usize)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryRecord {
    pub sequence: u64,
    pub operation: Operation,
    pub context: ActivityContext,
    pub object_id: ObjectId,
    pub before_length: u16,
    pub after_length: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndoOperation {
    Delete,
    Restore,
    RestoreVersion,
    MoveBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoAction {
    pub operation: UndoOperation,
    pub object_id: ObjectId,
    pub from_name: Name,
    pub to_name: Name,
    pub content: [u8; MAX_SNAPSHOT_BYTES],
    pub content_length: usize,
}

#[derive(Clone, Copy)]
struct Snapshot {
    content: [u8; MAX_SNAPSHOT_BYTES],
    length: usize,
}

impl Snapshot {
    const EMPTY: Self = Self {
        content: [0; MAX_SNAPSHOT_BYTES],
        length: 0,
    };

    fn from_bytes(bytes: &[u8]) -> Result<Self, HistoryError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(HistoryError::SnapshotTooLarge);
        }
        let mut snapshot = Self::EMPTY;
        snapshot.content[..bytes.len()].copy_from_slice(bytes);
        snapshot.length = bytes.len();
        Ok(snapshot)
    }
}

#[derive(Clone, Copy)]
struct Entry {
    record: HistoryRecord,
    before_name: Name,
    after_name: Name,
    before: Snapshot,
    after: Snapshot,
}

pub struct HistoryService {
    entries: [Option<Entry>; MAX_RECORDS],
    length: usize,
    next_sequence: u64,
}

impl HistoryService {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_RECORDS],
            length: 0,
            next_sequence: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn record_create(
        &mut self,
        context: ActivityContext,
        object_id: ObjectId,
        name: &[u8],
        content: &[u8],
    ) -> Result<(), HistoryError> {
        self.push(
            Operation::Create,
            context,
            object_id,
            Name::new(name)?,
            Name::new(name)?,
            &[],
            content,
        )
    }

    pub fn record_edit(
        &mut self,
        context: ActivityContext,
        object_id: ObjectId,
        name: &[u8],
        before: &[u8],
        after: &[u8],
    ) -> Result<(), HistoryError> {
        let name = Name::new(name)?;
        self.push(
            Operation::Edit,
            context,
            object_id,
            name,
            name,
            before,
            after,
        )
    }

    pub fn record_move(
        &mut self,
        context: ActivityContext,
        object_id: ObjectId,
        before_name: &[u8],
        after_name: &[u8],
    ) -> Result<(), HistoryError> {
        self.push(
            Operation::Move,
            context,
            object_id,
            Name::new(before_name)?,
            Name::new(after_name)?,
            &[],
            &[],
        )
    }

    pub fn record_delete(
        &mut self,
        context: ActivityContext,
        object_id: ObjectId,
        name: &[u8],
        content: &[u8],
    ) -> Result<(), HistoryError> {
        let name = Name::new(name)?;
        self.push(
            Operation::Delete,
            context,
            object_id,
            name,
            name,
            content,
            &[],
        )
    }

    pub fn record_restore(
        &mut self,
        context: ActivityContext,
        object_id: ObjectId,
        name: &[u8],
        content: &[u8],
    ) -> Result<(), HistoryError> {
        let name = Name::new(name)?;
        self.push(
            Operation::Restore,
            context,
            object_id,
            name,
            name,
            &[],
            content,
        )
    }

    pub fn undo_last(&mut self) -> Result<UndoAction, HistoryError> {
        if self.length == 0 {
            return Err(HistoryError::NoHistory);
        }
        let index = self.length - 1;
        let entry = self.entries[index]
            .take()
            .ok_or(HistoryError::InvalidRecord)?;
        self.length = index;
        let (operation, content, content_length) = match entry.record.operation {
            Operation::Create => (UndoOperation::Delete, entry.after, entry.after.length),
            Operation::Edit => (
                UndoOperation::RestoreVersion,
                entry.before,
                entry.before.length,
            ),
            Operation::Move => (UndoOperation::MoveBack, Snapshot::EMPTY, 0),
            Operation::Delete => (UndoOperation::Restore, entry.before, entry.before.length),
            Operation::Restore => (UndoOperation::Delete, entry.after, entry.after.length),
        };
        Ok(UndoAction {
            operation,
            object_id: entry.record.object_id,
            from_name: entry.after_name,
            to_name: entry.before_name,
            content: content.content,
            content_length,
        })
    }

    pub fn serialize(&self, output: &mut [u8]) -> Result<usize, HistoryError> {
        let required = 4 + self.length * 32;
        if output.len() < required {
            return Err(HistoryError::BufferTooSmall);
        }
        output[..4].copy_from_slice(b"NH15");
        let mut offset = 4;
        for index in 0..self.length {
            let entry = self.entries[index].ok_or(HistoryError::InvalidRecord)?;
            let record = entry.record;
            output[offset..offset + 8].copy_from_slice(&record.sequence.to_le_bytes());
            output[offset + 8..offset + 16].copy_from_slice(&record.object_id.0.to_le_bytes());
            output[offset + 16] = record.operation.code();
            output[offset + 17] = record.context.app_id.0 as u8;
            output[offset + 18] = record.context.app_session_id.0 as u8;
            output[offset + 19] = record.context.node_id.0 as u8;
            output[offset + 20] = record.context.surface_id.is_some() as u8;
            output[offset + 21] = record.context.workspace_id.is_some() as u8;
            output[offset + 22..offset + 24].copy_from_slice(&record.before_length.to_le_bytes());
            output[offset + 24..offset + 26].copy_from_slice(&record.after_length.to_le_bytes());
            output[offset + 26] = entry.before_name.length;
            output[offset + 27] = entry.after_name.length;
            output[offset + 28] = entry.before.length as u8;
            output[offset + 29] = entry.after.length as u8;
            output[offset + 30] = 0;
            output[offset + 31] = 0;
            offset += 32;
        }
        Ok(required)
    }

    // Keep the internal entry shape explicit at the call sites; the public
    // record_* methods are the intentionally smaller API surface.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        operation: Operation,
        context: ActivityContext,
        object_id: ObjectId,
        before_name: Name,
        after_name: Name,
        before: &[u8],
        after: &[u8],
    ) -> Result<(), HistoryError> {
        if self.length == MAX_RECORDS {
            return Err(HistoryError::Capacity);
        }
        let before = Snapshot::from_bytes(before)?;
        let after = Snapshot::from_bytes(after)?;
        self.entries[self.length] = Some(Entry {
            record: HistoryRecord {
                sequence: self.next_sequence,
                operation,
                context,
                object_id,
                before_length: before.length as u16,
                after_length: after.length as u16,
            },
            before_name,
            after_name,
            before,
            after,
        });
        self.length += 1;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(())
    }
}

impl Default for HistoryService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityContext, AppId, AppSessionId, HistoryError, HistoryService, NodeId, ObjectId,
        Operation, UndoOperation, WorkspaceId,
    };

    const CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(7),
        app_session_id: AppSessionId(8),
        node_id: NodeId(9),
        surface_id: None,
        workspace_id: Some(WorkspaceId(10)),
    };

    #[test]
    fn records_the_full_create_edit_move_delete_restore_flow() {
        let mut history = HistoryService::new();
        history
            .record_create(CONTEXT, ObjectId(42), b"note", b"one")
            .unwrap();
        history
            .record_edit(CONTEXT, ObjectId(42), b"note", b"one", b"two")
            .unwrap();
        history
            .record_move(CONTEXT, ObjectId(42), b"note", b"moved")
            .unwrap();
        history
            .record_delete(CONTEXT, ObjectId(42), b"moved", b"two")
            .unwrap();
        history
            .record_restore(CONTEXT, ObjectId(42), b"note", b"two")
            .unwrap();
        assert_eq!(history.len(), 5);
        let mut ledger = [0; 256];
        assert!(history.serialize(&mut ledger).unwrap() > 4);
    }

    #[test]
    fn undo_returns_the_real_inverse_and_version_bytes() {
        let mut history = HistoryService::new();
        history
            .record_edit(CONTEXT, ObjectId(42), b"note", b"before", b"after")
            .unwrap();
        let undo = history.undo_last().unwrap();
        assert_eq!(undo.operation, UndoOperation::RestoreVersion);
        assert_eq!(&undo.content[..undo.content_length], b"before");
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn rejects_unbounded_records_names_and_snapshots() {
        let mut history = HistoryService::new();
        assert_eq!(
            history.record_create(CONTEXT, ObjectId(1), b"", b"x"),
            Err(HistoryError::EmptyName)
        );
        assert_eq!(
            history.record_create(CONTEXT, ObjectId(1), b"x", &[0; 1025]),
            Err(HistoryError::SnapshotTooLarge)
        );
        for index in 0..16 {
            history
                .record_move(CONTEXT, ObjectId(index), b"a", b"b")
                .unwrap();
        }
        assert_eq!(
            history.record_move(CONTEXT, ObjectId(99), b"a", b"b"),
            Err(HistoryError::Capacity)
        );
    }

    #[test]
    fn operation_codes_are_stable_for_the_persisted_ledger() {
        assert_eq!(Operation::Create.code(), 1);
        assert_eq!(Operation::Edit.code(), 2);
        assert_eq!(Operation::Move.code(), 3);
        assert_eq!(Operation::Delete.code(), 4);
        assert_eq!(Operation::Restore.code(), 5);
    }
}
