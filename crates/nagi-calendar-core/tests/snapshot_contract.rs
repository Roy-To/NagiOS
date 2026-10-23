use nagi_calendar_core::model::*;
use nagi_calendar_core::recurrence::*;
use nagi_calendar_core::snapshot::*;
use nagi_calendar_core::store::*;
use nagi_calendar_core::time::*;
use nagi_calendar_core::{AppId, Error, ObjectId, UserId, WorkspaceId};
use std::cell::Cell;
use std::rc::Rc;

type Store = InMemoryStore<FixedClock, SequentialIds>;
fn store(capacity: usize) -> Store {
    InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), capacity).unwrap()
}
fn schedule() -> Schedule {
    Schedule::Instant(Interval::new(Instant(0), Instant(100)).unwrap())
}
fn populated() -> Store {
    let mut s = store(20);
    let c = s.create_calendar("予定 / calendar", None).unwrap();
    s.create(EventDraft::new(c.id, "会議 / meeting", schedule()))
        .unwrap();
    s
}
fn reject_unchanged(snapshot: StoreSnapshot, expected: Error) {
    let mut target = populated();
    let before = target.snapshot();
    assert_eq!(target.restore_snapshot(snapshot), Err(expected));
    assert_eq!(target.snapshot(), before);
}

#[test]
fn rejects_unknown_versions_without_mutation() {
    for version in [0, SNAPSHOT_VERSION + 1, u32::MAX] {
        let mut snapshot = populated().snapshot();
        snapshot.version = version;
        reject_unchanged(snapshot, Error::UnsupportedSnapshotVersion);
    }
}

#[test]
fn rejects_duplicate_live_and_history_ids_without_mutation() {
    let original = populated().snapshot();
    let mut snapshot = original.clone();
    snapshot.calendars.push(snapshot.calendars[0].clone());
    reject_unchanged(snapshot, Error::DuplicateId);
    let mut snapshot = original.clone();
    snapshot.events.push(snapshot.events[0].clone());
    reject_unchanged(snapshot, Error::DuplicateId);
    let mut snapshot = original.clone();
    snapshot
        .used_calendar_ids
        .push(snapshot.used_calendar_ids[0].clone());
    reject_unchanged(snapshot, Error::DuplicateId);
    let mut snapshot = original;
    snapshot
        .used_event_ids
        .push(snapshot.used_event_ids[0].clone());
    reject_unchanged(snapshot, Error::DuplicateId);
}

#[test]
fn rejects_missing_history_and_dangling_calendar_without_mutation() {
    let original = populated().snapshot();
    let mut snapshot = original.clone();
    snapshot.used_calendar_ids.clear();
    reject_unchanged(snapshot, Error::InvalidSnapshot);
    let mut snapshot = original.clone();
    snapshot.used_event_ids.clear();
    reject_unchanged(snapshot, Error::InvalidSnapshot);
    let mut snapshot = original;
    snapshot.events[0].draft.calendar_id = CalendarId::new("cal_missing").unwrap();
    reject_unchanged(snapshot, Error::InvalidSnapshot);
}

#[test]
fn rejects_corrupt_records_without_mutation() {
    let original = populated().snapshot();
    let mut snapshot = original.clone();
    snapshot.calendars[0].name = "\0".into();
    reject_unchanged(snapshot, Error::InvalidText);
    let mut snapshot = original.clone();
    snapshot.events[0].draft.title = " ".into();
    reject_unchanged(snapshot, Error::InvalidText);
    let mut snapshot = original.clone();
    snapshot.events[0].draft.recurrence = Some(Recurrence {
        frequency: Frequency::InstantEvery { micros: 100 },
        interval: 0,
        bound: RecurrenceBound::Count(2),
    });
    reject_unchanged(snapshot, Error::InvalidRecurrence);
    let mut snapshot = original.clone();
    snapshot.events[0].modified_at = Instant(999);
    reject_unchanged(snapshot, Error::InvalidSnapshot);
    let mut snapshot = original;
    snapshot.events[0].draft.participants = vec!["p".into(); 257];
    reject_unchanged(snapshot, Error::Capacity);
}

