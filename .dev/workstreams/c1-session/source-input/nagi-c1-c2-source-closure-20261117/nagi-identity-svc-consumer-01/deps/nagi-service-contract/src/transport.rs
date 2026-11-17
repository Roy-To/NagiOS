use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CallContext,
    CancellationToken, ContractVersion, IpcError, IpcErrorCode, PrincipalId, ProviderError,
    ProviderErrorCode, RequestEnvelope, ResponseEnvelope, ServiceAvailability, ServiceDescriptor,
    ServiceId, ServiceRegistry,
};

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, ProviderError>> + Send + 'a>>;
pub type ServiceFuture<'a> = Pin<Box<dyn Future<Output = ResponseEnvelope> + Send + 'a>>;

pub trait ServiceProvider: Send + Sync {
    fn descriptor(&self) -> &ServiceDescriptor;

    fn availability(&self) -> ServiceAvailability {
        ServiceAvailability::Available
    }

    fn handle<'a>(&'a self, request: RequestEnvelope, context: CallContext) -> ProviderFuture<'a>;
}

pub trait ServiceDiscovery: Send + Sync {
    fn descriptors(&self) -> Vec<ServiceDescriptor>;

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError>;
}

pub trait ServiceTransport: ServiceDiscovery + Send + Sync {
    /// Dispatches a request. A transport must establish the caller from its
    /// trusted execution boundary; caller identity is deliberately absent
    /// from `RequestEnvelope`.
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a>;
}

/// A client wrapper over a replaceable transport and its discovery surface.
pub struct ServiceClient {
    transport: Arc<dyn ServiceTransport>,
}

impl ServiceClient {
    pub fn new(transport: Arc<dyn ServiceTransport>) -> Self {
        Self { transport }
    }

    pub fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.transport.descriptors()
    }

    pub fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.transport.resolve(service_id, version)
    }

    pub fn call(
        &self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'_> {
        self.transport.call(request, cancellation)
    }
}

/// Host-testable reference transport. It keeps the public request contract
/// independent from the target Channel and NIPC serialization layers.
pub struct InProcessReferenceTransport {
    registry: Arc<ServiceRegistry>,
    authorization: Arc<dyn AuthorizationPolicy>,
    authenticated_caller: PrincipalId,
}

impl InProcessReferenceTransport {
    /// Build a reference adapter with a principal supplied by trusted host
    /// wiring. A target transport must derive this identity from its runtime
    /// endpoint instead of request data.
    pub fn new(
        registry: Arc<ServiceRegistry>,
        authorization: Arc<dyn AuthorizationPolicy>,
        authenticated_caller: PrincipalId,
    ) -> Self {
        Self {
            registry,
            authorization,
            authenticated_caller,
        }
    }
}

impl ServiceDiscovery for InProcessReferenceTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.registry.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.registry
            .resolve_provider(service_id, version)
            .map(|(_, descriptor)| descriptor)
    }
}

impl ServiceTransport for InProcessReferenceTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        Box::pin(async move {
            let metadata = request.response_metadata();
            if cancellation.is_cancelled() {
                return ResponseEnvelope::failure(metadata, IpcError::new(IpcErrorCode::Cancelled));
            }

            let (provider, descriptor) = match self
                .registry
                .resolve_provider(request.service_id(), request.contract_version())
            {
                Ok(resolved) => resolved,
                Err(error) => return ResponseEnvelope::failure(metadata, error),
            };
            let Some(operation) = descriptor.operation(request.operation_id()).cloned() else {
                return ResponseEnvelope::failure(
                    metadata,
                    IpcError::new(IpcErrorCode::OperationNotFound),
                );
            };
            let authorization_context = AuthorizationContext::from_request(
                &request,
                &self.authenticated_caller,
                operation.required_capability(),
            );
            if self.authorization.authorize(&authorization_context) == AuthorizationDecision::Deny {
                return ResponseEnvelope::failure(
                    metadata,
                    IpcError::new(IpcErrorCode::PermissionDenied),
                );
            }
            match provider.availability() {
                ServiceAvailability::Available => {}
                ServiceAvailability::Busy => {
                    return ResponseEnvelope::failure(metadata, IpcError::new(IpcErrorCode::Busy));
                }
                ServiceAvailability::Unavailable => {
                    return ResponseEnvelope::failure(
                        metadata,
                        IpcError::new(IpcErrorCode::Unavailable),
                    );
                }
            }
            if cancellation.is_cancelled() {
                return ResponseEnvelope::failure(metadata, IpcError::new(IpcErrorCode::Cancelled));
            }

            let result = provider
                .handle(
                    request,
                    CallContext::new(cancellation.clone(), self.authenticated_caller.clone()),
                )
                .await;
            if cancellation.is_cancelled() {
                return ResponseEnvelope::failure(metadata, IpcError::new(IpcErrorCode::Cancelled));
            }
            match result {
                Ok(payload) => ResponseEnvelope::success(metadata, payload),
                Err(error) => ResponseEnvelope::failure(metadata, map_provider_error(error.code())),
            }
        })
    }
}

fn map_provider_error(code: ProviderErrorCode) -> IpcError {
    let code = match code {
        ProviderErrorCode::InvalidRequest => IpcErrorCode::InvalidRequest,
        ProviderErrorCode::PermissionDenied => IpcErrorCode::PermissionDenied,
        ProviderErrorCode::Unavailable => IpcErrorCode::Unavailable,
        ProviderErrorCode::Busy => IpcErrorCode::Busy,
        ProviderErrorCode::Cancelled => IpcErrorCode::Cancelled,
        ProviderErrorCode::DeadlineExceeded => IpcErrorCode::DeadlineExceeded,
        ProviderErrorCode::SerializationFailure => IpcErrorCode::SerializationFailure,
        ProviderErrorCode::TransportFailure => IpcErrorCode::TransportFailure,
        ProviderErrorCode::Failure => IpcErrorCode::ProviderFailure,
    };
    IpcError::new(code)
}
