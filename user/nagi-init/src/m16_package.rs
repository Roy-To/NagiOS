use libnagi::storage::{StorageError, SyscallBlockDevice, Vfs};
use nagi_package::{
    build_xapp, InstallPolicy, PackageError, PackageService, PackageView, MAX_PACKAGE_BYTES,
};
use nagi_sdk::{AppSessionId, Application, PresentationContext, HELLO_APP_ID, HELLO_MANIFEST};

const PACKAGE_FILE: &[u8] = b"m16-package";
const PACKAGE_NEXT_FILE: &[u8] = b"m16-package-next";
const LICENSE: &[u8] = b"MIT\n";
const RESOURCES: &[u8] = b"hello-resource";
const SCHEMAS: &[u8] = b"hello-action@1";
const UPDATED_EXECUTABLE: &[u8] = b"NAPP\x01\x01\x1d\x00Hello from updated Nagi app\r\n";

static BUNDLED_PACKAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/m16-package.xapp"));
static mut PACKAGE: [u8; MAX_PACKAGE_BYTES] = [0; MAX_PACKAGE_BYTES];
static mut UPDATED_PACKAGE: [u8; MAX_PACKAGE_BYTES] = [0; MAX_PACKAGE_BYTES];

type GuestVolume = Vfs<SyscallBlockDevice>;

pub fn run(block_capability: u64) -> bool {
    let Ok((mut volume, _formatted)) =
        Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
    else {
        return false;
    };
    cleanup(&mut volume);
    let app = Application::new(nagi_sdk::HELLO_APP_ID, AppSessionId(1));
    let surface = app.presentation(PresentationContext::compact(320, 200));
    if surface.node_id.0 != 1 || surface.surface_id.0 != app.session_id.0 {
        return false;
    }
    marker(b"Nagi M16 SDK identity PASS\r\n");

    if BUNDLED_PACKAGE.len() > MAX_PACKAGE_BYTES {
        return false;
    }
    let package = match PackageView::parse(BUNDLED_PACKAGE) {
        Ok(package) => package,
        Err(_) => return false,
    };
    if package.manifest().app_id() != HELLO_APP_ID || package.is_signed() {
        return false;
    }
    let mut service = PackageService::new();
    if service.install(
        &package,
        InstallPolicy {
            developer_mode: false,
            require_signature: false,
        },
    ) != Err(PackageError::SignatureRequired)
    {
        return false;
    }
    let report = match service.install(
        &package,
        InstallPolicy {
            developer_mode: true,
            require_signature: false,
        },
    ) {
        Ok(report) => report,
        Err(_) => return false,
    };
    if !report.unsigned_warning || report.replaced {
        return false;
    }
    if write_or_create(&mut volume, PACKAGE_FILE, BUNDLED_PACKAGE).is_err() {
        return false;
    }
    marker(b"Nagi M16 package install PASS\r\n");

    let Ok(installed_info) = service.info(report.app_id) else {
        return false;
    };
    let mut installed = [installed_info; 2];
    let Ok(installed_count) = service.list(&mut installed) else {
        return false;
    };
    if installed_count != 1 {
        return false;
    }
    marker(b"Nagi M16 package list/info PASS\r\n");

    let stored_package = match read_package(&mut volume) {
        Ok(package) => package,
        Err(_) => return false,
    };
    if stored_package.manifest().app_id() != report.app_id
        || !execute_napp(stored_package.executable())
    {
        return false;
    }
    marker(b"Nagi M16 Hello app launch PASS\r\n");

    let updated_bytes = unsafe { &mut *core::ptr::addr_of_mut!(UPDATED_PACKAGE) };
    let updated_length = match build_xapp(
        HELLO_MANIFEST,
        UPDATED_EXECUTABLE,
        RESOURCES,
        SCHEMAS,
        LICENSE,
        b"",
        updated_bytes,
    ) {
        Ok(length) => length,
        Err(_) => return false,
    };
    let updated = match PackageView::parse(&updated_bytes[..updated_length]) {
        Ok(package) => package,
        Err(_) => return false,
    };
    let updated_report = match service.install(
        &updated,
        InstallPolicy {
            developer_mode: true,
            require_signature: false,
        },
    ) {
        Ok(report) => report,
        Err(_) => return false,
    };
    if !updated_report.replaced || !updated_report.unsigned_warning {
        return false;
    }
    if write_or_create(
        &mut volume,
        PACKAGE_NEXT_FILE,
        &updated_bytes[..updated_length],
    )
    .is_err()
        || volume.replace(PACKAGE_NEXT_FILE, PACKAGE_FILE).is_err()
    {
        return false;
    }
    let updated_stored = match read_package(&mut volume) {
        Ok(package) => package,
        Err(_) => return false,
    };
    if updated_stored.executable() != updated.executable()
        || !execute_napp(updated_stored.executable())
    {
        return false;
    }
    marker(b"Nagi M16 package atomic update PASS\r\n");

    if service.remove(report.app_id).is_err()
        || volume.remove(PACKAGE_FILE).is_err()
        || volume.open(PACKAGE_FILE) != Err(StorageError::NotFound)
    {
        return false;
    }
    marker(b"Nagi M16 package remove PASS\r\n");
    marker(b"Nagi M16 Package Service PASS\r\n");
    marker(b"Nagi M16 acceptance PASS\r\n");
    true
}

fn execute_napp(executable: &[u8]) -> bool {
    if executable.len() < 8 || &executable[..4] != b"NAPP" || executable[4] != 1 {
        return false;
    }
    let length = usize::from(u16::from_le_bytes([executable[6], executable[7]]));
    if executable[5] != 1 || executable.len() != 8 + length || length == 0 {
        return false;
    }
    libnagi::console_write(&executable[8..]) == length
}

fn cleanup(volume: &mut GuestVolume) {
    for name in [PACKAGE_FILE, PACKAGE_NEXT_FILE] {
        let _ = volume.remove(name);
    }
}

fn read_package(volume: &mut GuestVolume) -> Result<PackageView<'static>, StorageError> {
    let handle = volume.open(PACKAGE_FILE)?;
    let package_bytes: &'static mut [u8; MAX_PACKAGE_BYTES] =
        unsafe { &mut *core::ptr::addr_of_mut!(PACKAGE) };
    let length = volume
        .read(handle, package_bytes)
        .map_err(|_| StorageError::Corrupt)?;
    PackageView::parse(&package_bytes[..length]).map_err(|_| StorageError::Corrupt)
}

fn write_or_create(
    volume: &mut GuestVolume,
    name: &[u8],
    bytes: &[u8],
) -> Result<(), StorageError> {
    let handle = match volume.open(name) {
        Ok(handle) => handle,
        Err(StorageError::NotFound) => volume.create(name)?,
        Err(error) => return Err(error),
    };
    volume.write(handle, bytes)
}

fn marker(message: &[u8]) {
    libnagi::console_write(message);
}
