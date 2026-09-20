pub const MAX_SERVICES: usize = 8;
pub const MAX_DEPENDENCIES: usize = 4;
pub const MAX_SERVICE_NAME: usize = 32;
pub const MAX_SERVICE_MESSAGE: usize = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ServiceId {
    name: [u8; MAX_SERVICE_NAME],
    name_len: u8,
    version: u16,
}

impl ServiceId {
    pub const fn empty() -> Self {
        Self {
            name: [0; MAX_SERVICE_NAME],
            name_len: 0,
            version: 0,
        }
    }

    pub fn new(name: &[u8], version: u16) -> Option<Self> {
        if name.is_empty() || name.len() > MAX_SERVICE_NAME || version == 0 {
            return None;
        }
        let mut id = Self {
            name: [0; MAX_SERVICE_NAME],
            name_len: name.len() as u8,
            version,
        };
        let mut index = 0;
        while index < name.len() {
            unsafe {
                core::ptr::write_volatile(
                    id.name.as_mut_ptr().add(index),
                    core::ptr::read_volatile(name.as_ptr().add(index)),
                );
            }
            index += 1;
        }
        Some(id)
    }

    pub fn name(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.name.as_ptr(), self.name_len as usize) }
    }

    pub const fn version(&self) -> u16 {
        self.version
    }
}

impl PartialEq for ServiceId {
    fn eq(&self, other: &Self) -> bool {
        if self.name_len != other.name_len || self.version != other.version {
            return false;
        }
        let mut index = 0;
        while index < self.name_len as usize {
            let left = unsafe { core::ptr::read_volatile(self.name.as_ptr().add(index)) };
            let right = unsafe { core::ptr::read_volatile(other.name.as_ptr().add(index)) };
            if left != right {
                return false;
            }
            index += 1;
        }
        true
    }
}

