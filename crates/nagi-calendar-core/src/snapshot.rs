//! Typed in-memory snapshots, not a byte format or durable guest storage.
//! Version 1 is the first schema. Unknown versions require an explicit future
//! migration; they are never interpreted as version 1.
use crate::model::{validate_text, Calendar, CalendarId, Event, EventId};
use crate::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

pub const SNAPSHOT_VERSION: u32 = 1;

/// An owned, untrusted candidate for explicit store state replacement.
///
/// ID history includes both live records and deletion tombstones. Clock,
/// IdSource internals and receiving-store capacity are intentionally absent.
/// External resource/workspace/owner IDs remain opaque metadata, not authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreSnapshot {
    pub version: u32,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub used_calendar_ids: Vec<CalendarId>,
    pub used_event_ids: Vec<EventId>,
}

pub(crate) struct ValidatedSnapshot {
    pub calendars: BTreeMap<CalendarId, Calendar>,
    pub events: BTreeMap<EventId, Event>,
    pub used_calendars: BTreeSet<CalendarId>,
    pub used_events: BTreeSet<EventId>,
}

impl StoreSnapshot {
    pub(crate) fn validate(self, capacity: usize) -> Result<ValidatedSnapshot> {
        if self.version != SNAPSHOT_VERSION {
            return Err(Error::UnsupportedSnapshotVersion);
        }
        // Check counts before scanning records or allocating replacement indexes.
        if [
            self.calendars.len(),
            self.events.len(),
            self.used_calendar_ids.len(),
            self.used_event_ids.len(),
        ]
        .iter()
        .any(|n| *n > capacity)
        {
            return Err(Error::Capacity);
        }
        let mut used_calendars = BTreeSet::new();
        for id in self.used_calendar_ids {
            if !used_calendars.insert(id) {
                return Err(Error::DuplicateId);
            }
        }
        let mut used_events = BTreeSet::new();
        for id in self.used_event_ids {
            if !used_events.insert(id) {
                return Err(Error::DuplicateId);
            }
        }
        let mut calendars = BTreeMap::new();
        for calendar in self.calendars {
            validate_text(&calendar.name, 4096, true)?;
            if !used_calendars.contains(&calendar.id) {
                return Err(Error::InvalidSnapshot);
            }
            if calendars.insert(calendar.id.clone(), calendar).is_some() {
                return Err(Error::DuplicateId);
            }
        }
        let mut events = BTreeMap::new();
        for event in self.events {
            event.draft.validate()?;
            if event.modified_at < event.created_at
                || !used_events.contains(&event.id)
                || !calendars.contains_key(&event.draft.calendar_id)
            {
                return Err(Error::InvalidSnapshot);
            }
            if events.insert(event.id.clone(), event).is_some() {
                return Err(Error::DuplicateId);
            }
        }
        Ok(ValidatedSnapshot {
            calendars,
            events,
            used_calendars,
            used_events,
        })
    }
}
