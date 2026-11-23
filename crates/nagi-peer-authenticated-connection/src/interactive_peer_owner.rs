use super::{KernelAuthenticatedUnixPeer, PeerAuthenticationError, PeerConnectionId, PeerOption};
use std::collections::HashSet;
use std::fmt;
use std::num::NonZeroU64;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;

const MAX_ACCEPTED_CONNECTIONS: usize = 1024;
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(1);
const ACCEPT_INTERRUPTED_RETRY_LIMIT: u32 = 32;
static NEXT_TRUSTED_OWNER_KEY: AtomicU64 = AtomicU64::new(1);

/// Opaque key allocated to one leaf listener registration. C2 must compare it
/// with the key retained by its spawn-validated endpoint record before C1 sees
/// the proof. C1 may use it only as a lookup key in its own trusted account
/// mapping; the key carries no account or capability itself.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TrustedInteractivePeerKey(NonZeroU64);

impl fmt::Debug for TrustedInteractivePeerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrustedInteractivePeerKey(<opaque>)")
    }
}

/// Opaque composite selector for one accepted connection and C2
/// registration/session selectors supplied by the live C2 entry. Its fields
/// cannot be constructed or read by consumers; the selector is not identity
/// evidence until C2 owner checks and C1's exact SessionLease agree.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct AuthenticatedInteractivePeerSelector {
    connection: PeerConnectionId,
    owner_key: TrustedInteractivePeerKey,
    c2_registration_id: NonZeroU64,
    run_generation: NonZeroU64,
    endpoint_generation: NonZeroU64,
    process_instance_selector: NonZeroU64,
    app_id: u64,
    app_session_id: u64,
    user_session_generation: NonZeroU64,
}

impl fmt::Debug for AuthenticatedInteractivePeerSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedInteractivePeerSelector(<opaque>)")
    }
}

impl AuthenticatedInteractivePeerSelector {
    /// Compare the selector with the exact C2-owned endpoint tuple. This is a
    /// read-only comparison; it does not mint or validate the peer proof by
    /// itself.
    pub fn matches_c2_binding(
        &self,
        owner_key: TrustedInteractivePeerKey,
        c2_registration_id: NonZeroU64,
        run_generation: NonZeroU64,
        endpoint_generation: NonZeroU64,
        process_instance_selector: NonZeroU64,
    ) -> bool {
        self.owner_key == owner_key
            && self.c2_registration_id == c2_registration_id
            && self.run_generation == run_generation
            && self.endpoint_generation == endpoint_generation
            && self.process_instance_selector == process_instance_selector
    }

    /// Compare the selector with the exact C1 session lease tuple. This is a
    /// read-only comparison; C1 must still validate its live session owner.
    pub fn matches_session(
        &self,
        app_id: u64,
        app_session_id: u64,
        user_session_generation: NonZeroU64,
    ) -> bool {
        self.app_id == app_id
            && self.app_session_id == app_session_id
            && self.user_session_generation == user_session_generation
    }
}

/// Borrowed proof delivered only while its accepted handle, stream, and leaf
/// owner are borrowed. The callback bound on `with_live_interactive_peer`
/// prevents this borrow from escaping. Membership is checked before callback
/// without holding the connection-set lock; the non-cloneable handle keeps
/// that membership live until the callback returns, and Drop retires it. The
/// leaf registration API alone does not establish canonical C2 ownership;
/// C2 must match `owner_key()` against its spawn-validated endpoint record
/// before passing this proof to C1.
pub struct AuthenticatedInteractivePeer<'connection> {
    owner_key: &'connection TrustedInteractivePeerKey,
    selector: &'connection AuthenticatedInteractivePeerSelector,
    kernel_peer: &'connection KernelAuthenticatedUnixPeer,
    _stream: &'connection UnixStream,
    _owner_guard: &'connection OwnerRecord,
}

