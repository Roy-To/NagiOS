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

// POSIX-compatible protection bits used by the Nagi user-space mapping ABI.
// They intentionally match the standard mmap contract so relibc and Servo
// can use the same values without a host syscall translation.
pub const PROT_EXEC: u64 = 0x1;
pub const PROT_WRITE: u64 = 0x2;
pub const PROT_READ: u64 = 0x4;
pub const PROT_NONE: u64 = 0x0;

/// Nagi 0.1's initial wall-clock contract.  The reference firmware does not
/// yet pass an RTC value through BootInfo, so realtime is defined as elapsed
/// nanoseconds from the guest's Nagi epoch.  It is deliberately a guest
/// clock, never a host clock fallback.
pub const NAGI_REALTIME_EPOCH_NS: u64 = 0;

pub const BLOCK_SECTOR_SIZE: usize = 512;
pub const MAX_CONSOLE_WRITE: usize = 256;
pub const MAX_CONSOLE_READ: usize = 1;
pub const MAX_LOG_READ: usize = 512;
pub const MAX_PROCESS_NAME: usize = 16;
pub const MAX_NET_FRAME_SIZE: usize = 1536;
pub const MAX_AUDIO_BUFFER: usize = 4096;
pub const MAX_RANDOM_BYTES: usize = 256;
pub const SURFACE_WIDTH: u32 = 320;
pub const SURFACE_HEIGHT: u32 = 200;
pub const SURFACE_BYTES: usize = SURFACE_WIDTH as usize * SURFACE_HEIGHT as usize * 4;
pub const PIXEL_FORMAT_RGBA8888: u32 = 0;
pub const INPUT_EVENT_KEY: u16 = 1;
pub const INPUT_EVENT_REL: u16 = 2;
pub const INPUT_EVENT_ABS: u16 = 3;
pub const INPUT_KEY_LEFT: u16 = 0x110;
pub const INPUT_REL_X: u16 = 0;
pub const INPUT_REL_Y: u16 = 1;

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
