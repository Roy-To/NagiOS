use crate::{PrincipalId, ResolvedIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalMappingFailure {
    MissingIdentity,
    StaleSession,
    Denied,
    Unavailable,
}

/// Adapter seam to the capability owner's canonical PrincipalId and binding
/// contract. Mapping returns an identifier only; it does not grant authority.
pub trait CapabilityPrincipalAdapter {
    fn principal_for(
        &self,
        identity: &ResolvedIdentity,
    ) -> Result<PrincipalId, PrincipalMappingFailure>;
}

/// Fail-closed default for hosts without an approved capability binding.
#[derive(Default)]
pub struct DenyCapabilityPrincipalAdapter;

impl CapabilityPrincipalAdapter for DenyCapabilityPrincipalAdapter {
    fn principal_for(
        &self,
        identity: &ResolvedIdentity,
    ) -> Result<PrincipalId, PrincipalMappingFailure> {
        if !identity.is_live() {
            return Err(PrincipalMappingFailure::StaleSession);
        }
        Err(PrincipalMappingFailure::Unavailable)
    }
}

pub fn resolve_capability_principal(
    adapter: &impl CapabilityPrincipalAdapter,
    identity: &ResolvedIdentity,
) -> Result<PrincipalId, PrincipalMappingFailure> {
    if !identity.is_live() {
        return Err(PrincipalMappingFailure::StaleSession);
    }
    let principal = adapter.principal_for(identity)?;
    if principal.as_str().is_empty() {
        return Err(PrincipalMappingFailure::Denied);
    }
    Ok(principal)
}
