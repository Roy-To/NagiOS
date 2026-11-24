//! The ordinary Calendar screen. The shell supplies the authenticated session.

use super::desktop_shell::ShellConsole;
use crate::calendar_editor_model::{CalendarEditor, EditorAction};
use crate::calendar_session::{CalendarEdit, CalendarSession, CalendarSnapshotScope, SaveRejected};
use crate::linux_task24_calendar_keys::{CalendarKey, CalendarKeys};
use crate::ui::{Painter, Rect};
use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::day_view::{list_day, DayAgendaState, DisplayPolicy};
use nagi_calendar_core::model::{Event, Schedule};
use nagi_calendar_core::time::{Date, LocalDateTime, RealtimeSource, DAY, SECOND};
use nagi_calendar_input_adapter::{AgendaKeyboardAction, AgendaKeyboardAdapter};
use nagi_identity_session::ResolutionError;
use nagi_localization::{text as localized, Locale};
use nagi_ui::interaction::KeyCode;
use nagi_ui::{color, ColorRole, ThemeMode};
use std::{io, time::Duration};

const VISIBLE: usize = 10;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CalendarExit {
    ClosedByUser,
    RequiredIdentityExited,
    StateUnknown,
    SessionEnded,
}

pub(crate) enum CalendarNotice<'a> {
    Ready {
        day: Date,
        events: usize,
    },
    Day {
        day: Date,
        rows: &'a [Event],
    },
    Saved {
        op: &'static str,
        event: &'a Event,
        bytes: &'a [u8],
    },
    SaveFailed(&'static str),
}

enum Screen {
    Agenda,
    Edit(Box<CalendarEditor>, Option<Event>),
    ConfirmDelete(Event),
}

pub(crate) fn run<C: ShellConsole, I: CalendarSnapshotScope, T: RealtimeSource>(
    console: &mut C,
    session: &mut CalendarSession<I, T>,
    clock: &dyn RealtimeSource,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), ResolutionError>,
    mut notice: impl FnMut(CalendarNotice<'_>),
) -> io::Result<CalendarExit> {
    if poll_identity()? {
        return Ok(CalendarExit::RequiredIdentityExited);
    }
    if let Some(exit) = session_exit(check_session()) {
        return Ok(exit);
    }

    let policy = DisplayPolicy::tokyo().map_err(model_error)?;
    let mut events = session.events().map_err(model_error)?;
    let (mut today, mut clock_missing) = policy
        .initial_date(clock, None, &events)
        .map_err(model_error)?;
    let mut agenda = DayAgendaState::new(today);
    let mut rows = list_day(&events, today, &policy, 1_000).map_err(model_error)?;
    agenda.reconcile(&rows);
    let mut screen = Screen::Agenda;
    let mut keys = CalendarKeys::default();
    let mut status = "";
    console.present(|painter| {
        render(
            painter,
            &screen,
            &agenda,
            &rows,
            &policy,
            today,
            clock_missing,
            status,
        )
    })?;
    notice(CalendarNotice::Ready {
        day: today,
        events: events.len(),
    });
    notice(CalendarNotice::Day {
        day: today,
        rows: &rows,
    });

    loop {
        let input = console.next_key_timeout(Duration::from_millis(100))?;
        if poll_identity()? {
            return Ok(CalendarExit::RequiredIdentityExited);
        }
        if let Some(exit) = session_exit(check_session()) {
            return Ok(exit);
        }
        let Some(key) = input.and_then(|event| keys.translate(event)) else {
            continue;
        };

        let mut operation = None;
        let old_day = agenda.selected_date();
        let selected = || {
            rows.iter()
                .find(|row| Some(&row.id) == agenda.selected_event_id())
        };
        match &mut screen {
            Screen::Agenda => match key {
                CalendarKey::Navigation(KeyCode::Escape) => return Ok(CalendarExit::ClosedByUser),
                CalendarKey::Navigation(KeyCode::ArrowLeft) => {
                    let _ = agenda.previous_day();
                }
                CalendarKey::Navigation(KeyCode::ArrowRight) => {
                    let _ = agenda.next_day();
                }
                CalendarKey::Text('t') => {
                    (today, clock_missing) = policy
                        .initial_date(clock, Some(old_day), &events)
                        .map_err(model_error)?;
                    agenda.go_to(today);
                }
                CalendarKey::Text('n') => {
                    let editor = CalendarEditor::new(session.calendar_id().clone(), old_day)
                        .map_err(model_error)?;
                    screen = Screen::Edit(Box::new(editor), None);
                }
                CalendarKey::Text('d') | CalendarKey::Delete => {
                    if let Some(event) = selected() {
                        screen = Screen::ConfirmDelete(event.clone());
                    }
                }
                CalendarKey::Navigation(key) => {
                    let mut view = AgendaViewModel::new(&rows);
                    if let Some(id) = agenda.selected_event_id() {
                        view.select_event(id);
                    }
                    let keyboard = AgendaKeyboardAdapter::new(&mut view);
                    match keyboard.handle_key(key, &mut view) {
                        Some(AgendaKeyboardAction::SelectionChanged(id)) => agenda.select_event(id),
                        Some(AgendaKeyboardAction::Open(id)) => {
                            if let Some(event) = rows.iter().find(|row| row.id == id) {
                                screen = Screen::Edit(
                                    Box::new(CalendarEditor::edit(event.draft.clone())),
                                    Some(event.clone()),
                                );
                            }
                        }
                        None => continue,
                    }
                }
                _ => continue,
            },
            Screen::ConfirmDelete(event) => {
                if matches!(
                    key,
                    CalendarKey::Text('y') | CalendarKey::Navigation(KeyCode::Enter)
                ) {
                    operation = Some((
                        "delete",
                        CalendarEdit::Delete(event.id.clone(), event.revision),
                    ));
                }
                screen = Screen::Agenda;
            }
            Screen::Edit(editor, original) => match editor.handle(key) {
                EditorAction::Cancel => {
                    screen = Screen::Agenda;
                    status = "";
                }
                EditorAction::Submit => match editor.draft() {
                    Ok(draft) => {
                        operation = Some(match original {
                            Some(event) => (
                                "update",
                                CalendarEdit::Update(event.id.clone(), event.revision, draft),
                            ),
                            None => ("create", CalendarEdit::Create(draft)),
                        });
                    }
                    Err(_) => status = localized(Locale::EnUs, "calendar.invalid_edit"),
                },
                EditorAction::Continue => {
                    status = "";
                }
            },
        }

        let mut refresh = agenda.selected_date() != old_day;
        if let Some((op, edit)) = operation {
            match session.commit(edit) {
                Ok(event) => {
                    let bytes = session.snapshot_bytes().map_err(model_error)?;
                    notice(CalendarNotice::Saved {
                        op,
                        event: &event,
                        bytes: &bytes,
                    });
                    if op != "delete" {
                        agenda.select_event(event.id);
                    }
                    screen = Screen::Agenda;
                    status = "";
                    refresh = true;
                    events = session.events().map_err(model_error)?;
                }
                Err(SaveRejected::Conflict | SaveRejected::StateUnknown) => {
                    notice(CalendarNotice::SaveFailed("state_unknown"));
                    return Ok(CalendarExit::StateUnknown);
                }
                Err(_) => {
                    status = localized(Locale::EnUs, "calendar.save_failed");
                    notice(CalendarNotice::SaveFailed("not_applied"));
                }
            }
        }
        if refresh {
            rows =
                list_day(&events, agenda.selected_date(), &policy, 1_000).map_err(model_error)?;
            agenda.reconcile(&rows);
            notice(CalendarNotice::Day {
                day: agenda.selected_date(),
                rows: &rows,
            });
        }
        console.present(|painter| {
            render(
                painter,
                &screen,
                &agenda,
                &rows,
                &policy,
                today,
                clock_missing,
                status,
            )
        })?;
    }
}

