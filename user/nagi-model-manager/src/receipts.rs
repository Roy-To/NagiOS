//! Trusted-owner in-process adapters. No guest storage, authority issuer or wire
//! schema is implemented here. Host fake stores cannot establish guest durability.
use crate::{
    verify_model_artifact_integrity, ArtifactId, ArtifactReadError, ArtifactReference,
    CancellationToken, IntegrityMetadata, ModelArtifactReader, ModelId, ModelManifest,
    RuntimeError,
};
use alloc::string::String;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptError {
    Unavailable,
    Denied,
    Cancelled,
    Stale,
    Missing,
    Corrupt,
    TermsRequired,
    Capacity,
    RecoveryRequired,
    InvalidManifest,
    Artifact(RuntimeError),
}

/// Implement only in the trusted live-session owner. Stamp is an equality fence,
/// never an authorization grant; every operation checks the actual live session.
/// Stamp must include issuer namespace, live session incarnation and authorization
/// revision so equal numeric tokens from different issuers are never equivalent.
pub trait LiveAuthority {
    type Session: ?Sized;
    type Stamp: Clone + Eq;
    fn check(&self, live: &Self::Session) -> Result<Self::Stamp, ReceiptError>;
}

/// Owner guarantees immutable bytes and artifact identity from capture through
/// commit. validate_lease must detect revoked/rebound storage revisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Stable namespace identity. Rebinding storage creates a different identity;
/// a captured lease never changes its namespace even when its owner rebinds.
pub struct ModelStoreIdentity(pub u128);

pub trait ModelStoreArtifact: ModelArtifactReader {
    fn store_identity(&self) -> ModelStoreIdentity;
    fn revision(&self) -> u64;
    fn validate_lease(&self) -> Result<(), ReceiptError>;
}

pub trait ModelStoreArtifacts {
    type Snapshot: ModelStoreArtifact;
    fn capture(
        &mut self,
        manifest: &ModelManifest,
        store_revision: u64,
    ) -> Result<Self::Snapshot, ReceiptError>;
}

/// Must be checked by storage at its atomic compare/write/flush point. A failed
/// check must not publish a new installed receipt. This is not a cached grant.
pub trait CommitFence {
    fn check(&mut self) -> Result<(), ReceiptError>;
}

/// Untrusted storage representation, not installed evidence or canonical schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptEnvelope {
    pub store_identity: ModelStoreIdentity,
    pub manifest: ModelManifest,
    pub artifact_revision: u64,
    pub acknowledged_terms: Option<String>,
    pub committed_revision: u64,
}

pub trait ReceiptStore {
    fn store_identity(&self) -> ModelStoreIdentity;
    fn revision(&self) -> Result<u64, ReceiptError>;
    /// Owner guarantees expected-revision compare, fence check, atomic write and
    /// durable flush as one transaction with the immutable lease held throughout.
    /// Reject leases/envelopes not belonging to this exact Store namespace or
    /// whose artifact revisions differ. Owner compare/check must also bind
    /// current manifest identity and acknowledged terms provenance.
    /// Torn or uncertain results MUST return RecoveryRequired, never success.
    /// Any other error guarantees no durable receipt/revision mutation.
    /// Owner must protect stored acknowledgement provenance and envelope integrity;
    /// exact terms string equality by itself is not proof of user consent.
    fn commit<S: ModelStoreArtifact>(
        &mut self,
        expected_revision: u64,
        envelope: &ReceiptEnvelope,
        lease: &S,
        fence: &mut dyn CommitFence,
    ) -> Result<u64, ReceiptError>;
    fn read(&mut self, model: &ModelId) -> Result<Option<ReceiptEnvelope>, ReceiptError>;
}

/// Verified and acknowledged bytes, but not installed/durable evidence.
/// Not Clone: the original immutable lease remains owned until commit completes.
pub struct VerifiedInstallCandidate<S, Stamp> {
    snapshot: S,
    stamp: Stamp,
    store_revision: u64,
    envelope: ReceiptEnvelope,
}
impl<S, Stamp> VerifiedInstallCandidate<S, Stamp> {
    pub fn store_identity(&self) -> ModelStoreIdentity {
        self.envelope.store_identity
    }
    pub fn manifest(&self) -> &ModelManifest {
        &self.envelope.manifest
    }
    pub fn artifact_revision(&self) -> u64 {
        self.envelope.artifact_revision
    }
}

