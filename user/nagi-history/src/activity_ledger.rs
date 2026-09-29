//! Bounded AI activity records, separate from NH16 undo history.
//!
//! The archive records bounded intent, model selection, plan/action summary,
//! resolved object context, transaction ID, and result transitions. It has no
//! field for chain-of-thought, credentials, or unrestricted context.

use crate::guest::{ArchiveSlot, GUEST_ARCHIVE_FILE_BYTES, MAX_GUEST_ARCHIVE_BYTES};
use crate::{
    ActivityContext, AppId, AppSessionId, NodeId, ObjectId, SurfaceId, TransactionId, WorkspaceId,
};

const ARCHIVE_HEADER_BYTES: usize = 36;
const ARCHIVE_VERSION: u16 = 1;
const SLOT_HEADER_BYTES: usize = 36;
const SLOT_VERSION: u16 = 1;
const SLOT_MAGIC: &[u8; 4] = b"NLA1";

pub const MAX_ACTIVITY_RECORDS: usize = 4;
pub const MAX_ACTIVITY_OBJECTS: usize = 4;
pub const MAX_ACTIVITY_TRANSITIONS: usize = 4;
pub const MAX_USER_INTENT_BYTES: usize = 48;
pub const MAX_MODEL_ID_BYTES: usize = 24;
pub const MAX_ACTION_ID_BYTES: usize = 24;
pub const MAX_PLAN_SUMMARY_BYTES: usize = 32;
pub const MAX_ACTIVITY_ARCHIVE_BYTES: usize = MAX_GUEST_ARCHIVE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityLedgerError {
    Capacity,
    TextTooLong,
    InvalidText,
    TooManyObjects,
    BufferTooSmall,
    CorruptArchive,
    UnsupportedArchiveVersion(u16),
    RecordNotFound,
    DuplicateTransaction,
    InvalidTransition,
    TransitionCapacity,
    Storage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ActivityOutcome {
    Prepared = 1,
    Committed = 2,
    Denied = 3,
    Failed = 4,
    UndoPending = 5,
    Undone = 6,
}

impl ActivityOutcome {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Prepared),
            2 => Some(Self::Committed),
            3 => Some(Self::Denied),
            4 => Some(Self::Failed),
            5 => Some(Self::UndoPending),
            6 => Some(Self::Undone),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BoundedText<const N: usize> {
    bytes: [u8; N],
    length: u8,
}

impl<const N: usize> BoundedText<N> {
    fn new(value: &str, allow_empty: bool) -> Result<Self, ActivityLedgerError> {
        Self::from_bytes(value.as_bytes(), allow_empty)
    }

    fn from_bytes(value: &[u8], allow_empty: bool) -> Result<Self, ActivityLedgerError> {
        if value.len() > N {
            return Err(ActivityLedgerError::TextTooLong);
        }
        if (!allow_empty && value.is_empty()) || core::str::from_utf8(value).is_err() {
            return Err(ActivityLedgerError::InvalidText);
        }
        let mut bytes = [0; N];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }

    fn as_str(&self) -> &str {
        // Construction and archive restoration both validate UTF-8.
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

#[derive(Clone, Copy)]
pub struct ActivityRecordInput<'a> {
    pub occurred_at: u64,
    pub context: ActivityContext,
    pub transaction_id: Option<TransactionId>,
    pub user_intent: &'a str,
    pub selected_model: Option<&'a str>,
    pub action_id: &'a str,
    pub plan_summary: &'a str,
    pub object_ids: &'a [ObjectId],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityRecord {
    sequence: u64,
    occurred_at: u64,
    context: ActivityContext,
    transaction_id: Option<TransactionId>,
    user_intent: BoundedText<MAX_USER_INTENT_BYTES>,
    selected_model: BoundedText<MAX_MODEL_ID_BYTES>,
    action_id: BoundedText<MAX_ACTION_ID_BYTES>,
    plan_summary: BoundedText<MAX_PLAN_SUMMARY_BYTES>,
    object_ids: [ObjectId; MAX_ACTIVITY_OBJECTS],
    object_count: u8,
    outcomes: [ActivityOutcome; MAX_ACTIVITY_TRANSITIONS],
    outcome_count: u8,
}

impl ActivityRecord {
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn occurred_at(&self) -> u64 {
        self.occurred_at
    }

    pub const fn context(&self) -> ActivityContext {
        self.context
    }

    pub const fn transaction_id(&self) -> Option<TransactionId> {
        self.transaction_id
    }

    pub fn user_intent(&self) -> &str {
        self.user_intent.as_str()
    }

    pub fn selected_model(&self) -> Option<&str> {
        (!self.selected_model.as_bytes().is_empty()).then(|| self.selected_model.as_str())
    }

    pub fn action_id(&self) -> &str {
        self.action_id.as_str()
    }

    pub fn plan_summary(&self) -> &str {
        self.plan_summary.as_str()
    }

    pub fn object_ids(&self) -> &[ObjectId] {
        &self.object_ids[..usize::from(self.object_count)]
    }

    pub fn outcomes(&self) -> &[ActivityOutcome] {
        &self.outcomes[..usize::from(self.outcome_count)]
    }

    pub fn current_outcome(&self) -> ActivityOutcome {
        self.outcomes[usize::from(self.outcome_count) - 1]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityLedger {
    records: [Option<ActivityRecord>; MAX_ACTIVITY_RECORDS],
    length: usize,
    next_sequence: u64,
}

impl ActivityLedger {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_ACTIVITY_RECORDS],
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

    pub fn record_at(&self, index: usize) -> Option<&ActivityRecord> {
        self.records.get(index).and_then(Option::as_ref)
    }

    pub fn record_for_transaction(&self, transaction_id: TransactionId) -> Option<&ActivityRecord> {
        self.records[..self.length]
            .iter()
            .flatten()
            .find(|record| record.transaction_id == Some(transaction_id))
    }

    /// Adds a bounded plan/action record in `Prepared` state.
    pub fn record_action(
        &mut self,
        input: ActivityRecordInput<'_>,
    ) -> Result<u64, ActivityLedgerError> {
        if self.length == MAX_ACTIVITY_RECORDS || self.next_sequence == u64::MAX {
            return Err(ActivityLedgerError::Capacity);
        }
        if input.object_ids.len() > MAX_ACTIVITY_OBJECTS {
            return Err(ActivityLedgerError::TooManyObjects);
        }
        if input
            .transaction_id
            .is_some_and(|id| self.record_for_transaction(id).is_some())
        {
            return Err(ActivityLedgerError::DuplicateTransaction);
        }

        let user_intent = BoundedText::new(input.user_intent, false)?;
        let selected_model = BoundedText::new(input.selected_model.unwrap_or(""), true)?;
        let action_id = BoundedText::new(input.action_id, false)?;
        let plan_summary = BoundedText::new(input.plan_summary, false)?;
        let mut object_ids = [ObjectId(0); MAX_ACTIVITY_OBJECTS];
        object_ids[..input.object_ids.len()].copy_from_slice(input.object_ids);

        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.records[self.length] = Some(ActivityRecord {
            sequence,
            occurred_at: input.occurred_at,
            context: input.context,
            transaction_id: input.transaction_id,
            user_intent,
            selected_model,
            action_id,
            plan_summary,
            object_ids,
            object_count: input.object_ids.len() as u8,
            outcomes: [ActivityOutcome::Prepared; MAX_ACTIVITY_TRANSITIONS],
            outcome_count: 1,
        });
        self.length += 1;
        Ok(sequence)
    }

    /// Persists a monotonic result transition for an existing action record.
    pub fn transition(
        &mut self,
        sequence: u64,
        next: ActivityOutcome,
    ) -> Result<(), ActivityLedgerError> {
        let record = self.records[..self.length]
            .iter_mut()
            .flatten()
            .find(|record| record.sequence == sequence)
            .ok_or(ActivityLedgerError::RecordNotFound)?;
        if usize::from(record.outcome_count) == MAX_ACTIVITY_TRANSITIONS {
            return Err(ActivityLedgerError::TransitionCapacity);
        }
        if !valid_transition(record.current_outcome(), next) {
            return Err(ActivityLedgerError::InvalidTransition);
        }
        record.outcomes[usize::from(record.outcome_count)] = next;
        record.outcome_count += 1;
        Ok(())
    }

    pub fn serialize_recoverable(&self, output: &mut [u8]) -> Result<usize, ActivityLedgerError> {
        let mut payload_length = 0usize;
        for record in self.records[..self.length].iter().flatten() {
            payload_length = payload_length
                .checked_add(record_encoded_length(record))
                .ok_or(ActivityLedgerError::Capacity)?;
        }
        let required = ARCHIVE_HEADER_BYTES
            .checked_add(payload_length)
            .filter(|length| *length <= MAX_ACTIVITY_ARCHIVE_BYTES)
            .ok_or(ActivityLedgerError::Capacity)?;
        if output.len() < required {
            return Err(ActivityLedgerError::BufferTooSmall);
        }
        output[..required].fill(0);
        output[..4].copy_from_slice(b"NAL1");
        output[4..6].copy_from_slice(&ARCHIVE_VERSION.to_le_bytes());
        output[6..8].copy_from_slice(&(self.length as u16).to_le_bytes());
        output[8..16].copy_from_slice(&self.next_sequence.to_le_bytes());
        output[16..20].copy_from_slice(&(payload_length as u32).to_le_bytes());

        let mut offset = ARCHIVE_HEADER_BYTES;
        for record in self.records[..self.length].iter().flatten() {
            encode_record(record, output, &mut offset);
        }
        if offset != required {
            return Err(ActivityLedgerError::CorruptArchive);
        }
        let checksum = crate::archive_checksum(&output[..required]);
        output[28..36].copy_from_slice(&checksum.to_le_bytes());
        Ok(required)
    }

    pub fn restore_recoverable(input: &[u8]) -> Result<Self, ActivityLedgerError> {
        if input.len() < ARCHIVE_HEADER_BYTES || input.len() > MAX_ACTIVITY_ARCHIVE_BYTES {
            return Err(ActivityLedgerError::CorruptArchive);
        }
        if input.get(..4) != Some(b"NAL1") {
            return Err(ActivityLedgerError::CorruptArchive);
        }
        let version = read_u16(input, 4).ok_or(ActivityLedgerError::CorruptArchive)?;
        if version != ARCHIVE_VERSION {
            return Err(ActivityLedgerError::UnsupportedArchiveVersion(version));
        }
        let count = usize::from(read_u16(input, 6).ok_or(ActivityLedgerError::CorruptArchive)?);
        let next_sequence = read_u64(input, 8).ok_or(ActivityLedgerError::CorruptArchive)?;
        let payload_length =
            usize::try_from(read_u32(input, 16).ok_or(ActivityLedgerError::CorruptArchive)?)
                .map_err(|_| ActivityLedgerError::CorruptArchive)?;
        let expected_checksum = read_u64(input, 28).ok_or(ActivityLedgerError::CorruptArchive)?;
        if count > MAX_ACTIVITY_RECORDS
            || next_sequence == 0
            || input[20..28].iter().any(|byte| *byte != 0)
            || payload_length != input.len() - ARCHIVE_HEADER_BYTES
            || crate::archive_checksum(input) != expected_checksum
        {
            return Err(ActivityLedgerError::CorruptArchive);
        }

        let mut restored = Self::new();
        restored.next_sequence = next_sequence;
        let mut offset = ARCHIVE_HEADER_BYTES;
        let mut previous_sequence = 0;
        for index in 0..count {
            let record = decode_record(input, &mut offset)?;
            if record.sequence <= previous_sequence || record.sequence >= next_sequence {
                return Err(ActivityLedgerError::CorruptArchive);
            }
            if let Some(transaction_id) = record.transaction_id {
                if restored.record_for_transaction(transaction_id).is_some() {
                    return Err(ActivityLedgerError::CorruptArchive);
                }
            }
            previous_sequence = record.sequence;
            restored.records[index] = Some(record);
        }
        if offset != input.len() {
            return Err(ActivityLedgerError::CorruptArchive);
        }
        restored.length = count;
        Ok(restored)
    }
}

impl Default for ActivityLedger {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_transition(current: ActivityOutcome, next: ActivityOutcome) -> bool {
    matches!(
        (current, next),
        (ActivityOutcome::Prepared, ActivityOutcome::Committed)
            | (ActivityOutcome::Prepared, ActivityOutcome::Denied)
            | (ActivityOutcome::Prepared, ActivityOutcome::Failed)
            | (ActivityOutcome::Committed, ActivityOutcome::UndoPending)
            | (ActivityOutcome::UndoPending, ActivityOutcome::Undone)
    )
}

fn record_encoded_length(record: &ActivityRecord) -> usize {
    16 + 42
        + 9
        + 6
        + usize::from(record.outcome_count)
        + usize::from(record.object_count) * 8
        + record.user_intent.as_bytes().len()
        + record.selected_model.as_bytes().len()
        + record.action_id.as_bytes().len()
        + record.plan_summary.as_bytes().len()
}

fn encode_record(record: &ActivityRecord, output: &mut [u8], offset: &mut usize) {
    write_u64(output, offset, record.sequence);
    write_u64(output, offset, record.occurred_at);
    write_u64(output, offset, record.context.app_id.0);
    write_u64(output, offset, record.context.app_session_id.0);
    write_u64(output, offset, record.context.node_id.0);
    write_optional_id(output, offset, record.context.surface_id.map(|id| id.0));
    write_optional_id(output, offset, record.context.workspace_id.map(|id| id.0));
    write_optional_id(output, offset, record.transaction_id.map(|id| id.0));
    output[*offset] = record.outcome_count;
    output[*offset + 1] = record.object_count;
    output[*offset + 2] = record.user_intent.length;
    output[*offset + 3] = record.selected_model.length;
    output[*offset + 4] = record.action_id.length;
    output[*offset + 5] = record.plan_summary.length;
    *offset += 6;
    for outcome in record.outcomes() {
        output[*offset] = *outcome as u8;
        *offset += 1;
    }
    for object_id in record.object_ids() {
        write_u64(output, offset, object_id.0);
    }
    write_bytes(output, offset, record.user_intent.as_bytes());
    write_bytes(output, offset, record.selected_model.as_bytes());
    write_bytes(output, offset, record.action_id.as_bytes());
    write_bytes(output, offset, record.plan_summary.as_bytes());
}

fn decode_record(input: &[u8], offset: &mut usize) -> Result<ActivityRecord, ActivityLedgerError> {
    let sequence = take_u64(input, offset)?;
    let occurred_at = take_u64(input, offset)?;
    let context = ActivityContext {
        app_id: AppId(take_u64(input, offset)?),
        app_session_id: AppSessionId(take_u64(input, offset)?),
        node_id: NodeId(take_u64(input, offset)?),
        surface_id: take_optional_id(input, offset)?.map(SurfaceId),
        workspace_id: take_optional_id(input, offset)?.map(WorkspaceId),
    };
    let transaction_id = take_optional_id(input, offset)?.map(TransactionId);
    let outcome_count = usize::from(take_u8(input, offset)?);
    let object_count = usize::from(take_u8(input, offset)?);
    let intent_length = usize::from(take_u8(input, offset)?);
    let model_length = usize::from(take_u8(input, offset)?);
    let action_length = usize::from(take_u8(input, offset)?);
    let summary_length = usize::from(take_u8(input, offset)?);
    if outcome_count == 0
        || outcome_count > MAX_ACTIVITY_TRANSITIONS
        || object_count > MAX_ACTIVITY_OBJECTS
        || intent_length > MAX_USER_INTENT_BYTES
        || model_length > MAX_MODEL_ID_BYTES
        || action_length == 0
        || action_length > MAX_ACTION_ID_BYTES
        || summary_length == 0
        || summary_length > MAX_PLAN_SUMMARY_BYTES
    {
        return Err(ActivityLedgerError::CorruptArchive);
    }

    let mut outcomes = [ActivityOutcome::Prepared; MAX_ACTIVITY_TRANSITIONS];
    for index in 0..outcome_count {
        outcomes[index] = ActivityOutcome::from_code(take_u8(input, offset)?)
            .ok_or(ActivityLedgerError::CorruptArchive)?;
        if (index == 0 && outcomes[index] != ActivityOutcome::Prepared)
            || (index > 0 && !valid_transition(outcomes[index - 1], outcomes[index]))
        {
            return Err(ActivityLedgerError::CorruptArchive);
        }
    }

    let mut object_ids = [ObjectId(0); MAX_ACTIVITY_OBJECTS];
    for object_id in object_ids.iter_mut().take(object_count) {
        *object_id = ObjectId(take_u64(input, offset)?);
    }
    let user_intent = take_text(input, offset, intent_length, false)?;
    let selected_model = take_text(input, offset, model_length, true)?;
    let action_id = take_text(input, offset, action_length, false)?;
    let plan_summary = take_text(input, offset, summary_length, false)?;
    Ok(ActivityRecord {
        sequence,
        occurred_at,
        context,
        transaction_id,
        user_intent,
        selected_model,
        action_id,
        plan_summary,
        object_ids,
        object_count: object_count as u8,
        outcomes,
        outcome_count: outcome_count as u8,
    })
}

fn take_text<const N: usize>(
    input: &[u8],
    offset: &mut usize,
    length: usize,
    allow_empty: bool,
) -> Result<BoundedText<N>, ActivityLedgerError> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= input.len())
        .ok_or(ActivityLedgerError::CorruptArchive)?;
    let text = BoundedText::from_bytes(&input[*offset..end], allow_empty)
        .map_err(|_| ActivityLedgerError::CorruptArchive)?;
    *offset = end;
    Ok(text)
}

fn take_optional_id(input: &[u8], offset: &mut usize) -> Result<Option<u64>, ActivityLedgerError> {
    match take_u8(input, offset)? {
        0 => Ok(None),
        1 => Ok(Some(take_u64(input, offset)?)),
        _ => Err(ActivityLedgerError::CorruptArchive),
    }
}

fn write_optional_id(output: &mut [u8], offset: &mut usize, value: Option<u64>) {
    output[*offset] = u8::from(value.is_some());
    *offset += 1;
    if let Some(value) = value {
        write_u64(output, offset, value);
    }
}

fn write_u64(output: &mut [u8], offset: &mut usize, value: u64) {
    output[*offset..*offset + 8].copy_from_slice(&value.to_le_bytes());
    *offset += 8;
}

fn write_bytes(output: &mut [u8], offset: &mut usize, value: &[u8]) {
    output[*offset..*offset + value.len()].copy_from_slice(value);
    *offset += value.len();
}

fn take_u8(input: &[u8], offset: &mut usize) -> Result<u8, ActivityLedgerError> {
    let value = *input
        .get(*offset)
        .ok_or(ActivityLedgerError::CorruptArchive)?;
    *offset += 1;
    Ok(value)
}

fn take_u64(input: &[u8], offset: &mut usize) -> Result<u64, ActivityLedgerError> {
    let end = offset
        .checked_add(8)
        .filter(|end| *end <= input.len())
        .ok_or(ActivityLedgerError::CorruptArchive)?;
    let bytes = &input[*offset..end];
    *offset = end;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_u16(input: &[u8], offset: usize) -> Option<u16> {
    let bytes = input.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(input: &[u8], offset: usize) -> Option<u32> {
    let bytes = input.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(input: &[u8], offset: usize) -> Option<u64> {
    let bytes = input.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

/// VFS-facing two-slot store for the independent `NAL1` activity archive.
pub trait ActivityLedgerFileStore {
    fn read_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, ActivityLedgerError>;

    fn write_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        bytes: &[u8],
    ) -> Result<(), ActivityLedgerError>;

    fn flush_activity_slots(&mut self) -> Result<(), ActivityLedgerError>;
}

impl<T: ActivityLedgerFileStore + ?Sized> ActivityLedgerFileStore for &mut T {
    fn read_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, ActivityLedgerError> {
        (**self).read_activity_slot(slot, buffer)
    }

    fn write_activity_slot(
        &mut self,
        slot: ArchiveSlot,
        bytes: &[u8],
    ) -> Result<(), ActivityLedgerError> {
        (**self).write_activity_slot(slot, bytes)
    }

    fn flush_activity_slots(&mut self) -> Result<(), ActivityLedgerError> {
        (**self).flush_activity_slots()
    }
}

/// Crash-recoverable storage that shares the existing archive slot adapter
/// pattern while writing a distinct pair of AI ledger files.
pub struct ActivityLedgerArchiveBackend<F: ActivityLedgerFileStore> {
    files: F,
}

impl<F: ActivityLedgerFileStore> ActivityLedgerArchiveBackend<F> {
    pub const fn new(files: F) -> Self {
        Self { files }
    }

    pub fn file_store_mut(&mut self) -> &mut F {
        &mut self.files
    }

    pub fn into_file_store(self) -> F {
        self.files
    }

    pub fn load_archive(
        &mut self,
        output: &mut [u8],
    ) -> Result<Option<usize>, ActivityLedgerError> {
        let (a_present, a) = self.inspect_slot(ArchiveSlot::A)?;
        let (b_present, b) = self.inspect_slot(ArchiveSlot::B)?;
        let selected = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => Some((ArchiveSlot::B, b)),
            (Some(a), Some(_)) => Some((ArchiveSlot::A, a)),
            (Some(a), None) => Some((ArchiveSlot::A, a)),
            (None, Some(b)) => Some((ArchiveSlot::B, b)),
            (None, None) if a_present || b_present => {
                return Err(ActivityLedgerError::CorruptArchive)
            }
            (None, None) => return Ok(None),
        };
        let Some((slot, info)) = selected else {
            return Ok(None);
        };
        if output.len() < info.archive_length {
            return Err(ActivityLedgerError::BufferTooSmall);
        }
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let length = self
            .files
            .read_activity_slot(slot, &mut bytes)?
            .ok_or(ActivityLedgerError::CorruptArchive)?;
        if length > bytes.len() {
            return Err(ActivityLedgerError::CorruptArchive);
        }
        let decoded =
            decode_slot(slot, &bytes[..length]).ok_or(ActivityLedgerError::CorruptArchive)?;
        let end = SLOT_HEADER_BYTES + decoded.archive_length;
        output[..decoded.archive_length].copy_from_slice(&bytes[SLOT_HEADER_BYTES..end]);
        Ok(Some(decoded.archive_length))
    }

    pub fn write_archive(&mut self, archive: &[u8]) -> Result<(), ActivityLedgerError> {
        if archive.len() > MAX_GUEST_ARCHIVE_BYTES {
            return Err(ActivityLedgerError::Capacity);
        }
        ActivityLedger::restore_recoverable(archive)?;

        let (a_present, a) = self.inspect_slot(ArchiveSlot::A)?;
        let (b_present, b) = self.inspect_slot(ArchiveSlot::B)?;
        let (active_slot, generation) = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => (ArchiveSlot::B, b.generation),
            (Some(a), Some(_)) => (ArchiveSlot::A, a.generation),
            (Some(a), None) => (ArchiveSlot::A, a.generation),
            (None, Some(b)) => (ArchiveSlot::B, b.generation),
            (None, None) if a_present || b_present => {
                return Err(ActivityLedgerError::CorruptArchive)
            }
            (None, None) => (ArchiveSlot::B, 0),
        };
        let generation = generation
            .checked_add(1)
            .ok_or(ActivityLedgerError::Capacity)?;
        let target = active_slot.other();
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let length = encode_slot(target, generation, archive, &mut bytes)?;
        self.files.write_activity_slot(target, &bytes[..length])?;
        self.files.flush_activity_slots()
    }

    fn inspect_slot(
        &mut self,
        slot: ArchiveSlot,
    ) -> Result<(bool, Option<SlotInfo>), ActivityLedgerError> {
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let Some(length) = self.files.read_activity_slot(slot, &mut bytes)? else {
            return Ok((false, None));
        };
        if length > bytes.len() {
            return Ok((true, None));
        }
        Ok((true, decode_slot(slot, &bytes[..length])))
    }
}

#[derive(Clone, Copy)]
struct SlotInfo {
    generation: u64,
    archive_length: usize,
}

fn encode_slot(
    slot: ArchiveSlot,
    generation: u64,
    archive: &[u8],
    output: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
) -> Result<usize, ActivityLedgerError> {
    if generation == 0 || archive.len() > MAX_GUEST_ARCHIVE_BYTES {
        return Err(ActivityLedgerError::Capacity);
    }
    output[..4].copy_from_slice(SLOT_MAGIC);
    output[4..6].copy_from_slice(&SLOT_VERSION.to_le_bytes());
    output[6] = slot_index(slot);
    output[7] = 0;
    output[8..16].copy_from_slice(&generation.to_le_bytes());
    output[16..20].copy_from_slice(&(archive.len() as u32).to_le_bytes());
    output[20..28].copy_from_slice(&slot_checksum(archive).to_le_bytes());
    let header_checksum = slot_checksum(&output[..28]);
    output[28..36].copy_from_slice(&header_checksum.to_le_bytes());
    let end = SLOT_HEADER_BYTES + archive.len();
    output[SLOT_HEADER_BYTES..end].copy_from_slice(archive);
    Ok(end)
}

fn decode_slot(slot: ArchiveSlot, bytes: &[u8]) -> Option<SlotInfo> {
    if bytes.len() < SLOT_HEADER_BYTES
        || bytes.len() > GUEST_ARCHIVE_FILE_BYTES
        || bytes.get(..4) != Some(SLOT_MAGIC.as_slice())
        || read_u16(bytes, 4)? != SLOT_VERSION
        || *bytes.get(6)? != slot_index(slot)
        || *bytes.get(7)? != 0
        || read_u64(bytes, 28)? != slot_checksum(bytes.get(..28)?)
    {
        return None;
    }
    let generation = read_u64(bytes, 8)?;
    let archive_length = usize::try_from(read_u32(bytes, 16)?).ok()?;
    let end = SLOT_HEADER_BYTES.checked_add(archive_length)?;
    let archive = bytes.get(SLOT_HEADER_BYTES..end)?;
    if generation == 0
        || archive_length > MAX_GUEST_ARCHIVE_BYTES
        || end != bytes.len()
        || read_u64(bytes, 20)? != slot_checksum(archive)
        || ActivityLedger::restore_recoverable(archive).is_err()
    {
        return None;
    }
    Some(SlotInfo {
        generation,
        archive_length,
    })
}

const fn slot_index(slot: ArchiveSlot) -> u8 {
    match slot {
        ArchiveSlot::A => 0,
        ArchiveSlot::B => 1,
    }
}

fn slot_checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    const CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(0x1_0000_0001),
        app_session_id: AppSessionId(0x2_0000_0002),
        node_id: NodeId(0x3_0000_0003),
        surface_id: Some(SurfaceId(0x4_0000_0004)),
        workspace_id: Some(WorkspaceId(0x5_0000_0005)),
    };
    const OBJECTS: [ObjectId; 3] = [
        ObjectId(0x11_0000_0011),
        ObjectId(0x12_0000_0012),
        ObjectId(0x13_0000_0013),
    ];

    fn new_move_ledger() -> (ActivityLedger, u64) {
        let mut ledger = ActivityLedger::new();
        let sequence = ledger
            .record_action(ActivityRecordInput {
                occurred_at: 0x1_0000_0000,
                context: CONTEXT,
                transaction_id: Some(TransactionId(0x22_0000_0022)),
                user_intent: "move the three M22 fixture files",
                selected_model: Some("granite-4.2-3b"),
                action_id: "file.move",
                plan_summary: "destinations=m22-A,m22-B,m22-C",
                object_ids: &OBJECTS,
            })
            .expect("record bounded action");
        (ledger, sequence)
    }

    #[test]
    fn archive_round_trip_preserves_context_plan_model_transaction_and_results() {
        let (mut ledger, sequence) = new_move_ledger();
        ledger
            .transition(sequence, ActivityOutcome::Committed)
            .expect("commit result");
        ledger
            .transition(sequence, ActivityOutcome::UndoPending)
            .expect("undo begins");
        ledger
            .transition(sequence, ActivityOutcome::Undone)
            .expect("undo result");

        let mut bytes = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
        let length = ledger
            .serialize_recoverable(&mut bytes)
            .expect("serialize bounded archive");
        let restored =
            ActivityLedger::restore_recoverable(&bytes[..length]).expect("restore bounded archive");
        let record = restored.record_at(0).expect("record restored");
        assert_eq!(record.sequence(), sequence);
        assert_eq!(record.occurred_at(), 0x1_0000_0000);
        assert_eq!(record.context(), CONTEXT);
        assert_eq!(record.transaction_id(), Some(TransactionId(0x22_0000_0022)));
        assert_eq!(record.user_intent(), "move the three M22 fixture files");
        assert_eq!(record.selected_model(), Some("granite-4.2-3b"));
        assert_eq!(record.action_id(), "file.move");
        assert_eq!(record.plan_summary(), "destinations=m22-A,m22-B,m22-C");
        assert_eq!(record.object_ids(), OBJECTS);
        assert_eq!(
            record.outcomes(),
            &[
                ActivityOutcome::Prepared,
                ActivityOutcome::Committed,
                ActivityOutcome::UndoPending,
                ActivityOutcome::Undone,
            ]
        );
    }

    #[test]
    fn denies_invalid_text_duplicate_transactions_and_non_monotonic_results() {
        let (mut ledger, sequence) = new_move_ledger();
        assert_eq!(
            ledger.transition(sequence, ActivityOutcome::Undone),
            Err(ActivityLedgerError::InvalidTransition)
        );
        assert_eq!(
            ledger.record_action(ActivityRecordInput {
                occurred_at: 1,
                context: CONTEXT,
                transaction_id: Some(TransactionId(0x22_0000_0022)),
                user_intent: "another request",
                selected_model: None,
                action_id: "file.move",
                plan_summary: "destinations=m22-A,m22-B,m22-C",
                object_ids: &OBJECTS,
            }),
            Err(ActivityLedgerError::DuplicateTransaction)
        );
        assert_eq!(
            ledger.record_action(ActivityRecordInput {
                occurred_at: 1,
                context: CONTEXT,
                transaction_id: None,
                user_intent: "",
                selected_model: None,
                action_id: "file.move",
                plan_summary: "plan",
                object_ids: &[],
            }),
            Err(ActivityLedgerError::InvalidText)
        );
    }

    #[test]
    fn rejects_corruption_unsupported_versions_and_oversized_archives() {
        let (ledger, _) = new_move_ledger();
        let mut bytes = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
        let length = ledger.serialize_recoverable(&mut bytes).expect("serialize");
        let mut corrupt = bytes;
        corrupt[length - 1] ^= 0x80;
        assert_eq!(
            ActivityLedger::restore_recoverable(&corrupt[..length]),
            Err(ActivityLedgerError::CorruptArchive)
        );

        let mut unsupported = bytes;
        unsupported[4..6].copy_from_slice(&2_u16.to_le_bytes());
        let checksum = crate::archive_checksum(&unsupported[..length]);
        unsupported[28..36].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(
            ActivityLedger::restore_recoverable(&unsupported[..length]),
            Err(ActivityLedgerError::UnsupportedArchiveVersion(2))
        );
        assert_eq!(
            ledger.serialize_recoverable(&mut [0; 35]),
            Err(ActivityLedgerError::BufferTooSmall)
        );
    }

    #[derive(Default)]
    struct MemoryStore {
        slots: [Option<alloc::vec::Vec<u8>>; 2],
    }

    impl ActivityLedgerFileStore for MemoryStore {
        fn read_activity_slot(
            &mut self,
            slot: ArchiveSlot,
            buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
        ) -> Result<Option<usize>, ActivityLedgerError> {
            let Some(bytes) = self.slots[slot_index(slot) as usize].as_ref() else {
                return Ok(None);
            };
            buffer[..bytes.len()].copy_from_slice(bytes);
            Ok(Some(bytes.len()))
        }

        fn write_activity_slot(
            &mut self,
            slot: ArchiveSlot,
            bytes: &[u8],
        ) -> Result<(), ActivityLedgerError> {
            self.slots[slot_index(slot) as usize] = Some(bytes.to_vec());
            Ok(())
        }

        fn flush_activity_slots(&mut self) -> Result<(), ActivityLedgerError> {
            Ok(())
        }
    }

    #[test]
    fn two_slot_store_recovers_previous_ledger_when_new_slot_is_corrupt() {
        let (mut ledger, sequence) = new_move_ledger();
        let mut backend = ActivityLedgerArchiveBackend::new(MemoryStore::default());
        let mut archive = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
        let length = ledger
            .serialize_recoverable(&mut archive)
            .expect("serialize prepared");
        backend
            .write_archive(&archive[..length])
            .expect("write prepared");

        ledger
            .transition(sequence, ActivityOutcome::Committed)
            .expect("commit");
        let length = ledger
            .serialize_recoverable(&mut archive)
            .expect("serialize committed");
        backend
            .write_archive(&archive[..length])
            .expect("write committed");

        let mut store = backend.into_file_store();
        let newer = store.slots[slot_index(ArchiveSlot::B) as usize]
            .as_mut()
            .expect("second generation uses slot B");
        newer[40] ^= 0x40;
        let mut recovered = ActivityLedgerArchiveBackend::new(store);
        let mut restored_archive = [0; MAX_ACTIVITY_ARCHIVE_BYTES];
        let length = recovered
            .load_archive(&mut restored_archive)
            .expect("fallback to older slot")
            .expect("archive exists");
        let restored = ActivityLedger::restore_recoverable(&restored_archive[..length])
            .expect("restore older generation");
        assert_eq!(
            restored.record_at(0).unwrap().current_outcome(),
            ActivityOutcome::Prepared
        );
    }

    #[test]
    fn archive_capacity_fails_closed_when_objects_exceed_limit() {
        let mut ledger = ActivityLedger::new();
        let objects = vec![ObjectId(1); MAX_ACTIVITY_OBJECTS + 1];
        assert_eq!(
            ledger.record_action(ActivityRecordInput {
                occurred_at: 1,
                context: CONTEXT,
                transaction_id: None,
                user_intent: "move files",
                selected_model: None,
                action_id: "file.move",
                plan_summary: "bounded fixture plan",
                object_ids: &objects,
            }),
            Err(ActivityLedgerError::TooManyObjects)
        );
    }
}
