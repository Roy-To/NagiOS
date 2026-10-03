//! Supervisor side of `action@1` (ADR 0045, ADR 0046).
//!
//! The Supervisor launches the real `nagi-action-client` ELF through its
//! launch registry. Each request's caller is the launch record resolved from
//! the kernel-stamped sender Process ID. That identity is the only one handed
//! to Context resolution, Plan validation, policy, and execution. Policies
//! consult the registry for the caller's live manifest grants.

use alloc::string::{String, ToString};

use libnagi::launch::{LaunchPlacement, LaunchRecord};
use libnagi::{
    channel_receive, channel_send, console_write, ChannelReceiveResult, ChannelSendRequest,
};
use nagi_action_ipc::{
    decode_intent, decode_result, encode_intent, encode_result, ActionResult, ActionStatus,
    OPCODE_LAUNCH_INTENT, OPCODE_REQUEST, OPCODE_RESULT, PROTOCOL_ID, PROTOCOL_VERSION,
};
use nagi_ai::CallerIdentity;
use nagi_model::AppId;

use crate::supervisor::{self, Launched};

static ACTION_CLIENT_ELF: &[u8] = include_bytes!(env!("NAGI_ACTION_CLIENT_ELF"));

/// Acceptance-only report opcode; keep in sync with
/// `user/nagi-isolated-app/src/bin/action_client.rs`.
const OPCODE_CLIENT_REPORT: u16 = 0x7f02;

fn payload(message: &ChannelReceiveResult) -> &[u8] {
    let length = (message.payload_len as usize).min(message.payload.len());
    &message.payload[..length]
}

fn caller_identity(record: LaunchRecord) -> CallerIdentity {
    CallerIdentity {
        app_id: record.app_id,
        app_session_id: record.app_session_id,
        node_id: record.node_id,
        workspace_id: record.workspace_id,
    }
}

/// Whether the live launched session behind `caller` holds `capability`.
/// Identities no live launch holds have no grants.
pub fn caller_has_grant(caller: CallerIdentity, capability: &str) -> bool {
    supervisor::has_grant(caller.app_id, caller.app_session_id, capability.as_bytes())
}

/// Receive one request. The caller is the registry's launch record for the
/// kernel-stamped sender ID, never anything in the payload.
fn receive_request(launched: &Launched) -> Option<(u64, Option<CallerIdentity>, Option<String>)> {
    let mut message = ChannelReceiveResult::default();
    channel_receive(launched.endpoint, &mut message)?;
    if message.protocol_id != PROTOCOL_ID || message.opcode != OPCODE_REQUEST {
        return None;
    }
    let caller = supervisor::resolve(message.sender_process_id).map(caller_identity);
    let intent = decode_intent(payload(&message))
        .ok()
        .map(ToString::to_string);
    Some((message.request_id, caller, intent))
}

fn respond(launched: &Launched, request_id: u64, result: &ActionResult) -> bool {
    let mut reply =
        ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, request_id, OPCODE_RESULT);
    let Ok(length) = encode_result(result, &mut reply.payload) else {
        return false;
    };
    reply.payload_len = length as u32;
    channel_send(launched.endpoint, &reply)
}

/// Collect the client's report of what it received, then reap it.
fn finish(launched: Launched) -> Option<ActionResult> {
    let mut report = ChannelReceiveResult::default();
    channel_receive(launched.endpoint, &mut report)?;
    if report.opcode != OPCODE_CLIENT_REPORT
        || report.sender_process_id != launched.record.process_id
    {
        return None;
    }
    let reported = decode_result(payload(&report)).ok()?;
    supervisor::reap(launched)
        .filter(supervisor::exited_cleanly)
        .map(|_| reported)
}

/// Launch the declared application `app_id` at `placement` with `intent`
/// as its launch argument, and serve its single request with `handle`.
/// `handle` receives only the identity resolved from the launch record.
///
/// Returns the result the client reports having received. It must be exactly
/// what the service sent.
pub fn serve_isolated_request(
    app_id: AppId,
    placement: LaunchPlacement,
    intent: &str,
    handle: impl FnOnce(CallerIdentity, &str) -> ActionResult,
) -> Option<ActionResult> {
    let launched = supervisor::launch(ACTION_CLIENT_ELF, app_id, placement).ok()?;
    let mut launch =
        ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, OPCODE_LAUNCH_INTENT);
    launch.payload_len = encode_intent(intent, &mut launch.payload).ok()? as u32;
    if !channel_send(launched.endpoint, &launch) {
        return None;
    }
    let (request_id, caller, intent) = receive_request(&launched)?;
    let result = match (caller, intent) {
        (None, _) => ActionResult::status_only(ActionStatus::UnknownCaller),
        (_, None) => ActionResult::status_only(ActionStatus::InvalidRequest),
        (Some(caller), Some(intent)) => handle(caller, &intent),
    };
    if !respond(&launched, request_id, &result) {
        return None;
    }
    let reported = finish(launched)?;
    if reported != result {
        console_write(b"Nagi action IPC report mismatch\r\n");
        return None;
    }
    Some(reported)
}

/// The placement an acceptance caller identity is launched at.
pub fn placement_of(caller: CallerIdentity) -> LaunchPlacement {
    LaunchPlacement {
        app_session_id: caller.app_session_id,
        node_id: caller.node_id,
        workspace_id: caller.workspace_id,
    }
}
