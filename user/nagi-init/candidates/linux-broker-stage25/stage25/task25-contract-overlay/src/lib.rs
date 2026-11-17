//! Task-local consumer of the existing Linux package identity and inventory APIs.
//!
//! This adapter exercises the same package boundary used by the Linux
//! Developer Preview READY path. The caller supplies its trust anchor and
//! inventory; this module does not create production authority or consent.

use nagi_init_package_provider::package_trust::{
    InstallationState, InstalledPackageInventory, PackageInspection, PackageProviderError,
    PackageSecurityProvider,
};
use nagi_model::AppId;
use nagi_package::{
    InstallPolicy, InstalledPackage, PackageError, PackageManifest, PackageService, PackageView,
    MAX_SLOTS,
};

#[path = "linux_package_identity.rs"]
mod linux_package_identity;

pub mod model_artifact_lease;
pub mod stage13_borrow_adapter_candidate;

#[derive(Debug, Eq, PartialEq)]
pub enum ConsumerError {
    InputUnavailable,
    InvalidPackage(PackageError),
    IdentityMismatch { actual: AppId, expected: AppId },
    Install(PackageError),
    Provider(PackageProviderError),
    InstallWasNotVisible,
    UnexpectedAcceptance,
}

fn parse_expected_package<'a>(
    bytes: &'a [u8],
    expected_id: &[u8],
) -> Result<PackageView<'a>, ConsumerError> {
    let package = PackageView::parse(bytes).map_err(ConsumerError::InvalidPackage)?;
    let actual = package.manifest().app_id();
    let expected = AppId::from_identifier(expected_id);
    if package.manifest().id() != expected_id {
        return Err(ConsumerError::IdentityMismatch { actual, expected });
    }

    linux_package_identity::with_expected_app_id(actual, expected, || package)
        .map_err(|actual| ConsumerError::IdentityMismatch { actual, expected })
}

/// Authenticate the supplied package bytes and inspect the caller-owned catalog
/// using the existing production provider. `Installed` means the authenticated
/// package's AppId and manifest identifier occur in inventory; the current
/// inventory stores only a manifest and generation, so this does not establish
/// byte-identical stored executable content. This method does not install the
/// package and preserves the provider's explicit NotInstalled state.
pub fn inspect_expected<'a, Inventory: InstalledPackageInventory>(
    bytes: &'a [u8],
    expected_id: &[u8],
    trusted_key: Option<&[u8; 32]>,
    inventory: &Inventory,
) -> Result<PackageInspection<'a>, ConsumerError> {
    let parsed = parse_expected_package(bytes, expected_id)?;
    drop(parsed);

    PackageSecurityProvider::new(trusted_key, inventory)
        .inspect(bytes)
        .map_err(ConsumerError::Provider)
}

/// Mirror the development-only install boundary: check the expected manifest
/// identity, install through PackageService with its explicit development
/// policy, then re-inspect the same bytes and resulting logical inventory.
pub fn install_and_inspect_development_candidate<'a>(
    bytes: &'a [u8],
    expected_id: &[u8],
    trusted_key: &[u8; 32],
    inventory: &mut PackageService,
) -> Result<PackageInspection<'a>, ConsumerError> {
    let package = parse_expected_package(bytes, expected_id)?;
    let expected = package.manifest().app_id();

    let report = inventory
        .install_with_trusted_key(
            &package,
            InstallPolicy {
                developer_mode: true,
                require_signature: true,
            },
            trusted_key,
        )
        .map_err(ConsumerError::Install)?;
    if report.app_id != expected || report.unsigned_warning {
        return Err(ConsumerError::IdentityMismatch {
            actual: report.app_id,
            expected,
        });
    }

    let inspection = PackageSecurityProvider::new(Some(trusted_key), inventory)
        .inspect(bytes)
        .map_err(ConsumerError::Provider)?;
    if !matches!(inspection.installation, InstallationState::Installed { .. }) {
        return Err(ConsumerError::InstallWasNotVisible);
    }
    Ok(inspection)
}

/// Read-only identity/generation snapshot supplied from an authenticated
/// XAPP and the live PackageService catalog. It carries no executable bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageInventorySnapshot {
    manifest: PackageManifest,
    generation: u64,
}

impl PackageInventorySnapshot {
    pub const fn app_id(self) -> AppId {
        self.manifest.app_id()
    }

    pub fn manifest_id(&self) -> &[u8] {
        self.manifest.id()
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Snapshot minted by one retained PackageService owner. The private owner
/// incarnation prevents a same-AppId/same-generation snapshot from a different
/// PackageService instance being replayed into this owner's validator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedPackageInventorySnapshot {
    inventory: PackageInventorySnapshot,
    owner_incarnation: u64,
}

impl OwnedPackageInventorySnapshot {
    pub const fn app_id(self) -> AppId {
        self.inventory.app_id()
    }

    pub fn manifest_id(&self) -> &[u8] {
        self.inventory.manifest_id()
    }

    pub const fn generation(self) -> u64 {
        self.inventory.generation()
    }
}

/// A short-lived, read-only borrow that keeps the exact PackageService
/// instance immutably borrowed across a caller's synchronous use operation.
/// Holding it prevents safe Rust code from uninstalling/reinstalling through
/// `&mut PackageService` until the lease is dropped.
pub struct CurrentSnapshotLease<'a> {
    inventory: &'a PackageService,
    snapshot: PackageInventorySnapshot,
}

impl<'a> CurrentSnapshotLease<'a> {
    pub const fn snapshot(&self) -> &PackageInventorySnapshot {
        &self.snapshot
    }

