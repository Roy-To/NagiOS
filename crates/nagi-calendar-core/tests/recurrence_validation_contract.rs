use nagi_calendar_core::model::OccurrenceKey;
use nagi_calendar_core::recurrence::{Frequency, Recurrence, RecurrenceBound};
use nagi_calendar_core::time::{Date, Instant, LocalDateTime};
use nagi_calendar_core::Error;
use std::collections::BTreeSet;

fn instant_rule() -> Recurrence {
    Recurrence {
        frequency: Frequency::InstantEvery { micros: 10 },
        interval: 1,
        bound: RecurrenceBound::Count(3),
    }
}

#[test]
fn empty_exceptions_cannot_bypass_period_or_count_validation() {
    let base = OccurrenceKey::Instant(Instant(0));
    let mut rules = Vec::new();
    for interval in [0, 65_537] {
        rules.push(Recurrence {
            interval,
            ..instant_rule()
        });
    }
    for count in [0, 65_537] {
        rules.push(Recurrence {
            bound: RecurrenceBound::Count(count),
            ..instant_rule()
        });
    }
    for micros in [0, -1, i64::MIN] {
        rules.push(Recurrence {
            frequency: Frequency::InstantEvery { micros },
            ..instant_rule()
        });
    }
    for rule in rules {
        assert_eq!(
            rule.validate_exceptions(base, BTreeSet::new()),
            Err(Error::InvalidRecurrence),
            "invalid rule: {rule:?}"
        );
    }
}

#[test]
fn empty_exceptions_cannot_bypass_frequency_weekday_or_until_domains() {
    let instant = OccurrenceKey::Instant(Instant(0));
    let date = Date::new(2026, 1, 1).unwrap();
    let local = OccurrenceKey::Local(LocalDateTime::midnight(date));
    for (base, rule) in [
        (
            instant,
            Recurrence {
                frequency: Frequency::Daily,
                ..instant_rule()
            },
        ),
        (
            local,
            Recurrence {
                frequency: Frequency::Weekly {
                    weekdays: BTreeSet::new(),
                },
                ..instant_rule()
            },
        ),
        (
            local,
            Recurrence {
                frequency: Frequency::Weekly {
                    weekdays: BTreeSet::from([7]),
                },
                ..instant_rule()
            },
        ),
        (
            instant,
            Recurrence {
                bound: RecurrenceBound::Until(local),
                ..instant_rule()
            },
        ),
        (
            instant,
            Recurrence {
                bound: RecurrenceBound::Until(OccurrenceKey::Instant(Instant(-1))),
                ..instant_rule()
            },
        ),
    ] {
        assert_eq!(
            rule.validate_exceptions(base, BTreeSet::new()),
            Err(Error::InvalidRecurrence),
            "invalid rule: {rule:?}"
        );
    }
}

#[test]
fn valid_empty_exceptions_keep_the_no_expansion_fast_path() {
    let rule = Recurrence {
        frequency: Frequency::InstantEvery { micros: 1 },
        bound: RecurrenceBound::Until(OccurrenceKey::Instant(Instant(i64::MAX))),
        ..instant_rule()
    };
    assert_eq!(
        rule.validate_exceptions(OccurrenceKey::Instant(Instant(0)), BTreeSet::new()),
        Ok(())
    );
}

#[test]
fn nonempty_exception_membership_still_rejects_out_of_series_keys() {
    let base = OccurrenceKey::Instant(Instant(0));
    let key = |n| OccurrenceKey::Instant(Instant(n));
    assert_eq!(
        instant_rule().validate_exceptions(base, BTreeSet::from([key(0), key(20)])),
        Ok(())
    );
    assert_eq!(
        instant_rule().validate_exceptions(base, BTreeSet::from([key(30)])),
        Err(Error::InvalidException)
    );
}
