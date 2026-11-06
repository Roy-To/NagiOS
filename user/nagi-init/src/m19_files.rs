//! Ordinary owner Files operations, independent of the semantic index.
//!
//! The VFS journals regular-file creation and directory-entry rename. A
//! pending journal blocks ordinary access without writes; only explicit
//! Recovery restores it. Trash uses the same transaction mechanism.
//! without freeing the inode or reading the file contents.
//! A mirrored, checksummed intent journal outside the searchable directory keeps
//! the original UTF-8 name across restart. Only journal-confirmed trash is hidden.

use alloc::{string::String, vec::Vec};
use libnagi::storage::{BlockDevice, DirectoryEntry, StorageError, Vfs, MAX_NAME_LENGTH};

fn with_files_access<D: BlockDevice, T>(
    volume: &mut Vfs<D>,
    operation: impl FnOnce(&mut Vfs<D>) -> Result<T, Error>,
) -> Result<T, Error> {
    #[cfg(test)]
    tests::access_attempted();
    volume
        .with_access(|volume| {
            #[cfg(test)]
            tests::access_acquired();
            Ok::<_, StorageError>(operation(volume))
        })
        .map_err(Error::from)?
}

pub(crate) const ROOT: &[u8] = b"/home/owner/files";
pub(crate) const TRASH_PREFIX: &str = ".nagi-trash-";
const JOURNALS: [&[u8]; 2] = [b"/home/owner/.files-trash-a", b"/home/owner/.files-trash-b"];
const MAX_TRASH: usize = 8;
const MAX_JOURNAL_BYTES: usize = 25 + MAX_TRASH * (9 + MAX_NAME_LENGTH);
const MAGIC: &[u8; 8] = b"NAGITR01";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub name: String,
    pub inode: u32,
    pub generation: u32,
}

impl Entry {
    pub fn key(&self) -> String {
        alloc::format!("{}:{}", self.inode, self.generation)
    }

    fn trash_name(&self) -> String {
        alloc::format!("{TRASH_PREFIX}{:x}-{:x}", self.inode, self.generation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    InvalidName,
    Conflict,
    StaleSelection,
    Capacity,
    CorruptJournal,
    RecoveryRequired,
    Storage,
    DurabilityUnknown,
}

impl From<StorageError> for Error {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::AlreadyExists => Self::Conflict,
            StorageError::InvalidName | StorageError::NameTooLong => Self::InvalidName,
            StorageError::Capacity | StorageError::DirectoryFull => Self::Capacity,
            StorageError::Indeterminate => Self::DurabilityUnknown,
            StorageError::RecoveryRequired => Self::RecoveryRequired,
            _ => Self::Storage,
        }
    }
}

pub(crate) fn initialize<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<(), Error> {
    with_files_access(volume, |volume| {
        for path in [b"/home".as_slice(), b"/home/owner", ROOT] {
            volume.ensure_directory_path(path)?;
        }
        Ok(())
    })
}

pub(crate) fn valid_name(name: &[u8]) -> bool {
    valid_existing_name(name)
        && core::str::from_utf8(name).is_ok_and(|name| !name.starts_with(TRASH_PREFIX))
}

fn valid_existing_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LENGTH
        && name != b"."
        && name != b".."
        && !name.iter().any(|byte| *byte == 0 || *byte == b'/')
        && core::str::from_utf8(name).is_ok()
}

pub(crate) fn path(name: &[u8]) -> Result<Vec<u8>, Error> {
    if !valid_name(name) {
        return Err(Error::InvalidName);
    }
    Ok(raw_path(name))
}

fn raw_path(name: &[u8]) -> Vec<u8> {
    let mut path = ROOT.to_vec();
    path.push(b'/');
    path.extend_from_slice(name);
    path
}

fn entry_at<D: BlockDevice>(volume: &mut Vfs<D>, name: &str) -> Result<Entry, Error> {
    let metadata = volume.metadata_path(&raw_path(name.as_bytes()))?;
    // Reject directories and other inode types before any rename.
    if metadata.mode & 0xf000 != 0x8000 {
        return Err(Error::StaleSelection);
    }
    Ok(Entry {
        name: name.into(),
        inode: metadata.inode,
        generation: metadata.generation,
    })
}

