//! Linux/POSIX durable package blobs and digest-index adapter.
//!
//! The caller supplies an already-open directory owned by trusted boot. This
//! module chooses names only relative to that directory; it does not choose a
//! mount, filesystem, or product path.

use crate::package_trust::{
    durable_storage_sealed, InstalledPackageBlobCommit, InstalledPackageBlobError,
    InstalledPackageBlobReadback, InstalledPackageBlobStore, InstalledPackageDigestIndex,
    InstalledPackageDigestIndexCommit, InstalledPackageDigestIndexError,
    InstalledPackageDigestIndexReadback, InstalledPackageDigestRecord,
    InstalledPackageDigestRecordError,
};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int};
use core::sync::atomic::{AtomicU64, Ordering};
use nagi_model::AppId;
use nagi_package::MAX_SLOTS;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;

const INDEX_NAME: &str = "package-digest-index.v1";
const LOCK_NAME: &str = ".package-store.lock";
const BLOB_PREFIX: &str = "package-";
const BLOB_SUFFIX: &str = ".xapp";
const INDEX_MAGIC: &[u8; 8] = b"NGPKIDX1";
const INDEX_HEADER_BYTES: usize = 12;
const INDEX_RECORD_BYTES: usize = 88;
const INDEX_CHECKSUM_BYTES: usize = 32;
const INDEX_MAX_BYTES: usize =
    INDEX_HEADER_BYTES + MAX_SLOTS * INDEX_RECORD_BYTES + INDEX_CHECKSUM_BYTES;

#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "macos")]
const O_CREAT: c_int = 0x0200;
#[cfg(target_os = "linux")]
const O_EXCL: c_int = 0o200;
#[cfg(target_os = "macos")]
const O_EXCL: c_int = 0x0800;
#[cfg(target_os = "linux")]
const O_CLOEXEC: c_int = 0o2000000;
#[cfg(target_os = "macos")]
const O_CLOEXEC: c_int = 0x01000000;
#[cfg(target_os = "linux")]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: c_int = 0x00000100;
const LOCK_SH: c_int = 1;
const LOCK_EX: c_int = 2;
const LOCK_UN: c_int = 8;

unsafe extern "C" {
    fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, ...) -> c_int;
    fn renameat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
    ) -> c_int;
    fn unlinkat(dirfd: c_int, pathname: *const c_char, flags: c_int) -> c_int;
    fn linkat(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
        flags: c_int,
    ) -> c_int;
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

/// Durable package storage rooted at a directory descriptor supplied by
/// trusted boot. Both the index and immutable blobs are serialized by one
/// advisory cross-process lock and use no-follow relative opens.
pub struct LinuxInstalledPackagePersistence {
    root: File,
    owner_uid: u32,
}

impl LinuxInstalledPackagePersistence {
    /// Adopt a trusted boot-owned directory descriptor. `owner_uid` must be
    /// selected by the boot owner (normally root); this checks that the open
    /// directory has that owner and is not writable by group or others.
    pub fn from_trusted_directory(
        root: File,
        owner_uid: u32,
    ) -> Result<Self, InstalledPackageDigestIndexError> {
        let metadata = root
            .metadata()
            .map_err(|_| InstalledPackageDigestIndexError::Unavailable)?;
        if !metadata.file_type().is_dir()
            || metadata.uid() != owner_uid
            || metadata.mode() & 0o022 != 0
        {
            return Err(InstalledPackageDigestIndexError::Unavailable);
        }
        Ok(Self { root, owner_uid })
    }

