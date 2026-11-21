//! C2 registration and READY selection for C3-owned system services.
//!
//! C3's proof remains bound to its control endpoint. This module only binds
//! that proof to an already-installed, distinct C2 service listener and its
//! retained NIPC handshake connection. It never creates or attaches an
//! interactive `SessionLease`. Consumers must run `with_current` inside the
//! actual C1 after-login Session revalidation gate before publishing READY.

use super::{
    CompletedServiceHandshakeAuthority, Entry, LaunchRecordView, PeerDenial,
    PeerProcessInstanceAuthority, Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary,
    Pid1RunInstance, ProcessInstanceKey, RegistrationId, ServiceHandshakeProtocol,
    SupervisorLaunchAuthority, TrustedRouteKey, TrustedServiceEndpoint,
    TrustedServiceEndpointAuthority,
};
use nagi_system_principal_contract::{
    SystemServiceEndpointOwnerProof, SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION,
};
use std::fmt;

/// C2's selected registration for one verified system-service route. Its
/// principal is a system service and carries no interactive Session selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadySystemServiceInstance {
    registration: RegistrationId,
    process: ProcessInstanceKey,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    service_id: String,
    catalog_generation: std::num::NonZeroU64,
    c3_launch_generation: std::num::NonZeroU64,
    c3_control_endpoint_generation: std::num::NonZeroU64,
    c2_service_endpoint_generation: std::num::NonZeroU64,
}

impl ReadySystemServiceInstance {
    pub const fn registration(&self) -> RegistrationId {
        self.registration
    }

    pub const fn process(&self) -> ProcessInstanceKey {
        self.process
    }

    pub const fn route(&self) -> TrustedRouteKey {
        self.route
    }

    pub const fn endpoint(&self) -> TrustedServiceEndpoint {
        self.endpoint
    }

    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub const fn catalog_generation(&self) -> std::num::NonZeroU64 {
        self.catalog_generation
    }

    pub const fn c3_launch_generation(&self) -> std::num::NonZeroU64 {
        self.c3_launch_generation
    }

    pub const fn c3_control_endpoint_generation(&self) -> std::num::NonZeroU64 {
        self.c3_control_endpoint_generation
    }

    pub const fn c2_service_endpoint_generation(&self) -> std::num::NonZeroU64 {
        self.c2_service_endpoint_generation
    }
}

/// C2 readiness evidence for one system-service process. Callers must consume
/// this under C1's checked after-login Session gate; the event itself does not
/// turn the service principal into a Session or perform Session validation.
pub struct Pid1SystemServiceReadyEvent<'a> {
    run: Pid1RunInstance,
    service: ReadySystemServiceInstance,
    with_current: Box<
        dyn Fn(&mut dyn FnMut() -> Result<(), PeerDenial>) -> Result<(), PeerDenial>
            + Send
            + Sync
            + 'a,
    >,
}

impl fmt::Debug for Pid1SystemServiceReadyEvent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Pid1SystemServiceReadyEvent")
            .field("run", &self.run)
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl Pid1SystemServiceReadyEvent<'_> {
    pub fn run(&self) -> &Pid1RunInstance {
        &self.run
    }

    pub fn service(&self) -> &ReadySystemServiceInstance {
        &self.service
    }

    /// Diagnostic snapshot only. Publication must use `with_current` while
    /// the C1 after-login Session gate is also held by the caller.
    pub fn validate_current(&self) -> Result<(), PeerDenial> {
        self.with_current(|_| ())
    }

    /// Revalidate the same C3 proof, C2 run, route, listener generation,
    /// process, and retained accepted stream under C3 then C2 owner gates.
    pub fn with_current<T>(&self, map: impl FnOnce(&Self) -> T) -> Result<T, PeerDenial> {
        let mut map = Some(map);
        let mut output = None;
        let mut consume = || {
            let map = map.take().ok_or(PeerDenial::Unavailable)?;
            output = Some(map(self));
            Ok(())
        };
        (self.with_current)(&mut consume)?;
        output.ok_or(PeerDenial::Unavailable)
    }
}

