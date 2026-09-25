#![cfg_attr(all(target_os = "nagi", not(feature = "m13-std")), no_std)]
#![cfg_attr(target_os = "nagi", no_main)]
#![cfg_attr(all(target_os = "nagi", feature = "m13-std"), feature(restricted_std))]

#[cfg(target_os = "nagi")]
use core::arch::asm;
#[cfg(all(target_os = "nagi", not(feature = "m13-std")))]
use core::panic::PanicInfo;
#[cfg(all(target_os = "nagi", feature = "m17-servo"))]
use nagi_albert::run_first_web_pixel;

#[cfg(all(
    target_os = "nagi",
    feature = "m10-desktop",
    not(any(
        feature = "m11-security",
        feature = "m12-network",
        feature = "m13-posix"
    ))
))]
mod boot;
#[cfg(all(target_os = "nagi", feature = "m10-desktop"))]
mod desktop;
#[cfg(all(target_os = "nagi", feature = "m10-desktop"))]
mod font;
#[cfg(all(target_os = "nagi", feature = "m13-posix"))]
mod m13;
#[cfg(all(target_os = "nagi", feature = "m13-std"))]
mod m13_std;
#[cfg(all(target_os = "nagi", feature = "m14-audio"))]
mod m14_audio;
#[cfg(all(target_os = "nagi", feature = "m15-history"))]
mod m15_history;
#[cfg(all(target_os = "nagi", feature = "m16-package"))]
mod m16_package;
#[cfg(all(
    target_os = "nagi",
    feature = "m12-network",
    not(feature = "m13-posix")
))]
mod network;
#[cfg(all(target_os = "nagi", feature = "m11-security"))]
mod security;
#[cfg(all(
    target_os = "nagi",
    feature = "m8-shell",
    not(feature = "m9-window"),
    not(feature = "m10-desktop"),
    not(feature = "m11-security"),
    not(feature = "m12-network"),
    not(feature = "m13-posix")
))]
mod shell;
#[cfg(all(target_os = "nagi", feature = "m10-desktop"))]
mod ui;
#[cfg(all(target_os = "nagi", feature = "m9-window"))]
mod window;

#[cfg(target_os = "nagi")]
const MESSAGE_LEN: usize = 23;

#[cfg(target_os = "nagi")]
const FPU_STATE_FAIL_LEN: usize = 24;

#[cfg(target_os = "nagi")]
const FPU_STATE_INITIAL_PASS_LEN: usize = 32;

#[cfg(target_os = "nagi")]
const FPU_STATE_ROUND_TRIP_PASS_LEN: usize = 35;

#[cfg(target_os = "nagi")]
const M6_SUPERVISOR_START_LEN: usize = 26;

#[cfg(target_os = "nagi")]
const M6_DEPENDENCY_ORDER_PASS_LEN: usize = 40;

#[cfg(target_os = "nagi")]
const M6_SERVICE_HEALTH_PASS_LEN: usize = 29;

#[cfg(target_os = "nagi")]
const M6_SERVICE_REGISTRY_START_LEN: usize = 32;

#[cfg(target_os = "nagi")]
const M6_ECHO_CALL_PASS_LEN: usize = 26;

#[cfg(target_os = "nagi")]
const M6_ACCEPTANCE_FAIL_LEN: usize = 25;

#[cfg(target_os = "nagi")]
const ECHO_NAME_LEN: usize = 4;

#[cfg(target_os = "nagi")]
const ECHO_REQUEST_LEN: usize = 4;

#[cfg(target_os = "nagi")]
const M7_FORMAT_PASS_LEN: usize = 26;

#[cfg(target_os = "nagi")]
const M7_CREATE_PASS_LEN: usize = 26;

#[cfg(target_os = "nagi")]
const M7_WRITE_PASS_LEN: usize = 25;

#[cfg(target_os = "nagi")]
const M7_MMAP_PASS_LEN: usize = 31;

