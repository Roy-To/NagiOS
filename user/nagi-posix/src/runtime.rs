use core::time::Duration;
use libnagi::storage::{
    DirectoryEntry, FileHandle, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE,
    MAX_DIRECTORY_ENTRIES,
};
use nagi_net::{Ipv4Address, NetError, SocketApi, SyscallDevice};
use nagi_pal::sync::SpinMutex;
use nagi_pal::time::{Clock, GuestClock};

const PIPE_CAPACITY: usize = 4096;
const O_NONBLOCK: i32 = 0x0004_0000;
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

#[derive(Clone, Copy)]
enum FdEntry {
    File {
        handle: FileHandle,
        offset: usize,
    },
    Socket {
        connected: bool,
        peer: Option<(Ipv4Address, u16)>,
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
static PIPES: SpinMutex<[Option<Pipe>; 8]> = SpinMutex::new([None; 8]);
static NETWORK: SpinMutex<Option<SocketApi<SyscallDevice>>> = SpinMutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    NotInitialized,
    InvalidFd,
    Storage(StorageError),
    Network(NetError),
    NotConnected,
    Shutdown,
    WouldBlock,
    BrokenPipe,
    Unsupported,
}

pub fn initialize(capability: u64) -> bool {
    let Ok((volume, _formatted)) = Vfs::mount_or_format(SyscallBlockDevice::new(capability)) else {
        return false;
    };
    *FILESYSTEM.lock() = Some(volume);
    true
}

pub fn initialize_network(capability: u64) -> bool {
    let mut network = NETWORK.lock();
    if network.is_some() {
        return false;
    }
    *network = Some(SocketApi::new(SyscallDevice::new(capability)));
    true
}

pub fn resolve_ipv4(name: &str) -> Result<Ipv4Address, RuntimeError> {
    NETWORK
        .lock()
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .resolve_ipv4(name)
        .map_err(RuntimeError::Network)
}

pub fn default_gateway() -> Result<Ipv4Address, RuntimeError> {
    NETWORK
        .lock()
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
    NETWORK
        .lock()
        .as_mut()
        .ok_or(RuntimeError::NotInitialized)?
        .http_get(target, target_port, path, expected_body, response)
        .map_err(RuntimeError::Network)
}

pub fn open(name: &[u8], create: bool, truncate: bool) -> Result<i32, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let handle = match volume.open(name) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) if create => {
            volume.create(name).map_err(RuntimeError::Storage)?
        }
        Err(error) => return Err(RuntimeError::Storage(error)),
    };
    if truncate {
        volume.write(handle, &[]).map_err(RuntimeError::Storage)?;
    }
    drop(filesystem);

    let mut descriptors = FILE_DESCRIPTORS.lock();
    let Some((index, slot)) = descriptors
        .iter_mut()
        .enumerate()
        .skip(3)
        .find(|(_, slot)| slot.is_none())
    else {
        return Err(RuntimeError::Storage(StorageError::Capacity));
    };
    *slot = Some(FdEntry::File { handle, offset: 0 });
    Ok(index as i32)
}

pub fn remove(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.remove(name).map_err(RuntimeError::Storage)
}

pub fn mkdir(name: &[u8]) -> Result<(), RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.mkdir(name).map_err(RuntimeError::Storage)
}

pub fn list_root(
    entries: &mut [DirectoryEntry; MAX_DIRECTORY_ENTRIES],
) -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    volume.list_root(entries).map_err(RuntimeError::Storage)
}

