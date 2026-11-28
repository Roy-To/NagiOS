//! Pure checks for the mount flags reported by Linux `fstatfs(2)`.

pub(crate) const ST_RDONLY: u64 = 1;
pub(crate) const ST_NOSUID: u64 = 2;
pub(crate) const ST_NODEV: u64 = 4;
pub(crate) const ST_NOEXEC: u64 = 8;
pub(crate) const ST_VALID: u64 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationError {
    Unsupported {
        actual: u64,
    },
    Missing {
        required: u64,
        actual: u64,
        missing: u64,
    },
    ReadOnly {
        actual: u64,
    },
}

pub(crate) fn verify(required: u64, actual: u64) -> Result<(), VerificationError> {
    if actual & ST_VALID == 0 {
        return Err(VerificationError::Unsupported { actual });
    }

    let missing = required & !actual;
    if missing != 0 {
        return Err(VerificationError::Missing {
            required,
            actual,
            missing,
        });
    }
    Ok(())
}

pub(crate) fn verify_rw(required: u64, actual: u64) -> Result<(), VerificationError> {
    if actual & ST_RDONLY != 0 {
        return Err(VerificationError::ReadOnly { actual });
    }
    verify(required, actual)
}

#[cfg(test)]
mod tests {
    use super::{
        verify, verify_rw, VerificationError, ST_NODEV, ST_NOEXEC, ST_NOSUID, ST_RDONLY, ST_VALID,
    };

    const PROC_REQUIRED: u64 = ST_NOSUID | ST_NODEV | ST_NOEXEC;
    const SYS_REQUIRED: u64 = ST_RDONLY | PROC_REQUIRED;

    #[test]
    fn missing_flags_are_computed_from_the_actual_flags() {
        let actual = ST_VALID | ST_NOSUID;
        assert_eq!(
            verify(PROC_REQUIRED, actual),
            Err(VerificationError::Missing {
                required: PROC_REQUIRED,
                actual,
                missing: PROC_REQUIRED & !actual,
            })
        );
    }

    #[test]
    fn missing_st_valid_fails_closed_even_when_required_bits_are_set() {
        let actual = PROC_REQUIRED;
        assert_eq!(
            verify(PROC_REQUIRED, actual),
            Err(VerificationError::Unsupported { actual })
        );
    }

    #[test]
    fn sysfs_requires_read_only_in_addition_to_shared_flags() {
        let actual = ST_VALID | PROC_REQUIRED;
        assert_eq!(
            verify(SYS_REQUIRED, actual),
            Err(VerificationError::Missing {
                required: SYS_REQUIRED,
                actual,
                missing: SYS_REQUIRED & !actual,
            })
        );
    }

    #[test]
    fn all_required_flags_are_accepted_when_st_valid_is_present() {
        assert_eq!(verify(PROC_REQUIRED, PROC_REQUIRED | ST_VALID), Ok(()));
        assert_eq!(verify(SYS_REQUIRED, SYS_REQUIRED | ST_VALID), Ok(()));
    }

    #[test]
    fn data_rw_flags_reject_read_only_even_when_other_flags_are_present() {
        assert_eq!(
            verify_rw(PROC_REQUIRED, PROC_REQUIRED | ST_RDONLY | ST_VALID),
            Err(VerificationError::ReadOnly {
                actual: PROC_REQUIRED | ST_RDONLY | ST_VALID,
            })
        );
        assert_eq!(verify_rw(PROC_REQUIRED, PROC_REQUIRED | ST_VALID), Ok(()));
    }
}
