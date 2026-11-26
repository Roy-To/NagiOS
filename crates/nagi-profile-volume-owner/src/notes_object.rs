//! Filesystem operations on one owner-reserved Notes index.

use nagi_model::ObjectId;
use rustix::fs::{
    fstat, fsync, linkat, openat, renameat, statat, unlinkat, AtFlags, FileType, Mode, OFlags,
};
use rustix::io::Errno;
use std::fs::File;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::os::fd::OwnedFd;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const MAX_PROFILE_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEMP_NAME_ATTEMPTS: usize = 32;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotesObjectError {
    Denied,
    Unavailable,
}

/// Non-transferable I/O view of the exact Notes index reserved by the owner.
/// It provides neither a directory descriptor nor a caller-selected path.
pub struct NotesObject<'owner> {
    directory: &'owner OwnedFd,
    object_id: ObjectId,
    allow_initial_index: bool,
    io_budget: Duration,
    _local: PhantomData<Rc<()>>,
}

impl<'owner> NotesObject<'owner> {
    pub(super) fn new(
        directory: &'owner OwnedFd,
        object_id: ObjectId,
        allow_initial_index: bool,
        io_budget: Duration,
    ) -> Self {
        Self {
            directory,
            object_id,
            allow_initial_index,
            io_budget,
            _local: PhantomData,
        }
    }

    pub fn read(&self) -> Result<Vec<u8>, NotesObjectError> {
        self.read_object_inner(make_deadline(self.io_budget)?)
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), NotesObjectError> {
        self.write_object_inner(bytes, make_deadline(self.io_budget)?, sync_directory)
    }

    pub fn confirm_directory_sync(&self) -> Result<(), NotesObjectError> {
        sync_directory(self.directory, make_deadline(self.io_budget)?)
    }

    /// Recover an interrupted first initialization without overwriting a
    /// previously published index. Validate the same object's contents before
    /// committing the initialization marker.
    pub fn initialize(
        &mut self,
        empty_index: &[u8],
        validate: impl FnOnce(&[u8]) -> Result<(), NotesObjectError>,
    ) -> Result<(), NotesObjectError> {
        if empty_index.len() > MAX_PROFILE_OBJECT_BYTES {
            return Err(NotesObjectError::Unavailable);
        }
        initialize_notes_index(
            self.directory,
            self.object_id,
            self.allow_initial_index,
            empty_index,
            self.io_budget,
            || validate(&self.read()?),
        )
    }

    fn read_object_inner(&self, deadline: Instant) -> Result<Vec<u8>, NotesObjectError> {
        check_deadline(deadline)?;
        let name = object_name(self.object_id);
        let fd = openat(
            self.directory,
            name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_errno)?;
        check_deadline(deadline)?;

        let metadata = fstat(&fd).map_err(map_errno)?;
        check_deadline(deadline)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(NotesObjectError::Denied);
        }
        if metadata.st_size < 0 || metadata.st_size as u64 > MAX_PROFILE_OBJECT_BYTES as u64 {
            return Err(NotesObjectError::Unavailable);
        }

        let expected_size = metadata.st_size as usize;
        let mut file = File::from(fd);
        let mut bytes = Vec::new();
        bytes
            .try_reserve(expected_size)
            .map_err(|_| NotesObjectError::Unavailable)?;
        let mut chunk = [0_u8; 8192];
        loop {
            check_deadline(deadline)?;
            let count = file
                .read(&mut chunk)
                .map_err(|_| NotesObjectError::Unavailable)?;
            check_deadline(deadline)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_PROFILE_OBJECT_BYTES {
                return Err(NotesObjectError::Unavailable);
            }
            bytes
                .try_reserve(count)
                .map_err(|_| NotesObjectError::Unavailable)?;
            bytes.extend_from_slice(&chunk[..count]);
        }
        check_deadline(deadline)?;
        if bytes.len() != expected_size {
            return Err(NotesObjectError::Unavailable);
        }
        Ok(bytes)
    }

    fn write_object_inner(
        &self,
        data: &[u8],
        deadline: Instant,
        sync: impl FnOnce(&OwnedFd, Instant) -> Result<(), NotesObjectError>,
    ) -> Result<(), NotesObjectError> {
        if data.len() > MAX_PROFILE_OBJECT_BYTES {
            return Err(NotesObjectError::Unavailable);
        }
        check_deadline(deadline)?;
        let name = object_name(self.object_id);
        ensure_regular_destination(self.directory, &name, deadline)?;
        let (temporary_name, fd) = create_temp_file(self.directory, deadline)?;
        let mut renamed = false;

        let result = (|| {
            let mut file = File::from(fd);
            let mut offset = 0;
            while offset < data.len() {
                check_deadline(deadline)?;
                let count = file
                    .write(&data[offset..])
                    .map_err(|_| NotesObjectError::Unavailable)?;
                check_deadline(deadline)?;
                if count == 0 {
                    return Err(NotesObjectError::Unavailable);
                }
                offset += count;
            }

            check_deadline(deadline)?;
            file.sync_all().map_err(|_| NotesObjectError::Unavailable)?;
            check_deadline(deadline)?;
            drop(file);

            ensure_regular_destination(self.directory, &name, deadline)?;
            check_deadline(deadline)?;
            renameat(
                self.directory,
                temporary_name.as_str(),
                self.directory,
                name.as_str(),
            )
            .map_err(map_errno)?;
            renamed = true;

            sync(self.directory, deadline)
        })();

        if result.is_err() && !renamed {
            let _ = unlinkat(self.directory, temporary_name.as_str(), AtFlags::empty());
        }
        result
    }
}