#[cfg(target_os = "nagi")]
const M7_PERSISTENT_WRITE_PASS_LEN: usize = 31;

#[cfg(target_os = "nagi")]
const M7_MOUNT_PASS_LEN: usize = 25;

#[cfg(target_os = "nagi")]
const M7_LOOKUP_PASS_LEN: usize = 31;

#[cfg(target_os = "nagi")]
const M7_READ_PASS_LEN: usize = 24;

#[cfg(target_os = "nagi")]
const M7_PERSISTENT_READ_PASS_LEN: usize = 30;

#[cfg(target_os = "nagi")]
const M7_STORAGE_FAIL_LEN: usize = 22;

#[cfg(target_os = "nagi")]
const M7_FILE_NAME_LEN: usize = 19;

#[cfg(target_os = "nagi")]
const M7_PAYLOAD_LEN: usize = 28;

#[cfg(target_os = "nagi")]
const M5_SYSCALL_PASS_LEN: usize = 22;

#[cfg(target_os = "nagi")]
const M5_ACCEPTANCE_PASS_LEN: usize = 25;

#[cfg(target_os = "nagi")]
const M6_ACCEPTANCE_PASS_LEN: usize = 25;

#[cfg(target_os = "nagi")]
const M7_ACCEPTANCE_PASS_LEN: usize = 25;

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_MESSAGE: [u8; MESSAGE_LEN] = *b"Hello from user space\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_FPU_STATE_FAIL: [u8; FPU_STATE_FAIL_LEN] = *b"Nagi M5 FPU state FAIL\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_FPU_STATE_INITIAL_PASS: [u8; FPU_STATE_INITIAL_PASS_LEN] =
    *b"Nagi M5 FPU state initial PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_FPU_STATE_ROUND_TRIP_PASS: [u8; FPU_STATE_ROUND_TRIP_PASS_LEN] =
    *b"Nagi M5 FPU state round-trip PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_SUPERVISOR_START: [u8; M6_SUPERVISOR_START_LEN] =
    *b"Nagi M6 supervisor START\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_DEPENDENCY_ORDER_PASS: [u8; M6_DEPENDENCY_ORDER_PASS_LEN] =
    *b"Nagi M6 manifest dependency order PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_SERVICE_HEALTH_PASS: [u8; M6_SERVICE_HEALTH_PASS_LEN] =
    *b"Nagi M6 service health PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_SERVICE_REGISTRY_START: [u8; M6_SERVICE_REGISTRY_START_LEN] =
    *b"Nagi M6 service registry START\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_ECHO_CALL_PASS: [u8; M6_ECHO_CALL_PASS_LEN] = *b"Nagi M6 echo@1 call PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_ACCEPTANCE_FAIL: [u8; M6_ACCEPTANCE_FAIL_LEN] = *b"Nagi M6 acceptance FAIL\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_ECHO_NAME: [u8; ECHO_NAME_LEN] = *b"echo";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_ECHO_REQUEST: [u8; ECHO_REQUEST_LEN] = *b"nagi";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_FORMAT_PASS: [u8; M7_FORMAT_PASS_LEN] = *b"Nagi M7 ext2 format PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_CREATE_PASS: [u8; M7_CREATE_PASS_LEN] = *b"Nagi M7 file create PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_WRITE_PASS: [u8; M7_WRITE_PASS_LEN] = *b"Nagi M7 file write PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_MMAP_PASS: [u8; M7_MMAP_PASS_LEN] = *b"Nagi M7 file-backed mmap PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_PERSISTENT_WRITE_PASS: [u8; M7_PERSISTENT_WRITE_PASS_LEN] =
    *b"Nagi M7 persistent write PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_MOUNT_PASS: [u8; M7_MOUNT_PASS_LEN] = *b"Nagi M7 ext2 mount PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_LOOKUP_PASS: [u8; M7_LOOKUP_PASS_LEN] = *b"Nagi M7 directory lookup PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_READ_PASS: [u8; M7_READ_PASS_LEN] = *b"Nagi M7 file read PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_PERSISTENT_READ_PASS: [u8; M7_PERSISTENT_READ_PASS_LEN] =
    *b"Nagi M7 persistent read PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_STORAGE_FAIL: [u8; M7_STORAGE_FAIL_LEN] = *b"Nagi M7 storage FAIL\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_FILE_NAME: [u8; M7_FILE_NAME_LEN] = *b"nagi-persistent.txt";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_PAYLOAD: [u8; M7_PAYLOAD_LEN] = *b"Nagi OS persistent storage\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M5_SYSCALL_PASS: [u8; M5_SYSCALL_PASS_LEN] = *b"Nagi M5 syscall PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M5_ACCEPTANCE_PASS: [u8; M5_ACCEPTANCE_PASS_LEN] = *b"Nagi M5 acceptance PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M6_ACCEPTANCE_PASS: [u8; M6_ACCEPTANCE_PASS_LEN] = *b"Nagi M6 acceptance PASS\r\n";

