#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeError {
    Unsupported,
}

pub const TIMER_TICK_NS: u64 = 10_000_000;

pub fn ticks_to_ns(ticks: u64) -> Option<u64> {
    ticks.checked_mul(TIMER_TICK_NS)
}

/// The clock interface is explicit so callers cannot accidentally fall back
/// to a host clock or a loop counter.
pub trait Clock {
    fn monotonic_ns(&self) -> Result<u64, TimeError>;
    fn realtime_ns(&self) -> Result<u64, TimeError>;
    fn sleep_ns(&self, _duration: u64) -> Result<(), TimeError> {
        Err(TimeError::Unsupported)
    }
}

pub struct UnavailableClock;

impl Clock for UnavailableClock {
    fn monotonic_ns(&self) -> Result<u64, TimeError> {
        Err(TimeError::Unsupported)
    }

    fn realtime_ns(&self) -> Result<u64, TimeError> {
        Err(TimeError::Unsupported)
    }
}

/// Guest clock backed by the kernel's APIC timer and guest epoch contract.
pub struct GuestClock;

impl Clock for GuestClock {
    fn monotonic_ns(&self) -> Result<u64, TimeError> {
        #[cfg(target_os = "nagi")]
        {
            ticks_to_ns(libnagi::time_ticks()).ok_or(TimeError::Unsupported)
        }
        #[cfg(not(target_os = "nagi"))]
        {
            Err(TimeError::Unsupported)
        }
    }

    fn realtime_ns(&self) -> Result<u64, TimeError> {
        #[cfg(target_os = "nagi")]
        {
            libnagi::time_realtime_ns().ok_or(TimeError::Unsupported)
        }
        #[cfg(not(target_os = "nagi"))]
        {
            Err(TimeError::Unsupported)
        }
    }

    fn sleep_ns(&self, duration: u64) -> Result<(), TimeError> {
        #[cfg(target_os = "nagi")]
        {
            libnagi::sleep_ns(duration)
                .then_some(())
                .ok_or(TimeError::Unsupported)
        }
        #[cfg(not(target_os = "nagi"))]
        {
            let _ = duration;
            Err(TimeError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ticks_to_ns, Clock, TimeError, UnavailableClock};

    #[test]
    fn clock_does_not_fall_back_to_the_host() {
        let clock = UnavailableClock;
        assert_eq!(clock.monotonic_ns(), Err(TimeError::Unsupported));
        assert_eq!(clock.realtime_ns(), Err(TimeError::Unsupported));
    }

    #[test]
    fn guest_timer_ticks_use_the_documented_coarse_unit() {
        assert_eq!(ticks_to_ns(3), Some(30_000_000));
    }
}
