use nagi_automation_core::*;
use std::cell::Cell;

struct Boundary {
    calls: Cell<usize>,
    effect: Effects,
    restricted: bool,
}
impl ActionBoundary<String, Vec<u64>> for Boundary {
    fn inspect(&self, action: &String, scope: &Vec<u64>) -> Result<ActionInfo, BoundaryError> {
        self.calls.set(self.calls.get() + 1);
        if action != "notes.create" {
            return Err(BoundaryError::UnknownAction);
        }
        if scope != &[7] {
            return Err(BoundaryError::Denied);
        }
        Ok(ActionInfo {
            action_id: action.clone(),
            effects: self.effect,
            permissions: vec!["notes.write".into()],
            resources: vec![7],
            scope_restricted: self.restricted,
        })
    }
}
fn boundary() -> Boundary {
    Boundary {
        calls: Cell::new(0),
        effect: Effects::Local,
        restricted: true,
    }
}
fn draft() -> Draft<String, Vec<u64>> {
    Draft {
        enabled: true,
        trigger: Trigger::Manual,
        conditions: vec![],
        actions: vec!["notes.create".into()],
        scope: vec![7],
        error_policy: ErrorPolicy {
            failure: FailurePolicy::Stop,
            retries: 0,
            notify_user: true,
        },
    }
}
#[test]
fn unknown_action_is_rejected() {
    let mut d = draft();
    d.actions[0] = "unknown".into();
    assert_eq!(
        preview(&d, &boundary()).unwrap_err(),
        ValidationError::Action {
            index: 0,
            reason: BoundaryError::UnknownAction
        }
    );
}
#[test]
fn oversized_workflow_and_retry_are_rejected_before_adapter_calls() {
    let b = boundary();
    let mut d = draft();
    d.actions = vec!["notes.create".into(); MAX_ACTIONS + 1];
    assert_eq!(preview(&d, &b).unwrap_err(), ValidationError::ActionCount);
    d.actions.truncate(1);
    d.error_policy.retries = MAX_RETRIES + 1;
    assert_eq!(preview(&d, &b).unwrap_err(), ValidationError::RetryLimit);
    assert_eq!(b.calls.get(), 0);
}
#[test]
fn denied_scope_is_not_previewed_as_authorized() {
    let mut d = draft();
    d.scope.clear();
    assert_eq!(
        preview(&d, &boundary()).unwrap_err(),
        ValidationError::Action {
            index: 0,
            reason: BoundaryError::Denied
        }
    );
}
#[test]
fn disabled_request_short_circuits_and_preview_is_read_only() {
    let b = boundary();
    let mut d = draft();
    d.enabled = false;
    assert_eq!(request_run_once(&d, &b).unwrap(), Admission::Disabled);
    assert_eq!(b.calls.get(), 0);
    let p = preview(&d, &b).unwrap();
    assert_eq!(p.draft, &d);
    assert_eq!(p.actions[0].permissions, ["notes.write"]);
    assert_eq!(p.actions[0].resources, [7]);
    assert_eq!(b.calls.get(), 1);
    // The only injected API is inspect; no execution hook exists.
}
#[test]
fn risky_effects_never_become_ready_and_require_restricted_scope() {
    for effect in [
        Effects::External,
        Effects::Destructive,
        Effects::ExternalAndDestructive,
    ] {
        let mut b = boundary();
        b.effect = effect;
        assert!(preview(&draft(), &b).unwrap().confirmation_required);
        assert_eq!(
            request_run_once(&draft(), &b).unwrap(),
            Admission::PendingConfirmation
        );
        b.restricted = false;
        assert_eq!(
            preview(&draft(), &b).unwrap_err(),
            ValidationError::ScopeRestrictionRequired { index: 0 }
        );
    }
}
#[test]
fn incomplete_failed_and_pending_runs_never_succeed() {
    assert_eq!(
        summarize_run(2, &[StepOutcome::Succeeded]),
        RunStatus::Pending
    );
    assert_eq!(summarize_run(1, &[StepOutcome::Failed]), RunStatus::Failed);
    assert_eq!(
        summarize_run(2, &[StepOutcome::Succeeded, StepOutcome::Failed]),
        RunStatus::Partial
    );
    assert_eq!(
        summarize_run(1, &[StepOutcome::Pending]),
        RunStatus::Pending
    );
    assert_eq!(summarize_run(0, &[]), RunStatus::Skipped);
}

