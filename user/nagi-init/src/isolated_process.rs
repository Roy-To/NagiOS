//! Supervisor-side acceptance for ADR 0043/0046: launch a real ELF into its
//! own address space through the Supervisor launch registry, and authorize
//! its requests by kernel-stamped caller identity.
//!
//! A request's identity is always the launch record resolved from the
//! kernel-stamped `sender_process_id`. Identity claims inside the payload are
//! data, never authority.

use libnagi::launch::{LaunchError, LaunchPlacement};
use libnagi::PROCESS_EXIT_KIND_FAULTED;
use libnagi::{
    channel_create_pair, channel_receive, channel_send, console_write, handle_close, process_spawn,
    ChannelReceiveResult, ChannelSendRequest, RIGHT_READ, RIGHT_WRITE,
};
use nagi_model::{AppId, AppSessionId, NodeId};

use crate::supervisor::{self, LaunchFailure, FAULTING_APP, ISOLATED_APP};

static ISOLATED_APP_PACKAGE: &[u8] = crate::acceptance_package!("isolated-app");
static FAULTING_APP_PACKAGE: &[u8] = crate::acceptance_package!("faulting-app");
/// Scratch copy for the tampered-package refusal check.
static mut TAMPERED_PACKAGE: [u8; nagi_package::MAX_PACKAGE_BYTES] =
    [0; nagi_package::MAX_PACKAGE_BYTES];

