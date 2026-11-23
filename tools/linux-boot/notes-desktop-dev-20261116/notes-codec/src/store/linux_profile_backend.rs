//! Linux `ProfileStorageBackend` over one already-authorized object directory.
//!
//! The profile-volume owner supplies the directory descriptor and its exact
//! identity tuple. This adapter never chooses a volume, resolves a host path,
//! creates a profile root, or derives authority from profile data. All object
//! names are fixed hexadecimal Object IDs relative to that descriptor.

use nagi_identity_session::{
    AuthenticatedInteractiveCaller, ProfileId, ProfileStorageBackend, ResolvedIdentity,
    StorageRootFailure, UserId,
};
use nagi_model::ObjectId;
use rustix::fs::{
    fstat, fsync, linkat, openat, renameat, statat, unlinkat, AtFlags, FileType, Mode, OFlags,
};
use rustix::io::Errno;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const MAX_PROFILE_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEMP_NAME_ATTEMPTS: usize = 32;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[doc(hidden)]
pub struct LinuxProfileStorageRoot {
    user_id: Option<UserId>,
    profile_id: ProfileId,
    directory: OwnedFd,
    operation_gate: Mutex<()>,
    io_budget: Duration,
    #[cfg(test)]
    fail_next_directory_sync: std::sync::atomic::AtomicBool,
}

impl LinuxProfileStorageRoot {
    fn lock(&self, deadline: Instant) -> Result<MutexGuard<'_, ()>, StorageRootFailure> {
        check_deadline(deadline)?;
        let guard = match self.operation_gate.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => {
                return Err(StorageRootFailure::Unavailable);
            }
        };
        check_deadline(deadline)?;
        Ok(guard)
    }

    fn sync_directory(&self, deadline: Instant) -> Result<(), StorageRootFailure> {
        check_deadline(deadline)?;
        #[cfg(test)]
        let path_only = if self.fail_next_directory_sync.swap(false, Ordering::SeqCst) {
            // O_PATH identifies the same directory but fsync on it returns EBADF.
            Some(
                openat(
                    &self.directory,
                    ".",
                    OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .expect("open the same directory with O_PATH"),
            )
        } else {
            None
        };
        #[cfg(test)]
        let directory = path_only.as_ref().unwrap_or(&self.directory);
        #[cfg(not(test))]
        let directory = &self.directory;
        fsync(directory).map_err(|_| StorageRootFailure::Unavailable)?;
        check_deadline(deadline)
    }

    /// Confirm an already-published object after Notes matched the candidate,
    /// inside the same authorized C1 effect as the read.
    pub(super) fn confirm_directory_sync(&self) -> Result<(), StorageRootFailure> {
        let deadline = make_deadline(self.io_budget)?;
        let _guard = self.lock(deadline)?;
        self.sync_directory(deadline)
    }
}

/// A concrete Linux C1 profile-storage backend for one root supplied by the
/// profile-volume owner.
///
/// The descriptor must already name the authorized profile's object
/// directory. `io_budget` bounds the time spent between synchronous system
/// calls and rejects an operation whose calls return after the budget. Linux
/// does not provide cancellation for a blocking regular-file `read`, `write`,
/// or `fsync`, so the volume owner must also provide a filesystem/device whose
/// synchronous calls honor the same bound. This implementation never spawns,
/// joins, or calls back into C1 while a C1 root-effect lease is held.
pub struct LinuxProfileStorageBackend {
    user_id: Option<UserId>,
    profile_id: ProfileId,
    root: Arc<LinuxProfileStorageRoot>,
    root_opened: bool,
}

impl LinuxProfileStorageBackend {
    /// Tie the retained object-directory descriptor to a live C1 identity and
    /// the same C2-authenticated caller that will be passed into the root
    /// adapter. The profile-volume owner remains responsible for supplying the
    /// descriptor and Notes Object ID from the same mounted profile volume.
    pub(super) fn from_authenticated_identity_fd(
        root_fd: OwnedFd,
        identity: &ResolvedIdentity,
        caller: &impl AuthenticatedInteractiveCaller,
        requested_profile: &ProfileId,
        io_budget: Duration,
    ) -> Result<Self, StorageRootFailure> {
        if !identity.is_live() {
            return Err(StorageRootFailure::StaleSession);
        }
        if identity.principal_id() != caller.principal_id()
            || identity.caller_context_id() != caller.context_id()
        {
            return Err(StorageRootFailure::CallerMismatch);
        }
        if identity.profile_id() != requested_profile {
            return Err(StorageRootFailure::CrossProfile);
        }
        let user_id = identity.user_id();
        let profile_id = requested_profile.clone();
        let backend = Self::from_profile_scope_fd(root_fd, user_id, profile_id, io_budget)?;
        if !identity.is_live() {
            return Err(StorageRootFailure::StaleSession);
        }
        Ok(backend)
    }

