use crate::net::{socket_is_nonblocking, socket_status_flags, O_NONBLOCK};
use crate::readonly_callback_file::{CallbackFileError, ReadAtCallback, ReadOnlyCallbackFile};
use crate::static_files::{read_static, StaticFileError, StaticFileTable};
#[cfg(all(feature = "browser-storage", target_os = "nagi"))]
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use libnagi::storage::{
    DirectoryEntry, FileHandle, FileMetadata, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE,
    MAX_DIRECTORY_ENTRIES, MAX_PATH_LENGTH,
};
use nagi_net::{Ipv4Address, NetError, SocketApi, SyscallDevice, TcpConnectionId};
use nagi_pal::sync::{SpinGuard, SpinMutex};
use nagi_pal::time::{Clock, GuestClock};

const PIPE_CAPACITY: usize = 4096;
const O_CLOEXEC: i32 = 0x0100_0000;
const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const F_GETFL: i32 = 3;
const F_SETFL: i32 = 4;
const FD_CLOEXEC: i32 = 0x0100_0000;
const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;
#[cfg(feature = "browser-storage")]
const SERVO_TEMP_DIRECTORY_ENTRIES: usize = 32;
#[cfg(feature = "browser-storage")]
const SERVO_TEMP_DIRECTORY_MAX_DEPTH: usize = 8;

#[derive(Clone, Copy)]
enum FdEntry {
    Random,
    File {
        handle: FileHandle,
        offset: usize,
    },
    Socket {
        connected: bool,
        tcp_connection: Option<TcpConnectionId>,
        peer: Option<(Ipv4Address, u16)>,
        last_error: i32,
        nonblocking: bool,
        read_shutdown: bool,
        write_shutdown: bool,
        nagle_enabled: bool,
        timeout: Option<Duration>,
    },
    PipeRead {
        pipe: usize,
        nonblocking: bool,
    },
    PipeWrite {
        pipe: usize,
        nonblocking: bool,
    },
    CallbackFile(ReadOnlyCallbackFile),
}

#[derive(Clone, Copy)]
struct Pipe {
    buffer: [u8; PIPE_CAPACITY],
    head: usize,
    length: usize,
    reader_open: bool,
    writer_open: bool,
}

impl Pipe {
    const fn new() -> Self {
        Self {
            buffer: [0; PIPE_CAPACITY],
            head: 0,
            length: 0,
            reader_open: true,
            writer_open: true,
        }
    }
}

static FILESYSTEM: SpinMutex<Option<Vfs<SyscallBlockDevice>>> = SpinMutex::new(None);
static FILE_DESCRIPTORS: SpinMutex<[Option<FdEntry>; 32]> = SpinMutex::new([None; 32]);
static STATIC_FILES: SpinMutex<StaticFileTable> = SpinMutex::new(StaticFileTable::new());

/// Regular file, read-only for everyone.
const STATIC_FILE_MODE: u16 = 0o100444;
static PIPES: SpinMutex<[Option<Pipe>; 8]> = SpinMutex::new([None; 8]);
static NETWORK: SpinMutex<Option<SocketApi<SyscallDevice>>> = SpinMutex::new(None);
#[cfg(all(feature = "browser-storage", target_os = "nagi"))]
static M18_NETWORK_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "browser-storage", target_os = "nagi"))]
static M18_TCP_CONNECT_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "browser-storage", target_os = "nagi"))]
static M18_DNS_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "browser-storage", target_os = "nagi"))]
static M18_NETWORK_LOCK_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn trace_m18_network_lock(message: &[u8]) {
    #[cfg(all(feature = "browser-storage", target_os = "nagi"))]
    if M18_NETWORK_LOCK_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 8 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "browser-storage", target_os = "nagi")))]
    let _ = message;
}

fn network_lock() -> SpinGuard<'static, Option<SocketApi<SyscallDevice>>> {
    #[cfg(feature = "browser-storage")]
    let mut yielded_for_lock = false;
    #[cfg(feature = "browser-storage")]
    loop {
        if let Some(guard) = NETWORK.try_lock() {
            if yielded_for_lock {
                trace_m18_network_lock(b"Nagi M18 network lock acquired after yield\r\n");
            }
            return guard;
        }
        if !yielded_for_lock {
            trace_m18_network_lock(b"Nagi M18 network lock contention; yielding\r\n");
            yielded_for_lock = true;
        }
        #[cfg(target_os = "nagi")]
        {
            let _ = libnagi::thread_yield();
        }
        #[cfg(not(target_os = "nagi"))]
        core::hint::spin_loop();
    }

    #[cfg(not(feature = "browser-storage"))]
    NETWORK.lock()
}

#[inline]
pub(crate) fn trace_m18_network(message: &[u8]) {
    #[cfg(all(feature = "browser-storage", target_os = "nagi"))]
    if M18_NETWORK_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 64 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "browser-storage", target_os = "nagi")))]
    let _ = message;
}

#[inline]
fn trace_m18_tcp_connect(message: &[u8]) {
    #[cfg(all(feature = "browser-storage", target_os = "nagi"))]
    if M18_TCP_CONNECT_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 16 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "browser-storage", target_os = "nagi")))]
    let _ = message;
}

#[inline]
fn trace_m18_dns(message: &[u8]) {
    #[cfg(all(feature = "browser-storage", target_os = "nagi"))]
    if M18_DNS_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 16 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "browser-storage", target_os = "nagi")))]
    let _ = message;
}

#[cfg(feature = "browser-storage")]
const BROWSER_STORAGE_FILE: &[u8] = b".nagi-browser-state";
#[cfg(feature = "browser-storage")]
const BROWSER_STORAGE_PENDING_FILE: &[u8] = b".nagi-browser-pending";
/// Largest Albert snapshot (session, history, bookmarks). Multi-block VFS
/// files (ADR 0056) allow more than the former single 1 KiB block; Albert's
/// `MAX_STORAGE_BUNDLE_BYTES` must match.
#[cfg(feature = "browser-storage")]
pub const BROWSER_STORAGE_MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    NotInitialized,
    InvalidFd,
    InvalidArgument,
    Storage(StorageError),
    Network(NetError),
    EntropyUnavailable,
    NotConnected,
    Shutdown,
    WouldBlock,
    BrokenPipe,
    Unsupported,
    ReadOnly,
    CallbackReadFailed,
}

