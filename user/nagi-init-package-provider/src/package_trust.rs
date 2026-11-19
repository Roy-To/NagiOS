//! Fail-closed package authentication and installed-inventory adapter for the
//! Linux `nagi-init` backend. Trust and inventory are supplied by the caller;
//! this module does not provision a product key, invent consent, or fake READY.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use nagi_model::AppId;
use nagi_package::{
    InstallPolicy, InstalledPackage, PackageError, PackageManifest, PackageService, PackageView,
    MAX_PACKAGE_BYTES, MAX_SLOTS,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledPackageDigestIndexError {
    Unavailable,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledPackageBlobError {
    Unavailable,
    Failed,
    OutcomeUnknown,
}

pub(crate) mod durable_storage_sealed {
    pub trait DigestIndex {}
    pub trait PackageBlobs {}
}

/// Commit evidence for one immutable content-addressed package blob.
pub struct InstalledPackageBlobCommit {
    sha256: [u8; 32],
    bytes_len: u32,
}

impl InstalledPackageBlobCommit {
    pub(crate) fn after_durable_insert(sha256: [u8; 32], bytes_len: usize) -> Self {
        Self {
            sha256,
            bytes_len: bytes_len as u32,
        }
    }

    pub(crate) fn matches(&self, sha256: &[u8; 32], bytes_len: usize) -> bool {
        &self.sha256 == sha256 && self.bytes_len as usize == bytes_len
    }
}

/// Read evidence for one immutable package blob. Callers still hash the bytes
/// they received and compare them with both this record and the digest index.
pub struct InstalledPackageBlobReadback {
    sha256: [u8; 32],
    bytes_len: u32,
}

impl InstalledPackageBlobReadback {
    pub(crate) fn after_durable_read(sha256: [u8; 32], bytes_len: usize) -> Self {
        Self {
            sha256,
            bytes_len: bytes_len as u32,
        }
    }

    pub(crate) fn matches(&self, sha256: &[u8; 32], bytes_len: usize) -> bool {
        &self.sha256 == sha256 && self.bytes_len as usize == bytes_len
    }
}

/// Immutable content-addressed package blob boundary. Implementations must
/// never replace existing bytes at a digest key and must sync before returning
/// their commit token.
pub trait InstalledPackageBlobStore: durable_storage_sealed::PackageBlobs {
    fn insert_immutable(
        &mut self,
        sha256: &[u8; 32],
        bytes: &[u8],
    ) -> Result<InstalledPackageBlobCommit, InstalledPackageBlobError>;

    fn read_immutable(
        &self,
        sha256: &[u8; 32],
        output: &mut [u8],
    ) -> Result<InstalledPackageBlobReadback, InstalledPackageBlobError>;
}

/// Evidence returned only after a concrete provider-owned adapter has
/// completed a durable whole-snapshot replacement. External in-memory
/// implementations cannot construct this value.
pub struct InstalledPackageDigestIndexCommit {
    record_count: u8,
    snapshot_sha256: [u8; 32],
}

impl InstalledPackageDigestIndexCommit {
    pub(crate) fn after_durable_replace(records: &[InstalledPackageDigestRecord]) -> Self {
        Self {
            record_count: records.len() as u8,
            snapshot_sha256: digest_index_snapshot(records),
        }
    }

    pub(crate) fn matches(&self, records: &[InstalledPackageDigestRecord]) -> bool {
        self.record_count as usize == records.len()
            && self.snapshot_sha256 == digest_index_snapshot(records)
    }
}

/// Evidence that `read_snapshot` decoded records from its durable source.
/// The owner verifies this digest against the returned fields before using it.
pub struct InstalledPackageDigestIndexReadback {
    record_count: u8,
    snapshot_sha256: [u8; 32],
}

impl InstalledPackageDigestIndexReadback {
    pub(crate) fn after_durable_read(records: &[InstalledPackageDigestRecord]) -> Self {
        Self {
            record_count: records.len() as u8,
            snapshot_sha256: digest_index_snapshot(records),
        }
    }

    pub(crate) const fn record_count(&self) -> usize {
        self.record_count as usize
    }

    pub(crate) fn matches(&self, records: &[InstalledPackageDigestRecord]) -> bool {
        self.record_count as usize == records.len()
            && self.snapshot_sha256 == digest_index_snapshot(records)
    }
}

/// Exact digest metadata for one installed package generation. This record is
/// consistency metadata only; package signature verification and the trusted
/// catalog digest remain required before a service launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledPackageDigestRecord {
    app_id: AppId,
    generation: u64,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
    signed_package_bytes_len: u32,
    executable_bytes_len: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledPackageDigestRecordError {
    InvalidGeneration,
    InvalidPackageLength,
    InvalidExecutableLength,
}

impl InstalledPackageDigestRecord {
    /// Rebuild a record decoded from the durable package index. These fields
    /// are not authority by themselves; `capture_for_system_service` still
    /// verifies the package signature and exact bytes against this generation.
    pub fn from_parts(
        app_id: AppId,
        generation: u64,
        signed_package_sha256: [u8; 32],
        executable_sha256: [u8; 32],
        signed_package_bytes_len: u32,
        executable_bytes_len: u32,
    ) -> Result<Self, InstalledPackageDigestRecordError> {
        if generation == 0 || generation == u64::MAX {
            return Err(InstalledPackageDigestRecordError::InvalidGeneration);
        }
        if signed_package_bytes_len == 0 || signed_package_bytes_len as usize > MAX_PACKAGE_BYTES {
            return Err(InstalledPackageDigestRecordError::InvalidPackageLength);
        }
        if executable_bytes_len == 0
            || executable_bytes_len > signed_package_bytes_len
            || executable_bytes_len as usize > MAX_PACKAGE_BYTES
        {
            return Err(InstalledPackageDigestRecordError::InvalidExecutableLength);
        }
        Ok(Self {
            app_id,
            generation,
            signed_package_sha256,
            executable_sha256,
            signed_package_bytes_len,
            executable_bytes_len,
        })
    }

    pub const fn app_id(self) -> AppId {
        self.app_id
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

    pub const fn signed_package_bytes_len(self) -> u32 {
        self.signed_package_bytes_len
    }

    pub const fn executable_bytes_len(self) -> u32 {
        self.executable_bytes_len
    }
}

/// Durable index boundary for the digest associated with each installed
/// generation. `replace_snapshot` must make the whole bounded snapshot durable
/// before it returns. Records are serialized by ascending `app_id`. The owner
/// then reads them back and compares every field; an implementation returning
/// a cached write buffer does not meet this contract. No production
/// implementation is selected in this crate yet.
pub trait InstalledPackageDigestIndex: durable_storage_sealed::DigestIndex {
    fn replace_snapshot(
        &mut self,
        records: &[InstalledPackageDigestRecord],
    ) -> Result<InstalledPackageDigestIndexCommit, InstalledPackageDigestIndexError>;

    fn read_snapshot(
        &self,
        output: &mut [Option<InstalledPackageDigestRecord>],
    ) -> Result<InstalledPackageDigestIndexReadback, InstalledPackageDigestIndexError>;
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
    DigestIndexUnavailable,
    DigestIndexFailed,
    DigestIndexOutcomeUnknown,
    DigestIndexCorrupt,
    DigestIndexReadbackMismatch,
    PackageBlobUnavailable,
    PackageBlobFailed,
    PackageBlobOutcomeUnknown,
    PackageBlobReadbackMismatch,
}

#[must_use = "retain the snapshot and validate it through its owning session before use"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedPackageInventorySnapshot {
    manifest: PackageManifest,
    generation: u64,
    owner_incarnation: u64,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
    signed_package_bytes_len: u32,
    executable_bytes_len: u32,
}

type InstalledPackageDigest = InstalledPackageDigestRecord;

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

    pub const fn signed_package_bytes_len(self) -> u32 {
        self.signed_package_bytes_len
    }

    pub const fn executable_bytes_len(self) -> u32 {
        self.executable_bytes_len
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
    digest_index_verified: bool,
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
            digest_index_verified: false,
        }
    }

    fn ensure_not_quarantined(&self) -> Result<(), PackageServiceSessionOwnerError> {
        if self.quarantined.load(Ordering::SeqCst) {
            Err(PackageServiceSessionOwnerError::Quarantined)
        } else {
            Ok(())
        }
    }

    /// Report the local fence without granting authority to clear it. A
    /// quarantine remains sticky until this owner can consume an exact,
    /// operation-bound terminal receipt from the C1/C3 recovery path.
    pub fn is_quarantined(&self) -> bool {
        self.quarantined.load(Ordering::SeqCst)
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
        let signed_package_bytes_len = u32::try_from(bytes.len())
            .map_err(|_| PackageServiceSessionOwnerError::DigestIndexCorrupt)?;
        let executable_bytes_len = u32::try_from(package.executable().len())
            .map_err(|_| PackageServiceSessionOwnerError::DigestIndexCorrupt)?;
        self.package_digests[digest_slot] = Some(InstalledPackageDigest {
            app_id,
            generation: installed.generation,
            signed_package_sha256: sha256(bytes),
            executable_sha256: sha256(package.executable()),
            signed_package_bytes_len,
            executable_bytes_len,
        });
        self.digest_index_verified = false;
        Ok(report)
    }

    /// Install and require a durable digest-index write followed by an exact
    /// readback before this owner can bind the generation for a system
    /// service. Any uncertain or inconsistent post-install result quarantines
    /// the owner because the package store has already changed.
    pub fn install_with_digest_index(
        &mut self,
        bytes: &[u8],
        policy: InstallPolicy,
        index: &mut dyn InstalledPackageDigestIndex,
    ) -> Result<nagi_package::InstallReport, PackageServiceSessionOwnerError> {
        let report = self.install(bytes, policy)?;
        self.replace_and_readback_digest_index(index)?;
        Ok(report)
    }

    /// Production install order: authenticate first, persist and read back the
    /// immutable package blob, mutate the in-memory PackageService, then
    /// durably replace and read back the generation digest index. A failure
    /// before inventory mutation may leave an unreferenced content-addressed
    /// blob; a failure after mutation quarantines this owner.
    pub fn install_with_persistent_storage(
        &mut self,
        bytes: &[u8],
        policy: InstallPolicy,
        blobs: &mut dyn InstalledPackageBlobStore,
        index: &mut dyn InstalledPackageDigestIndex,
    ) -> Result<nagi_package::InstallReport, PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        let trusted_key = self
            .trusted_key
            .ok_or(PackageServiceSessionOwnerError::TrustUnavailable)?;
        let package =
            PackageView::parse(bytes).map_err(PackageServiceSessionOwnerError::Package)?;
        let signed = package.is_signed_by(trusted_key);
        if (!package.signature().is_empty() && !signed)
            || (policy.require_signature && !signed)
            || (!signed && !policy.developer_mode)
        {
            return Err(PackageServiceSessionOwnerError::Package(
                if !package.signature().is_empty() && !signed {
                    PackageError::InvalidSignature
                } else {
                    PackageError::SignatureRequired
                },
            ));
        }
        let digest = sha256(bytes);
        let commit = blobs
            .insert_immutable(&digest, bytes)
            .map_err(map_package_blob_error)?;
        if !commit.matches(&digest, bytes.len()) {
            self.quarantined.store(true, Ordering::SeqCst);
            return Err(PackageServiceSessionOwnerError::PackageBlobReadbackMismatch);
        }
        let mut blob_readback = vec![0; bytes.len()];
        let readback = blobs
            .read_immutable(&digest, &mut blob_readback)
            .map_err(map_package_blob_error)?;
        if !readback.matches(&digest, bytes.len())
            || blob_readback != bytes
            || sha256(&blob_readback) != digest
        {
            self.quarantined.store(true, Ordering::SeqCst);
            return Err(PackageServiceSessionOwnerError::PackageBlobReadbackMismatch);
        }
        self.install_with_digest_index(bytes, policy, index)
    }

    /// Reconstitute the PackageService generations from the durable digest
    /// snapshot and immutable content-addressed blobs. Both index readback and
    /// each blob are verified before the new owner becomes system-service
    /// eligible.
    pub fn restore_from_persistent_storage(
        trusted_key: Option<&'key [u8; 32]>,
        policy: InstallPolicy,
        blobs: &dyn InstalledPackageBlobStore,
        index: &dyn InstalledPackageDigestIndex,
    ) -> Result<Self, PackageServiceSessionOwnerError> {
        let trusted_key = trusted_key.ok_or(PackageServiceSessionOwnerError::TrustUnavailable)?;
        let mut records = [None; MAX_SLOTS];
        let readback = index
            .read_snapshot(&mut records)
            .map_err(map_digest_index_error)?;
        let count = readback.record_count();
        if count > records.len()
            || records[..count].iter().any(Option::is_none)
            || records[count..].iter().any(Option::is_some)
        {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }
        let durable_records = records[..count]
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        if !readback.matches(&durable_records) {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }

        let mut package_blobs = Vec::with_capacity(count);
        let mut generations = Vec::with_capacity(count);
        for record in &durable_records {
            let bytes = read_record_blob(blobs, record)?;
            let package =
                PackageView::parse(&bytes).map_err(PackageServiceSessionOwnerError::Package)?;
            if package.manifest().app_id() != record.app_id
                || package.executable().len() != record.executable_bytes_len as usize
                || sha256(package.executable()) != record.executable_sha256
            {
                return Err(PackageServiceSessionOwnerError::PackageBlobReadbackMismatch);
            }
            package_blobs.push(bytes);
            generations.push(record.generation);
        }

        let entries = package_blobs
            .iter()
            .zip(generations.iter().copied())
            .map(|(bytes, generation)| (bytes.as_slice(), generation))
            .collect::<Vec<_>>();
        let inventory = PackageService::restore_with_trusted_key(&entries, policy, trusted_key)
            .map_err(PackageServiceSessionOwnerError::Package)?;
        let mut owner = Self::new(Some(trusted_key), inventory);
        owner.load_digest_records(&records, count)?;
        Ok(owner)
    }

    /// Supply the signed package bytes selected by the verified inventory.
    /// Launch still authenticates the manifest, signature and executable
    /// against the catalog and captures the exact generation lease.
    pub fn read_installed_system_service_package(
        &self,
        expected_manifest_id: &[u8],
        blobs: &dyn InstalledPackageBlobStore,
    ) -> Result<Vec<u8>, PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        if !self.digest_index_verified {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        }
        let app_id = AppId::from_identifier(expected_manifest_id);
        let record = self
            .package_digests
            .iter()
            .flatten()
            .find(|record| record.app_id == app_id)
            .ok_or(PackageServiceSessionOwnerError::NotInstalled)?;
        read_record_blob(blobs, record)
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
        self.digest_index_verified = false;
        Ok(())
    }

    /// Remove and durably read back the exact remaining package digest
    /// snapshot. An uncertain index update quarantines the owner.
    pub fn remove_with_digest_index(
        &mut self,
        app_id: AppId,
        index: &mut dyn InstalledPackageDigestIndex,
    ) -> Result<(), PackageServiceSessionOwnerError> {
        self.remove(app_id)?;
        self.replace_and_readback_digest_index(index)
    }

    /// Rehydrate generation digests from the durable index after the package
    /// inventory has been restored. Every record must match exactly one live
    /// installed app and generation. Missing, stale, duplicate, oversized, or
    /// foreign records cannot make a generation launchable.
    pub fn restore_digest_index(
        &mut self,
        index: &dyn InstalledPackageDigestIndex,
    ) -> Result<(), PackageServiceSessionOwnerError> {
        self.ensure_not_quarantined()?;
        self.digest_index_verified = false;
        let mut records = [None; MAX_SLOTS];
        let readback = index
            .read_snapshot(&mut records)
            .map_err(map_digest_index_error)?;
        let count = readback.record_count();
        if count > records.len()
            || records[..count].iter().any(Option::is_none)
            || records[count..].iter().any(Option::is_some)
        {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }
        let durable = records[..count]
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        if !readback.matches(&durable) {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }
        self.load_digest_records(&records, count)
    }

    fn replace_and_readback_digest_index(
        &mut self,
        index: &mut dyn InstalledPackageDigestIndex,
    ) -> Result<(), PackageServiceSessionOwnerError> {
        let mut expected = self
            .package_digests
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        expected.sort_by_key(|record| record.app_id);
        let result = (|| {
            let commit = index
                .replace_snapshot(&expected)
                .map_err(map_digest_index_error)?;
            if !commit.matches(&expected) {
                return Err(PackageServiceSessionOwnerError::DigestIndexReadbackMismatch);
            }
            let mut readback = [None; MAX_SLOTS];
            let receipt = index
                .read_snapshot(&mut readback)
                .map_err(map_digest_index_error)?;
            let count = receipt.record_count();
            if count > readback.len()
                || readback[..count].iter().any(Option::is_none)
                || readback[count..].iter().any(Option::is_some)
            {
                return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
            }
            let actual = readback[..count]
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            if actual != expected || !receipt.matches(&actual) {
                return Err(PackageServiceSessionOwnerError::DigestIndexReadbackMismatch);
            }
            self.load_digest_records(&readback, count)
        })();
        if result.is_err() {
            self.quarantined.store(true, Ordering::SeqCst);
            self.digest_index_verified = false;
        }
        result
    }

    fn load_digest_records(
        &mut self,
        records: &[Option<InstalledPackageDigestRecord>],
        count: usize,
    ) -> Result<(), PackageServiceSessionOwnerError> {
        if count > records.len()
            || records[..count].iter().any(Option::is_none)
            || records[count..].iter().any(Option::is_some)
        {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }
        if self.trusted_key.is_none() {
            return Err(PackageServiceSessionOwnerError::TrustUnavailable);
        }
        let mut inventory = [None; MAX_SLOTS];
        let inventory_count = self.inventory.list_inventory(&mut inventory).map_err(|_| {
            PackageServiceSessionOwnerError::Provider(PackageProviderError::InventoryUnavailable)
        })?;
        validate_inventory(&inventory, inventory_count)
            .map_err(PackageServiceSessionOwnerError::Provider)?;
        if count != inventory_count {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        }
        if records[..count]
            .windows(2)
            .any(|pair| pair[0].unwrap().app_id >= pair[1].unwrap().app_id)
        {
            return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
        }

        let mut loaded = [None; MAX_SLOTS];
        for (index, record) in records[..count].iter().flatten().copied().enumerate() {
            if record.generation == 0
                || record.generation == u64::MAX
                || record.signed_package_bytes_len == 0
                || record.signed_package_bytes_len as usize > MAX_PACKAGE_BYTES
                || record.executable_bytes_len == 0
                || record.executable_bytes_len > record.signed_package_bytes_len
                || record.executable_bytes_len as usize > MAX_PACKAGE_BYTES
                || records[..index].iter().flatten().any(|previous| {
                    previous.app_id == record.app_id || previous.generation == record.generation
                })
            {
                return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
            }
            let Some(installed) = inventory[..inventory_count]
                .iter()
                .flatten()
                .find(|installed| installed.manifest.app_id() == record.app_id)
            else {
                return Err(PackageServiceSessionOwnerError::DigestIndexCorrupt);
            };
            if installed.generation != record.generation {
                return Err(PackageServiceSessionOwnerError::StaleGeneration {
                    captured: record.generation,
                    current: installed.generation,
                });
            }
            loaded[index] = Some(record);
        }
        self.package_digests = loaded;
        self.digest_index_verified = true;
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
            || record.signed_package_bytes_len as usize != bytes.len()
            || record.executable_bytes_len as usize != parsed.executable().len()
        {
            return Err(PackageServiceSessionOwnerError::InstalledPackageBytesMismatch);
        }

        Ok(OwnedPackageInventorySnapshot {
            manifest: verified_manifest,
            generation,
            owner_incarnation,
            signed_package_sha256,
            executable_sha256,
            signed_package_bytes_len: u32::try_from(bytes.len())
                .map_err(|_| PackageServiceSessionOwnerError::DigestIndexCorrupt)?,
            executable_bytes_len: u32::try_from(parsed.executable().len())
                .map_err(|_| PackageServiceSessionOwnerError::DigestIndexCorrupt)?,
        })
    }

    /// Capture only after the durable digest index has been read back and
    /// matched to this exact installed inventory.
    pub fn capture_for_system_service(
        &self,
        bytes: &[u8],
        expected_id: &[u8],
    ) -> Result<OwnedPackageInventorySnapshot, PackageServiceSessionOwnerError> {
        if !self.digest_index_verified {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        }
        self.capture(bytes, expected_id)
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
            || record.signed_package_bytes_len != snapshot.signed_package_bytes_len
            || record.executable_bytes_len != snapshot.executable_bytes_len
        {
            return Err(PackageServiceSessionOwnerError::InstalledPackageBytesMismatch);
        }

        Ok(PackageServiceGenerationLease {
            inventory: &self.inventory,
            snapshot: *snapshot,
            quarantined: Arc::clone(&self.quarantined),
        })
    }

    /// Dispatch lease for a system-service generation. This requires the
    /// current durable digest index readback, in addition to the normal
    /// same-owner and exact-generation checks.
    pub fn lease_for_system_service_dispatch<'owner>(
        &'owner self,
        snapshot: &OwnedPackageInventorySnapshot,
    ) -> Result<PackageServiceGenerationLease<'owner>, PackageServiceSessionOwnerError> {
        if !self.digest_index_verified {
            return Err(PackageServiceSessionOwnerError::PackageDigestUnavailable);
        }
        self.lease_for_dispatch(snapshot)
    }

    /// Consume the owner after the lifecycle owner has handled process cleanup.
    pub fn end_session(self) {}
}

