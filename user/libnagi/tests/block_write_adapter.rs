#![cfg(target_os = "linux")]

// Exercise the actual adapter and VFS against a synthetic syscall boundary.
// The RAM disk models the kernel's writable-source admission and failures;
// /proc/self/maps observes compiler buffer placement only. This is not guest
// I/O or acceptance evidence, and no host file supplies the disk contents.
#[allow(dead_code)]
#[path = "../src/storage.rs"]
mod storage;

use std::sync::Mutex;
use storage::{BlockDevice, StorageError, SyscallBlockDevice, Vfs, SECTOR_SIZE};

const CAPABILITY: u64 = 42;
const DISK_SIZE: usize = 8 * 1024 * 1024;
static SERIAL: Mutex<()> = Mutex::new(());
static DISK: Mutex<Option<Disk>> = Mutex::new(None);

#[derive(Debug)]
struct Attempt {
    capability: u64,
    sector: u64,
    address: usize,
    writable: bool,
}

struct Disk {
    bytes: Vec<u8>,
    flush_supported: bool,
    fail_after: Option<usize>,
    writes: usize,
    flushes: usize,
    attempts: Vec<Attempt>,
}

fn setup(bytes: Vec<u8>, flush_supported: bool) {
    *DISK.lock().unwrap() = Some(Disk {
        bytes,
        flush_supported,
        fail_after: None,
        writes: 0,
        flushes: 0,
        attempts: Vec::new(),
    });
}

fn writable_range(address: usize, length: usize) -> bool {
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if length == 0 {
        return false;
    }
    std::fs::read_to_string("/proc/self/maps")
        .unwrap()
        .lines()
        .any(|row| {
            let mut fields = row.split_whitespace();
            let (start, limit) = fields.next().unwrap().split_once('-').unwrap();
            let flags = fields.next().unwrap().as_bytes();
            let start = usize::from_str_radix(start, 16).unwrap();
            let limit = usize::from_str_radix(limit, 16).unwrap();
            address >= start && end <= limit && flags[1] == b'w'
        })
}

fn sector_range(sector: u64) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(sector).ok()?.checked_mul(SECTOR_SIZE)?;
    Some(start..start.checked_add(SECTOR_SIZE)?)
}

fn block_read(capability: u64, sector: u64, destination: &mut [u8; SECTOR_SIZE]) -> bool {
    if capability != CAPABILITY
        || !writable_range(destination.as_mut_ptr() as usize, destination.len())
    {
        return false;
    }
    let disk = DISK.lock().unwrap();
    let disk = disk.as_ref().unwrap();
    let Some(bytes) = sector_range(sector).and_then(|range| disk.bytes.get(range)) else {
        return false;
    };
    destination.copy_from_slice(bytes);
    true
}

fn block_write(capability: u64, sector: u64, source: &[u8; SECTOR_SIZE]) -> bool {
    let address = source.as_ptr() as usize;
    let writable = writable_range(address, source.len());
    let mut disk = DISK.lock().unwrap();
    let disk = disk.as_mut().unwrap();
    disk.attempts.push(Attempt {
        capability,
        sector,
        address,
        writable,
    });
    if capability != CAPABILITY
        || !writable
        || disk.fail_after.is_some_and(|limit| disk.writes >= limit)
    {
        return false;
    }
    let Some(bytes) = sector_range(sector).and_then(|range| disk.bytes.get_mut(range)) else {
        return false;
    };
    bytes.copy_from_slice(source);
    disk.writes += 1;
    true
}

fn block_flush(capability: u64) -> bool {
    if capability != CAPABILITY {
        return false;
    }
    let mut disk = DISK.lock().unwrap();
    let disk = disk.as_mut().unwrap();
    disk.flushes += 1;
    disk.flush_supported
}

