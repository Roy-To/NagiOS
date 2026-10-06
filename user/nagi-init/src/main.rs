#![cfg_attr(all(target_os = "nagi", not(feature = "m13-std")), no_std)]
#![cfg_attr(target_os = "nagi", no_main)]
#![cfg_attr(all(target_os = "nagi", feature = "m13-std"), feature(restricted_std))]

#[cfg(any(
    feature = "m16-package",
    feature = "m19-search",
    feature = "m27-recovery",
    feature = "m25-whisper-inference-acceptance"
))]
extern crate alloc;

#[cfg(all(
    target_os = "nagi",
    not(feature = "m17-servo"),
    any(
        feature = "m16-package",
        feature = "m19-search",
        feature = "m20-llama-inference-acceptance",
        feature = "m27-recovery",
        feature = "m25-whisper-inference-acceptance"
    )
))]
struct GuestAllocator;

#[cfg(all(
    target_os = "nagi",
    not(feature = "m17-servo"),
    any(
        feature = "m16-package",
        feature = "m19-search",
        feature = "m20-llama-inference-acceptance",
        feature = "m27-recovery",
        feature = "m25-whisper-inference-acceptance"
    )
))]
unsafe impl core::alloc::GlobalAlloc for GuestAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }
        nagi_posix::nagi_posix_malloc_aligned(
            layout.size(),
            layout.align().max(core::mem::size_of::<usize>()),
        )
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: core::alloc::Layout) {
        if layout.size() != 0 {
            nagi_posix::nagi_posix_free(pointer);
        }
    }
}

#[cfg(all(
    target_os = "nagi",
    not(feature = "m17-servo"),
    any(
        feature = "m16-package",
        feature = "m19-search",
        feature = "m27-recovery",
        feature = "m25-whisper-inference-acceptance"
    )
))]
#[global_allocator]
static GUEST_ALLOCATOR: GuestAllocator = GuestAllocator;

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
use core::arch::asm;
#[cfg(all(target_os = "nagi", not(feature = "m13-std")))]
use core::panic::PanicInfo;
#[cfg(all(
    target_os = "nagi",
    feature = "m17-servo",
    not(feature = "m18-acceptance")
))]
use nagi_albert::run_first_web_pixel;
#[cfg(all(target_os = "nagi", feature = "m18-acceptance"))]
use nagi_albert::run_m18_https_acceptance;

/// Pinned system fonts embedded by build.rs (ADR 0062).
#[cfg(all(target_os = "nagi", feature = "m17-servo"))]
mod system_fonts {
    include!(concat!(env!("OUT_DIR"), "/system_fonts.rs"));
}

#[cfg(all(target_os = "nagi", feature = "m20-llama-link-smoke"))]
unsafe extern "C" {
    fn nagi_m20_llama_backend_init_smoke() -> i32;
}