fn same_identity(a: &Entry, b: &Entry) -> bool {
    a.inode == b.inode && a.generation == b.generation
}

fn map_selection_check_error(error: Error) -> Error {
    match error {
        Error::RecoveryRequired => Error::RecoveryRequired,
        _ => Error::StaleSelection,
    }
}

pub(crate) fn is_managed_trash(current: &Entry, journal_entries: &[Entry]) -> bool {
    journal_entries
        .iter()
        .any(|entry| same_identity(current, entry) && current.name == entry.trash_name())
}

fn check<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    if !valid_existing_name(selected.name.as_bytes()) {
        return Err(Error::InvalidName);
    }
    let current = entry_at(volume, &selected.name).map_err(map_selection_check_error)?;
    if !same_identity(&current, selected) {
        return Err(Error::StaleSelection);
    }
    Ok(())
}

pub(crate) fn list<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Vec<Entry>, Error> {
    with_files_access(volume, |volume| {
        let journal = Journal::load(volume)?;
        let mut entries = [DirectoryEntry::empty(); 64];
        let count = volume.list_directory_path(ROOT, &mut entries)?;
        let mut result = Vec::new();
        for entry in &entries[..count] {
            let Ok(name) = core::str::from_utf8(entry.name()) else {
                continue;
            };
            if entry.file_type == 1 && valid_existing_name(name.as_bytes()) {
                let current = entry_at(volume, name)?;
                if is_managed_trash(&current, &journal.entries) {
                    continue;
                }
                result.push(current);
            }
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    })
}

pub(crate) fn create<D: BlockDevice>(volume: &mut Vfs<D>, name: &[u8]) -> Result<Entry, Error> {
    with_files_access(volume, |volume| {
        volume.create_path(&path(name)?)?;
        volume.flush().map_err(|_| Error::DurabilityUnknown)?;
        entry_at(
            volume,
            core::str::from_utf8(name).map_err(|_| Error::InvalidName)?,
        )
    })
}

pub(crate) fn rename<D: BlockDevice>(
    volume: &mut Vfs<D>,
    selected: &Entry,
    name: &[u8],
) -> Result<Entry, Error> {
    with_files_access(volume, |volume| {
        path(name)?;
        check(volume, selected)?;
        volume.rename_child(ROOT, selected.name.as_bytes(), name)?;
        volume.flush().map_err(|_| Error::DurabilityUnknown)?;
        entry_at(
            volume,
            core::str::from_utf8(name).map_err(|_| Error::InvalidName)?,
        )
    })
}

struct Journal {
    sequence: u64,
    entries: Vec<Entry>,
}

impl Journal {
    fn load<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Self, Error> {
        let mut latest: Option<Self> = None;
        for path in JOURNALS {
            let handle = match volume.open_path(path) {
                Ok(handle) => handle,
                Err(StorageError::NotFound) => continue,
                Err(error) => return Err(error.into()),
            };
            let metadata = volume.metadata_path(path)?;
            let length = usize::try_from(metadata.size).map_err(|_| Error::CorruptJournal)?;
            if length > MAX_JOURNAL_BYTES {
                continue;
            }
            let mut buffer = [0; MAX_JOURNAL_BYTES];
            let read = volume.read_at(handle, 0, &mut buffer[..length])?;
            if read != length {
                continue;
            }
            if let Some(journal) = Self::decode(&buffer[..read]) {
                if latest
                    .as_ref()
                    .is_none_or(|previous| journal.sequence > previous.sequence)
                {
                    latest = Some(journal);
                }
            }
        }
        if let Some(journal) = latest {
            return Ok(journal);
        }
        Ok(Self {
            sequence: 0,
            entries: Vec::new(),
        })
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 25 || &bytes[..8] != MAGIC {
            return None;
        }
        let payload = bytes.get(..bytes.len() - 8)?;
        let digest = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().ok()?);
        if checksum(payload) != digest {
            return None;
        }
        let sequence = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
        let count = usize::from(bytes[16]);
        if count > MAX_TRASH {
            return None;
        }
        let mut entries = Vec::new();
        let mut offset = 17;
        for _ in 0..count {
            let inode = u32::from_le_bytes(payload.get(offset..offset + 4)?.try_into().ok()?);
            let generation =
                u32::from_le_bytes(payload.get(offset + 4..offset + 8)?.try_into().ok()?);
            let length = usize::from(*payload.get(offset + 8)?);
            offset += 9;
            let name = payload.get(offset..offset + length)?;
            offset += length;
            if !valid_existing_name(name) || inode == 0 || generation == 0 {
                return None;
            }
            let entry = Entry {
                name: core::str::from_utf8(name).ok()?.into(),
                inode,
                generation,
            };
            if entries
                .iter()
                .any(|previous| same_identity(previous, &entry))
            {
                return None;
            }
            entries.push(entry);
        }
        if offset != payload.len() {
            return None;
        }
        Some(Self { sequence, entries })
    }

    fn save<D: BlockDevice>(&mut self, volume: &mut Vfs<D>) -> Result<(), Error> {
        self.sequence = self.sequence.checked_add(1).ok_or(Error::CorruptJournal)?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.push(self.entries.len() as u8);
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.inode.to_le_bytes());
            bytes.extend_from_slice(&entry.generation.to_le_bytes());
            bytes.push(entry.name.len() as u8);
            bytes.extend_from_slice(entry.name.as_bytes());
        }
        bytes.extend_from_slice(&checksum(&bytes).to_le_bytes());
        let path = JOURNALS[(self.sequence % 2) as usize];
        let handle = match volume.open_path(path) {
            Ok(handle) => {
                // Preserve unrelated or damaged files occupying a journal path.
                let metadata = volume.metadata_path(path)?;
                let length = usize::try_from(metadata.size).map_err(|_| Error::CorruptJournal)?;
                if length > MAX_JOURNAL_BYTES {
                    return Err(Error::CorruptJournal);
                }
                let mut existing = [0; MAX_JOURNAL_BYTES];
                let read = volume.read_at(handle, 0, &mut existing[..length])?;
                if read != length || Self::decode(&existing[..read]).is_none() {
                    return Err(Error::CorruptJournal);
                }
                handle
            }
            Err(StorageError::NotFound) => {
                // Publish a complete initial journal under the create undo.
                // A failed payload write must not strand an empty reserved slot.
                volume.create_path_with_initial_contents(path, &bytes)?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        volume.write(handle, &bytes)?;
        volume.flush().map_err(|_| Error::DurabilityUnknown)
    }
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |value, byte| {
        (value ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn ensure_unapplied_intent_source<D: BlockDevice>(
    volume: &mut Vfs<D>,
    entry: &Entry,
) -> Result<(), Error> {
    match volume.metadata_path(&raw_path(entry.name.as_bytes())) {
        Ok(source)
            if source.mode & 0xf000 == 0x8000
                && source.inode == entry.inode
                && source.generation == entry.generation =>
        {
            Ok(())
        }
        Ok(_) | Err(StorageError::NotFound) => Err(Error::CorruptJournal),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn trash_entries<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Vec<Entry>, Error> {
    with_files_access(volume, |volume| {
        let journal = Journal::load(volume)?;
        let mut result = Vec::new();
        for entry in journal.entries {
            match entry_at(volume, &entry.trash_name()) {
                Ok(current) if same_identity(&current, &entry) => result.push(entry),
                Ok(_) | Err(Error::StaleSelection) => {
                    // A prepared intent can outlive a failed or raced rename. A
                    // mismatched file or non-file destination is unapplied while
                    // the original identity remains live at its source name.
                    // Missing or reused sources remain a corrupt journal.
                    ensure_unapplied_intent_source(volume, &entry)?;
                }
                Err(Error::Storage)
                    if volume.metadata_path(&raw_path(entry.trash_name().as_bytes()))
                        == Err(StorageError::NotFound) =>
                {
                    // An absent destination alone does not prove the intent was
                    // never applied: its recorded source must still be live.
                    ensure_unapplied_intent_source(volume, &entry)?;
                }
                Err(error) => return Err(error),
            }
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    })
}

pub(crate) fn trash<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    with_files_access(volume, |volume| {
        if !valid_existing_name(selected.name.as_bytes()) {
            return Err(Error::InvalidName);
        }
        if selected.name == selected.trash_name() {
            return Err(Error::Conflict);
        }
        let existing = trash_entries(volume)?;
        if existing.contains(selected) {
            return Ok(());
        }
        check(volume, selected)?;
        if existing.len() >= MAX_TRASH {
            return Err(Error::Capacity);
        }
        // The process-local VFS gate now spans this complete journal/rename
        // protocol. Cross-process volume ownership remains a separate contract.
        // Reject any remaining occupant before persisting a new intent.
        match volume.metadata_path(&raw_path(selected.trash_name().as_bytes())) {
            Ok(_) => return Err(Error::Conflict),
            Err(StorageError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
        let mut journal = Journal::load(volume)?;
        journal.entries = existing;
        journal.entries.push(selected.clone());
        // Both copies contain the original name BEFORE the physical rename. A
        // torn latest journal can therefore fall back without losing a trash entry.
        journal.save(volume)?;
        journal.save(volume)?;
        volume.rename_child(
            ROOT,
            selected.name.as_bytes(),
            selected.trash_name().as_bytes(),
        )?;
        volume.flush().map_err(|_| Error::DurabilityUnknown)
    })
}

#[cfg(test)]
pub(super) fn persist_test_trash_intent<D: BlockDevice>(
    volume: &mut Vfs<D>,
    entry: &Entry,
) -> Result<(), Error> {
    with_files_access(volume, |volume| {
        let mut journal = Journal::load(volume)?;
        journal.entries.push(entry.clone());
        journal.save(volume)?;
        journal.save(volume)
    })
}

pub(crate) fn restore<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    with_files_access(volume, |volume| {
        if !valid_existing_name(selected.name.as_bytes()) {
            return Err(Error::InvalidName);
        }
        let entries = trash_entries(volume)?;
        if !entries.contains(selected) {
            return check(volume, selected); // Idempotent only for this exact identity.
        }
        volume.rename_child(
            ROOT,
            selected.trash_name().as_bytes(),
            selected.name.as_bytes(),
        )?;
        volume.flush().map_err(|_| Error::DurabilityUnknown)?;
        // Stale intent records are harmless after restore; lookup checks physical
        // identity. Cleanup failure must not misreport a completed restore.
        let mut journal = Journal::load(volume)?;
        journal.entries = entries
            .into_iter()
            .filter(|entry| entry != selected)
            .collect();
        let _ = journal.save(volume);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };

    static ACCESS_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
    static ACCESS_ACQUIRED: AtomicUsize = AtomicUsize::new(0);
    static PAUSE_NEXT_ACCESS: AtomicBool = AtomicBool::new(false);
    static ACCESS_PAUSED: AtomicBool = AtomicBool::new(false);
    static RELEASE_ACCESS: AtomicBool = AtomicBool::new(false);

    std::thread_local! {
        static FILES_TEST_ROLE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    }

    pub(super) fn access_attempted() {
        FILES_TEST_ROLE.with(|role| {
            if role.get() != 0 {
                ACCESS_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
            }
        });
    }

    pub(super) fn access_acquired() {
        let role = FILES_TEST_ROLE.with(std::cell::Cell::get);
        if role == 0 {
            return;
        }
        ACCESS_ACQUIRED.fetch_add(1, Ordering::SeqCst);
        if role == 1 && PAUSE_NEXT_ACCESS.swap(false, Ordering::SeqCst) {
            ACCESS_PAUSED.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !RELEASE_ACCESS.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::yield_now();
            }
        }
    }

    #[derive(Clone)]
    struct SharedDisk(Arc<Mutex<std::vec::Vec<u8>>>);

    impl Default for SharedDisk {
        fn default() -> Self {
            Self(Arc::new(Mutex::new(std::vec![0; 8 * 1024 * 1024])))
        }
    }

    impl libnagi::storage::ReadOnlyBlockDevice for SharedDisk {
        fn read_sector(
            &mut self,
            sector: u64,
            output: &mut [u8; libnagi::storage::SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let bytes = self.0.lock().unwrap();
            let offset = sector as usize * libnagi::storage::SECTOR_SIZE;
            output.copy_from_slice(
                bytes
                    .get(offset..offset + libnagi::storage::SECTOR_SIZE)
                    .ok_or(StorageError::Block)?,
            );
            Ok(())
        }
    }

    impl BlockDevice for SharedDisk {
        fn write_sector(
            &mut self,
            sector: u64,
            input: &[u8; libnagi::storage::SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let mut bytes = self.0.lock().unwrap();
            let offset = sector as usize * libnagi::storage::SECTOR_SIZE;
            bytes
                .get_mut(offset..offset + libnagi::storage::SECTOR_SIZE)
                .ok_or(StorageError::Block)?
                .copy_from_slice(input);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), StorageError> {
            Ok(())
        }
    }

    fn wait_until(condition: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::yield_now();
        }
        condition()
    }

    struct ReleaseGateOnDrop;

    impl Drop for ReleaseGateOnDrop {
        fn drop(&mut self) {
            RELEASE_ACCESS.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn selection_check_keeps_recovery_required_distinct_from_stale() {
        assert_eq!(
            map_selection_check_error(Error::RecoveryRequired),
            Error::RecoveryRequired
        );
        assert_eq!(
            map_selection_check_error(Error::Storage),
            Error::StaleSelection
        );
    }

    #[test]
    fn only_matching_journal_identity_is_managed_trash() {
        let record = Entry {
            name: "legacy.txt".into(),
            inode: 7,
            generation: 3,
        };
        let trash = Entry {
            name: record.trash_name(),
            inode: record.inode,
            generation: record.generation,
        };
        let unrelated = Entry {
            name: ".nagi-trash-legacy.txt".into(),
            inode: 8,
            generation: 4,
        };
        assert!(is_managed_trash(&trash, core::slice::from_ref(&record)));
        assert!(!is_managed_trash(
            &unrelated,
            core::slice::from_ref(&record)
        ));
        assert!(!is_managed_trash(&unrelated, &[]));
    }

    #[test]
    fn trash_journal_protocol_is_serialized_across_vfs_instances() {
        ACCESS_ATTEMPTS.store(0, Ordering::SeqCst);
        ACCESS_ACQUIRED.store(0, Ordering::SeqCst);
        PAUSE_NEXT_ACCESS.store(false, Ordering::SeqCst);
        ACCESS_PAUSED.store(false, Ordering::SeqCst);
        RELEASE_ACCESS.store(false, Ordering::SeqCst);
        let _release = ReleaseGateOnDrop;

        let disk = SharedDisk::default();
        let (mut first, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        initialize(&mut first).unwrap();
        let first_entry = create(&mut first, b"first.txt").unwrap();
        let second_entry = create(&mut first, b"second.txt").unwrap();
        let mut second = Vfs::mount_existing(disk.clone()).unwrap();
        ACCESS_ATTEMPTS.store(0, Ordering::SeqCst);
        ACCESS_ACQUIRED.store(0, Ordering::SeqCst);
        ACCESS_PAUSED.store(false, Ordering::SeqCst);
        PAUSE_NEXT_ACCESS.store(true, Ordering::SeqCst);

        let first_worker = thread::spawn(move || {
            FILES_TEST_ROLE.with(|role| role.set(1));
            trash(&mut first, &first_entry)
        });
        assert!(wait_until(|| ACCESS_PAUSED.load(Ordering::SeqCst)));

        let (second_done_tx, second_done_rx) = std::sync::mpsc::sync_channel(1);
        let second_worker = thread::spawn(move || {
            FILES_TEST_ROLE.with(|role| role.set(2));
            let result = trash(&mut second, &second_entry);
            let _ = second_done_tx.send(());
            result
        });
        assert!(wait_until(|| ACCESS_ATTEMPTS.load(Ordering::SeqCst) >= 2));
        assert!(
            matches!(
                second_done_rx.recv_timeout(Duration::from_millis(250)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ),
            "the second Files call completed while the first held the VFS gate"
        );
        assert_eq!(
            ACCESS_ACQUIRED.load(Ordering::SeqCst),
            1,
            "the second Vfs instance entered the Files protocol while the first held it"
        );

        RELEASE_ACCESS.store(true, Ordering::SeqCst);
        first_worker.join().unwrap().unwrap();
        second_worker.join().unwrap().unwrap();

        let mut recovered = Vfs::mount_existing(disk).unwrap();
        let entries = trash_entries(&mut recovered).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.name == "first.txt"));
        assert!(entries.iter().any(|entry| entry.name == "second.txt"));
    }
}
