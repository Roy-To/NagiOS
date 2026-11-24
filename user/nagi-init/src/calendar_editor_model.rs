//! Pure editor for the first Linux Calendar console.

use crate::linux_task24_calendar_keys::CalendarKey;
use nagi_calendar_core::model::{CalendarId, DurationMode, EventDraft, Schedule};
use nagi_calendar_core::time::{Date, LocalDateTime, ResolveChoice, ZoneId, DAY, SECOND};
use nagi_calendar_core::{Error, Result};
use nagi_ui::interaction::KeyCode;

pub(crate) enum EditorAction {
    Continue,
    Cancel,
    Submit,
}

pub(crate) struct CalendarEditor {
    original: EventDraft,
    fields: [String; 5],
    focused: usize,
    editable_times: bool,
}

impl CalendarEditor {
    pub(crate) fn new(calendar: CalendarId, date: Date) -> Result<Self> {
        let start = LocalDateTime::new(date, 9, 0, 0, 0)?;
        let end = LocalDateTime::new(date, 10, 0, 0, 0)?;
        Ok(Self::edit(EventDraft::new(
            calendar,
            "",
            Schedule::Zoned {
                start,
                end,
                zone: ZoneId::new("Asia/Tokyo")?,
                start_choice: ResolveChoice::Earlier,
                end_choice: ResolveChoice::Earlier,
                duration_mode: DurationMode::Wall,
            },
        )))
    }

    pub(crate) fn edit(original: EventDraft) -> Self {
        let mut fields = std::array::from_fn(|_| String::new());
        fields[0] = original.title.clone();
        let editable_times = match &original.schedule {
            Schedule::Zoned {
                start, end, zone, ..
            } if zone.as_str() == "Asia/Tokyo" && original.recurrence.is_none() => {
                fields[1] = date_digits(start.date());
                fields[2] = time_digits(*start);
                fields[3] = date_digits(end.date());
                fields[4] = time_digits(*end);
                [start, end]
                    .iter()
                    .all(|time| time.naive_micros() % (60 * SECOND) == 0)
            }
            _ => false,
        };
        Self {
            original,
            fields,
            focused: 0,
            editable_times,
        }
    }

    pub(crate) fn fields(&self) -> &[String; 5] {
        &self.fields
    }

    pub(crate) fn focused(&self) -> usize {
        self.focused
    }

    pub(crate) fn handle(&mut self, key: CalendarKey) -> EditorAction {
        use CalendarKey::{Backspace, Navigation, Text};
        let count = if self.editable_times { 5 } else { 1 };
        match key {
            Navigation(KeyCode::Escape) => return EditorAction::Cancel,
            Navigation(KeyCode::Enter) => return EditorAction::Submit,
            Navigation(KeyCode::Tab | KeyCode::ArrowDown) => {
                self.focused = (self.focused + 1) % count
            }
            Navigation(KeyCode::ArrowUp) => self.focused = (self.focused + count - 1) % count,
            Backspace => {
                self.fields[self.focused].pop();
            }
            Text(ch) => {
                let field = &mut self.fields[self.focused];
                let max = match self.focused {
                    0 => 64,
                    1 | 3 => 8,
                    _ => 4,
                };
                let valid = ch.is_ascii_digit()
                    || (self.focused == 0 && (ch.is_ascii_lowercase() || matches!(ch, ' ' | '-')));
                if valid && field.chars().count() < max {
                    field.push(ch);
                }
            }
            _ => {}
        }
        EditorAction::Continue
    }

    pub(crate) fn draft(&self) -> Result<EventDraft> {
        let mut draft = self.original.clone();
        if self.fields[0].trim().is_empty() {
            return Err(Error::InvalidText);
        }
        draft.title.clone_from(&self.fields[0]);
        if self.editable_times {
            let Schedule::Zoned { start, end, .. } = &mut draft.schedule else {
                unreachable!("editable times require a Zoned schedule");
            };
            *start = parse_datetime(&self.fields[1], &self.fields[2])?;
            *end = parse_datetime(&self.fields[3], &self.fields[4])?;
        }
        draft.validate()?;
        Ok(draft)
    }
}

fn date_digits(date: Date) -> String {
    format!("{:04}{:02}{:02}", date.year(), date.month(), date.day())
}

fn time_digits(local: LocalDateTime) -> String {
    let minutes = local.naive_micros().rem_euclid(DAY) / (60 * SECOND);
    format!("{:02}{:02}", minutes / 60, minutes % 60)
}

fn parse_datetime(date: &str, time: &str) -> Result<LocalDateTime> {
    if date.len() != 8
        || time.len() != 4
        || !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit())
    {
        return Err(Error::InvalidDate);
    }
    fn number<T: std::str::FromStr>(input: &str) -> Result<T> {
        input.parse().map_err(|_| Error::InvalidDate)
    }
    let day = Date::new(
        number(&date[..4])?,
        number(&date[4..6])?,
        number(&date[6..])?,
    )?;
    LocalDateTime::new(day, number(&time[..2])?, number(&time[2..])?, 0, 0)
}
