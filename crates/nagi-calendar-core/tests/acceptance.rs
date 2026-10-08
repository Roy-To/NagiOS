use nagi_calendar_core::adapter::*;
use nagi_calendar_core::availability::*;
use nagi_calendar_core::model::*;
use nagi_calendar_core::recurrence::*;
use nagi_calendar_core::store::*;
use nagi_calendar_core::time::*;
use nagi_calendar_core::{Error, ObjectId, Result};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

fn date(y: u16, m: u8, d: u8) -> Date {
    Date::new(y, m, d).unwrap()
}
fn local(y: u16, m: u8, d: u8, h: u8, min: u8) -> LocalDateTime {
    LocalDateTime::new(date(y, m, d), h, min, 0, 0).unwrap()
}
fn utc(y: u16, m: u8, d: u8, h: u8, min: u8) -> Instant {
    Instant(local(y, m, d, h, min).naive_micros())
}
fn zone() -> ZoneId {
    ZoneId::new("fixture/new-york-2026").unwrap()
}
fn fixed() -> FixedOffsetResolver {
    FixedOffsetResolver::new(ZoneId::new("UTC").unwrap(), 0).unwrap()
}
fn draft(schedule: Schedule) -> EventDraft {
    EventDraft::new(
        CalendarId::new("cal_1").unwrap(),
        "会議 / meeting",
        schedule,
    )
}
fn instant_schedule() -> Schedule {
    Schedule::Instant(interval(0, SECOND * 3600).unwrap())
}
fn floating(start: LocalDateTime, end: LocalDateTime) -> Schedule {
    Schedule::Floating { start, end }
}
fn zoned(start: LocalDateTime, end: LocalDateTime, choice: ResolveChoice) -> Schedule {
    Schedule::Zoned {
        start,
        end,
        zone: zone(),
        start_choice: choice,
        end_choice: choice,
        duration_mode: DurationMode::Wall,
    }
}
fn limits() -> ExpansionLimits {
    ExpansionLimits {
        max_scan: 10_000,
        max_output: 10_000,
    }
}
fn year_window() -> Interval {
    Interval::new(utc(2026, 1, 1, 0, 0), utc(2027, 1, 1, 0, 0)).unwrap()
}
fn utc_context() -> ZoneContext {
    ZoneContext {
        zone: ZoneId::new("UTC").unwrap(),
        choice: ResolveChoice::Reject,
    }
}
fn recurrence(frequency: Frequency, bound: RecurrenceBound) -> Recurrence {
    Recurrence {
        frequency,
        interval: 1,
        bound,
    }
}
fn store() -> InMemoryStore<FixedClock, SequentialIds> {
    let mut s = InMemoryStore::new(FixedClock(1_000_000), SequentialIds::new(), 100).unwrap();
    s.create_calendar("予定", None).unwrap();
    s
}
/// Fixture only: transition table for two 2026 boundaries, not a timezone DB.
struct NewYork;
impl TimeZoneResolver for NewYork {
    fn resolve_local(&self, z: &ZoneId, t: LocalDateTime) -> Result<LocalResolution> {
        if z != &zone() {
            return Err(Error::UnknownZone);
        }
        if t < local(2026, 1, 1, 0, 0) || t >= local(2027, 1, 1, 0, 0) {
            return Err(Error::ResolverUnavailable);
        }
        if t >= local(2026, 3, 8, 2, 0) && t < local(2026, 3, 8, 3, 0) {
            return Ok(LocalResolution::Gap);
        }
        if t >= local(2026, 11, 1, 1, 0) && t < local(2026, 11, 1, 2, 0) {
            return Ok(LocalResolution::Fold {
                earlier: Instant(t.naive_micros() + 4 * 3600 * SECOND),
                later: Instant(t.naive_micros() + 5 * 3600 * SECOND),
            });
        }
        let offset = if t >= local(2026, 3, 8, 3, 0) && t < local(2026, 11, 1, 1, 0) {
            -4
        } else {
            -5
        };
        Ok(LocalResolution::Unique(Instant(
            t.naive_micros() - offset * 3600 * SECOND,
        )))
    }
    fn local_at(&self, z: &ZoneId, i: Instant) -> Result<ZonedPoint> {
        if z != &zone() {
            return Err(Error::UnknownZone);
        }
        let offset = if i >= utc(2026, 3, 8, 7, 0) && i < utc(2026, 11, 1, 6, 0) {
            -4 * 3600
        } else {
            -5 * 3600
        };
        Ok(ZonedPoint {
            local: LocalDateTime::from_naive_micros(i.0 + i64::from(offset) * SECOND)?,
            offset_seconds: offset,
        })
    }
}

