use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nagi_model::{AppSessionId, ObjectId, WorkspaceId};
use nagi_notes::{
    ActionPolicyError, ActionPrincipal, ActionResult, ActivityError, ActivityEvent, ActivityKind,
    ActivityOrigin, ActivitySink, AlbertReference, AppError, Block, BlockKind, Clock,
    HostPreviewStore, InMemoryNoteStore, Locale, Localizer, NoteDocument, NoteStore, NotesAction,
    NotesActionExecutor, NotesActionKind, NotesActionPolicy, NotesApp, NotesSearchProvider,
    ObjectIdSource, SearchProvider, StoreError, Timestamp,
};

struct SequenceIds(AtomicU64);

impl SequenceIds {
    fn new(start: u64) -> Self {
        Self(AtomicU64::new(start))
    }
}

impl ObjectIdSource for SequenceIds {
    fn next_object_id(&self) -> ObjectId {
        ObjectId(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

struct TestClock(AtomicU64);

impl TestClock {
    fn new() -> Self {
        Self(AtomicU64::new(10_000))
    }
}

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Default)]
struct RecordingActivity(Mutex<Vec<ActivityEvent>>);

impl ActivitySink for RecordingActivity {
    fn record(&self, event: ActivityEvent) -> Result<(), ActivityError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct FailingActivity;

impl ActivitySink for FailingActivity {
    fn record(&self, _event: ActivityEvent) -> Result<(), ActivityError> {
        Err(ActivityError("activity service unavailable".to_owned()))
    }
}

#[derive(Default)]
struct ReentrantActivity {
    app: Mutex<Option<Weak<NotesApp>>>,
    attempts: Mutex<Vec<Result<(), AppError>>>,
}

impl ActivitySink for ReentrantActivity {
    fn record(&self, event: ActivityEvent) -> Result<(), ActivityError> {
        if event.kind == ActivityKind::Edited {
            if let Some(app) = self.app.lock().unwrap().as_ref().and_then(Weak::upgrade) {
                self.attempts
                    .lock()
                    .unwrap()
                    .push(app.open_note(event.object_id).map(|_| ()));
            }
        }
        Ok(())
    }
}

struct SelectivePolicy {
    denied_note: Option<ObjectId>,
    deny_writes: bool,
}

impl NotesActionPolicy for SelectivePolicy {
    fn authorize(
        &self,
        _principal: ActionPrincipal,
        action: NotesActionKind,
        object_id: Option<ObjectId>,
    ) -> Result<(), ActionPolicyError> {
        if object_id.is_some() && object_id == self.denied_note {
            return Err(ActionPolicyError::Denied);
        }
        if self.deny_writes
            && matches!(
                action,
                NotesActionKind::AppendBlock
                    | NotesActionKind::InsertBlock
                    | NotesActionKind::UpdateBlock
                    | NotesActionKind::DeleteBlock
                    | NotesActionKind::MoveBlock
                    | NotesActionKind::AddReference
                    | NotesActionKind::RemoveReference
                    | NotesActionKind::SetTags
                    | NotesActionKind::AddToWorkspace
                    | NotesActionKind::RemoveFromWorkspace
            )
        {
            return Err(ActionPolicyError::Denied);
        }
        Ok(())
    }
}

struct FailingOnceStore {
    inner: InMemoryNoteStore,
    failures: AtomicUsize,
}

impl FailingOnceStore {
    fn new() -> Self {
        Self {
            inner: InMemoryNoteStore::new(),
            failures: AtomicUsize::new(1),
        }
    }
}

impl NoteStore for FailingOnceStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<nagi_notes::NoteSummary>, StoreError> {
        self.inner.list(include_deleted)
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.inner.load(id)
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.inner.load_revision(id, revision)
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        if self
            .failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(StoreError::Io("injected one-time write failure".to_owned()));
        }
        self.inner.commit(note, expected_revision)
    }
}

