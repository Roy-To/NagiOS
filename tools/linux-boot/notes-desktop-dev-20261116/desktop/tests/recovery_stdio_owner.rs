#[path = "../src/input_failure_guard.rs"]
mod input_failure_guard;
#[path = "../src/notification_stdio.rs"]
mod notification_stdio;
#[path = "../src/recovery_stdio.rs"]
mod recovery_stdio;

use input_failure_guard::{guard_input_failure, InputFailureAction};
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

struct DirtyOwner {
    drops: Arc<AtomicUsize>,
    notice_attempts: usize,
}

impl Drop for DirtyOwner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn stderr_epipe_and_notice_failure_keep_the_dirty_owner() {
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = DirtyOwner {
        drops: Arc::clone(&drops),
        notice_attempts: 0,
    };

    let action = guard_input_failure(
        owner,
        "evdev disconnected",
        |_owner| Err("final Notes flush failed".to_owned()),
        |owner, notice| {
            assert_eq!(
                recovery_stdio::try_diagnostic(&mut BrokenPipeWriter, notice),
                notification_stdio::WriteProgress::Failed {
                    written: 0,
                    reason: notification_stdio::FailureReason::BrokenPipe,
                }
            );
            owner.notice_attempts += 1;
            Err("framebuffer notification failed".to_owned())
        },
    );

    match action {
        InputFailureAction::HoldUnsavedDraft {
            owner,
            input_error,
            save_error,
            notification_error,
        } => {
            assert_eq!(input_error, "evdev disconnected");
            assert_eq!(save_error, "final Notes flush failed");
            assert_eq!(
                notification_error.as_deref(),
                Some("framebuffer notification failed")
            );
            assert_eq!(owner.notice_attempts, 1);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(owner);
        }
        InputFailureAction::ExitWithInputErrorAfterSave { owner, input_error } => {
            assert_eq!(input_error, "evdev disconnected");
            drop(owner);
            panic!("a failed final flush must retain the dirty owner")
        }
    }

    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