impl<R: LaunchRecordView> Pid1LivePeerDirectory<R> {
    /// Bind an existing C2 pre-session listener registration to C3's exact
    /// authenticated system-service launch proof and the signed-plan route.
    /// The caller first installs the separate C2 listener through the normal
    /// spawn-owned endpoint path. The C3 control FD is never used as that
    /// listener or handed to the NIPC handshake owner.
    pub fn bind_system_service_registration<E, A, B, H>(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        proof: &SystemServiceEndpointOwnerProof,
        endpoints: &E,
        supervisor: &A,
        readiness_boundary: &B,
        plan: &H,
    ) -> Result<(), PeerDenial>
    where
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        H: ServiceHandshakeProtocol + ?Sized,
    {
        if proof.version() != SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION {
            return Err(PeerDenial::StaleRegistration);
        }
        let (service_id, catalog_generation) = plan
            .system_service_slot()
            .ok_or(PeerDenial::InvalidNipcRequest)?;
        let slot = proof.service_slot_identity();
        if slot.service_id() != service_id || slot.catalog_generation() != catalog_generation {
            return Err(PeerDenial::StaleRegistration);
        }
        if plan.route() != self.snapshot(registration)?.route {
            return Err(PeerDenial::RouteMismatch);
        }
        if !run
            .readiness_boundary()
            .is_same_domain(readiness_boundary.authority_domain())
        {
            return Err(PeerDenial::StaleRun);
        }

        proof
            .with_live_authority_effect(|| {
                readiness_boundary.with_owner_gate(run, None, || {
                    self.require_current_run(run)?;
                    let mut state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
                    let entry = state
                        .entries
                        .iter_mut()
                        .find(|entry| entry.id == registration)
                        .ok_or(PeerDenial::StaleRegistration)?;
                    let identity = proof.process_identity();
                    if !entry.is_live()
                        || entry.run_generation != run.generation()
                        || entry.route != plan.route()
                        || entry.session.is_some()
                        || entry.system_service_owner.is_some()
                        || entry
                            .initial_session_issuance_pending
                            .load(super::Ordering::SeqCst)
                        || entry.record.process_id() != identity.process_id()
                        || !entry.task24_identity.same_instance(&identity)
                        || !endpoints.is_current(entry.endpoint)?
                        || !endpoints.fd_delegation_is_prevented(entry.endpoint)?
                        || supervisor.resolve(entry.record.process_id()) != Some(entry.record)
                    {
                        return Err(PeerDenial::StaleRegistration);
                    }
                    entry.system_service_owner = Some(super::SystemServiceOwnerBinding {
                        run_generation: entry.run_generation,
                        process_identity: identity,
                        service_id: service_id.to_owned(),
                        catalog_generation,
                        c3_launch_generation: proof.launch_generation(),
                        c3_control_endpoint_generation: proof.endpoint_generation(),
                        c2_service_endpoint_generation: entry.endpoint.endpoint_generation,
                    });
                    Ok(())
                })
            })
            .map_err(|_| PeerDenial::StalePeer)?
    }

