//! M19 SearchService over `search@1` Channels to isolated clients (ADR 0044).
//!
//! The Supervisor spawns the real `nagi-m19-search-client` ELF into its own
//! address space (ADR 0043) and records `ProcessId -> AccessContext` at
//! launch. Each request is authorized only through the kernel-stamped sender
//! Process ID and that record. The SearchService's `VisibilityFilter` then
//! applies the resolved application/session identity. The client never sends
//! its own identity.

use alloc::string::ToString;

use libnagi::{
    channel_create_pair, channel_receive, channel_send, console_write, handle_close, process_spawn,
    sleep_ns, ChannelReceiveResult, ChannelSendRequest, RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
use nagi_model::{AppId, AppSessionId, ObjectId};
use nagi_search::{AccessContext, ObjectKind, SearchQuery};
use nagi_search_ipc::{
    decode_request, decode_results, encode_results, KindFilter, ResultStatus, SearchResults,
    MAX_RESULT_IDS, OPCODE_RESULTS, OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
};

use super::{M19SearchService, APP_ID, SESSION_ID};

static SEARCH_CLIENT_ELF: &[u8] = include_bytes!(env!("NAGI_M19_SEARCH_CLIENT_ELF"));

/// Acceptance-only client report; keep in sync with
/// `user/nagi-isolated-app/src/bin/search_client.rs`.
const OPCODE_CLIENT_REPORT: u16 = 0x7f01;
const FOREIGN_APP_ID: &[u8] = b"org.nagi.acceptance.foreign-search-client";
const FOREIGN_SESSION_ID: AppSessionId = AppSessionId(0x4e41_4749_4d19_00ff);
// Stay below the Channel queue capacity (see `isolated_process.rs`).
const EXIT_OBSERVATION_YIELDS: usize = 8;

#[derive(Clone, Copy)]
struct LaunchRecord {
    process_id: u32,
    access: AccessContext,
}

/// Evaluate one request for the caller resolved from `launch`. A missing
/// launch record yields `UnknownCaller` without touching the index.
fn evaluate(
    service: &M19SearchService,
    launch: Option<LaunchRecord>,
    payload: &[u8],
) -> SearchResults {
    let Some(launch) = launch else {
        return SearchResults::status_only(ResultStatus::UnknownCaller);
    };
    let Ok(request) = decode_request(payload) else {
        return SearchResults::status_only(ResultStatus::InvalidRequest);
    };
    let query = SearchQuery {
        text: Some(request.text.to_string()),
        kind: match request.kind {
            KindFilter::Any => None,
            KindFilter::File => Some(ObjectKind::File),
        },
        ..SearchQuery::default()
    };
    let Ok(response) = service.search(launch.access, &query) else {
        return SearchResults::status_only(ResultStatus::Unavailable);
    };
    let mut results = SearchResults::status_only(ResultStatus::Ok);
    results.visible_total = u16::try_from(response.objects.len()).unwrap_or(u16::MAX);
    for hit in response.objects.iter().take(MAX_RESULT_IDS) {
        results.ids[results.count] = hit.record.object_id.0;
        results.count += 1;
    }
    results
}

/// Serve exactly one `search@1` request on `endpoint`.
fn serve_one(service: &M19SearchService, launches: &[LaunchRecord], endpoint: u64) -> bool {
    let mut message = ChannelReceiveResult::default();
    if channel_receive(endpoint, &mut message).is_none()
        || message.protocol_id != PROTOCOL_ID
        || message.opcode != OPCODE_SEARCH
    {
        return false;
    }
    let launch = launches
        .iter()
        .copied()
        .find(|record| record.process_id == message.sender_process_id);
    let payload_len = (message.payload_len as usize).min(message.payload.len());
    let results = evaluate(service, launch, &message.payload[..payload_len]);
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

/// Launch one client as `access`, serve its request, and return the results
/// the client reports having received.
fn run_client(service: &M19SearchService, access: AccessContext) -> Option<SearchResults> {
    let endpoints = channel_create_pair()?;
    let Some(process_id) = process_spawn(
        SEARCH_CLIENT_ELF,
        endpoints.endpoint_b,
        RIGHT_READ | RIGHT_WRITE | RIGHT_WAIT,
    ) else {
        let _ = handle_close(endpoints.endpoint_a);
        let _ = handle_close(endpoints.endpoint_b);
        return None;
    };
    let launches = [LaunchRecord { process_id, access }];
    if !serve_one(service, &launches, endpoints.endpoint_a) {
        return None;
    }
    let mut report = ChannelReceiveResult::default();
    channel_receive(endpoints.endpoint_a, &mut report)?;
    if report.opcode != OPCODE_CLIENT_REPORT || report.sender_process_id != process_id {
        return None;
    }
    let payload_len = (report.payload_len as usize).min(report.payload.len());
    let results = decode_results(&report.payload[..payload_len]).ok()?;

    let mut exited = false;
    for _ in 0..EXIT_OBSERVATION_YIELDS {
        sleep_ns(0);
        if !channel_send(
            endpoints.endpoint_a,
            &ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, 0),
        ) {
            exited = true;
            break;
        }
    }
    if !exited || !handle_close(endpoints.endpoint_a) {
        return None;
    }
    Some(results)
}

pub fn run(service: &M19SearchService, live_file: ObjectId) -> bool {
    console_write(b"Nagi M19 Search IPC trace start\r\n");
    let authorized = AccessContext::for_application(APP_ID, SESSION_ID);
    let Some(results) = run_client(service, authorized) else {
        console_write(b"Nagi M19 Search IPC FAIL authorized client\r\n");
        return false;
    };
    if results.status != ResultStatus::Ok || results.ids() != [live_file.0] {
        console_write(b"Nagi M19 Search IPC FAIL authorized results\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC authorized isolated client PASS\r\n");

    // Same ELF, same query, launched as a different application: the
    // resolved identity sees nothing, so nothing crosses the boundary.
    let foreign =
        AccessContext::for_application(AppId::from_identifier(FOREIGN_APP_ID), FOREIGN_SESSION_ID);
    let Some(results) = run_client(service, foreign) else {
        console_write(b"Nagi M19 Search IPC FAIL foreign client\r\n");
        return false;
    };
    if results.status != ResultStatus::Ok || results.count != 0 || results.visible_total != 0 {
        console_write(b"Nagi M19 Search IPC FAIL foreign results leaked\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC foreign isolated client hidden PASS\r\n");

    // A sender without a launch record is refused before the index is read.
    if evaluate(service, None, &[1, 1, b'n']).status != ResultStatus::UnknownCaller {
        console_write(b"Nagi M19 Search IPC FAIL unknown caller\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC authenticated caller PASS\r\n");
    true
}