#[test]
fn cal_h01_ids_stable_across_crud_and_calendars() {
    let mut s = store();
    let c = s.create_calendar("Work", None).unwrap();
    let a = s.create(draft(instant_schedule())).unwrap();
    let mut other = draft(instant_schedule());
    other.calendar_id = c.id.clone();
    let b = s.create(other).unwrap();
    assert_ne!(a.id, b.id);
    assert_ne!(a.draft.calendar_id, b.draft.calendar_id);
    let mut changed = a.draft.clone();
    changed.title = "更新".into();
    let updated = s.update(&a.id, a.revision, changed).unwrap();
    assert_eq!(updated.id, a.id);
    assert_eq!(updated.created_at, a.created_at);
    assert_eq!(updated.revision.get(), 2);
    assert_eq!(s.list(&c.id, 10).unwrap(), vec![b]);
    s.delete(&a.id, updated.revision).unwrap();
    assert_eq!(s.get(&a.id), Err(Error::NotFound));
}
#[test]
fn ids_validate_namespaces_and_empty_values() {
    for v in ["", "cal_", "../cal_x", "event_x", "cal_日本", "cal_X"] {
        assert_eq!(CalendarId::new(v), Err(Error::InvalidId));
    }
    assert_eq!(EventId::new("cal_x"), Err(Error::InvalidId));
    assert_eq!(EventRevision::new(0), Err(Error::RevisionConflict));
}
#[test]
fn cal_h08_stale_update_and_delete_are_atomic() {
    let mut s = store();
    let a = s.create(draft(instant_schedule())).unwrap();
    let updated = s.update(&a.id, a.revision, a.draft.clone()).unwrap();
    let mut bad = a.draft.clone();
    bad.title = "stale".into();
    assert_eq!(
        s.update(&a.id, a.revision, bad),
        Err(Error::RevisionConflict)
    );
    assert_eq!(s.delete(&a.id, a.revision), Err(Error::RevisionConflict));
    assert_eq!(s.get(&a.id).unwrap(), updated);
}
#[test]
fn revision_overflow_is_explicit() {
    assert_eq!(
        EventRevision::new(u64::MAX).unwrap().next(),
        Err(Error::Overflow)
    );
}
struct ReusedIds;
impl IdSource for ReusedIds {
    fn next_calendar_id(&mut self) -> Result<CalendarId> {
        CalendarId::new("cal_1")
    }
    fn next_event_id(&mut self) -> Result<EventId> {
        EventId::new("event_1")
    }
}
#[test]
fn duplicate_and_deleted_ids_cannot_be_reused() {
    let mut s = InMemoryStore::new(FixedClock(0), ReusedIds, 10).unwrap();
    s.create_calendar("a", None).unwrap();
    assert_eq!(s.create_calendar("b", None), Err(Error::DuplicateId));
    let a = s.create(draft(instant_schedule())).unwrap();
    assert_eq!(s.create(a.draft.clone()), Err(Error::DuplicateId));
    s.delete(&a.id, a.revision).unwrap();
    assert_eq!(s.create(a.draft), Err(Error::DuplicateId));
}
#[test]
fn store_capacity_includes_tombstones() {
    let mut s = InMemoryStore::new(FixedClock(0), SequentialIds::new(), 1).unwrap();
    s.create_calendar("a", None).unwrap();
    let a = s.create(draft(instant_schedule())).unwrap();
    s.delete(&a.id, a.revision).unwrap();
    assert_eq!(s.create(a.draft), Err(Error::Capacity));
}
#[test]
fn invalid_draft_does_not_mutate_store() {
    let mut s = store();
    let a = s.create(draft(instant_schedule())).unwrap();
    let mut bad = a.draft.clone();
    bad.title = " ".into();
    assert_eq!(
        s.update(&a.id, a.revision, bad.clone()),
        Err(Error::InvalidText)
    );
    assert_eq!(s.create(bad), Err(Error::InvalidText));
    assert_eq!(s.list(&a.draft.calendar_id, 10).unwrap(), vec![a]);
}
#[test]
fn cal_h09_fixed_clock_and_ids_reproduce_exact_output() {
    let mut a = store();
    let mut b = store();
    assert_eq!(
        a.create(draft(instant_schedule())),
        b.create(draft(instant_schedule()))
    );
}
#[test]
fn unavailable_clock_does_not_create_events() {
    let mut s = InMemoryStore::new(UnavailableClock, SequentialIds::new(), 10).unwrap();
    let c = s.create_calendar("a", None).unwrap();
    assert_eq!(
        s.create(draft(instant_schedule())),
        Err(Error::ClockUnavailable)
    );
    assert!(s.list(&c.id, 10).unwrap().is_empty());
}
struct RollbackClock(Cell<u64>);
impl RealtimeSource for RollbackClock {
    fn realtime_ns(&self) -> Result<u64> {
        let n = self.0.get();
        self.0.set(0);
        Ok(n)
    }
}
#[test]
fn clock_rollback_does_not_change_revision() {
    let mut s =
        InMemoryStore::new(RollbackClock(Cell::new(2000)), SequentialIds::new(), 10).unwrap();
    s.create_calendar("a", None).unwrap();
    let a = s.create(draft(instant_schedule())).unwrap();
    assert_eq!(
        s.update(&a.id, a.revision, a.draft.clone()),
        Err(Error::ClockRollback)
    );
    assert_eq!(s.get(&a.id).unwrap(), a);
}
#[test]
fn precision_and_time_overflows_rejected() {
    assert_eq!(Instant::from_realtime_ns(1), Err(Error::PrecisionLoss));
    assert_eq!(Instant(-SECOND).to_unix_seconds(), Err(Error::Overflow));
    assert_eq!(Instant(1).to_unix_seconds(), Err(Error::PrecisionLoss));
    assert_eq!(Instant(i64::MAX).checked_add(1), Err(Error::Overflow));
    assert_eq!(interval(i64::MIN, i64::MAX), Err(Error::Overflow));
}
#[test]
fn cal_h11_text_utf8_and_search_are_bounded() {
    let mut s = store();
    let mut d = draft(instant_schedule());
    d.description = "日本語とEnglish".into();
    d.participants.push("田中さん".into());
    let a = s.create(d.clone()).unwrap();
    assert_eq!(s.search(&d.calendar_id, "田中", 10).unwrap(), vec![a]);
    d.title = "日".repeat(1366);
    assert_eq!(s.create(d.clone()), Err(Error::InvalidText));
    d.title = "test\0".into();
    assert_eq!(s.create(d.clone()), Err(Error::InvalidText));
    d.title = "valid".into();
    d.participants = vec!["a".into(); 257];
    assert_eq!(s.create(d), Err(Error::Capacity));
}
#[test]
fn list_does_not_silently_truncate() {
    let mut s = store();
    let a = s.create(draft(instant_schedule())).unwrap();
    s.create(a.draft.clone()).unwrap();
    assert_eq!(
        s.list(&a.draft.calendar_id, 1),
        Err(Error::OutputLimitExceeded)
    );
    assert_eq!(s.list(&a.draft.calendar_id, 0), Err(Error::InvalidLimit));
}
#[test]
fn cal_h02_temporal_domains_require_explicit_context() {
    let f = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    assert_eq!(
        f.schedule.resolve(&fixed(), None),
        Err(Error::ZoneContextRequired)
    );
    let d = Schedule::AllDay {
        start: date(2026, 1, 1),
        end_exclusive: date(2026, 1, 2),
    };
    assert_eq!(d.resolve(&fixed(), None), Err(Error::ZoneContextRequired));
    assert_eq!(
        d.resolve(&fixed(), Some(&utc_context()))
            .unwrap()
            .duration(),
        DAY
    );
    assert_eq!(
        instant_schedule()
            .resolve(&NewYork, None)
            .unwrap()
            .duration(),
        3600 * SECOND
    );
}
#[test]
fn cal_h03_offsets_and_year_rollover() {
    for (name, offset, t, expected) in [
        (
            "plus09",
            9 * 3600,
            local(2027, 1, 1, 8, 30),
            utc(2026, 12, 31, 23, 30),
        ),
        (
            "minus05",
            -5 * 3600,
            local(2026, 12, 31, 20, 30),
            utc(2027, 1, 1, 1, 30),
        ),
        (
            "plus0545",
            5 * 3600 + 45 * 60,
            local(2026, 12, 31, 5, 45),
            utc(2026, 12, 31, 0, 0),
        ),
    ] {
        let z = ZoneId::new(name).unwrap();
        let r = FixedOffsetResolver::new(z.clone(), offset).unwrap();
        assert_eq!(resolve(&r, &z, t, ResolveChoice::Reject).unwrap(), expected);
    }
    assert_eq!(
        resolve(
            &fixed(),
            &zone(),
            local(2026, 1, 1, 1, 0),
            ResolveChoice::Reject
        ),
        Err(Error::UnknownZone)
    );
}
#[test]
fn cal_h04_gap_rejects_without_normalizing() {
    for choice in [
        ResolveChoice::Reject,
        ResolveChoice::Earlier,
        ResolveChoice::Later,
    ] {
        assert_eq!(
            resolve(&NewYork, &zone(), local(2026, 3, 8, 2, 30), choice),
            Err(Error::NonexistentLocalTime)
        );
    }
}
#[test]
fn cal_h04_fold_requires_choice() {
    let t = local(2026, 11, 1, 1, 30);
    assert_eq!(
        resolve(&NewYork, &zone(), t, ResolveChoice::Reject),
        Err(Error::AmbiguousLocalTime)
    );
    assert_eq!(
        resolve(&NewYork, &zone(), t, ResolveChoice::Earlier).unwrap(),
        utc(2026, 11, 1, 5, 30)
    );
    assert_eq!(
        resolve(&NewYork, &zone(), t, ResolveChoice::Later).unwrap(),
        utc(2026, 11, 1, 6, 30)
    );
}
#[test]
fn dst_transition_microsecond_boundary() {
    let t = LocalDateTime::new(date(2026, 3, 8), 1, 59, 59, 999_999).unwrap();
    assert_eq!(
        resolve(&NewYork, &zone(), t, ResolveChoice::Reject).unwrap(),
        utc(2026, 3, 8, 7, 0).checked_add(-1).unwrap()
    );
    assert_eq!(
        resolve(
            &NewYork,
            &zone(),
            local(2026, 3, 8, 3, 0),
            ResolveChoice::Reject
        )
        .unwrap(),
        utc(2026, 3, 8, 7, 0)
    );
}
#[test]
fn all_day_dst_has_23_or_25_hours() {
    let c = ZoneContext {
        zone: zone(),
        choice: ResolveChoice::Reject,
    };
    for (d, hours) in [(date(2026, 3, 8), 23), (date(2026, 11, 1), 25)] {
        let s = Schedule::AllDay {
            start: d,
            end_exclusive: d.add_days(1).unwrap(),
        };
        assert_eq!(
            s.resolve(&NewYork, Some(&c)).unwrap().duration(),
            hours * 3600 * SECOND
        );
    }
}
struct BrokenResolver;
impl TimeZoneResolver for BrokenResolver {
    fn resolve_local(&self, _: &ZoneId, _: LocalDateTime) -> Result<LocalResolution> {
        Ok(LocalResolution::Fold {
            earlier: Instant(2),
            later: Instant(1),
        })
    }
    fn local_at(&self, _: &ZoneId, _: Instant) -> Result<ZonedPoint> {
        panic!("unordered fold must be rejected before roundtrip")
    }
}
#[test]
fn resolver_fold_order_is_validated() {
    assert_eq!(
        resolve(
            &BrokenResolver,
            &zone(),
            local(2026, 1, 1, 0, 0),
            ResolveChoice::Earlier
        ),
        Err(Error::InvalidResolver)
    );
}
struct MismatchResolver;
impl TimeZoneResolver for MismatchResolver {
    fn resolve_local(&self, _: &ZoneId, _: LocalDateTime) -> Result<LocalResolution> {
        Ok(LocalResolution::Unique(Instant(0)))
    }
    fn local_at(&self, _: &ZoneId, _: Instant) -> Result<ZonedPoint> {
        Ok(ZonedPoint {
            local: local(2026, 1, 1, 0, 0),
            offset_seconds: 0,
        })
    }
}
#[test]
fn resolver_roundtrip_is_validated() {
    assert_eq!(
        resolve(
            &MismatchResolver,
            &zone(),
            local(2026, 1, 1, 0, 0),
            ResolveChoice::Reject
        ),
        Err(Error::InvalidResolver)
    );
}
#[test]
fn date_bounds_leap_rules_and_roundtrip() {
    assert!(Date::new(2000, 2, 29).is_ok());
    assert!(Date::new(2100, 2, 29).is_err());
    assert!(Date::new(2024, 2, 29).is_ok());
    assert!(Date::new(2025, 2, 29).is_err());
    assert_eq!(date(1970, 1, 1).epoch_days(), 0);
    for d in [
        date(1, 1, 1),
        date(9999, 12, 31),
        date(2024, 2, 29),
        date(2000, 12, 31),
    ] {
        assert_eq!(Date::from_epoch_days(d.epoch_days()).unwrap(), d);
    }
    assert!(date(9999, 12, 31).add_days(1).is_err());
    assert!(date(1, 1, 1).add_days(-1).is_err());
    assert!(Date::new(0, 1, 1).is_err());
    assert!(LocalDateTime::new(date(2026, 1, 1), 0, 0, 60, 0).is_err());
}
#[test]
fn invalid_ranges_rejected_in_all_domains() {
    assert_eq!(interval(0, 0), Err(Error::InvalidRange));
    assert_eq!(interval(10, 0), Err(Error::InvalidRange));
    assert_eq!(
        floating(local(2026, 1, 2, 0, 0), local(2026, 1, 1, 0, 0)).validate(),
        Err(Error::InvalidRange)
    );
    assert_eq!(
        Schedule::AllDay {
            start: date(2026, 1, 1),
            end_exclusive: date(2026, 1, 1)
        }
        .validate(),
        Err(Error::InvalidRange)
    );
}
#[test]
fn cal_h05_local_daily_maintains_nine_am_across_dst() {
    let mut d = draft(zoned(
        local(2026, 3, 7, 9, 0),
        local(2026, 3, 7, 10, 0),
        ResolveChoice::Reject,
    ));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(3)));
    let o = expand(&d, year_window(), &NewYork, None, limits()).unwrap();
    assert_eq!(
        o.iter().map(|o| o.interval.start()).collect::<Vec<_>>(),
        vec![
            utc(2026, 3, 7, 14, 0),
            utc(2026, 3, 8, 13, 0),
            utc(2026, 3, 9, 13, 0)
        ]
    );
}
#[test]
fn instant_daily_is_elapsed_not_wall_time() {
    let mut d = draft(Schedule::Instant(
        Interval::new(utc(2026, 3, 7, 14, 0), utc(2026, 3, 7, 15, 0)).unwrap(),
    ));
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: DAY },
        RecurrenceBound::Count(3),
    ));
    let o = expand(&d, year_window(), &NewYork, None, limits()).unwrap();
    assert_eq!(o[1].interval.start(), utc(2026, 3, 8, 14, 0));
}
#[test]
fn elapsed_and_wall_event_duration_are_distinct() {
    let mut d = draft(zoned(
        local(2026, 3, 7, 1, 30),
        local(2026, 3, 7, 3, 30),
        ResolveChoice::Reject,
    ));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(2)));
    assert_eq!(
        expand(&d, year_window(), &NewYork, None, limits()).unwrap()[1]
            .interval
            .duration(),
        3600 * SECOND
    );
    if let Schedule::Zoned { duration_mode, .. } = &mut d.schedule {
        *duration_mode = DurationMode::Elapsed;
    }
    assert_eq!(
        expand(&d, year_window(), &NewYork, None, limits()).unwrap()[1]
            .interval
            .duration(),
        2 * 3600 * SECOND
    );
}
#[test]
fn cal_h06_month_end_skips_and_does_not_drift() {
    let mut d = draft(floating(
        local(2026, 1, 31, 9, 0),
        local(2026, 1, 31, 10, 0),
    ));
    d.recurrence = Some(recurrence(Frequency::Monthly, RecurrenceBound::Count(3)));
    let o = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(
        o.iter().map(|o| o.original_key).collect::<Vec<_>>(),
        vec![
            OccurrenceKey::Local(local(2026, 1, 31, 9, 0)),
            OccurrenceKey::Local(local(2026, 3, 31, 9, 0)),
            OccurrenceKey::Local(local(2026, 5, 31, 9, 0))
        ]
    );
}
#[test]
fn leap_day_monthly_interval_skips_nonleap_years() {
    let mut d = draft(Schedule::AllDay {
        start: date(2024, 2, 29),
        end_exclusive: date(2024, 3, 1),
    });
    d.recurrence = Some(Recurrence {
        frequency: Frequency::Monthly,
        interval: 12,
        bound: RecurrenceBound::Count(2),
    });
    let window = Interval::new(utc(2024, 1, 1, 0, 0), utc(2029, 1, 1, 0, 0)).unwrap();
    let o = expand(&d, window, &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(o[1].original_key, OccurrenceKey::Date(date(2028, 2, 29)));
}
#[test]
fn weekly_interval_is_anchored_to_monday_week() {
    let mut d = draft(floating(local(2026, 1, 5, 9, 0), local(2026, 1, 5, 10, 0)));
    d.recurrence = Some(Recurrence {
        frequency: Frequency::Weekly {
            weekdays: BTreeSet::from([0, 2]),
        },
        interval: 2,
        bound: RecurrenceBound::Count(4),
    });
    let o = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(
        o.iter().map(|o| o.interval.start()).collect::<Vec<_>>(),
        vec![
            utc(2026, 1, 5, 9, 0),
            utc(2026, 1, 7, 9, 0),
            utc(2026, 1, 19, 9, 0),
            utc(2026, 1, 21, 9, 0)
        ]
    );
}
#[test]
fn until_is_inclusive_across_year_end() {
    let mut d = draft(floating(
        local(2026, 12, 30, 9, 0),
        local(2026, 12, 30, 10, 0),
    ));
    d.recurrence = Some(recurrence(
        Frequency::Daily,
        RecurrenceBound::Until(OccurrenceKey::Local(local(2027, 1, 1, 9, 0))),
    ));
    let w = Interval::new(utc(2026, 12, 1, 0, 0), utc(2027, 2, 1, 0, 0)).unwrap();
    assert_eq!(
        expand(&d, w, &fixed(), Some(&utc_context()), limits())
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn exdate_consumes_count_without_replacement() {
    let mut d = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(3)));
    d.exdates
        .insert(OccurrenceKey::Local(local(2026, 1, 2, 9, 0)));
    let o = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(o.len(), 2);
    assert_eq!(o[1].interval.start(), utc(2026, 1, 3, 9, 0));
}
#[test]
fn moved_override_keeps_original_identity_and_enters_query() {
    let mut d = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(3)));
    let original = OccurrenceKey::Local(local(2026, 1, 2, 9, 0));
    d.overrides.insert(
        original,
        floating(local(2026, 2, 2, 9, 0), local(2026, 2, 2, 10, 0)),
    );
    let w = Interval::new(utc(2026, 2, 2, 0, 0), utc(2026, 2, 3, 0, 0)).unwrap();
    let o = expand(&d, w, &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].original_key, original);
    let january = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(january.len(), 3);
}
#[test]
fn exceptions_outside_series_and_conflicts_rejected() {
    let mut d = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(3)));
    d.exdates
        .insert(OccurrenceKey::Local(local(2026, 1, 10, 9, 0)));
    assert_eq!(d.validate(), Err(Error::InvalidException));
    d.exdates.clear();
    let k = d.schedule.key();
    d.exdates.insert(k);
    d.overrides = BTreeMap::from([(k, d.schedule.clone())]);
    assert_eq!(d.validate(), Err(Error::InvalidException));
}
#[test]
fn overnight_event_overlaps_window_and_end_is_exclusive() {
    let d = draft(Schedule::Instant(
        Interval::new(utc(2026, 12, 31, 23, 30), utc(2027, 1, 1, 1, 0)).unwrap(),
    ));
    assert_eq!(
        expand(
            &d,
            Interval::new(utc(2027, 1, 1, 0, 0), utc(2027, 1, 1, 2, 0)).unwrap(),
            &fixed(),
            None,
            limits()
        )
        .unwrap()
        .len(),
        1
    );
    assert!(expand(
        &d,
        Interval::new(utc(2027, 1, 1, 1, 0), utc(2027, 1, 1, 2, 0)).unwrap(),
        &fixed(),
        None,
        limits()
    )
    .unwrap()
    .is_empty());
}
#[test]
fn invalid_recurrence_modes_and_bounds_rejected() {
    let base = OccurrenceKey::Local(local(2026, 1, 1, 9, 0));
    for r in [
        Recurrence {
            frequency: Frequency::Daily,
            interval: 0,
            bound: RecurrenceBound::Count(1),
        },
        recurrence(Frequency::Daily, RecurrenceBound::Count(0)),
        recurrence(
            Frequency::Weekly {
                weekdays: BTreeSet::from([7]),
            },
            RecurrenceBound::Count(2),
        ),
        recurrence(
            Frequency::InstantEvery { micros: DAY },
            RecurrenceBound::Count(2),
        ),
        recurrence(
            Frequency::Daily,
            RecurrenceBound::Until(OccurrenceKey::Instant(Instant(0))),
        ),
    ] {
        assert_eq!(r.validate(base), Err(Error::InvalidRecurrence));
    }
}
#[test]
fn cal_h08_scan_limit_applies_when_no_occurrences_are_returned() {
    let mut d = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(20)));
    let w = Interval::new(utc(2026, 2, 1, 0, 0), utc(2026, 2, 2, 0, 0)).unwrap();
    let small = ExpansionLimits {
        max_scan: 4,
        max_output: 10,
    };
    assert_eq!(
        expand(&d, w, &fixed(), Some(&utc_context()), small),
        Err(Error::ScanLimitExceeded)
    );
    for day in 1..=5 {
        d.exdates
            .insert(OccurrenceKey::Local(local(2026, 1, day, 9, 0)));
    }
    assert_eq!(
        expand(&d, year_window(), &fixed(), Some(&utc_context()), small),
        Err(Error::ScanLimitExceeded)
    );
}
#[test]
fn skipped_month_and_weekday_scans_are_bounded() {
    let mut d = draft(floating(
        local(2026, 1, 31, 9, 0),
        local(2026, 1, 31, 10, 0),
    ));
    d.recurrence = Some(recurrence(Frequency::Monthly, RecurrenceBound::Count(3)));
    assert_eq!(
        expand(
            &d,
            year_window(),
            &fixed(),
            Some(&utc_context()),
            ExpansionLimits {
                max_scan: 2,
                max_output: 10
            }
        ),
        Err(Error::ScanLimitExceeded)
    );
    d.schedule = floating(local(2026, 1, 5, 9, 0), local(2026, 1, 5, 10, 0));
    d.recurrence = Some(recurrence(
        Frequency::Weekly {
            weekdays: BTreeSet::from([6]),
        },
        RecurrenceBound::Count(2),
    ));
    assert_eq!(
        expand(
            &d,
            year_window(),
            &fixed(),
            Some(&utc_context()),
            ExpansionLimits {
                max_scan: 2,
                max_output: 10
            }
        ),
        Err(Error::ScanLimitExceeded)
    );
}
#[test]
fn recurrence_output_limit_and_overflow_are_errors() {
    let mut d = draft(instant_schedule());
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: DAY },
        RecurrenceBound::Count(3),
    ));
    assert_eq!(
        expand(
            &d,
            interval(0, 4 * DAY).unwrap(),
            &fixed(),
            None,
            ExpansionLimits {
                max_scan: 10,
                max_output: 1
            }
        ),
        Err(Error::OutputLimitExceeded)
    );
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: i64::MAX },
        RecurrenceBound::Count(3),
    ));
    assert_eq!(
        expand(&d, interval(0, DAY).unwrap(), &fixed(), None, limits()),
        Err(Error::Overflow)
    );
}
#[test]
fn recurrence_gap_is_error_not_silent_skip() {
    let mut d = draft(zoned(
        local(2026, 3, 7, 2, 30),
        local(2026, 3, 7, 3, 30),
        ResolveChoice::Reject,
    ));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(2)));
    assert_eq!(
        expand(&d, year_window(), &NewYork, None, limits()),
        Err(Error::NonexistentLocalTime)
    );
}
fn request() -> AvailabilityRequest {
    AvailabilityRequest {
        window: interval(0, 240).unwrap(),
        duration_micros: 30,
        step_micros: 30,
        buffer_before_micros: 0,
        buffer_after_micros: 0,
        max_scan: 100,
        max_slots: 100,
        participant_working_windows: vec![],
    }
}
fn pairs(slots: Vec<Interval>) -> Vec<(i64, i64)> {
    slots.iter().map(|i| (i.start().0, i.end().0)).collect()
}
#[test]
fn cal_h07_busy_union_and_free_slots() {
    let busy = vec![
        interval(90, 180).unwrap(),
        interval(60, 120).unwrap(),
        interval(180, 210).unwrap(),
    ];
    assert_eq!(
        pairs(normalize_busy(request().window, &busy, 0, 0).unwrap()),
        vec![(60, 210)]
    );
    assert_eq!(
        pairs(find_availability(&request(), &busy).unwrap()),
        vec![(0, 30), (30, 60), (210, 240)]
    );
}
#[test]
fn availability_clips_busy_and_allows_endpoint_contact() {
    let mut r = request();
    r.window = interval(0, 120).unwrap();
    r.duration_micros = 60;
    assert_eq!(
        pairs(
            find_availability(&r, &[interval(-30, 0).unwrap(), interval(60, 180).unwrap()])
                .unwrap()
        ),
        vec![(0, 60)]
    );
}
#[test]
fn availability_buffers_and_global_grid() {
    let mut r = request();
    r.buffer_before_micros = 15;
    r.buffer_after_micros = 15;
    assert_eq!(
        pairs(find_availability(&r, &[interval(60, 90).unwrap()]).unwrap()),
        vec![(0, 30), (120, 150), (150, 180), (180, 210), (210, 240)]
    );
}
#[test]
fn participant_windows_intersect_and_unknown_does_not_mean_free() {
    let mut r = request();
    r.participant_working_windows = vec![
        Some(vec![interval(0, 180).unwrap()]),
        Some(vec![interval(60, 120).unwrap()]),
    ];
    assert_eq!(
        pairs(find_availability(&r, &[]).unwrap()),
        vec![(60, 90), (90, 120)]
    );
    r.participant_working_windows.push(None);
    assert_eq!(find_availability(&r, &[]), Err(Error::UnknownAvailability));
    r.participant_working_windows = vec![Some(vec![])];
    assert!(find_availability(&r, &[]).unwrap().is_empty());
}
#[test]
fn availability_invalid_bounds_and_limits_rejected() {
    let mut r = request();
    r.step_micros = 0;
    assert_eq!(find_availability(&r, &[]), Err(Error::InvalidStep));
    r = request();
    r.duration_micros = 0;
    assert_eq!(find_availability(&r, &[]), Err(Error::InvalidRange));
    r = request();
    r.buffer_before_micros = -1;
    assert_eq!(find_availability(&r, &[]), Err(Error::InvalidRange));
    r = request();
    r.max_scan = 1;
    assert_eq!(find_availability(&r, &[]), Err(Error::ScanLimitExceeded));
    r = request();
    r.max_slots = 1;
    assert_eq!(find_availability(&r, &[]), Err(Error::OutputLimitExceeded));
}
#[test]
fn availability_overflow_and_fully_busy() {
    let mut r = request();
    r.window = interval(i64::MAX - 10, i64::MAX).unwrap();
    r.duration_micros = 20;
    assert_eq!(find_availability(&r, &[]), Err(Error::Overflow));
    assert!(find_availability(&request(), &[request().window])
        .unwrap()
        .is_empty());
}
#[test]
fn different_zone_events_normalize_on_instant_axis() {
    let first = draft(zoned(
        local(2026, 3, 8, 9, 0),
        local(2026, 3, 8, 10, 0),
        ResolveChoice::Reject,
    ));
    let second = draft(Schedule::Instant(
        Interval::new(utc(2026, 3, 8, 13, 30), utc(2026, 3, 8, 14, 30)).unwrap(),
    ));
    let busy = busy_from_events(&[first, second], year_window(), &NewYork, None, limits()).unwrap();
    assert_eq!(
        busy,
        vec![Interval::new(utc(2026, 3, 8, 13, 0), utc(2026, 3, 8, 14, 30)).unwrap()]
    );
}
#[test]
fn invalid_transparent_event_is_not_silently_ignored() {
    let mut d = draft(instant_schedule());
    d.busy = false;
    d.title.clear();
    assert_eq!(
        busy_from_events(&[d], year_window(), &fixed(), None, limits()),
        Err(Error::InvalidText)
    );
}
#[test]
fn cal_h10_invite_and_response_never_report_delivery() {
    let mut s = store();
    let e = s.create(draft(instant_schedule())).unwrap();
    for action in [Action::Invite, Action::RespondInvite] {
        assert_eq!(action.effect(), Effect::External);
        assert!(action.requires_confirmation());
        let d = ExternalDraft::new(&e, action).unwrap();
        assert_eq!(d.state, DraftState::RequiresConfirmation);
        assert_eq!(d.execute(false), Err(Error::ExternalActionUnavailable));
        assert_eq!(d.execute(true), Err(Error::ExternalActionUnavailable));
    }
    assert_eq!(s.get(&e.id).unwrap(), e);
}
#[test]
fn adapter_reminder_and_resource_are_local_cas_mutations() {
    let mut a = LocalAdapter { store: store() };
    let e = a.create_event(draft(instant_schedule())).unwrap();
    let r = a
        .add_reminder(&e.id, e.revision, Reminder::RelativeMicros(-900 * SECOND))
        .unwrap();
    assert_eq!(r.draft.reminders.len(), 1);
    assert_eq!(
        a.attach_resource(&e.id, e.revision, ObjectId(10)),
        Err(Error::RevisionConflict)
    );
    let linked = a.attach_resource(&e.id, r.revision, ObjectId(10)).unwrap();
    assert_eq!(linked.draft.resources, vec![ObjectId(10)]);
    assert_eq!(Action::AddReminder.effect(), Effect::LocalMutation);
    assert_eq!(Action::FindAvailability.effect(), Effect::ReadOnly);
}

