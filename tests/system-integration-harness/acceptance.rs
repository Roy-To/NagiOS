use nagi_test_harness::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn harness(test_id: &str) -> Harness {
    HarnessBuilder::new(test_id)
        .build()
        .expect("valid harness configuration")
}

fn service_id(value: &str) -> ServiceId {
    ServiceId::new(value).unwrap()
}

fn principal(value: &str) -> PrincipalId {
    PrincipalId::new(value).unwrap()
}

#[test]
fn virtual_clock_orders_due_work_and_cancels_without_sleeping() {
    let harness = harness("clock-order");
    let later = harness.clock.schedule(9, "later").unwrap();
    let first = harness.clock.schedule(4, "first").unwrap();
    let cancelled = harness.clock.schedule(4, "cancelled").unwrap();
    assert!(harness.clock.cancel(cancelled));
    assert!(!harness.clock.cancel(cancelled));

    assert_eq!(harness.advance_time(4).unwrap(), 4);
    assert_eq!(
        harness
            .clock
            .drain_due()
            .into_iter()
            .map(|event| event.label)
            .collect::<Vec<_>>(),
        ["first"]
    );
    assert_eq!(harness.clock.now(), 4);
    assert_eq!(harness.clock.pending_timers(), 1);
    assert_eq!(harness.advance_time(5).unwrap(), 9);
    let events = harness.clock.drain_due();
    assert_eq!(
        events,
        [ScheduledEvent {
            id: later,
            due_at: 9,
            label: "later".to_owned()
        }]
    );
    assert_eq!(harness.clock.pending_timers(), 0);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Timers), 0);
    let _ = first;
}

#[test]
fn fake_filesystem_is_confined_bounded_and_injects_atomic_and_corrupt_writes() {
    let harness = harness("filesystem-failures");
    let root = harness.filesystem.temporary_root().unwrap();
    root.write_atomic("state/current", b"committed").unwrap();
    assert_eq!(root.read("state/current").unwrap().as_slice(), b"committed");
    assert_eq!(
        root.write_atomic("../outside", b"no"),
        Err(HarnessError::PathEscapesRoot)
    );
    assert_eq!(
        root.write_atomic("/absolute", b"no"),
        Err(HarnessError::InvalidPath)
    );

    harness.failures.arm(Failpoint::InterruptedWrite, 1);
    assert_eq!(
        root.write_atomic("state/current", b"partial"),
        Err(HarnessError::Injected(Failpoint::InterruptedWrite))
    );
    assert_eq!(root.read("state/current").unwrap().as_slice(), b"committed");

    harness.failures.arm(Failpoint::CorruptState, 1);
    assert_eq!(
        root.write_atomic("state/current", b"\x00corrupt\xff"),
        Err(HarnessError::CorruptStateInjected)
    );
    assert_eq!(
        root.read("state/current").unwrap().as_slice(),
        b"\x00corrupt\xff"
    );
}

#[test]
fn fake_filesystem_rejects_quota_before_storing_oversized_input() {
    let limits = ResourceLimits::default()
        .set(ResourceKind::Bytes, 8)
        .set(ResourceKind::Files, 2);
    let harness = HarnessBuilder::new("filesystem-quota")
        .resource_limits(limits)
        .build()
        .unwrap();
    let root = harness.filesystem.temporary_root().unwrap();
    assert_eq!(
        root.write_atomic("too-large", b"123456789"),
        Err(HarnessError::QuotaExceeded(ResourceKind::Bytes))
    );
    assert_eq!(root.file_count(), 0);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Bytes), 0);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Files), 0);
    harness.failures.arm(Failpoint::ResourceExhaustion, 1);
    assert!(matches!(
        harness.resources.acquire(ResourceKind::Tasks, 1),
        Err(HarnessError::QuotaExceeded(ResourceKind::Tasks))
    ));
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Tasks), 0);
}

