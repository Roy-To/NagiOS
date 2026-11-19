//! Task-local command surface for the recovery screen.
//!
//! This module only translates visible user actions into recovery-state
//! commands. It does not reopen devices, mint reconnect proofs, request input
//! grants, retry automatically, or choose a save destination. The caller must
//! keep the root `RecoveryOwner` alive; this short-lived borrow cannot take or
//! discard the draft by being dropped.

use crate::input_recovery::{
    RecoveryHost, RecoveryInput, RecoveryNotice, RecoveryOwner, RecoveryPhase, RecoveryStep,
};
use std::time::Instant;

/// Map one retained recovery notice to its existing localized framebuffer
/// status. Keeping this mapping beside the UI command adapter makes new notice
/// variants exhaustive before they reach the real presentation host.
pub(crate) fn status_for_notice(notice: &RecoveryNotice) -> (&'static str, Option<&'static str>) {
    match notice {
        RecoveryNotice::InitialInputSaveFailed { .. } => (
            "desktop.notes.recovery.retry_failed",
            Some("desktop.notes.recovery.reconnect_lost_hint"),
        ),
        RecoveryNotice::InputReconnected { .. } => (
            "desktop.notes.recovery.ready",
            Some("desktop.notes.recovery.ready_hint"),
        ),
        RecoveryNotice::InputReconnectUnavailable { .. } => (
            "desktop.notes.recovery.reconnect_lost",
            Some("desktop.notes.recovery.reconnect_lost_hint"),
        ),
        RecoveryNotice::RetrySaved => ("desktop.notes.recovery.saved", None),
        RecoveryNotice::RetryFailed(_) => (
            "desktop.notes.recovery.retry_failed",
            Some("desktop.notes.recovery.retry_failed_hint"),
        ),
        RecoveryNotice::DraftDiscarded => ("desktop.notes.recovery.discarded", None),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryUiCommand {
    RetryNotification,
    RetrySave,
    BeginDiscard,
    ConfirmDiscard,
    CancelDiscard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryUiStatus {
    pub phase: RecoveryPhase,
    pub input_error: String,
    pub save_error: String,
    pub initial_notification_error: Option<String>,
    pub last_notification_error: Option<String>,
    pub last_reconnect_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidRecoveryUiCommand {
    pub phase: RecoveryPhase,
    pub command: RecoveryUiCommand,
}

/// A short-lived view of the commands available for one UI event.
///
/// The confirmation phase lives in `RecoveryOwner`, so dropping this view or
/// returning early cannot turn a pending discard into an actual discard.
pub(crate) struct RecoveryUiCommandAdapter<'a, T: Send + 'static> {
    owner: &'a mut RecoveryOwner<T>,
}

impl<'a, T: Send + 'static> RecoveryUiCommandAdapter<'a, T> {
    pub(crate) fn borrow(owner: &'a mut RecoveryOwner<T>) -> Self {
        Self { owner }
    }

    pub(crate) fn status(&self) -> RecoveryUiStatus {
        RecoveryUiStatus {
            phase: self.owner.phase(),
            input_error: self.owner.input_error().to_owned(),
            save_error: self.owner.last_save_error().to_owned(),
            initial_notification_error: self.owner.initial_notification_error().map(str::to_owned),
            last_notification_error: self.owner.last_notification_error().map(str::to_owned),
            last_reconnect_error: self.owner.last_reconnect_error().map(str::to_owned),
        }
    }

    pub(crate) fn available_commands(&self) -> &'static [RecoveryUiCommand] {
        match self.owner.phase() {
            RecoveryPhase::WaitingForInput => {
                if self.owner.notification_retry_available() {
                    &[RecoveryUiCommand::RetryNotification]
                } else {
                    &[]
                }
            }
            RecoveryPhase::Saved | RecoveryPhase::Discarded => &[],
            RecoveryPhase::NotificationUnavailable => &[RecoveryUiCommand::RetryNotification],
            RecoveryPhase::AwaitingUserChoice => &[
                RecoveryUiCommand::RetrySave,
                RecoveryUiCommand::BeginDiscard,
            ],
            RecoveryPhase::ConfirmingDiscard => &[
                RecoveryUiCommand::ConfirmDiscard,
                RecoveryUiCommand::CancelDiscard,
            ],
        }
    }

    pub(crate) fn apply<H: RecoveryHost<T>>(
        &mut self,
        command: RecoveryUiCommand,
        host: &mut H,
    ) -> Result<RecoveryStep, InvalidRecoveryUiCommand> {
        let phase = self.owner.phase();
        if !self.available_commands().contains(&command) {
            return Err(InvalidRecoveryUiCommand { phase, command });
        }

        let input = match command {
            RecoveryUiCommand::RetryNotification => RecoveryInput::RetryNotification,
            RecoveryUiCommand::RetrySave => RecoveryInput::RetryRequested,
            RecoveryUiCommand::BeginDiscard => RecoveryInput::BeginDiscard,
            RecoveryUiCommand::ConfirmDiscard => RecoveryInput::DiscardConfirmed,
            RecoveryUiCommand::CancelDiscard => RecoveryInput::CancelDiscard,
        };
        self.owner
            .recovery()
            .apply(input, host)
            .map_err(|_| InvalidRecoveryUiCommand { phase, command })
    }

    /// Expose the owner's next bounded notification retry deadline to a main
    /// loop that must keep notices available while input is unavailable.
    pub(crate) fn next_periodic_notification_retry_at(&self) -> Option<Instant> {
        self.owner.next_periodic_notification_retry_at()
    }

    /// Perform at most one due periodic retry. The retry budget is stored with
    /// the root owner, not this short-lived UI borrow.
    pub(crate) fn retry_notification_if_due<H: RecoveryHost<T>>(
        &mut self,
        now: Instant,
        host: &mut H,
    ) -> Option<RecoveryStep> {
        self.owner.recovery().retry_notification_if_due(now, host)
    }

    pub(crate) fn draft(&self) -> Option<&T> {
        self.owner.draft()
    }

    pub(crate) fn is_resolved(&self) -> bool {
        self.owner.is_resolved()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        status_for_notice, InvalidRecoveryUiCommand, RecoveryUiCommand, RecoveryUiCommandAdapter,
    };
    use crate::input_failure_guard::InputFailureAction;
    use crate::input_reconnect::{DeviceIdentity, ReconnectCandidate, ReconnectGate};
    use crate::input_recovery::{
        RecoveryHost, RecoveryInput, RecoveryNotice, RecoveryOwner, RecoveryPhase, RecoveryStart,
        RecoveryStep, MAX_PERIODIC_NOTICE_RETRIES,
    };
    use crate::linux_input_reopen::VerifiedReconnectProof;
    use linux_evdev_stream::LinuxEvdevTarget;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn initial_save_failure_notice_shows_safe_failure_and_memory_retention() {
        let notice = RecoveryNotice::InitialInputSaveFailed {
            input_error: "evdev source closed".to_owned(),
            save_error: "original VFS flush failed".to_owned(),
        };

        assert_eq!(
            status_for_notice(&notice),
            (
                "desktop.notes.recovery.retry_failed",
                Some("desktop.notes.recovery.reconnect_lost_hint")
            )
        );
    }

    #[test]
    fn reconnect_unavailable_notice_does_not_offer_an_input_action() {
        let notice = RecoveryNotice::InputReconnectUnavailable {
            path: "/dev/input/event1".to_owned(),
            reason: "temporarily unavailable".to_owned(),
        };

        assert_eq!(
            status_for_notice(&notice),
            (
                "desktop.notes.recovery.reconnect_lost",
                Some("desktop.notes.recovery.reconnect_lost_hint")
            )
        );
    }

    #[test]
    fn periodic_notice_retry_waits_caps_attempts_and_keeps_owner_and_payload() {
        let (mut owner, drops) = held_owner(Some("initial notice delivery failed"));
        let mut host = Host {
            fail_notice: true,
            ..Host::default()
        };
        let expected = RecoveryNotice::InitialInputSaveFailed {
            input_error: "evdev source closed".to_owned(),
            save_error: "original VFS flush failed".to_owned(),
        };
        let first_due = RecoveryUiCommandAdapter::borrow(&mut owner)
            .next_periodic_notification_retry_at()
            .expect("initial notice failure schedules a bounded retry");

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(ui
                .retry_notification_if_due(first_due - Duration::from_nanos(1), &mut host)
                .is_none());
            assert!(ui.draft().is_some());
        }
        assert!(host.notices.is_empty(), "early poll sends no notice");
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        for expected_attempt in 1..=MAX_PERIODIC_NOTICE_RETRIES {
            let due = RecoveryUiCommandAdapter::borrow(&mut owner)
                .next_periodic_notification_retry_at()
                .expect("retry deadline remains only within the attempt budget");
            {
                let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
                assert!(matches!(
                    ui.retry_notification_if_due(due, &mut host),
                    Some(RecoveryStep::NotificationRetried {
                        notification_error: Some(_)
                    })
                ));
                assert_eq!(ui.status().phase, RecoveryPhase::WaitingForInput);
                assert_eq!(ui.status().save_error, "original VFS flush failed");
                assert!(ui.draft().is_some_and(|draft| draft.dirty));
            }
            assert_eq!(host.notices.len(), usize::from(expected_attempt));
            assert_eq!(host.notices.last(), Some(&expected));
            assert_eq!(drops.load(Ordering::SeqCst), 0);

            let next_due =
                RecoveryUiCommandAdapter::borrow(&mut owner).next_periodic_notification_retry_at();
            if expected_attempt == MAX_PERIODIC_NOTICE_RETRIES {
                assert!(next_due.is_none(), "the retry budget is exhausted");
            } else {
                let next_due = next_due.expect("budget permits another delayed retry");
                assert!(
                    RecoveryUiCommandAdapter::borrow(&mut owner)
                        .retry_notification_if_due(due, &mut host)
                        .is_none(),
                    "a late loop tick cannot issue a catch-up burst"
                );
                assert_eq!(
                    RecoveryUiCommandAdapter::borrow(&mut owner)
                        .next_periodic_notification_retry_at(),
                    Some(next_due)
                );
            }
        }

        let late_poll = first_due + Duration::from_secs(600);
        assert!(RecoveryUiCommandAdapter::borrow(&mut owner)
            .retry_notification_if_due(late_poll, &mut host)
            .is_none());
        assert_eq!(host.notices.len(), usize::from(MAX_PERIODIC_NOTICE_RETRIES));
        assert_eq!(owner.phase(), RecoveryPhase::WaitingForInput);
        assert!(owner.draft().is_some_and(|draft| draft.dirty));
        assert_eq!(host.saves, 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        // The next explicit recovery action remains possible; periodic
        // exhaustion neither releases nor silently discards the draft.
        host.fail_notice = false;
        accept_verified_reconnect(&mut owner, &mut host);
        discard_for_cleanup(&mut owner, &mut host);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    struct Draft {
        content: String,
        dirty: bool,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for Draft {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct Host {
        fail_save: bool,
        fail_notice: bool,
        saves: usize,
        notices: Vec<RecoveryNotice>,
    }

    impl RecoveryHost<Draft> for Host {
        type SaveError = &'static str;
        type NotificationError = &'static str;

        fn retry_save(&mut self, draft: &mut Draft) -> Result<(), Self::SaveError> {
            self.saves += 1;
            if self.fail_save {
                return Err("original store is still unavailable");
            }
            draft.content.push_str(" [saved]");
            draft.dirty = false;
            Ok(())
        }

        fn notify(&mut self, notice: RecoveryNotice) -> Result<(), Self::NotificationError> {
            self.notices.push(notice);
            if self.fail_notice {
                Err("UI notice transport is unavailable")
            } else {
                Ok(())
            }
        }
    }

    fn held_owner(
        initial_notice_error: Option<&'static str>,
    ) -> (RecoveryOwner<Draft>, Arc<AtomicUsize>) {
        let drops = Arc::new(AtomicUsize::new(0));
        let RecoveryStart::Held(owner) =
            RecoveryOwner::from_input_failure(InputFailureAction::HoldUnsavedDraft {
                owner: Draft {
                    content: "unsaved note".to_owned(),
                    dirty: true,
                    drops: Arc::clone(&drops),
                },
                input_error: "evdev source closed",
                save_error: "original VFS flush failed".to_owned(),
                notification_error: initial_notice_error.map(str::to_owned),
            })
        else {
            panic!("failed flush must produce a retained recovery owner");
        };
        (owner, drops)
    }

    fn accept_verified_reconnect(owner: &mut RecoveryOwner<Draft>, host: &mut Host) {
        let identity = DeviceIdentity::from_test_identity("fixture:keyboard-unique-id")
            .expect("test-only identity is non-empty");
        let mut gate = ReconnectGate::authorize(Some(identity.clone())).unwrap();
        gate.disconnect(0).unwrap();
        let accepted = gate
            .reconnect(vec![ReconnectCandidate::new(
                "/dev/input/event-fixture",
                Some(identity),
                Cursor::new(Vec::<u8>::new()),
            )])
            .expect("gate accepts the matching fixture identity");
        let reopened = accepted.start_fresh_stream(LinuxEvdevTarget::X86_64);
        let mut proof = VerifiedReconnectProof::test_only(owner.ticket(), reopened.parser_epoch);
        owner
            .accept_verified_reconnect(&mut proof, host)
            .expect("only the proof from this identity gate advances the owner");
    }

    fn discard_for_cleanup(owner: &mut RecoveryOwner<Draft>, host: &mut Host) {
        if owner.phase() == RecoveryPhase::WaitingForInput {
            accept_verified_reconnect(owner, host);
        }
        let mut ui = RecoveryUiCommandAdapter::borrow(owner);
        if ui.status().phase == RecoveryPhase::NotificationUnavailable {
            ui.apply(RecoveryUiCommand::RetryNotification, host)
                .expect("cleanup notice is available");
        }
        ui.apply(RecoveryUiCommand::BeginDiscard, host).unwrap();
        ui.apply(RecoveryUiCommand::ConfirmDiscard, host).unwrap();
    }

    #[test]
    fn initial_input_failure_notice_retries_same_semantic_payload_while_waiting() {
        let (mut owner, drops) = held_owner(Some("initial failure notice unavailable"));
        let mut host = Host {
            fail_notice: true,
            ..Host::default()
        };
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            let status = ui.status();
            assert_eq!(status.phase, RecoveryPhase::WaitingForInput);
            assert_eq!(status.input_error, "evdev source closed");
            assert_eq!(
                status.initial_notification_error.as_deref(),
                Some("initial failure notice unavailable")
            );
            assert_eq!(
                ui.available_commands(),
                &[RecoveryUiCommand::RetryNotification]
            );
            assert!(ui.draft().is_some());
            assert_eq!(
                ui.apply(RecoveryUiCommand::RetrySave, &mut host),
                Err(InvalidRecoveryUiCommand {
                    phase: RecoveryPhase::WaitingForInput,
                    command: RecoveryUiCommand::RetrySave,
                })
            );
        }
        assert_eq!(owner.phase(), RecoveryPhase::WaitingForInput);
        assert!(owner.draft().is_some());
        assert_eq!(host.saves, 0, "disabled actions do not perform hidden work");
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        let expected = RecoveryNotice::InitialInputSaveFailed {
            input_error: "evdev source closed".to_owned(),
            save_error: "original VFS flush failed".to_owned(),
        };
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetryNotification, &mut host),
                Ok(RecoveryStep::NotificationRetried {
                    notification_error: Some(_)
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::WaitingForInput);
            assert_eq!(ui.status().save_error, "original VFS flush failed");
            assert!(ui.draft().unwrap().dirty);
            assert_eq!(
                ui.available_commands(),
                &[RecoveryUiCommand::RetryNotification]
            );
        }
        assert_eq!(host.notices.last(), Some(&expected));
        host.fail_notice = false;
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetryNotification, &mut host),
                Ok(RecoveryStep::NotificationRetried {
                    notification_error: None
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::WaitingForInput);
            assert_eq!(ui.status().save_error, "original VFS flush failed");
            assert!(ui.draft().unwrap().dirty);
            assert!(ui.available_commands().is_empty());
        }
        assert_eq!(host.notices.last(), Some(&expected));
        assert_eq!(host.saves, 0, "notice retry does not attempt persistence");
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        discard_for_cleanup(&mut owner, &mut host);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(owner.is_resolved());
    }

    #[test]
    fn failed_reconnect_notice_blocks_save_until_one_explicit_notice_retry() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host {
            fail_notice: true,
            ..Host::default()
        };
        accept_verified_reconnect(&mut owner, &mut host);
        assert_eq!(owner.phase(), RecoveryPhase::NotificationUnavailable);
        assert_eq!(
            owner.last_notification_error(),
            Some("UI notice transport is unavailable")
        );
        let failed_reconnect_notice = host
            .notices
            .last()
            .cloned()
            .expect("failed reconnect notice was attempted");

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert_eq!(
                ui.available_commands(),
                &[RecoveryUiCommand::RetryNotification]
            );
            assert!(ui.apply(RecoveryUiCommand::RetrySave, &mut host).is_err());
            assert_eq!(host.saves, 0);
            assert!(ui.draft().is_some());
        }
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        host.fail_notice = false;
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetryNotification, &mut host),
                Ok(RecoveryStep::NotificationRetried {
                    notification_error: None
                })
            ));
        }
        assert_eq!(owner.phase(), RecoveryPhase::AwaitingUserChoice);
        assert_eq!(host.notices.last(), Some(&failed_reconnect_notice));
        discard_for_cleanup(&mut owner, &mut host);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reconnect_unavailable_notice_retries_same_path_and_reason_without_enabling_save() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host {
            fail_notice: true,
            ..Host::default()
        };
        let expected = RecoveryNotice::InputReconnectUnavailable {
            path: "/dev/input/event-original".to_owned(),
            reason: "stable identity is unavailable".to_owned(),
        };
        owner
            .recovery()
            .report_reconnect_unavailable(
                "/dev/input/event-original",
                "stable identity is unavailable",
                &mut host,
            )
            .expect("reopen failure stays in input-waiting state");
        assert_eq!(owner.phase(), RecoveryPhase::WaitingForInput);
        assert_eq!(
            owner.last_reconnect_error(),
            Some("stable identity is unavailable")
        );
        assert_eq!(host.notices.last(), Some(&expected));

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert_eq!(
                ui.available_commands(),
                &[RecoveryUiCommand::RetryNotification]
            );
            assert!(ui.apply(RecoveryUiCommand::RetrySave, &mut host).is_err());
            assert!(ui
                .apply(RecoveryUiCommand::BeginDiscard, &mut host)
                .is_err());
            assert!(ui.draft().is_some());
        }
        host.fail_notice = false;
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetryNotification, &mut host),
                Ok(RecoveryStep::NotificationRetried {
                    notification_error: None
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::WaitingForInput);
            assert_eq!(
                ui.status().last_reconnect_error.as_deref(),
                Some("stable identity is unavailable")
            );
            assert!(ui.draft().unwrap().dirty);
            assert!(ui.available_commands().is_empty());
        }
        assert_eq!(host.notices.last(), Some(&expected));
        assert_eq!(host.saves, 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        discard_for_cleanup(&mut owner, &mut host);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_save_notice_retry_repeats_failure_reason_and_preserves_dirty_draft() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host::default();
        accept_verified_reconnect(&mut owner, &mut host);

        host.fail_save = true;
        host.fail_notice = true;
        let original_save_error = "original store is still unavailable";
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetrySave, &mut host),
                Ok(RecoveryStep::RetryFailed {
                    notification_error: Some(_),
                    ..
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::NotificationUnavailable);
            assert_eq!(ui.status().save_error, original_save_error);
            assert_eq!(
                ui.draft()
                    .map(|draft| (draft.content.as_str(), draft.dirty)),
                Some(("unsaved note", true))
            );
        }
        assert_eq!(
            host.notices.last(),
            Some(&RecoveryNotice::RetryFailed(original_save_error.to_owned()))
        );
        assert_eq!(host.saves, 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        host.fail_notice = false;
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetryNotification, &mut host),
                Ok(RecoveryStep::NotificationRetried {
                    notification_error: None
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::AwaitingUserChoice);
            assert_eq!(ui.status().save_error, original_save_error);
            assert_eq!(
                ui.draft()
                    .map(|draft| (draft.content.as_str(), draft.dirty)),
                Some(("unsaved note", true))
            );
        }
        assert_eq!(
            host.notices.last(),
            Some(&RecoveryNotice::RetryFailed(original_save_error.to_owned())),
            "notice retry must repeat the failed save notice, not a generic reconnect notice"
        );
        assert_eq!(host.saves, 1, "notifying again does not retry persistence");
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        discard_for_cleanup(&mut owner, &mut host);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn retry_failure_keeps_draft_and_success_precedes_a_failed_receipt() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host {
            fail_save: true,
            ..Host::default()
        };
        accept_verified_reconnect(&mut owner, &mut host);
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetrySave, &mut host),
                Ok(RecoveryStep::RetryFailed { .. })
            ));
            assert_eq!(
                ui.status().save_error,
                "original store is still unavailable"
            );
            assert_eq!(ui.status().phase, RecoveryPhase::AwaitingUserChoice);
            assert!(ui.draft().is_some());
        }
        assert_eq!(
            host.saves, 1,
            "one command performs exactly one save attempt"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        host.fail_save = false;
        host.fail_notice = true;
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(matches!(
                ui.apply(RecoveryUiCommand::RetrySave, &mut host),
                Ok(RecoveryStep::RetrySaved {
                    notification_error: Some(_)
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::Saved);
            assert!(ui.draft().is_none());
            assert!(ui.available_commands().is_empty());
            assert!(ui.status().last_notification_error.is_some());
            assert!(ui.is_resolved());
        }
        assert_eq!(host.saves, 2);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(owner.is_resolved());
        // The save succeeded before its receipt failed; owner release is final.
    }

    #[test]
    fn discard_confirmation_is_owner_state_and_only_confirm_releases_draft() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host::default();
        accept_verified_reconnect(&mut owner, &mut host);
        assert!(owner
            .recovery()
            .apply(RecoveryInput::DiscardConfirmed, &mut host)
            .is_err());
        assert!(owner.draft().is_some());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert_eq!(
                ui.available_commands(),
                &[
                    RecoveryUiCommand::RetrySave,
                    RecoveryUiCommand::BeginDiscard
                ]
            );
            assert_eq!(
                ui.apply(RecoveryUiCommand::BeginDiscard, &mut host),
                Ok(RecoveryStep::DiscardConfirmationStarted)
            );
            assert_eq!(ui.status().phase, RecoveryPhase::ConfirmingDiscard);
            assert!(ui.draft().is_some());
            assert!(ui.apply(RecoveryUiCommand::RetrySave, &mut host).is_err());
        }
        assert_eq!(owner.phase(), RecoveryPhase::ConfirmingDiscard);
        assert!(owner.draft().is_some());
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert_eq!(
                ui.available_commands(),
                &[
                    RecoveryUiCommand::ConfirmDiscard,
                    RecoveryUiCommand::CancelDiscard
                ]
            );
            assert_eq!(
                ui.apply(RecoveryUiCommand::CancelDiscard, &mut host),
                Ok(RecoveryStep::DiscardConfirmationCancelled)
            );
            assert_eq!(ui.status().phase, RecoveryPhase::AwaitingUserChoice);
            ui.apply(RecoveryUiCommand::BeginDiscard, &mut host)
                .unwrap();
            assert!(ui.draft().is_some());
            assert!(matches!(
                ui.apply(RecoveryUiCommand::ConfirmDiscard, &mut host),
                Ok(RecoveryStep::Discarded {
                    notification_error: None
                })
            ));
            assert_eq!(ui.status().phase, RecoveryPhase::Discarded);
            assert!(ui.draft().is_none());
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(owner.is_resolved());
    }

    #[test]
    fn root_owner_drop_and_ticket_resume_preserve_discard_confirmation_and_draft() {
        let (mut owner, drops) = held_owner(None);
        let mut host = Host::default();
        accept_verified_reconnect(&mut owner, &mut host);
        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut owner);
            assert!(ui.apply(RecoveryUiCommand::BeginDiscard, &mut host).is_ok());
        }
        assert_eq!(owner.phase(), RecoveryPhase::ConfirmingDiscard);
        let ticket = owner.ticket();
        drop(owner);
        assert!(RecoveryOwner::<Draft>::pending_tickets().contains(&ticket));
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        let mut resumed = RecoveryOwner::<Draft>::resume(ticket)
            .expect("parked root owner resumes in the same process");
        assert_eq!(resumed.phase(), RecoveryPhase::ConfirmingDiscard);
        assert_eq!(
            resumed.draft().map(|draft| draft.content.as_str()),
            Some("unsaved note")
        );
        assert!(resumed.draft().unwrap().dirty);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(!RecoveryOwner::<Draft>::pending_tickets().contains(&ticket));

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut resumed);
            assert_eq!(
                ui.available_commands(),
                &[
                    RecoveryUiCommand::ConfirmDiscard,
                    RecoveryUiCommand::CancelDiscard
                ]
            );
            assert_eq!(
                ui.apply(RecoveryUiCommand::CancelDiscard, &mut host),
                Ok(RecoveryStep::DiscardConfirmationCancelled)
            );
            assert_eq!(ui.status().phase, RecoveryPhase::AwaitingUserChoice);
            assert!(ui.draft().is_some());
        }
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        {
            let mut ui = RecoveryUiCommandAdapter::borrow(&mut resumed);
            ui.apply(RecoveryUiCommand::BeginDiscard, &mut host)
                .unwrap();
            assert_eq!(ui.status().phase, RecoveryPhase::ConfirmingDiscard);
            assert!(ui.draft().is_some());
            assert!(matches!(
                ui.apply(RecoveryUiCommand::ConfirmDiscard, &mut host),
                Ok(RecoveryStep::Discarded {
                    notification_error: None
                })
            ));
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(resumed.is_resolved());
    }
}
