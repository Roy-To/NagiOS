use nagi_calendar_core::adapter::LocalAdapter;
use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::model::{EventDraft, Interval, Schedule};
use nagi_calendar_core::store::{InMemoryStore, SequentialIds};
use nagi_calendar_core::time::{FixedClock, Instant};
use nagi_calendar_input_adapter::{
    AgendaCreateController, AgendaDeleteController, AgendaEditController, AgendaKeyboardAction,
    AgendaKeyboardAdapter,
};
use nagi_ui::interaction::KeyCode;

#[test]
fn first_row_down_up_enter_opens_the_selected_local_event() {
    let mut store = InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), 8).unwrap();
    let calendar = store.create_calendar("Agenda input test", None).unwrap();
    let mut adapter = LocalAdapter { store };
    let schedule = Schedule::Instant(Interval::new(Instant(10), Instant(20)).unwrap());
    let mut create = AgendaCreateController::new(EventDraft::new(
        calendar.id.clone(),
        "New appointment",
        schedule.clone(),
    ));
    assert_eq!(create.draft().title, "New appointment");
    create.draft_mut().title = "First".into();
    let first = create.create(&mut adapter).unwrap();
    assert_eq!(first.revision.get(), 1);
    assert_eq!(first.draft.title, "First");
    let second = adapter
        .create_event(EventDraft::new(calendar.id.clone(), "Second", schedule))
        .unwrap();
    let listed = adapter.list(&calendar.id, 8).unwrap();
    let mut agenda = AgendaViewModel::new(&listed);
    let keys = AgendaKeyboardAdapter::new(&mut agenda);

    assert_eq!(agenda.selected_event_id(), Some(&first.id));
    assert_eq!(
        keys.handle_key(KeyCode::ArrowDown, &mut agenda),
        Some(AgendaKeyboardAction::SelectionChanged(second.id.clone()))
    );
    assert_eq!(
        keys.handle_key(KeyCode::ArrowUp, &mut agenda),
        Some(AgendaKeyboardAction::SelectionChanged(first.id.clone()))
    );
    let opened_id = match keys.handle_key(KeyCode::Enter, &mut agenda) {
        Some(AgendaKeyboardAction::Open(event_id)) => event_id,
        action => panic!("expected open action, got {action:?}"),
    };

    let mut editor = AgendaEditController::open(&adapter, &opened_id).unwrap();
    assert_eq!(editor.event().id, first.id);
    let mut draft = editor.event().draft.clone();
    draft.title = "First revised".into();
    let updated_schedule = Schedule::Instant(Interval::new(Instant(30), Instant(50)).unwrap());
    draft.schedule = updated_schedule.clone();

    let saved = editor.save(&mut adapter, draft).unwrap().clone();
    assert_eq!(saved.id, first.id);
    assert_eq!(saved.revision.get(), first.revision.get() + 1);
    assert_eq!(saved.draft.title, "First revised");
    assert_eq!(saved.draft.schedule, updated_schedule);

    let refreshed = adapter.list(&calendar.id, 8).unwrap();
    let mut refreshed_agenda = AgendaViewModel::new(&refreshed);
    assert!(refreshed_agenda.select_event(&first.id));
    let row = refreshed_agenda
        .rows()
        .find(|row| row.event_id() == &first.id)
        .unwrap();
    assert_eq!(row.title(), "First revised");
    assert_eq!(row.schedule(), &updated_schedule);
    assert_eq!(row.revision(), saved.revision);
}

#[test]
fn selected_event_delete_uses_its_canonical_revision_and_updates_the_list() {
    let mut store = InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), 8).unwrap();
    let calendar = store.create_calendar("Agenda delete test", None).unwrap();
    let mut adapter = LocalAdapter { store };
    let schedule = Schedule::Instant(Interval::new(Instant(10), Instant(20)).unwrap());
    let event = adapter
        .create_event(EventDraft::new(calendar.id.clone(), "Remove me", schedule))
        .unwrap();

    let deletion = AgendaDeleteController::open(&adapter, &event.id).unwrap();
    assert_eq!(deletion.event(), &event);
    assert_eq!(deletion.delete(&mut adapter).unwrap(), event);
    assert!(adapter.list(&calendar.id, 8).unwrap().is_empty());
}