pub fn initialize(capability: u64) -> bool {
    let Ok((volume, _formatted)) = Vfs::mount_or_format(SyscallBlockDevice::new(capability)) else {
        return false;
    };
    *FILESYSTEM.lock() = Some(volume);
    true
}

/// Read the browser's single versioned snapshot through the initialized guest
/// VFS. The service owns both storage names, so callers never supply paths.
#[cfg(feature = "browser-storage")]
pub fn browser_storage_read(output: &mut [u8]) -> Result<Option<usize>, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let handle = match volume.open(BROWSER_STORAGE_FILE) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => return Ok(None),
        Err(error) => return Err(RuntimeError::Storage(error)),
    };
    let metadata = volume.metadata(handle).map_err(RuntimeError::Storage)?;
    let length =
        usize::try_from(metadata.size).map_err(|_| RuntimeError::Storage(StorageError::Corrupt))?;
    if length > output.len() || length > BROWSER_STORAGE_MAX_BYTES {
        return Err(RuntimeError::Storage(StorageError::FileTooLarge));
    }
    if length == 0 {
        return Err(RuntimeError::Storage(StorageError::Corrupt));
    }
    let read = volume
        .read(handle, &mut output[..length])
        .map_err(RuntimeError::Storage)?;
    if read != length {
        return Err(RuntimeError::Storage(StorageError::Corrupt));
    }
    Ok(Some(length))
}

/// Commit one bounded browser snapshot. The pending inode is flushed before a
/// single root-directory update makes it active; the old snapshot remains
/// reachable if preparing the new file fails.
#[cfg(feature = "browser-storage")]
pub fn browser_storage_write(bytes: &[u8]) -> Result<(), RuntimeError> {
    if bytes.len() > BROWSER_STORAGE_MAX_BYTES {
        return Err(RuntimeError::Storage(StorageError::FileTooLarge));
    }

    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    match volume.remove(BROWSER_STORAGE_PENDING_FILE) {
        Ok(()) | Err(StorageError::NotFound) => {}
        Err(error) => return Err(RuntimeError::Storage(error)),
    }
    let pending = volume
        .create(BROWSER_STORAGE_PENDING_FILE)
        .map_err(RuntimeError::Storage)?;
    if let Err(error) = volume.write(pending, bytes) {
        let _ = volume.remove(BROWSER_STORAGE_PENDING_FILE);
        return Err(RuntimeError::Storage(error));
    }
    volume.flush().map_err(RuntimeError::Storage)?;

    match volume.open(BROWSER_STORAGE_FILE) {
        Ok(_) => volume
            .replace(BROWSER_STORAGE_PENDING_FILE, BROWSER_STORAGE_FILE)
            .map_err(RuntimeError::Storage)?,
        Err(StorageError::NotFound) => volume
            .rename(BROWSER_STORAGE_PENDING_FILE, BROWSER_STORAGE_FILE)
            .map_err(RuntimeError::Storage)?,
        Err(error) => return Err(RuntimeError::Storage(error)),
    };
    volume.flush().map_err(RuntimeError::Storage)
}

pub fn initialize_network(capability: u64) -> bool {
    let mut network = network_lock();
    if network.is_some() {
        return false;
    }
    *network = Some(SocketApi::new(SyscallDevice::new(capability)));
    true
}

/// Remove only stale `tempfile::tempdir()` trees left under `/tmp` by an
/// earlier Servo process exit. The pinned target tempfile backend uses the
/// exact `.tmp` plus six alphanumeric characters naming scheme. Servo's
/// storage threads use these trees as temporary storage, so they must not
/// accumulate on the small persistent VFS after a process exits without
/// running Rust destructors.
#[cfg(feature = "browser-storage")]
pub fn cleanup_m18_servo_temp_directories() -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let mut entries = [DirectoryEntry::empty(); SERVO_TEMP_DIRECTORY_ENTRIES];
    let count = volume
        .list_directory_path(b"/tmp", &mut entries)
        .map_err(RuntimeError::Storage)?;
    let mut candidates = [([0; MAX_PATH_LENGTH], 0); SERVO_TEMP_DIRECTORY_ENTRIES];
    let mut candidate_count = 0;
    for entry in entries.iter().take(count) {
        if !crate::fs::is_servo_tempdir_name(entry.name()) {
            continue;
        }
        if entry.file_type != 2 {
            return Err(RuntimeError::Storage(StorageError::NotDirectory));
        }
        let (path, length) =
            append_child_path(b"/tmp", entry.name()).map_err(RuntimeError::Storage)?;
        if !is_servo_temp_layout(volume, &path[..length]).map_err(RuntimeError::Storage)? {
            continue;
        }
        candidates[candidate_count] = (path, length);
        candidate_count += 1;
    }

    // Validate every candidate before removing anything, so an unexpected
    // VFS shape cannot leave only part of the old temporary storage pruned.
    for (path, length) in candidates.iter().take(candidate_count) {
        remove_temporary_tree(volume, &path[..*length], 0).map_err(RuntimeError::Storage)?;
        volume
            .rmdir_path(&path[..*length])
            .map_err(RuntimeError::Storage)?;
    }
    Ok(candidate_count)
}

#[cfg(feature = "browser-storage")]
fn is_servo_temp_layout<D: libnagi::storage::BlockDevice>(
    volume: &mut Vfs<D>,
    path: &[u8],
) -> Result<bool, StorageError> {
    let mut root_entries = [DirectoryEntry::empty(); 2];
    let root_count = volume.list_directory_path(path, &mut root_entries)?;
    if root_count != 1
        || root_entries[0].file_type != 2
        || !matches!(root_entries[0].name(), b"clientstorage" | b"cachestorage")
    {
        return Ok(false);
    }

    let (storage_path, storage_path_length) = append_child_path(path, root_entries[0].name())?;
    let mut storage_entries = [DirectoryEntry::empty(); 2];
    let storage_count =
        volume.list_directory_path(&storage_path[..storage_path_length], &mut storage_entries)?;
    if storage_count != 1
        || storage_entries[0].file_type != 2
        || storage_entries[0].name() != b"default_v1"
    {
        return Ok(false);
    }
    let (profile_path, profile_path_length) = append_child_path(
        &storage_path[..storage_path_length],
        storage_entries[0].name(),
    )?;
    validate_temporary_tree(volume, &profile_path[..profile_path_length], 0)?;
    Ok(true)
}