#[test]
fn readonly_zero_and_nonzero_sources_are_copied_into_owned_writable_buffers() {
    let _serial = SERIAL.lock().unwrap();
    static ZERO: [u8; SECTOR_SIZE] = [0; SECTOR_SIZE];
    static DATA: [u8; SECTOR_SIZE] = [0x5a; SECTOR_SIZE];
    setup(vec![0xff; DISK_SIZE], true);
    let mut device = SyscallBlockDevice::new(CAPABILITY);
    for (sector, source) in [(3, &ZERO), (7, &DATA)] {
        assert!(!writable_range(source.as_ptr() as usize, source.len()));
        device.write_sector(sector, source).unwrap();
        let disk = DISK.lock().unwrap();
        let disk = disk.as_ref().unwrap();
        let attempt = disk.attempts.last().unwrap();
        assert_eq!((attempt.capability, attempt.sector), (CAPABILITY, sector));
        assert!(attempt.writable);
        // Keep addresses only for comparison; never dereference after return.
        assert_ne!(attempt.address, source.as_ptr() as usize);
        assert_eq!(&disk.bytes[sector_range(sector).unwrap()], source);
    }
    assert_eq!(ZERO, [0; SECTOR_SIZE]);
    assert_eq!(DATA, [0x5a; SECTOR_SIZE]);
}

#[test]
fn write_failures_and_capability_limits_are_preserved() {
    let _serial = SERIAL.lock().unwrap();
    setup(vec![0; DISK_SIZE], true);
    let mut device = SyscallBlockDevice::new(0);
    assert_eq!(
        device.write_sector(0, &[1; SECTOR_SIZE]),
        Err(StorageError::Block)
    );
    assert_eq!(device.flush(), Err(StorageError::Block));
    let mut device = SyscallBlockDevice::new(CAPABILITY);
    assert_eq!(
        device.write_sector(u64::MAX, &[1; SECTOR_SIZE]),
        Err(StorageError::Block)
    );
    DISK.lock().unwrap().as_mut().unwrap().fail_after = Some(0);
    assert_eq!(
        device.write_sector(0, &[1; SECTOR_SIZE]),
        Err(StorageError::Block)
    );
    let disk = DISK.lock().unwrap();
    let disk = disk.as_ref().unwrap();
    assert_eq!((disk.writes, disk.flushes), (0, 0));
    assert!(disk.bytes.iter().all(|byte| *byte == 0));
}

#[test]
fn format_create_and_rename_keep_contents_identity_and_flush_barriers() {
    let _serial = SERIAL.lock().unwrap();
    setup(vec![0; DISK_SIZE], true);
    let (mut volume, formatted) =
        Vfs::mount_or_format(SyscallBlockDevice::new(CAPABILITY)).unwrap();
    assert!(formatted);
    let handle = volume.create(b"kept").unwrap();
    volume.write(handle, b"retained neighbor").unwrap();
    let renamed = volume.rename(b"kept", b"renamed").unwrap();
    assert_eq!(handle, renamed);
    volume.flush().unwrap();
    let mut bytes = [0; 32];
    let count = volume.read_at(renamed, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], b"retained neighbor");
    Vfs::<SyscallBlockDevice>::check_existing(&mut SyscallBlockDevice::new(CAPABILITY)).unwrap();
    let disk = DISK.lock().unwrap();
    let disk = disk.as_ref().unwrap();
    assert!(disk.writes > 0);
    assert!(disk.flushes >= 12);
    assert!(disk.attempts.iter().all(|attempt| attempt.writable));
}

#[test]
fn unsupported_flush_aborts_before_publishing_a_formatted_volume() {
    let _serial = SERIAL.lock().unwrap();
    setup(vec![0; DISK_SIZE], false);
    assert_eq!(
        Vfs::mount_or_format(SyscallBlockDevice::new(CAPABILITY)).err(),
        Some(StorageError::Block)
    );
    let disk = DISK.lock().unwrap();
    let disk = disk.as_ref().unwrap();
    assert_eq!((disk.writes, disk.flushes), (1, 1));
    assert!(disk.bytes.iter().all(|byte| *byte == 0));
}