    fn from_profile_scope_fd(
        root_fd: OwnedFd,
        user_id: Option<UserId>,
        profile_id: ProfileId,
        io_budget: Duration,
    ) -> Result<Self, StorageRootFailure> {
        if io_budget.is_zero() {
            return Err(StorageRootFailure::Unavailable);
        }
        let deadline = make_deadline(io_budget)?;
        let metadata = fstat(&root_fd).map_err(map_errno)?;
        check_deadline(deadline)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(StorageRootFailure::Denied);
        }

        // Reopen through the supplied descriptor so O_PATH descriptors become
        // a usable directory handle without resolving or copying a path.
        let directory = openat(
            &root_fd,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_errno)?;
        check_deadline(deadline)?;
        let reopened = fstat(&directory).map_err(map_errno)?;
        check_deadline(deadline)?;
        if !FileType::from_raw_mode(reopened.st_mode).is_dir() {
            return Err(StorageRootFailure::Denied);
        }

        let root = Arc::new(LinuxProfileStorageRoot {
            user_id,
            profile_id: profile_id.clone(),
            directory,
            operation_gate: Mutex::new(()),
            io_budget,
            #[cfg(test)]
            fail_next_directory_sync: std::sync::atomic::AtomicBool::new(false),
        });
        Ok(Self {
            user_id,
            profile_id,
            root,
            root_opened: false,
        })
    }

    fn check_root(&self, root: &Arc<LinuxProfileStorageRoot>) -> Result<(), StorageRootFailure> {
        if !Arc::ptr_eq(root, &self.root)
            || root.user_id != self.user_id
            || root.profile_id != self.profile_id
        {
            return Err(StorageRootFailure::CrossProfile);
        }
        Ok(())
    }

    pub(super) fn root_for_durability(&self) -> Arc<LinuxProfileStorageRoot> {
        Arc::clone(&self.root)
    }

    fn read_object_inner(
        &self,
        root: &Arc<LinuxProfileStorageRoot>,
        object_id: ObjectId,
        deadline: Instant,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        check_deadline(deadline)?;
        let name = object_name(object_id);
        let fd = openat(
            &root.directory,
            name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_errno)?;
        check_deadline(deadline)?;

        let metadata = fstat(&fd).map_err(map_errno)?;
        check_deadline(deadline)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(StorageRootFailure::Denied);
        }
        if metadata.st_size < 0 || metadata.st_size as u64 > MAX_PROFILE_OBJECT_BYTES as u64 {
            return Err(StorageRootFailure::Unavailable);
        }

        let expected_size = metadata.st_size as usize;
        let mut file = File::from(fd);
        let mut bytes = Vec::new();
        bytes
            .try_reserve(expected_size)
            .map_err(|_| StorageRootFailure::Unavailable)?;
        let mut chunk = [0_u8; 8192];
        loop {
            check_deadline(deadline)?;
            let count = file
                .read(&mut chunk)
                .map_err(|_| StorageRootFailure::Unavailable)?;
            check_deadline(deadline)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_PROFILE_OBJECT_BYTES {
                return Err(StorageRootFailure::Unavailable);
            }
            bytes
                .try_reserve(count)
                .map_err(|_| StorageRootFailure::Unavailable)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        check_deadline(deadline)?;
        if bytes.len() != expected_size {
            return Err(StorageRootFailure::Unavailable);
        }
        Ok(bytes)
    }

    fn write_object_inner(
        &self,
        root: &Arc<LinuxProfileStorageRoot>,
        object_id: ObjectId,
        data: &[u8],
        deadline: Instant,
    ) -> Result<(), StorageRootFailure> {
        if data.len() > MAX_PROFILE_OBJECT_BYTES {
            return Err(StorageRootFailure::Unavailable);
        }
        check_deadline(deadline)?;
        let name = object_name(object_id);
        ensure_regular_destination(&root.directory, &name, deadline)?;
        let (temporary_name, fd) = create_temp_file(&root.directory, deadline)?;
        let mut renamed = false;

        let result = (|| {
            let mut file = File::from(fd);
            let mut offset = 0;
            while offset < data.len() {
                check_deadline(deadline)?;
                let count = file
                    .write(&data[offset..])
                    .map_err(|_| StorageRootFailure::Unavailable)?;
                check_deadline(deadline)?;
                if count == 0 {
                    return Err(StorageRootFailure::Unavailable);
                }
                offset += count;
            }

            check_deadline(deadline)?;
            file.sync_all()
                .map_err(|_| StorageRootFailure::Unavailable)?;
            check_deadline(deadline)?;
            drop(file);

            ensure_regular_destination(&root.directory, &name, deadline)?;
            check_deadline(deadline)?;
            renameat(
                &root.directory,
                temporary_name.as_str(),
                &root.directory,
                name.as_str(),
            )
            .map_err(map_errno)?;
            renamed = true;

            root.sync_directory(deadline)
        })();

        if result.is_err() && !renamed {
            let _ = unlinkat(&root.directory, temporary_name.as_str(), AtFlags::empty());
        }
        result
    }
}

