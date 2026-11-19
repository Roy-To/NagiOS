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

/// Guest clock backed by the active platform clock contract.
pub struct GuestClock;

impl Clock for GuestClock {
    fn monotonic_ns(&self) -> Result<u64, TimeError> {
        #[cfg(target_os = "nagi")]
        {
            ticks_to_ns(libnagi::time_ticks()).ok_or(TimeError::Unsupported)
        }
        #[cfg(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_pointer_width = "64"
        ))]
        {
            linux_clock::monotonic_ns()
        }
        #[cfg(not(any(
            target_os = "nagi",
            all(
                target_os = "linux",
                target_arch = "x86_64",
                target_pointer_width = "64"
            )
        )))]
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

// Linux x86_64 64-bit ABI uses two signed 64-bit C longs in timespec and
// syscall 228 for clock_gettime. Other Linux ABIs stay explicitly unsupported.
#[cfg(any(
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ),
    all(
        test,
        target_pointer_width = "64",
        any(target_os = "macos", target_os = "linux")
    )
))]
mod linux_clock {
    use super::TimeError;
    use core::ffi::{c_int, c_long};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum LinuxClockError {
        SyscallFailed,
        InvalidTimespec,
        Overflow,
    }

    const CLOCK_MONOTONIC: c_int = 1;
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    const SYS_CLOCK_GETTIME: c_long = 228;
    const NANOS_PER_SECOND: u64 = 1_000_000_000;

    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct LinuxTimespec {
        tv_sec: c_long,
        tv_nsec: c_long,
    }

    fn timespec_to_ns(timespec: LinuxTimespec) -> Result<u64, LinuxClockError> {
        if timespec.tv_sec < 0 || timespec.tv_nsec < 0 {
            return Err(LinuxClockError::InvalidTimespec);
        }
        let nanoseconds = timespec.tv_nsec as u64;
        if nanoseconds >= NANOS_PER_SECOND {
            return Err(LinuxClockError::InvalidTimespec);
        }

        (timespec.tv_sec as u64)
            .checked_mul(NANOS_PER_SECOND)
            .and_then(|seconds| seconds.checked_add(nanoseconds))
            .ok_or(LinuxClockError::Overflow)
    }

    fn monotonic_ns_with(
        mut clock_gettime: impl FnMut(c_int, &mut LinuxTimespec) -> c_long,
    ) -> Result<u64, LinuxClockError> {
        let mut timespec = LinuxTimespec::default();
        if clock_gettime(CLOCK_MONOTONIC, &mut timespec) != 0 {
            return Err(LinuxClockError::SyscallFailed);
        }
        timespec_to_ns(timespec)
    }

    fn public_result(result: Result<u64, LinuxClockError>) -> Result<u64, TimeError> {
        result.map_err(|_| TimeError::Unsupported)
    }

    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    pub(super) fn monotonic_ns() -> Result<u64, TimeError> {
        use core::arch::asm;

        unsafe fn clock_gettime_syscall(clock_id: c_int, timespec: &mut LinuxTimespec) -> c_long {
            let result: c_long;
            asm!(
                "syscall",
                inlateout("rax") SYS_CLOCK_GETTIME => result,
                in("rdi") clock_id as c_long,
                in("rsi") timespec as *mut LinuxTimespec as c_long,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
            result
        }

        public_result(monotonic_ns_with(|clock_id, timespec| unsafe {
            clock_gettime_syscall(clock_id, timespec)
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::{
            monotonic_ns_with, public_result, timespec_to_ns, LinuxClockError, LinuxTimespec,
        };
        use crate::TimeError;
        use core::ffi::c_long;
        use core::mem::{align_of, offset_of, size_of};

        #[cfg(target_pointer_width = "64")]
        #[test]
        fn linux_64_bit_timespec_uses_two_c_longs() {
            assert_eq!(size_of::<LinuxTimespec>(), 16);
            assert_eq!(align_of::<LinuxTimespec>(), 8);
            assert_eq!(offset_of!(LinuxTimespec, tv_sec), 0);
            assert_eq!(offset_of!(LinuxTimespec, tv_nsec), 8);
        }

        #[test]
        fn conversion_preserves_seconds_and_nanoseconds() {
            assert_eq!(
                timespec_to_ns(LinuxTimespec {
                    tv_sec: 2,
                    tv_nsec: 3,
                }),
                Ok(2_000_000_003)
            );
        }

        #[test]
        fn syscall_failure_is_returned_without_using_output() {
            let result = monotonic_ns_with(|clock_id, output| {
                assert_eq!(clock_id, 1);
                output.tv_sec = -1;
                -22
            });
            assert_eq!(result, Err(LinuxClockError::SyscallFailed));
            assert_eq!(public_result(result), Err(TimeError::Unsupported));
        }

        #[test]
        fn negative_seconds_are_invalid() {
            assert_eq!(
                timespec_to_ns(LinuxTimespec {
                    tv_sec: -1,
                    tv_nsec: 0,
                }),
                Err(LinuxClockError::InvalidTimespec)
            );
        }

        #[test]
        fn nanoseconds_must_be_in_the_normalized_range() {
            for tv_nsec in [-1, 1_000_000_000] {
                assert_eq!(
                    timespec_to_ns(LinuxTimespec { tv_sec: 1, tv_nsec }),
                    Err(LinuxClockError::InvalidTimespec)
                );
            }
        }

        #[cfg(target_pointer_width = "64")]
        #[test]
        fn conversion_overflow_is_reported() {
            assert_eq!(
                timespec_to_ns(LinuxTimespec {
                    tv_sec: c_long::MAX,
                    tv_nsec: 0,
                }),
                Err(LinuxClockError::Overflow)
            );
        }

        #[test]
        fn injected_success_uses_clock_monotonic() {
            let value = monotonic_ns_with(|clock_id, output| {
                assert_eq!(clock_id, 1);
                output.tv_sec = 4;
                output.tv_nsec = 5;
                0
            });
            assert_eq!(value, Ok(4_000_000_005));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ticks_to_ns, Clock, GuestClock, TimeError, UnavailableClock};

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

    #[cfg(not(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    )))]
    #[test]
    fn guest_clock_remains_unsupported_outside_linux_x86_64() {
        let clock = GuestClock;
        assert_eq!(clock.monotonic_ns(), Err(TimeError::Unsupported));
        assert_eq!(clock.realtime_ns(), Err(TimeError::Unsupported));
    }

    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_pointer_width = "64"
    ))]
    #[test]
    fn linux_guest_clock_reads_real_monotonic_time_and_fails_closed_elsewhere() {
        let clock = GuestClock;
        let first = clock
            .monotonic_ns()
            .expect("CLOCK_MONOTONIC must be available");
        let second = clock
            .monotonic_ns()
            .expect("CLOCK_MONOTONIC must be available");
        assert!(second >= first);
        assert_eq!(clock.realtime_ns(), Err(TimeError::Unsupported));
        assert_eq!(clock.sleep_ns(1), Err(TimeError::Unsupported));
    }
}
