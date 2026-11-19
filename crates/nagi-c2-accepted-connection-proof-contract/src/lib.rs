#![no_std]

use core::fmt;
use core::marker::PhantomData;
use nagi_peer_authenticated_connection::{KernelAuthenticatedUnixPeer, PeerConnectionId};

#[cfg(test)]
extern crate std;

/// C2's assertion that its spawn/endpoint owner check accepted this exact
/// leaf-authenticated connection. This value is correlation evidence only:
/// consumers must match it against their own borrowed peer proof and perform
/// their own identity, Session, liveness, and authorization checks.
///
/// The borrow ties the assertion to the leaf proof that C2 checked, so it
/// cannot be retained after the accepted-peer callback ends.
pub struct C2AcceptedConnectionProof<'peer> {
    connection_id: PeerConnectionId,
    _peer: PhantomData<&'peer KernelAuthenticatedUnixPeer>,
}

impl fmt::Debug for C2AcceptedConnectionProof<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("C2AcceptedConnectionProof(<opaque>)")
    }
}

impl<'peer> C2AcceptedConnectionProof<'peer> {
    /// Create the shared proof after C2 has validated the accepted peer's
    /// current spawn, endpoint, registration, and owner tuple. C2 enables the
    /// issuer feature and calls this only from its private owner-check path;
    /// the feature is API visibility, not an independent security boundary.
    /// Consumers receive the value and compare it with their own leaf proof.
    #[cfg(feature = "c2-issuer")]
    pub fn after_c2_owner_check(peer: &'peer KernelAuthenticatedUnixPeer) -> Self {
        Self {
            connection_id: peer.connection_id(),
            _peer: PhantomData,
        }
    }

    /// Check that this is the same leaf-authenticated connection. This does
    /// not establish process liveness, account identity, Session ownership,
    /// or authority to perform an effect.
    pub fn matches_kernel_peer(&self, peer: &KernelAuthenticatedUnixPeer) -> bool {
        self.connection_id == peer.connection_id()
    }
}

#[cfg(all(test, feature = "c2-issuer", target_os = "linux"))]
mod tests {
    use super::C2AcceptedConnectionProof;
    use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
    use std::os::unix::net::UnixStream;

    #[test]
    fn proof_matches_only_the_kernel_authenticated_connection_it_binds() {
        let (first_stream, second_stream) = UnixStream::pair().expect("connected Unix pair");
        let first_peer = KernelAuthenticatedUnixPeer::authenticate(&first_stream)
            .expect("authenticate first connection");
        let second_peer = KernelAuthenticatedUnixPeer::authenticate(&second_stream)
            .expect("authenticate second connection");

        let proof = C2AcceptedConnectionProof::after_c2_owner_check(&first_peer);

        assert!(proof.matches_kernel_peer(&first_peer));
        assert!(!proof.matches_kernel_peer(&second_peer));
    }
}
