use super::*;
use crate::desktop::files_panel::{Focus, Operation, Panel};
use libnagi::storage::{ReadOnlyBlockDevice, SECTOR_SIZE};
use nagi_search::BackendError;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Disk(Rc<RefCell<DiskState>>);
struct DiskState {
    bytes: Vec<u8>,
    fail_write_after: Option<usize>,
    fail_flush_after: Option<usize>,
}
impl Default for Disk {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(DiskState {
            bytes: alloc::vec![0; 8 * 1024 * 1024],
            fail_write_after: None,
            fail_flush_after: None,
        })))
    }
}
fn trip(counter: &mut Option<usize>) -> bool {
    match counter.as_mut() {
        Some(0) => {
            *counter = None;
            true
        }
        Some(value) => {
            *value -= 1;
            false
        }
        None => false,
    }
}
impl ReadOnlyBlockDevice for Disk {
    fn read_sector(
        &mut self,
        sector: u64,
        output: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let offset = sector as usize * SECTOR_SIZE;
        output.copy_from_slice(
            self.0
                .borrow()
                .bytes
                .get(offset..offset + SECTOR_SIZE)
                .ok_or(StorageError::Block)?,
        );
        Ok(())
    }
}
impl BlockDevice for Disk {
    fn write_sector(&mut self, sector: u64, input: &[u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        let mut state = self.0.borrow_mut();
        if trip(&mut state.fail_write_after) {
            return Err(StorageError::Block);
        }
        let offset = sector as usize * SECTOR_SIZE;
        state
            .bytes
            .get_mut(offset..offset + SECTOR_SIZE)
            .ok_or(StorageError::Block)?
            .copy_from_slice(input);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), StorageError> {
        if trip(&mut self.0.borrow_mut().fail_flush_after) {
            Err(StorageError::Block)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Default)]
struct Backend(Rc<RefCell<(Option<Vec<u8>>, bool)>>);
impl SnapshotBackend for Backend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(self.0.borrow().0.clone())
    }
    fn write_snapshot(&mut self, bytes: &[u8]) -> Result<(), BackendError> {
        if self.0.borrow().1 {
            return Err(BackendError::Io);
        }
        self.0.borrow_mut().0 = Some(bytes.to_vec());
        Ok(())
    }
}
fn runtime(backend: Backend) -> Runtime<Backend> {
    Runtime {
        service: SearchService::open(backend, OwnerFilesVisibility).unwrap(),
        synchronized: false,
    }
}
fn volume() -> (Disk, Vfs<Disk>) {
    let disk = Disk::default();
    let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
    files::initialize(&mut volume).unwrap();
    (disk, volume)
}
fn contents(volume: &mut Vfs<Disk>, name: &str) -> Vec<u8> {
    let handle = volume
        .open_path(&files::path(name.as_bytes()).unwrap())
        .unwrap();
    let mut buffer = [0; 1024];
    let count = volume.read(handle, &mut buffer).unwrap();
    buffer[..count].to_vec()
}

#[test]
fn trash_restart_restore_preserves_bytes_inode_generation_and_search_object_id() {
    let (disk, mut volume) = volume();
    let original = files::create(&mut volume, "計画.txt".as_bytes()).unwrap();
    let handle = volume
        .open_path(&files::path(original.name.as_bytes()).unwrap())
        .unwrap();
    volume.write(handle, b"retain actual file bytes").unwrap();
    let backend = Backend::default();
    let mut search = runtime(backend.clone());
    search.sync_files(&mut volume).unwrap();
    let id = search.search_files("計画").unwrap()[0];
    files::trash(&mut volume, &original).unwrap();
    search.sync_files(&mut volume).unwrap();
    assert!(files::list(&mut volume).unwrap().is_empty());
    assert!(search.search_files("計画").unwrap().is_empty());
    assert_eq!(
        files::trash_entries(&mut volume).unwrap(),
        vec![original.clone()]
    );
    let mut remounted = Vfs::mount_existing(disk).unwrap();
    let mut reopened = runtime(backend);
    reopened.sync_files(&mut remounted).unwrap();
    assert!(reopened.search_files("計画").unwrap().is_empty());
    let selected = files::trash_entries(&mut remounted).unwrap().remove(0);
    files::restore(&mut remounted, &selected).unwrap();
    reopened.sync_files(&mut remounted).unwrap();
    assert_eq!(files::list(&mut remounted).unwrap(), vec![original]);
    assert_eq!(
        contents(&mut remounted, "計画.txt"),
        b"retain actual file bytes"
    );
    assert_eq!(reopened.search_files("計画").unwrap(), vec![id]);
}