fn session_exit(result: Result<(), ResolutionError>) -> Option<CalendarExit> {
    match result {
        Ok(()) => None,
        Err(ResolutionError::Ended | ResolutionError::Expired) => Some(CalendarExit::SessionEnded),
        Err(_) => Some(CalendarExit::StateUnknown),
    }
}

pub(super) fn unavailable<C: ShellConsole>(
    console: &mut C,
    mut poll_identity: impl FnMut() -> io::Result<bool>,
    mut check_session: impl FnMut() -> Result<(), ResolutionError>,
) -> io::Result<CalendarExit> {
    console.present(|painter| {
        painter.fill(
            Rect::new(0, 0, 320, 200),
            color(ThemeMode::Light, ColorRole::Canvas).to_pixel(),
        );
        painter.text(
            8,
            24,
            localized(Locale::EnUs, "calendar.load_failed").as_bytes(),
            color(ThemeMode::Light, ColorRole::TextPrimary).to_pixel(),
        );
    })?;
    let mut keys = CalendarKeys::default();
    loop {
        let input = console.next_key_timeout(Duration::from_millis(100))?;
        if poll_identity()? {
            return Ok(CalendarExit::RequiredIdentityExited);
        }
        if let Some(exit) = session_exit(check_session()) {
            return Ok(exit);
        }
        if input.and_then(|event| keys.translate(event))
            == Some(CalendarKey::Navigation(KeyCode::Escape))
        {
            return Ok(CalendarExit::ClosedByUser);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CalendarClock;

impl RealtimeSource for CalendarClock {
    fn realtime_ns(&self) -> nagi_calendar_core::Result<u64> {
        use nagi_calendar_core::Error::ClockUnavailable;
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ClockUnavailable)?
            .as_nanos()
            .try_into()
            .map_err(|_| ClockUnavailable)
    }
}

pub(super) fn log_notice(notice: CalendarNotice<'_>) {
    use super::super::log_pid1_stderr;
    use sha2::{Digest, Sha256};
    match notice {
        CalendarNotice::Ready { day, events } => log_pid1_stderr(format_args!(
            "NAGI_DEV_CALENDAR_READY events={events} date={}",
            date_label(day)
        )),
        CalendarNotice::Day { day, rows } => {
            let ids = rows
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            log_pid1_stderr(format_args!(
                "NAGI_DEV_CALENDAR_DAY date={} rows={} ids={ids}",
                date_label(day),
                rows.len()
            ));
        }
        CalendarNotice::Saved { op, event, bytes } => log_pid1_stderr(format_args!(
            "NAGI_DEV_CALENDAR_SAVED op={op} id={} bytes={} sha256={:x}",
            event.id.as_str(),
            bytes.len(),
            Sha256::digest(bytes)
        )),
        CalendarNotice::SaveFailed(reason) => log_pid1_stderr(format_args!(
            "NAGI_DEV_CALENDAR_SAVE_FAILED reason={reason}"
        )),
    }
}

fn model_error(error: nagi_calendar_core::Error) -> io::Error {
    io::Error::other(format!("Calendar: {error:?}"))
}

pub(crate) fn date_label(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

fn time_label(time: LocalDateTime) -> String {
    let minutes = time.naive_micros().rem_euclid(DAY) / (60 * SECOND);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn schedule_label(schedule: &Schedule, day: Date, policy: &DisplayPolicy) -> String {
    if matches!(schedule, Schedule::AllDay { .. }) {
        return localized(Locale::EnUs, "calendar.all_day").into();
    }
    let format = || {
        let interval = policy.display_interval(schedule)?;
        let start = policy.local_time(interval.start())?;
        let end = policy.local_time(interval.end())?;
        Ok::<_, nagi_calendar_core::Error>(format!(
            "{}{}-{}{}",
            if start.date() < day { "-" } else { "" },
            time_label(start),
            time_label(end),
            if end.date() > day { "+" } else { "" },
        ))
    };
    format().unwrap_or_else(|_| "?".into())
}

#[allow(clippy::too_many_arguments)]
fn render(
    painter: &mut Painter<'_>,
    screen: &Screen,
    agenda: &DayAgendaState,
    rows: &[Event],
    policy: &DisplayPolicy,
    today: Date,
    clock_missing: bool,
    status: &str,
) {
    let ink = |role| color(ThemeMode::Light, role).to_pixel();
    painter.fill(Rect::new(0, 0, 320, 200), ink(ColorRole::Canvas));
    let text = |p: &mut Painter<'_>, y, value: &str| {
        let bounded: String = value.chars().take(44).collect();
        p.text(8, y, bounded.as_bytes(), ink(ColorRole::TextPrimary));
    };
    match screen {
        Screen::Agenda | Screen::ConfirmDelete(_) => {
            let day = agenda.selected_date();
            let tag = if clock_missing {
                localized(Locale::EnUs, "calendar.clock_unavailable")
            } else if day == today {
                localized(Locale::EnUs, "calendar.today")
            } else {
                ""
            };
            text(painter, 5, &format!("{} {}", date_label(day), tag));
            let selected = rows
                .iter()
                .position(|row| Some(&row.id) == agenda.selected_event_id())
                .unwrap_or(0);
            let first = selected.saturating_sub(VISIBLE - 1);
            if rows.is_empty() {
                text(painter, 24, localized(Locale::EnUs, "calendar.empty"));
            }
            for (index, event) in rows.iter().enumerate().skip(first).take(VISIBLE) {
                let y = 22 + (index - first) as i32 * 14;
                if index == selected {
                    painter.frame(Rect::new(4, y - 1, 312, 14), ink(ColorRole::Focus));
                }
                text(
                    painter,
                    y,
                    &format!(
                        "{}  {}",
                        schedule_label(&event.draft.schedule, day, policy),
                        event.draft.title
                    ),
                );
            }
            text(
                painter,
                177,
                if matches!(screen, Screen::ConfirmDelete(_)) {
                    localized(Locale::EnUs, "calendar.delete_hint")
                } else {
                    localized(Locale::EnUs, "calendar.agenda_hint")
                },
            );
        }
        Screen::Edit(editor, _) => {
            for (index, label) in ["title", "start_date", "start_time", "end_date", "end_time"]
                .iter()
                .enumerate()
            {
                let y = 17 + index as i32 * 26;
                if editor.focused() == index {
                    painter.frame(Rect::new(4, y - 3, 312, 17), ink(ColorRole::Focus));
                }
                let value = &editor.fields()[index];
                let display = match (index, value.len()) {
                    (1 | 3, 8) => format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..]),
                    (2 | 4, 4) => format!("{}:{}", &value[..2], &value[2..]),
                    _ => value.clone(),
                };
                let label = localized(Locale::EnUs, &format!("calendar.field.{label}"));
                text(painter, y, &format!("{label}: {display}"));
            }
            text(painter, 177, localized(Locale::EnUs, "calendar.edit_hint"));
        }
    }
    if !status.is_empty() {
        text(painter, 188, status);
    }
}