impl AuthenticatedInteractivePeer<'_> {
    /// Opaque key allocated to this listener registration. C2 compares it
    /// against the exact endpoint owner's retained key before the C1 callback.
    pub fn owner_key(&self) -> &TrustedInteractivePeerKey {
        self.owner_key
    }

    /// One opaque selector binding the kernel connection, C2 registration,
    /// run/endpoint generations, process-instance selector, and exact
    /// `(app_id, app_session_id, user_session_generation)` tuple.
    pub fn selector(&self) -> &AuthenticatedInteractivePeerSelector {
        self.selector
    }

    /// Kernel-reported peer snapshot for C1's own real credential checks.
    /// These values do not establish a Nagi account or principal by themselves.
    pub fn kernel_peer(&self) -> &KernelAuthenticatedUnixPeer {
        self.kernel_peer
    }
}

struct OwnerRecord {
    key: TrustedInteractivePeerKey,
    run_generation: NonZeroU64,
    endpoint_generation: NonZeroU64,
    process_instance_selector: NonZeroU64,
    expires_at: Instant,
    revoked: AtomicBool,
    // IDs for handles currently alive. AcceptedInteractivePeerHandle::drop
    // retires them, so this is a live-handle bound rather than a lifetime cap.
    seen_connections: Mutex<HashSet<PeerConnectionId>>,
}

impl OwnerRecord {
    fn require_current(&self, now: Instant) -> Result<(), PeerAuthenticationError> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(PeerAuthenticationError::OwnerRevoked);
        }
        if now >= self.expires_at {
            return Err(PeerAuthenticationError::OwnerExpired);
        }
        Ok(())
    }
}

/// Leaf owner for a C2 accepted listener. It consumes the listener, configures
/// current-writer credentials before accepting, and records each accepted
/// kernel-authenticated connection under one opaque owner key.
///
/// The PID 1 check in [`register_from_c2_owner`](Self::register_from_c2_owner)
/// rejects ordinary non-PID-1 consumers. It does not prove that the caller is
/// the canonical C2 registration owner when arbitrary code shares the PID 1
/// process. Until that authority is represented by an unforgeable registration
/// capability or an independently trusted C1 owner-key enrollment, this API
/// alone must not be treated as a C1 caller seal.
pub struct C2AcceptedPeerListenerOwner {
    listener: Mutex<Option<UnixListener>>,
    owner: Arc<OwnerRecord>,
}

