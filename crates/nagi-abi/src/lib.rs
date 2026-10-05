#![no_std]

pub const SYS_CONSOLE_WRITE: u64 = 1;
pub const SYS_PROCESS_EXIT: u64 = 2;
pub const SYS_BLOCK_READ: u64 = 3;
pub const SYS_BLOCK_WRITE: u64 = 4;
pub const SYS_CONSOLE_READ: u64 = 5;
pub const SYS_PROCESS_INFO: u64 = 6;
pub const SYS_MEMORY_INFO: u64 = 7;
pub const SYS_LOG_READ: u64 = 8;
pub const SYS_DISPLAY_INFO: u64 = 9;
pub const SYS_DISPLAY_PRESENT: u64 = 10;
pub const SYS_INPUT_READ: u64 = 11;
pub const SYS_NET_SEND: u64 = 12;
pub const SYS_NET_RECEIVE: u64 = 13;
pub const SYS_TIME_READ: u64 = 14;
pub const SYS_TIME_REALTIME: u64 = 15;
pub const SYS_THREAD_SLEEP: u64 = 16;
pub const SYS_MEMORY_MAP: u64 = 17;
pub const SYS_MEMORY_UNMAP: u64 = 18;
pub const SYS_MEMORY_PROTECT: u64 = 19;
pub const SYS_THREAD_CREATE: u64 = 20;
pub const SYS_THREAD_JOIN: u64 = 21;
pub const SYS_THREAD_EXIT: u64 = 22;
pub const SYS_THREAD_SELF: u64 = 23;
pub const SYS_AUDIO_PLAY: u64 = 24;
pub const SYS_AUDIO_CAPTURE: u64 = 25;
pub const SYS_RANDOM_GET: u64 = 26;
pub const SYS_MEMORY_MAP_AT: u64 = 27;
pub const SYS_BLOCK_FLUSH: u64 = 28;
pub const SYS_THREAD_DETACH: u64 = 29;
/// Request kernel confirmation of the current A/B boot candidate after the
/// guest readiness gate has completed. The kernel derives all coordinates.
pub const SYS_BOOT_READY: u64 = 30;
/// Create a bounded pair of user-visible bootstrap Channel endpoint handles.
pub const SYS_CHANNEL_CREATE: u64 = 31;
/// Send one bounded message over a Channel endpoint.
pub const SYS_CHANNEL_SEND: u64 = 32;
/// Receive one message without waiting for Channel readability.
pub const SYS_CHANNEL_TRY_RECEIVE: u64 = 33;
/// Close a bootstrap handle returned by the Channel ABI.
pub const SYS_HANDLE_CLOSE: u64 = 34;
/// Block until a Channel endpoint becomes readable; the wake may be spurious
/// if another receiver consumes the queued message first.
pub const SYS_CHANNEL_WAIT_READABLE: u64 = 35;
/// Spawn the single isolated child process from an ELF image in the caller's
/// memory and move one Channel endpoint into it. Only the bootstrap init
/// process (the Supervisor) may call it (ADR 0043). Returns the child's
/// kernel Process ID.
pub const SYS_PROCESS_SPAWN: u64 = 36;
/// Wait for an isolated process spawned by the caller (init only) to exit
/// and consume its `ProcessExitStatus` (ADR 0048). Returns 0 with the
/// status written, `PROCESS_WAIT_RETRY` after a wake (call again), or
/// failure for an unknown or already-consumed Process ID.
pub const SYS_PROCESS_WAIT: u64 = 37;
/// Claim the inactive-system-slot update capability (ADR 0055). Only init
/// may call it, only once per boot, and only on a confirmed-slot boot with
/// no pending trial; it writes an `UpdateSlotInfo` and returns 0.
pub const SYS_UPDATE_SLOT_CLAIM: u64 = 38;
/// Ask the loader to trial the inactive slot on the next boot. Requires the
/// claimed update capability; one-shot per boot.
pub const SYS_UPDATE_SLOT_STAGE: u64 = 39;
pub const PROCESS_WAIT_RETRY: u64 = 1;
pub const PROCESS_EXIT_KIND_EXITED: u32 = 1;
pub const PROCESS_EXIT_KIND_FAULTED: u32 = 2;

/// Optional `SYS_THREAD_CREATE` flag for a child that should be detached
/// before it can be scheduled.
pub const THREAD_CREATE_DETACHED: u64 = 1;

/// Capacity of the bounded, cooperative bootstrap thread pool. ID zero is the
/// initial user-init thread; all remaining IDs are reusable child slots. M18
/// enables a larger pool for Servo's additional browser worker threads while
/// preserving the accepted M17 capacity.
#[cfg(feature = "m18-browser-threads")]
pub const BOOTSTRAP_USER_THREAD_COUNT: usize = 64;
#[cfg(not(feature = "m18-browser-threads"))]
pub const BOOTSTRAP_USER_THREAD_COUNT: usize = 32;