struct BlockingStore {
    inner: InMemoryNoteStore,
    should_block: AtomicBool,
    entered: Mutex<Option<Sender<()>>>,
    release: Mutex<Receiver<()>>,
}

impl BlockingStore {
    fn new(entered: Sender<()>, release: Receiver<()>) -> Self {
        Self {
            inner: InMemoryNoteStore::new(),
            should_block: AtomicBool::new(true),
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(release),
        }
    }
}

impl NoteStore for BlockingStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<nagi_notes::NoteSummary>, StoreError> {
        self.inner.list(include_deleted)
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.inner.load(id)
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.inner.load_revision(id, revision)
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        if self.should_block.swap(false, Ordering::SeqCst) {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
            }
            self.release.lock().unwrap().recv().unwrap();
        }
        self.inner.commit(note, expected_revision)
    }
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-test-{}-{time}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_app(
    store: Arc<dyn NoteStore>,
    activity: Arc<dyn ActivitySink>,
    debounce: Duration,
) -> NotesApp {
    NotesApp::with_providers(
        store,
        Arc::new(SequenceIds::new(100)),
        Arc::new(TestClock::new()),
        activity,
        debounce,
    )
}

fn paragraph_text(note: &NoteDocument) -> Vec<String> {
    note.blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::Paragraph(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn creates_edits_saves_closes_and_reopens_with_stable_identities() {
    let store = Arc::new(InMemoryNoteStore::new());
    let activity = Arc::new(RecordingActivity::default());
    let app = make_app(store.clone(), activity.clone(), Duration::from_secs(30));
    let session = app.create_note("会議メモ").unwrap();
    let note_id = session.note().id;
    let block_id = app.new_block_id();
    session
        .append_block(Block::new(
            block_id,
            BlockKind::Paragraph("日本語の本文".to_owned()),
        ))
        .unwrap();
    session
        .set_tags(vec!["Project".to_owned(), "project".to_owned()])
        .unwrap();
    session.set_workspace(Some(WorkspaceId(77))).unwrap();
    assert!(session.is_dirty());
    let saved = session.flush().unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.tags, vec!["project"]);
    assert_eq!(saved.workspace_id, Some(WorkspaceId(77)));
    assert!(!session.is_dirty());
    app.close_note(note_id).unwrap();

    let reopened = app.open_note(note_id).unwrap();
    assert_eq!(reopened.note().id, note_id);
    assert_eq!(reopened.note().blocks[0].id, block_id);
    assert_eq!(reopened.note().search_text(), "日本語の本文");
    assert_eq!(reopened.note().revision, 1);
    assert_eq!(
        activity.0.lock().unwrap().last().unwrap().kind,
        ActivityKind::Opened
    );
    app.close_note(note_id).unwrap();
}

#[test]
fn failed_save_keeps_dirty_content_and_retry_persists_it() {
    let store = Arc::new(FailingOnceStore::new());
    let app = make_app(
        store,
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("retry me").unwrap();
    assert!(session.flush().is_err());
    assert!(session.is_dirty());
    assert!(matches!(
        session.status(),
        nagi_notes::SaveStatus::Failed(_)
    ));
    let saved = session.retry().unwrap();
    assert_eq!(saved.revision, 1);
    assert!(!session.is_dirty());
    assert_eq!(saved.title, "retry me");
}

#[test]
fn close_failure_keeps_document_open_and_dirty_until_retry_succeeds() {
    let store = Arc::new(FailingOnceStore::new());
    let app = make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("close failure").unwrap();
    let id = session.note().id;
    assert!(app.close_note(id).is_err());
    assert!(!session.is_closed());
    assert!(session.is_dirty());
    assert_eq!(app.list_notes().unwrap()[0].title, "close failure");
    let persisted = session.retry().unwrap();
    assert_eq!(persisted.revision, 1);
    assert_eq!(app.close_note(id).unwrap().revision, 1);
    assert!(session.is_closed());
    assert_eq!(store.load(id).unwrap().unwrap().title, "close failure");
}

#[test]
fn activity_callback_reentry_cannot_deadlock_note_close_or_delete() {
    let activity = Arc::new(ReentrantActivity::default());
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(NotesApp::with_providers(
        store,
        Arc::new(SequenceIds::new(200)),
        Arc::new(TestClock::new()),
        activity.clone(),
        Duration::from_secs(30),
    ));
    *activity.app.lock().unwrap() = Some(Arc::downgrade(&app));
    let session = app.create_note("callback safety").unwrap();
    let id = session.flush().unwrap().id;
    session.set_title("edited before delete").unwrap();

    let deleting_app = Arc::clone(&app);
    let (result_tx, result_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = result_tx.send(deleting_app.delete_note(id));
    });
    assert!(
        result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .deleted
    );
    assert!(activity
        .attempts
        .lock()
        .unwrap()
        .contains(&Err(AppError::DocumentBusy(id))));
}

#[test]
fn edits_arriving_during_a_save_are_committed_in_a_followup_revision() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let store = Arc::new(BlockingStore::new(entered_tx, release_rx));
    let app = make_app(
        store,
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("coalesced").unwrap();
    session
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("first edit".to_owned()),
        ))
        .unwrap();

    let flush_session = Arc::clone(&session);
    let flush = thread::spawn(move || flush_session.flush());
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    session
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("edit during save".to_owned()),
        ))
        .unwrap();
    release_tx.send(()).unwrap();
    let saved = flush.join().unwrap().unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(
        paragraph_text(&saved),
        vec!["first edit".to_owned(), "edit during save".to_owned()]
    );
    assert!(!session.is_dirty());
}

