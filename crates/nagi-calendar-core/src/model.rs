use crate::recurrence::Recurrence;
use crate::time::{resolve, Date, Instant, LocalDateTime, ResolveChoice, TimeZoneResolver, ZoneId};
use crate::{AppId, Error, ObjectId, Result, UserId, WorkspaceId};
use std::collections::{BTreeMap, BTreeSet};

macro_rules! id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                let suffix = value.strip_prefix($prefix).ok_or(Error::InvalidId)?;
                if value.len() > 128
                    || suffix.is_empty()
                    || !suffix
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
                {
                    return Err(Error::InvalidId);
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}
id!(CalendarId, "cal_");
id!(CalendarEventId, "event_");
pub type EventId = CalendarEventId;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventRevision(u64);
impl EventRevision {
    pub fn new(value: u64) -> Result<Self> {
        if value == 0 {
            Err(Error::RevisionConflict)
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> u64 {
        self.0
    }
    pub fn next(self) -> Result<Self> {
        self.0.checked_add(1).map(Self).ok_or(Error::Overflow)
    }
}
/// Owner/provenance metadata; never an authorization token or authenticated caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerMetadata {
    pub app: AppId,
    pub user: Option<UserId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Calendar {
    pub id: CalendarId,
    pub name: String,
    pub owner: Option<OwnerMetadata>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interval {
    start: Instant,
    end: Instant,
}
impl Interval {
    pub fn new(start: Instant, end: Instant) -> Result<Self> {
        if end <= start {
            return Err(Error::InvalidRange);
        }
        end.0.checked_sub(start.0).ok_or(Error::Overflow)?;
        Ok(Self { start, end })
    }
    pub fn start(self) -> Instant {
        self.start
    }
    pub fn end(self) -> Instant {
        self.end
    }
    pub fn duration(self) -> i64 {
        self.end.0 - self.start.0
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && self.end > other.start
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurationMode {
    Wall,
    Elapsed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Schedule {
    Instant(Interval),
    Zoned {
        start: LocalDateTime,
        end: LocalDateTime,
        zone: ZoneId,
        start_choice: ResolveChoice,
        end_choice: ResolveChoice,
        duration_mode: DurationMode,
    },
    Floating {
        start: LocalDateTime,
        end: LocalDateTime,
    },
    AllDay {
        start: Date,
        end_exclusive: Date,
    },
}
#[derive(Clone, Debug)]
pub struct ZoneContext {
    pub zone: ZoneId,
    pub choice: ResolveChoice,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum OccurrenceKey {
    Instant(Instant),
    Local(LocalDateTime),
    Date(Date),
}
impl Schedule {
    pub fn key(&self) -> OccurrenceKey {
        match self {
            Self::Instant(i) => OccurrenceKey::Instant(i.start()),
            Self::Zoned { start, .. } | Self::Floating { start, .. } => {
                OccurrenceKey::Local(*start)
            }
            Self::AllDay { start, .. } => OccurrenceKey::Date(*start),
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Instant(_) => Ok(()),
            Self::Zoned { start, end, .. } | Self::Floating { start, end } if end > start => Ok(()),
            Self::AllDay {
                start,
                end_exclusive,
            } if end_exclusive > start => Ok(()),
            _ => Err(Error::InvalidRange),
        }
    }
    pub fn resolve(
        &self,
        resolver: &dyn TimeZoneResolver,
        context: Option<&ZoneContext>,
    ) -> Result<Interval> {
        self.validate()?;
        match self {
            Self::Instant(i) => Ok(*i),
            Self::Zoned {
                start,
                end,
                zone,
                start_choice,
                end_choice,
                ..
            } => Interval::new(
                resolve(resolver, zone, *start, *start_choice)?,
                resolve(resolver, zone, *end, *end_choice)?,
            ),
            Self::Floating { start, end } => {
                let c = context.ok_or(Error::ZoneContextRequired)?;
                Interval::new(
                    resolve(resolver, &c.zone, *start, c.choice)?,
                    resolve(resolver, &c.zone, *end, c.choice)?,
                )
            }
            Self::AllDay {
                start,
                end_exclusive,
            } => {
                let c = context.ok_or(Error::ZoneContextRequired)?;
                Interval::new(
                    resolve(resolver, &c.zone, LocalDateTime::midnight(*start), c.choice)?,
                    resolve(
                        resolver,
                        &c.zone,
                        LocalDateTime::midnight(*end_exclusive),
                        c.choice,
                    )?,
                )
            }
        }
    }
    pub(crate) fn shifted(
        &self,
        key: OccurrenceKey,
        resolver: &dyn TimeZoneResolver,
    ) -> Result<Self> {
        let next = match (self, key) {
            (Self::Instant(i), OccurrenceKey::Instant(start)) => {
                Self::Instant(Interval::new(start, start.checked_add(i.duration())?)?)
            }
            (Self::Floating { start, end }, OccurrenceKey::Local(next)) => Self::Floating {
                start: next,
                end: next.checked_add(end.naive_micros() - start.naive_micros())?,
            },
            (
                Self::Zoned {
                    start,
                    end,
                    zone,
                    start_choice,
                    end_choice,
                    duration_mode,
                },
                OccurrenceKey::Local(next),
            ) => match duration_mode {
                DurationMode::Wall => Self::Zoned {
                    start: next,
                    end: next.checked_add(end.naive_micros() - start.naive_micros())?,
                    zone: zone.clone(),
                    start_choice: *start_choice,
                    end_choice: *end_choice,
                    duration_mode: *duration_mode,
                },
                DurationMode::Elapsed => {
                    let duration = self.resolve(resolver, None)?.duration();
                    let instant = resolve(resolver, zone, next, *start_choice)?;
                    Self::Instant(Interval::new(instant, instant.checked_add(duration)?)?)
                }
            },
            (
                Self::AllDay {
                    start,
                    end_exclusive,
                },
                OccurrenceKey::Date(next),
            ) => Self::AllDay {
                start: next,
                end_exclusive: next.add_days(end_exclusive.epoch_days() - start.epoch_days())?,
            },
            _ => return Err(Error::InvalidRecurrence),
        };
        next.validate()?;
        Ok(next)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reminder {
    RelativeMicros(i64),
    Absolute(Instant),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftState {
    Draft,
    Validated,
    RequiresConfirmation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDraft {
    pub calendar_id: CalendarId,
    pub title: String,
    pub description: String,
    pub location: String,
    /// Adapter-provided contact labels, not authenticated Person IDs.
    pub organizer: Option<String>,
    pub participants: Vec<String>,
    pub schedule: Schedule,
    pub recurrence: Option<Recurrence>,
    pub exdates: BTreeSet<OccurrenceKey>,
    pub overrides: BTreeMap<OccurrenceKey, Schedule>,
    pub reminders: Vec<Reminder>,
    pub resources: Vec<ObjectId>,
    pub workspaces: Vec<WorkspaceId>,
    pub owner: Option<OwnerMetadata>,
    pub state: DraftState,
    pub busy: bool,
}
impl EventDraft {
    pub fn new(calendar_id: CalendarId, title: impl Into<String>, schedule: Schedule) -> Self {
        Self {
            calendar_id,
            title: title.into(),
            description: String::new(),
            location: String::new(),
            organizer: None,
            participants: vec![],
            schedule,
            recurrence: None,
            exdates: BTreeSet::new(),
            overrides: BTreeMap::new(),
            reminders: vec![],
            resources: vec![],
            workspaces: vec![],
            owner: None,
            state: DraftState::Draft,
            busy: true,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_text(&self.title, 4096, true)?;
        validate_text(&self.description, 65_536, false)?;
        validate_text(&self.location, 4096, false)?;
        if let Some(o) = &self.organizer {
            validate_text(o, 4096, true)?;
        }
        if [
            self.participants.len(),
            self.reminders.len(),
            self.resources.len(),
            self.workspaces.len(),
        ]
        .iter()
        .any(|n| *n > 256)
            || self.exdates.len() > 4096
            || self.overrides.len() > 4096
        {
            return Err(Error::Capacity);
        }
        for p in &self.participants {
            validate_text(p, 4096, true)?;
        }
        self.schedule.validate()?;
        if let Some(r) = &self.recurrence {
            r.validate(self.schedule.key())?;
            r.validate_exceptions(
                self.schedule.key(),
                self.exdates
                    .iter()
                    .chain(self.overrides.keys())
                    .copied()
                    .collect(),
            )?;
        } else if !self.exdates.is_empty() || !self.overrides.is_empty() {
            return Err(Error::InvalidException);
        }
        for key in self.exdates.iter().chain(self.overrides.keys()) {
            if std::mem::discriminant(key) != std::mem::discriminant(&self.schedule.key())
                || *key < self.schedule.key()
            {
                return Err(Error::InvalidException);
            }
        }
        for (key, replacement) in &self.overrides {
            if self.exdates.contains(key) {
                return Err(Error::InvalidException);
            }
            replacement.validate()?;
            if std::mem::discriminant(&replacement.key())
                != std::mem::discriminant(&self.schedule.key())
            {
                return Err(Error::InvalidException);
            }
        }
        Ok(())
    }
}
pub(crate) fn validate_text(text: &str, max: usize, required: bool) -> Result<()> {
    if text.len() > max || text.contains('\0') || required && text.trim().is_empty() {
        Err(Error::InvalidText)
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub id: EventId,
    pub revision: EventRevision,
    pub created_at: Instant,
    pub modified_at: Instant,
    pub draft: EventDraft,
}