#[cfg(all(target_os = "nagi", feature = "m21-action-ipc"))]
mod action_ipc;
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
#[cfg(all(target_os = "nagi", feature = "consent-dialog-acceptance"))]
mod consent_dialog;
#[cfg(all(target_os = "nagi", feature = "m10-desktop"))]
mod desktop;
#[cfg(all(target_os = "nagi", feature = "m10-desktop"))]
mod font;
#[cfg(all(target_os = "nagi", feature = "isolated-process-acceptance"))]
mod isolated_process;
#[cfg(all(target_os = "nagi", feature = "desktop-login"))]
mod login_screen;
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
#[cfg(all(target_os = "nagi", feature = "m19-search"))]
mod m19_search;
#[cfg(all(target_os = "nagi", feature = "m20-llama-inference-acceptance"))]
mod m20_granite;
#[cfg(feature = "m20-fixture-acceptance")]
#[path = "../../../tests/fixtures/m20_model_store_reader.rs"]
mod m20_model_store_fixture;
#[cfg(all(target_os = "nagi", feature = "m22-history"))]
mod m22_history;
#[cfg(all(target_os = "nagi", feature = "m25-voice-acceptance"))]
mod m25_voice;
#[cfg(all(target_os = "nagi", feature = "m25-whisper-inference-acceptance"))]
mod m25_whisper;
#[cfg(all(
    target_os = "nagi",
    feature = "m12-network",
    not(feature = "m13-posix")
))]
mod network;
#[cfg(all(target_os = "nagi", feature = "m27-recovery"))]
mod recovery;
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
#[cfg(all(
    target_os = "nagi",
    any(
        feature = "isolated-process-acceptance",
        feature = "m19-search-ipc",
        feature = "consent-dialog-acceptance"
    )
))]
// The consent dialog acceptance uses only the launch and consent paths.
#[cfg_attr(
    not(any(feature = "isolated-process-acceptance", feature = "m19-search-ipc")),
    allow(dead_code)
)]
mod supervisor;
#[cfg(all(target_os = "nagi", feature = "m30-update-install"))]
mod system_update;
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
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

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
type GuestVolume = libnagi::storage::Vfs<libnagi::storage::SyscallBlockDevice>;

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
fn run_m7_storage_acceptance(block_capability: u64) -> Option<(u64, Option<GuestVolume>)> {
    let file_name = static_message!(NAGI_INIT_M7_FILE_NAME, M7_FILE_NAME_LEN);
    let payload = static_message!(NAGI_INIT_M7_PAYLOAD, M7_PAYLOAD_LEN);
    #[cfg(feature = "m27-ro-vfs-check")]
    {
        let mut device = libnagi::storage::SyscallBlockDevice::new(block_capability);
        let report = libnagi::storage::Vfs::<libnagi::storage::SyscallBlockDevice>::check_existing(
            &mut device,
        )
        .ok()?;
        if report.regular_files == 0
            || report.directories == 0
            || report.allocated_data_blocks == 0
            || report.directory_entries == 0
        {
            return None;
        }
        libnagi::console_write(b"Nagi M27 read-only VFS check PASS\r\n");
    }
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
    // The ext2 fixture has 64 inodes and M19/M22 add guest-side state files.
    // Keep the M7 persistence lookup bounded by that filesystem limit rather
    // than assuming the root directory still contains at most eight entries.
    let mut entries = [libnagi::storage::DirectoryEntry::empty(); 64];
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

#[cfg(all(
    target_os = "nagi",
    feature = "m20-model-store-acceptance",
    not(feature = "m27-recovery")
))]
struct SyscallModelStoreReader(u64);

#[cfg(all(
    target_os = "nagi",
    feature = "m20-model-store-acceptance",
    not(feature = "m27-recovery")
))]
impl nagi_model_manager::ModelStoreSectorReader for SyscallModelStoreReader {
    fn read_sector(
        &mut self,
        partition_relative_sector: u64,
        destination: &mut [u8; nagi_model_manager::FAT32_SECTOR_SIZE],
    ) -> Result<(), nagi_model_manager::ArtifactReadError> {
        if libnagi::block_read(self.0, partition_relative_sector, destination) {
            Ok(())
        } else {
            Err(nagi_model_manager::ArtifactReadError::Unavailable)
        }
    }
}

