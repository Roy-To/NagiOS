use crate::notification_stdio::{PendingStdioNotification, WriteProgress};
use std::io::{self, Write};

/// Best-effort diagnostic output. A closed or broken stderr must not unwind a
/// caller that is responsible for retaining an unsaved Notes owner.
pub(crate) fn try_diagnostic<W: Write>(writer: &mut W, message: &str) -> WriteProgress {
    let mut pending = PendingStdioNotification::new((), format!("{message}\n").into_bytes());
    pending.try_write(writer)
}

#[allow(dead_code)] // used by the application binary; standalone test includes the module
pub(crate) fn log_stderr(message: &str) {
    let stderr = io::stderr();
    let _ = try_diagnostic(&mut stderr.lock(), message);
}
