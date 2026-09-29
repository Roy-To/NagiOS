use crate::{HarnessError, ResourceBudget, ResourceKind, Result};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TimerId(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledEvent {
    pub id: TimerId,
    pub due_at: u64,
    pub label: String,
}

#[derive(Clone)]
pub struct DeterministicClock {
    inner: Arc<Mutex<ClockState>>,
    budget: ResourceBudget,
}

struct ClockState {
    now: u64,
    next_id: u64,
    timers: BTreeMap<(u64, u64), (String, crate::resources::ResourceLease)>,
    by_id: BTreeMap<u64, u64>,
}

impl DeterministicClock {
    pub fn new(budget: ResourceBudget) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClockState {
                now: 0,
                next_id: 1,
                timers: BTreeMap::new(),
                by_id: BTreeMap::new(),
            })),
            budget,
        }
    }

    pub fn now(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .now
    }

    pub fn advance(&self, delta: u64) -> Result<u64> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.now = state
            .now
            .checked_add(delta)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        Ok(state.now)
    }

    pub fn schedule(&self, delay: u64, label: impl Into<String>) -> Result<TimerId> {
        let label = label.into();
        if label.is_empty() || label.len() > 128 {
            return Err(HarnessError::InvalidConfiguration(
                "timer label must contain 1 to 128 bytes".to_owned(),
            ));
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let due_at = state
            .now
            .checked_add(delay)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        let id = state.next_id;
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        let lease = self.budget.acquire(ResourceKind::Timers, 1)?;
        state.by_id.insert(id, due_at);
        state.timers.insert((due_at, id), (label, lease));
        Ok(TimerId(id))
    }

    pub fn cancel(&self, id: TimerId) -> bool {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(due_at) = state.by_id.remove(&id.0) else {
            return false;
        };
        state.timers.remove(&(due_at, id.0));
        true
    }

    /// Return due events in virtual deadline then insertion order.
    /// The caller executes each event explicitly, so scheduled work never races.
    pub fn drain_due(&self) -> Vec<ScheduledEvent> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = state.now;
        let keys: Vec<_> = state
            .timers
            .range(..=(now, u64::MAX))
            .map(|(key, _)| *key)
            .collect();
        keys.into_iter()
            .filter_map(|(due_at, id)| {
                let (label, _lease) = state.timers.remove(&(due_at, id))?;
                state.by_id.remove(&id);
                Some(ScheduledEvent {
                    id: TimerId(id),
                    due_at,
                    label,
                })
            })
            .collect()
    }

    pub fn pending_timers(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .timers
            .len()
    }
}
