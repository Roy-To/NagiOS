//! Connect the C3-owned standalone Identity launch to C2's exact-child lease.

use super::{
    BootLoginRun, LoginLease, PendingSpawnOwnedEndpoint, RouteConsentRequirement,
    ServiceHandshakeProtocol, TrustedInventory, TrustedManifest, TrustedServiceRequirements,
};
use crate::linux_init::{
    linux_init_c3_system_service_owner::{
        C3IdentityLaunchBinding, C3IdentityListenerLaunchLease, C3PendingIdentityListener,
    },
    pid1_live_peer_supplier::{ConsentSource, PeerDenial, SupervisorLaunchAuthority},
    FdDelegationRestrictionAttestation,
};
use crate::supervisor;
use libnagi::launch::{AppManifest, GrantCheck, LaunchPlacement, LaunchRecord};
use nagi_init_package_provider::package_trust::{
    OwnedPackageInventorySnapshot, PackageServiceSessionOwner,
};
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
use nagi_model::{AppSessionId, NodeId};
use nagi_system_principal_contract::SystemServiceLaunchRecord;
use nagi_system_service_launch::VerifiedServicePackageBinding;
use std::io;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, OnceLock};

pub(in crate::linux_init) fn pin_boot_package_key(key: &[u8; 32]) -> io::Result<&'static [u8; 32]> {
    static BOOT_PACKAGE_KEY: OnceLock<[u8; 32]> = OnceLock::new();
    let pinned = BOOT_PACKAGE_KEY.get_or_init(|| *key);
    (pinned == key)
        .then_some(pinned)
        .ok_or_else(|| denied("boot package signer changed"))
}

pub(in crate::linux_init) struct PendingIdentityListener {
    pending: Option<PendingSpawnOwnedEndpoint>,
    lease: Arc<Mutex<IdentityListenerLeaseState>>,
    run: Arc<BootLoginRun>,
    inventory: IdentityInventory,
    placement: LaunchPlacement,
}

#[derive(Default)]
struct IdentityListenerLeaseState {
    lease: Option<LoginLease>,
    ready_spawn: Option<IdentityReadySpawnBinding>,
}

struct IdentityReadySpawnBinding {
    proof: SpawnedProcessPidfd,
    record: LaunchRecord,
}

struct IdentityInventory {
    owner: Arc<PackageServiceSessionOwner<'static>>,
    snapshot: OwnedPackageInventorySnapshot,
    manifest: TrustedManifest,
}

impl TrustedInventory for IdentityInventory {
    fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
        if app_id != self.snapshot.app_id().0 {
            return Ok(None);
        }
        self.owner
            .lease_for_system_service_dispatch(&self.snapshot)
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        Ok(Some(self.manifest.clone()))
    }

    fn generation_is_current(
        &self,
        app_id: u64,
        generation: NonZeroU64,
    ) -> Result<bool, PeerDenial> {
        if app_id != self.snapshot.app_id().0 || generation.get() != self.snapshot.generation() {
            return Ok(false);
        }
        self.owner
            .lease_for_system_service_dispatch(&self.snapshot)
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        Ok(true)
    }

    fn with_current_manifests(
        &self,
        requested: &[(u64, NonZeroU64)],
        inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        let [(app_id, generation)] = requested else {
            return Err(PeerDenial::StaleManifest);
        };
        if *app_id != self.snapshot.app_id().0 || generation.get() != self.snapshot.generation() {
            return Err(PeerDenial::StaleManifest);
        }
        let _lease = self
            .owner
            .lease_for_system_service_dispatch(&self.snapshot)
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        inspect(std::slice::from_ref(&self.manifest))
    }
}

impl PendingIdentityListener {
    pub(in crate::linux_init) fn new(
        run: Arc<BootLoginRun>,
        owner: Arc<PackageServiceSessionOwner<'static>>,
        package: &VerifiedServicePackageBinding<'_>,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let plan = run
            .identity_ready
            .as_ref()
            .ok_or_else(|| denied("Identity plan unavailable"))?;
        let snapshot = owner
            .capture_for_system_service(bytes, package.package_id().as_bytes())
            .map_err(|error| {
                io::Error::other(format!("Identity inventory unavailable ({error:?})"))
            })?;
        if snapshot.app_id() != package.app_id()
            || snapshot.signed_package_sha256() != package.signed_package_sha256()
            || snapshot.generation() != package.package_generation()
            || plan.destination_app_id() != package.app_id()
        {
            return Err(denied(
                "Identity package differs from its admitted generation",
            ));
        }
        let view = nagi_package::PackageView::parse(bytes)
            .map_err(|_| denied("invalid installed Identity package"))?;
        let manifest = view.manifest();
        let supervisor_manifest = AppManifest::from_declaration(manifest.id(), manifest.grants())
            .map_err(|_| denied("Identity Supervisor manifest rejected"))?;
        let trusted_manifest = TrustedManifest::from_verified_installed_manifest(
            package.app_id().0,
            package.package_generation(),
            manifest.grants().map(|g| g.to_vec()),
        )
        .ok_or_else(|| denied("Identity C2 manifest rejected"))?;

        let placement = LaunchPlacement {
            app_session_id: AppSessionId(2),
            node_id: NodeId(1),
            workspace_id: None,
        };
        supervisor::register_or_match_manifest(supervisor_manifest)
            .map_err(|_| denied("Identity Supervisor manifest unavailable"))?;
        supervisor::check_launch(package.app_id(), placement)
            .map_err(|_| denied("Identity Supervisor launch rejected"))?;
        let pending = run.prepare_identity_listener().map_err(|error| {
            io::Error::other(format!("Identity listener unavailable ({error:?})"))
        })?;
        Ok(Self {
            pending: Some(pending),
            lease: Arc::new(Mutex::new(IdentityListenerLeaseState::default())),
            run,
            inventory: IdentityInventory {
                owner,
                snapshot,
                manifest: trusted_manifest,
            },
            placement,
        })
    }

