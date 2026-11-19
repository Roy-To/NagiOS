//! Task-local borrow adapter candidate for a synchronous Stage13 consumer.
//!
//! The current task-41 adapter imports this module's lease and synchronous
//! consumer traits. Its `SerializedPackageServiceAuthority` is generic over
//! the task25 candidate owner and the distinct boot DevPreview session owner;
//! each keeps its own snapshot and lease type. The generation lease stays
//! alive across the borrowed `&PackageService` callback. This module neither
//! owns a service nor adds an authority, process boundary, stored-byte
//! attestation, or production route.
//!
//! The host callback case below exercises only this external consumer seam;
//! it does not invoke task-41's `AuthorizedPeer` effect wrapper or establish a
//! production service effect.
//!
//! An ended session owner cannot be used again:
//!
//! ```compile_fail
//! use nagi_dev_service_catalog_consumer::PackageServiceSessionOwner;
//! use nagi_package::PackageService;
//!
//! let session = PackageServiceSessionOwner::new(None, PackageService::new());
//! session.end_session();
//! let _ = session.capture(&[], b"expected");
//! ```
//!
//! The actual boot provider owner cannot be mutated during its lease callback:
//!
//! ```compile_fail
//! use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
//! use nagi_model::AppId;
//! use nagi_package::PackageService;
//!
//! let mut session = PackageServiceSessionOwner::new(None, PackageService::new());
//! let snapshot = session.capture(&[], b"expected").unwrap();
//! let lease = session.lease_for_dispatch(&snapshot).unwrap();
//! let _commit_window = lease.inventory().info(AppId(1));
//! session.remove(AppId(1));
//! drop(lease);
//! ```

use crate::PackageServiceGenerationLease as CandidateLease;
use nagi_init_package_provider::package_trust::PackageServiceGenerationLease as BootLease;
use nagi_package::PackageService;

/// Narrow synchronous shape used by the current task-41 adapter to borrow the
/// live PackageService during one operation.
pub trait Stage13BorrowedInventoryConsumer {
    type Error;

    fn consume(&mut self, inventory: &PackageService) -> Result<(), Self::Error>;
}

/// Lease shapes exposed by the dd1107 task candidate and the current boot
/// package-provider owner. Both yield a scoped read-only view of PackageService.
pub trait Stage13PackageServiceLease {
    fn inventory(&self) -> &PackageService;
}

impl Stage13PackageServiceLease for CandidateLease<'_> {
    fn inventory(&self) -> &PackageService {
        CandidateLease::inventory(self)
    }
}

impl Stage13PackageServiceLease for BootLease<'_> {
    fn inventory(&self) -> &PackageService {
        BootLease::inventory(self)
    }
}