#[cfg(feature = "browser-storage")]
fn validate_temporary_tree<D: libnagi::storage::BlockDevice>(
    volume: &mut Vfs<D>,
    path: &[u8],
    depth: usize,
) -> Result<(), StorageError> {
    if depth >= SERVO_TEMP_DIRECTORY_MAX_DEPTH {
        return Err(StorageError::Corrupt);
    }
    let mut entries = [DirectoryEntry::empty(); SERVO_TEMP_DIRECTORY_ENTRIES];
    let count = volume.list_directory_path(path, &mut entries)?;
    for entry in entries.iter().take(count) {
        match entry.file_type {
            1 => {}
            2 => {
                let (child_path, length) = append_child_path(path, entry.name())?;
                validate_temporary_tree(volume, &child_path[..length], depth + 1)?;
            }
            _ => return Err(StorageError::InvalidHandle),
        }
    }
    Ok(())
}

#[cfg(feature = "browser-storage")]
fn remove_temporary_tree<D: libnagi::storage::BlockDevice>(
    volume: &mut Vfs<D>,
    path: &[u8],
    depth: usize,
) -> Result<(), StorageError> {
    if depth >= SERVO_TEMP_DIRECTORY_MAX_DEPTH {
        return Err(StorageError::Corrupt);
    }
    let mut entries = [DirectoryEntry::empty(); SERVO_TEMP_DIRECTORY_ENTRIES];
    let count = volume.list_directory_path(path, &mut entries)?;
    for entry in entries.iter().take(count) {
        let (child_path, length) = append_child_path(path, entry.name())?;
        match entry.file_type {
            1 => volume.remove_path(&child_path[..length])?,
            2 => {
                remove_temporary_tree(volume, &child_path[..length], depth + 1)?;
                volume.rmdir_path(&child_path[..length])?;
            }
            _ => return Err(StorageError::InvalidHandle),
        }
    }
    Ok(())
}

#[cfg(feature = "browser-storage")]
fn append_child_path(
    parent: &[u8],
    name: &[u8],
) -> Result<([u8; MAX_PATH_LENGTH], usize), StorageError> {
    let length = parent
        .len()
        .checked_add(1)
        .and_then(|length| length.checked_add(name.len()))
        .filter(|length| *length < MAX_PATH_LENGTH)
        .ok_or(StorageError::NameTooLong)?;
    let mut path = [0; MAX_PATH_LENGTH];
    path[..parent.len()].copy_from_slice(parent);
    path[parent.len()] = b'/';
    path[parent.len() + 1..length].copy_from_slice(name);
    Ok((path, length))
}

pub fn resolve_ipv4(name: &str) -> Result<Ipv4Address, RuntimeError> {
    trace_m18_dns(b"Nagi M18 network DNS waiting for lock\r\n");
    let mut network = network_lock();
    trace_m18_dns(b"Nagi M18 network DNS lock acquired\r\n");
    let network = network.as_mut().ok_or(RuntimeError::NotInitialized)?;
    trace_m18_dns(b"Nagi M18 network DNS lookup started\r\n");
    let result = network.resolve_ipv4(name);
    trace_m18_dns(if result.is_ok() {
        b"Nagi M18 network DNS lookup completed\r\n"
    } else {
        b"Nagi M18 network DNS lookup failed\r\n"
    });
    result.map_err(RuntimeError::Network)
}

pub fn default_gateway() -> Result<Ipv4Address, RuntimeError> {
    network_lock()
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .dhcp_gateway()
        .map_err(RuntimeError::Network)
}

pub fn http_get(
    target: Ipv4Address,
    target_port: u16,
    path: &[u8],
    expected_body: &[u8],
    response: &mut [u8],
) -> Result<usize, RuntimeError> {
    network_lock()
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .http_get(target, target_port, path, expected_body, response)
        .map_err(RuntimeError::Network)
}

pub fn register_static_file(
    path: &'static [u8],
    data: &'static [u8],
) -> Result<(), StaticFileError> {
    STATIC_FILES.lock().register(path, data)
}

fn static_file_metadata(length: usize) -> FileMetadata {
    FileMetadata {
        inode: 0,
        mode: STATIC_FILE_MODE,
        uid: 0,
        gid: 0,
        size: length as u32,
        atime: 0,
        ctime: 0,
        mtime: 0,
        blocks: length.div_ceil(512) as u32,
    }
}

pub fn open(name: &[u8], create: bool, truncate: bool) -> Result<i32, RuntimeError> {
    if name == b"/dev/urandom" {
        return allocate_descriptor(FdEntry::Random);
    }
    let static_file = STATIC_FILES.lock().lookup(name);
    if let Some(file) = static_file {
        if truncate {
            return Err(RuntimeError::ReadOnly);
        }
        // SAFETY: the bytes are 'static and the callback only reads ranges
        // the callback-file wrapper has bounded by their length.
        return unsafe {
            open_readonly_callback(
                file.data.as_ptr() as *mut core::ffi::c_void,
                file.data.len() as u64,
                read_static,
            )
        };
    }

    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let handle = match volume.open_path(name) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) if create => {
            volume.create_path(name).map_err(RuntimeError::Storage)?
        }
        Err(error) => return Err(RuntimeError::Storage(error)),
    };
    if truncate {
        volume.write(handle, &[]).map_err(RuntimeError::Storage)?;
    }
    drop(filesystem);

    allocate_descriptor(FdEntry::File { handle, offset: 0 })
}

pub unsafe fn open_readonly_callback(
    context: *mut core::ffi::c_void,
    length: u64,
    read_at: ReadAtCallback,
) -> Result<i32, RuntimeError> {
    let file = unsafe { ReadOnlyCallbackFile::new(context, length, read_at) }
        .map_err(map_callback_file_error)?;
    allocate_descriptor(FdEntry::CallbackFile(file))
}

