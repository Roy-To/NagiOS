//! Host-side contracts and reference transport for Nagi system-service calls.
//!
//! This crate does not implement a target kernel transport, NIPC codec, service
//! supervisor, permission policy, or product service behavior.

#![forbid(unsafe_code)]

mod error;
mod ids;
mod model;
mod registry;
mod transport;

pub use error::{IpcError, IpcErrorCode, ProviderError, ProviderErrorCode};
pub use ids::{
    ContractVersion, CorrelationId, IdentifierError, OperationId, RequestId, ServiceId, TraceId,
    VersionError,
};
pub use model::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CallContext,
    CancellationFuture, CancellationToken, DenyAllAuthorization, DescriptorError,
    RequestBuildError, RequestEnvelope, ResponseEnvelope, ResponseMetadata, ServiceAvailability,
    ServiceDescriptor, ServiceOperation, MAX_SERVICE_PAYLOAD_BYTES,
};
pub use nagi_capability::{CapabilityId, PrincipalId};
pub use registry::{RegistrationHandle, ServiceRegistry};
pub use transport::{
    InProcessReferenceTransport, ProviderFuture, ServiceClient, ServiceDiscovery, ServiceFuture,
    ServiceProvider, ServiceTransport,
};
