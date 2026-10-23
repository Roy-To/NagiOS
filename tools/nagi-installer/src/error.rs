use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidPackage,
    InvalidMetadata,
    UnsafePath,
    DuplicateDestination,
    VersionConflict,
    DowngradeRejected,
    AlreadyInstalled,
    NotInstalled,
    StagingFailure,
    CommitFailure,
    RollbackFailure,
    InventoryFailure,
    PolicyDenied,
    PolicyFailure,
    UnsupportedSchema,
    IoFailure,
    InvalidTransition,
    ConcurrentModification,
    RecoveryFailure,
    InjectedFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstallerError {
    InvalidPackage(String),
    InvalidMetadata(String),
    UnsafePath(String),
    DuplicateDestination(String),
    VersionConflict {
        installed: String,
        requested: String,
    },
    DowngradeRejected {
        installed: String,
        requested: String,
    },
    AlreadyInstalled(String),
    NotInstalled(String),
    StagingFailure(String),
    CommitFailure(String),
    RollbackFailure(String),
    InventoryFailure(String),
    PolicyDenied(PolicyName),
    PolicyFailure {
        policy: PolicyName,
        message: String,
    },
    UnsupportedSchema {
        kind: &'static str,
        version: u32,
    },
    IoFailure {
        operation: &'static str,
        message: String,
    },
    InvalidTransition {
        from: String,
        to: String,
    },
    ConcurrentModification(String),
    RecoveryFailure(String),
    InjectedFailure {
        point: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyName {
    CapabilityChanges,
    Sbom,
    License,
    Provenance,
    Trust,
    Uninstall,
}

impl InstallerError {
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidPackage(_) => ErrorKind::InvalidPackage,
            Self::InvalidMetadata(_) => ErrorKind::InvalidMetadata,
            Self::UnsafePath(_) => ErrorKind::UnsafePath,
            Self::DuplicateDestination(_) => ErrorKind::DuplicateDestination,
            Self::VersionConflict { .. } => ErrorKind::VersionConflict,
            Self::DowngradeRejected { .. } => ErrorKind::DowngradeRejected,
            Self::AlreadyInstalled(_) => ErrorKind::AlreadyInstalled,
            Self::NotInstalled(_) => ErrorKind::NotInstalled,
            Self::StagingFailure(_) => ErrorKind::StagingFailure,
            Self::CommitFailure(_) => ErrorKind::CommitFailure,
            Self::RollbackFailure(_) => ErrorKind::RollbackFailure,
            Self::InventoryFailure(_) => ErrorKind::InventoryFailure,
            Self::PolicyDenied(_) => ErrorKind::PolicyDenied,
            Self::PolicyFailure { .. } => ErrorKind::PolicyFailure,
            Self::UnsupportedSchema { .. } => ErrorKind::UnsupportedSchema,
            Self::IoFailure { .. } => ErrorKind::IoFailure,
            Self::InvalidTransition { .. } => ErrorKind::InvalidTransition,
            Self::ConcurrentModification(_) => ErrorKind::ConcurrentModification,
            Self::RecoveryFailure(_) => ErrorKind::RecoveryFailure,
            Self::InjectedFailure { .. } => ErrorKind::InjectedFailure,
        }
    }
}

impl fmt::Display for InstallerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPackage(message)
            | Self::InvalidMetadata(message)
            | Self::UnsafePath(message)
            | Self::DuplicateDestination(message)
            | Self::StagingFailure(message)
            | Self::CommitFailure(message)
            | Self::RollbackFailure(message)
            | Self::InventoryFailure(message)
            | Self::ConcurrentModification(message)
            | Self::RecoveryFailure(message) => formatter.write_str(message),
            Self::VersionConflict {
                installed,
                requested,
            } => write!(
                formatter,
                "version {requested} is already installed as {installed}"
            ),
            Self::DowngradeRejected {
                installed,
                requested,
            } => write!(
                formatter,
                "downgrade from {installed} to {requested} is rejected by default"
            ),
            Self::AlreadyInstalled(app_id) => write!(formatter, "{app_id} is already installed"),
            Self::NotInstalled(app_id) => write!(formatter, "{app_id} is not installed"),
            Self::PolicyDenied(policy) => {
                write!(formatter, "policy {policy:?} denied the operation")
            }
            Self::PolicyFailure { policy, message } => {
                write!(formatter, "policy {policy:?} failed: {message}")
            }
            Self::UnsupportedSchema { kind, version } => {
                write!(formatter, "unsupported {kind} schema version {version}")
            }
            Self::IoFailure { operation, message } => write!(formatter, "{operation}: {message}"),
            Self::InvalidTransition { from, to } => {
                write!(formatter, "illegal transaction transition {from} -> {to}")
            }
            Self::InjectedFailure { point, message } => {
                write!(formatter, "injected failure at {point}: {message}")
            }
        }
    }
}

impl std::error::Error for InstallerError {}

pub(crate) fn io_error(operation: &'static str, error: std::io::Error) -> InstallerError {
    InstallerError::IoFailure {
        operation,
        message: error.to_string(),
    }
}
