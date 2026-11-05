//! One-shot caller-free Bar summary transport. No model, page or device access.
//! A signed package and a real launch/teardown coordinator are still required;
//! adding this ELF source does not activate it in the normal Desktop.
#![cfg_attr(target_os = "nagi", no_std)]
#![cfg_attr(target_os = "nagi", no_main)]
#[cfg(target_os = "nagi")]
#[path = "../sys.rs"]
mod sys;
#[cfg(target_os = "nagi")]
mod client {
    use crate::sys::{exit, receive, send, syscall};
    use core::panic::PanicInfo;
    use nagi_abi::{ChannelReceiveResult, SYS_HANDLE_CLOSE};
    use nagi_action_ipc::bar_summary::{decode_terminal, request_for_offer};
    fn reject(message: &ChannelReceiveResult) -> ! {
        for handle in message
            .handles
            .iter()
            .take((message.transfer_count as usize).min(message.handles.len()))
            .copied()
            .filter(|handle| *handle != 0)
        {
            let _ = syscall(SYS_HANDLE_CLOSE, handle, 0, 0);
        }
        // Kernel process teardown releases any remaining handles on failure.
        exit(1)
    }
    #[no_mangle]
    pub extern "C" fn _start(endpoint: u64, _process_id: u64) -> ! {
        let mut offer = ChannelReceiveResult::default();
        if !receive(endpoint, &mut offer) {
            exit(1);
        }
        let request = match request_for_offer(&offer) {
            Ok(r) => r,
            Err(_) => reject(&offer),
        };
        if !send(endpoint, &request) {
            exit(1);
        }
        let mut result = ChannelReceiveResult::default();
        if !receive(endpoint, &mut result) {
            exit(1);
        }
        if decode_terminal(&result, request.request_id).is_err() {
            reject(&result);
        }
        // Any valid terminal is transport completion only, never model PASS.
        exit(0)
    }
    #[panic_handler]
    fn panic(_: &PanicInfo<'_>) -> ! {
        exit(1)
    }
}
#[cfg(not(target_os = "nagi"))]
fn main() {}