#[test]
fn calendar_update_retains_id_and_delete_rejects_nonempty_calendar() {
    let mut s = store();
    let c = CalendarId::new("cal_1").unwrap();
    let renamed = s.update_calendar(&c, "仕事", None).unwrap();
    assert_eq!(renamed.id, c);
    assert_eq!(renamed.name, "仕事");
    let event = s.create(draft(instant_schedule())).unwrap();
    assert_eq!(s.delete_calendar(&c), Err(Error::CalendarNotEmpty));
    s.delete(&event.id, event.revision).unwrap();
    s.delete_calendar(&c).unwrap();
    assert_eq!(s.calendar(&c), Err(Error::NotFound));
}
#[test]
fn deleted_calendar_ids_are_not_reused() {
    let mut s = InMemoryStore::new(FixedClock(0), ReusedIds, 10).unwrap();
    let c = s.create_calendar("a", None).unwrap();
    s.delete_calendar(&c.id).unwrap();
    assert_eq!(s.create_calendar("b", None), Err(Error::DuplicateId));
}
#[test]
fn adapter_computes_availability_without_services() {
    let mut a = LocalAdapter { store: store() };
    a.create_event(draft(Schedule::Instant(interval(60, 120).unwrap())))
        .unwrap();
    let slots = a
        .find_availability(
            &CalendarId::new("cal_1").unwrap(),
            &request(),
            &fixed(),
            None,
            limits(),
        )
        .unwrap();
    assert_eq!(
        pairs(slots),
        vec![
            (0, 30),
            (30, 60),
            (120, 150),
            (150, 180),
            (180, 210),
            (210, 240)
        ]
    );
}
#[test]
fn date_roundtrips_every_month_boundary_in_supported_years() {
    for year in 1..=9999 {
        for month in 1..=12 {
            for day in [1, days_in_month(year, month)] {
                let d = date(year, month, day);
                assert_eq!(Date::from_epoch_days(d.epoch_days()).unwrap(), d);
            }
        }
    }
}
#[test]
fn weekly_midweek_start_and_daily_interval() {
    let mut d = draft(floating(local(2026, 1, 7, 9, 0), local(2026, 1, 7, 10, 0)));
    d.recurrence = Some(Recurrence {
        frequency: Frequency::Weekly {
            weekdays: BTreeSet::from([0, 2]),
        },
        interval: 2,
        bound: RecurrenceBound::Count(3),
    });
    let out = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(
        out.iter().map(|o| o.interval.start()).collect::<Vec<_>>(),
        vec![
            utc(2026, 1, 7, 9, 0),
            utc(2026, 1, 19, 9, 0),
            utc(2026, 1, 21, 9, 0)
        ]
    );
    d.recurrence = Some(Recurrence {
        frequency: Frequency::Daily,
        interval: 2,
        bound: RecurrenceBound::Count(2),
    });
    let out = expand(&d, year_window(), &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(out[1].interval.start(), utc(2026, 1, 9, 9, 0));
}
#[test]
fn moved_override_leaves_original_query_window() {
    let mut d = draft(floating(local(2026, 1, 1, 9, 0), local(2026, 1, 1, 10, 0)));
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(3)));
    d.overrides.insert(
        OccurrenceKey::Local(local(2026, 1, 2, 9, 0)),
        floating(local(2026, 2, 2, 9, 0), local(2026, 2, 2, 10, 0)),
    );
    let w = Interval::new(utc(2026, 1, 1, 0, 0), utc(2026, 1, 4, 0, 0)).unwrap();
    let out = expand(&d, w, &fixed(), Some(&utc_context()), limits()).unwrap();
    assert_eq!(out.len(), 2);
}
#[test]
fn fold_endpoints_can_resolve_to_nonpositive_instant_range() {
    let s = Schedule::Zoned {
        start: local(2026, 11, 1, 1, 15),
        end: local(2026, 11, 1, 1, 45),
        zone: zone(),
        start_choice: ResolveChoice::Later,
        end_choice: ResolveChoice::Earlier,
        duration_mode: DurationMode::Wall,
    };
    assert_eq!(s.resolve(&NewYork, None), Err(Error::InvalidRange));
}
#[test]
fn calendar_foreign_reference_and_exception_domain_fail_closed() {
    let mut s = store();
    let mut d = draft(instant_schedule());
    d.calendar_id = CalendarId::new("cal_missing").unwrap();
    assert_eq!(s.create(d), Err(Error::NotFound));
    let mut d = draft(instant_schedule());
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: DAY },
        RecurrenceBound::Count(2),
    ));
    d.exdates.insert(OccurrenceKey::Date(date(2026, 1, 1)));
    assert_eq!(d.validate(), Err(Error::InvalidException));
}

