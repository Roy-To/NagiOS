//! Private Linux candidate. Inputs are externally issued fds, never paths.
//! Possessing/constructing this backend is not an SDK authorization decision.

use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

#[cfg(test)]
use consumer::session_binding::{AdmissionFailure, SessionBinding};
use identity::{ProfileId, ProfileStorageBackend, StorageRootFailure, UserId};
use model::ObjectId;

#[cfg(test)]
use crate::{SessionProfile, SupplyFailure};

mod scoped;

struct DirectoryBackend {
    directory: File,
    user: Option<UserId>,
    profile: ProfileId,
    cancellation: Option<svc::CancellationToken>,
    #[cfg(test)]
    probe: Probe,
}

impl DirectoryBackend {
    // The external fd issuer supplies the binding. It is checked, not promoted
    // into authority: SDK profile authorization must still run on every open.
    fn from_external(directory: OwnedFd, user: Option<UserId>, profile: ProfileId) -> Self {
        Self {
            directory: File::from(directory),
            user,
            profile,
            cancellation: None,
            #[cfg(test)]
            probe: Probe::default(),
        }
    }

    // Called only by SDK backend operations while their private operation
    // lease is held. This fd is neither duplicated nor returned to a consumer.
    fn borrow_root<'root>(
        &self,
        root: &'root File,
    ) -> Result<BorrowedFd<'root>, StorageRootFailure> {
        let cancellation = self
            .cancellation
            .as_ref()
            .ok_or(StorageRootFailure::Unavailable)?;
        if cancellation.is_cancelled() {
            return Err(StorageRootFailure::Unavailable);
        }
        if !root
            .metadata()
            .map_err(|_| StorageRootFailure::Unavailable)?
            .is_dir()
        {
            return Err(StorageRootFailure::Unavailable);
        }
        #[cfg(test)]
        {
            use std::os::fd::AsRawFd;
            self.probe.borrows.borrow_mut().push(root.as_raw_fd());
        }
        Ok(root.as_fd())
    }
}

impl ProfileStorageBackend for DirectoryBackend {
    type Root = File;

    fn open_profile_root(
        &mut self,
        user: Option<UserId>,
        profile: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        #[cfg(test)]
        self.probe.calls.set(self.probe.calls.get() + 1);
        if user != self.user || profile != &self.profile {
            return Err(StorageRootFailure::Denied);
        }
        let metadata = self
            .directory
            .metadata()
            .map_err(|_| StorageRootFailure::Unavailable)?;
        if !metadata.is_dir() {
            return Err(StorageRootFailure::Unavailable);
        }
        let root = self
            .directory
            .try_clone()
            .map_err(|_| StorageRootFailure::Unavailable)?;
        #[cfg(test)]
        {
            use std::os::fd::AsRawFd;
            self.probe.roots.borrow_mut().push(root.as_raw_fd());
            let hook = self.probe.hook.borrow_mut().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        Ok(root)
    }

    // ObjectId-to-entry mapping is not specified by this candidate. In
    // particular, no filename, reserved ObjectId or fake Notes API is invented.
    fn read_object(
        &mut self,
        root: &Self::Root,
        _: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        #[cfg(test)]
        {
            self.probe.reads.set(self.probe.reads.get() + 1);
            let hook = self.probe.io_hook.borrow_mut().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let _root_fd = self.borrow_root(root)?;
        Err(StorageRootFailure::Unavailable)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        _: ObjectId,
        _: &[u8],
    ) -> Result<(), StorageRootFailure> {
        #[cfg(test)]
        {
            self.probe.writes.set(self.probe.writes.get() + 1);
            let hook = self.probe.io_hook.borrow_mut().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let _root_fd = self.borrow_root(root)?;
        Err(StorageRootFailure::Unavailable)
    }
}

#[cfg(test)]
struct DirectorySupply<'caller> {
    session: SessionBinding<'caller>,
    directory: File,
}

#[cfg(test)]
impl<'caller> DirectorySupply<'caller> {
    // Lifetime/admission probe only. After consuming the storage adapter this
    // does not run ProfileRootAuthorizer again and is not a Notes I/O grant.
    // Product fd delivery and consent reauthorization require owner decisions.
    fn borrow_directory(&self) -> Result<BorrowedFd<'_>, AdmissionFailure> {
        self.session.check()?;
        Ok(self.directory.as_fd())
    }

    // Prototype ownership transfer, not an adopted public API. The receiver
    // must retain the binding and manage fd closure/cancellation itself. A
    // transferred fd cannot be remotely closed by revoking an SDK observation.
    fn into_owned(self) -> Result<(SessionBinding<'caller>, OwnedFd), AdmissionFailure> {
        self.session.check()?;
        Ok((self.session, self.directory.into()))
    }
}

#[cfg(test)]
fn take_directory(
    supplied: SessionProfile<'_, DirectoryBackend>,
) -> Result<DirectorySupply<'_>, SupplyFailure> {
    let SessionProfile {
        session,
        root: _,
        roots,
    } = supplied;
    session.check().map_err(SupplyFailure::Session)?;
    // Real public SDK route: consumes adapter, drops its bound Root files and
    // returns B. StorageRootHandle is never read, cast or treated as an fd.
    let backend = roots.into_backend();
    Ok(DirectorySupply {
        session,
        directory: backend.directory,
    })
}