#[cfg(all(
    target_os = "nagi",
    feature = "m20-model-store-acceptance",
    not(feature = "m27-recovery")
))]
fn run_m20_model_store_capability_acceptance(model_store_capability: u64) -> bool {
    if model_store_capability == 0 {
        return false;
    }
    let mut boot_sector = [0u8; libnagi::BLOCK_SECTOR_SIZE];
    if !libnagi::block_read(model_store_capability, 0, &mut boot_sector)
        || boot_sector[11..13] != [0, 2]
        || &boot_sector[82..87] != b"FAT32"
        || boot_sector[510..512] != [0x55, 0xaa]
    {
        return false;
    }
    if libnagi::block_write(model_store_capability, 0, &boot_sector) {
        return false;
    }
    let mut after_rejected_write = [0u8; libnagi::BLOCK_SECTOR_SIZE];
    if !libnagi::block_read(model_store_capability, 0, &mut after_rejected_write)
        || !bytes_equal(&boot_sector, &after_rejected_write)
    {
        return false;
    }
    let artifact_id = match nagi_model_manager::ArtifactId::new("ibm.granite-4.2-3b") {
        Ok(artifact_id) => artifact_id,
        Err(_) => return false,
    };
    // ADR-0013 fixes the reference Model Store at 32 GiB, or 67,108,864
    // 512-byte sectors. The kernel still enforces the exact GPT extent on
    // every read.
    const M30_MODEL_STORE_SECTORS: u64 = 67_108_864;
    let artifact = nagi_model_manager::Fat32ArtifactReader::open(
        SyscallModelStoreReader(model_store_capability),
        M30_MODEL_STORE_SECTORS,
        artifact_id,
    );
    match artifact {
        Ok(mut artifact) => {
            let mut header = [0u8; 4];
            if nagi_model_manager::ModelArtifactReader::read_at(&mut artifact, 0, &mut header)
                != Ok(4)
                || &header != b"GGUF"
            {
                return false;
            }
            #[cfg(feature = "m20-granite-artifact-acceptance")]
            {
                let manifest = match nagi_model_manager::ModelManifest::parse_json(include_bytes!(
                    "../../nagi-model-manager/tests/fixtures/granite-4.2-3b.json"
                )) {
                    Ok(manifest) => manifest,
                    Err(_) => return false,
                };
                if manifest.artifact.size_bytes
                    != Some(nagi_model_manager::ModelArtifactReader::len(&artifact))
                {
                    return false;
                }
                let Some(integrity) = manifest.artifact.integrity.as_ref() else {
                    return false;
                };
                if nagi_model_manager::verify_model_artifact_integrity(&mut artifact, integrity)
                    .is_err()
                {
                    return false;
                }
                let marker = b"Nagi M20 Granite artifact digest PASS\r\n";
                if libnagi::console_write(marker) != marker.len() {
                    return false;
                }
            }
        }
        Err(nagi_model_manager::Fat32ArtifactError::ArtifactNotFound) => {
            #[cfg(feature = "m20-granite-artifact-acceptance")]
            return false;
        }
        Err(_) => return false,
    }
    #[cfg(feature = "m25-whisper-artifact-acceptance")]
    if !run_m25_whisper_artifact_acceptance(model_store_capability) {
        return false;
    }
    #[cfg(feature = "m26-qwen-artifact-acceptance")]
    if !run_m26_model_store_artifact_acceptance(
        model_store_capability,
        "qwen.qwen3-4b",
        2_497_280_256,
        "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
        b"Nagi M26 Qwen artifact digest PASS\r\n",
    ) {
        return false;
    }
    #[cfg(feature = "m26-gemma-artifact-acceptance")]
    if !run_m26_model_store_artifact_acceptance(
        model_store_capability,
        "google.gemma-3-1b",
        806_058_240,
        "8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135",
        b"Nagi M26 Gemma artifact digest PASS\r\n",
    ) {
        return false;
    }
    #[cfg(feature = "m20-fixture-acceptance")]
    {
        let fixture_id =
            match nagi_model_manager::ArtifactId::new(m20_model_store_fixture::ARTIFACT_ID) {
                Ok(artifact_id) => artifact_id,
                Err(_) => return false,
            };
        let mut fixture = match nagi_model_manager::Fat32ArtifactReader::open(
            SyscallModelStoreReader(model_store_capability),
            M30_MODEL_STORE_SECTORS,
            fixture_id,
        ) {
            Ok(fixture) => fixture,
            Err(_) => return false,
        };
        let fixture_len = nagi_model_manager::ModelArtifactReader::len(&fixture);
        if fixture_len != m20_model_store_fixture::FIXTURE_LEN as u64 {
            return false;
        }
        let mut offset = 0u64;
        let mut chunk = [0u8; 512];
        while offset < fixture_len {
            let expected_len = (fixture_len - offset).min(chunk.len() as u64) as usize;
            if nagi_model_manager::ModelArtifactReader::read_at(
                &mut fixture,
                offset,
                &mut chunk[..expected_len],
            ) != Ok(expected_len)
            {
                return false;
            }
            for (index, actual) in chunk[..expected_len].iter().copied().enumerate() {
                if actual != m20_model_store_fixture::fixture_byte_at(offset as usize + index) {
                    return false;
                }
            }
            offset += expected_len as u64;
        }

        let mut boundary = [0u8; 64];
        if nagi_model_manager::ModelArtifactReader::read_at(&mut fixture, 4_075, &mut boundary)
            != Ok(boundary.len())
        {
            return false;
        }
        for (index, actual) in boundary.iter().copied().enumerate() {
            if actual != m20_model_store_fixture::fixture_byte_at(4_075 + index) {
                return false;
            }
        }
        let mut eof = [0u8; 1];
        if nagi_model_manager::ModelArtifactReader::read_at(&mut fixture, fixture_len, &mut eof)
            != Ok(0)
        {
            return false;
        }
    }
    if libnagi::console_write(b"Nagi M20 Model Store capability PASS\r\n")
        != b"Nagi M20 Model Store capability PASS\r\n".len()
    {
        return false;
    }
    #[cfg(feature = "m20-fixture-acceptance")]
    {
        return libnagi::console_write(b"Nagi M20 FAT32 fixture read PASS\r\n")
            == b"Nagi M20 FAT32 fixture read PASS\r\n".len();
    }
    #[cfg(not(feature = "m20-fixture-acceptance"))]
    true
}

