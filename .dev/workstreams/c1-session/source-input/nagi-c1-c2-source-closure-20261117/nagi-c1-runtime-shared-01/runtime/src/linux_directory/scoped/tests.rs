use super::super::Probe;
use super::*;
use crate::session_fixture::{Fixture, Issuer};
use std::cell::RefCell;
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "scoped-fd-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn fd(&self) -> OwnedFd {
        File::open(&self.0).unwrap().into()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn is_open(raw: RawFd) -> bool {
    match std::fs::metadata(format!("/proc/self/fd/{raw}")) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!("self fd inspection failed: {error}"),
    }
}
fn assert_closed(raw: RawFd, probe: &Probe) {
    assert!(!is_open(raw));
    for fd in probe.roots.borrow().iter() {
        assert!(!is_open(*fd));
    }
}
fn input(fd: OwnedFd, fixture: &Fixture) -> (DirectoryBackend, Probe) {
    let backend = DirectoryBackend::from_external(
        fd,
        fixture.identity.user_id(),
        fixture.identity.profile_id().clone(),
    );
    let probe = backend.probe.clone();
    (backend, probe)
}

// Test-only policy. Real decisions must be supplied by the runtime owner.
struct Policy {
    result: Result<(), StorageRootFailure>,
    calls: usize,
    hook: Option<Box<dyn FnOnce()>>,
}
impl Policy {
    fn allow() -> Self {
        Self {
            result: Ok(()),
            calls: 0,
            hook: None,
        }
    }
}
impl ProfileRootAuthorizer for Policy {
    fn authorize_profile_root(
        &mut self,
        _: &dyn TrustedCallerContext,
        identity: &ResolvedIdentity,
        profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        assert_eq!(identity.profile_id(), profile);
        self.calls += 1;
        if let Some(hook) = self.hook.take() {
            hook()
        }
        self.result
    }
}
struct Resolver(Rc<RefCell<Option<Issuer>>>);
impl CurrentIdentityResolver for Resolver {
    fn resolve_current_identity(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<ResolvedIdentity, identity::ResolutionError> {
        self.0
            .borrow_mut()
            .as_mut()
            .unwrap()
            .resolve_current_identity(caller)
    }
}

#[test]
fn scoped_read_write_reauthorize_and_borrow_only_the_sdk_bound_root() {
    let sandbox = Sandbox::new();
    let mut fixture = Fixture::new();
    let fd = sandbox.fd();
    let raw = fd.as_raw_fd();
    let (input, probe) = input(fd, &fixture);
    let mut policy = Policy::allow();
    let mut directory = ScopedDirectory::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        CancellationToken::new(),
        input,
        &mut policy,
    )
    .unwrap();
    assert_eq!(policy.calls, 1);
    // IDs are passed to the real SDK but never mapped to invented filenames.
    assert_eq!(
        directory.read(ObjectId(0), &mut policy),
        Err(SupplyFailure::Profile(StorageRootFailure::Unavailable))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.reads.get(), 1);
    assert_eq!(
        directory.write(ObjectId(u64::MAX), b"not written", &mut policy),
        Err(SupplyFailure::Profile(StorageRootFailure::Unavailable))
    );
    assert_eq!(policy.calls, 3);
    assert_eq!(probe.writes.get(), 1);
    let root = probe.roots.borrow()[0];
    assert_eq!(*probe.borrows.borrow(), [root, root]);
    assert_ne!(root, raw);
    assert!(is_open(root));
    drop(directory);
    assert_closed(raw, &probe);
}

#[test]
fn scoped_policy_denied_or_unavailable_never_enters_backend_or_borrows_fd() {
    let sandbox = Sandbox::new();
    for reason in [StorageRootFailure::Denied, StorageRootFailure::Unavailable] {
        let mut fixture = Fixture::new();
        let fd = sandbox.fd();
        let raw = fd.as_raw_fd();
        let (input, probe) = input(fd, &fixture);
        let mut policy = Policy::allow();
        let mut directory = ScopedDirectory::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            input,
            &mut policy,
        )
        .unwrap();
        policy.result = Err(reason);
        assert_eq!(
            directory.read(ObjectId(1), &mut policy),
            Err(SupplyFailure::Profile(reason))
        );
        assert_eq!(policy.calls, 2);
        assert_eq!(
            directory.write(ObjectId(1), b"no", &mut policy),
            Err(SupplyFailure::Profile(reason))
        );
        assert_eq!(policy.calls, 3);
        assert_eq!(probe.reads.get(), 0);
        assert_eq!(probe.writes.get(), 0);
        assert!(probe.borrows.borrow().is_empty());
        drop(directory);
        assert_closed(raw, &probe);
    }
}

