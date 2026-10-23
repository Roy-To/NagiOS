//! Bounded host-only Calendar metadata projection into canonical SearchCore.
//! Trusted integration supplies provider identity and an existing stable object
//! mapping. Read policy and publish authority remain owned by SearchCore.
use nagi_calendar_core::model::{Event, EventId, EventRevision};
use nagi_search_core::{
    nagi_model::{AppId, ObjectId},
    nagi_search::{MetadataRecord, ObjectKind, SnapshotBackend, VisibilityFilter},
    DocumentRevision, IngestOutcome, ProviderAuthority, RemoveOutcome, SearchCore, SearchCoreError,
};

/// Trusted integration mapping; it must be stable and injective for the provider,
/// including deleted events, so delayed replays retain the same object identity.
/// This adapter never allocates IDs or treats payload resource IDs as identity.
pub trait EventObjects {
    fn object_id(&self, event: &EventId) -> Option<ObjectId>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    UnmappedEvent,
    SourceMismatch,
    Calendar(nagi_calendar_core::Error),
    Search(SearchCoreError),
}

pub struct CalendarSearchAdapter<R> {
    provider: AppId,
    objects: R,
}

impl<R: EventObjects> CalendarSearchAdapter<R> {
    /// `provider` is authenticated integration context, never payload metadata.
    /// Runtime provider registration is deliberately outside this adapter.
    pub fn new(provider: AppId, objects: R) -> Self {
        Self { provider, objects }
    }

    /// Publish title, participant labels (canonical tags), and location only.
    /// The canonical record defaults to Private. VisibilityFilter remains the
    /// sole disclosure authority; Calendar owner metadata cannot grant access.
    /// Validation is atomic with respect to the index: no truncation or partial
    /// projection is accepted. SearchCore errors retain their durability meaning.
    pub fn publish<MB, LB, V, A>(
        &self,
        core: &mut SearchCore<MB, LB, V, A>,
        event: &Event,
    ) -> Result<IngestOutcome, Error>
    where
        MB: SnapshotBackend,
        LB: SnapshotBackend,
        V: VisibilityFilter,
        A: ProviderAuthority,
    {
        if event
            .draft
            .owner
            .as_ref()
            .is_some_and(|owner| owner.app != self.provider)
        {
            return Err(Error::SourceMismatch);
        }
        event.draft.validate().map_err(Error::Calendar)?;
        let object = self
            .objects
            .object_id(&event.id)
            .ok_or(Error::UnmappedEvent)?;
        let mut record =
            MetadataRecord::new(object, ObjectKind::ApplicationData, &event.draft.title);
        record.source_app = Some(self.provider);
        if !event.draft.location.is_empty() {
            record.location = Some(event.draft.location.clone());
        }
        record.tags = event.draft.participants.clone();
        core.upsert(self.provider, revision(event.revision), record)
            .map_err(Error::Search)
    }

    /// `removed_at` uses the canonical Search timestamp convention. Integration
    /// supplies it explicitly rather than silently converting Calendar units.
    pub fn remove<MB, LB, V, A>(
        &self,
        core: &mut SearchCore<MB, LB, V, A>,
        event: &EventId,
        event_revision: EventRevision,
        removed_at: i64,
    ) -> Result<RemoveOutcome, Error>
    where
        MB: SnapshotBackend,
        LB: SnapshotBackend,
        V: VisibilityFilter,
        A: ProviderAuthority,
    {
        let object = self.objects.object_id(event).ok_or(Error::UnmappedEvent)?;
        core.remove(self.provider, object, revision(event_revision), removed_at)
            .map_err(Error::Search)
    }
}

fn revision(value: EventRevision) -> DocumentRevision {
    // Calendar's checked revision type has the identical nonzero invariant.
    DocumentRevision::new(value.get()).expect("Calendar revision is nonzero")
}