#[cfg(test)]
type HookSlot = std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce()>>>>;

#[cfg(test)]
#[derive(Clone, Default)]
struct Probe {
    calls: std::rc::Rc<std::cell::Cell<usize>>,
    roots: std::rc::Rc<std::cell::RefCell<Vec<std::os::fd::RawFd>>>,
    hook: HookSlot,
    io_hook: HookSlot,
    reads: std::rc::Rc<std::cell::Cell<usize>>,
    writes: std::rc::Rc<std::cell::Cell<usize>>,
    borrows: std::rc::Rc<std::cell::RefCell<Vec<std::os::fd::RawFd>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_fixture::{Fixture, Issuer};
    use crate::supply_current;
    use identity::{
        CurrentIdentityResolver, ProfileRootAuthorizer, ResolutionError, ResolvedIdentity,
        TrustedCallerContext, UserStorageRootAdapter,
    };
    use std::cell::RefCell;
    use std::os::fd::{AsRawFd, RawFd};
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use svc::CancellationToken;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!(
                    "fd-fixture-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::SeqCst)
                ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn directory(&self) -> OwnedFd {
            File::open(&self.0).unwrap().into()
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn is_open(fd: RawFd) -> bool {
        match std::fs::metadata(format!("/proc/self/fd/{fd}")) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("self fd inspection failed: {e}"),
        }
    }
    fn assert_closed(input: RawFd, probe: &Probe) {
        assert!(!is_open(input));
        for &fd in probe.roots.borrow().iter() {
            assert!(!is_open(fd));
        }
    }
    fn backend(
        fd: OwnedFd,
        fixture: &Fixture,
    ) -> (UserStorageRootAdapter<DirectoryBackend>, Probe) {
        let backend = DirectoryBackend::from_external(
            fd,
            fixture.identity.user_id(),
            fixture.identity.profile_id().clone(),
        );
        let probe = backend.probe.clone();
        (UserStorageRootAdapter::new(backend), probe)
    }
    struct Decision {
        result: Result<(), StorageRootFailure>,
        calls: usize,
        hook: Option<Box<dyn FnOnce()>>,
    }
    impl Decision {
        fn allow() -> Self {
            Self {
                result: Ok(()),
                calls: 0,
                hook: None,
            }
        }
    }
    impl ProfileRootAuthorizer for Decision {
        fn authorize_profile_root(
            &mut self,
            _: &dyn TrustedCallerContext,
            identity: &ResolvedIdentity,
            profile: &ProfileId,
        ) -> Result<(), StorageRootFailure> {
            self.calls += 1;
            assert_eq!(profile, identity.profile_id());
            if let Some(hook) = self.hook.take() {
                hook();
            }
            self.result
        }
    }
    struct Resolver(Rc<RefCell<Option<Issuer>>>);
    impl CurrentIdentityResolver for Resolver {
        fn resolve_current_identity(
            &mut self,
            caller: &impl TrustedCallerContext,
        ) -> Result<ResolvedIdentity, ResolutionError> {
            self.0
                .borrow_mut()
                .as_mut()
                .unwrap()
                .resolve_current_identity(caller)
        }
    }

    #[test]
    fn authorized_supply_consumes_adapter_and_transfers_real_directory_ownership() {
        let sandbox = Sandbox::new();
        let mut fixture = Fixture::new();
        let fd = sandbox.directory();
        let raw = fd.as_raw_fd();
        let (roots, probe) = backend(fd, &fixture);
        let supplied = supply_current(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            roots,
            &mut Decision::allow(),
        )
        .unwrap();
        assert_eq!(probe.calls.get(), 1);
        assert_eq!(probe.roots.borrow().len(), 1);
        assert!(is_open(probe.roots.borrow()[0]));
        let directory = take_directory(supplied).unwrap();
        assert_eq!(directory.borrow_directory().unwrap().as_raw_fd(), raw);
        assert!(!is_open(probe.roots.borrow()[0]));
        let (session, owned) = directory.into_owned().unwrap();
        let file = File::from(owned);
        assert!(file.metadata().unwrap().is_dir());
        assert!(session.check().is_ok());
        drop(file);
        assert_closed(raw, &probe);
    }

    #[test]
    fn dropping_successful_supply_closes_input_and_sdk_bound_root() {
        let sandbox = Sandbox::new();
        let mut fixture = Fixture::new();
        let fd = sandbox.directory();
        let raw = fd.as_raw_fd();
        let (roots, probe) = backend(fd, &fixture);
        let supplied = supply_current(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            roots,
            &mut Decision::allow(),
        )
        .unwrap();
        drop(supplied);
        assert_closed(raw, &probe);
    }

    #[test]
    fn denied_unavailable_or_cancelled_supply_closes_fd_without_backend_open() {
        let sandbox = Sandbox::new();
        for mode in 0..3 {
            let mut fixture = Fixture::new();
            let fd = sandbox.directory();
            let raw = fd.as_raw_fd();
            let (roots, probe) = backend(fd, &fixture);
            let token = CancellationToken::new();
            let mut policy = Decision::allow();
            match mode {
                0 => policy.result = Err(StorageRootFailure::Denied),
                1 => policy.result = Err(StorageRootFailure::Unavailable),
                _ => token.cancel(),
            }
            let result = supply_current(
                fixture.issuer.as_mut().unwrap(),
                &fixture.caller,
                token,
                roots,
                &mut policy,
            );
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("unexpected supply"),
            };
            let expected = match mode {
                0 => SupplyFailure::Profile(StorageRootFailure::Denied),
                1 => SupplyFailure::Profile(StorageRootFailure::Unavailable),
                _ => SupplyFailure::Session(AdmissionFailure::Cancelled),
            };
            assert_eq!(error, expected);
            assert_eq!(policy.calls, usize::from(mode != 2));
            assert_eq!(probe.calls.get(), 0);
            assert!(probe.roots.borrow().is_empty());
            assert_closed(raw, &probe);
        }
    }

