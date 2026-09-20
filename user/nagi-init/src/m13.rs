use core::{arch::asm, slice};

use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs};
use nagi_pal::{format_u64, FileSystem};

const RUST_PASS_LEN: usize = 24;
const C_PASS_LEN: usize = 23;
const SOCKET_PASS_LEN: usize = b"Nagi M13 socket/DNS PASS\r\n".len();
const RELIBC_PASS_LEN: usize = 24;
const OSS_PASS_LEN: usize = 27;
const ACCEPTANCE_LEN: usize = 26;
const MMAP_PASS_LEN: usize = b"Nagi M13 mmap PASS\r\n".len();
const TIME_PASS_LEN: usize = b"Nagi M13 time/sleep PASS\r\n".len();
const POLL_PASS_LEN: usize = b"Nagi M13 poll PASS\r\n".len();
const THREAD_PASS_LEN: usize = b"Nagi M13 thread/TLS PASS\r\n".len();
const SPAWN_PASS_LEN: usize = b"Nagi M13 native spawn PASS\r\n".len();
const VFS_FAILURE_LEN: usize = 28;
const NETWORK_FAILURE_LEN: usize = 32;
const PATH_LEN: usize = 13;
const EXPECTED_LEN: usize = 26;
const HTTP_RESPONSE_CAPACITY: usize = 1536;

#[no_mangle]
static NAGI_M13_RUST_PASS: [u8; RUST_PASS_LEN] = *b"Nagi M13 Rust PAL PASS\r\n";
#[no_mangle]
static NAGI_M13_C_PASS: [u8; C_PASS_LEN] = *b"Nagi M13 C POSIX PASS\r\n";
#[no_mangle]
static NAGI_M13_SOCKET_PASS: [u8; SOCKET_PASS_LEN] = *b"Nagi M13 socket/DNS PASS\r\n";
#[no_mangle]
static NAGI_M13_RELIBC_PASS: [u8; RELIBC_PASS_LEN] = *b"Nagi M13 relibc C PASS\r\n";
#[no_mangle]
static NAGI_M13_MMAP_PASS: [u8; MMAP_PASS_LEN] = *b"Nagi M13 mmap PASS\r\n";
#[no_mangle]
static NAGI_M13_TIME_PASS: [u8; TIME_PASS_LEN] = *b"Nagi M13 time/sleep PASS\r\n";
#[no_mangle]
static NAGI_M13_POLL_PASS: [u8; POLL_PASS_LEN] = *b"Nagi M13 poll PASS\r\n";
#[no_mangle]
static NAGI_M13_THREAD_PASS: [u8; THREAD_PASS_LEN] = *b"Nagi M13 thread/TLS PASS\r\n";
#[no_mangle]
static NAGI_M13_SPAWN_PASS: [u8; SPAWN_PASS_LEN] = *b"Nagi M13 native spawn PASS\r\n";
#[no_mangle]
static NAGI_M13_VFS_FAILURE: [u8; VFS_FAILURE_LEN] = *b"Nagi M13 Rust PAL VFS FAIL\r\n";
#[no_mangle]
static NAGI_M13_NETWORK_FAILURE: [u8; NETWORK_FAILURE_LEN] = *b"Nagi M13 Rust PAL network FAIL\r\n";
#[no_mangle]
static NAGI_M13_OSS_PASS: [u8; OSS_PASS_LEN] = *b"Nagi M13 OSS library PASS\r\n";
#[no_mangle]
static NAGI_M13_ACCEPTANCE: [u8; ACCEPTANCE_LEN] = *b"Nagi M13 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M13_FAILURE: [u8; ACCEPTANCE_LEN] = *b"Nagi M13 acceptance FAIL\r\n";
#[no_mangle]
static NAGI_M13_FILE: [u8; 17] = *b"m13-pal-check.txt";
#[no_mangle]
static NAGI_M13_PAYLOAD: [u8; 17] = *b"Nagi M13 VFS PASS";
#[no_mangle]
static NAGI_M13_PATH: [u8; PATH_LEN] = *b"/nagi-m12.txt";
#[no_mangle]
static NAGI_M13_EXPECTED: [u8; EXPECTED_LEN] = *b"NAGI_M12_HTTP_FIXTURE_PASS";

// The bootstrap process has one serialized M13 network probe. Keep its
// response storage outside the small initial user stack so the nested
// smoltcp/POSIX call frames cannot consume the caller's stack mapping.
static mut NAGI_M13_HTTP_RESPONSE: [u8; HTTP_RESPONSE_CAPACITY] = [0; HTTP_RESPONSE_CAPACITY];

unsafe extern "C" {
    fn nagi_m13_c_posix_test() -> i32;
    fn nagi_m13_relibc_test() -> i32;
}

