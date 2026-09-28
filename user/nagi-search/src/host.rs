//! Host-only reference persistence for development and restart verification.
//! This uses host files solely as a test/reference backend; Nagi target
//! persistence must be provided by a guest storage adapter implementing
//! `SnapshotBackend`.

use alloc::format;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{BackendError, SnapshotBackend};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct HostFileBackend {
    path: PathBuf,
}

impl HostFileBackend {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SnapshotBackend for HostFileBackend {
    fn load_snapshot(&mut self) -> Result<Option<alloc::vec::Vec<u8>>, BackendError> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(BackendError::Io),
        };
        let maximum = crate::store::MAX_SNAPSHOT_BYTES;
        let mut bytes = alloc::vec::Vec::new();
        Read::by_ref(&mut file)
            .take((maximum + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| BackendError::Io)?;
        if bytes.len() > maximum {
            return Err(BackendError::SnapshotTooLarge);
        }
        Ok(Some(bytes))
    }

    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        if snapshot.len() > crate::store::MAX_SNAPSHOT_BYTES {
            return Err(BackendError::SnapshotTooLarge);
        }
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|_| BackendError::Io)?;
        let name = self
            .path
            .file_name()
            .ok_or(BackendError::Io)?
            .to_string_lossy();

        let (temp_path, mut file) = create_unique_temp(parent, &name)?;
        let result = (|| {
            file.write_all(snapshot).map_err(|_| BackendError::Io)?;
            file.sync_all().map_err(|_| BackendError::Io)?;
            drop(file);
            fs::rename(&temp_path, &self.path).map_err(|_| BackendError::Io)?;
            // The rename is the visible commit point. A directory fsync is a
            // best-effort crash-durability enhancement; it must not report a
            // failed mutation after the new snapshot is already installed.
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp_path);
        }
        result
    }
}

fn create_unique_temp(parent: &Path, name: &str) -> Result<(PathBuf, File), BackendError> {
    for _ in 0..64 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{name}.nagi-search-{sequence}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(BackendError::Io),
        }
    }
    Err(BackendError::Io)
}
