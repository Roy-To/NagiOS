//! Fail-closed package authentication and installed-inventory adapter for the
//! Linux `nagi-init` backend. Trust and inventory are supplied by the caller;
//! this module does not provision a product key, invent consent, or fake READY.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use nagi_model::AppId;
use nagi_package::{
    InstallPolicy, InstalledPackage, PackageError, PackageManifest, PackageService, PackageView,
    MAX_SLOTS,
};
use sha2::{Digest, Sha256};

/// An authoritative inventory read can be unavailable or internally invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryReadError {
    Unavailable,
    Failed,
}

/// Source of the installed packages held by the Package Service.
///
/// A Linux typed-IPC client can implement this boundary once its independent
/// transport/session work is ready. `PackageService` is the current in-process
/// implementation used by M16 and by host regressions.
pub trait InstalledPackageInventory {
    fn list_inventory(
        &self,
        output: &mut [Option<InstalledPackage>],
    ) -> Result<usize, InventoryReadError>;
}

impl InstalledPackageInventory for PackageService {
    fn list_inventory(
        &self,
        output: &mut [Option<InstalledPackage>],
    ) -> Result<usize, InventoryReadError> {
        PackageService::list_inventory(self, output).map_err(|error| match error {
            PackageError::OutputTooSmall => InventoryReadError::Failed,
            _ => InventoryReadError::Failed,
        })
    }
}

/// The input is the exact key selected by the authenticated boot/configuration
/// provider. `None` means trust provisioning is absent and must stop startup.
pub struct PackageSecurityProvider<'a, I: InstalledPackageInventory> {
    trusted_signing_key: Option<&'a [u8; 32]>,
    inventory: &'a I,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageProviderError {
    TrustUnavailable,
    InvalidPackage(PackageError),
    UnsignedPackage,
    UntrustedSigner,
    InventoryUnavailable,
    InventoryCorrupt,
    InventoryOutputTooSmall,
    IdentityCollision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallationState {
    Installed { generation: u64 },
    NotInstalled,
}

/// A `PackageView` that was authenticated against the injected trust anchor.
/// Keeping it with the result binds launch consumers to the exact immutable
/// bytes that were verified.
pub struct VerifiedPackage<'a> {
    view: PackageView<'a>,
}

impl<'a> VerifiedPackage<'a> {
    pub const fn view(&self) -> &PackageView<'a> {
        &self.view
    }

    pub const fn manifest(&self) -> PackageManifest {
        self.view.manifest()
    }
}

pub struct PackageInspection<'a> {
    pub verified_package: VerifiedPackage<'a>,
    pub installation: InstallationState,
}

impl<'a, I: InstalledPackageInventory> PackageSecurityProvider<'a, I> {
    pub const fn new(trusted_signing_key: Option<&'a [u8; 32]>, inventory: &'a I) -> Self {
        Self {
            trusted_signing_key,
            inventory,
        }
    }

    /// Parse and authenticate the exact `.xapp` bytes, then consult the real
    /// installed-package catalog. Absence is a successful, explicit state;
    /// a broken inventory or missing trust input is an error.
    pub fn inspect<'b>(
        &self,
        bytes: &'b [u8],
    ) -> Result<PackageInspection<'b>, PackageProviderError> {
        let package = PackageView::parse(bytes).map_err(PackageProviderError::InvalidPackage)?;
        if package.signature().is_empty() {
            return Err(PackageProviderError::UnsignedPackage);
        }
        let trusted_key = self
            .trusted_signing_key
            .ok_or(PackageProviderError::TrustUnavailable)?;
        if !package.is_signed_by(trusted_key) {
            return Err(PackageProviderError::UntrustedSigner);
        }

        let manifest = package.manifest();
        let mut inventory = [None; MAX_SLOTS];
        let count = self
            .inventory
            .list_inventory(&mut inventory)
            .map_err(|_| PackageProviderError::InventoryUnavailable)?;
        validate_inventory(&inventory, count)?;

        let app_id = manifest.app_id();
        let mut installed = None;
        for record in inventory[..count].iter().flatten() {
            let installed_id = record.manifest.app_id();
            if installed_id == app_id {
                if record.manifest.id() != manifest.id() {
                    return Err(PackageProviderError::IdentityCollision);
                }
                if installed.is_some() {
                    return Err(PackageProviderError::InventoryCorrupt);
                }
                installed = Some(record.generation);
            }
        }

        let installation = match installed {
            Some(generation) => InstallationState::Installed { generation },
            None => InstallationState::NotInstalled,
        };
        Ok(PackageInspection {
            verified_package: VerifiedPackage { view: package },
            installation,
        })
    }

    /// Copy the actual installed list into caller-owned storage. It is not a
    /// hardcoded catalog and reports no packages when the source is empty.
    pub fn list_installed(
        &self,
        output: &mut [InstalledPackage],
    ) -> Result<usize, PackageProviderError> {
        let mut inventory = [None; MAX_SLOTS];
        let count = self
            .inventory
            .list_inventory(&mut inventory)
            .map_err(|_| PackageProviderError::InventoryUnavailable)?;
        validate_inventory(&inventory, count)?;
        if output.len() < count {
            return Err(PackageProviderError::InventoryOutputTooSmall);
        }
        for (destination, record) in output.iter_mut().zip(inventory[..count].iter()) {
            *destination = record.ok_or(PackageProviderError::InventoryCorrupt)?;
        }
        Ok(count)
    }
}

