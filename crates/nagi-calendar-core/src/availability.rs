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
        // The request window spans at most i64::MAX, so a grid point whose offset
        // or position is unrepresentable lies beyond this free window: no candidate.
        let Some(mut cursor) = steps
            .checked_mul(request.step_micros)
            .and_then(|offset| request.window.start().0.checked_add(offset))
            .map(Instant)
        else {
            continue;
        };
        // Durations are positive, so a start at or past the window end cannot fit.
        // Stopping here is normal grid termination, not an arithmetic failure.
        while cursor < window.end() {
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
            match cursor.0.checked_add(request.step_micros) {
                Some(next) => cursor = Instant(next),
                // The next grid point is past i64::MAX >= window end: grid exhausted.
                None => break,
            }
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

#[cfg(test)]
mod boundary_tests {
    use super::*;

    const MAX: i64 = i64::MAX;

    fn req(start: i64, end: i64, duration: i64, step: i64) -> AvailabilityRequest {
        AvailabilityRequest {
            window: interval(start, end).unwrap(),
            duration_micros: duration,
            step_micros: step,
            buffer_before_micros: 0,
            buffer_after_micros: 0,
            max_scan: 100,
            max_slots: 100,
            participant_working_windows: vec![],
        }
    }
    fn pairs(slots: Result<Vec<Interval>>) -> Result<Vec<(i64, i64)>> {
        slots.map(|s| s.iter().map(|i| (i.start().0, i.end().0)).collect())
    }

    #[test]
    fn final_candidate_ending_at_max_is_returned() {
        let r = req(MAX - 1, MAX, 1, 1);
        assert_eq!(pairs(find_availability(&r, &[])), Ok(vec![(MAX - 1, MAX)]));
    }

    #[test]
    fn grid_stops_when_next_start_overflows() {
        // Slots at MAX-4 and MAX-1; the next start (MAX+2) is unrepresentable.
        let r = req(MAX - 4, MAX, 1, 3);
        assert_eq!(
            pairs(find_availability(&r, &[])),
            Ok(vec![(MAX - 4, MAX - 3), (MAX - 1, MAX)])
        );
    }

    #[test]
    fn grid_stops_when_next_start_reaches_window_end_at_max() {
        // Two exact-fit slots; the next start equals MAX == window end.
        let r = req(MAX - 4, MAX, 2, 2);
        assert_eq!(
            pairs(find_availability(&r, &[])),
            Ok(vec![(MAX - 4, MAX - 2), (MAX - 2, MAX)])
        );
    }

    #[test]
    fn maximal_duration_and_step_fill_widest_window_once() {
        let r = req(0, MAX, MAX, MAX);
        assert_eq!(pairs(find_availability(&r, &[])), Ok(vec![(0, MAX)]));
    }

    #[test]
    fn huge_step_yields_only_first_candidate() {
        let r = req(0, MAX, 1, MAX);
        assert_eq!(pairs(find_availability(&r, &[])), Ok(vec![(0, 1)]));
        let r = req(-10, 10, 1, MAX);
        assert_eq!(pairs(find_availability(&r, &[])), Ok(vec![(-10, -9)]));
    }

    #[test]
    fn aligned_first_candidate_beyond_free_window_is_skipped() {
        // Grid anchored at MAX-10 with step 10: the next grid point is MAX,
        // which is the window end, so free [MAX-5, MAX) has no candidate.
        let r = req(MAX - 10, MAX, 1, 10);
        let busy = [interval(MAX - 10, MAX - 5).unwrap()];
        assert_eq!(pairs(find_availability(&r, &busy)), Ok(vec![]));
    }

    #[test]
    fn aligned_first_candidate_unrepresentable_is_skipped() {
        // Grid anchored at 1 with step MAX: next grid point 1+MAX overflows,
        // so free [2, MAX) has no candidate.
        let r = req(1, MAX, 1, MAX);
        let busy = [interval(1, 2).unwrap()];
        assert_eq!(pairs(find_availability(&r, &busy)), Ok(vec![]));
    }

    #[test]
    fn half_open_exact_fit_and_endpoint_contact() {
        // Exact fit: window length == duration.
        assert_eq!(
            pairs(find_availability(&req(0, 30, 30, 30), &[])),
            Ok(vec![(0, 30)])
        );
        // A slot may end exactly where busy starts and start where busy ends.
        let busy = [interval(30, 60).unwrap()];
        assert_eq!(
            pairs(find_availability(&req(0, 90, 30, 30), &busy)),
            Ok(vec![(0, 30), (60, 90)])
        );
        // Window one micro shorter than duration: no slot.
        assert_eq!(
            pairs(find_availability(&req(0, 29, 30, 1), &[])),
            Ok(vec![])
        );
    }

    #[test]
    fn empty_free_time_yields_no_slots() {
        let r = req(MAX - 10, MAX, 1, 1);
        assert_eq!(pairs(find_availability(&r, &[r.window])), Ok(vec![]));
        let mut r = req(0, 100, 10, 10);
        r.participant_working_windows = vec![Some(vec![])];
        assert_eq!(pairs(find_availability(&r, &[])), Ok(vec![]));
    }

    #[test]
    fn limits_still_enforced_at_max_boundary() {
        let mut r = req(MAX - 3, MAX, 1, 1);
        r.max_slots = 3;
        r.max_scan = 3;
        assert_eq!(
            pairs(find_availability(&r, &[])),
            Ok(vec![(MAX - 3, MAX - 2), (MAX - 2, MAX - 1), (MAX - 1, MAX)])
        );
        r.max_slots = 2;
        assert_eq!(find_availability(&r, &[]), Err(Error::OutputLimitExceeded));
        r.max_slots = 3;
        r.max_scan = 2;
        assert_eq!(find_availability(&r, &[]), Err(Error::ScanLimitExceeded));
    }

    #[test]
    fn invalid_step_and_duration_contract_unchanged() {
        for step in [0, -1, i64::MIN] {
            assert_eq!(
                find_availability(&req(0, 10, 1, step), &[]),
                Err(Error::InvalidStep)
            );
        }
        for duration in [0, -1, i64::MIN] {
            assert_eq!(
                find_availability(&req(0, 10, duration, 1), &[]),
                Err(Error::InvalidRange)
            );
        }
        let mut r = req(0, 10, 1, 1);
        r.max_scan = 0;
        assert_eq!(find_availability(&r, &[]), Err(Error::InvalidLimit));
    }

    #[test]
    fn evaluated_candidate_end_overflow_still_errors() {
        // Existing contract: the first in-window candidate's end overflows.
        assert_eq!(
            find_availability(&req(MAX - 10, MAX, 20, 1), &[]),
            Err(Error::Overflow)
        );
        // An in-window candidate (MAX-4) whose end MAX+1 is unrepresentable
        // must still be evaluated and still reports Overflow.
        assert_eq!(
            find_availability(&req(MAX - 10, MAX, 5, 3), &[]),
            Err(Error::Overflow)
        );
    }
}
