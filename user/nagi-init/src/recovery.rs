use libnagi::storage::{
    DirectoryEntry, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE, MAX_SMALL_FILE_SIZE,
};
use nagi_history::guest::{
    ArchiveSlot, HistoryArchiveBackend, HistoryArchiveFileStore, HistoryArchiveStore,
    GUEST_ARCHIVE_FILE_BYTES, MAX_GUEST_ARCHIVE_BYTES,
};
use nagi_history::{
    HistoryError, HistoryRecord, HistoryService, Operation, TransactionId, TransactionState,
    UndoAction, UndoBatch, UndoOperation, MAX_RECORDS,
};

type GuestVolume = Vfs<SyscallBlockDevice>;

const COMMAND_BYTES: usize = 64;
const RECOVERY_ARCHIVE_A: &[u8] = b"/m22-archive-a";
const RECOVERY_ARCHIVE_B: &[u8] = b"/m22-archive-b";

struct RecoveryFiles<'a> {
    volume: &'a mut GuestVolume,
}

impl HistoryArchiveFileStore for RecoveryFiles<'_> {
    fn read_file(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, HistoryError> {
        let path = match slot {
            ArchiveSlot::A => RECOVERY_ARCHIVE_A,
            ArchiveSlot::B => RECOVERY_ARCHIVE_B,
        };
        let handle = match self.volume.open_path(path) {
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
        if bytes.len() > MAX_SMALL_FILE_SIZE {
            return Err(HistoryError::Capacity);
        }
        let path = match slot {
            ArchiveSlot::A => RECOVERY_ARCHIVE_A,
            ArchiveSlot::B => RECOVERY_ARCHIVE_B,
        };
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

pub fn run(block_capability: u64) -> ! {
    libnagi::console_write(b"Nagi M27 Recovery Environment START\r\n");
    let mut volume = checked_mount(block_capability);
    libnagi::console_write(b"Nagi M27 Recovery console READY\r\n");

    let mut command = [0; COMMAND_BYTES];
    let mut length = 0;
    let mut overflow = false;
    let mut show_prompt = true;
    loop {
        if show_prompt {
            libnagi::console_write(b"recovery> ");
            show_prompt = false;
        }
        let mut input = [0; 1];
        if !libnagi::console_read(&mut input) {
            let _ = libnagi::sleep_ns(1_000_000);
            continue;
        }
        match input[0] {
            b'\r' | b'\n' => {
                libnagi::console_write(b"\r\n");
                if overflow {
                    libnagi::console_write(b"Nagi M27 Recovery command rejected (too long)\r\n");
                } else {
                    dispatch(&command[..length], &mut volume, block_capability);
                }
                command.fill(0);
                length = 0;
                overflow = false;
                show_prompt = true;
            }
            8 | 127 => {
                if length != 0 {
                    length -= 1;
                    command[length] = 0;
                }
            }
            byte if byte.is_ascii_graphic() || byte == b' ' => {
                libnagi::console_write(&[byte]);
                if length < command.len() {
                    command[length] = byte.to_ascii_lowercase();
                    length += 1;
                } else {
                    overflow = true;
                }
            }
            _ => {}
        }
    }
}

fn checked_mount(block_capability: u64) -> Option<GuestVolume> {
    let mut device = SyscallBlockDevice::new(block_capability);
    match GuestVolume::check_existing(&mut device) {
        Ok(report) => match GuestVolume::mount_existing(device) {
            Ok(volume) => {
                libnagi::console_write(b"Nagi M27 Recovery VFS check PASS files=");
                write_decimal(report.regular_files as usize);
                libnagi::console_write(b" directories=");
                write_decimal(report.directories as usize);
                libnagi::console_write(b"\r\n");
                Some(volume)
            }
            Err(_) => {
                libnagi::console_write(b"Nagi M27 Recovery VFS mount FAIL (not formatted)\r\n");
                None
            }
        },
        Err(_) => {
            libnagi::console_write(
                b"Nagi M27 Recovery VFS check FAIL (read-only; not formatted)\r\n",
            );
            None
        }
    }
}

fn dispatch(command: &[u8], volume: &mut Option<GuestVolume>, block_capability: u64) {
    match command {
        b"help" | b"" => {
            libnagi::console_write(
                b"Commands: check, files, help, history, log, slots, undo\r\n\
                  slots are selected in the UEFI boot menu; undo applies the latest committed NH16 transaction\r\n",
            );
            libnagi::console_write(b"Nagi M27 Recovery command help PASS\r\n");
        }
        b"check" => {
            *volume = checked_mount(block_capability);
        }
        b"files" => match volume.as_mut() {
            Some(volume) => {
                let mut entries = [DirectoryEntry::empty(); 64];
                match volume.list_root(&mut entries) {
                    Ok(count) => {
                        for entry in entries.iter().take(count) {
                            libnagi::console_write(b"- ");
                            libnagi::console_write(entry.name());
                            libnagi::console_write(b"\r\n");
                        }
                        libnagi::console_write(b"Nagi M27 Recovery files PASS\r\n");
                    }
                    Err(_) => {
                        libnagi::console_write(b"Nagi M27 Recovery files FAIL\r\n");
                    }
                }
            }
            None => {
                libnagi::console_write(
                    b"Nagi M27 Recovery files unavailable (volume not mounted)\r\n",
                );
            }
        },
        b"log" => {
            let mut log = [0; libnagi::MAX_LOG_READ];
            let length = libnagi::log_read(&mut log);
            libnagi::console_write(&log[..length]);
            libnagi::console_write(b"\r\nNagi M27 Recovery current-boot log PASS\r\n");
        }
        b"history" => match volume.as_mut() {
            Some(volume) => show_history(volume),
            None => {
                libnagi::console_write(
                    b"Nagi M27 Recovery history unavailable (volume not mounted)\r\n",
                );
            }
        },
        #[cfg(feature = "m27-recovery-undo-acceptance")]
        b"undo-conflict-test" => match volume.as_mut() {
            Some(volume) => match undo_conflict_fixture(volume) {
                UndoFixtureResult::Passed => {
                    libnagi::console_write(b"Nagi M27 Recovery undo preflight conflict PASS\r\n");
                    libnagi::console_write(b"Nagi M27 Recovery interrupted undo retry PASS\r\n");
                }
                UndoFixtureResult::ConflictFailed => {
                    libnagi::console_write(b"Nagi M27 Recovery undo preflight conflict FAIL\r\n");
                }
                UndoFixtureResult::RetryFailed => {
                    libnagi::console_write(b"Nagi M27 Recovery undo preflight conflict PASS\r\n");
                    libnagi::console_write(b"Nagi M27 Recovery interrupted undo retry FAIL\r\n");
                }
            },
            None => {
                libnagi::console_write(
                    b"Nagi M27 Recovery undo preflight conflict FAIL (volume not mounted)\r\n",
                );
            }
        },
        b"slots" => {
            libnagi::console_write(
                b"Select System A, System B, or Recovery from the UEFI boot menu.\r\n",
            );
        }
        b"undo" => {
            match volume.as_mut() {
                Some(volume) => match undo_latest(volume) {
                    UndoResult::Applied => {
                        libnagi::console_write(b"Nagi M27 Recovery NH16 undo PASS\r\n");
                    }
                    UndoResult::NoArchive => {
                        libnagi::console_write(
                            b"Nagi M27 Recovery undo unavailable (no NH16 archive)\r\n",
                        );
                    }
                    UndoResult::NoCommittedTransaction => {
                        libnagi::console_write(b"Nagi M27 Recovery undo unavailable (no committed NH16 transaction)\r\n");
                    }
                    UndoResult::Conflict => {
                        libnagi::console_write(
                            b"Nagi M27 Recovery undo conflict; no changes applied\r\n",
                        );
                    }
                    UndoResult::Failed => {
                        libnagi::console_write(b"Nagi M27 Recovery NH16 undo FAIL\r\n");
                    }
                },
                None => {
                    libnagi::console_write(
                        b"Nagi M27 Recovery undo unavailable (volume not mounted)\r\n",
                    );
                }
            }
        }
        _ => {
            libnagi::console_write(b"Unknown command. Type help.\r\n");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UndoResult {
    Applied,
    NoArchive,
    NoCommittedTransaction,
    Conflict,
    Failed,
}

fn show_history(volume: &mut GuestVolume) {
    let mut backend = HistoryArchiveBackend::new(RecoveryFiles { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let length = match backend.load_archive(&mut archive) {
        Ok(Some(length)) => length,
        Ok(None) => {
            libnagi::console_write(b"Nagi M27 Recovery history unavailable (no NH16 archive)\r\n");
            return;
        }
        Err(_) => {
            libnagi::console_write(b"Nagi M27 Recovery history FAIL\r\n");
            return;
        }
    };
    let history = match HistoryService::restore_recoverable(&archive[..length]) {
        Ok(history) => history,
        Err(_) => {
            libnagi::console_write(b"Nagi M27 Recovery history FAIL\r\n");
            return;
        }
    };
    let start = history.len().saturating_sub(MAX_RECORDS);
    for index in start..history.len() {
        let Some(record) = history.record_at(index) else {
            continue;
        };
        libnagi::console_write(b"NH16 sequence=");
        write_decimal_u64(record.sequence);
        libnagi::console_write(b" transaction=");
        write_decimal_u64(record.transaction_id.0);
        libnagi::console_write(b" operation=");
        libnagi::console_write(operation_name(record.operation));
        libnagi::console_write(b" state=");
        libnagi::console_write(transaction_state_name(record.transaction_state));
        libnagi::console_write(b" object=");
        write_hex_u64(record.object_id.0);
        libnagi::console_write(b"\r\n");
    }
    libnagi::console_write(b"Nagi M27 Recovery history PASS entries=");
    write_decimal(history.len() - start);
    libnagi::console_write(b"\r\n");
}

fn operation_name(operation: Operation) -> &'static [u8] {
    match operation {
        Operation::Create => b"CREATE",
        Operation::Edit => b"EDIT",
        Operation::Move => b"MOVE",
        Operation::Delete => b"DELETE",
        Operation::Restore => b"RESTORE",
    }
}

fn transaction_state_name(state: TransactionState) -> &'static [u8] {
    match state {
        TransactionState::Prepared => b"PREPARED",
        TransactionState::Committed => b"COMMITTED",
        TransactionState::UndoPending => b"UNDO_PENDING",
        TransactionState::Undone => b"UNDONE",
    }
}

fn write_decimal_u64(value: u64) {
    let mut digits = [0; 20];
    let mut cursor = digits.len();
    let mut remaining = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    libnagi::console_write(&digits[cursor..]);
}

fn write_hex_u64(value: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digits = [0; 16];
    for (index, digit) in digits.iter_mut().enumerate() {
        let shift = (16 - index - 1) * 4;
        *digit = HEX[((value >> shift) & 0x0f) as usize];
    }
    libnagi::console_write(b"0x");
    libnagi::console_write(&digits);
}

fn undo_latest(volume: &mut GuestVolume) -> UndoResult {
    let mut backend = HistoryArchiveBackend::new(RecoveryFiles { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let length = match backend.load_archive(&mut archive) {
        Ok(Some(length)) => length,
        Ok(None) => return UndoResult::NoArchive,
        Err(_) => return UndoResult::Failed,
    };
    let mut history = match HistoryService::restore_recoverable(&archive[..length]) {
        Ok(history) => history,
        Err(_) => return UndoResult::Failed,
    };
    let Some(record) = latest_undo_record(&history) else {
        return UndoResult::NoCommittedTransaction;
    };
    let transaction = record.transaction_id;
    let batch = match history.prepare_undo_transaction(transaction, record.context) {
        Ok(batch) => batch,
        Err(_) => return UndoResult::Failed,
    };
    apply_prepared_undo(&mut history, &mut backend, record, batch)
}

fn apply_prepared_undo(
    history: &mut HistoryService,
    backend: &mut HistoryArchiveBackend<RecoveryFiles<'_>>,
    record: HistoryRecord,
    batch: UndoBatch,
) -> UndoResult {
    let transaction = batch.transaction_id();
    let preflight_passed = {
        let files = backend.file_store_mut();
        preflight_undo_batch(history, transaction, &batch, files.volume)
    };
    if !preflight_passed {
        return UndoResult::Conflict;
    }
    let mut pending = [0; MAX_GUEST_ARCHIVE_BYTES];
    let pending_length = match history.serialize_recoverable(&mut pending) {
        Ok(length) => length,
        Err(_) => return UndoResult::Failed,
    };
    if backend.write_archive(&pending[..pending_length]).is_err() {
        return UndoResult::Failed;
    }

    {
        let files = backend.file_store_mut();
        for action in batch.actions() {
            if !apply_undo_action(files.volume, action) {
                return UndoResult::Failed;
            }
        }
        if files.volume.flush().is_err() {
            return UndoResult::Failed;
        }
    }
    if history
        .complete_undo_transaction(transaction, record.context)
        .is_err()
    {
        return UndoResult::Failed;
    }
    let mut completed = [0; MAX_GUEST_ARCHIVE_BYTES];
    let completed_length = match history.serialize_recoverable(&mut completed) {
        Ok(length) => length,
        Err(_) => return UndoResult::Failed,
    };
    if backend
        .write_archive(&completed[..completed_length])
        .is_err()
    {
        return UndoResult::Failed;
    }
    UndoResult::Applied
}

fn latest_undo_record(history: &HistoryService) -> Option<HistoryRecord> {
    (0..history.len())
        .rev()
        .find_map(|index| {
            history
                .record_at(index)
                .filter(|record| record.transaction_state == TransactionState::UndoPending)
        })
        .or_else(|| {
            (0..history.len()).rev().find_map(|index| {
                history
                    .record_at(index)
                    .filter(|record| record.transaction_state == TransactionState::Committed)
            })
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UndoActionState {
    Forward,
    Inverse,
    Conflict,
}

fn preflight_undo_batch(
    history: &HistoryService,
    transaction_id: TransactionId,
    batch: &UndoBatch,
    volume: &mut GuestVolume,
) -> bool {
    batch.actions().all(|action| {
        undo_action_state(history, transaction_id, volume, action) != UndoActionState::Conflict
    })
}

#[cfg(feature = "m27-recovery-undo-acceptance")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UndoFixtureResult {
    Passed,
    ConflictFailed,
    RetryFailed,
}

/// Exercises the Recovery Undo conflict guard and interrupted-Undo replay on
/// the persisted three-file M22 move transaction. This command is included
/// only in the M27 acceptance image.
#[cfg(feature = "m27-recovery-undo-acceptance")]
fn undo_conflict_fixture(volume: &mut GuestVolume) -> UndoFixtureResult {
    let mut backend = HistoryArchiveBackend::new(RecoveryFiles { volume });
    let mut archive = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Some(length) = backend.load_archive(&mut archive).ok().flatten() else {
        return UndoFixtureResult::ConflictFailed;
    };
    let Ok(mut history) = HistoryService::restore_recoverable(&archive[..length]) else {
        return UndoFixtureResult::ConflictFailed;
    };
    let Some(record) = history.record_at(0) else {
        return UndoFixtureResult::ConflictFailed;
    };
    if record.operation != Operation::Move
        || record.transaction_state != TransactionState::Committed
    {
        return UndoFixtureResult::ConflictFailed;
    }
    let transaction_id = record.transaction_id;
    let Ok(batch) = history.prepare_undo_transaction(transaction_id, record.context) else {
        return UndoFixtureResult::ConflictFailed;
    };
    let action_count = batch.actions().count();
    let Some(last_action) = batch.actions().last() else {
        return UndoFixtureResult::ConflictFailed;
    };
    if action_count < 2
        || last_action.operation != UndoOperation::MoveBack
        || !batch
            .actions()
            .all(|action| action.operation == UndoOperation::MoveBack)
    {
        return UndoFixtureResult::ConflictFailed;
    }

    let Some(content_conflict_action) = batch.actions().next() else {
        return UndoFixtureResult::ConflictFailed;
    };
    let (content_conflict_name, content_conflict_name_length) =
        content_conflict_action.from_name.bytes();
    let content_conflict_name = &content_conflict_name[..content_conflict_name_length];
    let mut original_contents = [0; MAX_SMALL_FILE_SIZE];
    let original_length = {
        let volume = &mut backend.file_store_mut().volume;
        let Ok(handle) = volume.open(content_conflict_name) else {
            return UndoFixtureResult::ConflictFailed;
        };
        match volume.read(handle, &mut original_contents) {
            Ok(length) if length > 0 => length,
            _ => return UndoFixtureResult::ConflictFailed,
        }
    };
    let original_first_byte = original_contents[0];
    original_contents[0] ^= 0xff;
    let tamper_written = {
        let volume = &mut backend.file_store_mut().volume;
        volume
            .open(content_conflict_name)
            .and_then(|handle| volume.write(handle, &original_contents[..original_length]))
            .and_then(|()| volume.flush())
            .is_ok()
    };
    if !tamper_written {
        return UndoFixtureResult::ConflictFailed;
    }
    let content_conflict_result = apply_prepared_undo(&mut history, &mut backend, record, batch);
    let persisted_content_conflict = if content_conflict_result == UndoResult::Conflict {
        let loaded = backend
            .load_archive(&mut archive)
            .ok()
            .flatten()
            .and_then(|length| HistoryService::restore_recoverable(&archive[..length]).ok());
        loaded.is_some_and(|persisted| {
            transaction_has_state(
                &persisted,
                transaction_id,
                TransactionState::Committed,
                action_count,
            ) && batch.actions().all(|action| {
                let expected = if action.sequence == content_conflict_action.sequence {
                    UndoActionState::Conflict
                } else {
                    UndoActionState::Forward
                };
                undo_action_state(
                    &persisted,
                    transaction_id,
                    backend.file_store_mut().volume,
                    action,
                ) == expected
            })
        })
    } else {
        false
    };
    original_contents[0] = original_first_byte;
    let original_restored = {
        let volume = &mut backend.file_store_mut().volume;
        volume
            .open(content_conflict_name)
            .and_then(|handle| volume.write(handle, &original_contents[..original_length]))
            .and_then(|()| volume.flush())
            .is_ok()
    };
    if !persisted_content_conflict || !original_restored {
        return UndoFixtureResult::ConflictFailed;
    }
    libnagi::console_write(b"Nagi M27 Recovery same-path move content conflict PASS\r\n");

    let conflict_name = last_action.to_name;
    let (conflict_name_bytes, conflict_name_length) = conflict_name.bytes();
    let conflict_name_bytes = &conflict_name_bytes[..conflict_name_length];
    {
        let files = backend.file_store_mut();
        if !matches!(
            files.volume.open(conflict_name_bytes),
            Err(StorageError::NotFound)
        ) {
            return UndoFixtureResult::ConflictFailed;
        }
        let Ok(handle) = files.volume.create(conflict_name_bytes) else {
            return UndoFixtureResult::ConflictFailed;
        };
        if files
            .volume
            .write(handle, b"m27 recovery conflict sentinel")
            .is_err()
            || files.volume.flush().is_err()
        {
            let _ = files.volume.remove(conflict_name_bytes);
            let _ = files.volume.flush();
            return UndoFixtureResult::ConflictFailed;
        }
    }

    let conflict_result = apply_prepared_undo(&mut history, &mut backend, record, batch);
    let persisted_conflict_state = if conflict_result == UndoResult::Conflict {
        let mut bytes = [0; MAX_GUEST_ARCHIVE_BYTES];
        let loaded = backend
            .load_archive(&mut bytes)
            .ok()
            .flatten()
            .and_then(|length| HistoryService::restore_recoverable(&bytes[..length]).ok());
        loaded.is_some_and(|persisted| {
            transaction_has_state(
                &persisted,
                transaction_id,
                TransactionState::Committed,
                action_count,
            ) && batch.actions().enumerate().all(|(index, action)| {
                let expected = if index + 1 == action_count {
                    UndoActionState::Conflict
                } else {
                    UndoActionState::Forward
                };
                undo_action_state(
                    &persisted,
                    transaction_id,
                    backend.file_store_mut().volume,
                    action,
                ) == expected
            })
        })
    } else {
        false
    };

    let cleanup_succeeded = {
        let files = backend.file_store_mut();
        matches!(files.volume.remove(conflict_name_bytes), Ok(())) && files.volume.flush().is_ok()
    };
    if !persisted_conflict_state || !cleanup_succeeded {
        return UndoFixtureResult::ConflictFailed;
    }

    let mut bytes = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Some(length) = backend.load_archive(&mut bytes).ok().flatten() else {
        return UndoFixtureResult::RetryFailed;
    };
    let Ok(mut retry_history) = HistoryService::restore_recoverable(&bytes[..length]) else {
        return UndoFixtureResult::RetryFailed;
    };
    let Some(retry_record) = retry_history.record_at(0) else {
        return UndoFixtureResult::RetryFailed;
    };
    let Ok(retry_batch) =
        retry_history.prepare_undo_transaction(transaction_id, retry_record.context)
    else {
        return UndoFixtureResult::RetryFailed;
    };
    if !preflight_undo_batch(
        &retry_history,
        transaction_id,
        &retry_batch,
        backend.file_store_mut().volume,
    ) {
        return UndoFixtureResult::RetryFailed;
    }
    let mut pending = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Ok(pending_length) = retry_history.serialize_recoverable(&mut pending) else {
        return UndoFixtureResult::RetryFailed;
    };
    if backend.write_archive(&pending[..pending_length]).is_err() {
        return UndoFixtureResult::RetryFailed;
    }
    let Some(first_action) = retry_batch.actions().next() else {
        return UndoFixtureResult::RetryFailed;
    };
    {
        let files = backend.file_store_mut();
        if !apply_undo_action(files.volume, first_action) || files.volume.flush().is_err() {
            return UndoFixtureResult::RetryFailed;
        }
    }

    drop(backend);
    if undo_latest(volume) != UndoResult::Applied {
        return UndoFixtureResult::RetryFailed;
    }
    let mut backend = HistoryArchiveBackend::new(RecoveryFiles { volume });
    let mut bytes = [0; MAX_GUEST_ARCHIVE_BYTES];
    let Some(length) = backend.load_archive(&mut bytes).ok().flatten() else {
        return UndoFixtureResult::RetryFailed;
    };
    let Ok(completed_history) = HistoryService::restore_recoverable(&bytes[..length]) else {
        return UndoFixtureResult::RetryFailed;
    };
    if !transaction_has_state(
        &completed_history,
        transaction_id,
        TransactionState::Undone,
        action_count,
    ) || !batch.actions().all(|action| {
        undo_action_state(
            &completed_history,
            transaction_id,
            backend.file_store_mut().volume,
            action,
        ) == UndoActionState::Inverse
    }) {
        return UndoFixtureResult::RetryFailed;
    }
    UndoFixtureResult::Passed
}

#[cfg(feature = "m27-recovery-undo-acceptance")]
fn transaction_has_state(
    history: &HistoryService,
    transaction_id: TransactionId,
    expected_state: TransactionState,
    expected_count: usize,
) -> bool {
    let mut count = 0;
    for record in (0..history.len()).filter_map(|index| history.record_at(index)) {
        if record.transaction_id == transaction_id {
            count += 1;
            if record.transaction_state != expected_state {
                return false;
            }
        }
    }
    count == expected_count
}

fn undo_action_state(
    history: &HistoryService,
    transaction_id: TransactionId,
    volume: &mut GuestVolume,
    action: UndoAction,
) -> UndoActionState {
    let (from_name, from_length) = action.from_name.bytes();
    let (to_name, to_length) = action.to_name.bytes();
    let from_name = &from_name[..from_length];
    let to_name = &to_name[..to_length];
    let content = &action.content[..action.content_length];

    match action.operation {
        UndoOperation::Delete => match named_file_matches(volume, from_name, content) {
            Some(true) => UndoActionState::Forward,
            None => UndoActionState::Inverse,
            Some(false) => UndoActionState::Conflict,
        },
        UndoOperation::Restore => match named_file_matches(volume, to_name, content) {
            None => UndoActionState::Forward,
            Some(true) => UndoActionState::Inverse,
            Some(false) => UndoActionState::Conflict,
        },
        UndoOperation::RestoreVersion => {
            let Some(forward_content) =
                history.expected_edit_after_for_undo(transaction_id, action.sequence)
            else {
                return UndoActionState::Conflict;
            };
            match named_file_matches(volume, to_name, forward_content) {
                Some(true) => UndoActionState::Forward,
                Some(false) => match named_file_matches(volume, to_name, content) {
                    Some(true) => UndoActionState::Inverse,
                    _ => UndoActionState::Conflict,
                },
                None => UndoActionState::Conflict,
            }
        }
        UndoOperation::MoveBack => {
            let expected_digest = &action.content[..action.content_length];
            match (
                named_file_exists(volume, from_name),
                named_file_exists(volume, to_name),
            ) {
                (Some(true), Some(false))
                    if action.content_length == 0
                        || named_file_digest_matches(volume, from_name, expected_digest)
                            == Some(true) =>
                {
                    UndoActionState::Forward
                }
                (Some(false), Some(true))
                    if action.content_length == 0
                        || named_file_digest_matches(volume, to_name, expected_digest)
                            == Some(true) =>
                {
                    UndoActionState::Inverse
                }
                _ => UndoActionState::Conflict,
            }
        }
    }
}

/// `Some(true)` means the named file exists and exactly matches, `Some(false)`
/// means it exists with different bytes or could not be read, and `None` means
/// it is absent.
fn named_file_matches(volume: &mut GuestVolume, name: &[u8], expected: &[u8]) -> Option<bool> {
    match volume.open(name) {
        Ok(handle) => Some(file_matches(volume, handle, expected)),
        Err(StorageError::NotFound) => None,
        Err(_) => Some(false),
    }
}

fn named_file_exists(volume: &mut GuestVolume, name: &[u8]) -> Option<bool> {
    match volume.open(name) {
        Ok(_) => Some(true),
        Err(StorageError::NotFound) => Some(false),
        Err(_) => None,
    }
}

fn named_file_digest_matches(
    volume: &mut GuestVolume,
    name: &[u8],
    expected_digest: &[u8],
) -> Option<bool> {
    match volume.open(name) {
        Ok(handle) => {
            let mut contents = [0; MAX_SMALL_FILE_SIZE];
            match volume.read(handle, &mut contents) {
                Ok(length) => Some(nagi_history::move_content_matches_digest(
                    &contents[..length],
                    expected_digest,
                )),
                Err(_) => Some(false),
            }
        }
        Err(StorageError::NotFound) => None,
        Err(_) => Some(false),
    }
}

fn apply_undo_action(volume: &mut GuestVolume, action: UndoAction) -> bool {
    let (from_name, from_length) = action.from_name.bytes();
    let (to_name, to_length) = action.to_name.bytes();
    let from_name = &from_name[..from_length];
    let to_name = &to_name[..to_length];
    let content = &action.content[..action.content_length.min(MAX_SMALL_FILE_SIZE)];

    match action.operation {
        UndoOperation::Delete => match volume.remove(from_name) {
            Ok(()) | Err(StorageError::NotFound) => true,
            Err(_) => false,
        },
        UndoOperation::Restore => restore_missing_file(volume, to_name, content),
        UndoOperation::RestoreVersion => restore_file_version(volume, to_name, content),
        UndoOperation::MoveBack => {
            let source = volume.open(from_name);
            let destination = volume.open(to_name);
            match (source, destination) {
                (Ok(_), Err(StorageError::NotFound)) => volume.rename(from_name, to_name).is_ok(),
                (Err(StorageError::NotFound), Ok(_)) => true,
                _ => false,
            }
        }
    }
}

fn restore_missing_file(volume: &mut GuestVolume, name: &[u8], content: &[u8]) -> bool {
    match volume.open(name) {
        Ok(handle) => file_matches(volume, handle, content),
        Err(StorageError::NotFound) => volume
            .create(name)
            .and_then(|handle| volume.write(handle, content))
            .is_ok(),
        Err(_) => false,
    }
}

fn restore_file_version(volume: &mut GuestVolume, name: &[u8], content: &[u8]) -> bool {
    match volume.open(name) {
        Ok(handle) => {
            if file_matches(volume, handle, content) {
                true
            } else {
                volume.write(handle, content).is_ok()
            }
        }
        Err(StorageError::NotFound) => volume
            .create(name)
            .and_then(|handle| volume.write(handle, content))
            .is_ok(),
        Err(_) => false,
    }
}

fn file_matches(
    volume: &mut GuestVolume,
    handle: libnagi::storage::FileHandle,
    expected: &[u8],
) -> bool {
    let mut actual = [0; BLOCK_SIZE];
    volume
        .read(handle, &mut actual)
        .is_ok_and(|length| &actual[..length] == expected)
}

fn write_decimal(value: usize) {
    let mut digits = [0; 20];
    let mut cursor = digits.len();
    let mut remaining = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    libnagi::console_write(&digits[cursor..]);
}