    fn lock(&self, mode: c_int) -> io::Result<AdvisoryLock> {
        let name = c_string(LOCK_NAME)?;
        let file = self.open_at(&name, 2 | O_CREAT | O_CLOEXEC | O_NOFOLLOW, 0o600)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file()
            || metadata.uid() != self.owner_uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "package-store lock is not a trusted private regular file",
            ));
        }
        loop {
            // SAFETY: `file` is an open descriptor and `mode` is LOCK_SH/LOCK_EX.
            if unsafe { flock(file.as_raw_fd(), mode) } == 0 {
                return Ok(AdvisoryLock(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    fn open_at(&self, name: &CString, flags: c_int, mode: c_int) -> io::Result<File> {
        // SAFETY: `name` is NUL-terminated and the descriptor is an open directory.
        let fd = unsafe { openat(self.root.as_raw_fd(), name.as_ptr(), flags, mode) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn read_blob_locked(
        &self,
        sha256: &[u8; 32],
        output: &mut [u8],
    ) -> Result<InstalledPackageBlobReadback, InstalledPackageBlobError> {
        let name = c_string(&blob_name(sha256)).map_err(|_| InstalledPackageBlobError::Failed)?;
        let file = self
            .open_at(&name, O_CLOEXEC | O_NOFOLLOW, 0)
            .map_err(map_blob_io_error)?;
        let metadata = file
            .metadata()
            .map_err(|_| InstalledPackageBlobError::Failed)?;
        if !trusted_regular_file(&metadata, self.owner_uid) || metadata.len() > output.len() as u64
        {
            return Err(InstalledPackageBlobError::Failed);
        }
        let count = metadata.len() as usize;
        let mut reader = file.take(count as u64 + 1);
        reader
            .read_exact(&mut output[..count])
            .map_err(|_| InstalledPackageBlobError::Failed)?;
        let mut extra = [0];
        if reader
            .read(&mut extra)
            .map_err(|_| InstalledPackageBlobError::Failed)?
            != 0
            || sha256_bytes(&output[..count]) != *sha256
        {
            return Err(InstalledPackageBlobError::Failed);
        }
        Ok(InstalledPackageBlobReadback::after_durable_read(
            *sha256, count,
        ))
    }

    fn write_temp_and_sync(&self, name: &CString, bytes: &[u8]) -> io::Result<File> {
        let mut file = self.open_at(name, 1 | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, 0o600)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(file)
    }

    fn replace_index_durably(&self, bytes: &[u8]) -> Result<(), InstalledPackageDigestIndexError> {
        let temp_name = temp_name("index");
        let temp = c_string(&temp_name).map_err(|_| InstalledPackageDigestIndexError::Failed)?;
        let final_name =
            c_string(INDEX_NAME).map_err(|_| InstalledPackageDigestIndexError::Failed)?;
        let file = match self.write_temp_and_sync(&temp, bytes) {
            Ok(file) => file,
            Err(_) => {
                let _ = unlink_at(self.root.as_raw_fd(), &temp);
                return Err(InstalledPackageDigestIndexError::Failed);
            }
        };
        drop(file);
        // SAFETY: both names are NUL-terminated single path components and the
        // same trusted directory descriptor is used for source and destination.
        if unsafe {
            renameat(
                self.root.as_raw_fd(),
                temp.as_ptr(),
                self.root.as_raw_fd(),
                final_name.as_ptr(),
            )
        } != 0
        {
            let _ = unlink_at(self.root.as_raw_fd(), &temp);
            return Err(InstalledPackageDigestIndexError::Failed);
        }
        self.root
            .sync_all()
            .map_err(|_| InstalledPackageDigestIndexError::OutcomeUnknown)
    }

    fn insert_blob_durably(
        &self,
        sha256: &[u8; 32],
        bytes: &[u8],
    ) -> Result<(), InstalledPackageBlobError> {
        let _lock = self.lock(LOCK_EX).map_err(map_blob_io_error)?;
        let final_name = blob_name(sha256);
        let temp_name = temp_name("blob");
        let temp = c_string(&temp_name).map_err(|_| InstalledPackageBlobError::Failed)?;
        let destination =
            c_string_from_blob(&final_name).map_err(|_| InstalledPackageBlobError::Failed)?;
        let file = match self.write_temp_and_sync(&temp, bytes) {
            Ok(file) => file,
            Err(_) => {
                let _ = unlink_at(self.root.as_raw_fd(), &temp);
                return Err(InstalledPackageBlobError::Failed);
            }
        };
        drop(file);
        // SAFETY: both names are single components below the same trusted
        // directory, and linkat does not replace an existing digest blob.
        if unsafe {
            linkat(
                self.root.as_raw_fd(),
                temp.as_ptr(),
                self.root.as_raw_fd(),
                destination.as_ptr(),
                0,
            )
        } != 0
        {
            let error = io::Error::last_os_error();
            let _ = unlink_at(self.root.as_raw_fd(), &temp);
            if error.raw_os_error() == Some(17) {
                let mut existing = alloc::vec![0; bytes.len()];
                let readback = self.read_blob_locked(sha256, &mut existing)?;
                if !readback.matches(sha256, bytes.len()) || existing != bytes {
                    return Err(InstalledPackageBlobError::Failed);
                }
                return Ok(());
            }
            return Err(InstalledPackageBlobError::Failed);
        }
        let _ = unlink_at(self.root.as_raw_fd(), &temp);
        self.root
            .sync_all()
            .map_err(|_| InstalledPackageBlobError::OutcomeUnknown)
    }
}

impl InstalledPackageDigestIndex for LinuxInstalledPackagePersistence {
    fn replace_snapshot(
        &mut self,
        records: &[InstalledPackageDigestRecord],
    ) -> Result<InstalledPackageDigestIndexCommit, InstalledPackageDigestIndexError> {
        let _lock = self
            .lock(LOCK_EX)
            .map_err(|_| InstalledPackageDigestIndexError::Unavailable)?;
        validate_records(records)?;
        let encoded = encode_snapshot(records)?;
        self.replace_index_durably(&encoded)?;
        Ok(InstalledPackageDigestIndexCommit::after_durable_replace(
            records,
        ))
    }

    fn read_snapshot(
        &self,
        output: &mut [Option<InstalledPackageDigestRecord>],
    ) -> Result<InstalledPackageDigestIndexReadback, InstalledPackageDigestIndexError> {
        let _lock = self
            .lock(LOCK_SH)
            .map_err(|_| InstalledPackageDigestIndexError::Unavailable)?;
        let name = c_string(INDEX_NAME).map_err(|_| InstalledPackageDigestIndexError::Failed)?;
        let file = self
            .open_at(&name, O_CLOEXEC | O_NOFOLLOW, 0)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    InstalledPackageDigestIndexError::Unavailable
                } else {
                    InstalledPackageDigestIndexError::Failed
                }
            })?;
        let metadata = file
            .metadata()
            .map_err(|_| InstalledPackageDigestIndexError::Failed)?;
        if !trusted_regular_file(&metadata, self.owner_uid)
            || metadata.len() > INDEX_MAX_BYTES as u64
        {
            return Err(InstalledPackageDigestIndexError::Failed);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take((INDEX_MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| InstalledPackageDigestIndexError::Failed)?;
        let records = decode_snapshot(&bytes)?;
        if output.len() < records.len() {
            return Err(InstalledPackageDigestIndexError::Failed);
        }
        output.fill(None);
        for (slot, record) in output.iter_mut().zip(records.iter().copied()) {
            *slot = Some(record);
        }
        Ok(InstalledPackageDigestIndexReadback::after_durable_read(
            &records,
        ))
    }
}

impl durable_storage_sealed::DigestIndex for LinuxInstalledPackagePersistence {}

impl InstalledPackageBlobStore for LinuxInstalledPackagePersistence {
    fn insert_immutable(
        &mut self,
        sha256: &[u8; 32],
        bytes: &[u8],
    ) -> Result<InstalledPackageBlobCommit, InstalledPackageBlobError> {
        if bytes.is_empty() || sha256_bytes(bytes) != *sha256 {
            return Err(InstalledPackageBlobError::Failed);
        }
        self.insert_blob_durably(sha256, bytes)?;
        Ok(InstalledPackageBlobCommit::after_durable_insert(
            *sha256,
            bytes.len(),
        ))
    }

    fn read_immutable(
        &self,
        sha256: &[u8; 32],
        output: &mut [u8],
    ) -> Result<InstalledPackageBlobReadback, InstalledPackageBlobError> {
        let _lock = self.lock(LOCK_SH).map_err(map_blob_io_error)?;
        self.read_blob_locked(sha256, output)
    }
}

impl durable_storage_sealed::PackageBlobs for LinuxInstalledPackagePersistence {}

struct AdvisoryLock(File);

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        // SAFETY: the file remains open until the guard is dropped.
        let _ = unsafe { flock(self.0.as_raw_fd(), LOCK_UN) };
    }
}

fn validate_records(
    records: &[InstalledPackageDigestRecord],
) -> Result<(), InstalledPackageDigestIndexError> {
    if records.len() > MAX_SLOTS {
        return Err(InstalledPackageDigestIndexError::Failed);
    }
    let mut previous_app_id = None;
    for (index, record) in records.iter().enumerate() {
        if previous_app_id.is_some_and(|previous| previous >= record.app_id())
            || records[..index]
                .iter()
                .any(|earlier| earlier.generation() == record.generation())
        {
            return Err(InstalledPackageDigestIndexError::Failed);
        }
        InstalledPackageDigestRecord::from_parts(
            record.app_id(),
            record.generation(),
            *record.signed_package_sha256(),
            *record.executable_sha256(),
            record.signed_package_bytes_len(),
            record.executable_bytes_len(),
        )
        .map_err(map_record_error)?;
        previous_app_id = Some(record.app_id());
    }
    Ok(())
}

fn encode_snapshot(
    records: &[InstalledPackageDigestRecord],
) -> Result<Vec<u8>, InstalledPackageDigestIndexError> {
    let mut bytes =
        Vec::with_capacity(INDEX_HEADER_BYTES + records.len() * INDEX_RECORD_BYTES + 32);
    bytes.extend_from_slice(INDEX_MAGIC);
    bytes.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for record in records {
        bytes.extend_from_slice(&record.app_id().0.to_le_bytes());
        bytes.extend_from_slice(&record.generation().to_le_bytes());
        bytes.extend_from_slice(record.signed_package_sha256());
        bytes.extend_from_slice(record.executable_sha256());
        bytes.extend_from_slice(&record.signed_package_bytes_len().to_le_bytes());
        bytes.extend_from_slice(&record.executable_bytes_len().to_le_bytes());
    }
    let checksum = sha256_bytes(&bytes);
    bytes.extend_from_slice(&checksum);
    if bytes.len() > INDEX_MAX_BYTES {
        return Err(InstalledPackageDigestIndexError::Failed);
    }
    Ok(bytes)
}

fn decode_snapshot(
    bytes: &[u8],
) -> Result<Vec<InstalledPackageDigestRecord>, InstalledPackageDigestIndexError> {
    if bytes.len() < INDEX_HEADER_BYTES + INDEX_CHECKSUM_BYTES
        || bytes.get(..8) != Some(INDEX_MAGIC)
    {
        return Err(InstalledPackageDigestIndexError::Failed);
    }
    let count = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| InstalledPackageDigestIndexError::Failed)?,
    ) as usize;
    let expected_len = INDEX_HEADER_BYTES
        .checked_add(
            count
                .checked_mul(INDEX_RECORD_BYTES)
                .ok_or(InstalledPackageDigestIndexError::Failed)?,
        )
        .and_then(|length| length.checked_add(INDEX_CHECKSUM_BYTES))
        .ok_or(InstalledPackageDigestIndexError::Failed)?;
    if count > MAX_SLOTS || bytes.len() != expected_len || bytes.len() > INDEX_MAX_BYTES {
        return Err(InstalledPackageDigestIndexError::Failed);
    }
    let checksum_start = bytes.len() - INDEX_CHECKSUM_BYTES;
    if sha256_bytes(&bytes[..checksum_start]) != bytes[checksum_start..] {
        return Err(InstalledPackageDigestIndexError::Failed);
    }
    let mut records = Vec::with_capacity(count);
    let mut offset = INDEX_HEADER_BYTES;
    for _ in 0..count {
        let app_id = AppId(read_u64(bytes, &mut offset)?);
        let generation = read_u64(bytes, &mut offset)?;
        let signed_package_sha256 = read_array::<32>(bytes, &mut offset)?;
        let executable_sha256 = read_array::<32>(bytes, &mut offset)?;
        let signed_package_bytes_len = read_u32(bytes, &mut offset)?;
        let executable_bytes_len = read_u32(bytes, &mut offset)?;
        records.push(
            InstalledPackageDigestRecord::from_parts(
                app_id,
                generation,
                signed_package_sha256,
                executable_sha256,
                signed_package_bytes_len,
                executable_bytes_len,
            )
            .map_err(map_record_error)?,
        );
    }
    validate_records(&records)?;
    Ok(records)
}

fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, InstalledPackageDigestIndexError> {
    let end = offset
        .checked_add(8)
        .ok_or(InstalledPackageDigestIndexError::Failed)?;
    let value = u64::from_le_bytes(
        bytes
            .get(*offset..end)
            .ok_or(InstalledPackageDigestIndexError::Failed)?
            .try_into()
            .map_err(|_| InstalledPackageDigestIndexError::Failed)?,
    );
    *offset = end;
    Ok(value)
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, InstalledPackageDigestIndexError> {
    let end = offset
        .checked_add(4)
        .ok_or(InstalledPackageDigestIndexError::Failed)?;
    let value = u32::from_le_bytes(
        bytes
            .get(*offset..end)
            .ok_or(InstalledPackageDigestIndexError::Failed)?
            .try_into()
            .map_err(|_| InstalledPackageDigestIndexError::Failed)?,
    );
    *offset = end;
    Ok(value)
}

fn read_array<const N: usize>(
    bytes: &[u8],
    offset: &mut usize,
) -> Result<[u8; N], InstalledPackageDigestIndexError> {
    let end = offset
        .checked_add(N)
        .ok_or(InstalledPackageDigestIndexError::Failed)?;
    let value = bytes
        .get(*offset..end)
        .ok_or(InstalledPackageDigestIndexError::Failed)?
        .try_into()
        .map_err(|_| InstalledPackageDigestIndexError::Failed)?;
    *offset = end;
    Ok(value)
}

fn map_record_error(_: InstalledPackageDigestRecordError) -> InstalledPackageDigestIndexError {
    InstalledPackageDigestIndexError::Failed
}

fn trusted_regular_file(metadata: &std::fs::Metadata, owner_uid: u32) -> bool {
    metadata.file_type().is_file()
        && metadata.uid() == owner_uid
        && metadata.nlink() == 1
        && metadata.mode() & 0o077 == 0
}

fn blob_name(sha256: &[u8; 32]) -> String {
    let mut name = String::with_capacity(BLOB_PREFIX.len() + 64 + BLOB_SUFFIX.len());
    name.push_str(BLOB_PREFIX);
    for byte in sha256 {
        use core::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(BLOB_SUFFIX);
    name
}

fn c_string(value: &str) -> io::Result<CString> {
    CString::new(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in name"))
}

fn c_string_from_blob(value: &str) -> io::Result<CString> {
    c_string(value)
}

fn temp_name(kind: &str) -> String {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    format!(".package-{kind}-{}-{serial}.tmp", std::process::id())
}

fn unlink_at(dirfd: c_int, name: &CString) -> io::Result<()> {
    // SAFETY: `name` is a NUL-terminated single component below an open directory.
    if unsafe { unlinkat(dirfd, name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn map_blob_io_error(error: io::Error) -> InstalledPackageBlobError {
    if error.kind() == io::ErrorKind::NotFound {
        InstalledPackageBlobError::Unavailable
    } else {
        InstalledPackageBlobError::Failed
    }
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::{
        InstalledPackageDigestIndexError, LinuxInstalledPackagePersistence, LOCK_EX, LOCK_NAME,
    };
    use std::format;
    use std::fs::{self, File};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    fn test_path(label: &str) -> std::path::PathBuf {
        let serial = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "nagi-package-store-{label}-{}-{serial}",
            std::process::id()
        ))
    }

    fn assert_untrusted_mode_is_rejected(label: &str, mode: u32) {
        let path = test_path(label);
        fs::create_dir(&path).expect("create package directory");
        let mut permissions = fs::metadata(&path)
            .expect("directory metadata")
            .permissions();
        permissions.set_mode(mode);
        fs::set_permissions(&path, permissions).expect("set package directory mode");
        let root = File::open(&path).expect("open package directory");
        let owner_uid = root.metadata().expect("metadata").uid();

        assert!(matches!(
            LinuxInstalledPackagePersistence::from_trusted_directory(root, owner_uid),
            Err(InstalledPackageDigestIndexError::Unavailable)
        ));
        fs::remove_dir_all(path).expect("remove package directory");
    }

    #[test]
    fn trusted_directory_lock_uses_private_regular_file() {
        let path = test_path("lock-test");
        fs::create_dir(&path).expect("create trusted package directory");
        let root = File::open(&path).expect("open trusted package directory");
        let owner_uid = root.metadata().expect("metadata").uid();
        let persistence = LinuxInstalledPackagePersistence::from_trusted_directory(root, owner_uid)
            .expect("adopt trusted package directory");
        let lock = match persistence.lock(LOCK_EX) {
            Ok(lock) => lock,
            Err(error) => {
                let metadata = fs::metadata(path.join(LOCK_NAME)).expect("inspect lock file");
                panic!(
                    "acquire package-store lock: {error:?}; regular={} owner_matches={} one_link={} private_mode={} permissions={:o}",
                    metadata.file_type().is_file(),
                    metadata.uid() == owner_uid,
                    metadata.nlink() == 1,
                    metadata.mode() & 0o077 == 0,
                    metadata.mode() & 0o777
                );
            }
        };
        drop(lock);
        drop(persistence);
        fs::remove_dir_all(path).expect("remove trusted package directory");
    }

    #[test]
    fn trusted_directory_rejects_unexpected_owner_uid() {
        let path = test_path("wrong-owner");
        fs::create_dir(&path).expect("create package directory");
        let root = File::open(&path).expect("open package directory");
        let actual_uid = root.metadata().expect("metadata").uid();
        let unexpected_uid = if actual_uid == u32::MAX {
            actual_uid - 1
        } else {
            actual_uid + 1
        };

        assert!(matches!(
            LinuxInstalledPackagePersistence::from_trusted_directory(root, unexpected_uid),
            Err(InstalledPackageDigestIndexError::Unavailable)
        ));
        fs::remove_dir_all(path).expect("remove package directory");
    }

    #[test]
    fn trusted_directory_rejects_non_directory_descriptor() {
        let path = test_path("not-directory");
        fs::write(&path, b"regular file").expect("create regular file");
        let root = File::open(&path).expect("open regular file");
        let owner_uid = root.metadata().expect("metadata").uid();

        assert!(matches!(
            LinuxInstalledPackagePersistence::from_trusted_directory(root, owner_uid),
            Err(InstalledPackageDigestIndexError::Unavailable)
        ));
        fs::remove_file(path).expect("remove regular file");
    }

    #[test]
    fn trusted_directory_rejects_group_writable_descriptor() {
        assert_untrusted_mode_is_rejected("group-writable", 0o720);
    }

    #[test]
    fn trusted_directory_rejects_other_writable_descriptor() {
        assert_untrusted_mode_is_rejected("other-writable", 0o702);
    }
}
