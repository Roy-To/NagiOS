use super::*;
use nagi_calendar_core::model::{EventDraft, Interval, Schedule};
use nagi_calendar_core::time::{FixedClock, Instant};
use nagi_calendar_core::ObjectId;
use nagi_persistence_contract::{ConditionalWriteOutcome, Revision, Snapshot, SnapshotStorage};

struct MemoryObject {
    object: ObjectId,
    saved: Option<Snapshot>,
    writes: usize,
    claimed: bool,
    ended: bool,
    mode: WriteMode,
}

impl Default for MemoryObject {
    fn default() -> Self {
        Self {
            object: ObjectId(701),
            saved: None,
            writes: 0,
            claimed: false,
            ended: false,
            mode: WriteMode::Apply,
        }
    }
}

enum WriteMode {
    Apply,
    KeepOld,
    Unverified,
    Conflict,
}

impl SnapshotStorage for MemoryObject {
    type Error = ();

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, ()> {
        assert_eq!(key, self.object);
        Ok(self.saved.clone())
    }

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<()> {
        assert_eq!(key, self.object);
        self.writes += 1;
        let current_revision = self.saved.as_ref().map(|s| s.revision);
        if current_revision != expected || matches!(self.mode, WriteMode::Conflict) {
            return ConditionalWriteOutcome::Conflict { current_revision };
        }
        match self.mode {
            WriteMode::KeepOld => ConditionalWriteOutcome::NotApplied { error: () },
            WriteMode::Unverified | WriteMode::Apply => {
                let revision = Revision::new(current_revision.map_or(1, |r| r.get() + 1)).unwrap();
                self.saved = Some(Snapshot::new(revision, bytes.to_vec()));
                if matches!(self.mode, WriteMode::Unverified) {
                    ConditionalWriteOutcome::OutcomeUnknown { error: () }
                } else {
                    ConditionalWriteOutcome::Applied { revision }
                }
            }
            WriteMode::Conflict => unreachable!(),
        }
    }
}

impl CalendarSnapshotScope for &mut MemoryObject {
    type ScopeError = ();
    type StorageError = ();

    fn claim_initialization(&mut self) -> Result<bool, ()> {
        if self.ended {
            return Err(());
        }
        Ok(!std::mem::replace(&mut self.claimed, true))
    }

    fn with_storage<R>(
        &mut self,
        operation: impl FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = ()>) -> R,
    ) -> Result<R, ()> {
        if self.ended {
            return Err(());
        }
        Ok(operation(self.object, *self))
    }
}

fn draft(calendar: &nagi_calendar_core::model::CalendarId) -> EventDraft {
    EventDraft::new(
        calendar.clone(),
        "Planning",
        Schedule::Instant(Interval::new(Instant(1), Instant(2)).unwrap()),
    )
}

#[test]
fn writes_once_per_commit_and_reopen_retains_the_deleted_id_history() {
    let mut io = MemoryObject::default();
    let deleted_id = {
        let mut session = CalendarSession::open(&mut io, FixedClock(1000)).unwrap();
        assert_eq!(session.calendar_id().as_str(), "cal_1");
        assert!(session.snapshot_bytes().unwrap().starts_with(b"NCS1"));
        let first = session
            .commit(CalendarEdit::Create(draft(session.calendar_id())))
            .unwrap();
        let mut edit = first.draft.clone();
        edit.title = "Updated".into();
        let updated = session
            .commit(CalendarEdit::Update(first.id.clone(), first.revision, edit))
            .unwrap();
        assert_eq!(session.events().unwrap(), std::slice::from_ref(&updated));
        assert_eq!(
            session
                .commit(CalendarEdit::Delete(updated.id.clone(), updated.revision))
                .unwrap(),
            updated
        );
        updated.id
    };
    assert_eq!(io.writes, 4);
    let mut reopened = CalendarSession::open(&mut io, FixedClock(2000)).unwrap();
    assert!(reopened.events().unwrap().is_empty());
    let later = reopened
        .commit(CalendarEdit::Create(draft(reopened.calendar_id())))
        .unwrap();
    assert_ne!(deleted_id, later.id);
    assert_eq!(later.id.as_str(), "event_2");
}

#[test]
fn reopened_session_reads_the_saved_event_without_writing_again() {
    let mut io = MemoryObject::default();
    let saved_event = {
        let mut session = CalendarSession::open(&mut io, FixedClock(1000)).unwrap();
        session
            .commit(CalendarEdit::Create(draft(session.calendar_id())))
            .unwrap()
    };
    let writes_before_reopen = io.writes;
    let persisted = io.saved.clone().unwrap();

    let reopened = CalendarSession::open(&mut io, FixedClock(2000)).unwrap();

    assert_eq!(reopened.events().unwrap(), [saved_event]);
    assert_eq!(reopened.snapshot_bytes().unwrap(), persisted.bytes);
    assert_eq!(io.writes, writes_before_reopen);
}

#[test]
fn known_failure_rolls_back_and_unverified_readback_fences_further_writes() {
    let mut io = MemoryObject::default();
    CalendarSession::open(&mut io, FixedClock(1000)).unwrap();
    io.mode = WriteMode::KeepOld;
    {
        let mut session = CalendarSession::open(&mut io, FixedClock(1000)).unwrap();
        assert_eq!(
            session.commit(CalendarEdit::Create(draft(session.calendar_id()))),
            Err(SaveRejected::NotApplied)
        );
        assert!(session.events().unwrap().is_empty());
    }
    io.mode = WriteMode::Unverified;
    {
        let mut session = CalendarSession::open(&mut io, FixedClock(1000)).unwrap();
        let edit = || {
            CalendarEdit::Create(draft(
                &nagi_calendar_core::model::CalendarId::new("cal_1").unwrap(),
            ))
        };
        assert_eq!(session.commit(edit()), Err(SaveRejected::StateUnknown));
        assert!(session.events().unwrap().is_empty());
        assert_eq!(session.commit(edit()), Err(SaveRejected::StateUnknown));
    }
    assert_eq!(io.writes, 3);
}

#[test]
fn an_uncertain_initial_write_fences_later_missing_data() {
    let mut io = MemoryObject {
        mode: WriteMode::KeepOld,
        ..MemoryObject::default()
    };
    assert!(matches!(
        CalendarSession::open(&mut io, FixedClock(1000)),
        Err(LoadRejected::UnverifiedInitialization)
    ));
    io.mode = WriteMode::Apply;
    assert!(matches!(
        CalendarSession::open(&mut io, FixedClock(1000)),
        Err(LoadRejected::Missing)
    ));
    assert_eq!(io.writes, 1);
    assert!(io.saved.is_none());
}

#[test]
fn revoked_scope_denies_open_without_claiming_or_writing() {
    let mut io = MemoryObject {
        ended: true,
        ..MemoryObject::default()
    };
    assert!(matches!(
        CalendarSession::open(&mut io, FixedClock(1000)),
        Err(LoadRejected::Io)
    ));
    assert!(!io.claimed);
    assert_eq!(io.writes, 0);
}

#[test]
fn corrupt_snapshot_never_claims_or_writes() {
    let bytes = b"corrupt snapshot";
    let mut io = MemoryObject {
        saved: Some(Snapshot::new(Revision::new(1).unwrap(), bytes.to_vec())),
        ..MemoryObject::default()
    };
    assert!(matches!(
        CalendarSession::open(&mut io, FixedClock(1000)),
        Err(LoadRejected::Invalid(_))
    ));
    assert!(!io.claimed);
    assert_eq!(io.writes, 0);
    assert_eq!(io.saved.unwrap().bytes, bytes);
}
