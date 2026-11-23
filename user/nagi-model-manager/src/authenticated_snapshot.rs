//! Logically immutable artifact snapshot authenticated by a private sealed chunk
//! table. No model duplication, write namespace or installed receipt is created.
use crate::{
    sha256_bytes, verify_model_artifact_integrity, ArtifactId, ArtifactReadError,
    ArtifactReference, CancellationToken, IntegrityMetadata, LiveAuthority, LiveRequestAuthority,
    ModelArtifactReader, ModelManifest, ModelStoreArtifact, ModelStoreIdentity, ReceiptError,
    RuntimeError,
};
use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;

/// Trusted canonical owner only: Clone of Session is correlation, never a grant.
/// Final OS checks must be synchronous actual live reads. Diagnostic callbacks
/// cannot mint authority and are followed by complete live/lease fences.
pub trait AuthenticatedSnapshotAuthority: LiveRequestAuthority
where
    Self::Session: Clone,
{
    fn check_os_terms(&self, live: &Self::Session, reference: &str) -> Result<(), ReceiptError>;
    fn current_request_epoch(&self, live: &Self::Session) -> Result<u64, ReceiptError>;
    fn snapshot_stage(&self, _stage: AuthenticatedSnapshotStage) {}
}
/// Trusted lease pins the actual storage instance, namespace and full manifest.
pub trait AuthenticatedArtifactLease {
    fn store_identity(&self) -> ModelStoreIdentity;
    fn revision(&self) -> u64;
    fn check(&self, manifest: &ModelManifest) -> Result<(), ReceiptError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticatedSnapshotStage {
    SealBegin,
    HashBegin,
    HashDone,
    SealDone,
}
mod immutable_seal {
    pub trait Sealed {}
}
/// Eligibility is sealed inside this crate. Only complete authenticated capture
/// can produce this logical view; raw readers/metadata cannot implement it.
///
/// An ordinary owner-supplied reader is not eligible merely because it implements
/// the existing Store artifact interface:
/// ```compile_fail
/// use nagi_model_manager::{AuthenticatedImmutableArtifact, ModelStoreArtifact};
/// fn require_authenticated<T: AuthenticatedImmutableArtifact>(_: T) {}
/// fn substitute_raw<T: ModelStoreArtifact>(raw: T) {
///     require_authenticated(raw);
/// }
/// ```
pub trait AuthenticatedImmutableArtifact: ModelStoreArtifact + immutable_seal::Sealed {}

pub const SEALED_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_SEALED_TABLE_BYTES: usize = 2 * 1024 * 1024;
const SOURCE_READ_BYTES: usize = 8 * 1024;

/// Private-field authenticated capture, never Clone or externally constructible.
/// The only public constructor performs complete byte verification.
///
/// ```compile_fail
/// use nagi_model_manager::{AuthenticatedModelStoreSnapshot, LiveAuthority};
/// fn duplicate<R: Clone, L: Clone, A: LiveAuthority + Clone, C: Clone>(
///     view: AuthenticatedModelStoreSnapshot<R, L, A, C>,
/// ) where A::Session: Clone {
///     let _ = view.clone();
/// }
/// ```
///
/// ```compile_fail
/// use nagi_model_manager::{AuthenticatedModelStoreSnapshot, LiveAuthority};
/// fn mint_from_metadata<R, L, A: LiveAuthority, C>()
///     -> AuthenticatedModelStoreSnapshot<R, L, A, C>
/// where A::Session: Clone {
///     AuthenticatedModelStoreSnapshot {}
/// }
/// ```
pub struct AuthenticatedModelStoreSnapshot<R, L, A: LiveAuthority, C>
where
    A::Session: Clone,
{
    source: R,
    lease: L,
    authority: A,
    // Correlation only. The authority rechecks its actual canonical live owner.
    live: A::Session,
    stamp: A::Stamp,
    captured_epoch: u64,
    cancel: C,
    manifest: ModelManifest,
    identity: ModelStoreIdentity,
    revision: u64,
    chunks: Vec<[u8; 32]>,
    scratch: Box<[u8]>,
    // The sole scratch buffer doubles as a logical-view cache. Published only
    // after whole-chunk hash plus post-read fences, never from partial capture.
    authenticated_chunk: Option<(usize, usize)>,
    failed: Cell<Option<ReceiptError>>,
}
impl<R, L, A, C> AuthenticatedModelStoreSnapshot<R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
    /// Only complete verification of actual bytes seals the private table. The
    /// source and actual owner lease stay owned through later verification/load.
    pub fn capture_verified(
        source: R,
        lease: L,
        authority: A,
        live: &A::Session,
        cancel: C,
        manifest: &ModelManifest,
    ) -> Result<Self, ReceiptError> {
        manifest
            .validate()
            .map_err(|_| ReceiptError::InvalidManifest)?;
        let length = manifest
            .artifact
            .size_bytes
            .filter(|length| *length != 0)
            .ok_or(ReceiptError::InvalidManifest)?;
        let integrity = manifest
            .artifact
            .integrity
            .as_ref()
            .ok_or(ReceiptError::InvalidManifest)?;
        if cancel.is_cancelled() {
            return Err(ReceiptError::Cancelled);
        }
        let stamp = authority.check(live)?;
        let captured_epoch = authority.current_request_epoch(live)?;
        if captured_epoch == 0 {
            return Err(ReceiptError::Stale);
        }
        authority.check_request_epoch(live, captured_epoch)?;
        let terms = manifest
            .license
            .terms_reference
            .as_deref()
            .ok_or(ReceiptError::InvalidManifest)?;
        authority.check_os_terms(live, terms)?;
        lease.check(manifest)?;
        let identity = lease.store_identity();
        let revision = lease.revision();
        let ArtifactReference::ModelStore { artifact_id } = &manifest.artifact.reference;
        if source.artifact_id() != artifact_id || source.len() != length {
            return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
        }
        lease.check(manifest)?;
        if lease.store_identity() != identity || lease.revision() != revision {
            return Err(ReceiptError::Stale);
        }
        if cancel.is_cancelled() {
            return Err(ReceiptError::Cancelled);
        }
        authority.check_request_epoch(live, captured_epoch)?;
        authority.check_os_terms(live, terms)?;
        if authority.check(live)? != stamp {
            return Err(ReceiptError::Stale);
        }
        let count = usize::try_from((length - 1) / SEALED_CHUNK_BYTES as u64 + 1)
            .map_err(|_| ReceiptError::Capacity)?;
        if count.checked_mul(32).ok_or(ReceiptError::Capacity)? > MAX_SEALED_TABLE_BYTES {
            return Err(ReceiptError::Capacity);
        }
        let mut chunks = Vec::new();
        chunks
            .try_reserve_exact(count)
            .map_err(|_| ReceiptError::Capacity)?;
        let mut scratch = Vec::new();
        scratch
            .try_reserve_exact(SEALED_CHUNK_BYTES)
            .map_err(|_| ReceiptError::Capacity)?;
        scratch.resize(SEALED_CHUNK_BYTES, 0);
        let mut snapshot = Self {
            source,
            lease,
            authority,
            live: live.clone(),
            stamp,
            captured_epoch,
            cancel,
            manifest: manifest.clone(),
            identity,
            revision,
            chunks,
            scratch: scratch.into_boxed_slice(),
            authenticated_chunk: None,
            failed: Cell::new(None),
        };
        snapshot.checkpoint()?;
        // The existing verifier reads actual bytes in bounded sequential pieces.
        // Its private destination is accumulated into whole chunk hashes.
        snapshot
            .authority
            .snapshot_stage(AuthenticatedSnapshotStage::SealBegin);
        snapshot.checkpoint()?;
        let mut accumulating = AccumulatingReader {
            snapshot: &mut snapshot,
            offset: 0,
            used: 0,
        };
        accumulating
            .snapshot
            .authority
            .snapshot_stage(AuthenticatedSnapshotStage::HashBegin);
        accumulating.snapshot.checkpoint()?;
        let verified = verify_model_artifact_integrity(&mut accumulating, integrity);
        accumulating.snapshot.checkpoint()?;
        verified.map_err(ReceiptError::Artifact)?;
        if accumulating.offset != length {
            return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
        }
        accumulating
            .snapshot
            .authority
            .snapshot_stage(AuthenticatedSnapshotStage::HashDone);
        accumulating.snapshot.checkpoint()?;
        if accumulating.used != 0 {
            let digest = sha256_bytes(&accumulating.snapshot.scratch[..accumulating.used]);
            accumulating.snapshot.chunks.push(digest);
        }
        if accumulating.snapshot.chunks.len() != count {
            return Err(ReceiptError::Corrupt);
        }
        accumulating.snapshot.scratch.fill(0);
        accumulating.snapshot.checkpoint()?;
        accumulating
            .snapshot
            .authority
            .snapshot_stage(AuthenticatedSnapshotStage::SealDone);
        accumulating.snapshot.checkpoint()?;
        Ok(snapshot)
    }
    pub fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    pub fn sealed_table_bytes(&self) -> usize {
        self.chunks.len() * 32
    }
    fn fail(&self, error: ReceiptError) -> ReceiptError {
        if self.failed.get().is_none() {
            self.failed.set(Some(error));
        }
        self.failed.get().unwrap_or(error)
    }
    fn checkpoint(&self) -> Result<(), ReceiptError> {
        if let Some(error) = self.failed.get() {
            return Err(error);
        }
        let checked = (|| {
            if self.cancel.is_cancelled() {
                return Err(ReceiptError::Cancelled);
            }
            if self.authority.check(&self.live)? != self.stamp {
                return Err(ReceiptError::Stale);
            }
            self.authority
                .check_request_epoch(&self.live, self.captured_epoch)?;
            self.authority.check_os_terms(
                &self.live,
                self.manifest
                    .license
                    .terms_reference
                    .as_deref()
                    .ok_or(ReceiptError::InvalidManifest)?,
            )?;
            self.lease.check(&self.manifest)?;
            if self.lease.store_identity() != self.identity
                || self.lease.revision() != self.revision
            {
                return Err(ReceiptError::Stale);
            }
            let ArtifactReference::ModelStore { artifact_id } = &self.manifest.artifact.reference;
            if self.source.artifact_id() != artifact_id
                || Some(self.source.len()) != self.manifest.artifact.size_bytes
            {
                return Err(ReceiptError::Artifact(RuntimeError::ArtifactMismatch));
            }
            // Source metadata callbacks must not hide owner mutation.
            self.authority
                .check_request_epoch(&self.live, self.captured_epoch)?;
            self.authority.check_os_terms(
                &self.live,
                self.manifest
                    .license
                    .terms_reference
                    .as_deref()
                    .ok_or(ReceiptError::InvalidManifest)?,
            )?;
            self.lease.check(&self.manifest)?;
            if self.lease.store_identity() != self.identity
                || self.lease.revision() != self.revision
            {
                return Err(ReceiptError::Stale);
            }
            // The owner/source callback may revoke while returning Ok.
            if self.cancel.is_cancelled() {
                return Err(ReceiptError::Cancelled);
            }
            if self.authority.check(&self.live)? != self.stamp {
                return Err(ReceiptError::Stale);
            }
            // All lease/source callbacks are finished. These trusted canonical
            // OS checks are synchronous and must consult actual live state;
            // a same-session pending-slot advance is not covered by Stamp.
            self.authority.check_os_terms(
                &self.live,
                self.manifest
                    .license
                    .terms_reference
                    .as_deref()
                    .ok_or(ReceiptError::InvalidManifest)?,
            )?;
            if self.authority.check(&self.live)? != self.stamp {
                return Err(ReceiptError::Stale);
            }
            self.authority
                .check_request_epoch(&self.live, self.captured_epoch)?;
            if self.cancel.is_cancelled() {
                return Err(ReceiptError::Cancelled);
            }
            Ok(())
        })();
        checked.map_err(|error| self.fail(error))
    }
    fn read_chunk(&mut self, index: usize) -> Result<usize, ReceiptError> {
        self.checkpoint()?;
        if let Some((cached_index, cached_len)) = self.authenticated_chunk {
            if cached_index == index {
                // Return previously authenticated captured bytes, not a claim
                // that physical backing is immutable. Every hit still fences
                // actual namespace/lease/live/epoch/terms/cancel before/after.
                self.checkpoint()?;
                return Ok(cached_len);
            }
        }
        self.authenticated_chunk = None;
        let start = (index as u64)
            .checked_mul(SEALED_CHUNK_BYTES as u64)
            .ok_or(ReceiptError::Capacity)?;
        let length = self
            .manifest
            .artifact
            .size_bytes
            .ok_or(ReceiptError::InvalidManifest)?;
        let remaining = length.checked_sub(start).ok_or(ReceiptError::Corrupt)?;
        let chunk_len = usize::try_from(remaining.min(SEALED_CHUNK_BYTES as u64))
            .map_err(|_| ReceiptError::Capacity)?;
        if chunk_len == 0 {
            return Err(ReceiptError::Corrupt);
        }
        self.scratch.fill(0);
        let mut read_total = 0usize;
        while read_total < chunk_len {
            self.checkpoint()?;
            let request_len = (chunk_len - read_total).min(SOURCE_READ_BYTES);
            let position = start
                .checked_add(read_total as u64)
                .ok_or(ReceiptError::Capacity)?;
            let result = self.source.read_at(
                position,
                &mut self.scratch[read_total..read_total + request_len],
            );
            self.checkpoint()?;
            let read =
                result.map_err(|_| ReceiptError::Artifact(RuntimeError::ArtifactUnavailable))?;
            if read == 0 || read > request_len {
                return Err(ReceiptError::Artifact(RuntimeError::ArtifactUnavailable));
            }
            read_total = read_total.checked_add(read).ok_or(ReceiptError::Capacity)?;
        }
        let expected = self.chunks.get(index).ok_or(ReceiptError::Corrupt)?;
        if &sha256_bytes(&self.scratch[..chunk_len]) != expected {
            return Err(ReceiptError::Artifact(RuntimeError::IntegrityMismatch));
        }
        self.checkpoint()?;
        self.authenticated_chunk = Some((index, chunk_len));
        Ok(chunk_len)
    }
    fn read_verified(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, ReceiptError> {
        self.checkpoint()?;
        offset
            .checked_add(u64::try_from(destination.len()).map_err(|_| ReceiptError::Capacity)?)
            .ok_or(ReceiptError::Artifact(RuntimeError::ArtifactUnavailable))?;
        let length = self
            .manifest
            .artifact
            .size_bytes
            .ok_or(ReceiptError::InvalidManifest)?;
        let available = length
            .checked_sub(offset)
            .ok_or(ReceiptError::Artifact(RuntimeError::ArtifactUnavailable))?;
        let wanted = usize::try_from(available.min(destination.len() as u64))
            .map_err(|_| ReceiptError::Capacity)?;
        let mut copied = 0usize;
        while copied < wanted {
            let position = offset
                .checked_add(copied as u64)
                .ok_or(ReceiptError::Capacity)?;
            let index = usize::try_from(position / SEALED_CHUNK_BYTES as u64)
                .map_err(|_| ReceiptError::Capacity)?;
            let within = usize::try_from(position % SEALED_CHUNK_BYTES as u64)
                .map_err(|_| ReceiptError::Capacity)?;
            let chunk_len = self.read_chunk(index)?;
            let copy_len =
                (chunk_len.checked_sub(within).ok_or(ReceiptError::Corrupt)?).min(wanted - copied);
            if copy_len == 0 {
                return Err(ReceiptError::Corrupt);
            }
            self.checkpoint()?;
            destination[copied..copied + copy_len]
                .copy_from_slice(&self.scratch[within..within + copy_len]);
            copied += copy_len;
        }
        // Across-chunk success is published only after all chunks and final live
        // checks pass; any later failure clears the entire caller destination.
        self.checkpoint()?;
        Ok(copied)
    }
}
impl<R, L, A, C> ModelArtifactReader for AuthenticatedModelStoreSnapshot<R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
    fn artifact_id(&self) -> &ArtifactId {
        let ArtifactReference::ModelStore { artifact_id } = &self.manifest.artifact.reference;
        artifact_id
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        None
    }
    fn len(&self) -> u64 {
        self.manifest.artifact.size_bytes.unwrap_or(0)
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        destination.fill(0);
        match self.read_verified(offset, destination) {
            Ok(read) => Ok(read),
            Err(error) => {
                self.fail(error);
                self.authenticated_chunk = None;
                self.scratch.fill(0);
                destination.fill(0);
                Err(
                    if offset > self.len() || offset.checked_add(destination.len() as u64).is_none()
                    {
                        ArtifactReadError::OutOfRange
                    } else {
                        ArtifactReadError::Unavailable
                    },
                )
            }
        }
    }
}
impl<R, L, A, C> ModelStoreArtifact for AuthenticatedModelStoreSnapshot<R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
    fn store_identity(&self) -> ModelStoreIdentity {
        self.identity
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn validate_lease(&self) -> Result<(), ReceiptError> {
        self.checkpoint()
    }
}
struct AccumulatingReader<'a, R, L, A: LiveAuthority, C>
where
    A::Session: Clone,
{
    snapshot: &'a mut AuthenticatedModelStoreSnapshot<R, L, A, C>,
    offset: u64,
    used: usize,
}
impl<R, L, A, C> ModelArtifactReader for AccumulatingReader<'_, R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
    fn artifact_id(&self) -> &ArtifactId {
        self.snapshot.artifact_id()
    }
    fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
        None
    }
    fn len(&self) -> u64 {
        self.snapshot.len()
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, ArtifactReadError> {
        if let Err(error) = self.snapshot.checkpoint() {
            self.snapshot.fail(error);
            destination.fill(0);
            return Err(ArtifactReadError::Unavailable);
        }
        if offset != self.offset {
            self.snapshot.fail(ReceiptError::Stale);
            destination.fill(0);
            return Err(ArtifactReadError::Unavailable);
        }
        let result = self.snapshot.source.read_at(offset, destination);
        if let Err(error) = self.snapshot.checkpoint() {
            self.snapshot.fail(error);
            destination.fill(0);
            return Err(ArtifactReadError::Unavailable);
        }
        let read = match result {
            Ok(read) if read != 0 && read <= destination.len() => read,
            _ => {
                self.snapshot
                    .fail(ReceiptError::Artifact(RuntimeError::ArtifactUnavailable));
                destination.fill(0);
                return Err(ArtifactReadError::Unavailable);
            }
        };
        let mut consumed = 0usize;
        while consumed < read {
            let take = (SEALED_CHUNK_BYTES - self.used).min(read - consumed);
            self.snapshot.scratch[self.used..self.used + take]
                .copy_from_slice(&destination[consumed..consumed + take]);
            self.used += take;
            consumed += take;
            if self.used == SEALED_CHUNK_BYTES {
                self.snapshot
                    .chunks
                    .push(sha256_bytes(&self.snapshot.scratch));
                self.used = 0;
            }
        }
        self.offset = self
            .offset
            .checked_add(read as u64)
            .ok_or(ArtifactReadError::Unavailable)?;
        Ok(read)
    }
}