#[test]
fn autosave_coalesces_several_edits_and_marks_the_session_clean_only_after_commit() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_millis(80),
    );
    let session = app.create_note("autosave").unwrap();
    assert_eq!(session.flush().unwrap().revision, 1);
    session.set_title("a").unwrap();
    session.set_title("ab").unwrap();
    session.set_title("final").unwrap();
    assert!(session.is_dirty());

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while session.is_dirty() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!session.is_dirty());
    assert_eq!(session.note().revision, 2);
    assert_eq!(
        store.load(session.note().id).unwrap().unwrap().title,
        "final"
    );
}

#[test]
fn markdown_supports_note_blocks_and_preserves_references() {
    let ids = SequenceIds::new(1_000);
    let note = nagi_notes::import_markdown(
        ObjectId(90),
        concat!(
            "# 仕様メモ\n\n",
            "## Heading\n\n",
            "Plain paragraph.\ncontinued line\n\n",
            "- [x] done\n\n",
            "\x60\x60\x60rust\nlet x = 1;\n\x60\x60\x60\n\n",
            "> quoted\n> text\n\n",
            "![diagram](nagi-object://0000000000000033)\n\n",
            "[source](nagi-object://0000000000000044)\n\n",
            "[website](https://example.invalid/path)\n\n",
            "| A | B |\n| --- | --- |\n| 1 | 2 |\n\n",
            "> [!NOTE] A callout\n> detail\n"
        ),
        &ids,
        Timestamp(1),
    );
    assert_eq!(note.title, "仕様メモ");
    assert!(matches!(
        note.blocks[0].kind,
        BlockKind::Heading { level: 2, .. }
    ));
    assert!(matches!(note.blocks[1].kind, BlockKind::Paragraph(_)));
    assert!(matches!(
        note.blocks[2].kind,
        BlockKind::Checklist {
            completed: true,
            ..
        }
    ));
    assert!(matches!(note.blocks[3].kind, BlockKind::Code { .. }));
    assert!(matches!(note.blocks[4].kind, BlockKind::Quote(_)));
    assert!(matches!(
        note.blocks[5].kind,
        BlockKind::ImageReference { .. }
    ));
    assert!(matches!(
        note.blocks[6].kind,
        BlockKind::FileReference {
            object_id: ObjectId(68),
            ..
        }
    ));
    assert!(matches!(
        note.blocks[7].kind,
        BlockKind::WebReference { .. }
    ));
    assert!(matches!(note.blocks[8].kind, BlockKind::Table { .. }));
    assert!(matches!(note.blocks[9].kind, BlockKind::Callout { .. }));
    assert_eq!(note.blocks.len(), 10);
    let block_ids = note
        .blocks
        .iter()
        .map(|block| block.id.0)
        .collect::<Vec<_>>();
    assert_eq!(
        block_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        block_ids.len()
    );
    let exported = nagi_notes::export_markdown(&note);
    assert!(exported.starts_with("# 仕様メモ\n"));
    assert!(!exported.contains("nagi:block-id"));
    assert!(exported.contains("[website](https://example.invalid/path)"));
}