    pub const fn inventory(&self) -> &'a PackageService {
        self.inventory
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum SnapshotError {
    InvalidPackage(PackageError),
    IdentityMismatch { actual: AppId, expected: AppId },
    Provider(PackageProviderError),
    NotInstalled,
    ManifestChanged,
    StaleGeneration { captured: u64, current: u64 },
    GenerationSaturated,
}

/// Read-only supplier candidate backed by the existing PackageService.
/// PackageSecurityProvider verifies the supplied XAPP signature and matches
/// its logical identity to inventory; neither signature nor snapshot implies
/// Model consent/terms or byte-identical executable storage.
pub struct PackageInventorySnapshotSupplier<'a> {
    trusted_key: Option<&'a [u8; 32]>,
    inventory: &'a PackageService,
}

impl<'a> PackageInventorySnapshotSupplier<'a> {
    pub const fn new(trusted_key: Option<&'a [u8; 32]>, inventory: &'a PackageService) -> Self {
        Self {
            trusted_key,
            inventory,
        }
    }

    /// Authenticate the supplied XAPP, check the exact expected manifest ID,
    /// and capture the corresponding live inventory generation without writes.
    pub fn capture(
        &self,
        bytes: &[u8],
        expected_id: &[u8],
    ) -> Result<PackageInventorySnapshot, SnapshotError> {
        let package = PackageView::parse(bytes).map_err(SnapshotError::InvalidPackage)?;
        let manifest = package.manifest();
        let actual = manifest.app_id();
        let expected = AppId::from_identifier(expected_id);
        if manifest.id() != expected_id {
            return Err(SnapshotError::IdentityMismatch { actual, expected });
        }
        let manifest = linux_package_identity::with_expected_app_id(actual, expected, || manifest)
            .map_err(|actual| SnapshotError::IdentityMismatch { actual, expected })?;

        let inspection = PackageSecurityProvider::new(self.trusted_key, self.inventory)
            .inspect(bytes)
            .map_err(SnapshotError::Provider)?;
        let generation = match inspection.installation {
            InstallationState::Installed { generation } => generation,
            InstallationState::NotInstalled => return Err(SnapshotError::NotInstalled),
        };
        if generation == u64::MAX {
            return Err(SnapshotError::GenerationSaturated);
        }
        let verified_manifest = inspection.verified_package.manifest();
        if verified_manifest != manifest {
            return Err(SnapshotError::ManifestChanged);
        }

        Ok(PackageInventorySnapshot {
            manifest: verified_manifest,
            generation,
        })
    }

    /// Point-in-time check only. It does not keep the catalog borrowed after
    /// returning; callers must use `lease_current` to guard a following
    /// synchronous operation against same-instance mutation.
    pub fn validate_current(
        &self,
        snapshot: &PackageInventorySnapshot,
    ) -> Result<(), SnapshotError> {
        if snapshot.generation == u64::MAX {
            return Err(SnapshotError::GenerationSaturated);
        }
        // Read and validate the complete live inventory before selecting the
        // target entry; do not accept a cached/partial view of the catalog.
        let placeholder = InstalledPackage {
            manifest: snapshot.manifest,
            generation: snapshot.generation,
        };
        let mut entries = [placeholder; MAX_SLOTS];
        let count = PackageSecurityProvider::new(self.trusted_key, self.inventory)
            .list_installed(&mut entries)
            .map_err(SnapshotError::Provider)?;
        let installed = entries[..count]
            .iter()
            .find(|record| record.manifest.app_id() == snapshot.app_id())
            .copied()
            .ok_or(SnapshotError::NotInstalled)?;
        if installed.manifest.id() != snapshot.manifest.id() {
            return Err(SnapshotError::ManifestChanged);
        }
        if installed.generation != snapshot.generation {
            return Err(SnapshotError::StaleGeneration {
                captured: snapshot.generation,
                current: installed.generation,
            });
        }
        if installed.manifest != snapshot.manifest {
            return Err(SnapshotError::ManifestChanged);
        }
        Ok(())
    }

    /// Validate the snapshot and return a borrow of the same PackageService
    /// instance. Keep this lease alive across the operation that consumes the
    /// snapshot. This is an in-process Rust borrow guarantee, not an IPC lease.
    pub fn lease_current(
        &self,
        snapshot: &PackageInventorySnapshot,
    ) -> Result<CurrentSnapshotLease<'a>, SnapshotError> {
        self.validate_current(snapshot)?;
        Ok(CurrentSnapshotLease {
            inventory: self.inventory,
            snapshot: *snapshot,
        })
    }
}

const MAX_TASK_LOCAL_COMMIT_RECORDS: usize = 32;

/// A fixed request for one bounded task-local commit-record operation.
/// `record_id` must be nonzero and unique in the retained record table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationBoundOperationRequest {
    snapshot: OwnedPackageInventorySnapshot,
    record_id: u64,
}

impl GenerationBoundOperationRequest {
    pub const fn new(snapshot: OwnedPackageInventorySnapshot, record_id: u64) -> Self {
        Self {
            snapshot,
            record_id,
        }
    }
}

/// Historical result of the fixed task-local record operation. Its generation
/// identifies the package generation at commit time; it is not a current lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationBoundCommitReceipt {
    record_id: u64,
    snapshot: OwnedPackageInventorySnapshot,
}

impl GenerationBoundCommitReceipt {
    pub const fn record_id(self) -> u64 {
        self.record_id
    }

    pub const fn app_id(self) -> AppId {
        self.snapshot.app_id()
    }

