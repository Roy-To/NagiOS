//! Fail-closed gate for closing the initial login screen.
//!
//! Kept independent of the Linux process owner so the exact cancellation gate
//! and its delayed-child regression can run in the supported host test target.

use std::time::Instant;

#[derive(Debug)]
pub(super) struct VerifiedLoginCancellation(());

impl VerifiedLoginCancellation {
    pub(super) fn from_input_close<E>(
        require_unattached: impl FnOnce() -> Result<(), E>,
        active_children: impl FnOnce() -> usize,
        unresolved_cleanup: impl FnOnce() -> Result<bool, E>,
        unresolved_denial: E,
    ) -> Result<Self, E> {
        // Credential entry precedes child startup. A prior attempt may return
        // here only after exact reap and lease retirement.
        require_unattached()?;
        if active_children() != 0 || unresolved_cleanup()? {
            return Err(unresolved_denial);
        }
        Ok(Self(()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoginCleanupPoll {
    Complete,
    Pending,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LoginCleanupWait {
    Complete,
    DeadlineExpired,
    Unknown,
}

pub(super) fn classify_login_denial_after_cleanup<E>(
    error: E,
    cleanup_confirmed: bool,
    is_retryable_denial: impl FnOnce(&E) -> bool,
    unresolved_denial: E,
    unknown_denial: E,
) -> E {
    if cleanup_confirmed {
        return error;
    }
    if is_retryable_denial(&error) {
        unresolved_denial
    } else {
        unknown_denial
    }
}

pub(super) fn wait_for_login_cleanup_until(
    deadline: Instant,
    mut poll_cleanup: impl FnMut() -> LoginCleanupPoll,
    mut wait_for_event: impl FnMut(std::time::Duration),
) -> LoginCleanupWait {
    loop {
        if Instant::now() >= deadline {
            return LoginCleanupWait::DeadlineExpired;
        }
        let state = poll_cleanup();
        let now = Instant::now();
        if now >= deadline {
            return LoginCleanupWait::DeadlineExpired;
        }
        match state {
            LoginCleanupPoll::Complete => return LoginCleanupWait::Complete,
            LoginCleanupPoll::Unknown => return LoginCleanupWait::Unknown,
            LoginCleanupPoll::Pending => {
                wait_for_event(deadline.saturating_duration_since(now));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_login_denial_after_cleanup, wait_for_login_cleanup_until, LoginCleanupPoll,
        LoginCleanupWait, VerifiedLoginCancellation,
    };
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[derive(Debug, Eq, PartialEq)]
    enum TestDenial {
        AccountRejected,
        UnresolvedSpawnCleanup,
        RequestOutcomeUnknown,
        Unavailable,
    }

    #[test]
    fn explicit_close_requires_no_session_or_unretired_child() {
        use TestDenial::*;

        assert!(VerifiedLoginCancellation::from_input_close(
            || Ok(()),
            || 0,
            || Ok(false),
            UnresolvedSpawnCleanup,
        )
        .is_ok());
        assert_eq!(
            VerifiedLoginCancellation::from_input_close(
                || Ok(()),
                || 1,
                || Ok(false),
                UnresolvedSpawnCleanup,
            )
            .unwrap_err(),
            UnresolvedSpawnCleanup
        );
        assert_eq!(
            VerifiedLoginCancellation::from_input_close(
                || Ok(()),
                || 0,
                || Ok(true),
                UnresolvedSpawnCleanup,
            )
            .unwrap_err(),
            UnresolvedSpawnCleanup
        );
        assert_eq!(
            VerifiedLoginCancellation::from_input_close(
                || Ok(()),
                || 0,
                || Err(Unavailable),
                UnresolvedSpawnCleanup,
            )
            .unwrap_err(),
            Unavailable
        );
        assert_eq!(
            VerifiedLoginCancellation::from_input_close(
                || Err(Unavailable),
                || 0,
                || Ok(false),
                UnresolvedSpawnCleanup,
            )
            .unwrap_err(),
            Unavailable
        );
        assert_eq!(
            classify_login_denial_after_cleanup(
                Unavailable,
                false,
                |denial| matches!(denial, TestDenial::AccountRejected),
                UnresolvedSpawnCleanup,
                TestDenial::RequestOutcomeUnknown,
            ),
            TestDenial::RequestOutcomeUnknown
        );
    }

    #[test]
    fn rejected_login_waits_for_delayed_child_exit_before_escape_closes() {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("printf 'ready\\n'; read -r token; exec /bin/sleep 0.25")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn delayed cleanup child");
        let child_stdin = child.stdin.take().expect("child stdin");
        let mut child_stdout = std::io::BufReader::new(child.stdout.take().expect("child stdout"));
        let mut ready = String::new();
        child_stdout
            .read_line(&mut ready)
            .expect("child ready marker");
        assert_eq!(ready, "ready\n");
        assert!(child.try_wait().unwrap().is_none());

        // An authentication rejection is not retryable until the delayed
        // child and its owned cleanup finish, so Escape cannot close yet.
        let denial_before_cleanup = classify_login_denial_after_cleanup(
            TestDenial::AccountRejected,
            false,
            |denial| matches!(denial, TestDenial::AccountRejected),
            TestDenial::UnresolvedSpawnCleanup,
            TestDenial::RequestOutcomeUnknown,
        );
        assert_eq!(denial_before_cleanup, TestDenial::UnresolvedSpawnCleanup);
        assert_eq!(
            VerifiedLoginCancellation::from_input_close(
                || Ok(()),
                || 1,
                || Ok(false),
                TestDenial::UnresolvedSpawnCleanup,
            )
            .unwrap_err(),
            TestDenial::UnresolvedSpawnCleanup
        );

        // Closing the test control pipe represents the cleanup request. The
        // child stays alive briefly, as a just-signalled child can in PID 1.
        drop(child_stdin);
        assert!(child.try_wait().unwrap().is_none());
        let mut pending_polls = 0;
        let cleanup = wait_for_login_cleanup_until(
            Instant::now() + Duration::from_secs(1),
            || match child.try_wait() {
                Ok(Some(_)) => LoginCleanupPoll::Complete,
                Ok(None) => {
                    pending_polls += 1;
                    LoginCleanupPoll::Pending
                }
                Err(_) => LoginCleanupPoll::Unknown,
            },
            |remaining| std::thread::sleep(remaining.min(Duration::from_millis(1))),
        );
        assert_eq!(cleanup, LoginCleanupWait::Complete);
        assert!(
            pending_polls > 0,
            "cleanup must observe the delayed child alive"
        );
        assert_eq!(
            classify_login_denial_after_cleanup(
                TestDenial::AccountRejected,
                true,
                |denial| matches!(denial, TestDenial::AccountRejected),
                TestDenial::UnresolvedSpawnCleanup,
                TestDenial::RequestOutcomeUnknown,
            ),
            TestDenial::AccountRejected
        );

        // Once the exact child is reaped and no cleanup is unresolved, Escape
        // follows the ordinary cancellation path.
        let cancelled = VerifiedLoginCancellation::from_input_close(
            || Ok(()),
            || 0,
            || Ok(false),
            TestDenial::UnresolvedSpawnCleanup,
        );
        assert!(cancelled.is_ok());
    }

    #[test]
    fn unknown_or_expired_login_cleanup_stays_fail_closed() {
        assert_eq!(
            wait_for_login_cleanup_until(
                Instant::now() + Duration::from_secs(1),
                || LoginCleanupPoll::Unknown,
                |_| {},
            ),
            LoginCleanupWait::Unknown
        );
        assert_eq!(
            wait_for_login_cleanup_until(Instant::now(), || LoginCleanupPoll::Complete, |_| {}),
            LoginCleanupWait::DeadlineExpired
        );
        assert_eq!(
            wait_for_login_cleanup_until(
                Instant::now() + Duration::from_millis(1),
                || {
                    std::thread::sleep(Duration::from_millis(5));
                    LoginCleanupPoll::Complete
                },
                |_| panic!("must not wait after a poll crosses the deadline"),
            ),
            LoginCleanupWait::DeadlineExpired
        );
    }
}
