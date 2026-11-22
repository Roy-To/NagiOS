//! Task24-only adapter from the verified development Data root to C3's
//! service-scoped durable journal path.

use crate::profile_volume_notes_owner::SelectedDataRoot;
use crate::task24_c3_journal_bootstrap_contract::{
    c3_first_provision_marker_bytes, has_only_known_data_journal_entries,
    is_fresh_data_journal_bootstrap, DATA_BOOTSTRAP_MARKER_BYTES, DATA_BOOTSTRAP_MARKER_NAME,
    DATA_JOURNAL_DIRECTORY, DATA_JOURNAL_FILE,
};
use rustix::fs::{openat, renameat_with, statat, unlinkat, AtFlags, Mode, OFlags, RenameFlags};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const C3_FIRST_PROVISION_SUFFIX: &str = ".first-provision";
const C3_FIRST_PROVISION_STAGING_SUFFIX: &str = ".pending";
const C3_LOCK_SUFFIX: &str = ".lock";

/// Keeps the selected Data journal directory alive for the entire C3 owner
/// lifetime while supplying a path C3 can canonicalize beneath that mount.
pub(super) struct Task24C3DataJournalDirectory {
    _directory: File,
    journal_path: PathBuf,
}

impl Task24C3DataJournalDirectory {
    pub(super) fn open(root: &SelectedDataRoot, service_id: &str) -> io::Result<Self> {
        let directory = root
            .retain_identity_snapshot_directory(Path::new(DATA_JOURNAL_DIRECTORY))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Task24 C3 Data journal directory rejected: {error}"),
                )
            })?;
        let journal_path = PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            directory.as_raw_fd(),
            DATA_JOURNAL_FILE
        ));
        prepare_c3_first_provision_marker(&directory, service_id)?;
        Ok(Self {
            _directory: directory,
            journal_path,
        })
    }

    pub(super) fn journal_path(&self) -> &Path {
        &self.journal_path
    }
}

fn prepare_c3_first_provision_marker(directory: &File, service_id: &str) -> io::Result<()> {
    let names = read_directory_entries(directory)?;
    let c3_marker_name = format!("{DATA_JOURNAL_FILE}{C3_FIRST_PROVISION_SUFFIX}");
    let lock_name = format!("{DATA_JOURNAL_FILE}{C3_LOCK_SUFFIX}");
    if !has_only_known_data_journal_entries(&names, &c3_marker_name) {
        return Err(invalid_marker(
            "Task24 C3 journal directory has an unexpected entry",
        ));
    }
    let Some(bootstrap_marker) = open_private_marker(directory, DATA_BOOTSTRAP_MARKER_NAME)? else {
        // Once the generic first-provision marker is gone, C3 must open its
        // existing journal or consume an already staged C3 marker itself.
        return Ok(());
    };
    if read_bounded(&bootstrap_marker, DATA_BOOTSTRAP_MARKER_BYTES.len())?.as_slice()
        != DATA_BOOTSTRAP_MARKER_BYTES
    {
        return Err(invalid_marker("Task24 C3 Data bootstrap marker mismatch"));
    }

    let c3_marker_bytes = c3_first_provision_marker_bytes(service_id)
        .ok_or_else(|| invalid_marker("verified C3 service ID is not canonical"))?;
    if !is_fresh_data_journal_bootstrap(&names, &c3_marker_name) {
        return Err(invalid_marker(
            "Task24 C3 journal bootstrap directory is not fresh",
        ));
    }
    require_missing(directory, DATA_JOURNAL_FILE)?;
    require_missing(directory, &lock_name)?;

    let published = match open_private_marker(directory, &c3_marker_name)? {
        Some(existing) => existing,
        None => {
            let staging_name = format!("{c3_marker_name}{C3_FIRST_PROVISION_STAGING_SUFFIX}");
            let marker_fd = openat(
                directory,
                staging_name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_raw_mode(0o600),
            )
            .map_err(io::Error::from)?;
            let mut marker = File::from(marker_fd);
            validate_private_file(&marker)?;
            marker.write_all(&c3_marker_bytes)?;
            marker.sync_all()?;
            drop(marker);
            publish_marker_without_replacement(directory, &staging_name, &c3_marker_name)?;
            open_private_marker(directory, &c3_marker_name)?
                .ok_or_else(|| invalid_marker("C3 first-provision marker disappeared"))?
        }
    };

    if read_bounded(&published, c3_marker_bytes.len())?.as_slice() != c3_marker_bytes {
        return Err(invalid_marker(
            "C3 first-provision marker readback mismatch",
        ));
    }

    // Sync the exact read/write marker descriptor, then its name, before
    // consuming the generic one-time bootstrap authorization.
    published.sync_all()?;
    directory.sync_all()?;

    drop(bootstrap_marker);
    unlinkat(directory, DATA_BOOTSTRAP_MARKER_NAME, AtFlags::empty()).map_err(io::Error::from)?;
    directory.sync_all()?;
    Ok(())
}