#[test]
fn revision_restore_is_reversible_and_restore_as_copy_gets_new_object_ids() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = make_app(
        store,
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("history").unwrap();
    let note_id = session.note().id;
    let old_block_id = app.new_block_id();
    session
        .append_block(Block::new(
            old_block_id,
            BlockKind::Paragraph("before".to_owned()),
        ))
        .unwrap();
    session.flush().unwrap();
    session
        .update_block(old_block_id, BlockKind::Paragraph("after".to_owned()))
        .unwrap();
    session.flush().unwrap();
    app.close_note(note_id).unwrap();

    let restored_old = app.restore_revision(note_id, 1).unwrap();
    assert_eq!(paragraph_text(&restored_old), vec!["before"]);
    assert_eq!(restored_old.revision, 3);
    let restored_new = app.restore_revision(note_id, 2).unwrap();
    assert_eq!(paragraph_text(&restored_new), vec!["after"]);
    assert_eq!(restored_new.revision, 4);

    let copy = app.restore_as_copy(note_id, 1).unwrap();
    assert_ne!(copy.id, note_id);
    assert_eq!(paragraph_text(&copy), vec!["before"]);
    assert_ne!(copy.blocks[0].id, old_block_id);
}

#[test]
fn delete_and_restore_keep_the_note_recoverable() {
    let app = make_app(
        Arc::new(InMemoryNoteStore::new()),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("trash").unwrap();
    let id = session.flush().unwrap().id;
    let deleted = app.delete_note(id).unwrap();
    assert!(deleted.deleted);
    assert!(app.list_notes().unwrap().is_empty());
    assert_eq!(app.list_trash().unwrap().len(), 1);
    assert!(app.open_note(id).is_err());
    let restored = app.restore_note(id).unwrap();
    assert!(!restored.deleted);
    assert_eq!(app.list_notes().unwrap().len(), 1);
}

#[test]
fn search_provider_emits_note_and_block_metadata_without_owning_an_index() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("Project Nagi").unwrap();
    let block_id = app.new_block_id();
    session
        .append_block(Block::new(
            block_id,
            BlockKind::Paragraph("検索対象の日本語".to_owned()),
        ))
        .unwrap();
    session.set_workspace(Some(WorkspaceId(23))).unwrap();
    session.set_tags(vec!["planning".to_owned()]).unwrap();
    session.flush().unwrap();
    let provider = NotesSearchProvider::new(store);
    let records = provider.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].object_id, session.note().id);
    assert_eq!(records[0].workspace_id, Some(WorkspaceId(23)));
    assert_eq!(records[0].tags, vec!["planning"]);
    let hits = provider.search("日本語").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].block_id, Some(block_id));
}

#[test]
fn app_bound_search_includes_unsaved_edits_and_stable_block_ids() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(make_app(
        store,
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    ));
    let session = app.create_note("live searchable note").unwrap();
    let block_id = app.new_block_id();
    session
        .append_block(Block::new(
            block_id,
            BlockKind::Paragraph("uncommitted phrase".to_owned()),
        ))
        .unwrap();
    let provider = NotesSearchProvider::for_app(app);
    let hits = provider.search("uncommitted").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].block_id, Some(block_id));
}

