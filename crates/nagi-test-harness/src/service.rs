use crate::{
    DiagnosticKind, DiagnosticsRecorder, Failpoint, FailurePlan, HarnessError, ResourceBudget,
    ResourceKind, Result, ServiceId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub trait TestService: Send {
    fn start(&mut self) -> Result<()>;
    fn health_check(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
}

pub type ServiceFactory = Arc<dyn Fn() -> Box<dyn TestService> + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceHandle {
    pub service: ServiceId,
    pub instance_id: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleReport {
    pub started: Vec<ServiceHandle>,
    pub stopped: Vec<ServiceHandle>,
    pub errors: Vec<String>,
    pub primary_error: Option<String>,
    pub cleanup_complete: bool,
}

struct ServiceDefinition {
    dependencies: BTreeSet<ServiceId>,
    factory: ServiceFactory,
}

struct RunningService {
    handle: ServiceHandle,
    service: Box<dyn TestService>,
    _lease: crate::resources::ResourceLease,
}

struct StartFailure {
    primary: HarnessError,
    cleanup: Option<HarnessError>,
}

impl StartFailure {
    fn into_error(self) -> HarnessError {
        match self.cleanup {
            Some(cleanup) => HarnessError::ServiceFailure(format!(
                "{}; failed-service cleanup also failed: {}",
                self.primary, cleanup
            )),
            None => self.primary,
        }
    }
}

pub struct ServiceHarness {
    definitions: BTreeMap<ServiceId, ServiceDefinition>,
    running: BTreeMap<ServiceId, RunningService>,
    next_instance_id: u64,
    budget: ResourceBudget,
    failures: FailurePlan,
    diagnostics: Option<DiagnosticsRecorder>,
}

impl ServiceHarness {
    pub fn new(
        budget: ResourceBudget,
        failures: FailurePlan,
        diagnostics: Option<DiagnosticsRecorder>,
    ) -> Self {
        Self {
            definitions: BTreeMap::new(),
            running: BTreeMap::new(),
            next_instance_id: 1,
            budget,
            failures,
            diagnostics,
        }
    }

    pub fn register(
        &mut self,
        id: ServiceId,
        dependencies: impl IntoIterator<Item = ServiceId>,
        factory: ServiceFactory,
    ) -> Result<()> {
        if self.definitions.contains_key(&id) {
            return Err(HarnessError::AlreadyExists);
        }
        if self.definitions.len() >= 128 {
            return Err(HarnessError::QuotaExceeded(ResourceKind::Services));
        }
        self.definitions.insert(
            id,
            ServiceDefinition {
                dependencies: dependencies.into_iter().collect(),
                factory,
            },
        );
        Ok(())
    }

    pub fn start_all(&mut self) -> Result<LifecycleReport> {
        let order = self.start_order()?;
        let mut report = LifecycleReport {
            cleanup_complete: true,
            ..LifecycleReport::default()
        };
        for id in order {
            if let Err(failure) = self.start_one(&id) {
                report.primary_error = Some(failure.primary.to_string());
                let failed_service_cleanup = failure.cleanup.is_some();
                if let Some(cleanup) = failure.cleanup {
                    report
                        .errors
                        .push(format!("{} failed-start cleanup: {cleanup}", id));
                }
                report.cleanup_complete =
                    self.stop_reverse_into(&mut report) && !failed_service_cleanup;
                return Ok(report);
            }
            let handle = self
                .running
                .get(&id)
                .expect("started service inserted")
                .handle
                .clone();
            report.started.push(handle.clone());
            self.record_lifecycle(&handle, "started");
        }
        Ok(report)
    }

    pub fn stop_all(&mut self) -> Result<LifecycleReport> {
        let order = self.start_order()?;
        let mut report = LifecycleReport {
            cleanup_complete: true,
            ..LifecycleReport::default()
        };
        for id in order.into_iter().rev() {
            self.stop_one(&id, &mut report);
        }
        report.cleanup_complete = report.errors.is_empty();
        Ok(report)
    }

    pub fn crash(&mut self, id: &ServiceId) -> Result<ServiceHandle> {
        if !self.failures.trip(Failpoint::ServiceCrash) {
            return Err(HarnessError::InvalidConfiguration(
                "ServiceCrash failpoint is not armed".to_owned(),
            ));
        }
        let running = self
            .running
            .remove(id)
            .ok_or(HarnessError::ServiceNotFound)?;
        self.record_lifecycle(&running.handle, "crashed");
        Ok(running.handle)
    }

    pub fn restart(&mut self, id: &ServiceId) -> Result<ServiceHandle> {
        if self.running.contains_key(id) {
            return Err(HarnessError::InvalidConfiguration(
                "service is already running".to_owned(),
            ));
        }
        let definition = self
            .definitions
            .get(id)
            .ok_or(HarnessError::ServiceNotFound)?;
        for dependency in &definition.dependencies {
            if !self.running.contains_key(dependency) {
                return Err(HarnessError::DependencyMissing(dependency.to_string()));
            }
        }
        if self.failures.trip(Failpoint::RestartFailure) {
            return Err(HarnessError::Injected(Failpoint::RestartFailure));
        }
        self.start_one(id).map_err(StartFailure::into_error)?;
        let handle = self
            .running
            .get(id)
            .expect("restarted service inserted")
            .handle
            .clone();
        self.record_lifecycle(&handle, "restarted");
        Ok(handle)
    }

    pub fn current_handle(&self, id: &ServiceId) -> Option<ServiceHandle> {
        self.running.get(id).map(|running| running.handle.clone())
    }

    pub fn is_current(&self, handle: &ServiceHandle) -> bool {
        self.running
            .get(&handle.service)
            .is_some_and(|running| running.handle.instance_id == handle.instance_id)
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    fn start_one(&mut self, id: &ServiceId) -> std::result::Result<(), StartFailure> {
        let definition = self.definitions.get(id).ok_or(StartFailure {
            primary: HarnessError::ServiceNotFound,
            cleanup: None,
        })?;
        let factory = definition.factory.clone();
        let lease = self
            .budget
            .acquire(ResourceKind::Services, 1)
            .map_err(|primary| StartFailure {
                primary,
                cleanup: None,
            })?;
        let mut service = factory();
        if let Err(error) = service.start().and_then(|()| service.health_check()) {
            return Err(StartFailure {
                primary: error,
                cleanup: service.stop().err(),
            });
        }
        let instance_id = self.next_instance_id;
        self.next_instance_id =
            self.next_instance_id
                .checked_add(1)
                .ok_or_else(|| StartFailure {
                    primary: HarnessError::ArithmeticOverflow,
                    cleanup: service.stop().err(),
                })?;
        let handle = ServiceHandle {
            service: id.clone(),
            instance_id,
        };
        self.running.insert(
            id.clone(),
            RunningService {
                handle,
                service,
                _lease: lease,
            },
        );
        Ok(())
    }

    fn stop_one(&mut self, id: &ServiceId, report: &mut LifecycleReport) {
        let Some(mut running) = self.running.remove(id) else {
            return;
        };
        if let Err(error) = running.service.stop() {
            report.errors.push(format!("{} stop: {error}", id));
        }
        report.stopped.push(running.handle.clone());
        self.record_lifecycle(&running.handle, "stopped");
    }

    fn stop_reverse_into(&mut self, report: &mut LifecycleReport) -> bool {
        let order = match self.start_order() {
            Ok(order) => order,
            Err(error) => {
                report.errors.push(error.to_string());
                self.running.clear();
                return false;
            }
        };
        for id in order.into_iter().rev() {
            self.stop_one(&id, report);
        }
        report.errors.is_empty()
    }

    fn start_order(&self) -> Result<Vec<ServiceId>> {
        for (id, definition) in &self.definitions {
            for dependency in &definition.dependencies {
                if !self.definitions.contains_key(dependency) {
                    return Err(HarnessError::DependencyMissing(dependency.to_string()));
                }
                if dependency == id {
                    return Err(HarnessError::DependencyCycle);
                }
            }
        }
        let mut indegree: BTreeMap<ServiceId, usize> =
            self.definitions.keys().cloned().map(|id| (id, 0)).collect();
        let mut dependents: BTreeMap<ServiceId, Vec<ServiceId>> = BTreeMap::new();
        for (id, definition) in &self.definitions {
            indegree.insert(id.clone(), definition.dependencies.len());
            for dependency in &definition.dependencies {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
        let mut ready: BTreeSet<ServiceId> = indegree
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(id.clone()))
            .collect();
        let mut order = Vec::with_capacity(self.definitions.len());
        while let Some(id) = ready.pop_first() {
            order.push(id.clone());
            if let Some(next) = dependents.get(&id) {
                for dependent in next {
                    let count = indegree.get_mut(dependent).expect("registered dependent");
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(dependent.clone());
                    }
                }
            }
        }
        if order.len() != self.definitions.len() {
            return Err(HarnessError::DependencyCycle);
        }
        Ok(order)
    }

    fn record_lifecycle(&self, handle: &ServiceHandle, action: &str) {
        if let Some(diagnostics) = &self.diagnostics {
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("instance_id".to_owned(), handle.instance_id.to_string());
            fields.insert("service".to_owned(), handle.service.to_string());
            diagnostics.record(DiagnosticKind::ServiceLifecycle, None, action, fields);
        }
    }
}
