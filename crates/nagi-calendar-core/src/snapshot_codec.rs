//! Bounded host-only byte format for typed Calendar snapshots.
//! Integrity detection is not authentication or an anti-rollback protocol.
use crate::model::{
    Calendar, CalendarId, DraftState, DurationMode, Event, EventDraft, EventId, EventRevision,
    Interval, OccurrenceKey, OwnerMetadata, Reminder, Schedule,
};
use crate::recurrence::{Frequency, Recurrence, RecurrenceBound};
use crate::snapshot::{StoreSnapshot, SNAPSHOT_VERSION};
use crate::time::{Date, Instant, LocalDateTime, ResolveChoice, ZoneId};
use crate::{AppId, Error, ObjectId, Result, UserId, WorkspaceId};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const HEADER: usize = 20;
const MAGIC: &[u8; 4] = b"NCS1";
const MAX_ITEMS: usize = 65_536;
const MAX_EXCEPTIONS: usize = 4096;

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
struct Writer(Vec<u8>);
impl Writer {
    fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_SNAPSHOT_BYTES - HEADER)
        {
            return Err(Error::Capacity);
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn byte(&mut self, v: u8) -> Result<()> {
        self.raw(&[v])
    }
    fn u32(&mut self, v: u32) -> Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) -> Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn i64(&mut self, v: i64) -> Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn count(&mut self, n: usize) -> Result<()> {
        self.u32(u32::try_from(n).map_err(|_| Error::Capacity)?)
    }
    fn string(&mut self, s: &str) -> Result<()> {
        self.count(s.len())?;
        self.raw(s.as_bytes())
    }
    fn option<T>(
        &mut self,
        value: Option<&T>,
        write: impl FnOnce(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        match value {
            Some(v) => {
                self.byte(1)?;
                write(self, v)
            }
            None => self.byte(0),
        }
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn raw(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::InvalidSnapshot)?;
        let part = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::InvalidSnapshot)?;
        self.pos = end;
        Ok(part)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.raw(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.raw(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.raw(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.raw(8)?.try_into().unwrap()))
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > max {
            Err(Error::Capacity)
        } else {
            Ok(n)
        }
    }
    fn string(&mut self, max: usize) -> Result<String> {
        let n = self.count(max)?;
        String::from_utf8(self.raw(n)?.to_vec()).map_err(|_| Error::InvalidSnapshot)
    }
    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        match self.byte()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(Error::InvalidSnapshot),
        }
    }
    fn finish(&self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::InvalidSnapshot)
        }
    }
}
fn write_owner(w: &mut Writer, owner: &OwnerMetadata) -> Result<()> {
    w.u64(owner.app.0)?;
    w.option(owner.user.as_ref(), |w, id| w.u64(id.0))
}
fn read_owner(r: &mut Reader<'_>) -> Result<OwnerMetadata> {
    Ok(OwnerMetadata {
        app: AppId(r.u64()?),
        user: r.option(|r| Ok(UserId(r.u64()?)))?,
    })
}
fn write_date(w: &mut Writer, date: Date) -> Result<()> {
    w.i64(date.epoch_days())
}
fn read_date(r: &mut Reader<'_>) -> Result<Date> {
    Date::from_epoch_days(r.i64()?)
}
fn write_local(w: &mut Writer, local: LocalDateTime) -> Result<()> {
    w.i64(local.naive_micros())
}
fn read_local(r: &mut Reader<'_>) -> Result<LocalDateTime> {
    LocalDateTime::from_naive_micros(r.i64()?)
}
fn write_choice(w: &mut Writer, choice: ResolveChoice) -> Result<()> {
    w.byte(match choice {
        ResolveChoice::Reject => 0,
        ResolveChoice::Earlier => 1,
        ResolveChoice::Later => 2,
    })
}
fn read_choice(r: &mut Reader<'_>) -> Result<ResolveChoice> {
    match r.byte()? {
        0 => Ok(ResolveChoice::Reject),
        1 => Ok(ResolveChoice::Earlier),
        2 => Ok(ResolveChoice::Later),
        _ => Err(Error::InvalidSnapshot),
    }
}
fn write_key(w: &mut Writer, key: OccurrenceKey) -> Result<()> {
    match key {
        OccurrenceKey::Instant(i) => {
            w.byte(0)?;
            w.i64(i.0)
        }
        OccurrenceKey::Local(t) => {
            w.byte(1)?;
            write_local(w, t)
        }
        OccurrenceKey::Date(d) => {
            w.byte(2)?;
            write_date(w, d)
        }
    }
}
fn read_key(r: &mut Reader<'_>) -> Result<OccurrenceKey> {
    match r.byte()? {
        0 => Ok(OccurrenceKey::Instant(Instant(r.i64()?))),
        1 => Ok(OccurrenceKey::Local(read_local(r)?)),
        2 => Ok(OccurrenceKey::Date(read_date(r)?)),
        _ => Err(Error::InvalidSnapshot),
    }
}
fn write_schedule(w: &mut Writer, schedule: &Schedule) -> Result<()> {
    match schedule {
        Schedule::Instant(i) => {
            w.byte(0)?;
            w.i64(i.start().0)?;
            w.i64(i.end().0)
        }
        Schedule::Zoned {
            start,
            end,
            zone,
            start_choice,
            end_choice,
            duration_mode,
        } => {
            w.byte(1)?;
            write_local(w, *start)?;
            write_local(w, *end)?;
            w.string(zone.as_str())?;
            write_choice(w, *start_choice)?;
            write_choice(w, *end_choice)?;
            w.byte(match duration_mode {
                DurationMode::Wall => 0,
                DurationMode::Elapsed => 1,
            })
        }
        Schedule::Floating { start, end } => {
            w.byte(2)?;
            write_local(w, *start)?;
            write_local(w, *end)
        }
        Schedule::AllDay {
            start,
            end_exclusive,
        } => {
            w.byte(3)?;
            write_date(w, *start)?;
            write_date(w, *end_exclusive)
        }
    }
}
fn read_schedule(r: &mut Reader<'_>) -> Result<Schedule> {
    match r.byte()? {
        0 => Ok(Schedule::Instant(Interval::new(
            Instant(r.i64()?),
            Instant(r.i64()?),
        )?)),
        1 => {
            let start = read_local(r)?;
            let end = read_local(r)?;
            let zone = ZoneId::new(r.string(128)?)?;
            let start_choice = read_choice(r)?;
            let end_choice = read_choice(r)?;
            let duration_mode = match r.byte()? {
                0 => DurationMode::Wall,
                1 => DurationMode::Elapsed,
                _ => return Err(Error::InvalidSnapshot),
            };
            Ok(Schedule::Zoned {
                start,
                end,
                zone,
                start_choice,
                end_choice,
                duration_mode,
            })
        }
        2 => Ok(Schedule::Floating {
            start: read_local(r)?,
            end: read_local(r)?,
        }),
        3 => Ok(Schedule::AllDay {
            start: read_date(r)?,
            end_exclusive: read_date(r)?,
        }),
        _ => Err(Error::InvalidSnapshot),
    }
}
fn write_recurrence(w: &mut Writer, rec: &Recurrence) -> Result<()> {
    match &rec.frequency {
        Frequency::Daily => w.byte(0)?,
        Frequency::Weekly { weekdays } => {
            w.byte(1)?;
            w.count(weekdays.len())?;
            for day in weekdays {
                w.byte(*day)?;
            }
        }
        Frequency::Monthly => w.byte(2)?,
        Frequency::InstantEvery { micros } => {
            w.byte(3)?;
            w.i64(*micros)?;
        }
    }
    w.u32(rec.interval)?;
    match rec.bound {
        RecurrenceBound::Count(n) => {
            w.byte(0)?;
            w.u32(n)
        }
        RecurrenceBound::Until(key) => {
            w.byte(1)?;
            write_key(w, key)
        }
    }
}
fn read_recurrence(r: &mut Reader<'_>) -> Result<Recurrence> {
    let frequency = match r.byte()? {
        0 => Frequency::Daily,
        1 => {
            let mut weekdays = BTreeSet::new();
            for _ in 0..r.count(7)? {
                if !weekdays.insert(r.byte()?) {
                    return Err(Error::InvalidSnapshot);
                }
            }
            Frequency::Weekly { weekdays }
        }
        2 => Frequency::Monthly,
        3 => Frequency::InstantEvery { micros: r.i64()? },
        _ => return Err(Error::InvalidSnapshot),
    };
    let interval = r.u32()?;
    let bound = match r.byte()? {
        0 => RecurrenceBound::Count(r.u32()?),
        1 => RecurrenceBound::Until(read_key(r)?),
        _ => return Err(Error::InvalidSnapshot),
    };
    Ok(Recurrence {
        frequency,
        interval,
        bound,
    })
}

