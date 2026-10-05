//! Isolated `action@1` client used by the M21/M22 acceptance (ADR 0045).
//!
//! The Supervisor passes the intent as a launch argument (the `argv`
//! equivalent). The client requests it from the action service and reports
//! the outcome back for verification. The client never states who it is;
//! the service authorizes it by kernel-stamped sender Process ID and launch
//! record.
#![cfg_attr(target_os = "nagi", no_std)]
#![cfg_attr(target_os = "nagi", no_main)]

#[cfg(target_os = "nagi")]
#[path = "../sys.rs"]
mod sys;

#[cfg(target_os = "nagi")]
mod client {
    use core::panic::PanicInfo;

    use nagi_abi::{ChannelReceiveResult, ChannelSendRequest, MAX_CHANNEL_INLINE_PAYLOAD};
    use nagi_action_ipc::{
        decode_intent, decode_result, encode_intent, encode_result, OPCODE_LAUNCH_INTENT,
        OPCODE_REQUEST, OPCODE_RESULT, PROTOCOL_ID, PROTOCOL_VERSION,
    };

    use crate::sys::{exit, receive, send, write};

    /// Acceptance-only report opcode; keep in sync with
    /// `user/nagi-init/src/supervisor.rs`.
    const OPCODE_CLIENT_REPORT: u16 = 0x7f02;

    fn payload(message: &ChannelReceiveResult) -> &[u8] {
        let length = (message.payload_len as usize).min(MAX_CHANNEL_INLINE_PAYLOAD);
        &message.payload[..length]
    }

    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, _process_id: u64) -> ! {
        write(b"Nagi action client started\r\n");
        let mut launch = ChannelReceiveResult::default();
        if !receive(endpoint, &mut launch)
            || launch.protocol_id != PROTOCOL_ID
            || launch.opcode != OPCODE_LAUNCH_INTENT
        {
            write(b"Nagi action client launch FAIL\r\n");
            exit(1);
        }
        let Ok(intent) = decode_intent(payload(&launch)) else {
            exit(1)
        };
        let mut request = ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 1, OPCODE_REQUEST);
        let Ok(length) = encode_intent(intent, &mut request.payload) else {
            exit(1)
        };
        request.payload_len = length as u32;
        if !send(endpoint, &request) {
            write(b"Nagi action client send FAIL\r\n");
            exit(1);
        }
        let mut reply = ChannelReceiveResult::default();
        if !receive(endpoint, &mut reply)
            || reply.protocol_id != PROTOCOL_ID
            || reply.opcode != OPCODE_RESULT
            || reply.request_id != 1
        {
            write(b"Nagi action client reply FAIL\r\n");
            exit(1);
        }
        let Ok(result) = decode_result(payload(&reply)) else {
            write(b"Nagi action client decode FAIL\r\n");
            exit(1)
        };
        let mut report =
            ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, 2, OPCODE_CLIENT_REPORT);
        let Ok(length) = encode_result(&result, &mut report.payload) else {
            exit(1)
        };
        report.payload_len = length as u32;
        if !send(endpoint, &report) {
            exit(1);
        }
        exit(0)
    }

    #[panic_handler]
    fn panic(_info: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
