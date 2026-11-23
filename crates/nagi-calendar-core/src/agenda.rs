//! Reusable agenda rows over an existing Calendar event listing.
//!
//! Rows preserve the supplied list order and expose the canonical `Schedule`
//! unchanged, including its instant, timezone, floating-time, or all-day form.
//! A UI can format that schedule for its display timezone without this model
//! inventing a second time representation.

use crate::model::{Event, EventId, EventRevision, Schedule};

#[derive(Clone, Copy, Debug)]
pub struct AgendaRow<'event> {
    event: &'event Event,
}

impl<'event> AgendaRow<'event> {
    pub fn event_id(&self) -> &'event EventId {
        &self.event.id
    }

    pub fn title(&self) -> &'event str {
        &self.event.draft.title
    }

    /// The event's canonical start/end representation, without conversion.
    pub fn schedule(&self) -> &'event Schedule {
        &self.event.draft.schedule
    }

    pub fn revision(&self) -> EventRevision {
        self.event.revision
    }

    pub fn event(&self) -> &'event Event {
        self.event
    }
}

/// A lightweight agenda view model over results returned by `LocalAdapter::list`.
///
/// The model borrows its events and stores only a selected ID. It does not sort,
/// persist, authorize, or resolve a schedule. The selected ID can be passed to
/// `LocalAdapter::get_event` to retrieve the current stored event.
pub struct AgendaViewModel<'events> {
    events: &'events [Event],
    selected_event_id: Option<EventId>,
}

impl<'events> AgendaViewModel<'events> {
    pub fn new(events: &'events [Event]) -> Self {
        Self {
            events,
            selected_event_id: None,
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = AgendaRow<'events>> + '_ {
        self.events.iter().map(|event| AgendaRow { event })
    }

    pub fn selected_event_id(&self) -> Option<&EventId> {
        self.selected_event_id.as_ref()
    }

    pub fn is_selected(&self, event_id: &EventId) -> bool {
        self.selected_event_id.as_ref() == Some(event_id)
    }

    /// Select an event represented by this agenda's source listing.
    pub fn select_event(&mut self, event_id: &EventId) -> bool {
        if self.events.iter().any(|event| &event.id == event_id) {
            self.selected_event_id = Some(event_id.clone());
            true
        } else {
            false
        }
    }
}
