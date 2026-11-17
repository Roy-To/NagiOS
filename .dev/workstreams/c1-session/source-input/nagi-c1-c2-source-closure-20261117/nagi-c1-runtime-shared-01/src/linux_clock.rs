//! Candidate of d26d's shared issuer clock, adding one try_lock path. The same
//! state/source governs normal issuer now() and external try_now(). No file,
//! device, background sampler, credential or clock-setting operation is used.

use identity::{Clock, TryClockError, UnixMillis};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct LinuxClock(Arc<Mutex<State>>);

struct State {
    base: u64,
    started: Instant,
    last_wall: u64,
    last_returned: u64,
    failed: bool,
}

fn wall_millis() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

impl LinuxClock {
    /// The persistent issuer must supply the durable floor from its real store.
    /// Passing 0 in a test does not verify restart/store generation consistency.
    pub fn open(persisted_floor: u64) -> Result<Self, TryClockError> {
        let wall = wall_millis()
            .filter(|wall| *wall >= persisted_floor && *wall != u64::MAX)
            .ok_or(TryClockError::Unavailable)?;
        Ok(Self(Arc::new(Mutex::new(State {
            base: wall,
            started: Instant::now(),
            last_wall: wall,
            last_returned: wall,
            failed: false,
        }))))
    }

    fn sample(
        &self,
        nonwaiting: bool,
        source: impl FnOnce() -> Option<u64>,
    ) -> Result<UnixMillis, TryClockError> {
        let mut state = if nonwaiting {
            self.0.try_lock().map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => TryClockError::Busy,
                std::sync::TryLockError::Poisoned(_) => TryClockError::Unavailable,
            })?
        } else {
            self.0.lock().map_err(|_| TryClockError::Unavailable)?
        };
        if state.failed {
            return Err(TryClockError::Unavailable);
        }
        // Sampling while holding the one shared lock avoids mistaking delayed
        // concurrent samples for clock rollback, as in the fixed d26d clock.
        let Some(wall) = source().filter(|wall| *wall >= state.last_wall && *wall != u64::MAX)
        else {
            state.failed = true;
            return Err(TryClockError::Unavailable);
        };
        let Some(monotonic) = u64::try_from(state.started.elapsed().as_millis())
            .ok()
            .and_then(|elapsed| state.base.checked_add(elapsed))
            .filter(|n| *n != u64::MAX)
        else {
            state.failed = true;
            return Err(TryClockError::Unavailable);
        };
        state.last_wall = wall;
        state.last_returned = wall.max(monotonic).max(state.last_returned);
        Ok(UnixMillis::new(state.last_returned))
    }
}

impl Clock for LinuxClock {
    fn now(&self) -> UnixMillis {
        self.sample(false, wall_millis)
            .unwrap_or(UnixMillis::new(u64::MAX))
    }
    fn try_now(&self) -> Result<UnixMillis, TryClockError> {
        self.sample(true, wall_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_clock_returns_without_waiting_or_sampling_and_recovers_after_unlock() {
        let clock = LinuxClock::open(0).unwrap();
        let held = clock.0.lock().unwrap();
        assert_eq!(
            clock.sample(true, || panic!("sampled while busy")),
            Err(TryClockError::Busy)
        );
        drop(held);
        assert!(clock.try_now().is_ok());
    }

    #[test]
    fn rollback_latches_across_clones_and_normal_clock_fails_closed() {
        let clock = LinuxClock::open(0).unwrap();
        let clone = clock.clone();
        assert_eq!(
            clock.sample(true, || Some(0)),
            Err(TryClockError::Unavailable)
        );
        assert_eq!(clone.try_now(), Err(TryClockError::Unavailable));
        assert_eq!(clone.now().get(), u64::MAX);
        assert!(matches!(
            LinuxClock::open(u64::MAX),
            Err(TryClockError::Unavailable)
        ));
    }

    #[test]
    fn real_os_clock_uses_shared_authoritative_samples() {
        let clock = LinuxClock::open(0).unwrap();
        let first = clock.try_now().unwrap();
        assert!(first.get() > 0);
        assert!(clock.clone().now() >= first);
        assert!(clock.try_now().unwrap() >= first);
    }
}