/// Page granularity for M17 bootstrap user-thread stack mappings.
pub const BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE: usize = 4096;
/// Smallest stack accepted by the M17 bootstrap thread ABI.
pub const BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE: usize = BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE;
/// Default stack for M17 bootstrap threads without an explicit request.
pub const BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE: usize = 2 * 1024 * 1024;
/// Largest individual stack accepted by the M17 bootstrap ABI. The pinned
/// Servo ScriptThread explicitly requests this 8 MiB stack.
pub const BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE: usize = 8 * 1024 * 1024;

/// Round an M17 bootstrap thread stack request to guest page granularity.
/// A request of zero selects the unchanged 2 MiB default.
pub fn round_bootstrap_user_thread_stack_size(requested: usize) -> Option<usize> {
    let size = if requested == 0 {
        BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE
    } else {
        requested
    };
    if !(BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE..=BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE)
        .contains(&size)
    {
        return None;
    }
    let rounded = size.checked_add(BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE - 1)?
        & !(BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE - 1);
    (rounded <= BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE).then_some(rounded)
}

/// Validate the page-aligned size passed to `SYS_THREAD_CREATE`.
// Keep this const-compatible with pinned nightly-2025-08-01, where
// `usize::is_multiple_of` is still an unstable const API.
#[allow(clippy::manual_is_multiple_of)]
pub const fn is_valid_bootstrap_user_thread_stack_size(size: usize) -> bool {
    size >= BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE
        && size <= BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE
        && size % BOOTSTRAP_USER_THREAD_STACK_PAGE_SIZE == 0
}

// POSIX-compatible protection bits used by the Nagi user-space mapping ABI.
// They intentionally match the standard mmap contract so relibc and Servo
// can use the same values without a host syscall translation.
pub const PROT_EXEC: u64 = 0x1;
pub const PROT_WRITE: u64 = 0x2;
pub const PROT_READ: u64 = 0x4;
pub const PROT_NONE: u64 = 0x0;

/// Convert a validated firmware RTC seed and elapsed guest timer ticks into
/// Unix nanoseconds. The unavailable sentinel and arithmetic overflow fail
/// closed; this never falls back to the host clock.
pub const fn realtime_ns_at_ticks(epoch_ns: u64, ticks: u64) -> u64 {
    if epoch_ns == u64::MAX {
        return u64::MAX;
    }
    let Some(elapsed_ns) = ticks.checked_mul(10_000_000) else {
        return u64::MAX;
    };
    match epoch_ns.checked_add(elapsed_ns) {
        Some(realtime_ns) => realtime_ns,
        None => u64::MAX,
    }
}

pub const BLOCK_SECTOR_SIZE: usize = 512;
pub const MAX_CONSOLE_WRITE: usize = 256;
pub const MAX_CONSOLE_READ: usize = 1;
pub const MAX_LOG_READ: usize = 512;
pub const MAX_PROCESS_NAME: usize = 16;
pub const MAX_NET_FRAME_SIZE: usize = 1536;
pub const MAX_AUDIO_BUFFER: usize = 4096;
pub const MAX_RANDOM_BYTES: usize = 256;
pub const RIGHT_READ: u32 = 1 << 0;
pub const RIGHT_WRITE: u32 = 1 << 1;
pub const RIGHT_MAP: u32 = 1 << 2;
pub const RIGHT_TRANSFER: u32 = 1 << 3;
pub const RIGHT_CONTROL: u32 = 1 << 4;
pub const RIGHT_DUPLICATE: u32 = 1 << 5;
pub const RIGHT_WAIT: u32 = 1 << 6;
pub const RIGHT_SIGNAL: u32 = 1 << 7;
pub const RIGHT_EXECUTE: u32 = 1 << 8;
pub const SURFACE_WIDTH: u32 = 320;
pub const SURFACE_HEIGHT: u32 = 200;
pub const SURFACE_BYTES: usize = SURFACE_WIDTH as usize * SURFACE_HEIGHT as usize * 4;
pub const PIXEL_FORMAT_RGBA8888: u32 = 0;
pub const INPUT_EVENT_KEY: u16 = 1;
pub const INPUT_EVENT_REL: u16 = 2;
pub const INPUT_EVENT_ABS: u16 = 3;
/// Virtio/Linux input event code for the primary pointer button.
pub const INPUT_BUTTON_PRIMARY: u16 = 0x110;
/// Compatibility name used by the earlier window acceptance path.
pub const INPUT_KEY_LEFT: u16 = INPUT_BUTTON_PRIMARY;
/// Virtio/Linux input event codes for keyboard keys.
pub const INPUT_KEY_ESCAPE: u16 = 1;
pub const INPUT_KEY_TAB: u16 = 15;
pub const INPUT_KEY_ENTER: u16 = 28;
pub const INPUT_KEY_SPACE: u16 = 57;
pub const INPUT_KEY_UP: u16 = 103;
pub const INPUT_KEY_DOWN: u16 = 108;
pub const INPUT_REL_X: u16 = 0;
pub const INPUT_REL_Y: u16 = 1;

