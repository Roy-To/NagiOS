use std::fmt;

use crate::ContractVersion;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IpcErrorCode {
    ServiceNotFound,
    UnsupportedContractVersion,
    OperationNotFound,
    InvalidRequest,
    PermissionDenied,
    Unavailable,
    Busy,
    Cancelled,
    DeadlineExceeded,
    ProviderFailure,
    SerializationFailure,
    TransportFailure,
    DuplicateRegistration,
    StaleRegistration,
    ResponseTooLarge,
}

impl IpcErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServiceNotFound => "service_not_found",
            Self::UnsupportedContractVersion => "unsupported_contract_version",
            Self::OperationNotFound => "operation_not_found",
            Self::InvalidRequest => "invalid_request",
            Self::PermissionDenied => "permission_denied",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::ProviderFailure => "provider_failure",
            Self::SerializationFailure => "serialization_failure",
            Self::TransportFailure => "transport_failure",
            Self::DuplicateRegistration => "duplicate_registration",
            Self::StaleRegistration => "stale_registration",
            Self::ResponseTooLarge => "response_too_large",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpcError {
    code: IpcErrorCode,
    requested_version: Option<ContractVersion>,
    supported_versions: Vec<ContractVersion>,
}

impl IpcError {
    pub const fn new(code: IpcErrorCode) -> Self {
        Self {
            code,
            requested_version: None,
            supported_versions: Vec::new(),
        }
    }

    pub fn unsupported_version(
        requested: ContractVersion,
        supported: impl IntoIterator<Item = ContractVersion>,
    ) -> Self {
        let mut supported_versions = supported.into_iter().collect::<Vec<_>>();
        supported_versions.sort_unstable();
        supported_versions.dedup();
        Self {
            code: IpcErrorCode::UnsupportedContractVersion,
            requested_version: Some(requested),
            supported_versions,
        }
    }

    pub const fn code(&self) -> IpcErrorCode {
        self.code
    }

    pub const fn requested_version(&self) -> Option<ContractVersion> {
        self.requested_version
    }

    pub fn supported_versions(&self) -> &[ContractVersion] {
        &self.supported_versions
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl std::error::Error for IpcError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProviderErrorCode {
    InvalidRequest,
    PermissionDenied,
    Unavailable,
    Busy,
    Cancelled,
    DeadlineExceeded,
    SerializationFailure,
    TransportFailure,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderError {
    code: ProviderErrorCode,
}

impl ProviderError {
    pub const fn new(code: ProviderErrorCode) -> Self {
        Self { code }
    }

    pub const fn code(self) -> ProviderErrorCode {
        self.code
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.code {
            ProviderErrorCode::InvalidRequest => "invalid_request",
            ProviderErrorCode::PermissionDenied => "permission_denied",
            ProviderErrorCode::Unavailable => "unavailable",
            ProviderErrorCode::Busy => "busy",
            ProviderErrorCode::Cancelled => "cancelled",
            ProviderErrorCode::DeadlineExceeded => "deadline_exceeded",
            ProviderErrorCode::SerializationFailure => "serialization_failure",
            ProviderErrorCode::TransportFailure => "transport_failure",
            ProviderErrorCode::Failure => "provider_failure",
        })
    }
}

impl std::error::Error for ProviderError {}
