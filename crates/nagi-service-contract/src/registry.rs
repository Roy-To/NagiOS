use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::transport::ServiceProvider;
use crate::{ContractVersion, IpcError, IpcErrorCode, ServiceDescriptor, ServiceId};

struct RegisteredProvider {
    provider: Arc<dyn ServiceProvider>,
    descriptor: ServiceDescriptor,
    generation: u64,
}

struct RegistryState {
    next_generation: u64,
    providers: BTreeMap<ServiceId, RegisteredProvider>,
}

/// Isolated host-side registration and discovery state for reference tests.
/// It does not replace `libnagi`'s target service registry or supervisor.
pub struct ServiceRegistry {
    state: RwLock<RegistryState>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(RegistryState {
                next_generation: 1,
                providers: BTreeMap::new(),
            }),
        }
    }

    pub fn register(
        &self,
        provider: Arc<dyn ServiceProvider>,
    ) -> Result<RegistrationHandle, IpcError> {
        let descriptor = provider.descriptor().clone();
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.providers.contains_key(descriptor.id()) {
            return Err(IpcError::new(IpcErrorCode::DuplicateRegistration));
        }
        let generation = state.next_generation;
        state.next_generation = next_generation(generation);
        let service_id = descriptor.id().clone();
        state.providers.insert(
            service_id.clone(),
            RegisteredProvider {
                provider,
                descriptor,
                generation,
            },
        );
        Ok(RegistrationHandle {
            service_id,
            generation,
        })
    }

    pub fn unregister(&self, registration: &RegistrationHandle) -> Result<(), IpcError> {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = state.providers.get(registration.service_id()) else {
            return Err(IpcError::new(IpcErrorCode::ServiceNotFound));
        };
        if current.generation != registration.generation {
            return Err(IpcError::new(IpcErrorCode::StaleRegistration));
        }
        state.providers.remove(registration.service_id());
        Ok(())
    }

    pub fn descriptors(&self) -> Vec<ServiceDescriptor> {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .providers
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect()
    }

    pub(crate) fn resolve_provider(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<(Arc<dyn ServiceProvider>, ServiceDescriptor), IpcError> {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(registered) = state.providers.get(service_id) else {
            return Err(IpcError::new(IpcErrorCode::ServiceNotFound));
        };
        if !registered.descriptor.supports_version(version) {
            return Err(IpcError::unsupported_version(
                version,
                registered.descriptor.versions(),
            ));
        }
        Ok((
            Arc::clone(&registered.provider),
            registered.descriptor.clone(),
        ))
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationHandle {
    service_id: ServiceId,
    generation: u64,
}

impl RegistrationHandle {
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

fn next_generation(current: u64) -> u64 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}
