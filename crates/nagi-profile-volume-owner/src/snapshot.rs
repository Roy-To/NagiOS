//! Development-only snapshot provider for an already-selected private root FD.
//!
//! The record and lock-file names here are local candidate details, not a
//! product storage format. Callers remain responsible for authorization and
//! passing the exact selected profile-root FD.

use nagi_model::ObjectId;
use nagi_persistence_contract::{ConditionalWriteOutcome, Revision, Snapshot, SnapshotStorage};
use nagi_service_contract::MAX_SERVICE_PAYLOAD_BYTES;
#[cfg(not(target_os = "linux"))]
use rustix::fs::openat;
use rustix::fs::{fchmod, flock, fsync, renameat, unlinkat, AtFlags, FlockOperation, Mode, OFlags};
#[cfg(target_os = "linux")]
use rustix::fs::{openat2, statx, FileType, ResolveFlags, StatxFlags};
use rustix::io::{fcntl_getfd, fcntl_setfd, Errno, FdFlags};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};

const LOCK_FILE: &str = ".nagi-data-snapshot-store.lock";
const TAINT_FILE: &str = ".nagi-data-snapshot-store.tainted";
const PRIVATE_FILE_MODE: Mode = Mode::RUSR.union(Mode::WUSR);
const FILE_MAGIC: &[u8; 8] = b"NAGSNP01";
const HEADER_LEN: u64 = 24;
const TEMP_OPEN_ATTEMPTS: usize = 16;

/// Candidate limit follows the existing service request/response payload cap.
/// It is not a new limit in the shared `SnapshotStorage` contract.
pub const MAX_SNAPSHOT_BYTES: usize = MAX_SERVICE_PAYLOAD_BYTES;

static NEXT_SNAPSHOT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStorageError {
    InvalidRoot,
    InvalidSnapshot,
    SnapshotTooLarge,
    Unavailable,
    RevisionExhausted,
}

/// FD-relative snapshot provider using a provider-specific local record format.
///
/// Construction accepts only an already-selected private directory FD. It does
/// not perform Identity authorization or choose a User/Profile root. The FD
/// must be an ordinary fsync-capable directory descriptor; Linux `O_PATH` FDs
/// are rejected before the provider creates its lock file. The retained root
/// FD is marked close-on-exec at the provider entry. The root must remain
/// private to the trusted storage owner. Linux validates the mount ID;
/// host fixtures use the device/inode identity and no-follow opens.
pub struct DirectoryFdSnapshotStorage {
    root: File,
    root_identity: RootIdentity,
    lock_file: File,
    lock_identity: EntryIdentity,
    fenced: bool,
    #[cfg(test)]
    fail_next_publish_before_rename: bool,
    #[cfg(test)]
    fail_next_directory_sync_after_publish: bool,
}

impl DirectoryFdSnapshotStorage {
    pub fn new(root: File) -> Result<Self, SnapshotStorageError> {
        ensure_close_on_exec(&root)?;
        let root_identity = capture_root_identity(&root)?;
        reject_path_only_root(&root)?;
        let _directory_lock = lock_root(&root, &root_identity, FlockOperation::LockExclusive)?;
        require_untainted(&root)?;
        let lock_file = open_or_create_lock(&root)?;
        let lock_identity = validate_private_file(&root_identity, &lock_file)?;
        require_root_identity(&root, &root_identity)?;
        fsync(&root).map_err(|_| SnapshotStorageError::Unavailable)?;
        require_root_identity(&root, &root_identity)?;
        Ok(Self {
            root,
            root_identity,
            lock_file,
            lock_identity,
            fenced: false,
            #[cfg(test)]
            fail_next_publish_before_rename: false,
            #[cfg(test)]
            fail_next_directory_sync_after_publish: false,
        })
    }

    #[cfg(test)]
    fn inject_next_directory_sync_failure_after_publish(&mut self) {
        self.fail_next_directory_sync_after_publish = true;
    }

    #[cfg(test)]
    fn inject_next_publish_failure_before_rename(&mut self) {
        self.fail_next_publish_before_rename = true;
    }
}

impl SnapshotStorage for DirectoryFdSnapshotStorage {
    type Error = SnapshotStorageError;

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error> {
        let _directory_lock =
            lock_root(&self.root, &self.root_identity, FlockOperation::LockShared)?;
        self.validate_namespace()?;
        read_snapshot(&self.root, &self.root_identity, key)
    }

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected_revision: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<Self::Error> {
        if self.fenced {
            return ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::Unavailable,
            };
        }
        let _directory_lock = match lock_root(
            &self.root,
            &self.root_identity,
            FlockOperation::LockExclusive,
        ) {
            Ok(lock) => lock,
            Err(error) => return ConditionalWriteOutcome::NotApplied { error },
        };
        if let Err(error) = self.validate_namespace() {
            return ConditionalWriteOutcome::NotApplied { error };
        }
        let _lock = match ExclusiveLock::acquire(&self.lock_file) {
            Ok(lock) => lock,
            Err(error) => return ConditionalWriteOutcome::NotApplied { error },
        };
        if let Err(error) = self.validate_namespace() {
            return ConditionalWriteOutcome::NotApplied { error };
        }

        let current = match read_snapshot(&self.root, &self.root_identity, key) {
            Ok(current) => current,
            Err(error) => return ConditionalWriteOutcome::NotApplied { error },
        };
        let current_revision = current.as_ref().map(|snapshot| snapshot.revision);
        if current_revision != expected_revision {
            return ConditionalWriteOutcome::Conflict { current_revision };
        }

        let new_revision = match next_revision(expected_revision) {
            Ok(revision) => revision,
            Err(error) => return ConditionalWriteOutcome::NotApplied { error },
        };
        #[cfg(test)]
        let write_result = {
            let fail_before_publish = std::mem::take(&mut self.fail_next_publish_before_rename);
            write_snapshot(
                &self.root,
                &self.root_identity,
                key,
                new_revision,
                bytes,
                || {
                    self.validate_namespace()?;
                    if fail_before_publish {
                        Err(SnapshotStorageError::Unavailable)
                    } else {
                        Ok(())
                    }
                },
            )
        };
        #[cfg(not(test))]
        let write_result = write_snapshot(
            &self.root,
            &self.root_identity,
            key,
            new_revision,
            bytes,
            || self.validate_namespace(),
        );
        match write_result {
            Ok(()) => {
                #[cfg(test)]
                let sync_failed = std::mem::take(&mut self.fail_next_directory_sync_after_publish)
                    || fsync(&self.root).is_err();
                #[cfg(not(test))]
                let sync_failed = fsync(&self.root).is_err();
                if sync_failed
                    || require_root_identity(&self.root, &self.root_identity).is_err()
                    || self.validate_namespace().is_err()
                {
                    // rename may already be visible; do not report a definite
                    // failure or permit another write through this instance.
                    self.fenced = true;
                    return ConditionalWriteOutcome::OutcomeUnknown {
                        error: SnapshotStorageError::Unavailable,
                    };
                }
                ConditionalWriteOutcome::Applied {
                    revision: new_revision,
                }
            }
            Err(SnapshotWriteFailure::NotApplied(error)) => {
                ConditionalWriteOutcome::NotApplied { error }
            }
            Err(SnapshotWriteFailure::OutcomeUnknown(error)) => {
                self.fenced = true;
                ConditionalWriteOutcome::OutcomeUnknown { error }
            }
        }
    }
}

impl DirectoryFdSnapshotStorage {
    fn validate_namespace(&self) -> Result<(), SnapshotStorageError> {
        require_root_identity(&self.root, &self.root_identity)?;
        require_untainted(&self.root)?;
        let lock = File::from(
            open_beneath(
                &self.root,
                LOCK_FILE,
                OFlags::RDWR | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| SnapshotStorageError::InvalidRoot)?,
        );
        let observed = validate_private_file(&self.root_identity, &lock)?;
        if observed != self.lock_identity {
            return Err(SnapshotStorageError::InvalidRoot);
        }
        Ok(())
    }
}

fn open_or_create_lock(root: &File) -> Result<File, SnapshotStorageError> {
    let flags = OFlags::RDWR | OFlags::NONBLOCK;
    match open_beneath(
        root,
        LOCK_FILE,
        flags | OFlags::CREATE | OFlags::EXCL,
        PRIVATE_FILE_MODE,
    ) {
        Ok(fd) => {
            let file = File::from(fd);
            if fchmod(&file, PRIVATE_FILE_MODE)
                .and_then(|()| fsync(&file))
                .is_err()
            {
                let _ = unlinkat(root, LOCK_FILE, AtFlags::empty());
                return Err(SnapshotStorageError::Unavailable);
            }
            Ok(file)
        }
        Err(Errno::EXIST) => open_beneath(root, LOCK_FILE, flags, Mode::empty())
            .map(File::from)
            .map_err(|_| SnapshotStorageError::Unavailable),
        Err(_) => Err(SnapshotStorageError::Unavailable),
    }
}

fn require_untainted(root: &File) -> Result<(), SnapshotStorageError> {
    match open_beneath(
        root,
        TAINT_FILE,
        OFlags::RDONLY | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Err(Errno::NOENT) => Ok(()),
        _ => Err(SnapshotStorageError::InvalidRoot),
    }
}

fn record_publish_taint(root: &File) -> Result<(), SnapshotStorageError> {
    // Existence alone fences the root. Keep the final entry intact for
    // diagnosis and sync the marker before releasing the directory lock.
    let marker = open_beneath(
        root,
        TAINT_FILE,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        Mode::empty(),
    )
    .map_err(|_| SnapshotStorageError::Unavailable)?;
    fsync(&marker).map_err(|_| SnapshotStorageError::Unavailable)?;
    fsync(root).map_err(|_| SnapshotStorageError::Unavailable)
}

/// A fixed-object view of an existing snapshot provider.
///
/// The key is supplied by trusted composition and cannot be changed through
/// this view. This limits object selection only; it does not authenticate a
/// caller, select a User/Profile root, or provide a C1 Session lease. Keep the
/// underlying provider private to the composing owner.
pub struct SingleObjectSnapshotStorage<S> {
    inner: S,
    object_id: ObjectId,
}

impl<S> SingleObjectSnapshotStorage<S> {
    pub fn new(inner: S, object_id: ObjectId) -> Self {
        Self { inner, object_id }
    }
}

/// Run an operation through a fixed-object view while borrowing an existing
/// typed provider. Invoke this inside the caller's already-authorized,
/// selected-profile live-lease callback. The provider must belong to that same
/// selected profile; this helper does not authorize, select a root, or expose
/// its FD. The view cannot escape the synchronous operation callback.
pub fn with_single_object_snapshot_storage<S: SnapshotStorage, R>(
    provider: &mut S,
    object_id: ObjectId,
    operation: impl FnOnce(
        &mut dyn SnapshotStorage<Error = SingleObjectSnapshotStorageError<S::Error>>,
    ) -> R,
) -> R {
    let borrowed = BorrowedSnapshotStorage { inner: provider };
    let mut view = SingleObjectSnapshotStorage::new(borrowed, object_id);
    operation(&mut view)
}

struct BorrowedSnapshotStorage<'a, S> {
    inner: &'a mut S,
}

impl<S: SnapshotStorage> SnapshotStorage for BorrowedSnapshotStorage<'_, S> {
    type Error = S::Error;

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error> {
        self.inner.read(key)
    }

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected_revision: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<Self::Error> {
        self.inner
            .compare_and_replace(key, expected_revision, bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SingleObjectSnapshotStorageError<E> {
    ObjectIdMismatch,
    Backend(E),
}

impl<S: SnapshotStorage> SnapshotStorage for SingleObjectSnapshotStorage<S> {
    type Error = SingleObjectSnapshotStorageError<S::Error>;

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error> {
        if key != self.object_id {
            return Err(SingleObjectSnapshotStorageError::ObjectIdMismatch);
        }
        self.inner
            .read(self.object_id)
            .map_err(SingleObjectSnapshotStorageError::Backend)
    }

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected_revision: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<Self::Error> {
        if key != self.object_id {
            return ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::ObjectIdMismatch,
            };
        }

        match self
            .inner
            .compare_and_replace(self.object_id, expected_revision, bytes)
        {
            ConditionalWriteOutcome::Applied { revision } => {
                ConditionalWriteOutcome::Applied { revision }
            }
            ConditionalWriteOutcome::Conflict { current_revision } => {
                ConditionalWriteOutcome::Conflict { current_revision }
            }
            ConditionalWriteOutcome::NotApplied { error } => ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::Backend(error),
            },
            ConditionalWriteOutcome::OutcomeUnknown { error } => {
                ConditionalWriteOutcome::OutcomeUnknown {
                    error: SingleObjectSnapshotStorageError::Backend(error),
                }
            }
        }
    }
}