fn sync_directory(directory: &OwnedFd, deadline: Instant) -> Result<(), NotesObjectError> {
    check_deadline(deadline)?;
    fsync(directory).map_err(|_| NotesObjectError::Unavailable)?;
    check_deadline(deadline)
}

const INITIALIZED_INDEX: &str = ".notes-index-initialized-v1";

/// Run only inside the authorized C1 root effect for this exact directory.
/// The marker is durable before the factory exposes the store to Notes.
fn initialize_notes_index(
    directory: &OwnedFd,
    object: ObjectId,
    allow_initial_index: bool,
    empty_index: &[u8],
    budget: Duration,
    validate: impl FnOnce() -> Result<(), NotesObjectError>,
) -> Result<(), NotesObjectError> {
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
            return Err(NotesObjectError::Unavailable);
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
) -> Result<bool, NotesObjectError> {
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
        return Err(NotesObjectError::Denied);
    }
    let mut marker_bytes = [0u8; 16];
    (&marker)
        .read_exact(&mut marker_bytes)
        .map_err(|_| NotesObjectError::Unavailable)?;
    check_deadline(deadline)?;
    if marker_bytes != object_name.as_bytes() {
        return Err(NotesObjectError::Denied);
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
) -> Result<(), NotesObjectError> {
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
        .map_err(|_| NotesObjectError::Unavailable)?;
    check_deadline(deadline)?;
    file.sync_all().map_err(|_| NotesObjectError::Unavailable)?;
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

fn make_deadline(budget: Duration) -> Result<Instant, NotesObjectError> {
    if budget.is_zero() {
        return Err(NotesObjectError::Unavailable);
    }
    Instant::now()
        .checked_add(budget)
        .ok_or(NotesObjectError::Unavailable)
}

fn check_deadline(deadline: Instant) -> Result<(), NotesObjectError> {
    if Instant::now() < deadline {
        Ok(())
    } else {
        Err(NotesObjectError::Unavailable)
    }
}

fn map_errno(error: Errno) -> NotesObjectError {
    match error {
        Errno::ACCESS | Errno::PERM | Errno::LOOP | Errno::NOTDIR => NotesObjectError::Denied,
        _ => NotesObjectError::Unavailable,
    }
}

fn ensure_regular_destination(
    directory: &OwnedFd,
    name: &str,
    deadline: Instant,
) -> Result<(), NotesObjectError> {
    check_deadline(deadline)?;
    match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode).is_file() => {
            check_deadline(deadline)
        }
        Ok(_) => Err(NotesObjectError::Denied),
        Err(Errno::NOENT) => check_deadline(deadline),
        Err(error) => Err(map_errno(error)),
    }
}

fn create_temp_file(
    directory: &OwnedFd,
    deadline: Instant,
) -> Result<(String, OwnedFd), NotesObjectError> {
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
    Err(NotesObjectError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("nagi-notes-object-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn open(&self) -> OwnedFd {
            File::open(&self.0).unwrap().into()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn notes_index_reopen_preserves_the_reserved_object_and_rejects_absence() {
        let directory = TestDirectory::new();
        let fd = directory.open();
        let id = ObjectId(0xfeed);
        let budget = Duration::from_secs(3);
        let mut object = NotesObject::new(&fd, id, false, budget);
        assert_eq!(
            object.initialize(b"empty", |_| panic!("missing index validated")),
            Err(NotesObjectError::Unavailable),
        );
        object.write(b"persisted").unwrap();
        fs::write(directory.0.join(INITIALIZED_INDEX), object_name(id)).unwrap();

        let reopened_fd = directory.open();
        let mut reopened = NotesObject::new(&reopened_fd, id, false, budget);
        reopened
            .initialize(b"must not be written", |bytes| {
                assert_eq!(bytes, b"persisted");
                Ok(())
            })
            .unwrap();
        assert_eq!(reopened.read().unwrap(), b"persisted");
    }

    #[test]
    fn failed_directory_sync_leaves_notes_bytes_available_for_exact_readback() {
        let directory = TestDirectory::new();
        let fd = directory.open();
        let budget = Duration::from_secs(3);
        let object = NotesObject::new(&fd, ObjectId(0xfeed), true, budget);
        assert_eq!(
            object.write_object_inner(
                b"published",
                make_deadline(budget).unwrap(),
                |fd, deadline| {
                    let path_only = openat(
                        fd,
                        ".",
                        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .unwrap();
                    sync_directory(&path_only, deadline)
                }
            ),
            Err(NotesObjectError::Unavailable),
        );
        assert_eq!(object.read().unwrap(), b"published");
        object.confirm_directory_sync().unwrap();
    }

    #[test]
    fn notes_object_rejects_symlinks_and_expired_io() {
        let directory = TestDirectory::new();
        let fd = directory.open();
        let id = ObjectId(0xfeed);
        fs::write(directory.0.join("other"), b"other bytes").unwrap();
        symlink("other", directory.0.join(object_name(id))).unwrap();
        let mut object = NotesObject::new(&fd, id, false, Duration::from_secs(3));
        assert_eq!(object.read(), Err(NotesObjectError::Denied));
        assert_eq!(
            object.write(b"must not replace"),
            Err(NotesObjectError::Denied)
        );
        assert_eq!(fs::read(directory.0.join("other")).unwrap(), b"other bytes");
        assert_eq!(
            NotesObject::new(&fd, id, false, Duration::ZERO).read(),
            Err(NotesObjectError::Unavailable),
        );
        let expired = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert_eq!(
            object.read_object_inner(expired),
            Err(NotesObjectError::Unavailable)
        );
    }
}
