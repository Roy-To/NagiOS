use nagi_files::{FileName, FilesystemProvider, Location};
use nagi_storage_consumer_adapters::SettingsSaveOutcome;
use nagi_storage_consumer_adapters::{FilesVfsProvider, SettingsStorageAdapter};
use nagi_storage_provider::{
    fail_flush_after_for_test, StorageError, StorageProviderOwner, MIN_VFS_BYTES,
};
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_disk() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nagi-storage-consumers-{}-{stamp}.img",
        std::process::id()
    ))
}

#[test]
fn files_and_settings_adapters_share_one_serialized_vfs_volume() {
    let disk = temp_disk();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk)
        .expect("create disposable provider disk");
    file.set_len(MIN_VFS_BYTES)
        .expect("size disposable provider disk");
    file.sync_all().expect("flush disposable provider disk");
    let owner = StorageProviderOwner::format_blank_file(file).expect("format blank test volume");
    let storage = owner.consumer_handle();

    let mut files = FilesVfsProvider::new(storage.clone());
    files.initialize().expect("initialize Files namespace");
    let folder = FileName::parse("docs").expect("valid folder name");
    let created = files
        .create_folder(&Location::root(), &folder)
        .expect("create Files folder through provider");
    assert_eq!(created.kind, nagi_files::EntryKind::Folder);

    let handle = storage
        .create_path(b"/files/docs/report.txt")
        .expect("create report through common provider");
    storage
        .write(handle, b"same provider payload")
        .expect("write report through common provider");
    storage.flush().expect("durably acknowledge report");
    let report = Location::parse("docs/report.txt").expect("valid provider-relative location");
    let contents = files
        .read_file(&report, 1024)
        .expect("read through Files adapter");
    assert_eq!(contents, b"same provider payload");

    let settings = SettingsStorageAdapter::new(storage.clone());
    settings
        .save_system_language(b"ja-JP")
        .expect("persist locale through the same provider");
    assert_eq!(
        settings
            .load_system_language()
            .expect("read persisted locale"),
        Some(b"ja-JP".to_vec())
    );

    let entries = files.list(&Location::root()).expect("enumerate Files root");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name.as_str(), "docs");

    assert!(Location::parse("../outside").is_err());
    drop(settings);
    drop(files);
    drop(storage);
    drop(owner);
    std::fs::remove_file(disk).expect("remove disposable provider disk");
}

#[test]
fn files_initialize_retries_flush_for_a_directory_created_before_failure() {
    let disk = temp_disk();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk)
        .expect("create disposable provider disk");
    file.set_len(MIN_VFS_BYTES)
        .expect("size disposable provider disk");
    file.sync_all().expect("flush disposable provider disk");
    let owner = StorageProviderOwner::format_blank_file(file).expect("format blank volume");
    let storage = owner.consumer_handle();
    let files = FilesVfsProvider::new(storage.clone());

    fail_flush_after_for_test(0);
    assert!(
        files.initialize().is_err(),
        "a failed flush after mkdir must not be reported as successful initialization"
    );
    assert_eq!(
        storage
            .metadata_path(b"/files")
            .expect("directory creation is visible in the live VFS")
            .mode
            & 0xf000,
        0x4000
    );

    fail_flush_after_for_test(0);
    assert!(
        files.initialize().is_err(),
        "an existing directory must still pass through the durability boundary"
    );
    files
        .initialize()
        .expect("retry flush for existing Files root");

    drop(files);
    drop(storage);
    drop(owner);
    std::fs::remove_file(disk).expect("remove disposable provider disk");
}

#[test]
fn settings_prepared_replacement_preserves_old_and_reports_unknown_after_publish() {
    let disk = temp_disk();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk)
        .expect("create disposable provider disk");
    file.set_len(MIN_VFS_BYTES)
        .expect("size disposable provider disk");
    file.sync_all().expect("flush disposable provider disk");
    let owner = StorageProviderOwner::format_blank_file(file).expect("format blank volume");
    let storage = owner.consumer_handle();
    let settings = SettingsStorageAdapter::new(storage.clone());

    storage
        .create_path(b"system-language")
        .expect("create an empty legacy locale file");
    storage.flush().expect("flush empty legacy locale file");
    assert_eq!(
        settings
            .load_system_language()
            .expect("empty is no setting"),
        None
    );

    assert_eq!(
        settings
            .save_system_language(b"en-US")
            .expect("save first locale"),
        SettingsSaveOutcome::New
    );

    fail_flush_after_for_test(0);
    let before_publish = settings
        .save_system_language(b"ja-JP")
        .expect_err("prepared-file flush failure must not report Saved");
    assert_eq!(before_publish.outcome, SettingsSaveOutcome::Old);
    assert_eq!(
        settings
            .load_system_language()
            .expect("old locale remains readable before publish"),
        Some(b"en-US".to_vec())
    );

    fail_flush_after_for_test(1);
    let after_publish = settings
        .save_system_language(b"ja-JP")
        .expect_err("publish flush failure has no durable acknowledgement");
    assert_eq!(after_publish.outcome, SettingsSaveOutcome::Unknown);
    assert_eq!(
        settings
            .load_system_language()
            .expect("replacement is visible in the live VFS"),
        Some(b"ja-JP".to_vec())
    );

    drop(settings);
    drop(storage);
    drop(owner);
    std::fs::remove_file(disk).expect("remove disposable provider disk");
}