fn allocate_descriptor(entry: FdEntry) -> Result<i32, RuntimeError> {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let Some((index, slot)) = descriptors
        .iter_mut()
        .enumerate()
        .skip(3)
        .find(|(_, slot)| slot.is_none())
    else {
        return Err(RuntimeError::Storage(StorageError::Capacity));
    };
    *slot = Some(entry);
    Ok(index as i32)
}

pub fn remove(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.remove_path(name).map_err(RuntimeError::Storage)
}

pub fn mkdir(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.mkdir_path(name).map_err(RuntimeError::Storage)
}

pub fn ensure_directory(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume
        .ensure_directory_path(name)
        .map_err(RuntimeError::Storage)
}

pub fn rmdir(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.rmdir_path(name).map_err(RuntimeError::Storage)
}

pub fn metadata_path(name: &[u8]) -> Result<FileMetadata, RuntimeError> {
    if let Some(file) = STATIC_FILES.lock().lookup(name) {
        return Ok(static_file_metadata(file.data.len()));
    }
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.metadata_path(name).map_err(RuntimeError::Storage)
}

pub fn list_root(
    entries: &mut [DirectoryEntry; MAX_DIRECTORY_ENTRIES],
) -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.list_root(entries).map_err(RuntimeError::Storage)
}

pub fn list_directory(
    name: &[u8],
    entries: &mut [DirectoryEntry; MAX_DIRECTORY_ENTRIES],
) -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume
        .list_directory_path(name, entries)
        .map_err(RuntimeError::Storage)
}

pub fn socket() -> Result<i32, RuntimeError> {
    if network_lock().is_none() {
        return Err(RuntimeError::NotInitialized);
    }
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let Some((index, slot)) = descriptors
        .iter_mut()
        .enumerate()
        .skip(3)
        .find(|(_, slot)| slot.is_none())
    else {
        return Err(RuntimeError::Storage(StorageError::Capacity));
    };
    *slot = Some(FdEntry::Socket {
        connected: false,
        tcp_connection: None,
        peer: None,
        last_error: 0,
        nonblocking: false,
        read_shutdown: false,
        write_shutdown: false,
        nagle_enabled: true,
        timeout: None,
    });
    Ok(index as i32)
}

pub fn pipe2(flags: i32) -> Result<(i32, i32), RuntimeError> {
    if flags & !(O_NONBLOCK | O_CLOEXEC) != 0 {
        return Err(RuntimeError::Unsupported);
    }
    let nonblocking = flags & O_NONBLOCK != 0;
    let pipe_index = {
        let mut pipes = PIPES.lock();
        let Some((index, slot)) = pipes
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        else {
            return Err(RuntimeError::Storage(StorageError::Capacity));
        };
        *slot = Some(Pipe::new());
        index
    };

    let mut descriptors = FILE_DESCRIPTORS.lock();
    let mut free = descriptors
        .iter_mut()
        .enumerate()
        .skip(3)
        .filter(|(_, slot)| slot.is_none());
    let Some((read_index, read_slot)) = free.next() else {
        PIPES.lock()[pipe_index] = None;
        return Err(RuntimeError::Storage(StorageError::Capacity));
    };
    let Some((write_index, write_slot)) = free.next() else {
        PIPES.lock()[pipe_index] = None;
        return Err(RuntimeError::Storage(StorageError::Capacity));
    };
    *read_slot = Some(FdEntry::PipeRead {
        pipe: pipe_index,
        nonblocking,
    });
    *write_slot = Some(FdEntry::PipeWrite {
        pipe: pipe_index,
        nonblocking,
    });
    Ok((read_index as i32, write_index as i32))
}

pub fn pipe() -> Result<(i32, i32), RuntimeError> {
    pipe2(0)
}

/// Duplicate a descriptor into the requested slot without crossing into a
/// host descriptor table. The current M17 VFS descriptor model can safely
/// duplicate regular files; sockets and pipe endpoints need shared ownership
/// bookkeeping that is not yet part of this bounded slice, so they fail
/// closed instead of pretending that a shallow copy is POSIX-correct.
pub fn dup2(old_fd: i32, new_fd: i32) -> Result<i32, RuntimeError> {
    let source = descriptor(old_fd)?;
    if new_fd < 0 || new_fd as usize >= 32 {
        return Err(RuntimeError::InvalidFd);
    }
    if old_fd == new_fd {
        return Ok(new_fd);
    }
    if !matches!(source, FdEntry::File { .. } | FdEntry::Random) {
        return Err(RuntimeError::Unsupported);
    }
    if descriptor(new_fd).is_ok() {
        close(new_fd)?;
    }
    FILE_DESCRIPTORS.lock()[new_fd as usize] = Some(source);
    Ok(new_fd)
}

pub fn fcntl(fd: i32, command: i32, argument: i32) -> Result<i32, RuntimeError> {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let entry = descriptors
        .get_mut(fd as usize)
        .and_then(Option::as_mut)
        .ok_or(RuntimeError::InvalidFd)?;
    match command {
        F_GETFD => Ok(FD_CLOEXEC),
        F_SETFD => Ok(0),
        F_GETFL => match entry {
            FdEntry::PipeRead { nonblocking, .. } | FdEntry::PipeWrite { nonblocking, .. } => {
                Ok(if *nonblocking { O_NONBLOCK } else { 0 })
            }
            FdEntry::Socket { nonblocking, .. } => Ok(socket_status_flags(*nonblocking)),
            _ => Ok(0),
        },
        F_SETFL => {
            let nonblocking = socket_is_nonblocking(argument);
            match entry {
                FdEntry::PipeRead {
                    nonblocking: current,
                    ..
                }
                | FdEntry::PipeWrite {
                    nonblocking: current,
                    ..
                }
                | FdEntry::Socket {
                    nonblocking: current,
                    ..
                } => *current = nonblocking,
                _ => {}
            }
            Ok(0)
        }
        _ => Err(RuntimeError::Unsupported),
    }
}