impl ProfileStorageBackend for LinuxProfileStorageBackend {
    type Root = Arc<LinuxProfileStorageRoot>;

    fn open_profile_root(
        &mut self,
        user_id: Option<UserId>,
        profile_id: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        if user_id != self.user_id || profile_id != &self.profile_id {
            return Err(StorageRootFailure::CrossProfile);
        }
        if self.root_opened {
            return Err(StorageRootFailure::InvalidHandle);
        }
        self.root_opened = true;
        Ok(Arc::clone(&self.root))
    }

    fn read_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.check_root(root)?;
        let deadline = make_deadline(root.io_budget)?;
        let _guard = root.lock(deadline)?;
        self.read_object_inner(root, object_id, deadline)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.check_root(root)?;
        let deadline = make_deadline(root.io_budget)?;
        let _guard = root.lock(deadline)?;
        self.write_object_inner(root, object_id, data, deadline)
    }
}

const INITIALIZED_INDEX: &str = ".notes-index-initialized-v1";

/// Run only inside the authorized C1 root effect for this exact directory.
/// The marker is durable before the factory exposes the store to Notes.
pub(super) fn initialize_notes_index(
    directory: &OwnedFd,
    object: ObjectId,
    allow_initial_index: bool,
    empty_index: &[u8],
    budget: Duration,
    validate: impl FnOnce() -> Result<(), StorageRootFailure>,
) -> Result<(), StorageRootFailure> {
    let deadline = make_deadline(budget)?;
    let name = object_name(object);
    if !has_initialized_index(directory, &name, deadline)? {
        match statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => {}
            Err(Errno::NOENT) if allow_initial_index => {
                publish_new(directory, &name, empty_index, deadline)?;
            }
            Err(error) => return Err(map_errno(error)),
        }
        check_deadline(deadline)?;
        validate()?;
        publish_new(directory, INITIALIZED_INDEX, name.as_bytes(), deadline)?;
        if !has_initialized_index(directory, &name, deadline)? {
            return Err(StorageRootFailure::Unavailable);
        }
    } else {
        validate()?;
    }
    check_deadline(deadline)
}

fn has_initialized_index(
    directory: &OwnedFd,
    object_name: &str,
    deadline: Instant,
) -> Result<bool, StorageRootFailure> {
    check_deadline(deadline)?;
    let marker = match openat(
        directory,
        INITIALIZED_INDEX,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(marker) => File::from(marker),
        Err(Errno::NOENT) => return Ok(false),
        Err(error) => return Err(map_errno(error)),
    };
    let metadata = fstat(&marker).map_err(map_errno)?;
    check_deadline(deadline)?;
    if !FileType::from_raw_mode(metadata.st_mode).is_file()
        || metadata.st_size != object_name.len() as i64
    {
        return Err(StorageRootFailure::Denied);
    }
    let mut marker_bytes = [0u8; 16];
    (&marker)
        .read_exact(&mut marker_bytes)
        .map_err(|_| StorageRootFailure::Unavailable)?;
    check_deadline(deadline)?;
    if marker_bytes != object_name.as_bytes() {
        return Err(StorageRootFailure::Denied);
    }
    // A previous link may have succeeded before its directory fsync failed.
    fsync(directory).map_err(map_errno)?;
    check_deadline(deadline)?;
    Ok(true)
}

fn publish_new(
    directory: &OwnedFd,
    name: &str,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), StorageRootFailure> {
    check_deadline(deadline)?;
    let mut file = File::from(
        openat(
            directory,
            ".",
            OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(map_errno)?,
    );
    check_deadline(deadline)?;
    file.write_all(bytes)
        .map_err(|_| StorageRootFailure::Unavailable)?;
    check_deadline(deadline)?;
    file.sync_all()
        .map_err(|_| StorageRootFailure::Unavailable)?;
    check_deadline(deadline)?;
    match linkat(&file, c"", directory, name, AtFlags::EMPTY_PATH) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(map_errno(error)),
    }
    check_deadline(deadline)?;
    fsync(directory).map_err(map_errno)?;
    check_deadline(deadline)
}

