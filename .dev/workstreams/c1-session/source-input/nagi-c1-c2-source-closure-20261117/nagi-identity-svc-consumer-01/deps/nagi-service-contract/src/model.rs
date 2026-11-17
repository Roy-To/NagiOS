use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use nagi_capability::{CapabilityId, PrincipalId};

use crate::{ContractVersion, CorrelationId, IpcError, OperationId, RequestId, ServiceId, TraceId};

pub const MAX_SERVICE_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceOperation {
    id: OperationId,
    required_capability: Option<CapabilityId>,
}

impl ServiceOperation {
    pub fn new(id: OperationId, required_capability: Option<CapabilityId>) -> Self {
        Self {
            id,
            required_capability,
        }
    }

    pub fn id(&self) -> &OperationId {
        &self.id
    }

    pub fn required_capability(&self) -> Option<&CapabilityId> {
        self.required_capability.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceDescriptor {
    id: ServiceId,
    versions: BTreeSet<ContractVersion>,
    operations: BTreeMap<OperationId, ServiceOperation>,
}

impl ServiceDescriptor {
    pub fn new(
        id: ServiceId,
        versions: impl IntoIterator<Item = ContractVersion>,
        operations: impl IntoIterator<Item = ServiceOperation>,
    ) -> Result<Self, DescriptorError> {
        let version_list = versions.into_iter().collect::<Vec<_>>();
        if version_list.is_empty() {
            return Err(DescriptorError::NoContractVersions);
        }
        let versions = version_list.iter().copied().collect::<BTreeSet<_>>();
        if versions.len() != version_list.len() {
            return Err(DescriptorError::DuplicateContractVersion);
        }
        let mut operation_map = BTreeMap::new();
        for operation in operations {
            if operation_map
                .insert(operation.id.clone(), operation)
                .is_some()
            {
                return Err(DescriptorError::DuplicateOperation);
            }
        }
        if operation_map.is_empty() {
            return Err(DescriptorError::NoOperations);
        }
        Ok(Self {
            id,
            versions,
            operations: operation_map,
        })
    }

    pub fn id(&self) -> &ServiceId {
        &self.id
    }

    pub fn versions(&self) -> impl Iterator<Item = ContractVersion> + '_ {
        self.versions.iter().copied()
    }

    pub fn operations(&self) -> impl Iterator<Item = &ServiceOperation> {
        self.operations.values()
    }

    pub fn operation(&self, id: &OperationId) -> Option<&ServiceOperation> {
        self.operations.get(id)
    }

    pub fn supports_version(&self, version: ContractVersion) -> bool {
        self.versions.contains(&version)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    NoContractVersions,
    DuplicateContractVersion,
    NoOperations,
    DuplicateOperation,
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoContractVersions => "service descriptor needs at least one contract version",
            Self::DuplicateContractVersion => {
                "service descriptor contains a duplicate contract version"
            }
            Self::NoOperations => "service descriptor needs at least one operation",
            Self::DuplicateOperation => "service descriptor contains a duplicate operation",
        })
    }
}

impl std::error::Error for DescriptorError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceAvailability {
    Available,
    Busy,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestEnvelope {
    request_id: RequestId,
    correlation_id: Option<CorrelationId>,
    trace_id: Option<TraceId>,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    payload: Vec<u8>,
}

impl RequestEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: RequestId,
        correlation_id: Option<CorrelationId>,
        trace_id: Option<TraceId>,
        service_id: ServiceId,
        contract_version: ContractVersion,
        operation_id: OperationId,
        payload: Vec<u8>,
    ) -> Result<Self, RequestBuildError> {
        if payload.len() > MAX_SERVICE_PAYLOAD_BYTES {
            return Err(RequestBuildError::PayloadTooLarge {
                actual: payload.len(),
                maximum: MAX_SERVICE_PAYLOAD_BYTES,
            });
        }
        Ok(Self {
            request_id,
            correlation_id,
            trace_id,
            service_id,
            contract_version,
            operation_id,
            payload,
        })
    }

    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub const fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    pub const fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }

    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub const fn contract_version(&self) -> ContractVersion {
        self.contract_version
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(crate) fn response_metadata(&self) -> ResponseMetadata {
        ResponseMetadata {
            request_id: self.request_id,
            correlation_id: self.correlation_id,
            trace_id: self.trace_id,
            service_id: self.service_id.clone(),
            contract_version: self.contract_version,
            operation_id: self.operation_id.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestBuildError {
    PayloadTooLarge { actual: usize, maximum: usize },
}

impl fmt::Display for RequestBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge { actual, maximum } => {
                write!(formatter, "payload size {actual} exceeds maximum {maximum}")
            }
        }
    }
}

impl std::error::Error for RequestBuildError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseMetadata {
    request_id: RequestId,
    correlation_id: Option<CorrelationId>,
    trace_id: Option<TraceId>,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
}

