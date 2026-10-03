//! Supervisor-side acceptance for ADR 0043: spawn a real ELF into its own
//! address space and authorize its requests by kernel-stamped caller
//! identity.
//!
//! The Supervisor keeps the launch record `ProcessId -> (AppId,
//! AppSessionId)`. A request's identity is always the record found by the
//! kernel-stamped `sender_process_id`. Identity claims inside the payload are
//! data, never authority.

use libnagi::{
    channel_create_pair, channel_receive, channel_send, console_write, handle_close, process_spawn,
    ChannelReceiveResult, ChannelSendRequest, RIGHT_READ, RIGHT_WAIT, RIGHT_WRITE,
};
use nagi_model::{AppId, AppSessionId};

static ISOLATED_APP_ELF: &[u8] = include_bytes!(env!("NAGI_ISOLATED_APP_ELF"));

// Keep in sync with `user/nagi-isolated-app/src/main.rs`.
const PROTOCOL_ID: u16 = 0x4f43;
const PROTOCOL_VERSION: u16 = 1;
const OPCODE_REQUEST: u16 = 1;
const OPCODE_REPLY: u16 = 2;
const OPCODE_REPORT: u16 = 3;
const FORGED_CLAIM: &[u8] = b"caller=org.nagi.system;pid=1";
const DECISION_DENY: u8 = 0;
const DECISION_ALLOW: u8 = 1;
const ALL_CHILD_CHECKS: u32 = (1 << 11) - 1;
// Stay below the Channel queue capacity so a live peer cannot make a probe
// send fail with QueueFull and be mistaken for an exit.
const EXIT_OBSERVATION_YIELDS: usize = 8;

const ISOLATED_APP_ID: &[u8] = b"org.nagi.acceptance.isolated-app";
const SYSTEM_APP_ID: &[u8] = b"org.nagi.system";

#[derive(Clone, Copy)]
struct LaunchRecord {
    process_id: u32,
    app_id: AppId,
    session_id: AppSessionId,
}

/// Bounded Supervisor launch table: the only source of process-to-app
/// identity. A process ID with no record has no identity.
struct LaunchRecords {
    records: [Option<LaunchRecord>; 2],
}

impl LaunchRecords {
    const fn new() -> Self {
        Self { records: [None; 2] }
    }

    fn record(&mut self, record: LaunchRecord) -> bool {
        let Some(slot) = self.records.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(record);
        true
    }

    fn resolve(&self, process_id: u32) -> Option<LaunchRecord> {
        self.records
            .iter()
            .flatten()
            .copied()
            .find(|record| record.process_id == process_id)
    }

    fn remove(&mut self, process_id: u32) {
        for slot in &mut self.records {
            if slot.is_some_and(|record| record.process_id == process_id) {
                *slot = None;
            }
        }
    }
}

/// Policy for the acceptance's privileged operation: only the system app may
/// perform it. The decision depends solely on the resolved launch record.
fn decide(caller: Option<LaunchRecord>) -> u8 {
    match caller {
        Some(record) if record.app_id == AppId::from_identifier(SYSTEM_APP_ID) => DECISION_ALLOW,
        _ => DECISION_DENY,
    }
}

fn fail(reason: &[u8]) -> bool {
    console_write(b"Nagi isolated process acceptance FAIL ");
    console_write(reason);
    console_write(b"\r\n");
    false
}

fn reply(endpoint: u64, resolved: u32, decision: u8, app_id: AppId) -> bool {
    let mut request = ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 1, OPCODE_REPLY);
    request.payload[..4].copy_from_slice(&resolved.to_le_bytes());
    request.payload[4] = decision;
    request.payload[5..13].copy_from_slice(&app_id.0.to_le_bytes());
    request.payload_len = 13;
    channel_send(endpoint, &request)
}

