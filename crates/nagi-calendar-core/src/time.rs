use crate::{Error, Result};

pub const SECOND: i64 = 1_000_000;
pub const DAY: i64 = 86_400 * SECOND;

/// Signed Unix microseconds, excluding leap-second representations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Instant(pub i64);
impl Instant {
    pub fn checked_add(self, micros: i64) -> Result<Self> {
        self.0.checked_add(micros).map(Self).ok_or(Error::Overflow)
    }
    /// PAL realtime nanoseconds: reject precision loss rather than truncate.
    pub fn from_realtime_ns(ns: u64) -> Result<Self> {
        if !ns.is_multiple_of(1000) {
            return Err(Error::PrecisionLoss);
        }
        i64::try_from(ns / 1000)
            .map(Self)
            .map_err(|_| Error::Overflow)
    }
    /// Grant-lifetime Unix seconds, with explicit precision and sign checks.
    pub fn to_unix_seconds(self) -> Result<u64> {
        if self.0 % SECOND != 0 {
            return Err(Error::PrecisionLoss);
        }
        u64::try_from(self.0 / SECOND).map_err(|_| Error::Overflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}
impl Date {
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
        {
            return Err(Error::InvalidDate);
        }
        Ok(Self { year, month, day })
    }
    pub fn year(self) -> u16 {
        self.year
    }
    pub fn month(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }
    pub fn epoch_days(self) -> i64 {
        let y = i64::from(self.year) - 1;
        let mut days = y * 365 + y / 4 - y / 100 + y / 400;
        for m in 1..self.month {
            days += i64::from(days_in_month(self.year, m));
        }
        days + i64::from(self.day) - 1 - 719_162
    }
    pub fn from_epoch_days(days: i64) -> Result<Self> {
        let ordinal = days.checked_add(719_162).ok_or(Error::Overflow)?;
        let upper = Self::new(9999, 12, 31)?.epoch_days() + 719_162;
        if !(0..=upper).contains(&ordinal) {
            return Err(Error::InvalidDate);
        }
        let mut lo = 1u16;
        let mut hi = 9999u16;
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if Self::new(mid, 1, 1)?.epoch_days() + 719_162 <= ordinal {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let mut rest = ordinal - (Self::new(lo, 1, 1)?.epoch_days() + 719_162);
        let mut month = 1;
        while rest >= i64::from(days_in_month(lo, month)) {
            rest -= i64::from(days_in_month(lo, month));
            month += 1;
        }
        Self::new(lo, month, (rest + 1) as u8)
    }
    pub fn add_days(self, days: i64) -> Result<Self> {
        Self::from_epoch_days(self.epoch_days().checked_add(days).ok_or(Error::Overflow)?)
    }
    /// Monday=0 through Sunday=6.
    pub fn weekday(self) -> u8 {
        (self.epoch_days() + 3).rem_euclid(7) as u8
    }
}
pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct LocalDateTime {
    date: Date,
    micros: i64,
}
impl LocalDateTime {
    pub fn new(date: Date, hour: u8, minute: u8, second: u8, microsecond: u32) -> Result<Self> {
        if hour >= 24 || minute >= 60 || second >= 60 || microsecond >= 1_000_000 {
            return Err(Error::InvalidDate);
        }
        Ok(Self {
            date,
            micros: (i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)) * SECOND
                + i64::from(microsecond),
        })
    }
    pub fn date(self) -> Date {
        self.date
    }
    pub fn midnight(date: Date) -> Self {
        Self { date, micros: 0 }
    }
    pub fn with_date(self, date: Date) -> Self {
        Self { date, ..self }
    }
    pub fn naive_micros(self) -> i64 {
        self.date.epoch_days() * DAY + self.micros
    }
    pub fn from_naive_micros(micros: i64) -> Result<Self> {
        Ok(Self {
            date: Date::from_epoch_days(micros.div_euclid(DAY))?,
            micros: micros.rem_euclid(DAY),
        })
    }
    pub fn checked_add(self, micros: i64) -> Result<Self> {
        Self::from_naive_micros(
            self.naive_micros()
                .checked_add(micros)
                .ok_or(Error::Overflow)?,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ZoneId(String);
impl ZoneId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
        {
            return Err(Error::UnknownZone);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveChoice {
    Reject,
    Earlier,
    Later,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalResolution {
    Unique(Instant),
    Gap,
    Fold { earlier: Instant, later: Instant },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZonedPoint {
    pub local: LocalDateTime,
    pub offset_seconds: i32,
}
/// Implementations supply their own timezone rules. No host TZ or timezone DB lookup.
pub trait TimeZoneResolver {
    fn resolve_local(&self, zone: &ZoneId, local: LocalDateTime) -> Result<LocalResolution>;
    fn local_at(&self, zone: &ZoneId, instant: Instant) -> Result<ZonedPoint>;
}
pub fn resolve(
    resolver: &dyn TimeZoneResolver,
    zone: &ZoneId,
    local: LocalDateTime,
    choice: ResolveChoice,
) -> Result<Instant> {
    let check = |instant: Instant| -> Result<()> {
        let point = resolver.local_at(zone, instant)?;
        if point.local != local
            || point.offset_seconds.unsigned_abs() >= 86_400
            || local
                .naive_micros()
                .checked_sub(i64::from(point.offset_seconds) * SECOND)
                != Some(instant.0)
        {
            return Err(Error::InvalidResolver);
        }
        Ok(())
    };
    match resolver.resolve_local(zone, local)? {
        LocalResolution::Gap => Err(Error::NonexistentLocalTime),
        LocalResolution::Unique(i) => {
            check(i)?;
            Ok(i)
        }
        LocalResolution::Fold { earlier, later } => {
            if earlier >= later {
                return Err(Error::InvalidResolver);
            }
            check(earlier)?;
            check(later)?;
            match choice {
                ResolveChoice::Reject => Err(Error::AmbiguousLocalTime),
                ResolveChoice::Earlier => Ok(earlier),
                ResolveChoice::Later => Ok(later),
            }
        }
    }
}
/// An explicitly configured single fixed-offset zone; not a timezone database.
pub struct FixedOffsetResolver {
    zone: ZoneId,
    offset_seconds: i32,
}
impl FixedOffsetResolver {
    pub fn new(zone: ZoneId, offset_seconds: i32) -> Result<Self> {
        if offset_seconds.unsigned_abs() >= 86_400 {
            return Err(Error::InvalidResolver);
        }
        Ok(Self {
            zone,
            offset_seconds,
        })
    }
}
impl TimeZoneResolver for FixedOffsetResolver {
    fn resolve_local(&self, zone: &ZoneId, local: LocalDateTime) -> Result<LocalResolution> {
        if zone != &self.zone {
            return Err(Error::UnknownZone);
        }
        Ok(LocalResolution::Unique(
            Instant(local.naive_micros()).checked_add(-i64::from(self.offset_seconds) * SECOND)?,
        ))
    }
    fn local_at(&self, zone: &ZoneId, instant: Instant) -> Result<ZonedPoint> {
        if zone != &self.zone {
            return Err(Error::UnknownZone);
        }
        Ok(ZonedPoint {
            local: LocalDateTime::from_naive_micros(
                instant
                    .checked_add(i64::from(self.offset_seconds) * SECOND)?
                    .0,
            )?,
            offset_seconds: self.offset_seconds,
        })
    }
}
/// Adapter seam for PAL realtime_ns. Jobs' virtual monotonic milliseconds are not UTC.
pub trait RealtimeSource {
    fn realtime_ns(&self) -> Result<u64>;
}
pub struct FixedClock(pub u64);
impl RealtimeSource for FixedClock {
    fn realtime_ns(&self) -> Result<u64> {
        Ok(self.0)
    }
}
pub struct UnavailableClock;
impl RealtimeSource for UnavailableClock {
    fn realtime_ns(&self) -> Result<u64> {
        Err(Error::ClockUnavailable)
    }
}
