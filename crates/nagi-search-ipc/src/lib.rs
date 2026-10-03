//! `search@1`: bounded Channel wire format for the SearchService (ADR 0044).
//!
//! Requests carry only the query. The caller's identity is never in the
//! payload: the service derives it from the kernel-stamped sender Process ID
//! and the Supervisor launch record (ADR 0043). Both directions fit one
//! inline Channel message and need no allocation, so isolated `no_std`
//! clients can use this crate directly.
#![no_std]

use nagi_abi::MAX_CHANNEL_INLINE_PAYLOAD;

pub const PROTOCOL_ID: u16 = 0x5352;
pub const PROTOCOL_VERSION: u16 = 1;
pub const OPCODE_SEARCH: u16 = 1;
pub const OPCODE_RESULTS: u16 = 2;

/// Request layout: `[kind u8][text_len u8][text UTF-8 ...]`.
pub const MAX_QUERY_TEXT: usize = MAX_CHANNEL_INLINE_PAYLOAD - 2;
/// Response layout: `[status u8][count u8][visible_total u16 LE][ids u64 LE ...]`.
pub const MAX_RESULT_IDS: usize = (MAX_CHANNEL_INLINE_PAYLOAD - 4) / 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KindFilter {
    Any = 0,
    File = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchRequest<'a> {
    pub kind: KindFilter,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultStatus {
    Ok = 0,
    /// The caller has no Supervisor launch record.
    UnknownCaller = 1,
    /// The request could not be decoded or validated.
    InvalidRequest = 2,
    /// The service failed to evaluate a valid request.
    Unavailable = 3,
}

impl ResultStatus {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Ok),
            1 => Some(Self::UnknownCaller),
            2 => Some(Self::InvalidRequest),
            3 => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchResults {
    pub status: ResultStatus,
    /// Number of visible matches; may exceed the `ids` carried.
    pub visible_total: u16,
    pub count: usize,
    pub ids: [u64; MAX_RESULT_IDS],
}

impl SearchResults {
    pub const fn status_only(status: ResultStatus) -> Self {
        Self {
            status,
            visible_total: 0,
            count: 0,
            ids: [0; MAX_RESULT_IDS],
        }
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids[..self.count]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    TooLong,
    Truncated,
    InvalidKind,
    InvalidStatus,
    InvalidUtf8,
    EmptyQuery,
    TrailingBytes,
}

pub fn encode_request(
    request: SearchRequest<'_>,
    output: &mut [u8; MAX_CHANNEL_INLINE_PAYLOAD],
) -> Result<usize, WireError> {
    let text = request.text.as_bytes();
    if text.is_empty() {
        return Err(WireError::EmptyQuery);
    }
    if text.len() > MAX_QUERY_TEXT {
        return Err(WireError::TooLong);
    }
    output[0] = request.kind as u8;
    output[1] = text.len() as u8;
    output[2..2 + text.len()].copy_from_slice(text);
    Ok(2 + text.len())
}

pub fn decode_request(payload: &[u8]) -> Result<SearchRequest<'_>, WireError> {
    if payload.len() < 2 {
        return Err(WireError::Truncated);
    }
    let kind = match payload[0] {
        0 => KindFilter::Any,
        1 => KindFilter::File,
        _ => return Err(WireError::InvalidKind),
    };
    let length = usize::from(payload[1]);
    if length == 0 {
        return Err(WireError::EmptyQuery);
    }
    if length > MAX_QUERY_TEXT {
        return Err(WireError::TooLong);
    }
    let end = 2 + length;
    if payload.len() < end {
        return Err(WireError::Truncated);
    }
    if payload.len() > end {
        return Err(WireError::TrailingBytes);
    }
    let text = core::str::from_utf8(&payload[2..end]).map_err(|_| WireError::InvalidUtf8)?;
    Ok(SearchRequest { kind, text })
}

pub fn encode_results(
    results: &SearchResults,
    output: &mut [u8; MAX_CHANNEL_INLINE_PAYLOAD],
) -> Result<usize, WireError> {
    if results.count > MAX_RESULT_IDS || usize::from(results.visible_total) < results.count {
        return Err(WireError::TooLong);
    }
    output[0] = results.status as u8;
    output[1] = results.count as u8;
    output[2..4].copy_from_slice(&results.visible_total.to_le_bytes());
    for (index, id) in results.ids().iter().enumerate() {
        let start = 4 + index * 8;
        output[start..start + 8].copy_from_slice(&id.to_le_bytes());
    }
    Ok(4 + results.count * 8)
}

pub fn decode_results(payload: &[u8]) -> Result<SearchResults, WireError> {
    if payload.len() < 4 {
        return Err(WireError::Truncated);
    }
    let status = ResultStatus::from_byte(payload[0]).ok_or(WireError::InvalidStatus)?;
    let count = usize::from(payload[1]);
    if count > MAX_RESULT_IDS {
        return Err(WireError::TooLong);
    }
    let visible_total = u16::from_le_bytes([payload[2], payload[3]]);
    if usize::from(visible_total) < count || (status != ResultStatus::Ok && count != 0) {
        return Err(WireError::InvalidStatus);
    }
    let end = 4 + count * 8;
    if payload.len() < end {
        return Err(WireError::Truncated);
    }
    if payload.len() > end {
        return Err(WireError::TrailingBytes);
    }
    let mut ids = [0_u64; MAX_RESULT_IDS];
    for (index, id) in ids.iter_mut().take(count).enumerate() {
        let start = 4 + index * 8;
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&payload[start..start + 8]);
        *id = u64::from_le_bytes(bytes);
    }
    Ok(SearchResults {
        status,
        visible_total,
        count,
        ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_and_rejects_malformed_payloads() {
        let mut buffer = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
        let request = SearchRequest {
            kind: KindFilter::File,
            text: "nagi-m19-live-file.txt",
        };
        let length = encode_request(request, &mut buffer).expect("encode");
        assert_eq!(decode_request(&buffer[..length]), Ok(request));
        assert_eq!(
            decode_request(&buffer[..length - 1]),
            Err(WireError::Truncated)
        );
        assert_eq!(
            decode_request(&buffer[..length + 1]),
            Err(WireError::TrailingBytes)
        );
        let mut bad_kind = buffer;
        bad_kind[0] = 9;
        assert_eq!(
            decode_request(&bad_kind[..length]),
            Err(WireError::InvalidKind)
        );
        let mut bad_utf8 = buffer;
        bad_utf8[2] = 0xff;
        assert_eq!(
            decode_request(&bad_utf8[..length]),
            Err(WireError::InvalidUtf8)
        );
        assert_eq!(decode_request(&[1, 0]), Err(WireError::EmptyQuery));
        let long = core::str::from_utf8(&[b'a'; MAX_QUERY_TEXT + 1]).unwrap();
        assert_eq!(
            encode_request(
                SearchRequest {
                    kind: KindFilter::Any,
                    text: long
                },
                &mut buffer
            ),
            Err(WireError::TooLong)
        );
    }

    #[test]
    fn results_round_trip_and_status_invariants_hold() {
        let mut buffer = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
        let mut results = SearchResults::status_only(ResultStatus::Ok);
        results.count = MAX_RESULT_IDS;
        results.visible_total = 40;
        for (index, id) in results.ids.iter_mut().enumerate() {
            *id = 0x4e41_0000 + index as u64;
        }
        let length = encode_results(&results, &mut buffer).expect("encode");
        assert_eq!(length, 4 + MAX_RESULT_IDS * 8);
        assert!(length <= MAX_CHANNEL_INLINE_PAYLOAD);
        assert_eq!(decode_results(&buffer[..length]), Ok(results));

        let denied = SearchResults::status_only(ResultStatus::UnknownCaller);
        let length = encode_results(&denied, &mut buffer).expect("encode denied");
        assert_eq!(decode_results(&buffer[..length]), Ok(denied));
        // A non-OK status must not carry identifiers.
        assert_eq!(
            decode_results(&[1, 1, 1, 0, 1, 2, 3, 4, 5, 6, 7, 8]),
            Err(WireError::InvalidStatus)
        );
        assert_eq!(decode_results(&[0, 2, 1, 0]), Err(WireError::InvalidStatus));
        assert_eq!(decode_results(&[7, 0, 0, 0]), Err(WireError::InvalidStatus));
        assert_eq!(decode_results(&[0, 1, 1, 0]), Err(WireError::Truncated));
    }
}