pub fn connect(fd: i32, address: Ipv4Address, port: u16) -> Result<(), RuntimeError> {
    let (nagle_enabled, timeout) = {
        let descriptors = FILE_DESCRIPTORS.lock();
        match descriptors
            .get(fd as usize)
            .and_then(Option::as_ref)
            .copied()
        {
            Some(FdEntry::Socket {
                connected: false,
                nagle_enabled,
                timeout,
                ..
            }) => (nagle_enabled, timeout),
            Some(FdEntry::Socket {
                connected: true, ..
            }) => return Err(RuntimeError::InvalidFd),
            _ => return Err(RuntimeError::InvalidFd),
        }
    };
    trace_m18_network(b"Nagi M18 network TCP connect waiting for lock\r\n");
    trace_m18_tcp_connect(b"Nagi M18 network TCP connect attempt started\r\n");
    let connect_result = {
        let mut network_guard = network_lock();
        trace_m18_network(b"Nagi M18 network TCP connect lock acquired\r\n");
        let network = network_guard.as_mut().ok_or(RuntimeError::NotInitialized)?;
        trace_m18_network(b"Nagi M18 network TCP handshake started\r\n");
        match network.tcp_connect(address, port) {
            Err(error) => {
                trace_m18_network(b"Nagi M18 network TCP handshake failed\r\n");
                trace_m18_tcp_connect(match error {
                    NetError::Unsupported => {
                        b"Nagi M18 network TCP connect attempt failed: unsupported\r\n"
                    }
                    NetError::ConnectionReset => {
                        b"Nagi M18 network TCP connect attempt failed: reset\r\n"
                    }
                    _ => b"Nagi M18 network TCP connect attempt failed: other\r\n",
                });
                Err(RuntimeError::Network(error))
            }
            Ok(connection) => {
                trace_m18_network(b"Nagi M18 network TCP handshake completed\r\n");
                trace_m18_tcp_connect(b"Nagi M18 network TCP connect attempt succeeded\r\n");
                if let Err(error) = network
                    .tcp_set_nagle(connection, nagle_enabled)
                    .and_then(|()| network.tcp_set_timeout(connection, timeout))
                {
                    let _ = network.tcp_close(connection);
                    Err(RuntimeError::Network(error))
                } else {
                    trace_m18_network(b"Nagi M18 network TCP socket options completed\r\n");
                    Ok(connection)
                }
            }
        }
    };
    let connection = match connect_result {
        Ok(connection) => connection,
        Err(error) => {
            record_socket_error(fd, map_error(error));
            return Err(error);
        }
    };
    let mut descriptors = FILE_DESCRIPTORS.lock();
    match descriptors.get_mut(fd as usize).and_then(Option::as_mut) {
        Some(FdEntry::Socket {
            connected,
            tcp_connection,
            peer,
            ..
        }) => {
            *connected = true;
            *tcp_connection = Some(connection);
            *peer = Some((address, port));
            trace_m18_network(b"Nagi M18 network TCP connect API returned\r\n");
            Ok(())
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

fn record_socket_error(fd: i32, error: i32) {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    if let Some(FdEntry::Socket { last_error, .. }) =
        descriptors.get_mut(fd as usize).and_then(Option::as_mut)
    {
        *last_error = error;
    }
}

pub fn take_socket_error(fd: i32) -> Result<i32, RuntimeError> {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    match descriptors.get_mut(fd as usize).and_then(Option::as_mut) {
        Some(FdEntry::Socket { last_error, .. }) => {
            Ok(crate::net::take_pending_socket_error(last_error))
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn peer_name(fd: i32) -> Result<(Ipv4Address, u16), RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Socket {
            connected: true,
            peer: Some(peer),
            ..
        } => Ok(peer),
        FdEntry::Socket { .. } => Err(RuntimeError::NotConnected),
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn local_name(fd: i32) -> Result<(Ipv4Address, u16), RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Socket {
            connected: true,
            tcp_connection: Some(connection),
            ..
        } => network_lock()
            .as_ref()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_local_name(connection)
            .map_err(RuntimeError::Network),
        FdEntry::Socket { .. } => Err(RuntimeError::NotConnected),
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn close(fd: i32) -> Result<(), RuntimeError> {
    let entry = {
        let descriptors = FILE_DESCRIPTORS.lock();
        descriptors
            .get(fd as usize)
            .and_then(Option::as_ref)
            .copied()
            .ok_or(RuntimeError::InvalidFd)?
    };
    if matches!(
        entry,
        FdEntry::Socket {
            connected: true,
            tcp_connection: Some(_),
            ..
        }
    ) {
        let FdEntry::Socket {
            tcp_connection: Some(connection),
            ..
        } = entry
        else {
            unreachable!();
        };
        network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_close(connection)
            .map_err(RuntimeError::Network)?;
    }
    match entry {
        FdEntry::PipeRead { pipe, .. } => close_pipe_endpoint(pipe, true),
        FdEntry::PipeWrite { pipe, .. } => close_pipe_endpoint(pipe, false),
        _ => {}
    }
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let slot = descriptors
        .get_mut(fd as usize)
        .ok_or(RuntimeError::InvalidFd)?;
    if slot.take().is_none() {
        return Err(RuntimeError::InvalidFd);
    }
    Ok(())
}

pub fn shutdown(fd: i32, how: i32) -> Result<(), RuntimeError> {
    let entry = descriptor(fd)?;
    let FdEntry::Socket {
        connected,
        tcp_connection,
        ..
    } = entry
    else {
        return Err(RuntimeError::InvalidFd);
    };
    let Some(connection) = tcp_connection.filter(|_| connected) else {
        return Err(RuntimeError::NotConnected);
    };

    if how == 1 || how == 2 {
        network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_shutdown_write(connection)
            .map_err(RuntimeError::Network)?;
    }

    let mut descriptors = FILE_DESCRIPTORS.lock();
    let Some(Some(FdEntry::Socket {
        read_shutdown: current_read_shutdown,
        write_shutdown: current_write_shutdown,
        ..
    })) = descriptors.get_mut(fd as usize)
    else {
        return Err(RuntimeError::InvalidFd);
    };
    if how == 0 || how == 2 {
        *current_read_shutdown = true;
    }
    if how == 1 || how == 2 {
        *current_write_shutdown = true;
    }
    Ok(())
}

pub fn set_tcp_nodelay(fd: i32, enabled: bool) -> Result<(), RuntimeError> {
    let entry = descriptor(fd)?;
    let FdEntry::Socket {
        connected,
        tcp_connection,
        ..
    } = entry
    else {
        return Err(RuntimeError::InvalidFd);
    };
    if connected {
        let connection = tcp_connection.ok_or(RuntimeError::NotConnected)?;
        network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_set_nagle(connection, !enabled)
            .map_err(RuntimeError::Network)?;
    }
    let mut descriptors = FILE_DESCRIPTORS.lock();
    match descriptors.get_mut(fd as usize).and_then(Option::as_mut) {
        Some(FdEntry::Socket { nagle_enabled, .. }) => {
            *nagle_enabled = !enabled;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn set_socket_timeout(fd: i32, timeout: Option<Duration>) -> Result<(), RuntimeError> {
    let entry = descriptor(fd)?;
    let FdEntry::Socket {
        connected,
        tcp_connection,
        ..
    } = entry
    else {
        return Err(RuntimeError::InvalidFd);
    };
    if connected {
        let connection = tcp_connection.ok_or(RuntimeError::NotConnected)?;
        network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_set_timeout(connection, timeout)
            .map_err(RuntimeError::Network)?;
    }
    let mut descriptors = FILE_DESCRIPTORS.lock();
    match descriptors.get_mut(fd as usize).and_then(Option::as_mut) {
        Some(FdEntry::Socket {
            timeout: current_timeout,
            ..
        }) => {
            *current_timeout = timeout;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn tcp_nodelay(fd: i32) -> Result<bool, RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Socket { nagle_enabled, .. } => Ok(!nagle_enabled),
        _ => Err(RuntimeError::InvalidFd),
    }
}

pub fn socket_timeout(fd: i32) -> Result<Option<Duration>, RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Socket { timeout, .. } => Ok(timeout),
        _ => Err(RuntimeError::InvalidFd),
    }
}

fn close_pipe_endpoint(pipe_index: usize, reader: bool) {
    let mut pipes = PIPES.lock();
    let remove = {
        let Some(pipe) = pipes.get_mut(pipe_index).and_then(Option::as_mut) else {
            return;
        };
        if reader {
            pipe.reader_open = false;
        } else {
            pipe.writer_open = false;
        }
        !pipe.reader_open && !pipe.writer_open
    };
    if remove {
        pipes[pipe_index] = None;
    }
}

pub fn read(fd: i32, destination: &mut [u8]) -> Result<usize, RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Random => {
            if fill_random(destination) {
                Ok(destination.len())
            } else {
                Err(RuntimeError::EntropyUnavailable)
            }
        }
        FdEntry::File { handle, offset } => {
            let count = read_at(fd, offset, destination)?;
            update_offset(fd, handle, offset + count)?;
            Ok(count)
        }
        FdEntry::CallbackFile(mut file) => {
            let count = file.read(destination).map_err(map_callback_file_error)?;
            update_callback_file(fd, file)?;
            Ok(count)
        }
        FdEntry::Socket {
            connected: true,
            read_shutdown: true,
            ..
        } => Ok(0),
        FdEntry::Socket {
            connected: true,
            read_shutdown: false,
            tcp_connection: Some(connection),
            nonblocking,
            ..
        } => {
            trace_m18_network(b"Nagi M18 network TCP receive waiting for lock\r\n");
            let mut network = network_lock();
            trace_m18_network(b"Nagi M18 network TCP receive lock acquired\r\n");
            let network = network.as_mut().ok_or(RuntimeError::NotInitialized)?;
            trace_m18_network(b"Nagi M18 network TCP receive started\r\n");
            let result = if nonblocking {
                network.tcp_try_receive(connection, destination)
            } else {
                network.tcp_receive(connection, destination)
            };
            trace_m18_network(if result.is_ok() {
                b"Nagi M18 network TCP receive completed\r\n"
            } else {
                b"Nagi M18 network TCP receive failed\r\n"
            });
            result.map_err(RuntimeError::Network)
        }
        FdEntry::Socket {
            connected: true,
            tcp_connection: None,
            ..
        } => Err(RuntimeError::NotConnected),
        FdEntry::Socket {
            connected: false, ..
        } => Err(RuntimeError::InvalidFd),
        FdEntry::PipeRead { pipe, nonblocking } => read_pipe(pipe, nonblocking, destination),
        FdEntry::PipeWrite { .. } => Err(RuntimeError::InvalidFd),
    }
}

fn read_pipe(
    pipe_index: usize,
    nonblocking: bool,
    destination: &mut [u8],
) -> Result<usize, RuntimeError> {
    loop {
        let mut pipes = PIPES.lock();
        let Some(pipe) = pipes.get_mut(pipe_index).and_then(Option::as_mut) else {
            return Err(RuntimeError::InvalidFd);
        };
        if pipe.length != 0 {
            let count = core::cmp::min(destination.len(), pipe.length);
            for byte in destination.iter_mut().take(count) {
                *byte = pipe.buffer[pipe.head];
                pipe.head = (pipe.head + 1) % PIPE_CAPACITY;
            }
            pipe.length -= count;
            return Ok(count);
        }
        if !pipe.writer_open {
            return Ok(0);
        }
        if nonblocking {
            return Err(RuntimeError::WouldBlock);
        }
        drop(pipes);
        if GuestClock.sleep_ns(1_000_000).is_err() {
            return Err(RuntimeError::WouldBlock);
        }
    }
}

pub fn read_at(fd: i32, offset: usize, destination: &mut [u8]) -> Result<usize, RuntimeError> {
    let entry = descriptor(fd)?;
    if let FdEntry::CallbackFile(file) = entry {
        return file
            .read_at(offset, destination)
            .map_err(map_callback_file_error);
    }
    let FdEntry::File { handle, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume
        .read_at(handle, offset, destination)
        .map_err(RuntimeError::Storage)
}

pub fn readiness(fd: i32, requested: i16) -> Result<i16, RuntimeError> {
    if fd == 1 || fd == 2 {
        return Ok(requested & 0x0004);
    }
    match descriptor(fd)? {
        FdEntry::Random => Ok(requested & POLLIN),
        FdEntry::File { handle, offset } => {
            let mut ready = requested & 0x0004;
            if requested & 0x0001 != 0 && offset < file_size(handle)? {
                ready |= 0x0001;
            }
            Ok(ready)
        }
        FdEntry::CallbackFile(file) => {
            let mut ready = 0;
            if requested & POLLIN != 0 && file.offset() < file.len() {
                ready |= POLLIN;
            }
            Ok(ready)
        }
        FdEntry::Socket {
            connected: true,
            read_shutdown: true,
            write_shutdown,
            ..
        } => {
            let mut ready = if requested & POLLIN != 0 {
                POLLIN | POLLHUP
            } else {
                0
            };
            if !write_shutdown && requested & POLLOUT != 0 {
                ready |= POLLOUT;
            }
            Ok(ready)
        }
        FdEntry::Socket {
            connected: true,
            write_shutdown: true,
            tcp_connection: Some(connection),
            ..
        } => network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_ready(connection, requested & !POLLOUT)
            .map(|ready| ready | (requested & POLLOUT))
            .map_err(RuntimeError::Network),
        FdEntry::Socket {
            connected: true,
            tcp_connection: Some(connection),
            ..
        } => network_lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_ready(connection, requested)
            .map_err(RuntimeError::Network),
        FdEntry::Socket {
            connected: true,
            tcp_connection: None,
            ..
        } => Err(RuntimeError::NotConnected),
        FdEntry::Socket {
            connected: false, ..
        } => Ok(requested & POLLOUT),
        FdEntry::PipeRead { pipe, .. } => pipe_readiness(pipe, requested),
        FdEntry::PipeWrite { pipe, .. } => pipe_write_readiness(pipe, requested),
    }
}

fn pipe_readiness(pipe_index: usize, requested: i16) -> Result<i16, RuntimeError> {
    let pipes = PIPES.lock();
    let Some(pipe) = pipes.get(pipe_index).and_then(Option::as_ref) else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut ready = 0;
    if requested & POLLIN != 0 && (pipe.length != 0 || !pipe.writer_open) {
        ready |= POLLIN;
    }
    if !pipe.writer_open {
        ready |= POLLHUP;
    }
    Ok(ready)
}

fn pipe_write_readiness(pipe_index: usize, requested: i16) -> Result<i16, RuntimeError> {
    let pipes = PIPES.lock();
    let Some(pipe) = pipes.get(pipe_index).and_then(Option::as_ref) else {
        return Err(RuntimeError::InvalidFd);
    };
    if !pipe.reader_open {
        return Ok(POLLERR | POLLHUP);
    }
    Ok(if requested & POLLOUT != 0 && pipe.length < PIPE_CAPACITY {
        POLLOUT
    } else {
        0
    })
}

pub fn write(fd: i32, bytes: &[u8]) -> Result<usize, RuntimeError> {
    match descriptor(fd)? {
        FdEntry::Random => Err(RuntimeError::InvalidFd),
        FdEntry::File { handle, offset } => {
            let mut filesystem = FILESYSTEM.lock();
            let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
            volume
                .write_at(handle, offset, bytes)
                .map_err(RuntimeError::Storage)?;
            drop(filesystem);
            update_offset(fd, handle, offset + bytes.len())?;
            Ok(bytes.len())
        }
        FdEntry::CallbackFile(_) => Err(RuntimeError::ReadOnly),
        FdEntry::Socket {
            connected: true,
            write_shutdown: true,
            ..
        } => Err(RuntimeError::Shutdown),
        FdEntry::Socket {
            connected: true,
            write_shutdown: false,
            tcp_connection: Some(connection),
            nonblocking,
            ..
        } => {
            trace_m18_network(b"Nagi M18 network TCP send waiting for lock\r\n");
            let mut network = network_lock();
            trace_m18_network(b"Nagi M18 network TCP send lock acquired\r\n");
            let network = network.as_mut().ok_or(RuntimeError::NotInitialized)?;
            trace_m18_network(b"Nagi M18 network TCP send started\r\n");
            let result = if nonblocking {
                network.tcp_try_send(connection, bytes)
            } else {
                network.tcp_send(connection, bytes)
            };
            trace_m18_network(if result.is_ok() {
                b"Nagi M18 network TCP send completed\r\n"
            } else {
                b"Nagi M18 network TCP send failed\r\n"
            });
            result.map_err(RuntimeError::Network)
        }
        FdEntry::Socket {
            connected: true,
            tcp_connection: None,
            ..
        } => Err(RuntimeError::NotConnected),
        FdEntry::Socket {
            connected: false, ..
        } => Err(RuntimeError::InvalidFd),
        FdEntry::PipeWrite { pipe, nonblocking } => write_pipe(pipe, nonblocking, bytes),
        FdEntry::PipeRead { .. } => Err(RuntimeError::InvalidFd),
    }
}

fn write_pipe(pipe_index: usize, nonblocking: bool, bytes: &[u8]) -> Result<usize, RuntimeError> {
    if bytes.is_empty() {
        return Ok(0);
    }
    loop {
        let mut pipes = PIPES.lock();
        let Some(pipe) = pipes.get_mut(pipe_index).and_then(Option::as_mut) else {
            return Err(RuntimeError::InvalidFd);
        };
        if !pipe.reader_open {
            return Err(RuntimeError::BrokenPipe);
        }
        let available = PIPE_CAPACITY - pipe.length;
        if available != 0 {
            let count = core::cmp::min(bytes.len(), available);
            let mut tail = (pipe.head + pipe.length) % PIPE_CAPACITY;
            for &byte in bytes.iter().take(count) {
                pipe.buffer[tail] = byte;
                tail = (tail + 1) % PIPE_CAPACITY;
            }
            pipe.length += count;
            return Ok(count);
        }
        if nonblocking {
            return Err(RuntimeError::WouldBlock);
        }
        drop(pipes);
        if GuestClock.sleep_ns(1_000_000).is_err() {
            return Err(RuntimeError::WouldBlock);
        }
    }
}

pub fn seek(fd: i32, offset: isize, whence: i32) -> Result<usize, RuntimeError> {
    let entry = descriptor(fd)?;
    if let FdEntry::CallbackFile(mut file) = entry {
        let position = file
            .seek(offset as i64, whence)
            .map_err(map_callback_file_error)?;
        update_callback_file(fd, file)?;
        return Ok(position);
    }
    let FdEntry::File {
        handle,
        offset: current,
    } = entry
    else {
        return Err(RuntimeError::InvalidFd);
    };
    let next = match whence {
        0 => offset,
        1 => current as isize + offset,
        2 => file_size(handle)? as isize + offset,
        _ => return Err(RuntimeError::InvalidFd),
    };
    if next < 0 {
        return Err(RuntimeError::InvalidFd);
    }
    let next = next as usize;
    update_offset(fd, handle, next)?;
    Ok(next)
}

pub fn size(fd: i32) -> Result<usize, RuntimeError> {
    let entry = descriptor(fd)?;
    if let FdEntry::CallbackFile(file) = entry {
        return Ok(file.len());
    }
    let FdEntry::File { handle, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    file_size(handle)
}

pub fn metadata(fd: i32) -> Result<FileMetadata, RuntimeError> {
    let entry = descriptor(fd)?;
    if let FdEntry::CallbackFile(file) = entry {
        return Ok(static_file_metadata(file.len()));
    }
    let FdEntry::File { handle, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .metadata(handle)
        .map_err(RuntimeError::Storage)
}

pub fn truncate(fd: i32, length: usize) -> Result<(), RuntimeError> {
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .truncate(handle, length)
        .map_err(RuntimeError::Storage)
}

pub fn set_mode(fd: i32, mode: u16) -> Result<(), RuntimeError> {
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .set_mode(handle, mode)
        .map_err(RuntimeError::Storage)
}

pub fn set_owner(fd: i32, uid: Option<u16>, gid: Option<u16>) -> Result<(), RuntimeError> {
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .set_owner(handle, uid, gid)
        .map_err(RuntimeError::Storage)
}

pub fn set_times(fd: i32, atime: u32, mtime: u32) -> Result<(), RuntimeError> {
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .set_times(handle, atime, mtime)
        .map_err(RuntimeError::Storage)
}

pub fn sync(fd: i32) -> Result<(), RuntimeError> {
    let FdEntry::File { .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    filesystem
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .flush()
        .map_err(RuntimeError::Storage)
}

fn descriptor(fd: i32) -> Result<FdEntry, RuntimeError> {
    let descriptors = FILE_DESCRIPTORS.lock();
    descriptors
        .get(fd as usize)
        .and_then(Option::as_ref)
        .copied()
        .ok_or(RuntimeError::InvalidFd)
}

fn update_offset(fd: i32, handle: FileHandle, offset: usize) -> Result<(), RuntimeError> {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let entry = descriptors
        .get_mut(fd as usize)
        .and_then(Option::as_mut)
        .ok_or(RuntimeError::InvalidFd)?;
    match entry {
        FdEntry::File {
            handle: entry_handle,
            offset: entry_offset,
        } if *entry_handle == handle => {
            *entry_offset = offset;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

fn update_callback_file(fd: i32, file: ReadOnlyCallbackFile) -> Result<(), RuntimeError> {
    let mut descriptors = FILE_DESCRIPTORS.lock();
    let entry = descriptors
        .get_mut(fd as usize)
        .and_then(Option::as_mut)
        .ok_or(RuntimeError::InvalidFd)?;
    match entry {
        FdEntry::CallbackFile(current) => {
            *current = file;
            Ok(())
        }
        _ => Err(RuntimeError::InvalidFd),
    }
}

fn map_callback_file_error(error: CallbackFileError) -> RuntimeError {
    match error {
        CallbackFileError::InvalidRange => RuntimeError::InvalidArgument,
        CallbackFileError::ReadFailed => RuntimeError::CallbackReadFailed,
    }
}

fn file_size(handle: FileHandle) -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume
        .metadata(handle)
        .map(|metadata| metadata.size as usize)
        .map_err(RuntimeError::Storage)
}

pub fn map_error(error: RuntimeError) -> i32 {
    match error {
        RuntimeError::NotInitialized => 38,
        RuntimeError::InvalidFd => 9,
        RuntimeError::InvalidArgument => 22,
        RuntimeError::EntropyUnavailable => 5,
        RuntimeError::Storage(StorageError::NotFound) => 2,
        RuntimeError::Storage(StorageError::AlreadyExists) => 17,
        RuntimeError::Storage(StorageError::NameTooLong) => 36,
        RuntimeError::Storage(StorageError::InvalidName) => 22,
        RuntimeError::Storage(StorageError::NotDirectory) => 20,
        RuntimeError::Storage(StorageError::IsDirectory) => 21,
        RuntimeError::Storage(StorageError::DirectoryNotEmpty) => 39,
        RuntimeError::Storage(StorageError::DirectoryFull) => 28,
        RuntimeError::Storage(StorageError::FileTooLarge) => 27,
        RuntimeError::Storage(StorageError::Capacity) => 12,
        RuntimeError::Network(error) => crate::net::network_errno(error),
        RuntimeError::NotConnected => 107,
        RuntimeError::Shutdown => 108,
        RuntimeError::WouldBlock => 11,
        RuntimeError::BrokenPipe => 32,
        RuntimeError::Unsupported => 95,
        RuntimeError::ReadOnly => 30,
        RuntimeError::CallbackReadFailed => 5,
        RuntimeError::Storage(_) => 5,
    }
}

#[cfg(target_os = "nagi")]
fn fill_random(bytes: &mut [u8]) -> bool {
    libnagi::random_fill(bytes)
}

// Host builds type-check the POSIX adapter but never supply guest entropy.
// Returning failure keeps this compatibility path fail-closed off-target.
#[cfg(not(target_os = "nagi"))]
fn fill_random(_bytes: &mut [u8]) -> bool {
    false
}