    pub const fn generation(self) -> u64 {
        self.snapshot.generation()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum PackageOwnerError {
    Unavailable,
    Busy,
    OwnerMismatch,
    Package(PackageError),
    Snapshot(SnapshotError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum GenerationBoundCommitError {
    Unavailable,
    Busy,
    OwnerMismatch,
    Snapshot(SnapshotError),
    InvalidRecordId,
    DuplicateRecordId(u64),
    RecordCapacity,
}

struct PackageOwnerState {
    inventory: PackageService,
    records: [Option<GenerationBoundCommitReceipt>; MAX_TASK_LOCAL_COMMIT_RECORDS],
}

/// Task-local candidate for an owner-side generation transaction. The commit
/// region has fixed work: validate the bounded PackageService inventory, scan
/// a fixed-size record table, then publish one receipt with a single infallible
/// assignment. No caller code, callback, allocation, blocking wait, or
/// post-publication fallible step runs in the transaction. A second/reentrant
/// commit attempt while busy is refused immediately. Ordinary owner operations
/// serialize with one another; while a dispatch lease is live, new leases and
/// generation-changing operations fail immediately with `Busy`.
///
/// The receipt table is only this candidate's bounded local effect. This is not
/// a production IPC endpoint or an application effect authority; a separate
/// process cannot share its lock. Until a real package-service owner implements
/// the equivalent transaction, callers must use
/// `UnavailablePackageOperationOwner` and fail closed.
pub struct SerializedPackageServiceOwner<'a> {
    trusted_key: Option<&'a [u8; 32]>,
    owner_incarnation: Option<u64>,
    dispatch_lease_active: std::sync::atomic::AtomicBool,
    state: std::sync::Mutex<PackageOwnerState>,
}

fn next_package_owner_incarnation() -> Option<u64> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_add(1)
    })
    .ok()
}

impl<'a> SerializedPackageServiceOwner<'a> {
    /// Adopt an existing PackageService by value and retain that exact instance.
    /// The lifecycle owner must keep this wrapper for the service lifetime and
    /// route all install/remove operations through it; constructing a second
    /// PackageService does not join this generation domain.
    pub fn new(trusted_key: Option<&'a [u8; 32]>, inventory: PackageService) -> Self {
        Self {
            trusted_key,
            owner_incarnation: next_package_owner_incarnation(),
            dispatch_lease_active: std::sync::atomic::AtomicBool::new(false),
            state: std::sync::Mutex::new(PackageOwnerState {
                inventory,
                records: [None; MAX_TASK_LOCAL_COMMIT_RECORDS],
            }),
        }
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, PackageOwnerState>, PackageOwnerError> {
        use std::sync::atomic::Ordering;

        if self.dispatch_lease_active.load(Ordering::Acquire) {
            return Err(PackageOwnerError::Busy);
        }
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(std::sync::TryLockError::WouldBlock)
                if self.dispatch_lease_active.load(Ordering::Acquire) =>
            {
                Err(PackageOwnerError::Busy)
            }
            Err(std::sync::TryLockError::WouldBlock) => self
                .state
                .lock()
                .map_err(|_| PackageOwnerError::Unavailable),
            Err(std::sync::TryLockError::Poisoned(_)) => Err(PackageOwnerError::Unavailable),
        }
    }

