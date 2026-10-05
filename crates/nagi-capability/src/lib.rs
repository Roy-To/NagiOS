//! Host-side capability declaration and permission policy foundation.
//!
//! This crate has no kernel, service, compositor, browser, or device hooks.
//! Runtime owners can implement the enforcer contract after the Nagi 0.2
//! integration gate opens.

#![forbid(unsafe_code)]

mod declaration;
mod evaluator;
mod ids;
mod model;
mod registry;
mod store;

pub use declaration::{
    bind_declaration, BoundCapabilityRequest, CapabilityDeclaration, DeclarationError,
    DeclaredCapability,
};
pub use evaluator::{
    AccessRequest, CapabilityEnforcer, Decision, DecisionEffect, DecisionReason, EvaluationContext,
    PermissionEvaluator,
};
pub use ids::{CapabilityId, IdentifierError, PrincipalId, SessionId};
pub use model::{
    CapabilityScope, Grant, GrantEffect, GrantId, GrantLifetime, GrantSource, GrantValidationError,
    Principal, PrincipalError, PrincipalKind, ScopeError, UnixTimestamp,
};
pub use registry::{CapabilityDefinition, CapabilityRegistry, RegistryError};
pub use store::{
    FailingPolicyStore, InMemoryPolicyStore, PolicyDocument, PolicyStore, PolicyStoreError,
};

#[cfg(test)]
mod tests;