    #[test]
    fn backend_rejects_regular_file_and_wrong_user_or_profile_binding() {
        let sandbox = Sandbox::new();
        for mode in 0..3 {
            let mut fixture = Fixture::new();
            let fd: OwnedFd = if mode == 0 {
                File::create(sandbox.0.join("regular-file")).unwrap().into()
            } else {
                sandbox.directory()
            };
            let raw = fd.as_raw_fd();
            let mut backend = DirectoryBackend::from_external(
                fd,
                fixture.identity.user_id(),
                fixture.identity.profile_id().clone(),
            );
            if mode == 1 {
                backend.user = Some(UserId::new(99).unwrap());
            }
            if mode == 2 {
                backend.profile = ProfileId::new("profile_other").unwrap();
            }
            let probe = backend.probe.clone();
            let result = supply_current(
                fixture.issuer.as_mut().unwrap(),
                &fixture.caller,
                CancellationToken::new(),
                UserStorageRootAdapter::new(backend),
                &mut Decision::allow(),
            );
            let expected = if mode == 0 {
                StorageRootFailure::Unavailable
            } else {
                StorageRootFailure::Denied
            };
            assert!(matches!(result, Err(SupplyFailure::Profile(error)) if error==expected));
            assert_eq!(probe.calls.get(), 1);
            assert!(probe.roots.borrow().is_empty());
            assert_closed(raw, &probe);
        }
    }