#[test]
fn activity_failure_does_not_fail_note_persistence_or_leak_body_text() {
    let app = make_app(
        Arc::new(InMemoryNoteStore::new()),
        Arc::new(FailingActivity),
        Duration::from_secs(30),
    );
    let session = app.create_note("private title").unwrap();
    session
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("private body".to_owned()),
        ))
        .unwrap();
    let saved = session.flush().unwrap();
    assert_eq!(saved.revision, 1);
    assert!(!session.is_dirty());
    assert!(session.activity_failures() > 0);
    assert_eq!(app.activity_failures(), 0);
    let id = saved.id;
    app.close_note(id).unwrap();
    app.open_note(id).unwrap();
    assert!(app.activity_failures() > 0);
}

#[test]
fn host_backend_roundtrips_unicode_revisions_and_keeps_old_revision() {
    let sandbox = TempDirectory::new();
    let store = Arc::new(HostPreviewStore::open(sandbox.path()).unwrap());
    let app = make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("日本語と emoji 🌊").unwrap();
    let block_id = app.new_block_id();
    session
        .append_block(Block::new(
            block_id,
            BlockKind::Paragraph("永続化された本文".to_owned()),
        ))
        .unwrap();
    let rev1 = session.flush().unwrap();
    session.set_title("更新後のタイトル").unwrap();
    let rev2 = session.flush().unwrap();
    assert_eq!(rev2.revision, 2);
    assert_eq!(
        store.load_revision(rev1.id, 1).unwrap().unwrap().title,
        "日本語と emoji 🌊"
    );
    assert_eq!(store.load(rev1.id).unwrap().unwrap().blocks[0].id, block_id);
}

#[test]
fn host_backend_handles_missing_and_corrupt_notes_without_falling_back_silently() {
    let sandbox = TempDirectory::new();
    let store = HostPreviewStore::open(sandbox.path()).unwrap();
    assert!(store.load(ObjectId(42)).unwrap().is_none());
    let note_dir = sandbox.path().join(format!("{:016x}", 42));
    fs::create_dir_all(&note_dir).unwrap();
    fs::write(note_dir.join("rev-00000000000000000001.md"), "truncated").unwrap();
    assert!(matches!(
        store.load(ObjectId(42)),
        Err(StoreError::Corrupt(_))
    ));
}

#[test]
fn host_backend_never_overwrites_a_revision_written_by_another_store_instance() {
    let sandbox = TempDirectory::new();
    let first = HostPreviewStore::open(sandbox.path()).unwrap();
    let second = HostPreviewStore::open(sandbox.path()).unwrap();
    let mut note = NoteDocument::new(ObjectId(70), "base", Timestamp(1));
    let created = first.commit(&note, 0).unwrap();
    note.revision = created.revision;

    let mut first_candidate = note.clone();
    first_candidate.title = "first writer".to_owned();
    first_candidate.updated_at = Timestamp(2);
    first.commit(&first_candidate, 1).unwrap();

    let mut stale_candidate = note;
    stale_candidate.title = "stale writer".to_owned();
    stale_candidate.updated_at = Timestamp(3);
    assert!(matches!(
        second.commit(&stale_candidate, 1),
        Err(StoreError::RevisionConflict {
            expected: 1,
            actual: 2
        })
    ));
    assert_eq!(
        first.load(ObjectId(70)).unwrap().unwrap().title,
        "first writer"
    );
}

#[test]
fn host_preview_is_confined_to_object_derived_regular_entries() {
    let sandbox = TempDirectory::new();
    let store = HostPreviewStore::open(sandbox.path()).unwrap();
    let outside = TempDirectory::new();
    let link_path = sandbox.path().join(format!("{:016x}", 15));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), &link_path).unwrap();
        assert!(matches!(
            store.load(ObjectId(15)),
            Err(StoreError::SandboxViolation(_))
        ));
    }
    #[cfg(not(unix))]
    {
        let _ = (outside, link_path);
    }
}