pub fn socket() -> Result<i32, RuntimeError> {
    if NETWORK.lock().is_none() {
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
        peer: None,
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
    if !matches!(source, FdEntry::File { .. }) {
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
            _ => Ok(0),
        },
        F_SETFL => {
            let nonblocking = argument & O_NONBLOCK != 0;
            match entry {
                FdEntry::PipeRead {
                    nonblocking: current,
                    ..
                }
                | FdEntry::PipeWrite {
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
    {
        let mut network = NETWORK.lock();
        let network = network.as_mut().ok_or(RuntimeError::NotInitialized)?;
        network
            .tcp_connect(address, port)
            .map_err(RuntimeError::Network)?;
        if let Err(error) = network
            .tcp_set_nagle(nagle_enabled)
            .and_then(|()| network.tcp_set_timeout(timeout))
        {
            let _ = network.tcp_close();
            return Err(RuntimeError::Network(error));
        }
    }
    let mut descriptors = FILE_DESCRIPTORS.lock();
    match descriptors.get_mut(fd as usize).and_then(Option::as_mut) {
        Some(FdEntry::Socket {
            connected, peer, ..
        }) => {
            *connected = true;
            *peer = Some((address, port));
            Ok(())
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
            ..
        }
    ) {
        NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_close()
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
    let FdEntry::Socket { connected, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    if !connected {
        return Err(RuntimeError::NotConnected);
    }

    if how == 1 || how == 2 {
        NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_shutdown_write()
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
    let FdEntry::Socket { connected, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    if connected {
        NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_set_nagle(!enabled)
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
    let FdEntry::Socket { connected, .. } = entry else {
        return Err(RuntimeError::InvalidFd);
    };
    if connected {
        NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_set_timeout(timeout)
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
        FdEntry::File { handle, offset } => {
            let count = read_at(fd, offset, destination)?;
            update_offset(fd, handle, offset + count)?;
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
            ..
        } => NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_receive(destination)
            .map_err(RuntimeError::Network),
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
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let mut file = [0; BLOCK_SIZE];
    let length = volume
        .read(handle, &mut file)
        .map_err(RuntimeError::Storage)?;
    if offset >= length {
        return Ok(0);
    }
    let count = core::cmp::min(destination.len(), length - offset);
    destination[..count].copy_from_slice(&file[offset..offset + count]);
    Ok(count)
}

pub fn readiness(fd: i32, requested: i16) -> Result<i16, RuntimeError> {
    if fd == 1 || fd == 2 {
        return Ok(requested & 0x0004);
    }
    match descriptor(fd)? {
        FdEntry::File { handle, offset } => {
            let mut ready = requested & 0x0004;
            if requested & 0x0001 != 0 && offset < file_size(handle)? {
                ready |= 0x0001;
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
            ..
        } => NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_ready(requested & !POLLOUT)
            .map(|ready| ready | (requested & POLLOUT))
            .map_err(RuntimeError::Network),
        FdEntry::Socket {
            connected: true, ..
        } => NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_ready(requested)
            .map_err(RuntimeError::Network),
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
        FdEntry::File { handle, offset } => {
            if offset != 0 || bytes.len() > BLOCK_SIZE {
                return Err(RuntimeError::Storage(StorageError::FileTooLarge));
            }
            let mut filesystem = FILESYSTEM.lock();
            let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
            volume.write(handle, bytes).map_err(RuntimeError::Storage)?;
            drop(filesystem);
            update_offset(fd, handle, bytes.len())?;
            Ok(bytes.len())
        }
        FdEntry::Socket {
            connected: true,
            write_shutdown: true,
            ..
        } => Err(RuntimeError::Shutdown),
        FdEntry::Socket {
            connected: true,
            write_shutdown: false,
            ..
        } => NETWORK
            .lock()
            .as_mut()
            .ok_or(RuntimeError::NotInitialized)?
            .tcp_send(bytes)
            .map_err(RuntimeError::Network),
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
    let FdEntry::File {
        handle,
        offset: current,
    } = descriptor(fd)?
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
    let FdEntry::File { handle, .. } = descriptor(fd)? else {
        return Err(RuntimeError::InvalidFd);
    };
    file_size(handle)
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

fn file_size(handle: FileHandle) -> Result<usize, RuntimeError> {
    let mut filesystem = FILESYSTEM.lock();
    let volume = filesystem.as_mut().ok_or(RuntimeError::NotInitialized)?;
    let mut file = [0; BLOCK_SIZE];
    volume
        .read(handle, &mut file)
        .map_err(RuntimeError::Storage)
}

pub fn map_error(error: RuntimeError) -> i32 {
    match error {
        RuntimeError::NotInitialized => 38,
        RuntimeError::InvalidFd => 9,
        RuntimeError::Storage(StorageError::NotFound) => 2,
        RuntimeError::Storage(StorageError::AlreadyExists) => 17,
        RuntimeError::Storage(StorageError::NameTooLong) => 36,
        RuntimeError::Storage(StorageError::InvalidName) => 22,
        RuntimeError::Storage(StorageError::DirectoryFull) => 39,
        RuntimeError::Storage(StorageError::FileTooLarge) => 27,
        RuntimeError::Storage(StorageError::Capacity) => 12,
        RuntimeError::Network(NetError::TcpTimeout) => 11,
        RuntimeError::Network(NetError::DnsTimeout) => 11,
        RuntimeError::Network(NetError::ConnectionReset) => 104,
        RuntimeError::Network(NetError::Unsupported) => 95,
        RuntimeError::NotConnected => 107,
        RuntimeError::Shutdown => 108,
        RuntimeError::WouldBlock => 11,
        RuntimeError::BrokenPipe => 32,
        RuntimeError::Unsupported => 95,
        RuntimeError::Network(_) => 5,
        RuntimeError::Storage(_) => 5,
    }
}