#[cfg(test)]
mod tests {
    use super::{
        is_valid_bootstrap_user_thread_stack_size, realtime_ns_at_ticks,
        round_bootstrap_user_thread_stack_size, BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE,
        BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE, BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE,
    };

    #[test]
    fn bootstrap_thread_stack_rounding_preserves_default_and_servo_limit() {
        assert_eq!(BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE, 2 * 1024 * 1024);
        assert_eq!(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE, 8 * 1024 * 1024);
        assert_eq!(
            round_bootstrap_user_thread_stack_size(0),
            Some(2 * 1024 * 1024)
        );
        assert_eq!(
            round_bootstrap_user_thread_stack_size(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE),
            Some(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE)
        );
        assert_eq!(
            round_bootstrap_user_thread_stack_size(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE - 1),
            Some(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE)
        );
        assert_eq!(
            round_bootstrap_user_thread_stack_size(BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE + 1),
            None
        );
        assert_eq!(round_bootstrap_user_thread_stack_size(usize::MAX), None);
        assert_eq!(round_bootstrap_user_thread_stack_size(1), None);
    }

    #[test]
    fn bootstrap_thread_capacity_matches_the_selected_milestone_contract() {
        #[cfg(feature = "m18-browser-threads")]
        assert_eq!(super::BOOTSTRAP_USER_THREAD_COUNT, 64);
        #[cfg(not(feature = "m18-browser-threads"))]
        assert_eq!(super::BOOTSTRAP_USER_THREAD_COUNT, 32);
    }

    #[test]
    fn kernel_thread_stack_contract_requires_aligned_bounded_sizes() {
        assert!(is_valid_bootstrap_user_thread_stack_size(
            BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE
        ));
        assert!(is_valid_bootstrap_user_thread_stack_size(
            BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE
        ));
        assert!(is_valid_bootstrap_user_thread_stack_size(
            BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE
        ));
        assert!(!is_valid_bootstrap_user_thread_stack_size(0));
        assert!(!is_valid_bootstrap_user_thread_stack_size(
            BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE + 4096
        ));
        assert!(!is_valid_bootstrap_user_thread_stack_size(
            BOOTSTRAP_USER_THREAD_STACK_MAX_SIZE - 1
        ));
    }

    #[test]
    fn realtime_clock_preserves_unavailable_and_fails_closed_on_overflow() {
        assert_eq!(realtime_ns_at_ticks(u64::MAX, 10), u64::MAX);
        assert_eq!(realtime_ns_at_ticks(1_000, 3), 30_001_000);
        assert_eq!(realtime_ns_at_ticks(u64::MAX - 5, 1), u64::MAX);
        assert_eq!(realtime_ns_at_ticks(0, u64::MAX), u64::MAX);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessInfo {
    pub pid: u64,
    pub parent_pid: u64,
    pub state: u32,
    pub flags: u32,
    pub image_pages: u32,
    pub stack_pages: u32,
    pub name: [u8; MAX_PROCESS_NAME],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryInfo {
    pub image_pages: u64,
    pub stack_pages: u64,
    pub tls_pages: u64,
    pub image_base: u64,
    pub image_limit: u64,
    pub stack_base: u64,
    pub stack_limit: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisplayInfo {
    pub surface_address: u64,
    pub surface_bytes: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

/// Two handles created by `SYS_CHANNEL_CREATE`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChannelEndpoints {
    pub endpoint_a: u64,
    pub endpoint_b: u64,
}

/// Request for `SYS_PROCESS_SPAWN`. The child starts with the moved
/// endpoint's child-local handle in `rdi` and its Process ID in `rsi`.
/// `endpoint_rights` must be a subset of the caller's rights for `endpoint`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessSpawnRequest {
    pub image_address: u64,
    pub image_len: u64,
    pub endpoint: u64,
    pub endpoint_rights: u32,
    pub reserved: u32,
}

/// Exit status of an isolated process, written by `SYS_PROCESS_WAIT`.
/// `fault_vector` is meaningful only for `PROCESS_EXIT_KIND_FAULTED`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessExitStatus {
    pub process_id: u32,
    pub kind: u32,
    pub code: u64,
    pub fault_vector: u64,
}

/// The inactive system slot an installer may write (ADR 0055). The
/// capability reads, writes and flushes only that partition, with
/// partition-relative sectors.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UpdateSlotInfo {
    pub capability: u64,
    pub sector_count: u64,
    /// 0 = System A, 1 = System B.
    pub slot: u8,
    pub reserved: [u8; 7],
}

/// One handle moved with an attenuated rights set in a Channel message.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ChannelHandleTransfer {
    pub handle: u64,
    pub rights: u32,
    pub reserved: u32,
}