#[test]
fn localization_catalogs_cover_required_ui_and_fallback_never_exposes_keys() {
    assert!(Localizer::catalog_complete(Locale::EnUs));
    assert!(Localizer::catalog_complete(Locale::JaJp));
    let localizer = Localizer::new(Locale::JaJp);
    assert_eq!(localizer.text("state.saved"), "保存済み");
    assert!(!localizer
        .text("missing.internal.key")
        .contains("missing.internal.key"));
}

#[test]
fn large_empty_and_unicode_documents_are_valid_and_multiple_ids_stay_distinct() {
    let app = make_app(
        Arc::new(InMemoryNoteStore::new()),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let empty = app.create_note("empty").unwrap();
    let empty_id = empty.flush().unwrap().id;
    let large = app.create_note("large").unwrap();
    let large_id = large.note().id;
    assert_ne!(empty_id, large_id);
    let text = "凪とNagi 🌊 ".repeat(24_000);
    large
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph(text.clone()),
        ))
        .unwrap();
    let saved = large.flush().unwrap();
    assert_eq!(saved.search_text(), text);
    assert!(app.open_note(ObjectId(u64::MAX)).is_err());
}

#[test]
fn agent_actions_require_policy_and_record_agent_provenance() {
    let store = Arc::new(InMemoryNoteStore::new());
    let activity = Arc::new(RecordingActivity::default());
    let app = Arc::new(make_app(
        store.clone(),
        activity.clone(),
        Duration::from_secs(30),
    ));
    let note = app.create_note("authorized note").unwrap().flush().unwrap();
    let search: Arc<dyn SearchProvider> = Arc::new(NotesSearchProvider::new(store));
    let denied = NotesActionExecutor::new(
        Arc::clone(&app),
        Arc::clone(&search),
        Arc::new(SelectivePolicy {
            denied_note: Some(note.id),
            deny_writes: false,
        }),
    );
    let action = NotesAction::AppendBlock {
        note_id: note.id,
        block: Block::new(ObjectId(900), BlockKind::Paragraph("blocked".to_owned())),
    };
    assert!(matches!(
        denied.execute(ActionPrincipal::Agent(AppSessionId(51)), action.clone()),
        Err(nagi_notes::ActionError::Policy(ActionPolicyError::Denied))
    ));
    assert!(app.get_note(note.id).unwrap().blocks.is_empty());

    let allowed = NotesActionExecutor::new(
        Arc::clone(&app),
        search,
        Arc::new(SelectivePolicy {
            denied_note: None,
            deny_writes: false,
        }),
    );
    let result = allowed
        .execute(ActionPrincipal::Agent(AppSessionId(51)), action)
        .unwrap();
    assert!(matches!(result, ActionResult::Document(document) if document.blocks.len() == 1));
    let events = activity.0.lock().unwrap();
    assert!(events.iter().any(|event| {
        event.kind == ActivityKind::Edited
            && event.origin == ActivityOrigin::Agent(AppSessionId(51))
            && event.object_id == note.id
    }));
}

#[test]
fn agent_search_hides_notes_without_object_read_authority() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    ));
    let hidden = app.create_note("hidden").unwrap();
    hidden
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("needle in hidden note".to_owned()),
        ))
        .unwrap();
    let hidden_id = hidden.flush().unwrap().id;
    let visible = app.create_note("visible").unwrap();
    visible
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("needle in visible note".to_owned()),
        ))
        .unwrap();
    let visible_id = visible.flush().unwrap().id;
    let executor = NotesActionExecutor::new(
        app,
        Arc::new(NotesSearchProvider::new(store)),
        Arc::new(SelectivePolicy {
            denied_note: Some(hidden_id),
            deny_writes: false,
        }),
    );
    let result = executor
        .execute(
            ActionPrincipal::Agent(AppSessionId(7)),
            NotesAction::Search {
                query: "needle".to_owned(),
            },
        )
        .unwrap();
    let ActionResult::SearchResults(hits) = result else {
        panic!("search action must return search hits");
    };
    assert!(!hits.iter().any(|hit| hit.object_id == hidden_id));
    assert!(hits.iter().any(|hit| hit.object_id == visible_id));
}

