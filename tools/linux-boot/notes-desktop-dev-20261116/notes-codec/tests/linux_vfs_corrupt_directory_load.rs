#![cfg(target_os = "linux")]

use libnagi::storage::{StorageError, Vfs, BLOCK_SIZE, MAX_FILE_SIZE};
use notes_files_codec_candidate::{
    LinuxBlockDevice, NoteDocument, NoteStore, ObjectId, SharedVfs, StoreError, Timestamp,
    VfsNoteStore,
};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MIN_VFS_BYTES: u64 = 8 * 1024 * 1024;
const INODE_TABLE_BLOCK: u64 = 5;
const INODE_SIZE: u64 = 128;
const DIRECT_BLOCK_OFFSET: u64 = 40;

struct DiskFixture(PathBuf);

impl DiskFixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nagi-vfs-corrupt-load-{}-{stamp}.img",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("create disposable VFS image");
        file.set_len(MIN_VFS_BYTES)
            .expect("size disposable VFS image");
        file.sync_all().expect("flush disposable VFS image");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for DiskFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn read_exact_at(file: &File, destination: &mut [u8], offset: u64) {
    let mut completed = 0;
    while completed < destination.len() {
        let count = file
            .read_at(&mut destination[completed..], offset + completed as u64)
            .expect("read VFS image bytes");
        assert_ne!(count, 0, "short VFS image read");
        completed += count;
    }
}

fn write_all_at(file: &File, source: &[u8], offset: u64) {
    let mut completed = 0;
    while completed < source.len() {
        let count = file
            .write_at(&source[completed..], offset + completed as u64)
            .expect("write VFS image bytes");
        assert_ne!(count, 0, "short VFS image write");
        completed += count;
    }
}

fn directory_data_offset(file: &File, inode: u32) -> u64 {
    assert_ne!(inode, 0, "directory inode number is nonzero");
    let inode_index = u64::from(inode - 1);
    let inodes_per_block = BLOCK_SIZE as u64 / INODE_SIZE;
    let inode_block = INODE_TABLE_BLOCK + inode_index / inodes_per_block;
    let inode_offset = (inode_index % inodes_per_block) * INODE_SIZE + DIRECT_BLOCK_OFFSET;
    let mut direct_block_bytes = [0; 4];
    read_exact_at(
        file,
        &mut direct_block_bytes,
        inode_block * BLOCK_SIZE as u64 + inode_offset,
    );
    let direct_block = u32::from_le_bytes(direct_block_bytes);
    assert_ne!(direct_block, 0, "directory has a data block");
    u64::from(direct_block) * BLOCK_SIZE as u64
}

fn corrupt_revision_name_length(file: &File, directory_offset: u64) {
    let revision_name = b"rev-00000000000000000001.md";
    let mut directory = [0; BLOCK_SIZE];
    read_exact_at(file, &mut directory, directory_offset);

    let mut offset = 0;
    let mut revision_offset = None;
    while offset < BLOCK_SIZE {
        assert!(
            BLOCK_SIZE - offset >= 8,
            "fixture directory header is complete"
        );
        let inode = u32::from_le_bytes(directory[offset..offset + 4].try_into().unwrap());
        let record_length = usize::from(u16::from_le_bytes(
            directory[offset + 4..offset + 6].try_into().unwrap(),
        ));
        let name_length = usize::from(directory[offset + 6]);
        if inode != 0
            && name_length == revision_name.len()
            && &directory[offset + 8..offset + 8 + name_length] == revision_name
        {
            revision_offset = Some(offset);
            break;
        }
        assert!(record_length >= 8, "fixture record makes progress");
        assert!(
            record_length <= BLOCK_SIZE - offset,
            "fixture record stays in block"
        );
        offset += record_length;
    }

    let revision_offset = revision_offset.expect("saved revision directory entry");
    let original_length = usize::from(u16::from_le_bytes(
        directory[revision_offset + 4..revision_offset + 6]
            .try_into()
            .unwrap(),
    ));
    assert_eq!(original_length, 36);
    let free_offset = revision_offset + original_length;
    let free_length = usize::from(u16::from_le_bytes(
        directory[free_offset + 4..free_offset + 6]
            .try_into()
            .unwrap(),
    ));
    assert_eq!(free_offset + free_length, BLOCK_SIZE);

    // Keep the record and following free record in bounds while making the
    // on-disk name longer than DirectoryEntry.name's 32-byte storage.
    directory[revision_offset + 4..revision_offset + 6].copy_from_slice(&44_u16.to_le_bytes());
    directory[revision_offset + 6] = 33;
    let shifted_free_offset = revision_offset + 44;
    let shifted_free_length = free_length - 8;
    directory[shifted_free_offset..shifted_free_offset + 4].fill(0);
    directory[shifted_free_offset + 4..shifted_free_offset + 6]
        .copy_from_slice(&(shifted_free_length as u16).to_le_bytes());
    directory[shifted_free_offset + 6] = 0;
    directory[shifted_free_offset + 7] = 0;
    assert_eq!(shifted_free_offset + shifted_free_length, BLOCK_SIZE);
    write_all_at(file, &directory, directory_offset);
    file.sync_all().expect("flush malformed directory fixture");
}

