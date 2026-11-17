//! M19 SearchService over `search@1` Channels to isolated clients
//! (ADR 0044, ADR 0046).
//!
//! The Supervisor launches the real `nagi-m19-search-client` ELF through its
//! launch registry. Each request's caller is the launch record resolved from
//! the kernel-stamped sender Process ID. The caller must hold a live
//! `search.query` grant. The SearchService's `VisibilityFilter` then applies
//! the resolved application/session identity. The client never sends its own
//! identity.

use alloc::string::ToString;

use libnagi::launch::{LaunchPlacement, LaunchRecord};
use libnagi::{
    channel_receive, channel_send, console_write, ChannelReceiveResult, ChannelSendRequest,
};
use nagi_model::{AppSessionId, ObjectId};
use nagi_search::{AccessContext, ObjectKind, SearchQuery};
use nagi_search_ipc::{
    decode_request, decode_results, encode_results, KindFilter, ResultStatus, SearchResults,
    MAX_RESULT_IDS, OPCODE_RESULTS, OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
};

use super::{M19SearchService, APP_ID, NODE_ID, SESSION_ID, WORKSPACE_ID};
use crate::supervisor::{self, FOREIGN_APP};

static M19_SEARCH_CLIENT_PACKAGE: &[u8] = crate::acceptance_package!("m19-search-search-client");
static FOREIGN_SEARCH_CLIENT_PACKAGE: &[u8] = crate::acceptance_package!("foreign-search-client");

/// Acceptance-only client report; keep in sync with
/// `user/nagi-isolated-app/src/bin/search_client.rs`.
const OPCODE_CLIENT_REPORT: u16 = 0x7f01;
const SEARCH_GRANT: &[u8] = b"search.query";
const FOREIGN_SESSION_ID: AppSessionId = AppSessionId(0x4e41_4749_4d19_00ff);