/// Constructed only after durable owner acknowledgement or verified reopen.
#[derive(Debug, Eq, PartialEq)]
pub struct InstalledReceipt {
    envelope: ReceiptEnvelope,
}
impl InstalledReceipt {
    pub fn store_identity(&self) -> ModelStoreIdentity {
        self.envelope.store_identity
    }
    pub fn manifest(&self) -> &ModelManifest {
        &self.envelope.manifest
    }
    pub fn artifact_revision(&self) -> u64 {
        self.envelope.artifact_revision
    }
    pub fn committed_revision(&self) -> u64 {
        self.envelope.committed_revision
    }
    pub fn acknowledged_terms(&self) -> Option<&str> {
        self.envelope.acknowledged_terms.as_deref()
    }
}

struct AuthorityFence<'a, A: LiveAuthority> {
    authority: &'a A,
    live: &'a A::Session,
    stamp: &'a A::Stamp,
    cancel: &'a dyn CancellationToken,
}
impl<A: LiveAuthority> CommitFence for AuthorityFence<'_, A> {
    fn check(&mut self) -> Result<(), ReceiptError> {
        if self.cancel.is_cancelled() {
            return Err(ReceiptError::Cancelled);
        }
        if self.authority.check(self.live)? != *self.stamp {
            return Err(ReceiptError::Stale);
        }
        Ok(())
    }
}

