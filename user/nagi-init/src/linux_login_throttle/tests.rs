use super::*;
use nagi_account_credential::FREE_ATTEMPTS;
use std::cell::Cell;

#[test]
fn third_rejection_blocks_verification_until_the_deadline() {
    let start = Instant::now();
    let clock = Cell::new(start);
    let mut failures = LoginFailureCount::restored_at(FREE_ATTEMPTS - 1, start);
    assert_eq!(
        failures.attempt(
            || clock.get(),
            || {
                clock.set(start + Duration::from_secs(1));
                Err::<(), _>(())
            },
            |_| true,
            |count| {
                assert_eq!(count, FREE_ATTEMPTS);
                Ok(())
            }
        ),
        Err(LoginAttemptError::Verification(()))
    );
    let deadline = clock.get() + Duration::from_millis(throttle_wait_ms(FREE_ATTEMPTS));
    for time in [clock.get(), deadline - Duration::from_nanos(1)] {
        assert_eq!(
            failures.attempt(
                || time,
                || -> Result<(), ()> { panic!("wait must precede verification") },
                |_| panic!("a blocked attempt cannot be rejected"),
                |_| panic!("a blocked attempt cannot persist a failure")
            ),
            Err(LoginAttemptError::Throttled)
        );
    }
    assert_eq!(
        failures.attempt(|| deadline, || Ok(()), |_: &()| true, |_| Ok(())),
        Ok(())
    );
    assert_eq!(failures.current(), FREE_ATTEMPTS);
    failures
        .persist_at(
            0,
            || deadline,
            |count| {
                assert_eq!(count, 0);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(failures.current(), 0);
    assert_eq!(
        failures.attempt(|| deadline, || Ok(()), |_: &()| true, |_| Ok(())),
        Ok(())
    );
}

#[test]
fn restored_failures_restart_the_wait_and_store_error_fails_closed() {
    let start = Instant::now();
    let mut failures = LoginFailureCount::restored_at(FREE_ATTEMPTS, start);
    let verify = || -> Result<(), ()> { panic!("verification must be skipped") };
    assert_eq!(
        failures.attempt(|| start, verify, |_| true, |_| Ok(())),
        Err(LoginAttemptError::Throttled)
    );
    let deadline = start + Duration::from_millis(throttle_wait_ms(FREE_ATTEMPTS));
    assert_eq!(
        failures.attempt(
            || deadline,
            || Err::<(), _>(()),
            |_| true,
            |_| Err(io::Error::other("fixture store unavailable"))
        ),
        Err(LoginAttemptError::PersistenceUnavailable)
    );
    assert_eq!(failures.current(), FREE_ATTEMPTS);
    assert_eq!(
        failures.attempt(
            || deadline + Duration::from_secs(60),
            verify,
            |_| true,
            |_| Ok(())
        ),
        Err(LoginAttemptError::PersistenceUnavailable)
    );
}
