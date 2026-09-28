use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use nagi_service_contract::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CancellationToken,
    ContractVersion, CorrelationId, InProcessReferenceTransport, OperationId, PrincipalId,
    ProviderError, ProviderErrorCode, ProviderFuture, RequestEnvelope, RequestId,
    ServiceAvailability, ServiceClient, ServiceDescriptor, ServiceId, ServiceOperation,
    ServiceProvider, ServiceRegistry, ServiceTransport,
};

struct EchoService {
    descriptor: ServiceDescriptor,
}

impl EchoService {
    fn new() -> Self {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new("example.echo").expect("service id"),
            [ContractVersion::new(1, 0).expect("version")],
            [ServiceOperation::new(
                OperationId::new("echo").expect("operation id"),
                None,
            )],
        )
        .expect("descriptor");
        Self { descriptor }
    }
}

impl ServiceProvider for EchoService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn availability(&self) -> ServiceAvailability {
        ServiceAvailability::Available
    }

    fn handle<'a>(
        &'a self,
        request: RequestEnvelope,
        context: nagi_service_contract::CallContext,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            if context.cancellation().is_cancelled() {
                return Err(ProviderError::new(ProviderErrorCode::Cancelled));
            }
            Ok(request.payload().to_vec())
        })
    }
}

struct ExamplePolicy;

impl AuthorizationPolicy for ExamplePolicy {
    fn authorize(&self, context: &AuthorizationContext) -> AuthorizationDecision {
        if context.caller().as_str() == "example.echo-client"
            && context.service_id().as_str() == "example.echo"
            && context.operation_id().as_str() == "echo"
        {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        }
    }
}

fn poll_ready<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the reference echo example has no pending work"),
    }
}

fn main() {
    let registry = Arc::new(ServiceRegistry::new());
    registry
        .register(Arc::new(EchoService::new()))
        .expect("register echo provider");
    let transport = Arc::new(InProcessReferenceTransport::new(
        registry,
        Arc::new(ExamplePolicy),
        PrincipalId::new("example.echo-client").expect("trusted example caller"),
    ));
    let client = ServiceClient::new(transport as Arc<dyn ServiceTransport>);
    let request = RequestEnvelope::new(
        RequestId::new(1).expect("request id"),
        Some(CorrelationId::new(1).expect("correlation id")),
        None,
        ServiceId::new("example.echo").expect("service id"),
        ContractVersion::new(1, 0).expect("contract version"),
        OperationId::new("echo").expect("operation id"),
        b"hello from the service client".to_vec(),
    )
    .expect("request envelope");
    let response = poll_ready(client.call(request, CancellationToken::new()));
    println!(
        "{}",
        std::str::from_utf8(response.result().expect("echo response")).expect("UTF-8 echo")
    );
}
