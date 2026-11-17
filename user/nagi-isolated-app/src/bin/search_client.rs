//! Isolated `search@1` client used by the M19 Search IPC acceptance
//! (ADR 0044).
//!
//! The client sends bounded file and page queries to the init-hosted
//! SearchService over its only endpoint. It reports each decoded result set
//! back so the Supervisor can verify what crossed the process boundary. The
//! client never states who it is; the service learns that only from the
//! kernel-stamped sender Process ID and its launch record.
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
        decode_results, encode_request, encode_results, KindFilter, SearchRequest, SearchResults,
        OPCODE_RESULTS, OPCODE_SEARCH, PROTOCOL_ID, PROTOCOL_VERSION,
    };

    use crate::sys::{exit, receive, send, write};

    /// Acceptance-only report opcode; keep in sync with
    /// `user/nagi-init/src/m19_search_ipc.rs`.
    const OPCODE_CLIENT_REPORT: u16 = 0x7f01;
    const FILE_QUERY: &str = "nagi-m19-live-file.txt";
    const PAGE_QUERY: &str = "M19 persisted page";
    const BROWSER_HISTORY_QUERY: &str = "Example Domain";

    fn query(endpoint: u64, request_id: u64, kind: KindFilter, text: &str) -> SearchResults {
        let mut request =
            ChannelSendRequest::new(PROTOCOL_ID, PROTOCOL_VERSION, request_id, OPCODE_SEARCH);
        let Ok(length) = encode_request(SearchRequest { kind, text }, &mut request.payload) else {
            exit(1)
        };
        request.payload_len = length as u32;
        if !send(endpoint, &request) {
            write(b"Nagi M19 search client send FAIL\r\n");
            exit(1);
        }
        let mut reply = ChannelReceiveResult::default();
        if !receive(endpoint, &mut reply)
            || reply.protocol_id != PROTOCOL_ID
            || reply.opcode != OPCODE_RESULTS
            || reply.request_id != request_id
            || reply.payload_len as usize > MAX_CHANNEL_INLINE_PAYLOAD
        {
            write(b"Nagi M19 search client reply FAIL\r\n");
            exit(1);
        }
        let Ok(results) = decode_results(&reply.payload[..reply.payload_len as usize]) else {
            write(b"Nagi M19 search client decode FAIL\r\n");
            exit(1)
        };
        results
    }

    fn report(endpoint: u64, request_id: u64, results: &SearchResults) {
        let mut report = ChannelSendRequest::new(
            PROTOCOL_ID,
            PROTOCOL_VERSION,
            request_id,
            OPCODE_CLIENT_REPORT,
        );
        let Ok(length) = encode_results(results, &mut report.payload) else {
            exit(1)
        };
        report.payload_len = length as u32;
        if !send(endpoint, &report) {
            write(b"Nagi M19 search client report FAIL\r\n");
            exit(1);
        }
    }

    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, _process_id: u64) -> ! {
        write(b"Nagi M19 search client started\r\n");
        let file_results = query(endpoint, 1, KindFilter::File, FILE_QUERY);
        report(endpoint, 2, &file_results);
        let page_results = query(endpoint, 3, KindFilter::Page, PAGE_QUERY);
        report(endpoint, 4, &page_results);
        let browser_history_results = query(endpoint, 5, KindFilter::Page, BROWSER_HISTORY_QUERY);
        report(endpoint, 6, &browser_history_results);
        exit(0)
    }

    #[panic_handler]
    fn panic(_info: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
