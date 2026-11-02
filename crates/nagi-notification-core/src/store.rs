use crate::model::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationRecord<S> {
    pub(crate) id: NotificationId,
    pub(crate) source: VerifiedSource<S>,
    pub(crate) title: NotificationContent,
    pub(crate) body: NotificationContent,
    pub(crate) priority: Priority,
    pub(crate) severity: Severity,
    pub(crate) created_at: Timestamp,
    pub(crate) expires_at: Option<Timestamp>,
    pub(crate) read_state: ReadState,
    pub(crate) lifecycle: LifecycleState,
    pub(crate) grouping_key: Option<GroupingKey>,
    pub(crate) actions: Vec<ActionDescriptor>,
    pub(crate) delivery: DeliveryState,
}

impl<S> NotificationRecord<S> {
    pub const fn id(&self) -> NotificationId {
        self.id
    }

    pub fn source(&self) -> &VerifiedSource<S> {
        &self.source
    }

    pub fn title(&self) -> &NotificationContent {
        &self.title
    }

    pub fn body(&self) -> &NotificationContent {
        &self.body
    }

    pub const fn priority(&self) -> Priority {
        self.priority
    }

    pub const fn severity(&self) -> Severity {
        self.severity
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    pub const fn read_state(&self) -> ReadState {
        self.read_state
    }

    pub const fn lifecycle(&self) -> LifecycleState {
        self.lifecycle
    }

    pub fn grouping_key(&self) -> Option<&GroupingKey> {
        self.grouping_key.as_ref()
    }

    pub fn actions(&self) -> &[ActionDescriptor] {
        &self.actions
    }

    pub const fn delivery(&self) -> DeliveryState {
        self.delivery
    }

    pub fn estimated_bytes(&self) -> usize {
        64 + MAX_NOTIFICATION_ID_BYTES
            + self.source.diagnostic_reference().as_str().len()
            + self.source.attribution().encoded_size()
            + self.title.encoded_size()
            + self.body.encoded_size()
            + self
                .grouping_key
                .as_ref()
                .map_or(0, |key| key.as_str().len())
            + self
                .actions
                .iter()
                .map(ActionDescriptor::encoded_size)
                .sum::<usize>()
    }

    pub(crate) fn visible_at(&self, now: Timestamp) -> bool {
        if self.expires_at.is_some_and(|expiry| now >= expiry) {
            return false;
        }
        match self.delivery {
            DeliveryState::Immediate => true,
            DeliveryState::Deferred { until } => now >= until,
            DeliveryState::Suppressed { .. } => false,
        }
    }

    pub(crate) fn expired_at(&self, now: Timestamp) -> bool {
        self.expires_at.is_some_and(|expiry| now >= expiry)
            || matches!(self.delivery, DeliveryState::Suppressed { retain_until } if now >= retain_until)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationSnapshot<S> {
    schema_version: u16,
    generation: u64,
    records: Vec<NotificationRecord<S>>,
}

impl<S> NotificationSnapshot<S> {
    pub fn empty() -> Self {
        Self {
            schema_version: NOTIFICATION_STORE_SCHEMA_VERSION,
            generation: 0,
            records: Vec::new(),
        }
    }

    /// Construct a snapshot received from a persistence adapter. The service
    /// validates version and record invariants before exposing it.
    pub fn from_parts(
        schema_version: u16,
        generation: u64,
        records: Vec<NotificationRecord<S>>,
    ) -> Self {
        Self {
            schema_version,
            generation,
            records,
        }
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn records(&self) -> &[NotificationRecord<S>] {
        &self.records
    }

    pub(crate) fn records_mut(&mut self) -> &mut Vec<NotificationRecord<S>> {
        &mut self.records
    }

    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    pub(crate) fn set_schema_version(&mut self, version: u16) {
        self.schema_version = version;
    }

    pub fn estimated_bytes(&self) -> usize {
        self.records
            .iter()
            .map(NotificationRecord::estimated_bytes)
            .fold(0_usize, usize::saturating_add)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorruptionCode {
    MalformedSnapshot,
    TruncatedSnapshot,
    UnsupportedEncoding,
    IntegrityFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreLoad<S> {
    Empty,
    Snapshot(NotificationSnapshot<S>),
    Corrupt(CorruptionCode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceError {
    Unavailable,
    Conflict,
    Corrupt,
    QuotaExceeded,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StoreUsage {
    pub profile_count: usize,
    pub total_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreQuota {
    pub max_profiles: usize,
    pub max_notifications_per_profile: usize,
    pub max_notification_bytes: usize,
    pub max_store_bytes_per_profile: usize,
    pub max_total_store_bytes: usize,
    pub max_groups_per_profile: usize,
    pub max_query_results: usize,
    pub max_actions_per_notification: usize,
}

impl Default for StoreQuota {
    fn default() -> Self {
        Self {
            max_profiles: 32,
            max_notifications_per_profile: 256,
            max_notification_bytes: MAX_NOTIFICATION_BYTES,
            max_store_bytes_per_profile: 512 * 1024,
            max_total_store_bytes: 4 * 1024 * 1024,
            max_groups_per_profile: 128,
            max_query_results: 100,
            max_actions_per_notification: MAX_ACTIONS,
        }
    }
}

impl StoreQuota {
    pub fn validate(self) -> Result<Self, ValidationError> {
        if self.max_profiles == 0
            || self.max_notifications_per_profile == 0
            || self.max_notification_bytes == 0
            || self.max_notification_bytes > MAX_NOTIFICATION_BYTES
            || self.max_store_bytes_per_profile == 0
            || self.max_total_store_bytes == 0
            || self.max_groups_per_profile == 0
            || self.max_query_results == 0
            || self.max_query_results > self.max_notifications_per_profile
            || self.max_actions_per_notification > MAX_ACTIONS
        {
            return Err(ValidationError::InvalidQueryLimit);
        }
        Ok(self)
    }
}

pub trait NotificationPersistence<P, S> {
    fn load(&mut self, profile: &P) -> Result<StoreLoad<S>, PersistenceError>;

    /// Must be atomic: either the full next snapshot is visible or the prior
    /// generation remains intact.
    fn commit_atomic(
        &mut self,
        profile: &P,
        expected_generation: u64,
        next: NotificationSnapshot<S>,
    ) -> Result<(), PersistenceError>;

    fn usage(&self) -> StoreUsage;
}

/// Host-only reference persistence. It is intentionally in-memory and is not
/// evidence of durable Nagi storage or production persistence.
pub struct InMemoryNotificationPersistence<P, S> {
    snapshots: Vec<(P, NotificationSnapshot<S>)>,
    corrupt: Vec<(P, CorruptionCode)>,
    fail_next_commit: bool,
    quota: StoreQuota,
}

impl<P, S> InMemoryNotificationPersistence<P, S>
where
    P: Clone + Eq,
    S: Clone,
{
    pub fn new(quota: StoreQuota) -> Result<Self, ValidationError> {
        Ok(Self {
            snapshots: Vec::new(),
            corrupt: Vec::new(),
            fail_next_commit: false,
            quota: quota.validate()?,
        })
    }

    /// Test-fixture hook for exercising corrupt-state recovery.
    pub fn inject_corruption(&mut self, profile: P, code: CorruptionCode) {
        self.snapshots.retain(|(existing, _)| existing != &profile);
        self.corrupt.retain(|(existing, _)| existing != &profile);
        self.corrupt.push((profile, code));
    }

    /// Test-fixture hook for migration and unsupported-version cases.
    pub fn inject_snapshot(&mut self, profile: P, snapshot: NotificationSnapshot<S>) {
        self.corrupt.retain(|(existing, _)| existing != &profile);
        self.snapshots.retain(|(existing, _)| existing != &profile);
        self.snapshots.push((profile, snapshot));
    }

    /// Test-fixture hook for an atomic-commit failure.
    pub fn fail_next_commit(&mut self) {
        self.fail_next_commit = true;
    }

    pub fn snapshot(&self, profile: &P) -> Option<&NotificationSnapshot<S>> {
        self.snapshots
            .iter()
            .find(|(existing, _)| existing == profile)
            .map(|(_, snapshot)| snapshot)
    }

    pub fn quota(&self) -> StoreQuota {
        self.quota
    }
}

impl<P, S> NotificationPersistence<P, S> for InMemoryNotificationPersistence<P, S>
where
    P: Clone + Eq,
    S: Clone,
{
    fn load(&mut self, profile: &P) -> Result<StoreLoad<S>, PersistenceError> {
        if let Some((_, code)) = self
            .corrupt
            .iter()
            .find(|(existing, _)| existing == profile)
        {
            return Ok(StoreLoad::Corrupt(*code));
        }
        Ok(self
            .snapshots
            .iter()
            .find(|(existing, _)| existing == profile)
            .map(|(_, snapshot)| snapshot.clone())
            .map(StoreLoad::Snapshot)
            .unwrap_or(StoreLoad::Empty))
    }

    fn commit_atomic(
        &mut self,
        profile: &P,
        expected_generation: u64,
        next: NotificationSnapshot<S>,
    ) -> Result<(), PersistenceError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(PersistenceError::Unavailable);
        }
        if self.corrupt.iter().any(|(existing, _)| existing == profile) {
            return Err(PersistenceError::Corrupt);
        }
        let current_index = self
            .snapshots
            .iter()
            .position(|(existing, _)| existing == profile);
        let current_generation =
            current_index.map_or(0, |index| self.snapshots[index].1.generation());
        if current_generation != expected_generation
            || expected_generation.checked_add(1) != Some(next.generation())
        {
            return Err(PersistenceError::Conflict);
        }
        if next.schema_version() != NOTIFICATION_STORE_SCHEMA_VERSION {
            return Err(PersistenceError::Corrupt);
        }
        if current_index.is_none()
            && self.snapshots.len() + self.corrupt.len() >= self.quota.max_profiles
        {
            return Err(PersistenceError::QuotaExceeded);
        }
        if next.estimated_bytes() > self.quota.max_store_bytes_per_profile {
            return Err(PersistenceError::QuotaExceeded);
        }

        let old_bytes = current_index.map_or(0, |index| self.snapshots[index].1.estimated_bytes());
        let total_after = self
            .usage()
            .total_bytes
            .saturating_sub(old_bytes)
            .saturating_add(next.estimated_bytes());
        if total_after > self.quota.max_total_store_bytes {
            return Err(PersistenceError::QuotaExceeded);
        }

        // All validation happens before this replacement: an error above never
        // alters the previous snapshot.
        self.corrupt.retain(|(existing, _)| existing != profile);
        if let Some(index) = current_index {
            self.snapshots[index].1 = next;
        } else {
            self.snapshots.push((profile.clone(), next));
        }
        Ok(())
    }

    fn usage(&self) -> StoreUsage {
        StoreUsage {
            profile_count: self.snapshots.len() + self.corrupt.len(),
            total_bytes: self
                .snapshots
                .iter()
                .map(|(_, snapshot)| snapshot.estimated_bytes())
                .fold(0_usize, usize::saturating_add),
        }
    }
}
