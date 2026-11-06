//! Allocation-free preparation for the Windows host-reference read adapter.
//!
//! Keep the pinned std short-absolute/verbatim fast paths and normalize other
//! paths once before adding a long-path prefix. Buffers include the final NUL.
use std::io;

pub(super) const MAX_PATH_UNITS: usize = 32_768;
const SEP: u16 = b'\\' as u16;
const ALT: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const VERBATIM: &[u16] = &[SEP, SEP, b'?' as u16, SEP];
const NT: &[u16] = &[SEP, b'?' as u16, b'?' as u16, SEP];
const DEVICE: &[u16] = &[SEP, SEP, b'.' as u16, SEP];
const UNC: &[u16] = &[
    SEP,
    SEP,
    b'?' as u16,
    SEP,
    b'U' as u16,
    b'N' as u16,
    b'C' as u16,
    SEP,
];

fn too_long() -> io::Error {
    io::Error::from_raw_os_error(206) // ERROR_FILENAME_EXCED_RANGE
}

/// Returns the initialized, NUL-terminated prefix of `path`.
/// `full_path` models GetFullPathNameW: its successful length excludes NUL.
pub(super) fn prepare_path(
    units: impl Iterator<Item = u16>,
    path: &mut [u16],
    absolute: &mut [u16],
    full_path: impl FnOnce(&[u16], &mut [u16]) -> io::Result<usize>,
) -> io::Result<usize> {
    let mut length = 0;
    for unit in units {
        if unit == 0 {
            return Err(io::Error::from_raw_os_error(123)); // ERROR_INVALID_NAME
        }
        if length + 1 >= path.len() {
            return Err(too_long());
        }
        path[length] = unit;
        length += 1;
    }
    if length >= path.len() {
        return Err(too_long());
    }
    path[length] = 0;
    let terminated = &path[..=length];
    let separator = |u| u == SEP || u == ALT;
    let short_absolute = length + 1 < 248
        && ((length >= 2
            && !separator(path[0])
            && path[1] == COLON
            && (length == 2 || separator(path[2])))
            || (length >= 2 && separator(path[0]) && separator(path[1])));
    if length == 0
        || terminated.starts_with(VERBATIM)
        || terminated.starts_with(NT)
        || short_absolute
    {
        return Ok(length + 1);
    }

    let length = full_path(terminated, absolute)?;
    if length >= absolute.len() {
        return Err(too_long());
    }
    let mut normalized = &absolute[..length];
    let prefix = if normalized.len() >= 3 && normalized[1] == COLON && normalized[2] == SEP {
        VERBATIM
    } else if normalized.starts_with(DEVICE) {
        normalized = &normalized[4..];
        VERBATIM
    } else if normalized.starts_with(VERBATIM) || normalized.starts_with(NT) {
        &[]
    } else if normalized.starts_with(&[SEP, SEP]) {
        normalized = &normalized[2..];
        UNC
    } else {
        &[]
    };
    let needed = prefix
        .len()
        .checked_add(normalized.len())
        .and_then(|n| n.checked_add(1))
        .ok_or_else(too_long)?;
    if needed > path.len() {
        return Err(too_long());
    }
    path[..prefix.len()].copy_from_slice(prefix);
    path[prefix.len()..needed - 1].copy_from_slice(normalized);
    path[needed - 1] = 0;
    Ok(needed)
}
