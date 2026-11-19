//! Shared main-loop dispatch for input loss after an identity-verified reopen.
//! Kept Linux-independent so the host regression exercises the same transition
//! that the Linux desktop loop calls.

use super::input_recovery::{RecoveryHost, RecoveryOwner};
use super::linux_input_reopen::{LinuxGrantedReopen, LinuxInputGrant, ReconnectedInputLossAction};
use std::time::{Duration, Instant};

pub(crate) enum InitialGrantDispatch<T: Send + 'static> {
    Granted(LinuxGrantedReopen<T>),
    MissingOriginalGrant(RecoveryOwner<T>),
}

/// Keep the actual initial-loss `Option<LinuxInputGrant>` decision shared by
/// main and host tests. The missing case carries the dirty owner but provides
/// no reopen adapter or path, so it cannot authorize a fallback input source.
pub(crate) fn dispatch_initial_grant<T: Send + 'static>(
    grant: Option<LinuxInputGrant>,
    partial_record_bytes: usize,
    owner: RecoveryOwner<T>,
) -> InitialGrantDispatch<T> {
    match grant {
        Some(grant) => {
            InitialGrantDispatch::Granted(grant.into_recovery(partial_record_bytes, owner))
        }
        None => InitialGrantDispatch::MissingOriginalGrant(owner),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReconnectedInputLoss {
    reason: String,
    partial_record_bytes: usize,
}

impl ReconnectedInputLoss {
    pub(crate) fn from_read_error(reason: impl Into<String>, partial_record_bytes: usize) -> Self {
        Self {
            reason: reason.into(),
            partial_record_bytes,
        }
    }

    pub(crate) fn reason(&self) -> &str {
        &self.reason
    }

    pub(crate) const fn partial_record_bytes(&self) -> usize {
        self.partial_record_bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconnectedInputLossDispatch {
    RetryOriginalGrant { notification_error: Option<String> },
    ReArmFailed { error: String },
}

/// Apply the main loop's reconnected-read-error transition to the same owner
/// and retained identity gate. A rejected re-arm is returned as a fail-closed
/// outcome so the caller can remain in its wait state without replacing or
/// dropping the dirty owner.
pub(crate) fn dispatch_reconnected_input_loss<T, H>(
    recovery: &mut LinuxGrantedReopen<T>,
    loss: &ReconnectedInputLoss,
    host: &mut H,
) -> ReconnectedInputLossDispatch
where
    T: Send + 'static,
    H: RecoveryHost<T>,
{
    match recovery.handle_reconnected_input_loss(
        loss.partial_record_bytes,
        loss.reason.clone(),
        host,
    ) {
        Ok((ReconnectedInputLossAction::RetryOriginalGrant, notification_error)) => {
            ReconnectedInputLossDispatch::RetryOriginalGrant { notification_error }
        }
        Err(error) => ReconnectedInputLossDispatch::ReArmFailed {
            error: format!("{error:?}"),
        },
    }
}

/// Keep the input reopen loop and the owner's bounded notification timer
/// responsive to the same monotonic clock.
pub(crate) fn next_recovery_wait<T: Send + 'static>(
    owner: &RecoveryOwner<T>,
    reconnect_delay: Duration,
    now: Instant,
) -> Duration {
    owner
        .next_periodic_notification_retry_at()
        .map(|deadline| reconnect_delay.min(deadline.saturating_duration_since(now)))
        .unwrap_or(reconnect_delay)
}
