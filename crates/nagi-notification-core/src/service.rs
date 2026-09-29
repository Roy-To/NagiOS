use std::collections::BTreeSet;

use nagi_localization::LocaleContext;

use crate::model::*;
use crate::store::*;

const GLOBAL_DIAGNOSTIC_CORRELATION: SafeProfileCorrelation = SafeProfileCorrelation::new([0; 16]);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishRequest<S> {
    source: S,
    title: NotificationContent,
    body: NotificationContent,
    priority: Priority,
    severity: Severity,
    expires_at: Option<Timestamp>,
    grouping_key: Option<GroupingKey>,
    actions: Vec<ActionDescriptor>,
}

impl<S> PublishRequest<S> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: S,
        title: NotificationContent,
        body: NotificationContent,
        priority: Priority,
        severity: Severity,
        expires_at: Option<Timestamp>,
        grouping_key: Option<GroupingKey>,
        actions: Vec<ActionDescriptor>,
    ) -> Result<Self, ValidationError> {
        if actions.len() > MAX_ACTIONS {
            return Err(ValidationError::TooManyActions);
        }
        let bytes = title.encoded_size()
            + body.encoded_size()
            + grouping_key.as_ref().map_or(0, |key| key.as_str().len())
            + actions
                .iter()
                .map(ActionDescriptor::encoded_size)
                .sum::<usize>();
        if bytes > MAX_NOTIFICATION_BYTES {
            return Err(ValidationError::ContentTooLarge);
        }
        Ok(Self {
            source,
            title,
            body,
            priority,
            severity,
            expires_at,
            grouping_key,
            actions,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationQuery<S> {
    pub limit: usize,
    pub read_state: Option<ReadState>,
    pub lifecycle: Option<LifecycleState>,
    pub grouping_key: Option<GroupingKey>,
    pub source: Option<S>,
}

impl<S> Default for NotificationQuery<S> {
    fn default() -> Self {
        Self {
            limit: 50,
            read_state: None,
            lifecycle: Some(LifecycleState::Active),
            grouping_key: None,
            source: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationSummary {
    pub id: NotificationId,
    pub source_kind: SourceKind,
    pub source_attribution: NotificationContent,
    pub title: NotificationContent,
    pub body: NotificationContent,
    pub priority: Priority,
    pub severity: Severity,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub read_state: ReadState,
    pub lifecycle: LifecycleState,
    pub grouping_key: Option<GroupingKey>,
    pub actions: Vec<ActionDescriptor>,
    pub delivery: DeliveryState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationUiAction {
    pub id: ActionId,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationUiView {
    pub id: NotificationId,
    pub source_kind: SourceKind,
    pub source_attribution: String,
    pub title: String,
    pub body: String,
    pub priority: Priority,
    pub severity: Severity,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub read_state: ReadState,
    pub lifecycle: LifecycleState,
    pub grouping_key: Option<GroupingKey>,
    pub actions: Vec<NotificationUiAction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationOutcome {
    Changed,
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishOutcome {
    pub id: NotificationId,
    pub delivery: DeliveryState,
    pub evicted: Vec<NotificationId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupOutcome {
    pub expired: Vec<NotificationId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionResolution {
    pub notification_id: NotificationId,
    pub action_id: ActionId,
    pub availability: ActionAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterFailure {
    Unavailable,
    Rejected,
    StaleIdentity,
    InvalidLocalization,
    MigrationFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationError {
    Validation(ValidationError),
    ProfileUnavailable,
    SourceRejected,
    PermissionDenied,
    PolicyUnavailable,
    PolicyRejected(PolicyReason),
    LocalizationRejected,
    PersistenceUnavailable,
    PersistenceConflict,
    CapacityRejected,
    RecoveryReadOnly,
    CorruptState,
    UnsupportedSchema,
    MigrationFailed,
    GenerationExhausted,
    NotFound,
    Expired,
    ActionUnavailable,
    DuplicateNotificationId,
}

impl NotificationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Validation(_) => "NOTIFICATION_INVALID_INPUT",
            Self::ProfileUnavailable => "NOTIFICATION_PROFILE_UNAVAILABLE",
            Self::SourceRejected => "NOTIFICATION_SOURCE_REJECTED",
            Self::PermissionDenied => "NOTIFICATION_PERMISSION_DENIED",
            Self::PolicyUnavailable => "NOTIFICATION_POLICY_UNAVAILABLE",
            Self::PolicyRejected(_) => "NOTIFICATION_POLICY_REJECTED",
            Self::LocalizationRejected => "NOTIFICATION_LOCALIZATION_REJECTED",
            Self::PersistenceUnavailable => "NOTIFICATION_STORE_UNAVAILABLE",
            Self::PersistenceConflict => "NOTIFICATION_STORE_CONFLICT",
            Self::CapacityRejected => "NOTIFICATION_CAPACITY_REJECTED",
            Self::RecoveryReadOnly => "NOTIFICATION_RECOVERY_READ_ONLY",
            Self::CorruptState => "NOTIFICATION_CORRUPT_STATE",
            Self::UnsupportedSchema => "NOTIFICATION_UNSUPPORTED_SCHEMA",
            Self::MigrationFailed => "NOTIFICATION_MIGRATION_FAILED",
            Self::GenerationExhausted => "NOTIFICATION_GENERATION_EXHAUSTED",
            Self::NotFound => "NOTIFICATION_NOT_FOUND",
            Self::Expired => "NOTIFICATION_EXPIRED",
            Self::ActionUnavailable => "NOTIFICATION_ACTION_UNAVAILABLE",
            Self::DuplicateNotificationId => "NOTIFICATION_DUPLICATE_ID",
        }
    }
}

impl From<ValidationError> for NotificationError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

/// All external identity, policy, permission, localization, redaction and
/// action behavior is supplied by narrow host-testable adapters. Implementors
/// must derive profile and source context from trusted inputs, never from
/// caller-asserted names or notification payloads.
pub trait NotificationAdapters {
    type Caller;
    type Profile: Clone + Eq;
    type Source: Clone + Eq;

    fn profile_scope(&mut self, caller: &Self::Caller) -> Result<Self::Profile, AdapterFailure>;
    fn profile_correlation(&self, profile: &Self::Profile) -> SafeProfileCorrelation;

    fn authenticate_source(
        &mut self,
        caller: &Self::Caller,
        source: &Self::Source,
    ) -> Result<VerifiedSource<Self::Source>, AdapterFailure>;

    fn revalidate_source(
        &mut self,
        caller: &Self::Caller,
        source: &VerifiedSource<Self::Source>,
    ) -> Result<(), AdapterFailure>;

    fn authorize(
        &mut self,
        caller: &Self::Caller,
        profile: &Self::Profile,
        source: Option<&VerifiedSource<Self::Source>>,
        operation: NotificationOperation,
    ) -> CapabilityDecision;

    fn quiet_focus_decision(
        &mut self,
        source: &VerifiedSource<Self::Source>,
        priority: Priority,
        severity: Severity,
        now: Timestamp,
    ) -> Result<PolicyDecision, AdapterFailure>;

    /// Validate both the message key and the bounded argument names against
    /// the shared localization catalog adapter.
    fn validate_localized(&mut self, message: &LocalizedMessage) -> Result<(), AdapterFailure>;

    fn render_localized(
        &mut self,
        message: &LocalizedMessage,
        locale: &LocaleContext,
    ) -> Result<String, AdapterFailure>;

    fn render_redacted(&mut self, locale: &LocaleContext) -> Result<String, AdapterFailure>;

    /// A policy hook may add redaction. Core rules below prevent it from
    /// declassifying content at persistence or diagnostics boundaries.
    fn must_redact(
        &mut self,
        boundary: RedactionBoundary,
        source: &VerifiedSource<Self::Source>,
        sensitivity: Sensitivity,
    ) -> bool;

    /// Resolves descriptor availability only. This must not execute an action
    /// or return an authority-bearing token.
    fn resolve_action_descriptor(
        &mut self,
        source: &VerifiedSource<Self::Source>,
        descriptor: &ActionDescriptor,
    ) -> Result<ActionAvailability, AdapterFailure>;

    /// Migration adapters return a fully validated current-version snapshot.
    fn migrate_snapshot(
        &mut self,
        snapshot: NotificationSnapshot<Self::Source>,
    ) -> Result<NotificationSnapshot<Self::Source>, AdapterFailure>;
}

pub trait NotificationClock {
    fn now(&self) -> Timestamp;
}

pub trait NotificationIdGenerator {
    fn next_id(&mut self) -> Result<NotificationId, AdapterFailure>;
}

pub trait NotificationDiagnosticsSink {
    fn emit(&mut self, event: NotificationDiagnosticEvent) -> Result<(), AdapterFailure>;
}

pub struct NullNotificationDiagnostics;

impl NotificationDiagnosticsSink for NullNotificationDiagnostics {
    fn emit(&mut self, _event: NotificationDiagnosticEvent) -> Result<(), AdapterFailure> {
        Ok(())
    }
}

/// Future UI boundary. It receives a localized, authorized and redacted value
/// object and has no service/store/capability handle.
pub trait NotificationUiAdapter {
    type Error;
    fn present(&mut self, view: NotificationUiView) -> Result<(), Self::Error>;
}

pub struct NotificationService<A, Store, Clock, Ids, Diagnostics>
where
    A: NotificationAdapters,
    Store: NotificationPersistence<A::Profile, A::Source>,
{
    adapters: A,
    persistence: Store,
    clock: Clock,
    ids: Ids,
    diagnostics: Diagnostics,
    quota: StoreQuota,
    policy_failure_mode: PolicyFailureMode,
    read_only_profiles: Vec<A::Profile>,
    diagnostics_failures: u64,
    diagnostics_dropped: u64,
    diagnostics_dropped_reported: u64,
    diagnostics_window_start: Option<Timestamp>,
    diagnostics_events_in_window: u32,
    pending_sink_failure: bool,
}

impl<A, Store, Clock, Ids, Diagnostics> NotificationService<A, Store, Clock, Ids, Diagnostics>
where
    A: NotificationAdapters,
    Store: NotificationPersistence<A::Profile, A::Source>,
    Clock: NotificationClock,
    Ids: NotificationIdGenerator,
    Diagnostics: NotificationDiagnosticsSink,
{
    pub fn new(
        adapters: A,
        persistence: Store,
        clock: Clock,
        ids: Ids,
        diagnostics: Diagnostics,
        quota: StoreQuota,
        policy_failure_mode: PolicyFailureMode,
    ) -> Result<Self, ValidationError> {
        let quota = quota.validate()?;
        if matches!(policy_failure_mode, PolicyFailureMode::DeferForMillis(0)) {
            return Err(ValidationError::InvalidExpiry);
        }
        Ok(Self {
            adapters,
            persistence,
            clock,
            ids,
            diagnostics,
            quota,
            policy_failure_mode,
            read_only_profiles: Vec::new(),
            diagnostics_failures: 0,
            diagnostics_dropped: 0,
            diagnostics_dropped_reported: 0,
            diagnostics_window_start: None,
            diagnostics_events_in_window: 0,
            pending_sink_failure: false,
        })
    }

    pub fn diagnostics_failures(&self) -> u64 {
        self.diagnostics_failures
    }

    pub fn publish(
        &mut self,
        caller: &A::Caller,
        request: PublishRequest<A::Source>,
    ) -> Result<PublishOutcome, NotificationError> {
        if request.actions.len() > self.quota.max_actions_per_notification {
            return Err(ValidationError::TooManyActions.into());
        }
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        self.ensure_writable(&profile)?;
        let source = match self.adapters.authenticate_source(caller, &request.source) {
            Ok(source) => source,
            Err(_) => {
                self.emit(
                    &profile,
                    None,
                    None,
                    NotificationEventCode::PublishRejected,
                    Some(NotificationReasonCode::SourceRejected),
                    1,
                );
                return Err(NotificationError::SourceRejected);
            }
        };
        if let Err(error) = self.validate_content(source.attribution()) {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::PublishRejected,
                Some(NotificationReasonCode::InvalidInput),
                1,
            );
            return Err(error);
        }
        let decision = self.adapters.authorize(
            caller,
            &profile,
            Some(&source),
            NotificationOperation::Publish,
        );
        if !decision.allowed {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::PublishRejected,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }

        let now = self.clock.now();
        if request.expires_at.is_some_and(|expiry| expiry <= now) {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::PublishRejected,
                Some(NotificationReasonCode::InvalidInput),
                1,
            );
            return Err(ValidationError::InvalidExpiry.into());
        }
        if let Err(error) = self.validate_content(&request.title) {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::PublishRejected,
                Some(NotificationReasonCode::InvalidInput),
                1,
            );
            return Err(error);
        }
        if let Err(error) = self.validate_content(&request.body) {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::PublishRejected,
                Some(NotificationReasonCode::InvalidInput),
                1,
            );
            return Err(error);
        }
        for action in &request.actions {
            if let Some(label) = action.label() {
                if self.adapters.validate_localized(label).is_err() {
                    self.emit(
                        &profile,
                        None,
                        Some(&source),
                        NotificationEventCode::PublishRejected,
                        Some(NotificationReasonCode::InvalidInput),
                        1,
                    );
                    return Err(NotificationError::LocalizationRejected);
                }
            }
        }

        let policy_result =
            self.adapters
                .quiet_focus_decision(&source, request.priority, request.severity, now);
        let (policy_decision, policy_failed) = match policy_result {
            Ok(decision) => (decision, false),
            Err(_) => match self.policy_failure_mode {
                PolicyFailureMode::Reject => {
                    self.emit(
                        &profile,
                        None,
                        Some(&source),
                        NotificationEventCode::PublishRejected,
                        Some(NotificationReasonCode::PolicyUnavailable),
                        1,
                    );
                    return Err(NotificationError::PolicyUnavailable);
                }
                PolicyFailureMode::DeferForMillis(milliseconds) => {
                    let Some(until) = now.0.checked_add(milliseconds) else {
                        self.emit(
                            &profile,
                            None,
                            Some(&source),
                            NotificationEventCode::PublishRejected,
                            Some(NotificationReasonCode::PolicyUnavailable),
                            1,
                        );
                        return Err(NotificationError::PolicyUnavailable);
                    };
                    (PolicyDecision::DeferUntil(Timestamp(until)), true)
                }
            },
        };
        let delivery = match policy_decision {
            PolicyDecision::Permit => DeliveryState::Immediate,
            PolicyDecision::DeferUntil(until) if until > now => DeliveryState::Deferred { until },
            PolicyDecision::SuppressWithRetention(retain_until) if retain_until > now => {
                DeliveryState::Suppressed { retain_until }
            }
            PolicyDecision::Reject(reason) => {
                self.emit(
                    &profile,
                    None,
                    Some(&source),
                    NotificationEventCode::PublishRejected,
                    Some(NotificationReasonCode::PolicyRejected),
                    1,
                );
                return Err(NotificationError::PolicyRejected(reason));
            }
            PolicyDecision::DeferUntil(_) | PolicyDecision::SuppressWithRetention(_) => {
                self.emit(
                    &profile,
                    None,
                    Some(&source),
                    NotificationEventCode::PublishRejected,
                    Some(NotificationReasonCode::PolicyRejected),
                    1,
                );
                return Err(NotificationError::PolicyRejected(
                    PolicyReason::InvalidDecision,
                ));
            }
        };

        let id = self
            .ids
            .next_id()
            .map_err(|_| NotificationError::PersistenceUnavailable)?;
        let safe_source = self.sanitize_source(caller, &profile, &source);
        let (title, redacted_title) = self.sanitize_content(
            caller,
            &profile,
            &source,
            RedactionBoundary::Persistence,
            &request.title,
            false,
        );
        let (body, redacted_body) = self.sanitize_content(
            caller,
            &profile,
            &source,
            RedactionBoundary::Persistence,
            &request.body,
            false,
        );
        let mut actions = Vec::with_capacity(request.actions.len());
        let mut redacted_values = redacted_title as u32 + redacted_body as u32;
        for action in request.actions {
            let (action, redacted) = self.sanitize_action(
                caller,
                &profile,
                &source,
                RedactionBoundary::Persistence,
                action,
                false,
            );
            redacted_values = redacted_values.saturating_add(redacted as u32);
            actions.push(action);
        }
        let record = NotificationRecord {
            id,
            source: safe_source,
            title,
            body,
            priority: request.priority,
            severity: request.severity,
            created_at: now,
            expires_at: request.expires_at,
            read_state: ReadState::Unread,
            lifecycle: LifecycleState::Active,
            grouping_key: request.grouping_key,
            actions,
            delivery,
        };
        if record.estimated_bytes() > self.quota.max_notification_bytes {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::CapacityRejected,
                Some(NotificationReasonCode::CapacityExceeded),
                1,
            );
            return Err(NotificationError::CapacityRejected);
        }

        let mut snapshot = self.load_snapshot(&profile)?;
        if snapshot.records().iter().any(|existing| existing.id == id) {
            return Err(NotificationError::DuplicateNotificationId);
        }
        let now_expired = snapshot
            .records()
            .iter()
            .filter(|existing| existing.expired_at(now))
            .count();
        snapshot
            .records_mut()
            .retain(|existing| !existing.expired_at(now));
        snapshot.records_mut().push(record);
        let mut evicted = Vec::new();
        if !self.make_room(&profile, &mut snapshot, &mut evicted)? {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::CapacityRejected,
                Some(NotificationReasonCode::CapacityExceeded),
                1,
            );
            return Err(NotificationError::CapacityRejected);
        }
        self.commit_next(&profile, snapshot)?;

        if now_expired > 0 {
            self.emit(
                &profile,
                None,
                Some(&source),
                NotificationEventCode::Expired,
                None,
                now_expired.min(u32::MAX as usize) as u32,
            );
        }
        if redacted_values > 0 {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::Redacted,
                None,
                redacted_values,
            );
        }
        if policy_failed {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::PolicyDeferred,
                Some(NotificationReasonCode::PolicyUnavailable),
                1,
            );
        } else {
            let code = match delivery {
                DeliveryState::Immediate => NotificationEventCode::PolicyPermitted,
                DeliveryState::Deferred { .. } => NotificationEventCode::PolicyDeferred,
                DeliveryState::Suppressed { .. } => NotificationEventCode::PolicySuppressed,
            };
            self.emit(&profile, Some(id), Some(&source), code, None, 1);
        }
        if !evicted.is_empty() {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::Evicted,
                None,
                evicted.len().min(u32::MAX as usize) as u32,
            );
        }
        self.emit(
            &profile,
            Some(id),
            Some(&source),
            NotificationEventCode::Published,
            None,
            1,
        );
        Ok(PublishOutcome {
            id,
            delivery,
            evicted,
        })
    }

    pub fn query(
        &mut self,
        caller: &A::Caller,
        query: NotificationQuery<A::Source>,
    ) -> Result<Vec<NotificationSummary>, NotificationError> {
        if query.limit == 0 || query.limit > self.quota.max_query_results {
            return Err(ValidationError::InvalidQueryLimit.into());
        }
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        let decision = self
            .adapters
            .authorize(caller, &profile, None, NotificationOperation::List);
        if !decision.allowed {
            self.emit(
                &profile,
                None,
                None,
                NotificationEventCode::QueryDenied,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        let snapshot = self.load_snapshot(&profile)?;
        let now = self.clock.now();
        let mut records: Vec<_> = snapshot
            .records()
            .iter()
            .filter(|record| {
                if record.expired_at(now)
                    || !record.visible_at(now)
                    || record.lifecycle == LifecycleState::Dismissed
                        && query.lifecycle != Some(LifecycleState::Dismissed)
                {
                    return false;
                }
                if query
                    .lifecycle
                    .is_some_and(|state| state != record.lifecycle)
                    || query
                        .read_state
                        .is_some_and(|state| state != record.read_state)
                    || query
                        .grouping_key
                        .as_ref()
                        .is_some_and(|key| record.grouping_key.as_ref() != Some(key))
                    || query
                        .source
                        .as_ref()
                        .is_some_and(|source| record.source.identity() != source)
                {
                    return false;
                }
                true
            })
            .collect();
        records.sort_by(|left, right| compare_records(left, right));
        let mut result = Vec::new();
        let mut denied_reads = 0_u32;
        for record in records {
            if result.len() >= query.limit {
                break;
            }
            let read = self.adapters.authorize(
                caller,
                &profile,
                Some(&record.source),
                NotificationOperation::Read,
            );
            if !read.allowed {
                denied_reads = denied_reads.saturating_add(1);
                continue;
            }
            result.push(self.summary_for(
                caller,
                &profile,
                record,
                RedactionBoundary::Query,
                read.may_view_sensitive,
            ));
        }
        if denied_reads > 0 {
            self.emit(
                &profile,
                None,
                None,
                NotificationEventCode::CapabilityDenied,
                Some(NotificationReasonCode::PermissionDenied),
                denied_reads,
            );
        }
        Ok(result)
    }

    pub fn mark_read(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
    ) -> Result<MutationOutcome, NotificationError> {
        self.mutate_state(caller, id, NotificationOperation::MarkRead, |record| {
            if record.read_state == ReadState::Read {
                MutationOutcome::Unchanged
            } else {
                record.read_state = ReadState::Read;
                MutationOutcome::Changed
            }
        })
    }

    pub fn acknowledge(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
    ) -> Result<MutationOutcome, NotificationError> {
        self.mutate_state(caller, id, NotificationOperation::Acknowledge, |record| {
            if record.lifecycle == LifecycleState::Dismissed
                || record.lifecycle == LifecycleState::Acknowledged
            {
                MutationOutcome::Unchanged
            } else {
                record.lifecycle = LifecycleState::Acknowledged;
                record.read_state = ReadState::Read;
                MutationOutcome::Changed
            }
        })
    }

    pub fn dismiss(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
    ) -> Result<MutationOutcome, NotificationError> {
        self.mutate_state(caller, id, NotificationOperation::Dismiss, |record| {
            if record.lifecycle == LifecycleState::Dismissed {
                MutationOutcome::Unchanged
            } else {
                record.lifecycle = LifecycleState::Dismissed;
                record.read_state = ReadState::Read;
                MutationOutcome::Changed
            }
        })
    }

    pub fn cleanup_expired(
        &mut self,
        caller: &A::Caller,
    ) -> Result<CleanupOutcome, NotificationError> {
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        self.ensure_writable(&profile)?;
        if !self
            .adapters
            .authorize(
                caller,
                &profile,
                None,
                NotificationOperation::CleanupExpired,
            )
            .allowed
        {
            self.emit(
                &profile,
                None,
                None,
                NotificationEventCode::CapabilityDenied,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        let mut snapshot = self.load_snapshot(&profile)?;
        let now = self.clock.now();
        let mut expired = Vec::new();
        let mut retained = Vec::with_capacity(snapshot.records().len());
        for record in snapshot.records_mut().drain(..) {
            if record.expired_at(now) && expired.len() < MAX_CLEANUP_BATCH {
                expired.push(record.id);
            } else {
                retained.push(record);
            }
        }
        *snapshot.records_mut() = retained;
        if !expired.is_empty() {
            self.commit_next(&profile, snapshot)?;
            self.emit(
                &profile,
                None,
                None,
                NotificationEventCode::Expired,
                None,
                expired.len() as u32,
            );
        }
        Ok(CleanupOutcome { expired })
    }

    pub fn export(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
    ) -> Result<NotificationSummary, NotificationError> {
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        let snapshot = self.load_snapshot(&profile)?;
        let record = snapshot
            .records()
            .iter()
            .find(|record| record.id == id)
            .ok_or(NotificationError::NotFound)?;
        if record.expired_at(self.clock.now()) {
            return Err(NotificationError::Expired);
        }
        let capability = self.adapters.authorize(
            caller,
            &profile,
            Some(&record.source),
            NotificationOperation::Export,
        );
        if !capability.allowed {
            self.emit(
                &profile,
                Some(id),
                Some(&record.source),
                NotificationEventCode::CapabilityDenied,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        Ok(self.summary_for(
            caller,
            &profile,
            record,
            RedactionBoundary::Export,
            capability.may_view_sensitive,
        ))
    }

    pub fn ui_view(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
        locale: &LocaleContext,
    ) -> Result<NotificationUiView, NotificationError> {
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        let snapshot = self.load_snapshot(&profile)?;
        let record = snapshot
            .records()
            .iter()
            .find(|record| record.id == id)
            .ok_or(NotificationError::NotFound)?;
        let now = self.clock.now();
        if record.expired_at(now)
            || !record.visible_at(now)
            || record.lifecycle == LifecycleState::Dismissed
        {
            return Err(NotificationError::Expired);
        }
        let capability = self.adapters.authorize(
            caller,
            &profile,
            Some(&record.source),
            NotificationOperation::Read,
        );
        if !capability.allowed {
            self.emit(
                &profile,
                Some(id),
                Some(&record.source),
                NotificationEventCode::CapabilityDenied,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        let source_attribution = self.redact_content(
            caller,
            &profile,
            &record.source,
            RedactionBoundary::Ui,
            &record.source.attribution,
            capability.may_view_sensitive,
        );
        let title = self.redact_content(
            caller,
            &profile,
            &record.source,
            RedactionBoundary::Ui,
            &record.title,
            capability.may_view_sensitive,
        );
        let body = self.redact_content(
            caller,
            &profile,
            &record.source,
            RedactionBoundary::Ui,
            &record.body,
            capability.may_view_sensitive,
        );
        let mut actions = Vec::with_capacity(record.actions.len());
        for action in &record.actions {
            let action = self.redact_action(
                caller,
                &profile,
                &record.source,
                RedactionBoundary::Ui,
                action,
                capability.may_view_sensitive,
            );
            let label = action
                .label()
                .map(|message| self.adapters.render_localized(message, locale))
                .transpose()
                .map_err(|_| NotificationError::LocalizationRejected)?;
            actions.push(NotificationUiAction {
                id: action.id().clone(),
                label,
            });
        }
        Ok(NotificationUiView {
            id,
            source_kind: record.source.kind,
            source_attribution: self.render_content(&source_attribution, locale)?,
            title: self.render_content(&title, locale)?,
            body: self.render_content(&body, locale)?,
            priority: record.priority,
            severity: record.severity,
            created_at: record.created_at,
            expires_at: record.expires_at,
            read_state: record.read_state,
            lifecycle: record.lifecycle,
            grouping_key: record.grouping_key.clone(),
            actions,
        })
    }

    /// Revalidates the source and rechecks notification capability at the
    /// moment a UI activates a descriptor. It only resolves descriptor
    /// availability; the source provider remains responsible for executing
    /// its operation under its own fresh capability checks.
    pub fn activate_action(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
        action_id: &ActionId,
    ) -> Result<ActionResolution, NotificationError> {
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        let snapshot = self.load_snapshot(&profile)?;
        let record = snapshot
            .records()
            .iter()
            .find(|record| record.id == id)
            .ok_or(NotificationError::NotFound)?;
        if !record.visible_at(self.clock.now()) || record.lifecycle != LifecycleState::Active {
            return Err(NotificationError::Expired);
        }
        let source = record.source.clone();
        let action = record
            .actions
            .iter()
            .find(|action| action.id() == action_id)
            .cloned()
            .ok_or(NotificationError::ActionUnavailable)?;
        if self.adapters.revalidate_source(caller, &source).is_err() {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::SourceUnavailable,
                Some(NotificationReasonCode::SourceRejected),
                1,
            );
            return Err(NotificationError::SourceRejected);
        }
        if !self
            .adapters
            .authorize(
                caller,
                &profile,
                Some(&source),
                NotificationOperation::ResolveAction,
            )
            .allowed
        {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::ActionDenied,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        let availability = self
            .adapters
            .resolve_action_descriptor(&source, &action)
            .map_err(|_| {
                self.emit(
                    &profile,
                    Some(id),
                    Some(&source),
                    NotificationEventCode::ActionDenied,
                    Some(NotificationReasonCode::ActionUnavailable),
                    1,
                );
                NotificationError::ActionUnavailable
            })?;
        if availability == ActionAvailability::Unavailable {
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                NotificationEventCode::ActionDenied,
                Some(NotificationReasonCode::ActionUnavailable),
                1,
            );
            return Err(NotificationError::ActionUnavailable);
        }
        self.emit(
            &profile,
            Some(id),
            Some(&source),
            NotificationEventCode::ActionAllowed,
            None,
            1,
        );
        Ok(ActionResolution {
            notification_id: id,
            action_id: action.id().clone(),
            availability,
        })
    }

    fn mutate_state<F>(
        &mut self,
        caller: &A::Caller,
        id: NotificationId,
        operation: NotificationOperation,
        mut transition: F,
    ) -> Result<MutationOutcome, NotificationError>
    where
        F: FnMut(&mut NotificationRecord<A::Source>) -> MutationOutcome,
    {
        let profile = self
            .adapters
            .profile_scope(caller)
            .map_err(|_| NotificationError::ProfileUnavailable)?;
        self.ensure_writable(&profile)?;
        let mut snapshot = self.load_snapshot(&profile)?;
        let index = snapshot
            .records()
            .iter()
            .position(|record| record.id == id)
            .ok_or(NotificationError::NotFound)?;
        let now = self.clock.now();
        if snapshot.records()[index].expired_at(now) {
            return Err(NotificationError::Expired);
        }
        let source = snapshot.records()[index].source.clone();
        if !self
            .adapters
            .authorize(caller, &profile, Some(&source), operation)
            .allowed
        {
            let code = if operation == NotificationOperation::List {
                NotificationEventCode::QueryDenied
            } else {
                NotificationEventCode::CapabilityDenied
            };
            self.emit(
                &profile,
                Some(id),
                Some(&source),
                code,
                Some(NotificationReasonCode::PermissionDenied),
                1,
            );
            return Err(NotificationError::PermissionDenied);
        }
        let outcome = transition(&mut snapshot.records_mut()[index]);
        if outcome == MutationOutcome::Unchanged {
            return Ok(outcome);
        }
        self.commit_next(&profile, snapshot)?;
        let (code, reason) = match operation {
            NotificationOperation::MarkRead => (NotificationEventCode::ReadChanged, None),
            NotificationOperation::Acknowledge => (NotificationEventCode::Acknowledged, None),
            NotificationOperation::Dismiss => (NotificationEventCode::Dismissed, None),
            _ => (
                NotificationEventCode::StoreFailure,
                Some(NotificationReasonCode::InvalidInput),
            ),
        };
        self.emit(&profile, Some(id), Some(&source), code, reason, 1);
        Ok(outcome)
    }

    fn validate_content(&mut self, content: &NotificationContent) -> Result<(), NotificationError> {
        if let NotificationContent::Localized(message) = content {
            self.adapters
                .validate_localized(message)
                .map_err(|_| NotificationError::LocalizationRejected)?;
        }
        Ok(())
    }

    fn load_snapshot(
        &mut self,
        profile: &A::Profile,
    ) -> Result<NotificationSnapshot<A::Source>, NotificationError> {
        self.ensure_writable(profile)?;
        let loaded = match self.persistence.load(profile) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.emit(
                    profile,
                    None,
                    None,
                    NotificationEventCode::StoreFailure,
                    Some(persistence_reason(error)),
                    1,
                );
                return Err(map_persistence_error(error));
            }
        };
        match loaded {
            StoreLoad::Empty => Ok(NotificationSnapshot::empty()),
            StoreLoad::Corrupt(_code) => {
                self.read_only_profiles.push(profile.clone());
                self.emit(
                    profile,
                    None,
                    None,
                    NotificationEventCode::StoreRecovered,
                    Some(NotificationReasonCode::CorruptState),
                    1,
                );
                Err(NotificationError::CorruptState)
            }
            StoreLoad::Snapshot(snapshot) => {
                if snapshot.schema_version() != NOTIFICATION_STORE_SCHEMA_VERSION {
                    if snapshot.schema_version() > NOTIFICATION_STORE_SCHEMA_VERSION {
                        self.read_only_profiles.push(profile.clone());
                        self.emit(
                            profile,
                            None,
                            None,
                            NotificationEventCode::StoreRecovered,
                            Some(NotificationReasonCode::UnsupportedSchema),
                            1,
                        );
                        return Err(NotificationError::UnsupportedSchema);
                    }
                    let old_generation = snapshot.generation();
                    let mut migrated = match self.adapters.migrate_snapshot(snapshot) {
                        Ok(migrated) => migrated,
                        Err(_) => {
                            self.read_only_profiles.push(profile.clone());
                            self.emit(
                                profile,
                                None,
                                None,
                                NotificationEventCode::StoreFailure,
                                Some(NotificationReasonCode::MigrationFailed),
                                1,
                            );
                            return Err(NotificationError::MigrationFailed);
                        }
                    };
                    if migrated.schema_version() != NOTIFICATION_STORE_SCHEMA_VERSION {
                        self.read_only_profiles.push(profile.clone());
                        self.emit(
                            profile,
                            None,
                            None,
                            NotificationEventCode::StoreRecovered,
                            Some(NotificationReasonCode::UnsupportedSchema),
                            1,
                        );
                        return Err(NotificationError::UnsupportedSchema);
                    }
                    let Some(next_generation) = old_generation.checked_add(1) else {
                        self.read_only_profiles.push(profile.clone());
                        self.emit(
                            profile,
                            None,
                            None,
                            NotificationEventCode::StoreFailure,
                            Some(NotificationReasonCode::GenerationExhausted),
                            1,
                        );
                        return Err(NotificationError::GenerationExhausted);
                    };
                    migrated.set_generation(next_generation);
                    self.validate_snapshot(profile, &migrated)?;
                    if let Err(error) =
                        self.persistence
                            .commit_atomic(profile, old_generation, migrated.clone())
                    {
                        self.read_only_profiles.push(profile.clone());
                        self.emit(
                            profile,
                            None,
                            None,
                            NotificationEventCode::StoreFailure,
                            Some(persistence_reason(error)),
                            1,
                        );
                        return Err(map_persistence_error(error));
                    }
                    self.emit(
                        profile,
                        None,
                        None,
                        NotificationEventCode::MigrationApplied,
                        None,
                        1,
                    );
                    Ok(migrated)
                } else {
                    self.validate_snapshot(profile, &snapshot)?;
                    Ok(snapshot)
                }
            }
        }
    }

    fn validate_snapshot(
        &mut self,
        profile: &A::Profile,
        snapshot: &NotificationSnapshot<A::Source>,
    ) -> Result<(), NotificationError> {
        let mut ids = BTreeSet::new();
        if snapshot.records().len() > self.quota.max_notifications_per_profile
            || snapshot.estimated_bytes() > self.quota.max_store_bytes_per_profile
            || snapshot.records().iter().any(|record| {
                record.estimated_bytes() > self.quota.max_notification_bytes
                    || !ids.insert(record.id)
            })
        {
            self.read_only_profiles.push(profile.clone());
            self.emit(
                profile,
                None,
                None,
                NotificationEventCode::StoreRecovered,
                Some(NotificationReasonCode::CorruptState),
                1,
            );
            return Err(NotificationError::CorruptState);
        }
        Ok(())
    }

    fn ensure_writable(&self, profile: &A::Profile) -> Result<(), NotificationError> {
        if self.read_only_profiles.contains(profile) {
            Err(NotificationError::RecoveryReadOnly)
        } else {
            Ok(())
        }
    }

    fn commit_next(
        &mut self,
        profile: &A::Profile,
        mut snapshot: NotificationSnapshot<A::Source>,
    ) -> Result<(), NotificationError> {
        let expected = snapshot.generation();
        let Some(next_generation) = expected.checked_add(1) else {
            self.emit(
                profile,
                None,
                None,
                NotificationEventCode::StoreFailure,
                Some(NotificationReasonCode::GenerationExhausted),
                1,
            );
            return Err(NotificationError::GenerationExhausted);
        };
        snapshot.set_schema_version(NOTIFICATION_STORE_SCHEMA_VERSION);
        snapshot.set_generation(next_generation);
        match self.persistence.commit_atomic(profile, expected, snapshot) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.emit(
                    profile,
                    None,
                    None,
                    NotificationEventCode::StoreFailure,
                    Some(persistence_reason(error)),
                    1,
                );
                Err(map_persistence_error(error))
            }
        }
    }

    fn make_room(
        &mut self,
        profile: &A::Profile,
        snapshot: &mut NotificationSnapshot<A::Source>,
        evicted: &mut Vec<NotificationId>,
    ) -> Result<bool, NotificationError> {
        let loaded = match self.persistence.load(profile) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.emit(
                    profile,
                    None,
                    None,
                    NotificationEventCode::StoreFailure,
                    Some(persistence_reason(error)),
                    1,
                );
                return Err(map_persistence_error(error));
            }
        };
        let (profile_exists, old_profile_bytes) = match loaded {
            StoreLoad::Empty => (false, 0),
            StoreLoad::Snapshot(current) => (true, current.estimated_bytes()),
            StoreLoad::Corrupt(_) => {
                self.read_only_profiles.push(profile.clone());
                return Err(NotificationError::CorruptState);
            }
        };
        let usage = self.persistence.usage();
        if !profile_exists && usage.profile_count >= self.quota.max_profiles {
            return Ok(false);
        }
        let bytes_elsewhere = usage.total_bytes.saturating_sub(old_profile_bytes);
        loop {
            let group_count = snapshot
                .records()
                .iter()
                .filter_map(|record| record.grouping_key.as_ref())
                .collect::<BTreeSet<_>>()
                .len();
            let profile_bytes = snapshot.estimated_bytes();
            let total_bytes = bytes_elsewhere.saturating_add(profile_bytes);
            let fits = snapshot.records().len() <= self.quota.max_notifications_per_profile
                && profile_bytes <= self.quota.max_store_bytes_per_profile
                && total_bytes <= self.quota.max_total_store_bytes
                && group_count <= self.quota.max_groups_per_profile;
            if fits {
                return Ok(true);
            }
            let Some(index) = eviction_candidate(snapshot.records()) else {
                return Ok(false);
            };
            evicted.push(snapshot.records_mut().remove(index).id);
        }
    }

    fn summary_for(
        &mut self,
        caller: &A::Caller,
        profile: &A::Profile,
        record: &NotificationRecord<A::Source>,
        boundary: RedactionBoundary,
        may_view_sensitive: bool,
    ) -> NotificationSummary {
        let source_attribution = self.redact_content(
            caller,
            profile,
            &record.source,
            boundary,
            &record.source.attribution,
            may_view_sensitive,
        );
        let title = self.redact_content(
            caller,
            profile,
            &record.source,
            boundary,
            &record.title,
            may_view_sensitive,
        );
        let body = self.redact_content(
            caller,
            profile,
            &record.source,
            boundary,
            &record.body,
            may_view_sensitive,
        );
        let actions = record
            .actions
            .iter()
            .cloned()
            .map(|action| {
                self.redact_action(
                    caller,
                    profile,
                    &record.source,
                    boundary,
                    &action,
                    may_view_sensitive,
                )
            })
            .collect();
        NotificationSummary {
            id: record.id,
            source_kind: record.source.kind,
            source_attribution,
            title,
            body,
            priority: record.priority,
            severity: record.severity,
            created_at: record.created_at,
            expires_at: record.expires_at,
            read_state: record.read_state,
            lifecycle: record.lifecycle,
            grouping_key: record.grouping_key.clone(),
            actions,
            delivery: record.delivery,
        }
    }

    fn sanitize_source(
        &mut self,
        caller: &A::Caller,
        profile: &A::Profile,
        source: &VerifiedSource<A::Source>,
    ) -> VerifiedSource<A::Source> {
        let attribution = self.redact_content(
            caller,
            profile,
            source,
            RedactionBoundary::Persistence,
            &source.attribution,
            false,
        );
        VerifiedSource::from_identity_adapter(
            source.identity.clone(),
            source.kind,
            attribution,
            source.diagnostic_reference.clone(),
        )
    }

    fn sanitize_content(
        &mut self,
        caller: &A::Caller,
        profile: &A::Profile,
        source: &VerifiedSource<A::Source>,
        boundary: RedactionBoundary,
        content: &NotificationContent,
        may_view_sensitive: bool,
    ) -> (NotificationContent, bool) {
        let result = self.redact_content(
            caller,
            profile,
            source,
            boundary,
            content,
            may_view_sensitive,
        );
        let changed = &result != content;
        (result, changed)
    }

    fn redact_content(
        &mut self,
        _caller: &A::Caller,
        _profile: &A::Profile,
        source: &VerifiedSource<A::Source>,
        boundary: RedactionBoundary,
        content: &NotificationContent,
        may_view_sensitive: bool,
    ) -> NotificationContent {
        match content {
            NotificationContent::Localized(message) => {
                let arguments = message
                    .arguments()
                    .iter()
                    .map(|argument| {
                        let mandatory = matches!(
                            boundary,
                            RedactionBoundary::Persistence | RedactionBoundary::Diagnostics
                        ) && argument.sensitivity() != Sensitivity::Public;
                        let capability_denied =
                            argument.sensitivity() != Sensitivity::Public && !may_view_sensitive;
                        let policy_redacted =
                            self.adapters
                                .must_redact(boundary, source, argument.sensitivity());
                        if mandatory || capability_denied || policy_redacted {
                            LocalizedArgument {
                                name: argument.name.clone(),
                                value: ArgumentValue::Redacted,
                                sensitivity: argument.sensitivity,
                            }
                        } else {
                            argument.clone()
                        }
                    })
                    .collect();
                NotificationContent::Localized(LocalizedMessage {
                    message_id: message.message_id.clone(),
                    arguments,
                })
            }
            NotificationContent::UserText { text, sensitivity } => {
                let mandatory = matches!(
                    boundary,
                    RedactionBoundary::Persistence | RedactionBoundary::Diagnostics
                ) && *sensitivity != Sensitivity::Public;
                let capability_denied = *sensitivity != Sensitivity::Public && !may_view_sensitive;
                let policy_redacted = self.adapters.must_redact(boundary, source, *sensitivity);
                if mandatory || capability_denied || policy_redacted {
                    NotificationContent::Redacted
                } else {
                    NotificationContent::UserText {
                        text: text.clone(),
                        sensitivity: *sensitivity,
                    }
                }
            }
            NotificationContent::Redacted => NotificationContent::Redacted,
        }
    }

    fn sanitize_action(
        &mut self,
        caller: &A::Caller,
        profile: &A::Profile,
        source: &VerifiedSource<A::Source>,
        boundary: RedactionBoundary,
        action: ActionDescriptor,
        may_view_sensitive: bool,
    ) -> (ActionDescriptor, bool) {
        let label = action.label.as_ref().and_then(|label| {
            let content = NotificationContent::Localized(label.clone());
            match self.redact_content(
                caller,
                profile,
                source,
                boundary,
                &content,
                may_view_sensitive,
            ) {
                NotificationContent::Localized(message) => Some(message),
                _ => None,
            }
        });
        let mut redacted = label != action.label;
        let parameters = action
            .parameters
            .iter()
            .map(|parameter| {
                let mandatory = matches!(
                    boundary,
                    RedactionBoundary::Persistence | RedactionBoundary::Diagnostics
                ) && parameter.sensitivity != Sensitivity::Public;
                let capability_denied =
                    parameter.sensitivity != Sensitivity::Public && !may_view_sensitive;
                let policy_redacted =
                    self.adapters
                        .must_redact(boundary, source, parameter.sensitivity);
                if mandatory || capability_denied || policy_redacted {
                    redacted = true;
                    ActionParameter {
                        name: parameter.name.clone(),
                        value: ActionValue::Redacted,
                        sensitivity: parameter.sensitivity,
                    }
                } else {
                    parameter.clone()
                }
            })
            .collect();
        (
            ActionDescriptor {
                id: action.id,
                label,
                parameters,
            },
            redacted,
        )
    }

    fn redact_action(
        &mut self,
        caller: &A::Caller,
        profile: &A::Profile,
        source: &VerifiedSource<A::Source>,
        boundary: RedactionBoundary,
        action: &ActionDescriptor,
        may_view_sensitive: bool,
    ) -> ActionDescriptor {
        self.sanitize_action(
            caller,
            profile,
            source,
            boundary,
            action.clone(),
            may_view_sensitive,
        )
        .0
    }

    fn render_content(
        &mut self,
        content: &NotificationContent,
        locale: &LocaleContext,
    ) -> Result<String, NotificationError> {
        let rendered = match content {
            NotificationContent::Localized(message) => {
                self.adapters.render_localized(message, locale)
            }
            NotificationContent::UserText { text, .. } => Ok(text.clone()),
            NotificationContent::Redacted => self.adapters.render_redacted(locale),
        }
        .map_err(|_| NotificationError::LocalizationRejected)?;
        if rendered.is_empty() || rendered.len() > MAX_CONTENT_BYTES {
            return Err(NotificationError::Validation(
                ValidationError::ContentTooLarge,
            ));
        }
        Ok(rendered)
    }

    fn emit(
        &mut self,
        profile: &A::Profile,
        id: Option<NotificationId>,
        source: Option<&VerifiedSource<A::Source>>,
        code: NotificationEventCode,
        reason: Option<NotificationReasonCode>,
        count: u32,
    ) {
        let now = self.clock.now();
        match self.diagnostics_window_start {
            Some(start) if now.0.saturating_sub(start.0) >= DIAGNOSTIC_WINDOW_MILLIS => {
                self.diagnostics_window_start = Some(now);
                self.diagnostics_events_in_window = 0;
            }
            None => self.diagnostics_window_start = Some(now),
            Some(_) => {}
        }
        if self.diagnostics_events_in_window >= MAX_DIAGNOSTIC_EVENTS_PER_WINDOW {
            self.diagnostics_dropped = self.diagnostics_dropped.saturating_add(1);
            return;
        }

        let correlation = self.adapters.profile_correlation(profile);
        if self.pending_sink_failure {
            let notice = NotificationDiagnosticEvent {
                code: NotificationEventCode::DiagnosticsSinkFailure,
                notification_id: None,
                source_reference: None,
                profile_correlation: correlation,
                reason: Some(NotificationReasonCode::DiagnosticsUnavailable),
                count: 1,
            };
            self.diagnostics_events_in_window += 1;
            match self.diagnostics.emit(notice) {
                Ok(()) => self.pending_sink_failure = false,
                Err(_) => {
                    self.diagnostics_failures = self.diagnostics_failures.saturating_add(1);
                    self.pending_sink_failure = true;
                }
            }
        }
        if self.diagnostics_events_in_window >= MAX_DIAGNOSTIC_EVENTS_PER_WINDOW {
            self.diagnostics_dropped = self.diagnostics_dropped.saturating_add(1);
            return;
        }
        if self.diagnostics_dropped > self.diagnostics_dropped_reported {
            let dropped = self
                .diagnostics_dropped
                .saturating_sub(self.diagnostics_dropped_reported);
            let notice = NotificationDiagnosticEvent {
                code: NotificationEventCode::DiagnosticsRateLimited,
                notification_id: None,
                source_reference: None,
                profile_correlation: GLOBAL_DIAGNOSTIC_CORRELATION,
                reason: Some(NotificationReasonCode::CapacityExceeded),
                count: dropped.min(u32::MAX as u64) as u32,
            };
            self.diagnostics_events_in_window += 1;
            if self.diagnostics.emit(notice).is_ok() {
                self.diagnostics_dropped_reported = self.diagnostics_dropped;
            } else {
                self.diagnostics_failures = self.diagnostics_failures.saturating_add(1);
                self.pending_sink_failure = true;
            }
        }
        if self.diagnostics_events_in_window >= MAX_DIAGNOSTIC_EVENTS_PER_WINDOW {
            self.diagnostics_dropped = self.diagnostics_dropped.saturating_add(1);
            return;
        }
        let event = NotificationDiagnosticEvent {
            code,
            notification_id: id,
            source_reference: source.map(|value| value.diagnostic_reference.clone()),
            profile_correlation: correlation,
            reason,
            count,
        };
        self.diagnostics_events_in_window += 1;
        if self.diagnostics.emit(event).is_err() {
            self.diagnostics_failures = self.diagnostics_failures.saturating_add(1);
            self.pending_sink_failure = true;
        }
    }
}

fn map_persistence_error(error: PersistenceError) -> NotificationError {
    match error {
        PersistenceError::Unavailable => NotificationError::PersistenceUnavailable,
        PersistenceError::Conflict => NotificationError::PersistenceConflict,
        PersistenceError::Corrupt => NotificationError::CorruptState,
        PersistenceError::QuotaExceeded => NotificationError::CapacityRejected,
    }
}

fn persistence_reason(error: PersistenceError) -> NotificationReasonCode {
    match error {
        PersistenceError::Unavailable => NotificationReasonCode::PersistenceUnavailable,
        PersistenceError::Conflict => NotificationReasonCode::PersistenceConflict,
        PersistenceError::Corrupt => NotificationReasonCode::CorruptState,
        PersistenceError::QuotaExceeded => NotificationReasonCode::CapacityExceeded,
    }
}

fn compare_records<S>(
    left: &NotificationRecord<S>,
    right: &NotificationRecord<S>,
) -> std::cmp::Ordering {
    left.grouping_key
        .cmp(&right.grouping_key)
        .then_with(|| right.priority.rank().cmp(&left.priority.rank()))
        .then_with(|| right.created_at.cmp(&left.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn eviction_candidate<S>(records: &[NotificationRecord<S>]) -> Option<usize> {
    let terminal = records
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            matches!(
                record.lifecycle,
                LifecycleState::Acknowledged | LifecycleState::Dismissed
            )
        })
        .min_by(|(_, left), (_, right)| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|(index, _)| index);
    terminal.or_else(|| {
        records
            .iter()
            .enumerate()
            .filter(|(_, record)| record.read_state == ReadState::Read)
            .min_by(|(_, left), (_, right)| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.id.cmp(&right.id))
            })
            .map(|(index, _)| index)
    })
}