    pub(in crate::linux_init) fn child_arguments(&self) -> io::Result<Vec<String>> {
        self.pending
            .as_ref()
            .ok_or_else(|| denied("Identity listener already transferred"))?
            .child_arguments(&[])
            .map_err(|_| denied("Identity listener locator rejected"))
    }

    /// Lend the exact C2 spawn binding retained by this launch. READY
    /// publication must pass both values back through the C2 owner gate.
    pub(in crate::linux_init) fn with_ready_owner<T>(
        &self,
        use_owner: impl FnOnce(&LoginLease, &SpawnedProcessPidfd, LaunchRecord) -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        let state = self.lease.try_lock().map_err(|_| PeerDenial::Unavailable)?;
        let lease = state
            .lease
            .as_ref()
            .ok_or(PeerDenial::RequiredServiceMissing)?;
        let spawn = state
            .ready_spawn
            .as_ref()
            .ok_or(PeerDenial::RequiredServiceMissing)?;
        use_owner(lease, &spawn.proof, spawn.record)
    }
}

impl C3PendingIdentityListener for PendingIdentityListener {
    fn bind_pending_before_exec(
        &mut self,
        child: SpawnedProcessPidfd,
        binding: &C3IdentityLaunchBinding,
    ) -> io::Result<Box<dyn C3IdentityListenerLaunchLease>> {
        let identity = child.identity();
        let plan = self
            .run
            .identity_ready
            .as_ref()
            .ok_or_else(|| denied("Identity plan unavailable"))?;
        if !identity.same_instance(binding.process_identity())
            || !c3_launch_record_matches_binding(binding, binding.launch_record())
            || plan.system_service_slot()
                != Some((binding.service_id(), binding.catalog_generation()))
        {
            return Err(denied("C3 Identity spawn does not match the signed plan"));
        }
        let mut lease_slot = self
            .lease
            .try_lock()
            .map_err(|_| denied("Identity lease busy"))?;
        if self.pending.is_none() || lease_slot.lease.is_some() || lease_slot.ready_spawn.is_some()
        {
            return Err(denied("Identity listener already transferred"));
        }
        // Keep an identity-only pidfd duplicate for the later C2 READY gate.
        // The C2 lease below retains the separate spawn proof it owns for
        // activation and retirement; this duplicate does not gain reap power.
        let ready_proof = child.duplicate_for_authority()?;
        let record = supervisor::try_with_registry(|registry| {
            registry.record_launch(
                identity.process_id(),
                self.inventory.snapshot.app_id(),
                self.placement,
            )
        })
        .ok_or_else(|| denied("Identity Supervisor busy"))?
        .map_err(|_| denied("Identity live record rejected"))?;
        let pending = self.pending.take().expect("pending listener checked above");
        lease_slot.lease = Some(LoginLease::retain_pending_before_exec_gate(
            pending,
            child,
            record,
            self.run.run_owner.run(),
            &self.run.instances,
        ));
        lease_slot.ready_spawn = Some(IdentityReadySpawnBinding {
            proof: ready_proof,
            record,
        });
        Ok(Box::new(IdentityLaunchLease {
            run: Arc::clone(&self.run),
            inventory: IdentityInventory {
                owner: Arc::clone(&self.inventory.owner),
                snapshot: self.inventory.snapshot,
                manifest: self.inventory.manifest.clone(),
            },
            lease: Arc::clone(&self.lease),
            identity,
            record,
            c3_launch_record: binding.launch_record().clone(),
            binding_digest: *binding.scope_binding_digest(),
            launch_generation: binding.c3_launch_generation(),
        }))
    }
}

struct IdentityLaunchLease {
    run: Arc<BootLoginRun>,
    inventory: IdentityInventory,
    lease: Arc<Mutex<IdentityListenerLeaseState>>,
    identity: SpawnedProcessIdentity,
    record: LaunchRecord,
    c3_launch_record: SystemServiceLaunchRecord,
    binding_digest: [u8; 32],
    launch_generation: NonZeroU64,
}

impl IdentityLaunchLease {
    fn same_binding(&self, binding: &C3IdentityLaunchBinding) -> bool {
        self.identity.same_instance(binding.process_identity())
            && self.binding_digest == *binding.scope_binding_digest()
            && self.launch_generation == binding.c3_launch_generation()
            && self
                .c3_launch_record
                .process_identity()
                .same_instance(&self.identity)
            && c3_launch_record_matches_binding(binding, &self.c3_launch_record)
    }
}