impl fmt::Debug for C2AcceptedPeerListenerOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("C2AcceptedPeerListenerOwner")
            .field("owner_key", &self.owner.key)
            .field("active", &!self.owner.revoked.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl C2AcceptedPeerListenerOwner {
    /// Return the opaque key retained by this exact listener owner. C2 uses
    /// this to compare a borrowed proof against its spawn-validated endpoint
    /// record before it invokes C1.
    pub fn owner_key(&self) -> TrustedInteractivePeerKey {
        self.owner.key
    }

    /// Take ownership of a listener after C2 validates the paired spawn and
    /// endpoint record. The generation and process-instance values are opaque
    /// selectors supplied by that C2 path; neither is independent identity
    /// evidence, and this public method does not validate their provenance.
    ///
    /// This process-boundary guard is not itself proof that the caller is the
    /// canonical C2 endpoint-registration path. The candidate integration must
    /// call it only after validating its sealed spawn and endpoint owners, and
    /// C1 must independently reject owner keys that were not enrolled through
    /// its trusted C2 registration path.
    pub fn register_from_c2_owner(
        listener: UnixListener,
        run_generation: NonZeroU64,
        endpoint_generation: NonZeroU64,
        process_instance_selector: NonZeroU64,
        expires_at: Instant,
    ) -> Result<Self, PeerAuthenticationError> {
        require_pid1_owner_process()?;
        if Instant::now() >= expires_at {
            return Err(PeerAuthenticationError::OwnerExpired);
        }
        configure_listener(&listener)?;
        listener
            .set_nonblocking(true)
            .map_err(|error| map_listener_error(error.raw_os_error()))?;
        let key = TrustedInteractivePeerKey(allocate_owner_key()?);
        Ok(Self {
            listener: Mutex::new(Some(listener)),
            owner: Arc::new(OwnerRecord {
                key,
                run_generation,
                endpoint_generation,
                process_instance_selector,
                expires_at,
                revoked: AtomicBool::new(false),
                seen_connections: Mutex::new(HashSet::new()),
            }),
        })
    }

    /// Accept one connection from this listener before `deadline`. The
    /// returned connection handle is the only path that can issue the
    /// borrowed interactive-peer proof for this accepted stream.
    pub fn accept_until(
        &self,
        deadline: Instant,
    ) -> Result<AcceptedInteractivePeerConnection, PeerAuthenticationError> {
        let deadline = deadline.min(self.owner.expires_at);
        let mut interrupted_retries = 0;
        let stream = loop {
            self.owner.require_current(Instant::now())?;
            if Instant::now() >= deadline {
                return Err(PeerAuthenticationError::AcceptDeadlineExpired);
            }
            let accepted = {
                let listener = self
                    .listener
                    .try_lock()
                    .map_err(|_| PeerAuthenticationError::OwnerUnavailable)?;
                listener
                    .as_ref()
                    .ok_or(PeerAuthenticationError::OwnerRevoked)?
                    .accept()
            };
            match accepted {
                Ok((stream, _)) => {
                    if Instant::now() >= deadline {
                        return Err(PeerAuthenticationError::AcceptDeadlineExpired);
                    }
                    break stream;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    interrupted_retries = 0;
                    wait_before_deadline(deadline, ACCEPT_POLL_INTERVAL)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    interrupted_retries += 1;
                    if interrupted_retries >= ACCEPT_INTERRUPTED_RETRY_LIMIT {
                        return Err(map_listener_error(error.raw_os_error()));
                    }
                    wait_before_deadline(deadline, Duration::from_micros(100))?;
                }
                Err(error) => return Err(map_listener_error(error.raw_os_error())),
            }
        };

        require_accepted_stream_passcred(&stream)?;
        let kernel_peer = Arc::new(KernelAuthenticatedUnixPeer::authenticate(&stream)?);
        kernel_peer.revalidate(&stream)?;
        self.owner.require_current(Instant::now())?;
        if Instant::now() >= deadline {
            return Err(PeerAuthenticationError::AcceptDeadlineExpired);
        }
        let connection_id = kernel_peer.connection_id();
        let mut seen = self
            .owner
            .seen_connections
            .try_lock()
            .map_err(|_| PeerAuthenticationError::OwnerUnavailable)?;
        if seen.len() >= MAX_ACCEPTED_CONNECTIONS || !seen.insert(connection_id) {
            return Err(PeerAuthenticationError::OwnerUnavailable);
        }
        drop(seen);

        Ok(AcceptedInteractivePeerConnection {
            stream,
            handle: AcceptedInteractivePeerHandle {
                owner: Arc::clone(&self.owner),
                kernel_peer,
                callback_active: AtomicBool::new(false),
            },
        })
    }

    /// Invalidate this exact listener registration. Revoke takes effect before
    /// the descriptor is closed, so callbacks already in flight fail their
    /// post-callback owner check. The listener mutex covers only one
    /// nonblocking accept syscall, so taking it here closes the descriptor
    /// instead of returning an outcome-unknown contention error.
    pub fn revoke(&self) -> Result<(), PeerAuthenticationError> {
        self.owner.revoked.store(true, Ordering::SeqCst);
        let mut listener = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        listener.take();
        Ok(())
    }
}

impl Drop for C2AcceptedPeerListenerOwner {
    fn drop(&mut self) {
        self.owner.revoked.store(true, Ordering::SeqCst);
    }
}

/// An accepted stream paired with its leaf-issued, non-forgeable owner handle.
pub struct AcceptedInteractivePeerConnection {
    stream: UnixStream,
    handle: AcceptedInteractivePeerHandle,
}

impl AcceptedInteractivePeerConnection {
    /// Transfer the exact accepted stream into C2's existing one-shot transport
    /// path while retaining the leaf handle in the endpoint checkout.
    pub fn into_stream_and_handle(self) -> (UnixStream, AcceptedInteractivePeerHandle) {
        (self.stream, self.handle)
    }
}

/// Non-cloneable handle to one connection accepted by a leaf-owned listener
/// registration. It cannot be created from a raw stream or credential
/// snapshot; C2 still has to match its owner key and selector to the canonical
/// endpoint registration before the proof can reach C1.
pub struct AcceptedInteractivePeerHandle {
    owner: Arc<OwnerRecord>,
    kernel_peer: Arc<KernelAuthenticatedUnixPeer>,
    callback_active: AtomicBool,
}

struct PeerCallbackInProgress<'a>(&'a AtomicBool);

