//! `Read`-based Linux evdev record framing for an already-open caller-owned source.
//!
//! This crate does not open device paths, acquire file descriptors, or perform
//! authorization. It frames exactly one selected Linux ABI record at a time and
//! delegates field validation to `linux-evdev-decoder`.

use std::io::{self, Read};

use linux_evdev_decoder::{decode_evdev_record, DecodeError, EV_SYN, SYN_DROPPED};
use nagi_abi::InputEvent;

pub use linux_evdev_decoder::LinuxEvdevTarget;

/// One outcome from polling the byte stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamItem {
    /// A decoded event, except `SYN_DROPPED`, which is surfaced separately.
    Event(InputEvent),
    /// The source reported `SYN_DROPPED`; caller-owned key state needs recovery.
    /// The stream remains latched until it is dropped and recreated after recovery.
    NeedsResync,
    /// The source would block. Any reported partial record remains buffered.
    WouldBlock { buffered_bytes: usize },
    /// End of stream with no partial record pending.
    Eof,
}

/// Framing, decoder, and non-retryable I/O failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamError {
    /// EOF arrived after only part of the selected record.
    TruncatedRecord { expected: usize, received: usize },
    /// The complete record was framed but rejected by the decoder.
    Decode(DecodeError),
    /// An I/O error other than `Interrupted` or `WouldBlock`.
    Io(io::ErrorKind),
}

/// Incrementally frames records from a caller-supplied `Read` source.
pub struct EvdevStream<R> {
    reader: R,
    target: LinuxEvdevTarget,
    record: [u8; 24],
    buffered: usize,
    eof: bool,
    needs_resync: bool,
}

impl<R: Read> EvdevStream<R> {
    /// Create a stream over an existing reader and an explicit Linux evdev layout.
    pub fn new(reader: R, target: LinuxEvdevTarget) -> Self {
        Self {
            reader,
            target,
            record: [0; 24],
            buffered: 0,
            eof: false,
            needs_resync: false,
        }
    }

    /// Return the number of bytes held for the current, incomplete record.
    pub const fn buffered_bytes(&self) -> usize {
        self.buffered
    }

    /// Borrow the caller-owned reader without disturbing the parser state.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Recover the reader so the caller can create a fresh parser after
    /// handling a latched `SYN_DROPPED` state.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Resume framing after the caller has successfully recovered device key
    /// state for a latched `SYN_DROPPED`. Returns `false` when no resync is
    /// pending, leaving the stream unchanged.
    pub fn reset_after_resync(&mut self) -> bool {
        if !self.needs_resync {
            return false;
        }
        self.record.fill(0);
        self.buffered = 0;
        self.eof = false;
        self.needs_resync = false;
        true
    }

    /// Read and decode one record, retrying `Interrupted` and reporting `WouldBlock`.
    ///
    /// `SYN_DROPPED` is never returned as `Event`; it latches this stream at
    /// `NeedsResync`. The caller must perform actual key-state recovery outside this
    /// crate, then drop and recreate the stream over the same reader before resuming.
    pub fn next_item(&mut self) -> Result<StreamItem, StreamError> {
        if self.needs_resync {
            return Ok(StreamItem::NeedsResync);
        }
        if self.eof {
            return Ok(StreamItem::Eof);
        }

        let expected = self.target.record_len();
        loop {
            if self.buffered == expected {
                let decoded = decode_evdev_record(self.target, &self.record[..expected]);
                self.buffered = 0;
                return match decoded {
                    Ok(event) if event.event_type == EV_SYN && event.code == SYN_DROPPED => {
                        self.needs_resync = true;
                        Ok(StreamItem::NeedsResync)
                    }
                    Ok(event) => Ok(StreamItem::Event(event)),
                    Err(error) => Err(StreamError::Decode(error)),
                };
            }

            match self.reader.read(&mut self.record[self.buffered..expected]) {
                Ok(0) => {
                    self.eof = true;
                    if self.buffered == 0 {
                        return Ok(StreamItem::Eof);
                    }
                    let received = self.buffered;
                    self.buffered = 0;
                    return Err(StreamError::TruncatedRecord { expected, received });
                }
                Ok(count) => self.buffered += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(StreamItem::WouldBlock {
                        buffered_bytes: self.buffered,
                    });
                }
                Err(error) => return Err(StreamError::Io(error.kind())),
            }
        }
    }

    /// Whether `SYN_DROPPED` has latched this stream pending caller-owned recovery.
    pub const fn needs_resync(&self) -> bool {
        self.needs_resync
    }
}

#[cfg(test)]
mod tests {
    use super::{EvdevStream, LinuxEvdevTarget, StreamItem};
    use std::io::{self, Read};

    struct PartialThenWouldBlock(bool);

    impl Read for PartialThenWouldBlock {
        fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
            if !self.0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.0 = false;
            destination[..7].fill(0x5a);
            Ok(7)
        }
    }

    #[test]
    fn stream_exposes_partial_record_and_preserves_reader_for_reset() {
        let mut stream = EvdevStream::new(PartialThenWouldBlock(true), LinuxEvdevTarget::X86_64);
        assert_eq!(
            stream.next_item(),
            Ok(StreamItem::WouldBlock { buffered_bytes: 7 })
        );
        assert_eq!(stream.buffered_bytes(), 7);

        let reader = stream.into_inner();
        let restarted = EvdevStream::new(reader, LinuxEvdevTarget::X86_64);
        assert_eq!(restarted.buffered_bytes(), 0);
    }
}