#[cfg(target_os = "nagi")]
#[no_mangle]
static NAGI_INIT_M7_ACCEPTANCE_PASS: [u8; M7_ACCEPTANCE_PASS_LEN] = *b"Nagi M7 acceptance PASS\r\n";

#[cfg(target_os = "nagi")]
macro_rules! static_message {
    ($symbol:ident, $length:expr) => {{
        let message: *const u8;
        unsafe {
            asm!(
                "lea {message}, [rip + {symbol}]",
                message = out(reg) message,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            core::slice::from_raw_parts(message, $length)
        }
    }};
}

#[cfg(target_os = "nagi")]
fn echo_handler(
    request: &[u8],
    response: &mut [u8],
) -> Result<usize, libnagi::service::ServiceCallError> {
    if response.len() < request.len() {
        return Err(libnagi::service::ServiceCallError::ResponseTooSmall);
    }
    let mut index = 0;
    while index < request.len() {
        unsafe {
            core::ptr::write_volatile(
                response.as_mut_ptr().add(index),
                core::ptr::read_volatile(request.as_ptr().add(index)),
            );
        }
        index += 1;
    }
    Ok(request.len())
}

#[cfg(target_os = "nagi")]
fn echo_handler_pointer() -> libnagi::service::ServiceHandler {
    let address: usize;
    unsafe {
        asm!(
            "lea {address}, [rip + {symbol}]",
            address = out(reg) address,
            symbol = sym echo_handler,
            options(nostack, preserves_flags, readonly),
        );
        core::mem::transmute(address)
    }
}

#[cfg(target_os = "nagi")]
fn run_m6_service_acceptance() -> bool {
    libnagi::console_write(static_message!(
        NAGI_INIT_M6_SUPERVISOR_START,
        M6_SUPERVISOR_START_LEN
    ));

    let echo_name = static_message!(NAGI_INIT_ECHO_NAME, ECHO_NAME_LEN);
    let Some(echo_id) = libnagi::service::ServiceId::new(echo_name, 1) else {
        return false;
    };
    let Some(echo_manifest) = libnagi::service::ServiceManifest::new(
        echo_id,
        &[],
        libnagi::service::RestartPolicy::Never,
    ) else {
        return false;
    };

    let mut supervisor = libnagi::service::Supervisor::new();
    let mut order = [libnagi::service::ServiceId::empty(); 1];
    if supervisor.start_in_dependency_order(&[echo_manifest], &mut order) != Ok(1)
        || unsafe { *order.as_ptr() } != echo_id
    {
        return false;
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_M6_DEPENDENCY_ORDER_PASS,
        M6_DEPENDENCY_ORDER_PASS_LEN
    ));
    if supervisor.mark_ready(echo_id).is_err() || supervisor.mark_healthy(echo_id).is_err() {
        return false;
    }
    if supervisor.health(echo_id) != Ok(libnagi::service::ServiceHealth::Healthy) {
        return false;
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_M6_SERVICE_HEALTH_PASS,
        M6_SERVICE_HEALTH_PASS_LEN
    ));

    libnagi::console_write(static_message!(
        NAGI_INIT_M6_SERVICE_REGISTRY_START,
        M6_SERVICE_REGISTRY_START_LEN
    ));
    let mut registry = libnagi::service::ServiceRegistry::new();
    let Ok(handle) = registry.register(echo_manifest, echo_handler_pointer()) else {
        return false;
    };
    if registry.mark_ready(handle).is_err() || registry.mark_healthy(handle).is_err() {
        return false;
    }
    let Ok(resolved) = registry.resolve(echo_id) else {
        return false;
    };
    let request = static_message!(NAGI_INIT_ECHO_REQUEST, ECHO_REQUEST_LEN);
    let mut response = [0_u8; libnagi::service::MAX_SERVICE_MESSAGE];
    let Ok(size) = registry.call(resolved, request, &mut response) else {
        return false;
    };
    if size != request.len() {
        return false;
    }
    let mut index = 0;
    while index < size {
        let response_byte = unsafe { core::ptr::read_volatile(response.as_ptr().add(index)) };
        let request_byte = unsafe { core::ptr::read_volatile(request.as_ptr().add(index)) };
        if response_byte != request_byte {
            return false;
        }
        index += 1;
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_M6_ECHO_CALL_PASS,
        M6_ECHO_CALL_PASS_LEN
    ));
    true
}