impl<R, L, A, C> immutable_seal::Sealed for AuthenticatedModelStoreSnapshot<R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
}
impl<R, L, A, C> AuthenticatedImmutableArtifact for AuthenticatedModelStoreSnapshot<R, L, A, C>
where
    R: ModelArtifactReader,
    L: AuthenticatedArtifactLease,
    A: AuthenticatedSnapshotAuthority,
    A::Session: Clone,
    C: CancellationToken,
{
}

#[cfg(test)]
mod tests {
    // Synthetic canonical owner/source only. No OS Session, guest storage or
    // native inference claim. Session deliberately Clone but not Copy.
    use super::*;
    use alloc::{format, rc::Rc, string::String, vec};
    use core::cell::Cell;

    #[derive(Clone)]
    struct Session(String);
    struct State {
        alive: Cell<bool>,
        epoch: Cell<u64>,
        revoke_stage: Option<AuthenticatedSnapshotStage>,
        epoch_stage: Option<AuthenticatedSnapshotStage>,
        stages: Cell<usize>,
    }
    struct Authority(Rc<State>);
    impl LiveAuthority for Authority {
        type Session = Session;
        type Stamp = u64;
        fn check(&self, live: &Session) -> Result<u64, ReceiptError> {
            if !self.0.alive.get() || live.0 != "synthetic-owner" {
                Err(ReceiptError::Denied)
            } else {
                Ok(1)
            }
        }
    }
    impl LiveRequestAuthority for Authority {
        fn check_request_epoch(&self, live: &Session, epoch: u64) -> Result<(), ReceiptError> {
            self.check(live)?;
            if epoch == 0 || epoch != self.0.epoch.get() {
                Err(ReceiptError::Stale)
            } else {
                Ok(())
            }
        }
    }
    impl AuthenticatedSnapshotAuthority for Authority {
        fn check_os_terms(&self, live: &Session, reference: &str) -> Result<(), ReceiptError> {
            self.check(live)?;
            if reference.is_empty() {
                Err(ReceiptError::TermsRequired)
            } else {
                Ok(())
            }
        }
        fn current_request_epoch(&self, live: &Session) -> Result<u64, ReceiptError> {
            self.check(live)?;
            Ok(self.0.epoch.get())
        }
        fn snapshot_stage(&self, stage: AuthenticatedSnapshotStage) {
            self.0.stages.set(self.0.stages.get() + 1);
            if self.0.revoke_stage == Some(stage) {
                self.0.alive.set(false);
            }
            if self.0.epoch_stage == Some(stage) {
                self.0.epoch.set(2);
            }
        }
    }
    struct Lease(ModelManifest);
    impl AuthenticatedArtifactLease for Lease {
        fn store_identity(&self) -> ModelStoreIdentity {
            ModelStoreIdentity(12)
        }
        fn revision(&self) -> u64 {
            3
        }
        fn check(&self, manifest: &ModelManifest) -> Result<(), ReceiptError> {
            if manifest == &self.0 {
                Ok(())
            } else {
                Err(ReceiptError::Stale)
            }
        }
    }
    struct Source {
        id: ArtifactId,
        reads: Rc<Cell<usize>>,
        bytes: Vec<u8>,
    }
    impl ModelArtifactReader for Source {
        fn artifact_id(&self) -> &ArtifactId {
            &self.id
        }
        fn verified_integrity(&self) -> Option<&IntegrityMetadata> {
            None
        }
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, ArtifactReadError> {
            self.reads.set(self.reads.get() + 1);
            let offset = usize::try_from(offset).map_err(|_| ArtifactReadError::OutOfRange)?;
            let available = self
                .bytes
                .len()
                .checked_sub(offset)
                .ok_or(ArtifactReadError::OutOfRange)?;
            let n = available.min(out.len());
            out[..n].copy_from_slice(&self.bytes[offset..offset + n]);
            Ok(n)
        }
    }
    struct Cancel;
    impl CancellationToken for Cancel {
        fn is_cancelled(&self) -> bool {
            false
        }
    }
    type Snapshot = AuthenticatedModelStoreSnapshot<Source, Lease, Authority, Cancel>;
    fn capture(
        revoke: Option<AuthenticatedSnapshotStage>,
        epoch: Option<AuthenticatedSnapshotStage>,
    ) -> (Result<Snapshot, ReceiptError>, Rc<State>, Rc<Cell<usize>>) {
        let bytes = vec![17; 100];
        let mut manifest =
            ModelManifest::parse_json(include_bytes!("../tests/fixtures/granite-4.2-3b.json"))
                .unwrap();
        manifest.artifact.size_bytes = Some(bytes.len() as u64);
        manifest.artifact.integrity.as_mut().unwrap().digest = sha256_bytes(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let state = Rc::new(State {
            alive: Cell::new(true),
            epoch: Cell::new(1),
            revoke_stage: revoke,
            epoch_stage: epoch,
            stages: Cell::new(0),
        });
        let reads = Rc::new(Cell::new(0));
        let source = Source {
            id: ArtifactId::new("ibm.granite-4.2-3b").unwrap(),
            reads: reads.clone(),
            bytes,
        };
        let result = Snapshot::capture_verified(
            source,
            Lease(manifest.clone()),
            Authority(state.clone()),
            &Session(String::from("synthetic-owner")),
            Cancel,
            &manifest,
        );
        (result, state, reads)
    }
    #[test]
    fn non_copy_session_and_same_authenticated_view_move_preserve_cache_and_lease() {
        let (snapshot, state, reads) = capture(None, None);
        let mut snapshot = snapshot.unwrap();
        assert_eq!(state.stages.get(), 4);
        assert_eq!(reads.get(), 1); // complete bounded initial hash
        assert_eq!(snapshot.read_at(0, &mut [0; 7]), Ok(7));
        assert_eq!(reads.get(), 2); // first complete authenticated chunk
        fn move_view<T: AuthenticatedImmutableArtifact>(view: T) -> T {
            view
        }
        let mut moved = move_view(snapshot);
        assert_eq!(moved.read_at(1, &mut [0; 7]), Ok(7));
        assert_eq!(reads.get(), 2); // same scratch/cache, no rehash/copy/reopen
        state.epoch.set(2);
        let mut out = [0xcc; 7];
        assert!(moved.read_at(0, &mut out).is_err());
        assert_eq!(out, [0; 7]);
        assert_eq!(moved.validate_lease(), Err(ReceiptError::Stale));
    }
    #[test]
    fn every_capture_diagnostic_callback_revalidates_live_and_pinned_epoch() {
        use AuthenticatedSnapshotStage::*;
        for stage in [SealBegin, HashBegin, HashDone, SealDone] {
            assert!(matches!(
                capture(Some(stage), None).0,
                Err(ReceiptError::Denied)
            ));
            assert!(matches!(
                capture(None, Some(stage)).0,
                Err(ReceiptError::Stale)
            ));
        }
    }
}
