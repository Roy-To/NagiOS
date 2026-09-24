use crate::{
    Actor, CapabilityId, CapabilityScope, DelegationGrant, DelegationId, PermissionDecision,
    PermissionGrant,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionKey {
    pub actor: Actor,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Full,
    InvalidDelegation,
}

/// Persistence boundary for permission rows and explicit AI delegation.
/// Implementations belong to a trusted user-space policy service.
pub trait PermissionStore {
    fn get_permission(&self, key: &PermissionKey) -> Option<PermissionGrant>;
    fn set_permission(&mut self, grant: PermissionGrant) -> Result<(), StoreError>;
    fn revoke_permission(&mut self, key: &PermissionKey) -> bool;
    fn permission_count(&self) -> usize;
    fn permission_at(&self, index: usize) -> Option<PermissionGrant>;

    fn get_delegation(&self, id: DelegationId) -> Option<DelegationGrant>;
    fn set_delegation(&mut self, grant: DelegationGrant) -> Result<(), StoreError>;
    fn revoke_delegation(&mut self, id: DelegationId) -> bool;
    fn delegation_count(&self) -> usize;
    fn delegation_at(&self, index: usize) -> Option<DelegationGrant>;

    fn current_decision(&self, key: &PermissionKey) -> Option<PermissionDecision> {
        self.get_permission(key).map(|grant| grant.decision)
    }
}

/// Bounded reference backend. A production service can replace this with its
/// durable store without changing evaluator semantics or the kernel ABI.
pub struct InMemoryPermissionStore<const PERMISSIONS: usize, const DELEGATIONS: usize> {
    permissions: [Option<PermissionGrant>; PERMISSIONS],
    delegations: [Option<DelegationGrant>; DELEGATIONS],
}

impl<const PERMISSIONS: usize, const DELEGATIONS: usize>
    InMemoryPermissionStore<PERMISSIONS, DELEGATIONS>
{
    pub const fn new() -> Self {
        Self {
            permissions: [None; PERMISSIONS],
            delegations: [None; DELEGATIONS],
        }
    }

    fn permission_slot(&self, key: &PermissionKey) -> Option<usize> {
        self.permissions
            .iter()
            .position(|entry| entry.is_some_and(|grant| grant.key() == *key))
    }

    fn delegation_slot(&self, id: DelegationId) -> Option<usize> {
        self.delegations
            .iter()
            .position(|entry| entry.is_some_and(|grant| grant.id == id))
    }

    fn nth_permission(&self, index: usize) -> Option<PermissionGrant> {
        self.permissions
            .iter()
            .filter_map(|entry| *entry)
            .nth(index)
    }

    fn nth_delegation(&self, index: usize) -> Option<DelegationGrant> {
        self.delegations
            .iter()
            .filter_map(|entry| *entry)
            .nth(index)
    }
}

impl<const PERMISSIONS: usize, const DELEGATIONS: usize> Default
    for InMemoryPermissionStore<PERMISSIONS, DELEGATIONS>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const PERMISSIONS: usize, const DELEGATIONS: usize> PermissionStore
    for InMemoryPermissionStore<PERMISSIONS, DELEGATIONS>
{
    fn get_permission(&self, key: &PermissionKey) -> Option<PermissionGrant> {
        self.permission_slot(key)
            .and_then(|index| self.permissions[index])
    }

    fn set_permission(&mut self, grant: PermissionGrant) -> Result<(), StoreError> {
        let key = grant.key();
        if let Some(index) = self.permission_slot(&key) {
            self.permissions[index] = Some(grant);
            return Ok(());
        }
        let index = self
            .permissions
            .iter()
            .position(Option::is_none)
            .ok_or(StoreError::Full)?;
        self.permissions[index] = Some(grant);
        Ok(())
    }

    fn revoke_permission(&mut self, key: &PermissionKey) -> bool {
        let Some(index) = self.permission_slot(key) else {
            return false;
        };
        self.permissions[index] = None;
        true
    }

    fn permission_count(&self) -> usize {
        self.permissions
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }

    fn permission_at(&self, index: usize) -> Option<PermissionGrant> {
        self.nth_permission(index)
    }

    fn get_delegation(&self, id: DelegationId) -> Option<DelegationGrant> {
        self.delegation_slot(id)
            .and_then(|index| self.delegations[index])
    }

    fn set_delegation(&mut self, grant: DelegationGrant) -> Result<(), StoreError> {
        if !matches!(grant.agent, Actor::AiAgent(_)) || grant.expires_at == 0 {
            return Err(StoreError::InvalidDelegation);
        }
        if let Some(index) = self.delegation_slot(grant.id) {
            self.delegations[index] = Some(grant);
            return Ok(());
        }
        let index = self
            .delegations
            .iter()
            .position(Option::is_none)
            .ok_or(StoreError::Full)?;
        self.delegations[index] = Some(grant);
        Ok(())
    }

    fn revoke_delegation(&mut self, id: DelegationId) -> bool {
        let Some(index) = self.delegation_slot(id) else {
            return false;
        };
        self.delegations[index] = None;
        true
    }

    fn delegation_count(&self) -> usize {
        self.delegations
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }

    fn delegation_at(&self, index: usize) -> Option<DelegationGrant> {
        self.nth_delegation(index)
    }
}