fn next_revision(expected: Option<Revision>) -> Result<Revision, SnapshotStorageError> {
    let next = match expected {
        None => 1,
        Some(revision) => revision
            .get()
            .checked_add(1)
            .ok_or(SnapshotStorageError::RevisionExhausted)?,
    };
    Revision::new(next).ok_or(SnapshotStorageError::RevisionExhausted)
}

fn read_snapshot(
    root: &File,
    root_identity: &RootIdentity,
    key: ObjectId,
) -> Result<Option<Snapshot>, SnapshotStorageError> {
    require_root_identity(root, root_identity)?;
    let name = snapshot_name(key);
    let fd = match open_beneath(
        root,
        name.as_str(),
        OFlags::RDONLY | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(_) => return Err(SnapshotStorageError::Unavailable),
    };
    let file = File::from(fd);
    validate_private_file(root_identity, &file)?;
    let total_len = file
        .metadata()
        .map_err(|_| SnapshotStorageError::Unavailable)?
        .len();
    if total_len < HEADER_LEN {
        return Err(SnapshotStorageError::InvalidSnapshot);
    }
    if total_len > HEADER_LEN + MAX_SNAPSHOT_BYTES as u64 {
        return Err(SnapshotStorageError::SnapshotTooLarge);
    }

    let mut header = [0u8; HEADER_LEN as usize];
    let mut reader = file.take(total_len);
    reader
        .read_exact(&mut header)
        .map_err(|_| SnapshotStorageError::InvalidSnapshot)?;
    if &header[..FILE_MAGIC.len()] != FILE_MAGIC {
        return Err(SnapshotStorageError::InvalidSnapshot);
    }
    let revision_raw = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| SnapshotStorageError::InvalidSnapshot)?,
    );
    let revision = Revision::new(revision_raw).ok_or(SnapshotStorageError::InvalidSnapshot)?;
    let payload_len = u64::from_le_bytes(
        header[16..24]
            .try_into()
            .map_err(|_| SnapshotStorageError::InvalidSnapshot)?,
    );
    if payload_len > MAX_SNAPSHOT_BYTES as u64 {
        return Err(SnapshotStorageError::SnapshotTooLarge);
    }
    if payload_len.checked_add(HEADER_LEN) != Some(total_len) {
        return Err(SnapshotStorageError::InvalidSnapshot);
    }
    let payload_len_usize =
        usize::try_from(payload_len).map_err(|_| SnapshotStorageError::Unavailable)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(payload_len_usize)
        .map_err(|_| SnapshotStorageError::Unavailable)?;
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| SnapshotStorageError::Unavailable)?;
    if bytes.len() != payload_len_usize {
        return Err(SnapshotStorageError::InvalidSnapshot);
    }
    Ok(Some(Snapshot::new(revision, bytes)))
}

enum SnapshotWriteFailure {
    NotApplied(SnapshotStorageError),
    OutcomeUnknown(SnapshotStorageError),
}

impl From<SnapshotStorageError> for SnapshotWriteFailure {
    fn from(error: SnapshotStorageError) -> Self {
        Self::NotApplied(error)
    }
}

fn write_snapshot(
    root: &File,
    root_identity: &RootIdentity,
    key: ObjectId,
    revision: Revision,
    bytes: &[u8],
    before_publish: impl FnOnce() -> Result<(), SnapshotStorageError>,
) -> Result<(), SnapshotWriteFailure> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotStorageError::SnapshotTooLarge.into());
    }
    require_root_identity(root, root_identity)?;
    let payload_len = u64::try_from(bytes.len()).map_err(|_| SnapshotStorageError::Unavailable)?;
    let (temporary_name, mut temporary) = create_temporary_file(root, root_identity)?;
    let cleanup = |root: &File, name: &str| {
        let _ = unlinkat(root, name, AtFlags::empty());
    };
    let temporary_identity = match validate_private_file(root_identity, &temporary) {
        Ok(identity) => identity,
        Err(error) => {
            cleanup(root, &temporary_name);
            return Err(error.into());
        }
    };
    let write_result = (|| {
        let metadata = temporary
            .metadata()
            .map_err(|_| SnapshotStorageError::Unavailable)?;
        if !metadata.is_file()
            || metadata.uid() != root_identity.owner_uid
            || metadata.mode() & 0o077 != 0
            || metadata.mode() & 0o200 == 0
        {
            return Err(SnapshotStorageError::InvalidRoot);
        }
        temporary
            .write_all(FILE_MAGIC)
            .and_then(|_| temporary.write_all(&revision.get().to_le_bytes()))
            .and_then(|_| temporary.write_all(&payload_len.to_le_bytes()))
            .and_then(|_| temporary.write_all(bytes))
            .map_err(|_| SnapshotStorageError::Unavailable)?;
        temporary
            .sync_all()
            .map_err(|_| SnapshotStorageError::Unavailable)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        cleanup(root, &temporary_name);
        return Err(error.into());
    }

    if let Err(error) = before_publish() {
        cleanup(root, &temporary_name);
        return Err(error.into());
    }

    let final_name = snapshot_name(key);
    if let Err(error) = require_root_identity(root, root_identity) {
        cleanup(root, &temporary_name);
        return Err(error.into());
    }
    if let Err(error) = validate_publish_target(root, root_identity, &final_name) {
        cleanup(root, &temporary_name);
        return Err(error.into());
    }
    // Keep the original descriptor alive so its inode cannot be reused.
    // The named source must still identify that descriptor before publication.
    verify_named_identity(root, root_identity, &temporary_name, temporary_identity)?;
    if renameat(root, temporary_name.as_str(), root, final_name.as_str()).is_err() {
        cleanup(root, &temporary_name);
        return Err(SnapshotWriteFailure::OutcomeUnknown(
            SnapshotStorageError::Unavailable,
        ));
    }
    verify_published_identity(root, root_identity, &final_name, temporary_identity)
}

fn verify_named_identity(
    root: &File,
    root_identity: &RootIdentity,
    name: &str,
    expected: EntryIdentity,
) -> Result<(), SnapshotStorageError> {
    let file = open_beneath(root, name, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty())
        .map(File::from)
        .map_err(|_| SnapshotStorageError::InvalidRoot)?;
    if validate_private_file(root_identity, &file)? != expected {
        return Err(SnapshotStorageError::InvalidRoot);
    }
    Ok(())
}

fn verify_published_identity(
    root: &File,
    root_identity: &RootIdentity,
    name: &str,
    expected: EntryIdentity,
) -> Result<(), SnapshotWriteFailure> {
    let verification = verify_named_identity(root, root_identity, name, expected)
        .and_then(|()| require_root_identity(root, root_identity));
    if let Err(error) = verification {
        return Err(SnapshotWriteFailure::OutcomeUnknown(
            record_publish_taint(root).err().unwrap_or(error),
        ));
    }
    Ok(())
}

fn create_temporary_file(
    root: &File,
    root_identity: &RootIdentity,
) -> Result<(String, File), SnapshotStorageError> {
    require_root_identity(root, root_identity)?;
    for _ in 0..TEMP_OPEN_ATTEMPTS {
        let sequence = NEXT_SNAPSHOT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            ".nagi-snapshot-{:08x}-{sequence:016x}.tmp",
            std::process::id()
        );
        match open_beneath(
            root,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NONBLOCK,
            PRIVATE_FILE_MODE,
        ) {
            Ok(fd) => {
                let file = File::from(fd);
                if fchmod(&file, PRIVATE_FILE_MODE).is_err() {
                    let _ = unlinkat(root, name.as_str(), AtFlags::empty());
                    return Err(SnapshotStorageError::Unavailable);
                }
                if let Err(error) = validate_private_file(root_identity, &file) {
                    let _ = unlinkat(root, name.as_str(), AtFlags::empty());
                    return Err(error);
                }
                if let Err(error) = require_root_identity(root, root_identity) {
                    let _ = unlinkat(root, name.as_str(), AtFlags::empty());
                    return Err(error);
                }
                return Ok((name, file));
            }
            Err(Errno::EXIST) => continue,
            Err(_) => return Err(SnapshotStorageError::Unavailable),
        }
    }
    Err(SnapshotStorageError::Unavailable)
}

fn validate_publish_target(
    root: &File,
    root_identity: &RootIdentity,
    name: &str,
) -> Result<(), SnapshotStorageError> {
    match open_beneath(root, name, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty()) {
        Ok(fd) => {
            validate_private_file(root_identity, &File::from(fd))?;
            Ok(())
        }
        Err(Errno::NOENT) => Ok(()),
        Err(_) => Err(SnapshotStorageError::InvalidRoot),
    }
}

fn snapshot_name(key: ObjectId) -> String {
    format!("snapshot-{:016x}", key.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootIdentity {
    device: u64,
    inode: u64,
    owner_uid: u32,
    #[cfg(target_os = "linux")]
    mount_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EntryIdentity {
    device: u64,
    inode: u64,
}

fn capture_root_identity(root: &File) -> Result<RootIdentity, SnapshotStorageError> {
    let metadata = root
        .metadata()
        .map_err(|_| SnapshotStorageError::InvalidRoot)?;
    if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
        return Err(SnapshotStorageError::InvalidRoot);
    }
    #[cfg(target_os = "linux")]
    let mount_id = {
        let requested = StatxFlags::TYPE | StatxFlags::MNT_ID;
        let stat = statx(root, c"", AtFlags::EMPTY_PATH, requested)
            .map_err(|_| SnapshotStorageError::InvalidRoot)?;
        if stat.stx_mask & requested.bits() != requested.bits()
            || FileType::from_raw_mode(stat.stx_mode.into()) != FileType::Directory
            || stat.stx_ino != metadata.ino()
        {
            return Err(SnapshotStorageError::InvalidRoot);
        }
        stat.stx_mnt_id
    };
    Ok(RootIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner_uid: metadata.uid(),
        #[cfg(target_os = "linux")]
        mount_id,
    })
}

fn require_root_identity(root: &File, expected: &RootIdentity) -> Result<(), SnapshotStorageError> {
    if capture_root_identity(root)? == *expected {
        Ok(())
    } else {
        Err(SnapshotStorageError::InvalidRoot)
    }
}

fn open_beneath(
    root: &impl AsFd,
    name: &str,
    flags: OFlags,
    mode: Mode,
) -> rustix::io::Result<OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        openat2(
            root,
            name,
            flags | OFlags::CLOEXEC,
            mode,
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        openat(root, name, flags | OFlags::CLOEXEC | OFlags::NOFOLLOW, mode)
    }
}

