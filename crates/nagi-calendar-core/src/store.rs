use crate::model::*;
use crate::time::{Instant, RealtimeSource};
use crate::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

pub trait IdSource {
    fn next_calendar_id(&mut self) -> Result<CalendarId>;
    fn next_event_id(&mut self) -> Result<EventId>;
}
/// Deterministic fixture ID source. Durable/global identity allocation is deferred.
pub struct SequentialIds {
    calendar: u64,
    event: u64,
}
impl SequentialIds {
    pub fn new() -> Self {
        Self {
            calendar: 0,
            event: 0,
        }
    }
}
impl Default for SequentialIds {
    fn default() -> Self {
        Self::new()
    }
}
impl IdSource for SequentialIds {
    fn next_calendar_id(&mut self) -> Result<CalendarId> {
        self.calendar = self.calendar.checked_add(1).ok_or(Error::Overflow)?;
        CalendarId::new(format!("cal_{}", self.calendar))
    }
    fn next_event_id(&mut self) -> Result<EventId> {
        self.event = self.event.checked_add(1).ok_or(Error::Overflow)?;
        EventId::new(format!("event_{}", self.event))
    }
}
pub trait EventStore {
    fn create(&mut self, draft: EventDraft) -> Result<Event>;
    fn get(&self, id: &EventId) -> Result<Event>;
    fn list(&self, calendar: &CalendarId, limit: usize) -> Result<Vec<Event>>;
    fn search(&self, calendar: &CalendarId, text: &str, limit: usize) -> Result<Vec<Event>>;
    fn update(&mut self, id: &EventId, expected: EventRevision, draft: EventDraft)
        -> Result<Event>;
    fn delete(&mut self, id: &EventId, expected: EventRevision) -> Result<Event>;
}
/// Local reference store. No persistence, authentication, provider or runtime access.
pub struct InMemoryStore<C, I> {
    clock: C,
    ids: I,
    capacity: usize,
    calendars: BTreeMap<CalendarId, Calendar>,
    events: BTreeMap<EventId, Event>,
    used_events: BTreeSet<EventId>,
    used_calendars: BTreeSet<CalendarId>,
}
impl<C: RealtimeSource, I: IdSource> InMemoryStore<C, I> {
    pub fn new(clock: C, ids: I, capacity: usize) -> Result<Self> {
        if capacity == 0 || capacity > 65_536 {
            return Err(Error::InvalidLimit);
        }
        Ok(Self {
            clock,
            ids,
            capacity,
            calendars: BTreeMap::new(),
            events: BTreeMap::new(),
            used_events: BTreeSet::new(),
            used_calendars: BTreeSet::new(),
        })
    }
    pub fn create_calendar(
        &mut self,
        name: impl Into<String>,
        owner: Option<OwnerMetadata>,
    ) -> Result<Calendar> {
        let name = name.into();
        validate_text(&name, 4096, true)?;
        if self.used_calendars.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        let id = self.ids.next_calendar_id()?;
        if self.used_calendars.contains(&id) {
            return Err(Error::DuplicateId);
        }
        let calendar = Calendar {
            id: id.clone(),
            name,
            owner,
        };
        self.used_calendars.insert(id.clone());
        self.calendars.insert(id, calendar.clone());
        Ok(calendar)
    }
    pub fn calendar(&self, id: &CalendarId) -> Result<Calendar> {
        self.calendars.get(id).cloned().ok_or(Error::NotFound)
    }
    pub fn update_calendar(
        &mut self,
        id: &CalendarId,
        name: impl Into<String>,
        owner: Option<OwnerMetadata>,
    ) -> Result<Calendar> {
        self.calendar(id)?;
        let name = name.into();
        validate_text(&name, 4096, true)?;
        let calendar = Calendar {
            id: id.clone(),
            name,
            owner,
        };
        self.calendars.insert(id.clone(), calendar.clone());
        Ok(calendar)
    }
    pub fn delete_calendar(&mut self, id: &CalendarId) -> Result<Calendar> {
        let calendar = self.calendar(id)?;
        if self.events.values().any(|e| &e.draft.calendar_id == id) {
            return Err(Error::CalendarNotEmpty);
        }
        self.calendars.remove(id);
        Ok(calendar)
    }
    pub fn list_calendars(&self, limit: usize) -> Result<Vec<Calendar>> {
        check_limit(limit)?;
        if self.calendars.len() > limit {
            return Err(Error::OutputLimitExceeded);
        }
        Ok(self.calendars.values().cloned().collect())
    }
    fn validate_draft(&self, draft: &EventDraft) -> Result<()> {
        self.calendar(&draft.calendar_id)?;
        draft.validate()
    }
    fn now(&self) -> Result<Instant> {
        Instant::from_realtime_ns(self.clock.realtime_ns()?)
    }
}
pub(crate) fn check_limit(limit: usize) -> Result<()> {
    if limit == 0 || limit > 65_536 {
        Err(Error::InvalidLimit)
    } else {
        Ok(())
    }
}
impl<C: RealtimeSource, I: IdSource> EventStore for InMemoryStore<C, I> {
    fn create(&mut self, draft: EventDraft) -> Result<Event> {
        self.validate_draft(&draft)?;
        // Tombstones are bounded too: no ID reuse after deletion in this lifetime.
        if self.used_events.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        let now = self.now()?;
        let id = self.ids.next_event_id()?;
        if self.used_events.contains(&id) {
            return Err(Error::DuplicateId);
        }
        let event = Event {
            id: id.clone(),
            revision: EventRevision::new(1)?,
            created_at: now,
            modified_at: now,
            draft,
        };
        self.used_events.insert(id.clone());
        self.events.insert(id, event.clone());
        Ok(event)
    }
    fn get(&self, id: &EventId) -> Result<Event> {
        self.events.get(id).cloned().ok_or(Error::NotFound)
    }
    fn list(&self, calendar: &CalendarId, limit: usize) -> Result<Vec<Event>> {
        self.search(calendar, "", limit)
    }
    fn search(&self, calendar: &CalendarId, text: &str, limit: usize) -> Result<Vec<Event>> {
        self.calendar(calendar)?;
        check_limit(limit)?;
        validate_text(text, 4096, false)?;
        let mut found = vec![];
        for event in self
            .events
            .values()
            .filter(|e| &e.draft.calendar_id == calendar)
        {
            let d = &event.draft;
            if [
                d.title.as_str(),
                d.description.as_str(),
                d.location.as_str(),
            ]
            .iter()
            .any(|s| s.contains(text))
                || d.participants.iter().any(|s| s.contains(text))
            {
                if found.len() == limit {
                    return Err(Error::OutputLimitExceeded);
                }
                found.push(event.clone());
            }
        }
        Ok(found)
    }
    fn update(
        &mut self,
        id: &EventId,
        expected: EventRevision,
        draft: EventDraft,
    ) -> Result<Event> {
        let old = self.get(id)?;
        if old.revision != expected {
            return Err(Error::RevisionConflict);
        }
        self.validate_draft(&draft)?;
        let revision = old.revision.next()?;
        let now = self.now()?;
        if now < old.modified_at {
            return Err(Error::ClockRollback);
        }
        let next = Event {
            id: id.clone(),
            revision,
            created_at: old.created_at,
            modified_at: now,
            draft,
        };
        self.events.insert(id.clone(), next.clone());
        Ok(next)
    }
    fn delete(&mut self, id: &EventId, expected: EventRevision) -> Result<Event> {
        let event = self.get(id)?;
        if event.revision != expected {
            return Err(Error::RevisionConflict);
        }
        self.events.remove(id);
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FixedClock;
    #[test]
    fn stored_revision_overflow_does_not_mutate_the_record() {
        let mut store = InMemoryStore::new(FixedClock(0), SequentialIds::new(), 10).unwrap();
        let calendar = store.create_calendar("test", None).unwrap();
        let draft = EventDraft::new(
            calendar.id,
            "meeting",
            Schedule::Instant(Interval::new(Instant(0), Instant(100)).unwrap()),
        );
        let event = store.create(draft).unwrap();
        store.events.get_mut(&event.id).unwrap().revision = EventRevision::new(u64::MAX).unwrap();
        let before = store.get(&event.id).unwrap();
        assert_eq!(
            store.update(&event.id, before.revision, before.draft.clone()),
            Err(Error::Overflow)
        );
        assert_eq!(store.get(&event.id).unwrap(), before);
    }
}