#[test]
fn scoped_caller_change_in_authorization_prevents_backend_entry() {
    let sandbox = Sandbox::new();
    for principal in [false, true] {
        let mut fixture = Fixture::new();
        let switch = fixture.caller.arm_change(principal);
        let fd = sandbox.fd();
        let raw = fd.as_raw_fd();
        let (input, probe) = input(fd, &fixture);
        let mut policy = Policy::allow();
        let mut directory = ScopedDirectory::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            input,
            &mut policy,
        )
        .unwrap();
        policy.hook = Some(Box::new(move || switch.store(true, Ordering::SeqCst)));
        assert_eq!(
            directory.read(ObjectId(2), &mut policy),
            Err(SupplyFailure::Session(AdmissionFailure::CallerMismatch))
        );
        assert_eq!(policy.calls, 2);
        assert_eq!(
            directory.write(ObjectId(2), b"no", &mut policy),
            Err(SupplyFailure::Session(AdmissionFailure::CallerMismatch))
        );
        assert_eq!(policy.calls, 2);
        assert_eq!(probe.reads.get(), 0);
        assert_eq!(probe.writes.get(), 0);
        assert!(probe.borrows.borrow().is_empty());
        drop(directory);
        assert_closed(raw, &probe);
    }
}

#[test]
fn scoped_cancel_in_authorization_prevents_backend_entry_and_retry() {
    let sandbox = Sandbox::new();
    let mut fixture = Fixture::new();
    let fd = sandbox.fd();
    let raw = fd.as_raw_fd();
    let (input, probe) = input(fd, &fixture);
    let token = CancellationToken::new();
    let from_policy = token.clone();
    let mut policy = Policy::allow();
    let mut directory = ScopedDirectory::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        token,
        input,
        &mut policy,
    )
    .unwrap();
    policy.hook = Some(Box::new(move || from_policy.cancel()));
    assert_eq!(
        directory.read(ObjectId(3), &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(
        directory.write(ObjectId(3), b"no", &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.reads.get(), 0);
    assert_eq!(probe.writes.get(), 0);
    assert!(probe.borrows.borrow().is_empty());
    directory.revoke();
    assert!(!is_open(probe.roots.borrow()[0]));
    assert!(is_open(raw));
    drop(directory);
    assert_closed(raw, &probe);
}

#[test]
fn scoped_issuer_revocation_in_authorization_prevents_backend_entry() {
    let sandbox = Sandbox::new();
    let mut fixture = Fixture::new();
    let fd = sandbox.fd();
    let raw = fd.as_raw_fd();
    let (input, probe) = input(fd, &fixture);
    let owner = Rc::new(RefCell::new(fixture.issuer.take()));
    let mut resolver = Resolver(owner.clone());
    let mut policy = Policy::allow();
    let mut directory = ScopedDirectory::supply(
        &mut resolver,
        &fixture.caller,
        CancellationToken::new(),
        input,
        &mut policy,
    )
    .unwrap();
    policy.hook = Some(Box::new(move || drop(owner.borrow_mut().take())));
    assert_eq!(
        directory.read(ObjectId(4), &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::StaleIdentity))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.reads.get(), 0);
    assert!(probe.borrows.borrow().is_empty());
    assert_eq!(
        directory.write(ObjectId(4), b"no", &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::StaleIdentity))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.writes.get(), 0);
    directory.revoke();
    assert!(!is_open(probe.roots.borrow()[0]));
    drop(directory);
    assert_closed(raw, &probe);
}

#[test]
fn scoped_backend_observes_late_cancel_before_any_fd_borrow() {
    let sandbox = Sandbox::new();
    let mut fixture = Fixture::new();
    let fd = sandbox.fd();
    let raw = fd.as_raw_fd();
    let (input, probe) = input(fd, &fixture);
    let token = CancellationToken::new();
    let from_backend = token.clone();
    let mut policy = Policy::allow();
    let mut directory = ScopedDirectory::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        token,
        input,
        &mut policy,
    )
    .unwrap();
    *probe.io_hook.borrow_mut() = Some(Box::new(move || from_backend.cancel()));
    // The SDK has no StorageRootFailure::Cancelled. Backend stop preserves its
    // existing Unavailable error, while subsequent admission is Cancelled.
    assert_eq!(
        directory.read(ObjectId(5), &mut policy),
        Err(SupplyFailure::Profile(StorageRootFailure::Unavailable))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.reads.get(), 1);
    assert!(probe.borrows.borrow().is_empty());
    assert_eq!(
        directory.write(ObjectId(5), b"no", &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    );
    assert_eq!(policy.calls, 2);
    assert_eq!(probe.writes.get(), 0);
    drop(directory);
    assert_closed(raw, &probe);
}

#[test]
fn scoped_local_revoke_releases_root_and_blocks_work_without_logging_out() {
    let sandbox = Sandbox::new();
    let mut fixture = Fixture::new();
    let fd = sandbox.fd();
    let raw = fd.as_raw_fd();
    let (input, probe) = input(fd, &fixture);
    let mut policy = Policy::allow();
    let mut directory = ScopedDirectory::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        CancellationToken::new(),
        input,
        &mut policy,
    )
    .unwrap();
    directory.revoke();
    directory.revoke();
    assert!(!is_open(probe.roots.borrow()[0]));
    assert!(is_open(raw));
    assert!(fixture.identity.is_live());
    assert_eq!(
        directory.read(ObjectId(6), &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    );
    assert_eq!(
        directory.write(ObjectId(6), b"no", &mut policy),
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    );
    assert_eq!(policy.calls, 1);
    assert_eq!(probe.reads.get(), 0);
    assert_eq!(probe.writes.get(), 0);
    assert!(probe.borrows.borrow().is_empty());
    drop(directory);
    assert_closed(raw, &probe);
}
