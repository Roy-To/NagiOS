use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use nagi_localization::{LocaleContext, LocaleId, MessageId};
use nagi_notification_core::*;

#[test]
fn notification_and_localized_ids_reject_malformed_values() {
    assert!(NotificationId::new([0; 16]).is_err());
    assert!(NotificationId::new([1; 16]).is_ok());
    assert!(MessageId::new("bad key").is_err());

    let key = MessageId::new("notify.test.title").unwrap();
    let argument =
        LocalizedArgument::new("count", ArgumentValue::Unsigned(2), Sensitivity::Public).unwrap();
    assert!(LocalizedMessage::new(key, vec![argument]).is_ok());
    assert!(LocalizedArgument::new(
        "bad name",
        ArgumentValue::Text("x".into()),
        Sensitivity::Public
    )
    .is_err());
    assert!(LocalizedMessage::new(
        MessageId::new("notify.test.title").unwrap(),
        vec![
            LocalizedArgument::new("count", ArgumentValue::Unsigned(1), Sensitivity::Public)
                .unwrap(),
            LocalizedArgument::new("count", ArgumentValue::Unsigned(2), Sensitivity::Public)
                .unwrap(),
        ],
    )
    .is_err());
    assert!(
        NotificationContent::user_text("x".repeat(MAX_CONTENT_BYTES + 1), Sensitivity::Public)
            .is_err()
    );
}

#[test]
fn malformed_action_descriptors_are_rejected_before_publish() {
    for invalid_id in ["", ".notes.open", "notes.", "Notes.open", "../notes"] {
        assert!(
            ActionId::new(invalid_id).is_err(),
            "accepted {invalid_id:?}"
        );
    }

    let action_id = ActionId::new("notes.open").unwrap();
    let duplicate_parameters = vec![
        ActionParameter::new("item", ActionValue::Unsigned(1), Sensitivity::Public).unwrap(),
        ActionParameter::new("item", ActionValue::Unsigned(2), Sensitivity::Public).unwrap(),
    ];
    assert!(matches!(
        ActionDescriptor::new(action_id.clone(), None, duplicate_parameters),
        Err(ValidationError::DuplicateArgument)
    ));

    let too_many_parameters = (0..=MAX_ACTION_PARAMETERS)
        .map(|index| {
            ActionParameter::new(
                format!("p{index}"),
                ActionValue::Boolean(true),
                Sensitivity::Public,
            )
            .unwrap()
        })
        .collect();
    assert!(matches!(
        ActionDescriptor::new(action_id.clone(), None, too_many_parameters),
        Err(ValidationError::TooManyActionParameters)
    ));

    let individually_bounded_but_oversized_descriptor = vec![
        ActionParameter::new(
            "first",
            ActionValue::Text("x".repeat(MAX_CONTENT_BYTES / 2 + 1)),
            Sensitivity::Public,
        )
        .unwrap(),
        ActionParameter::new(
            "second",
            ActionValue::Text("y".repeat(MAX_CONTENT_BYTES / 2 + 1)),
            Sensitivity::Public,
        )
        .unwrap(),
    ];
    assert!(matches!(
        ActionDescriptor::new(
            action_id,
            None,
            individually_bounded_but_oversized_descriptor
        ),
        Err(ValidationError::ContentTooLarge)
    ));
}

#[derive(Clone)]
struct TestCaller {
    profile: String,
}

#[derive(Clone)]
struct Shared(Rc<RefCell<MockState>>);

struct MockState {
    denied: Vec<NotificationOperation>,
    policy: Result<PolicyDecision, AdapterFailure>,
    source_available: bool,
    action_available: ActionAvailability,
    action_provider_calls: usize,
    view_sensitive: bool,
    redact_boundary: Option<RedactionBoundary>,
    reject_localization: bool,
    invalid_source_localization: bool,
    accepted_locales: BTreeSet<String>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            denied: Vec::new(),
            policy: Ok(PolicyDecision::Permit),
            source_available: true,
            action_available: ActionAvailability::Available,
            action_provider_calls: 0,
            view_sensitive: false,
            redact_boundary: None,
            reject_localization: false,
            invalid_source_localization: false,
            accepted_locales: BTreeSet::from([
                "notify.test.title".into(),
                "notify.test.body".into(),
                "notify.test.action".into(),
            ]),
        }
    }
}

impl Shared {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(MockState::default())))
    }
}

struct MockAdapters(Shared);

impl NotificationAdapters for MockAdapters {
    type Caller = TestCaller;
    type Profile = String;
    type Source = String;

    fn profile_scope(&mut self, caller: &Self::Caller) -> Result<Self::Profile, AdapterFailure> {
        if caller.profile.is_empty() {
            Err(AdapterFailure::Rejected)
        } else {
            Ok(caller.profile.clone())
        }
    }

    fn profile_correlation(&self, profile: &Self::Profile) -> SafeProfileCorrelation {
        let mut bytes = [0; 16];
        for (destination, source) in bytes.iter_mut().zip(profile.as_bytes()) {
            *destination = *source;
        }
        SafeProfileCorrelation::new(bytes)
    }

