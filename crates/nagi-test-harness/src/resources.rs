use crate::{Failpoint, FailurePlan, HarnessError, Result};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ResourceKind {
    Services,
    Timers,
    Tasks,
    Messages,
    Events,
    Files,
    Bytes,
    Handles,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    limits: BTreeMap<ResourceKind, u64>,
}

impl ResourceLimits {
    pub fn new() -> Self {
        Self {
            limits: BTreeMap::new(),
        }
    }

    pub fn set(mut self, kind: ResourceKind, maximum: u64) -> Self {
        self.limits.insert(kind, maximum);
        self
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::new()
            .set(ResourceKind::Services, 32)
            .set(ResourceKind::Timers, 512)
            .set(ResourceKind::Tasks, 512)
            .set(ResourceKind::Messages, 512)
            .set(ResourceKind::Events, 2048)
            .set(ResourceKind::Files, 512)
            .set(ResourceKind::Bytes, 4 * 1024 * 1024)
            .set(ResourceKind::Handles, 2048)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceSnapshot {
    pub used: BTreeMap<ResourceKind, u64>,
    pub limits: BTreeMap<ResourceKind, u64>,
}

impl ResourceSnapshot {
    pub fn used(&self, kind: ResourceKind) -> u64 {
        self.used.get(&kind).copied().unwrap_or(0)
    }

    pub fn limit(&self, kind: ResourceKind) -> u64 {
        self.limits.get(&kind).copied().unwrap_or(u64::MAX)
    }

    pub fn leaks(&self) -> LeakReport {
        LeakReport {
            leaked: self
                .used
                .iter()
                .filter(|(_, count)| **count > 0)
                .map(|(kind, count)| (*kind, *count))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LeakReport {
    pub leaked: BTreeMap<ResourceKind, u64>,
}

impl LeakReport {
    pub fn is_clean(&self) -> bool {
        self.leaked.is_empty()
    }
}

#[derive(Clone)]
pub struct ResourceBudget {
    inner: Arc<Mutex<BudgetState>>,
    failures: FailurePlan,
}

struct BudgetState {
    limits: BTreeMap<ResourceKind, u64>,
    used: BTreeMap<ResourceKind, u64>,
}

impl ResourceBudget {
    pub fn new(limits: ResourceLimits, failures: FailurePlan) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BudgetState {
                limits: limits.limits,
                used: BTreeMap::new(),
            })),
            failures,
        }
    }

    pub fn acquire(&self, kind: ResourceKind, amount: u64) -> Result<ResourceLease> {
        if self.failures.trip(Failpoint::ResourceExhaustion) {
            return Err(HarnessError::QuotaExceeded(kind));
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let current = state.used.get(&kind).copied().unwrap_or(0);
        let next = current
            .checked_add(amount)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        let maximum = state.limits.get(&kind).copied().unwrap_or(u64::MAX);
        if next > maximum {
            return Err(HarnessError::QuotaExceeded(kind));
        }
        state.used.insert(kind, next);
        drop(state);
        Ok(ResourceLease {
            _inner: Arc::new(LeaseInner {
                budget: self.clone(),
                kind,
                amount,
            }),
        })
    }

    pub fn snapshot(&self) -> ResourceSnapshot {
        let state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        ResourceSnapshot {
            used: state.used.clone(),
            limits: state.limits.clone(),
        }
    }

    pub fn leak_report(&self) -> LeakReport {
        self.snapshot().leaks()
    }

    fn release(&self, kind: ResourceKind, amount: u64) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(current) = state.used.get_mut(&kind) {
            *current = current.saturating_sub(amount);
            if *current == 0 {
                state.used.remove(&kind);
            }
        }
    }
}

#[derive(Clone)]
pub struct ResourceLease {
    _inner: Arc<LeaseInner>,
}

struct LeaseInner {
    budget: ResourceBudget,
    kind: ResourceKind,
    amount: u64,
}

impl Drop for LeaseInner {
    fn drop(&mut self) {
        self.budget.release(self.kind, self.amount);
    }
}