#[test]
fn until_at_maximum_date_finishes_without_advancing_out_of_domain() {
    let mut d = draft(floating(
        local(9999, 12, 31, 9, 0),
        local(9999, 12, 31, 10, 0),
    ));
    d.recurrence = Some(recurrence(
        Frequency::Daily,
        RecurrenceBound::Until(OccurrenceKey::Local(local(9999, 12, 31, 23, 59))),
    ));
    let w = Interval::new(utc(9999, 12, 31, 0, 0), utc(9999, 12, 31, 23, 59)).unwrap();
    assert_eq!(
        expand(&d, w, &fixed(), Some(&utc_context()), limits())
            .unwrap()
            .len(),
        1
    );
    d.recurrence = Some(recurrence(Frequency::Daily, RecurrenceBound::Count(2)));
    assert_eq!(
        expand(&d, w, &fixed(), Some(&utc_context()), limits()),
        Err(Error::InvalidDate)
    );
}
#[test]
fn until_near_maximum_instant_finishes_without_overflow() {
    let start = i64::MAX - 100;
    let mut d = draft(Schedule::Instant(interval(start, start + 10).unwrap()));
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: 200 },
        RecurrenceBound::Until(OccurrenceKey::Instant(Instant(i64::MAX))),
    ));
    assert_eq!(
        expand(
            &d,
            interval(start, i64::MAX).unwrap(),
            &fixed(),
            None,
            limits()
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn recurrence_rejects_exactly_one_occurrence_over_output_limit() {
    let mut d = draft(instant_schedule());
    d.recurrence = Some(recurrence(
        Frequency::InstantEvery { micros: DAY },
        RecurrenceBound::Count(2),
    ));
    assert_eq!(
        expand(
            &d,
            interval(0, 3 * DAY).unwrap(),
            &fixed(),
            None,
            ExpansionLimits {
                max_scan: 10,
                max_output: 1
            }
        ),
        Err(Error::OutputLimitExceeded)
    );
}