fn ensure_close_on_exec(root: &File) -> Result<(), SnapshotStorageError> {
    let flags = fcntl_getfd(root).map_err(|_| SnapshotStorageError::InvalidRoot)?;
    if !flags.contains(FdFlags::CLOEXEC) {
        fcntl_setfd(root, flags | FdFlags::CLOEXEC)
            .map_err(|_| SnapshotStorageError::InvalidRoot)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_path_only_root(root: &File) -> Result<(), SnapshotStorageError> {
    let flags = rustix::fs::fcntl_getfl(root).map_err(|_| SnapshotStorageError::Unavailable)?;
    if flags.contains(OFlags::PATH) {
        return Err(SnapshotStorageError::InvalidRoot);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn reject_path_only_root(_root: &File) -> Result<(), SnapshotStorageError> {
    Ok(())
}

fn validate_private_file(
    root_identity: &RootIdentity,
    file: &File,
) -> Result<EntryIdentity, SnapshotStorageError> {
    let metadata = file
        .metadata()
        .map_err(|_| SnapshotStorageError::Unavailable)?;
    if !metadata.is_file()
        || metadata.dev() != root_identity.device
        || metadata.uid() != root_identity.owner_uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(SnapshotStorageError::InvalidRoot);
    }
    #[cfg(target_os = "linux")]
    {
        let requested = StatxFlags::TYPE | StatxFlags::MNT_ID;
        let stat = statx(file, c"", AtFlags::EMPTY_PATH, requested)
            .map_err(|_| SnapshotStorageError::InvalidRoot)?;
        if stat.stx_mask & requested.bits() != requested.bits()
            || FileType::from_raw_mode(stat.stx_mode.into()) != FileType::RegularFile
            || stat.stx_mnt_id != root_identity.mount_id
            || stat.stx_ino != metadata.ino()
        {
            return Err(SnapshotStorageError::InvalidRoot);
        }
    }
    Ok(EntryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

// A separate open description is needed even if the caller supplied a
// duplicate of another provider's root FD. flock locks open descriptions.
fn lock_root(
    root: &File,
    root_identity: &RootIdentity,
    operation: FlockOperation,
) -> Result<File, SnapshotStorageError> {
    let lock = open_beneath(root, ".", OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())
        .map(File::from)
        .map_err(|_| SnapshotStorageError::InvalidRoot)?;
    require_root_identity(&lock, root_identity)?;
    flock(&lock, operation).map_err(|_| SnapshotStorageError::Unavailable)?;
    Ok(lock)
}

struct ExclusiveLock<'a>(&'a File);

impl<'a> ExclusiveLock<'a> {
    fn acquire(file: &'a File) -> Result<Self, SnapshotStorageError> {
        flock(file, FlockOperation::LockExclusive)
            .map_err(|_| SnapshotStorageError::Unavailable)?;
        Ok(Self(file))
    }
}

impl Drop for ExclusiveLock<'_> {
    fn drop(&mut self) {
        let _ = flock(self.0, FlockOperation::Unlock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{symlink, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    const PROCESS_TEST_MODE_ENV: &str = "NAGI_DATA_SNAPSHOT_PROCESS_TEST_MODE";
    const PROCESS_TEST_ROOT_ENV: &str = "NAGI_DATA_SNAPSHOT_PROCESS_TEST_ROOT";
    const PROCESS_TEST_CASE: &str =
        "snapshot::tests::snapshot_survives_writer_exit_and_reopens_in_new_process";
    const PROFILE_PROCESS_TEST_MODE_ENV: &str = "NAGI_DATA_SNAPSHOT_PROFILE_PROCESS_TEST_MODE";
    const PROFILE_PROCESS_TEST_ROOT_ENV: &str = "NAGI_DATA_SNAPSHOT_PROFILE_PROCESS_TEST_ROOT";
    const PROFILE_PROCESS_TEST_CASE: &str =
        "snapshot::tests::same_object_id_isolated_across_profile_reader_processes";

    struct TestDirectory {
        path: PathBuf,
        root: File,
    }

    impl TestDirectory {
        fn new(mode: u32) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nagi-data-snapshot-storage-20261119-{}-{sequence}",
                std::process::id(),
            ));
            fs::create_dir(&path).expect("create snapshot root");
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .expect("set snapshot root mode");
            let root = File::open(&path).expect("open root FD");
            Self { path, root }
        }

        fn duplicate_root(&self) -> File {
            self.root.try_clone().expect("duplicate root FD")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[derive(Default)]
    struct SnapshotCallCounts {
        reads: Cell<usize>,
        writes: Cell<usize>,
    }

    struct CountingSnapshotStorage<S> {
        inner: S,
        calls: Rc<SnapshotCallCounts>,
    }

    impl<S: SnapshotStorage> SnapshotStorage for CountingSnapshotStorage<S> {
        type Error = S::Error;

        fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error> {
            self.calls.reads.set(self.calls.reads.get() + 1);
            self.inner.read(key)
        }

        fn compare_and_replace(
            &mut self,
            key: ObjectId,
            expected_revision: Option<Revision>,
            bytes: &[u8],
        ) -> ConditionalWriteOutcome<Self::Error> {
            self.calls.writes.set(self.calls.writes.get() + 1);
            self.inner
                .compare_and_replace(key, expected_revision, bytes)
        }
    }

    #[test]
    fn single_object_view_limits_real_directory_provider_to_assigned_key() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only keys; this does not assign a Settings or Calendar ID.
        let assigned_key = ObjectId(0x5343_4f50);
        let other_key = ObjectId(0x4f54_4845);
        let calls = Rc::new(SnapshotCallCounts::default());
        let counted = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        let mut view = SingleObjectSnapshotStorage::new(counted, assigned_key);

        assert_eq!(view.read(assigned_key).unwrap(), None);
        let first_revision = match view.compare_and_replace(assigned_key, None, b"first") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected first save result: {other:?}"),
        };
        assert_eq!(first_revision.get(), 1);
        let second_revision =
            match view.compare_and_replace(assigned_key, Some(first_revision), b"updated") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected update result: {other:?}"),
            };
        assert_eq!(second_revision.get(), 2);

        let calls_before_rejected_key = (calls.reads.get(), calls.writes.get());
        assert_eq!(
            view.read(other_key),
            Err(SingleObjectSnapshotStorageError::ObjectIdMismatch)
        );
        assert_eq!(
            view.compare_and_replace(other_key, None, b"must not be stored"),
            ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::ObjectIdMismatch,
            }
        );
        assert_eq!(
            (calls.reads.get(), calls.writes.get()),
            calls_before_rejected_key,
            "a mismatched key must not reach the backing provider"
        );
        assert_eq!(
            view.read(assigned_key).unwrap(),
            Some(Snapshot::new(second_revision, b"updated".to_vec()))
        );
        drop(view);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let mut reopened_view = SingleObjectSnapshotStorage::new(reopened, assigned_key);
        assert_eq!(
            reopened_view.read(assigned_key).unwrap(),
            Some(Snapshot::new(second_revision, b"updated".to_vec()))
        );
        let third_revision = match reopened_view.compare_and_replace(
            assigned_key,
            Some(second_revision),
            b"updated after reopen",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected reopened view update result: {other:?}"),
        };
        assert_eq!(third_revision.get(), 3);
        drop(reopened_view);

        let final_view = SingleObjectSnapshotStorage::new(
            DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            assigned_key,
        );
        assert_eq!(
            final_view.read(assigned_key).unwrap(),
            Some(Snapshot::new(
                third_revision,
                b"updated after reopen".to_vec()
            ))
        );
    }

    #[test]
    fn successful_single_object_read_does_not_write_another_object() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only keys; this does not assign product ObjectIds.
        let read_key = ObjectId(0x5244_4f4e);
        let other_key = ObjectId(0x5244_4f54);
        let read_snapshot = Snapshot::new(
            Revision::new(1).unwrap(),
            b"read target remains stable".to_vec(),
        );
        let other_snapshot = Snapshot::new(
            Revision::new(1).unwrap(),
            b"neighbor remains stable".to_vec(),
        );

        let mut seed = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            seed.compare_and_replace(read_key, None, &read_snapshot.bytes),
            ConditionalWriteOutcome::Applied {
                revision: read_snapshot.revision
            }
        );
        assert_eq!(
            seed.compare_and_replace(other_key, None, &other_snapshot.bytes),
            ConditionalWriteOutcome::Applied {
                revision: other_snapshot.revision
            }
        );
        drop(seed);

        let calls = Rc::new(SnapshotCallCounts::default());
        let mut provider = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        let observed = with_single_object_snapshot_storage(&mut provider, read_key, |view| {
            view.read(read_key).unwrap()
        });
        assert_eq!(observed, Some(read_snapshot.clone()));
        assert_eq!(calls.reads.get(), 1);
        assert_eq!(
            calls.writes.get(),
            0,
            "a successful one-object read must not issue a provider write"
        );
        drop(provider);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(reopened.read(read_key).unwrap(), Some(read_snapshot));
        assert_eq!(reopened.read(other_key).unwrap(), Some(other_snapshot));
    }

    #[test]
    fn applied_write_confirmation_and_fresh_reopen_add_no_writes() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x434f_4e46);
        let payload = b"durably applied snapshot";
        let calls = Rc::new(SnapshotCallCounts::default());
        let mut writer = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };

        let applied_revision =
            with_single_object_snapshot_storage(&mut writer, assigned_key, |view| {
                match view.compare_and_replace(assigned_key, None, payload) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial save result: {other:?}"),
                }
            });
        assert_eq!(applied_revision.get(), 1);
        assert_eq!(calls.writes.get(), 1);
        assert_eq!(calls.reads.get(), 0);

        let same_provider_confirmation =
            with_single_object_snapshot_storage(&mut writer, assigned_key, |view| {
                view.read(assigned_key).unwrap()
            });
        let expected = Some(Snapshot::new(applied_revision, payload.to_vec()));
        assert_eq!(same_provider_confirmation, expected);
        assert_eq!(calls.reads.get(), 1);
        assert_eq!(calls.writes.get(), 1, "confirmation must not write again");
        drop(writer);

        let reopened_calls = Rc::new(SnapshotCallCounts::default());
        let mut reopened = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&reopened_calls),
        };
        let fresh_provider_confirmation =
            with_single_object_snapshot_storage(&mut reopened, assigned_key, |view| {
                view.read(assigned_key).unwrap()
            });
        assert_eq!(fresh_provider_confirmation, expected);
        assert_eq!(reopened_calls.reads.get(), 1);
        assert_eq!(
            reopened_calls.writes.get(),
            0,
            "fresh-provider confirmation must remain read-only"
        );
    }

    #[test]
    fn repeated_identical_payload_write_advances_revision_after_reopen() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x5341_4d45);
        let payload = b"explicitly saved identical bytes";
        let mut provider = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();

        let first_revision =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                match view.compare_and_replace(assigned_key, None, payload) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial save result: {other:?}"),
                }
            });
        assert_eq!(first_revision.get(), 1);

        // An explicit replacement is a new version even when its bytes match.
        let second_revision =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                match view.compare_and_replace(assigned_key, Some(first_revision), payload) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected repeated save result: {other:?}"),
                }
            });
        assert_eq!(second_revision.get(), 2);
        drop(provider);

        let mut reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let reopened_snapshot =
            with_single_object_snapshot_storage(&mut reopened, assigned_key, |view| {
                view.read(assigned_key).unwrap()
            });
        assert_eq!(
            reopened_snapshot,
            Some(Snapshot::new(second_revision, payload.to_vec()))
        );
    }

    #[test]
    fn reads_two_explicit_objects_without_writes_during_read_only_snapshot() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only IDs; this reads a caller-supplied set and does not
        // define a provider listing or backup/export contract.
        let notes_key = ObjectId(0x4e4f_5445);
        let settings_key = ObjectId(0x5345_5433);
        let mut seed = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let notes_revision = match seed.compare_and_replace(notes_key, None, b"notes-v1") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected notes fixture write: {other:?}"),
        };
        let settings_revision = match seed.compare_and_replace(settings_key, None, b"settings-v1") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected settings fixture write: {other:?}"),
        };
        drop(seed);

        let calls = Rc::new(SnapshotCallCounts::default());
        let reader = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        let observed = [notes_key, settings_key].map(|key| (key, reader.read(key).unwrap()));

        assert_eq!(
            observed,
            [
                (
                    notes_key,
                    Some(Snapshot::new(notes_revision, b"notes-v1".to_vec()))
                ),
                (
                    settings_key,
                    Some(Snapshot::new(settings_revision, b"settings-v1".to_vec()))
                ),
            ]
        );
        assert_eq!(calls.reads.get(), 2);
        assert_eq!(
            calls.writes.get(),
            0,
            "reading the explicit snapshot set must not modify the provider"
        );
    }

    #[test]
    fn borrowed_single_object_view_reopens_initial_save_through_a_second_view() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x4252_4f57);
        let mut provider = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let initial_default = b"explicit initial default";

        let (revision, same_view_snapshot) =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                assert_eq!(view.read(assigned_key).unwrap(), None);
                let revision = match view.compare_and_replace(assigned_key, None, initial_default) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected borrowed-view save result: {other:?}"),
                };
                let reread = view.read(assigned_key).unwrap();
                (revision, reread)
            });

        let expected = Snapshot::new(revision, initial_default.to_vec());
        assert_eq!(revision.get(), 1);
        assert_eq!(same_view_snapshot, Some(expected.clone()));

        let second_view_snapshot =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                view.read(assigned_key).unwrap()
            });
        assert_eq!(second_view_snapshot, Some(expected.clone()));
        assert_eq!(provider.read(assigned_key), Ok(Some(expected)));
    }

    #[test]
    fn borrowed_view_update_matches_same_provider_and_fresh_provider_readback() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x4252_4f58);
        let mut provider = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();

        let revision_one =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                match view.compare_and_replace(assigned_key, None, b"snapshot-v1") {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial borrowed-view write: {other:?}"),
                }
            });
        let (revision_two, same_provider_readback) =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                let revision = match view.compare_and_replace(
                    assigned_key,
                    Some(revision_one),
                    b"snapshot-v2",
                ) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected borrowed-view update: {other:?}"),
                };
                (revision, view.read(assigned_key).unwrap())
            });
        let expected = Some(Snapshot::new(revision_two, b"snapshot-v2".to_vec()));
        assert_eq!(revision_one.get(), 1);
        assert_eq!(revision_two.get(), 2);
        assert_eq!(same_provider_readback, expected);
        assert_eq!(provider.read(assigned_key), Ok(expected.clone()));
        drop(provider);

        let mut fresh_provider =
            DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let fresh_readback =
            with_single_object_snapshot_storage(&mut fresh_provider, assigned_key, |view| {
                view.read(assigned_key).unwrap()
            });
        assert_eq!(fresh_readback, expected);
        assert_eq!(fresh_provider.read(assigned_key), Ok(fresh_readback));
    }

    #[test]
    fn borrowed_view_reopens_flushed_empty_object_distinct_from_missing_object() {
        let profile = TestDirectory::new(0o700);
        // Fixture-only keys; these do not assign product ObjectIds.
        let empty_key = ObjectId(0x454d_5054);
        let missing_key = ObjectId(0x4d49_5353);
        let mut provider = DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();

        let empty_revision =
            with_single_object_snapshot_storage(&mut provider, empty_key, |view| {
                assert_eq!(view.read(empty_key).unwrap(), None);
                match view.compare_and_replace(empty_key, None, b"") {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected empty-object save result: {other:?}"),
                }
            });
        assert_eq!(empty_revision.get(), 1);
        drop(provider);

        let mut reopened_provider =
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();
        let empty_object =
            with_single_object_snapshot_storage(&mut reopened_provider, empty_key, |view| {
                view.read(empty_key).unwrap()
            });
        let missing_object =
            with_single_object_snapshot_storage(&mut reopened_provider, missing_key, |view| {
                view.read(missing_key).unwrap()
            });

        assert_eq!(
            empty_object,
            Some(Snapshot::new(empty_revision, Vec::new()))
        );
        assert_eq!(missing_object, None);

        let next_revision = with_single_object_snapshot_storage(
            &mut reopened_provider,
            empty_key,
            |view| match view.compare_and_replace(
                empty_key,
                Some(empty_revision),
                b"ordinary update after reopening an empty object",
            ) {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected update after empty-object reopen: {other:?}"),
            },
        );
        assert_eq!(next_revision.get(), 2);
        assert_eq!(
            with_single_object_snapshot_storage(&mut reopened_provider, empty_key, |view| {
                view.read(empty_key).unwrap()
            }),
            Some(Snapshot::new(
                next_revision,
                b"ordinary update after reopening an empty object".to_vec()
            ))
        );
        assert_eq!(
            with_single_object_snapshot_storage(&mut reopened_provider, missing_key, |view| {
                view.read(missing_key).unwrap()
            }),
            None
        );
        drop(reopened_provider);

        let mut final_provider = DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();
        assert_eq!(
            with_single_object_snapshot_storage(&mut final_provider, empty_key, |view| {
                view.read(empty_key).unwrap()
            }),
            Some(Snapshot::new(
                next_revision,
                b"ordinary update after reopening an empty object".to_vec()
            ))
        );
        assert_eq!(
            with_single_object_snapshot_storage(&mut final_provider, missing_key, |view| {
                view.read(missing_key).unwrap()
            }),
            None
        );
    }

    #[test]
    fn borrowed_single_object_views_keep_other_object_revision_independent_after_reopen() {
        let profile = TestDirectory::new(0o700);
        // Fixture-only IDs in one private profile root; these are not product assignments.
        let settings_key = ObjectId(0x5345_5432);
        let calendar_key = ObjectId(0x4341_4c32);
        let mut provider = DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();

        let settings_revision_one =
            with_single_object_snapshot_storage(&mut provider, settings_key, |view| {
                match view.compare_and_replace(settings_key, None, b"settings-v1") {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial settings save result: {other:?}"),
                }
            });
        let calendar_revision_one =
            with_single_object_snapshot_storage(&mut provider, calendar_key, |view| {
                match view.compare_and_replace(calendar_key, None, b"calendar-v1") {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial calendar save result: {other:?}"),
                }
            });
        assert_eq!(settings_revision_one.get(), 1);
        assert_eq!(calendar_revision_one.get(), 1);

        let settings_revision_two =
            with_single_object_snapshot_storage(&mut provider, settings_key, |view| {
                match view.compare_and_replace(
                    settings_key,
                    Some(settings_revision_one),
                    b"settings-v2",
                ) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected settings update result: {other:?}"),
                }
            });
        assert_eq!(settings_revision_two.get(), 2);
        drop(provider);

        let mut reopened_provider =
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();
        let settings =
            with_single_object_snapshot_storage(&mut reopened_provider, settings_key, |view| {
                view.read(settings_key).unwrap().unwrap()
            });
        let calendar =
            with_single_object_snapshot_storage(&mut reopened_provider, calendar_key, |view| {
                view.read(calendar_key).unwrap().unwrap()
            });

        // A revision-only consumer sees the same version through the reopened view.
        let revision_only_settings_result = settings.revision;
        assert_eq!(revision_only_settings_result, settings_revision_two);
        assert_eq!(settings.bytes, b"settings-v2");
        let revision_only_calendar_result = calendar.revision;
        assert_eq!(revision_only_calendar_result, calendar_revision_one);
        assert_eq!(calendar.bytes, b"calendar-v1");
    }

    #[test]
    fn separate_single_object_views_recover_independent_latest_revisions_after_publish_failure() {
        let profile = TestDirectory::new(0o700);
        // Fixture-only keys in one private root; these are not product assignments.
        let settings_key = ObjectId(0x5346_4149);
        let calendar_key = ObjectId(0x4346_4149);
        let mut settings = SingleObjectSnapshotStorage::new(
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap(),
            settings_key,
        );
        let mut calendar = SingleObjectSnapshotStorage::new(
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap(),
            calendar_key,
        );

        let settings_revision_one =
            match settings.compare_and_replace(settings_key, None, b"settings-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected initial settings save result: {other:?}"),
            };
        let calendar_revision_one =
            match calendar.compare_and_replace(calendar_key, None, b"calendar-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected initial calendar save result: {other:?}"),
            };
        assert_eq!(settings_revision_one.get(), 1);
        assert_eq!(calendar_revision_one.get(), 1);

        settings.inner.inject_next_publish_failure_before_rename();
        assert_eq!(
            settings.compare_and_replace(
                settings_key,
                Some(settings_revision_one),
                b"settings-not-published",
            ),
            ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::Backend(SnapshotStorageError::Unavailable,),
            }
        );
        let calendar_revision_two = match calendar.compare_and_replace(
            calendar_key,
            Some(calendar_revision_one),
            b"calendar-v2",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected calendar update result: {other:?}"),
        };
        assert_eq!(calendar_revision_two.get(), 2);
        assert_eq!(
            settings.read(settings_key).unwrap(),
            Some(Snapshot::new(
                settings_revision_one,
                b"settings-v1".to_vec()
            ))
        );
        assert_eq!(
            calendar.read(calendar_key).unwrap(),
            Some(Snapshot::new(
                calendar_revision_two,
                b"calendar-v2".to_vec()
            ))
        );
        drop(settings);
        drop(calendar);

        let mut reopened_settings = SingleObjectSnapshotStorage::new(
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap(),
            settings_key,
        );
        let reopened_calendar = SingleObjectSnapshotStorage::new(
            DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap(),
            calendar_key,
        );
        assert_eq!(
            reopened_settings.read(settings_key).unwrap(),
            Some(Snapshot::new(
                settings_revision_one,
                b"settings-v1".to_vec()
            ))
        );
        assert_eq!(
            reopened_calendar.read(calendar_key).unwrap(),
            Some(Snapshot::new(
                calendar_revision_two,
                b"calendar-v2".to_vec()
            ))
        );

        let settings_revision_two = match reopened_settings.compare_and_replace(
            settings_key,
            Some(settings_revision_one),
            b"settings-v2",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected settings retry result: {other:?}"),
        };
        assert_eq!(settings_revision_two.get(), 2);
        drop(reopened_settings);
        drop(reopened_calendar);

        let fresh_reader = DirectoryFdSnapshotStorage::new(profile.duplicate_root()).unwrap();
        assert_eq!(
            fresh_reader.read(settings_key).unwrap(),
            Some(Snapshot::new(
                settings_revision_two,
                b"settings-v2".to_vec()
            ))
        );
        assert_eq!(
            fresh_reader.read(calendar_key).unwrap(),
            Some(Snapshot::new(
                calendar_revision_two,
                b"calendar-v2".to_vec()
            ))
        );
    }

    #[test]
    fn single_object_view_preserves_unknown_publish_outcome_and_read_recovers_state() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a Settings or Calendar ID.
        let assigned_key = ObjectId(0x554e_4b4e);
        let calls = Rc::new(SnapshotCallCounts::default());
        let counted = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        let mut view = SingleObjectSnapshotStorage::new(counted, assigned_key);

        let first_revision = match view.compare_and_replace(assigned_key, None, b"committed") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected initial save result: {other:?}"),
        };
        assert_eq!(first_revision.get(), 1);
        assert_eq!(calls.writes.get(), 1);

        let confirmed_revision =
            match view.compare_and_replace(assigned_key, Some(first_revision), b"confirmed update")
            {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected confirmed update result: {other:?}"),
            };
        assert_eq!(confirmed_revision.get(), 2);
        assert_eq!(calls.writes.get(), 2);

        view.inner.inner.inject_next_publish_failure_before_rename();
        assert_eq!(
            view.compare_and_replace(assigned_key, Some(confirmed_revision), b"not published",),
            ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::Backend(SnapshotStorageError::Unavailable,),
            }
        );
        assert_eq!(calls.writes.get(), 3);
        assert_eq!(
            view.read(assigned_key).unwrap(),
            Some(Snapshot::new(
                confirmed_revision,
                b"confirmed update".to_vec()
            ))
        );

        // Inject the existing backend fault after the candidate has been
        // renamed into place but before the parent-directory sync succeeds.
        view.inner
            .inner
            .inject_next_directory_sync_failure_after_publish();
        let uncertain_outcome = view.compare_and_replace(
            assigned_key,
            Some(confirmed_revision),
            b"published but uncertain",
        );
        assert_eq!(
            uncertain_outcome,
            ConditionalWriteOutcome::OutcomeUnknown {
                error: SingleObjectSnapshotStorageError::Backend(SnapshotStorageError::Unavailable,),
            }
        );
        assert_eq!(calls.writes.get(), 4, "the update is issued exactly once");

        let recovered = view.read(assigned_key).unwrap().unwrap();
        assert_eq!(recovered.revision.get(), 3);
        assert_eq!(recovered.bytes, b"published but uncertain");
        assert_eq!(
            calls.writes.get(),
            4,
            "read reconciliation must not reissue the uncertain update"
        );
    }

    #[test]
    fn borrowed_view_readback_does_not_promote_unknown_publish_outcome() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x4255_4e4b);
        let calls = Rc::new(SnapshotCallCounts::default());
        let mut provider = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        provider
            .inner
            .inject_next_directory_sync_failure_after_publish();

        let (outcome, observed) =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                let outcome = view.compare_and_replace(
                    assigned_key,
                    None,
                    b"visible after uncertain publication",
                );
                let observed = view.read(assigned_key).unwrap();
                (outcome, observed)
            });
        assert_eq!(
            outcome,
            ConditionalWriteOutcome::OutcomeUnknown {
                error: SingleObjectSnapshotStorageError::Backend(SnapshotStorageError::Unavailable,),
            }
        );
        let published = Snapshot::new(
            Revision::new(1).unwrap(),
            b"visible after uncertain publication".to_vec(),
        );
        assert_eq!(observed, Some(published.clone()));
        assert_eq!(calls.writes.get(), 1, "readback must not retry the write");
        assert_eq!(calls.reads.get(), 1);
        drop(provider);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(reopened.read(assigned_key).unwrap(), Some(published));
    }

    #[test]
    fn borrowed_view_preserves_corrupt_snapshot_error_without_overwrite() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let assigned_key = ObjectId(0x4252_45a1);
        let record_path = directory.path.join(snapshot_name(assigned_key));
        let corrupt_bytes = b"damaged snapshot record";
        let mut corrupt_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&record_path)
            .unwrap();
        corrupt_file.write_all(corrupt_bytes).unwrap();
        drop(corrupt_file);

        let original_record = fs::read(&record_path).unwrap();
        let mut provider = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let borrowed_read =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                view.read(assigned_key)
            });
        assert_eq!(
            borrowed_read,
            Err(SingleObjectSnapshotStorageError::Backend(
                SnapshotStorageError::InvalidSnapshot
            ))
        );

        let borrowed_create_only_write =
            with_single_object_snapshot_storage(&mut provider, assigned_key, |view| {
                view.compare_and_replace(assigned_key, None, b"replacement")
            });
        assert_eq!(
            borrowed_create_only_write,
            ConditionalWriteOutcome::NotApplied {
                error: SingleObjectSnapshotStorageError::Backend(
                    SnapshotStorageError::InvalidSnapshot
                )
            }
        );
        assert_eq!(fs::read(&record_path).unwrap(), original_record);
        drop(provider);

        let mut reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let reopened_read =
            with_single_object_snapshot_storage(&mut reopened, assigned_key, |view| {
                view.read(assigned_key)
            });
        assert_eq!(
            reopened_read,
            Err(SingleObjectSnapshotStorageError::Backend(
                SnapshotStorageError::InvalidSnapshot
            ))
        );
        assert_eq!(fs::read(&record_path).unwrap(), original_record);
    }

    #[test]
    fn compare_and_replace_reads_back_after_reopen_and_rejects_stale_revision() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x1234);
        let mut first = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(first.read(key).unwrap(), None);
        let revision_one = match first.compare_and_replace(key, None, b"first") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected first write result: {other:?}"),
        };
        assert_eq!(revision_one.get(), 1);
        assert_eq!(first.read(key).unwrap().unwrap().bytes, b"first");

        let mut reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let revision_two = match reopened.compare_and_replace(key, Some(revision_one), b"second") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected replacement result: {other:?}"),
        };
        assert_eq!(revision_two.get(), 2);
        assert_eq!(
            reopened.compare_and_replace(key, Some(revision_one), b"stale"),
            ConditionalWriteOutcome::Conflict {
                current_revision: Some(revision_two)
            }
        );
        let expected = Some(Snapshot::new(revision_two, b"second".to_vec()));
        assert_eq!(reopened.read(key).unwrap(), expected);
        drop(reopened);
        drop(first);

        let fresh_reader = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(fresh_reader.read(key).unwrap(), expected);
    }

    #[test]
    fn parent_directory_sync_failure_after_publish_is_unknown_and_fences_retries() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x5359_4e43);
        let mut writer = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        writer.inject_next_publish_failure_before_rename();
        assert_eq!(
            writer.compare_and_replace(key, None, b"not published"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::Unavailable,
            }
        );
        assert_eq!(writer.read(key).unwrap(), None);
        assert!(fs::read_dir(&directory.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name.to_string_lossy().ends_with(".tmp")));

        writer.inject_next_directory_sync_failure_after_publish();

        assert_eq!(
            writer.compare_and_replace(key, None, b"published candidate"),
            ConditionalWriteOutcome::OutcomeUnknown {
                error: SnapshotStorageError::Unavailable,
            }
        );

        // Rename completed before the injected parent-directory sync failure.
        // Visibility does not turn the uncertain outcome into a confirmed save.
        let visible = writer
            .read(key)
            .unwrap()
            .expect("published candidate is visible");
        assert_eq!(visible.revision.get(), 1);
        assert_eq!(visible.bytes, b"published candidate");
        assert_eq!(
            writer.compare_and_replace(key, None, b"duplicate retry"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::Unavailable,
            }
        );
        assert_eq!(writer.read(key).unwrap(), Some(visible.clone()));

        drop(writer);
        let mut reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened.compare_and_replace(key, None, b"duplicate retry"),
            ConditionalWriteOutcome::Conflict {
                current_revision: Some(visible.revision),
            }
        );
        assert_eq!(reopened.read(key).unwrap(), Some(visible));
    }

    #[test]
    fn pre_publish_update_failure_preserves_existing_snapshot_after_reopen() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x5052_4550);
        let mut writer = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let revision = match writer.compare_and_replace(key, None, b"committed before failure") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected initial write result: {other:?}"),
        };

        writer.inject_next_publish_failure_before_rename();
        assert_eq!(
            writer.compare_and_replace(key, Some(revision), b"not published"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::Unavailable
            }
        );
        assert!(fs::read_dir(&directory.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name.to_string_lossy().ends_with(".tmp")));
        drop(writer);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened.read(key).unwrap(),
            Some(Snapshot::new(
                revision,
                b"committed before failure".to_vec()
            ))
        );
        assert!(fs::read_dir(&directory.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name.to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn independent_reader_reopens_each_committed_revision_with_matching_body_stamp() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x5245_4144);
        let mut writer = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let mut expected_revision = None;
        let mut final_body = Vec::new();

        for sequence in 1..=8u64 {
            let body = format!("revision={sequence};body=profile-snapshot-{sequence:04x}");
            let revision = match writer.compare_and_replace(key, expected_revision, body.as_bytes())
            {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected stamped snapshot write result: {other:?}"),
            };
            assert_eq!(revision.get(), sequence);

            let reader = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
            let observed = reader.read(key).unwrap().unwrap();
            assert_eq!(observed.revision, revision);
            assert_eq!(observed.bytes, body.as_bytes());

            expected_revision = Some(revision);
            final_body = body.into_bytes();
        }

        drop(writer);
        let final_reader = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let final_snapshot = final_reader.read(key).unwrap().unwrap();
        assert_eq!(final_snapshot.revision, expected_revision.unwrap());
        assert_eq!(final_snapshot.revision.get(), 8);
        assert_eq!(final_snapshot.bytes, final_body);
    }

    #[test]
    fn multiple_consumers_share_only_the_selected_snapshot_root() {
        let selected_profile = TestDirectory::new(0o700);
        let unrelated_profile = TestDirectory::new(0o700);
        let key = ObjectId(0x434f_4e53);
        let mut first_consumer =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();
        let mut second_consumer =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();

        let first_revision = match first_consumer.compare_and_replace(key, None, b"consumer one") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected first consumer write result: {other:?}"),
        };
        let observed_by_second = second_consumer.read(key).unwrap().unwrap();
        assert_eq!(observed_by_second.revision, first_revision);
        assert_eq!(observed_by_second.bytes, b"consumer one");

        let second_revision =
            match second_consumer.compare_and_replace(key, Some(first_revision), b"consumer two") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected second consumer write result: {other:?}"),
            };
        assert_eq!(second_revision.get(), first_revision.get() + 1);
        let observed_by_first = first_consumer.read(key).unwrap().unwrap();
        assert_eq!(observed_by_first.revision, second_revision);
        assert_eq!(observed_by_first.bytes, b"consumer two");

        assert!(selected_profile.path.join(snapshot_name(key)).exists());
        assert!(!unrelated_profile.path.join(snapshot_name(key)).exists());
        assert!(!unrelated_profile.path.join(LOCK_FILE).exists());
    }

    #[test]
    fn different_object_ids_remain_independent_when_consumers_update_shared_root() {
        let selected_profile = TestDirectory::new(0o700);
        let settings_key = ObjectId(0x5345_5454);
        let calendar_key = ObjectId(0x4341_4c45);
        let mut settings_consumer =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();
        let mut calendar_consumer =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();

        let settings_revision_one =
            match settings_consumer.compare_and_replace(settings_key, None, b"settings-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected settings write result: {other:?}"),
            };
        let calendar_revision_one =
            match calendar_consumer.compare_and_replace(calendar_key, None, b"calendar-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected calendar write result: {other:?}"),
            };
        assert_eq!(settings_revision_one.get(), 1);
        assert_eq!(calendar_revision_one.get(), 1);

        let first_reader =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();
        let settings = first_reader.read(settings_key).unwrap().unwrap();
        let calendar = first_reader.read(calendar_key).unwrap().unwrap();
        assert_eq!(settings.revision, settings_revision_one);
        assert_eq!(settings.bytes, b"settings-v1");
        assert_eq!(calendar.revision, calendar_revision_one);
        assert_eq!(calendar.bytes, b"calendar-v1");

        let settings_revision_two = match settings_consumer.compare_and_replace(
            settings_key,
            Some(settings_revision_one),
            b"settings-v2",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected settings update result: {other:?}"),
        };
        assert_eq!(settings_revision_two.get(), 2);

        // Reopen only after every previous provider instance has been dropped.
        drop(settings_consumer);
        drop(calendar_consumer);
        drop(first_reader);

        let final_reader =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();
        let settings = final_reader.read(settings_key).unwrap().unwrap();
        let calendar = final_reader.read(calendar_key).unwrap().unwrap();
        assert_eq!(settings.revision, settings_revision_two);
        assert_eq!(settings.bytes, b"settings-v2");
        assert_eq!(calendar.revision, calendar_revision_one);
        assert_eq!(calendar.bytes, b"calendar-v1");
    }

    #[test]
    fn stale_revision_from_another_object_does_not_modify_either_snapshot_after_reopen() {
        let selected_profile = TestDirectory::new(0o700);
        let settings_key = ObjectId(0x5345_5453);
        let calendar_key = ObjectId(0x4341_4c53);
        let mut writer =
            DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();

        let settings_revision_one =
            match writer.compare_and_replace(settings_key, None, b"settings-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected settings fixture write: {other:?}"),
            };
        let calendar_revision_one =
            match writer.compare_and_replace(calendar_key, None, b"calendar-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected calendar fixture write: {other:?}"),
            };
        let calendar_revision_two = match writer.compare_and_replace(
            calendar_key,
            Some(calendar_revision_one),
            b"calendar-v2",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected calendar fixture update: {other:?}"),
        };
        assert_eq!(settings_revision_one.get(), 1);
        assert_eq!(calendar_revision_two.get(), 2);

        assert_eq!(
            writer.compare_and_replace(
                settings_key,
                Some(calendar_revision_two),
                b"must not replace settings"
            ),
            ConditionalWriteOutcome::Conflict {
                current_revision: Some(settings_revision_one)
            }
        );
        drop(writer);

        let reopened = DirectoryFdSnapshotStorage::new(selected_profile.duplicate_root()).unwrap();
        assert_eq!(
            reopened.read(settings_key).unwrap(),
            Some(Snapshot::new(
                settings_revision_one,
                b"settings-v1".to_vec()
            ))
        );
        assert_eq!(
            reopened.read(calendar_key).unwrap(),
            Some(Snapshot::new(
                calendar_revision_two,
                b"calendar-v2".to_vec()
            ))
        );
    }

    #[test]
    fn same_snapshot_key_is_isolated_between_profile_roots_after_conflict() {
        let first_profile = TestDirectory::new(0o700);
        let second_profile = TestDirectory::new(0o700);
        let key = ObjectId(0x5052_4f46);
        let mut first = DirectoryFdSnapshotStorage::new(first_profile.duplicate_root()).unwrap();
        let mut second = DirectoryFdSnapshotStorage::new(second_profile.duplicate_root()).unwrap();

        let first_revision = match first.compare_and_replace(key, None, b"first profile") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected first profile write result: {other:?}"),
        };
        assert_eq!(first_revision.get(), 1);
        assert_eq!(
            first.compare_and_replace(key, None, b"must not replace first profile"),
            ConditionalWriteOutcome::Conflict {
                current_revision: Some(first_revision)
            }
        );

        let first_snapshot = first.read(key).unwrap().unwrap();
        assert_eq!(first_snapshot.revision, first_revision);
        assert_eq!(first_snapshot.bytes, b"first profile");

        let second_revision = match second.compare_and_replace(key, None, b"second profile") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected second profile write result: {other:?}"),
        };
        assert_eq!(second_revision.get(), 1);
        let second_snapshot = second.read(key).unwrap().unwrap();
        assert_eq!(second_snapshot.revision, second_revision);
        assert_eq!(second_snapshot.bytes, b"second profile");

        let first_snapshot = first.read(key).unwrap().unwrap();
        assert_eq!(first_snapshot.revision, first_revision);
        assert_eq!(first_snapshot.bytes, b"first profile");

        drop(first);
        drop(second);

        let reopened_first =
            DirectoryFdSnapshotStorage::new(first_profile.duplicate_root()).unwrap();
        let reopened_second =
            DirectoryFdSnapshotStorage::new(second_profile.duplicate_root()).unwrap();
        assert_eq!(
            reopened_first.read(key).unwrap(),
            Some(Snapshot::new(first_revision, b"first profile".to_vec()))
        );
        assert_eq!(
            reopened_second.read(key).unwrap(),
            Some(Snapshot::new(second_revision, b"second profile".to_vec()))
        );
    }

    #[test]
    fn opened_snapshot_provider_stays_bound_when_selected_root_path_is_replaced() {
        let fixture = TestDirectory::new(0o700);
        let selected_path = fixture.path.join("selected-profile");
        let moved_path = fixture.path.join("moved-selected-profile");
        fs::create_dir(&selected_path).unwrap();
        fs::set_permissions(&selected_path, fs::Permissions::from_mode(0o700)).unwrap();

        let key = ObjectId(0x524f_4f54);
        let mut selected =
            DirectoryFdSnapshotStorage::new(File::open(&selected_path).unwrap()).unwrap();
        let selected_revision_one = match selected.compare_and_replace(key, None, b"selected-v1") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected selected-root write result: {other:?}"),
        };
        assert_eq!(selected_revision_one.get(), 1);

        fs::rename(&selected_path, &moved_path).unwrap();
        fs::create_dir(&selected_path).unwrap();
        fs::set_permissions(&selected_path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut replacement =
            DirectoryFdSnapshotStorage::new(File::open(&selected_path).unwrap()).unwrap();
        let replacement_revision =
            match replacement.compare_and_replace(key, None, b"replacement-v1") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected replacement-root write result: {other:?}"),
            };
        assert_eq!(replacement_revision.get(), 1);
        let replacement_record_path = selected_path.join(snapshot_name(key));
        let replacement_record_before = fs::read(&replacement_record_path).unwrap();

        let selected_revision_two =
            match selected.compare_and_replace(key, Some(selected_revision_one), b"selected-v2") {
                ConditionalWriteOutcome::Applied { revision } => revision,
                other => panic!("unexpected update through retained root FD: {other:?}"),
            };
        assert_eq!(selected_revision_two.get(), 2);
        assert_eq!(
            selected.read(key).unwrap(),
            Some(Snapshot::new(
                selected_revision_two,
                b"selected-v2".to_vec()
            ))
        );
        assert_eq!(
            replacement.read(key).unwrap(),
            Some(Snapshot::new(
                replacement_revision,
                b"replacement-v1".to_vec()
            ))
        );
        assert_eq!(
            fs::read(&replacement_record_path).unwrap(),
            replacement_record_before
        );
        drop(selected);
        drop(replacement);

        let reopened_selected =
            DirectoryFdSnapshotStorage::new(File::open(&moved_path).unwrap()).unwrap();
        let reopened_replacement =
            DirectoryFdSnapshotStorage::new(File::open(&selected_path).unwrap()).unwrap();
        assert_eq!(
            reopened_selected.read(key).unwrap(),
            Some(Snapshot::new(
                selected_revision_two,
                b"selected-v2".to_vec()
            ))
        );
        assert_eq!(
            reopened_replacement.read(key).unwrap(),
            Some(Snapshot::new(
                replacement_revision,
                b"replacement-v1".to_vec()
            ))
        );
    }

    #[test]
    fn ignores_orphaned_prepared_snapshot_when_reading_and_committing_next_revision() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x2026_1119);
        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let committed_revision_one = match storage.compare_and_replace(key, None, b"committed one")
        {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected initial write result: {other:?}"),
        };

        // Model a writer that synced a complete next-revision temp file, then
        // stopped before publishing it with the canonical snapshot rename.
        let previous_process = std::process::id().wrapping_add(1);
        let prepared_name = format!(
            ".nagi-snapshot-{previous_process:08x}-{:016x}.tmp",
            0xdead_beef_u64
        );
        let prepared_path = directory.path.join(prepared_name);
        let prepared_payload = b"prepared but not committed";
        let mut prepared_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&prepared_path)
            .unwrap();
        let mut prepared_record = Vec::new();
        prepared_record.extend_from_slice(FILE_MAGIC);
        prepared_record.extend_from_slice(&2u64.to_le_bytes());
        prepared_record.extend_from_slice(&(prepared_payload.len() as u64).to_le_bytes());
        prepared_record.extend_from_slice(prepared_payload);
        prepared_file.write_all(&prepared_record).unwrap();
        prepared_file.sync_all().unwrap();
        drop(prepared_file);

        let observed = storage.read(key).unwrap().unwrap();
        assert_eq!(observed.revision, committed_revision_one);
        assert_eq!(observed.bytes, b"committed one");
        assert_eq!(fs::read(&prepared_path).unwrap(), prepared_record);

        let committed_revision_two = match storage.compare_and_replace(
            key,
            Some(committed_revision_one),
            b"committed two",
        ) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected next write result: {other:?}"),
        };
        assert_eq!(committed_revision_two.get(), 2);
        drop(storage);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let observed = reopened.read(key).unwrap().unwrap();
        assert_eq!(observed.revision, committed_revision_two);
        assert_eq!(observed.bytes, b"committed two");
        assert_eq!(fs::read(&prepared_path).unwrap(), prepared_record);
    }

    #[test]
    fn corrupt_snapshot_is_not_absent_or_overwritten_on_create_only_write() {
        let directory = TestDirectory::new(0o700);
        let missing_key = ObjectId(0x101);
        let corrupt_key = ObjectId(0x102);
        let corrupt_bytes = b"damaged snapshot";
        let corrupt_path = directory.path.join(snapshot_name(corrupt_key));
        let mut corrupt_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&corrupt_path)
            .unwrap();
        corrupt_file.write_all(corrupt_bytes).unwrap();
        drop(corrupt_file);

        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(storage.read(missing_key).unwrap(), None);
        assert_eq!(
            storage.read(corrupt_key),
            Err(SnapshotStorageError::InvalidSnapshot)
        );
        assert_eq!(
            storage.compare_and_replace(corrupt_key, None, b""),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::InvalidSnapshot
            }
        );
        assert_eq!(fs::read(&corrupt_path).unwrap(), corrupt_bytes);
        assert_eq!(
            storage.read(corrupt_key),
            Err(SnapshotStorageError::InvalidSnapshot)
        );
        drop(storage);

        let mut reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(reopened.read(missing_key).unwrap(), None);
        assert_eq!(
            reopened.read(corrupt_key),
            Err(SnapshotStorageError::InvalidSnapshot)
        );
        assert_eq!(
            reopened.compare_and_replace(corrupt_key, None, b""),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::InvalidSnapshot
            }
        );
        assert_eq!(fs::read(&corrupt_path).unwrap(), corrupt_bytes);
    }

    #[test]
    fn invalid_snapshot_headers_are_not_absent_or_overwritten() {
        let directory = TestDirectory::new(0o700);
        let payload = b"header body";
        let cases = [
            (ObjectId(0x107), *b"BADMAGIC", 1u64),
            (ObjectId(0x108), *FILE_MAGIC, 0u64),
        ];
        let mut original_records = Vec::new();

        for (key, magic, revision) in cases {
            let path = directory.path.join(snapshot_name(key));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(&magic).unwrap();
            file.write_all(&revision.to_le_bytes()).unwrap();
            file.write_all(&(payload.len() as u64).to_le_bytes())
                .unwrap();
            file.write_all(payload).unwrap();
            drop(file);
            original_records.push((key, fs::read(path).unwrap()));
        }

        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        for (key, original) in &original_records {
            assert_eq!(
                storage.read(*key),
                Err(SnapshotStorageError::InvalidSnapshot)
            );
            assert_eq!(
                storage.compare_and_replace(*key, None, b"replacement"),
                ConditionalWriteOutcome::NotApplied {
                    error: SnapshotStorageError::InvalidSnapshot
                }
            );
            assert_eq!(
                fs::read(directory.path.join(snapshot_name(*key))).unwrap(),
                *original
            );
        }
        drop(storage);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        for (key, original) in original_records {
            assert_eq!(
                reopened.read(key),
                Err(SnapshotStorageError::InvalidSnapshot)
            );
            assert_eq!(
                fs::read(directory.path.join(snapshot_name(key))).unwrap(),
                original
            );
        }
    }

    #[test]
    fn mismatched_payload_length_is_not_absent_or_overwritten() {
        let directory = TestDirectory::new(0o700);
        let payload = b"record payload";
        let payload_len = payload.len() as u64;
        let cases = [
            (ObjectId(0x103), payload_len - 1),
            (ObjectId(0x104), payload_len + 1),
        ];
        let mut original_records = Vec::new();

        for (key, declared_payload_len) in cases {
            let path = directory.path.join(snapshot_name(key));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(FILE_MAGIC).unwrap();
            file.write_all(&1u64.to_le_bytes()).unwrap();
            file.write_all(&declared_payload_len.to_le_bytes()).unwrap();
            file.write_all(payload).unwrap();
            drop(file);
            original_records.push((key, fs::read(path).unwrap()));
        }

        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        for (key, original) in &original_records {
            assert_eq!(
                storage.read(*key),
                Err(SnapshotStorageError::InvalidSnapshot)
            );
            assert_eq!(
                storage.compare_and_replace(*key, None, b"replacement"),
                ConditionalWriteOutcome::NotApplied {
                    error: SnapshotStorageError::InvalidSnapshot
                }
            );
            assert_eq!(
                fs::read(directory.path.join(snapshot_name(*key))).unwrap(),
                *original
            );
        }
        drop(storage);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        for (key, original) in original_records {
            assert_eq!(
                reopened.read(key),
                Err(SnapshotStorageError::InvalidSnapshot)
            );
            assert_eq!(
                fs::read(directory.path.join(snapshot_name(key))).unwrap(),
                original
            );
        }
    }

    #[test]
    fn revision_exhaustion_does_not_wrap_or_overwrite_snapshot() {
        let directory = TestDirectory::new(0o700);
        // Fixture-only key; this does not assign a product ObjectId.
        let key = ObjectId(0x105);
        let max_revision = Revision::new(u64::MAX).unwrap();
        let payload = b"last representable revision";
        let path = directory.path.join(snapshot_name(key));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(FILE_MAGIC).unwrap();
        file.write_all(&u64::MAX.to_le_bytes()).unwrap();
        file.write_all(&(payload.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(payload).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let original_record = fs::read(&path).unwrap();

        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            storage.read(key).unwrap(),
            Some(Snapshot::new(max_revision, payload.to_vec()))
        );
        assert_eq!(
            storage.compare_and_replace(key, Some(max_revision), b"wrapped revision"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::RevisionExhausted
            }
        );
        assert_eq!(fs::read(&path).unwrap(), original_record);
        drop(storage);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened.read(key).unwrap(),
            Some(Snapshot::new(max_revision, payload.to_vec()))
        );
        assert_eq!(fs::read(path).unwrap(), original_record);
    }

    #[test]
    fn rejects_hard_linked_snapshot_without_changing_external_target() {
        let source_directory = TestDirectory::new(0o700);
        let data_directory = TestDirectory::new(0o700);
        let key = ObjectId(0x106);
        let payload = b"external hard-link target";
        let revision = match DirectoryFdSnapshotStorage::new(source_directory.duplicate_root())
            .unwrap()
            .compare_and_replace(key, None, payload)
        {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected source snapshot result: {other:?}"),
        };
        let source_path = source_directory.path.join(snapshot_name(key));
        let data_path = data_directory.path.join(snapshot_name(key));
        fs::hard_link(&source_path, &data_path).unwrap();
        let original_record = fs::read(&source_path).unwrap();
        assert_eq!(fs::read(&data_path).unwrap(), original_record);
        assert_eq!(fs::metadata(&data_path).unwrap().nlink(), 2);

        let mut storage = DirectoryFdSnapshotStorage::new(data_directory.duplicate_root()).unwrap();
        assert_eq!(storage.read(key), Err(SnapshotStorageError::InvalidRoot));
        assert_eq!(
            storage.compare_and_replace(key, None, b"replacement"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::InvalidRoot
            }
        );
        assert_eq!(fs::read(&source_path).unwrap(), original_record);
        assert_eq!(fs::read(&data_path).unwrap(), original_record);
        drop(storage);

        let reopened_data =
            DirectoryFdSnapshotStorage::new(data_directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened_data.read(key),
            Err(SnapshotStorageError::InvalidRoot)
        );
        assert_eq!(fs::read(&source_path).unwrap(), original_record);
        assert_eq!(fs::read(&data_path).unwrap(), original_record);
        drop(reopened_data);

        fs::remove_file(&data_path).unwrap();
        assert_eq!(fs::metadata(&source_path).unwrap().nlink(), 1);
        let reopened_source =
            DirectoryFdSnapshotStorage::new(source_directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened_source.read(key).unwrap(),
            Some(Snapshot::new(revision, payload.to_vec()))
        );
        assert_eq!(fs::read(&source_path).unwrap(), original_record);
    }

    #[test]
    fn snapshot_creation_uses_private_modes_even_with_restrictive_umask() {
        const ROOT_ENV: &str = "NAGI_TEST_SNAPSHOT_UMASK_ROOT";
        if let Some(path) = std::env::var_os(ROOT_ENV) {
            // This exact selector runs alone in the child process.
            rustix::process::umask(Mode::from_bits_truncate(0o777));
            let path = Path::new(&path);
            let key = ObjectId(57);
            let mut store = DirectoryFdSnapshotStorage::new(File::open(path).unwrap()).unwrap();
            let revision = Revision::new(1).unwrap();
            assert_eq!(
                store.compare_and_replace(key, None, b"saved"),
                ConditionalWriteOutcome::Applied { revision }
            );
            assert_eq!(
                fs::metadata(path.join(LOCK_FILE)).unwrap().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.join(snapshot_name(key))).unwrap().mode() & 0o777,
                0o600
            );
            drop(store);
            let reopened = DirectoryFdSnapshotStorage::new(File::open(path).unwrap()).unwrap();
            assert_eq!(
                reopened.read(key).unwrap(),
                Some(Snapshot::new(revision, b"saved".to_vec()))
            );
            return;
        }

        let directory = TestDirectory::new(0o700);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "snapshot::tests::snapshot_creation_uses_private_modes_even_with_restrictive_umask",
            ])
            .env(ROOT_ENV, &directory.path)
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}
{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn root_lock_still_serializes_when_lock_name_changes() {
        let directory = TestDirectory::new(0o700);
        let store = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let next = directory.duplicate_root();
        let lock = lock_root(
            &store.root,
            &store.root_identity,
            FlockOperation::LockExclusive,
        )
        .unwrap();
        let lock_name = directory.path.join(LOCK_FILE);
        fs::rename(&lock_name, directory.path.join("old-lock")).unwrap();
        fs::write(&lock_name, b"").unwrap();

        assert!(matches!(
            lock_root(
                &next,
                &store.root_identity,
                FlockOperation::NonBlockingLockExclusive
            ),
            Err(SnapshotStorageError::Unavailable)
        ));
        drop(lock);
        let _next_lock = lock_root(
            &next,
            &store.root_identity,
            FlockOperation::NonBlockingLockExclusive,
        )
        .unwrap();
        assert_eq!(
            store.validate_namespace(),
            Err(SnapshotStorageError::InvalidRoot)
        );
    }

    #[test]
    fn mismatched_publication_fences_same_and_fresh_provider_without_deleting_evidence() {
        let directory = TestDirectory::new(0o700);
        let mut store = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let key = ObjectId(58);
        let ConditionalWriteOutcome::Applied { revision } =
            store.compare_and_replace(key, None, b"observed record")
        else {
            panic!("initial snapshot did not commit");
        };
        let name = snapshot_name(key);
        let record = fs::read(directory.path.join(&name)).unwrap();
        let lock = lock_root(
            &store.root,
            &store.root_identity,
            FlockOperation::LockExclusive,
        )
        .unwrap();
        let (_temp_name, temporary) =
            create_temporary_file(&store.root, &store.root_identity).unwrap();
        let expected = validate_private_file(&store.root_identity, &temporary).unwrap();
        assert!(matches!(
            verify_published_identity(&store.root, &store.root_identity, &name, expected),
            Err(SnapshotWriteFailure::OutcomeUnknown(
                SnapshotStorageError::InvalidRoot
            ))
        ));
        drop(lock);
        assert!(directory.path.join(TAINT_FILE).is_file());
        assert_eq!(store.read(key), Err(SnapshotStorageError::InvalidRoot));
        assert!(matches!(
            store.compare_and_replace(key, Some(revision), b"must not publish"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::InvalidRoot
            }
        ));
        assert!(matches!(
            DirectoryFdSnapshotStorage::new(directory.duplicate_root()),
            Err(SnapshotStorageError::InvalidRoot)
        ));
        assert_eq!(fs::read(directory.path.join(name)).unwrap(), record);
    }

    #[test]
    fn rejects_unsafe_lock_file_links_without_changing_targets() {
        let hard_link_source = TestDirectory::new(0o700);
        let hard_link_root = TestDirectory::new(0o700);
        let hard_link_target = hard_link_source.path.join("lock target");
        let hard_link_entry = hard_link_root.path.join(LOCK_FILE);
        fs::write(&hard_link_target, b"external hard-link lock target").unwrap();
        fs::set_permissions(&hard_link_target, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&hard_link_target, &hard_link_entry).unwrap();
        let hard_link_bytes = fs::read(&hard_link_target).unwrap();
        assert_eq!(fs::metadata(&hard_link_entry).unwrap().nlink(), 2);
        assert!(matches!(
            DirectoryFdSnapshotStorage::new(hard_link_root.duplicate_root()),
            Err(SnapshotStorageError::InvalidRoot)
        ));
        assert_eq!(fs::read(&hard_link_target).unwrap(), hard_link_bytes);
        assert_eq!(fs::read(&hard_link_entry).unwrap(), hard_link_bytes);
        fs::remove_file(&hard_link_entry).unwrap();

        let symlink_source = TestDirectory::new(0o700);
        let symlink_root = TestDirectory::new(0o700);
        let symlink_target = symlink_source.path.join("lock target");
        let symlink_entry = symlink_root.path.join(LOCK_FILE);
        fs::write(&symlink_target, b"external symlink lock target").unwrap();
        symlink(&symlink_target, &symlink_entry).unwrap();
        let symlink_bytes = fs::read(&symlink_target).unwrap();
        assert!(matches!(
            DirectoryFdSnapshotStorage::new(symlink_root.duplicate_root()),
            Err(SnapshotStorageError::Unavailable)
        ));
        assert_eq!(fs::read(&symlink_target).unwrap(), symlink_bytes);
        assert!(fs::symlink_metadata(&symlink_entry)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn rejects_directory_at_snapshot_name_without_replacing_entry() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x109);
        let record_path = directory.path.join(snapshot_name(key));
        fs::create_dir(&record_path).unwrap();

        let mut storage = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(storage.read(key), Err(SnapshotStorageError::InvalidRoot));
        assert_eq!(
            storage.compare_and_replace(key, None, b"replacement"),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::InvalidRoot
            }
        );
        assert!(fs::metadata(&record_path).unwrap().is_dir());
        assert_eq!(fs::read_dir(&record_path).unwrap().count(), 0);
        drop(storage);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(reopened.read(key), Err(SnapshotStorageError::InvalidRoot));
        assert!(fs::metadata(&record_path).unwrap().is_dir());
        assert_eq!(fs::read_dir(&record_path).unwrap().count(), 0);
    }

    #[test]
    fn snapshot_survives_writer_exit_and_reopens_in_new_process() {
        // Child roles use the ordinary Rust test harness. This checks process
        // separation with a private host fixture, not a Data mount or service.
        const KEY: ObjectId = ObjectId(0x2026_1119);
        const OTHER_KEY: ObjectId = ObjectId(0x2026_1120);
        const FIRST_BYTES: &[u8] = b"initial value written by a separate process";
        const UPDATED_BYTES: &[u8] = b"updated value written by a separate process";
        const OTHER_BYTES: &[u8] = b"unrelated object remains unchanged";

        match std::env::var(PROCESS_TEST_MODE_ENV).as_deref() {
            Ok("writer") => {
                let root_path = std::env::var_os(PROCESS_TEST_ROOT_ENV)
                    .expect("writer process receives fixture root");
                let root = File::open(root_path).expect("writer opens fixture root");
                let mut storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                assert_eq!(storage.read(KEY).unwrap(), None);
                assert_eq!(storage.read(OTHER_KEY).unwrap(), None);
                let first_revision = match storage.compare_and_replace(KEY, None, FIRST_BYTES) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected initial snapshot write: {other:?}"),
                };
                assert_eq!(first_revision, Revision::new(1).unwrap());
                assert_eq!(
                    storage.compare_and_replace(OTHER_KEY, None, OTHER_BYTES),
                    ConditionalWriteOutcome::Applied {
                        revision: Revision::new(1).unwrap()
                    }
                );
                assert_eq!(
                    storage.compare_and_replace(KEY, Some(first_revision), UPDATED_BYTES),
                    ConditionalWriteOutcome::Applied {
                        revision: Revision::new(2).unwrap()
                    }
                );
            }
            Ok("reader") => {
                let root_path = std::env::var_os(PROCESS_TEST_ROOT_ENV)
                    .expect("reader process receives fixture root");
                let root = File::open(root_path).expect("reader opens fixture root");
                let storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                let updated_snapshot = storage
                    .read(KEY)
                    .unwrap()
                    .expect("writer process snapshot remains readable");
                assert_eq!(updated_snapshot.revision, Revision::new(2).unwrap());
                assert_eq!(updated_snapshot.bytes, UPDATED_BYTES);
                assert_eq!(
                    storage.read(OTHER_KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(1).unwrap(),
                        OTHER_BYTES.to_vec()
                    ))
                );
            }
            Ok(mode) => panic!("unexpected process test mode: {mode}"),
            Err(std::env::VarError::NotPresent) => {
                let directory = TestDirectory::new(0o700);
                let root_path = directory.path.as_os_str();
                let key_record_path = directory.path.join(snapshot_name(KEY));
                let other_record_path = directory.path.join(snapshot_name(OTHER_KEY));
                let test_binary = std::env::current_exe().expect("resolve Rust test binary");
                let mut record_after_writer = None;

                for mode in ["writer", "reader"] {
                    let output = Command::new(&test_binary)
                        .args(["--exact", PROCESS_TEST_CASE, "--nocapture"])
                        .env(PROCESS_TEST_MODE_ENV, mode)
                        .env(PROCESS_TEST_ROOT_ENV, root_path)
                        .output()
                        .expect("start process test child");
                    assert!(
                        output.status.success(),
                        "{mode} child failed: stdout={} stderr={}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    if mode == "writer" {
                        record_after_writer = Some((
                            fs::read(&key_record_path).unwrap(),
                            fs::read(&other_record_path).unwrap(),
                        ));
                    } else {
                        let (key_record, other_record) = record_after_writer
                            .as_ref()
                            .expect("writer snapshot records");
                        assert_eq!(fs::read(&key_record_path).unwrap(), *key_record);
                        assert_eq!(fs::read(&other_record_path).unwrap(), *other_record);
                    }
                }

                let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
                assert_eq!(
                    reopened.read(KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(2).unwrap(),
                        UPDATED_BYTES.to_vec()
                    ))
                );
                assert_eq!(
                    reopened.read(OTHER_KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(1).unwrap(),
                        OTHER_BYTES.to_vec()
                    ))
                );
            }
            Err(error) => panic!("read process test mode: {error}"),
        }
    }

    #[test]
    fn same_object_id_isolated_across_profile_reader_processes() {
        const KEY: ObjectId = ObjectId(0x5052_4f46);
        const FIRST_INITIAL: &[u8] = b"first profile initial value";
        const FIRST_UPDATED: &[u8] = b"first profile updated value";
        const SECOND_VALUE: &[u8] = b"second profile value";

        match std::env::var(PROFILE_PROCESS_TEST_MODE_ENV).as_deref() {
            Ok("write-first") => {
                let root_path = std::env::var_os(PROFILE_PROCESS_TEST_ROOT_ENV)
                    .expect("first-profile writer receives fixture root");
                let root = File::open(root_path).expect("first-profile writer opens root");
                let mut storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                assert_eq!(storage.read(KEY).unwrap(), None);
                let first_revision = match storage.compare_and_replace(KEY, None, FIRST_INITIAL) {
                    ConditionalWriteOutcome::Applied { revision } => revision,
                    other => panic!("unexpected first-profile initial write: {other:?}"),
                };
                assert_eq!(first_revision, Revision::new(1).unwrap());
                assert_eq!(
                    storage.compare_and_replace(KEY, Some(first_revision), FIRST_UPDATED),
                    ConditionalWriteOutcome::Applied {
                        revision: Revision::new(2).unwrap()
                    }
                );
            }
            Ok("write-second") => {
                let root_path = std::env::var_os(PROFILE_PROCESS_TEST_ROOT_ENV)
                    .expect("second-profile writer receives fixture root");
                let root = File::open(root_path).expect("second-profile writer opens root");
                let mut storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                assert_eq!(storage.read(KEY).unwrap(), None);
                assert_eq!(
                    storage.compare_and_replace(KEY, None, SECOND_VALUE),
                    ConditionalWriteOutcome::Applied {
                        revision: Revision::new(1).unwrap()
                    }
                );
            }
            Ok("read-first") => {
                let root_path = std::env::var_os(PROFILE_PROCESS_TEST_ROOT_ENV)
                    .expect("first-profile reader receives fixture root");
                let root = File::open(root_path).expect("first-profile reader opens root");
                let storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                assert_eq!(
                    storage.read(KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(2).unwrap(),
                        FIRST_UPDATED.to_vec()
                    ))
                );
            }
            Ok("read-second") => {
                let root_path = std::env::var_os(PROFILE_PROCESS_TEST_ROOT_ENV)
                    .expect("second-profile reader receives fixture root");
                let root = File::open(root_path).expect("second-profile reader opens root");
                let storage = DirectoryFdSnapshotStorage::new(root).unwrap();
                assert_eq!(
                    storage.read(KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(1).unwrap(),
                        SECOND_VALUE.to_vec()
                    ))
                );
            }
            Ok(mode) => panic!("unexpected profile process test mode: {mode}"),
            Err(std::env::VarError::NotPresent) => {
                let first_profile = TestDirectory::new(0o700);
                let second_profile = TestDirectory::new(0o700);
                let test_binary = std::env::current_exe().expect("resolve Rust test binary");
                let run_child = |mode: &str, root_path: &Path| {
                    let output = Command::new(&test_binary)
                        .args(["--exact", PROFILE_PROCESS_TEST_CASE, "--nocapture"])
                        .env(PROFILE_PROCESS_TEST_MODE_ENV, mode)
                        .env(PROFILE_PROCESS_TEST_ROOT_ENV, root_path)
                        .output()
                        .expect("start profile process test child");
                    assert!(
                        output.status.success(),
                        "{mode} child failed: stdout={} stderr={}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                };

                run_child("write-first", &first_profile.path);
                run_child("write-second", &second_profile.path);
                let first_record_path = first_profile.path.join(snapshot_name(KEY));
                let second_record_path = second_profile.path.join(snapshot_name(KEY));
                let first_record_before_read = fs::read(&first_record_path).unwrap();
                let second_record_before_read = fs::read(&second_record_path).unwrap();

                run_child("read-first", &first_profile.path);
                run_child("read-second", &second_profile.path);
                assert_eq!(
                    fs::read(&first_record_path).unwrap(),
                    first_record_before_read
                );
                assert_eq!(
                    fs::read(&second_record_path).unwrap(),
                    second_record_before_read
                );

                let first_reader =
                    DirectoryFdSnapshotStorage::new(first_profile.duplicate_root()).unwrap();
                let second_reader =
                    DirectoryFdSnapshotStorage::new(second_profile.duplicate_root()).unwrap();
                assert_eq!(
                    first_reader.read(KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(2).unwrap(),
                        FIRST_UPDATED.to_vec()
                    ))
                );
                assert_eq!(
                    second_reader.read(KEY).unwrap(),
                    Some(Snapshot::new(
                        Revision::new(1).unwrap(),
                        SECOND_VALUE.to_vec()
                    ))
                );
            }
            Err(error) => panic!("read profile process test mode: {error}"),
        }
    }

    #[test]
    fn normal_provider_close_releases_fds_before_fresh_readback() {
        const KEY: ObjectId = ObjectId(0x2026_1122);
        const BYTES: &[u8] = b"snapshot remains readable after provider close";

        let directory = TestDirectory::new(0o700);
        let mut writer = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            writer.compare_and_replace(KEY, None, BYTES),
            ConditionalWriteOutcome::Applied {
                revision: Revision::new(1).unwrap()
            }
        );
        let root_fd = writer.root.as_raw_fd();
        let lock_fd = writer.lock_file.as_raw_fd();

        drop(writer);

        for closed_fd in [root_fd, lock_fd] {
            assert!(
                File::open(format!("/dev/fd/{closed_fd}")).is_err(),
                "provider close must release owned fd {closed_fd}"
            );
        }

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened.read(KEY).unwrap(),
            Some(Snapshot::new(Revision::new(1).unwrap(), BYTES.to_vec()))
        );
    }

    #[test]
    fn read_only_provider_close_releases_fds_without_changing_snapshot() {
        const KEY: ObjectId = ObjectId(0x2026_1123);
        const BYTES: &[u8] = b"read-only snapshot survives provider close";

        let directory = TestDirectory::new(0o700);
        let mut seed = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let revision = match seed.compare_and_replace(KEY, None, BYTES) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected seed write result: {other:?}"),
        };
        drop(seed);

        let record_path = directory.path.join(snapshot_name(KEY));
        let record_before_read = fs::read(&record_path).unwrap();
        let mut entries_before_read = fs::read_dir(&directory.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        entries_before_read.sort();
        let calls = Rc::new(SnapshotCallCounts::default());
        let mut provider = CountingSnapshotStorage {
            inner: DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap(),
            calls: Rc::clone(&calls),
        };
        let root_fd = provider.inner.root.as_raw_fd();
        let lock_fd = provider.inner.lock_file.as_raw_fd();

        let observed =
            with_single_object_snapshot_storage(&mut provider, KEY, |view| view.read(KEY).unwrap());
        assert_eq!(observed, Some(Snapshot::new(revision, BYTES.to_vec())));
        assert_eq!(calls.reads.get(), 1);
        assert_eq!(calls.writes.get(), 0);
        drop(provider);

        for closed_fd in [root_fd, lock_fd] {
            assert!(
                File::open(format!("/dev/fd/{closed_fd}")).is_err(),
                "read-only provider close must release owned fd {closed_fd}"
            );
        }
        assert_eq!(fs::read(&record_path).unwrap(), record_before_read);
        let mut entries_after_read = fs::read_dir(&directory.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        entries_after_read.sort();
        assert_eq!(entries_after_read, entries_before_read);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            reopened.read(KEY).unwrap(),
            Some(Snapshot::new(revision, BYTES.to_vec()))
        );
    }

    #[test]
    fn concurrent_updates_from_same_revision_have_one_winner_across_provider_handles() {
        const FIRST_BODY: &[u8] = b"first concurrent update";
        const SECOND_BODY: &[u8] = b"second concurrent update";

        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x77);
        let mut seed = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let base_revision = match seed.compare_and_replace(key, None, b"seed revision") {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected seed write result: {other:?}"),
        };
        assert_eq!(base_revision.get(), 1);
        drop(seed);

        let first = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let second = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let first_barrier = Arc::clone(&barrier);
        let first_thread = thread::spawn(move || {
            let mut store = first;
            first_barrier.wait();
            store.compare_and_replace(key, Some(base_revision), FIRST_BODY)
        });
        let second_thread = thread::spawn(move || {
            let mut store = second;
            barrier.wait();
            store.compare_and_replace(key, Some(base_revision), SECOND_BODY)
        });
        let results = [first_thread.join().unwrap(), second_thread.join().unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ConditionalWriteOutcome::Applied { .. }))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, ConditionalWriteOutcome::Conflict { .. }))
                .count(),
            1
        );

        let (winner_body, winner_revision) = match (&results[0], &results[1]) {
            (
                ConditionalWriteOutcome::Applied { revision },
                ConditionalWriteOutcome::Conflict { current_revision },
            ) => {
                assert_eq!(*current_revision, Some(*revision));
                (FIRST_BODY, *revision)
            }
            (
                ConditionalWriteOutcome::Conflict { current_revision },
                ConditionalWriteOutcome::Applied { revision },
            ) => {
                assert_eq!(*current_revision, Some(*revision));
                (SECOND_BODY, *revision)
            }
            _ => panic!("expected one winner and one conflict: {results:?}"),
        };
        assert_eq!(winner_revision.get(), 2);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let committed = reopened.read(key).unwrap().unwrap();
        assert_eq!(committed.revision, winner_revision);
        assert_eq!(committed.bytes, winner_body);
    }

    #[test]
    fn rejects_non_private_roots_and_snapshot_symlinks() {
        let non_private = TestDirectory::new(0o755);
        assert!(matches!(
            DirectoryFdSnapshotStorage::new(non_private.duplicate_root()),
            Err(SnapshotStorageError::InvalidRoot)
        ));
        drop(non_private);

        let directory = TestDirectory::new(0o700);
        let outside = directory.path.with_extension("outside");
        fs::write(&outside, b"outside payload").unwrap();
        symlink(&outside, directory.path.join(snapshot_name(ObjectId(9)))).unwrap();
        let store = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        assert_eq!(
            store.read(ObjectId(9)),
            Err(SnapshotStorageError::Unavailable)
        );
        assert_eq!(fs::read(&outside).unwrap(), b"outside payload");
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn marks_received_root_descriptor_close_on_exec() {
        let directory = TestDirectory::new(0o700);
        let root = directory.duplicate_root();
        fcntl_setfd(&root, FdFlags::empty()).unwrap();
        assert!(!fcntl_getfd(&root).unwrap().contains(FdFlags::CLOEXEC));

        let store = DirectoryFdSnapshotStorage::new(root).unwrap();
        assert!(fcntl_getfd(&store.root).unwrap().contains(FdFlags::CLOEXEC));
    }

    #[test]
    fn accepts_exact_service_payload_limit_and_reopens_it() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x887);
        let payload = vec![0x5a; MAX_SNAPSHOT_BYTES];
        let mut store = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let revision = match store.compare_and_replace(key, None, &payload) {
            ConditionalWriteOutcome::Applied { revision } => revision,
            other => panic!("unexpected exact-limit write result: {other:?}"),
        };
        assert_eq!(revision.get(), 1);
        drop(store);

        let reopened = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let snapshot = reopened.read(key).unwrap().unwrap();
        assert_eq!(snapshot.revision, revision);
        assert_eq!(snapshot.bytes, payload);
    }

    #[test]
    fn enforces_existing_service_payload_limit_before_snapshot_allocation() {
        let directory = TestDirectory::new(0o700);
        let key = ObjectId(0x888);
        let mut store = DirectoryFdSnapshotStorage::new(directory.duplicate_root()).unwrap();
        let oversized_payload = vec![0; MAX_SNAPSHOT_BYTES + 1];
        assert_eq!(
            store.compare_and_replace(key, None, &oversized_payload),
            ConditionalWriteOutcome::NotApplied {
                error: SnapshotStorageError::SnapshotTooLarge
            }
        );
        assert_eq!(store.read(key).unwrap(), None);

        let path = directory.path.join(snapshot_name(key));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(FILE_MAGIC).unwrap();
        file.write_all(&1u64.to_le_bytes()).unwrap();
        file.write_all(&((MAX_SNAPSHOT_BYTES as u64) + 1).to_le_bytes())
            .unwrap();
        file.set_len(HEADER_LEN + MAX_SNAPSHOT_BYTES as u64 + 1)
            .unwrap();
        drop(file);

        assert_eq!(store.read(key), Err(SnapshotStorageError::SnapshotTooLarge));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_path_only_root_before_creating_the_lock_file() {
        let directory = TestDirectory::new(0o700);
        let root = rustix::fs::open(
            &directory.path,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();

        assert!(matches!(
            DirectoryFdSnapshotStorage::new(File::from(root)),
            Err(SnapshotStorageError::InvalidRoot)
        ));
        assert!(!directory.path.join(LOCK_FILE).exists());
    }
}
