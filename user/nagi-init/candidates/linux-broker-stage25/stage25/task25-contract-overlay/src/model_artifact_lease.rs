//! Task-local consumer adapter for the existing Model Store artifact lease API.
//!
//! The boot tree has no production `ModelStoreArtifacts` owner. The production
//! candidate below therefore returns `Unavailable` for every capture. The
//! wrapper validates the owner-issued immutable lease and pins its namespace,
//! revision, manifest, and artifact identity; it does not install bytes, verify
//! their digest, record terms consent, or create an installed receipt.

use nagi_model_manager::{
    ArtifactId, ArtifactReadError, ArtifactReference, AuthenticatedArtifactLease,
    IntegrityMetadata, ModelArtifactReader, ModelManifest, ModelStoreArtifact, ModelStoreArtifacts,
    ModelStoreIdentity, ReceiptError, RuntimeError,
};

/// A lease returned by an actual owner only after its install-completion check.
/// The candidate never creates an installation record itself.
pub struct CompletedModelArtifactLease<S> {
    source: S,
    manifest: ModelManifest,
    store_identity: ModelStoreIdentity,
    revision: u64,
}

impl<S> CompletedModelArtifactLease<S>
where
    S: ModelStoreArtifact + AuthenticatedArtifactLease,
{
    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    pub const fn store_identity(&self) -> ModelStoreIdentity {
        self.store_identity
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Recheck the original owner lease immediately before using this handle.
    pub fn validate_current(&self) -> Result<(), ReceiptError> {
        if ModelStoreArtifact::store_identity(&self.source) != self.store_identity
            || ModelStoreArtifact::revision(&self.source) != self.revision
            || AuthenticatedArtifactLease::store_identity(&self.source) != self.store_identity
            || AuthenticatedArtifactLease::revision(&self.source) != self.revision
        {
            return Err(ReceiptError::Stale);
        }
        self.source.validate_lease()?;
        self.source.check(&self.manifest)?;
        if ModelStoreArtifact::store_identity(&self.source) != self.store_identity
            || ModelStoreArtifact::revision(&self.source) != self.revision
            || AuthenticatedArtifactLease::store_identity(&self.source) != self.store_identity
            || AuthenticatedArtifactLease::revision(&self.source) != self.revision
        {
            return Err(ReceiptError::Stale);
        }
        Ok(())
    }
}

/// Capture only through the Model Store owner's completed-install API. A
/// successful result pins one immutable artifact revision. The owner remains
/// responsible for proving install completion, namespace identity, and lease
/// validity. This wrapper checks the manifest/artifact shape and revalidates
/// before and after every read.
fn capture_from_owner<S>(
    owner: &mut S,
    manifest: &ModelManifest,
    store_revision: u64,
) -> Result<CompletedModelArtifactLease<S::Snapshot>, ReceiptError>
where
    S: ModelStoreArtifacts,
    S::Snapshot: AuthenticatedArtifactLease,
{
    manifest
        .validate()
        .map_err(|_| ReceiptError::InvalidManifest)?;
    let size = manifest
        .artifact
        .size_bytes
        .filter(|size| *size > 0)
        .ok_or(ReceiptError::InvalidManifest)?;
    let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;

    let source = owner.capture(manifest, store_revision)?;
    if source.artifact_id() != artifact_id || source.len() != size {
        return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
    }
    let store_identity = ModelStoreArtifact::store_identity(&source);
    let revision = ModelStoreArtifact::revision(&source);
    if AuthenticatedArtifactLease::store_identity(&source) != store_identity
        || AuthenticatedArtifactLease::revision(&source) != revision
    {
        return Err(ReceiptError::Stale);
    }
    source.validate_lease()?;
    source.check(manifest)?;
    if ModelStoreArtifact::store_identity(&source) != store_identity
        || ModelStoreArtifact::revision(&source) != revision
        || AuthenticatedArtifactLease::store_identity(&source) != store_identity
        || AuthenticatedArtifactLease::revision(&source) != revision
    {
        return Err(ReceiptError::Stale);
    }

    Ok(CompletedModelArtifactLease {
        source,
        manifest: manifest.clone(),
        store_identity,
        revision,
    })
}

/// No production Model Store owner is connected in this candidate. The public
/// construction boundary therefore cannot accept an arbitrary trait
/// implementation and always fails closed. `capture_from_owner` remains
/// private so tests can exercise the wrapper contract with synthetic bytes.
pub fn capture_completed_model_artifact(
    manifest: &ModelManifest,
    store_revision: u64,
) -> Result<CompletedModelArtifactLease<UnavailableModelArtifactLease>, ReceiptError> {
    let mut owner = UnavailableModelArtifactLeaseOwner;
    capture_from_owner(&mut owner, manifest, store_revision)
}

impl<S> ModelArtifactReader for CompletedModelArtifactLease<S>
where
    S: ModelStoreArtifact + AuthenticatedArtifactLease,
{
    fn artifact_id(&self) -> &ArtifactId {
        self.source.artifact_id()
    }

    /// A lease does not mean the bytes were hashed. Integrity verification is
    /// a separate operation owned by the existing authenticated snapshot path.
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.source.verified_integrity()
    }

    fn len(&self) -> u64 {
        self.source.len()
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        self.validate_current()
            .map_err(|_| ArtifactReadError::Unavailable)?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(destination.len())
            .map_err(|_| ArtifactReadError::Unavailable)?;
        staged.resize(destination.len(), 0);
        let count = self.source.read_at(offset, &mut staged)?;
        if count > staged.len() {
            return Err(ArtifactReadError::Unavailable);
        }
        self.validate_current()
            .map_err(|_| ArtifactReadError::Unavailable)?;
        destination[..count].copy_from_slice(&staged[..count]);
        Ok(count)
    }
}

