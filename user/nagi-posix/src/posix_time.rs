//! Shared POSIX clock conversion used by the Nagi ABI and Linux smoke tests.
//!
//! The Linux build exercises the same user-space PAL consumer without
//! exporting the Nagi C ABI or adding Linux behavior to the Nagi kernel.

use crate::errno::{EINVAL, ENOSYS};
use nagi_pal::time::Clock;

const CLOCK_REALTIME: i32 = 1;
const CLOCK_MONOTONIC: i32 = 4;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PosixTimespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// Read a POSIX clock through the supplied PAL clock and write a `timespec`.
///
/// `output` must be null or point to a writable `PosixTimespec`. On failure,
/// the output is left unchanged and the returned value is a POSIX errno.
pub(crate) unsafe fn clock_gettime<C: Clock>(
    clock_id: i32,
    output: *mut PosixTimespec,
    clock: &C,
) -> Result<(), i32> {
    if output.is_null() {
        return Err(EINVAL);
    }

    let nanoseconds = match clock_id {
        CLOCK_REALTIME => clock.realtime_ns(),
        CLOCK_MONOTONIC => clock.monotonic_ns(),
        _ => return Err(EINVAL),
    }
    .map_err(|_| ENOSYS)?;

    let seconds = nanoseconds / NANOS_PER_SECOND;
    let seconds = i64::try_from(seconds).map_err(|_| ENOSYS)?;
    let result = PosixTimespec {
        tv_sec: seconds,
        tv_nsec: (nanoseconds % NANOS_PER_SECOND) as i64,
    };

    // SAFETY: the caller guarantees that non-null `output` is writable.
    unsafe { output.write(result) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clock_gettime, PosixTimespec};
    use crate::errno::{EINVAL, ENOSYS};
    use nagi_pal::time::{Clock, TimeError};

    struct FixedClock {
        monotonic: Result<u64, TimeError>,
        realtime: Result<u64, TimeError>,
    }

    impl Clock for FixedClock {
        fn monotonic_ns(&self) -> Result<u64, TimeError> {
            self.monotonic
        }

        fn realtime_ns(&self) -> Result<u64, TimeError> {
            self.realtime
        }
    }

    #[test]
    fn monotonic_time_is_converted_to_posix_timespec() {
        let clock = FixedClock {
            monotonic: Ok(4_000_000_005),
            realtime: Err(TimeError::Unsupported),
        };
        let mut output = PosixTimespec::default();
        assert_eq!(unsafe { clock_gettime(4, &mut output, &clock) }, Ok(()));
        assert_eq!(
            output,
            PosixTimespec {
                tv_sec: 4,
                tv_nsec: 5,
            }
        );
    }

    #[test]
    fn unsupported_clock_fails_without_changing_output() {
        let clock = FixedClock {
            monotonic: Err(TimeError::Unsupported),
            realtime: Err(TimeError::Unsupported),
        };
        let mut output = PosixTimespec {
            tv_sec: 17,
            tv_nsec: 19,
        };
        assert_eq!(
            unsafe { clock_gettime(4, &mut output, &clock) },
            Err(ENOSYS)
        );
        assert_eq!(
            unsafe { clock_gettime(1, &mut output, &clock) },
            Err(ENOSYS)
        );
        assert_eq!(output.tv_sec, 17);
        assert_eq!(output.tv_nsec, 19);
    }

    #[test]
    fn invalid_clock_and_null_output_fail_closed() {
        let clock = FixedClock {
            monotonic: Ok(1),
            realtime: Ok(1),
        };
        let mut output = PosixTimespec {
            tv_sec: 23,
            tv_nsec: 29,
        };
        assert_eq!(
            unsafe { clock_gettime(99, &mut output, &clock) },
            Err(EINVAL)
        );
        assert_eq!(
            unsafe { clock_gettime(4, core::ptr::null_mut(), &clock) },
            Err(EINVAL)
        );
        assert_eq!(output.tv_sec, 23);
        assert_eq!(output.tv_nsec, 29);
    }
}
