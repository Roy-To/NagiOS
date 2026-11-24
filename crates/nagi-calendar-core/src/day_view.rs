//! Calendar 0.1 day projection. Recurrence rules do not expand the base event.

use crate::model::{Event, EventId, Interval, Schedule};
use crate::store::check_limit;
use crate::time::{
    Date, FixedOffsetResolver, Instant, RealtimeSource, TimeZoneResolver, ZoneId, DAY, SECOND,
};
use crate::{Error, Result};

pub struct DisplayPolicy {
    resolver: FixedOffsetResolver,
    zone: ZoneId,
    offset_micros: i64,
}

impl DisplayPolicy {
    pub fn tokyo() -> Result<Self> {
        Self::fixed(ZoneId::new("Asia/Tokyo")?, 9 * 3600)
    }

    pub fn fixed(zone: ZoneId, offset_seconds: i32) -> Result<Self> {
        Ok(Self {
            resolver: FixedOffsetResolver::new(zone.clone(), offset_seconds)?,
            zone,
            offset_micros: i64::from(offset_seconds) * SECOND,
        })
    }

    pub fn date_of(&self, at: Instant) -> Result<Date> {
        Ok(self.resolver.local_at(&self.zone, at)?.local.date())
    }

    pub fn day_bounds(&self, day: Date) -> Result<Interval> {
        let start = Instant(day.epoch_days() * DAY - self.offset_micros);
        Interval::new(start, start.checked_add(DAY)?)
    }

    /// The boolean indicates that the caller should show its clock unavailable status.
    pub fn initial_date(
        &self,
        clock: &dyn RealtimeSource,
        previous: Option<Date>,
        events: &[Event],
    ) -> Result<(Date, bool)> {
        match clock.realtime_ns() {
            // Day projection can discard sub-microsecond clock precision.
            Ok(ns) => Ok((self.date_of(Instant((ns / 1000) as i64))?, false)),
            Err(Error::ClockUnavailable) => {
                if let Some(day) = previous {
                    return Ok((day, true));
                }
                let earliest =
                    events
                        .iter()
                        .try_fold(None, |earliest: Option<Instant>, event| {
                            let (_, interval) = self.project(&event.draft.schedule)?;
                            Ok::<_, Error>(Some(
                                earliest
                                    .map_or(interval.start(), |start| start.min(interval.start())),
                            ))
                        })?;
                let day = match earliest {
                    Some(start) => self.date_of(start)?,
                    None => Date::new(2026, 1, 1)?,
                };
                Ok((day, true))
            }
            Err(error) => Err(error),
        }
    }

    /// The original event interval on the display time axis.
    pub fn display_interval(&self, schedule: &Schedule) -> Result<Interval> {
        Ok(self.project(schedule)?.1)
    }

    pub fn local_time(&self, instant: Instant) -> Result<crate::time::LocalDateTime> {
        Ok(self.resolver.local_at(&self.zone, instant)?.local)
    }

    fn project(&self, schedule: &Schedule) -> Result<(bool, Interval)> {
        // Translate local wall times by the same constant for comparison and sorting.
        // Only explicitly resolved Zoned values use the resolver.
        let local = |micros| Instant(micros - self.offset_micros);
        let (all_day, interval) = match schedule {
            Schedule::Instant(interval) => (false, *interval),
            Schedule::Zoned { .. } => (false, schedule.resolve(&self.resolver, None)?),
            Schedule::Floating { start, end } => (
                false,
                Interval::new(local(start.naive_micros()), local(end.naive_micros()))?,
            ),
            Schedule::AllDay {
                start,
                end_exclusive,
            } => (
                true,
                Interval::new(
                    local(start.epoch_days() * DAY),
                    local(end_exclusive.epoch_days() * DAY),
                )?,
            ),
        };
        Ok((all_day, interval))
    }
}

/// Test the original schedule against the display day's half-open interval.
/// A Zoned event in an unsupported zone returns UnknownZone.
pub fn occurs_on(event: &Event, day: Date, policy: &DisplayPolicy) -> Result<bool> {
    Ok(policy
        .project(&event.draft.schedule)?
        .1
        .overlaps(policy.day_bounds(day)?))
}

pub fn list_day(
    events: &[Event],
    day: Date,
    policy: &DisplayPolicy,
    limit: usize,
) -> Result<Vec<Event>> {
    check_limit(limit)?;
    let bounds = policy.day_bounds(day)?;
    let mut rows = Vec::new();
    for event in events {
        let (all_day, interval) = policy.project(&event.draft.schedule)?;
        if interval.overlaps(bounds) {
            if rows.len() == limit {
                return Err(Error::OutputLimitExceeded);
            }
            rows.push((
                (!all_day, interval.start(), interval.end(), &event.id),
                event,
            ));
        }
    }
    rows.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    Ok(rows.into_iter().map(|(_, event)| event.clone()).collect())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DayAgendaState {
    selected_date: Date,
    selected_event: Option<EventId>,
}

impl DayAgendaState {
    pub fn new(today: Date) -> Self {
        Self {
            selected_date: today,
            selected_event: None,
        }
    }

    pub fn selected_date(&self) -> Date {
        self.selected_date
    }

    pub fn selected_event_id(&self) -> Option<&EventId> {
        self.selected_event.as_ref()
    }

    pub fn select_event(&mut self, id: EventId) {
        self.selected_event = Some(id);
    }

    pub fn previous_day(&mut self) -> Result<()> {
        self.go_to(self.selected_date.add_days(-1)?);
        Ok(())
    }

    pub fn next_day(&mut self) -> Result<()> {
        self.go_to(self.selected_date.add_days(1)?);
        Ok(())
    }

    pub fn go_to(&mut self, day: Date) {
        if self.selected_date != day {
            self.selected_date = day;
            self.selected_event = None;
        }
    }

    pub fn reconcile(&mut self, rows: &[Event]) {
        if !rows
            .iter()
            .any(|event| Some(&event.id) == self.selected_event.as_ref())
        {
            self.selected_event = rows.first().map(|event| event.id.clone());
        }
    }
}
