use crate::{CapabilityRequest, CapabilityRight, Location, PermissionDecision};

pub trait CapabilityAuthorizer {
    fn decide(&self, request: &CapabilityRequest) -> PermissionDecision;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityGrant {
    pub scope: Location,
    pub right: CapabilityRight,
    pub decision: PermissionDecision,
}

impl CapabilityGrant {
    pub fn allow(scope: Location, right: CapabilityRight) -> Self {
        Self {
            scope,
            right,
            decision: PermissionDecision::Allow,
        }
    }

    pub fn ask(scope: Location, right: CapabilityRight) -> Self {
        Self {
            scope,
            right,
            decision: PermissionDecision::Ask,
        }
    }

    pub fn deny(scope: Location, right: CapabilityRight) -> Self {
        Self {
            scope,
            right,
            decision: PermissionDecision::Deny,
        }
    }

    pub fn unavailable(scope: Location, right: CapabilityRight) -> Self {
        Self {
            scope,
            right,
            decision: PermissionDecision::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    grants: Vec<CapabilityGrant>,
}

impl CapabilitySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_grants(grants: impl IntoIterator<Item = CapabilityGrant>) -> Self {
        Self {
            grants: grants.into_iter().collect(),
        }
    }

    pub fn grant(&mut self, grant: CapabilityGrant) {
        self.grants.push(grant);
    }

    pub fn grants(&self) -> &[CapabilityGrant] {
        &self.grants
    }
}

impl CapabilityAuthorizer for CapabilitySet {
    fn decide(&self, request: &CapabilityRequest) -> PermissionDecision {
        let matching: Vec<_> = self
            .grants
            .iter()
            .filter(|grant| {
                grant.right == request.right && request.location.is_within(&grant.scope)
            })
            .collect();
        let most_specific = matching
            .iter()
            .map(|grant| grant.scope.components().count())
            .max();
        let applicable: Vec<_> = matching
            .into_iter()
            .filter(|grant| Some(grant.scope.components().count()) == most_specific)
            .collect();

        if applicable
            .iter()
            .any(|grant| grant.decision == PermissionDecision::Deny)
        {
            return PermissionDecision::Deny;
        }
        if applicable
            .iter()
            .any(|grant| grant.decision == PermissionDecision::Allow)
        {
            return PermissionDecision::Allow;
        }
        if applicable
            .iter()
            .any(|grant| grant.decision == PermissionDecision::Ask)
        {
            return PermissionDecision::Ask;
        }
        if applicable
            .iter()
            .any(|grant| grant.decision == PermissionDecision::Unavailable)
        {
            return PermissionDecision::Unavailable;
        }
        PermissionDecision::Deny
    }
}