    fn try_lease_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, PackageOwnerState>, PackageOwnerError> {
        use std::sync::atomic::Ordering;

        if self.dispatch_lease_active.load(Ordering::Acquire) {
            return Err(PackageOwnerError::Busy);
        }
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(std::sync::TryLockError::WouldBlock) => Err(PackageOwnerError::Busy),
            Err(std::sync::TryLockError::Poisoned(_)) => Err(PackageOwnerError::Unavailable),
        }
    }

    pub fn capture(
        &self,
        bytes: &[u8],
        expected_id: &[u8],
    ) -> Result<OwnedPackageInventorySnapshot, PackageOwnerError> {
        let trusted_key = self.trusted_key.ok_or(PackageOwnerError::Unavailable)?;
        let owner_incarnation = self
            .owner_incarnation
            .ok_or(PackageOwnerError::Unavailable)?;
        let state = self.lock_state()?;
        let inventory = PackageInventorySnapshotSupplier::new(Some(trusted_key), &state.inventory)
            .capture(bytes, expected_id)
            .map_err(PackageOwnerError::Snapshot)?;
        Ok(OwnedPackageInventorySnapshot {
            inventory,
            owner_incarnation,
        })
    }

    /// Validate a snapshot and lend the same owned PackageService instance to
    /// synchronous downstream dispatch. The guard keeps install/remove and
    /// other owner operations from changing generation until the lease drops.
    /// This is an in-process handoff, not an IPC or cross-process guarantee.
    pub fn lease_current<'owner>(
        &'owner self,
        snapshot: &OwnedPackageInventorySnapshot,
    ) -> Result<PackageServiceGenerationLease<'owner>, PackageOwnerError> {
        let trusted_key = self.trusted_key.ok_or(PackageOwnerError::Unavailable)?;
        let owner_incarnation = self
            .owner_incarnation
            .ok_or(PackageOwnerError::Unavailable)?;
        if snapshot.owner_incarnation != owner_incarnation {
            return Err(PackageOwnerError::OwnerMismatch);
        }
        let state = self.try_lease_state()?;
        PackageInventorySnapshotSupplier::new(Some(trusted_key), &state.inventory)
            .validate_current(&snapshot.inventory)
            .map_err(PackageOwnerError::Snapshot)?;
        self.dispatch_lease_active
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(PackageServiceGenerationLease {
            state,
            snapshot: *snapshot,
            dispatch_lease_active: &self.dispatch_lease_active,
        })
    }

    /// Route installation through the same owner lock used by the transaction.
    pub fn install(
        &self,
        bytes: &[u8],
        policy: InstallPolicy,
    ) -> Result<nagi_package::InstallReport, PackageOwnerError> {
        let trusted_key = self.trusted_key.ok_or(PackageOwnerError::Unavailable)?;
        let package = PackageView::parse(bytes).map_err(PackageOwnerError::Package)?;
        let mut state = self.lock_state()?;
        state
            .inventory
            .install_with_trusted_key(&package, policy, trusted_key)
            .map_err(PackageOwnerError::Package)
    }

    /// Route uninstall through the same owner lock used by the transaction.
    pub fn remove(&self, app_id: AppId) -> Result<(), PackageOwnerError> {
        let mut state = self.lock_state()?;
        state
            .inventory
            .remove(app_id)
            .map_err(PackageOwnerError::Package)
    }

    /// Commit one fixed, bounded local receipt after revalidating the complete
    /// live inventory. Every fallible check precedes the single receipt write;
    /// therefore an error cannot be returned after this operation publishes a
    /// receipt. Later operations outside this method do not roll it back.
    pub fn commit_record_if_current(
        &self,
        request: GenerationBoundOperationRequest,
    ) -> Result<GenerationBoundCommitReceipt, GenerationBoundCommitError> {
        let Some(trusted_key) = self.trusted_key else {
            return Err(GenerationBoundCommitError::Unavailable);
        };
        if request.record_id == 0 {
            return Err(GenerationBoundCommitError::InvalidRecordId);
        }
        let Some(owner_incarnation) = self.owner_incarnation else {
            return Err(GenerationBoundCommitError::Unavailable);
        };
        if request.snapshot.owner_incarnation != owner_incarnation {
            return Err(GenerationBoundCommitError::OwnerMismatch);
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(GenerationBoundCommitError::Busy)
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(GenerationBoundCommitError::Unavailable)
            }
        };
        {
            PackageInventorySnapshotSupplier::new(Some(trusted_key), &state.inventory)
                .validate_current(&request.snapshot.inventory)
                .map_err(GenerationBoundCommitError::Snapshot)?;
        }
        if state
            .records
            .iter()
            .flatten()
            .any(|record| record.record_id == request.record_id)
        {
            return Err(GenerationBoundCommitError::DuplicateRecordId(
                request.record_id,
            ));
        }
        let slot = state
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(GenerationBoundCommitError::RecordCapacity)?;
        let receipt = GenerationBoundCommitReceipt {
            record_id: request.record_id,
            snapshot: request.snapshot,
        };

        // This is the operation's only side effect. There are no fallible
        // operations after publication.
        state.records[slot] = Some(receipt);
        Ok(receipt)
    }
}

/// Read-only borrow of the exact PackageService instance retained by its
/// lifecycle owner. Keep the lease alive through synchronous dispatch; dropping
/// it releases the owner gate and permits generation-changing operations.
pub struct PackageServiceGenerationLease<'owner> {
    state: std::sync::MutexGuard<'owner, PackageOwnerState>,
    snapshot: OwnedPackageInventorySnapshot,
    dispatch_lease_active: &'owner std::sync::atomic::AtomicBool,
}

impl Drop for PackageServiceGenerationLease<'_> {
    fn drop(&mut self) {
        self.dispatch_lease_active
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

impl PackageServiceGenerationLease<'_> {
    pub const fn snapshot(&self) -> &OwnedPackageInventorySnapshot {
        &self.snapshot
    }

    pub fn inventory(&self) -> &PackageService {
        &self.state.inventory
    }
}

/// Session-lifetime owner that takes the exact preinitialized PackageService
/// instance and lends its inventory to a synchronous dispatcher under a
/// generation lease. It does not create a second PackageService or own process
/// startup/shutdown.
///
/// Ending the session while a dispatch borrow is live is rejected by Rust's
/// borrow checker:
///
/// ```compile_fail
/// use nagi_dev_service_catalog_consumer::PackageServiceSessionOwner;
/// use nagi_package::PackageService;
///
/// let session = PackageServiceSessionOwner::new(None, PackageService::new());
/// let snapshot = session.capture(&[], b"expected").unwrap();
/// let dispatch_lease = session.lease_for_dispatch(&snapshot).unwrap();
/// session.end_session();
/// drop(dispatch_lease);
/// ```
pub struct PackageServiceSessionOwner<'owner> {
    package_owner: SerializedPackageServiceOwner<'owner>,
}

impl<'owner> PackageServiceSessionOwner<'owner> {
    /// Move the exact already initialized service into the session owner.
    pub fn new(trusted_key: Option<&'owner [u8; 32]>, service: PackageService) -> Self {
        Self {
            package_owner: SerializedPackageServiceOwner::new(trusted_key, service),
        }
    }

    pub fn capture(
        &self,
        bytes: &[u8],
        expected_id: &[u8],
    ) -> Result<OwnedPackageInventorySnapshot, PackageOwnerError> {
        self.package_owner.capture(bytes, expected_id)
    }

