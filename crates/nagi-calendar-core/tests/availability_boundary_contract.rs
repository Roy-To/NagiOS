use nagi_calendar_core::availability::{find_availability, interval, AvailabilityRequest};
use nagi_calendar_core::model::Interval;
use nagi_calendar_core::{Error, Result};

const I64_MIN: i128 = i64::MIN as i128;
const I64_MAX: i128 = i64::MAX as i128;

fn request(start: i64, end: i64, duration: i64, step: i64) -> AvailabilityRequest {
    AvailabilityRequest {
        window: interval(start, end).unwrap(),
        duration_micros: duration,
        step_micros: step,
        buffer_before_micros: 0,
        buffer_after_micros: 0,
        max_scan: 128,
        max_slots: 128,
        participant_working_windows: vec![],
    }
}

fn pairs(slots: Vec<Interval>) -> Vec<(i64, i64)> {
    slots
        .into_iter()
        .map(|slot| (slot.start().0, slot.end().0))
        .collect()
}

fn normalize_reference(
    window: (i128, i128),
    intervals: &[Interval],
    before: i64,
    after: i64,
) -> Result<Vec<(i128, i128)>> {
    if before < 0 || after < 0 {
        return Err(Error::InvalidRange);
    }
    if intervals.len() > 65_536 {
        return Err(Error::Capacity);
    }

    let mut clipped = Vec::new();
    for interval in intervals {
        let start = i128::from(interval.start().0) - i128::from(before);
        let end = i128::from(interval.end().0) + i128::from(after);
        if start < I64_MIN || end > I64_MAX {
            return Err(Error::Overflow);
        }
        let start = start.max(window.0);
        let end = end.min(window.1);
        if start < end {
            clipped.push((start, end));
        }
    }

    clipped.sort_unstable_by_key(|entry| entry.0);
    let mut merged: Vec<(i128, i128)> = Vec::new();
    for entry in clipped {
        if let Some(last) = merged.last_mut() {
            if entry.0 <= last.1 {
                last.1 = last.1.max(entry.1);
                continue;
            }
        }
        merged.push(entry);
    }
    Ok(merged)
}

fn reference_enumeration(
    request: &AvailabilityRequest,
    busy: &[Interval],
) -> Result<Vec<(i64, i64)>> {
    if request.duration_micros <= 0 {
        return Err(Error::InvalidRange);
    }
    if request.step_micros <= 0 {
        return Err(Error::InvalidStep);
    }
    if request.participant_working_windows.len() > 256 {
        return Err(Error::Capacity);
    }

    let request_window = (
        i128::from(request.window.start().0),
        i128::from(request.window.end().0),
    );
    let busy = normalize_reference(
        request_window,
        busy,
        request.buffer_before_micros,
        request.buffer_after_micros,
    )?;

    let mut free = Vec::new();
    let mut cursor = request_window.0;
    for busy_window in busy {
        if cursor < busy_window.0 {
            free.push((cursor, busy_window.0));
        }
        cursor = busy_window.1;
    }
    if cursor < request_window.1 {
        free.push((cursor, request_window.1));
    }

    for participant in &request.participant_working_windows {
        let Some(windows) = participant.as_ref() else {
            return Err(Error::UnknownAvailability);
        };
        let working = normalize_reference(request_window, windows, 0, 0)?;
        let mut intersection = Vec::new();
        let (mut free_index, mut working_index) = (0, 0);
        while free_index < free.len() && working_index < working.len() {
            let start = free[free_index].0.max(working[working_index].0);
            let end = free[free_index].1.min(working[working_index].1);
            if start < end {
                if intersection.len() == 65_536 {
                    return Err(Error::Capacity);
                }
                intersection.push((start, end));
            }
            if free[free_index].1 < working[working_index].1 {
                free_index += 1;
            } else {
                working_index += 1;
            }
        }
        free = intersection;
    }

    let origin = request_window.0;
    let step = i128::from(request.step_micros);
    let duration = i128::from(request.duration_micros);
    let mut output = Vec::new();
    let mut scanned = 0usize;
    for available in free {
        let distance = available.0 - origin;
        let first_index = (distance + step - 1) / step;
        let mut candidate = origin + first_index * step;
        while candidate < available.1 {
            let end = candidate + duration;
            if end > I64_MAX {
                return Err(Error::Overflow);
            }
            if end > available.1 {
                break;
            }
            if scanned == request.max_scan {
                return Err(Error::ScanLimitExceeded);
            }
            scanned += 1;
            if output.len() == request.max_slots {
                return Err(Error::OutputLimitExceeded);
            }
            output.push((candidate as i64, end as i64));
            candidate += step;
        }
    }
    Ok(output)
}

