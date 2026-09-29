//! Deterministic host-only fixtures for testing service orchestration contracts.
//!
//! This crate intentionally has no network, process, wall-clock, or host-file
//! system adapter. A passing test validates only the fake contract exercised by
//! that test; it does not establish target, provider, or product acceptance.

mod bus;
mod capability;
mod clock;
mod diagnostics;
mod error;
mod failure;
mod filesystem;
mod fixture;
mod harness;
mod ids;
mod offline;
mod resources;
mod service;

pub use bus::{
    BusFault, BusMessage, BusOutcome, CancellationToken, FakeServiceBus, ReceivedMessage,
};
pub use capability::{AuthorizationDecision, CapabilityRule, FakeCapabilityBroker};
pub use clock::{DeterministicClock, ScheduledEvent, TimerId};
pub use diagnostics::{DiagnosticEvent, DiagnosticKind, DiagnosticsRecorder};
pub use error::{HarnessError, Result};
pub use failure::{Failpoint, FailurePlan};
pub use filesystem::{FakeFilesystem, FileData, TemporaryRoot};
pub use fixture::{FixtureCleanupReport, FixtureIds, UserProfileFixture};
pub use harness::{Harness, HarnessBuilder, HarnessConfig};
pub use ids::{PrincipalId, ServiceId};
pub use offline::{OfflineNetworkPolicy, OfflineRequest};
pub use resources::{
    LeakReport, ResourceBudget, ResourceKind, ResourceLease, ResourceLimits, ResourceSnapshot,
};
pub use service::{LifecycleReport, ServiceFactory, ServiceHandle, ServiceHarness, TestService};

/// A stable disclaimer included in every harness result and reusable in reports.
pub const ACCEPTANCE_DISCLAIMER: &str =
    "Host-only fakes validate orchestration/contracts only; they do not confer provider, target, or product acceptance.";

/// Result metadata suitable for inclusion in an acceptance report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessResult {
    pub test_id: String,
    pub stage: String,
    pub virtual_time: u64,
    pub failure_class: Option<String>,
    pub cleanup_complete: bool,
    pub disclaimer: &'static str,
}

impl HarnessResult {
    pub fn new(test_id: impl Into<String>, stage: impl Into<String>, virtual_time: u64) -> Self {
        Self {
            test_id: test_id.into(),
            stage: stage.into(),
            virtual_time,
            failure_class: None,
            cleanup_complete: true,
            disclaimer: ACCEPTANCE_DISCLAIMER,
        }
    }
}