#[test]
fn failed_first_settings_save_remains_unset_until_explicit_save() {
    let disk = temp_disk();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk)
        .expect("create disposable provider disk");
    file.set_len(MIN_VFS_BYTES)
        .expect("size disposable provider disk");
    file.sync_all().expect("flush disposable provider disk");
    let owner = StorageProviderOwner::format_blank_file(file).expect("format blank volume");
    let storage = owner.consumer_handle();
    let settings = SettingsStorageAdapter::new(storage.clone());

    assert_eq!(settings.load_system_language().unwrap(), None);
    fail_flush_after_for_test(0);
    let failed = settings
        .save_system_language(b"ja-JP")
        .expect_err("first prepared-file flush failure must not report Saved");
    assert_eq!(failed.outcome, SettingsSaveOutcome::Old);
    assert_eq!(
        settings.load_system_language().unwrap(),
        None,
        "empty target placeholder is still an unset preference"
    );

    assert_eq!(
        settings
            .save_system_language(b"ja-JP")
            .expect("explicit second save commits the preference"),
        SettingsSaveOutcome::New
    );
    assert_eq!(
        settings.load_system_language().unwrap(),
        Some(b"ja-JP".to_vec())
    );

    drop(settings);
    drop(storage);
    drop(owner);
    std::fs::remove_file(disk).expect("remove disposable provider disk");
}

#[test]
fn files_and_settings_reads_wait_for_a_complete_serialized_update() {
    let disk = temp_disk();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk)
        .expect("create disposable provider disk");
    file.set_len(MIN_VFS_BYTES)
        .expect("size disposable provider disk");
    file.sync_all().expect("flush disposable provider disk");
    let owner = StorageProviderOwner::format_blank_file(file).expect("format blank volume");
    let storage = owner.consumer_handle();
    let mut files = FilesVfsProvider::new(storage.clone());
    files.initialize().expect("initialize Files namespace");
    let folder = FileName::parse("docs").expect("valid folder name");
    files
        .create_folder(&Location::root(), &folder)
        .expect("create concurrent-read fixture folder");
    let file_handle = storage
        .create_path(b"/files/docs/concurrent.txt")
        .expect("create concurrent-read fixture file");
    storage
        .write(file_handle, b"old-file")
        .expect("write original Files contents");
    let settings = SettingsStorageAdapter::new(storage.clone());
    settings
        .save_system_language(b"en-US")
        .expect("write original Settings value");
    let language_handle = storage
        .open_path(b"system-language")
        .expect("open current Settings value");
    storage.flush().expect("flush initial values");

    let (locked_tx, locked_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let writer_storage = storage.clone();
    let writer = thread::spawn(move || {
        writer_storage.with_volume(|volume| {
            volume.truncate(file_handle, 0)?;
            volume.truncate(language_handle, 0)?;
            locked_tx.send(()).map_err(|_| StorageError::Block)?;
            continue_rx.recv().map_err(|_| StorageError::Block)?;
            volume.write(file_handle, b"new-file-with-complete-length")?;
            volume.write(language_handle, b"ja-JP")?;
            volume.flush()
        })
    });
    locked_rx.recv().expect("writer holds provider transaction");

    let files_reader = FilesVfsProvider::new(storage.clone());
    let settings_reader = SettingsStorageAdapter::new(storage.clone());
    let file_location = Location::parse("docs/concurrent.txt").expect("valid file location");
    let (files_started_tx, files_started_rx) = mpsc::channel();
    let (files_done_tx, files_done_rx) = mpsc::channel();
    let files_thread = thread::spawn(move || {
        files_started_tx.send(()).unwrap();
        files_done_tx
            .send(files_reader.read_file(&file_location, 1024))
            .unwrap();
    });
    let (settings_started_tx, settings_started_rx) = mpsc::channel();
    let (settings_done_tx, settings_done_rx) = mpsc::channel();
    let settings_thread = thread::spawn(move || {
        settings_started_tx.send(()).unwrap();
        settings_done_tx
            .send(settings_reader.load_system_language())
            .unwrap();
    });

    files_started_rx.recv().unwrap();
    settings_started_rx.recv().unwrap();
    assert!(files_done_rx.try_recv().is_err());
    assert!(settings_done_rx.try_recv().is_err());
    continue_tx.send(()).expect("release writer transaction");
    writer.join().unwrap().expect("writer completes update");
    assert_eq!(
        files_done_rx.recv().unwrap().unwrap(),
        b"new-file-with-complete-length"
    );
    assert_eq!(
        settings_done_rx.recv().unwrap().unwrap(),
        Some(b"ja-JP".to_vec())
    );
    files_thread.join().unwrap();
    settings_thread.join().unwrap();

    drop(settings);
    drop(files);
    drop(storage);
    drop(owner);
    std::fs::remove_file(disk).expect("remove disposable provider disk");
}
