use alloc::{collections::BTreeSet, rc::Rc, vec::Vec};
use core::cell::RefCell;

use nagi_model::{AppId, ObjectId};
use nagi_search::{
    AccessContext, MetadataRecord, SearchService, SnapshotBackend, VisibilityFilter, Workspace,
    WorkspaceSession,
};

use crate::{
    ledger::{record_fingerprint, Ledger, LedgerEntry, MAX_LEDGER_ENTRIES},
    model::{
        DocumentRevision, IngestOutcome, ProviderAuthority, ProviderHit, ProviderScope,
        RemoveOutcome, ScopedQuery, ScopedSearchResponse, SearchCoreError, MAX_PROVIDERS,
    },
};

type ScopeCell = Rc<RefCell<Option<BTreeSet<AppId>>>>;

/// Wraps the caller's canonical `VisibilityFilter` with the provider scope of
/// the operation in progress. Outside a scoped operation the scope is `None`
/// and every object is denied, so the wrapped service fails closed.
struct ScopedVisibility<V> {
    inner: V,
    scope: ScopeCell,
}

impl<V: VisibilityFilter> VisibilityFilter for ScopedVisibility<V> {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        let in_scope = match (&*self.scope.borrow(), record.source_app) {
            (Some(scope), Some(provider)) => scope.contains(&provider),
            _ => false,
        };
        // Scope is checked first; the caller policy still decides visibility.
        in_scope && self.inner.can_read_object(access, record)
    }

    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool {
        self.inner.can_read_workspace(access, workspace)
    }

    fn can_read_workspace_session(
        &self,
        access: AccessContext,
        workspace: &Workspace,
        session: WorkspaceSession,
    ) -> bool {
        self.inner
            .can_read_workspace_session(access, workspace, session)
    }
}

/// Clears the active scope even on early return.
struct ScopeGuard<'a>(&'a ScopeCell);

impl<'a> ScopeGuard<'a> {
    fn enter(cell: &'a ScopeCell, scope: BTreeSet<AppId>) -> Self {
        *cell.borrow_mut() = Some(scope);
        Self(cell)
    }
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        *self.0.borrow_mut() = None;
    }
}

/// Provider-facing Search/Index core over the canonical `nagi_search`
/// metadata index.
///
/// `provider` arguments are trusted identities supplied by the integration
/// layer (in production: the capability-checked caller of a `search.provider`
/// channel). Payload fields such as `MetadataRecord::source_app` are never
/// treated as proof of identity.
pub struct SearchCore<MB, LB, V, A> {
    service: SearchService<MB, ScopedVisibility<V>>,
    scope: ScopeCell,
    ledger: Ledger,
    ledger_backend: LB,
    ledger_dirty: bool,
    authority: A,
}