    /// Borrow the retained PackageService through the owner-bound generation
    /// validator. Keep the returned guard through the downstream dispatch.
    pub fn lease_for_dispatch<'session>(
        &'session self,
        snapshot: &OwnedPackageInventorySnapshot,
    ) -> Result<PackageServiceGenerationLease<'session>, PackageOwnerError> {
        self.package_owner.lease_current(snapshot)
    }

    pub fn install(
        &self,
        bytes: &[u8],
        policy: InstallPolicy,
    ) -> Result<nagi_package::InstallReport, PackageOwnerError> {
        self.package_owner.install(bytes, policy)
    }

    pub fn remove(&self, app_id: AppId) -> Result<(), PackageOwnerError> {
        self.package_owner.remove(app_id)
    }

    pub fn commit_record_if_current(
        &self,
        request: GenerationBoundOperationRequest,
    ) -> Result<GenerationBoundCommitReceipt, GenerationBoundCommitError> {
        self.package_owner.commit_record_if_current(request)
    }

    /// Consume and drop the session-owned PackageService after the lifecycle
    /// owner has performed its separate process shutdown/revocation work.
    pub fn end_session(self) {}
}

/// Fail-closed stand-in used until a production package-service IPC route is
/// connected. It never publishes a record.
pub struct UnavailablePackageOperationOwner;

impl UnavailablePackageOperationOwner {
    pub fn commit_record_if_current(
        &self,
        _request: GenerationBoundOperationRequest,
    ) -> Result<GenerationBoundCommitReceipt, GenerationBoundCommitError> {
        Err(GenerationBoundCommitError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_init_package_provider::package_trust::InventoryReadError;
    use nagi_package::{
        build_xapp, InstalledPackage, MAX_PACKAGE_BYTES, MAX_SLOTS, TRUSTED_SIGNING_PUBLIC_KEY,
    };
    use std::path::Path;
    use std::{env, fs};

    const EXPECTED_ID: &[u8] = b"org.nagi.acceptance.isolated-app";
    const OTHER_ID: &[u8] = b"org.nagi.acceptance.other-app";
    const OTHER_ID_MANIFEST: &[u8] =
        b"id=org.nagi.acceptance.other-app\nname=Other app\nversion=1.0.0\nentry=nagi-isolated-app\n";
    const REPLACEMENT_MANIFEST: &[u8] = b"id=org.nagi.acceptance.isolated-app\nname=Unsigned replacement\nversion=0.2.0\nentry=nagi-isolated-app\n";

    fn package_bytes() -> Vec<u8> {
        let path = env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the task-local signed XAPP");
        fs::read(path).expect("read task-local signed XAPP")
    }

    fn unsigned_package_bytes(manifest: &[u8]) -> Vec<u8> {
        let package_path = env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the task-local signed XAPP");
        let artifact_dir = Path::new(&package_path)
            .parent()
            .expect("signed XAPP has an artifact directory");
        let executable =
            fs::read(artifact_dir.join("nagi-isolated-app")).expect("read existing Linux ELF");
        let mut bytes = vec![0; MAX_PACKAGE_BYTES];
        let length = build_xapp(manifest, &executable, &[], &[], &[], &[], &mut bytes)
            .expect("build structurally valid unsigned XAPP through the existing producer");
        bytes.truncate(length);
        bytes
    }

    fn assert_empty(inventory: &PackageService) {
        let mut slots = [None; MAX_SLOTS];
        assert_eq!(inventory.list_inventory(&mut slots), Ok(0));
        assert!(slots.iter().all(Option::is_none));
    }

    fn inventory_snapshot(inventory: &PackageService) -> Vec<InstalledPackage> {
        let mut slots = [None; MAX_SLOTS];
        let count = inventory
            .list_inventory(&mut slots)
            .expect("read installed catalog snapshot");
        assert!(slots[..count].iter().all(Option::is_some));
        assert!(slots[count..].iter().all(Option::is_none));
        slots[..count].iter().copied().flatten().collect()
    }

    fn error_of<T>(result: Result<T, ConsumerError>) -> ConsumerError {
        match result {
            Ok(_) => panic!("expected consumer failure"),
            Err(error) => error,
        }
    }

    struct DuplicateIdentityInventory([InstalledPackage; 2]);

    impl InstalledPackageInventory for DuplicateIdentityInventory {
        fn list_inventory(
            &self,
            output: &mut [Option<InstalledPackage>],
        ) -> Result<usize, InventoryReadError> {
            if output.len() < self.0.len() {
                return Err(InventoryReadError::Failed);
            }
            output[..self.0.len()].copy_from_slice(&self.0.map(Some));
            Ok(self.0.len())
        }
    }

    #[test]
    fn production_package_provider_reports_exact_signed_app_as_installed() {
        let bytes = package_bytes();
        let mut inventory = PackageService::new();
        let inspection = install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("install then inspect with existing production APIs");

        assert_eq!(inspection.verified_package.manifest().id(), EXPECTED_ID);
        assert_eq!(
            inspection.installation,
            InstallationState::Installed { generation: 1 }
        );
    }

    #[test]
    fn wrong_expected_id_is_rejected_before_catalog_mutation() {
        let bytes = package_bytes();
        let mut inventory = PackageService::new();
        assert!(matches!(
            install_and_inspect_development_candidate(
                &bytes,
                b"org.nagi.acceptance.faulting-app",
                &TRUSTED_SIGNING_PUBLIC_KEY,
                &mut inventory,
            ),
            Err(ConsumerError::IdentityMismatch { .. })
        ));
        assert_empty(&inventory);
    }

    #[test]
    fn unsigned_same_identity_is_rejected_without_replacing_existing_entry() {
        let signed_bytes = package_bytes();
        let mut inventory = PackageService::new();
        install_and_inspect_development_candidate(
            &signed_bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("seed the non-empty catalog with the valid signed package");
        let before = inventory_snapshot(&inventory);
        assert_eq!(before.len(), 1);
        let existing = before[0];
        assert_eq!(existing.manifest.id(), EXPECTED_ID);
        assert_eq!(existing.generation, 1);

        let unsigned_replacement = unsigned_package_bytes(REPLACEMENT_MANIFEST);
        let result = install_and_inspect_development_candidate(
            &unsigned_replacement,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        );

        assert_eq!(
            error_of(result),
            ConsumerError::Install(PackageError::SignatureRequired)
        );
        assert_eq!(inventory_snapshot(&inventory), before);
        assert_eq!(inventory.info(existing.manifest.app_id()), Ok(existing));
    }

    #[test]
    fn unsigned_different_identity_is_rejected_without_adding_a_catalog_entry() {
        let signed_bytes = package_bytes();
        let mut inventory = PackageService::new();
        install_and_inspect_development_candidate(
            &signed_bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("seed the non-empty catalog with the valid signed package");
        let before = inventory_snapshot(&inventory);
        let existing = before[0];
        assert_ne!(existing.manifest.app_id(), AppId::from_identifier(OTHER_ID));

        let unsigned_other = unsigned_package_bytes(OTHER_ID_MANIFEST);
        let result = install_and_inspect_development_candidate(
            &unsigned_other,
            OTHER_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        );

        assert_eq!(
            error_of(result),
            ConsumerError::Install(PackageError::SignatureRequired)
        );
        assert_eq!(inventory_snapshot(&inventory), before);
        assert_eq!(inventory.info(existing.manifest.app_id()), Ok(existing));
        assert_eq!(
            inventory.info(AppId::from_identifier(OTHER_ID)),
            Err(PackageError::NotInstalled)
        );
    }

    #[test]
    fn valid_same_identity_reinstall_replaces_one_entry_and_advances_generation() {
        let bytes = package_bytes();
        let mut inventory = PackageService::new();
        let first = install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("install first signed version");
        assert_eq!(
            first.installation,
            InstallationState::Installed { generation: 1 }
        );

        let replacement = install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("reinstall the same valid identity");
        assert_eq!(
            replacement.installation,
            InstallationState::Installed { generation: 2 }
        );

        let entries = inventory_snapshot(&inventory);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.id(), EXPECTED_ID);
        assert_eq!(entries[0].generation, 2);
    }

    #[test]
    fn provider_rejects_duplicate_existing_app_id_as_corrupt_inventory() {
        let bytes = package_bytes();
        let mut service = PackageService::new();
        install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut service,
        )
        .expect("seed an existing valid identity");
        let installed = inventory_snapshot(&service)[0];
        let duplicate = InstalledPackage {
            manifest: installed.manifest,
            generation: installed.generation + 1,
        };
        let corrupt = DuplicateIdentityInventory([installed, duplicate]);

        assert!(matches!(
            PackageSecurityProvider::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &corrupt)
                .inspect(&bytes),
            Err(PackageProviderError::InventoryCorrupt)
        ));
    }

    #[test]
    fn snapshot_supplier_rejects_stale_generation_after_remove_and_reinstall() {
        let bytes = package_bytes();
        let mut inventory = PackageService::new();
        install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("install the signed package");

        let snapshot = {
            let supplier = PackageInventorySnapshotSupplier::new(
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                &inventory,
            );
            supplier
                .capture(&bytes, EXPECTED_ID)
                .expect("capture signed package's live inventory generation")
        };
        assert_eq!(snapshot.app_id(), AppId::from_identifier(EXPECTED_ID));
        assert_eq!(snapshot.manifest_id(), EXPECTED_ID);
        assert_eq!(snapshot.generation(), 1);
        let supplier =
            PackageInventorySnapshotSupplier::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &inventory);
        let lease = supplier
            .lease_current(&snapshot)
            .expect("lease a fresh snapshot before synchronous use");
        assert_eq!(lease.snapshot().generation(), 1);
        assert!(std::ptr::eq(lease.inventory(), &inventory));
        assert_eq!(
            lease
                .inventory()
                .info(snapshot.app_id())
                .unwrap()
                .generation,
            1
        );
        drop(lease);
        supplier
            .validate_current(&snapshot)
            .expect("point-in-time check passes before the inventory changes");
        drop(supplier);

        inventory
            .remove(snapshot.app_id())
            .expect("uninstall package");
        assert_eq!(
            PackageInventorySnapshotSupplier::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &inventory)
                .validate_current(&snapshot),
            Err(SnapshotError::NotInstalled)
        );