#[cfg(all(
    target_os = "nagi",
    feature = "m25-whisper-artifact-acceptance",
    not(feature = "m27-recovery")
))]
fn run_m25_whisper_artifact_acceptance(model_store_capability: u64) -> bool {
    const MODEL_STORE_SECTORS: u64 = 67_108_864;
    const EXPECTED_SIZE: u64 = 487_601_967;
    const EXPECTED_SHA256: &str =
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";
    let artifact_id = match nagi_model_manager::ArtifactId::new("openai.whisper-small-multilingual")
    {
        Ok(artifact_id) => artifact_id,
        Err(_) => return false,
    };
    let mut artifact = match nagi_model_manager::Fat32ArtifactReader::open(
        SyscallModelStoreReader(model_store_capability),
        MODEL_STORE_SECTORS,
        artifact_id,
    ) {
        Ok(artifact) => artifact,
        Err(_) => return false,
    };
    if nagi_model_manager::ModelArtifactReader::len(&artifact) != EXPECTED_SIZE {
        return false;
    }
    // GGML_FILE_MAGIC is 0x67676d6c; the pinned little-endian artifact starts
    // with the bytes `lmgg`.
    let mut magic = [0u8; 4];
    if nagi_model_manager::ModelArtifactReader::read_at(&mut artifact, 0, &mut magic) != Ok(4)
        || &magic != b"lmgg"
    {
        return false;
    }
    let integrity = nagi_model_manager::IntegrityMetadata {
        algorithm: "sha256".into(),
        digest: EXPECTED_SHA256.into(),
    };
    if nagi_model_manager::verify_model_artifact_integrity(&mut artifact, &integrity).is_err() {
        return false;
    }
    let marker = b"Nagi M25 Whisper artifact digest PASS\r\n";
    libnagi::console_write(marker) == marker.len()
}

#[cfg(all(
    target_os = "nagi",
    any(
        feature = "m26-qwen-artifact-acceptance",
        feature = "m26-gemma-artifact-acceptance"
    ),
    not(feature = "m27-recovery")
))]
fn run_m26_model_store_artifact_acceptance(
    model_store_capability: u64,
    artifact_id: &str,
    expected_size: u64,
    expected_sha256: &str,
    marker: &[u8],
) -> bool {
    const MODEL_STORE_SECTORS: u64 = 67_108_864;
    let artifact_id = match nagi_model_manager::ArtifactId::new(artifact_id) {
        Ok(artifact_id) => artifact_id,
        Err(_) => return false,
    };
    let mut artifact = match nagi_model_manager::Fat32ArtifactReader::open(
        SyscallModelStoreReader(model_store_capability),
        MODEL_STORE_SECTORS,
        artifact_id,
    ) {
        Ok(artifact) => artifact,
        Err(_) => return false,
    };
    if nagi_model_manager::ModelArtifactReader::len(&artifact) != expected_size {
        return false;
    }
    let mut magic = [0u8; 4];
    if nagi_model_manager::ModelArtifactReader::read_at(&mut artifact, 0, &mut magic) != Ok(4)
        || &magic != b"GGUF"
    {
        return false;
    }
    let integrity = nagi_model_manager::IntegrityMetadata {
        algorithm: "sha256".into(),
        digest: expected_sha256.into(),
    };
    if nagi_model_manager::verify_model_artifact_integrity(&mut artifact, &integrity).is_err() {
        return false;
    }
    libnagi::console_write(marker) == marker.len()
}