#[cfg(target_os = "nagi")]
fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        let left_byte = unsafe { core::ptr::read_volatile(left.as_ptr().add(index)) };
        let right_byte = unsafe { core::ptr::read_volatile(right.as_ptr().add(index)) };
        if left_byte != right_byte {
            return false;
        }
        index += 1;
    }
    true
}

#[cfg(target_os = "nagi")]
type GuestVolume = libnagi::storage::Vfs<libnagi::storage::SyscallBlockDevice>;

#[cfg(target_os = "nagi")]
fn run_m7_storage_acceptance(block_capability: u64) -> Option<(u64, Option<GuestVolume>)> {
    let file_name = static_message!(NAGI_INIT_M7_FILE_NAME, M7_FILE_NAME_LEN);
    let payload = static_message!(NAGI_INIT_M7_PAYLOAD, M7_PAYLOAD_LEN);
    let device = libnagi::storage::SyscallBlockDevice::new(block_capability);
    let (mut volume, formatted) = libnagi::storage::Vfs::mount_or_format(device).ok()?;

    if formatted {
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_FORMAT_PASS,
            M7_FORMAT_PASS_LEN
        ));
        let handle = volume.create(file_name).ok()?;
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_CREATE_PASS,
            M7_CREATE_PASS_LEN
        ));
        volume.write(handle, payload).ok()?;
        libnagi::console_write(static_message!(NAGI_INIT_M7_WRITE_PASS, M7_WRITE_PASS_LEN));
        let mapping = volume.mmap(handle).ok()?;
        if !bytes_equal(mapping.bytes(), payload) {
            return None;
        }
        libnagi::console_write(static_message!(NAGI_INIT_M7_MMAP_PASS, M7_MMAP_PASS_LEN));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_PERSISTENT_WRITE_PASS,
            M7_PERSISTENT_WRITE_PASS_LEN
        ));
        return Some((2, None));
    }

    libnagi::console_write(static_message!(NAGI_INIT_M7_MOUNT_PASS, M7_MOUNT_PASS_LEN));
    let mut entries = [libnagi::storage::DirectoryEntry::empty(); 8];
    let count = volume.list_root(&mut entries).ok()?;
    let mut directory_inode = 0;
    let mut index = 0;
    while index < count {
        let entry = unsafe { entries.as_ptr().add(index).read() };
        if bytes_equal(entry.name(), file_name) {
            directory_inode = entry.inode;
            break;
        }
        index += 1;
    }
    let handle = volume.open(file_name).ok()?;
    if directory_inode == 0 || directory_inode != handle.inode() {
        return None;
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_M7_LOOKUP_PASS,
        M7_LOOKUP_PASS_LEN
    ));
    let mut read_back = [0_u8; M7_PAYLOAD_LEN];
    let length = volume.read(handle, &mut read_back).ok()?;
    let read_bytes = unsafe { core::slice::from_raw_parts(read_back.as_ptr(), length) };
    if !bytes_equal(read_bytes, payload) {
        return None;
    }
    libnagi::console_write(static_message!(NAGI_INIT_M7_READ_PASS, M7_READ_PASS_LEN));
    let mapping = volume.mmap(handle).ok()?;
    if !bytes_equal(mapping.bytes(), payload) {
        return None;
    }
    libnagi::console_write(static_message!(NAGI_INIT_M7_MMAP_PASS, M7_MMAP_PASS_LEN));
    libnagi::console_write(static_message!(
        NAGI_INIT_M7_PERSISTENT_READ_PASS,
        M7_PERSISTENT_READ_PASS_LEN
    ));
    Some((0, Some(volume)))
}

