//! Fail-closed stdio delivery for a recovery notification.
//!
//! The owner and pending bytes stay together across short writes, `WouldBlock`,
//! closed descriptors, and broken pipes. `Delivered` means the writer accepted
//! all bytes and its `flush` returned success; it does not mean a person saw or
//! acknowledged the message, and it never means the draft was saved.

use std::io::{self, ErrorKind, Write};

const POSIX_EBADF: i32 = 9;
const POSIX_EPIPE: i32 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryReason {
    ShortWrite,
    Interrupted,
    WouldBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureReason {
    BrokenPipe,
    ClosedDescriptor,
    WriteZero,
    InvalidWriteCount,
    Other {
        kind: ErrorKind,
        raw_os_error: Option<i32>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteProgress {
    Delivered {
        written: usize,
    },
    Retry {
        written: usize,
        reason: RetryReason,
    },
    Failed {
        written: usize,
        reason: FailureReason,
    },
}

/// Owns a recovery owner and its notification until the output is complete.
///
/// Callers must keep this value on every `Retry` or `Failed` result. The
/// adapter does not modify save/discard state. A separate successful save or
/// explicit discard is required before a dirty owner may be released.
pub struct PendingStdioNotification<O> {
    owner: O,
    bytes: Vec<u8>,
    written: usize,
    delivered: bool,
    last_progress: Option<WriteProgress>,
}

impl<O> PendingStdioNotification<O> {
    pub fn new(owner: O, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            owner,
            bytes: bytes.into(),
            written: 0,
            delivered: false,
            last_progress: None,
        }
    }

    pub fn owner(&self) -> &O {
        &self.owner
    }

    pub fn written(&self) -> usize {
        self.written
    }

    pub fn remaining(&self) -> &[u8] {
        &self.bytes[self.written..]
    }

    pub fn is_delivered(&self) -> bool {
        self.delivered
    }

    pub fn last_progress(&self) -> Option<WriteProgress> {
        self.last_progress
    }

    fn record(&mut self, progress: WriteProgress) -> WriteProgress {
        self.last_progress = Some(progress);
        progress
    }

    /// Make at most one write and, once all bytes have been accepted, one
    /// flush. The owner and remaining payload stay in `self` for every result.
    pub fn try_write<W: Write>(&mut self, writer: &mut W) -> WriteProgress {
        if self.delivered {
            return self.record(WriteProgress::Delivered {
                written: self.written,
            });
        }
        if self.bytes.is_empty() {
            self.delivered = true;
            return self.record(WriteProgress::Delivered { written: 0 });
        }

        if self.written < self.bytes.len() {
            let remaining = &self.bytes[self.written..];
            match writer.write(remaining) {
                Ok(0) => {
                    return self.record(WriteProgress::Failed {
                        written: self.written,
                        reason: FailureReason::WriteZero,
                    });
                }
                Ok(count) if count <= remaining.len() => {
                    self.written += count;
                    if self.written < self.bytes.len() {
                        return self.record(WriteProgress::Retry {
                            written: self.written,
                            reason: RetryReason::ShortWrite,
                        });
                    }
                }
                Ok(_) => {
                    return self.record(WriteProgress::Failed {
                        written: self.written,
                        reason: FailureReason::InvalidWriteCount,
                    });
                }
                Err(error) => {
                    return self.record(classify_error(error, self.written));
                }
            }
        }

        let progress = match writer.flush() {
            Ok(()) => {
                self.delivered = true;
                WriteProgress::Delivered {
                    written: self.written,
                }
            }
            Err(error) => classify_error(error, self.written),
        };
        self.record(progress)
    }

    /// Transfer the owner and the complete payload/cursor together.
    pub fn into_parts(self) -> (O, Vec<u8>, usize, bool, Option<WriteProgress>) {
        (
            self.owner,
            self.bytes,
            self.written,
            self.delivered,
            self.last_progress,
        )
    }
}

fn classify_error(error: io::Error, written: usize) -> WriteProgress {
    match error.kind() {
        ErrorKind::Interrupted => WriteProgress::Retry {
            written,
            reason: RetryReason::Interrupted,
        },
        ErrorKind::WouldBlock => WriteProgress::Retry {
            written,
            reason: RetryReason::WouldBlock,
        },
        _ => WriteProgress::Failed {
            written,
            reason: classify_failure(&error),
        },
    }
}

fn classify_failure(error: &io::Error) -> FailureReason {
    let raw_os_error = error.raw_os_error();
    if error.kind() == ErrorKind::BrokenPipe || raw_os_error == Some(POSIX_EPIPE) {
        FailureReason::BrokenPipe
    } else if raw_os_error == Some(POSIX_EBADF) {
        // EBADF on Linux and the supported POSIX host test target.
        FailureReason::ClosedDescriptor
    } else {
        FailureReason::Other {
            kind: error.kind(),
            raw_os_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Copy)]
    enum WriteStep {
        Count(usize),
        Error(ErrorKind),
        RawError(i32),
        Zero,
    }

    #[derive(Clone, Copy)]
    enum FlushStep {
        Okay,
        Error(ErrorKind),
        RawError(i32),
    }

    #[derive(Default)]
    struct ScriptedWriter {
        writes: VecDeque<WriteStep>,
        flushes: VecDeque<FlushStep>,
        accepted: Vec<u8>,
        flush_calls: usize,
    }

    impl Write for ScriptedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self
                .writes
                .pop_front()
                .unwrap_or(WriteStep::Count(bytes.len()))
            {
                WriteStep::Count(count) => {
                    let accepted = count.min(bytes.len());
                    self.accepted.extend_from_slice(&bytes[..accepted]);
                    Ok(count)
                }
                WriteStep::Error(kind) => Err(io::Error::from(kind)),
                WriteStep::RawError(errno) => Err(io::Error::from_raw_os_error(errno)),
                WriteStep::Zero => Ok(0),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flush_calls += 1;
            match self.flushes.pop_front().unwrap_or(FlushStep::Okay) {
                FlushStep::Okay => Ok(()),
                FlushStep::Error(kind) => Err(io::Error::from(kind)),
                FlushStep::RawError(errno) => Err(io::Error::from_raw_os_error(errno)),
            }
        }
    }

    #[derive(Debug)]
    struct DirtyOwner {
        dirty: bool,
    }

    #[derive(Debug)]
    struct DropTrackedOwner {
        dirty: bool,
        drops: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Drop for DropTrackedOwner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn dirty_notice() -> PendingStdioNotification<DirtyOwner> {
        PendingStdioNotification::new(DirtyOwner { dirty: true }, b"unsaved draft\n".to_vec())
    }

    #[test]
    fn short_write_then_completion_keeps_owner_until_separate_resolution() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Count(4)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Retry {
                written: 4,
                reason: RetryReason::ShortWrite,
            }
        );
        assert!(notice.owner().dirty);
        assert_eq!(notice.remaining(), b"ved draft\n");

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Delivered {
                written: b"unsaved draft\n".len(),
            }
        );
        assert!(notice.is_delivered());
        assert!(notice.owner().dirty);
        assert_eq!(writer.accepted, b"unsaved draft\n");
    }

    #[test]
    fn would_block_after_partial_write_retains_cursor_payload_and_owner() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Count(3), WriteStep::Error(ErrorKind::WouldBlock)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert!(matches!(
            notice.try_write(&mut writer),
            WriteProgress::Retry {
                reason: RetryReason::ShortWrite,
                ..
            }
        ));
        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Retry {
                written: 3,
                reason: RetryReason::WouldBlock,
            }
        );
        assert_eq!(
            notice.last_progress(),
            Some(WriteProgress::Retry {
                written: 3,
                reason: RetryReason::WouldBlock,
            })
        );
        assert_eq!(notice.remaining(), b"aved draft\n");
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn epipe_after_partial_write_is_failure_not_delivery() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Count(2), WriteStep::RawError(32)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();
        let _ = notice.try_write(&mut writer);

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 2,
                reason: FailureReason::BrokenPipe,
            }
        );
        assert_eq!(notice.written(), 2);
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn closed_stderr_descriptor_is_failure_and_keeps_owner() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::RawError(9)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 0,
                reason: FailureReason::ClosedDescriptor,
            }
        );
        assert_eq!(
            notice.last_progress(),
            Some(WriteProgress::Failed {
                written: 0,
                reason: FailureReason::ClosedDescriptor,
            })
        );
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn failed_notice_keeps_dirty_owner_and_payload_transferable() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let drops = Arc::new(AtomicUsize::new(0));
        let owner = DropTrackedOwner {
            dirty: true,
            drops: Arc::clone(&drops),
        };
        let mut notice = PendingStdioNotification::new(owner, b"unsaved\n".to_vec());
        let mut writer = ScriptedWriter {
            writes: [WriteStep::RawError(32)].into(),
            ..ScriptedWriter::default()
        };

        assert!(matches!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                reason: FailureReason::BrokenPipe,
                ..
            }
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        let (owner, bytes, written, delivered, last_progress) = notice.into_parts();
        assert!(owner.dirty);
        assert_eq!(bytes, b"unsaved\n");
        assert_eq!(written, 0);
        assert!(!delivered);
        assert_eq!(
            last_progress,
            Some(WriteProgress::Failed {
                written: 0,
                reason: FailureReason::BrokenPipe,
            })
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn write_zero_is_not_delivery() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Zero].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 0,
                reason: FailureReason::WriteZero,
            }
        );
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn interrupted_and_flush_would_block_are_retryable() {
        let mut writer = ScriptedWriter {
            writes: [
                WriteStep::Error(ErrorKind::Interrupted),
                WriteStep::Count(14),
            ]
            .into(),
            flushes: [FlushStep::Error(ErrorKind::WouldBlock), FlushStep::Okay].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Retry {
                written: 0,
                reason: RetryReason::Interrupted,
            }
        );
        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Retry {
                written: 14,
                reason: RetryReason::WouldBlock,
            }
        );
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Delivered { written: 14 }
        );
        assert!(notice.owner().dirty);
        assert_eq!(writer.flush_calls, 2);
    }

    #[test]
    fn flush_epipe_after_full_write_is_not_delivery() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Count(14)].into(),
            flushes: [FlushStep::RawError(32)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 14,
                reason: FailureReason::BrokenPipe,
            }
        );
        assert_eq!(writer.accepted, b"unsaved draft\n");
        assert!(notice.remaining().is_empty());
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn non_epipe_write_errors_remain_failures() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Error(ErrorKind::PermissionDenied)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 0,
                reason: FailureReason::Other {
                    kind: ErrorKind::PermissionDenied,
                    raw_os_error: None,
                },
            }
        );
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }

    #[test]
    fn overreported_write_count_is_rejected() {
        let mut writer = ScriptedWriter {
            writes: [WriteStep::Count(100)].into(),
            ..ScriptedWriter::default()
        };
        let mut notice = dirty_notice();

        assert_eq!(
            notice.try_write(&mut writer),
            WriteProgress::Failed {
                written: 0,
                reason: FailureReason::InvalidWriteCount,
            }
        );
        assert!(notice.owner().dirty);
        assert!(!notice.is_delivered());
    }
}