#[test]
fn rejects_each_oversized_collection_before_record_validation() {
    for collection in 0..4 {
        let mut target = store(1);
        target.create_calendar("retained", None).unwrap();
        let before = target.snapshot();
        let mut snapshot = populated().snapshot();
        match collection {
            0 => snapshot.calendars.resize(2, snapshot.calendars[0].clone()),
            1 => snapshot.events.resize(2, snapshot.events[0].clone()),
            2 => snapshot
                .used_calendar_ids
                .resize(2, snapshot.used_calendar_ids[0].clone()),
            _ => snapshot
                .used_event_ids
                .resize(2, snapshot.used_event_ids[0].clone()),
        }
        assert_eq!(target.restore_snapshot(snapshot), Err(Error::Capacity));
        assert_eq!(target.snapshot(), before);
    }
}

#[test]
fn roundtrip_preserves_full_event_models_revisions_and_tombstones() {
    let mut source = store(20);
    let owner = Some(OwnerMetadata {
        app: AppId(7),
        user: Some(UserId(8)),
    });
    let calendar = source.create_calendar("予定", owner.clone()).unwrap();
    let date = Date::new(2026, 10, 23).unwrap();
    let start = LocalDateTime::new(date, 9, 0, 0, 123).unwrap();
    let end = start.checked_add(SECOND * 3600).unwrap();
    let schedules = [
        schedule(),
        Schedule::Floating { start, end },
        Schedule::Zoned {
            start,
            end,
            zone: ZoneId::new("fixture/zone").unwrap(),
            start_choice: ResolveChoice::Earlier,
            end_choice: ResolveChoice::Later,
            duration_mode: DurationMode::Elapsed,
        },
        Schedule::AllDay {
            start: date,
            end_exclusive: date.add_days(1).unwrap(),
        },
    ];
    for (index, schedule) in schedules.into_iter().enumerate() {
        let mut draft = EventDraft::new(calendar.id.clone(), "会議 / meeting", schedule);
        draft.description = "notes / メモ".into();
        draft.location = "office".into();
        draft.organizer = Some("organizer".into());
        draft.participants = vec!["参加者".into(), "participant".into()];
        draft.reminders = vec![
            Reminder::RelativeMicros(-SECOND),
            Reminder::Absolute(Instant(123)),
        ];
        draft.resources = vec![ObjectId(17)];
        draft.workspaces = vec![WorkspaceId(19)];
        draft.owner = owner.clone();
        draft.state = [
            DraftState::Draft,
            DraftState::Validated,
            DraftState::RequiresConfirmation,
        ][index % 3];
        draft.busy = index % 2 == 0;
        let (frequency, excluded, overridden) = match draft.schedule.key() {
            OccurrenceKey::Instant(t) => (
                Frequency::InstantEvery { micros: 1000 },
                OccurrenceKey::Instant(t.checked_add(1000).unwrap()),
                OccurrenceKey::Instant(t.checked_add(2000).unwrap()),
            ),
            OccurrenceKey::Local(t) => (
                Frequency::Daily,
                OccurrenceKey::Local(t.checked_add(DAY).unwrap()),
                OccurrenceKey::Local(t.checked_add(2 * DAY).unwrap()),
            ),
            OccurrenceKey::Date(d) => (
                Frequency::Daily,
                OccurrenceKey::Date(d.add_days(1).unwrap()),
                OccurrenceKey::Date(d.add_days(2).unwrap()),
            ),
        };
        draft.recurrence = Some(Recurrence {
            frequency,
            interval: 1,
            bound: RecurrenceBound::Count(3),
        });
        draft.exdates.insert(excluded);
        draft.overrides.insert(overridden, draft.schedule.clone());
        let event = source.create(draft).unwrap();
        source
            .update(&event.id, event.revision, event.draft)
            .unwrap();
    }
    let deleted = source
        .create(EventDraft::new(calendar.id.clone(), "deleted", schedule()))
        .unwrap();
    source.delete(&deleted.id, deleted.revision).unwrap();
    let deleted_calendar = source.create_calendar("deleted calendar", None).unwrap();
    source.delete_calendar(&deleted_calendar.id).unwrap();
    let snapshot = source.snapshot();
    assert_eq!(snapshot.version, SNAPSHOT_VERSION);
    assert_eq!(snapshot.calendars.len(), 1);
    assert_eq!(snapshot.events.len(), 4);
    assert_eq!(snapshot.used_calendar_ids.len(), 2);
    assert_eq!(snapshot.used_event_ids.len(), 5);
    let mut restored = populated();
    restored.restore_snapshot(snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(
        restored.list(&calendar.id, 20).unwrap(),
        source.list(&calendar.id, 20).unwrap()
    );
    assert_eq!(restored.get(&deleted.id), Err(Error::NotFound));
    assert_eq!(
        restored.calendar(&deleted_calendar.id),
        Err(Error::NotFound)
    );
}

#[test]
fn restored_revisions_reject_stale_updates_and_deletes() {
    let mut source = populated();
    let original = source.snapshot().events[0].clone();
    let updated = source
        .update(&original.id, original.revision, original.draft.clone())
        .unwrap();
    let mut target = store(20);
    target.restore_snapshot(source.snapshot()).unwrap();
    assert_eq!(
        target.update(&original.id, original.revision, original.draft),
        Err(Error::RevisionConflict)
    );
    assert_eq!(
        target.delete(&original.id, original.revision),
        Err(Error::RevisionConflict)
    );
    assert_eq!(target.get(&original.id).unwrap(), updated);
    let next = target
        .update(&updated.id, updated.revision, updated.draft.clone())
        .unwrap();
    assert_eq!(next.revision.get(), 3);
    assert_eq!(next.created_at, Instant(1000));
    assert_eq!(next.modified_at, Instant(1000));
}

#[test]
fn restores_tombstones_and_rejects_restarted_allocator_collisions() {
    let mut source = populated();
    let event = source.snapshot().events[0].clone();
    source.delete(&event.id, event.revision).unwrap();
    let calendar = source.snapshot().calendars[0].clone();
    source.delete_calendar(&calendar.id).unwrap();
    let mut target = store(20);
    target.restore_snapshot(source.snapshot()).unwrap();
    assert_eq!(
        target.create_calendar("collision", None),
        Err(Error::DuplicateId)
    );
    let new_calendar = target.create_calendar("new", None).unwrap();
    assert_eq!(
        target.create(EventDraft::new(
            new_calendar.id.clone(),
            "collision",
            schedule()
        )),
        Err(Error::DuplicateId)
    );
    assert!(target
        .create(EventDraft::new(new_calendar.id, "new", schedule()))
        .is_ok());
}

#[test]
fn snapshot_order_and_fixed_clock_are_deterministic() {
    assert_eq!(populated().snapshot(), populated().snapshot());
    let mut snapshot = populated().snapshot();
    snapshot
        .used_calendar_ids
        .push(CalendarId::new("cal_0").unwrap());
    snapshot
        .used_event_ids
        .push(EventId::new("event_0").unwrap());
    let mut target = store(20);
    target.restore_snapshot(snapshot).unwrap();
    let sorted = target.snapshot();
    assert_eq!(sorted.used_calendar_ids[0].as_str(), "cal_0");
    assert_eq!(sorted.used_event_ids[0].as_str(), "event_0");
}

#[test]
fn restore_keeps_injected_clock_and_does_not_consume_ids() {
    struct Clock(Rc<Cell<usize>>);
    impl RealtimeSource for Clock {
        fn realtime_ns(&self) -> nagi_calendar_core::Result<u64> {
            self.0.set(self.0.get() + 1);
            Ok(2_000_000)
        }
    }
    struct Ids(Rc<Cell<usize>>);
    impl IdSource for Ids {
        fn next_calendar_id(&mut self) -> nagi_calendar_core::Result<CalendarId> {
            self.0.set(self.0.get() + 1);
            CalendarId::new("cal_custom")
        }
        fn next_event_id(&mut self) -> nagi_calendar_core::Result<EventId> {
            self.0.set(self.0.get() + 1);
            EventId::new("event_custom")
        }
    }
    let clocks = Rc::new(Cell::new(0));
    let ids = Rc::new(Cell::new(0));
    let mut target = InMemoryStore::new(Clock(clocks.clone()), Ids(ids.clone()), 20).unwrap();
    target.restore_snapshot(populated().snapshot()).unwrap();
    assert_eq!(clocks.get(), 0);
    assert_eq!(ids.get(), 0);
    let c = target.create_calendar("new", None).unwrap();
    let e = target
        .create(EventDraft::new(c.id, "new", schedule()))
        .unwrap();
    assert_eq!(e.created_at, Instant(2000));
    assert_eq!(clocks.get(), 1);
    assert_eq!(ids.get(), 2);
}

#[test]
fn empty_snapshot_replaces_state_and_keeps_receiving_capacity() {
    let mut target = populated();
    target.restore_snapshot(store(1).snapshot()).unwrap();
    assert_eq!(target.snapshot(), store(1).snapshot());
    for n in 0..20 {
        target
            .create_calendar(format!("calendar {n}"), None)
            .unwrap();
    }
    assert_eq!(
        target.create_calendar("over capacity", None),
        Err(Error::Capacity)
    );
}

#[test]
fn rejects_invalid_schedule_and_exception_even_in_later_record() {
    let mut source = populated();
    let calendar = source.snapshot().calendars[0].id.clone();
    source
        .create(EventDraft::new(calendar, "second", schedule()))
        .unwrap();
    let original = source.snapshot();
    let mut snapshot = original.clone();
    let date = Date::new(2026, 1, 1).unwrap();
    snapshot.events[1].draft.schedule = Schedule::AllDay {
        start: date,
        end_exclusive: date,
    };
    reject_unchanged(snapshot, Error::InvalidRange);
    let mut snapshot = original;
    snapshot.events[1]
        .draft
        .exdates
        .insert(OccurrenceKey::Instant(Instant(0)));
    reject_unchanged(snapshot, Error::InvalidException);
}

#[test]
fn restored_tombstones_count_toward_receiving_capacity() {
    let mut source = populated();
    let event = source.snapshot().events[0].clone();
    source.delete(&event.id, event.revision).unwrap();
    let mut target = store(1);
    target.restore_snapshot(source.snapshot()).unwrap();
    let before = target.snapshot();
    assert_eq!(
        target.create(EventDraft::new(event.draft.calendar_id, "new", schedule())),
        Err(Error::Capacity)
    );
    assert_eq!(target.create_calendar("new", None), Err(Error::Capacity));
    assert_eq!(target.snapshot(), before);
}

#[test]
fn restored_modified_time_still_rejects_clock_rollback() {
    let source = populated();
    let event = source.snapshot().events[0].clone();
    let mut target = InMemoryStore::new(FixedClock(0), SequentialIds::new(), 20).unwrap();
    target.restore_snapshot(source.snapshot()).unwrap();
    let before = target.snapshot();
    assert_eq!(
        target.update(&event.id, event.revision, event.draft),
        Err(Error::ClockRollback)
    );
    assert_eq!(target.snapshot(), before);
}

#[test]
fn snapshots_are_owned_and_do_not_alias_the_source() {
    let source = populated();
    let mut snapshot = source.snapshot();
    let before = source.snapshot();
    snapshot.events[0].draft.title = "changed".into();
    snapshot.calendars.clear();
    assert_eq!(source.snapshot(), before);
}