/// Evaluate one request for the caller resolved from `launch`. A missing
/// launch record yields `UnknownCaller`, and a launch without a live
/// `search.query` grant yields `Denied`; neither reads the index.
fn evaluate(
    service: &M19SearchService,
    launch: Option<LaunchRecord>,
    payload: &[u8],
) -> SearchResults {
    if payload.first() == Some(&(KindFilter::File as u8)) {
        return crate::m19_search_service::evaluate_files(launch, payload, |access, text| {
            let query = SearchQuery {
                text: Some(text.to_string()),
                kind: Some(ObjectKind::File),
                ..SearchQuery::default()
            };
            service.search(access, &query).ok().map(|response| {
                response
                    .objects
                    .into_iter()
                    .map(|hit| hit.record.object_id)
                    .collect()
            })
        });
    }
    let Some(launch) = launch else {
        return SearchResults::status_only(ResultStatus::UnknownCaller);
    };
    if !supervisor::has_grant(launch.app_id, launch.app_session_id, SEARCH_GRANT) {
        return SearchResults::status_only(ResultStatus::Denied);
    }
    let Ok(request) = decode_request(payload) else {
        return SearchResults::status_only(ResultStatus::InvalidRequest);
    };
    let query = SearchQuery {
        text: Some(request.text.to_string()),
        kind: match request.kind {
            KindFilter::Any => None,
            KindFilter::File => Some(ObjectKind::File),
            KindFilter::Page => Some(ObjectKind::Page),
        },
        ..SearchQuery::default()
    };
    let access = AccessContext::for_application(launch.app_id, launch.app_session_id);
    let Ok(response) = service.search(access, &query) else {
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
fn serve_one(service: &M19SearchService, endpoint: u64) -> bool {
    let mut message = ChannelReceiveResult::default();
    if channel_receive(endpoint, &mut message).is_none()
        || message.protocol_id != PROTOCOL_ID
        || message.opcode != OPCODE_SEARCH
    {
        return false;
    }
    let launch = supervisor::resolve(message.sender_process_id);
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

/// Launch one client as the declared `app_id`, serve its request, and return
/// the results the client reports having received.
fn run_client(
    service: &M19SearchService,
    app_id: nagi_model::AppId,
    app_session_id: AppSessionId,
) -> Option<[SearchResults; 3]> {
    let placement = LaunchPlacement {
        app_session_id,
        node_id: NODE_ID,
        workspace_id: Some(WORKSPACE_ID),
    };
    let package = if app_id == APP_ID {
        M19_SEARCH_CLIENT_PACKAGE
    } else {
        FOREIGN_SEARCH_CLIENT_PACKAGE
    };
    let launched = supervisor::launch(package, app_id, placement).ok()?;
    let mut results = [SearchResults::status_only(ResultStatus::Unavailable); 3];
    for (index, report_request_id) in [2_u64, 4, 6].iter().enumerate() {
        if !serve_one(service, launched.endpoint) {
            return None;
        }
        let mut report = ChannelReceiveResult::default();
        channel_receive(launched.endpoint, &mut report)?;
        if report.opcode != OPCODE_CLIENT_REPORT
            || report.request_id != *report_request_id
            || report.sender_process_id != launched.record.process_id
        {
            return None;
        }
        let payload_len = (report.payload_len as usize).min(report.payload.len());
        results[index] = decode_results(&report.payload[..payload_len]).ok()?;
    }
    supervisor::reap(launched)
        .filter(supervisor::exited_cleanly)
        .map(|_| [results[0], results[1], results[2]])
}

pub fn run(
    service: &M19SearchService,
    live_file: ObjectId,
    page: ObjectId,
    browser_history_page: Option<ObjectId>,
) -> bool {
    console_write(b"Nagi M19 Search IPC trace start\r\n");
    let Some([file_results, page_results, browser_history_results]) =
        run_client(service, APP_ID, SESSION_ID)
    else {
        console_write(b"Nagi M19 Search IPC FAIL authorized client\r\n");
        return false;
    };
    if file_results.status != ResultStatus::Ok || file_results.ids() != [live_file.0] {
        console_write(b"Nagi M19 Search IPC FAIL authorized results\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC authorized isolated client PASS\r\n");
    if page_results.status != ResultStatus::Ok || page_results.ids() != [page.0] {
        console_write(b"Nagi M19 Search IPC FAIL authorized page results\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC page authorized isolated client PASS\r\n");
    match browser_history_page {
        Some(expected_id)
            if browser_history_results.status == ResultStatus::Ok
                && browser_history_results.ids().contains(&expected_id.0) =>
        {
            console_write(b"Nagi M19 Browser history authenticated Search IPC PASS\r\n");
        }
        Some(_) => {
            console_write(b"Nagi M19 Search IPC FAIL authorized Browser history results\r\n");
            return false;
        }
        None if browser_history_results.status == ResultStatus::Ok
            && browser_history_results.count == 0 => {}
        None => {
            console_write(b"Nagi M19 Search IPC FAIL unexpected Browser history results\r\n");
            return false;
        }
    }

    // A different declared application has `search.query` only, so its Files
    // query is denied before the Search index is read. Page queries still run
    // through the fixture's caller-specific visibility policy.
    let Some([foreign_files, foreign_pages, foreign_browser_history]) =
        run_client(service, FOREIGN_APP, FOREIGN_SESSION_ID)
    else {
        console_write(b"Nagi M19 Search IPC FAIL foreign client\r\n");
        return false;
    };
    if foreign_files.status != ResultStatus::Denied
        || foreign_files.count != 0
        || foreign_files.visible_total != 0
    {
        console_write(b"Nagi M19 Search IPC FAIL foreign results leaked\r\n");
        return false;
    }
    if foreign_pages.status != ResultStatus::Ok
        || foreign_pages.count != 0
        || foreign_pages.visible_total != 0
    {
        console_write(b"Nagi M19 Search IPC FAIL foreign page results leaked\r\n");
        return false;
    }
    if foreign_browser_history.status != ResultStatus::Ok
        || foreign_browser_history.count != 0
        || foreign_browser_history.visible_total != 0
    {
        console_write(b"Nagi M19 Search IPC FAIL foreign Browser history leaked\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC foreign isolated client hidden PASS\r\n");

    // The authorized manifest and grants with a different live app session
    // still cannot cross the Search visibility filter for Files metadata.
    let Some([other_session_files, _, _]) = run_client(service, APP_ID, FOREIGN_SESSION_ID) else {
        console_write(b"Nagi M19 Search IPC FAIL other-session client\r\n");
        return false;
    };
    if other_session_files.status != ResultStatus::Ok
        || other_session_files.count != 0
        || other_session_files.visible_total != 0
    {
        console_write(b"Nagi M19 Search IPC FAIL Files visibility filter\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC Files visibility filter PASS\r\n");

    // A sender without a launch record is refused before the index is read,
    // and so is a reaped launch whose grant was revoked with its exit.
    let revoked = LaunchRecord {
        process_id: 2,
        app_id: APP_ID,
        app_session_id: SESSION_ID,
        node_id: NODE_ID,
        workspace_id: Some(WORKSPACE_ID),
    };
    if evaluate(service, None, &[1, 1, b'n']).status != ResultStatus::UnknownCaller
        || evaluate(service, Some(revoked), &[1, 1, b'n']).status != ResultStatus::Denied
    {
        console_write(b"Nagi M19 Search IPC FAIL unknown or revoked caller\r\n");
        return false;
    }
    console_write(b"Nagi M19 Search IPC authenticated caller PASS\r\n");
    true
}