#[test]
fn workspace_actions_preserve_multiple_associations_and_primary_context() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    ));
    let note = app.create_note("workspace note").unwrap().flush().unwrap();
    let executor = NotesActionExecutor::new(
        app,
        Arc::new(NotesSearchProvider::new(store)),
        Arc::new(SelectivePolicy {
            denied_note: None,
            deny_writes: false,
        }),
    );
    for workspace_id in [WorkspaceId(20), WorkspaceId(21)] {
        executor
            .execute(
                ActionPrincipal::User,
                NotesAction::AddToWorkspace {
                    note_id: note.id,
                    workspace_id,
                },
            )
            .unwrap();
    }
    let document = match executor
        .execute(ActionPrincipal::User, NotesAction::Get { note_id: note.id })
        .unwrap()
    {
        ActionResult::Document(document) => document,
        ActionResult::SearchResults(_) => panic!("get action must return a document"),
    };
    assert_eq!(document.workspace_id, Some(WorkspaceId(20)));
    assert_eq!(
        document.workspace_ids,
        vec![WorkspaceId(20), WorkspaceId(21)]
    );

    executor
        .execute(
            ActionPrincipal::User,
            NotesAction::RemoveFromWorkspace {
                note_id: note.id,
                workspace_id: WorkspaceId(20),
            },
        )
        .unwrap();
    let document = match executor
        .execute(ActionPrincipal::User, NotesAction::Get { note_id: note.id })
        .unwrap()
    {
        ActionResult::Document(document) => document,
        ActionResult::SearchResults(_) => panic!("get action must return a document"),
    };
    assert_eq!(document.workspace_id, Some(WorkspaceId(21)));
    assert_eq!(document.workspace_ids, vec![WorkspaceId(21)]);
}

#[test]
fn reference_actions_reject_unsafe_urls_and_non_reference_blocks() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    ));
    let note = app.create_note("references").unwrap().flush().unwrap();
    let executor = NotesActionExecutor::new(
        app,
        Arc::new(NotesSearchProvider::new(store)),
        Arc::new(SelectivePolicy {
            denied_note: None,
            deny_writes: false,
        }),
    );
    let unsafe_reference = NotesAction::AddReference {
        note_id: note.id,
        block: Block::new(
            ObjectId(901),
            BlockKind::WebReference {
                title: "unsafe".to_owned(),
                url: "javascript:alert(1)".to_owned(),
            },
        ),
    };
    assert!(matches!(
        executor.execute(ActionPrincipal::User, unsafe_reference),
        Err(nagi_notes::ActionError::InvalidReference)
    ));
    let paragraph = NotesAction::AddReference {
        note_id: note.id,
        block: Block::new(ObjectId(902), BlockKind::Paragraph("text".to_owned())),
    };
    assert!(matches!(
        executor.execute(ActionPrincipal::User, paragraph),
        Err(nagi_notes::ActionError::InvalidReference)
    ));
    let accepted = NotesAction::AddReference {
        note_id: note.id,
        block: Block::new(
            ObjectId(903),
            BlockKind::WebReference {
                title: "source".to_owned(),
                url: "https://example.invalid/page".to_owned(),
            },
        ),
    };
    assert!(matches!(
        executor.execute(ActionPrincipal::User, accepted),
        Ok(ActionResult::Document(_))
    ));
}

