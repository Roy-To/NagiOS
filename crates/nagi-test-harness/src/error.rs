use crate::{Failpoint, ResourceKind};
use std::fmt;

pub type Result<T> = std::result::Result<T, HarnessError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessError {
    InvalidIdentifier,
    InvalidPath,
    PathEscapesRoot,
    NotFound,
    AlreadyExists,
    QuotaExceeded(ResourceKind),
    Injected(Failpoint),
    CorruptStateInjected,
    Unauthorized,
    MissingContext,
    DeadlineExpired,
    Cancelled,
    MalformedMessage,
    ProviderFailed,
    QueueFull,
    ServiceNotFound,
    DependencyMissing(String),
    DependencyCycle,
    ServiceFailure(String),
    OfflineNetworkDenied,
    ArithmeticOverflow,
    InvalidConfiguration(String),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => formatter.write_str("invalid identifier"),
            Self::InvalidPath => formatter.write_str("invalid fixture path"),
            Self::PathEscapesRoot => formatter.write_str("fixture path escapes its root"),
            Self::NotFound => formatter.write_str("fixture entry not found"),
            Self::AlreadyExists => formatter.write_str("fixture entry already exists"),
            Self::QuotaExceeded(kind) => write!(formatter, "resource quota exceeded: {kind:?}"),
            Self::Injected(point) => write!(formatter, "failure injected: {point:?}"),
            Self::CorruptStateInjected => formatter.write_str("corrupt state injected"),
            Self::Unauthorized => formatter.write_str("operation denied by fake capability broker"),
            Self::MissingContext => formatter.write_str("authorization context is missing"),
            Self::DeadlineExpired => formatter.write_str("virtual deadline expired"),
            Self::Cancelled => formatter.write_str("operation cancelled"),
            Self::MalformedMessage => formatter.write_str("malformed IPC message"),
            Self::ProviderFailed => formatter.write_str("fake provider is unavailable"),
            Self::QueueFull => formatter.write_str("fake IPC queue is full"),
            Self::ServiceNotFound => formatter.write_str("fake service not found"),
            Self::DependencyMissing(service) => {
                write!(formatter, "service dependency is missing: {service}")
            }
            Self::DependencyCycle => formatter.write_str("service dependency cycle detected"),
            Self::ServiceFailure(reason) => {
                write!(formatter, "fake service lifecycle failed: {reason}")
            }
            Self::OfflineNetworkDenied => {
                formatter.write_str("network access denied by offline policy")
            }
            Self::ArithmeticOverflow => {
                formatter.write_str("deterministic resource arithmetic overflow")
            }
            Self::InvalidConfiguration(reason) => {
                write!(formatter, "invalid harness configuration: {reason}")
            }
        }
    }
}

impl std::error::Error for HarnessError {}
