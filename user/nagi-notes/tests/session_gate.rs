use nagi_notes::{
    Block, BlockKind, InMemoryNoteStore, NoteDocument, NoteSessionEffectGate, NoteStore, ObjectId,
    SessionGateError, SessionGatedNoteStore, StoreError, Timestamp,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InjectedSessionState {
    Live,
    LoggedOut,
    Expired,
}

#[derive(Clone)]
struct InjectedTestGate {
    state: Arc<Mutex<InjectedSessionState>>,
}

impl InjectedTestGate {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(InjectedSessionState::Live)),
        }
    }

    fn set_state(&self, state: InjectedSessionState) {
        *self.state.lock().expect("test gate lock") = state;
    }
}

impl NoteSessionEffectGate for InjectedTestGate {
    fn with_live_session_effect<R, F>(&self, effect: F) -> Result<R, SessionGateError>
    where
        F: FnOnce() -> R,
    {
        let state = self.state.lock().map_err(|_| SessionGateError)?;
        match *state {
            InjectedSessionState::Live => Ok(effect()),
            InjectedSessionState::LoggedOut | InjectedSessionState::Expired => {
                Err(SessionGateError)
            }
        }
    }
}

struct CommitCountingStore {
    inner: InMemoryNoteStore,
    commit_calls: AtomicUsize,
}

impl CommitCountingStore {
    fn new() -> Self {
        Self {
            inner: InMemoryNoteStore::new(),
            commit_calls: AtomicUsize::new(0),
        }
    }
}

impl NoteStore for CommitCountingStore {
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
        self.commit_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.commit(note, expected_revision)
    }
}

#[test]
fn logout_and_expiry_denials_leave_note_and_persistence_unchanged() {
    let store = Arc::new(CommitCountingStore::new());
    let mut original = NoteDocument::new(ObjectId(701), "Session gated", Timestamp(1));
    original.blocks.push(Block::new(
        ObjectId(702),
        BlockKind::Paragraph("before".to_owned()),
    ));
    let saved = store.commit(&original, 0).expect("seed persisted note");
    let side_effects_before_denials = store.commit_calls.load(Ordering::SeqCst);
    let gate = InjectedTestGate::new();
    let gated = SessionGatedNoteStore::new(Arc::clone(&store), gate.clone());

    for denied_state in [
        InjectedSessionState::LoggedOut,
        InjectedSessionState::Expired,
    ] {
        gate.set_state(denied_state);
        let mut attempted = saved.clone();
        attempted.blocks[0].kind = BlockKind::Paragraph("after".to_owned());

        assert_eq!(
            gated.commit(&attempted, saved.revision),
            Err(StoreError::SessionGateRejected),
            "denied state: {denied_state:?}"
        );
        let after = store
            .load(saved.id)
            .expect("read seeded note")
            .expect("seed remains present");
        assert_eq!(after, saved, "denied state: {denied_state:?}");
        assert_eq!(after.revision, saved.revision);
        assert_eq!(after.search_text(), "before");
        assert_eq!(
            store.commit_calls.load(Ordering::SeqCst),
            side_effects_before_denials,
            "denied state: {denied_state:?}"
        );
    }
}
