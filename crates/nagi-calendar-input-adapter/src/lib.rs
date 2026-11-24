//! Calendar keyboard navigation over the existing agenda model.
//!
//! This adapter consumes Nagi UI's backend-neutral `KeyCode`; opening an event
//! returns its existing `EventId` so the caller can use `LocalAdapter::get_event`.

use nagi_calendar_core::adapter::LocalAdapter;
use nagi_calendar_core::agenda::AgendaViewModel;
use nagi_calendar_core::model::{Event, EventDraft, EventId};
use nagi_calendar_core::store::EventStore;
use nagi_calendar_core::Result;
use nagi_ui::interaction::KeyCode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgendaKeyboardAction {
    SelectionChanged(EventId),
    Open(EventId),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AgendaKeyboardAdapter;

impl AgendaKeyboardAdapter {
    /// Start keyboard navigation at the first agenda row when nothing is selected.
    pub fn new(agenda: &mut AgendaViewModel<'_>) -> Self {
        if agenda.selected_event_id().is_none() {
            let first_id = { agenda.rows().next().map(|row| row.event_id().clone()) };
            if let Some(first_id) = first_id {
                agenda.select_event(&first_id);
            }
        }
        Self
    }

    pub fn handle_key(
        &self,
        key: KeyCode,
        agenda: &mut AgendaViewModel<'_>,
    ) -> Option<AgendaKeyboardAction> {
        match key {
            KeyCode::ArrowDown => move_selection(agenda, true),
            KeyCode::ArrowUp => move_selection(agenda, false),
            KeyCode::Enter => agenda
                .selected_event_id()
                .cloned()
                .map(AgendaKeyboardAction::Open),
            _ => None,
        }
    }
}

/// Edit controller for an event opened from the agenda.
///
/// Opening resolves the selected canonical ID through `LocalAdapter::get_event`.
/// Saving uses that event's existing revision through `update_event`; the cached
/// event advances only after the store accepts the update. The caller edits the
/// canonical `EventDraft` and may rebuild the agenda from `LocalAdapter::list`.
pub struct AgendaEditController {
    event: Event,
}

/// Delete controller for an event opened from an agenda row.
///
/// It retains the canonical Event revision and submits that revision to the
/// existing `delete_event` operation, so a later edit cannot be silently
/// deleted using stale screen state.
pub struct AgendaDeleteController {
    event: Event,
}

/// Composer controller for a new event, keeping the canonical draft as form state.
///
/// The UI can edit the existing `EventDraft` in place. Creation returns the
/// canonical `Event`, whose ID can be selected after rebuilding the agenda from
/// `LocalAdapter::list`.
pub struct AgendaCreateController {
    draft: EventDraft,
}

impl AgendaCreateController {
    pub fn new(draft: EventDraft) -> Self {
        Self { draft }
    }

    pub fn draft(&self) -> &EventDraft {
        &self.draft
    }

    pub fn draft_mut(&mut self) -> &mut EventDraft {
        &mut self.draft
    }

    pub fn create<S: EventStore>(self, adapter: &mut LocalAdapter<S>) -> Result<Event> {
        adapter.create_event(self.draft)
    }
}

impl AgendaEditController {
    pub fn open<S: EventStore>(adapter: &LocalAdapter<S>, event_id: &EventId) -> Result<Self> {
        Ok(Self {
            event: adapter.get_event(event_id)?,
        })
    }

    pub fn event(&self) -> &Event {
        &self.event
    }

    /// Close the edit form without writing and return the canonical event used
    /// to rebuild the agenda after the caller discards its local draft.
    pub fn cancel(self) -> Event {
        self.event
    }

    pub fn save<S: EventStore>(
        &mut self,
        adapter: &mut LocalAdapter<S>,
        draft: EventDraft,
    ) -> Result<&Event> {
        self.event = adapter.update_event(&self.event.id, self.event.revision, draft)?;
        Ok(&self.event)
    }
}

impl AgendaDeleteController {
    pub fn open<S: EventStore>(adapter: &LocalAdapter<S>, event_id: &EventId) -> Result<Self> {
        Ok(Self {
            event: adapter.get_event(event_id)?,
        })
    }

    pub fn event(&self) -> &Event {
        &self.event
    }

    pub fn delete<S: EventStore>(self, adapter: &mut LocalAdapter<S>) -> Result<Event> {
        adapter.delete_event(&self.event.id, self.event.revision)
    }
}

fn move_selection(agenda: &mut AgendaViewModel<'_>, forward: bool) -> Option<AgendaKeyboardAction> {
    let row_count = agenda.rows().count();
    if row_count == 0 {
        return None;
    }

    let current = agenda
        .selected_event_id()
        .and_then(|selected| agenda.rows().position(|row| row.event_id() == selected))
        .unwrap_or(0);
    let next = if forward {
        (current + 1) % row_count
    } else if current == 0 {
        row_count - 1
    } else {
        current - 1
    };
    let event_id = { agenda.rows().nth(next).map(|row| row.event_id().clone()) }?;
    agenda
        .select_event(&event_id)
        .then_some(AgendaKeyboardAction::SelectionChanged(event_id))
}