fn object_name(object_id: ObjectId) -> String {
    format!("{:016x}", object_id.0)
}

fn make_deadline(budget: Duration) -> Result<Instant, StorageRootFailure> {
    if budget.is_zero() {
        return Err(StorageRootFailure::Unavailable);
    }
    Instant::now()
        .checked_add(budget)
        .ok_or(StorageRootFailure::Unavailable)
}

fn check_deadline(deadline: Instant) -> Result<(), StorageRootFailure> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(StorageRootFailure::Unavailable)
    }
}

fn map_errno(error: Errno) -> StorageRootFailure {
    match error {
        Errno::ACCESS | Errno::PERM | Errno::LOOP | Errno::NOTDIR => StorageRootFailure::Denied,
        _ => StorageRootFailure::Unavailable,
    }
}

fn ensure_regular_destination(
    directory: &OwnedFd,
    name: &str,
    deadline: Instant,
) -> Result<(), StorageRootFailure> {
    check_deadline(deadline)?;
    match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_file() => {
            check_deadline(deadline)
        }
        Ok(_) => Err(StorageRootFailure::Denied),
        Err(Errno::NOENT) => check_deadline(deadline),
        Err(error) => Err(map_errno(error)),
    }
}

fn create_temp_file(
    directory: &OwnedFd,
    deadline: Instant,
) -> Result<(String, OwnedFd), StorageRootFailure> {
    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        check_deadline(deadline)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".nagi-object-tmp-{}-{sequence:016x}", std::process::id());
        match openat(
            directory,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        ) {
            Ok(fd) => {
                if let Err(error) = check_deadline(deadline) {
                    drop(fd);
                    let _ = unlinkat(directory, name.as_str(), AtFlags::empty());
                    return Err(error);
                }
                return Ok((name, fd));
            }
            Err(Errno::EXIST) => continue,
            Err(error) => return Err(map_errno(error)),
        }
    }
    Err(StorageRootFailure::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::fs::open;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-linux-profile-backend-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn open_fd(&self) -> OwnedFd {
            open(
                &self.0,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap()
        }

        fn object_path(&self, object_id: ObjectId) -> PathBuf {
            self.0.join(object_name(object_id))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_backend(
        root: &TempRoot,
        budget: Duration,
    ) -> (LinuxProfileStorageBackend, UserId, ProfileId) {
        let user_id = UserId::new(77).unwrap();
        let profile_id = ProfileId::new("profile_linux_backend_test").unwrap();
        let backend = LinuxProfileStorageBackend::from_profile_scope_fd(
            root.open_fd(),
            Some(user_id),
            profile_id.clone(),
            budget,
        )
        .unwrap();
        (backend, user_id, profile_id)
    }

    #[test]
    fn linux_profile_root_save_waits_for_real_directory_sync() {
        use super::super::profile_root::{
            empty_profile_root_notes_object, finish_root_write, map_durable_write_failure,
            tests::{check_durable_retry, RETRY_INDEX_ID},
            ProfileRootNotesObjectIo,
        };
        use super::super::StoreError;
        use std::sync::atomic::AtomicUsize;

        // This fixture supplies the already-admitted root effect. Object I/O,
        // write outcome handling and directory sync are the production paths.
        struct BackendIo {
            backend: LinuxProfileStorageBackend,
            root: Arc<LinuxProfileStorageRoot>,
            writes: Arc<AtomicUsize>,
        }
        impl ProfileRootNotesObjectIo for BackendIo {
            fn read_object(&mut self, object: ObjectId) -> Result<Vec<u8>, StoreError> {
                self.backend
                    .read_object(&self.root, object)
                    .map_err(|_| StoreError::Unavailable)
            }

            fn write_object(&mut self, object: ObjectId, bytes: &[u8]) -> Result<(), StoreError> {
                self.writes.fetch_add(1, Ordering::SeqCst);
                let result = self.backend.write_object(&self.root, object, bytes);
                finish_root_write(result, true, bytes, || {
                    self.backend.read_object(&self.root, object)
                })
            }

            fn confirm_existing_object(&mut self) -> Result<(), StoreError> {
                self.root
                    .confirm_directory_sync()
                    .map_err(map_durable_write_failure)
            }
        }

        let directory = TempRoot::new();
        let (mut backend, user_id, profile_id) = make_backend(&directory, Duration::from_secs(1));
        let root = backend
            .open_profile_root(Some(user_id), &profile_id)
            .unwrap();
        backend
            .write_object(&root, RETRY_INDEX_ID, &empty_profile_root_notes_object())
            .unwrap();
        let writes = Arc::new(AtomicUsize::new(0));
        check_durable_retry(
            BackendIo {
                backend,
                root: root.clone(),
                writes: writes.clone(),
            },
            || root.fail_next_directory_sync.store(true, Ordering::SeqCst),
            &writes,
        );
    }

    #[test]
    fn parent_sync_failure_leaves_published_object_visible() {
        let directory = TempRoot::new();
        let (mut backend, user_id, profile_id) = make_backend(&directory, Duration::from_secs(1));
        let root = backend
            .open_profile_root(Some(user_id), &profile_id)
            .unwrap();
        let object_id = ObjectId(0xfeed_0001);
        let expected = b"published index bytes";

        root.fail_next_directory_sync.store(true, Ordering::SeqCst);
        assert_eq!(
            backend.write_object(&root, object_id, expected),
            Err(StorageRootFailure::Unavailable)
        );
        assert_eq!(backend.read_object(&root, object_id).unwrap(), expected);
        assert_eq!(
            fs::read(directory.object_path(object_id)).unwrap(),
            expected
        );
        assert_eq!(root.confirm_directory_sync(), Ok(()));
    }

    #[test]
    fn profile_and_root_mismatches_fail_closed() {
        let first_directory = TempRoot::new();
        let (mut first, user_id, profile_id) =
            make_backend(&first_directory, Duration::from_secs(1));
        let first_root = first.open_profile_root(Some(user_id), &profile_id).unwrap();
        let other_profile = ProfileId::new("profile_other_backend_test").unwrap();
        assert!(matches!(
            first.open_profile_root(Some(user_id), &other_profile),
            Err(StorageRootFailure::CrossProfile)
        ));

        let second_directory = TempRoot::new();
        let (mut second, _, _) = make_backend(&second_directory, Duration::from_secs(1));
        let foreign_root = second
            .open_profile_root(Some(user_id), &profile_id)
            .unwrap();
        assert!(matches!(
            first.read_object(&foreign_root, ObjectId(9)),
            Err(StorageRootFailure::CrossProfile)
        ));
        assert!(matches!(
            first.read_object(&first_root, ObjectId(9)),
            Err(StorageRootFailure::Unavailable)
        ));
    }

    #[test]
    fn symlink_and_nonblocking_reentry_are_rejected() {
        let directory = TempRoot::new();
        let (mut backend, user_id, profile_id) = make_backend(&directory, Duration::from_secs(1));
        let root = backend
            .open_profile_root(Some(user_id), &profile_id)
            .unwrap();
        let object_id = ObjectId(0xfeed_0002);
        let outside = directory.0.with_file_name(format!(
            "nagi-profile-backend-outside-{}",
            std::process::id()
        ));
        fs::write(&outside, b"outside bytes").unwrap();
        symlink(&outside, directory.object_path(object_id)).unwrap();
        assert!(matches!(
            backend.read_object(&root, object_id),
            Err(StorageRootFailure::Denied)
        ));
        assert!(matches!(
            backend.write_object(&root, object_id, b"replacement"),
            Err(StorageRootFailure::Denied)
        ));
        assert_eq!(fs::read(&outside).unwrap(), b"outside bytes");

        let _guard = root.operation_gate.try_lock().unwrap();
        assert!(matches!(
            backend.read_object(&root, ObjectId(0xfeed_0003)),
            Err(StorageRootFailure::Unavailable)
        ));
        drop(_guard);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn zero_and_expired_io_budgets_fail_closed() {
        let directory = TempRoot::new();
        assert!(matches!(
            LinuxProfileStorageBackend::from_profile_scope_fd(
                directory.open_fd(),
                None,
                ProfileId::new("profile_linux_backend_test").unwrap(),
                Duration::ZERO,
            ),
            Err(StorageRootFailure::Unavailable)
        ));

        let (mut backend, user_id, profile_id) = make_backend(&directory, Duration::from_secs(1));
        let root = backend
            .open_profile_root(Some(user_id), &profile_id)
            .unwrap();
        let expired = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert!(matches!(
            backend.read_object_inner(&root, ObjectId(0xfeed_0004), expired),
            Err(StorageRootFailure::Unavailable)
        ));
    }
}
