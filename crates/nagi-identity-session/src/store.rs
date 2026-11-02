use std::sync::{Arc, RwLock};

use crate::model::IdentitySnapshot;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SnapshotVersion(u64);

impl SnapshotVersion {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSnapshot {
    pub bytes: Vec<u8>,
    pub version: SnapshotVersion,
}

/// One atomic observation of the rollbackable current payload and its
/// independent, monotonically increasing store CAS revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotStoreState {
    pub current: Option<StoredSnapshot>,
    /// Highest version ever committed. This must survive current-payload loss
    /// and restoration from last-known-good data; it is not derived from either
    /// snapshot's embedded version.
    pub high_water_version: Option<SnapshotVersion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStoreError {
    Unavailable,
    Conflict,
}

/// A fresh read of the same committed current store, including mutations by
/// other service instances. It must not use a stale cache or last-known-good
/// snapshot. Readers are observations, not grants or transactions.
pub type CurrentSnapshotReader =
    Arc<dyn Fn() -> Result<Option<StoredSnapshot>, SnapshotStoreError> + Send + Sync>;

/// Implementations must atomically replace the current snapshot and preserve a
/// last-known-good snapshot. No host filesystem implementation is provided.
pub trait IdentitySnapshotStore {
    /// Protected operations need a reader that remains bound to this store.
    /// Returning None preserves source compatibility and denies live identities.
    /// A failed or unavailable reader must never fall back to cached authority.
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        None
    }

    /// Atomically observe current bytes with the independent store CAS
    /// high-water version. If current bytes are absent or restored from an
    /// older last-known-good snapshot, `high_water_version` must still retain
    /// the newest committed CAS version. `None` is valid only before the first
    /// commit; a current payload is present only when its version equals the
    /// high-water version. Report an older current payload as absent (or fail
    /// closed) so recovery can use last-known-good data.
    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError>;

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;

    /// Load the exact (bytes, version) pair from an earlier successful
    /// commit. The version is provenance: equal versions identify the same
    /// original commit generation and the history represented by those bytes.
    /// Implementations must not relabel restored LKG bytes with the current
    /// high-water or a newly allocated version. Restored bytes become current
    /// only through commit_atomic, which issues a distinct later CAS version;
    /// the backup keeps its source version until a later committed current
    /// snapshot naturally replaces it as LKG.
    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;

    /// Compare `expected_version` against the non-rollback high-water version,
    /// not the current or last-known-good payload version. A successful commit
    /// must issue a strictly newer version and must never reuse a prior one;
    /// `None` is valid only before the store's first committed snapshot.
    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        complete_snapshot: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError>;
}

/// Deterministic store fake for orchestration tests. It has no guest or host
/// file persistence semantics.
#[derive(Debug, Default)]
pub struct InMemorySnapshotStore {
    current: Option<StoredSnapshot>,
    last_known_good: Option<StoredSnapshot>,
    high_water_version: Option<SnapshotVersion>,
    fail_next_commit: bool,
    fail_reads: bool,
    observed: Arc<RwLock<MemoryObservation>>,
}

#[derive(Debug, Default)]
struct MemoryObservation {
    current: Option<StoredSnapshot>,
    unavailable: bool,
}

// This fake has always cloned independent snapshots, not shared stores.
impl Clone for InMemorySnapshotStore {
    fn clone(&self) -> Self {
        Self {
            current: self.current.clone(),
            last_known_good: self.last_known_good.clone(),
            high_water_version: self.high_water_version,
            fail_next_commit: self.fail_next_commit,
            fail_reads: self.fail_reads,
            observed: Arc::new(RwLock::new(MemoryObservation {
                current: self.current.clone(),
                unavailable: self.fail_reads,
            })),
        }
    }
}

impl InMemorySnapshotStore {
    pub fn fail_next_commit(&mut self) {
        self.fail_next_commit = true;
    }

    pub fn fail_reads(&mut self) {
        self.fail_reads = true;
        if let Ok(mut observation) = self.observed.write() {
            observation.unavailable = true;
        }
    }

    pub fn corrupt_current_for_test(&mut self, bytes: Vec<u8>) {
        let version = self
            .high_water_version
            .or_else(|| self.current.as_ref().map(|snapshot| snapshot.version))
            .unwrap_or(SnapshotVersion::new(1));
        self.high_water_version = Some(version);
        self.current = Some(StoredSnapshot { bytes, version });
        if let Ok(mut observation) = self.observed.write() {
            observation.current = self.current.clone();
        }
    }

    pub fn current_bytes(&self) -> Option<&[u8]> {
        self.current
            .as_ref()
            .map(|snapshot| snapshot.bytes.as_slice())
    }
}

