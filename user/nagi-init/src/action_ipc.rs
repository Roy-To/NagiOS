//! Supervisor side of `action@1` (ADR 0045).
//!
//! The Supervisor launches the real `nagi-action-client` ELF into its own
//! address space (ADR 0043) and records `ProcessId -> CallerIdentity` at
//! launch. Each request's caller is resolved only from the kernel-stamped
//! sender Process ID and that record. The resolved identity is the only one
//! handed to Context resolution, Plan validation, policy, and execution.

use alloc::string::{String, ToString};

use libnagi::{
    channel_create_pair, channel_receive, channel_send, console_write, handle_close, process_spawn,
    sleep_ns, ChannelReceiveResult, ChannelSendRequest, RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
use nagi_action_ipc::{
    decode_intent, decode_result, encode_intent, encode_result, ActionResult, ActionStatus,
    OPCODE_LAUNCH_INTENT, OPCODE_REQUEST, OPCODE_RESULT, PROTOCOL_ID, PROTOCOL_VERSION,
};
use nagi_ai::CallerIdentity;

static ACTION_CLIENT_ELF: &[u8] = include_bytes!(env!("NAGI_ACTION_CLIENT_ELF"));

/// Acceptance-only report opcode; keep in sync with
/// `user/nagi-isolated-app/src/bin/action_client.rs`.
const OPCODE_CLIENT_REPORT: u16 = 0x7f02;
// Stay below the Channel queue capacity (see `isolated_process.rs`).
const EXIT_OBSERVATION_YIELDS: usize = 8;

#[derive(Clone, Copy)]
struct LaunchRecord {
    process_id: u32,
    caller: CallerIdentity,
}

struct ActionClient {
    endpoint: u64,
    record: LaunchRecord,
}

fn payload(message: &ChannelReceiveResult) -> &[u8] {
    let length = (message.payload_len as usize).min(message.payload.len());
    &message.payload[..length]
}

impl ActionClient {
    /// Spawn the client as `caller` and pass `intent` as its launch argument.
    fn launch(caller: CallerIdentity, intent: &str) -> Option<Self> {
        let endpoints = channel_create_pair()?;
        let Some(process_id) = process_spawn(
            ACTION_CLIENT_ELF,
            endpoints.endpoint_b,
            RIGHT_READ | RIGHT_WRITE | RIGHT_WAIT,
        ) else {
            let _ = handle_close(endpoints.endpoint_a);
            let _ = handle_close(endpoints.endpoint_b);
            return None;
        };
        let client = Self {
            endpoint: endpoints.endpoint_a,
            record: LaunchRecord { process_id, caller },
        };
        let mut launch =
            ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, OPCODE_LAUNCH_INTENT);
        launch.payload_len = encode_intent(intent, &mut launch.payload).ok()? as u32;
        channel_send(client.endpoint, &launch).then_some(client)
    }

    /// Receive one request. The caller comes from the launch record matched
    /// by kernel-stamped sender ID, never from the payload.
    fn receive_request(&self) -> Option<(u64, Option<CallerIdentity>, Option<String>)> {
        let mut message = ChannelReceiveResult::default();
        channel_receive(self.endpoint, &mut message)?;
        if message.protocol_id != PROTOCOL_ID || message.opcode != OPCODE_REQUEST {
            return None;
        }
        let caller =
            (message.sender_process_id == self.record.process_id).then_some(self.record.caller);
        let intent = decode_intent(payload(&message))
            .ok()
            .map(ToString::to_string);
        Some((message.request_id, caller, intent))
    }

    fn respond(&self, request_id: u64, result: &ActionResult) -> bool {
        let mut reply =
            ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, request_id, OPCODE_RESULT);
        let Ok(length) = encode_result(result, &mut reply.payload) else {
            return false;
        };
        reply.payload_len = length as u32;
        channel_send(self.endpoint, &reply)
    }

    /// Collect the client's report of what it received, then observe its
    /// exit and release the endpoint.
    fn finish(self) -> Option<ActionResult> {
        let mut report = ChannelReceiveResult::default();
        channel_receive(self.endpoint, &mut report)?;
        if report.opcode != OPCODE_CLIENT_REPORT
            || report.sender_process_id != self.record.process_id
        {
            return None;
        }
        let reported = decode_result(payload(&report)).ok()?;
        let mut exited = false;
        for _ in 0..EXIT_OBSERVATION_YIELDS {
            sleep_ns(0);
            if !channel_send(
                self.endpoint,
                &ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, 0),
            ) {
                exited = true;
                break;
            }
        }
        (exited && handle_close(self.endpoint)).then_some(reported)
    }
}

/// Launch an isolated client as `launched_as` with `intent` and serve its
/// single request with `handle`. `handle` receives only the identity resolved
/// from the launch record.
///
/// Returns the result the client reports having received. It must be exactly
/// what the service sent.
pub fn serve_isolated_request(
    launched_as: CallerIdentity,
    intent: &str,
    handle: impl FnOnce(CallerIdentity, &str) -> ActionResult,
) -> Option<ActionResult> {
    let client = ActionClient::launch(launched_as, intent)?;
    let (request_id, caller, intent) = client.receive_request()?;
    let result = match (caller, intent) {
        (None, _) => ActionResult::status_only(ActionStatus::UnknownCaller),
        (_, None) => ActionResult::status_only(ActionStatus::InvalidRequest),
        (Some(caller), Some(intent)) => handle(caller, &intent),
    };
    if !client.respond(request_id, &result) {
        return None;
    }
    let reported = client.finish()?;
    if reported != result {
        console_write(b"Nagi action IPC report mismatch\r\n");
        return None;
    }
    Some(reported)
}
