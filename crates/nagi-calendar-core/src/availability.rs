use crate::model::{EventDraft, Interval, ZoneContext};
use crate::recurrence::{expand, ExpansionLimits};
use crate::store::check_limit;
use crate::time::{Instant, TimeZoneResolver};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct AvailabilityRequest {
    pub window: Interval,
    pub duration_micros: i64,
    pub step_micros: i64,
    pub buffer_before_micros: i64,
    pub buffer_after_micros: i64,
    pub max_scan: usize,
    pub max_slots: usize,
    /// None means participant data unavailable; empty vector means no working time.
    pub participant_working_windows: Vec<Option<Vec<Interval>>>,
}
pub fn normalize_busy(
    window: Interval,
    busy: &[Interval],
    before: i64,
    after: i64,
) -> Result<Vec<Interval>> {
    if before < 0 || after < 0 {
        return Err(Error::InvalidRange);
    }
    if busy.len() > 65_536 {
        return Err(Error::Capacity);
    }
    let mut clipped = vec![];
    for interval in busy {
        let start = interval.start().checked_add(-before)?.max(window.start());
        let end = interval.end().checked_add(after)?.min(window.end());
        if start < end {
            clipped.push(Interval::new(start, end)?);
        }
    }
    clipped.sort_by_key(|i| i.start());
    let mut merged: Vec<Interval> = vec![];
    for i in clipped {
        if let Some(last) = merged.last_mut() {
            if i.start() <= last.end() {
                *last = Interval::new(last.start(), last.end().max(i.end()))?;
                continue;
            }
        }
        merged.push(i);
    }
    Ok(merged)
}
pub fn free_windows(window: Interval, busy: &[Interval]) -> Result<Vec<Interval>> {
    let busy = normalize_busy(window, busy, 0, 0)?;
    let mut free = vec![];
    let mut cursor = window.start();
    for i in busy {
        if cursor < i.start() {
            free.push(Interval::new(cursor, i.start())?);
        }
        cursor = i.end();
    }
    if cursor < window.end() {
        free.push(Interval::new(cursor, window.end())?);
    }
    Ok(free)
}
pub fn find_availability(
    request: &AvailabilityRequest,
    busy: &[Interval],
) -> Result<Vec<Interval>> {
    check_limit(request.max_scan)?;
    check_limit(request.max_slots)?;
    if request.duration_micros <= 0 {
        return Err(Error::InvalidRange);
    }
    if request.step_micros <= 0 {
        return Err(Error::InvalidStep);
    }
    if request.participant_working_windows.len() > 256 {
        return Err(Error::Capacity);
    }
    let busy = normalize_busy(
        request.window,
        busy,
        request.buffer_before_micros,
        request.buffer_after_micros,
    )?;
    let mut free = free_windows(request.window, &busy)?;
    for participant in &request.participant_working_windows {
        let available = participant.as_ref().ok_or(Error::UnknownAvailability)?;
        let working = normalize_busy(request.window, available, 0, 0)?;
        let mut intersected = vec![];
        // Both lists are disjoint/sorted: intersect linearly, with bounded allocation.
        let (mut a, mut b) = (0, 0);
        while a < free.len() && b < working.len() {
            let start = free[a].start().max(working[b].start());
            let end = free[a].end().min(working[b].end());
            if start < end {
                if intersected.len() == 65_536 {
                    return Err(Error::Capacity);
                }
                intersected.push(Interval::new(start, end)?);
            }
            if free[a].end() < working[b].end() {
                a += 1;
            } else {
                b += 1;
            }
        }
        free = intersected;
    }
    let mut slots = vec![];
    let mut scanned = 0usize;
    for window in free {
        let delta = window
            .start()
            .0
            .checked_sub(request.window.start().0)
            .ok_or(Error::Overflow)?;
        let steps = delta / request.step_micros + i64::from(delta % request.step_micros != 0);
        let mut cursor = request.window.start().checked_add(
            steps
                .checked_mul(request.step_micros)
                .ok_or(Error::Overflow)?,
        )?;
        loop {
            let end = cursor.checked_add(request.duration_micros)?;
            if end > window.end() {
                break;
            }
            if scanned == request.max_scan {
                return Err(Error::ScanLimitExceeded);
            }
            scanned += 1;
            if slots.len() == request.max_slots {
                return Err(Error::OutputLimitExceeded);
            }
            slots.push(Interval::new(cursor, end)?);
            cursor = cursor.checked_add(request.step_micros)?;
        }
    }
    Ok(slots)
}
/// Resolve all events before filtering transparent events, so invalid input is visible.
pub fn busy_from_events(
    events: &[EventDraft],
    window: Interval,
    resolver: &dyn TimeZoneResolver,
    context: Option<&ZoneContext>,
    limits: ExpansionLimits,
) -> Result<Vec<Interval>> {
    if events.len() > 4096 {
        return Err(Error::Capacity);
    }
    let mut busy = vec![];
    for e in events {
        let occurrences = expand(e, window, resolver, context, limits)?;
        if e.busy {
            for occurrence in occurrences {
                if busy.len() == 65_536 {
                    return Err(Error::Capacity);
                }
                busy.push(occurrence.interval);
            }
        }
    }
    normalize_busy(window, &busy, 0, 0)
}
/// Convenience for fixture data on an arbitrary instant axis.
pub fn interval(start: i64, end: i64) -> Result<Interval> {
    Interval::new(Instant(start), Instant(end))
}
