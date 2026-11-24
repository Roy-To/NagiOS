//! Monotonic rate limit owned by the PID 1 login transaction.

use nagi_account_credential::throttle_wait_ms;
use std::io;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LoginAttemptError<E> {
    Throttled,
    PersistenceUnavailable,
    Verification(E),
}

pub(crate) struct LoginFailureCount {
    count: u32,
    blocked_until: Instant,
    persistence_unavailable: bool,
}

impl LoginFailureCount {
    pub(crate) fn restored(failures: u32) -> Self {
        Self::restored_at(failures, Instant::now())
    }

    fn restored_at(failures: u32, now: Instant) -> Self {
        Self {
            count: failures,
            blocked_until: now + Duration::from_millis(throttle_wait_ms(failures)),
            persistence_unavailable: false,
        }
    }

    pub(crate) fn current(&self) -> u32 {
        self.count
    }

    pub(crate) fn persist(
        &mut self,
        failures: u32,
        store: impl FnOnce(u32) -> io::Result<()>,
    ) -> io::Result<()> {
        self.persist_at(failures, Instant::now, store)
    }

    fn persist_at(
        &mut self,
        failures: u32,
        now: impl FnOnce() -> Instant,
        store: impl FnOnce(u32) -> io::Result<()>,
    ) -> io::Result<()> {
        let result = store(failures);
        self.persistence_unavailable = result.is_err();
        result?;
        self.count = failures;
        self.blocked_until = now() + Duration::from_millis(throttle_wait_ms(failures));
        Ok(())
    }

    pub(crate) fn attempt<T, E>(
        &mut self,
        now: impl Fn() -> Instant,
        verify: impl FnOnce() -> Result<T, E>,
        is_rejection: impl FnOnce(&E) -> bool,
        store: impl FnOnce(u32) -> io::Result<()>,
    ) -> Result<T, LoginAttemptError<E>> {
        if self.persistence_unavailable {
            return Err(LoginAttemptError::PersistenceUnavailable);
        }
        if now() < self.blocked_until {
            return Err(LoginAttemptError::Throttled);
        }
        match verify() {
            Ok(evidence) => Ok(evidence),
            Err(error) => {
                if is_rejection(&error) {
                    self.persist_at(self.count.saturating_add(1), now, store)
                        .map_err(|_| LoginAttemptError::PersistenceUnavailable)?;
                }
                Err(LoginAttemptError::Verification(error))
            }
        }
    }
}

#[cfg(test)]
mod tests;