struct GuardedReader<'a, S: ModelStoreArtifact, A: LiveAuthority, R: ReceiptStore> {
    snapshot: &'a mut S,
    fence: AuthorityFence<'a, A>,
    store: &'a R,
    revision: u64,
    artifact_revision: u64,
    failure: Option<ReceiptError>,
}
impl<S: ModelStoreArtifact, A: LiveAuthority, R: ReceiptStore> GuardedReader<'_, S, A, R> {
    fn checkpoint(&mut self) -> Result<(), ReceiptError> {
        self.fence.check()?;
        if self.snapshot.store_identity() != self.store.store_identity()
            || self.store.revision()? != self.revision
            || self.snapshot.revision() != self.artifact_revision
        {
            return Err(ReceiptError::Stale);
        }
        self.snapshot.validate_lease()
    }
}
impl<S: ModelStoreArtifact, A: LiveAuthority, R: ReceiptStore> ModelArtifactReader
    for GuardedReader<'_, S, A, R>
{
    fn artifact_id(&self) -> &ArtifactId {
        self.snapshot.artifact_id()
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        self.snapshot.verified_integrity()
    }
    fn len(&self) -> u64 {
        self.snapshot.len()
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, ArtifactReadError> {
        if let Err(e) = self.checkpoint() {
            self.failure = Some(e);
            return Err(ArtifactReadError::Unavailable);
        }
        let result = self.snapshot.read_at(offset, out);
        if let Err(e) = self.checkpoint() {
            self.failure = Some(e);
            return Err(ArtifactReadError::Unavailable);
        }
        result
    }
}

pub struct ReceiptManager<A: LiveAuthority, T: ModelStoreArtifacts, R: ReceiptStore> {
    authority: A,
    artifacts: T,
    store: R,
    recovery_required: bool,
}
impl<A: LiveAuthority, T: ModelStoreArtifacts, R: ReceiptStore> ReceiptManager<A, T, R> {
    pub const fn new(authority: A, artifacts: T, store: R) -> Self {
        Self {
            authority,
            artifacts,
            store,
            recovery_required: false,
        }
    }
    pub fn store(&self) -> &R {
        &self.store
    }

    fn check(
        &self,
        live: &A::Session,
        stamp: &A::Stamp,
        revision: u64,
        cancel: &dyn CancellationToken,
    ) -> Result<(), ReceiptError> {
        if self.recovery_required {
            return Err(ReceiptError::RecoveryRequired);
        }
        AuthorityFence {
            authority: &self.authority,
            live,
            stamp,
            cancel,
        }
        .check()?;
        if self.store.revision()? != revision {
            return Err(ReceiptError::Stale);
        }
        Ok(())
    }
    fn terms(
        manifest: &ModelManifest,
        acknowledged: Option<&str>,
    ) -> Result<Option<String>, ReceiptError> {
        if manifest.license.acknowledgement_required {
            match (manifest.license.terms_reference.as_deref(), acknowledged) {
                (Some(expected), Some(actual)) if expected == actual => {
                    Ok(Some(String::from(actual)))
                }
                _ => Err(ReceiptError::TermsRequired),
            }
        } else if acknowledged.is_some()
            && acknowledged != manifest.license.terms_reference.as_deref()
        {
            Err(ReceiptError::TermsRequired)
        } else {
            Ok(acknowledged.map(String::from))
        }
    }
    fn verify(
        &self,
        live: &A::Session,
        stamp: &A::Stamp,
        store_revision: u64,
        manifest: &ModelManifest,
        snapshot: &mut T::Snapshot,
        cancel: &dyn CancellationToken,
    ) -> Result<(), ReceiptError> {
        self.check(live, stamp, store_revision, cancel)?;
        snapshot.validate_lease()?;
        if snapshot.store_identity() != self.store.store_identity() {
            return Err(ReceiptError::Stale);
        }
        let revision = snapshot.revision();
        let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
        let length = manifest
            .artifact
            .size_bytes
            .filter(|n| *n > 0)
            .ok_or(ReceiptError::InvalidManifest)?;
        if snapshot.artifact_id() != artifact_id || snapshot.len() != length {
            return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
        }
        let integrity = manifest
            .artifact
            .integrity
            .as_ref()
            .ok_or(ReceiptError::Artifact(RuntimeError::IntegrityRequired))?;
        if snapshot
            .verified_integrity()
            .is_some_and(|cached| cached != integrity)
        {
            return Err(ReceiptError::Artifact(RuntimeError::IntegrityMismatch));
        }
        let mut reader = GuardedReader {
            snapshot,
            fence: AuthorityFence {
                authority: &self.authority,
                live,
                stamp,
                cancel,
            },
            store: &self.store,
            revision: store_revision,
            artifact_revision: revision,
            failure: None,
        };
        let result = verify_model_artifact_integrity(&mut reader, integrity);
        if let Some(e) = reader.failure {
            return Err(e);
        }
        result.map_err(ReceiptError::Artifact)?;
        reader.checkpoint()?;
        if reader.artifact_id() != artifact_id || reader.len() != length {
            return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
        }
        if reader
            .verified_integrity()
            .is_some_and(|cached| cached != integrity)
        {
            return Err(ReceiptError::Artifact(RuntimeError::IntegrityMismatch));
        }
        Ok(())
    }
    pub fn capture_artifact(
        &mut self,
        live: &A::Session,
        manifest: &ModelManifest,
        acknowledged_terms: Option<&str>,
        expected_revision: u64,
        cancel: &dyn CancellationToken,
    ) -> Result<VerifiedInstallCandidate<T::Snapshot, A::Stamp>, ReceiptError> {
        manifest
            .validate()
            .map_err(|_| ReceiptError::InvalidManifest)?;
        if manifest.execution != crate::ExecutionScope::Local {
            return Err(ReceiptError::InvalidManifest);
        }
        let stamp = self.authority.check(live)?;
        self.check(live, &stamp, expected_revision, cancel)?;
        let identity = self.store.store_identity();
        let terms = Self::terms(manifest, acknowledged_terms)?;
        let captured = self.artifacts.capture(manifest, expected_revision);
        self.check(live, &stamp, expected_revision, cancel)?;
        let mut snapshot = captured?;
        if self.store.store_identity() != identity || snapshot.store_identity() != identity {
            return Err(ReceiptError::Stale);
        }
        self.verify(
            live,
            &stamp,
            expected_revision,
            manifest,
            &mut snapshot,
            cancel,
        )?;
        if self.store.store_identity() != identity || snapshot.store_identity() != identity {
            return Err(ReceiptError::Stale);
        }
        let envelope = ReceiptEnvelope {
            store_identity: identity,
            manifest: manifest.clone(),
            artifact_revision: snapshot.revision(),
            acknowledged_terms: terms,
            committed_revision: expected_revision
                .checked_add(1)
                .ok_or(ReceiptError::Capacity)?,
        };
        Ok(VerifiedInstallCandidate {
            snapshot,
            stamp,
            store_revision: expected_revision,
            envelope,
        })
    }
    pub fn commit_verified_install(
        &mut self,
        live: &A::Session,
        current_manifest: &ModelManifest,
        candidate: VerifiedInstallCandidate<T::Snapshot, A::Stamp>,
        cancel: &dyn CancellationToken,
    ) -> Result<InstalledReceipt, ReceiptError> {
        self.commit_verified_install_with_snapshot(live, current_manifest, candidate, cancel)
            .map(|(receipt, _snapshot)| receipt)
    }
    /// Same verified lease, rehash, durable acknowledgement and final fences as
    /// the receipt-only entry. Success transfers the original snapshot once.
    pub fn commit_verified_install_with_snapshot(
        &mut self,
        live: &A::Session,
        current_manifest: &ModelManifest,
        mut candidate: VerifiedInstallCandidate<T::Snapshot, A::Stamp>,
        cancel: &dyn CancellationToken,
    ) -> Result<(InstalledReceipt, T::Snapshot), ReceiptError> {
        self.check(live, &candidate.stamp, candidate.store_revision, cancel)?;
        if current_manifest != &candidate.envelope.manifest
            || self.store.store_identity() != candidate.envelope.store_identity
        {
            return Err(ReceiptError::Stale);
        }
        Self::terms(
            current_manifest,
            candidate.envelope.acknowledged_terms.as_deref(),
        )?;
        if candidate.snapshot.store_identity() != candidate.envelope.store_identity
            || candidate.snapshot.revision() != candidate.envelope.artifact_revision
        {
            return Err(ReceiptError::Stale);
        }
        // Rehash under the same lease immediately before owner commit. Metadata
        // alone never grants installed evidence even if a source breaks its lease.
        self.verify(
            live,
            &candidate.stamp,
            candidate.store_revision,
            current_manifest,
            &mut candidate.snapshot,
            cancel,
        )?;
        let mut fence = AuthorityFence {
            authority: &self.authority,
            live,
            stamp: &candidate.stamp,
            cancel,
        };
        let committed = self.store.commit(
            candidate.store_revision,
            &candidate.envelope,
            &candidate.snapshot,
            &mut fence,
        );
        let ack = match committed {
            Ok(ack) => ack,
            Err(ReceiptError::RecoveryRequired) => {
                self.recovery_required = true;
                return Err(ReceiptError::RecoveryRequired);
            }
            Err(e) => {
                if self.store.revision() != Ok(candidate.store_revision)
                    || self.store.store_identity() != candidate.envelope.store_identity
                {
                    self.recovery_required = true;
                    return Err(ReceiptError::RecoveryRequired);
                }
                fence.check()?;
                return Err(e);
            }
        };
        // A successful storage result followed by a failed fence/revision check
        // is uncertain to this adapter. Never invent rollback or expose success.
        if ack != candidate.envelope.committed_revision
            || self.store.revision() != Ok(ack)
            || fence.check().is_err()
            || candidate.snapshot.store_identity() != candidate.envelope.store_identity
            || self.store.store_identity() != candidate.envelope.store_identity
            || candidate.snapshot.validate_lease().is_err()
            || candidate.snapshot.revision() != candidate.envelope.artifact_revision
        {
            self.recovery_required = true;
            return Err(ReceiptError::RecoveryRequired);
        }
        Ok((
            InstalledReceipt {
                envelope: candidate.envelope,
            },
            candidate.snapshot,
        ))
    }
    /// Compatibility entry; preserves the existing receipt-only result.
    pub fn reopen_receipts(
        &mut self,
        live: &A::Session,
        current_manifest: &ModelManifest,
        expected_revision: u64,
        cancel: &dyn CancellationToken,
    ) -> Result<InstalledReceipt, ReceiptError> {
        self.reopen_verified_artifact(live, current_manifest, expected_revision, cancel)
            .map(|(receipt, _)| receipt)
    }
    /// Retain the exact verified immutable view through later native load.
    /// No second recapture or metadata-only receipt construction is performed.
    pub fn reopen_verified_artifact(
        &mut self,
        live: &A::Session,
        current_manifest: &ModelManifest,
        expected_revision: u64,
        cancel: &dyn CancellationToken,
    ) -> Result<(InstalledReceipt, T::Snapshot), ReceiptError> {
        current_manifest
            .validate()
            .map_err(|_| ReceiptError::InvalidManifest)?;
        let stamp = self.authority.check(live)?;
        self.check(live, &stamp, expected_revision, cancel)?;
        let identity = self.store.store_identity();
        let read = self.store.read(&current_manifest.model_id);
        self.check(live, &stamp, expected_revision, cancel)?;
        if self.store.store_identity() != identity {
            return Err(ReceiptError::Stale);
        }
        let envelope = read?.ok_or(ReceiptError::Missing)?;
        if envelope.store_identity != identity
            || envelope.manifest != *current_manifest
            || envelope.committed_revision > expected_revision
            || envelope.committed_revision == 0
        {
            return Err(ReceiptError::Corrupt);
        }
        Self::terms(current_manifest, envelope.acknowledged_terms.as_deref())?;
        if current_manifest.execution != crate::ExecutionScope::Local {
            return Err(ReceiptError::InvalidManifest);
        }
        let captured = self.artifacts.capture(current_manifest, expected_revision);
        self.check(live, &stamp, expected_revision, cancel)?;
        let mut snapshot = captured?;
        if snapshot.revision() != envelope.artifact_revision
            || snapshot.store_identity() != envelope.store_identity
            || self.store.store_identity() != envelope.store_identity
        {
            return Err(ReceiptError::Stale);
        }
        self.verify(
            live,
            &stamp,
            expected_revision,
            current_manifest,
            &mut snapshot,
            cancel,
        )?;
        if snapshot.store_identity() != envelope.store_identity
            || self.store.store_identity() != envelope.store_identity
        {
            return Err(ReceiptError::Stale);
        }
        Ok((InstalledReceipt { envelope }, snapshot))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::{format, rc::Rc, vec, vec::Vec};
    use core::cell::RefCell;
    use sha2::{Digest, Sha256};
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum At {
        None,
        Capture,
        Read,
        Commit,
        Reopen,
    }
    struct State {
        identity: u128,
        unavailable: bool,
        read_error: bool,
        current: u64,
        epoch: u64,
        locked: bool,
        cancel: bool,
        revision: u64,
        artifact_revision: u64,
        bytes: Vec<u8>,
        receipt: Option<ReceiptEnvelope>,
        at: At,
        commit_error: Option<ReceiptError>,
        mutate_namespace: bool,
        mutate_stamp: bool,
        mutate_artifact: bool,
        ack_wrong: bool,
        writes: usize,
        reads: usize,
    }
    type Shared = Rc<RefCell<State>>;
    fn hook(s: &Shared, at: At) {
        let mut s = s.borrow_mut();
        if s.at == at {
            if s.mutate_namespace {
                s.identity += 1;
                if at == At::Reopen {
                    let identity = ModelStoreIdentity(s.identity);
                    if let Some(ref mut receipt) = s.receipt {
                        receipt.store_identity = identity;
                    }
                }
            } else if s.mutate_stamp {
                s.epoch += 1;
            } else if s.mutate_artifact {
                s.artifact_revision += 1;
            } else {
                s.cancel = true;
            }
        }
    }
    struct Authority(Shared);
    impl LiveAuthority for Authority {
        type Session = u64;
        type Stamp = (u64, u64);
        fn check(&self, live: &u64) -> Result<Self::Stamp, ReceiptError> {
            let s = self.0.borrow();
            if s.locked || *live != s.current {
                return Err(ReceiptError::Denied);
            }
            Ok((*live, s.epoch))
        }
    }
    struct Cancel(Shared);
    impl CancellationToken for Cancel {
        fn is_cancelled(&self) -> bool {
            self.0.borrow().cancel
        }
    }
    struct Snapshot {
        shared: Shared,
        identity: ModelStoreIdentity,
        id: ArtifactId,
        revision: u64,
        integrity: IntegrityMetadata,
    }
    impl ModelArtifactReader for Snapshot {
        fn artifact_id(&self) -> &ArtifactId {
            &self.id
        }
        fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
            Some(&self.integrity)
        }
        fn len(&self) -> u64 {
            self.shared.borrow().bytes.len() as u64
        }
        fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, ArtifactReadError> {
            hook(&self.shared, At::Read);
            let mut s = self.shared.borrow_mut();
            s.reads += 1;
            if s.read_error {
                return Err(ArtifactReadError::Unavailable);
            }
            let start = offset as usize;
            if start > s.bytes.len() {
                return Err(ArtifactReadError::OutOfRange);
            }
            let n = out.len().min(113).min(s.bytes.len() - start);
            out[..n].copy_from_slice(&s.bytes[start..start + n]);
            Ok(n)
        }
    }
    impl ModelStoreArtifact for Snapshot {
        fn store_identity(&self) -> ModelStoreIdentity {
            self.identity
        }
        fn revision(&self) -> u64 {
            self.revision
        }
        fn validate_lease(&self) -> Result<(), ReceiptError> {
            if self.shared.borrow().artifact_revision != self.revision {
                Err(ReceiptError::Stale)
            } else {
                Ok(())
            }
        }
    }
    struct Artifacts(Shared);
    impl ModelStoreArtifacts for Artifacts {
        type Snapshot = Snapshot;
        fn capture(&mut self, m: &ModelManifest, _: u64) -> Result<Snapshot, ReceiptError> {
            let ArtifactReference::ModelStore { artifact_id } = &m.artifact.reference;
            let revision = self.0.borrow().artifact_revision;
            hook(&self.0, At::Capture);
            Ok(Snapshot {
                identity: ModelStoreIdentity(self.0.borrow().identity),
                shared: self.0.clone(),
                id: artifact_id.clone(),
                revision,
                integrity: m.artifact.integrity.clone().unwrap(),
            })
        }
    }
    struct Store(Shared);
    impl ReceiptStore for Store {
        fn store_identity(&self) -> ModelStoreIdentity {
            ModelStoreIdentity(self.0.borrow().identity)
        }
        fn revision(&self) -> Result<u64, ReceiptError> {
            if self.0.borrow().unavailable {
                Err(ReceiptError::Unavailable)
            } else {
                Ok(self.0.borrow().revision)
            }
        }
        fn commit<S: ModelStoreArtifact>(
            &mut self,
            expected: u64,
            e: &ReceiptEnvelope,
            lease: &S,
            fence: &mut dyn CommitFence,
        ) -> Result<u64, ReceiptError> {
            if self.revision()? != expected {
                return Err(ReceiptError::Stale);
            }
            hook(&self.0, At::Commit);
            fence.check()?;
            lease.validate_lease()?;
            let mut s = self.0.borrow_mut();
            if let Some(err) = s.commit_error {
                if s.ack_wrong {
                    s.revision += 1;
                }
                return Err(err);
            }
            s.writes += 1;
            s.receipt = Some(e.clone());
            s.revision = e.committed_revision;
            if s.ack_wrong {
                Ok(s.revision + 1)
            } else {
                Ok(s.revision)
            }
        }
        fn read(&mut self, _: &ModelId) -> Result<Option<ReceiptEnvelope>, ReceiptError> {
            hook(&self.0, At::Reopen);
            Ok(self.0.borrow().receipt.clone())
        }
    }
    type Manager = ReceiptManager<Authority, Artifacts, Store>;
    fn setup() -> (Manager, Shared, ModelManifest, Cancel) {
        let bytes = vec![7; 20000];
        let mut m =
            ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json"))
                .unwrap();
        m.source = None;
        m.artifact.size_bytes = Some(bytes.len() as u64);
        m.artifact.integrity.as_mut().unwrap().digest = format!("{:x}", Sha256::digest(&bytes));
        let s = Rc::new(RefCell::new(State {
            identity: 17,
            unavailable: false,
            read_error: false,
            current: 5,
            epoch: 1,
            locked: false,
            cancel: false,
            revision: 10,
            artifact_revision: 7,
            bytes,
            receipt: None,
            at: At::None,
            commit_error: None,
            mutate_namespace: false,
            mutate_stamp: false,
            mutate_artifact: false,
            ack_wrong: false,
            writes: 0,
            reads: 0,
        }));
        (
            Manager::new(Authority(s.clone()), Artifacts(s.clone()), Store(s.clone())),
            s.clone(),
            m,
            Cancel(s),
        )
    }
    fn capture(
        manager: &mut Manager,
        m: &ModelManifest,
        cancel: &Cancel,
    ) -> VerifiedInstallCandidate<Snapshot, (u64, u64)> {
        manager
            .capture_artifact(&5, m, m.license.terms_reference.as_deref(), 10, cancel)
            .unwrap()
    }
    #[test]
    fn synthetic_durable_commit_and_new_manager_reopen_rehash_actual_bytes() {
        let (mut manager, s, m, c) = setup();
        let candidate = capture(&mut manager, &m, &c);
        assert!(s.borrow().reads > 1);
        assert_eq!(s.borrow().writes, 0);
        let receipt = manager
            .commit_verified_install(&5, &m, candidate, &c)
            .unwrap();
        assert_eq!(receipt.manifest(), &m);
        assert_eq!(receipt.committed_revision(), 11);
        assert_eq!(
            receipt.acknowledged_terms(),
            m.license.terms_reference.as_deref()
        );
        let prior = s.borrow().reads;
        let mut restarted =
            Manager::new(Authority(s.clone()), Artifacts(s.clone()), Store(s.clone()));
        assert_eq!(restarted.reopen_receipts(&5, &m, 11, &c).unwrap(), receipt);
        assert!(s.borrow().reads > prior);
        assert_eq!(s.borrow().writes, 1);
    }
    #[test]
    fn cached_digest_never_substitutes_for_actual_hash() {
        let (mut manager, s, m, c) = setup();
        s.borrow_mut().bytes[0] ^= 1;
        assert_eq!(
            manager
                .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Artifact(RuntimeError::IntegrityMismatch))
        );
        assert_eq!(s.borrow().writes, 0);
    }
    #[test]
    fn wrong_length_terms_identity_and_current_manifest_never_publish() {
        let (mut manager, s, m, c) = setup();
        assert_eq!(
            manager
                .capture_artifact(&5, &m, Some("wrong"), 10, &c)
                .err(),
            Some(ReceiptError::TermsRequired)
        );
        assert_eq!(s.borrow().reads, 0);
        let mut wrong = m.clone();
        wrong.artifact.size_bytes = Some(3);
        assert_eq!(
            manager
                .capture_artifact(&5, &wrong, wrong.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Artifact(RuntimeError::ArtifactMismatch))
        );
        let candidate = capture(&mut manager, &m, &c);
        wrong = m.clone();
        wrong.version = String::from("changed-current-version");
        assert_eq!(
            manager
                .commit_verified_install(&5, &wrong, candidate, &c)
                .err(),
            Some(ReceiptError::Stale)
        );
        assert_eq!(s.borrow().writes, 0);
    }
    #[test]
    fn each_capture_read_commit_reopen_checkpoint_rejects_changed_live_state() {
        for at in [At::Capture, At::Read, At::Commit, At::Reopen] {
            for mutate_stamp in [false, true] {
                let (mut manager, s, m, c) = setup();
                if at == At::Reopen {
                    let candidate = capture(&mut manager, &m, &c);
                    manager
                        .commit_verified_install(&5, &m, candidate, &c)
                        .unwrap();
                }
                if at == At::Commit {
                    let candidate = capture(&mut manager, &m, &c);
                    s.borrow_mut().at = at;
                    s.borrow_mut().mutate_stamp = mutate_stamp;
                    assert!(manager
                        .commit_verified_install(&5, &m, candidate, &c)
                        .is_err());
                } else {
                    s.borrow_mut().at = at;
                    s.borrow_mut().mutate_stamp = mutate_stamp;
                    if at == At::Reopen {
                        assert!(manager.reopen_receipts(&5, &m, 11, &c).is_err());
                    } else {
                        assert!(manager
                            .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                            .is_err());
                    }
                }
                assert_eq!(s.borrow().writes, usize::from(at == At::Reopen));
            }
        }
    }
    #[test]
    fn revision_revoke_session_change_and_cancel_between_verify_commit_reject() {
        for mode in 0..5 {
            let (mut manager, s, m, c) = setup();
            let candidate = capture(&mut manager, &m, &c);
            {
                let mut s = s.borrow_mut();
                match mode {
                    0 => s.revision += 1,
                    1 => s.locked = true,
                    2 => s.current += 1,
                    3 => s.cancel = true,
                    _ => s.artifact_revision += 1,
                };
            }
            assert!(manager
                .commit_verified_install(&5, &m, candidate, &c)
                .is_err());
            assert_eq!(s.borrow().writes, 0);
        }
    }
    #[test]
    fn uncertain_flush_and_bad_ack_require_recovery_without_receipt_publication() {
        for wrong_ack in [false, true] {
            let (mut manager, s, m, c) = setup();
            let candidate = capture(&mut manager, &m, &c);
            if wrong_ack {
                s.borrow_mut().ack_wrong = true;
            } else {
                s.borrow_mut().commit_error = Some(ReceiptError::RecoveryRequired);
            }
            assert_eq!(
                manager.commit_verified_install(&5, &m, candidate, &c).err(),
                Some(ReceiptError::RecoveryRequired)
            );
            let rev = s.borrow().revision;
            assert_eq!(
                manager
                    .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), rev, &c)
                    .err(),
                Some(ReceiptError::RecoveryRequired)
            );
        }
    }
    #[test]
    fn capacity_denial_missing_and_corrupt_reopen_are_failure_atomic() {
        let (mut manager, s, m, c) = setup();
        let candidate = capture(&mut manager, &m, &c);
        s.borrow_mut().commit_error = Some(ReceiptError::Capacity);
        assert_eq!(
            manager.commit_verified_install(&5, &m, candidate, &c).err(),
            Some(ReceiptError::Capacity)
        );
        assert_eq!(s.borrow().writes, 0);
        assert!(s.borrow().receipt.is_none());
        assert_eq!(
            manager.reopen_receipts(&5, &m, 10, &c).err(),
            Some(ReceiptError::Missing)
        );
        s.borrow_mut().commit_error = None;
        let candidate = capture(&mut manager, &m, &c);
        manager
            .commit_verified_install(&5, &m, candidate, &c)
            .unwrap();
        s.borrow_mut().receipt.as_mut().unwrap().manifest.version = String::from("torn");
        assert_eq!(
            manager.reopen_receipts(&5, &m, 11, &c).err(),
            Some(ReceiptError::Corrupt)
        );
    }
    #[test]
    fn reopened_receipt_does_not_trust_persisted_terms_or_artifact() {
        for corrupt_bytes in [false, true] {
            let (mut manager, s, m, c) = setup();
            let candidate = capture(&mut manager, &m, &c);
            manager
                .commit_verified_install(&5, &m, candidate, &c)
                .unwrap();
            if corrupt_bytes {
                s.borrow_mut().bytes[0] ^= 1;
            } else {
                s.borrow_mut().receipt.as_mut().unwrap().acknowledged_terms = None;
            }
            assert!(manager.reopen_receipts(&5, &m, 11, &c).is_err());
            assert_eq!(s.borrow().writes, 1);
        }
    }
    #[test]
    fn cross_store_candidate_and_receipt_namespace_are_rejected() {
        let (mut a, sa, m, ca) = setup();
        let candidate = capture(&mut a, &m, &ca);
        let (mut b, sb, _, cb) = setup();
        sb.borrow_mut().identity = 99;
        assert_eq!(
            b.commit_verified_install(&5, &m, candidate, &cb).err(),
            Some(ReceiptError::Stale)
        );
        assert_eq!(sa.borrow().writes, 0);
        assert_eq!(sb.borrow().writes, 0);
        let candidate = capture(&mut a, &m, &ca);
        a.commit_verified_install(&5, &m, candidate, &ca).unwrap();
        sa.borrow_mut().receipt.as_mut().unwrap().store_identity = ModelStoreIdentity(99);
        assert_eq!(
            a.reopen_receipts(&5, &m, 11, &ca).err(),
            Some(ReceiptError::Corrupt)
        );
    }
    #[test]
    fn disconnected_storage_and_denied_live_session_never_capture_or_commit() {
        let (mut manager, s, m, c) = setup();
        s.borrow_mut().unavailable = true;
        assert_eq!(
            manager
                .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Unavailable)
        );
        s.borrow_mut().unavailable = false;
        s.borrow_mut().locked = true;
        assert_eq!(
            manager
                .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Denied)
        );
        assert_eq!(s.borrow().reads, 0);
        assert_eq!(s.borrow().writes, 0);
    }
    #[test]
    fn rebound_candidate_and_read_failure_never_write() {
        let (mut manager, s, m, c) = setup();
        let mut candidate = capture(&mut manager, &m, &c);
        s.borrow_mut().artifact_revision += 1;
        candidate.snapshot.revision += 1;
        assert_eq!(
            manager.commit_verified_install(&5, &m, candidate, &c).err(),
            Some(ReceiptError::Stale)
        );
        s.borrow_mut().read_error = true;
        assert_eq!(
            manager
                .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Artifact(RuntimeError::ArtifactUnavailable))
        );
        assert_eq!(s.borrow().writes, 0);
    }
    #[test]
    fn capture_and_reopen_namespace_rebind_are_rejected() {
        let (mut manager, s, m, c) = setup();
        s.borrow_mut().at = At::Capture;
        s.borrow_mut().mutate_namespace = true;
        assert_eq!(
            manager
                .capture_artifact(&5, &m, m.license.terms_reference.as_deref(), 10, &c)
                .err(),
            Some(ReceiptError::Stale)
        );
        assert_eq!(s.borrow().writes, 0);
        s.borrow_mut().at = At::None;
        let candidate = capture(&mut manager, &m, &c);
        manager
            .commit_verified_install(&5, &m, candidate, &c)
            .unwrap();
        s.borrow_mut().at = At::Capture;
        assert_eq!(
            manager.reopen_receipts(&5, &m, 11, &c).err(),
            Some(ReceiptError::Stale)
        );
        assert_eq!(s.borrow().writes, 1);
    }
    #[test]
    fn error_with_changed_durable_revision_is_recovery_required() {
        let (mut manager, s, m, c) = setup();
        let candidate = capture(&mut manager, &m, &c);
        s.borrow_mut().commit_error = Some(ReceiptError::Capacity);
        s.borrow_mut().ack_wrong = true;
        assert_eq!(
            manager.commit_verified_install(&5, &m, candidate, &c).err(),
            Some(ReceiptError::RecoveryRequired)
        );
        assert_eq!(
            manager.reopen_receipts(&5, &m, 11, &c).err(),
            Some(ReceiptError::RecoveryRequired)
        );
    }
    #[test]
    fn read_rebind_with_matching_new_envelope_and_lease_is_rejected() {
        let (mut manager, s, m, c) = setup();
        let candidate = capture(&mut manager, &m, &c);
        manager
            .commit_verified_install(&5, &m, candidate, &c)
            .unwrap();
        s.borrow_mut().at = At::Reopen;
        s.borrow_mut().mutate_namespace = true;
        assert_eq!(
            manager.reopen_receipts(&5, &m, 11, &c).err(),
            Some(ReceiptError::Stale)
        );
        assert_eq!(s.borrow().writes, 1);
    }
    #[test]
    fn committed_snapshot_return_keeps_same_lease_and_rehashes_before_ack() {
        let (mut manager, s, m, c) = setup();
        let candidate = capture(&mut manager, &m, &c);
        let before = s.borrow().reads;
        let (receipt, snapshot) = manager
            .commit_verified_install_with_snapshot(&5, &m, candidate, &c)
            .unwrap();
        assert!(s.borrow().reads > before);
        assert_eq!(receipt.committed_revision(), 11);
        assert!(Rc::ptr_eq(&snapshot.shared, &s));
        assert_eq!(snapshot.store_identity(), receipt.store_identity());
        assert_eq!(snapshot.revision(), receipt.artifact_revision());
        assert!(snapshot.validate_lease().is_ok());
    }
    #[test]
    fn committed_snapshot_api_preserves_uncertain_ack_and_late_cancel_rejection() {
        for wrong_ack in [false, true] {
            let (mut manager, s, m, c) = setup();
            let candidate = capture(&mut manager, &m, &c);
            if wrong_ack {
                s.borrow_mut().ack_wrong = true;
            } else {
                s.borrow_mut().at = At::Commit;
            }
            assert!(manager
                .commit_verified_install_with_snapshot(&5, &m, candidate, &c)
                .is_err());
        }
    }
}
