use crate::{
    DeterministicClock, DiagnosticsRecorder, FailurePlan, FakeCapabilityBroker, FakeFilesystem,
    FakeServiceBus, HarnessError, OfflineNetworkPolicy, ResourceBudget, ResourceLimits, Result,
    ServiceHarness, UserProfileFixture,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessConfig {
    pub test_id: String,
    pub resource_limits: ResourceLimits,
    pub diagnostics_capacity: usize,
    pub ipc_queue_capacity: usize,
}

impl HarnessConfig {
    pub fn new(test_id: impl Into<String>) -> Self {
        Self {
            test_id: test_id.into(),
            resource_limits: ResourceLimits::default(),
            diagnostics_capacity: 512,
            ipc_queue_capacity: 128,
        }
    }
}

pub struct HarnessBuilder {
    config: HarnessConfig,
}

impl HarnessBuilder {
    pub fn new(test_id: impl Into<String>) -> Self {
        Self {
            config: HarnessConfig::new(test_id),
        }
    }

    pub fn config(mut self, config: HarnessConfig) -> Self {
        self.config = config;
        self
    }

    pub fn resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.config.resource_limits = limits;
        self
    }

    pub fn diagnostics_capacity(mut self, capacity: usize) -> Self {
        self.config.diagnostics_capacity = capacity;
        self
    }

    pub fn ipc_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.ipc_queue_capacity = capacity;
        self
    }

    pub fn build(self) -> Result<Harness> {
        if self.config.test_id.is_empty() || self.config.test_id.len() > 96 {
            return Err(HarnessError::InvalidConfiguration(
                "test_id must contain 1 to 96 bytes".to_owned(),
            ));
        }
        if self.config.diagnostics_capacity == 0 || self.config.ipc_queue_capacity == 0 {
            return Err(HarnessError::InvalidConfiguration(
                "diagnostics and IPC capacities must be nonzero".to_owned(),
            ));
        }
        let failures = FailurePlan::default();
        let resources = ResourceBudget::new(self.config.resource_limits, failures.clone());
        let clock = DeterministicClock::new(resources.clone());
        let diagnostics = DiagnosticsRecorder::new(
            clock.clone(),
            resources.clone(),
            self.config.test_id.clone(),
            self.config.diagnostics_capacity,
        );
        let filesystem = FakeFilesystem::new(resources.clone(), failures.clone());
        let bus = FakeServiceBus::new(
            resources.clone(),
            failures.clone(),
            clock.clone(),
            self.config.ipc_queue_capacity,
            Some(diagnostics.clone()),
        );
        let capabilities = FakeCapabilityBroker::new().with_diagnostics(diagnostics.clone());
        let network = OfflineNetworkPolicy::new().with_diagnostics(diagnostics.clone());
        let services = ServiceHarness::new(
            resources.clone(),
            failures.clone(),
            Some(diagnostics.clone()),
        );
        Ok(Harness {
            test_id: self.config.test_id,
            clock,
            filesystem,
            bus,
            capabilities,
            diagnostics,
            network,
            failures,
            resources,
            services,
            next_fixture: Arc::new(AtomicU64::new(1)),
        })
    }
}

pub struct Harness {
    pub test_id: String,
    pub clock: DeterministicClock,
    pub filesystem: FakeFilesystem,
    pub bus: FakeServiceBus,
    pub capabilities: FakeCapabilityBroker,
    pub diagnostics: DiagnosticsRecorder,
    pub network: OfflineNetworkPolicy,
    pub failures: FailurePlan,
    pub resources: ResourceBudget,
    pub services: ServiceHarness,
    next_fixture: Arc<AtomicU64>,
}

impl Harness {
    pub fn advance_time(&self, delta: u64) -> Result<u64> {
        let now = self.clock.advance(delta)?;
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("delta".to_owned(), delta.to_string());
        self.diagnostics
            .record(crate::DiagnosticKind::TimeAdvanced, None, "clock", fields);
        Ok(now)
    }

    pub fn user_profile_fixture(&self) -> Result<UserProfileFixture> {
        let sequence = self
            .next_fixture
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .map_err(|_| HarnessError::ArithmeticOverflow)?;
        UserProfileFixture::new(&self.filesystem, sequence)
    }

    pub fn result(
        &self,
        stage: impl Into<String>,
        failure_class: Option<String>,
        cleanup_complete: bool,
    ) -> crate::HarnessResult {
        let mut result = crate::HarnessResult::new(self.test_id.clone(), stage, self.clock.now());
        result.failure_class = failure_class;
        result.cleanup_complete = cleanup_complete;
        result
    }
}
