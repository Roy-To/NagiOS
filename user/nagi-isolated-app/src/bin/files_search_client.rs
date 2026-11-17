//! One-shot first-party Files `search@1` client (ADR 0069).
//!
//! Init supplies one bounded File query over the private launch Channel. This
//! child validates the bootstrap, sends the request as the authenticated
//! caller, then relays the bounded service response to init for visible-title
//! resolution. It has no device or file capability.
#![cfg_attr(target_os = "nagi", no_std)]
#![cfg_attr(target_os = "nagi", no_main)]

#[cfg(target_os = "nagi")]
#[path = "../sys.rs"]
mod sys;

#[cfg(target_os = "nagi")]
mod client {
    use core::panic::PanicInfo;

    use nagi_abi::{ChannelReceiveResult, ChannelSendRequest, MAX_CHANNEL_INLINE_PAYLOAD};
    use nagi_search_ipc::{
        decode_request, decode_results, encode_results, KindFilter, OPCODE_ABORT, OPCODE_RESULTS,
        OPCODE_RESULT_RELAY, OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
    };

    use crate::sys::{exit, receive, send, write};

    const INIT_PROCESS_ID: u32 = 1;
    const FIRST_REQUEST_ID: u64 = 1;

    fn payload(message: &ChannelReceiveResult) -> Option<&[u8]> {
        let length = message.payload_len as usize;
        (length <= MAX_CHANNEL_INLINE_PAYLOAD).then_some(&message.payload[..length])
    }

    fn start_request(endpoint: u64) -> ! {
        let mut launch = ChannelReceiveResult::default();
        if !receive(endpoint, &mut launch)
            || launch.protocol_id != PROTOCOL_ID
            || launch.version != PROTOCOL_VERSION
            || launch.request_id != 0
            || launch.sender_process_id != INIT_PROCESS_ID
            || launch.flags != 0
            || launch.transfer_count != 0
        {
            write(b"Nagi Files Search client launch request FAIL\r\n");
            exit(1);
        }
        if launch.opcode == OPCODE_ABORT {
            exit(0);
        }
        if launch.opcode != OPCODE_SEARCH {
            exit(1);
        }
        let Some(launch_payload) = payload(&launch) else {
            exit(1);
        };
        let Ok(request) = decode_request(launch_payload) else {
            exit(1);
        };
        if request.kind != KindFilter::File {
            exit(1);
        }

        let mut outgoing = ChannelSendRequest::new(
            PROTOCOL_ID,
            PROTOCOL_VERSION,
            FIRST_REQUEST_ID,
            OPCODE_SEARCH,
        );
        outgoing.payload_len = launch.payload_len;
        outgoing.payload[..launch_payload.len()].copy_from_slice(launch_payload);
        if !send(endpoint, &outgoing) {
            write(b"Nagi Files Search client send FAIL\r\n");
            exit(1);
        }

        let mut reply = ChannelReceiveResult::default();
        if !receive(endpoint, &mut reply)
            || reply.protocol_id != PROTOCOL_ID
            || reply.version != PROTOCOL_VERSION
            || reply.request_id != FIRST_REQUEST_ID
            || reply.sender_process_id != INIT_PROCESS_ID
            || reply.flags != 0
            || reply.transfer_count != 0
        {
            write(b"Nagi Files Search client reply FAIL\r\n");
            exit(1);
        }
        if reply.opcode == OPCODE_ABORT {
            exit(0);
        }
        if reply.opcode != OPCODE_RESULTS {
            write(b"Nagi Files Search client reply FAIL\r\n");
            exit(1);
        }
        let Some(reply_payload) = payload(&reply) else {
            exit(1);
        };
        let Ok(results) = decode_results(reply_payload) else {
            write(b"Nagi Files Search client decode FAIL\r\n");
            exit(1);
        };
        let mut relay = ChannelSendRequest::new(
            PROTOCOL_ID,
            PROTOCOL_VERSION,
            FIRST_REQUEST_ID,
            OPCODE_RESULT_RELAY,
        );
        let Ok(length) = encode_results(&results, &mut relay.payload) else {
            exit(1);
        };
        relay.payload_len = length as u32;
        if !send(endpoint, &relay) {
            write(b"Nagi Files Search client result relay FAIL\r\n");
            exit(1);
        }
        exit(0)
    }

    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, _process_id: u64) -> ! {
        start_request(endpoint)
    }

    #[panic_handler]
    fn panic(_info: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
