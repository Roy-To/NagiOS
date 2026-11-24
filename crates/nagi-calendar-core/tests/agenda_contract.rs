use nagi_calendar_core::adapter::LocalAdapter;
use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::model::{EventDraft, Interval, Schedule};
use nagi_calendar_core::store::{InMemoryStore, SequentialIds};
use nagi_calendar_core::time::{FixedClock, Instant};

#[test]
fn local_fixture_list_agenda_select_get_returns_same_event() {
    let mut store = InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), 8).unwrap();
    let calendar = store.create_calendar("Agenda fixture", None).unwrap();
    let mut adapter = LocalAdapter { store };

    let schedule = Schedule::Instant(Interval::new(Instant(10), Instant(20)).unwrap());
    let event = adapter
        .create_event(EventDraft::new(calendar.id.clone(), "Planning", schedule))
        .unwrap();
    let listed = adapter.list(&calendar.id, 8).unwrap();
    let mut agenda = AgendaViewModel::new(&listed);

    let row = agenda.rows().next().unwrap();
    assert_eq!(row.event_id(), &event.id);
    assert_eq!(row.title(), "Planning");
    assert_eq!(row.schedule(), &event.draft.schedule);
    assert_eq!(row.revision(), event.revision);
    assert!(agenda.select_event(row.event_id()));
    assert!(agenda.is_selected(&event.id));

    let selected_id = agenda.selected_event_id().unwrap().clone();
    let fetched = adapter.get_event(&selected_id).unwrap();
    assert_eq!(fetched.id, event.id);
    assert_eq!(fetched.draft.title, "Planning");
    assert_eq!(fetched.draft.schedule, event.draft.schedule);
}