impl Eq for ServiceId {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    Never,
    OnFailure { max_restarts: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceManifest {
    id: ServiceId,
    dependencies: [ServiceId; MAX_DEPENDENCIES],
    dependency_count: u8,
    restart_policy: RestartPolicy,
}

impl ServiceManifest {
    pub fn new(
        id: ServiceId,
        dependencies: &[ServiceId],
        restart_policy: RestartPolicy,
    ) -> Option<Self> {
        if dependencies.len() > MAX_DEPENDENCIES {
            return None;
        }
        let mut manifest = Self {
            id,
            dependencies: [ServiceId::empty(); MAX_DEPENDENCIES],
            dependency_count: dependencies.len() as u8,
            restart_policy,
        };
        let mut index = 0;
        while index < dependencies.len() {
            let destination = unsafe { manifest.dependencies.as_mut_ptr().add(index) };
            let source = unsafe { dependencies.as_ptr().add(index) };
            unsafe {
                core::ptr::write_volatile(&mut (*destination).name_len, (*source).name_len);
                core::ptr::write_volatile(&mut (*destination).version, (*source).version);
                let mut name_index = 0;
                while name_index < MAX_SERVICE_NAME {
                    core::ptr::write_volatile(
                        (*destination).name.as_mut_ptr().add(name_index),
                        core::ptr::read_volatile((*source).name.as_ptr().add(name_index)),
                    );
                    name_index += 1;
                }
            }
            index += 1;
        }
        Some(manifest)
    }

    pub const fn id(&self) -> ServiceId {
        self.id
    }

    pub fn dependencies(&self) -> &[ServiceId] {
        unsafe {
            core::slice::from_raw_parts(self.dependencies.as_ptr(), self.dependency_count as usize)
        }
    }

    pub const fn restart_policy(&self) -> RestartPolicy {
        self.restart_policy
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceHealth {
    Starting,
    Ready,
    Healthy,
    Degraded,
    Failed,
    Restarting,
    CrashLoop,
}

impl ServiceHealth {
    fn allows_call(self) -> bool {
        matches!(self, Self::Ready | Self::Healthy | Self::Degraded)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceHandle {
    slot: u8,
    generation: u16,
}

pub type ServiceHandler = fn(&[u8], &mut [u8]) -> Result<usize, ServiceCallError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceCallError {
    Rejected,
    ResponseTooSmall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    InvalidHandle,
    StaleHandle,
    Duplicate,
    Capacity,
    RequestTooLarge,
    Unavailable,
    InvalidTransition,
    Handler(ServiceCallError),
}

#[derive(Clone, Copy)]
struct ServiceEntry {
    manifest: ServiceManifest,
    handler: ServiceHandler,
    health: ServiceHealth,
}

#[derive(Clone, Copy)]
struct RegistrySlot {
    generation: u16,
    entry: Option<ServiceEntry>,
}

impl RegistrySlot {
    const fn empty() -> Self {
        Self {
            generation: 1,
            entry: None,
        }
    }
}

pub struct ServiceRegistry {
    slots: [RegistrySlot; MAX_SERVICES],
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    pub const fn new() -> Self {
        Self {
            slots: [RegistrySlot::empty(); MAX_SERVICES],
        }
    }

    pub fn register(
        &mut self,
        manifest: ServiceManifest,
        handler: ServiceHandler,
    ) -> Result<ServiceHandle, RegistryError> {
        if self
            .slots
            .iter()
            .filter_map(|slot| slot.entry.as_ref())
            .any(|entry| entry.manifest.id() == manifest.id())
        {
            return Err(RegistryError::Duplicate);
        }
        let Some((slot, target)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.entry.is_none())
        else {
            return Err(RegistryError::Capacity);
        };
        target.entry = Some(ServiceEntry {
            manifest,
            handler,
            health: ServiceHealth::Starting,
        });
        Ok(ServiceHandle {
            slot: slot as u8,
            generation: target.generation,
        })
    }

    pub fn resolve(&self, id: ServiceId) -> Result<ServiceHandle, RegistryError> {
        self.slots
            .iter()
            .enumerate()
            .find_map(|(slot, candidate)| {
                candidate.entry.as_ref().and_then(|entry| {
                    (entry.manifest.id() == id).then_some(ServiceHandle {
                        slot: slot as u8,
                        generation: candidate.generation,
                    })
                })
            })
            .ok_or(RegistryError::InvalidHandle)
    }

    pub fn mark_ready(&mut self, handle: ServiceHandle) -> Result<(), RegistryError> {
        let entry = self.entry_mut(handle)?;
        if entry.health != ServiceHealth::Starting && entry.health != ServiceHealth::Restarting {
            return Err(RegistryError::InvalidTransition);
        }
        entry.health = ServiceHealth::Ready;
        Ok(())
    }

    pub fn mark_healthy(&mut self, handle: ServiceHandle) -> Result<(), RegistryError> {
        let entry = self.entry_mut(handle)?;
        if entry.health != ServiceHealth::Ready && entry.health != ServiceHealth::Degraded {
            return Err(RegistryError::InvalidTransition);
        }
        entry.health = ServiceHealth::Healthy;
        Ok(())
    }

    pub fn health(&self, handle: ServiceHandle) -> Result<ServiceHealth, RegistryError> {
        Ok(self.entry(handle)?.health)
    }

    pub fn call(
        &self,
        handle: ServiceHandle,
        request: &[u8],
        response: &mut [u8],
    ) -> Result<usize, RegistryError> {
        if request.len() > MAX_SERVICE_MESSAGE {
            return Err(RegistryError::RequestTooLarge);
        }
        let entry = self.entry(handle)?;
        if !entry.health.allows_call() {
            return Err(RegistryError::Unavailable);
        }
        let size = (entry.handler)(request, response).map_err(RegistryError::Handler)?;
        if size > response.len() {
            return Err(RegistryError::Handler(ServiceCallError::ResponseTooSmall));
        }
        Ok(size)
    }

    pub fn unregister(&mut self, handle: ServiceHandle) -> Result<(), RegistryError> {
        let slot = self.slot_mut(handle)?;
        slot.entry = None;
        slot.generation = next_generation(slot.generation);
        Ok(())
    }

    fn entry(&self, handle: ServiceHandle) -> Result<&ServiceEntry, RegistryError> {
        let slot = self.slot(handle)?;
        slot.entry.as_ref().ok_or(RegistryError::StaleHandle)
    }

    fn entry_mut(&mut self, handle: ServiceHandle) -> Result<&mut ServiceEntry, RegistryError> {
        let slot = self.slot_mut(handle)?;
        slot.entry.as_mut().ok_or(RegistryError::StaleHandle)
    }

    fn slot(&self, handle: ServiceHandle) -> Result<&RegistrySlot, RegistryError> {
        let slot = self
            .slots
            .get(handle.slot as usize)
            .ok_or(RegistryError::InvalidHandle)?;
        if slot.generation != handle.generation {
            return Err(RegistryError::StaleHandle);
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, handle: ServiceHandle) -> Result<&mut RegistrySlot, RegistryError> {
        let slot = self
            .slots
            .get_mut(handle.slot as usize)
            .ok_or(RegistryError::InvalidHandle)?;
        if slot.generation != handle.generation {
            return Err(RegistryError::StaleHandle);
        }
        Ok(slot)
    }
}

const fn next_generation(generation: u16) -> u16 {
    let next = generation.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    Capacity,
    Duplicate,
    MissingDependency,
    DependencyCycle,
    UnknownService,
    InvalidTransition,
}

#[derive(Clone, Copy)]
struct SupervisorEntry {
    manifest: ServiceManifest,
    health: ServiceHealth,
    restart_count: u8,
}

pub struct Supervisor {
    entries: [Option<SupervisorEntry>; MAX_SERVICES],
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_SERVICES],
        }
    }

    pub fn start_in_dependency_order(
        &mut self,
        manifests: &[ServiceManifest],
        order: &mut [ServiceId],
    ) -> Result<usize, SupervisorError> {
        if manifests.is_empty() || manifests.len() > MAX_SERVICES || order.len() < manifests.len() {
            return Err(SupervisorError::Capacity);
        }
        for (index, manifest) in manifests.iter().enumerate() {
            if manifests
                .iter()
                .take(index)
                .any(|previous| previous.id() == manifest.id())
                || manifest.dependencies().iter().any(|dependency| {
                    !manifests
                        .iter()
                        .any(|candidate| candidate.id() == *dependency)
                })
            {
                return Err(
                    if manifests
                        .iter()
                        .take(index)
                        .any(|previous| previous.id() == manifest.id())
                    {
                        SupervisorError::Duplicate
                    } else {
                        SupervisorError::MissingDependency
                    },
                );
            }
        }

        let mut placed = [false; MAX_SERVICES];
        for output in 0..manifests.len() {
            let mut selected = None;
            for (candidate, manifest) in manifests.iter().enumerate() {
                let already_placed = unsafe { *placed.as_ptr().add(candidate) };
                if already_placed
                    || manifest.dependencies().iter().any(|dependency| {
                        manifests
                            .iter()
                            .position(|other| other.id() == *dependency)
                            .is_none_or(|index| unsafe { !*placed.as_ptr().add(index) })
                    })
                {
                    continue;
                }
                selected = Some(candidate);
                break;
            }
            let Some(candidate) = selected else {
                return Err(SupervisorError::DependencyCycle);
            };
            unsafe {
                *placed.as_mut_ptr().add(candidate) = true;
                *order.as_mut_ptr().add(output) = manifests.get_unchecked(candidate).id();
            }
        }

        self.entries.fill(None);
        for manifest in manifests {
            let slot = self
                .entries
                .iter_mut()
                .find(|entry| entry.is_none())
                .ok_or(SupervisorError::Capacity)?;
            *slot = Some(SupervisorEntry {
                manifest: *manifest,
                health: ServiceHealth::Starting,
                restart_count: 0,
            });
        }
        Ok(manifests.len())
    }

    pub fn mark_ready(&mut self, id: ServiceId) -> Result<(), SupervisorError> {
        let entry = self.entry_mut(id)?;
        if entry.health != ServiceHealth::Starting && entry.health != ServiceHealth::Restarting {
            return Err(SupervisorError::InvalidTransition);
        }
        entry.health = ServiceHealth::Ready;
        Ok(())
    }

    pub fn mark_healthy(&mut self, id: ServiceId) -> Result<(), SupervisorError> {
        let entry = self.entry_mut(id)?;
        if entry.health != ServiceHealth::Ready && entry.health != ServiceHealth::Degraded {
            return Err(SupervisorError::InvalidTransition);
        }
        entry.health = ServiceHealth::Healthy;
        Ok(())
    }

    pub fn record_failure(&mut self, id: ServiceId) -> Result<ServiceHealth, SupervisorError> {
        let entry = self.entry_mut(id)?;
        entry.health = ServiceHealth::Failed;
        let health = match entry.manifest.restart_policy() {
            RestartPolicy::Never => ServiceHealth::Failed,
            RestartPolicy::OnFailure { max_restarts } if entry.restart_count < max_restarts => {
                entry.restart_count = entry.restart_count.saturating_add(1);
                ServiceHealth::Restarting
            }
            RestartPolicy::OnFailure { .. } => ServiceHealth::CrashLoop,
        };
        entry.health = health;
        Ok(health)
    }

    pub fn health(&self, id: ServiceId) -> Result<ServiceHealth, SupervisorError> {
        Ok(self.entry(id)?.health)
    }

    fn entry(&self, id: ServiceId) -> Result<&SupervisorEntry, SupervisorError> {
        self.entries
            .iter()
            .filter_map(Option::as_ref)
            .find(|entry| entry.manifest.id() == id)
            .ok_or(SupervisorError::UnknownService)
    }

    fn entry_mut(&mut self, id: ServiceId) -> Result<&mut SupervisorEntry, SupervisorError> {
        self.entries
            .iter_mut()
            .filter_map(Option::as_mut)
            .find(|entry| entry.manifest.id() == id)
            .ok_or(SupervisorError::UnknownService)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RegistryError, RestartPolicy, ServiceCallError, ServiceHandler, ServiceHealth, ServiceId,
        ServiceManifest, ServiceRegistry, Supervisor, SupervisorError, MAX_DEPENDENCIES,
        MAX_SERVICES, MAX_SERVICE_MESSAGE, MAX_SERVICE_NAME,
    };

    fn echo_handler(request: &[u8], response: &mut [u8]) -> Result<usize, ServiceCallError> {
        if response.len() < request.len() {
            return Err(ServiceCallError::ResponseTooSmall);
        }
        response[..request.len()].copy_from_slice(request);
        Ok(request.len())
    }

    #[test]
    fn resolves_and_calls_a_registered_echo_handler() {
        let id = ServiceId::new(b"echo", 1).expect("id");
        let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
        let mut registry = ServiceRegistry::new();
        let handle = registry
            .register(manifest, echo_handler as ServiceHandler)
            .expect("register");
        registry.mark_ready(handle).expect("ready");
        registry.mark_healthy(handle).expect("healthy");
        let resolved = registry.resolve(id).expect("resolve");
        let mut response = [0; 16];
        let size = registry
            .call(resolved, b"nagi", &mut response)
            .expect("call");
        assert_eq!(&response[..size], b"nagi");
        assert_eq!(handle, resolved);
    }

    #[test]
    fn stale_handles_and_duplicate_identities_are_rejected() {
        let id = ServiceId::new(b"echo", 1).expect("id");
        let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
        let mut registry = ServiceRegistry::new();
        let handle = registry
            .register(manifest, echo_handler as ServiceHandler)
            .expect("register");
        assert_eq!(
            registry.register(manifest, echo_handler as ServiceHandler),
            Err(RegistryError::Duplicate)
        );
        assert!(registry.unregister(handle).is_ok());
        let mut response = [0; 4];
        assert_eq!(
            registry.call(handle, b"x", &mut response),
            Err(RegistryError::StaleHandle)
        );
    }

    #[test]
    fn rejects_unready_calls_and_undersized_responses() {
        let id = ServiceId::new(b"echo", 1).expect("id");
        let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
        let mut registry = ServiceRegistry::new();
        let handle = registry
            .register(manifest, echo_handler as ServiceHandler)
            .expect("register");
        let mut response = [0; 16];
        assert_eq!(
            registry.call(handle, b"nagi", &mut response),
            Err(RegistryError::Unavailable)
        );
        registry.mark_ready(handle).expect("ready");
        assert_eq!(
            registry.call(handle, b"nagi", &mut [0; 2]),
            Err(RegistryError::Handler(ServiceCallError::ResponseTooSmall))
        );
    }

    #[test]
    fn bounds_names_dependencies_requests_and_registry_capacity() {
        assert!(ServiceId::new(&[], 1).is_none());
        assert!(ServiceId::new(&[b'x'; MAX_SERVICE_NAME + 1], 1).is_none());
        assert!(ServiceId::new(b"echo", 0).is_none());

        let id = ServiceId::new(b"echo", 1).expect("id");
        let dependencies = [id; MAX_DEPENDENCIES + 1];
        assert!(ServiceManifest::new(id, &dependencies, RestartPolicy::Never).is_none());

        let mut registry = ServiceRegistry::new();
        for version in 1..=MAX_SERVICES as u16 {
            let id = ServiceId::new(b"echo", version).expect("id");
            let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
            let handle = registry
                .register(manifest, echo_handler as ServiceHandler)
                .expect("capacity slot");
            registry.mark_ready(handle).expect("ready");
            registry.mark_healthy(handle).expect("healthy");
        }
        let extra = ServiceId::new(b"extra", 1).expect("extra");
        let extra_manifest =
            ServiceManifest::new(extra, &[], RestartPolicy::Never).expect("manifest");
        assert_eq!(
            registry.register(extra_manifest, echo_handler as ServiceHandler),
            Err(RegistryError::Capacity)
        );
        let handle = registry
            .resolve(ServiceId::new(b"echo", 1).unwrap())
            .unwrap();
        let mut response = [0; 1];
        assert_eq!(
            registry.call(handle, &[0; MAX_SERVICE_MESSAGE + 1], &mut response),
            Err(RegistryError::RequestTooLarge)
        );
        assert_eq!(registry.health(handle), Ok(ServiceHealth::Healthy));
    }

    #[test]
    fn orders_dependencies_before_dependents_and_enters_healthy_state() {
        let echo = ServiceId::new(b"echo", 1).expect("echo id");
        let client = ServiceId::new(b"client", 1).expect("client id");
        let manifests = [
            ServiceManifest::new(
                client,
                &[echo],
                RestartPolicy::OnFailure { max_restarts: 1 },
            )
            .expect("client manifest"),
            ServiceManifest::new(echo, &[], RestartPolicy::Never).expect("echo manifest"),
        ];
        let mut supervisor = Supervisor::new();
        let mut order = [ServiceId::empty(); 2];

        assert_eq!(
            supervisor.start_in_dependency_order(&manifests, &mut order),
            Ok(2)
        );
        assert_eq!(order, [echo, client]);
        assert_eq!(supervisor.health(echo), Ok(ServiceHealth::Starting));
        supervisor.mark_ready(echo).expect("echo ready");
        supervisor.mark_healthy(echo).expect("echo healthy");
        supervisor.mark_ready(client).expect("client ready");
        supervisor.mark_healthy(client).expect("client healthy");
        assert_eq!(supervisor.health(client), Ok(ServiceHealth::Healthy));
    }

    #[test]
    fn rejects_missing_and_cyclic_dependencies() {
        let missing = ServiceId::new(b"missing", 1).expect("missing id");
        let client = ServiceId::new(b"client", 1).expect("client id");
        let manifests = [
            ServiceManifest::new(client, &[missing], RestartPolicy::Never)
                .expect("client manifest"),
        ];
        let mut supervisor = Supervisor::new();
        let mut order = [ServiceId::empty(); 1];
        assert_eq!(
            supervisor.start_in_dependency_order(&manifests, &mut order),
            Err(SupervisorError::MissingDependency)
        );

        let first = ServiceId::new(b"first", 1).expect("first id");
        let second = ServiceId::new(b"second", 1).expect("second id");
        let cyclic = [
            ServiceManifest::new(first, &[second], RestartPolicy::Never).expect("first"),
            ServiceManifest::new(second, &[first], RestartPolicy::Never).expect("second"),
        ];
        let mut order = [ServiceId::empty(); 2];
        assert_eq!(
            supervisor.start_in_dependency_order(&cyclic, &mut order),
            Err(SupervisorError::DependencyCycle)
        );
    }

    #[test]
    fn restart_budget_ends_in_crash_loop() {
        let id = ServiceId::new(b"echo", 1).expect("id");
        let manifest = ServiceManifest::new(id, &[], RestartPolicy::OnFailure { max_restarts: 2 })
            .expect("manifest");
        let mut supervisor = Supervisor::new();
        let mut order = [ServiceId::empty(); 1];
        supervisor
            .start_in_dependency_order(&[manifest], &mut order)
            .expect("start");

        assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::Restarting));
        assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::Restarting));
        assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::CrashLoop));
    }
}