fn corrupt_directory_with_four_byte_tail(file: &File, directory_offset: u64) -> [u8; BLOCK_SIZE] {
    let mut directory = [0; BLOCK_SIZE];
    read_exact_at(file, &mut directory, directory_offset);

    let dot_record_length = u16::from_le_bytes(directory[4..6].try_into().unwrap());
    assert_eq!(
        dot_record_length, 12,
        "fixture starts with a normal dot entry"
    );
    directory[4..6].copy_from_slice(&1020_u16.to_le_bytes());
    assert_eq!(BLOCK_SIZE - 1020, 4, "fixture leaves a short header tail");

    write_all_at(file, &directory, directory_offset);
    file.sync_all().expect("flush short-tail directory fixture");
    directory
}

#[test]
fn vfs_note_load_rejects_corrupt_directory_without_changing_saved_revision() {
    let disk = DiskFixture::new();
    let (shared, formatted) = SharedVfs::mount_or_format(disk.path()).expect("mount VFS");
    assert!(formatted);

    let store = VfsNoteStore::new(shared.clone());
    let id = ObjectId(0x4e41_4749_0000_0301);
    let initial = NoteDocument::new(id, "saved before corruption", Timestamp(1));
    let saved = store.commit(&initial, 0).expect("save revision 1");
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load(id).expect("load saved revision"), Some(saved));

    let note_directory = format!("/notes/{:016x}", id.0);
    let revision_path = format!("{note_directory}/rev-{:020}.md", 1);
    let mut revision_reader = shared.clone();
    let revision_handle = revision_reader
        .open_path(revision_path.as_bytes())
        .expect("open saved revision handle");
    let mut revision_before = vec![0; MAX_FILE_SIZE];
    let revision_length = revision_reader
        .read(revision_handle, &mut revision_before)
        .expect("read saved revision bytes");
    revision_before.truncate(revision_length);

    let directory_inode = {
        let device = LinuxBlockDevice::open(disk.path()).expect("open VFS inspection handle");
        let mut inspection_vfs =
            Vfs::<LinuxBlockDevice>::mount_existing(device).expect("mount inspection VFS");
        inspection_vfs
            .metadata_path(note_directory.as_bytes())
            .expect("read note directory metadata")
            .inode
    };
    {
        let image = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open disposable image for corruption");
        let directory_offset = directory_data_offset(&image, directory_inode);
        corrupt_revision_name_length(&image, directory_offset);
    }

    let error = store
        .load(id)
        .expect_err("corrupt directory must fail closed");
    assert!(matches!(
        error,
        StoreError::Io(message) if message.contains("Corrupt")
    ));

    let mut revision_after = vec![0; MAX_FILE_SIZE];
    let revision_length = revision_reader
        .read(revision_handle, &mut revision_after)
        .expect("saved revision remains readable by its existing handle");
    revision_after.truncate(revision_length);
    assert_eq!(revision_after, revision_before);

    drop(store);
    drop(revision_reader);
    drop(shared);
}