fn publish_marker_without_replacement(
    directory: &File,
    staging_name: &str,
    marker_name: &str,
) -> io::Result<()> {
    renameat_with(
        directory,
        staging_name,
        directory,
        marker_name,
        RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

fn open_private_marker(directory: &File, name: &str) -> io::Result<Option<File>> {
    match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => {
            let fd = openat(
                directory,
                name,
                // Keep the validated marker descriptor write-capable so the
                // exact bytes read below can be synced on that same inode.
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(io::Error::from)?;
            let file = File::from(fd);
            validate_private_file(&file)?;
            Ok(Some(file))
        }
    }
}

fn require_missing(directory: &File, name: &str) -> io::Result<()> {
    match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Task24 C3 first provision cannot replace existing journal state",
        )),
        Err(error) => Err(error.into()),
    }
}

fn validate_private_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Task24 C3 marker is not a private, singly linked regular file",
        ));
    }
    Ok(())
}

fn read_bounded(file: &File, expected_len: usize) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() != expected_len as u64 {
        return Err(invalid_marker("Task24 C3 marker length mismatch"));
    }
    let mut bytes = Vec::with_capacity(expected_len);
    Read::take(file, expected_len.saturating_add(1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() != expected_len {
        return Err(invalid_marker("Task24 C3 marker readback length mismatch"));
    }
    Ok(bytes)
}

fn invalid_marker(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_directory_entries(directory: &File) -> io::Result<Vec<std::ffi::OsString>> {
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    std::fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{open_private_marker, publish_marker_without_replacement};
    #[cfg(target_os = "linux")]
    use super::{prepare_c3_first_provision_marker, C3_FIRST_PROVISION_SUFFIX};
    use crate::task24_c3_journal_bootstrap_contract::DATA_BOOTSTRAP_MARKER_NAME;
    #[cfg(target_os = "linux")]
    use crate::task24_c3_journal_bootstrap_contract::{
        c3_first_provision_marker_bytes, DATA_BOOTSTRAP_MARKER_BYTES, DATA_JOURNAL_FILE,
    };
    use std::ffi::CString;
    use std::fs::{self, File};
    #[cfg(target_os = "linux")]
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);
    const FIFO_TEST_DIRECTORY_ENV: &str = "NAGI_TASK24_C3_FIFO_TEST_DIRECTORY";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-task24-c3-marker-{}-{}",
                std::process::id(),
                NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).expect("create private test directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("set private test directory mode");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fifo_bootstrap_markers_are_rejected_without_blocking() {
        if let Some(directory) = std::env::var_os(FIFO_TEST_DIRECTORY_ENV) {
            run_fifo_marker_checks(Path::new(&directory));
            return;
        }

        let directory = TestDirectory::new();
        let mut child = Command::new(std::env::current_exe().expect("test executable path"))
            .arg("fifo_bootstrap_markers_are_rejected_without_blocking")
            .arg("--nocapture")
            .env(FIFO_TEST_DIRECTORY_ENV, directory.path())
            .spawn()
            .expect("spawn FIFO marker regression child");
        wait_for_fifo_test_child(&mut child);
    }

    #[test]
    fn c3_first_provision_marker_publish_is_atomic_and_no_replace() {
        let directory = TestDirectory::new();
        let marker_directory = File::open(directory.path()).expect("open marker directory");
        let staging_name = "system-service-slot.v1.first-provision.pending";
        let marker_name = "system-service-slot.v1.first-provision";
        let staging_path = directory.path().join(staging_name);
        let marker_path = directory.path().join(marker_name);
        let marker_bytes = b"complete marker bytes\n";

        fs::write(&staging_path, marker_bytes).expect("write complete staged marker");
        File::open(&staging_path)
            .expect("open staged marker")
            .sync_all()
            .expect("sync staged marker");
        publish_marker_without_replacement(&marker_directory, staging_name, marker_name)
            .expect("atomically publish staged marker");
        assert_eq!(fs::read(&marker_path).unwrap(), marker_bytes);
        assert!(!staging_path.exists(), "published staging name remains");

        fs::write(&staging_path, b"replacement bytes\n").expect("create second staged marker");
        let error =
            publish_marker_without_replacement(&marker_directory, staging_name, marker_name)
                .expect_err("existing marker must not be replaced");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&marker_path).unwrap(), marker_bytes);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exact_existing_c3_marker_is_synced_before_bootstrap_consumption() {
        let service_id = "org.nagi.dev.task24-identity";
        let directory = TestDirectory::new();
        let marker_directory = File::open(directory.path()).expect("open marker directory");
        let bootstrap_path = directory.path().join(DATA_BOOTSTRAP_MARKER_NAME);
        let c3_marker_name = format!("{DATA_JOURNAL_FILE}{C3_FIRST_PROVISION_SUFFIX}");
        let c3_marker_path = directory.path().join(&c3_marker_name);
        let c3_marker_bytes = c3_first_provision_marker_bytes(service_id).unwrap();

        let mut bootstrap = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&bootstrap_path)
            .expect("create generic bootstrap marker");
        bootstrap
            .write_all(DATA_BOOTSTRAP_MARKER_BYTES)
            .expect("write generic bootstrap marker");
        bootstrap.sync_all().expect("sync generic bootstrap marker");
        drop(bootstrap);

        let mut c3_marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&c3_marker_path)
            .expect("create exact C3 marker");
        c3_marker
            .write_all(&c3_marker_bytes)
            .expect("write exact C3 marker");
        c3_marker.sync_all().expect("sync exact C3 marker");
        drop(c3_marker);
        marker_directory
            .sync_all()
            .expect("sync pre-existing marker entries");

        prepare_c3_first_provision_marker(&marker_directory, service_id)
            .expect("reuse and sync exact existing C3 marker");

        assert!(
            !bootstrap_path.exists(),
            "bootstrap marker was not consumed"
        );
        assert_eq!(fs::read(c3_marker_path).unwrap(), c3_marker_bytes);
    }

    fn run_fifo_marker_checks(directory: &Path) {
        let names = [
            DATA_BOOTSTRAP_MARKER_NAME,
            "system-service-slot.v1.first-provision",
        ];
        for name in names {
            let fifo_path = directory.join(name);
            let fifo_cstring =
                CString::new(fifo_path.as_os_str().as_bytes()).expect("FIFO path has no NUL bytes");
            assert_eq!(
                unsafe { libc::mkfifo(fifo_cstring.as_ptr(), 0o600) },
                0,
                "create FIFO marker",
            );
            let marker_directory = File::open(directory).expect("open test directory");
            let error = open_private_marker(&marker_directory, name)
                .expect_err("FIFO bootstrap marker must be rejected");
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            drop(marker_directory);
            fs::remove_file(fifo_path).expect("remove FIFO marker");
        }
    }

    fn wait_for_fifo_test_child(child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().expect("poll FIFO marker child") {
                assert!(status.success(), "FIFO marker child failed: {status}");
                return;
            }
            if Instant::now() >= deadline {
                // A broken blocking open is isolated to this child. Kill and
                // poll for its reap without allowing cleanup to hang the
                // parent test process.
                let kill_result = child.kill();
                let reap_deadline = Instant::now() + Duration::from_secs(1);
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => panic!(
                            "FIFO marker child exceeded 5-second deadline; kill={kill_result:?}, child reaped with {status}"
                        ),
                        Ok(None) if Instant::now() < reap_deadline => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Ok(None) => panic!(
                            "FIFO marker child exceeded 5-second deadline; kill={kill_result:?}, child reap unconfirmed after 1-second deadline"
                        ),
                        Err(error) => panic!(
                            "FIFO marker child exceeded 5-second deadline; kill={kill_result:?}, child reap failed: {error}"
                        ),
                    }
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
