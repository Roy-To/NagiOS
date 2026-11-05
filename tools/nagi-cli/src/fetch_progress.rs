//! Immediate, credential-free progress for potentially blocking source fetches.

use std::io::Write;
use std::time::Instant;

pub(crate) fn phase<T, E>(
    name: &'static str,
    action: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    phase_with_writer(name, &mut std::io::stderr(), action)
}

fn phase_with_writer<T, E>(
    name: &'static str,
    progress: &mut impl Write,
    action: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let started = Instant::now();
    // Flush before starting the operation: Command::output buffers its child
    // output, and a CI cancellation otherwise loses all evidence of the phase.
    // Labels are static; do not log command arguments, URLs, env or error text.
    let _ = writeln!(progress, "INFO fetch phase={name} status=start");
    let _ = progress.flush();
    let result = action();
    let status = if result.is_ok() { "done" } else { "failed" };
    let _ = writeln!(
        progress,
        "INFO fetch phase={name} status={status} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let _ = progress.flush();
    // Observability must neither weaken validation nor change its result when
    // stderr is unavailable. The caller retains the original error handling.
    result
}

#[cfg(test)]
mod tests {
    use super::phase_with_writer;
    use std::io::{self, Write};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Duration;

    struct FlushedLog {
        bytes: Arc<Mutex<Vec<u8>>>,
        flushed: mpsc::Sender<Vec<u8>>,
    }
    impl Write for FlushedLog {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed
                .send(self.bytes.lock().unwrap().clone())
                .unwrap();
            Ok(())
        }
    }

    #[test]
    fn progress_is_flushed_before_blocking_and_does_not_disclose_the_error() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (flushed_tx, flushed_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::sync_channel(0);
        let mut output = FlushedLog {
            bytes: Arc::clone(&bytes),
            flushed: flushed_tx,
        };
        let worker = std::thread::spawn(move || {
            phase_with_writer("fixture.fetch", &mut output, || -> Result<(), String> {
                resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Err("opaque caller error must not be copied into progress".to_owned())
            })
        });
        let before_completion = flushed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            before_completion,
            b"INFO fetch phase=fixture.fetch status=start\n"
        );
        assert!(
            !worker.is_finished(),
            "phase evidence must be visible while the operation is blocked"
        );
        resume_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            Err("opaque caller error must not be copied into progress".to_owned())
        );
        let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(log.contains("phase=fixture.fetch status=failed elapsed_ms="));
        assert!(!log.contains("opaque caller error"));
        assert!(!log.contains("status=done"));
    }

    #[test]
    fn successful_phase_returns_the_original_value() {
        let mut log = Vec::new();
        assert_eq!(
            phase_with_writer("fixture", &mut log, || Ok::<_, String>(42)),
            Ok(42)
        );
        assert!(String::from_utf8(log)
            .unwrap()
            .contains("phase=fixture status=done elapsed_ms="));
    }

    #[test]
    fn broken_progress_sink_does_not_skip_or_retry_the_operation() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let mut calls = 0;
        let result = phase_with_writer("fixture", &mut Broken, || {
            calls += 1;
            Err::<(), _>("validation failed".to_owned())
        });
        assert_eq!(calls, 1);
        assert_eq!(result, Err("validation failed".to_owned()));
    }
}