#[test]
fn user_profile_session_roots_are_isolated_and_cleanup_is_verifiable() {
    let harness = harness("fixture-isolation");
    let before = harness.filesystem.root_count();
    let first = harness.user_profile_fixture().unwrap();
    let second = harness.user_profile_fixture().unwrap();
    assert_ne!(first.ids().user_id, second.ids().user_id);
    let first_root = first.root();
    let second_root = second.root();
    first_root
        .write_atomic("profile/preferences", b"first-user")
        .unwrap();
    assert!(matches!(
        second_root.read("profile/preferences"),
        Err(HarnessError::NotFound)
    ));
    drop(first_root);
    drop(second_root);
    let first_report = first.teardown();
    let second_report = second.teardown();
    assert!(first_report.root_removed && second_report.root_removed);
    assert_eq!(first_report.files_removed, 1);
    assert_eq!(harness.filesystem.root_count(), before);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Files), 0);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Bytes), 0);
}

#[test]
fn ipc_fake_enforces_allowlists_and_covers_timeout_cancel_malformed_provider_and_queue_faults() {
    let harness = HarnessBuilder::new("ipc-failure-matrix")
        .ipc_queue_capacity(2)
        .build()
        .unwrap();
    let sender = principal("test-client");
    let receiver = service_id("settings-service");
    harness
        .bus
        .allow_route(sender.clone(), receiver.clone())
        .unwrap();
    let message = |correlation: &str| {
        BusMessage::new(
            sender.clone(),
            receiver.clone(),
            correlation,
            100,
            b"request".to_vec(),
        )
    };

    let denied = BusMessage::new(
        principal("other-client"),
        receiver.clone(),
        "denied",
        100,
        vec![],
    );
    assert_eq!(
        harness.bus.send(denied, None),
        Err(HarnessError::Unauthorized)
    );
    assert!(matches!(
        harness.bus.send(message("ok"), None),
        Ok(BusOutcome::Queued { copies: 1, .. })
    ));
    assert_eq!(harness.bus.receive(&receiver).unwrap().correlation_id, "ok");

    let mut malformed = message("bad-version");
    malformed.protocol_version = 2;
    assert_eq!(
        harness.bus.send(malformed, None),
        Err(HarnessError::MalformedMessage)
    );
    harness.failures.arm(Failpoint::Timeout, 1);
    assert_eq!(
        harness.bus.send(message("injected-timeout"), None),
        Err(HarnessError::DeadlineExpired)
    );
    harness.failures.arm(Failpoint::MalformedMessage, 1);
    assert_eq!(
        harness.bus.send(message("injected-malformed"), None),
        Err(HarnessError::MalformedMessage)
    );
    harness.failures.arm(Failpoint::Cancellation, 1);
    assert_eq!(
        harness.bus.send(message("injected-cancel"), None),
        Err(HarnessError::Cancelled)
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        harness.bus.send(message("cancelled"), Some(&cancelled)),
        Err(HarnessError::Cancelled)
    );
    harness.bus.fail_provider(receiver.clone());
    assert_eq!(
        harness.bus.send(message("provider-down"), None),
        Err(HarnessError::ProviderFailed)
    );
    harness.bus.recover_provider(&receiver);
    harness.failures.arm(Failpoint::ProviderFailure, 1);
    assert_eq!(
        harness.bus.send(message("injected-provider-failure"), None),
        Err(HarnessError::ProviderFailed)
    );

    harness.bus.set_next_fault(BusFault::DropNext);
    assert_eq!(
        harness.bus.send(message("drop"), None),
        Ok(BusOutcome::Dropped)
    );
    assert_eq!(harness.bus.queued(), 0);
    harness.bus.set_next_fault(BusFault::DuplicateNext);
    assert!(matches!(
        harness.bus.send(message("duplicate"), None),
        Ok(BusOutcome::Queued { copies: 2, .. })
    ));
    assert_eq!(
        harness.bus.send(message("full"), None),
        Err(HarnessError::QueueFull)
    );
    assert_eq!(
        harness.bus.receive(&receiver).unwrap().correlation_id,
        "duplicate"
    );
    assert_eq!(
        harness.bus.receive(&receiver).unwrap().correlation_id,
        "duplicate"
    );

    harness.bus.send(message("older"), None).unwrap();
    harness.bus.set_next_fault(BusFault::ReorderNextToFront);
    harness.bus.send(message("front"), None).unwrap();
    assert_eq!(
        harness.bus.receive(&receiver).unwrap().correlation_id,
        "front"
    );
    assert_eq!(
        harness.bus.receive(&receiver).unwrap().correlation_id,
        "older"
    );
    harness.bus.send(message("cancel-queued"), None).unwrap();
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Messages), 1);
    assert_eq!(harness.bus.cancel_correlation("cancel-queued"), 1);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Messages), 0);

    harness.advance_time(100).unwrap();
    assert_eq!(
        harness.bus.send(message("expired"), None),
        Err(HarnessError::DeadlineExpired)
    );
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Messages), 0);
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Bytes), 0);
}

