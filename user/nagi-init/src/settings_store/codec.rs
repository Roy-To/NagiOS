//! Encoding for the Linux Settings preferences object.

use nagi_localization::Locale;
use nagi_ui::ThemeMode;
use serde_json::{Map, Value};
use std::num::NonZeroU64;

// 13-byte prefix, NUL terminator, then the two-byte container version.
const MAGIC: &[u8; 16] = b"NAGI-SETTINGS\0\0\x01";
const HEADER_BYTES: usize = 32;
const MAX_PAYLOAD_BYTES: usize = 4096;
const SCHEMA: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsPreferences {
    pub(crate) locale: Locale,
    pub(crate) theme: ThemeMode,
    unknown: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecodeError {
    Corrupt,
    UnsupportedSchema(u64),
    PayloadTooLarge,
    RevisionExhausted,
}

#[derive(Debug)]
pub(crate) struct DecodedSettings<'a> {
    pub(crate) revision: NonZeroU64,
    pub(crate) preferences: SettingsPreferences,
    pub(crate) invalid_locale: bool,
    pub(crate) invalid_theme: bool,
    pub(crate) payload: &'a [u8],
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedSettings<'_>, DecodeError> {
    if bytes.len() < HEADER_BYTES || &bytes[..16] != MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let revision = NonZeroU64::new(u64::from_le_bytes(bytes[16..24].try_into().unwrap()))
        .ok_or(DecodeError::Corrupt)?;
    let length = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if length > MAX_PAYLOAD_BYTES {
        return Err(DecodeError::PayloadTooLarge);
    }
    let payload = &bytes[HEADER_BYTES..];
    let checksum = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
    if payload.len() != length || crc32c(payload) != checksum {
        return Err(DecodeError::Corrupt);
    }

    let mut unknown: Map<String, Value> =
        serde_json::from_slice(payload).map_err(|_| DecodeError::Corrupt)?;
    let schema = unknown
        .remove("schema")
        .and_then(|value| value.as_u64())
        .ok_or(DecodeError::Corrupt)?;
    if schema > SCHEMA {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    if schema != SCHEMA {
        return Err(DecodeError::Corrupt);
    }
    let locale = unknown
        .remove("locale")
        .and_then(|value| value.as_str().and_then(Locale::parse));
    let theme = unknown
        .remove("theme")
        .and_then(|value| match value.as_str() {
            Some("light") => Some(ThemeMode::Light),
            Some("dark") => Some(ThemeMode::Dark),
            _ => None,
        });
    Ok(DecodedSettings {
        revision,
        preferences: SettingsPreferences {
            locale: locale.unwrap_or(Locale::EnUs),
            theme: theme.unwrap_or(ThemeMode::Light),
            unknown,
        },
        invalid_locale: locale.is_none(),
        invalid_theme: theme.is_none(),
        payload,
    })
}

/// Prepare a successor from the actual stored bytes. The caller owns the
/// compare-and-replace check and must keep it in the same storage operation.
/// Only an absent object starts at revision one. Corrupt and newer objects
/// are returned as errors before any replacement bytes are produced.
pub(crate) fn encode_replacement(
    current: Option<&[u8]>,
    locale: Locale,
    theme: ThemeMode,
) -> Result<(NonZeroU64, Vec<u8>), DecodeError> {
    let (revision, mut fields) = match current {
        Some(bytes) => {
            let snapshot = decode(bytes)?;
            let revision = snapshot
                .revision
                .checked_add(1)
                .ok_or(DecodeError::RevisionExhausted)?;
            (revision, snapshot.preferences.unknown)
        }
        None => (NonZeroU64::MIN, Map::new()),
    };
    fields.insert("schema".into(), Value::from(SCHEMA));
    fields.insert("locale".into(), Value::from(locale.code()));
    let theme = match theme {
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    };
    fields.insert("theme".into(), Value::from(theme));
    let payload = serde_json::to_vec(&fields).map_err(|_| DecodeError::Corrupt)?;
    Ok((revision, encode_container(revision, &payload)?))
}

fn encode_container(revision: NonZeroU64, payload: &[u8]) -> Result<Vec<u8>, DecodeError> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(DecodeError::PayloadTooLarge);
    }
    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&revision.get().to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&crc32c(payload).to_le_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn crc32c(payload: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in payload {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