        install_and_inspect_development_candidate(
            &bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("reinstall the signed package");
        let supplier =
            PackageInventorySnapshotSupplier::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &inventory);
        assert_eq!(
            supplier.validate_current(&snapshot),
            Err(SnapshotError::StaleGeneration {
                captured: 1,
                current: 2,
            })
        );
        assert!(matches!(
            supplier.lease_current(&snapshot),
            Err(SnapshotError::StaleGeneration {
                captured: 1,
                current: 2,
            })
        ));
    }

    #[test]
    fn snapshot_supplier_rejects_expected_id_mismatch_without_writes() {
        let bytes = package_bytes();
        let inventory = PackageService::new();
        let supplier =
            PackageInventorySnapshotSupplier::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &inventory);

        assert_eq!(
            supplier.capture(&bytes, OTHER_ID),
            Err(SnapshotError::IdentityMismatch {
                actual: AppId::from_identifier(EXPECTED_ID),
                expected: AppId::from_identifier(OTHER_ID),
            })
        );
        assert_eq!(
            supplier.capture(&bytes, EXPECTED_ID),
            Err(SnapshotError::NotInstalled)
        );
        assert_empty(&inventory);
    }

    #[test]
    fn snapshot_supplier_rejects_unsigned_package_without_inventory_mutation() {
        let signed_bytes = package_bytes();
        let mut inventory = PackageService::new();
        install_and_inspect_development_candidate(
            &signed_bytes,
            EXPECTED_ID,
            &TRUSTED_SIGNING_PUBLIC_KEY,
            &mut inventory,
        )
        .expect("seed the catalog with a valid signed package");
        let before = inventory_snapshot(&inventory);
        let unsigned = unsigned_package_bytes(REPLACEMENT_MANIFEST);
        let supplier =
            PackageInventorySnapshotSupplier::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), &inventory);

        assert_eq!(
            supplier.capture(&unsigned, EXPECTED_ID),
            Err(SnapshotError::Provider(
                PackageProviderError::UnsignedPackage
            ))
        );
        assert_eq!(inventory_snapshot(&inventory), before);
    }

    #[test]
    fn provider_rejects_signature_tampering_after_identity_parse() {
        let mut bytes = package_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let inventory = PackageService::new();
        assert_eq!(
            error_of(inspect_expected(
                &bytes,
                EXPECTED_ID,
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                &inventory,
            )),
            ConsumerError::Provider(PackageProviderError::UntrustedSigner)
        );
        assert_empty(&inventory);
    }

    #[test]
    fn missing_trust_is_distinct_from_an_uninstalled_package() {
        let bytes = package_bytes();
        let empty = PackageService::new();
        assert_eq!(
            error_of(inspect_expected(&bytes, EXPECTED_ID, None, &empty)),
            ConsumerError::Provider(PackageProviderError::TrustUnavailable)
        );

        assert_eq!(
            inspect_expected(
                &bytes,
                EXPECTED_ID,
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                &empty,
            )
            .unwrap()
            .installation,
            InstallationState::NotInstalled
        );
    }

    #[test]
    fn generation_bound_owner_rejects_stale_commit_without_side_effect() {
        let bytes = package_bytes();
        let owner = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install signed package through the owner");
        let snapshot = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation through the owner");
        assert_eq!(snapshot.generation(), 1);

        owner
            .remove(snapshot.app_id())
            .expect("uninstall through the owner");
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("reinstall through the owner");

        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 41)),
            Err(GenerationBoundCommitError::Snapshot(
                SnapshotError::StaleGeneration {
                    captured: 1,
                    current: 2,
                },
            ))
        );
        let state = owner.state.lock().unwrap();
        assert!(state.records.iter().all(Option::is_none));
        assert_eq!(
            state.inventory.info(snapshot.app_id()).unwrap().generation,
            2
        );
    }

    #[test]
    fn lifecycle_owner_lends_the_adopted_service_and_rejects_after_reinstall() {
        let bytes = package_bytes();
        let package = PackageView::parse(&bytes).expect("parse the existing signed XAPP");
        let mut initialized_service = PackageService::new();
        initialized_service
            .install_with_trusted_key(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
                &TRUSTED_SIGNING_PUBLIC_KEY,
            )
            .expect("initialize the concrete PackageService before owner handoff");
        assert_eq!(
            initialized_service
                .info(package.manifest().app_id())
                .unwrap()
                .generation,
            1
        );

        // Move the already initialized instance into the lifecycle owner;
        // subsequent capture, borrow, install, and removal use that owner.
        let session_owner =
            PackageServiceSessionOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), initialized_service);
        let snapshot = session_owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation from the adopted service");
        let lease = session_owner
            .lease_for_dispatch(&snapshot)
            .expect("borrow the same owner service for dispatch");
        assert_eq!(lease.snapshot(), &snapshot);
        assert_eq!(
            lease
                .inventory()
                .info(snapshot.app_id())
                .unwrap()
                .generation,
            snapshot.generation()
        );
        let inventory_before_busy_install = inventory_snapshot(lease.inventory());
        assert_eq!(inventory_before_busy_install.len(), 1);
        assert_eq!(inventory_before_busy_install[0].generation, 1);
        assert_eq!(
            session_owner.install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            ),
            Err(PackageOwnerError::Busy)
        );
        assert_eq!(
            inventory_snapshot(lease.inventory()),
            inventory_before_busy_install,
            "a rejected install must leave the borrowed inventory unchanged"
        );
        assert_eq!(
            session_owner.remove(snapshot.app_id()),
            Err(PackageOwnerError::Busy)
        );
        assert_eq!(
            session_owner.lease_for_dispatch(&snapshot).err(),
            Some(PackageOwnerError::Busy)
        );
        assert_eq!(
            session_owner
                .commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 91)),
            Err(GenerationBoundCommitError::Busy)
        );
        drop(lease);

        session_owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install into the same retained service after lease release");
        assert_eq!(
            session_owner
                .capture(&bytes, EXPECTED_ID)
                .expect("capture after successful replacement install")
                .generation(),
            2
        );
        assert_eq!(
            session_owner.lease_for_dispatch(&snapshot).err(),
            Some(PackageOwnerError::Snapshot(
                SnapshotError::StaleGeneration {
                    captured: 1,
                    current: 2,
                }
            ))
        );
        session_owner.end_session();
    }

    #[test]
    fn owner_rejects_snapshot_replayed_from_another_package_service_instance() {
        let bytes = package_bytes();
        let policy = InstallPolicy {
            developer_mode: true,
            require_signature: true,
        };
        let first = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        first.install(&bytes, policy).unwrap();
        let foreign_snapshot = first
            .capture(&bytes, EXPECTED_ID)
            .expect("capture first owner generation");
        assert_eq!(foreign_snapshot.generation(), 1);

        let second = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        second.install(&bytes, policy).unwrap();
        assert_eq!(
            second.lease_current(&foreign_snapshot).err(),
            Some(PackageOwnerError::OwnerMismatch)
        );
        assert_eq!(
            second.commit_record_if_current(GenerationBoundOperationRequest::new(
                foreign_snapshot,
                92,
            )),
            Err(GenerationBoundCommitError::OwnerMismatch)
        );
    }

    #[test]
    fn generation_bound_owner_refuses_busy_reentrant_transaction_without_mutation() {
        let bytes = package_bytes();
        let owner = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install signed package through the owner");
        let snapshot = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation through the owner");

        // Holding the owner transaction lock models a nested/reentrant caller.
        // The fixed operation has no callback path, and try_lock refuses this
        // second entry immediately instead of deadlocking.
        let state = owner.state.lock().unwrap();
        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 51)),
            Err(GenerationBoundCommitError::Busy)
        );
        assert!(state.records.iter().all(Option::is_none));
        assert_eq!(
            state.inventory.info(snapshot.app_id()).unwrap().generation,
            1
        );
    }

    #[test]
    fn generation_bound_receipt_and_failures_remain_generation_consistent() {
        let bytes = package_bytes();
        let owner = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install signed package through the owner");
        let snapshot = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation through the owner");

        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 61)),
            Ok(GenerationBoundCommitReceipt {
                record_id: 61,
                snapshot,
            })
        );
        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 61)),
            Err(GenerationBoundCommitError::DuplicateRecordId(61))
        );
        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 0)),
            Err(GenerationBoundCommitError::InvalidRecordId)
        );
        let additional_records = MAX_TASK_LOCAL_COMMIT_RECORDS - 1;
        for record_id in 100..(100 + additional_records as u64) {
            owner
                .commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, record_id))
                .expect("fill the bounded task-local record table");
        }
        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 999)),
            Err(GenerationBoundCommitError::RecordCapacity)
        );
        {
            let state = owner.state.lock().unwrap();
            let records = state.records.iter().flatten().copied().collect::<Vec<_>>();
            assert_eq!(records.len(), MAX_TASK_LOCAL_COMMIT_RECORDS);
            assert_eq!(records[0].record_id(), 61);
            assert!(records
                .iter()
                .all(|record| record.app_id() == snapshot.app_id() && record.generation() == 1));
            assert_eq!(
                state.inventory.info(snapshot.app_id()).unwrap().generation,
                1
            );
        }

        owner.remove(snapshot.app_id()).unwrap();
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .unwrap();
        assert_eq!(
            owner.commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 62)),
            Err(GenerationBoundCommitError::Snapshot(
                SnapshotError::StaleGeneration {
                    captured: 1,
                    current: 2,
                }
            ))
        );
        let state = owner.state.lock().unwrap();
        let records = state.records.iter().flatten().copied().collect::<Vec<_>>();
        assert_eq!(records.len(), MAX_TASK_LOCAL_COMMIT_RECORDS);
        assert!(records.iter().all(|record| record.generation() == 1));
        assert_eq!(
            state.inventory.info(snapshot.app_id()).unwrap().generation,
            2
        );
    }

    #[test]
    fn generation_bound_owner_race_with_install_never_commits_stale_generation() {
        use std::sync::{Arc, Barrier};

        let bytes = package_bytes();
        let owner = Arc::new(SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        ));
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install signed package through the owner");
        let snapshot = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation through the owner");

        let start = Arc::new(Barrier::new(3));
        let commit_owner = Arc::clone(&owner);
        let commit_start = Arc::clone(&start);
        let commit = std::thread::spawn(move || {
            commit_start.wait();
            commit_owner
                .commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 71))
        });

        let install_owner = Arc::clone(&owner);
        let install_bytes = bytes.clone();
        let install_start = Arc::clone(&start);
        let install = std::thread::spawn(move || {
            install_start.wait();
            install_owner.install(
                &install_bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
        });
        start.wait();
        let commit_result = commit.join().expect("commit thread exits");
        install
            .join()
            .expect("install thread exits")
            .expect("serialized reinstall succeeds after commit");
        let current = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("read post-install generation");
        assert_eq!(current.generation(), 2);
        let state = owner.state.lock().unwrap();
        let records = state.records.iter().flatten().copied().collect::<Vec<_>>();
        match commit_result {
            Ok(receipt) => {
                assert_eq!(receipt.generation(), 1);
                assert_eq!(records, [receipt]);
            }
            Err(GenerationBoundCommitError::Busy)
            | Err(GenerationBoundCommitError::Snapshot(SnapshotError::StaleGeneration {
                captured: 1,
                current: 2,
            })) => assert!(records.is_empty()),
            other => panic!("unexpected concurrent commit result: {other:?}"),
        }
    }

    #[test]
    fn unavailable_package_owner_does_not_publish_a_record() {
        let bytes = package_bytes();
        let owner = SerializedPackageServiceOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &bytes,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install signed package through the task-local owner");
        let snapshot = owner
            .capture(&bytes, EXPECTED_ID)
            .expect("capture generation through the task-local owner");

        assert_eq!(
            UnavailablePackageOperationOwner
                .commit_record_if_current(GenerationBoundOperationRequest::new(snapshot, 81)),
            Err(GenerationBoundCommitError::Unavailable)
        );
        let state = owner.state.lock().unwrap();
        assert!(state.records.iter().all(Option::is_none));
        assert_eq!(
            state.inventory.info(snapshot.app_id()).unwrap().generation,
            1
        );
    }
}