macro_rules! static_bytes {
    ($symbol:ident, $length:expr) => {{
        let address: *const u8;
        unsafe {
            asm!(
                "lea {address}, [rip + {symbol}]",
                address = out(reg) address,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            slice::from_raw_parts(address, $length)
        }
    }};
}

type GuestVolume = Vfs<SyscallBlockDevice>;

pub fn run(
    volume: GuestVolume,
    block_capability: u64,
    net_capability: u64,
    audio_capability: u64,
) -> ! {
    #[cfg(not(feature = "m14-audio"))]
    let _ = audio_capability;
    print(b"Nagi M13 network init START\r\n");
    if unsafe { nagi_posix::nagi_posix_initialize_network(net_capability) } != 0 {
        fail();
    }
    print(b"Nagi M13 network init PASS\r\n");
    print(b"Nagi M13 Rust VFS START\r\n");
    if !run_rust_vfs(volume) {
        print(static_bytes!(NAGI_M13_VFS_FAILURE, VFS_FAILURE_LEN));
        fail();
    }
    print(b"Nagi M13 Rust VFS PASS\r\n");
    print(b"Nagi M13 HTTP START\r\n");
    if !run_rust_network() {
        print(static_bytes!(NAGI_M13_NETWORK_FAILURE, NETWORK_FAILURE_LEN));
        fail();
    }
    print(static_bytes!(NAGI_M13_RUST_PASS, RUST_PASS_LEN));

    if unsafe { nagi_posix::nagi_posix_initialize_filesystem(block_capability) } != 0 {
        fail();
    }
    if nagi_posix::errno::nagi_posix_errno_location().is_null()
        || unsafe { nagi_m13_c_posix_test() } != 0
    {
        fail();
    }
    print(static_bytes!(NAGI_M13_C_PASS, C_PASS_LEN));
    print(static_bytes!(NAGI_M13_SOCKET_PASS, SOCKET_PASS_LEN));
    print(static_bytes!(NAGI_M13_MMAP_PASS, MMAP_PASS_LEN));
    print(static_bytes!(NAGI_M13_TIME_PASS, TIME_PASS_LEN));
    print(static_bytes!(NAGI_M13_POLL_PASS, POLL_PASS_LEN));
    print(static_bytes!(NAGI_M13_THREAD_PASS, THREAD_PASS_LEN));
    print(static_bytes!(NAGI_M13_SPAWN_PASS, SPAWN_PASS_LEN));

    if unsafe { nagi_m13_relibc_test() } != 0 {
        fail();
    }
    print(static_bytes!(NAGI_M13_RELIBC_PASS, RELIBC_PASS_LEN));

    let mut number = [0_u8; 20];
    if format_u64(13, &mut number) != 2 || number[..2] != *b"13" {
        fail();
    }
    print(static_bytes!(NAGI_M13_OSS_PASS, OSS_PASS_LEN));
    #[cfg(feature = "m14-audio")]
    if !crate::m14_audio::run(audio_capability) {
        fail();
    }
    #[cfg(feature = "m15-history")]
    if !crate::m15_history::run(block_capability) {
        fail();
    }
    #[cfg(feature = "m16-package")]
    if !crate::m16_package::run(block_capability) {
        fail();
    }
    print(static_bytes!(NAGI_M13_ACCEPTANCE, ACCEPTANCE_LEN));
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

fn run_rust_vfs(volume: GuestVolume) -> bool {
    let mut filesystem = FileSystem::new(volume);
    let file_name = static_bytes!(NAGI_M13_FILE, 17);
    let handle = match filesystem.create(file_name) {
        Ok(handle) => handle,
        Err(StorageError::AlreadyExists) => match filesystem.open(file_name) {
            Ok(handle) => handle,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    if filesystem
        .write(handle, static_bytes!(NAGI_M13_PAYLOAD, 17))
        .is_err()
    {
        return false;
    }
    let mut contents = [0_u8; 64];
    let Ok(length) = filesystem.read(handle, &mut contents) else {
        return false;
    };
    if &contents[..length] != static_bytes!(NAGI_M13_PAYLOAD, 17) {
        return false;
    }

    true
}

fn run_rust_network() -> bool {
    let Some(gateway) = nagi_posix::nagi_posix_network_default_gateway() else {
        return false;
    };
    let response = unsafe { &mut *core::ptr::addr_of_mut!(NAGI_M13_HTTP_RESPONSE) };
    nagi_posix::nagi_posix_network_http_get(
        gateway,
        18_080,
        static_bytes!(NAGI_M13_PATH, PATH_LEN),
        static_bytes!(NAGI_M13_EXPECTED, EXPECTED_LEN),
        response,
    )
    .is_some()
}

fn print(bytes: &[u8]) {
    libnagi::console_write(bytes);
}

fn fail() -> ! {
    print(static_bytes!(NAGI_M13_FAILURE, ACCEPTANCE_LEN));
    libnagi::exit(1);
}