    #[test]
    fn policy_revocation_prevents_fd_clone_and_closes_external_input() {
        let sandbox = Sandbox::new();
        let mut fixture = Fixture::new();
        let fd = sandbox.directory();
        let raw = fd.as_raw_fd();
        let (roots, probe) = backend(fd, &fixture);
        let owner = Rc::new(RefCell::new(fixture.issuer.take()));
        let from_policy = owner.clone();
        let mut resolver = Resolver(owner);
        let mut policy = Decision::allow();
        policy.hook = Some(Box::new(move || drop(from_policy.borrow_mut().take())));
        let result = supply_current(
            &mut resolver,
            &fixture.caller,
            CancellationToken::new(),
            roots,
            &mut policy,
        );
        assert!(matches!(
            result,
            Err(SupplyFailure::Profile(StorageRootFailure::StaleSession))
        ));
        assert_eq!(probe.calls.get(), 0);
        assert!(probe.roots.borrow().is_empty());
        assert_closed(raw, &probe);
    }

    #[test]
    fn revocation_caller_change_or_cancel_in_backend_does_not_leak_fds() {
        let sandbox = Sandbox::new();
        for mode in 0..4 {
            let mut fixture = Fixture::new();
            let switch = fixture.caller.arm_change(mode == 1);
            let fd = sandbox.directory();
            let raw = fd.as_raw_fd();
            let (roots, probe) = backend(fd, &fixture);
            let owner = Rc::new(RefCell::new(fixture.issuer.take()));
            let from_backend = owner.clone();
            let mut resolver = Resolver(owner);
            let token = CancellationToken::new();
            let to_cancel = token.clone();
            *probe.hook.borrow_mut() = Some(Box::new(move || match mode {
                0 => drop(from_backend.borrow_mut().take()),
                1 | 2 => switch.store(true, Ordering::SeqCst),
                _ => to_cancel.cancel(),
            }));
            let result = supply_current(
                &mut resolver,
                &fixture.caller,
                token,
                roots,
                &mut Decision::allow(),
            );
            let expected = match mode {
                0 => AdmissionFailure::StaleIdentity,
                1 | 2 => AdmissionFailure::CallerMismatch,
                _ => AdmissionFailure::Cancelled,
            };
            assert!(matches!(result,Err(SupplyFailure::Session(error)) if error==expected));
            assert_eq!(probe.calls.get(), 1);
            assert_eq!(probe.roots.borrow().len(), 1);
            assert_closed(raw, &probe);
        }
    }

    #[test]
    fn revocation_or_cancel_before_or_after_take_refuses_handoff_and_closes_fd() {
        let sandbox = Sandbox::new();
        for after_take in [false, true] {
            for cancelled in [false, true] {
                let mut fixture = Fixture::new();
                let fd = sandbox.directory();
                let raw = fd.as_raw_fd();
                let (roots, probe) = backend(fd, &fixture);
                let token = CancellationToken::new();
                let supplied = supply_current(
                    fixture.issuer.as_mut().unwrap(),
                    &fixture.caller,
                    token.clone(),
                    roots,
                    &mut Decision::allow(),
                )
                .unwrap();
                if after_take {
                    let directory = take_directory(supplied).unwrap();
                    if cancelled {
                        token.cancel()
                    } else {
                        drop(fixture.issuer.take())
                    }
                    let expected = if cancelled {
                        AdmissionFailure::Cancelled
                    } else {
                        AdmissionFailure::StaleIdentity
                    };
                    assert!(matches!(directory.borrow_directory(),Err(error) if error==expected));
                    assert!(matches!(directory.into_owned(),Err(error) if error==expected));
                } else {
                    if cancelled {
                        token.cancel()
                    } else {
                        drop(fixture.issuer.take())
                    }
                    let expected = if cancelled {
                        AdmissionFailure::Cancelled
                    } else {
                        AdmissionFailure::StaleIdentity
                    };
                    assert!(
                        matches!(take_directory(supplied),Err(SupplyFailure::Session(error)) if error==expected)
                    );
                }
                assert_closed(raw, &probe);
            }
        }
    }
}
