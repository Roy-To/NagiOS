use crate::{PrincipalId, ResolutionError, ResolvedIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalMappingFailure {
    MissingIdentity,
    /// The identity was confirmed ended or expired.
    StaleSession,
    /// The committed identity snapshot was corrupt and was rejected. This is
    /// not confirmation that the Session was ended; explicit recovery is
    /// required before liveness can be established again.
    CorruptState,
    Denied,
    /// Liveness could not be confirmed because a gate or dependency was
    /// unavailable. This does not prove the Session was revoked.
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
        identity.check_live().map_err(map_resolution_error)?;
        Err(PrincipalMappingFailure::Unavailable)
    }
}

pub fn resolve_capability_principal(
    adapter: &impl CapabilityPrincipalAdapter,
    identity: &ResolvedIdentity,
) -> Result<PrincipalId, PrincipalMappingFailure> {
    identity.check_live().map_err(map_resolution_error)?;
    let principal = adapter.principal_for(identity)?;
    // Adapter callbacks may expire or end the session before returning.
    identity.check_live().map_err(map_resolution_error)?;
    if principal.as_str().is_empty() {
        return Err(PrincipalMappingFailure::Denied);
    }
    Ok(principal)
}

fn map_resolution_error(error: ResolutionError) -> PrincipalMappingFailure {
    match error {
        ResolutionError::NoSession | ResolutionError::Ended | ResolutionError::Expired => {
            PrincipalMappingFailure::StaleSession
        }
        ResolutionError::CorruptState => PrincipalMappingFailure::CorruptState,
        ResolutionError::Denied => PrincipalMappingFailure::Denied,
        ResolutionError::Unavailable => PrincipalMappingFailure::Unavailable,
    }
}
