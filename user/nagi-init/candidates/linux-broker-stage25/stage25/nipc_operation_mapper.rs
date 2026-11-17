//! Trusted table from NIPC v1 destination/payload type to a service-contract
//! operation and its declared capability.
//!
//! The SDK envelope has no standalone operation field. A route owner therefore
//! supplies this exact mapping from a trusted service descriptor; request body
//! bytes cannot select an operation or capability. Payload schema validation
//! remains the target service's responsibility.

use super::{
    CanonicalNipcOperationBinding, DecodedNipcRequest, PeerDenial, ServiceEffectOwnerId,
    TrustedRouteKey,
};
use nagi_service_contract::{ContractVersion, OperationId, ServiceDescriptor, ServiceId};
use std::collections::HashSet;

/// One trusted association between an NIPC request envelope and a declared
/// service-contract operation. Construction is reserved for the route owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedNipcOperationRoute {
    route: TrustedRouteKey,
    destination_app_id: u64,
    payload_type: String,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    capability: nagi_service_contract::CapabilityId,
    effect_owner: ServiceEffectOwnerId,
}

impl TrustedNipcOperationRoute {
    pub(crate) const fn route_key(&self) -> TrustedRouteKey {
        self.route
    }

    /// Bind an exact NIPC payload type to an operation in the trusted service
    /// descriptor. Operations without a declared capability are not admitted
    /// by this authority adapter.
    pub(crate) fn from_service_contract(
        route: TrustedRouteKey,
        destination_app_id: u64,
        payload_type: impl Into<String>,
        descriptor: &ServiceDescriptor,
        contract_version: ContractVersion,
        operation_id: &OperationId,
        effect_owner: ServiceEffectOwnerId,
    ) -> Result<Self, PeerDenial> {
        if !descriptor.supports_version(contract_version) {
            return Err(PeerDenial::UnknownNipcOperation);
        }
        let operation = descriptor
            .operation(operation_id)
            .ok_or(PeerDenial::UnknownNipcOperation)?;
        let capability = operation
            .required_capability()
            .cloned()
            .ok_or(PeerDenial::NotDeclared)?;
        let payload_type = payload_type.into();
        if payload_type.is_empty() || payload_type.len() > 128 {
            return Err(PeerDenial::InvalidNipcRequest);
        }
        Ok(Self {
            route,
            destination_app_id,
            payload_type,
            service_id: descriptor.id().clone(),
            contract_version,
            operation_id: operation.id().clone(),
            capability,
            effect_owner,
        })
    }
}

/// Immutable exact-match mapper owned by the canonical service-route owner.
/// A destination with no registered entry is an unknown route; a known
/// destination with an unregistered payload type is an unknown operation.
#[derive(Clone, Debug)]
pub(crate) struct CanonicalNipcOperationMapper {
    routes: Vec<TrustedNipcOperationRoute>,
}

impl CanonicalNipcOperationMapper {
    pub(crate) fn from_trusted_routes(
        routes: impl IntoIterator<Item = TrustedNipcOperationRoute>,
    ) -> Result<Self, PeerDenial> {
        let routes = routes.into_iter().collect::<Vec<_>>();
        if routes.is_empty() {
            return Err(PeerDenial::EmptyNipcOperationRouteSet);
        }
        let mut seen = HashSet::with_capacity(routes.len());
        for route in &routes {
            if !seen.insert((route.destination_app_id, route.payload_type.clone())) {
                return Err(PeerDenial::DuplicateNipcOperationRoute);
            }
        }
        Ok(Self { routes })
    }

    pub(crate) fn bind_request(
        &self,
        request: &DecodedNipcRequest,
    ) -> Result<CanonicalNipcOperationBinding, PeerDenial> {
        let destination_app_id = request
            .destination_app_id()
            .ok_or(PeerDenial::UnknownNipcRoute)?;
        let mut known_destination = false;
        for route in self
            .routes
            .iter()
            .filter(|route| route.destination_app_id == destination_app_id)
        {
            known_destination = true;
            if route.payload_type == request.payload_type() {
                return CanonicalNipcOperationBinding::from_route_owner(
                    route.route,
                    request,
                    route.service_id.clone(),
                    route.contract_version,
                    route.operation_id.clone(),
                    route.capability.clone(),
                    route.effect_owner,
                );
            }
        }
        Err(if known_destination {
            PeerDenial::UnknownNipcOperation
        } else {
            PeerDenial::UnknownNipcRoute
        })
    }
}
