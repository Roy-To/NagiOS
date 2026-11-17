//! Safe Linux OS inputs. No fixed, sequential, file-device or time ID fallback.

// Cargo does not inherit a path dependency's .cargo/config.toml. Require the
// integrating build to select the same syscall-only backend explicitly too.
#[cfg(not(getrandom_backend = "linux_getrandom"))]
compile_error!("persistent issuer requires --cfg getrandom_backend=\"linux_getrandom\" in the integrating build's RUSTFLAGS");

use identity::{
    Clock, IdSourceFailure, IdentityIdSource, ProfileId, SessionId, TryClockError, UnixMillis,
    UserId,
};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub(super) struct IssuerClock(Arc<Mutex<ClockState>>);

struct ClockState {
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

impl IssuerClock {
    pub(super) fn open(persisted_floor: u64) -> Result<Self, ()> {
        let wall = wall_millis().ok_or(())?;
        if wall < persisted_floor || wall == u64::MAX {
            return Err(());
        }
        Ok(Self(Arc::new(Mutex::new(ClockState {
            base: wall,
            started: Instant::now(),
            last_wall: wall,
            last_returned: wall,
            failed: false,
        }))))
    }

    pub(super) fn checked_now(&self) -> Result<UnixMillis, ()> {
        self.sample(wall_millis)
    }

    #[cfg(test)]
    pub(super) fn fail_for_test(&self) {
        let _ = self.observe(None);
    }

    #[cfg(test)]
    fn observe(&self, wall: Option<u64>) -> Result<UnixMillis, ()> {
        self.sample(|| wall)
    }

    fn sample(&self, source: impl FnOnce() -> Option<u64>) -> Result<UnixMillis, ()> {
        let mut state = self.0.lock().map_err(|_| ())?;
        Self::sample_locked(&mut state, source)
    }

    fn try_sample(
        &self,
        source: impl FnOnce() -> Option<u64>,
    ) -> Result<UnixMillis, TryClockError> {
        let mut state = self.0.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => TryClockError::Busy,
            std::sync::TryLockError::Poisoned(_) => TryClockError::Unavailable,
        })?;
        Self::sample_locked(&mut state, source).map_err(|_| TryClockError::Unavailable)
    }

    #[cfg(test)]
    pub(super) fn while_locked_for_test<R>(&self, operation: impl FnOnce() -> R) -> R {
        let _held = self.0.lock().unwrap();
        operation()
    }

    fn sample_locked(
        state: &mut ClockState,
        source: impl FnOnce() -> Option<u64>,
    ) -> Result<UnixMillis, ()> {
        // Sample under the same shared lock. Reading before locking can make
        // concurrent clones observe a normal earlier sample after a later one
        // and incorrectly latch it as an OS clock rollback.
        let wall = source();
        let Some(wall) = wall.filter(|wall| *wall >= state.last_wall && *wall != u64::MAX) else {
            state.failed = true;
            return Err(());
        };
        let monotonic = u64::try_from(state.started.elapsed().as_millis())
            .ok()
            .and_then(|elapsed| state.base.checked_add(elapsed));
        let Some(monotonic) = monotonic.filter(|value| *value != u64::MAX) else {
            state.failed = true;
            return Err(());
        };
        if state.failed {
            return Err(());
        }
        state.last_wall = wall;
        state.last_returned = wall.max(monotonic).max(state.last_returned);
        Ok(UnixMillis::new(state.last_returned))
    }
}

impl Clock for IssuerClock {
    fn now(&self) -> UnixMillis {
        // Clock has no failure variant. The runtime only issues finite leases;
        // MAX expires those leases, while checked admission/store commits fail.
        self.checked_now().unwrap_or(UnixMillis::new(u64::MAX))
    }
    fn try_now(&self) -> Result<UnixMillis, TryClockError> {
        // The SAME Arc/Mutex clock also belongs to FileSnapshotStore and the
        // issuing SDK service. Never construct a second clock for a consumer.
        self.try_sample(wall_millis)
    }
}

pub(super) struct LinuxIds;

pub(super) fn entropy_hex() -> Result<String, IdSourceFailure> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| IdSourceFailure::Unavailable)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(IdSourceFailure::Unavailable);
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl IdentityIdSource for LinuxIds {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| IdSourceFailure::Unavailable)?;
        UserId::new(u64::from_le_bytes(bytes)).map_err(|_| IdSourceFailure::Unavailable)
    }
    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        ProfileId::new(format!("profile_{}", entropy_hex()?))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        SessionId::new(format!("session_{}", entropy_hex()?))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_linux_clock_and_entropy_satisfy_sdk_types() {
        let clock = IssuerClock::open(0).unwrap();
        assert!(clock.checked_now().unwrap().get() > 0);
        let mut ids = LinuxIds;
        assert_ne!(ids.next_user_id().unwrap().get(), 0);
        assert_ne!(
            ids.next_profile_id().unwrap(),
            ids.next_profile_id().unwrap()
        );
        assert_ne!(
            ids.next_session_id().unwrap(),
            ids.next_session_id().unwrap()
        );
    }
    #[test]
    fn clock_rollback_is_latched_and_future_start_floor_is_rejected() {
        let clock = IssuerClock::open(0).unwrap();
        let shared = clock.clone();
        assert!(clock.observe(Some(0)).is_err());
        assert!(shared.checked_now().is_err());
        assert_eq!(shared.now().get(), u64::MAX);
        assert!(IssuerClock::open(u64::MAX).is_err());
    }

    #[test]
    fn shared_clock_serializes_sampling_before_comparing_wall_time() {
        let clock = IssuerClock::open(0).unwrap();
        let base = clock.0.lock().unwrap().last_wall;
        let (sampled, ready) = std::sync::mpsc::channel();
        let first = clock.clone();
        let delayed = std::thread::spawn(move || {
            first.sample(|| {
                sampled.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(30));
                Some(base)
            })
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let second = clock.clone();
        let fast = std::thread::spawn(move || second.sample(|| Some(base + 1)));
        assert!(fast.join().unwrap().is_ok());
        assert!(delayed.join().unwrap().is_ok());
    }
}