pub fn run() -> bool {
    console_write(b"Nagi isolated process acceptance START\r\n");
    let Some(endpoints) = channel_create_pair() else {
        return fail(b"channel");
    };
    let Some(child_pid) = process_spawn(
        ISOLATED_APP_ELF,
        endpoints.endpoint_b,
        RIGHT_READ | RIGHT_WRITE | RIGHT_WAIT,
    ) else {
        return fail(b"spawn");
    };
    if child_pid <= 1 {
        return fail(b"child pid");
    }
    let mut launches = LaunchRecords::new();
    let isolated_app = AppId::from_identifier(ISOLATED_APP_ID);
    if !launches.record(LaunchRecord {
        process_id: child_pid,
        app_id: isolated_app,
        session_id: AppSessionId(u64::from(child_pid)),
    }) {
        return fail(b"launch record");
    }
    // The moved endpoint is no longer in init's handle table.
    if channel_send(
        endpoints.endpoint_b,
        &ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 0, 0),
    ) {
        return fail(b"moved endpoint still usable by init");
    }
    // A second spawn is refused while the isolated slot is occupied.
    if let Some(second) = channel_create_pair() {
        let refused = process_spawn(
            ISOLATED_APP_ELF,
            second.endpoint_b,
            RIGHT_READ | RIGHT_WRITE,
        )
        .is_none();
        let _ = handle_close(second.endpoint_a);
        let _ = handle_close(second.endpoint_b);
        if !refused {
            return fail(b"second spawn accepted");
        }
    } else {
        return fail(b"second channel");
    }

    let mut message = ChannelReceiveResult::default();
    if channel_receive(endpoints.endpoint_a, &mut message).is_none() {
        return fail(b"request receive");
    }
    if message.protocol_id != PROTOCOL_ID || message.opcode != OPCODE_REQUEST {
        return fail(b"request header");
    }
    let payload = &message.payload[..message.payload_len as usize];
    if payload != FORGED_CLAIM {
        return fail(b"request payload");
    }
    if message.sender_process_id != child_pid {
        return fail(b"sender pid not kernel-stamped child");
    }
    console_write(b"Nagi isolated process kernel-stamped sender PASS\r\n");
    let caller = launches.resolve(message.sender_process_id);
    let Some(record) = caller else {
        return fail(b"unresolved caller");
    };
    if record.app_id != isolated_app || record.session_id != AppSessionId(u64::from(child_pid)) {
        return fail(b"launch record mismatch");
    }
    let decision = decide(caller);
    if decision != DECISION_DENY {
        return fail(b"forged system claim was authorized");
    }
    console_write(b"Nagi isolated process forged payload identity denied PASS\r\n");
    if !reply(
        endpoints.endpoint_a,
        record.process_id,
        decision,
        record.app_id,
    ) {
        return fail(b"reply send");
    }

    if channel_receive(endpoints.endpoint_a, &mut message).is_none() {
        return fail(b"report receive");
    }
    if message.opcode != OPCODE_REPORT
        || message.sender_process_id != child_pid
        || message.payload_len != 4
    {
        return fail(b"report header");
    }
    let checks = u32::from_le_bytes([
        message.payload[0],
        message.payload[1],
        message.payload[2],
        message.payload[3],
    ]);
    if checks != ALL_CHILD_CHECKS {
        console_write(b"Nagi isolated process child checks=");
        let mut digits = [0_u8; 8];
        for (index, digit) in digits.iter_mut().enumerate() {
            let nibble = (checks >> (28 - index * 4)) & 0xf;
            *digit = b"0123456789abcdef"[nibble as usize];
        }
        console_write(&digits);
        console_write(b"\r\n");
        return fail(b"child isolation checks");
    }
    console_write(b"Nagi isolated process address space and syscall isolation PASS\r\n");

    // Let the child run to its exit. Its handles are then closed by the
    // kernel and the Channel peer becomes unreachable.
    let mut exited = false;
    for _ in 0..EXIT_OBSERVATION_YIELDS {
        libnagi::sleep_ns(0);
        if !channel_send(
            endpoints.endpoint_a,
            &ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 2, 0),
        ) {
            exited = true;
            break;
        }
        // A message queued before exit is discarded with the Channel.
    }
    if !exited {
        return fail(b"child exit not observed");
    }
    launches.remove(child_pid);
    if launches.resolve(child_pid).is_some() {
        return fail(b"stale launch record");
    }
    if !handle_close(endpoints.endpoint_a) {
        return fail(b"endpoint close");
    }
    console_write(b"Nagi isolated process exit cleanup PASS\r\n");
    console_write(b"Nagi isolated process acceptance PASS\r\n");
    true
}
