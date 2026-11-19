use crate::input_failure_guard::InputFailureAction;
use crate::linux_input_reopen::VerifiedReconnectProof;
use std::any::Any;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const PERIODIC_NOTICE_RETRY_FIRST_DELAY: Duration = Duration::from_secs(5);
pub(crate) const PERIODIC_NOTICE_RETRY_INTERVAL: Duration = Duration::from_secs(15);
pub(crate) const MAX_PERIODIC_NOTICE_RETRIES: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RecoveryTicket(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryResumeError {
    NotFound,
    TypeMismatch,
}

static PARKED_RECOVERIES: OnceLock<Mutex<HashMap<RecoveryTicket, Box<dyn Any + Send>>>> =
    OnceLock::new();

fn parked_recoveries() -> &'static Mutex<HashMap<RecoveryTicket, Box<dyn Any + Send>>> {
    PARKED_RECOVERIES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_parked_recoveries() -> MutexGuard<'static, HashMap<RecoveryTicket, Box<dyn Any + Send>>> {
    parked_recoveries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static NEXT_RECOVERY_TICKET: AtomicU64 = AtomicU64::new(1);

fn next_recovery_ticket() -> RecoveryTicket {
    // Exhaustion would require more recovery sessions in one process than a
    // u64 can represent; zero is reserved and never issued.
    let mut ticket = NEXT_RECOVERY_TICKET.fetch_add(1, Ordering::Relaxed);
    if ticket == 0 {
        ticket = NEXT_RECOVERY_TICKET.fetch_add(1, Ordering::Relaxed);
    }
    RecoveryTicket(ticket)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryPhase {
    WaitingForInput,
    AwaitingUserChoice,
    NotificationUnavailable,
    ConfirmingDiscard,
    Saved,
    Discarded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryInput {
    RetryNotification,
    RetryRequested,
    BeginDiscard,
    CancelDiscard,
    DiscardConfirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconnectProofError {
    WrongOwner,
    AlreadyConsumed,
    WrongParserEpoch { expected: u64, actual: u64 },
    ParserEpochExhausted,
    WrongPhase(RecoveryPhase),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryNotice {
    /// The initial input-loss flush failed and the dirty draft is being held.
    InitialInputSaveFailed {
        input_error: String,
        save_error: String,
    },
    InputReconnected {
        input_error: String,
        initial_save_error: String,
        initial_notification_error: Option<String>,
        previous_notification_error: Option<String>,
        last_reconnect_error: Option<String>,
    },
    InputReconnectUnavailable {
        path: String,
        reason: String,
    },
    RetrySaved,
    RetryFailed(String),
    DraftDiscarded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryStep {
    Reconnected {
        notification_error: Option<String>,
    },
    NotificationRetried {
        notification_error: Option<String>,
    },
    RetrySaved {
        notification_error: Option<String>,
    },
    RetryFailed {
        save_error: String,
        notification_error: Option<String>,
    },
    DiscardConfirmationStarted,
    DiscardConfirmationCancelled,
    Discarded {
        notification_error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidRecoveryInput {
    pub phase: RecoveryPhase,
    pub input: RecoveryInput,
}

pub enum RecoveryStart<T: Send + 'static, E> {
    Held(RecoveryOwner<T>),
    FlushSucceeded { owner: T, input_error: E },
}

/// The scoped handle for an unresolved in-memory draft.
///
/// Dropping an unresolved handle transfers its state into a process-local RAM
/// keeper. `ticket()` identifies the entry, and `resume()` reacquires it from
/// any thread in this process. Save success or confirmed discard removes the
/// draft through the state machine. The keeper is volatile and cannot survive
/// process termination; it is not durable recovery storage.
#[must_use = "retain the ticket if this unresolved recovery handle may be dropped"]
pub struct RecoveryOwner<T: Send + 'static> {
    ticket: RecoveryTicket,
    state: Option<RecoveryState<T>>,
}

struct RecoveryState<T: Send + 'static> {
    draft: Option<T>,
    phase: RecoveryPhase,
    input_error: String,
    last_save_error: String,
    initial_notification_error: Option<String>,
    last_notification_error: Option<String>,
    pending_notice: Option<RecoveryNotice>,
    periodic_notice_retry: Option<PeriodicNoticeRetry>,
    last_reconnect_error: Option<String>,
    parser_epoch: u64,
}

#[derive(Clone, Copy)]
struct PeriodicNoticeRetry {
    next_at: Instant,
    attempts: u8,
}

impl PeriodicNoticeRetry {
    fn after_failure(now: Instant) -> Self {
        Self {
            next_at: now + PERIODIC_NOTICE_RETRY_FIRST_DELAY,
            attempts: 0,
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.attempts < MAX_PERIODIC_NOTICE_RETRIES && now >= self.next_at
    }

    fn record_attempt(&mut self, now: Instant) {
        self.attempts += 1;
        self.next_at = now + PERIODIC_NOTICE_RETRY_INTERVAL;
    }

    fn next_at(&self) -> Option<Instant> {
        (self.attempts < MAX_PERIODIC_NOTICE_RETRIES).then_some(self.next_at)
    }
}

/// A short-lived operation handle that borrows the long-lived recovery owner.
///
/// Dropping this handle releases only the borrow; the unresolved draft remains
/// in [`RecoveryOwner`]. The handle cannot extract or discard the draft.
pub struct InputRecovery<'a, T: Send + 'static> {
    state: &'a mut RecoveryState<T>,
}

/// Host seam for one user-initiated recovery action.
///
/// `retry_save` must call the existing note session's explicit retry against
/// the original store. `notify` presents the supplied status to the user.
pub trait RecoveryHost<T> {
    type SaveError: Display;
    type NotificationError: Display;

    fn retry_save(&mut self, draft: &mut T) -> Result<(), Self::SaveError>;
    fn notify(&mut self, notice: RecoveryNotice) -> Result<(), Self::NotificationError>;
}

impl<T: Send + 'static> RecoveryOwner<T> {
    /// Transfer ownership of a failed-flush draft into the caller's
    /// application-lifetime recovery owner.
    pub fn from_input_failure<E: Display>(result: InputFailureAction<T, E>) -> RecoveryStart<T, E> {
        match result {
            InputFailureAction::ExitWithInputErrorAfterSave { owner, input_error } => {
                RecoveryStart::FlushSucceeded { owner, input_error }
            }
            InputFailureAction::HoldUnsavedDraft {
                owner,
                input_error,
                save_error,
                notification_error,
            } => {
                let input_error = input_error.to_string();
                let pending_notice =
                    notification_error
                        .as_ref()
                        .map(|_| RecoveryNotice::InitialInputSaveFailed {
                            input_error: input_error.clone(),
                            save_error: save_error.clone(),
                        });
                let periodic_notice_retry = pending_notice
                    .as_ref()
                    .map(|_| PeriodicNoticeRetry::after_failure(Instant::now()));
                RecoveryStart::Held(Self {
                    ticket: next_recovery_ticket(),
                    state: Some(RecoveryState {
                        draft: Some(owner),
                        phase: RecoveryPhase::WaitingForInput,
                        input_error,
                        last_save_error: save_error,
                        initial_notification_error: notification_error.clone(),
                        last_notification_error: notification_error,
                        pending_notice,
                        periodic_notice_retry,
                        last_reconnect_error: None,
                        parser_epoch: 0,
                    }),
                })
            }
        }
    }

    /// Reacquire an unresolved draft parked by an earlier handle drop.
    pub fn resume(ticket: RecoveryTicket) -> Result<Self, RecoveryResumeError> {
        let parked = lock_parked_recoveries().remove(&ticket);
        let Some(parked) = parked else {
            return Err(RecoveryResumeError::NotFound);
        };
        match parked.downcast::<RecoveryState<T>>() {
            Ok(state) => Ok(Self {
                ticket,
                state: Some(*state),
            }),
            Err(parked) => {
                lock_parked_recoveries().insert(ticket, parked);
                Err(RecoveryResumeError::TypeMismatch)
            }
        }
    }

    /// Return every ticket still held by the process-local keeper.
    /// This is the recovery path if the original task-local ticket was lost.
    pub fn pending_tickets() -> Vec<RecoveryTicket> {
        let mut tickets = lock_parked_recoveries().keys().copied().collect::<Vec<_>>();
        tickets.sort_by_key(|ticket| ticket.0);
        tickets
    }

    pub fn ticket(&self) -> RecoveryTicket {
        self.ticket
    }

    fn state(&self) -> &RecoveryState<T> {
        self.state.as_ref().expect("active recovery owns its state")
    }

    fn state_mut(&mut self) -> &mut RecoveryState<T> {
        self.state.as_mut().expect("active recovery owns its state")
    }

    pub fn phase(&self) -> RecoveryPhase {
        self.state().phase
    }

    pub fn draft(&self) -> Option<&T> {
        self.state().draft.as_ref()
    }

    pub fn input_error(&self) -> &str {
        &self.state().input_error
    }

    pub fn last_save_error(&self) -> &str {
        &self.state().last_save_error
    }

    /// The first notification failure from the input-error path, if any.
    pub fn initial_notification_error(&self) -> Option<&str> {
        self.state().initial_notification_error.as_deref()
    }

    /// The most recent notification failure, if any.
    pub fn last_notification_error(&self) -> Option<&str> {
        self.state().last_notification_error.as_deref()
    }

    pub(crate) fn notification_retry_available(&self) -> bool {
        self.state().pending_notice.is_some()
    }

    /// Next monotonic deadline for the bounded periodic retry path.
    pub(crate) fn next_periodic_notification_retry_at(&self) -> Option<Instant> {
        self.state().pending_notice.as_ref()?;
        self.state().periodic_notice_retry?.next_at()
    }

    pub fn last_reconnect_error(&self) -> Option<&str> {
        self.state().last_reconnect_error.as_deref()
    }

    pub fn recovery_actions_available(&self) -> bool {
        self.state().phase == RecoveryPhase::AwaitingUserChoice
    }

    pub fn is_resolved(&self) -> bool {
        self.state().draft.is_none()
            && matches!(
                self.state().phase,
                RecoveryPhase::Saved | RecoveryPhase::Discarded
            )
    }

    /// Borrow the recovery state for a short-lived event handler.
    pub(crate) fn recovery(&mut self) -> InputRecovery<'_, T> {
        InputRecovery {
            state: self.state_mut(),
        }
    }

    /// Advance from input loss only with single-use evidence minted by the
    /// reconnect gate after matching the real opened handle to this owner.
    pub(crate) fn accept_verified_reconnect<H: RecoveryHost<T>>(
        &mut self,
        proof: &mut VerifiedReconnectProof,
        host: &mut H,
    ) -> Result<RecoveryStep, ReconnectProofError> {
        if self.phase() != RecoveryPhase::WaitingForInput {
            return Err(ReconnectProofError::WrongPhase(self.phase()));
        }
        let expected_parser_epoch = self
            .state()
            .parser_epoch
            .checked_add(1)
            .ok_or(ReconnectProofError::ParserEpochExhausted)?;
        let parser_epoch = proof.consume_for(self.ticket, expected_parser_epoch)?;
        let mut recovery = self.recovery();
        recovery.state.parser_epoch = parser_epoch;
        let notification_error = recovery.notify_reconnected(host);
        recovery.state.phase = if notification_error.is_some() {
            RecoveryPhase::NotificationUnavailable
        } else {
            RecoveryPhase::AwaitingUserChoice
        };
        Ok(RecoveryStep::Reconnected { notification_error })
    }

    /// Re-enter the input-wait phase when a previously verified replacement
    /// stream is lost. The unresolved draft and parser epoch remain owned by
    /// this same handle; the next reconnect must still present a fresh proof
    /// for the next epoch.
    pub(crate) fn wait_for_input_after_reconnected_loss(&mut self) -> Result<(), RecoveryPhase> {
        let phase = self.phase();
        if !matches!(
            phase,
            RecoveryPhase::AwaitingUserChoice
                | RecoveryPhase::NotificationUnavailable
                | RecoveryPhase::ConfirmingDiscard
        ) || self.state().draft.is_none()
        {
            return Err(phase);
        }
        self.state_mut().phase = RecoveryPhase::WaitingForInput;
        Ok(())
    }

    /// Release the root owner only after save success or confirmed discard.
    ///
    /// On an unresolved state, return the owner intact. If the caller later
    /// drops it, `Drop` parks the state under its ticket so it can be resumed
    /// in this process. Parking is only RAM protection; process exit still
    /// destroys the draft.
    pub fn release(self) -> Result<(), Self> {
        if self.is_resolved() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl<T: Send + 'static> Drop for RecoveryOwner<T> {
    fn drop(&mut self) {
        let Some(state) = self.state.take() else {
            return;
        };
        if state.draft.is_some() {
            lock_parked_recoveries().insert(self.ticket, Box::new(state));
        }
    }
}

impl<T: Send + 'static> InputRecovery<'_, T> {
    pub fn phase(&self) -> RecoveryPhase {
        self.state.phase
    }

    pub fn draft(&self) -> Option<&T> {
        self.state.draft.as_ref()
    }

    pub fn last_save_error(&self) -> &str {
        &self.state.last_save_error
    }

    pub fn initial_notification_error(&self) -> Option<&str> {
        self.state.initial_notification_error.as_deref()
    }

    pub fn last_notification_error(&self) -> Option<&str> {
        self.state.last_notification_error.as_deref()
    }

    pub fn last_reconnect_error(&self) -> Option<&str> {
        self.state.last_reconnect_error.as_deref()
    }

    pub fn recovery_actions_available(&self) -> bool {
        self.state.phase == RecoveryPhase::AwaitingUserChoice
    }

    /// Apply exactly one event. Save and discard actions are disabled until a
    /// recovery notice has been presented successfully after input returns.
    /// `RetryNotification` retries the exact pending notice once. A separate
    /// bounded timer API can retry a notice while input is unavailable.
    pub(crate) fn apply<H: RecoveryHost<T>>(
        &mut self,
        input: RecoveryInput,
        host: &mut H,
    ) -> Result<RecoveryStep, InvalidRecoveryInput> {
        match (self.state.phase, input) {
            (
                phase @ (RecoveryPhase::WaitingForInput | RecoveryPhase::NotificationUnavailable),
                RecoveryInput::RetryNotification,
            ) if self.state.pending_notice.is_some() => {
                debug_assert_eq!(phase, self.state.phase);
                Ok(self.retry_pending_notification(host))
            }
            (RecoveryPhase::AwaitingUserChoice, RecoveryInput::RetryRequested) => {
                let draft = self
                    .state
                    .draft
                    .as_mut()
                    .expect("held phase owns its draft");
                match host.retry_save(draft) {
                    Ok(()) => {
                        self.state.phase = RecoveryPhase::Saved;
                        self.state.last_save_error.clear();
                        // Persistence succeeded before notification. A later
                        // notice failure cannot make the saved owner dirty again.
                        drop(self.state.draft.take());
                        let notification_error = self.notify(host, RecoveryNotice::RetrySaved);
                        Ok(RecoveryStep::RetrySaved { notification_error })
                    }
                    Err(error) => {
                        let save_error = error.to_string();
                        self.state.last_save_error = save_error.clone();
                        let notification_error =
                            self.notify(host, RecoveryNotice::RetryFailed(save_error.clone()));
                        if notification_error.is_some() {
                            self.state.phase = RecoveryPhase::NotificationUnavailable;
                        }
                        Ok(RecoveryStep::RetryFailed {
                            save_error,
                            notification_error,
                        })
                    }
                }
            }
            (RecoveryPhase::AwaitingUserChoice, RecoveryInput::BeginDiscard) => {
                self.state.phase = RecoveryPhase::ConfirmingDiscard;
                Ok(RecoveryStep::DiscardConfirmationStarted)
            }
            (RecoveryPhase::ConfirmingDiscard, RecoveryInput::CancelDiscard) => {
                self.state.phase = RecoveryPhase::AwaitingUserChoice;
                Ok(RecoveryStep::DiscardConfirmationCancelled)
            }
            (RecoveryPhase::ConfirmingDiscard, RecoveryInput::DiscardConfirmed) => {
                self.state.phase = RecoveryPhase::Discarded;
                // The explicit discard is complete before its receipt is sent.
                drop(self.state.draft.take());
                let notification_error = self.notify(host, RecoveryNotice::DraftDiscarded);
                Ok(RecoveryStep::Discarded { notification_error })
            }
            (phase, input) => Err(InvalidRecoveryInput { phase, input }),
        }
    }

    /// Report one explicit reopen failure while keeping the dirty draft in
    /// `WaitingForInput`. This never retries, opens a different path, or
    /// releases the owner. The reason is carried into the next reconnect
    /// notice so the user can see what prevented recovery.
    pub fn report_reconnect_unavailable<H: RecoveryHost<T>>(
        &mut self,
        path: impl Into<String>,
        reason: impl Into<String>,
        host: &mut H,
    ) -> Result<Option<String>, RecoveryPhase> {
        if self.state.phase != RecoveryPhase::WaitingForInput {
            return Err(self.state.phase);
        }
        let path = path.into();
        let reason = reason.into();
        self.state.last_reconnect_error = Some(reason.clone());
        Ok(self.notify(
            host,
            RecoveryNotice::InputReconnectUnavailable { path, reason },
        ))
    }

    /// Retry the retained notification only when its monotonic deadline is
    /// due. The limit and deadline live in the recovery owner, so short-lived
    /// UI borrows and same-process park/resume do not reset the budget. A late
    /// poll performs at most one attempt and schedules from the current time.
    pub(crate) fn retry_notification_if_due<H: RecoveryHost<T>>(
        &mut self,
        now: Instant,
        host: &mut H,
    ) -> Option<RecoveryStep> {
        if !matches!(
            self.state.phase,
            RecoveryPhase::WaitingForInput | RecoveryPhase::NotificationUnavailable
        ) || self.state.pending_notice.is_none()
        {
            return None;
        }

        let due = self
            .state
            .periodic_notice_retry
            .is_some_and(|retry| retry.due(now));
        if !due {
            return None;
        }

        self.state
            .periodic_notice_retry
            .as_mut()
            .expect("due retry has a schedule")
            .record_attempt(now);
        Some(self.retry_pending_notification(host))
    }

    fn retry_pending_notification<H: RecoveryHost<T>>(&mut self, host: &mut H) -> RecoveryStep {
        let phase = self.state.phase;
        let notice = self
            .state
            .pending_notice
            .clone()
            .expect("notification retry requires a pending notice");
        let notification_error = self.notify(host, notice);
        self.state.phase = match (phase, notification_error.is_some()) {
            (RecoveryPhase::WaitingForInput, _) => RecoveryPhase::WaitingForInput,
            (RecoveryPhase::NotificationUnavailable, true) => {
                RecoveryPhase::NotificationUnavailable
            }
            (RecoveryPhase::NotificationUnavailable, false) => RecoveryPhase::AwaitingUserChoice,
            _ => unreachable!("only notification retry phases are supported"),
        };
        RecoveryStep::NotificationRetried { notification_error }
    }

    fn notify_reconnected<H: RecoveryHost<T>>(&mut self, host: &mut H) -> Option<String> {
        let notice = RecoveryNotice::InputReconnected {
            input_error: self.state.input_error.clone(),
            initial_save_error: self.state.last_save_error.clone(),
            initial_notification_error: self.state.initial_notification_error.clone(),
            previous_notification_error: self.state.last_notification_error.clone(),
            last_reconnect_error: self.state.last_reconnect_error.clone(),
        };
        self.notify(host, notice)
    }

    fn notify<H: RecoveryHost<T>>(
        &mut self,
        host: &mut H,
        notice: RecoveryNotice,
    ) -> Option<String> {
        let same_pending_notice = self.state.pending_notice.as_ref() == Some(&notice);
        let now = Instant::now();
        let error = host
            .notify(notice.clone())
            .err()
            .map(|error| error.to_string());
        if error.is_some() {
            self.state.pending_notice = Some(notice);
            if self.state.draft.is_some() {
                if !same_pending_notice || self.state.periodic_notice_retry.is_none() {
                    self.state.periodic_notice_retry =
                        Some(PeriodicNoticeRetry::after_failure(now));
                }
            } else {
                self.state.periodic_notice_retry = None;
            }
        } else {
            self.state.pending_notice = None;
            self.state.periodic_notice_retry = None;
        }
        self.state.last_notification_error = error.clone();
        error
    }
}
