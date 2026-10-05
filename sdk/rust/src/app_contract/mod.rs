//! Host-testable, transport-independent application contracts.
//!
//! These types describe the boundary between an app and a future user-space
//! app host. They do not launch processes, grant capabilities, or access host
//! files and devices.

mod capability;
mod error;
mod identity;
mod intent;
mod ipc;
mod lifecycle;
mod manifest;
mod registration;
mod state;

pub use capability::{
    resolve_capabilities, CapabilityDecision, CapabilityRequest, CapabilityResolver,
};
pub use error::{AppError, ErrorCode, ErrorEnvelope};
pub use identity::{
    is_valid_app_identifier, is_valid_app_version, AppIdentity, AppOrigin, CorrelationId,
    DisplayName, LocaleResolution, RequestId, EN_US, JA_JP,
};
pub use intent::{validate_intent, DeepLink, Intent, IntentDeclaration};
pub use ipc::{
    IpcEnvelope, MessageKind, OrderingMetadata, IPC_PROTOCOL_VERSION, MAX_IPC_PAYLOAD_BYTES,
};
pub use lifecycle::{
    invoke_graceful_shutdown, GracefulShutdownHook, LifecycleEvent, LifecycleEventKind,
    LifecycleMachine, LifecycleState, LifecycleTransition, ResumeTarget,
};
pub use manifest::{
    valid_locale_tag, AppEntrypoint, AppManifestContract, BackgroundServiceDeclaration,
    EntrypointKind, ResourceReference, StateCompatibility,
};
pub use registration::AppRegistry;
pub use state::{
    restore_state, AppStateBackend, AppStateNamespace, AppStateVersion, RestoreKind, RestoreResult,
    StateLoad, StateMigrator,
};

/// Version of the on-disk/serialized manifest schema. Independent from SDK,
/// IPC, and persisted-state versions.
pub const APP_MANIFEST_SCHEMA_VERSION: u16 = 1;

/// SDK contract version requested by a manifest. A compatible host must have
/// the same major version and at least the requested minor version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractVersion {
    pub major: u16,
    pub minor: u16,
}

pub const SDK_CONTRACT_VERSION: ContractVersion = ContractVersion { major: 1, minor: 0 };

impl ContractVersion {
    pub const fn is_compatible_with(self, host: Self) -> bool {
        self.major == host.major && self.minor <= host.minor
    }
}

#[cfg(test)]
mod version_tests {
    use super::{ContractVersion, APP_MANIFEST_SCHEMA_VERSION, SDK_CONTRACT_VERSION};

    #[test]
    fn schema_sdk_ipc_and_state_versions_are_distinct_contracts() {
        assert_eq!(APP_MANIFEST_SCHEMA_VERSION, 1);
        assert_eq!(SDK_CONTRACT_VERSION, ContractVersion { major: 1, minor: 0 });
        assert_eq!(super::IPC_PROTOCOL_VERSION, 1);
        assert_ne!(APP_MANIFEST_SCHEMA_VERSION as u32, 0);
    }

    #[test]
    fn compatible_contract_requires_same_major_and_sufficient_minor() {
        assert!(ContractVersion { major: 1, minor: 0 }
            .is_compatible_with(ContractVersion { major: 1, minor: 3 }));
        assert!(!ContractVersion { major: 1, minor: 4 }
            .is_compatible_with(ContractVersion { major: 1, minor: 3 }));
        assert!(!ContractVersion { major: 2, minor: 0 }
            .is_compatible_with(ContractVersion { major: 1, minor: 9 }));
    }
}