impl ResponseMetadata {
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub const fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    pub const fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }

    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub const fn contract_version(&self) -> ContractVersion {
        self.contract_version
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseEnvelope {
    metadata: ResponseMetadata,
    result: Result<Vec<u8>, IpcError>,
}

impl ResponseEnvelope {
    pub(crate) fn success(metadata: ResponseMetadata, payload: Vec<u8>) -> Self {
        if payload.len() > MAX_SERVICE_PAYLOAD_BYTES {
            return Self::failure(
                metadata,
                IpcError::new(crate::IpcErrorCode::ResponseTooLarge),
            );
        }
        Self {
            metadata,
            result: Ok(payload),
        }
    }

    pub(crate) fn failure(metadata: ResponseMetadata, error: IpcError) -> Self {
        Self {
            metadata,
            result: Err(error),
        }
    }

    pub fn metadata(&self) -> &ResponseMetadata {
        &self.metadata
    }

    pub fn result(&self) -> Result<&[u8], &IpcError> {
        self.result.as_deref()
    }

    pub fn into_result(self) -> Result<Vec<u8>, IpcError> {
        self.result
    }
}

struct CancellationState {
    cancelled: AtomicBool,
    next_waiter: AtomicU64,
    waiters: Mutex<BTreeMap<u64, Waker>>,
}

#[derive(Clone)]
pub struct CancellationToken(Arc<CancellationState>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(CancellationState {
            cancelled: AtomicBool::new(false),
            next_waiter: AtomicU64::new(1),
            waiters: Mutex::new(BTreeMap::new()),
        }))
    }

    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let waiters = std::mem::take(
            &mut *self
                .0
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, waiter) in waiters {
            waiter.wake();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// A runtime-neutral future providers can await while doing cooperative work.
    pub fn cancelled(&self) -> CancellationFuture {
        CancellationFuture {
            state: Arc::clone(&self.0),
            waiter_id: None,
        }
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl Future for CancellationFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.state.cancelled.load(Ordering::Acquire) {
            this.remove_waiter();
            return Poll::Ready(());
        }
        let mut waiters = this
            .state
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if this.state.cancelled.load(Ordering::Acquire) {
            drop(waiters);
            this.remove_waiter();
            return Poll::Ready(());
        }
        match this.waiter_id {
            Some(waiter_id) => {
                if let Some(waker) = waiters.get_mut(&waiter_id) {
                    if !waker.will_wake(context.waker()) {
                        *waker = context.waker().clone();
                    }
                }
            }
            None => {
                let waiter_id = this
                    .state
                    .next_waiter
                    .fetch_add(1, Ordering::Relaxed)
                    .max(1);
                waiters.insert(waiter_id, context.waker().clone());
                this.waiter_id = Some(waiter_id);
            }
        }
        Poll::Pending
    }
}

impl CancellationFuture {
    fn remove_waiter(&mut self) {
        if let Some(waiter_id) = self.waiter_id.take() {
            self.state
                .waiters
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&waiter_id);
        }
    }
}

impl Drop for CancellationFuture {
    fn drop(&mut self) {
        self.remove_waiter();
    }
}

pub struct CancellationFuture {
    state: Arc<CancellationState>,
    waiter_id: Option<u64>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CancellationToken {}

#[derive(Clone, Debug)]
pub struct CallContext {
    cancellation: CancellationToken,
    caller: PrincipalId,
}

impl CallContext {
    pub(crate) fn new(cancellation: CancellationToken, caller: PrincipalId) -> Self {
        Self {
            cancellation,
            caller,
        }
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    /// Principal established by the transport's trusted execution boundary.
    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationContext {
    caller: PrincipalId,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    request_id: RequestId,
    correlation_id: Option<CorrelationId>,
    required_capability: Option<CapabilityId>,
}

impl AuthorizationContext {
    pub(crate) fn from_request(
        request: &RequestEnvelope,
        caller: &PrincipalId,
        required_capability: Option<&CapabilityId>,
    ) -> Self {
        Self {
            caller: caller.clone(),
            service_id: request.service_id.clone(),
            contract_version: request.contract_version,
            operation_id: request.operation_id.clone(),
            request_id: request.request_id,
            correlation_id: request.correlation_id,
            required_capability: required_capability.cloned(),
        }
    }

    pub fn caller(&self) -> &PrincipalId {
        &self.caller
    }

    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub const fn contract_version(&self) -> ContractVersion {
        self.contract_version
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub const fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    pub fn required_capability(&self) -> Option<&CapabilityId> {
        self.required_capability.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationDecision {
    Allow,
    Deny,
}

pub trait AuthorizationPolicy: Send + Sync {
    fn authorize(&self, context: &AuthorizationContext) -> AuthorizationDecision;
}

pub struct DenyAllAuthorization;

impl AuthorizationPolicy for DenyAllAuthorization {
    fn authorize(&self, _context: &AuthorizationContext) -> AuthorizationDecision {
        AuthorizationDecision::Deny
    }
}