#[cfg(target_os = "nagi")]
unsafe fn run_elf_initializers() {
    unsafe extern "C" {
        static __preinit_array_start: u8;
        static __preinit_array_end: u8;
        static __init_array_start: u8;
        static __init_array_end: u8;
    }

    let run_array = |mut cursor: usize, end: usize| {
        while cursor < end {
            let constructor = unsafe { (cursor as *const extern "C" fn()).read() };
            constructor();
            cursor += core::mem::size_of::<extern "C" fn()>();
        }
    };

    let preinit_start = core::ptr::addr_of!(__preinit_array_start) as usize;
    let preinit_end = core::ptr::addr_of!(__preinit_array_end) as usize;
    run_array(preinit_start, preinit_end);

    let init_start = core::ptr::addr_of!(__init_array_start) as usize;
    let init_end = core::ptr::addr_of!(__init_array_end) as usize;
    run_array(init_start, init_end);
}

#[cfg(all(target_os = "nagi", feature = "m27-recovery"))]
#[no_mangle]
pub extern "C" fn _start(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    net_capability: u64,
    audio_capability: u64,
    model_store_capability: u64,
) -> ! {
    unsafe { run_elf_initializers() };
    let _ = (
        display_capability,
        input_capability,
        net_capability,
        audio_capability,
        model_store_capability,
    );
    recovery::run(block_capability)
}