#[test]
fn restore_conflict_keeps_both_files_and_retries_after_rename() {
    let (_, mut volume) = volume();
    let old = files::create(&mut volume, b"report.txt").unwrap();
    files::trash(&mut volume, &old).unwrap();
    let new = files::create(&mut volume, b"report.txt").unwrap();
    assert!(!same_identity_for_test(&old, &new));
    assert_eq!(
        files::restore(&mut volume, &old),
        Err(files::Error::Conflict)
    );
    assert_eq!(
        files::trash_entries(&mut volume).unwrap(),
        vec![old.clone()]
    );
    files::rename(&mut volume, &new, b"replacement.txt").unwrap();
    files::restore(&mut volume, &old).unwrap();
    assert_eq!(files::list(&mut volume).unwrap().len(), 2);
}
fn same_identity_for_test(a: &files::Entry, b: &files::Entry) -> bool {
    a.inode == b.inode && a.generation == b.generation
}

#[test]
fn duplicate_commands_are_idempotent_and_stale_selection_cannot_touch_reused_inode() {
    let (_, mut volume) = volume();
    let original = files::create(&mut volume, b"one.txt").unwrap();
    assert_eq!(
        files::create(&mut volume, b"one.txt"),
        Err(files::Error::Conflict)
    );
    assert_eq!(
        files::rename(&mut volume, &original, b"one.txt").unwrap(),
        original
    );
    for _ in 0..3 {
        files::trash(&mut volume, &original).unwrap();
    }
    for _ in 0..3 {
        files::restore(&mut volume, &original).unwrap();
    }
    volume
        .remove_path(&files::path(b"one.txt").unwrap())
        .unwrap();
    let replacement = files::create(&mut volume, b"one.txt").unwrap();
    assert_eq!(original.inode, replacement.inode);
    assert_ne!(original.generation, replacement.generation);
    assert_eq!(
        files::trash(&mut volume, &original),
        Err(files::Error::StaleSelection)
    );
    assert_eq!(
        files::rename(&mut volume, &original, b"oops.txt"),
        Err(files::Error::StaleSelection)
    );
    assert_eq!(
        files::restore(&mut volume, &original),
        Err(files::Error::StaleSelection)
    );
    assert_eq!(files::list(&mut volume).unwrap(), vec![replacement]);
}

#[test]
fn rename_conflict_and_utf8_maximum_name_keep_original_identity() {
    let (_, mut volume) = volume();
    let first = files::create(&mut volume, b"first.txt").unwrap();
    files::create(&mut volume, b"second.txt").unwrap();
    assert_eq!(
        files::rename(&mut volume, &first, b"second.txt"),
        Err(files::Error::Conflict)
    );
    let name = "ああああああああああab"; // Exactly 32 UTF-8 bytes.
    assert_eq!(name.len(), MAX_NAME_LENGTH);
    let renamed = files::rename(&mut volume, &first, name.as_bytes()).unwrap();
    assert!(same_identity_for_test(&first, &renamed));
    files::trash(&mut volume, &renamed).unwrap();
    files::restore(&mut volume, &renamed).unwrap();
    assert!(files::list(&mut volume).unwrap().contains(&renamed));
}