#[test]
fn clean_existing_mount_and_integrity_check_remain_read_only() {
    let _serial = SERIAL.lock().unwrap();
    setup(vec![0; DISK_SIZE], true);
    let (mut volume, _) = Vfs::mount_or_format(SyscallBlockDevice::new(CAPABILITY)).unwrap();
    let kept = volume.create(b"kept").unwrap();
    volume.write(kept, b"retained neighbor").unwrap();
    {
        let mut disk = DISK.lock().unwrap();
        let disk = disk.as_mut().unwrap();
        disk.writes = 0;
        disk.flushes = 0;
        disk.attempts.clear();
    }
    let mut volume = Vfs::mount_existing(SyscallBlockDevice::new(CAPABILITY)).unwrap();
    let mut bytes = [0; 32];
    let kept = volume.open(b"kept").unwrap();
    let count = volume.read_at(kept, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], b"retained neighbor");
    Vfs::<SyscallBlockDevice>::check_existing(&mut SyscallBlockDevice::new(CAPABILITY)).unwrap();
    let disk = DISK.lock().unwrap();
    let disk = disk.as_ref().unwrap();
    assert_eq!((disk.writes, disk.flushes, disk.attempts.len()), (0, 0, 0));
}

#[test]
fn pending_transaction_recovers_through_the_owned_adapter_without_losing_neighbors() {
    let _serial = SERIAL.lock().unwrap();
    setup(vec![0; DISK_SIZE], true);
    let (mut volume, _) = Vfs::mount_or_format(SyscallBlockDevice::new(CAPABILITY)).unwrap();
    let kept = volume.create(b"kept").unwrap();
    volume.write(kept, b"retained neighbor").unwrap();
    let baseline = DISK.lock().unwrap().as_ref().unwrap().bytes.clone();
    let mut recovered = 0;
    // Interrupt each write boundary of create, then reconnect without faults.
    for limit in 0..24 {
        setup(baseline.clone(), true);
        let mut volume = Vfs::mount_existing(SyscallBlockDevice::new(CAPABILITY)).unwrap();
        DISK.lock().unwrap().as_mut().unwrap().fail_after = Some(limit);
        let result = volume.create(b"interrupted");
        DISK.lock().unwrap().as_mut().unwrap().fail_after = None;
        if result.is_ok() {
            break;
        }
        assert_eq!(result, Err(StorageError::Block));
        let writes_before = DISK.lock().unwrap().as_ref().unwrap().writes;
        let check =
            Vfs::<SyscallBlockDevice>::check_existing(&mut SyscallBlockDevice::new(CAPABILITY));
        assert_eq!(DISK.lock().unwrap().as_ref().unwrap().writes, writes_before);
        if check == Err(StorageError::RecoveryRequired) {
            recovered += 1;
            let mut volume = Vfs::mount_existing(SyscallBlockDevice::new(CAPABILITY)).unwrap();
            let mut bytes = [0; 32];
            let kept = volume.open(b"kept").unwrap();
            let count = volume.read_at(kept, 0, &mut bytes).unwrap();
            assert_eq!(&bytes[..count], b"retained neighbor");
            assert_eq!(volume.open(b"interrupted"), Err(StorageError::NotFound));
            Vfs::<SyscallBlockDevice>::check_existing(&mut SyscallBlockDevice::new(CAPABILITY))
                .unwrap();
            let disk = DISK.lock().unwrap();
            let disk = disk.as_ref().unwrap();
            assert!(disk.bytes[..SECTOR_SIZE].iter().all(|byte| *byte == 0));
            assert!(disk.attempts.iter().all(|attempt| attempt.writable));
        } else {
            check.unwrap();
        }
    }
    assert!(recovered > 0, "must exercise valid pending undo records");
}
