#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub const MAX_RECORDS: usize = 16;
pub const MAX_SNAPSHOT_BYTES: usize = 1024;
pub const MAX_NAME_BYTES: usize = 32;
pub const MAX_ARCHIVE_BYTES: usize = 36 * 1024;

pub mod activity_ledger;
pub mod guest;

const ARCHIVE_HEADER_BYTES: usize = 36;
const ARCHIVE_VERSION: u16 = 1;

pub use nagi_model::{AppId, AppSessionId, NodeId, ObjectId, SurfaceId, WorkspaceId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityContext {
    pub app_id: AppId,
    pub app_session_id: AppSessionId,
    pub node_id: NodeId,
    pub surface_id: Option<SurfaceId>,
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransactionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TransactionState {
    Prepared = 1,
    Committed = 2,
    UndoPending = 3,
    Undone = 4,
}

impl TransactionState {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Prepared),
            2 => Some(Self::Committed),
            3 => Some(Self::UndoPending),
            4 => Some(Self::Undone),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoveRecord<'a> {
    pub object_id: ObjectId,
    pub from_name: &'a [u8],
    pub to_name: &'a [u8],
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

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Create),
            2 => Some(Self::Edit),
            3 => Some(Self::Move),
            4 => Some(Self::Delete),
            5 => Some(Self::Restore),
            _ => None,
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
    Unauthorized,
    TransactionNotFound,
    TransactionAlreadyUndone,
    UndoNotPrepared,
    CompositeUndoRequired,
    InvalidTransaction,
    TransactionNotPrepared,
    TransactionNotCommitted,
    CorruptArchive,
    UnsupportedArchiveVersion(u16),
    Storage,
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
    pub transaction_id: TransactionId,
    pub transaction_state: TransactionState,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoBatch {
    transaction_id: TransactionId,
    actions: [Option<UndoAction>; MAX_RECORDS],
    length: usize,
}

impl UndoBatch {
    pub const fn transaction_id(&self) -> TransactionId {
        self.transaction_id
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn actions(&self) -> impl Iterator<Item = UndoAction> + '_ {
        self.actions[..self.length]
            .iter()
            .filter_map(|action| *action)
    }
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
struct Entry {
    record: HistoryRecord,
    before_name: Name,
    after_name: Name,
    before: Snapshot,
    after: Snapshot,
}

#[derive(Debug)]
pub struct HistoryService {
    entries: [Option<Entry>; MAX_RECORDS],
    length: usize,
    next_sequence: u64,
    next_transaction_id: u64,
}

impl HistoryService {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_RECORDS],
            length: 0,
            next_sequence: 1,
            next_transaction_id: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn record_at(&self, index: usize) -> Option<HistoryRecord> {
        self.entries
            .get(index)
            .and_then(|entry| *entry)
            .map(|entry| entry.record)
    }

    /// Records a set of moves as one prepared transaction. The caller must
    /// persist the recoverable archive before applying the external mutations,
    /// then call `commit_transaction` and persist again after all moves succeed.
    /// Prepared records are not eligible for undo.
    pub fn record_move_group(
        &mut self,
        context: ActivityContext,
        moves: &[MoveRecord<'_>],
    ) -> Result<TransactionId, HistoryError> {
        if moves.is_empty()
            || moves.len() > MAX_RECORDS
            || self.length.saturating_add(moves.len()) > MAX_RECORDS
            || self.next_transaction_id == u64::MAX
            || self.next_sequence.checked_add(moves.len() as u64).is_none()
        {
            return Err(if moves.is_empty() {
                HistoryError::InvalidTransaction
            } else {
                HistoryError::Capacity
            });
        }
        let mut names = [None; MAX_RECORDS];
        for (index, movement) in moves.iter().enumerate() {
            names[index] = Some((Name::new(movement.from_name)?, Name::new(movement.to_name)?));
        }
        let transaction_id = TransactionId(self.next_transaction_id);
        self.next_transaction_id += 1;
        for (index, movement) in moves.iter().enumerate() {
            let (before_name, after_name) = names[index].ok_or(HistoryError::InvalidRecord)?;
            self.push_with_transaction(
                transaction_id,
                context,
                Operation::Move,
                movement.object_id,
                before_name,
                after_name,
                &[],
                &[],
                TransactionState::Prepared,
            )?;
        }
        Ok(transaction_id)
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
        let entry = self.entries[index].ok_or(HistoryError::InvalidRecord)?;
        if entry.record.transaction_state != TransactionState::Committed {
            return Err(HistoryError::UndoNotPrepared);
        }
        if self
            .entries
            .iter()
            .take(self.length)
            .flatten()
            .filter(|candidate| candidate.record.transaction_id == entry.record.transaction_id)
            .count()
            != 1
        {
            return Err(HistoryError::CompositeUndoRequired);
        }
        self.entries[index] = None;
        self.length = index;
        Ok(inverse_of(entry))
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

    /// Writes the complete versioned archive required to restore names,
    /// snapshots, transaction state, and full logical caller context. `NH15`
    /// remains a compatibility metadata format and is not restart-restorable.
    pub fn serialize_recoverable(&self, output: &mut [u8]) -> Result<usize, HistoryError> {
        let payload_length = self.archive_payload_length()?;
        let required = ARCHIVE_HEADER_BYTES
            .checked_add(payload_length)
            .filter(|length| *length <= MAX_ARCHIVE_BYTES)
            .ok_or(HistoryError::Capacity)?;
        if output.len() < required {
            return Err(HistoryError::BufferTooSmall);
        }

        output[..4].copy_from_slice(b"NH16");
        output[4..6].copy_from_slice(&ARCHIVE_VERSION.to_le_bytes());
        output[6..8].copy_from_slice(&(self.length as u16).to_le_bytes());
        output[8..16].copy_from_slice(&self.next_sequence.to_le_bytes());
        output[16..24].copy_from_slice(&self.next_transaction_id.to_le_bytes());
        output[24..28].copy_from_slice(&(payload_length as u32).to_le_bytes());

        let mut offset = ARCHIVE_HEADER_BYTES;
        for entry in self.entries.iter().take(self.length).flatten() {
            encode_entry(entry, output, &mut offset)?;
        }
        if offset != required {
            return Err(HistoryError::InvalidRecord);
        }
        let digest = archive_checksum(&output[..required]);
        output[28..36].copy_from_slice(&digest.to_le_bytes());
        Ok(required)
    }

    /// Restores a complete `NH16` archive. Invalid, truncated, oversized, or
    /// unsupported archives fail closed without returning partial history.
    pub fn restore_recoverable(input: &[u8]) -> Result<Self, HistoryError> {
        if input.len() < ARCHIVE_HEADER_BYTES || input.len() > MAX_ARCHIVE_BYTES {
            return Err(HistoryError::CorruptArchive);
        }
        if &input[..4] != b"NH16" {
            return Err(HistoryError::CorruptArchive);
        }
        let version = read_u16(input, 4)?;
        if version != ARCHIVE_VERSION {
            return Err(HistoryError::UnsupportedArchiveVersion(version));
        }
        let count = usize::from(read_u16(input, 6)?);
        let next_sequence = read_u64(input, 8)?;
        let next_transaction_id = read_u64(input, 16)?;
        let payload_length =
            usize::try_from(read_u32(input, 24)?).map_err(|_| HistoryError::CorruptArchive)?;
        let expected_checksum = read_u64(input, 28)?;
        if count > MAX_RECORDS
            || payload_length != input.len() - ARCHIVE_HEADER_BYTES
            || archive_checksum(input) != expected_checksum
            || next_sequence == 0
            || next_transaction_id == 0
        {
            return Err(HistoryError::CorruptArchive);
        }

        let mut restored = Self::new();
        restored.next_sequence = next_sequence;
        restored.next_transaction_id = next_transaction_id;
        let mut offset = ARCHIVE_HEADER_BYTES;
        let mut previous_sequence = 0;
        let mut maximum_transaction_id = 0;
        for index in 0..count {
            let entry = decode_entry(input, &mut offset)?;
            if entry.record.sequence <= previous_sequence
                || entry.record.sequence >= next_sequence
                || entry.record.transaction_id.0 == 0
                || entry.record.transaction_id.0 >= next_transaction_id
            {
                return Err(HistoryError::CorruptArchive);
            }
            previous_sequence = entry.record.sequence;
            maximum_transaction_id = maximum_transaction_id.max(entry.record.transaction_id.0);
            for earlier in restored.entries.iter().take(index).flatten() {
                if earlier.record.transaction_id == entry.record.transaction_id
                    && (earlier.record.transaction_state != entry.record.transaction_state
                        || !same_caller(earlier.record.context, entry.record.context))
                {
                    return Err(HistoryError::CorruptArchive);
                }
            }
            restored.entries[index] = Some(entry);
        }
        if offset != input.len() || maximum_transaction_id >= next_transaction_id {
            return Err(HistoryError::CorruptArchive);
        }
        restored.length = count;
        Ok(restored)
    }

    /// Marks a transaction ready for undo and returns inverse actions in
    /// reverse application order. The caller must durably serialize the
    /// `UndoPending` state before applying any action. Reopening that archive
    /// and calling this method again returns the same batch for recovery.
    pub fn prepare_undo_transaction(
        &mut self,
        transaction_id: TransactionId,
        caller: ActivityContext,
    ) -> Result<UndoBatch, HistoryError> {
        let mut found = false;
        let mut state = None;
        for entry in self.entries.iter().take(self.length).flatten() {
            if entry.record.transaction_id != transaction_id {
                continue;
            }
            found = true;
            if !same_caller(entry.record.context, caller) {
                return Err(HistoryError::Unauthorized);
            }
            match state {
                Some(previous) if previous != entry.record.transaction_state => {
                    return Err(HistoryError::InvalidTransaction)
                }
                _ => state = Some(entry.record.transaction_state),
            }
        }
        if !found {
            return Err(HistoryError::TransactionNotFound);
        }
        match state.ok_or(HistoryError::InvalidTransaction)? {
            TransactionState::Undone => return Err(HistoryError::TransactionAlreadyUndone),
            TransactionState::Prepared => return Err(HistoryError::TransactionNotCommitted),
            TransactionState::Committed | TransactionState::UndoPending => {}
        }

        for entry in self.entries.iter_mut().take(self.length).flatten() {
            if entry.record.transaction_id == transaction_id {
                entry.record.transaction_state = TransactionState::UndoPending;
            }
        }

        let mut batch = UndoBatch {
            transaction_id,
            actions: [None; MAX_RECORDS],
            length: 0,
        };
        for entry in self.entries.iter().take(self.length).flatten().rev() {
            if entry.record.transaction_id == transaction_id {
                batch.actions[batch.length] = Some(inverse_of(*entry));
                batch.length += 1;
            }
        }
        if batch.is_empty() {
            return Err(HistoryError::InvalidTransaction);
        }
        Ok(batch)
    }

    /// Commits a prepared group after every forward operation succeeds. The
    /// caller must persist the new archive before exposing undo for it.
    pub fn commit_transaction(
        &mut self,
        transaction_id: TransactionId,
        caller: ActivityContext,
    ) -> Result<(), HistoryError> {
        let mut found = false;
        for entry in self.entries.iter().take(self.length).flatten() {
            if entry.record.transaction_id != transaction_id {
                continue;
            }
            found = true;
            if !same_caller(entry.record.context, caller) {
                return Err(HistoryError::Unauthorized);
            }
            if entry.record.transaction_state != TransactionState::Prepared {
                return Err(
                    if entry.record.transaction_state == TransactionState::Committed {
                        HistoryError::InvalidTransaction
                    } else {
                        HistoryError::TransactionNotPrepared
                    },
                );
            }
        }
        if !found {
            return Err(HistoryError::TransactionNotFound);
        }
        for entry in self.entries.iter_mut().take(self.length).flatten() {
            if entry.record.transaction_id == transaction_id {
                entry.record.transaction_state = TransactionState::Committed;
            }
        }
        Ok(())
    }

    /// Marks a prepared composite undo complete. Identity is checked against
    /// the transaction's originating AppId and AppSessionId; NodeId remains
    /// audit context and never supplies or strengthens authority.
    pub fn complete_undo_transaction(
        &mut self,
        transaction_id: TransactionId,
        caller: ActivityContext,
    ) -> Result<(), HistoryError> {
        let mut found = false;
        for entry in self.entries.iter().take(self.length).flatten() {
            if entry.record.transaction_id != transaction_id {
                continue;
            }
            found = true;
            if !same_caller(entry.record.context, caller) {
                return Err(HistoryError::Unauthorized);
            }
            if entry.record.transaction_state != TransactionState::UndoPending {
                return Err(
                    if entry.record.transaction_state == TransactionState::Undone {
                        HistoryError::TransactionAlreadyUndone
                    } else {
                        HistoryError::UndoNotPrepared
                    },
                );
            }
        }
        if !found {
            return Err(HistoryError::TransactionNotFound);
        }
        for entry in self.entries.iter_mut().take(self.length).flatten() {
            if entry.record.transaction_id == transaction_id {
                entry.record.transaction_state = TransactionState::Undone;
            }
        }
        Ok(())
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
        if self.next_sequence == u64::MAX || self.next_transaction_id == u64::MAX {
            return Err(HistoryError::Capacity);
        }
        let transaction_id = TransactionId(self.next_transaction_id);
        self.next_transaction_id += 1;
        self.push_with_snapshots(
            transaction_id,
            context,
            operation,
            object_id,
            before_name,
            after_name,
            before,
            after,
            TransactionState::Committed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn push_with_transaction(
        &mut self,
        transaction_id: TransactionId,
        context: ActivityContext,
        operation: Operation,
        object_id: ObjectId,
        before_name: Name,
        after_name: Name,
        before: &[u8],
        after: &[u8],
        transaction_state: TransactionState,
    ) -> Result<(), HistoryError> {
        let before = Snapshot::from_bytes(before)?;
        let after = Snapshot::from_bytes(after)?;
        self.push_with_snapshots(
            transaction_id,
            context,
            operation,
            object_id,
            before_name,
            after_name,
            before,
            after,
            transaction_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn push_with_snapshots(
        &mut self,
        transaction_id: TransactionId,
        context: ActivityContext,
        operation: Operation,
        object_id: ObjectId,
        before_name: Name,
        after_name: Name,
        before: Snapshot,
        after: Snapshot,
        transaction_state: TransactionState,
    ) -> Result<(), HistoryError> {
        if self.length == MAX_RECORDS
            || self.next_sequence == u64::MAX
            || transaction_id.0 == 0
            || transaction_id.0 >= self.next_transaction_id
        {
            return Err(HistoryError::Capacity);
        }
        for existing in self.entries.iter().take(self.length).flatten() {
            if existing.record.transaction_id == transaction_id
                && (existing.record.transaction_state != transaction_state
                    || !same_caller(existing.record.context, context))
            {
                return Err(HistoryError::InvalidTransaction);
            }
        }
        self.entries[self.length] = Some(Entry {
            record: HistoryRecord {
                sequence: self.next_sequence,
                transaction_id,
                transaction_state,
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
        self.next_sequence += 1;
        Ok(())
    }

    fn archive_payload_length(&self) -> Result<usize, HistoryError> {
        self.entries
            .iter()
            .take(self.length)
            .flatten()
            .try_fold(0usize, |length, entry| {
                let before_name = usize::from(entry.before_name.length);
                let after_name = usize::from(entry.after_name.length);
                length
                    .checked_add(
                        74 + before_name + after_name + entry.before.length + entry.after.length,
                    )
                    .ok_or(HistoryError::Capacity)
            })
    }
}

fn inverse_of(entry: Entry) -> UndoAction {
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
    UndoAction {
        operation,
        object_id: entry.record.object_id,
        from_name: entry.after_name,
        to_name: entry.before_name,
        content: content.content,
        content_length,
    }
}

fn same_caller(recorded: ActivityContext, caller: ActivityContext) -> bool {
    recorded.app_id == caller.app_id && recorded.app_session_id == caller.app_session_id
}

fn encode_entry(entry: &Entry, output: &mut [u8], offset: &mut usize) -> Result<(), HistoryError> {
    let record = entry.record;
    write_u64(output, offset, record.sequence)?;
    write_u64(output, offset, record.transaction_id.0)?;
    write_u8(output, offset, record.operation.code())?;
    write_u8(output, offset, record.transaction_state as u8)?;
    write_u64(output, offset, record.context.app_id.0)?;
    write_u64(output, offset, record.context.app_session_id.0)?;
    write_u64(output, offset, record.context.node_id.0)?;
    write_optional_u64(
        output,
        offset,
        record.context.surface_id.map(|value| value.0),
    )?;
    write_optional_u64(
        output,
        offset,
        record.context.workspace_id.map(|value| value.0),
    )?;
    write_u64(output, offset, record.object_id.0)?;
    let (before_name, before_name_len) = entry.before_name.bytes();
    write_u8(output, offset, before_name_len as u8)?;
    write_bytes(output, offset, &before_name[..before_name_len])?;
    let (after_name, after_name_len) = entry.after_name.bytes();
    write_u8(output, offset, after_name_len as u8)?;
    write_bytes(output, offset, &after_name[..after_name_len])?;
    write_u16(output, offset, entry.before.length as u16)?;
    write_bytes(output, offset, &entry.before.content[..entry.before.length])?;
    write_u16(output, offset, entry.after.length as u16)?;
    write_bytes(output, offset, &entry.after.content[..entry.after.length])?;
    Ok(())
}

fn decode_entry(input: &[u8], offset: &mut usize) -> Result<Entry, HistoryError> {
    let sequence = read_u64_advance(input, offset)?;
    let transaction_id = TransactionId(read_u64_advance(input, offset)?);
    let operation = Operation::from_code(read_u8_advance(input, offset)?)
        .ok_or(HistoryError::CorruptArchive)?;
    let transaction_state = TransactionState::from_code(read_u8_advance(input, offset)?)
        .ok_or(HistoryError::CorruptArchive)?;
    let app_id = AppId(read_u64_advance(input, offset)?);
    let app_session_id = AppSessionId(read_u64_advance(input, offset)?);
    let node_id = NodeId(read_u64_advance(input, offset)?);
    let surface_id = read_optional_u64(input, offset)?.map(SurfaceId);
    let workspace_id = read_optional_u64(input, offset)?.map(WorkspaceId);
    let object_id = ObjectId(read_u64_advance(input, offset)?);
    let before_name = read_name(input, offset)?;
    let after_name = read_name(input, offset)?;
    let before_length = usize::from(read_u16_advance(input, offset)?);
    let before_bytes = read_bytes_advance(input, offset, before_length)?;
    let before = Snapshot::from_bytes(before_bytes).map_err(|_| HistoryError::CorruptArchive)?;
    let after_length = usize::from(read_u16_advance(input, offset)?);
    let after_bytes = read_bytes_advance(input, offset, after_length)?;
    let after = Snapshot::from_bytes(after_bytes).map_err(|_| HistoryError::CorruptArchive)?;
    Ok(Entry {
        record: HistoryRecord {
            sequence,
            transaction_id,
            transaction_state,
            operation,
            context: ActivityContext {
                app_id,
                app_session_id,
                node_id,
                surface_id,
                workspace_id,
            },
            object_id,
            before_length: before.length as u16,
            after_length: after.length as u16,
        },
        before_name,
        after_name,
        before,
        after,
    })
}

fn read_name(input: &[u8], offset: &mut usize) -> Result<Name, HistoryError> {
    let length = usize::from(read_u8_advance(input, offset)?);
    let bytes = read_bytes_advance(input, offset, length)?;
    Name::new(bytes).map_err(|_| HistoryError::CorruptArchive)
}

fn write_optional_u64(
    output: &mut [u8],
    offset: &mut usize,
    value: Option<u64>,
) -> Result<(), HistoryError> {
    write_u8(output, offset, u8::from(value.is_some()))?;
    write_u64(output, offset, value.unwrap_or(0))
}

fn read_optional_u64(input: &[u8], offset: &mut usize) -> Result<Option<u64>, HistoryError> {
    let present = read_u8_advance(input, offset)?;
    let value = read_u64_advance(input, offset)?;
    match (present, value) {
        (0, 0) => Ok(None),
        (1, value) => Ok(Some(value)),
        _ => Err(HistoryError::CorruptArchive),
    }
}

fn write_u8(output: &mut [u8], offset: &mut usize, value: u8) -> Result<(), HistoryError> {
    write_bytes(output, offset, &[value])
}

fn write_u16(output: &mut [u8], offset: &mut usize, value: u16) -> Result<(), HistoryError> {
    write_bytes(output, offset, &value.to_le_bytes())
}

fn write_u64(output: &mut [u8], offset: &mut usize, value: u64) -> Result<(), HistoryError> {
    write_bytes(output, offset, &value.to_le_bytes())
}

fn write_bytes(output: &mut [u8], offset: &mut usize, bytes: &[u8]) -> Result<(), HistoryError> {
    let end = offset
        .checked_add(bytes.len())
        .filter(|end| *end <= output.len())
        .ok_or(HistoryError::BufferTooSmall)?;
    output[*offset..end].copy_from_slice(bytes);
    *offset = end;
    Ok(())
}

fn read_u8_advance(input: &[u8], offset: &mut usize) -> Result<u8, HistoryError> {
    let bytes = read_bytes_advance(input, offset, 1)?;
    Ok(bytes[0])
}

fn read_u16_advance(input: &[u8], offset: &mut usize) -> Result<u16, HistoryError> {
    let bytes = read_bytes_advance(input, offset, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u64_advance(input: &[u8], offset: &mut usize) -> Result<u64, HistoryError> {
    let bytes = read_bytes_advance(input, offset, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_bytes_advance<'a>(
    input: &'a [u8],
    offset: &mut usize,
    length: usize,
) -> Result<&'a [u8], HistoryError> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= input.len())
        .ok_or(HistoryError::CorruptArchive)?;
    let bytes = &input[*offset..end];
    *offset = end;
    Ok(bytes)
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16, HistoryError> {
    let bytes = input
        .get(offset..offset + 2)
        .ok_or(HistoryError::CorruptArchive)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(input: &[u8], offset: usize) -> Result<u32, HistoryError> {
    let bytes = input
        .get(offset..offset + 4)
        .ok_or(HistoryError::CorruptArchive)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(input: &[u8], offset: usize) -> Result<u64, HistoryError> {
    let bytes = input
        .get(offset..offset + 8)
        .ok_or(HistoryError::CorruptArchive)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

pub(crate) fn archive_checksum(bytes: &[u8]) -> u64 {
    bytes
        .get(..28)
        .unwrap_or(&[])
        .iter()
        .chain(bytes.get(ARCHIVE_HEADER_BYTES..).unwrap_or(&[]).iter())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
        })
}

impl Default for HistoryService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActivityContext, AppId, AppSessionId, HistoryError, HistoryService, MoveRecord, NodeId,
        ObjectId, Operation, SurfaceId, UndoOperation, WorkspaceId,
    };
    use std::vec::Vec;

    const CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(7),
        app_session_id: AppSessionId(8),
        node_id: NodeId(9),
        surface_id: None,
        workspace_id: Some(WorkspaceId(10)),
    };
    const M22_CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(0x1020_3040_5060_7080),
        app_session_id: AppSessionId(0x2030_4050_6070_8090),
        node_id: NodeId(0x3040_5060_7080_9010),
        surface_id: Some(SurfaceId(0x4050_6070_8090_a0b0)),
        workspace_id: Some(WorkspaceId(0x5060_7080_90a0_b0c0)),
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

    #[test]
    fn grouped_move_undo_is_caller_scoped_and_survives_archive_restore() {
        let mut history = HistoryService::new();
        let transaction = history
            .record_move_group(
                M22_CONTEXT,
                &[
                    MoveRecord {
                        object_id: ObjectId(101),
                        from_name: b"one",
                        to_name: b"moved-one",
                    },
                    MoveRecord {
                        object_id: ObjectId(102),
                        from_name: b"two",
                        to_name: b"moved-two",
                    },
                    MoveRecord {
                        object_id: ObjectId(103),
                        from_name: b"three",
                        to_name: b"moved-three",
                    },
                ],
            )
            .unwrap();

        let mut archive = [0; 36 * 1024];
        let length = history.serialize_recoverable(&mut archive).unwrap();
        let mut restored = HistoryService::restore_recoverable(&archive[..length]).unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored.record_at(0).unwrap().context, M22_CONTEXT);
        assert_eq!(restored.record_at(0).unwrap().transaction_id, transaction);

        let wrong_caller = ActivityContext {
            app_id: AppId(88),
            ..M22_CONTEXT
        };
        assert_eq!(
            restored.prepare_undo_transaction(transaction, wrong_caller),
            Err(HistoryError::Unauthorized)
        );
        assert_eq!(
            restored.prepare_undo_transaction(transaction, M22_CONTEXT),
            Err(HistoryError::TransactionNotCommitted)
        );
        assert_eq!(
            restored.commit_transaction(transaction, wrong_caller),
            Err(HistoryError::Unauthorized)
        );
        restored
            .commit_transaction(transaction, M22_CONTEXT)
            .unwrap();

        let batch = restored
            .prepare_undo_transaction(transaction, M22_CONTEXT)
            .unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(
            batch
                .actions()
                .map(|action| action.object_id)
                .collect::<Vec<_>>(),
            [ObjectId(103), ObjectId(102), ObjectId(101)]
        );

        let pending_length = restored.serialize_recoverable(&mut archive).unwrap();
        let mut after_restart =
            HistoryService::restore_recoverable(&archive[..pending_length]).unwrap();
        let resumed = after_restart
            .prepare_undo_transaction(transaction, M22_CONTEXT)
            .unwrap();
        assert_eq!(
            resumed.actions().collect::<Vec<_>>(),
            batch.actions().collect::<Vec<_>>()
        );
        after_restart
            .complete_undo_transaction(transaction, M22_CONTEXT)
            .unwrap();
        assert_eq!(
            after_restart.prepare_undo_transaction(transaction, M22_CONTEXT),
            Err(HistoryError::TransactionAlreadyUndone)
        );
    }

    #[test]
    fn recoverable_archive_rejects_corruption_versions_and_small_buffers() {
        let mut history = HistoryService::new();
        history
            .record_edit(CONTEXT, ObjectId(5), b"note", b"before", b"after")
            .unwrap();
        let mut archive = [0; 36 * 1024];
        let length = history.serialize_recoverable(&mut archive).unwrap();

        let mut unchanged = [0xa5; 8];
        assert_eq!(
            history.serialize_recoverable(&mut unchanged),
            Err(HistoryError::BufferTooSmall)
        );
        assert_eq!(unchanged, [0xa5; 8]);

        archive[0] ^= 1;
        assert_eq!(
            HistoryService::restore_recoverable(&archive[..length]).unwrap_err(),
            HistoryError::CorruptArchive
        );
        archive[0] ^= 1;
        archive[4] = 2;
        assert_eq!(
            HistoryService::restore_recoverable(&archive[..length]).unwrap_err(),
            HistoryError::UnsupportedArchiveVersion(2)
        );
        archive[4] = 1;
        archive[8] ^= 1;
        assert_eq!(
            HistoryService::restore_recoverable(&archive[..length]).unwrap_err(),
            HistoryError::CorruptArchive
        );
    }

    #[test]
    fn invalid_move_group_is_rejected_without_a_partial_transaction() {
        let mut history = HistoryService::new();
        assert_eq!(
            history.record_move_group(
                CONTEXT,
                &[
                    MoveRecord {
                        object_id: ObjectId(1),
                        from_name: b"valid",
                        to_name: b"moved",
                    },
                    MoveRecord {
                        object_id: ObjectId(2),
                        from_name: b"x",
                        to_name: &[b'x'; super::MAX_NAME_BYTES + 1],
                    },
                ]
            ),
            Err(HistoryError::NameTooLong)
        );
        assert!(history.is_empty());
        let transaction = history
            .record_move_group(
                CONTEXT,
                &[MoveRecord {
                    object_id: ObjectId(3),
                    from_name: b"source",
                    to_name: b"destination",
                }],
            )
            .unwrap();
        assert_eq!(transaction, super::TransactionId(1));
    }
}