fn write_draft(w: &mut Writer, draft: &EventDraft) -> Result<()> {
    w.string(draft.calendar_id.as_str())?;
    w.string(&draft.title)?;
    w.string(&draft.description)?;
    w.string(&draft.location)?;
    w.option(draft.organizer.as_ref(), |w, s| w.string(s))?;
    w.count(draft.participants.len())?;
    for p in &draft.participants {
        w.string(p)?;
    }
    write_schedule(w, &draft.schedule)?;
    w.option(draft.recurrence.as_ref(), write_recurrence)?;
    w.count(draft.exdates.len())?;
    for key in &draft.exdates {
        write_key(w, *key)?;
    }
    w.count(draft.overrides.len())?;
    for (key, schedule) in &draft.overrides {
        write_key(w, *key)?;
        write_schedule(w, schedule)?;
    }
    w.count(draft.reminders.len())?;
    for reminder in &draft.reminders {
        match reminder {
            Reminder::RelativeMicros(v) => {
                w.byte(0)?;
                w.i64(*v)?;
            }
            Reminder::Absolute(v) => {
                w.byte(1)?;
                w.i64(v.0)?;
            }
        }
    }
    w.count(draft.resources.len())?;
    for id in &draft.resources {
        w.u64(id.0)?;
    }
    w.count(draft.workspaces.len())?;
    for id in &draft.workspaces {
        w.u64(id.0)?;
    }
    w.option(draft.owner.as_ref(), write_owner)?;
    w.byte(match draft.state {
        DraftState::Draft => 0,
        DraftState::Validated => 1,
        DraftState::RequiresConfirmation => 2,
    })?;
    w.byte(u8::from(draft.busy))
}
fn read_draft(r: &mut Reader<'_>) -> Result<EventDraft> {
    let calendar_id = CalendarId::new(r.string(128)?)?;
    let title = r.string(4096)?;
    let description = r.string(65_536)?;
    let location = r.string(4096)?;
    let organizer = r.option(|r| r.string(4096))?;
    let mut participants = Vec::new();
    for _ in 0..r.count(256)? {
        participants.push(r.string(4096)?);
    }
    let schedule = read_schedule(r)?;
    let recurrence = r.option(read_recurrence)?;
    let mut exdates = BTreeSet::new();
    for _ in 0..r.count(MAX_EXCEPTIONS)? {
        if !exdates.insert(read_key(r)?) {
            return Err(Error::InvalidSnapshot);
        }
    }
    let mut overrides = BTreeMap::new();
    for _ in 0..r.count(MAX_EXCEPTIONS)? {
        let key = read_key(r)?;
        let value = read_schedule(r)?;
        if overrides.insert(key, value).is_some() {
            return Err(Error::InvalidSnapshot);
        }
    }
    let mut reminders = Vec::new();
    for _ in 0..r.count(256)? {
        reminders.push(match r.byte()? {
            0 => Reminder::RelativeMicros(r.i64()?),
            1 => Reminder::Absolute(Instant(r.i64()?)),
            _ => return Err(Error::InvalidSnapshot),
        });
    }
    let mut resources = Vec::new();
    for _ in 0..r.count(256)? {
        resources.push(ObjectId(r.u64()?));
    }
    let mut workspaces = Vec::new();
    for _ in 0..r.count(256)? {
        workspaces.push(WorkspaceId(r.u64()?));
    }
    let owner = r.option(read_owner)?;
    let state = match r.byte()? {
        0 => DraftState::Draft,
        1 => DraftState::Validated,
        2 => DraftState::RequiresConfirmation,
        _ => return Err(Error::InvalidSnapshot),
    };
    let busy = match r.byte()? {
        0 => false,
        1 => true,
        _ => return Err(Error::InvalidSnapshot),
    };
    Ok(EventDraft {
        calendar_id,
        title,
        description,
        location,
        organizer,
        participants,
        schedule,
        recurrence,
        exdates,
        overrides,
        reminders,
        resources,
        workspaces,
        owner,
        state,
        busy,
    })
}
fn write_snapshot(w: &mut Writer, snapshot: &StoreSnapshot) -> Result<()> {
    w.count(snapshot.calendars.len())?;
    for c in &snapshot.calendars {
        w.string(c.id.as_str())?;
        w.string(&c.name)?;
        w.option(c.owner.as_ref(), write_owner)?;
    }
    w.count(snapshot.events.len())?;
    for e in &snapshot.events {
        w.string(e.id.as_str())?;
        w.u64(e.revision.get())?;
        w.i64(e.created_at.0)?;
        w.i64(e.modified_at.0)?;
        write_draft(w, &e.draft)?;
    }
    w.count(snapshot.used_calendar_ids.len())?;
    for id in &snapshot.used_calendar_ids {
        w.string(id.as_str())?;
    }
    w.count(snapshot.used_event_ids.len())?;
    for id in &snapshot.used_event_ids {
        w.string(id.as_str())?;
    }
    Ok(())
}
fn read_snapshot(r: &mut Reader<'_>) -> Result<StoreSnapshot> {
    let mut calendars = Vec::new();
    for _ in 0..r.count(MAX_ITEMS)? {
        calendars.push(Calendar {
            id: CalendarId::new(r.string(128)?)?,
            name: r.string(4096)?,
            owner: r.option(read_owner)?,
        });
    }
    let mut events = Vec::new();
    for _ in 0..r.count(MAX_ITEMS)? {
        events.push(Event {
            id: EventId::new(r.string(128)?)?,
            revision: EventRevision::new(r.u64()?)?,
            created_at: Instant(r.i64()?),
            modified_at: Instant(r.i64()?),
            draft: read_draft(r)?,
        });
    }
    let mut used_calendar_ids = Vec::new();
    for _ in 0..r.count(MAX_ITEMS)? {
        used_calendar_ids.push(CalendarId::new(r.string(128)?)?);
    }
    let mut used_event_ids = Vec::new();
    for _ in 0..r.count(MAX_ITEMS)? {
        used_event_ids.push(EventId::new(r.string(128)?)?);
    }
    Ok(StoreSnapshot {
        version: SNAPSHOT_VERSION,
        calendars,
        events,
        used_calendar_ids,
        used_event_ids,
    })
}
impl StoreSnapshot {
    /// Encode the full typed snapshot to a versioned, bounded byte sequence.
    /// The checksum detects accidental corruption; it does not authenticate
    /// bytes or prevent a caller from selecting an older valid snapshot.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if [
            self.calendars.len(),
            self.events.len(),
            self.used_calendar_ids.len(),
            self.used_event_ids.len(),
        ]
        .iter()
        .any(|n| *n > MAX_ITEMS)
        {
            return Err(Error::Capacity);
        }
        if self.version != SNAPSHOT_VERSION {
            return Err(Error::UnsupportedSnapshotVersion);
        }
        let mut writer = Writer(Vec::new());
        write_snapshot(&mut writer, self)?;
        let payload = writer.0;
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&checksum(&payload).to_le_bytes());
        out.extend_from_slice(&payload);
        // The byte cap is enforced before decoding and validating the output,
        // so a public typed snapshot cannot force an unbounded preflight clone.
        if Self::from_bytes(&out)? != *self {
            return Err(Error::InvalidSnapshot);
        }
        Ok(out)
    }
    /// Decode and validate an untrusted byte sequence before a receiving store
    /// decides whether to replace its state.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(Error::Capacity);
        }
        if bytes.len() < HEADER || &bytes[..4] != MAGIC {
            return Err(Error::InvalidSnapshot);
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != SNAPSHOT_VERSION {
            return Err(Error::UnsupportedSnapshotVersion);
        }
        let declared = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if declared != bytes.len() - HEADER {
            return Err(Error::InvalidSnapshot);
        }
        let expected = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let payload = &bytes[HEADER..];
        if checksum(payload) != expected {
            return Err(Error::InvalidSnapshot);
        }
        let mut reader = Reader {
            bytes: payload,
            pos: 0,
        };
        let snapshot = read_snapshot(&mut reader)?;
        reader.finish()?;
        snapshot.clone().validate(MAX_ITEMS)?;
        Ok(snapshot)
    }
}