impl IdentitySnapshotStore for InMemorySnapshotStore {
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        let observed = self.observed.clone();
        Some(Arc::new(move || {
            let observation = observed
                .read()
                .map_err(|_| SnapshotStoreError::Unavailable)?;
            if observation.unavailable {
                return Err(SnapshotStoreError::Unavailable);
            }
            Ok(observation.current.clone())
        }))
    }

    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(SnapshotStoreState {
            current: self.current.clone(),
            high_water_version: self.high_water_version,
        })
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(self.current.clone())
    }

    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(self.last_known_good.clone())
    }

    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        complete_snapshot: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(SnapshotStoreError::Unavailable);
        }
        IdentitySnapshot::from_json(complete_snapshot)
            .map_err(|_| SnapshotStoreError::Unavailable)?;

        if self.high_water_version != expected_version {
            return Err(SnapshotStoreError::Conflict);
        }

        let next_version = match expected_version {
            Some(version) => version
                .get()
                .checked_add(1)
                .ok_or(SnapshotStoreError::Unavailable)?,
            None => 1,
        };
        let next_version = SnapshotVersion::new(next_version);

        // Acquire observation access before changing either saved snapshot.
        // Poisoned observation state must never publish authority.
        let mut observation = self
            .observed
            .write()
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        if let Some(current) = self.current.as_ref() {
            if IdentitySnapshot::from_json(&current.bytes).is_ok() {
                self.last_known_good = Some(current.clone());
            }
        }
        self.current = Some(StoredSnapshot {
            bytes: complete_snapshot.to_vec(),
            version: next_version,
        });
        observation.current = self.current.clone();
        self.high_water_version = Some(next_version);
        Ok(next_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthorizationFailure, AuthorizationRequest, CallerContextId, Clock, GuestCleanupFailure,
        GuestProfileCleaner, IdSourceFailure, IdentityAuthorizer, IdentityIdSource,
        IdentityService, NoopIdentityEventSink, PrincipalId, RecoverySource, SessionId,
        TrustedCallerContext, UnixMillis, UserId,
    };

    #[derive(Clone, Copy)]
    struct TestClock;

    impl Clock for TestClock {
        fn now(&self) -> UnixMillis {
            UnixMillis::new(100)
        }
    }

    #[derive(Default)]
    struct TestIds {
        user: u64,
        profile: u64,
        session: u64,
    }

    impl IdentityIdSource for TestIds {
        fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
            self.user += 1;
            UserId::new(self.user).map_err(|_| IdSourceFailure::Unavailable)
        }

        fn next_profile_id(&mut self) -> Result<crate::ProfileId, IdSourceFailure> {
            self.profile += 1;
            crate::ProfileId::new(format!("profile_p{}", self.profile))
                .map_err(|_| IdSourceFailure::Unavailable)
        }

        fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
            self.session += 1;
            SessionId::new(format!("session_s{}", self.session))
                .map_err(|_| IdSourceFailure::Unavailable)
        }
    }

    struct AllowAuthorization;

    impl IdentityAuthorizer for AllowAuthorization {
        fn authorize(
            &mut self,
            _request: &AuthorizationRequest,
        ) -> Result<(), AuthorizationFailure> {
            Ok(())
        }
    }

    struct NoopCleaner;

    impl GuestProfileCleaner for NoopCleaner {
        fn cleanup_guest_profile(
            &mut self,
            _profile_id: &crate::ProfileId,
        ) -> Result<(), GuestCleanupFailure> {
            Ok(())
        }
    }

    struct TestCaller {
        principal: PrincipalId,
        context: CallerContextId,
    }

    impl TestCaller {
        fn new() -> Self {
            Self {
                principal: PrincipalId::new("system.identity-store-test").unwrap(),
                context: CallerContextId::new("caller_identity-store-test").unwrap(),
            }
        }
    }

    impl TrustedCallerContext for TestCaller {
        fn principal_id(&self) -> &PrincipalId {
            &self.principal
        }

        fn context_id(&self) -> &CallerContextId {
            &self.context
        }
    }

    type TestService =
        IdentityService<InMemorySnapshotStore, TestIds, TestClock, AllowAuthorization, NoopCleaner>;

    fn new_service(store: InMemorySnapshotStore) -> TestService {
        IdentityService::new(
            store,
            TestIds::default(),
            TestClock,
            AllowAuthorization,
            NoopCleaner,
            NoopIdentityEventSink,
        )
    }

    #[test]
    fn in_memory_store_keeps_high_water_after_current_loss_and_rejects_lkg_cas() {
        let mut store = InMemorySnapshotStore::default();
        let bytes = IdentitySnapshot::empty().to_json().unwrap();
        let first = store.commit_atomic(None, &bytes).unwrap();
        let latest = store.commit_atomic(Some(first), &bytes).unwrap();
        let lkg = store.load_last_known_good().unwrap().unwrap();
        assert_eq!(lkg.version, first);

        store.current = None;
        store.observed.write().unwrap().current = None;

        let observation = store.load_current_state().unwrap();
        assert!(observation.current.is_none());
        assert_eq!(observation.high_water_version, Some(latest));
        assert_eq!(
            store.commit_atomic(Some(lkg.version), &lkg.bytes),
            Err(SnapshotStoreError::Conflict),
            "a stale LKG revision cannot pass the independent high-water CAS"
        );
        assert_eq!(
            store.load_current_state().unwrap().high_water_version,
            Some(latest)
        );
    }

    #[test]
    fn in_memory_store_overflow_fails_without_mutating_current_or_lkg() {
        let mut store = InMemorySnapshotStore::default();
        let bytes = IdentitySnapshot::empty().to_json().unwrap();
        let maximum = SnapshotVersion::new(u64::MAX);
        let previous = SnapshotVersion::new(u64::MAX - 1);
        store.current = Some(StoredSnapshot {
            bytes: bytes.clone(),
            version: maximum,
        });
        store.last_known_good = Some(StoredSnapshot {
            bytes: bytes.clone(),
            version: previous,
        });
        store.high_water_version = Some(maximum);
        store.observed.write().unwrap().current = store.current.clone();

        assert_eq!(
            store.commit_atomic(Some(maximum), &bytes),
            Err(SnapshotStoreError::Unavailable)
        );

        let state = store.load_current_state().unwrap();
        assert_eq!(state.current.unwrap().version, maximum);
        assert_eq!(state.high_water_version, Some(maximum));
        assert_eq!(
            store.load_last_known_good().unwrap().unwrap().version,
            previous
        );
    }

    #[test]
    fn in_memory_store_lkg_recovery_persists_incomplete_profile_history() {
        let caller = TestCaller::new();
        let mut writer = new_service(InMemorySnapshotStore::default());
        assert_eq!(
            writer.recover(&caller).unwrap().source,
            RecoverySource::Empty
        );
        let guest = writer
            .create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("in-memory-lost-guest").unwrap(),
            )
            .unwrap();
        let lost_profile = writer
            .snapshot()
            .sessions
            .iter()
            .find(|session| session.session_id == guest)
            .unwrap()
            .profile_id
            .clone();

        let (mut store, _, _, _, _, _) = writer.into_parts();
        let state = store.load_current_state().unwrap();
        let latest = state.high_water_version.unwrap();
        let lkg = store.load_last_known_good().unwrap().unwrap();
        assert_eq!(latest.get(), lkg.version.get() + 1);
        let lkg_snapshot = IdentitySnapshot::from_json(&lkg.bytes).unwrap();
        assert!(!lkg_snapshot
            .profiles
            .iter()
            .any(|profile| profile.profile_id == lost_profile));
        assert!(!lkg_snapshot
            .sessions
            .iter()
            .any(|session| session.profile_id == lost_profile));

        store.current = None;
        store.observed.write().unwrap().current = None;
        let mut recovered = new_service(store);
        assert_eq!(
            recovered.recover(&caller).unwrap().source,
            RecoverySource::LastKnownGood
        );
        assert!(!recovered.snapshot().profile_id_history_complete);
        assert_eq!(
            recovered.create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("blocked-in-memory-reuse").unwrap()
            ),
            Err(crate::IdentityError::ProfileHistoryIncomplete)
        );

        let (store, _, _, _, _, _) = recovered.into_parts();
        let mut restarted = new_service(store);
        assert_eq!(
            restarted.recover(&caller).unwrap().source,
            RecoverySource::Current
        );
        assert!(!restarted.snapshot().profile_id_history_complete);
        assert_eq!(
            restarted.create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("blocked-after-restart").unwrap()
            ),
            Err(crate::IdentityError::ProfileHistoryIncomplete)
        );
    }

    #[test]
    fn in_memory_store_missing_current_and_lkg_after_commit_rejects_recovery() {
        let caller = TestCaller::new();
        let mut initialized = new_service(InMemorySnapshotStore::default());
        initialized.recover(&caller).unwrap();
        let (mut store, _, _, _, _, _) = initialized.into_parts();
        assert!(store
            .load_current_state()
            .unwrap()
            .high_water_version
            .is_some());
        assert!(store.load_last_known_good().unwrap().is_none());

        store.current = None;
        store.observed.write().unwrap().current = None;
        let mut recovered = new_service(store);
        assert_eq!(
            recovered.recover(&caller),
            Err(crate::IdentityError::CorruptState)
        );
    }
}
