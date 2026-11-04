use nagi_automation_core::*;

fn approval_run_ref(value: &str) -> ApprovalRunRef {
    ApprovalRunRef::new(value).unwrap()
}

fn step_id(value: &str) -> StepId {
    StepId::new(value).unwrap()
}

fn step_result(run_ref: &ApprovalRunRef, step_id: &StepId, outcome: StepOutcome) -> StepResult {
    StepResult {
        approval_run_ref: run_ref.clone(),
        step_id: step_id.clone(),
        outcome,
    }
}

#[test]
fn run_summary_requires_this_approval_run_and_exact_unique_registered_step_order() {
    let run_ref = approval_run_ref("approval/run-1");
    let other_run_ref = approval_run_ref("approval/run-2");
    let first = step_id("step/1");
    let second = step_id("step/2");
    let expected = vec![first.clone(), second.clone()];
    let success = vec![
        step_result(&run_ref, &first, StepOutcome::Succeeded),
        step_result(&run_ref, &second, StepOutcome::Succeeded),
    ];

    assert_eq!(
        summarize_run(&run_ref, &expected, &success),
        Ok(RunStatus::Success)
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                step_result(&run_ref, &first, StepOutcome::Succeeded),
                step_result(&run_ref, &first, StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::DuplicateStepResult { index: 1 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                step_result(&run_ref, &first, StepOutcome::Succeeded),
                step_result(&run_ref, &step_id("unregistered"), StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::UnregisteredStep { index: 1 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                step_result(&run_ref, &second, StepOutcome::Succeeded),
                step_result(&run_ref, &first, StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::OutOfOrderStepResult { index: 0 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[step_result(&other_run_ref, &first, StepOutcome::Succeeded)]
        ),
        Err(SummaryError::WrongApprovalRunRef { index: 0 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[step_result(&run_ref, &first, StepOutcome::Succeeded)]
        ),
        Ok(RunStatus::Pending)
    );
    assert_eq!(
        summarize_run(&run_ref, &[first.clone(), first], &[]),
        Err(SummaryError::DuplicateExpectedStep { index: 1 })
    );
}

#[test]
fn structural_summary_errors_precede_recovery_required_outcomes() {
    let run_ref = approval_run_ref("approval/run-structural");
    let other_run_ref = approval_run_ref("approval/run-other");
    let first = step_id("step/1");
    let second = step_id("step/2");
    let expected = vec![first.clone(), second.clone()];
    let recovery_first = step_result(&run_ref, &first, StepOutcome::RecoveryRequired);

    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                recovery_first.clone(),
                step_result(&other_run_ref, &second, StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::WrongApprovalRunRef { index: 1 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                recovery_first.clone(),
                step_result(&run_ref, &step_id("unregistered"), StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::UnregisteredStep { index: 1 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                recovery_first,
                step_result(&run_ref, &first, StepOutcome::Succeeded),
            ]
        ),
        Err(SummaryError::DuplicateStepResult { index: 1 })
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[step_result(
                &run_ref,
                &second,
                StepOutcome::RecoveryRequired,
            )]
        ),
        Err(SummaryError::OutOfOrderStepResult { index: 0 })
    );
}

#[test]
fn recovery_required_precedence_covers_every_pair_of_step_outcomes() {
    let run_ref = approval_run_ref("approval/run-3");
    let first = step_id("step/1");
    let second = step_id("step/2");
    let expected = vec![first.clone(), second.clone()];

    assert_eq!(summarize_run(&run_ref, &[], &[]), Ok(RunStatus::Skipped));
    assert_eq!(
        summarize_run(&run_ref, &expected, &[]),
        Ok(RunStatus::Pending)
    );
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[step_result(&run_ref, &first, StepOutcome::RecoveryRequired)]
        ),
        Ok(RunStatus::RecoveryRequired)
    );

    let outcomes = [
        StepOutcome::Succeeded,
        StepOutcome::Failed,
        StepOutcome::Skipped,
        StepOutcome::Pending,
        StepOutcome::Cancelled,
        StepOutcome::TimedOut,
        StepOutcome::RecoveryRequired,
    ];
    let expected_status = [
        [
            RunStatus::Success,
            RunStatus::Partial,
            RunStatus::Partial,
            RunStatus::Pending,
            RunStatus::Partial,
            RunStatus::Partial,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::Partial,
            RunStatus::Failed,
            RunStatus::Failed,
            RunStatus::Pending,
            RunStatus::Failed,
            RunStatus::Failed,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::Partial,
            RunStatus::Failed,
            RunStatus::Skipped,
            RunStatus::Pending,
            RunStatus::Cancelled,
            RunStatus::TimedOut,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::Pending,
            RunStatus::Pending,
            RunStatus::Pending,
            RunStatus::Pending,
            RunStatus::Pending,
            RunStatus::Pending,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::Partial,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Pending,
            RunStatus::Cancelled,
            RunStatus::Failed,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::Partial,
            RunStatus::Failed,
            RunStatus::TimedOut,
            RunStatus::Pending,
            RunStatus::Failed,
            RunStatus::TimedOut,
            RunStatus::RecoveryRequired,
        ],
        [
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
            RunStatus::RecoveryRequired,
        ],
    ];

    for (first_index, first_outcome) in outcomes.iter().copied().enumerate() {
        for (second_index, second_outcome) in outcomes.iter().copied().enumerate() {
            assert_eq!(
                summarize_run(
                    &run_ref,
                    &expected,
                    &[
                        step_result(&run_ref, &first, first_outcome),
                        step_result(&run_ref, &second, second_outcome),
                    ]
                ),
                Ok(expected_status[first_index][second_index]),
                "outcomes={first_outcome:?}+{second_outcome:?}"
            );
        }
    }

    // Preserve the explicit contract: skipped work is not a successful step.
    assert_eq!(
        summarize_run(
            &run_ref,
            &expected,
            &[
                step_result(&run_ref, &first, StepOutcome::Succeeded),
                step_result(&run_ref, &second, StepOutcome::Skipped),
            ]
        ),
        Ok(RunStatus::Partial)
    );
}

#[test]
fn core_step_ids_keep_their_grammar_and_scheduler_ids_require_explicit_validation() {
    let slash_id = step_id("run/step:1");
    assert_eq!(slash_id.as_str(), "run/step:1");
    assert_eq!(
        SchedulerJobId::new(slash_id.as_str()),
        Err(IdentifierError::InvalidCharacter)
    );
    assert!(ApprovalRunRef::new("_approval/run").is_ok());
    assert_eq!(
        SchedulerJobId::new("_not-a-job"),
        Err(IdentifierError::InvalidStart)
    );

    let max_core_id = "a".repeat(MAX_IDENTIFIER_BYTES);
    assert!(StepId::new(max_core_id.clone()).is_ok());
    assert_eq!(
        SchedulerJobId::new(&max_core_id),
        Err(IdentifierError::TooLong)
    );
    assert_eq!(
        StepId::new("a".repeat(MAX_IDENTIFIER_BYTES + 1)),
        Err(IdentifierError::TooLong)
    );

    let max_job_id = "a".repeat(MAX_JOB_IDENTIFIER_BYTES);
    assert_eq!(
        SchedulerJobId::new(&max_job_id).unwrap().as_str(),
        max_job_id
    );
    assert_eq!(
        SchedulerJobId::new(&"a".repeat(MAX_JOB_IDENTIFIER_BYTES + 1)),
        Err(IdentifierError::TooLong)
    );
}