#[cfg(target_os = "nagi")]
#[no_mangle]
pub extern "C" fn _start(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    net_capability: u64,
    audio_capability: u64,
) -> ! {
    #[cfg(all(feature = "m13-std", not(feature = "m17-servo")))]
    return m13_std::run(block_capability, net_capability);

    #[cfg(feature = "m17-servo")]
    {
        libnagi::console_write(b"Nagi M17 trace: user entry reached\r\n");
        let Some((exit_code, volume)) = run_m7_storage_acceptance(block_capability) else {
            libnagi::console_write(static_message!(
                NAGI_INIT_M7_STORAGE_FAIL,
                M7_STORAGE_FAIL_LEN
            ));
            libnagi::exit(1);
        };
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        drop(volume);
        libnagi::console_write(b"Nagi M17 trace: persistent storage accepted\r\n");
        return run_first_web_pixel(display_capability);
    }

    #[cfg(not(any(
        feature = "m9-window",
        feature = "m10-desktop",
        feature = "m11-security"
    )))]
    let _ = (display_capability, input_capability);
    #[cfg(feature = "m11-security")]
    let _ = (display_capability, input_capability);
    #[cfg(not(feature = "m12-network"))]
    let _ = net_capability;
    #[cfg(not(feature = "m14-audio"))]
    let _ = audio_capability;
    if !libnagi::fpu_state_is_initial() {
        libnagi::console_write(static_message!(
            NAGI_INIT_FPU_STATE_FAIL,
            FPU_STATE_FAIL_LEN
        ));
        libnagi::exit(1);
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_FPU_STATE_INITIAL_PASS,
        FPU_STATE_INITIAL_PASS_LEN
    ));
    let message: *const u8;
    unsafe {
        asm!(
            "lea {message}, [rip + {symbol}]",
            message = out(reg) message,
            symbol = sym NAGI_INIT_MESSAGE,
            options(nostack, preserves_flags, readonly),
        );
    }
    let bytes = unsafe { core::slice::from_raw_parts(message, MESSAGE_LEN) };
    libnagi::console_write(bytes);
    if !libnagi::fpu_state_is_initial() {
        libnagi::console_write(static_message!(
            NAGI_INIT_FPU_STATE_FAIL,
            FPU_STATE_FAIL_LEN
        ));
        libnagi::exit(1);
    }
    libnagi::console_write(static_message!(
        NAGI_INIT_FPU_STATE_ROUND_TRIP_PASS,
        FPU_STATE_ROUND_TRIP_PASS_LEN
    ));
    #[cfg(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    let mut boot_screen = match boot::BootScreen::new(display_capability) {
        Ok(screen) => screen,
        Err(_) => {
            libnagi::console_write(b"Nagi boot display FAIL\r\n");
            libnagi::exit(1);
        }
    };
    #[cfg(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    if boot_screen
        .present_stage(libnagi::boot::BootStage::Platform)
        .is_err()
    {
        libnagi::console_write(b"Nagi boot platform FAIL\r\n");
        libnagi::exit(1);
    }
    if !run_m6_service_acceptance() {
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_FAIL,
            M6_ACCEPTANCE_FAIL_LEN
        ));
        libnagi::exit(1);
    }
    #[cfg(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    if boot_screen
        .present_stage(libnagi::boot::BootStage::CoreServices)
        .is_err()
    {
        libnagi::console_write(b"Nagi boot core services FAIL\r\n");
        libnagi::exit(1);
    }
    let Some((exit_code, volume)) = run_m7_storage_acceptance(block_capability) else {
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_STORAGE_FAIL,
            M7_STORAGE_FAIL_LEN
        ));
        libnagi::exit(1);
    };
    #[cfg(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    if boot_screen
        .present_stage(libnagi::boot::BootStage::Storage)
        .is_err()
    {
        libnagi::console_write(b"Nagi boot storage FAIL\r\n");
        libnagi::exit(1);
    }
    #[cfg(any(
        feature = "m9-window",
        feature = "m10-desktop",
        feature = "m11-security",
        feature = "m12-network",
        all(
            not(feature = "m8-shell"),
            not(feature = "m9-window"),
            not(feature = "m10-desktop"),
            not(feature = "m11-security"),
            not(feature = "m12-network"),
            not(feature = "m13-posix")
        )
    ))]
    let _ = volume;
    #[cfg(all(feature = "m11-security", not(feature = "m12-network")))]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        security::run();
    }
    #[cfg(all(feature = "m12-network", not(feature = "m13-posix")))]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        network::run(net_capability);
    }
    #[cfg(feature = "m13-posix")]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        let Some(volume) = volume else {
            libnagi::exit(1);
        };
        m13::run(volume, block_capability, net_capability, audio_capability);
    }
    #[cfg(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        if boot_screen
            .present_stage(libnagi::boot::BootStage::Graphics)
            .and_then(|_| boot_screen.present_stage(libnagi::boot::BootStage::Session))
            .and_then(|_| boot_screen.finish_to_desktop())
            .is_err()
        {
            libnagi::console_write(b"Nagi boot transition FAIL\r\n");
            libnagi::exit(1);
        }
        desktop::run(display_capability, input_capability);
    }
    #[cfg(all(
        feature = "m9-window",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    ))]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        window::run(display_capability, input_capability);
    }

    #[cfg(all(
        feature = "m8-shell",
        not(feature = "m9-window"),
        not(feature = "m10-desktop"),
        not(feature = "m11-security"),
        not(feature = "m12-network"),
        not(feature = "m13-posix")
    ))]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_SYSCALL_PASS,
            M5_SYSCALL_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M5_ACCEPTANCE_PASS,
            M5_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M6_ACCEPTANCE_PASS,
            M6_ACCEPTANCE_PASS_LEN
        ));
        libnagi::console_write(static_message!(
            NAGI_INIT_M7_ACCEPTANCE_PASS,
            M7_ACCEPTANCE_PASS_LEN
        ));
        let Some(mut volume) = volume else {
            libnagi::exit(1);
        };
        shell::run(&mut volume);
    }

    #[cfg(not(any(
        feature = "m8-shell",
        feature = "m9-window",
        feature = "m10-desktop",
        feature = "m11-security",
        feature = "m12-network",
        feature = "m13-posix"
    )))]
    libnagi::exit(exit_code)
}

#[cfg(all(target_os = "nagi", not(feature = "m13-std")))]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    libnagi::exit(1)
}

#[cfg(not(target_os = "nagi"))]
fn main() {}