impl<S> ModelStoreArtifact for CompletedModelArtifactLease<S>
where
    S: ModelStoreArtifact + AuthenticatedArtifactLease,
{
    fn store_identity(&self) -> ModelStoreIdentity {
        self.store_identity
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn validate_lease(&self) -> Result<(), ReceiptError> {
        self.validate_current()
    }
}

impl<S> AuthenticatedArtifactLease for CompletedModelArtifactLease<S>
where
    S: ModelStoreArtifact + AuthenticatedArtifactLease,
{
    fn store_identity(&self) -> ModelStoreIdentity {
        self.store_identity
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn check(&self, manifest: &ModelManifest) -> Result<(), ReceiptError> {
        if manifest != &self.manifest {
            return Err(ReceiptError::Stale);
        }
        self.validate_current()
    }
}

/// Fail-closed production candidate until boot supplies a real Model Store
/// owner. It does not translate PackageService inventory into Model artifacts.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableModelArtifactLeaseOwner;

/// This associated type is never issued by `UnavailableModelArtifactLeaseOwner`.
/// Its placeholder identity and metadata must not be used as a lease.
pub struct UnavailableModelArtifactLease {
    artifact_id: ArtifactId,
}

impl ModelArtifactReader for UnavailableModelArtifactLease {
    fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        None
    }

    fn len(&self) -> u64 {
        0
    }

    fn read_at(
        &mut self,
        _offset: u64,
        _destination: &mut [u8],
    ) -> Result<usize, ArtifactReadError> {
        Err(ArtifactReadError::Unavailable)
    }
}

impl ModelStoreArtifact for UnavailableModelArtifactLease {
    fn store_identity(&self) -> ModelStoreIdentity {
        ModelStoreIdentity(0)
    }

    fn revision(&self) -> u64 {
        0
    }

    fn validate_lease(&self) -> Result<(), ReceiptError> {
        Err(ReceiptError::Unavailable)
    }
}

impl AuthenticatedArtifactLease for UnavailableModelArtifactLease {
    fn store_identity(&self) -> ModelStoreIdentity {
        ModelStoreIdentity(0)
    }

    fn revision(&self) -> u64 {
        0
    }

    fn check(&self, _manifest: &ModelManifest) -> Result<(), ReceiptError> {
        Err(ReceiptError::Unavailable)
    }
}

impl ModelStoreArtifacts for UnavailableModelArtifactLeaseOwner {
    type Snapshot = UnavailableModelArtifactLease;

    fn capture(
        &mut self,
        _manifest: &ModelManifest,
        _store_revision: u64,
    ) -> Result<Self::Snapshot, ReceiptError> {
        Err(ReceiptError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const TEST_BYTES: &[u8] = b"synthetic artifact fixture only";
    const TEST_MANIFEST: &str = r#"
    {
      "schema_version": 1,
      "model_id": "fixture.synthetic-model",
      "display_name": "Synthetic test model",
      "provider": {"provider_id":"fixture","display_name":"Fixture","family":"test"},
      "version": "1",
      "variant": "test",
      "runtime_api_version": "nagi.ai/1",
      "execution": "local",
      "artifact": {
        "format": "gguf",
        "reference": {"kind":"model_store","artifact_id":"fixture.synthetic-artifact"},
        "size_bytes": 31,
        "integrity": null
      },
      "capabilities": ["text.generate"],
      "modalities": ["text"],
      "context": {"max_input_tokens": 1,"max_output_tokens": 1},
      "resources": {"minimum_ram_bytes":1,"recommended_ram_bytes":null,"minimum_storage_bytes":1,"minimum_cpu_cores":1,"gpu_required":false},
      "supported_backends": ["fixture"],
      "roles": ["standard"],
      "compatibility": {"architectures":["x86_64"],"minimum_os_version":null},
      "license": {"identifier":"fixture-only","terms_reference":"fixture:terms","acknowledgement_required":true,"notices":[]},
      "source": null
    }
    "#;

    struct FixtureArtifact {
        manifest: ModelManifest,
        bytes: Arc<[u8]>,
        revision: u64,
    }

    struct FixtureState {
        revision: u64,
        current: Option<FixtureArtifact>,
        revoke_on_next_read: bool,
    }

    /// Host-only fake models the owner contract after a synthetic install has
    /// completed. It is not an install provider or a real Model Store.
    struct SyntheticCompletedArtifactOwner {
        identity: ModelStoreIdentity,
        state: Arc<Mutex<FixtureState>>,
    }

    impl SyntheticCompletedArtifactOwner {
        fn new() -> Self {
            Self {
                identity: ModelStoreIdentity(0x7465_7374),
                state: Arc::new(Mutex::new(FixtureState {
                    revision: 0,
                    current: None,
                    revoke_on_next_read: false,
                })),
            }
        }

        fn revoke_on_next_read(&mut self) -> Result<(), ReceiptError> {
            let mut state = self.state.lock().map_err(|_| ReceiptError::Unavailable)?;
            state.revoke_on_next_read = true;
            Ok(())
        }

        fn publish_completed_fixture(
            &mut self,
            manifest: ModelManifest,
            bytes: &[u8],
        ) -> Result<u64, ReceiptError> {
            if manifest.artifact.size_bytes != Some(bytes.len() as u64) {
                return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
            }
            let mut state = self.state.lock().map_err(|_| ReceiptError::Unavailable)?;
            let revision = state
                .revision
                .checked_add(1)
                .ok_or(ReceiptError::Capacity)?;
            state.revision = revision;
            state.current = Some(FixtureArtifact {
                manifest,
                bytes: Arc::from(bytes),
                revision,
            });
            Ok(revision)
        }

        fn remove_fixture(&mut self) -> Result<u64, ReceiptError> {
            let mut state = self.state.lock().map_err(|_| ReceiptError::Unavailable)?;
            let revision = state
                .revision
                .checked_add(1)
                .ok_or(ReceiptError::Capacity)?;
            state.revision = revision;
            state.current = None;
            Ok(revision)
        }
    }

    struct FixtureArtifactLease {
        identity: ModelStoreIdentity,
        state: Arc<Mutex<FixtureState>>,
        manifest: ModelManifest,
        artifact_id: ArtifactId,
        bytes: Arc<[u8]>,
        revision: u64,
    }

    impl FixtureArtifactLease {
        fn validate(&self) -> Result<(), ReceiptError> {
            let state = self.state.lock().map_err(|_| ReceiptError::Unavailable)?;
            let Some(current) = &state.current else {
                return Err(ReceiptError::Stale);
            };
            if state.revision != self.revision
                || current.revision != self.revision
                || current.manifest != self.manifest
                || !Arc::ptr_eq(&current.bytes, &self.bytes)
            {
                return Err(ReceiptError::Stale);
            }
            Ok(())
        }
    }

    impl ModelArtifactReader for FixtureArtifactLease {
        fn artifact_id(&self) -> &ArtifactId {
            &self.artifact_id
        }

        // The fixture is immutable but deliberately unverified; this API does
        // not claim a digest check or a terms acknowledgement.
        fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
            None
        }

        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn read_at(
            &mut self,
            offset: u64,
            destination: &mut [u8],
        ) -> Result<usize, ArtifactReadError> {
            let offset = usize::try_from(offset).map_err(|_| ArtifactReadError::OutOfRange)?;
            let mut state = self
                .state
                .lock()
                .map_err(|_| ArtifactReadError::Unavailable)?;
            let is_current = state.current.as_ref().is_some_and(|current| {
                state.revision == self.revision
                    && current.revision == self.revision
                    && current.manifest == self.manifest
                    && Arc::ptr_eq(&current.bytes, &self.bytes)
            });
            if !is_current {
                return Err(ArtifactReadError::Unavailable);
            }
            if offset > self.bytes.len() {
                return Err(ArtifactReadError::OutOfRange);
            }
            let count = destination.len().min(self.bytes.len() - offset);
            destination[..count].copy_from_slice(&self.bytes[offset..offset + count]);

            // Direct negative case: the owner invalidates this lease after it
            // has filled the adapter's staging buffer but before returning.
            if state.revoke_on_next_read {
                let revision = state
                    .revision
                    .checked_add(1)
                    .ok_or(ArtifactReadError::Unavailable)?;
                state.revision = revision;
                state.current = Some(FixtureArtifact {
                    manifest: self.manifest.clone(),
                    bytes: Arc::from(b"replacement fixture during read".as_slice()),
                    revision,
                });
                state.revoke_on_next_read = false;
            }
            Ok(count)
        }
    }

    impl ModelStoreArtifact for FixtureArtifactLease {
        fn store_identity(&self) -> ModelStoreIdentity {
            self.identity
        }

        fn revision(&self) -> u64 {
            self.revision
        }

        fn validate_lease(&self) -> Result<(), ReceiptError> {
            self.validate()
        }
    }

    impl AuthenticatedArtifactLease for FixtureArtifactLease {
        fn store_identity(&self) -> ModelStoreIdentity {
            self.identity
        }

        fn revision(&self) -> u64 {
            self.revision
        }

        fn check(&self, manifest: &ModelManifest) -> Result<(), ReceiptError> {
            if manifest != &self.manifest {
                return Err(ReceiptError::Stale);
            }
            self.validate()
        }
    }

    impl ModelStoreArtifacts for SyntheticCompletedArtifactOwner {
        type Snapshot = FixtureArtifactLease;

        fn capture(
            &mut self,
            manifest: &ModelManifest,
            store_revision: u64,
        ) -> Result<Self::Snapshot, ReceiptError> {
            manifest
                .validate()
                .map_err(|_| ReceiptError::InvalidManifest)?;
            let state = self.state.lock().map_err(|_| ReceiptError::Unavailable)?;
            if state.revision != store_revision {
                return Err(ReceiptError::Stale);
            }
            let current = state.current.as_ref().ok_or(ReceiptError::Missing)?;
            if current.manifest != *manifest {
                return Err(ReceiptError::Stale);
            }
            let size = manifest
                .artifact
                .size_bytes
                .filter(|size| *size > 0)
                .ok_or(ReceiptError::InvalidManifest)?;
            let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
            if current.bytes.len() as u64 != size {
                return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
            }
            Ok(FixtureArtifactLease {
                identity: self.identity,
                state: Arc::clone(&self.state),
                manifest: manifest.clone(),
                artifact_id: artifact_id.clone(),
                bytes: Arc::clone(&current.bytes),
                revision: current.revision,
            })
        }
    }

    fn test_manifest() -> ModelManifest {
        let mut manifest = ModelManifest::parse_json(TEST_MANIFEST.as_bytes())
            .expect("parse synthetic test-only manifest");
        manifest.artifact.size_bytes = Some(TEST_BYTES.len() as u64);
        manifest
    }

    fn read_fixture(lease: &mut CompletedModelArtifactLease<FixtureArtifactLease>) -> Vec<u8> {
        let mut bytes = vec![0; lease.len() as usize];
        let count = lease.read_at(0, &mut bytes).expect("read current lease");
        bytes.truncate(count);
        bytes
    }

    #[test]
    fn unavailable_production_owner_fails_closed_without_package_or_terms_substitution() {
        assert_eq!(
            capture_completed_model_artifact(&test_manifest(), 0)
                .err()
                .expect("no production Model Store owner is connected"),
            ReceiptError::Unavailable
        );
    }

    #[test]
    fn completed_artifact_capture_pins_immutable_bytes_and_owner_revision() {
        let mut owner = SyntheticCompletedArtifactOwner::new();
        let manifest = test_manifest();
        let revision = owner
            .publish_completed_fixture(manifest.clone(), TEST_BYTES)
            .expect("publish synthetic completed-install fixture");
        let mut lease = capture_from_owner(&mut owner, &manifest, revision)
            .expect("capture the installed immutable fixture through owner API");

        assert_eq!(lease.store_identity(), owner.identity);
        assert_eq!(lease.revision(), revision);
        assert_eq!(lease.manifest(), &manifest);
        assert_eq!(lease.validate_current(), Ok(()));
        assert_eq!(read_fixture(&mut lease), TEST_BYTES);
        assert_eq!(lease.verified_integrity(), None);
    }

    #[test]
    fn generation_change_rejects_old_lease_and_allows_only_new_revision() {
        let mut owner = SyntheticCompletedArtifactOwner::new();
        let manifest = test_manifest();
        let first_revision = owner
            .publish_completed_fixture(manifest.clone(), TEST_BYTES)
            .unwrap();
        let mut old = capture_from_owner(&mut owner, &manifest, first_revision)
            .expect("capture first revision");

        let replacement = b"replacement fixture, also synthetic";
        let mut replacement_manifest = manifest.clone();
        replacement_manifest.artifact.size_bytes = Some(replacement.len() as u64);
        let second_revision = owner
            .publish_completed_fixture(replacement_manifest.clone(), replacement)
            .expect("complete a replacement fixture");

        assert!(second_revision > first_revision);
        assert_eq!(old.validate_current(), Err(ReceiptError::Stale));
        assert_eq!(old.validate_lease(), Err(ReceiptError::Stale));
        assert_eq!(old.check(&manifest), Err(ReceiptError::Stale));
        assert_eq!(
            old.read_at(0, &mut [0; 8]),
            Err(ArtifactReadError::Unavailable)
        );

        let mut current = capture_from_owner(&mut owner, &replacement_manifest, second_revision)
            .expect("capture replacement revision");
        assert_eq!(current.revision(), second_revision);
        assert_eq!(read_fixture(&mut current), replacement);
    }

    #[test]
    fn removal_and_stale_capture_generation_are_rejected() {
        let mut owner = SyntheticCompletedArtifactOwner::new();
        let manifest = test_manifest();
        let revision = owner
            .publish_completed_fixture(manifest.clone(), TEST_BYTES)
            .unwrap();
        let mut lease =
            capture_from_owner(&mut owner, &manifest, revision).expect("capture before removal");

        assert_eq!(
            capture_from_owner(&mut owner, &manifest, revision - 1).err(),
            Some(ReceiptError::Stale)
        );
        owner.remove_fixture().expect("remove fixture");
        assert_eq!(lease.validate_current(), Err(ReceiptError::Stale));
        assert_eq!(
            lease.read_at(0, &mut [0; 8]),
            Err(ArtifactReadError::Unavailable)
        );
        assert_eq!(
            capture_from_owner(&mut owner, &manifest, revision + 1).err(),
            Some(ReceiptError::Missing)
        );
    }

    #[test]
    fn owner_cannot_return_different_manifest_or_artifact_as_the_requested_lease() {
        let mut owner = SyntheticCompletedArtifactOwner::new();
        let manifest = test_manifest();
        let revision = owner
            .publish_completed_fixture(manifest.clone(), TEST_BYTES)
            .unwrap();
        let mut wrong = manifest.clone();
        wrong.artifact.reference = ArtifactReference::ModelStore {
            artifact_id: ArtifactId::new("fixture.other-artifact").unwrap(),
        };
        assert!(matches!(
            capture_from_owner(&mut owner, &wrong, revision),
            Err(ReceiptError::Stale)
        ));
    }

    #[test]
    fn stale_after_source_read_leaves_caller_buffer_unchanged() {
        let mut owner = SyntheticCompletedArtifactOwner::new();
        let manifest = test_manifest();
        let revision = owner
            .publish_completed_fixture(manifest.clone(), TEST_BYTES)
            .unwrap();
        let mut lease = capture_from_owner(&mut owner, &manifest, revision)
            .expect("capture before controlled invalidation");
        owner
            .revoke_on_next_read()
            .expect("arm synthetic stale-after-read negative");

        let mut caller_buffer = [0xA5; 16];
        let before = caller_buffer;
        assert_eq!(
            lease.read_at(0, &mut caller_buffer),
            Err(ArtifactReadError::Unavailable)
        );
        assert_eq!(caller_buffer, before);
        assert_eq!(lease.validate_current(), Err(ReceiptError::Stale));
    }
}