    fn authenticate_source(
        &mut self,
        _caller: &Self::Caller,
        source: &Self::Source,
    ) -> Result<VerifiedSource<Self::Source>, AdapterFailure> {
        if source.is_empty()
            || source.len() > 96
            || !source.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
            })
            || !source.contains('.')
        {
            return Err(AdapterFailure::Rejected);
        }
        let kind = if source.starts_with("service.") {
            SourceKind::SystemService
        } else {
            SourceKind::Application
        };
        let attribution = if self.0 .0.borrow().invalid_source_localization {
            NotificationContent::localized(
                LocalizedMessage::new(MessageId::new("notify.unknown.source").unwrap(), Vec::new())
                    .map_err(|_| AdapterFailure::Rejected)?,
            )
        } else {
            NotificationContent::user_text("Notes", Sensitivity::Public)
                .map_err(|_| AdapterFailure::Rejected)?
        };
        let diagnostic_reference =
            SafeSourceReference::new(source.clone()).map_err(|_| AdapterFailure::Rejected)?;
        Ok(VerifiedSource::from_identity_adapter(
            source.clone(),
            kind,
            attribution,
            diagnostic_reference,
        ))
    }

    fn revalidate_source(
        &mut self,
        _caller: &Self::Caller,
        _source: &VerifiedSource<Self::Source>,
    ) -> Result<(), AdapterFailure> {
        if self.0 .0.borrow().source_available {
            Ok(())
        } else {
            Err(AdapterFailure::StaleIdentity)
        }
    }

    fn authorize(
        &mut self,
        _caller: &Self::Caller,
        _profile: &Self::Profile,
        _source: Option<&VerifiedSource<Self::Source>>,
        operation: NotificationOperation,
    ) -> CapabilityDecision {
        let state = self.0 .0.borrow();
        if state.denied.contains(&operation) {
            CapabilityDecision::deny()
        } else {
            CapabilityDecision {
                allowed: true,
                may_view_sensitive: state.view_sensitive,
            }
        }
    }

    fn quiet_focus_decision(
        &mut self,
        _source: &VerifiedSource<Self::Source>,
        _priority: Priority,
        _severity: Severity,
        _now: Timestamp,
    ) -> Result<PolicyDecision, AdapterFailure> {
        self.0 .0.borrow().policy
    }

    fn validate_localized(&mut self, message: &LocalizedMessage) -> Result<(), AdapterFailure> {
        let state = self.0 .0.borrow();
        if state.reject_localization
            || !state
                .accepted_locales
                .contains(message.message_id().as_str())
        {
            Err(AdapterFailure::InvalidLocalization)
        } else {
            Ok(())
        }
    }

    fn render_localized(
        &mut self,
        message: &LocalizedMessage,
        locale: &LocaleContext,
    ) -> Result<String, AdapterFailure> {
        let mut rendered = format!(
            "{}@{}",
            message.message_id().as_str(),
            locale.system_language.as_str()
        );
        for argument in message.arguments() {
            let value = match argument.value() {
                ArgumentValue::Text(value) => value.clone(),
                ArgumentValue::Signed(value) => value.to_string(),
                ArgumentValue::Unsigned(value) => value.to_string(),
                ArgumentValue::Boolean(value) => value.to_string(),
                ArgumentValue::Redacted => "<redacted>".to_owned(),
            };
            rendered.push_str(&format!(" {}={value}", argument.name()));
        }
        Ok(rendered)
    }

    fn render_redacted(&mut self, locale: &LocaleContext) -> Result<String, AdapterFailure> {
        Ok(format!("redacted@{}", locale.system_language.as_str()))
    }

    fn must_redact(
        &mut self,
        boundary: RedactionBoundary,
        _source: &VerifiedSource<Self::Source>,
        _sensitivity: Sensitivity,
    ) -> bool {
        self.0 .0.borrow().redact_boundary == Some(boundary)
    }

    fn resolve_action_descriptor(
        &mut self,
        _source: &VerifiedSource<Self::Source>,
        _descriptor: &ActionDescriptor,
    ) -> Result<ActionAvailability, AdapterFailure> {
        let mut state = self.0 .0.borrow_mut();
        state.action_provider_calls += 1;
        Ok(state.action_available)
    }

    fn migrate_snapshot(
        &mut self,
        snapshot: NotificationSnapshot<Self::Source>,
    ) -> Result<NotificationSnapshot<Self::Source>, AdapterFailure> {
        if snapshot.schema_version() != 0 {
            return Err(AdapterFailure::MigrationFailed);
        }
        Ok(NotificationSnapshot::from_parts(
            NOTIFICATION_STORE_SCHEMA_VERSION,
            snapshot.generation(),
            snapshot.records().to_vec(),
        ))
    }
}

#[derive(Clone)]
struct FakeClock(Rc<Cell<u64>>);

impl FakeClock {
    fn new(now: u64) -> Self {
        Self(Rc::new(Cell::new(now)))
    }

    fn set(&self, now: u64) {
        self.0.set(now);
    }
}

impl NotificationClock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.get())
    }
}

struct SequenceIds(u128);

impl NotificationIdGenerator for SequenceIds {
    fn next_id(&mut self) -> Result<NotificationId, AdapterFailure> {
        let id = NotificationId::new(self.0.to_be_bytes()).map_err(|_| AdapterFailure::Rejected)?;
        self.0 += 1;
        Ok(id)
    }
}

type ObservedSnapshots = Rc<RefCell<Vec<(String, NotificationSnapshot<String>)>>>;

