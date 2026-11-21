#![no_std]

use nagi_abi::InputEvent;

/// Linux evdev wire layouts supported by this decoder.
///
/// Timestamp values are framing only and are discarded because `nagi-abi::InputEvent`
/// has no timestamp fields. The target must be supplied by the caller; host `usize`
/// and host endianness are never used to infer the record format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxEvdevTarget {
    /// `x86_64-unknown-linux-gnu`: two little-endian 64-bit timeval fields.
    X86_64,
    /// `i686-unknown-linux-gnu`: two little-endian 32-bit timeval fields.
    I686,
}

impl LinuxEvdevTarget {
    /// Exact `struct input_event` byte size for the selected target ABI.
    pub const fn record_len(self) -> usize {
        match self {
            Self::X86_64 => 24,
            Self::I686 => 16,
        }
    }

    const fn timestamp_width(self) -> usize {
        match self {
            Self::X86_64 => 8,
            Self::I686 => 4,
        }
    }
}

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
/// Linux input metadata emitted before key events on common keyboards.
pub const EV_MSC: u16 = 0x04;

pub const SYN_REPORT: u16 = 0x00;
pub const SYN_CONFIG: u16 = 0x01;
pub const SYN_MT_REPORT: u16 = 0x02;
pub const SYN_DROPPED: u16 = 0x03;
pub const MSC_SCAN: u16 = 0x04;
pub const KEY_MAX: u16 = 0x02ff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidRecordLength { expected: usize, actual: usize },
    UnsupportedEventType(u16),
    UnsupportedCode { event_type: u16, code: u16 },
    InvalidKeyValue(i32),
}

/// Decode exactly one Linux x86 evdev `input_event` record into the existing Nagi ABI.
///
/// `EV_KEY` values 0, 1, and 2 are preserved as release, press, and repeat. The raw
/// physical keycode is preserved; this function does not map it to text. The four
/// defined `EV_SYN` markers are also preserved, including `SYN_DROPPED` so the caller
/// can perform its own state resynchronization. `EV_MSC/MSC_SCAN` is preserved as
/// keyboard metadata for caller-side filtering. Other event types and out-of-range
/// codes are rejected. Timestamp bits are intentionally not representable in the
/// current ABI and are ignored after selecting the caller-supplied target layout.
pub fn decode_evdev_record(
    target: LinuxEvdevTarget,
    record: &[u8],
) -> Result<InputEvent, DecodeError> {
    let expected = target.record_len();
    if record.len() != expected {
        return Err(DecodeError::InvalidRecordLength {
            expected,
            actual: record.len(),
        });
    }

    let offset = target.timestamp_width() * 2;
    let event_type = u16::from_le_bytes([record[offset], record[offset + 1]]);
    let code = u16::from_le_bytes([record[offset + 2], record[offset + 3]]);
    let value = i32::from_le_bytes([
        record[offset + 4],
        record[offset + 5],
        record[offset + 6],
        record[offset + 7],
    ]);

    match event_type {
        EV_KEY => {
            if code > KEY_MAX {
                return Err(DecodeError::UnsupportedCode { event_type, code });
            }
            if !(0..=2).contains(&value) {
                return Err(DecodeError::InvalidKeyValue(value));
            }
        }
        EV_SYN => match code {
            SYN_REPORT | SYN_CONFIG | SYN_MT_REPORT | SYN_DROPPED => {}
            _ => return Err(DecodeError::UnsupportedCode { event_type, code }),
        },
        EV_MSC if code == MSC_SCAN => {}
        EV_MSC => return Err(DecodeError::UnsupportedCode { event_type, code }),
        other => return Err(DecodeError::UnsupportedEventType(other)),
    }

    Ok(InputEvent {
        event_type,
        code,
        value,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec;
    use std::vec::Vec;

    fn record(target: LinuxEvdevTarget, event_type: u16, code: u16, value: i32) -> Vec<u8> {
        let mut bytes = vec![0; target.record_len()];
        let offset = target.timestamp_width() * 2;
        bytes[offset..offset + 2].copy_from_slice(&event_type.to_le_bytes());
        bytes[offset + 2..offset + 4].copy_from_slice(&code.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&value.to_le_bytes());
        bytes
    }

    #[test]
    fn x86_64_msc_scan_metadata_is_preserved() {
        let bytes = record(LinuxEvdevTarget::X86_64, EV_MSC, MSC_SCAN, 0x700e1);
        assert_eq!(
            decode_evdev_record(LinuxEvdevTarget::X86_64, &bytes),
            Ok(InputEvent {
                event_type: EV_MSC,
                code: MSC_SCAN,
                value: 0x700e1,
            })
        );
    }

    #[test]
    fn unsupported_msc_codes_and_other_event_types_remain_rejected() {
        let unsupported_msc = record(LinuxEvdevTarget::X86_64, EV_MSC, MSC_SCAN + 1, 1);
        assert_eq!(
            decode_evdev_record(LinuxEvdevTarget::X86_64, &unsupported_msc),
            Err(DecodeError::UnsupportedCode {
                event_type: EV_MSC,
                code: MSC_SCAN + 1,
            })
        );

        let relative = record(LinuxEvdevTarget::X86_64, 0x02, 0, 1);
        assert_eq!(
            decode_evdev_record(LinuxEvdevTarget::X86_64, &relative),
            Err(DecodeError::UnsupportedEventType(0x02))
        );
    }
}
