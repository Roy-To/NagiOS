//! Prepare fresh Task24 QEMU inputs with disposable development signing keys.

use ed25519_dalek::{Signer, SigningKey};
use nagi_init_package_provider::{
    linux_package_persistence::LinuxInstalledPackagePersistence,
    package_trust::PackageServiceSessionOwner,
};
use nagi_model::AppId;
use nagi_package::{
    build_xapp, InstallPolicy, PackageService, PackageView, MAX_PACKAGE_BYTES, SIGNATURE_BYTES,
};
use nagi_system_service_launch::{
    bind_installed_service_package, BootReadyRoute, VerifiedBootCatalog, SYSTEM_SERVICE_TARGET,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read};
use std::num::NonZeroU64;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::Path;

const CATALOG_DOMAIN: &[u8] = b"Nagi Linux boot system service catalog v1\0";
const IDENTITY_SERVICE: &str = "nagi.identity";
const IDENTITY_MANIFEST: &[u8] =
    include_bytes!("../../../user/nagi-init/manifests/org.nagi.dev.task24-identity.manifest");
const IDENTITY_FILE: &str = "task24-identity-ready.elf";
const IDENTITY_PATH: &str = "/system/task24-identity-ready.elf";
const READINESS_REQUEST: &str = "nagi.identity.readiness.request@1";
const GENERATION: u64 = 2;
const UID: u32 = 1001;
const GID: u32 = 1002;
const POLICY: InstallPolicy = InstallPolicy {
    developer_mode: false,
    require_signature: true,
};

fn read_package_input(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_PACKAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input exceeds XAPP capacity",
        ));
    }
    Ok(bytes)
}

fn sign_with(key: &SigningKey, package: &mut [u8]) {
    let split = package.len() - SIGNATURE_BYTES;
    let signature = key.sign(&package[..split]);
    package[split..].copy_from_slice(&signature.to_bytes());
}

fn fresh_key() -> io::Result<SigningKey> {
    let mut seed = [0; 32];
    File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let key = SigningKey::from_bytes(&seed);
    seed.fill(0);
    Ok(key)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [identity_elf, login_package, output] = args.as_slice() else {
        return Err("usage: prepare_task24_identity_inputs <identity-elf> <signed-task24-login-xapp> <new-output-directory>".into());
    };
    let identity = read_package_input(Path::new(identity_elf))?;
    let mut login = read_package_input(Path::new(login_package))?;
    let login_view =
        PackageView::parse(&login).map_err(|err| format!("invalid login XAPP ({err:?})"))?;
    if login_view.manifest().id() != b"org.nagi.dev.task24-login"
        || login_view.manifest().entry() != b"task24-login.elf"
        || login_view.signature().len() != SIGNATURE_BYTES
        || !login_view.executable().starts_with(b"\x7fELF")
        || !identity.starts_with(b"\x7fELF")
    {
        return Err("invalid Task24 development executable or login XAPP".into());
    }

    let boot_key = fresh_key()?;
    let package_key = fresh_key()?;
    let boot_public = boot_key.verifying_key().to_bytes();
    let package_public = package_key.verifying_key().to_bytes();
    let mut identity_xapp = vec![0; MAX_PACKAGE_BYTES];
    let identity_len = build_xapp(
        IDENTITY_MANIFEST,
        &identity,
        &[],
        &[],
        &[],
        &[0; SIGNATURE_BYTES],
        &mut identity_xapp,
    )
    .map_err(|err| format!("could not build Identity XAPP ({err:?})"))?;
    identity_xapp.truncate(identity_len);
    sign_with(&package_key, &mut identity_xapp);
    sign_with(&package_key, &mut login);

    let descriptor = json!({
        "schema_version": 1,
        "service_id": IDENTITY_SERVICE,
        "role": "system_service",
        "package": {"id": "org.nagi.dev.task24-identity", "canonical_entry": IDENTITY_FILE},
        "executable": {
            "artifact_path": IDENTITY_PATH,
            "expected_sha256": format!("{:x}", Sha256::digest(&identity)),
            "target": SYSTEM_SERVICE_TARGET,
        },
        "credentials": {"uid": UID, "gid": GID},
        "generation": GENERATION,
        "lifecycle": {"mode": "initial"},
        "endpoint": {
            "transport": "unix_stream",
            "owner": "c3_pidfd_supervisor_generation",
            "handoff": "connected_owned_fd",
            "generation_policy": "unique_per_connection",
            "expected_peer_uid": UID,
            "expected_peer_gid": GID,
        }
    });
    let mut signed_catalog = serde_json::to_vec(&json!({
        "schema_version": 1,
        "package_signing_public_key": package_public,
        "descriptor_json": descriptor.to_string(),
        "principal_id": "service:nagi.identity",
        "caller_context_id": "caller_task24_identity",
        "mapping_generation": 1,
    }))?;
    let signature = boot_key.sign(&[CATALOG_DOMAIN, signed_catalog.as_slice()].concat());
    signed_catalog.extend_from_slice(&signature.to_bytes());
    let catalog = VerifiedBootCatalog::verify(&signed_catalog, &boot_public)
        .map_err(|err| format!("catalog verification failed ({err:?})"))?;
    let plan = catalog
        .sign_development_identity_ready_plan(
            &boot_key,
            BootReadyRoute {
                service_id: NonZeroU64::new(AppId::from_identifier(IDENTITY_SERVICE.as_bytes()).0)
                    .ok_or("invalid Identity service selector")?,
                operation_id: NonZeroU64::new(
                    AppId::from_identifier(READINESS_REQUEST.as_bytes()).0,
                )
                .ok_or("invalid Identity readiness operation selector")?,
                version_major: 1,
                version_minor: 0,
            },
        )
        .map_err(|err| format!("READY plan verification failed ({err:?})"))?;

    let output = Path::new(output);
    fs::DirBuilder::new().mode(0o700).create(output)?;
    let package_store = output.join("package-store");
    fs::DirBuilder::new().mode(0o700).create(&package_store)?;
    let directory = File::open(&package_store)?;
    let owner = directory.metadata()?.uid();
    let mut blobs = LinuxInstalledPackagePersistence::from_trusted_directory(directory, owner)
        .map_err(|err| format!("package blob store unavailable ({err:?})"))?;
    let mut index =
        LinuxInstalledPackagePersistence::from_trusted_directory(File::open(package_store)?, owner)
            .map_err(|err| format!("package index unavailable ({err:?})"))?;
    let mut inventory =
        PackageServiceSessionOwner::new(Some(&package_public), PackageService::new());
    for bytes in [&identity_xapp, &login] {
        inventory
            .install_with_persistent_storage(bytes, POLICY, &mut blobs, &mut index)
            .map_err(|err| format!("development package install failed ({err:?})"))?;
    }
    let restored = PackageServiceSessionOwner::restore_from_persistent_storage(
        Some(&package_public),
        POLICY,
        &blobs,
        &index,
    )
    .map_err(|err| format!("development package store readback failed ({err:?})"))?;
    let service =
        bind_installed_service_package(catalog.descriptor(), &identity_xapp, &restored)
            .map_err(|err| format!("installed Identity does not match the catalog ({err:?})"))?;
    if service.executable_bytes() != identity {
        return Err("installed Identity bytes differ from the executable input".into());
    }
    fs::write(output.join("catalog.pub"), boot_public)?;
    fs::write(output.join("catalog.bin"), signed_catalog)?;
    fs::write(output.join("identity-ready-plan.bin"), plan)?;
    fs::write(output.join(IDENTITY_FILE), identity)?;
    fs::write(output.join("task24-login.xapp"), login)?;
    Ok(())
}