#[derive(Clone)]
struct StoreControl {
    fail_next_commit: Rc<Cell<bool>>,
    snapshots: ObservedSnapshots,
}

impl StoreControl {
    fn snapshot(&self, profile: &str) -> Option<NotificationSnapshot<String>> {
        self.snapshots
            .borrow()
            .iter()
            .find(|(existing, _)| existing == profile)
            .map(|(_, snapshot)| snapshot.clone())
    }
}

struct TestStore {
    inner: InMemoryNotificationPersistence<String, String>,
    control: StoreControl,
}

impl TestStore {
    fn new(quota: StoreQuota) -> Self {
        let control = StoreControl {
            fail_next_commit: Rc::new(Cell::new(false)),
            snapshots: Rc::new(RefCell::new(Vec::new())),
        };
        Self {
            inner: InMemoryNotificationPersistence::new(quota).unwrap(),
            control,
        }
    }

    fn sync_observed_snapshot(&self, profile: &String) {
        let mut snapshots = self.control.snapshots.borrow_mut();
        snapshots.retain(|(existing, _)| existing != profile);
        if let Some(snapshot) = self.inner.snapshot(profile) {
            snapshots.push((profile.clone(), snapshot.clone()));
        }
    }
}

impl NotificationPersistence<String, String> for TestStore {
    fn load(&mut self, profile: &String) -> Result<StoreLoad<String>, PersistenceError> {
        let result = self.inner.load(profile)?;
        self.sync_observed_snapshot(profile);
        Ok(result)
    }

    fn commit_atomic(
        &mut self,
        profile: &String,
        expected_generation: u64,
        next: NotificationSnapshot<String>,
    ) -> Result<(), PersistenceError> {
        if self.control.fail_next_commit.replace(false) {
            return Err(PersistenceError::Unavailable);
        }
        self.inner
            .commit_atomic(profile, expected_generation, next)?;
        self.sync_observed_snapshot(profile);
        Ok(())
    }

    fn usage(&self) -> StoreUsage {
        self.inner.usage()
    }
}

#[derive(Clone)]
struct DiagnosticsControl {
    events: Rc<RefCell<Vec<NotificationDiagnosticEvent>>>,
    fail: Rc<Cell<bool>>,
}

struct TestDiagnostics(DiagnosticsControl);

impl NotificationDiagnosticsSink for TestDiagnostics {
    fn emit(&mut self, event: NotificationDiagnosticEvent) -> Result<(), AdapterFailure> {
        if self.0.fail.get() {
            Err(AdapterFailure::Unavailable)
        } else {
            self.0.events.borrow_mut().push(event);
            Ok(())
        }
    }
}

type TestService =
    NotificationService<MockAdapters, TestStore, FakeClock, SequenceIds, TestDiagnostics>;

fn make_service(
    quota: StoreQuota,
) -> (
    TestService,
    Shared,
    FakeClock,
    StoreControl,
    DiagnosticsControl,
) {
    service_with(quota, PolicyFailureMode::Reject, TestStore::new(quota))
}

fn service_with(
    quota: StoreQuota,
    failure_mode: PolicyFailureMode,
    store: TestStore,
) -> (
    TestService,
    Shared,
    FakeClock,
    StoreControl,
    DiagnosticsControl,
) {
    let shared = Shared::new();
    let clock = FakeClock::new(100);
    let store_control = store.control.clone();
    let diagnostics_control = DiagnosticsControl {
        events: Rc::new(RefCell::new(Vec::new())),
        fail: Rc::new(Cell::new(false)),
    };
    let service = NotificationService::new(
        MockAdapters(shared.clone()),
        store,
        clock.clone(),
        SequenceIds(1),
        TestDiagnostics(diagnostics_control.clone()),
        quota,
        failure_mode,
    )
    .unwrap();
    (service, shared, clock, store_control, diagnostics_control)
}

fn caller(profile: &str) -> TestCaller {
    TestCaller {
        profile: profile.into(),
    }
}

fn public_text(text: &str) -> NotificationContent {
    NotificationContent::user_text(text, Sensitivity::Public).unwrap()
}

fn sensitive_text(text: &str) -> NotificationContent {
    NotificationContent::user_text(text, Sensitivity::Sensitive).unwrap()
}

fn localized(key: &str) -> NotificationContent {
    NotificationContent::localized(
        LocalizedMessage::new(MessageId::new(key).unwrap(), Vec::new()).unwrap(),
    )
}

fn action(id: &str) -> ActionDescriptor {
    ActionDescriptor::new(ActionId::new(id).unwrap(), None, Vec::new()).unwrap()
}

fn request(
    source: &str,
    title: NotificationContent,
    body: NotificationContent,
    priority: Priority,
    expires_at: Option<Timestamp>,
    grouping_key: Option<GroupingKey>,
    actions: Vec<ActionDescriptor>,
) -> PublishRequest<String> {
    PublishRequest::new(
        source.into(),
        title,
        body,
        priority,
        Severity::Informational,
        expires_at,
        grouping_key,
        actions,
    )
    .unwrap()
}

fn publish(
    service: &mut TestService,
    profile: &str,
    req: PublishRequest<String>,
) -> NotificationId {
    service.publish(&caller(profile), req).unwrap().id
}