impl<MB, LB, V, A> SearchCore<MB, LB, V, A>
where
    MB: SnapshotBackend,
    LB: SnapshotBackend,
    V: VisibilityFilter,
    A: ProviderAuthority,
{
    pub fn open(
        metadata_backend: MB,
        mut ledger_backend: LB,
        visibility: V,
        authority: A,
    ) -> Result<Self, SearchCoreError> {
        let ledger = match ledger_backend
            .load_snapshot()
            .map_err(SearchCoreError::LedgerLoad)?
        {
            Some(bytes) => Ledger::decode(&bytes).map_err(SearchCoreError::Ledger)?,
            None => Ledger::default(),
        };
        let scope: ScopeCell = Rc::new(RefCell::new(None));
        let service = SearchService::open(
            metadata_backend,
            ScopedVisibility {
                inner: visibility,
                scope: Rc::clone(&scope),
            },
        )
        .map_err(SearchCoreError::Index)?;
        Ok(Self {
            service,
            scope,
            ledger,
            ledger_backend,
            ledger_dirty: false,
            authority,
        })
    }

    pub fn registered_providers(&self) -> impl Iterator<Item = AppId> + '_ {
        self.ledger.providers.iter().copied()
    }

    pub fn register_provider(&mut self, provider: AppId) -> Result<(), SearchCoreError> {
        self.authorize(provider)?;
        if self.ledger.providers.contains(&provider) {
            return Err(SearchCoreError::ProviderAlreadyRegistered);
        }
        if self.ledger.providers.len() >= MAX_PROVIDERS {
            return Err(SearchCoreError::ProviderCapacity);
        }
        self.ledger.providers.insert(provider);
        self.ledger_dirty = true;
        self.persist_ledger()
    }

    /// Removes the provider and tombstones every live object it owns, so its
    /// content stops being discoverable immediately. Ledger entries remain as
    /// tombstones so stale replays stay rejected if the provider returns.
    /// Unregistration is a containment path and is not gated on the provider
    /// still holding publish authority.
    pub fn unregister_provider(
        &mut self,
        provider: AppId,
        removed_at: i64,
    ) -> Result<usize, SearchCoreError> {
        if !self.ledger.providers.contains(&provider) {
            return Err(SearchCoreError::UnknownProvider);
        }
        let owned: Vec<ObjectId> = self
            .ledger
            .entries
            .iter()
            .filter(|(_, entry)| entry.provider == provider && !entry.removed)
            .map(|(id, _)| *id)
            .collect();
        let mut removed = 0;
        for object_id in owned {
            self.service
                .remove_record(object_id, removed_at)
                .map_err(SearchCoreError::Index)?;
            if let Some(entry) = self.ledger.entries.get_mut(&object_id) {
                entry.removed = true;
            }
            self.ledger_dirty = true;
            removed += 1;
        }
        self.ledger.providers.remove(&provider);
        self.ledger_dirty = true;
        self.persist_ledger()?;
        Ok(removed)
    }

    /// Insert or replace one object owned by `provider` at `revision`.
    pub fn upsert(
        &mut self,
        provider: AppId,
        revision: DocumentRevision,
        mut record: MetadataRecord,
    ) -> Result<IngestOutcome, SearchCoreError> {
        self.check_provider(provider)?;
        match record.source_app {
            Some(claimed) if claimed != provider => return Err(SearchCoreError::SourceMismatch),
            _ => record.source_app = Some(provider),
        }
        if record.tombstoned_at.is_some() {
            return Err(SearchCoreError::TombstonedInput);
        }
        record.validate().map_err(SearchCoreError::InvalidRecord)?;
        let fingerprint = record_fingerprint(&record);
        let object_id = record.object_id;

        match self.ledger.entries.get(&object_id) {
            Some(entry) if entry.provider != provider => {
                return Err(SearchCoreError::ObjectOwnedByOtherProvider)
            }
            Some(entry) if revision < entry.revision => return Err(SearchCoreError::StaleRevision),
            Some(entry) if revision == entry.revision => {
                if !entry.removed && entry.fingerprint == fingerprint {
                    self.persist_ledger()?;
                    return Ok(IngestOutcome::Unchanged);
                }
                return Err(SearchCoreError::RevisionConflict);
            }
            Some(_) => {}
            None if self.ledger.entries.len() >= MAX_LEDGER_ENTRIES => {
                return Err(SearchCoreError::LedgerCapacity)
            }
            None => {}
        }

        // The canonical index commits first. If the ledger cannot then be
        // persisted, the error says so truthfully and a retry is idempotent.
        self.service
            .upsert_record(record)
            .map_err(SearchCoreError::Index)?;
        self.ledger.entries.insert(
            object_id,
            LedgerEntry {
                provider,
                revision,
                removed: false,
                fingerprint,
            },
        );
        self.ledger_dirty = true;
        self.persist_ledger()?;
        Ok(IngestOutcome::Applied)
    }

    /// Remove one object owned by `provider`. Removal is itself revisioned so
    /// a delayed older upsert cannot resurrect deleted content.
    pub fn remove(
        &mut self,
        provider: AppId,
        object_id: ObjectId,
        revision: DocumentRevision,
        removed_at: i64,
    ) -> Result<RemoveOutcome, SearchCoreError> {
        self.check_provider(provider)?;
        let entry = match self.ledger.entries.get(&object_id) {
            Some(entry) if entry.provider == provider => *entry,
            // Unknown and foreign objects are indistinguishable here.
            _ => return Err(SearchCoreError::UnknownObject),
        };
        if revision < entry.revision {
            return Err(SearchCoreError::StaleRevision);
        }
        if revision == entry.revision {
            if entry.removed {
                self.persist_ledger()?;
                return Ok(RemoveOutcome::AlreadyRemoved);
            }
            return Err(SearchCoreError::RevisionConflict);
        }
        // Removing an already removed object at a newer revision only advances
        // the tombstone revision; the canonical index is not touched again.
        if !entry.removed {
            self.service
                .remove_record(object_id, removed_at)
                .map_err(SearchCoreError::Index)?;
        }
        self.ledger.entries.insert(
            object_id,
            LedgerEntry {
                revision,
                removed: true,
                ..entry
            },
        );
        self.ledger_dirty = true;
        self.persist_ledger()?;
        Ok(RemoveOutcome::Removed)
    }

    /// The applied revision of an object, answered only to its owner. Unknown
    /// and foreign objects both return `None`.
    pub fn revision(&self, provider: AppId, object_id: ObjectId) -> Option<DocumentRevision> {
        self.ledger
            .entries
            .get(&object_id)
            .filter(|entry| entry.provider == provider)
            .map(|entry| entry.revision)
    }

    pub fn ledger_dirty(&self) -> bool {
        self.ledger_dirty
    }

    /// Persist a pending ledger snapshot. Succeeds without writing when clean.
    pub fn persist_ledger(&mut self) -> Result<(), SearchCoreError> {
        if !self.ledger_dirty {
            return Ok(());
        }
        let bytes = self.ledger.encode();
        self.ledger_backend
            .write_snapshot(&bytes)
            .map_err(SearchCoreError::LedgerPersist)?;
        self.ledger_dirty = false;
        Ok(())
    }

    /// Caller-visible, provider-scoped search. Provider scope and caller
    /// visibility are both applied by the canonical service before field
    /// matching, ranking, and truncation, so excluded objects cannot affect
    /// returned hits, rationale, Workspace groups, or counts.
    pub fn search(
        &self,
        access: AccessContext,
        query: &ScopedQuery,
    ) -> Result<ScopedSearchResponse, SearchCoreError> {
        let scope = self.resolve_scope(&query.providers)?;
        let response = {
            let _guard = ScopeGuard::enter(&self.scope, scope);
            self.service
                .search(access, &query.query)
                .map_err(SearchCoreError::Search)?
        };
        let mut objects = Vec::with_capacity(response.objects.len());
        for hit in response.objects {
            let entry = self
                .ledger
                .entries
                .get(&hit.record.object_id)
                .filter(|entry| !entry.removed && Some(entry.provider) == hit.record.source_app);
            // Every in-scope record was ingested through this core; a record
            // without a matching live ledger entry is withheld, not guessed.
            if let Some(entry) = entry {
                objects.push(ProviderHit {
                    provider: entry.provider,
                    revision: entry.revision,
                    hit,
                });
            }
        }
        let returned: BTreeSet<ObjectId> =
            objects.iter().map(|hit| hit.hit.record.object_id).collect();
        let mut workspace_groups = response.workspace_groups;
        for group in &mut workspace_groups {
            group.object_ids.retain(|id| returned.contains(id));
        }
        workspace_groups.retain(|group| !group.object_ids.is_empty());
        let workspaces = match query.providers {
            ProviderScope::All => response.workspaces,
            ProviderScope::Only(_) => Vec::new(),
        };
        Ok(ScopedSearchResponse {
            objects,
            workspaces,
            workspace_groups,
        })
    }

    /// Caller-visible, provider-scoped object lookup. Unknown, removed,
    /// out-of-scope, and unauthorized objects are indistinguishable (`None`).
    pub fn get(
        &self,
        access: AccessContext,
        scope: &ProviderScope,
        object_id: ObjectId,
    ) -> Result<Option<ProviderHit>, SearchCoreError> {
        let resolved = self.resolve_scope(scope)?;
        let record = {
            let _guard = ScopeGuard::enter(&self.scope, resolved);
            self.service.get_object(access, object_id)
        };
        let Some(record) = record else {
            return Ok(None);
        };
        Ok(self
            .ledger
            .entries
            .get(&object_id)
            .filter(|entry| !entry.removed && Some(entry.provider) == record.source_app)
            .map(|entry| ProviderHit {
                provider: entry.provider,
                revision: entry.revision,
                hit: nagi_search::SearchHit {
                    record,
                    rationale: Vec::new(),
                },
            }))
    }

    fn resolve_scope(&self, scope: &ProviderScope) -> Result<BTreeSet<AppId>, SearchCoreError> {
        match scope {
            ProviderScope::All => Ok(self.ledger.providers.clone()),
            ProviderScope::Only(providers) if providers.is_empty() => {
                Err(SearchCoreError::EmptyProviderScope)
            }
            ProviderScope::Only(providers) if providers.len() > MAX_PROVIDERS => {
                Err(SearchCoreError::ProviderScopeTooLarge)
            }
            ProviderScope::Only(providers) => Ok(providers
                .intersection(&self.ledger.providers)
                .copied()
                .collect()),
        }
    }

    fn authorize(&self, provider: AppId) -> Result<(), SearchCoreError> {
        if self.authority.may_publish(provider) {
            Ok(())
        } else {
            Err(SearchCoreError::ProviderNotAuthorized)
        }
    }

    fn check_provider(&self, provider: AppId) -> Result<(), SearchCoreError> {
        self.authorize(provider)?;
        if !self.ledger.providers.contains(&provider) {
            return Err(SearchCoreError::UnknownProvider);
        }
        Ok(())
    }
}
