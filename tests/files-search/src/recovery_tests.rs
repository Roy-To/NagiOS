use crate::vfs_recovery::{inspect, recover_existing, Inspection};
use libnagi::storage::{BlockDevice, ReadOnlyBlockDevice, StorageError, Vfs, SECTOR_SIZE};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Disk(Arc<Mutex<State>>);

struct State {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
    fail_after: Option<usize>,
    writes_blocked: bool,
}

impl Disk {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Mutex::new(State {
            bytes,
            writes: 0,
            flushes: 0,
            fail_after: None,
            writes_blocked: false,
        })))
    }

    fn snapshot(&self) -> (Vec<u8>, usize, usize) {
        let state = self.0.lock().unwrap();
        (state.bytes.clone(), state.writes, state.flushes)
    }

    fn inject(&self, after: usize) {
        let mut state = self.0.lock().unwrap();
        state.fail_after = Some(after);
        state.writes_blocked = false;
    }

    fn reconnect(&self) {
        let mut state = self.0.lock().unwrap();
        state.fail_after = None;
        state.writes_blocked = false;
    }
}

impl ReadOnlyBlockDevice for Disk {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let state = self.0.lock().unwrap();
        let offset = sector as usize * SECTOR_SIZE;
        destination.copy_from_slice(
            state
                .bytes
                .get(offset..offset + SECTOR_SIZE)
                .ok_or(StorageError::Block)?,
        );
        Ok(())
    }
}

impl BlockDevice for Disk {
    fn write_sector(
        &mut self,
        sector: u64,
        source: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let mut state = self.0.lock().unwrap();
        if state.writes_blocked || state.fail_after == Some(0) {
            state.writes_blocked = true;
            return Err(StorageError::Block);
        }
        if let Some(after) = state.fail_after.as_mut() {
            *after -= 1;
        }
        let offset = sector as usize * SECTOR_SIZE;
        state
            .bytes
            .get_mut(offset..offset + SECTOR_SIZE)
            .ok_or(StorageError::Block)?
            .copy_from_slice(source);
        state.writes += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        let mut state = self.0.lock().unwrap();
        if state.writes_blocked {
            return Err(StorageError::Block);
        }
        state.flushes += 1;
        Ok(())
    }
}

fn baseline() -> Disk {
    let disk = Disk::from_bytes(vec![0; 8 * 1024 * 1024]);
    let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
    let handle = volume.create(b"kept").unwrap();
    volume.write(handle, b"retained neighbor").unwrap();
    disk
}

fn pending(rename: bool) -> Disk {
    let bytes = baseline().snapshot().0;
    // Find a valid interrupted transaction through public operations, without
    // constructing headers or relying on a specific internal write number.
    for after in 0..64 {
        let disk = Disk::from_bytes(bytes.clone());
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        disk.inject(after);
        let result = if rename {
            volume.rename(b"kept", b"renamed")
        } else {
            volume.create(b"interrupted")
        };
        disk.reconnect();
        if result.is_err() && inspect(&mut disk.clone()) == Ok(Inspection::RecoveryRequired) {
            return disk;
        }
    }
    panic!("fixture did not create a valid pending undo transaction");
}

#[test]
fn clean_diagnosis_and_explicit_recovery_leave_bytes_and_counters_unchanged() {
    let disk = baseline();
    let before = disk.snapshot();
    assert!(matches!(
        inspect(&mut disk.clone()),
        Ok(Inspection::Clean(_))
    ));
    let (_, report) = recover_existing(disk.clone()).unwrap();
    assert_eq!(report.regular_files, 1);
    assert_eq!(disk.snapshot(), before);
}

#[test]
fn pending_create_diagnosis_never_writes_then_explicit_recovery_restores_data() {
    check_pending(false);
}

#[test]
fn pending_rename_diagnosis_never_writes_then_explicit_recovery_restores_data() {
    check_pending(true);
}

fn check_pending(rename: bool) {
    let disk = pending(rename);
    let before = disk.snapshot();
    assert_eq!(inspect(&mut disk.clone()), Ok(Inspection::RecoveryRequired));
    assert_eq!(disk.snapshot(), before);
    let (mut volume, report) = recover_existing(disk.clone()).unwrap();
    assert_eq!(report.regular_files, 1);
    let handle = volume.open(b"kept").unwrap();
    let mut bytes = [0; 32];
    let length = volume.read(handle, &mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"retained neighbor");
    assert_eq!(
        volume.open(if rename { b"renamed" } else { b"interrupted" }),
        Err(StorageError::NotFound)
    );
    assert!(matches!(
        inspect(&mut disk.clone()),
        Ok(Inspection::Clean(_))
    ));
    let after = disk.snapshot();
    assert!(after.1 > before.1 && after.2 > before.2);
}

#[test]
fn corrupt_unknown_and_unformatted_input_cannot_be_repaired_or_formatted() {
    let mut corrupt = pending(false).snapshot().0;
    corrupt[480] ^= 1; // The validated pending transaction checksum is invalid.
    let mut unknown = baseline().snapshot().0;
    unknown[0] = 0xff;
    for bytes in [corrupt, unknown, vec![0; 8 * 1024 * 1024]] {
        let disk = Disk::from_bytes(bytes);
        let before = disk.snapshot();
        assert_eq!(inspect(&mut disk.clone()), Err(StorageError::Corrupt));
        assert_eq!(
            recover_existing(disk.clone()).err(),
            Some(StorageError::Corrupt)
        );
        assert_eq!(disk.snapshot(), before);
    }
}

#[test]
fn failed_explicit_recovery_keeps_pending_state_and_can_be_retried() {
    let disk = pending(false);
    disk.inject(0);
    assert_eq!(
        recover_existing(disk.clone()).err(),
        Some(StorageError::Block)
    );
    disk.reconnect();
    assert_eq!(inspect(&mut disk.clone()), Ok(Inspection::RecoveryRequired));
    recover_existing(disk.clone()).unwrap();
    assert!(matches!(
        inspect(&mut disk.clone()),
        Ok(Inspection::Clean(_))
    ));
}

#[test]
fn restored_journal_does_not_hide_unrelated_integrity_failure() {
    let disk = pending(true);
    let clean = baseline();
    let mut clean_volume = Vfs::mount_existing(clean).unwrap();
    let inode = clean_volume.open(b"kept").unwrap().inode() as usize;
    {
        let mut state = disk.0.lock().unwrap();
        // Decode the fixture's ext2-like group descriptor. This inode is not
        // repaired by the pending rename journal; its invalid mode must fail
        // the full integrity check after restoring the parent directory.
        let table = u32::from_le_bytes(state.bytes[2 * 1024 + 8..2 * 1024 + 12].try_into().unwrap())
            as usize;
        let offset = table * 1024 + (inode - 1) * 128;
        state.bytes[offset..offset + 2].fill(0);
    }
    assert_eq!(inspect(&mut disk.clone()), Ok(Inspection::RecoveryRequired));
    assert_eq!(
        recover_existing(disk.clone()).err(),
        Some(StorageError::Corrupt)
    );
    assert_eq!(inspect(&mut disk.clone()), Err(StorageError::Corrupt));
}
