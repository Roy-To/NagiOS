//! Calendar snapshot transactions for one owner-issued object.

use nagi_calendar_core::adapter::LocalAdapter;
use nagi_calendar_core::model::{CalendarId, Event, EventDraft, EventId, EventRevision};
use nagi_calendar_core::snapshot::StoreSnapshot;
use nagi_calendar_core::store::{InMemoryStore, SequentialIds};
use nagi_calendar_core::time::RealtimeSource;
use nagi_calendar_core::{Error, ObjectId};
use nagi_persistence_contract::{ConditionalWriteOutcome, Revision, Snapshot, SnapshotStorage};

const CAPACITY: usize = 1_000;

/// The same caller-issued Calendar handle must supply every operation.
pub(crate) trait CalendarSnapshotScope {
    type ScopeError;
    type StorageError;

    fn claim_initialization(&mut self) -> Result<bool, Self::ScopeError>;

    fn with_storage<R>(
        &mut self,
        operation: impl FnOnce(ObjectId, &mut dyn SnapshotStorage<Error = Self::StorageError>) -> R,
    ) -> Result<R, Self::ScopeError>;
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum LoadRejected {
    Io,
    Missing,
    Invalid(Error),
    UnverifiedInitialization,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SaveRejected {
    Invalid(Error),
    NotApplied,
    Conflict,
    StateUnknown,
}

pub(crate) enum CalendarEdit {
    Create(EventDraft),
    Update(EventId, EventRevision, EventDraft),
    Delete(EventId, EventRevision),
}

pub(crate) struct CalendarSession<I, C: RealtimeSource> {
    io: I,
    adapter: LocalAdapter<InMemoryStore<C, SequentialIds>>,
    calendar: CalendarId,
    object: ObjectId,
    revision: Revision,
    write_blocked: bool,
}

impl<I: CalendarSnapshotScope, C: RealtimeSource> CalendarSession<I, C> {
    /// Missing data may only be initialized after an exclusive owner claim.
    pub(crate) fn open(mut io: I, clock: C) -> Result<Self, LoadRejected> {
        let mut read = || {
            io.with_storage(|object, storage| storage.read(object).map(|saved| (object, saved)))
                .map_err(|_| LoadRejected::Io)?
                .map_err(|_| LoadRejected::Io)
        };
        let (object, saved) = read().or_else(|_| read())?;
        let (store, revision) = match saved {
            Some(saved) => {
                let snapshot =
                    StoreSnapshot::from_bytes(&saved.bytes).map_err(LoadRejected::Invalid)?;
                let ids = SequentialIds::after_snapshot(&snapshot);
                let mut store =
                    InMemoryStore::new(clock, ids, CAPACITY).map_err(LoadRejected::Invalid)?;
                store
                    .restore_snapshot(snapshot)
                    .map_err(LoadRejected::Invalid)?;
                (store, saved.revision)
            }
            None => {
                let mut store = InMemoryStore::new(clock, SequentialIds::new(), CAPACITY)
                    .map_err(LoadRejected::Invalid)?;
                store
                    .create_calendar("Calendar", None)
                    .map_err(LoadRejected::Invalid)?;
                let bytes = store.snapshot().to_bytes().map_err(LoadRejected::Invalid)?;
                if !io
                    .claim_initialization()
                    .map_err(|_| LoadRejected::UnverifiedInitialization)?
                {
                    return Err(LoadRejected::Missing);
                }
                let revision = write_verified(&mut io, object, None, &bytes)
                    .map_err(|_| LoadRejected::UnverifiedInitialization)?;
                (store, revision)
            }
        };
        let calendar = store
            .list_calendars(CAPACITY)
            .map_err(LoadRejected::Invalid)?
            .into_iter()
            .next()
            .ok_or(LoadRejected::Invalid(Error::InvalidSnapshot))?
            .id;
        Ok(Self {
            io,
            adapter: LocalAdapter { store },
            calendar,
            object,
            revision,
            write_blocked: false,
        })
    }

    pub(crate) fn calendar_id(&self) -> &CalendarId {
        &self.calendar
    }

    pub(crate) fn events(&self) -> nagi_calendar_core::Result<Vec<Event>> {
        self.adapter.list(&self.calendar, CAPACITY)
    }

    pub(crate) fn snapshot_bytes(&self) -> nagi_calendar_core::Result<Vec<u8>> {
        self.adapter.store.snapshot().to_bytes()
    }

    pub(crate) fn commit(&mut self, edit: CalendarEdit) -> Result<Event, SaveRejected> {
        if self.write_blocked {
            return Err(SaveRejected::StateUnknown);
        }
        let before = self.adapter.store.snapshot();
        let event = match edit {
            CalendarEdit::Create(draft) if draft.calendar_id == self.calendar => {
                self.adapter.create_event(draft)
            }
            CalendarEdit::Update(id, revision, draft) if draft.calendar_id == self.calendar => self
                .require_visible(&id)
                .and_then(|()| self.adapter.update_event(&id, revision, draft)),
            CalendarEdit::Delete(id, revision) => self
                .require_visible(&id)
                .and_then(|()| self.adapter.delete_event(&id, revision)),
            _ => Err(Error::NotFound),
        }
        .map_err(SaveRejected::Invalid)?;

        let result = self.persist();
        if let Err(error) = result {
            if self.adapter.store.restore_snapshot(before).is_err() {
                self.write_blocked = true;
                return Err(SaveRejected::StateUnknown);
            }
            return Err(error);
        }
        Ok(event)
    }

    fn require_visible(&self, id: &EventId) -> nagi_calendar_core::Result<()> {
        if self.adapter.get_event(id)?.draft.calendar_id == self.calendar {
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    }

    fn persist(&mut self) -> Result<(), SaveRejected> {
        let next = self
            .adapter
            .store
            .snapshot()
            .to_bytes()
            .map_err(SaveRejected::Invalid)?;
        match write_verified(&mut self.io, self.object, Some(self.revision), &next) {
            Ok(revision) => {
                self.revision = revision;
                Ok(())
            }
            Err(error) => {
                self.write_blocked =
                    matches!(error, SaveRejected::Conflict | SaveRejected::StateUnknown);
                Err(error)
            }
        }
    }
}

fn write_verified(
    io: &mut impl CalendarSnapshotScope,
    object: ObjectId,
    expected: Option<Revision>,
    bytes: &[u8],
) -> Result<Revision, SaveRejected> {
    io.with_storage(|issued, storage| {
        if issued != object {
            return Err(SaveRejected::StateUnknown);
        }
        match storage.compare_and_replace(issued, expected, bytes) {
            ConditionalWriteOutcome::Applied { revision } => match storage.read(issued) {
                Ok(Some(Snapshot {
                    revision: observed,
                    bytes: readback,
                })) if Some(revision) != expected && observed == revision && readback == bytes => {
                    Ok(revision)
                }
                _ => Err(SaveRejected::StateUnknown),
            },
            ConditionalWriteOutcome::Conflict { .. } => Err(SaveRejected::Conflict),
            ConditionalWriteOutcome::NotApplied { .. } => Err(SaveRejected::NotApplied),
            ConditionalWriteOutcome::OutcomeUnknown { .. } => {
                // Observe within this effect, but a read cannot establish that an
                // uncertain publication survived a failed durability operation.
                let _ = storage.read(issued);
                Err(SaveRejected::StateUnknown)
            }
        }
    })
    .unwrap_or(Err(SaveRejected::StateUnknown))
}

#[cfg(test)]
mod tests;