#[cfg(all(target_os = "nagi", not(feature = "m27-recovery")))]
#[cfg_attr(feature = "m20-llama-link-smoke", allow(unreachable_code))]
#[no_mangle]
pub extern "C" fn _start(
    block_capability: u64,
    display_capability: u64,
    input_capability: u64,
    net_capability: u64,
    audio_capability: u64,
    model_store_capability: u64,
) -> ! {
    // Check the kernel-provided ring-3 FPU state before C/C++ ELF constructors
    // run. Those user-space initializers may legitimately use SIMD registers,
    // so checking after them would test constructor residue instead of the
    // process-entry contract.
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

    unsafe { run_elf_initializers() };

    #[cfg(feature = "isolated-process-acceptance")]
    if !isolated_process::run() {
        libnagi::exit(1);
    }

    #[cfg(feature = "m30-update-install")]
    if !system_update::run(model_store_capability) {
        libnagi::exit(1);
    }

    #[cfg(feature = "m20-llama-link-smoke")]
    {
        let _ = (
            block_capability,
            display_capability,
            input_capability,
            net_capability,
            audio_capability,
            model_store_capability,
        );
        if relibc::nagi_backend_probe() != 0x4e41_4749 {
            libnagi::console_write(b"Nagi M20 relibc link FAIL\r\n");
            libnagi::exit(1);
        }
        if unsafe { nagi_m20_llama_backend_init_smoke() } == 0 {
            libnagi::console_write(b"Nagi M20 llama backend init PASS\r\n");
            libnagi::exit(0);
        }
        libnagi::console_write(b"Nagi M20 llama backend init FAIL\r\n");
        libnagi::exit(1);
    }

    #[cfg(not(feature = "m20-model-store-acceptance"))]
    let _ = model_store_capability;

    #[cfg(all(feature = "m13-std", not(feature = "m17-servo")))]
    return m13_std::run(block_capability, net_capability);

    #[cfg(feature = "m17-servo")]
    {
        libnagi::console_write(b"Nagi M17 trace: ELF constructors completed\r\n");
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
        libnagi::console_write(b"Nagi M17 trace: POSIX filesystem initialization started\r\n");
        if unsafe { nagi_posix::nagi_posix_initialize_filesystem(block_capability) } != 0 {
            libnagi::console_write(b"Nagi M17 first web pixel FAIL POSIX filesystem\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M17 trace: POSIX filesystem initialized\r\n");
        if unsafe { nagi_posix::nagi_posix_ensure_directory(c"/tmp".as_ptr()) } != 0 {
            libnagi::console_write(b"Nagi M17 first web pixel FAIL temporary directory\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M17 trace: temporary directory ready\r\n");
        // Publish the pinned Noto fonts (ADR 0062) read-only for Servo.
        for (path, data) in system_fonts::SYSTEM_FILES {
            if nagi_posix::register_static_file(path, data).is_err() {
                libnagi::console_write(b"Nagi M17 first web pixel FAIL system fonts\r\n");
                libnagi::exit(1);
            }
        }
        if system_fonts::SYSTEM_FILES.is_empty() {
            libnagi::console_write(b"Nagi M17 first web pixel FAIL no system fonts\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M17 trace: system fonts published\r\n");
        #[cfg(feature = "m18-acceptance")]
        {
            libnagi::console_write(b"Nagi M18 browser trace: network initialization started\r\n");
            if unsafe { nagi_posix::nagi_posix_initialize_network(net_capability) } != 0 {
                libnagi::console_write(b"Nagi M18 browser FAIL network initialization\r\n");
                libnagi::exit(1);
            }
            libnagi::console_write(b"Nagi M18 browser trace: network capability initialized\r\n");
            if nagi_posix::cleanup_m18_servo_temp_directories().is_none() {
                libnagi::console_write(b"Nagi M18 browser FAIL temporary storage cleanup\r\n");
                libnagi::exit(1);
            }
            if libnagi::console_write(b"Nagi M18 browser temporary storage cleanup PASS\r\n")
                != b"Nagi M18 browser temporary storage cleanup PASS\r\n".len()
            {
                libnagi::exit(1);
            }
            if unsafe {
                nagi_posix::nagi_posix_ensure_directory(c"/tmp/nagi-servo-profile".as_ptr())
            } != 0
            {
                libnagi::console_write(b"Nagi M18 browser FAIL Servo profile directory\r\n");
                libnagi::exit(1);
            }
            // Init owns the user-space clipboard service and registers
            // Albert as a READ/WRITE client. Gesture windows are in Nagi
            // timer ticks (about 10 ms): a paste shortcut authorizes one
            // read for 5 s; other input permits writes for 10 s.
            let clipboard_service =
                nagi_clipboard::ClipboardServiceOwner::new(nagi_clipboard::ClipboardPolicy {
                    paste_grant_ticks: 500,
                    activation_ticks: 1_000,
                });
            let Ok(albert_clipboard) =
                clipboard_service.register_client(nagi_clipboard::ClipboardRights::READ_WRITE)
            else {
                libnagi::console_write(b"Nagi M18 browser FAIL clipboard service\r\n");
                libnagi::exit(1);
            };
            return run_m18_https_acceptance(
                display_capability,
                input_capability,
                albert_clipboard,
            );
        }
        #[cfg(not(feature = "m18-acceptance"))]
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
    #[cfg(feature = "m20-model-store-acceptance")]
    if !run_m20_model_store_capability_acceptance(model_store_capability) {
        libnagi::console_write(b"Nagi M20 Model Store capability FAIL\r\n");
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
    #[cfg(feature = "m20-llama-inference-acceptance")]
    {
        if relibc::nagi_backend_probe() != 0x4e41_4749 {
            libnagi::console_write(b"Nagi M20 relibc link FAIL\r\n");
            libnagi::exit(1);
        }
        if exit_code != 0 || !m20_granite::run(model_store_capability) {
            libnagi::console_write(b"Nagi M20 Granite structured inference FAIL\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M20 Granite structured inference PASS\r\n");
        libnagi::exit(0);
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
        .present_stage(libnagi::boot::BootStage::Storage)
        .is_err()
    {
        libnagi::console_write(b"Nagi boot storage FAIL\r\n");
        libnagi::exit(1);
    }
    #[cfg(not(all(
        feature = "m10-desktop",
        not(any(
            feature = "m11-security",
            feature = "m12-network",
            feature = "m13-posix"
        ))
    )))]
    let _ = volume;
    #[cfg(feature = "m25-whisper-inference-acceptance")]
    {
        if exit_code != 0 {
            libnagi::exit(exit_code);
        }
        if !m25_whisper::run(model_store_capability) {
            libnagi::console_write(b"Nagi M25 Whisper Japanese fixture inference FAIL\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M25 Whisper Japanese fixture inference PASS\r\n");
        libnagi::exit(0);
    }
    #[cfg(all(
        feature = "m25-voice-acceptance",
        not(feature = "m25-whisper-inference-acceptance")
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
        if !m25_voice::run() {
            libnagi::console_write(b"Nagi M25 voice orchestration FAIL\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M25 voice orchestration PASS\r\n");
        loop {
            unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
        }
    }
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
        let Some(volume) = volume else {
            libnagi::console_write(b"Nagi M10 User Data handoff FAIL\r\n");
            libnagi::exit(1);
        };
        desktop::run(display_capability, input_capability, volume);
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
        feature = "m13-posix",
        feature = "m25-voice-acceptance"
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