fn validate_inventory(
    inventory: &[Option<InstalledPackage>; MAX_SLOTS],
    count: usize,
) -> Result<(), PackageProviderError> {
    if count > inventory.len()
        || inventory[..count].iter().any(Option::is_none)
        || inventory[count..].iter().any(Option::is_some)
    {
        return Err(PackageProviderError::InventoryCorrupt);
    }
    for (index, record) in inventory[..count].iter().flatten().enumerate() {
        if inventory[..index]
            .iter()
            .flatten()
            .any(|previous| previous.manifest.app_id() == record.manifest.app_id())
        {
            return Err(PackageProviderError::InventoryCorrupt);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageServiceSessionOwnerError {
    TrustUnavailable,
    Package(PackageError),
    Provider(PackageProviderError),
    IdentityMismatch { actual: AppId, expected: AppId },
    NotInstalled,
    ManifestChanged,
    StaleGeneration { captured: u64, current: u64 },
    GenerationSaturated,
    OwnerMismatch,
    Unavailable,
    Quarantined,
    DigestInventoryFull,
    PackageDigestUnavailable,
    InstalledPackageBytesMismatch,
}

#[must_use = "retain the snapshot and validate it through its owning session before use"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedPackageInventorySnapshot {
    manifest: PackageManifest,
    generation: u64,
    owner_incarnation: u64,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
}

#[derive(Clone, Copy)]
struct InstalledPackageDigest {
    app_id: AppId,
    generation: u64,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
}

impl OwnedPackageInventorySnapshot {
    pub const fn app_id(self) -> AppId {
        self.manifest.app_id()
    }

    pub fn manifest_id(&self) -> &[u8] {
        self.manifest.id()
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn signed_package_sha256(&self) -> &[u8; 32] {
        &self.signed_package_sha256
    }

    pub const fn executable_sha256(&self) -> &[u8; 32] {
        &self.executable_sha256
    }
}

/// Read-only borrow of the exact PackageService retained by one init session.
/// The borrow prevents install/remove through that owner until it is dropped.
pub struct PackageServiceGenerationLease<'owner> {
    inventory: &'owner PackageService,
    snapshot: OwnedPackageInventorySnapshot,
    quarantined: Arc<AtomicBool>,
}

impl PackageServiceGenerationLease<'_> {
    pub const fn snapshot(&self) -> &OwnedPackageInventorySnapshot {
        &self.snapshot
    }

    pub const fn inventory(&self) -> &PackageService {
        self.inventory
    }

    /// Permanently fence package inventory mutations for this owner after an
    /// ambiguous process/recovery outcome. The C1/C3 reconciliation path must
    /// decide when a new owner may be created; this lease cannot clear it.
    pub fn quarantine(&self) {
        self.quarantined.store(true, Ordering::SeqCst);
    }
}

/// Session-lifetime owner for one PackageService value. Construct this owner
/// before the session's first install and route every mutation through it.
/// Its borrow-based lease is an in-process same-owner fence; this is not
/// cross-process exclusion, production trust provisioning, or authorization.
///
/// A live lease prevents ending the owner or mutating the inventory:
///
/// ```compile_fail
/// use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
/// use nagi_model::AppId;
/// use nagi_package::PackageService;
///
/// let mut session = PackageServiceSessionOwner::new(None, PackageService::new());
/// let snapshot = session.capture(&[], b"expected").unwrap();
/// let lease = session.lease_for_dispatch(&snapshot).unwrap();
/// session.remove(AppId(1));
/// drop(lease);
/// ```
pub struct PackageServiceSessionOwner<'key> {
    trusted_key: Option<&'key [u8; 32]>,
    owner_incarnation: Option<u64>,
    inventory: PackageService,
    quarantined: Arc<AtomicBool>,
    package_digests: [Option<InstalledPackageDigest>; MAX_SLOTS],
}