#[test]
fn capability_broker_is_default_deny_and_requires_exact_context() {
    let harness = harness("capability-default-deny");
    let owner = principal("fixture-owner");
    let mut broker = harness.capabilities.clone();
    broker
        .add_rule(CapabilityRule::new(
            owner.clone(),
            "read",
            "profile:self",
            true,
        ))
        .unwrap();
    broker
        .add_rule(CapabilityRule::new(
            owner.clone(),
            "delete",
            "profile:self",
            false,
        ))
        .unwrap();
    assert_eq!(
        broker.check(None, "read", "profile:self"),
        Err(HarnessError::MissingContext)
    );
    assert_eq!(broker.check(Some(&owner), "read", "profile:self"), Ok(()));
    assert_eq!(
        broker.check(Some(&owner), "delete", "profile:self"),
        Err(HarnessError::Unauthorized)
    );
    assert_eq!(
        broker.check(Some(&owner), "read", "profile:other"),
        Err(HarnessError::Unauthorized)
    );
}

#[test]
fn diagnostics_are_ordered_redacted_bounded_and_best_effort() {
    let harness = HarnessBuilder::new("diagnostics-redaction")
        .diagnostics_capacity(2)
        .build()
        .unwrap();
    let mut secret_fields = BTreeMap::new();
    secret_fields.insert(
        "password".to_owned(),
        "correct horse battery staple".to_owned(),
    );
    secret_fields.insert(
        "detail".to_owned(),
        "authorization=Bearer private-value".to_owned(),
    );
    assert!(harness
        .diagnostics
        .record(DiagnosticKind::TestStarted, None, "setup", secret_fields));
    harness.advance_time(3).unwrap();
    assert!(!harness.diagnostics.record(
        DiagnosticKind::TestEnded,
        None,
        "overflow",
        BTreeMap::new()
    ));
    let events = harness.diagnostics.events();
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(events[0].fields["password"], "[REDACTED]");
    assert_eq!(events[0].fields["detail"], "[REDACTED]");
    assert_eq!(events[1].virtual_time, 3);
    assert!(harness.diagnostics.truncated());
    harness.diagnostics.set_sink_enabled(false);
    assert!(!harness.diagnostics.record(
        DiagnosticKind::Custom("ignored".to_owned()),
        None,
        "disabled",
        BTreeMap::new()
    ));
}

