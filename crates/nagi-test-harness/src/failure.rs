use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Failpoint {
    ServiceCrash,
    RestartFailure,
    InterruptedWrite,
    CorruptState,
    Timeout,
    Cancellation,
    ProviderFailure,
    MalformedMessage,
    ResourceExhaustion,
}

#[derive(Clone, Default)]
pub struct FailurePlan {
    armed: Arc<Mutex<BTreeMap<Failpoint, u64>>>,
}

impl FailurePlan {
    /// Arm a named failpoint for the next `occurrences` matching operations.
    pub fn arm(&self, point: Failpoint, occurrences: u64) {
        if occurrences == 0 {
            return;
        }
        let mut armed = self
            .armed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let remaining = armed.entry(point).or_default();
        *remaining = remaining.saturating_add(occurrences);
    }

    /// Consume one occurrence, returning true exactly when this operation trips it.
    pub fn trip(&self, point: Failpoint) -> bool {
        let mut armed = self
            .armed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(remaining) = armed.get_mut(&point) else {
            return false;
        };
        *remaining -= 1;
        if *remaining == 0 {
            armed.remove(&point);
        }
        true
    }

    pub fn remaining(&self, point: Failpoint) -> u64 {
        self.armed
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&point)
            .copied()
            .unwrap_or(0)
    }
}