fn next_session_owner_incarnation() -> Option<u64> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_add(1)
    })
    .ok()
}

impl<'key> PackageServiceSessionOwner<'key> {
    /// Adopt the caller's preinitialized service by value; this does not create
    /// a replacement PackageService or join another owner's generation domain.
    pub fn new(trusted_key: Option<&'key [u8; 32]>, inventory: PackageService) -> Self {
        Self {
            trusted_key,
            owner_incarnation: next_session_owner_incarnation(),
            inventory,
            quarantined: Arc::new(AtomicBool::new(false)),
            package_digests: [None; MAX_SLOTS],
        }
    }

    fn ensure_not_quarantined(&self) -> Result<(), PackageServiceSessionOwnerError> {
        if self.quarantined.load(Ordering::SeqCst) {
            Err(PackageServiceSessionOwnerError::Quarantined)
        } else {
            Ok(())
        }
    }

    /// Clear an in-process quarantine only after the caller's exact external
    /// reconciliation succeeds. The launch coordinator must hold the matching
    /// C3 lifecycle gate while its C1 durable reconciliation runs in `check`.
    pub fn reconcile_quarantine_after<R, E>(
        &mut self,
        check: impl FnOnce() -> Result<R, E>,
    ) -> Result<R, E> {
        let result = check()?;
        self.quarantined.store(false, Ordering::SeqCst);
        Ok(result)
    }