#[test]
fn bounded_windows_match_i128_reference_at_signed_limits() {
    let starts = [i64::MIN, i64::MIN + 3, -3, 0, i64::MAX - 8, i64::MAX - 3];
    for start in starts {
        for width in 1_i128..=5 {
            let end = i128::from(start) + width;
            if end > I64_MAX {
                continue;
            }
            for duration in 1_i64..=5 {
                for step in 1_i64..=5 {
                    let request = request(start, end as i64, duration, step);
                    let expected = reference_enumeration(&request, &[]);

                    assert_eq!(
                        find_availability(&request, &[]).map(pairs),
                        expected,
                        "window=({start},{end}) duration={duration} step={step}"
                    );
                }
            }
        }
    }
}

#[test]
fn maximum_supported_span_keeps_its_slot_and_wider_spans_are_rejected() {
    assert_eq!(interval(i64::MIN, i64::MAX), Err(Error::Overflow));
    let request = request(i64::MIN, -1, i64::MAX, i64::MAX);
    assert_eq!(
        find_availability(&request, &[]).map(pairs),
        reference_enumeration(&request, &[])
    );
    assert_eq!(
        find_availability(&request, &[]).map(pairs),
        Ok(vec![(i64::MIN, -1)])
    );
}

#[test]
fn busy_and_participant_half_open_windows_keep_the_last_valid_slot() {
    let mut request = request(i64::MAX - 5, i64::MAX, 1, 2);
    request.participant_working_windows =
        vec![Some(vec![interval(i64::MAX - 3, i64::MAX).unwrap()])];
    let busy = [interval(i64::MAX - 5, i64::MAX - 4).unwrap()];

    assert_eq!(
        find_availability(&request, &busy).map(pairs),
        reference_enumeration(&request, &busy)
    );
    assert_eq!(
        find_availability(&request, &busy).map(pairs),
        Ok(vec![(i64::MAX - 3, i64::MAX - 2), (i64::MAX - 1, i64::MAX)])
    );
}

#[test]
fn an_unfittable_aligned_start_skips_only_that_free_region() {
    let request = request(1, i64::MAX, 1, i64::MAX);
    let busy = [interval(1, i64::MAX - 1).unwrap()];

    assert_eq!(
        find_availability(&request, &busy).map(pairs),
        reference_enumeration(&request, &busy)
    );
    assert_eq!(find_availability(&request, &busy).map(pairs), Ok(vec![]));
}

#[test]
fn an_in_window_end_overflow_remains_an_error() {
    let request = request(i64::MAX - 1, i64::MAX, 2, 1);

    assert_eq!(find_availability(&request, &[]), Err(Error::Overflow));
    assert_eq!(
        find_availability(&request, &[]).map(pairs),
        reference_enumeration(&request, &[])
    );
}

#[test]
fn scan_and_output_limits_still_fail_closed_near_the_endpoint() {
    let mut request = request(i64::MAX - 3, i64::MAX, 1, 1);
    request.max_scan = 2;
    assert_eq!(
        find_availability(&request, &[]),
        Err(Error::ScanLimitExceeded)
    );
    assert_eq!(
        find_availability(&request, &[]).map(pairs),
        reference_enumeration(&request, &[])
    );

    request.max_scan = 128;
    request.max_slots = 2;
    assert_eq!(
        find_availability(&request, &[]),
        Err(Error::OutputLimitExceeded)
    );
    assert_eq!(
        find_availability(&request, &[]).map(pairs),
        reference_enumeration(&request, &[])
    );
}

#[test]
fn nonpositive_periods_are_rejected_before_grid_arithmetic() {
    for value in [0, -1, i64::MIN] {
        let invalid_step = request(-10, 10, 1, value);
        assert_eq!(
            find_availability(&invalid_step, &[]),
            Err(Error::InvalidStep)
        );
        let invalid_duration = request(-10, 10, value, 1);
        assert_eq!(
            find_availability(&invalid_duration, &[]),
            Err(Error::InvalidRange)
        );
    }
}

#[test]
fn zero_limits_are_invalid_even_when_no_candidate_exists() {
    for no_free_time in [false, true] {
        let mut query = request(0, 10, 1, 1);
        let busy = if no_free_time {
            vec![query.window]
        } else {
            vec![]
        };
        query.max_scan = 0;
        assert_eq!(find_availability(&query, &busy), Err(Error::InvalidLimit));
        query.max_scan = 128;
        query.max_slots = 0;
        assert_eq!(find_availability(&query, &busy), Err(Error::InvalidLimit));
    }
}

#[test]
fn constant_divisibility_preserves_time_precision_and_leap_rules() {
    use nagi_calendar_core::time::{days_in_month, Instant};
    for (nanoseconds, expected) in [
        (0, Ok(Instant(0))),
        (1000, Ok(Instant(1))),
        (1001, Err(Error::PrecisionLoss)),
        (u64::MAX, Err(Error::PrecisionLoss)),
    ] {
        assert_eq!(Instant::from_realtime_ns(nanoseconds), expected);
    }
    for (year, february_days) in [(1900, 28), (2000, 29), (2024, 29), (2100, 28), (2400, 29)] {
        assert_eq!(days_in_month(year, 2), february_days);
    }
}
