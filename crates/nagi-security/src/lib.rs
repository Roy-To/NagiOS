#![no_std]

//! Shared capability identities and deterministic permission policy primitives.
//!
//! This crate defines policy data and evaluation only. It does not grant
//! kernel handles, execute actions, show dialogs, or persist data by itself.

mod codec;
mod evaluator;
mod store;
mod types;

pub use codec::{
    decode_delegation_grant, decode_permission_grant, encode_delegation_grant,
    encode_permission_grant, CodecError, MAX_ENCODED_DELEGATION_BYTES, MAX_ENCODED_GRANT_BYTES,
};
pub use evaluator::{evaluate, evaluate_and_record, AuditSink};
pub use store::{InMemoryPermissionStore, PermissionKey, PermissionStore, StoreError};
pub use types::{
    ActionCorrelationId, Actor, AuditEvent, CapabilityDeclaration, CapabilityId, CapabilityKind,
    CapabilityRequirement, CapabilityScope, CapabilityScopeTemplate, DecisionReason,
    DelegationGrant, DelegationId, DeviceClassId, DomainName, Evaluation, EvaluationContext,
    IdentifierError, InvocationKind, LocalizationKey, PermissionDecision, PermissionGrant,
    PolicyRequest, RiskClass, SystemServiceId,
};

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests;
