//! Proposed v1 host Action seam. No Action registry, policy engine or runtime wiring.
use crate::model::{CalendarId, DraftState, Event, EventDraft, EventId, EventRevision, Reminder};
use crate::store::EventStore;
use crate::ObjectId;
use crate::{Error, Result};

pub const CONTRACT_VERSION: u16 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    List,
    Search,
    GetEvent,
    CreateEvent,
    UpdateEvent,
    DeleteEvent,
    CreateEventDraft,
    FindAvailability,
    AddReminder,
    AttachResource,
    Invite,
    RespondInvite,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    ReadOnly,
    LocalMutation,
    External,
}
impl Action {
    pub fn id(self) -> &'static str {
        match self {
            Self::List => "calendar.list",
            Self::Search => "calendar.search",
            Self::GetEvent => "calendar.get_event",
            Self::CreateEvent => "calendar.create_event",
            Self::UpdateEvent => "calendar.update_event",
            Self::DeleteEvent => "calendar.delete_event",
            Self::CreateEventDraft => "calendar.create_event_draft",
            Self::FindAvailability => "calendar.find_availability",
            Self::AddReminder => "calendar.add_reminder",
            Self::AttachResource => "calendar.attach_resource",
            Self::Invite => "calendar.invite",
            Self::RespondInvite => "calendar.respond_invite",
        }
    }
    pub fn effect(self) -> Effect {
        match self {
            Self::Invite | Self::RespondInvite => Effect::External,
            Self::List | Self::Search | Self::GetEvent | Self::FindAvailability => Effect::ReadOnly,
            _ => Effect::LocalMutation,
        }
    }
    pub fn requires_confirmation(self) -> bool {
        self.effect() == Effect::External
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalDraft {
    pub event_id: EventId,
    pub revision: EventRevision,
    pub action: Action,
    pub state: DraftState,
}
impl ExternalDraft {
    pub fn new(event: &Event, action: Action) -> Result<Self> {
        if action.effect() != Effect::External {
            return Err(Error::InvalidRange);
        }
        event.draft.validate()?;
        Ok(Self {
            event_id: event.id.clone(),
            revision: event.revision,
            action,
            state: DraftState::RequiresConfirmation,
        })
    }
    /// No sender exists here. Confirmation never means successful delivery.
    pub fn execute(&self, _confirmed: bool) -> Result<()> {
        Err(Error::ExternalActionUnavailable)
    }
}
/// Local store wrapper; access control belongs to the future authenticated adapter.
pub struct LocalAdapter<S: EventStore> {
    pub store: S,
}
impl<S: EventStore> LocalAdapter<S> {
    pub fn list(&self, calendar: &CalendarId, limit: usize) -> Result<Vec<Event>> {
        self.store.list(calendar, limit)
    }
    pub fn search(&self, calendar: &CalendarId, text: &str, limit: usize) -> Result<Vec<Event>> {
        self.store.search(calendar, text, limit)
    }
    pub fn get_event(&self, id: &EventId) -> Result<Event> {
        self.store.get(id)
    }
    pub fn create_event(&mut self, draft: EventDraft) -> Result<Event> {
        self.store.create(draft)
    }
    pub fn update_event(
        &mut self,
        id: &EventId,
        expected: EventRevision,
        draft: EventDraft,
    ) -> Result<Event> {
        self.store.update(id, expected, draft)
    }
    pub fn delete_event(&mut self, id: &EventId, expected: EventRevision) -> Result<Event> {
        self.store.delete(id, expected)
    }
    pub fn create_event_draft(&self, mut draft: EventDraft) -> Result<EventDraft> {
        draft.validate()?;
        draft.state = DraftState::Draft;
        Ok(draft)
    }
    pub fn find_availability(
        &self,
        calendar: &CalendarId,
        request: &crate::availability::AvailabilityRequest,
        resolver: &dyn crate::time::TimeZoneResolver,
        context: Option<&crate::model::ZoneContext>,
        limits: crate::recurrence::ExpansionLimits,
    ) -> Result<Vec<crate::model::Interval>> {
        let events = self.store.list(calendar, 4096)?;
        let drafts = events
            .into_iter()
            .map(|event| event.draft)
            .collect::<Vec<_>>();
        let busy = crate::availability::busy_from_events(
            &drafts,
            request.window,
            resolver,
            context,
            limits,
        )?;
        crate::availability::find_availability(request, &busy)
    }
    pub fn add_reminder(
        &mut self,
        id: &EventId,
        expected: EventRevision,
        reminder: Reminder,
    ) -> Result<Event> {
        let mut draft = self.store.get(id)?.draft;
        draft.reminders.push(reminder);
        self.store.update(id, expected, draft)
    }
    pub fn attach_resource(
        &mut self,
        id: &EventId,
        expected: EventRevision,
        resource: ObjectId,
    ) -> Result<Event> {
        let mut draft = self.store.get(id)?.draft;
        if !draft.resources.contains(&resource) {
            draft.resources.push(resource);
        }
        self.store.update(id, expected, draft)
    }
}