    /// Select the one live C2 registration described by the signed-plan
    /// handshake protocol and cross-check it against C3's non-cloneable
    /// endpoint-owner proof. The returned event retains the C3 proof and C2
    /// handshake owner references through final READY publication.
    #[allow(clippy::too_many_arguments)]
    pub fn system_service_ready_event<'a, E, P, C, A, B, H>(
        &'a self,
        run: &'a Pid1RunInstance,
        proof: &'a SystemServiceEndpointOwnerProof,
        endpoints: &'a E,
        process_instances: &'a P,
        handshakes: &'a C,
        supervisor: &'a A,
        readiness_boundary: &'a B,
        plan: &'a H,
    ) -> Result<Pid1SystemServiceReadyEvent<'a>, PeerDenial>
    where
        E: TrustedServiceEndpointAuthority + 'a,
        P: PeerProcessInstanceAuthority + 'a,
        C: CompletedServiceHandshakeAuthority + 'a,
        A: SupervisorLaunchAuthority<Record = R> + 'a,
        B: Pid1ReadinessAtomicBoundary + 'a,
        H: ServiceHandshakeProtocol + ?Sized + 'a,
    {
        if proof.version() != SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION {
            return Err(PeerDenial::StaleRegistration);
        }
        let (service_id, catalog_generation) = plan
            .system_service_slot()
            .ok_or(PeerDenial::InvalidNipcRequest)?;
        let selected = self.select_system_service_entry(
            run,
            proof,
            plan.route(),
            service_id,
            catalog_generation,
        )?;
        self.validate_system_service_entry(
            run,
            proof,
            &selected,
            endpoints,
            process_instances,
            handshakes,
            supervisor,
            false,
        )?;
        let service = ready_instance(&selected, proof);
        let service_for_validation = service.clone();
        let run_for_validation = run.clone();
        let with_current = move |consume: &mut dyn FnMut() -> Result<(), PeerDenial>| {
            proof
                .with_live_authority_effect(|| {
                    readiness_boundary.with_owner_gate(&run_for_validation, None, || {
                        self.require_current_run(&run_for_validation)?;
                        let current = self.select_system_service_entry(
                            &run_for_validation,
                            proof,
                            plan.route(),
                            service_id,
                            catalog_generation,
                        )?;
                        let current_service = ready_instance(&current, proof);
                        if current_service != service_for_validation {
                            return Err(PeerDenial::StaleRegistration);
                        }
                        self.validate_system_service_entry(
                            &run_for_validation,
                            proof,
                            &current,
                            endpoints,
                            process_instances,
                            handshakes,
                            supervisor,
                            true,
                        )?;
                        consume_under_supervisor_owner(supervisor, current.record, consume)
                    })
                })
                .map_err(|_| PeerDenial::StalePeer)?
        };
        let event = Pid1SystemServiceReadyEvent {
            run: run.clone(),
            service,
            with_current: Box::new(with_current),
        };
        event.validate_current()?;
        Ok(event)
    }

    fn select_system_service_entry(
        &self,
        run: &Pid1RunInstance,
        proof: &SystemServiceEndpointOwnerProof,
        route: TrustedRouteKey,
        service_id: &str,
        catalog_generation: std::num::NonZeroU64,
    ) -> Result<Entry<R>, PeerDenial> {
        self.require_current_run(run)?;
        let entries = self
            .state
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .entries
            .clone();
        let mut matches = entries.iter().filter(|entry| {
            entry.run_generation == run.generation()
                && entry.route == route
                && entry.is_live()
                && entry.session.is_none()
                && !entry
                    .initial_session_issuance_pending
                    .load(super::Ordering::SeqCst)
                && binding_matches(entry, proof, service_id, catalog_generation)
        });
        let entry = matches
            .next()
            .cloned()
            .ok_or(PeerDenial::RequiredServiceMissing)?;
        if matches.next().is_some() {
            return Err(PeerDenial::ReadyUnavailable);
        }
        Ok(entry)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_system_service_entry<E, P, C, A>(
        &self,
        run: &Pid1RunInstance,
        proof: &SystemServiceEndpointOwnerProof,
        entry: &Entry<R>,
        endpoints: &E,
        process_instances: &P,
        handshakes: &C,
        supervisor: &A,
        c3_gate_held: bool,
    ) -> Result<(), PeerDenial>
    where
        E: TrustedServiceEndpointAuthority,
        P: PeerProcessInstanceAuthority,
        C: CompletedServiceHandshakeAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        if !c3_gate_held && !proof.is_live().map_err(|_| PeerDenial::StalePeer)? {
            return Err(PeerDenial::StalePeer);
        }
        if !entry.is_live()
            || entry.session.is_some()
            || entry.run_generation != run.generation()
            || entry.record.process_id() != entry.process.pid()
            || !binding_matches_from_entry(entry, proof)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        if process_instances.has_unresolved_cleanup()? {
            return Err(PeerDenial::UnresolvedSpawnCleanup);
        }
        if !endpoints.is_current(entry.endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !endpoints.fd_delegation_is_prevented(entry.endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        if supervisor.resolve(entry.record.process_id()) != Some(entry.record) {
            return Err(PeerDenial::NotLive);
        }
        if !process_instances.confirms_spawned_instance_live(entry.process)? {
            return Err(PeerDenial::StalePeer);
        }
        if !handshakes.confirms_completed_handshake(
            run,
            entry.id,
            entry.process,
            entry.route,
            entry.endpoint,
        )? {
            return Err(PeerDenial::HandshakeIncomplete);
        }
        Ok(())
    }
}

fn binding_matches<R: LaunchRecordView>(
    entry: &Entry<R>,
    proof: &SystemServiceEndpointOwnerProof,
    service_id: &str,
    catalog_generation: std::num::NonZeroU64,
) -> bool {
    let slot = proof.service_slot_identity();
    slot.service_id() == service_id
        && slot.catalog_generation() == catalog_generation
        && binding_matches_from_entry(entry, proof)
}

fn binding_matches_from_entry<R: LaunchRecordView>(
    entry: &Entry<R>,
    proof: &SystemServiceEndpointOwnerProof,
) -> bool {
    let Some(binding) = entry.system_service_owner.as_ref() else {
        return false;
    };
    let identity = proof.process_identity();
    let slot = proof.service_slot_identity();
    binding.run_generation == entry.run_generation
        && binding
            .process_identity
            .same_instance(&entry.task24_identity)
        && binding.process_identity.same_instance(&identity)
        && binding.service_id == slot.service_id()
        && binding.catalog_generation == slot.catalog_generation()
        && binding.c3_launch_generation == proof.launch_generation()
        && binding.c3_control_endpoint_generation == proof.endpoint_generation()
        && binding.c2_service_endpoint_generation == entry.endpoint.endpoint_generation
}

fn ready_instance<R: LaunchRecordView>(
    entry: &Entry<R>,
    _proof: &SystemServiceEndpointOwnerProof,
) -> ReadySystemServiceInstance {
    let binding = entry
        .system_service_owner
        .as_ref()
        .expect("selected system-service entry has an owner binding");
    ReadySystemServiceInstance {
        registration: entry.id,
        process: entry.process,
        route: entry.route,
        endpoint: entry.endpoint,
        service_id: binding.service_id.clone(),
        catalog_generation: binding.catalog_generation,
        c3_launch_generation: binding.c3_launch_generation,
        c3_control_endpoint_generation: binding.c3_control_endpoint_generation,
        c2_service_endpoint_generation: entry.endpoint.endpoint_generation,
    }
}

/// Keep the exact single Supervisor launch record current for the whole READY
/// consumer. An empty grant list confirms launch liveness; C1's live Session
/// gate remains an independent requirement at the callsite.
fn consume_under_supervisor_owner<R, A>(
    supervisor: &A,
    record: R,
    consume: &mut dyn FnMut() -> Result<(), PeerDenial>,
) -> Result<(), PeerDenial>
where
    R: LaunchRecordView,
    A: SupervisorLaunchAuthority<Record = R>,
{
    supervisor.authorize_and_commit(record, &[], || consume())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, TryLockError};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestRecord(u32);

    impl LaunchRecordView for TestRecord {
        fn process_id(self) -> u32 {
            self.0
        }

        fn app_id(self) -> u64 {
            1
        }

        fn app_session_id(self) -> u64 {
            1
        }
    }

    struct TestSupervisor {
        current: Mutex<Option<TestRecord>>,
        in_commit: AtomicBool,
    }

    impl SupervisorLaunchAuthority for TestSupervisor {
        type Record = TestRecord;

        fn resolve(&self, process_id: u32) -> Option<TestRecord> {
            self.current
                .try_lock()
                .ok()
                .and_then(|current| *current)
                .filter(|record| record.process_id() == process_id)
        }

        fn authorize_and_commit<T>(
            &self,
            record: TestRecord,
            capabilities: &[&[u8]],
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !capabilities.is_empty() {
                return Err(PeerDenial::NotDeclared);
            }
            let current = self
                .current
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if *current != Some(record) {
                return Err(PeerDenial::NotLive);
            }
            self.in_commit.store(true, Ordering::Release);
            let result = commit();
            self.in_commit.store(false, Ordering::Release);
            result
        }
    }

    #[test]
    fn ready_consumer_runs_under_the_current_supervisor_record() {
        let record = TestRecord(7);
        let supervisor = TestSupervisor {
            current: Mutex::new(Some(record)),
            in_commit: AtomicBool::new(false),
        };
        let mut consumed = false;
        let mut consume = || {
            assert!(supervisor.in_commit.load(Ordering::Acquire));
            assert!(matches!(
                supervisor.current.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            consumed = true;
            Ok(())
        };

        consume_under_supervisor_owner(&supervisor, record, &mut consume).unwrap();

        assert!(consumed);
        assert!(!supervisor.in_commit.load(Ordering::Acquire));
    }
}