impl Drop for PeerCallbackInProgress<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Drop for AcceptedInteractivePeerHandle {
    fn drop(&mut self) {
        let mut seen = self
            .owner
            .seen_connections
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.remove(&self.kernel_peer.connection_id());
    }
}

impl fmt::Debug for AcceptedInteractivePeerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcceptedInteractivePeerHandle")
            .field("owner_key", &self.owner.key)
            .field("connection", &self.kernel_peer.connection_id())
            .finish_non_exhaustive()
    }
}

impl AcceptedInteractivePeerHandle {
    /// Run C1's synchronous caller/session mapping while both this accepted
    /// stream and the leaf registration are live. The selector's C2
    /// registration and exact SessionLease tuple are supplied by the trusted
    /// live-entry callsite; the caller cannot supply an owner-validation
    /// closure or construct the proof value. Leaf owner and kernel checks run
    /// before and after the callback. One callback per accepted handle may run
    /// at a time; this per-handle gate does not hold the connection-set lock.
    #[allow(clippy::too_many_arguments)]
    pub fn with_live_interactive_peer<T, E>(
        &self,
        stream: &UnixStream,
        c2_registration_id: NonZeroU64,
        app_id: u64,
        app_session_id: u64,
        user_session_generation: NonZeroU64,
        callback: impl for<'connection> FnOnce(
            AuthenticatedInteractivePeer<'connection>,
        ) -> Result<T, E>,
    ) -> Result<Result<T, E>, PeerAuthenticationError> {
        let accepted = self
            .owner
            .seen_connections
            .try_lock()
            .map_err(|_| PeerAuthenticationError::OwnerUnavailable)?;
        if !accepted.contains(&self.kernel_peer.connection_id()) {
            return Err(PeerAuthenticationError::OwnerRevoked);
        }
        drop(accepted);
        if self
            .callback_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(PeerAuthenticationError::OwnerUnavailable);
        }
        let _callback_in_progress = PeerCallbackInProgress(&self.callback_active);
        self.owner.require_current(Instant::now())?;
        self.kernel_peer.revalidate(stream)?;

        let selector = AuthenticatedInteractivePeerSelector {
            connection: self.kernel_peer.connection_id(),
            owner_key: self.owner.key,
            c2_registration_id,
            run_generation: self.owner.run_generation,
            endpoint_generation: self.owner.endpoint_generation,
            process_instance_selector: self.owner.process_instance_selector,
            app_id,
            app_session_id,
            user_session_generation,
        };
        let proof = AuthenticatedInteractivePeer {
            owner_key: &self.owner.key,
            selector: &selector,
            kernel_peer: self.kernel_peer.as_ref(),
            _stream: stream,
            _owner_guard: self.owner.as_ref(),
        };
        let result = callback(proof);

        self.kernel_peer.revalidate(stream)?;
        self.owner.require_current(Instant::now())?;
        Ok(result)
    }
}

fn allocate_owner_key() -> Result<NonZeroU64, PeerAuthenticationError> {
    let raw = NEXT_TRUSTED_OWNER_KEY
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(1)
        })
        .map_err(|_| PeerAuthenticationError::OwnerUnavailable)?;
    NonZeroU64::new(raw).ok_or(PeerAuthenticationError::OwnerUnavailable)
}