fn map_digest_index_error(
    error: InstalledPackageDigestIndexError,
) -> PackageServiceSessionOwnerError {
    match error {
        InstalledPackageDigestIndexError::Unavailable => {
            PackageServiceSessionOwnerError::DigestIndexUnavailable
        }
        InstalledPackageDigestIndexError::Failed => {
            PackageServiceSessionOwnerError::DigestIndexFailed
        }
        InstalledPackageDigestIndexError::OutcomeUnknown => {
            PackageServiceSessionOwnerError::DigestIndexOutcomeUnknown
        }
    }
}

fn read_record_blob(
    blobs: &dyn InstalledPackageBlobStore,
    record: &InstalledPackageDigestRecord,
) -> Result<Vec<u8>, PackageServiceSessionOwnerError> {
    let mut bytes = vec![0; record.signed_package_bytes_len as usize];
    let readback = blobs
        .read_immutable(&record.signed_package_sha256, &mut bytes)
        .map_err(map_package_blob_error)?;
    if !readback.matches(&record.signed_package_sha256, bytes.len())
        || sha256(&bytes) != record.signed_package_sha256
    {
        return Err(PackageServiceSessionOwnerError::PackageBlobReadbackMismatch);
    }
    Ok(bytes)
}

fn map_package_blob_error(error: InstalledPackageBlobError) -> PackageServiceSessionOwnerError {
    match error {
        InstalledPackageBlobError::Unavailable => {
            PackageServiceSessionOwnerError::PackageBlobUnavailable
        }
        InstalledPackageBlobError::Failed => PackageServiceSessionOwnerError::PackageBlobFailed,
        InstalledPackageBlobError::OutcomeUnknown => {
            PackageServiceSessionOwnerError::PackageBlobOutcomeUnknown
        }
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut result = [0; 32];
    result.copy_from_slice(&digest);
    result
}

fn digest_index_snapshot(records: &[InstalledPackageDigestRecord]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"NAGI-PACKAGE-DIGEST-INDEX\0\x01");
    hasher.update((records.len() as u32).to_le_bytes());
    for record in records {
        hasher.update(record.app_id.0.to_le_bytes());
        hasher.update(record.generation.to_le_bytes());
        hasher.update(record.signed_package_sha256);
        hasher.update(record.executable_sha256);
        hasher.update(record.signed_package_bytes_len.to_le_bytes());
        hasher.update(record.executable_bytes_len.to_le_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod unit_tests {
    use super::{
        durable_storage_sealed, InstalledPackageDigestIndex, InstalledPackageDigestIndexCommit,
        InstalledPackageDigestIndexError, InstalledPackageDigestIndexReadback,
        InstalledPackageDigestRecord, PackageServiceSessionOwner, PackageServiceSessionOwnerError,
    };
    use nagi_model::AppId;
    use nagi_package::PackageService;

    struct ReadbackDropsWrittenRecords;

    impl durable_storage_sealed::DigestIndex for ReadbackDropsWrittenRecords {}

    impl InstalledPackageDigestIndex for ReadbackDropsWrittenRecords {
        fn replace_snapshot(
            &mut self,
            records: &[InstalledPackageDigestRecord],
        ) -> Result<InstalledPackageDigestIndexCommit, InstalledPackageDigestIndexError> {
            Ok(InstalledPackageDigestIndexCommit::after_durable_replace(
                records,
            ))
        }

        fn read_snapshot(
            &self,
            output: &mut [Option<InstalledPackageDigestRecord>],
        ) -> Result<InstalledPackageDigestIndexReadback, InstalledPackageDigestIndexError> {
            output.fill(None);
            Ok(InstalledPackageDigestIndexReadback::after_durable_read(&[]))
        }
    }

    #[test]
    fn mismatched_digest_index_readback_quarantines_installed_owner() {
        let trusted_key = [0; 32];
        let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
        owner.package_digests[0] = Some(
            InstalledPackageDigestRecord::from_parts(
                AppId::from_identifier(b"org.nagi.fixture"),
                1,
                [1; 32],
                [2; 32],
                32,
                8,
            )
            .expect("valid orchestration fixture record"),
        );
        let mut index = ReadbackDropsWrittenRecords;

        assert_eq!(
            owner.replace_and_readback_digest_index(&mut index),
            Err(PackageServiceSessionOwnerError::DigestIndexReadbackMismatch)
        );
        assert!(owner.is_quarantined());
    }
}