// Keep in sync with `user/nagi-isolated-app/src/bin/faulting_app.rs`.
const FAULT_PROTOCOL_ID: u16 = 0x4643;
const OPCODE_LAUNCH_FAULT: u16 = 1;
/// Page fault, invalid opcode, and general protection, in that order.
const FAULT_KINDS: [(u8, u64); 3] = [(1, 14), (2, 6), (3, 13)];
const FAULT_PLACEMENT: LaunchPlacement = LaunchPlacement {
    app_session_id: AppSessionId(0x4e41_4749_0047_0001),
    node_id: NodeId(0x4e41_4749_0047_0002),
    workspace_id: None,
};

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
/// The acceptance's privileged operation; no embedded manifest grants it.
const PRIVILEGED_CAPABILITY: &[u8] = b"system.acceptance-privileged";
const PLACEMENT: LaunchPlacement = LaunchPlacement {
    app_session_id: AppSessionId(0x4e41_4749_0043_0001),
    node_id: NodeId(0x4e41_4749_0043_0002),
    workspace_id: None,
};

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
    // An application without a manifest is refused before any process exists.
    if supervisor::check(AppId::from_identifier(b"org.nagi.system"), PLACEMENT)
        != Err(LaunchFailure::Registry(LaunchError::UnknownApplication))
    {
        return fail(b"undeclared application was launchable");
    }
    console_write(b"Nagi Supervisor undeclared application refused PASS\r\n");
    // ADR 0049: only signed packages launch, and only as the application
    // their signed manifest declares.
    let tampered = unsafe { &mut *core::ptr::addr_of_mut!(TAMPERED_PACKAGE) };
    let length = ISOLATED_APP_PACKAGE.len();
    tampered[..length].copy_from_slice(ISOLATED_APP_PACKAGE);
    // Flip one executable byte inside the signed region.
    tampered[length - nagi_package::SIGNATURE_BYTES - 64] ^= 0x01;
    if !matches!(
        supervisor::launch(&tampered[..length], ISOLATED_APP, PLACEMENT),
        Err(LaunchFailure::UnsignedPackage)
    ) || !matches!(
        supervisor::launch(ISOLATED_APP_PACKAGE, FAULTING_APP, PLACEMENT),
        Err(LaunchFailure::WrongApplication)
    ) || !matches!(
        supervisor::launch(&ISOLATED_APP_PACKAGE[..length - 1], ISOLATED_APP, PLACEMENT),
        Err(LaunchFailure::InvalidPackage)
    ) {
        return fail(b"unsigned, mismatched, or malformed package was accepted");
    }
    console_write(b"Nagi Supervisor signed package verification PASS\r\n");
    let launched = match supervisor::launch(ISOLATED_APP_PACKAGE, ISOLATED_APP, PLACEMENT) {
        Ok(launched) => launched,
        Err(_) => return fail(b"supervisor launch"),
    };
    let child_pid = launched.record.process_id;
    if child_pid <= 1 || supervisor::resolve(child_pid) != Some(launched.record) {
        return fail(b"launch record");
    }
    // The same live application session cannot be launched twice.
    if supervisor::check(ISOLATED_APP, PLACEMENT)
        != Err(LaunchFailure::Registry(LaunchError::SessionAlreadyLive))
    {
        return fail(b"duplicate live session accepted");
    }
    // Kernel bound: a second spawn is refused while the isolated slot is
    // occupied, even when called directly.
    if let Some(second) = channel_create_pair() {
        let refused = process_spawn(
            nagi_package::PackageView::parse(ISOLATED_APP_PACKAGE)
                .map(|view| view.executable())
                .unwrap_or_default(),
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
    if channel_receive(launched.endpoint, &mut message).is_none() {
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
    let Some(record) = supervisor::resolve(message.sender_process_id) else {
        return fail(b"unresolved caller");
    };
    if record.app_id != ISOLATED_APP || record.app_session_id != PLACEMENT.app_session_id {
        return fail(b"launch record mismatch");
    }
    let decision =
        if supervisor::has_grant(record.app_id, record.app_session_id, PRIVILEGED_CAPABILITY) {
            DECISION_ALLOW
        } else {
            DECISION_DENY
        };
    if decision != DECISION_DENY {
        return fail(b"forged system claim was authorized");
    }
    console_write(b"Nagi isolated process forged payload identity denied PASS\r\n");
    if !reply(
        launched.endpoint,
        record.process_id,
        decision,
        record.app_id,
    ) {
        return fail(b"reply send");
    }

    if channel_receive(launched.endpoint, &mut message).is_none() {
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

    // The child exits after its report; the kernel closes its handles and
    // the Supervisor revokes its launch record.
    let Some(status) = supervisor::reap(launched) else {
        return fail(b"child exit or launch revocation");
    };
    if !supervisor::exited_cleanly(&status) || status.process_id != child_pid {
        return fail(b"child exit status");
    }
    // The status was consumed: waiting again, or on an unknown ID, fails.
    if libnagi::process_wait(child_pid).is_some() || libnagi::process_wait(u32::MAX).is_some() {
        return fail(b"exit status consumed twice");
    }
    console_write(b"Nagi Supervisor process exit status PASS\r\n");
    if supervisor::resolve(child_pid).is_some()
        || supervisor::has_grant(
            ISOLATED_APP,
            PLACEMENT.app_session_id,
            PRIVILEGED_CAPABILITY,
        )
        || supervisor::check(ISOLATED_APP, PLACEMENT).is_err()
    {
        return fail(b"stale launch record");
    }
    console_write(b"Nagi isolated process exit cleanup PASS\r\n");
    if !fault_containment() {
        return false;
    }
    console_write(b"Nagi isolated process acceptance PASS\r\n");
    true
}

/// ADR 0047: an isolated process raising a CPU exception is terminated
/// alone. The kernel closes its handles (so its endpoint becomes
/// unreachable), the Supervisor revokes its launch, init keeps running, and
/// the slot can be used again by the next launch.
fn fault_containment() -> bool {
    for (kind, vector) in FAULT_KINDS {
        let launched = match supervisor::launch(FAULTING_APP_PACKAGE, FAULTING_APP, FAULT_PLACEMENT)
        {
            Ok(launched) => launched,
            Err(_) => return fail(b"faulting app launch"),
        };
        let mut request =
            ChannelSendRequest::new(FAULT_PROTOCOL_ID, PROTOCOL_VERSION, 0, OPCODE_LAUNCH_FAULT);
        request.payload[0] = kind;
        request.payload_len = 1;
        if !channel_send(launched.endpoint, &request) {
            return fail(b"fault launch argument");
        }
        let process_id = launched.record.process_id;
        let Some(status) = supervisor::reap(launched) else {
            return fail(b"faulting process was not terminated and reaped");
        };
        if status.kind != PROCESS_EXIT_KIND_FAULTED
            || status.fault_vector != vector
            || status.code != 128 + vector
            || status.process_id != process_id
            || supervisor::resolve(process_id).is_some()
        {
            return fail(b"fault exit status");
        }
    }
    console_write(b"Nagi isolated process fault containment PASS\r\n");
    true
}