fn wait_before_deadline(
    deadline: Instant,
    preferred_delay: Duration,
) -> Result<(), PeerAuthenticationError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(PeerAuthenticationError::AcceptDeadlineExpired);
    }
    let delay = deadline.saturating_duration_since(now).min(preferred_delay);
    if delay.is_zero() {
        return Err(PeerAuthenticationError::AcceptDeadlineExpired);
    }
    thread::sleep(delay);
    if Instant::now() >= deadline {
        return Err(PeerAuthenticationError::AcceptDeadlineExpired);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_listener(listener: &UnixListener) -> Result<(), PeerAuthenticationError> {
    let enabled: libc::c_int = 1;
    // SAFETY: `enabled` is a live integer buffer and the listener is borrowed.
    let set = unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as libc::socklen_t,
        )
    };
    if set != 0 {
        return Err(map_listener_error(
            std::io::Error::last_os_error().raw_os_error(),
        ));
    }
    require_passcred(listener.as_raw_fd())
}

#[cfg(not(target_os = "linux"))]
fn configure_listener(_listener: &UnixListener) -> Result<(), PeerAuthenticationError> {
    Err(PeerAuthenticationError::Unavailable)
}

#[cfg(target_os = "linux")]
fn require_accepted_stream_passcred(stream: &UnixStream) -> Result<(), PeerAuthenticationError> {
    require_passcred(stream.as_raw_fd())
}

#[cfg(not(target_os = "linux"))]
fn require_accepted_stream_passcred(_stream: &UnixStream) -> Result<(), PeerAuthenticationError> {
    Err(PeerAuthenticationError::Unavailable)
}

#[cfg(target_os = "linux")]
fn require_passcred(fd: std::os::fd::RawFd) -> Result<(), PeerAuthenticationError> {
    let mut enabled: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&enabled) as libc::socklen_t;
    // SAFETY: `enabled` and `length` are valid output buffers for getsockopt.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&mut enabled as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(PeerAuthenticationError::KernelRead {
            option: PeerOption::AcceptedListener,
            raw_os_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }
    if length as usize != std::mem::size_of_val(&enabled) || enabled != 1 {
        return Err(PeerAuthenticationError::MalformedKernelResponse);
    }
    Ok(())
}

fn map_listener_error(raw_os_error: Option<i32>) -> PeerAuthenticationError {
    PeerAuthenticationError::KernelRead {
        option: PeerOption::AcceptedListener,
        raw_os_error,
    }
}