#[test]
fn vfs_note_load_and_directory_scans_reject_four_byte_tail_without_writes() {
    let disk = DiskFixture::new();
    let (shared, formatted) = SharedVfs::mount_or_format(disk.path()).expect("mount VFS");
    assert!(formatted);

    let store = VfsNoteStore::new(shared.clone());
    let id = ObjectId(0x4e41_4749_0000_0302);
    let initial = NoteDocument::new(id, "saved before short-tail corruption", Timestamp(1));
    let saved = store.commit(&initial, 0).expect("save revision 1");
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load(id).expect("load saved revision"), Some(saved));

    let note_directory = format!("/notes/{:016x}", id.0);
    let revision_path = format!("{note_directory}/rev-{:020}.md", 1);
    let mut revision_reader = shared.clone();
    let revision_handle = revision_reader
        .open_path(revision_path.as_bytes())
        .expect("open saved revision handle");
    let mut revision_before = vec![0; MAX_FILE_SIZE];
    let revision_length = revision_reader
        .read(revision_handle, &mut revision_before)
        .expect("read saved revision bytes");
    revision_before.truncate(revision_length);

    let directory_inode = {
        let device = LinuxBlockDevice::open(disk.path()).expect("open VFS inspection handle");
        let mut inspection_vfs =
            Vfs::<LinuxBlockDevice>::mount_existing(device).expect("mount inspection VFS");
        inspection_vfs
            .metadata_path(note_directory.as_bytes())
            .expect("read note directory metadata")
            .inode
    };
    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk.path())
        .expect("open disposable image for corruption");
    let directory_offset = directory_data_offset(&image, directory_inode);
    let corrupt_directory = corrupt_directory_with_four_byte_tail(&image, directory_offset);

    let error = store
        .load(id)
        .expect_err("short-tail directory must fail closed in VfsNoteStore::load");
    assert!(matches!(
        error,
        StoreError::Io(message) if message.contains("Corrupt")
    ));

    // Exercise the public path operations against the same malformed mounted
    // directory. These cover insertion lookup, deletion lookup, and the
    // empty-directory scan without invoking private VFS methods.
    let device = LinuxBlockDevice::open(disk.path()).expect("reopen VFS inspection handle");
    let mut vfs = Vfs::<LinuxBlockDevice>::mount_existing(device).expect("remount inspection VFS");
    let new_path = format!("{note_directory}/new.md");
    assert_eq!(
        vfs.create_path(new_path.as_bytes()),
        Err(StorageError::Corrupt),
        "insertion path rejects the incomplete trailing header"
    );
    assert_eq!(
        vfs.remove_path(revision_path.as_bytes()),
        Err(StorageError::Corrupt),
        "deletion path rejects the incomplete trailing header"
    );
    assert_eq!(
        vfs.rmdir_path(note_directory.as_bytes()),
        Err(StorageError::Corrupt),
        "empty-directory scan rejects the incomplete trailing header"
    );

    let mut directory_after = [0; BLOCK_SIZE];
    read_exact_at(&image, &mut directory_after, directory_offset);
    assert_eq!(
        directory_after, corrupt_directory,
        "failed scans do not write"
    );

    let mut revision_after = vec![0; MAX_FILE_SIZE];
    let revision_length = revision_reader
        .read(revision_handle, &mut revision_after)
        .expect("saved revision remains readable by its existing handle");
    revision_after.truncate(revision_length);
    assert_eq!(
        revision_after, revision_before,
        "saved revision bytes are unchanged"
    );

    drop(vfs);
    drop(store);
    drop(revision_reader);
    drop(shared);
}