#[test]
fn invalid_names_reserved_namespace_and_directories_never_mutate() {
    let (_, mut volume) = volume();
    for name in [
        b"".as_slice(),
        b".",
        b"..",
        b"../secret",
        b"a/b",
        b"\0",
        b"\xff",
        b".nagi-trash-1-2",
        b"123456789012345678901234567890123",
    ] {
        assert_eq!(
            files::create(&mut volume, name),
            Err(files::Error::InvalidName)
        );
    }
    volume.mkdir_path(b"/home/owner/files/folder").unwrap();
    let metadata = volume.metadata_path(b"/home/owner/files/folder").unwrap();
    let folder = files::Entry {
        name: "folder".into(),
        inode: metadata.inode,
        generation: metadata.generation,
    };
    assert_eq!(
        files::trash(&mut volume, &folder),
        Err(files::Error::StaleSelection)
    );
    assert!(files::list(&mut volume).unwrap().is_empty());
}

#[test]
fn single_corrupt_journal_falls_back_without_losing_trash() {
    let (disk, mut volume) = volume();
    let entry = files::create(&mut volume, b"keep.txt").unwrap();
    files::trash(&mut volume, &entry).unwrap();
    let journal = volume.open_path(b"/home/owner/.files-trash-a").unwrap();
    volume.write(journal, b"torn slot").unwrap();
    let mut remounted = Vfs::mount_existing(disk).unwrap();
    assert_eq!(
        files::trash_entries(&mut remounted).unwrap(),
        vec![entry.clone()]
    );
    files::restore(&mut remounted, &entry).unwrap();
}

#[test]
fn corrupt_both_journals_fails_closed_without_removing_content_or_blocking_live_files() {
    let (_, mut volume) = volume();
    let entry = files::create(&mut volume, b"keep.txt").unwrap();
    files::trash(&mut volume, &entry).unwrap();
    for path in [
        b"/home/owner/.files-trash-a".as_slice(),
        b"/home/owner/.files-trash-b",
    ] {
        let handle = volume.open_path(path).unwrap();
        volume.write(handle, b"corrupt").unwrap();
    }
    assert_eq!(
        files::trash_entries(&mut volume),
        Err(files::Error::CorruptJournal)
    );
    assert_eq!(
        files::restore(&mut volume, &entry),
        Err(files::Error::CorruptJournal)
    );
    let live = files::create(&mut volume, b"ordinary.txt").unwrap();
    assert_eq!(files::list(&mut volume).unwrap(), vec![live]);
    assert!(volume
        .metadata_path(
            format!(
                "/home/owner/files/.nagi-trash-{:x}-{:x}",
                entry.inode, entry.generation
            )
            .as_bytes()
        )
        .is_ok());
}

#[test]
fn search_write_failure_hides_stale_metadata_but_files_remain_available_and_recover() {
    let (_, mut volume) = volume();
    let original = files::create(&mut volume, b"old.txt").unwrap();
    let backend = Backend::default();
    let mut search = runtime(backend.clone());
    search.sync_files(&mut volume).unwrap();
    let id = search.search_files("old.txt").unwrap()[0];
    backend.0.borrow_mut().1 = true;
    let renamed = files::rename(&mut volume, &original, b"new.txt").unwrap();
    assert_eq!(
        search.sync_files(&mut volume),
        Err(RuntimeError::SearchUnavailable)
    );
    assert!(search.search_files("old.txt").is_none());
    files::trash(&mut volume, &renamed).unwrap();
    files::restore(&mut volume, &renamed).unwrap();
    backend.0.borrow_mut().1 = false;
    search.sync_files(&mut volume).unwrap();
    assert_eq!(search.search_files("new.txt").unwrap(), vec![id]);
    assert!(search.search_files("old.txt").unwrap().is_empty());
}

#[test]
fn failed_journal_prepare_keeps_file_live_and_retry_recovers() {
    let (disk, mut volume) = volume();
    let original = files::create(&mut volume, b"keep.txt").unwrap();
    // Make both journal files exist before injecting a metadata write failure.
    files::trash(&mut volume, &original).unwrap();
    files::restore(&mut volume, &original).unwrap();
    disk.0.borrow_mut().fail_write_after = Some(0);
    assert!(files::trash(&mut volume, &original).is_err());
    let mut remounted = Vfs::mount_existing(disk).unwrap();
    assert!(files::list(&mut remounted).unwrap().contains(&original));
    files::trash(&mut remounted, &original).unwrap();
    files::restore(&mut remounted, &original).unwrap();
}