/// Pass an already owner-validated lease to one synchronous consumer call.
/// The caller keeps the lease alive across this call and drops it afterward.
/// No `PackageService` reference is returned by this adapter.
pub fn with_stage13_borrowed_inventory<Lease, Consumer>(
    lease: &Lease,
    consumer: &mut Consumer,
) -> Result<(), Consumer::Error>
where
    Lease: Stage13PackageServiceLease,
    Consumer: Stage13BorrowedInventoryConsumer,
{
    consumer.consume(lease.inventory())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GenerationBoundCommitError, GenerationBoundOperationRequest, OwnedPackageInventorySnapshot,
        PackageOwnerError, PackageServiceSessionOwner, SnapshotError,
    };
    use nagi_init_package_provider::package_trust::{
        PackageServiceSessionOwner as BootOwner, PackageServiceSessionOwnerError as BootOwnerError,
    };
    use nagi_model::AppId;
    use nagi_package::{InstallPolicy, PackageService, TRUSTED_SIGNING_PUBLIC_KEY};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{env, fs};

    const EXPECTED_ID: &[u8] = b"org.nagi.acceptance.isolated-app";

    struct Stage13Probe<'owner> {
        owner: Option<&'owner PackageServiceSessionOwner<'static>>,
        package: &'owner [u8],
        receipt_snapshot: Option<OwnedPackageInventorySnapshot>,
        app_id: AppId,
        observed_generation: Option<u64>,
    }

    impl Stage13BorrowedInventoryConsumer for Stage13Probe<'_> {
        type Error = PackageOwnerError;

        fn consume(&mut self, inventory: &PackageService) -> Result<(), Self::Error> {
            let installed = inventory
                .info(self.app_id)
                .map_err(PackageOwnerError::Package)?;
            self.observed_generation = Some(installed.generation);
            if let Some(owner) = self.owner {
                let snapshot = self
                    .receipt_snapshot
                    .ok_or(PackageOwnerError::Unavailable)?;
                assert_eq!(
                    owner.install(
                        self.package,
                        InstallPolicy {
                            developer_mode: true,
                            require_signature: true,
                        },
                    ),
                    Err(PackageOwnerError::Busy),
                    "the borrowed dispatch lease must fence owner mutation"
                );
                assert_eq!(
                    owner.commit_record_if_current(GenerationBoundOperationRequest::new(
                        snapshot, 991,
                    )),
                    Err(GenerationBoundCommitError::Busy),
                    "the local receipt transaction must not reenter the live lease"
                );
            }
            Ok(())
        }
    }

    struct ExternalConsumerProbe<'callback> {
        app_id: AppId,
        expected_generation: u64,
        callback_calls: &'callback AtomicUsize,
    }

    impl Stage13BorrowedInventoryConsumer for ExternalConsumerProbe<'_> {
        type Error = std::convert::Infallible;

        fn consume(&mut self, inventory: &PackageService) -> Result<(), Self::Error> {
            let installed = inventory
                .info(self.app_id)
                .expect("the live external consumer sees its installed package");
            assert_eq!(installed.generation, self.expected_generation);
            self.callback_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn stage13_borrow_uses_the_session_owner_and_holds_busy_until_return() {
        let package_path = env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the task-local signed XAPP");
        let package = fs::read(package_path).expect("read the pinned signed XAPP");

        // The same PackageService is placed in the owner before its first
        // install; the Stage13-shaped consumer receives only its live borrow.
        let owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("perform the first install through the retained owner");
        let snapshot = owner
            .capture(&package, EXPECTED_ID)
            .expect("capture the installed generation from the same owner");
        let app_id = snapshot.app_id();
        let lease = owner
            .lease_for_dispatch(&snapshot)
            .expect("validate and borrow the candidate owner's exact service");
        let mut probe = Stage13Probe {
            owner: Some(&owner),
            package: &package,
            receipt_snapshot: Some(snapshot),
            app_id,
            observed_generation: None,
        };

        assert_eq!(with_stage13_borrowed_inventory(&lease, &mut probe), Ok(()),);
        assert_eq!(probe.observed_generation, Some(1));
        assert_eq!(
            owner.install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            ),
            Err(PackageOwnerError::Busy),
            "the owner remains leased until the caller drops the lease"
        );
        drop(lease);

        let request = GenerationBoundOperationRequest::new(snapshot, 991);
        owner
            .commit_record_if_current(request)
            .expect("a Busy refusal while leased must not consume the receipt ID");

        owner
            .install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("owner mutation resumes after the synchronous lease drops");
        assert_eq!(
            owner.lease_for_dispatch(&snapshot).err(),
            Some(PackageOwnerError::Snapshot(
                SnapshotError::StaleGeneration {
                    captured: 1,
                    current: 2,
                },
            ))
        );
    }

    #[test]
    fn stage13_external_consumer_runs_under_the_boot_provider_lease() {
        let package_path = env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the task-local signed XAPP");
        let package = fs::read(package_path).expect("read the pinned signed XAPP");
        let mut owner = BootOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), PackageService::new());
        owner
            .install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("install through the boot provider's retained owner");
        let snapshot = owner
            .capture(&package, EXPECTED_ID)
            .expect("capture from the boot provider owner");
        let lease = owner
            .lease_for_dispatch(&snapshot)
            .expect("obtain the boot provider's validated borrow");
        let callback_calls = AtomicUsize::new(0);
        let mut probe = ExternalConsumerProbe {
            app_id: snapshot.app_id(),
            expected_generation: snapshot.generation(),
            callback_calls: &callback_calls,
        };

        assert_eq!(with_stage13_borrowed_inventory(&lease, &mut probe), Ok(()));
        assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
        drop(lease);

        owner
            .remove(snapshot.app_id())
            .expect("uninstall through the same boot provider owner");
        assert_eq!(
            owner.lease_for_dispatch(&snapshot).err(),
            Some(BootOwnerError::NotInstalled),
            "an absent package must report NotInstalled, not a generic stale failure"
        );
        owner
            .install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: true,
                },
            )
            .expect("reinstall through the same boot provider owner");
        assert_eq!(
            owner.capture(&package, EXPECTED_ID).unwrap().generation(),
            2
        );
        assert_eq!(
            owner.lease_for_dispatch(&snapshot).err(),
            Some(BootOwnerError::StaleGeneration {
                captured: 1,
                current: 2,
            }),
            "after reinstall, only the exact stale generation error is accepted"
        );
    }

    #[test]
    fn stage13_boot_owner_rejects_same_generation_snapshot_from_other_inventory() {
        let package_path = env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must name the task-local signed XAPP");
        let package = fs::read(package_path).expect("read the pinned signed XAPP");
        let mut first_owner =
            BootOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), PackageService::new());
        let mut second_owner =
            BootOwner::new(Some(&TRUSTED_SIGNING_PUBLIC_KEY), PackageService::new());
        let policy = InstallPolicy {
            developer_mode: true,
            require_signature: true,
        };
        first_owner
            .install(&package, policy)
            .expect("install into the first independent owner");
        second_owner
            .install(&package, policy)
            .expect("install the same package into the second independent owner");

        let first_snapshot = first_owner
            .capture(&package, EXPECTED_ID)
            .expect("capture the first owner's generation");
        let second_snapshot = second_owner
            .capture(&package, EXPECTED_ID)
            .expect("capture the second owner's generation");
        assert_eq!(first_snapshot.generation(), 1);
        assert_eq!(second_snapshot.generation(), first_snapshot.generation());
        assert_eq!(second_snapshot.manifest_id(), first_snapshot.manifest_id());
        assert!(second_owner.lease_for_dispatch(&second_snapshot).is_ok());
        assert_eq!(
            second_owner.lease_for_dispatch(&first_snapshot).err(),
            Some(BootOwnerError::OwnerMismatch),
            "same manifest and generation from another inventory must report OwnerMismatch"
        );
    }
}
