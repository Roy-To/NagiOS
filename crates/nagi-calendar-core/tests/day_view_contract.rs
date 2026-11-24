use nagi_calendar_core::day_view::{list_day, occurs_on, DayAgendaState, DisplayPolicy};
use nagi_calendar_core::model::{
    CalendarId, DurationMode, Event, EventDraft, EventId, EventRevision, Interval, Schedule,
};
use nagi_calendar_core::recurrence::{Frequency, Recurrence, RecurrenceBound};
use nagi_calendar_core::time::{
    Date, FixedClock, Instant, LocalDateTime, ResolveChoice, UnavailableClock, ZoneId, SECOND,
};
use nagi_calendar_core::Error;

fn date(year: u16, month: u8, day: u8) -> Date {
    Date::new(year, month, day).unwrap()
}

fn local(day: Date, hour: u8) -> LocalDateTime {
    LocalDateTime::new(day, hour, 0, 0, 0).unwrap()
}

fn event(id: &str, schedule: Schedule) -> Event {
    Event {
        id: EventId::new(id).unwrap(),
        revision: EventRevision::new(1).unwrap(),
        created_at: Instant(0),
        modified_at: Instant(0),
        draft: EventDraft::new(CalendarId::new("cal_1").unwrap(), "Same title", schedule),
    }
}

#[test]
fn tokyo_midnight_uses_half_open_intervals_in_all_schedule_domains() {
    let policy = DisplayPolicy::tokyo().unwrap();
    let day = date(2026, 11, 23);
    let next = day.add_days(1).unwrap();
    let midnight = policy.day_bounds(next).unwrap().start();
    assert_eq!(
        midnight.0,
        LocalDateTime::midnight(next).naive_micros() - 9 * 3600 * SECOND
    );
    assert_eq!(policy.date_of(midnight).unwrap(), next);
    assert_eq!(
        policy.date_of(midnight.checked_add(-1).unwrap()).unwrap(),
        day
    );

    let schedules = [
        Schedule::Instant(Interval::new(midnight.checked_add(-SECOND).unwrap(), midnight).unwrap()),
        Schedule::Floating {
            start: local(day, 23),
            end: local(next, 0),
        },
        Schedule::Zoned {
            start: local(day, 23),
            end: local(next, 0),
            zone: ZoneId::new("Asia/Tokyo").unwrap(),
            start_choice: ResolveChoice::Reject,
            end_choice: ResolveChoice::Reject,
            duration_mode: DurationMode::Wall,
        },
        Schedule::AllDay {
            start: day,
            end_exclusive: next,
        },
    ];
    for schedule in schedules {
        let e = event("event_end", schedule);
        assert!(occurs_on(&e, day, &policy).unwrap());
        assert!(!occurs_on(&e, next, &policy).unwrap());
    }
    assert_eq!(Interval::new(midnight, midnight), Err(Error::InvalidRange));

    let crossing = event(
        "event_crossing",
        Schedule::Floating {
            start: local(day, 23),
            end: local(next, 1),
        },
    );
    assert!(occurs_on(&crossing, day, &policy).unwrap());
    assert!(occurs_on(&crossing, next, &policy).unwrap());
    assert!(!occurs_on(&crossing, next.add_days(1).unwrap(), &policy).unwrap());
}

#[test]
fn day_rows_sort_by_all_day_start_end_and_id_without_expanding_recurrences() {
    let policy = DisplayPolicy::tokyo().unwrap();
    let day = date(2026, 11, 23);
    let next = day.add_days(1).unwrap();
    let floating = |end| Schedule::Floating {
        start: local(day, 9),
        end: local(day, end),
    };
    let mut a = event("event_a", floating(10));
    a.draft.recurrence = Some(Recurrence {
        frequency: Frequency::Daily,
        interval: 1,
        bound: RecurrenceBound::Count(3),
    });
    let start = policy
        .day_bounds(day)
        .unwrap()
        .start()
        .checked_add(9 * 3600 * SECOND)
        .unwrap();
    let b = event(
        "event_b",
        Schedule::Instant(Interval::new(start, start.checked_add(3600 * SECOND).unwrap()).unwrap()),
    );
    let c = event("event_c", floating(11));
    let all = event(
        "event_all",
        Schedule::AllDay {
            start: day,
            end_exclusive: next,
        },
    );
    let tomorrow = event(
        "event_tomorrow",
        Schedule::Floating {
            start: local(next, 9),
            end: local(next, 10),
        },
    );
    let input = [
        c.clone(),
        tomorrow.clone(),
        b.clone(),
        a.clone(),
        all.clone(),
    ];
    assert_eq!(list_day(&input, day, &policy, 4).unwrap(), [all, a, b, c]);
    assert_eq!(
        list_day(&input, day, &policy, 3),
        Err(Error::OutputLimitExceeded)
    );
    assert_eq!(list_day(&input, next, &policy, 1).unwrap(), [tomorrow]);
}

#[test]
fn navigation_and_schedule_edit_reconcile_selection_by_id() {
    let policy = DisplayPolicy::tokyo().unwrap();
    let day = date(2024, 2, 29);
    let next = date(2024, 3, 1);
    let mut e = event(
        "event_moved",
        Schedule::Floating {
            start: local(day, 9),
            end: local(day, 10),
        },
    );
    let mut state = DayAgendaState::new(day);
    state.previous_day().unwrap();
    assert!(
        list_day(std::slice::from_ref(&e), state.selected_date(), &policy, 1)
            .unwrap()
            .is_empty()
    );
    state.next_day().unwrap();
    state
        .reconcile(&list_day(std::slice::from_ref(&e), state.selected_date(), &policy, 1).unwrap());
    assert_eq!(state.selected_event_id(), Some(&e.id));

    e.draft.schedule = Schedule::Floating {
        start: local(next, 9),
        end: local(next, 10),
    };
    state.reconcile(&list_day(std::slice::from_ref(&e), day, &policy, 1).unwrap());
    assert_eq!(state.selected_event_id(), None);
    state.next_day().unwrap();
    assert_eq!(state.selected_date(), next);
    state.reconcile(&list_day(std::slice::from_ref(&e), next, &policy, 1).unwrap());
    assert_eq!(state.selected_event_id(), Some(&e.id));
    state.go_to(date(2026, 12, 31));
    assert_eq!(state.selected_event_id(), None);
    state.next_day().unwrap();
    assert_eq!(state.selected_date(), date(2027, 1, 1));
}

#[test]
fn unavailable_clock_uses_previous_then_first_event_then_default() {
    let policy = DisplayPolicy::tokyo().unwrap();
    let day = date(2026, 11, 23);
    let e = event(
        "event_a",
        Schedule::Floating {
            start: local(day, 9),
            end: local(day, 10),
        },
    );
    assert_eq!(
        policy.initial_date(&UnavailableClock, None, &[]).unwrap(),
        (date(2026, 1, 1), true)
    );
    assert_eq!(
        policy
            .initial_date(&UnavailableClock, None, std::slice::from_ref(&e))
            .unwrap(),
        (day, true)
    );
    assert_eq!(
        policy
            .initial_date(&UnavailableClock, Some(day.add_days(1).unwrap()), &[e])
            .unwrap(),
        (day.add_days(1).unwrap(), true)
    );
    let start = policy.day_bounds(day).unwrap().start();
    let now = FixedClock(u64::try_from(start.0).unwrap() * 1000 + 999);
    assert_eq!(policy.initial_date(&now, None, &[]).unwrap(), (day, false));
}
