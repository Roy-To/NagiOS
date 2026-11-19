use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use nagi_identity_session::{Clock, TryClockError, UnixMillis};

#[derive(Clone)]
pub struct LinuxIdentityClock(Arc<Mutex<ClockState>>);

struct ClockState {
    base: u64,
    started: Instant,
    last_wall: u64,
    last_returned: u64,
    failed: bool,
}

impl LinuxIdentityClock {
    pub fn open() -> Result<Self, ClockFailure> {
        let wall = wall_millis().ok_or(ClockFailure::Unavailable)?;
        if wall == u64::MAX {
            return Err(ClockFailure::Unavailable);
        }
        Ok(Self(Arc::new(Mutex::new(ClockState {
            base: wall,
            started: Instant::now(),
            last_wall: wall,
            last_returned: wall,
            failed: false,
        }))))
    }

    pub fn checked_now(&self) -> Result<UnixMillis, ClockFailure> {
        self.sample(wall_millis)
    }

    fn sample(&self, source: impl FnOnce() -> Option<u64>) -> Result<UnixMillis, ClockFailure> {
        let mut state = self.0.lock().map_err(|_| ClockFailure::Unavailable)?;
        sample_locked(&mut state, source)
    }

    #[cfg(test)]
    fn observe_wall(&self, wall: Option<u64>) -> Result<UnixMillis, ClockFailure> {
        self.sample(|| wall)
    }
}

impl Clock for LinuxIdentityClock {
    fn now(&self) -> UnixMillis {
        self.checked_now().unwrap_or(UnixMillis::new(u64::MAX))
    }

    fn try_now(&self) -> Result<UnixMillis, TryClockError> {
        let mut state = match self.0.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => return Err(TryClockError::Busy),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(TryClockError::Unavailable),
        };
        sample_locked(&mut state, wall_millis).map_err(|_| TryClockError::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockFailure {
    Unavailable,
}

fn wall_millis() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn sample_locked(
    state: &mut ClockState,
    source: impl FnOnce() -> Option<u64>,
) -> Result<UnixMillis, ClockFailure> {
    // Sample while holding the shared lock. Sampling first could make a
    // concurrent later observation appear to be a wall-clock rollback.
    let wall = source().filter(|value| *value >= state.last_wall && *value != u64::MAX);
    let Some(wall) = wall else {
        state.failed = true;
        return Err(ClockFailure::Unavailable);
    };
    let monotonic = u64::try_from(state.started.elapsed().as_millis())
        .ok()
        .and_then(|elapsed| state.base.checked_add(elapsed))
        .filter(|value| *value != u64::MAX);
    let Some(monotonic) = monotonic else {
        state.failed = true;
        return Err(ClockFailure::Unavailable);
    };
    if state.failed {
        return Err(ClockFailure::Unavailable);
    }
    state.last_wall = wall;
    state.last_returned = wall.max(monotonic).max(state.last_returned);
    Ok(UnixMillis::new(state.last_returned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_rollback_is_latched_across_all_clones() {
        let clock = LinuxIdentityClock::open().unwrap();
        let clone = clock.clone();
        let current = clock.checked_now().unwrap().get();
        assert!(clock.observe_wall(Some(current + 1)).is_ok());
        assert_eq!(
            clone.observe_wall(Some(current)),
            Err(ClockFailure::Unavailable)
        );
        assert_eq!(clock.now().get(), u64::MAX);
        assert_eq!(clock.try_now(), Err(TryClockError::Unavailable));
    }

    #[test]
    fn nonblocking_clock_reports_busy_without_waiting_for_the_shared_lock() {
        let clock = LinuxIdentityClock::open().unwrap();
        let guard = clock.0.lock().unwrap();
        assert_eq!(clock.try_now(), Err(TryClockError::Busy));
        drop(guard);
        assert!(clock.try_now().is_ok());
    }

    #[test]
    fn unavailable_wall_time_and_overflow_fail_closed() {
        let clock = LinuxIdentityClock::open().unwrap();
        assert_eq!(clock.observe_wall(None), Err(ClockFailure::Unavailable));
        assert_eq!(clock.now().get(), u64::MAX);
        assert!(LinuxIdentityClock::open().is_ok());

        let mut state = ClockState {
            base: u64::MAX,
            started: Instant::now(),
            last_wall: 1,
            last_returned: 1,
            failed: false,
        };
        assert_eq!(
            sample_locked(&mut state, || Some(1)),
            Err(ClockFailure::Unavailable)
        );
    }
}