fn c3_launch_record_matches_binding(
    binding: &C3IdentityLaunchBinding,
    record: &SystemServiceLaunchRecord,
) -> bool {
    let Some(slot) = record.system_service_slot_identity() else {
        return false;
    };
    record.generation() == binding.c3_launch_generation()
        && record
            .process_identity()
            .same_instance(binding.process_identity())
        && slot.service_id() == binding.service_id()
        && slot.catalog_generation() == binding.catalog_generation()
}

impl C3IdentityListenerLaunchLease for IdentityLaunchLease {
    fn matches_binding(&self, binding: &C3IdentityLaunchBinding) -> bool {
        // C3 also checks this binding after exec and after reaping the child.
        self.same_binding(binding)
    }

    fn activate_after_exec_ack(
        &mut self,
        binding: &C3IdentityLaunchBinding,
        attestation: &FdDelegationRestrictionAttestation,
    ) -> io::Result<()> {
        if !self.same_binding(binding) {
            return Err(denied("foreign Identity activation binding"));
        }
        let run = &self.run;
        let plan = run
            .identity_ready
            .as_ref()
            .ok_or_else(|| denied("Identity plan unavailable"))?;
        let requirements = TrustedServiceRequirements::from_canonical_route(
            plan.route(),
            &[],
            RouteConsentRequirement::NotRequired,
        );
        self.lease
            .try_lock()
            .map_err(|_| denied("Identity lease busy"))?
            .lease
            .as_mut()
            .ok_or_else(|| denied("Identity lease unavailable"))?
            .activate_after_c3_identity_exec_ack(
                binding.process_identity(),
                self.record,
                attestation,
                run.run_owner.run(),
                &run.instances,
                &run.registry,
                &run.directory,
                &requirements,
                &self.inventory,
                &DisconnectedConsent,
                &IdentitySupervisor,
                run.run_owner.readiness_boundary(),
            )
            .map_err(peer_error)
    }

    fn retire_after_exact_reap(&mut self, binding: &C3IdentityLaunchBinding) -> io::Result<()> {
        if !self.same_binding(binding) {
            return Err(denied("foreign Identity retirement binding"));
        }
        let mut lease_slot = self
            .lease
            .try_lock()
            .map_err(|_| denied("Identity lease busy"))?;
        let lease = lease_slot
            .lease
            .as_mut()
            .ok_or_else(|| denied("Identity lease unavailable"))?;
        if !lease.matches_spawn(binding.process_identity(), &self.record) {
            return Err(denied("foreign Identity C2 retirement binding"));
        }
        let revoked = supervisor::try_with_registry(|registry| {
            supervisor::record_exit_if_matches_in_registry(
                registry,
                self.identity.process_id(),
                &self.record,
            )
        });
        if revoked != Some(true) {
            return Err(denied("Identity Supervisor record could not be revoked"));
        }
        lease
            .retire_after_exact_child_reap(
                binding.process_identity(),
                self.record,
                &self.run.directory,
                &self.run.instances,
                self.run.run_owner.readiness_boundary(),
            )
            .map_err(peer_error)?
            .then_some(())
            .ok_or_else(|| io::Error::other("Identity lease retirement outcome unknown"))?;
        *lease_slot = IdentityListenerLeaseState::default();
        Ok(())
    }
}

struct DisconnectedConsent;
impl ConsentSource for DisconnectedConsent {
    fn connected(&self) -> Result<bool, PeerDenial> {
        Ok(false)
    }
}

struct IdentitySupervisor;
impl SupervisorLaunchAuthority for IdentitySupervisor {
    type Record = LaunchRecord;

    fn resolve(&self, process_id: u32) -> Option<LaunchRecord> {
        supervisor::try_with_registry(|registry| registry.resolve(process_id)).flatten()
    }

    fn authorize_and_commit<T>(
        &self,
        record: LaunchRecord,
        grants: &[&[u8]],
        commit: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        use supervisor::LaunchCommitError;
        supervisor::try_with_registry(|registry| {
            supervisor::authorize_launch_set_in_registry(registry, &[(&record, grants)], commit)
        })
        .ok_or(PeerDenial::Unavailable)?
        .map_err(|error| match error {
            LaunchCommitError::Commit(error) => error,
            LaunchCommitError::RecordMismatch | LaunchCommitError::Grant(GrantCheck::NotLive) => {
                PeerDenial::NotLive
            }
            LaunchCommitError::Grant(GrantCheck::NotDeclared) => PeerDenial::NotDeclared,
            LaunchCommitError::Grant(GrantCheck::ConsentRequired) => PeerDenial::ConsentRequired,
            LaunchCommitError::Grant(GrantCheck::Denied) => PeerDenial::ConsentDenied,
            _ => PeerDenial::Unavailable,
        })
    }
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn peer_error(error: PeerDenial) -> io::Error {
    io::Error::other(format!(
        "Identity endpoint owner rejected the operation ({error:?})"
    ))
}