fn query_active(service: &mut TestService, profile: &str) -> Vec<NotificationSummary> {
    service
        .query(
            &caller(profile),
            NotificationQuery {
                limit: 100,
                ..NotificationQuery::default()
            },
        )
        .unwrap()
}

fn locale() -> LocaleContext {
    LocaleContext::new(
        LocaleId::parse("ja-JP").unwrap(),
        LocaleId::parse("en-US").unwrap(),
    )
}

#[test]
fn source_identity_and_localization_adapters_reject_invalid_values() {
    let (mut service, shared, _, _, _) = make_service(StoreQuota::default());
    let invalid = PublishRequest::new(
        "../notes".into(),
        public_text("title"),
        public_text("body"),
        Priority::Normal,
        Severity::Informational,
        None,
        None,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        service.publish(&caller("A"), invalid),
        Err(NotificationError::SourceRejected)
    );

    shared.0.borrow_mut().reject_localization = true;
    let invalid_key = request(
        "app.notes",
        localized("notify.test.title"),
        public_text("body"),
        Priority::Normal,
        None,
        None,
        Vec::new(),
    );
    assert_eq!(
        service.publish(&caller("A"), invalid_key),
        Err(NotificationError::LocalizationRejected)
    );

    shared.0.borrow_mut().reject_localization = false;
    shared.0.borrow_mut().invalid_source_localization = true;
    assert_eq!(
        service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("title"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::LocalizationRejected)
    );
}

#[test]
fn localized_title_and_body_use_shared_message_ids_and_selected_locale_adapter() {
    let (mut service, _, _, _, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            localized("notify.test.title"),
            localized("notify.test.body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    let view = service.ui_view(&caller("A"), id, &locale()).unwrap();
    assert!(view.title.starts_with("notify.test.title@ja-JP"));
    assert!(view.body.starts_with("notify.test.body@ja-JP"));
}

#[test]
fn query_order_is_deterministic_by_group_then_priority_then_time_and_id() {
    let (mut service, _, clock, _, _) = make_service(StoreQuota::default());
    let z_group = GroupingKey::new("thread.z").unwrap();
    let a_group = GroupingKey::new("thread.a").unwrap();
    let z = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("z"),
            public_text("z"),
            Priority::Urgent,
            None,
            Some(z_group),
            Vec::new(),
        ),
    );
    clock.set(101);
    let a_normal = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("normal"),
            public_text("normal"),
            Priority::Normal,
            None,
            Some(a_group.clone()),
            Vec::new(),
        ),
    );
    clock.set(102);
    let a_urgent = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("urgent"),
            public_text("urgent"),
            Priority::Urgent,
            None,
            Some(a_group),
            Vec::new(),
        ),
    );

    let ids: Vec<_> = query_active(&mut service, "A")
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, [a_urgent, a_normal, z]);
}

#[test]
fn read_acknowledge_and_dismiss_transitions_are_idempotent_and_keep_audit_record() {
    let (mut service, _, _, store_control, _) = make_service(StoreQuota::default());
    let first = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("one"),
            public_text("one"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    let second = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("two"),
            public_text("two"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert_eq!(
        service.mark_read(&caller("A"), first).unwrap(),
        MutationOutcome::Changed
    );
    assert_eq!(
        service.mark_read(&caller("A"), first).unwrap(),
        MutationOutcome::Unchanged
    );
    assert_eq!(
        service.acknowledge(&caller("A"), first).unwrap(),
        MutationOutcome::Changed
    );
    assert_eq!(
        service.acknowledge(&caller("A"), first).unwrap(),
        MutationOutcome::Unchanged
    );
    assert_eq!(
        service.dismiss(&caller("A"), second).unwrap(),
        MutationOutcome::Changed
    );
    assert_eq!(
        service.dismiss(&caller("A"), second).unwrap(),
        MutationOutcome::Unchanged
    );

    let unread_only = service
        .query(
            &caller("A"),
            NotificationQuery {
                limit: 10,
                read_state: Some(ReadState::Unread),
                ..NotificationQuery::default()
            },
        )
        .unwrap();
    assert!(unread_only.is_empty());

    let snapshot = store_control.snapshot("A").unwrap();
    assert_eq!(snapshot.records().len(), 2);
    assert_eq!(snapshot.records()[0].read_state(), ReadState::Read);
    assert_eq!(
        snapshot.records()[0].lifecycle(),
        LifecycleState::Acknowledged
    );
    assert_eq!(snapshot.records()[1].lifecycle(), LifecycleState::Dismissed);
    let dismissed_query = service
        .query(
            &caller("A"),
            NotificationQuery {
                limit: 10,
                lifecycle: Some(LifecycleState::Dismissed),
                ..NotificationQuery::default()
            },
        )
        .unwrap();
    assert_eq!(dismissed_query.len(), 1);
}

#[test]
fn expiry_uses_injected_clock_and_excludes_and_cleans_at_the_boundary() {
    let (mut service, _, clock, store_control, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("body"),
            Priority::Normal,
            Some(Timestamp(110)),
            None,
            Vec::new(),
        ),
    );
    clock.set(109);
    assert_eq!(query_active(&mut service, "A").len(), 1);
    clock.set(110);
    assert!(query_active(&mut service, "A").is_empty());
    let cleaned = service.cleanup_expired(&caller("A")).unwrap();
    assert_eq!(cleaned.expired, [id]);
    assert!(store_control.snapshot("A").unwrap().records().is_empty());
}

#[test]
fn publish_evicts_expired_items_before_considering_read_or_unread_capacity() {
    let quota = StoreQuota {
        max_notifications_per_profile: 1,
        max_query_results: 1,
        ..StoreQuota::default()
    };
    let (mut service, _, clock, store_control, _) = make_service(quota);
    let expired = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("old"),
            public_text("body"),
            Priority::Normal,
            Some(Timestamp(101)),
            None,
            Vec::new(),
        ),
    );
    clock.set(101);
    let next = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("new"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    let snapshot = store_control.snapshot("A").unwrap();
    let records = snapshot.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id(), next);
    assert_ne!(records[0].id(), expired);
}

