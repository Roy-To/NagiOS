//! Fixed-purpose adapter to the Nagi POSIX browser-storage service.

use crate::persistence::{BrowserStorage, StorageError, StorageRecord, StorageWrite};
use crate::storage_bundle::{self, MAX_STORAGE_BUNDLE_BYTES};

#[cfg(target_os = "nagi")]
unsafe extern "C" {
    fn nagi_posix_browser_storage_read(output: *mut u8, capacity: usize) -> isize;
    fn nagi_posix_browser_storage_write(bytes: *const u8, length: usize) -> i32;
}

#[cfg(all(test, not(target_os = "nagi")))]
mod test_service {
    use std::sync::Mutex;

    static SNAPSHOT: Mutex<Option<Vec<u8>>> = Mutex::new(None);

    pub fn read(output: &mut [u8]) -> isize {
        let snapshot = SNAPSHOT.lock().unwrap();
        let Some(snapshot) = snapshot.as_ref() else {
            return 0;
        };
        if snapshot.len() > output.len() {
            return -1;
        }
        output[..snapshot.len()].copy_from_slice(snapshot);
        snapshot.len() as isize
    }

    pub fn write(bytes: &[u8]) -> i32 {
        *SNAPSHOT.lock().unwrap() = Some(bytes.to_vec());
        0
    }

    pub fn clear() {
        *SNAPSHOT.lock().unwrap() = None;
    }
}

fn read_snapshot(output: &mut [u8]) -> isize {
    #[cfg(target_os = "nagi")]
    {
        // SAFETY: the service writes only within this fixed-size writable buffer.
        unsafe { nagi_posix_browser_storage_read(output.as_mut_ptr(), output.len()) }
    }
    #[cfg(all(test, not(target_os = "nagi")))]
    {
        test_service::read(output)
    }
    #[cfg(all(not(target_os = "nagi"), not(test)))]
    {
        let _ = output;
        -1
    }
}

fn write_snapshot(bytes: &[u8]) -> i32 {
    #[cfg(target_os = "nagi")]
    {
        // SAFETY: the immutable slice remains valid for the duration of the call.
        unsafe { nagi_posix_browser_storage_write(bytes.as_ptr(), bytes.len()) }
    }
    #[cfg(all(test, not(target_os = "nagi")))]
    {
        test_service::write(bytes)
    }
    #[cfg(all(not(target_os = "nagi"), not(test)))]
    {
        let _ = bytes;
        -1
    }
}

pub(crate) struct GuestBrowserStorage;

impl BrowserStorage for GuestBrowserStorage {
    fn read_record(&mut self, record: StorageRecord) -> Result<Option<Vec<u8>>, StorageError> {
        let mut bundle = vec![0_u8; MAX_STORAGE_BUNDLE_BYTES];
        let length = read_snapshot(&mut bundle);
        if length == -2 {
            return Err(StorageError::Capacity);
        }
        if length == -3 {
            return Err(StorageError::Unavailable);
        }
        if length < 0 {
            return Err(StorageError::Io);
        }
        if length == 0 {
            return Ok(None);
        }
        let length = usize::try_from(length).map_err(|_| StorageError::Io)?;
        let writes = storage_bundle::decode(bundle.get(..length).ok_or(StorageError::Io)?)?;
        Ok(writes
            .into_iter()
            .find(|write| write.record == record)
            .map(|write| write.bytes))
    }

    fn write_batch(&mut self, writes: &[StorageWrite]) -> Result<(), StorageError> {
        let bundle = storage_bundle::encode(writes)?;
        let result = write_snapshot(&bundle);
        match result {
            0 => Ok(()),
            -2 => Err(StorageError::Capacity),
            -3 => Err(StorageError::Unavailable),
            _ => Err(StorageError::Io),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_adapter_round_trips_records_through_one_atomic_snapshot() {
        test_service::clear();
        let mut storage = GuestBrowserStorage;
        let writes = [
            StorageWrite {
                record: StorageRecord::Session,
                bytes: b"session".to_vec(),
            },
            StorageWrite {
                record: StorageRecord::History,
                bytes: b"history".to_vec(),
            },
            StorageWrite {
                record: StorageRecord::Bookmarks,
                bytes: b"bookmarks".to_vec(),
            },
        ];
        storage.write_batch(&writes).unwrap();
        for write in writes {
            assert_eq!(
                storage.read_record(write.record).unwrap(),
                Some(write.bytes)
            );
        }
    }
}
