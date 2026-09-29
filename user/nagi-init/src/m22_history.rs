use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, MAX_FILE_SIZE};
use nagi_history::guest::{
    ArchiveSlot, HistoryArchiveBackend, HistoryArchiveFileStore, HistoryArchiveStore,
    GUEST_ARCHIVE_FILE_BYTES, MAX_GUEST_ARCHIVE_BYTES,
};
use nagi_history::{
    ActivityContext, AppId, AppSessionId, HistoryError, HistoryService, MoveRecord, NodeId,
    ObjectId, SurfaceId, TransactionState, UndoOperation, WorkspaceId,
};

type GuestVolume = Vfs<SyscallBlockDevice>;

const CALLER: ActivityContext = ActivityContext {
    app_id: AppId(0x4d22),
    app_session_id: AppSessionId(0x2201),
    node_id: NodeId(0x2202),
    surface_id: Some(SurfaceId(0x2203)),
    workspace_id: Some(WorkspaceId(0x2204)),
};

const MOVES: [(ObjectId, &[u8], &[u8], &[u8]); 3] = [
    (ObjectId(0x2211), b"m22-a", b"m22-A", b"M22 fixture one"),
    (ObjectId(0x2212), b"m22-b", b"m22-B", b"M22 fixture two"),
    (ObjectId(0x2213), b"m22-c", b"m22-C", b"M22 fixture three"),
];

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

/// Exercises the NH16 archive over the persistent guest VFS. This fixture
/// validates storage and restart recovery only; it is not a production AI
/// action, authenticated policy, or M21 file-move acceptance.
pub fn run(block_capability: u64) -> bool {
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

    if is_new {
        if !ensure_original_fixture(&mut backend.file_store_mut().volume) {
            return false;
        }
        let moves = MOVES.map(|(object_id, from_name, to_name, _)| MoveRecord {
            object_id,
            from_name,
            to_name,
        });
        let Ok(transaction_id) = history.record_move_group(CALLER, &moves) else {
            return false;
        };
        if !save_history(&mut backend, &history) {
            return false;
        }
        if !apply_forward_group(&mut backend.file_store_mut().volume) {
            return false;
        }
        if history.commit_transaction(transaction_id, CALLER).is_err()
            || !save_history(&mut backend, &history)
            || !verify_names(&mut backend.file_store_mut().volume, false)
        {
            return false;
        }
        libnagi::console_write(b"Nagi M22 move group persisted in guest VFS PASS\r\n");
        return true;
    }

    let Some(first) = history.record_at(0) else {
        return false;
    };
    if !verify_context_and_group(&history, first.transaction_id.0) {
        return false;
    }

    match first.transaction_state {
        TransactionState::Prepared => {
            if !apply_forward_group(&mut backend.file_store_mut().volume)
                || history
                    .commit_transaction(first.transaction_id, CALLER)
                    .is_err()
                || !save_history(&mut backend, &history)
                || !verify_names(&mut backend.file_store_mut().volume, false)
            {
                return false;
            }
            libnagi::console_write(b"Nagi M22 recovered prepared move group PASS\r\n");
        }
        TransactionState::Committed | TransactionState::UndoPending => {
            if first.transaction_state == TransactionState::Committed
                && !verify_names(&mut backend.file_store_mut().volume, false)
            {
                return false;
            }
            let Ok(batch) = history.prepare_undo_transaction(first.transaction_id, CALLER) else {
                return false;
            };
            if !save_history(&mut backend, &history) {
                return false;
            }
            for action in batch.actions() {
                if action.operation != UndoOperation::MoveBack {
                    return false;
                }
                let (from_name, from_length) = action.from_name.bytes();
                let (to_name, to_length) = action.to_name.bytes();
                let Some((_, _, _, contents)) = MOVES
                    .iter()
                    .find(|(object_id, _, _, _)| *object_id == action.object_id)
                else {
                    return false;
                };
                if !apply_idempotent_move(
                    &mut backend.file_store_mut().volume,
                    &from_name[..from_length],
                    &to_name[..to_length],
                    contents,
                ) {
                    return false;
                }
            }
            if backend.file_store_mut().volume.flush().is_err()
                || history
                    .complete_undo_transaction(first.transaction_id, CALLER)
                    .is_err()
                || !save_history(&mut backend, &history)
                || !verify_names(&mut backend.file_store_mut().volume, true)
            {
                return false;
            }
            libnagi::console_write(b"Nagi M22 composite undo applied and persisted PASS\r\n");
        }
        TransactionState::Undone => {
            if !verify_names(&mut backend.file_store_mut().volume, true) {
                return false;
            }
            libnagi::console_write(b"Nagi M22 archive restart and restored files PASS\r\n");
        }
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

fn ensure_original_fixture(volume: &mut GuestVolume) -> bool {
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
    if history.len() != MOVES.len() {
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

fn write_named_file(volume: &mut GuestVolume, name: &[u8], contents: &[u8]) -> bool {
    if contents.len() > MAX_FILE_SIZE {
        return false;
    }
    let mut path = [0; 8];
    path[0] = b'/';
    path[1..1 + name.len()].copy_from_slice(name);
    let path = &path[..name.len() + 1];
    let Ok(handle) = volume.create_path(path) else {
        return false;
    };
    volume.write(handle, contents).is_ok()
}

fn named_file_matches(volume: &mut GuestVolume, name: &[u8], expected: &[u8]) -> bool {
    let mut path = [0; 8];
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
    let mut path = [0; 8];
    path[0] = b'/';
    path[1..1 + name.len()].copy_from_slice(name);
    matches!(
        volume.open_path(&path[..name.len() + 1]),
        Err(StorageError::NotFound)
    )
}
