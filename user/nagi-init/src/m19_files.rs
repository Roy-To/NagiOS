//! Ordinary owner Files operations, independent of the semantic index.
//!
//! The VFS journals regular-file creation and directory-entry rename, with
//! undo recovery before further access or restart. Trash uses that operation
//! without freeing the inode or reading the file contents.
//! A mirrored, checksummed intent journal outside the searchable directory keeps
//! the original UTF-8 name across restart. Search must exclude reserved names.

use alloc::{string::String, vec::Vec};
use libnagi::storage::{BlockDevice, DirectoryEntry, StorageError, Vfs, MAX_NAME_LENGTH};

pub(crate) const ROOT: &[u8] = b"/home/owner/files";
pub(crate) const TRASH_PREFIX: &str = ".nagi-trash-";
const JOURNALS: [&[u8]; 2] = [b"/home/owner/.files-trash-a", b"/home/owner/.files-trash-b"];
const MAX_TRASH: usize = 8;
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
    Storage,
    DurabilityUnknown,
}

impl From<StorageError> for Error {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::AlreadyExists => Self::Conflict,
            StorageError::InvalidName | StorageError::NameTooLong => Self::InvalidName,
            StorageError::Capacity | StorageError::DirectoryFull => Self::Capacity,
            _ => Self::Storage,
        }
    }
}

pub(crate) fn initialize<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<(), Error> {
    for path in [b"/home".as_slice(), b"/home/owner", ROOT] {
        volume.ensure_directory_path(path)?;
    }
    Ok(())
}

pub(crate) fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LENGTH
        && name != b"."
        && name != b".."
        && !name.iter().any(|byte| *byte == 0 || *byte == b'/')
        && core::str::from_utf8(name).is_ok_and(|name| !name.starts_with(TRASH_PREFIX))
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

fn check<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    path(selected.name.as_bytes())?;
    let current = entry_at(volume, &selected.name).map_err(|_| Error::StaleSelection)?;
    if !same_identity(&current, selected) {
        return Err(Error::StaleSelection);
    }
    Ok(())
}

pub(crate) fn list<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Vec<Entry>, Error> {
    let mut entries = [DirectoryEntry::empty(); 64];
    let count = volume.list_directory_path(ROOT, &mut entries)?;
    let mut result = Vec::new();
    for entry in &entries[..count] {
        let Ok(name) = core::str::from_utf8(entry.name()) else {
            continue;
        };
        if entry.file_type == 1 && valid_name(name.as_bytes()) {
            result.push(entry_at(volume, name)?);
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

pub(crate) fn create<D: BlockDevice>(volume: &mut Vfs<D>, name: &[u8]) -> Result<Entry, Error> {
    volume.create_path(&path(name)?)?;
    volume.flush().map_err(|_| Error::DurabilityUnknown)?;
    entry_at(
        volume,
        core::str::from_utf8(name).map_err(|_| Error::InvalidName)?,
    )
}

pub(crate) fn rename<D: BlockDevice>(
    volume: &mut Vfs<D>,
    selected: &Entry,
    name: &[u8],
) -> Result<Entry, Error> {
    path(name)?;
    check(volume, selected)?;
    volume.rename_child(ROOT, selected.name.as_bytes(), name)?;
    volume.flush().map_err(|_| Error::DurabilityUnknown)?;
    entry_at(
        volume,
        core::str::from_utf8(name).map_err(|_| Error::InvalidName)?,
    )
}

struct Journal {
    sequence: u64,
    entries: Vec<Entry>,
}

impl Journal {
    fn load<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Self, Error> {
        let mut latest: Option<Self> = None;
        let mut exists = false;
        for path in JOURNALS {
            let handle = match volume.open_path(path) {
                Ok(handle) => handle,
                Err(StorageError::NotFound) => continue,
                Err(error) => return Err(error.into()),
            };
            exists = true;
            let mut buffer = [0; 1024];
            let length = volume.read(handle, &mut buffer)?;
            if let Some(journal) = Self::decode(&buffer[..length]) {
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
        if exists {
            // A first prepare may have failed after allocating an empty journal.
            // Reinitialize only after proving no physical trash needs recovery.
            let mut entries = [DirectoryEntry::empty(); 64];
            let count = volume.list_directory_path(ROOT, &mut entries)?;
            if entries[..count].iter().any(|entry| {
                core::str::from_utf8(entry.name()).is_ok_and(|name| name.starts_with(TRASH_PREFIX))
            }) {
                return Err(Error::CorruptJournal);
            }
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
            if !valid_name(name) || inode == 0 || generation == 0 {
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
            Ok(handle) => handle,
            Err(StorageError::NotFound) => volume.create_path(path)?,
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

pub(crate) fn trash_entries<D: BlockDevice>(volume: &mut Vfs<D>) -> Result<Vec<Entry>, Error> {
    let journal = Journal::load(volume)?;
    let mut result = Vec::new();
    for entry in journal.entries {
        match entry_at(volume, &entry.trash_name()) {
            Ok(current) if same_identity(&current, &entry) => result.push(entry),
            Ok(_) => return Err(Error::CorruptJournal),
            Err(Error::Storage)
                if volume.metadata_path(&raw_path(entry.trash_name().as_bytes()))
                    == Err(StorageError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    // A reserved entry without its mirrored descriptor is never guessed or deleted.
    let mut entries = [DirectoryEntry::empty(); 64];
    let count = volume.list_directory_path(ROOT, &mut entries)?;
    for entry in &entries[..count] {
        if core::str::from_utf8(entry.name()).is_ok_and(|name| name.starts_with(TRASH_PREFIX))
            && !result
                .iter()
                .any(|record| record.trash_name().as_bytes() == entry.name())
        {
            return Err(Error::CorruptJournal);
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

pub(crate) fn trash<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    path(selected.name.as_bytes())?;
    let existing = trash_entries(volume)?;
    if existing.contains(selected) {
        return Ok(());
    }
    check(volume, selected)?;
    if existing.len() >= MAX_TRASH {
        return Err(Error::Capacity);
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
}

pub(crate) fn restore<D: BlockDevice>(volume: &mut Vfs<D>, selected: &Entry) -> Result<(), Error> {
    path(selected.name.as_bytes())?;
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
}