    /// Install through the retained service. Call this on the owner already
    /// stored in the lifecycle session, including for the first installation.
    pub fn install(
        &mut self,
        bytes: &[u8],
        policy: InstallPolicy,
    ) -> Result<nagi_package::InstallReport, PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        let trusted_key = self
            .trusted_key
            .ok_or(PackageServiceSessionOwnerError::TrustUnavailable)?;
        let package =
            PackageView::parse(bytes).map_err(PackageServiceSessionOwnerError::Package)?;
        let app_id = package.manifest().app_id();
        let digest_slot = self
            .package_digests
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.app_id == app_id))
            .or_else(|| self.package_digests.iter().position(Option::is_none))
            .ok_or(PackageServiceSessionOwnerError::DigestInventoryFull)?;
        let report = self
            .inventory
            .install_with_trusted_key(&package, policy, trusted_key)
            .map_err(PackageServiceSessionOwnerError::Package)?;
        let installed = match self.inventory.info(app_id) {
            Ok(installed) if installed.manifest == package.manifest() => installed,
            Ok(_) => {
                self.quarantined.store(true, Ordering::SeqCst);
                return Err(PackageServiceSessionOwnerError::ManifestChanged);
            }
            Err(error) => {
                self.quarantined.store(true, Ordering::SeqCst);
                return Err(PackageServiceSessionOwnerError::Package(error));
            }
        };
        self.package_digests[digest_slot] = Some(InstalledPackageDigest {
            app_id,
            generation: installed.generation,
            signed_package_sha256: sha256(bytes),
            executable_sha256: sha256(package.executable()),
        });
        Ok(report)
    }

    /// Uninstall through the same retained service. Safe Rust's exclusive
    /// borrow rules prevent this while a generation lease is live.
    pub fn remove(&mut self, app_id: AppId) -> Result<(), PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        self.inventory
            .remove(app_id)
            .map_err(PackageServiceSessionOwnerError::Package)?;
        for entry in &mut self.package_digests {
            if entry.is_some_and(|entry| entry.app_id == app_id) {
                *entry = None;
            }
        }
        Ok(())
    }

    /// Authenticate package bytes, require the expected manifest identity,
    /// and capture the generation currently installed in this owner.
    pub fn capture(
        &self,
        bytes: &[u8],
        expected_id: &[u8],
    ) -> Result<OwnedPackageInventorySnapshot, PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        let trusted_key = self
            .trusted_key
            .ok_or(PackageServiceSessionOwnerError::TrustUnavailable)?;
        let owner_incarnation = self
            .owner_incarnation
            .ok_or(PackageServiceSessionOwnerError::Unavailable)?;
        let parsed = PackageView::parse(bytes).map_err(PackageServiceSessionOwnerError::Package)?;
        let manifest = parsed.manifest();
        let expected = AppId::from_identifier(expected_id);
        if manifest.id() != expected_id || manifest.app_id() != expected {
            return Err(PackageServiceSessionOwnerError::IdentityMismatch {
                actual: manifest.app_id(),
                expected,
            });
        }

        let inspection = PackageSecurityProvider::new(Some(trusted_key), &self.inventory)
            .inspect(bytes)
            .map_err(PackageServiceSessionOwnerError::Provider)?;
        let generation = match inspection.installation {
            InstallationState::Installed { generation } => generation,
            InstallationState::NotInstalled => {
                return Err(PackageServiceSessionOwnerError::NotInstalled)
            }
        };
        if generation == u64::MAX {
            return Err(PackageServiceSessionOwnerError::GenerationSaturated);
        }
        let verified_manifest = inspection.verified_package.manifest();
        if verified_manifest != manifest {
            return Err(PackageServiceSessionOwnerError::ManifestChanged);
        }
        let signed_package_sha256 = sha256(bytes);
        let executable_sha256 = sha256(parsed.executable());
        let Some(record) = self
            .package_digests
            .iter()
            .flatten()
            .find(|record| record.app_id == expected && record.generation == generation)
        else {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        };
        if record.signed_package_sha256 != signed_package_sha256
            || record.executable_sha256 != executable_sha256
        {
            return Err(PackageServiceSessionOwnerError::InstalledPackageBytesMismatch);
        }

        Ok(OwnedPackageInventorySnapshot {
            manifest: verified_manifest,
            generation,
            owner_incarnation,
            signed_package_sha256,
            executable_sha256,
        })
    }

    /// Revalidate the full current inventory and return a borrow of the exact
    /// retained service. A different owner or generation is rejected.
    pub fn lease_for_dispatch<'owner>(
        &'owner self,
        snapshot: &OwnedPackageInventorySnapshot,
    ) -> Result<PackageServiceGenerationLease<'owner>, PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        let trusted_key = self
            .trusted_key
            .ok_or(PackageServiceSessionOwnerError::TrustUnavailable)?;
        let owner_incarnation = self
            .owner_incarnation
            .ok_or(PackageServiceSessionOwnerError::Unavailable)?;
        if snapshot.owner_incarnation != owner_incarnation {
            return Err(PackageServiceSessionOwnerError::OwnerMismatch);
        }
        if snapshot.generation == u64::MAX {
            return Err(PackageServiceSessionOwnerError::GenerationSaturated);
        }

        let placeholder = InstalledPackage {
            manifest: snapshot.manifest,
            generation: snapshot.generation,
        };
        let mut entries = [placeholder; MAX_SLOTS];
        let count = PackageSecurityProvider::new(Some(trusted_key), &self.inventory)
            .list_installed(&mut entries)
            .map_err(PackageServiceSessionOwnerError::Provider)?;
        let mut current = None;
        for installed in &entries[..count] {
            if installed.manifest.app_id() != snapshot.app_id() {
                continue;
            }
            if current.replace(*installed).is_some() {
                return Err(PackageServiceSessionOwnerError::Provider(
                    PackageProviderError::InventoryCorrupt,
                ));
            }
        }
        let Some(current_package) = current else {
            return Err(PackageServiceSessionOwnerError::NotInstalled);
        };
        if current_package.manifest.id() != snapshot.manifest.id() {
            return Err(PackageServiceSessionOwnerError::ManifestChanged);
        }
        if current_package.generation != snapshot.generation {
            return Err(PackageServiceSessionOwnerError::StaleGeneration {
                captured: snapshot.generation,
                current: current_package.generation,
            });
        }
        if current_package.manifest != snapshot.manifest {
            return Err(PackageServiceSessionOwnerError::ManifestChanged);
        }
        let Some(record) = self.package_digests.iter().flatten().find(|record| {
            record.app_id == snapshot.app_id() && record.generation == snapshot.generation
        }) else {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        };
        if record.signed_package_sha256 != snapshot.signed_package_sha256
            || record.executable_sha256 != snapshot.executable_sha256
        {
            return Err(PackageServiceSessionOwnerError::InstalledPackageBytesMismatch);
        }

        Ok(PackageServiceGenerationLease {
            inventory: &self.inventory,
            snapshot: *snapshot,
            quarantined: Arc::clone(&self.quarantined),
        })
    }

    /// Consume the owner after the lifecycle owner has handled process cleanup.
    pub fn end_session(self) {}
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut result = [0; 32];
    result.copy_from_slice(&digest);
    result
}