#[test]
fn multi_action_preview_preserves_order_and_complete_draft() {
    let mut d = draft();
    d.actions.push("notes.create".into());
    d.trigger = Trigger::Scheduled { unix_seconds: 42 };
    d.conditions.push(Condition {
        key: "workspace".into(),
        value: "work".into(),
    });
    d.error_policy = ErrorPolicy {
        failure: FailurePolicy::ContinueSafeSteps,
        retries: MAX_RETRIES,
        notify_user: false,
    };
    let b = boundary();
    let p = preview(&d, &b).unwrap();
    assert_eq!(p.actions.len(), 2);
    assert_eq!(
        p.actions
            .iter()
            .map(|a| a.action_id.as_str())
            .collect::<Vec<_>>(),
        ["notes.create", "notes.create"]
    );
    assert_eq!(p.draft, &d);
    assert!(!p.confirmation_required);
    assert_eq!(b.calls.get(), 2);
    assert_eq!(
        request_run_once(&d, &b).unwrap(),
        Admission::ReadyForPlatformValidation
    );
    d.enabled = false;
    assert_eq!(request_run_once(&d, &b).unwrap(), Admission::Disabled);
    d.enabled = true;
    assert_eq!(
        request_run_once(&d, &b).unwrap(),
        Admission::ReadyForPlatformValidation
    );
}
#[test]
fn malformed_trigger_and_conditions_fail_before_action_inspection() {
    let b = boundary();
    let mut d = draft();
    for trigger in [
        Trigger::Interval { seconds: 0 },
        Trigger::PlatformEvent { event: "".into() },
        Trigger::DeviceState {
            device: "device".into(),
            state: "bad\nstate".into(),
        },
    ] {
        d.trigger = trigger;
        assert_eq!(
            preview(&d, &b).unwrap_err(),
            ValidationError::InvalidTrigger
        );
    }
    d.trigger = Trigger::Manual;
    for (key, value) in [
        ("".to_string(), "valid".to_string()),
        ("workspace".into(), " ".into()),
        ("workspace".into(), "bad\0value".into()),
        (
            "workspace".into(),
            "x".repeat(MAX_CONDITION_VALUE_BYTES + 1),
        ),
    ] {
        d.conditions = vec![Condition { key, value }];
        assert_eq!(
            preview(&d, &b).unwrap_err(),
            ValidationError::InvalidCondition { index: 0 }
        );
    }
    d.conditions = vec![
        Condition {
            key: "workspace".into(),
            value: "work".into()
        };
        MAX_CONDITIONS + 1
    ];
    assert_eq!(
        preview(&d, &b).unwrap_err(),
        ValidationError::ConditionCount
    );
    d.conditions.clear();
    d.actions.clear();
    assert_eq!(preview(&d, &b).unwrap_err(), ValidationError::ActionCount);
    assert_eq!(b.calls.get(), 0);
}
#[test]
fn each_structured_trigger_is_described_without_scheduling() {
    for trigger in [
        Trigger::Manual,
        Trigger::Scheduled { unix_seconds: 42 },
        Trigger::Interval { seconds: 60 },
        Trigger::PlatformEvent {
            event: "files.changed".into(),
        },
        Trigger::ResourceChange { resource: 7 },
        Trigger::DeviceState {
            device: "pc".into(),
            state: "online".into(),
        },
    ] {
        let mut d = draft();
        d.trigger = trigger.clone();
        assert_eq!(preview(&d, &boundary()).unwrap().draft.trigger, trigger);
    }
}
struct MetadataBoundary(ActionInfo);
impl ActionBoundary<String, Vec<u64>> for MetadataBoundary {
    fn inspect(&self, _: &String, _: &Vec<u64>) -> Result<ActionInfo, BoundaryError> {
        Ok(self.0.clone())
    }
}
#[test]
fn malformed_or_oversized_injected_metadata_is_rejected() {
    let valid = boundary()
        .inspect(&"notes.create".into(), &vec![7])
        .unwrap();
    let mut cases = vec![];
    let mut info = valid.clone();
    info.action_id.clear();
    cases.push(info);
    let mut info = valid.clone();
    info.action_id = "x".repeat(MAX_IDENTIFIER_BYTES + 1);
    cases.push(info);
    let mut info = valid.clone();
    info.permissions = vec!["notes.write".into(); MAX_PERMISSIONS + 1];
    cases.push(info);
    let mut info = valid.clone();
    info.permissions = vec!["forged\npermission".into()];
    cases.push(info);
    let mut info = valid.clone();
    info.resources = vec![7; MAX_RESOURCES + 1];
    cases.push(info);
    for info in cases {
        assert_eq!(
            preview(&draft(), &MetadataBoundary(info)).unwrap_err(),
            ValidationError::InvalidMetadata { index: 0 }
        );
    }
    let mut info = valid;
    info.permissions = vec!["notes.write".into(); MAX_PERMISSIONS];
    info.resources = vec![7; MAX_RESOURCES];
    assert!(preview(&draft(), &MetadataBoundary(info)).is_ok());
}
#[test]
fn later_unknown_action_cannot_produce_partial_preview() {
    let mut d = draft();
    d.actions.push("unknown".into());
    assert_eq!(
        preview(&d, &boundary()).unwrap_err(),
        ValidationError::Action {
            index: 1,
            reason: BoundaryError::UnknownAction
        }
    );
}
#[test]
fn maximum_workflow_is_accepted_and_disabled_malformed_drafts_are_inert() {
    let b = boundary();
    let mut d = draft();
    d.actions = vec!["notes.create".into(); MAX_ACTIONS];
    assert_eq!(preview(&d, &b).unwrap().actions.len(), MAX_ACTIONS);
    d.enabled = false;
    d.actions.push("unknown".into());
    assert_eq!(request_run_once(&d, &b).unwrap(), Admission::Disabled);
    assert_eq!(b.calls.get(), MAX_ACTIONS);
}
#[test]
fn success_requires_exactly_one_successful_final_outcome_per_expected_step() {
    let choices = [
        StepOutcome::Succeeded,
        StepOutcome::Failed,
        StepOutcome::Skipped,
        StepOutcome::Pending,
    ];
    for expected in 0..=4 {
        for len in 0..=5 {
            for mut bits in 0..4_usize.pow(len as u32) {
                let outcomes: Vec<_> = (0..len)
                    .map(|_| {
                        let o = choices[bits % 4];
                        bits /= 4;
                        o
                    })
                    .collect();
                let should_succeed = expected > 0
                    && len == expected
                    && outcomes.iter().all(|o| *o == StepOutcome::Succeeded);
                assert_eq!(
                    summarize_run(expected, &outcomes) == RunStatus::Success,
                    should_succeed,
                    "expected={expected} outcomes={outcomes:?}"
                );
            }
        }
    }
    assert_eq!(summarize_run(MAX_ACTIONS + 1, &[]), RunStatus::Failed);
    assert_eq!(
        summarize_run(2, &[StepOutcome::Skipped, StepOutcome::Skipped]),
        RunStatus::Skipped
    );
    assert_eq!(
        summarize_run(2, &[StepOutcome::Succeeded, StepOutcome::Skipped]),
        RunStatus::Partial
    );
}
