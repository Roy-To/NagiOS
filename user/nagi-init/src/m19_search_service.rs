//! Capability-checked Files Search requests over `search@1` Channels.
//!
//! This module is shared by the ordinary signed-in Files runtime and the M19
//! isolated-client acceptance. Caller identity is resolved only from the
//! kernel-stamped sender PID; the request payload contains only a query.

use alloc::vec::Vec;

use libnagi::launch::LaunchRecord;
use libnagi::{
    channel_send, channel_try_receive, handle_close, ChannelReceiveResult, ChannelSendRequest,
};
use nagi_model::ObjectId;
use nagi_search::AccessContext;
use nagi_search_ipc::{
    decode_request, encode_results, KindFilter, ResultStatus, SearchResults, MAX_RESULT_IDS,
    OPCODE_RESULTS, OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
};

use crate::supervisor;

const SEARCH_QUERY_GRANT: &[u8] = b"search.query";
const FILES_SEARCH_GRANT: &[u8] = b"files.search";

/// Evaluate a Files request only after the caller's live launch holds both
/// required grants. The closure runs only after authorization and should
/// apply the producer's normal Search visibility policy.
pub(super) fn evaluate_files(
    launch: Option<LaunchRecord>,
    payload: &[u8],
    search: impl FnOnce(AccessContext, &str) -> Option<Vec<ObjectId>>,
) -> SearchResults {
    let Some(launch) = launch else {
        return SearchResults::status_only(ResultStatus::UnknownCaller);
    };
    if !supervisor::has_grant(launch.app_id, launch.app_session_id, SEARCH_QUERY_GRANT) {
        return SearchResults::status_only(ResultStatus::Denied);
    }
    let Ok(request) = decode_request(payload) else {
        return SearchResults::status_only(ResultStatus::InvalidRequest);
    };
    if request.kind != KindFilter::File {
        return SearchResults::status_only(ResultStatus::InvalidRequest);
    }
    if !supervisor::has_grant(launch.app_id, launch.app_session_id, FILES_SEARCH_GRANT) {
        return SearchResults::status_only(ResultStatus::Denied);
    }

    let access = AccessContext::for_application(launch.app_id, launch.app_session_id);
    let Some(object_ids) = search(access, request.text) else {
        return SearchResults::status_only(ResultStatus::Unavailable);
    };
    let mut results = SearchResults::status_only(ResultStatus::Ok);
    results.visible_total = u16::try_from(object_ids.len()).unwrap_or(u16::MAX);
    for object_id in object_ids.iter().take(MAX_RESULT_IDS) {
        results.ids[results.count] = object_id.0;
        results.count += 1;
    }
    results
}

/// Serve one Files-only `search@1` request on the endpoint owned by init.
///
/// The ordinary app launcher does not yet publish this endpoint. Keeping the
/// handler here provides the authenticated service boundary for that wiring;
/// it is not evidence of a connected production client path by itself.
#[cfg(all(target_os = "nagi", feature = "m10-desktop", feature = "desktop-login"))]
#[allow(dead_code)]
pub(super) fn serve_files_one(runtime: &crate::m19_runtime::Runtime, endpoint: u64) -> bool {
    let mut message = ChannelReceiveResult::default();
    if channel_try_receive(endpoint, &mut message) != Some(true) {
        return false;
    }

    // A caller must not be able to smuggle unrelated handles through this
    // query endpoint. Close every kernel-delivered handle before rejecting.
    let transfer_count = message.transfer_count as usize;
    for handle in message
        .handles
        .iter()
        .take(transfer_count.min(message.handles.len()))
    {
        let _ = handle_close(*handle);
    }

    // This is a dedicated search endpoint. Ignore unrelated protocols, and
    // return a bounded error for malformed search envelopes.
    if message.protocol_id != PROTOCOL_ID {
        return true;
    }
    let payload_len = message.payload_len as usize;
    let malformed_envelope = message.version != PROTOCOL_VERSION
        || message.opcode != OPCODE_SEARCH
        || message.flags != 0
        || transfer_count != 0
        || payload_len > message.payload.len();
    let results = if malformed_envelope {
        SearchResults::status_only(ResultStatus::InvalidRequest)
    } else {
        let launch = supervisor::resolve(message.sender_process_id);
        evaluate_files(launch, &message.payload[..payload_len], |_, query| {
            runtime.search_files(query)
        })
    };

    let mut reply = ChannelSendRequest::new(
        PROTOCOL_ID,
        PROTOCOL_VERSION,
        message.request_id,
        OPCODE_RESULTS,
    );
    let Ok(length) = encode_results(&results, &mut reply.payload) else {
        return false;
    };
    reply.payload_len = length as u32;
    channel_send(endpoint, &reply)
}