#[test]
fn services_start_in_dependency_order_stop_in_reverse_and_report_partial_cleanup() {
    struct RecordingService {
        name: &'static str,
        actions: Arc<Mutex<Vec<String>>>,
        fail_start: bool,
        fail_stop: bool,
    }
    impl TestService for RecordingService {
        fn start(&mut self) -> Result<()> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("start:{}", self.name));
            if self.fail_start {
                Err(HarnessError::ServiceFailure("start failed".to_owned()))
            } else {
                Ok(())
            }
        }
        fn health_check(&mut self) -> Result<()> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("health:{}", self.name));
            Ok(())
        }
        fn stop(&mut self) -> Result<()> {
            self.actions
                .lock()
                .unwrap()
                .push(format!("stop:{}", self.name));
            if self.fail_stop {
                Err(HarnessError::ServiceFailure("stop failed".to_owned()))
            } else {
                Ok(())
            }
        }
    }

    let mut harness = harness("service-order");
    let actions = Arc::new(Mutex::new(Vec::new()));
    let factory =
        |name, fail_start, fail_stop, actions: Arc<Mutex<Vec<String>>>| -> ServiceFactory {
            Arc::new(move || {
                Box::new(RecordingService {
                    name,
                    actions: actions.clone(),
                    fail_start,
                    fail_stop,
                })
            })
        };
    harness
        .services
        .register(
            service_id("base"),
            [],
            factory("base", false, false, actions.clone()),
        )
        .unwrap();
    harness
        .services
        .register(
            service_id("api"),
            [service_id("base")],
            factory("api", false, false, actions.clone()),
        )
        .unwrap();
    harness
        .services
        .register(
            service_id("ui"),
            [service_id("api")],
            factory("ui", false, false, actions.clone()),
        )
        .unwrap();
    let start = harness.services.start_all().unwrap();
    assert_eq!(
        start
            .started
            .iter()
            .map(|handle| handle.service.as_str())
            .collect::<Vec<_>>(),
        ["base", "api", "ui"]
    );
    let stop = harness.services.stop_all().unwrap();
    assert!(stop.cleanup_complete);
    assert_eq!(
        stop.stopped
            .iter()
            .map(|handle| handle.service.as_str())
            .collect::<Vec<_>>(),
        ["ui", "api", "base"]
    );
    let events = actions.lock().unwrap().clone();
    assert_eq!(&events[..3], ["start:base", "health:base", "start:api"]);
    assert_eq!(
        &events[events.len() - 3..],
        ["stop:ui", "stop:api", "stop:base"]
    );

    let mut failing =
        ServiceHarness::new(harness.resources.clone(), harness.failures.clone(), None);
    let cleanup = Arc::new(Mutex::new(Vec::new()));
    failing
        .register(
            service_id("ready"),
            [],
            factory("ready", false, false, cleanup.clone()),
        )
        .unwrap();
    failing
        .register(
            service_id("broken"),
            [service_id("ready")],
            factory("broken", true, true, cleanup.clone()),
        )
        .unwrap();
    let report = failing.start_all().unwrap();
    assert!(report
        .primary_error
        .as_ref()
        .unwrap()
        .contains("start failed"));
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("failed-start cleanup") && error.contains("stop failed")));
    assert!(!report.cleanup_complete);
    assert_eq!(report.started.len(), 1);
    assert_eq!(
        report
            .stopped
            .iter()
            .map(|handle| handle.service.as_str())
            .collect::<Vec<_>>(),
        ["ready"]
    );
    assert_eq!(failing.running_count(), 0);
}

#[test]
fn crash_restart_changes_instance_identity_and_named_failpoints_are_repeatable() {
    struct OkService;
    impl TestService for OkService {
        fn start(&mut self) -> Result<()> {
            Ok(())
        }
        fn health_check(&mut self) -> Result<()> {
            Ok(())
        }
        fn stop(&mut self) -> Result<()> {
            Ok(())
        }
    }
    let mut harness = harness("service-crash-restart");
    harness
        .services
        .register(service_id("store"), [], Arc::new(|| Box::new(OkService)))
        .unwrap();
    let before = harness.services.start_all().unwrap().started.remove(0);
    harness.failures.arm(Failpoint::ServiceCrash, 1);
    assert_eq!(harness.services.crash(&before.service).unwrap(), before);
    assert!(!harness.services.is_current(&before));
    harness.failures.arm(Failpoint::RestartFailure, 1);
    assert_eq!(
        harness.services.restart(&before.service),
        Err(HarnessError::Injected(Failpoint::RestartFailure))
    );
    let after = harness.services.restart(&before.service).unwrap();
    assert_ne!(before.instance_id, after.instance_id);
    assert!(!harness.services.is_current(&before));
    assert!(harness.services.is_current(&after));
    harness.services.stop_all().unwrap();
    assert_eq!(harness.resources.snapshot().used(ResourceKind::Services), 0);
}

#[test]
fn offline_policy_never_opens_network_and_resource_leases_report_and_clear_leaks() {
    let harness = harness("offline-and-leak-report");
    assert_eq!(
        harness.network.request(OfflineRequest {
            destination: "https://example.invalid".to_owned(),
            purpose: "test attempt".to_owned()
        }),
        Err(HarnessError::OfflineNetworkDenied),
    );
    assert_eq!(harness.network.attempts().len(), 1);

    let lease = harness.resources.acquire(ResourceKind::Tasks, 2).unwrap();
    assert_eq!(
        harness
            .resources
            .leak_report()
            .leaked
            .get(&ResourceKind::Tasks),
        Some(&2)
    );
    drop(lease);
    harness.diagnostics.clear();
    assert!(harness.resources.leak_report().is_clean());

    let report = harness.result("complete", None, true);
    assert!(report
        .disclaimer
        .contains("do not confer provider, target, or product acceptance"));
    assert_eq!(report.virtual_time, 0);
}