#[test]
fn profiles_are_isolated_for_query_update_and_action() {
    let (mut service, _, _, _, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "profile.a",
        request(
            "app.notes",
            public_text("private"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            vec![action("notes.open")],
        ),
    );
    assert!(query_active(&mut service, "profile.b").is_empty());
    assert_eq!(
        service.dismiss(&caller("profile.b"), id),
        Err(NotificationError::NotFound)
    );
    assert_eq!(
        service.mark_read(&caller("profile.b"), id),
        Err(NotificationError::NotFound)
    );
    assert_eq!(
        service.acknowledge(&caller("profile.b"), id),
        Err(NotificationError::NotFound)
    );
    assert_eq!(
        service.activate_action(
            &caller("profile.b"),
            id,
            &ActionId::new("notes.open").unwrap()
        ),
        Err(NotificationError::NotFound)
    );
    assert_eq!(
        query_active(&mut service, "profile.a")[0].read_state,
        ReadState::Unread
    );
    assert_eq!(
        service.dismiss(&caller("profile.a"), id).unwrap(),
        MutationOutcome::Changed
    );
}

#[test]
fn action_resolution_revalidates_source_and_permission_without_executing_payload() {
    let (mut service, shared, _, _, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            vec![action("notes.open")],
        ),
    );
    let action_id = ActionId::new("notes.open").unwrap();

    shared
        .0
        .borrow_mut()
        .denied
        .push(NotificationOperation::ResolveAction);
    assert_eq!(
        service.activate_action(&caller("A"), id, &action_id),
        Err(NotificationError::PermissionDenied)
    );
    assert_eq!(shared.0.borrow().action_provider_calls, 0);
    shared.0.borrow_mut().denied.clear();

    shared.0.borrow_mut().source_available = false;
    assert_eq!(
        service.activate_action(&caller("A"), id, &action_id),
        Err(NotificationError::SourceRejected)
    );
    assert_eq!(shared.0.borrow().action_provider_calls, 0);
    shared.0.borrow_mut().source_available = true;

    shared.0.borrow_mut().action_available = ActionAvailability::Unavailable;
    assert_eq!(
        service.activate_action(&caller("A"), id, &action_id),
        Err(NotificationError::ActionUnavailable)
    );
    shared.0.borrow_mut().action_available = ActionAvailability::Available;
    assert_eq!(
        service
            .activate_action(&caller("A"), id, &action_id)
            .unwrap()
            .availability,
        ActionAvailability::Available
    );
    assert_eq!(shared.0.borrow().action_provider_calls, 2);
}