/// Fixed-size send request. The sender identity is deliberately absent; the
/// kernel records it in receive metadata from the current Process object.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelSendRequest {
    pub protocol_id: u16,
    pub version: u16,
    pub opcode: u16,
    pub flags: u16,
    pub request_id: u64,
    pub payload_len: u32,
    pub transfer_count: u32,
    pub payload: [u8; MAX_CHANNEL_INLINE_PAYLOAD],
    pub transfers: [ChannelHandleTransfer; MAX_CHANNEL_TRANSFER_HANDLES],
}

impl ChannelSendRequest {
    pub const fn new(protocol_id: u16, version: u16, request_id: u64, opcode: u16) -> Self {
        Self {
            protocol_id,
            version,
            opcode,
            flags: 0,
            request_id,
            payload_len: 0,
            transfer_count: 0,
            payload: [0; MAX_CHANNEL_INLINE_PAYLOAD],
            transfers: [ChannelHandleTransfer {
                handle: 0,
                rights: 0,
                reserved: 0,
            }; MAX_CHANNEL_TRANSFER_HANDLES],
        }
    }
}

impl Default for ChannelSendRequest {
    fn default() -> Self {
        Self::new(0, 0, 0, 0)
    }
}

/// Result written by `SYS_CHANNEL_TRY_RECEIVE`. Unused payload and transfer
/// slots are zeroed. `sender_process_id` is kernel selected metadata.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelReceiveResult {
    pub sender_process_id: u32,
    pub reserved: u32,
    pub protocol_id: u16,
    pub version: u16,
    pub opcode: u16,
    pub flags: u16,
    pub request_id: u64,
    pub payload_len: u32,
    pub transfer_count: u32,
    pub payload: [u8; MAX_CHANNEL_INLINE_PAYLOAD],
    pub handles: [u64; MAX_CHANNEL_TRANSFER_HANDLES],
}

impl Default for ChannelReceiveResult {
    fn default() -> Self {
        Self {
            sender_process_id: 0,
            reserved: 0,
            protocol_id: 0,
            version: 0,
            opcode: 0,
            flags: 0,
            request_id: 0,
            payload_len: 0,
            transfer_count: 0,
            payload: [0; MAX_CHANNEL_INLINE_PAYLOAD],
            handles: [0; MAX_CHANNEL_TRANSFER_HANDLES],
        }
    }
}

pub const MAX_CHANNEL_INLINE_PAYLOAD: usize = 128;
pub const MAX_CHANNEL_TRANSFER_HANDLES: usize = 4;
pub const MAX_CHANNEL_QUEUE_MESSAGES: usize = 8;

#[cfg(test)]
mod channel_abi_tests {
    use core::mem::{align_of, size_of};

    use super::{
        ChannelEndpoints, ChannelHandleTransfer, ChannelReceiveResult, ChannelSendRequest,
        MAX_CHANNEL_INLINE_PAYLOAD, MAX_CHANNEL_QUEUE_MESSAGES, MAX_CHANNEL_TRANSFER_HANDLES,
        SYS_CHANNEL_CREATE, SYS_CHANNEL_SEND, SYS_CHANNEL_TRY_RECEIVE, SYS_CHANNEL_WAIT_READABLE,
        SYS_HANDLE_CLOSE,
    };

    #[test]
    fn bootstrap_channel_syscall_numbers_and_layout_are_stable() {
        assert_eq!(SYS_CHANNEL_CREATE, 31);
        assert_eq!(SYS_CHANNEL_SEND, 32);
        assert_eq!(SYS_CHANNEL_TRY_RECEIVE, 33);
        assert_eq!(SYS_HANDLE_CLOSE, 34);
        assert_eq!(SYS_CHANNEL_WAIT_READABLE, 35);
        assert_eq!(MAX_CHANNEL_INLINE_PAYLOAD, 128);
        assert_eq!(MAX_CHANNEL_TRANSFER_HANDLES, 4);
        assert_eq!(MAX_CHANNEL_QUEUE_MESSAGES, 8);
        assert_eq!(size_of::<ChannelEndpoints>(), 16);
        assert_eq!(align_of::<ChannelEndpoints>(), 8);
        assert_eq!(size_of::<ChannelHandleTransfer>(), 16);
        assert_eq!(size_of::<ChannelSendRequest>(), 216);
        assert_eq!(size_of::<ChannelReceiveResult>(), 192);
    }
}
