use alloc::vec::Vec;

use crate::model::{ActivityEvent, ACTIVITY_EVENT_SCHEMA_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    DuplicateEventId,
    UnsupportedSchemaVersion,
    InvalidContext,
}

/// Trusted persistence boundary. Application-facing access should go through
/// the ledger query API and its access policy hook, not expose this trait.
pub trait ActivityStore {
    fn append(&mut self, event: ActivityEvent) -> Result<(), StoreError>;
    fn read_event(&self, event_id: crate::EventId) -> Option<ActivityEvent>;
    fn all_events(&self) -> Vec<ActivityEvent>;
}

/// Optional retention extension for policy-driven physical erasure.
pub trait RetentionStore: ActivityStore {
    fn erase_event(&mut self, event_id: crate::EventId) -> Result<bool, StoreError>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InMemoryActivityStore {
    events: Vec<ActivityEvent>,
}

impl InMemoryActivityStore {
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl ActivityStore for InMemoryActivityStore {
    fn append(&mut self, event: ActivityEvent) -> Result<(), StoreError> {
        if event.schema_version != ACTIVITY_EVENT_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchemaVersion);
        }
        if !event.context.is_valid() {
            return Err(StoreError::InvalidContext);
        }
        if self
            .events
            .iter()
            .any(|stored| stored.event_id == event.event_id)
        {
            return Err(StoreError::DuplicateEventId);
        }
        self.events.push(event);
        Ok(())
    }

    fn read_event(&self, event_id: crate::EventId) -> Option<ActivityEvent> {
        self.events
            .iter()
            .find(|event| event.event_id == event_id)
            .cloned()
    }

    fn all_events(&self) -> Vec<ActivityEvent> {
        self.events.clone()
    }
}

impl RetentionStore for InMemoryActivityStore {
    fn erase_event(&mut self, event_id: crate::EventId) -> Result<bool, StoreError> {
        let Some(index) = self
            .events
            .iter()
            .position(|event| event.event_id == event_id)
        else {
            return Ok(false);
        };
        self.events.remove(index);
        Ok(true)
    }
}
