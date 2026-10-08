use crate::model::{EventDraft, Interval, OccurrenceKey, Schedule, ZoneContext};
use crate::store::check_limit;
use crate::time::{Date, LocalDateTime, TimeZoneResolver, DAY};
use crate::{Error, Result};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Frequency {
    Daily,
    Weekly { weekdays: BTreeSet<u8> },
    Monthly,
    InstantEvery { micros: i64 },
}
/// Exactly one bound, by construction. UNTIL is inclusive and uses the schedule domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecurrenceBound {
    Count(u32),
    Until(OccurrenceKey),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recurrence {
    pub frequency: Frequency,
    pub interval: u32,
    pub bound: RecurrenceBound,
}
#[derive(Clone, Copy, Debug)]
pub struct ExpansionLimits {
    pub max_scan: usize,
    pub max_output: usize,
}
impl ExpansionLimits {
    pub fn validate(self) -> Result<()> {
        check_limit(self.max_scan)?;
        check_limit(self.max_output)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Occurrence {
    pub original_key: OccurrenceKey,
    pub schedule: Schedule,
    pub interval: Interval,
}

fn local_of(key: OccurrenceKey) -> Result<LocalDateTime> {
    match key {
        OccurrenceKey::Local(t) => Ok(t),
        OccurrenceKey::Date(d) => Ok(LocalDateTime::midnight(d)),
        _ => Err(Error::InvalidRecurrence),
    }
}
fn key_like(base: OccurrenceKey, t: LocalDateTime) -> OccurrenceKey {
    match base {
        OccurrenceKey::Date(_) => OccurrenceKey::Date(t.date()),
        _ => OccurrenceKey::Local(t),
    }
}
impl Recurrence {
    pub fn validate(&self, base: OccurrenceKey) -> Result<()> {
        if self.interval == 0 || self.interval > 65_536 {
            return Err(Error::InvalidRecurrence);
        }
        match (&self.frequency, base) {
            (Frequency::InstantEvery { micros }, OccurrenceKey::Instant(_)) if *micros > 0 => {}
            (
                Frequency::Daily | Frequency::Monthly,
                OccurrenceKey::Local(_) | OccurrenceKey::Date(_),
            ) => {}
            (Frequency::Weekly { weekdays }, OccurrenceKey::Local(_) | OccurrenceKey::Date(_))
                if !weekdays.is_empty() && weekdays.iter().all(|d| *d < 7) => {}
            _ => return Err(Error::InvalidRecurrence),
        }
        match self.bound {
            RecurrenceBound::Count(n) if n == 0 || n > 65_536 => Err(Error::InvalidRecurrence),
            RecurrenceBound::Until(k)
                if std::mem::discriminant(&k) != std::mem::discriminant(&base) || k < base =>
            {
                Err(Error::InvalidRecurrence)
            }
            _ => Ok(()),
        }
    }
    /// Probe advances even when an invalid month/weekday emits no occurrence.
    fn candidate(
        &self,
        base: OccurrenceKey,
        index: u64,
    ) -> Result<(OccurrenceKey, Option<OccurrenceKey>)> {
        let step = index
            .checked_mul(u64::from(self.interval))
            .ok_or(Error::Overflow)?;
        match self.frequency {
            Frequency::InstantEvery { micros } => {
                let OccurrenceKey::Instant(start) = base else {
                    return Err(Error::InvalidRecurrence);
                };
                let delta = i64::try_from(step)
                    .map_err(|_| Error::Overflow)?
                    .checked_mul(micros)
                    .ok_or(Error::Overflow)?;
                let key = OccurrenceKey::Instant(start.checked_add(delta)?);
                Ok((key, Some(key)))
            }
            Frequency::Daily => {
                let t = local_of(base)?;
                let delta = i64::try_from(step)
                    .map_err(|_| Error::Overflow)?
                    .checked_mul(DAY)
                    .ok_or(Error::Overflow)?;
                let key = key_like(base, t.checked_add(delta)?);
                Ok((key, Some(key)))
            }
            Frequency::Weekly { ref weekdays } => {
                let t = local_of(base)?;
                let date = t
                    .date()
                    .add_days(i64::try_from(index).map_err(|_| Error::Overflow)?)?;
                let key = key_like(base, t.with_date(date));
                let weeks =
                    (date.epoch_days() - t.date().epoch_days() + i64::from(t.date().weekday())) / 7;
                let accepted =
                    weeks % i64::from(self.interval) == 0 && weekdays.contains(&date.weekday());
                Ok((key, accepted.then_some(key)))
            }
            Frequency::Monthly => {
                let t = local_of(base)?;
                let date = t.date();
                let absolute = u64::from(date.year() - 1) * 12 + u64::from(date.month() - 1) + step;
                if absolute >= 9999 * 12 {
                    return Err(Error::InvalidDate);
                }
                let year = (absolute / 12 + 1) as u16;
                let month = (absolute % 12 + 1) as u8;
                let probe = key_like(base, t.with_date(Date::new(year, month, 1)?));
                let candidate = Date::new(year, month, date.day())
                    .ok()
                    .map(|d| key_like(base, t.with_date(d)));
                Ok((candidate.unwrap_or(probe), candidate))
            }
        }
    }
    fn keys(
        &self,
        base: OccurrenceKey,
        max_scan: usize,
        mut accept: impl FnMut(OccurrenceKey) -> Result<()>,
    ) -> Result<()> {
        self.validate(base)?;
        check_limit(max_scan)?;
        let mut generated = 0u32;
        for index in 0..max_scan {
            if let RecurrenceBound::Count(count) = self.bound {
                if generated == count {
                    return Ok(());
                }
            }
            let (probe, candidate) = match self.candidate(base, index as u64) {
                // Positive progress beyond the representable domain is necessarily
                // beyond an UNTIL in that same domain. COUNT still reports overflow.
                Err(Error::InvalidDate | Error::Overflow)
                    if matches!(self.bound, RecurrenceBound::Until(_)) =>
                {
                    return Ok(())
                }
                result => result?,
            };
            if let RecurrenceBound::Until(until) = self.bound {
                if probe > until {
                    return Ok(());
                }
            }
            if let Some(key) = candidate {
                generated = generated.checked_add(1).ok_or(Error::Overflow)?;
                accept(key)?;
                match self.bound {
                    RecurrenceBound::Count(n) if generated == n => return Ok(()),
                    RecurrenceBound::Until(until) if key == until => return Ok(()),
                    _ => {}
                }
            }
        }
        Err(Error::ScanLimitExceeded)
    }
    /// Validate all exception keys in one bounded scan, without timezone resolution.
    pub fn validate_exceptions(
        &self,
        base: OccurrenceKey,
        keys: BTreeSet<OccurrenceKey>,
    ) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut remaining = keys;
        self.keys(base, 65_536, |k| {
            remaining.remove(&k);
            Ok(())
        })?;
        if remaining.is_empty() {
            Ok(())
        } else {
            Err(Error::InvalidException)
        }
    }
}

/// Expand only the bounded subset; errors never masquerade as complete results.
/// COUNT includes valid base occurrences before exclusions. Query uses overlap.
pub fn expand(
    draft: &EventDraft,
    window: Interval,
    resolver: &dyn TimeZoneResolver,
    context: Option<&ZoneContext>,
    limits: ExpansionLimits,
) -> Result<Vec<Occurrence>> {
    limits.validate()?;
    draft.validate()?;
    let mut result = vec![];
    let mut emit = |key: OccurrenceKey, schedule: Schedule| -> Result<()> {
        let interval = schedule.resolve(resolver, context)?;
        if interval.overlaps(window) {
            if result.len() == limits.max_output {
                return Err(Error::OutputLimitExceeded);
            }
            result.push(Occurrence {
                original_key: key,
                schedule,
                interval,
            });
        }
        Ok(())
    };
    if let Some(r) = &draft.recurrence {
        let mut matched = BTreeSet::new();
        r.keys(draft.schedule.key(), limits.max_scan, |key| {
            matched.insert(key);
            if draft.exdates.contains(&key) {
                return Ok(());
            }
            let schedule = if let Some(replacement) = draft.overrides.get(&key) {
                replacement.clone()
            } else {
                draft.schedule.shifted(key, resolver)?
            };
            emit(key, schedule)
        })?;
        if draft
            .exdates
            .iter()
            .chain(draft.overrides.keys())
            .any(|k| !matched.contains(k))
        {
            return Err(Error::InvalidException);
        }
    } else {
        emit(draft.schedule.key(), draft.schedule.clone())?;
    }
    result.sort_by_key(|o| (o.interval.start(), o.original_key));
    Ok(result)
}