#[test]
fn open_dirty_documents_are_visible_to_list_get_and_export() {
    let app = make_app(
        Arc::new(InMemoryNoteStore::new()),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("before save").unwrap();
    let id = session.note().id;
    session.set_title("live title").unwrap();
    session
        .append_block(Block::new(
            app.new_block_id(),
            BlockKind::Paragraph("live body".to_owned()),
        ))
        .unwrap();
    assert_eq!(app.list_notes().unwrap()[0].title, "live title");
    assert_eq!(app.get_note(id).unwrap().search_text(), "live body");
    assert!(app.export_note(id).unwrap().contains("live body"));
}

#[test]
fn quick_note_capture_creates_a_real_persistable_document() {
    let app = make_app(
        Arc::new(InMemoryNoteStore::new()),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app
        .create_quick_note(
            "Quick Note",
            "captured from quick entry",
            Some(WorkspaceId(22)),
        )
        .unwrap();
    assert_eq!(session.note().title, "Quick Note");
    assert_eq!(session.note().search_text(), "captured from quick entry");
    assert_eq!(session.note().workspace_id, Some(WorkspaceId(22)));
    assert_eq!(session.flush().unwrap().revision, 1);
}

#[test]
fn albert_page_and_selection_reference_survives_host_persistence() {
    let sandbox = TempDirectory::new();
    let store = Arc::new(HostPreviewStore::open(sandbox.path()).unwrap());
    let app = make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    );
    let session = app.create_note("research notes").unwrap();
    let block_id = app
        .add_albert_reference(
            &session,
            AlbertReference {
                page_id: ObjectId(0xabc),
                title: "Servo design 🌊".to_owned(),
                url: "https://example.invalid/design".to_owned(),
                selection: Some("第一行\nsecond line".to_owned()),
            },
        )
        .unwrap();
    let id = session.flush().unwrap().id;
    app.close_note(id).unwrap();
    let saved = store.load(id).unwrap().unwrap();
    assert_eq!(saved.blocks[0].id, block_id);
    assert_eq!(
        saved.blocks[0].kind,
        BlockKind::AlbertReference(AlbertReference {
            page_id: ObjectId(0xabc),
            title: "Servo design 🌊".to_owned(),
            url: "https://example.invalid/design".to_owned(),
            selection: Some("第一行\nsecond line".to_owned()),
        })
    );
    assert!(app
        .export_note(id)
        .unwrap()
        .contains("> 第一行\n> second line"));
}

#[test]
fn markdown_import_does_not_promote_unsafe_targets_to_clickable_references() {
    let imported = nagi_notes::import_markdown(
        ObjectId(71),
        "# source\n\n[unsafe](javascript:alert(1))\n\n![local](file:///etc/passwd)",
        &SequenceIds::new(1_000),
        Timestamp(1),
    );
    assert_eq!(imported.blocks.len(), 2);
    assert!(imported
        .blocks
        .iter()
        .all(|block| matches!(block.kind, BlockKind::Paragraph(_))));
}

#[test]
fn agent_cannot_reference_a_page_without_source_read_authority() {
    let store = Arc::new(InMemoryNoteStore::new());
    let app = Arc::new(make_app(
        store.clone(),
        Arc::new(RecordingActivity::default()),
        Duration::from_secs(30),
    ));
    let note = app.create_note("citation target").unwrap().flush().unwrap();
    let page_id = ObjectId(0x456);
    let executor = NotesActionExecutor::new(
        app,
        Arc::new(NotesSearchProvider::new(store)),
        Arc::new(SelectivePolicy {
            denied_note: Some(page_id),
            deny_writes: false,
        }),
    );
    let result = executor.execute(
        ActionPrincipal::Agent(AppSessionId(9)),
        NotesAction::AddReference {
            note_id: note.id,
            block: Block::new(
                ObjectId(904),
                BlockKind::AlbertReference(AlbertReference {
                    page_id,
                    title: "private source".to_owned(),
                    url: "https://example.invalid/private".to_owned(),
                    selection: Some("private excerpt".to_owned()),
                }),
            ),
        },
    );
    assert!(matches!(
        result,
        Err(nagi_notes::ActionError::Policy(ActionPolicyError::Denied))
    ));
    assert!(executor
        .execute(ActionPrincipal::User, NotesAction::Get { note_id: note.id })
        .is_ok());
}
