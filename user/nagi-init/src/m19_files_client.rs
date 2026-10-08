//! One-shot, signed first-party Files client route (ADR 0069).

use core::arch::asm;

use libnagi::launch::{GrantCheck, LaunchPlacement};
use libnagi::storage::{SyscallBlockDevice, Vfs};
use libnagi::{channel_send, display_present, input_read, ChannelSendRequest, InputEvent};
use nagi_model::{AppId, AppSessionId, NodeId};
use nagi_search_ipc::{
    encode_request, KindFilter, ResultStatus, SearchRequest, SearchResults, OPCODE_ABORT,
    OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
};

use crate::desktop::{Desktop, UserDataVolume};
use crate::m19_runtime::Runtime;
use crate::{m19_search_service, supervisor};

const APP_ID: AppId = AppId::from_identifier(b"org.nagi.files");
const NODE_ID: NodeId = NodeId(0x4e41_4749_4d19_0069);
const SEARCH_QUERY: &[u8] = b"search.query";
const FILES_SEARCH: &[u8] = b"files.search";
const PRODUCT_PACKAGE: &[u8] = include_bytes!(env!("NAGI_FILES_SEARCH_PACKAGE"));

type PromptVolume = Vfs<SyscallBlockDevice>;

/// Run one bounded Files query through a real signed launch. Every failure
/// aborts/reaps the child and returns no IDs to the UI.
pub(super) fn search(
    runtime: &Runtime,
    query: &str,
    desktop: &mut Desktop,
    input_capability: u64,
    display_capability: u64,
    surface: &mut [u32],
    volume: &mut UserDataVolume,
) -> Option<SearchResults> {
    let user = desktop.signed_in_session()?;
    let mut session_bytes = [0_u8; 8];
    if !libnagi::random_fill(&mut session_bytes) {
        return None;
    }
    let app_session_id = AppSessionId(u64::from_le_bytes(session_bytes));
    if app_session_id.0 == 0 {
        return None;
    }
    let launched = supervisor::launch(
        PRODUCT_PACKAGE,
        APP_ID,
        LaunchPlacement {
            app_session_id,
            node_id: NODE_ID,
            workspace_id: None,
        },
    )
    .ok()?;

    let grants_ready = ensure_grant(
        desktop,
        input_capability,
        display_capability,
        surface,
        volume,
        &user,
        app_session_id,
        SEARCH_QUERY,
    ) && ensure_grant(
        desktop,
        input_capability,
        display_capability,
        surface,
        volume,
        &user,
        app_session_id,
        FILES_SEARCH,
    );
    if !grants_ready {
        abort_and_reap(launched, 0);
        print(b"Nagi M19 Files Search consent denied or unavailable\r\n");
        return None;
    }

    let mut bootstrap = ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, OPCODE_SEARCH);
    let Ok(length) = encode_request(
        SearchRequest {
            kind: KindFilter::File,
            text: query,
        },
        &mut bootstrap.payload,
    ) else {
        abort_and_reap(launched, 0);
        return None;
    };
    bootstrap.payload_len = length as u32;
    if !channel_send(launched.endpoint, &bootstrap) {
        abort_and_reap(launched, 0);
        return None;
    }

    let response =
        m19_search_service::serve_files_one(runtime, launched.endpoint, launched.record.process_id);
    if response.is_none() {
        abort_and_reap(launched, 1);
        return None;
    }
    let Some(status) = supervisor::reap(launched) else {
        print(b"Nagi M19 Files Search child cleanup FAIL\r\n");
        return None;
    };
    if !supervisor::exited_cleanly(&status) {
        print(b"Nagi M19 Files Search child cleanup FAIL\r\n");
        return None;
    }
    print(b"Nagi M19 Files Search child reaped PASS\r\n");
    let response = response?;
    (response.status == ResultStatus::Ok).then_some(response)
}

fn ensure_grant(
    desktop: &mut Desktop,
    input_capability: u64,
    display_capability: u64,
    surface: &mut [u32],
    volume: &mut PromptVolume,
    user: &libnagi::security::Session,
    app_session_id: AppSessionId,
    capability: &[u8],
) -> bool {
    match supervisor::check_grant(APP_ID, app_session_id, capability) {
        GrantCheck::Granted => true,
        GrantCheck::ConsentRequired => {
            let Ok(request) = supervisor::request_consent(APP_ID, app_session_id, capability)
            else {
                return false;
            };
            if request.app_id != APP_ID
                || request.app_session_id != app_session_id
                || request.capability() != capability
                || supervisor::next_consent_request() != Some(request)
            {
                return false;
            }
            desktop.open_consent(crate::consent_dialog::ConsentDialog::new(request));
            desktop.render(surface);
            if !display_present(display_capability) {
                return false;
            }
            desktop.arm_consent();
            let Some((answered, answer_check)) = wait_for_answer(
                desktop,
                input_capability,
                display_capability,
                surface,
                volume,
                user,
            ) else {
                return false;
            };
            answered == request
                && answer_check == GrantCheck::Granted
                && desktop.signed_in_session() == Some(*user)
                && supervisor::check_grant(APP_ID, app_session_id, capability)
                    == GrantCheck::Granted
        }
        GrantCheck::Denied | GrantCheck::NotDeclared | GrantCheck::NotLive => false,
    }
}

fn wait_for_answer(
    desktop: &mut Desktop,
    input_capability: u64,
    display_capability: u64,
    surface: &mut [u32],
    volume: &mut PromptVolume,
    user: &libnagi::security::Session,
) -> Option<(libnagi::launch::ConsentRequest, GrantCheck)> {
    loop {
        if desktop.signed_in_session() != Some(*user) {
            return None;
        }
        let mut event = InputEvent::default();
        if !input_read(input_capability, &mut event) {
            unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
            continue;
        }
        let _ = desktop.handle_event(event, volume);
        desktop.render(surface);
        if !display_present(display_capability) {
            return None;
        }
        desktop.arm_consent();
        if let Some(answer) = desktop.take_consent_decision() {
            return Some(answer);
        }
    }
}

fn abort_and_reap(launched: supervisor::Launched, request_id: u64) {
    let abort = ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, request_id, OPCODE_ABORT);
    if channel_send(launched.endpoint, &abort) {
        if supervisor::reap(launched).is_some_and(|status| supervisor::exited_cleanly(&status)) {
            print(b"Nagi M19 Files Search child reaped after denial PASS\r\n");
            return;
        }
    } else {
        let _ = supervisor::reap(launched);
    }
    print(b"Nagi M19 Files Search child cleanup FAIL\r\n");
}

#[cfg(feature = "desktop-login-acceptance")]
pub(super) fn report_visible_object_id(object_id: u64) {
    let mut digits = [0_u8; 20];
    let mut length = 0;
    let mut value = object_id;
    loop {
        digits[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    print(b"Nagi M19 signed-in desktop Files UI Search visible ObjectId=");
    while length > 0 {
        length -= 1;
        print(&digits[length..length + 1]);
    }
    print(b" PASS\r\n");
}

#[cfg(any(
    feature = "desktop-login-acceptance",
    feature = "m19-files-search-production"
))]
fn print(bytes: &[u8]) {
    let _ = libnagi::console_write(bytes);
}