#[test]
fn flush_failure_during_trash_is_retryable_on_remount() {
    let (disk, mut volume) = volume();
    let original = files::create(&mut volume, b"keep.txt").unwrap();
    // Fail while preparing the journal; VFS recovery may retain the live file.
    disk.0.borrow_mut().fail_flush_after = Some(2);
    assert!(files::trash(&mut volume, &original).is_err());
    let mut remounted = Vfs::mount_existing(disk).unwrap();
    files::trash(&mut remounted, &original).unwrap();
    files::restore(&mut remounted, &original).unwrap();
    assert_eq!(files::list(&mut remounted).unwrap(), vec![original]);
}

#[test]
fn nested_search_and_foreign_app_visibility_remain_scoped() {
    let (_, mut volume) = volume();
    volume.mkdir_path(b"/home/owner/files/folder").unwrap();
    volume
        .create_path(b"/home/owner/files/folder/nested.txt")
        .unwrap();
    let mut search = runtime(Backend::default());
    search.sync_files(&mut volume).unwrap();
    assert_eq!(search.search_files("nested").unwrap().len(), 1);
    let foreign = AccessContext::for_application(
        AppId::from_identifier(b"org.nagi.foreign"),
        AppSessionId(42),
    );
    assert!(search
        .search_files_for_application(foreign, "nested")
        .is_none());
    let files = AccessContext::for_application(FILES_APP_ID, AppSessionId(42));
    assert_eq!(
        search
            .search_files_for_application(files, "nested")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn ui_keyboard_create_rename_trash_restore_and_repeat_have_concrete_requests() {
    let (_, mut volume) = volume();
    let mut panel = Panel::new();
    panel.open();
    panel.key(libnagi::INPUT_KEY_TAB); // List -> New
    panel.key(libnagi::INPUT_KEY_ENTER);
    assert!(panel.editing);
    panel.name = "new.txt".into();
    panel.key(libnagi::INPUT_KEY_ENTER);
    assert_eq!(panel.pending, Some(Operation::Create("new.txt".into())));
    panel.key(libnagi::INPUT_KEY_ENTER); // Ignore input until the request is consumed.
    let Some(Operation::Create(name)) = panel.pending.take() else {
        panic!("create")
    };
    let entry = files::create(&mut volume, name.as_bytes()).unwrap();
    panel.completed(Ok(()));
    panel.replace_entries(vec![entry.clone()]);
    panel.activate(Focus::Rename);
    panel.name = "renamed.txt".into();
    panel.key(libnagi::INPUT_KEY_ENTER);
    assert_eq!(
        panel.pending.take(),
        Some(Operation::Rename(entry.clone(), "renamed.txt".into()))
    );
    panel.completed(Ok(()));
    panel.activate(Focus::Trash);
    assert_eq!(panel.pending.take(), Some(Operation::Trash(entry.clone())));
    panel.completed(Ok(()));
    panel.key(libnagi::INPUT_KEY_ENTER);
    assert!(panel.pending.is_none());
    panel.activate(Focus::View);
    panel.replace_entries(vec![entry.clone()]);
    panel.activate(Focus::Trash);
    assert_eq!(panel.pending.take(), Some(Operation::Restore(entry)));
}

#[test]
fn ui_cancel_invalid_name_conflict_and_utf8_backspace_preserve_edit() {
    let mut panel = Panel::new();
    panel.open();
    panel.activate(Focus::New);
    panel.key(libnagi::INPUT_KEY_ENTER);
    assert!(panel.pending.is_none());
    assert_eq!(panel.status, "desktop.files.operation.invalid_name");
    panel.name = "名前".into();
    panel.key(libnagi::login::INPUT_KEY_BACKSPACE);
    assert_eq!(panel.name, "名");
    panel.completed(Err(files::Error::Conflict));
    assert!(panel.editing);
    assert_eq!(panel.name, "名");
    panel.key(libnagi::INPUT_KEY_ESCAPE);
    assert!(!panel.editing);
    assert!(panel.open);
    assert!(panel.pending.is_none());
    panel.key(libnagi::INPUT_KEY_ESCAPE);
    assert!(!panel.open);
}

#[test]
fn sector_write_failures_during_trash_and_restore_keep_a_recoverable_original() {
    // Test sector boundaries, including the first journal allocation, mirrored
    // prepares, and both halves of the VFS directory rename. This uses the real
    // VFS encoder/allocator and remounts after each injected failure.
    for restoring in [false, true] {
        for fail_after in 0..48 {
            let (disk, mut volume) = volume();
            let original = files::create(&mut volume, b"keep.txt").unwrap();
            let handle = volume
                .open_path(&files::path(b"keep.txt").unwrap())
                .unwrap();
            volume.write(handle, b"unique content").unwrap();
            if restoring {
                files::trash(&mut volume, &original).unwrap();
            }
            disk.0.borrow_mut().fail_write_after = Some(fail_after);
            let _ = if restoring {
                files::restore(&mut volume, &original)
            } else {
                files::trash(&mut volume, &original)
            };
            disk.0.borrow_mut().fail_write_after = None;
            let mut remounted = Vfs::mount_existing(disk).unwrap();
            let live = files::list(&mut remounted).unwrap();
            let trash = files::trash_entries(&mut remounted).unwrap_or_else(|error| {
                panic!("restore={restoring} fail_after={fail_after}: {error:?}")
            });
            assert_eq!(
                usize::from(live.contains(&original)) + usize::from(trash.contains(&original)),
                1,
                "restore={restoring} fail_after={fail_after}"
            );
            files::restore(&mut remounted, &original).unwrap();
            assert_eq!(contents(&mut remounted, "keep.txt"), b"unique content");
        }
    }
}

#[test]
fn real_guest_snapshot_backend_restores_trash_identity_on_same_vfs_disk() {
    use crate::m19_storage::SnapshotNamespace;
    let (disk, mut volume) = volume();
    let original = files::create(&mut volume, b"guest.txt").unwrap();
    let backend = GuestSnapshotBackend::new(
        VfsSnapshotFiles::new(
            Vfs::mount_existing(disk.clone()).unwrap(),
            SnapshotNamespace::Metadata,
        )
        .unwrap(),
    );
    let mut search = Runtime {
        service: SearchService::open(backend, OwnerFilesVisibility).unwrap(),
        synchronized: false,
    };
    search.sync_files(&mut volume).unwrap();
    let id = search.search_files("guest").unwrap()[0];
    files::trash(&mut volume, &original).unwrap();
    search.sync_files(&mut volume).unwrap();
    drop(search);
    let backend = GuestSnapshotBackend::new(
        VfsSnapshotFiles::new(
            Vfs::mount_existing(disk.clone()).unwrap(),
            SnapshotNamespace::Metadata,
        )
        .unwrap(),
    );
    let mut search = Runtime {
        service: SearchService::open(backend, OwnerFilesVisibility).unwrap(),
        synchronized: false,
    };
    let mut volume = Vfs::mount_existing(disk).unwrap();
    search.sync_files(&mut volume).unwrap();
    assert!(search.search_files("guest").unwrap().is_empty());
    files::restore(&mut volume, &original).unwrap();
    search.sync_files(&mut volume).unwrap();
    assert_eq!(search.search_files("guest").unwrap(), vec![id]);
}

#[test]
fn trash_capacity_failure_never_removes_the_selected_live_file() {
    let (_, mut volume) = volume();
    for index in 0..8 {
        let entry = files::create(&mut volume, format!("file{index}.txt").as_bytes()).unwrap();
        files::trash(&mut volume, &entry).unwrap();
    }
    let ninth = files::create(&mut volume, b"ninth.txt").unwrap();
    assert_eq!(
        files::trash(&mut volume, &ninth),
        Err(files::Error::Capacity)
    );
    assert!(files::list(&mut volume).unwrap().contains(&ninth));
    assert_eq!(files::trash_entries(&mut volume).unwrap().len(), 8);
}

#[test]
fn search_capacity_cannot_block_ordinary_create_rename_trash_restore() {
    let (_, mut volume) = volume();
    let mut search = runtime(Backend::default());
    for index in 0..9 {
        files::create(&mut volume, format!("file{index}.txt").as_bytes()).unwrap();
    }
    assert_eq!(
        search.sync_files(&mut volume),
        Err(RuntimeError::TooManyFiles)
    );
    assert!(search.search_files("file").is_none());
    let selected = files::list(&mut volume).unwrap().remove(0);
    let renamed = files::rename(&mut volume, &selected, b"ordinary.txt").unwrap();
    files::trash(&mut volume, &renamed).unwrap();
    files::restore(&mut volume, &renamed).unwrap();
    assert_eq!(files::list(&mut volume).unwrap().len(), 9);
}

#[test]
fn runtime_mutation_flush_failure_denies_stale_queries_until_reconciliation() {
    let (disk, mut volume) = volume();
    files::create(&mut volume, b"old.txt").unwrap();
    let mut search = runtime(Backend::default());
    search.sync_files(&mut volume).unwrap();
    let id = search.search_files("old.txt").unwrap()[0];
    // The five VFS transaction barriers succeed; the runtime trailing flush fails.
    disk.0.borrow_mut().fail_flush_after = Some(5);
    assert_eq!(
        search.rename_file(&mut volume, b"old.txt", b"new.txt"),
        Err(RuntimeError::Storage)
    );
    assert!(search.search_files("old.txt").is_none());
    search.sync_files(&mut volume).unwrap();
    assert_eq!(search.search_files("new.txt").unwrap(), vec![id]);
    disk.0.borrow_mut().fail_flush_after = Some(5);
    assert_eq!(
        search.create_file(&mut volume, b"created.txt"),
        Err(RuntimeError::Storage)
    );
    assert!(search.search_files("new.txt").is_none());
    search.sync_files(&mut volume).unwrap();
    disk.0.borrow_mut().fail_flush_after = Some(0);
    assert_eq!(
        search.delete_file(&mut volume, b"created.txt"),
        Err(RuntimeError::Storage)
    );
    assert!(search.search_files("created.txt").is_none());
    search.sync_files(&mut volume).unwrap();
    assert!(search.search_files("created.txt").unwrap().is_empty());
}

#[test]
fn crowded_files_directory_rename_trash_restart_restore_keeps_all_neighbors() {
    let (disk, mut volume) = volume();
    for index in 0..14 {
        files::create(
            &mut volume,
            format!("{index:02}{}", "x".repeat(30)).as_bytes(),
        )
        .unwrap();
    }
    let originals = files::list(&mut volume).unwrap();
    let renamed = files::rename(&mut volume, &originals[0], b"renamed").unwrap();
    files::trash(&mut volume, &renamed).unwrap();
    let mut remounted = Vfs::mount_existing(disk.clone()).unwrap();
    files::restore(&mut remounted, &renamed).unwrap();
    let live = files::list(&mut remounted).unwrap();
    assert_eq!(live.len(), 14);
    assert!(live.contains(&renamed));
    for neighbor in &originals[1..] {
        assert!(live.contains(neighbor));
    }
    Vfs::check_existing(&mut disk.clone()).unwrap();
}

#[test]
fn crowded_files_interruption_preserves_every_file_and_restore_identity() {
    for restoring in [false, true] {
        for fail_after in 0..48 {
            let (disk, mut volume) = volume();
            for index in 0..14 {
                let name = format!("{index:02}{}", "x".repeat(30));
                let entry = files::create(&mut volume, name.as_bytes()).unwrap();
                let handle = volume
                    .open_path(&files::path(entry.name.as_bytes()).unwrap())
                    .unwrap();
                volume.write(handle, name.as_bytes()).unwrap();
            }
            let originals = files::list(&mut volume).unwrap();
            let selected = &originals[0];
            if restoring {
                files::trash(&mut volume, selected).unwrap();
            }
            disk.0.borrow_mut().fail_write_after = Some(fail_after);
            let _ = if restoring {
                files::restore(&mut volume, selected)
            } else {
                files::trash(&mut volume, selected)
            };
            disk.0.borrow_mut().fail_write_after = None;
            let mut remounted = Vfs::mount_existing(disk).unwrap();
            files::restore(&mut remounted, selected).unwrap();
            assert_eq!(
                files::list(&mut remounted).unwrap(),
                originals,
                "restore={restoring} fail_after={fail_after}"
            );
            for entry in &originals {
                assert_eq!(contents(&mut remounted, &entry.name), entry.name.as_bytes());
            }
        }
    }
}

#[test]
fn rename_and_trash_preserve_hidden_non_utf8_and_directory_records() {
    let (_, mut volume) = volume();
    let selected = files::create(&mut volume, b"short").unwrap();
    for index in 1..=11 {
        let mut name = [0xff; 32];
        name[0] = index;
        let mut path = files::ROOT.to_vec();
        path.push(b'/');
        path.extend_from_slice(&name);
        volume.create_path(&path).unwrap();
    }
    volume.mkdir_path(b"/home/owner/files/long-folder").unwrap();
    assert_eq!(files::list(&mut volume).unwrap(), vec![selected.clone()]);
    let renamed = files::rename(&mut volume, &selected, &[b'x'; 32]).unwrap();
    files::trash(&mut volume, &renamed).unwrap();
    files::restore(&mut volume, &renamed).unwrap();
    assert_eq!(files::list(&mut volume).unwrap(), vec![renamed]);
    let mut physical = [DirectoryEntry::empty(); 64];
    assert_eq!(
        volume
            .list_directory_path(files::ROOT, &mut physical)
            .unwrap(),
        13
    );
}

#[test]
fn crowded_restore_keeps_original_identity_and_existing_live_entries() {
    let (disk, mut volume) = volume();
    let selected = files::create(&mut volume, &[b'x'; 32]).unwrap();
    files::trash(&mut volume, &selected).unwrap();
    for index in 0..12 {
        files::create(
            &mut volume,
            format!("{index:02}{}", "y".repeat(30)).as_bytes(),
        )
        .unwrap();
    }
    let live = files::list(&mut volume).unwrap();
    files::restore(&mut volume, &selected).unwrap();
    let mut remounted = Vfs::mount_existing(disk.clone()).unwrap();
    let restored = files::list(&mut remounted).unwrap();
    assert_eq!(restored.len(), live.len() + 1);
    assert!(restored.contains(&selected));
    for entry in live {
        assert!(restored.contains(&entry));
    }
    assert!(files::trash_entries(&mut remounted).unwrap().is_empty());
    Vfs::check_existing(&mut disk.clone()).unwrap();
}

#[test]
fn creation_beyond_twelve_children_keeps_trash_original_restorable() {
    let (disk, mut volume) = volume();
    let selected = files::create(&mut volume, &[b'x'; 32]).unwrap();
    files::trash(&mut volume, &selected).unwrap();
    for index in 0..13 {
        files::create(
            &mut volume,
            format!("{index:02}{}", "y".repeat(30)).as_bytes(),
        )
        .unwrap();
    }
    files::restore(&mut volume, &selected).unwrap();
    assert_eq!(files::list(&mut volume).unwrap().len(), 14);
    Vfs::check_existing(&mut disk.clone()).unwrap();
}

#[test]
fn interrupted_create_never_aliases_the_next_inode() {
    let (disk, mut volume) = volume();
    let kept = files::create(&mut volume, b"kept.txt").unwrap();
    disk.0.borrow_mut().fail_write_after = Some(7);
    assert_eq!(
        files::create(&mut volume, b"broken.txt"),
        Err(files::Error::Storage)
    );
    let mut remounted = Vfs::mount_existing(disk.clone()).unwrap();
    let next = files::create(&mut remounted, b"next.txt").unwrap();
    if let Ok(broken) = remounted.metadata_path(&files::path(b"broken.txt").unwrap()) {
        assert_ne!(
            next.inode, broken.inode,
            "failed create must not publish an inode that the next file reuses"
        );
    }
    assert!(Vfs::check_existing(&mut disk.clone()).is_ok());
    assert!(files::list(&mut remounted).unwrap().contains(&kept));
}
