use super::PeerDenial;
use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
use std::sync::Arc;

/// Legacy candidate-only C2 proof wrapper retained for source compatibility.
/// The live `LiveSessionAuthority::begin_authenticated_peer_binding` bridge
/// does not accept this type; it receives the leaf's borrowed
/// `AuthenticatedInteractivePeer` from the leaf-owned accepted-listener path
/// and returns a staged `PendingBoundPeerSession` transaction.
///
/// This wrapper demonstrates the earlier C2 callback boundary, but does not
/// establish the canonical leaf registration capability or C1's trusted
/// owner-key enrollment. It must not be treated as a C1 caller seal.
///
/// This type has no active issuer in the live path. External consumers cannot
/// build it from a raw stream, a `KernelAuthenticatedUnixPeer`, numeric
/// credentials, or caller-claimed IDs. It does not create a C1 caller,
/// PrincipalId, Session, capability, or effect authority. Its legacy callback
/// shape does not replace the leaf registration boundary or C1's own owner
/// enrollment.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::C2AcceptedPeerProof;
/// let _ = C2AcceptedPeerProof::from_credentials(41, 1000, 1000);
/// ```
/// A raw accepted stream is not a proof issuer:
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::C2AcceptedPeerProof;
/// use std::os::unix::net::UnixStream;
/// fn forge(stream: UnixStream) {
///     let _ = C2AcceptedPeerProof::from_stream(stream);
/// }
/// ```
/// Nor can an external consumer create the proof by filling its private
/// owner-bound fields:
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::C2AcceptedPeerProof;
/// let _ = C2AcceptedPeerProof {
///     authenticated_peer: todo!(),
///     validate: todo!(),
/// };
/// ```
pub struct C2AcceptedPeerProof<'a> {
    authenticated_peer: Arc<KernelAuthenticatedUnixPeer>,
    validate: Box<dyn Fn() -> Result<(), PeerDenial> + Send + Sync + 'a>,
}

impl C2AcceptedPeerProof<'_> {
    /// Legacy callback shape retained for source compatibility. The live C1
    /// binding path accepts `AuthenticatedInteractivePeer` instead.
    pub fn with_live_kernel_peer<T>(
        &self,
        bind: impl FnOnce(&KernelAuthenticatedUnixPeer) -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        (self.validate)()?;
        let bound = bind(self.authenticated_peer.as_ref())?;
        (self.validate)()?;
        Ok(bound)
    }
}