#[test]
fn quiet_focus_policy_permit_defer_suppress_and_provider_failure_are_explicit() {
    let (mut service, shared, clock, _, _) = make_service(StoreQuota::default());
    shared.0.borrow_mut().policy = Ok(PolicyDecision::DeferUntil(Timestamp(120)));
    let deferred = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("later"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert!(query_active(&mut service, "A").is_empty());
    clock.set(120);
    assert_eq!(query_active(&mut service, "A")[0].id, deferred);

    shared.0.borrow_mut().policy = Ok(PolicyDecision::SuppressWithRetention(Timestamp(130)));
    let suppressed = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("hidden"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert!(!query_active(&mut service, "A")
        .iter()
        .any(|item| item.id == suppressed));
    clock.set(130);
    assert!(service
        .cleanup_expired(&caller("A"))
        .unwrap()
        .expired
        .contains(&suppressed));

    shared.0.borrow_mut().policy = Err(AdapterFailure::Unavailable);
    let (mut reject_service, reject_shared, _, _, _) = make_service(StoreQuota::default());
    reject_shared.0.borrow_mut().policy = Err(AdapterFailure::Unavailable);
    assert_eq!(
        reject_service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("fail"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::PolicyUnavailable)
    );

    let quota = StoreQuota::default();
    let (mut defer_service, defer_shared, defer_clock, defer_store_control, _) = service_with(
        quota,
        PolicyFailureMode::DeferForMillis(25),
        TestStore::new(quota),
    );
    defer_shared.0.borrow_mut().policy = Err(AdapterFailure::Unavailable);
    let id = publish(
        &mut defer_service,
        "A",
        request(
            "app.notes",
            public_text("retry later"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert_eq!(
        defer_store_control.snapshot("A").unwrap().records()[0].delivery(),
        DeliveryState::Deferred {
            until: Timestamp(125)
        }
    );
    assert!(query_active(&mut defer_service, "A").is_empty());
    defer_clock.set(125);
    assert_eq!(query_active(&mut defer_service, "A")[0].id, id);
}

#[test]
fn sensitive_content_is_redacted_before_storage_query_ui_export_and_diagnostics() {
    let (mut service, _, _, store_control, diagnostics) = make_service(StoreQuota::default());
    let secret = "private-account-value-42";
    let sensitive_action = ActionDescriptor::new(
        ActionId::new("notes.open").unwrap(),
        None,
        vec![ActionParameter::new(
            "account",
            ActionValue::Text(secret.into()),
            Sensitivity::Sensitive,
        )
        .unwrap()],
    )
    .unwrap();
    let sensitive_title = NotificationContent::localized(
        LocalizedMessage::new(
            MessageId::new("notify.test.title").unwrap(),
            vec![LocalizedArgument::new(
                "account",
                ArgumentValue::Text(secret.into()),
                Sensitivity::Sensitive,
            )
            .unwrap()],
        )
        .unwrap(),
    );
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            sensitive_title,
            sensitive_text(secret),
            Priority::Normal,
            None,
            None,
            vec![sensitive_action],
        ),
    );
    let snapshot = store_control.snapshot("A").unwrap();
    let record = snapshot
        .records()
        .iter()
        .find(|record| record.id() == id)
        .unwrap();
    assert_eq!(record.body(), &NotificationContent::Redacted);
    assert!(
        matches!(record.title(), NotificationContent::Localized(message) if message.arguments()[0].value() == &ArgumentValue::Redacted)
    );
    assert_eq!(
        record.actions()[0].parameters()[0].value(),
        &ActionValue::Redacted
    );

    let queried = service
        .query(&caller("A"), NotificationQuery::default())
        .unwrap();
    assert_eq!(queried[0].body, NotificationContent::Redacted);
    let ui = service.ui_view(&caller("A"), id, &locale()).unwrap();
    assert_eq!(ui.body, "redacted@ja-JP");
    let exported = service.export(&caller("A"), id).unwrap();
    assert_eq!(exported.body, NotificationContent::Redacted);
    let event_log = format!("{:?}", diagnostics.events.borrow());
    assert!(!event_log.contains(secret));
}

#[test]
fn every_non_public_sensitivity_class_is_redacted_before_storage() {
    let (mut service, _, _, store_control, _) = make_service(StoreQuota::default());
    let classes = [
        Sensitivity::Personal,
        Sensitivity::Sensitive,
        Sensitivity::Secret,
    ];
    let ids: Vec<_> = classes
        .into_iter()
        .enumerate()
        .map(|(index, sensitivity)| {
            publish(
                &mut service,
                "A",
                request(
                    "app.notes",
                    NotificationContent::user_text(format!("title-{index}"), sensitivity).unwrap(),
                    NotificationContent::user_text(format!("body-{index}"), sensitivity).unwrap(),
                    Priority::Normal,
                    None,
                    None,
                    Vec::new(),
                ),
            )
        })
        .collect();
    let snapshot = store_control.snapshot("A").unwrap();
    for id in &ids {
        let record = snapshot
            .records()
            .iter()
            .find(|record| record.id() == *id)
            .unwrap();
        assert_eq!(record.title(), &NotificationContent::Redacted);
        assert_eq!(record.body(), &NotificationContent::Redacted);
    }
}

#[test]
fn redaction_policy_can_only_restrict_query_ui_and_export_views() {
    let (mut service, shared, _, _, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("visible"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    shared.0.borrow_mut().redact_boundary = Some(RedactionBoundary::Query);
    assert_eq!(
        service
            .query(&caller("A"), NotificationQuery::default())
            .unwrap()[0]
            .body,
        NotificationContent::Redacted
    );
    shared.0.borrow_mut().redact_boundary = Some(RedactionBoundary::Ui);
    assert_eq!(
        service.ui_view(&caller("A"), id, &locale()).unwrap().body,
        "redacted@ja-JP"
    );
    shared.0.borrow_mut().redact_boundary = Some(RedactionBoundary::Export);
    assert_eq!(
        service.export(&caller("A"), id).unwrap().body,
        NotificationContent::Redacted
    );
}

#[test]
fn denied_capability_fails_closed_for_publish_list_read_and_dismiss() {
    let (mut service, shared, _, _, _) = make_service(StoreQuota::default());
    shared
        .0
        .borrow_mut()
        .denied
        .push(NotificationOperation::Publish);
    assert_eq!(
        service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("title"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::PermissionDenied)
    );
    shared.0.borrow_mut().denied.clear();
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    shared
        .0
        .borrow_mut()
        .denied
        .push(NotificationOperation::List);
    assert_eq!(
        service.query(&caller("A"), NotificationQuery::default()),
        Err(NotificationError::PermissionDenied)
    );
    shared.0.borrow_mut().denied.clear();
    shared
        .0
        .borrow_mut()
        .denied
        .push(NotificationOperation::Read);
    assert!(query_active(&mut service, "A").is_empty());
    shared.0.borrow_mut().denied.clear();
    shared
        .0
        .borrow_mut()
        .denied
        .push(NotificationOperation::Dismiss);
    assert_eq!(
        service.dismiss(&caller("A"), id),
        Err(NotificationError::PermissionDenied)
    );
}

#[test]
fn failed_atomic_update_preserves_previous_snapshot_and_operation_result() {
    let (mut service, _, _, store_control, _) = make_service(StoreQuota::default());
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    store_control.fail_next_commit.set(true);
    assert_eq!(
        service.dismiss(&caller("A"), id),
        Err(NotificationError::PersistenceUnavailable)
    );
    let record = store_control.snapshot("A").unwrap().records()[0].clone();
    assert_eq!(record.lifecycle(), LifecycleState::Active);
    assert_eq!(record.read_state(), ReadState::Unread);
}

#[test]
fn corrupt_store_recovers_to_read_only_empty_state_without_inventing_records() {
    let quota = StoreQuota::default();
    let mut store = TestStore::new(quota);
    store
        .inner
        .inject_corruption("A".into(), CorruptionCode::MalformedSnapshot);
    let (mut service, _, _, _, _) = service_with(quota, PolicyFailureMode::Reject, store);
    assert_eq!(
        service.query(&caller("A"), NotificationQuery::default()),
        Err(NotificationError::CorruptState)
    );
    assert_eq!(
        service.query(&caller("A"), NotificationQuery::default()),
        Err(NotificationError::RecoveryReadOnly)
    );
    assert_eq!(
        service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("title"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::RecoveryReadOnly)
    );
}

#[test]
fn version_zero_snapshot_migrates_atomically_and_unknown_version_fails_closed() {
    let quota = StoreQuota::default();
    let mut store = TestStore::new(quota);
    store.inner.inject_snapshot(
        "A".into(),
        NotificationSnapshot::from_parts(0, 0, Vec::new()),
    );
    let (mut service, _, _, store_control, _) =
        service_with(quota, PolicyFailureMode::Reject, store);
    assert!(query_active(&mut service, "A").is_empty());
    let migrated = store_control.snapshot("A").unwrap();
    assert_eq!(migrated.schema_version(), NOTIFICATION_STORE_SCHEMA_VERSION);
    assert_eq!(migrated.generation(), 1);

    let mut store = TestStore::new(quota);
    store.inner.inject_snapshot(
        "B".into(),
        NotificationSnapshot::from_parts(99, 0, Vec::new()),
    );
    let (mut service, _, _, _, _) = service_with(quota, PolicyFailureMode::Reject, store);
    assert_eq!(
        service.query(&caller("B"), NotificationQuery::default()),
        Err(NotificationError::UnsupportedSchema)
    );
    assert_eq!(
        service.query(&caller("B"), NotificationQuery::default()),
        Err(NotificationError::RecoveryReadOnly)
    );
}

#[test]
fn generation_exhaustion_never_reuses_a_revision_or_changes_the_snapshot() {
    let quota = StoreQuota::default();
    let mut store = TestStore::new(quota);
    store.inner.inject_snapshot(
        "A".into(),
        NotificationSnapshot::from_parts(NOTIFICATION_STORE_SCHEMA_VERSION, u64::MAX, Vec::new()),
    );
    let (mut service, _, _, store_control, _) =
        service_with(quota, PolicyFailureMode::Reject, store);
    assert!(query_active(&mut service, "A").is_empty());
    assert_eq!(
        service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("title"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::GenerationExhausted)
    );
    let snapshot = store_control.snapshot("A").unwrap();
    assert_eq!(snapshot.generation(), u64::MAX);
    assert!(snapshot.records().is_empty());

    let mut store = TestStore::new(quota);
    store.inner.inject_snapshot(
        "B".into(),
        NotificationSnapshot::from_parts(0, u64::MAX, Vec::new()),
    );
    let (mut service, _, _, store_control, _) =
        service_with(quota, PolicyFailureMode::Reject, store);
    assert_eq!(
        service.query(&caller("B"), NotificationQuery::default()),
        Err(NotificationError::GenerationExhausted)
    );
    assert_eq!(
        service.query(&caller("B"), NotificationQuery::default()),
        Err(NotificationError::RecoveryReadOnly)
    );
    let snapshot = store_control.snapshot("B").unwrap();
    assert_eq!(snapshot.schema_version(), 0);
    assert_eq!(snapshot.generation(), u64::MAX);
}

#[test]
fn capacity_never_evicts_unread_and_evicts_oldest_read_after_terminal_items() {
    let quota = StoreQuota {
        max_notifications_per_profile: 2,
        max_query_results: 2,
        ..StoreQuota::default()
    };
    let (mut service, _, clock, store_control, _) = make_service(quota);
    let first = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("first"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    clock.set(101);
    let second = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("second"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    clock.set(102);
    assert_eq!(
        service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("third"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );
    assert_eq!(store_control.snapshot("A").unwrap().records().len(), 2);
    service.mark_read(&caller("A"), first).unwrap();
    clock.set(103);
    let third = service
        .publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("third"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new(),
            ),
        )
        .unwrap();
    assert_eq!(third.evicted, [first]);
    assert_eq!(
        service
            .query(
                &caller("A"),
                NotificationQuery {
                    limit: 2,
                    ..NotificationQuery::default()
                }
            )
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [third.id, second]
    );

    let (mut terminal_service, _, _, _, _) = make_service(quota);
    let acknowledged = publish(
        &mut terminal_service,
        "A",
        request(
            "app.notes",
            public_text("ack"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    terminal_service
        .acknowledge(&caller("A"), acknowledged)
        .unwrap();
    let _unread = publish(
        &mut terminal_service,
        "A",
        request(
            "app.notes",
            public_text("unread"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    let result = terminal_service
        .publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("new"),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new(),
            ),
        )
        .unwrap();
    assert_eq!(result.evicted, [acknowledged]);
}

#[test]
fn group_byte_profile_and_query_quotas_are_bounded_and_report_rejection() {
    let group_quota = StoreQuota {
        max_groups_per_profile: 1,
        ..StoreQuota::default()
    };
    let (mut group_service, _, _, _, _) = make_service(group_quota);
    publish(
        &mut group_service,
        "A",
        request(
            "app.notes",
            public_text("a"),
            public_text("body"),
            Priority::Normal,
            None,
            Some(GroupingKey::new("group.a").unwrap()),
            Vec::new(),
        ),
    );
    assert_eq!(
        group_service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("b"),
                public_text("body"),
                Priority::Normal,
                None,
                Some(GroupingKey::new("group.b").unwrap()),
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );

    let item_quota = StoreQuota {
        max_notification_bytes: 95,
        ..StoreQuota::default()
    };
    let (mut item_service, _, _, _, _) = make_service(item_quota);
    assert_eq!(
        item_service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("a"),
                public_text("b"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );

    let store_quota = StoreQuota {
        max_store_bytes_per_profile: 95,
        ..StoreQuota::default()
    };
    let (mut store_service, _, _, _, _) = make_service(store_quota);
    assert_eq!(
        store_service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("a"),
                public_text("b"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );

    let action_quota = StoreQuota {
        max_actions_per_notification: 0,
        ..StoreQuota::default()
    };
    let (mut action_service, _, _, _, _) = make_service(action_quota);
    assert_eq!(
        action_service.publish(
            &caller("A"),
            request(
                "app.notes",
                public_text("a"),
                public_text("b"),
                Priority::Normal,
                None,
                None,
                vec![action("notes.open")]
            )
        ),
        Err(NotificationError::Validation(
            ValidationError::TooManyActions
        ))
    );

    let profile_quota = StoreQuota {
        max_profiles: 1,
        ..StoreQuota::default()
    };
    let (mut profile_service, _, _, _, _) = make_service(profile_quota);
    publish(
        &mut profile_service,
        "A",
        request(
            "app.notes",
            public_text("a"),
            public_text("b"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert_eq!(
        profile_service.publish(
            &caller("B"),
            request(
                "app.notes",
                public_text("b"),
                public_text("b"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );

    let result_quota = StoreQuota {
        max_query_results: 1,
        ..StoreQuota::default()
    };
    let (mut result_service, _, _, _, _) = make_service(result_quota);
    assert_eq!(
        result_service.query(
            &caller("A"),
            NotificationQuery {
                limit: 2,
                ..NotificationQuery::default()
            }
        ),
        Err(NotificationError::Validation(
            ValidationError::InvalidQueryLimit
        ))
    );
}

#[test]
fn total_store_bytes_are_bounded_without_eviction_across_profiles() {
    let quota = StoreQuota {
        max_total_store_bytes: 120,
        ..StoreQuota::default()
    };
    let (mut service, _, _, store_control, _) = make_service(quota);
    publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("a"),
            public_text("b"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert_eq!(
        service.publish(
            &caller("B"),
            request(
                "app.notes",
                public_text("b"),
                public_text("b"),
                Priority::Normal,
                None,
                None,
                Vec::new()
            )
        ),
        Err(NotificationError::CapacityRejected)
    );
    assert!(store_control.snapshot("A").is_some());
    assert!(store_control.snapshot("B").is_none());
}

#[test]
fn diagnostics_sink_failure_does_not_change_publish_or_store_results() {
    let (mut service, _, _, store_control, diagnostics) = make_service(StoreQuota::default());
    diagnostics.fail.set(true);
    let id = publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("title"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert_eq!(service.diagnostics_failures(), 3);
    assert_eq!(store_control.snapshot("A").unwrap().records()[0].id(), id);
    diagnostics.fail.set(false);
    service.mark_read(&caller("A"), id).unwrap();
    assert!(diagnostics
        .events
        .borrow()
        .iter()
        .any(|event| event.code == NotificationEventCode::DiagnosticsSinkFailure));
}

#[test]
fn diagnostics_events_are_rate_limited_to_a_bounded_window() {
    let (mut service, _, clock, _, diagnostics) = make_service(StoreQuota::default());
    for index in 0..70 {
        publish(
            &mut service,
            "A",
            request(
                "app.notes",
                public_text(&format!("title-{index}")),
                public_text("body"),
                Priority::Normal,
                None,
                None,
                Vec::new(),
            ),
        );
    }
    assert_eq!(
        diagnostics.events.borrow().len(),
        MAX_DIAGNOSTIC_EVENTS_PER_WINDOW as usize
    );
    clock.set(100 + DIAGNOSTIC_WINDOW_MILLIS);
    publish(
        &mut service,
        "A",
        request(
            "app.notes",
            public_text("next window"),
            public_text("body"),
            Priority::Normal,
            None,
            None,
            Vec::new(),
        ),
    );
    assert!(diagnostics.events.borrow().iter().any(|event| {
        event.code == NotificationEventCode::DiagnosticsRateLimited
            && event.count == 12
            && event.profile_correlation == SafeProfileCorrelation::new([0; 16])
    }));
}
