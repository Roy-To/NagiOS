use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE};
use nagi_history::{
    ActivityContext, AppId, AppSessionId, HistoryService, NodeId, ObjectId, UndoOperation,
    WorkspaceId,
};

const CURRENT: &[u8] = b"m15-current";
const MOVED: &[u8] = b"m15-moved";
const VERSION: &[u8] = b"m15-version";
const TRASH: &[u8] = b"m15-trash";
const LEDGER: &[u8] = b"m15-ledger";
const FIRST: &[u8] = b"Nagi M15 version one\r\n";
const SECOND: &[u8] = b"Nagi M15 version two\r\n";

static mut HISTORY: HistoryService = HistoryService::new();
static mut READ_BUFFER: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];
static mut LEDGER_BUFFER: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];

type GuestVolume = Vfs<SyscallBlockDevice>;

pub fn run(block_capability: u64) -> bool {
    let Ok((mut volume, _formatted)) =
        Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
    else {
        return false;
    };
    cleanup(&mut volume);
    let context = ActivityContext {
        app_id: AppId(0x4e41_4749_4d15),
        app_session_id: AppSessionId(1),
        node_id: NodeId(1),
        surface_id: None,
        workspace_id: Some(WorkspaceId(1)),
    };
    let object = ObjectId(0x4e41_4749_4f42_4a15);

    let Ok(handle) = volume.create(CURRENT) else {
        return false;
    };
    if volume.write(handle, FIRST).is_err() {
        return false;
    }
    let history = unsafe { &mut *core::ptr::addr_of_mut!(HISTORY) };
    *history = HistoryService::new();
    if history
        .record_create(context, object, CURRENT, FIRST)
        .is_err()
    {
        return false;
    }
    if !read_equals(&mut volume, CURRENT, FIRST) {
        return false;
    }
    print(b"Nagi M15 create PASS\r\n");

    let before = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUFFER) };
    let before_length = match read_into(&mut volume, CURRENT, before) {
        Some(length) => length,
        None => return false,
    };
    if volume.write(handle, SECOND).is_err()
        || write_or_create(&mut volume, VERSION, &before[..before_length]).is_err()
        || history
            .record_edit(context, object, CURRENT, &before[..before_length], SECOND)
            .is_err()
        || !read_equals(&mut volume, CURRENT, SECOND)
        || !read_equals(&mut volume, VERSION, FIRST)
    {
        return false;
    }
    print(b"Nagi M15 edit/version PASS\r\n");

    if volume.rename(CURRENT, MOVED).is_err()
        || history
            .record_move(context, object, CURRENT, MOVED)
            .is_err()
        || !read_equals(&mut volume, MOVED, SECOND)
    {
        return false;
    }
    print(b"Nagi M15 move PASS\r\n");

    let moved = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUFFER) };
    let moved_length = match read_into(&mut volume, MOVED, moved) {
        Some(length) => length,
        None => return false,
    };
    if write_or_create(&mut volume, TRASH, &moved[..moved_length]).is_err()
        || history
            .record_delete(context, object, MOVED, &moved[..moved_length])
            .is_err()
        || volume.remove(MOVED).is_err()
        || volume.open(MOVED) != Err(StorageError::NotFound)
        || !read_equals(&mut volume, TRASH, SECOND)
    {
        return false;
    }
    print(b"Nagi M15 delete/trash PASS\r\n");

    let Ok(restored) = volume.create(CURRENT) else {
        return false;
    };
    let trash = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUFFER) };
    let trash_length = match read_into(&mut volume, TRASH, trash) {
        Some(length) => length,
        None => return false,
    };
    if volume.write(restored, &trash[..trash_length]).is_err()
        || history
            .record_restore(context, object, CURRENT, &trash[..trash_length])
            .is_err()
        || !read_equals(&mut volume, CURRENT, SECOND)
    {
        return false;
    }
    print(b"Nagi M15 restore PASS\r\n");

    let Ok(undo) = history.undo_last() else {
        return false;
    };
    if undo.operation != UndoOperation::Delete
        || undo.object_id != object
        || volume.remove(CURRENT).is_err()
        || volume.open(CURRENT) != Err(StorageError::NotFound)
    {
        return false;
    }
    print(b"Nagi M15 undo PASS\r\n");

    let ledger = unsafe { &mut *core::ptr::addr_of_mut!(LEDGER_BUFFER) };
    let ledger_length = match history.serialize(ledger) {
        Ok(length) if length > 4 => length,
        _ => return false,
    };
    if write_or_create(&mut volume, LEDGER, &ledger[..ledger_length]).is_err()
        || !read_at_least(&mut volume, LEDGER, 4)
    {
        return false;
    }
    print(b"Nagi M15 transaction ledger PASS\r\n");
    print(b"Nagi M15 History Service PASS\r\n");
    print(b"Nagi M15 acceptance PASS\r\n");
    true
}

fn cleanup(volume: &mut GuestVolume) {
    for name in [CURRENT, MOVED, VERSION, TRASH, LEDGER] {
        let _ = volume.remove(name);
    }
}

fn write_or_create(
    volume: &mut GuestVolume,
    name: &[u8],
    bytes: &[u8],
) -> Result<(), StorageError> {
    let handle = match volume.open(name) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => volume.create(name)?,
        Err(error) => return Err(error),
    };
    volume.write(handle, bytes)
}

fn read_into(
    volume: &mut GuestVolume,
    name: &[u8],
    buffer: &mut [u8; BLOCK_SIZE],
) -> Option<usize> {
    let handle = volume.open(name).ok()?;
    volume.read(handle, buffer).ok()
}

fn read_equals(volume: &mut GuestVolume, name: &[u8], expected: &[u8]) -> bool {
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUFFER) };
    let Some(length) = read_into(volume, name, buffer) else {
        return false;
    };
    &buffer[..length] == expected
}

fn read_at_least(volume: &mut GuestVolume, name: &[u8], minimum: usize) -> bool {
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUFFER) };
    read_into(volume, name, buffer).is_some_and(|length| length >= minimum)
}

fn print(message: &[u8]) {
    libnagi::console_write(message);
}
