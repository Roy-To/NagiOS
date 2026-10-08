//! Capability-checked Files Search requests over `search@1` Channels.
//!
//! This module is shared by the ordinary signed-in Files runtime and the M19
//! isolated-client acceptance. Caller identity is resolved only from the
//! kernel-stamped sender PID; the request payload contains only a query.

use alloc::vec::Vec;

use libnagi::launch::LaunchRecord;
use libnagi::{
    channel_receive, channel_send, handle_close, ChannelReceiveResult, ChannelSendRequest,
};
use nagi_model::ObjectId;
use nagi_search::AccessContext;
use nagi_search_ipc::{
    decode_request, decode_results, encode_results, KindFilter, ResultStatus, SearchResults,
    MAX_RESULT_IDS, OPCODE_RESULTS, OPCODE_RESULT_RELAY, OPCODE_SEARCH, PROTOCOL_ID,
    PROTOCOL_VERSION,
};

use crate::supervisor;

const SEARCH_QUERY_GRANT: &[u8] = b"search.query";
const FILES_SEARCH_GRANT: &[u8] = b"files.search";
#[cfg(all(target_os = "nagi", feature = "m19-files-search-production"))]
const FILES_SEARCH_REQUEST_ID: u64 = 1;

#[cfg(all(target_os = "nagi", feature = "m19-files-search-production"))]
fn abort_files_client(endpoint: u64) {
    let abort = ChannelSendRequest::new(
        PROTOCOL_ID,
        PROTOCOL_VERSION,
        FILES_SEARCH_REQUEST_ID,
        nagi_search_ipc::OPCODE_ABORT,
    );
    let _ = channel_send(endpoint, &abort);
}

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
/// Serve one request from a first-party Files child on its private launch
/// Channel, then require that same live process to relay the exact bounded
/// result set before init presents any ObjectId.
#[cfg(all(target_os = "nagi", feature = "m19-files-search-production"))]
pub(super) fn serve_files_one(
    runtime: &crate::m19_runtime::Runtime,
    endpoint: u64,
    expected_process_id: u32,
) -> Option<SearchResults> {
    let mut message = ChannelReceiveResult::default();
    channel_receive(endpoint, &mut message)?;

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
        abort_files_client(endpoint);
        return None;
    }
    let payload_len = message.payload_len as usize;
    let malformed_envelope = message.version != PROTOCOL_VERSION
        || message.opcode != OPCODE_SEARCH
        || message.request_id != FILES_SEARCH_REQUEST_ID
        || message.flags != 0
        || transfer_count != 0
        || payload_len > message.payload.len();
    let launch = supervisor::resolve(message.sender_process_id);
    let results = if malformed_envelope || message.sender_process_id != expected_process_id {
        SearchResults::status_only(ResultStatus::InvalidRequest)
    } else {
        evaluate_files(launch, &message.payload[..payload_len], |access, query| {
            runtime.search_files_for_application(access, query)
        })
    };

    let mut reply = ChannelSendRequest::new(
        PROTOCOL_ID,
        PROTOCOL_VERSION,
        FILES_SEARCH_REQUEST_ID,
        OPCODE_RESULTS,
    );
    let Ok(length) = encode_results(&results, &mut reply.payload) else {
        abort_files_client(endpoint);
        return None;
    };
    reply.payload_len = length as u32;
    if !channel_send(endpoint, &reply) {
        abort_files_client(endpoint);
        return None;
    }

    let mut relay = ChannelReceiveResult::default();
    if channel_receive(endpoint, &mut relay).is_none() {
        abort_files_client(endpoint);
        return None;
    }
    let transfer_count = relay.transfer_count as usize;
    for handle in relay
        .handles
        .iter()
        .take(transfer_count.min(relay.handles.len()))
    {
        let _ = handle_close(*handle);
    }
    let relay_len = relay.payload_len as usize;
    if relay.sender_process_id != expected_process_id
        || supervisor::resolve(relay.sender_process_id) != launch
        || !supervisor::has_grant(launch?.app_id, launch?.app_session_id, SEARCH_QUERY_GRANT)
        || !supervisor::has_grant(launch?.app_id, launch?.app_session_id, FILES_SEARCH_GRANT)
        || relay.protocol_id != PROTOCOL_ID
        || relay.version != PROTOCOL_VERSION
        || relay.opcode != OPCODE_RESULT_RELAY
        || relay.request_id != FILES_SEARCH_REQUEST_ID
        || relay.flags != 0
        || transfer_count != 0
        || relay_len > relay.payload.len()
    {
        abort_files_client(endpoint);
        return None;
    }
    let Ok(relayed) = decode_results(&relay.payload[..relay_len]) else {
        abort_files_client(endpoint);
        return None;
    };
    if relayed != results {
        abort_files_client(endpoint);
        return None;
    }
    Some(results)
}
