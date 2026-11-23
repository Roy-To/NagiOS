use nagi_calendar_canvas_adapter::{render_agenda, AgendaCanvas, AgendaCanvasRect};
use nagi_calendar_core::adapter::LocalAdapter;
use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::model::{EventDraft, Interval, Schedule};
use nagi_calendar_core::store::{InMemoryStore, SequentialIds};
use nagi_calendar_core::time::{FixedClock, Instant};
use nagi_calendar_input_adapter::{AgendaKeyboardAction, AgendaKeyboardAdapter};
use nagi_ui::interaction::KeyCode;
use nagi_ui::{color, Color, ColorRole, ThemeMode, TypeRole};

#[derive(Clone, Debug, Eq, PartialEq)]
struct DrawnText {
    bounds: AgendaCanvasRect,
    value: String,
    color: Color,
    role: TypeRole,
}

#[derive(Default)]
struct RecordingCanvas {
    fills: Vec<(AgendaCanvasRect, Color)>,
    frames: Vec<(AgendaCanvasRect, Color)>,
    texts: Vec<DrawnText>,
}

impl AgendaCanvas for RecordingCanvas {
    fn fill_rect(&mut self, rect: AgendaCanvasRect, color: Color) {
        self.fills.push((rect, color));
    }

    fn frame_rect(&mut self, rect: AgendaCanvasRect, color: Color) {
        self.frames.push((rect, color));
    }

    fn text(&mut self, rect: AgendaCanvasRect, value: &str, color: Color, role: TypeRole) {
        self.texts.push(DrawnText {
            bounds: rect,
            value: value.to_owned(),
            color,
            role,
        });
    }
}

#[test]
fn keyboard_selected_agenda_row_draws_title_schedule_and_selection() {
    let mut store = InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), 8).unwrap();
    let calendar = store.create_calendar("Agenda canvas test", None).unwrap();
    let mut adapter = LocalAdapter { store };
    let first_schedule = Schedule::Instant(Interval::new(Instant(10), Instant(20)).unwrap());
    let second_schedule = Schedule::Instant(Interval::new(Instant(30), Instant(40)).unwrap());
    let first = adapter
        .create_event(EventDraft::new(
            calendar.id.clone(),
            "Planning",
            first_schedule.clone(),
        ))
        .unwrap();
    let second = adapter
        .create_event(EventDraft::new(
            calendar.id.clone(),
            "Review",
            second_schedule.clone(),
        ))
        .unwrap();
    let listed = adapter.list(&calendar.id, 8).unwrap();
    let mut agenda = AgendaViewModel::new(&listed);
    let keys = AgendaKeyboardAdapter::new(&mut agenda);

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
    assert_eq!(adapter.get_event(&opened_id).unwrap().id, first.id);

    let mut canvas = RecordingCanvas::default();
    let mut seen_schedules = Vec::new();
    let drawn = render_agenda(
        &mut canvas,
        AgendaCanvasRect::new(10, 20, 300, 160),
        &agenda,
        ThemeMode::Light,
        |schedule| {
            seen_schedules.push(schedule.clone());
            match schedule {
                Schedule::Instant(interval) => {
                    format!("{}–{}", interval.start().0, interval.end().0)
                }
                other => format!("{other:?}"),
            }
        },
    );

    assert_eq!(drawn, 2);
    assert_eq!(seen_schedules, vec![first_schedule, second_schedule]);
    assert!(canvas.fills.contains(&(
        AgendaCanvasRect::new(10, 20, 300, 63),
        color(ThemeMode::Light, ColorRole::Selection)
    )));
    assert!(canvas.frames.contains(&(
        AgendaCanvasRect::new(10, 20, 300, 63),
        color(ThemeMode::Light, ColorRole::Focus)
    )));
    assert!(canvas
        .texts
        .iter()
        .any(|text| { text.value == "Planning" && text.role == TypeRole::BodyStrong }));
    assert!(canvas
        .texts
        .iter()
        .any(|text| { text.value == "10–20" && text.role == TypeRole::SecondaryBody }));
}