fn require_pid1_owner_process() -> Result<(), PeerAuthenticationError> {
    if std::process::id() != 1 {
        return Err(PeerAuthenticationError::OwnerUnavailable);
    }
    #[cfg(target_os = "linux")]
    {
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(PeerAuthenticationError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct RemoveSocketPath(PathBuf);

    impl Drop for RemoveSocketPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[cfg(target_os = "linux")]
    fn test_listener_owner() -> (C2AcceptedPeerListenerOwner, PathBuf, RemoveSocketPath) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/nagi-peer-owner-{}-{unique}.sock",
            std::process::id(),
        ));
        let listener = UnixListener::bind(&path).expect("bind isolated test listener");
        configure_listener(&listener).expect("configure test listener credentials");
        listener
            .set_nonblocking(true)
            .expect("set test listener nonblocking");
        let owner = C2AcceptedPeerListenerOwner {
            listener: Mutex::new(Some(listener)),
            owner: Arc::new(OwnerRecord {
                key: TrustedInteractivePeerKey(allocate_owner_key().expect("owner key")),
                run_generation: NonZeroU64::new(1).unwrap(),
                endpoint_generation: NonZeroU64::new(2).unwrap(),
                process_instance_selector: NonZeroU64::new(3).unwrap(),
                expires_at: Instant::now() + Duration::from_secs(30),
                revoked: AtomicBool::new(false),
                seen_connections: Mutex::new(HashSet::new()),
            }),
        };
        (owner, path.clone(), RemoveSocketPath(path))
    }

    #[test]
    fn non_pid1_consumer_cannot_register_its_own_listener_key() {
        if std::process::id() == 1 {
            return;
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nagi-peer-owner-untrusted-{}-{unique}.sock",
            std::process::id(),
        ));
        let listener = UnixListener::bind(&path).expect("bind isolated test listener");
        let _remove_path = RemoveSocketPath(path);
        let error = C2AcceptedPeerListenerOwner::register_from_c2_owner(
            listener,
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1).unwrap(),
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("a general consumer must not mint a trusted listener key");
        assert_eq!(error, PeerAuthenticationError::OwnerUnavailable);
    }

    #[test]
    fn selector_comparisons_require_the_exact_c2_and_session_tuples() {
        let selector = AuthenticatedInteractivePeerSelector {
            connection: PeerConnectionId(1),
            owner_key: TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            c2_registration_id: NonZeroU64::new(3).unwrap(),
            run_generation: NonZeroU64::new(4).unwrap(),
            endpoint_generation: NonZeroU64::new(5).unwrap(),
            process_instance_selector: NonZeroU64::new(6).unwrap(),
            app_id: 7,
            app_session_id: 8,
            user_session_generation: NonZeroU64::new(9).unwrap(),
        };

        assert!(selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            NonZeroU64::new(3).unwrap(),
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(5).unwrap(),
            NonZeroU64::new(6).unwrap(),
        ));
        assert!(!selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            NonZeroU64::new(30).unwrap(),
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(5).unwrap(),
            NonZeroU64::new(6).unwrap(),
        ));
        assert!(!selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            NonZeroU64::new(3).unwrap(),
            NonZeroU64::new(40).unwrap(),
            NonZeroU64::new(5).unwrap(),
            NonZeroU64::new(6).unwrap(),
        ));
        assert!(!selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            NonZeroU64::new(3).unwrap(),
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(50).unwrap(),
            NonZeroU64::new(6).unwrap(),
        ));
        assert!(!selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(2).unwrap()),
            NonZeroU64::new(3).unwrap(),
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(5).unwrap(),
            NonZeroU64::new(60).unwrap(),
        ));
        assert!(!selector.matches_c2_binding(
            TrustedInteractivePeerKey(NonZeroU64::new(20).unwrap()),
            NonZeroU64::new(3).unwrap(),
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(5).unwrap(),
            NonZeroU64::new(6).unwrap(),
        ));

        assert!(selector.matches_session(7, 8, NonZeroU64::new(9).unwrap()));
        assert!(!selector.matches_session(70, 8, NonZeroU64::new(9).unwrap()));
        assert!(!selector.matches_session(7, 80, NonZeroU64::new(9).unwrap()));
        assert!(!selector.matches_session(7, 8, NonZeroU64::new(90).unwrap()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn callback_does_not_hold_seen_connections_lock_during_another_accept() {
        use std::sync::Barrier;

        let (owner, path, _remove_path) = test_listener_owner();
        let client = UnixStream::connect(&path).expect("connect first test peer");
        let (stream, handle) = owner
            .accept_until(Instant::now() + Duration::from_secs(2))
            .expect("accept first test peer")
            .into_stream_and_handle();
        let callback_entered = Arc::new(Barrier::new(2));
        let callback_release = Arc::new(Barrier::new(2));

        std::thread::scope(|scope| {
            let callback_entered_for_thread = Arc::clone(&callback_entered);
            let callback_release_for_thread = Arc::clone(&callback_release);
            let handle_ref = &handle;
            let stream_ref = &stream;
            let callback = scope.spawn(move || {
                handle_ref.with_live_interactive_peer(
                    stream_ref,
                    NonZeroU64::new(4).unwrap(),
                    5,
                    6,
                    NonZeroU64::new(7).unwrap(),
                    |_peer| {
                        callback_entered_for_thread.wait();
                        callback_release_for_thread.wait();
                        Ok::<_, ()>(())
                    },
                )
            });
            callback_entered.wait();

            let duplicate_callback = handle.with_live_interactive_peer(
                &stream,
                NonZeroU64::new(4).unwrap(),
                5,
                6,
                NonZeroU64::new(7).unwrap(),
                |_peer| Ok::<_, ()>(()),
            );
            assert!(matches!(
                duplicate_callback,
                Err(PeerAuthenticationError::OwnerUnavailable)
            ));

            let second_client = UnixStream::connect(&path).expect("connect second test peer");
            let (second_stream, second_handle) = owner
                .accept_until(Instant::now() + Duration::from_secs(2))
                .expect("accept while first callback is held")
                .into_stream_and_handle();
            callback_release.wait();

            assert!(callback.join().unwrap().is_ok());
            drop((second_stream, second_handle, second_client));
        });

        drop((stream, handle, client));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn accepted_connection_capacity_is_reclaimed_when_handles_drop() {
        let (owner, path, _remove_path) = test_listener_owner();

        for _ in 0..=MAX_ACCEPTED_CONNECTIONS {
            let client = UnixStream::connect(&path).expect("connect sequential test peer");
            let (stream, handle) = owner
                .accept_until(Instant::now() + Duration::from_secs(2))
                .expect("accept after prior handle was retired")
                .into_stream_and_handle();
            assert_eq!(
                owner.owner.seen_connections.lock().unwrap().len(),
                1,
                "one live accepted handle should occupy one slot"
            );
            drop((stream, handle, client));
            assert!(owner.owner.seen_connections.lock().unwrap().is_empty());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn panicking_callback_does_not_poison_active_connection_registry() {
        let (owner, path, _remove_path) = test_listener_owner();
        let client = UnixStream::connect(&path).expect("connect first test peer");
        let (stream, handle) = owner
            .accept_until(Instant::now() + Duration::from_secs(2))
            .expect("accept first test peer")
            .into_stream_and_handle();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = handle.with_live_interactive_peer(
                &stream,
                NonZeroU64::new(4).unwrap(),
                5,
                6,
                NonZeroU64::new(7).unwrap(),
                |_peer| -> Result<(), ()> { panic!("exercise callback unwind") },
            );
        }));
        assert!(result.is_err());
        assert!(owner.owner.seen_connections.try_lock().is_ok());
        assert!(handle
            .with_live_interactive_peer(
                &stream,
                NonZeroU64::new(4).unwrap(),
                5,
                6,
                NonZeroU64::new(7).unwrap(),
                |_peer| Ok::<_, ()>(()),
            )
            .is_ok());

        let second_client = UnixStream::connect(&path).expect("connect second test peer");
        let (second_stream, second_handle) = owner
            .accept_until(Instant::now() + Duration::from_secs(2))
            .expect("accept after callback panic")
            .into_stream_and_handle();
        drop((second_stream, second_handle, second_client));
        drop((stream, handle, client));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn revoke_waits_for_listener_slot_and_takes_listener() {
        use std::sync::mpsc;

        let (owner, _path, _remove_path) = test_listener_owner();
        let owner = Arc::new(owner);
        let listener_guard = owner.listener.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            let owner_ref = Arc::clone(&owner);
            let revoke = scope.spawn(move || {
                started_tx.send(()).expect("signal revoke start");
                done_tx
                    .send(owner_ref.revoke())
                    .expect("send revoke result");
            });
            started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("revoke thread starts");
            assert!(
                done_rx.try_recv().is_err(),
                "revoke waits for the listener slot"
            );
            drop(listener_guard);
            assert_eq!(
                done_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("revoke finishes after slot release"),
                Ok(())
            );
            revoke.join().expect("revoke thread exits");
        });

        assert!(owner.listener.lock().unwrap().is_none());
        assert!(matches!(
            owner.accept_until(Instant::now() + Duration::from_secs(1)),
            Err(PeerAuthenticationError::OwnerRevoked)
        ));
    }
}
