use crate::domain::{Block, BlockKind, NoteDocument, Timestamp};
use crate::identity::HostObjectIdSource;
use crate::markdown::{decode_stored, encode_stored, MarkdownError};
use crate::store::{
    load_note_from_files_entry, FilesEntryLoadError, HostPreviewStore, NoteStore, StoreError,
};
use nagi_files::{
    Actor, CapabilityGrant, CapabilityRight, CapabilitySet, FilesErrorKind, FilesService,
    InMemoryProvider, Location, MAX_PREVIEW_BYTES,
};
use nagi_model::{ObjectId, WorkspaceId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// Exact fixed source value: inputs/nagi-notes/src/store.rs:16.
const NOTES_STORE_LIMIT_BYTES: usize = 16 * 1024 * 1024;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn test_root(label: &str) -> PathBuf {
    let base = std::env::var_os("CODEC_CANDIDATE_TMP")
        .map(PathBuf::from)
        .expect("runner sets CODEC_CANDIDATE_TMP inside codec-candidate");
    let path = base.join(format!(
        "{label}-{}-{}",
        std::process::id(),
        TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("create isolated candidate test root");
    path
}

fn note_with_body(id: u64, text: String) -> NoteDocument {
    let mut note = NoteDocument::new(ObjectId(id), "Codec boundary", Timestamp(10));
    note.workspace_id = Some(WorkspaceId(9));
    note.workspace_ids = vec![WorkspaceId(9), WorkspaceId(10)];
    note.tags = vec!["codec".to_owned()];
    note.blocks
        .push(Block::new(ObjectId(id + 1), BlockKind::Paragraph(text)));
    note
}

fn load_with_files_entry(
    bytes: &[u8],
    path: &str,
    expected_note_id: ObjectId,
) -> Result<NoteDocument, FilesEntryLoadError> {
    let root = Location::root();
    let location = Location::parse(path).expect("valid provider-relative location");
    let mut provider = InMemoryProvider::new();
    provider
        .insert_file(&location, bytes)
        .expect("seed actual Files provider");
    let authorizer = CapabilitySet::from_grants([
        CapabilityGrant::allow(root.clone(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(root.clone(), CapabilityRight::Read),
    ]);
    let mut service = FilesService::new(provider, authorizer);
    let entries = service.list(&root).expect("list is authorized");
    let entry = entries
        .iter()
        .find(|entry| entry.name.as_str() == path)
        .expect("listed file");
    load_note_from_files_entry(
        &mut service,
        entry,
        expected_note_id,
        Actor::User,
        &HostObjectIdSource::new(),
    )
}

fn open_with_files(bytes: &[u8], path: &str) -> Result<Vec<u8>, FilesErrorKind> {
    let root = Location::root();
    let location = Location::parse(path).expect("valid provider-relative location");
    let mut provider = InMemoryProvider::new();
    provider
        .insert_file(&location, bytes)
        .expect("seed actual Files provider");
    let authorizer = CapabilitySet::from_grants([
        CapabilityGrant::allow(root.clone(), CapabilityRight::Enumerate),
        CapabilityGrant::allow(root.clone(), CapabilityRight::Read),
    ]);
    let mut service = FilesService::new(provider, authorizer);
    let entries = service.list(&root).expect("enumeration is granted");
    let entry = entries
        .iter()
        .find(|entry| entry.name.as_str() == path)
        .expect("listed file");
    service
        .open_file(entry, Actor::User)
        .map(|(contents, _)| contents)
        .map_err(|error| error.kind)
}

#[test]
fn actual_notes_codec_roundtrips_model_identity_revision_and_content() {
    let mut note = note_with_body(0x1234, "first line\nsecond line — 日本語".to_owned());
    note.revision = 1; // Stored-note decoding rejects the unpersisted revision zero.
    note.blocks.push(Block::new(
        ObjectId(0x1236),
        BlockKind::Checklist {
            text: "review".to_owned(),
            completed: true,
        },
    ));
    let encoded = encode_stored(&note);
    let decoded =
        decode_stored(&encoded, &HostObjectIdSource::new()).expect("actual Notes decoder");
    assert_eq!(decoded, note);
    assert_eq!(decoded.id, ObjectId(0x1234));
    assert_eq!(decoded.blocks[0].id, ObjectId(0x1235));
    assert_eq!(decoded.revision, 1);
}

#[test]
fn stored_codec_rejects_unpersisted_revision_zero() {
    let note = note_with_body(0x1236, "not yet committed".to_owned());
    assert_eq!(
        decode_stored(&encode_stored(&note), &HostObjectIdSource::new()).unwrap_err(),
        MarkdownError::InvalidField("revision"),
    );
}

#[test]
fn supplied_files_entry_and_note_id_load_through_actual_notes_store_boundary() {
    let mut note = note_with_body(
        0x2345,
        "Notes bytes cross Files without an ID cast".to_owned(),
    );
    note.revision = 1;
    let expected = note.clone();
    let stored = encode_stored(&note).into_bytes();
    let loaded = load_with_files_entry(&stored, "note.md", expected.id)
        .expect("actual Files read and Notes load decode");
    assert_eq!(loaded, expected);
}

#[test]
fn files_entry_with_different_note_id_is_rejected_by_notes_load_boundary() {
    let mut note = note_with_body(0x2347, "identity stays explicit".to_owned());
    note.revision = 1;
    let stored = encode_stored(&note).into_bytes();
    let error = load_with_files_entry(&stored, "mapped.md", ObjectId(0x9999))
        .expect_err("a caller-supplied NoteId must match stored content");
    assert!(matches!(
        error,
        FilesEntryLoadError::Notes(StoreError::Corrupt(message))
            if message == "file identity does not match requested note id"
    ));
}

#[test]
fn files_returns_invalid_utf8_as_bytes_and_notes_codec_requires_valid_utf8() {
    let raw = [0xff, 0x00, 0x61];
    let opened = open_with_files(&raw, "invalid.md").expect("Files API is byte-oriented");
    assert_eq!(opened, raw);
    let utf8_error =
        String::from_utf8(opened).expect_err("invalid bytes cannot reach the str codec");
    assert_eq!(utf8_error.utf8_error().valid_up_to(), 0);
    assert!(matches!(
        load_with_files_entry(&raw, "invalid-again.md", ObjectId(0x3010)),
        Err(FilesEntryLoadError::Notes(StoreError::Corrupt(_)))
    ));
}

#[test]
fn files_read_denial_remains_a_files_error_at_notes_load_boundary() {
    let root = Location::root();
    let path = Location::parse("private.md").unwrap();
    let mut provider = InMemoryProvider::new();
    provider.insert_file(&path, b"not opened").unwrap();
    let authorizer = CapabilitySet::from_grants([CapabilityGrant::allow(
        root.clone(),
        CapabilityRight::Enumerate,
    )]);
    let mut service = FilesService::new(provider, authorizer);
    let entry = service.list(&root).unwrap().remove(0);
    let error = load_note_from_files_entry(
        &mut service,
        &entry,
        ObjectId(0x3011),
        Actor::User,
        &HostObjectIdSource::new(),
    )
    .expect_err("missing Read grant is still enforced by Files");
    assert!(matches!(
        error,
        FilesEntryLoadError::Files(files_error)
            if files_error.kind == FilesErrorKind::PermissionDenied
    ));
}

#[test]
fn files_preview_limit_is_inclusive_at_one_mib_and_rejects_one_byte_more() {
    let exact = vec![b'x'; MAX_PREVIEW_BYTES];
    assert_eq!(open_with_files(&exact, "exact.md").unwrap(), exact);
    let above = vec![b'x'; MAX_PREVIEW_BYTES + 1];
    assert_eq!(
        open_with_files(&above, "above.md").unwrap_err(),
        FilesErrorKind::FileTooLarge
    );
}

// These integration cases exercise HostPreviewStore's Unix directory-sync
// contract. The codec and pre-write size rejection still run on Windows.
#[cfg(unix)]
#[test]
fn notes_host_store_accepts_its_existing_sixteen_mib_range_that_files_cannot_open() {
    let note = note_with_body(0x3456, "x".repeat(MAX_PREVIEW_BYTES + 4096));
    let stored = encode_stored(&note).into_bytes();
    assert!(stored.len() > MAX_PREVIEW_BYTES);
    assert!(stored.len() < NOTES_STORE_LIMIT_BYTES);

    let root = test_root("large-notes-store");
    let store = HostPreviewStore::open(&root).expect("actual Notes host preview store");
    let saved = store.commit(&note, 0).expect("under Notes 16 MiB limit");
    assert_eq!(saved.id, note.id);
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load(saved.id).unwrap().unwrap(), saved);
    assert_eq!(
        open_with_files(&stored, "large-note.md").unwrap_err(),
        FilesErrorKind::FileTooLarge
    );
    std::fs::remove_dir_all(root).expect("remove candidate-only test data");
}

#[cfg(unix)]
#[test]
fn host_store_keeps_actual_revision_and_immutable_revision_read_semantics() {
    let root = test_root("revision-roundtrip");
    let store = HostPreviewStore::open(&root).expect("actual Notes host store");
    let first = note_with_body(0x4567, "revision one".to_owned());
    let saved1 = store.commit(&first, 0).expect("first actual commit");
    assert_eq!(saved1.revision, 1);

    let mut second = saved1.clone();
    second.title = "revision two".to_owned();
    second.blocks[0].kind = BlockKind::Paragraph("changed body".to_owned());
    second.updated_at = Timestamp(20);
    let saved2 = store.commit(&second, 1).expect("second actual commit");
    assert_eq!(saved2.id, first.id);
    assert_eq!(saved2.revision, 2);
    assert_eq!(store.load(first.id).unwrap().unwrap(), saved2);
    assert_eq!(store.load_revision(first.id, 1).unwrap().unwrap(), saved1);
    assert_eq!(store.load_revision(first.id, 2).unwrap().unwrap(), saved2);
    assert!(store.load_revision(first.id, 3).unwrap().is_none());
    assert_eq!(store.list(false).unwrap()[0].id, first.id);
    std::fs::remove_dir_all(root).expect("remove candidate-only test data");
}

#[test]
fn host_store_rejects_encoded_notes_over_its_existing_limit() {
    let note = note_with_body(0x5678, "x".repeat(NOTES_STORE_LIMIT_BYTES + 1));
    assert!(encode_stored(&note).len() > NOTES_STORE_LIMIT_BYTES);
    let root = test_root("over-notes-store-limit");
    let store = HostPreviewStore::open(&root).expect("actual Notes host store");
    assert_eq!(
        store.commit(&note, 0).unwrap_err(),
        StoreError::InvalidDocument("note exceeds 16 MiB"),
    );
    std::fs::remove_dir_all(root).expect("remove candidate-only test data");
}
