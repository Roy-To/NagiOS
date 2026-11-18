//! Receive package inputs from the trusted Linux boot stage that execs PID 1.
//! The stage retains its selected writable package-store directory as fd 3
//! and supplies the signed catalog as fd 4. Init pins the catalog signer in
//! its build and closes both descriptor slots to subsequent execs.

use nagi_system_service_launch::{
    ArtifactPreflight, ArtifactPreflightRequest, LaunchPortError, VerifiedBootCatalog,
    MAX_SIGNED_BOOT_CATALOG_BYTES,
};
use rustix::fs::{openat2, Mode, OFlags, ResolveFlags};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::os::fd::{FromRawFd, RawFd};

const BOOT_CATALOG_PUBLIC_KEY: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/linux-boot-catalog-public-key"));
const PACKAGE_STORE_FD: RawFd = 3;
const SIGNED_CATALOG_FD: RawFd = 4;
const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const FD_CLOEXEC: i32 = 1;

unsafe extern "C" {
    fn fcntl(fd: RawFd, command: i32, ...) -> i32;
}

pub(super) struct BootPackageInputs {
    pub(super) directory: File,
    pub(super) catalog: VerifiedBootCatalog,
}

impl BootPackageInputs {
    pub(super) fn take_inherited() -> Result<Self, &'static str> {
        let key = <&[u8; 32]>::try_from(BOOT_CATALOG_PUBLIC_KEY)
            .map_err(|_| "boot_catalog_key_unconfigured")?;
        let directory = take_boot_descriptor(PACKAGE_STORE_FD);
        let catalog = take_boot_descriptor(SIGNED_CATALOG_FD);
        let directory = directory?;
        let catalog = catalog?;
        if !catalog.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return Err("boot_catalog_file_unavailable");
        }
        let mut bytes = Vec::new();
        catalog
            .take((MAX_SIGNED_BOOT_CATALOG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "boot_catalog_read_failed")?;
        let catalog = VerifiedBootCatalog::verify(&bytes, key)
            .map_err(|_| "boot_catalog_verification_failed")?;
        Ok(Self { directory, catalog })
    }
}

/// Inspect the catalog-selected artifact in PID 1 using the same ELF parser
/// used by the package execution path. The returned report is bound to the
/// installed executable by the launcher. C3 executes the installed bytes.
pub(super) struct LinuxBootArtifactPreflight {
    root: File,
}

impl LinuxBootArtifactPreflight {
    pub(super) fn new() -> Result<Self, LaunchPortError> {
        let root = File::open("/").map_err(|_| LaunchPortError::Unavailable)?;
        Ok(Self { root })
    }
}

impl ArtifactPreflight for LinuxBootArtifactPreflight {
    fn inspect(&mut self, request: &ArtifactPreflightRequest) -> Result<String, LaunchPortError> {
        let path = request.artifact_path().trim_start_matches('/');
        let descriptor = openat2(
            &self.root,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| LaunchPortError::Unavailable)?;
        let file = File::from(descriptor);
        if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return Err(LaunchPortError::Rejected);
        }
        let mut artifact = Vec::new();
        file.take((nagi_package::MAX_PACKAGE_BYTES + 1) as u64)
            .read_to_end(&mut artifact)
            .map_err(|_| LaunchPortError::Failed)?;
        if artifact.len() > nagi_package::MAX_PACKAGE_BYTES {
            return Err(LaunchPortError::Rejected);
        }
        super::validate_static_x86_64_elf(&artifact).map_err(|_| LaunchPortError::Rejected)?;
        let artifact_digest = format!("{:x}", Sha256::digest(&artifact));
        if artifact_digest != request.expected_sha256() {
            return Err(LaunchPortError::Rejected);
        }
        let program_header_count =
            super::elf_u16(&artifact, 56).map_err(|_| LaunchPortError::Rejected)?;
        Ok(serde_json::json!({
            "status": "artifact_preflight_passed",
            "schema_version": 1,
            "target": request.target(),
            "artifact_sha256": artifact_digest,
            "elf_type": "ET_EXEC",
            "program_header_count": program_header_count,
            "checks": {
                "regular_file": true,
                "no_symlink_path_components": true,
                "sha256_matches_expected": true,
                "elf64_little_endian": true,
                "x86_64": true,
                "executable_elf": true,
                "no_pt_interp": true,
            },
            "claim_scope": "artifact_preflight_only",
            "not_proven": [
                "signature", "canonical_package_identity", "service_role",
                "user_authentication", "policy_authorization", "installation",
                "runtime_ready"
            ]
        })
        .to_string())
    }
}

fn take_boot_descriptor(fd: RawFd) -> Result<File, &'static str> {
    // Called once at the configured PID 1 entry, before spawning threads or
    // opening other files. The boot exec contract reserves both descriptors.
    let flags = unsafe { fcntl(fd, F_GETFD) };
    if flags < 0 || unsafe { fcntl(fd, F_SETFD, flags | FD_CLOEXEC) } < 0 {
        return Err(if fd == PACKAGE_STORE_FD {
            "package_store_descriptor_unavailable"
        } else {
            "boot_catalog_descriptor_unavailable"
        });
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
