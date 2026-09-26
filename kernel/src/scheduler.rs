use nagi_abi::BOOTSTRAP_USER_THREAD_COUNT;

pub const BLOCKED: u32 = 0;
pub const RUNNABLE: u32 = 1;
pub const RUNNING: u32 = 2;
pub const DONE: u32 = 3;

pub fn wake_transition(state: u32) -> Option<u32> {
    (state == BLOCKED).then_some(RUNNABLE)
}

const NO_THREAD: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserThreadState {
    Empty,
    Runnable,
    Running,
    Sleeping { wake_at: u64 },
    JoinBlocked { target: u8 },
    Zombie { exit_code: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ThreadSlot {
    state: UserThreadState,
    joiner: u8,
    detached: bool,
}

impl ThreadSlot {
    const EMPTY: Self = Self {
        state: UserThreadState::Empty,
        joiner: NO_THREAD,
        detached: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinOutcome {
    Completed(u64),
    Blocked,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitOutcome {
    pub next_thread: Option<u8>,
    pub woken_joiner: Option<(u8, u64)>,
}

/// Fixed-capacity cooperative scheduler for the one-process M17 bootstrap.
///
/// This state machine is deliberately independent of x86 syscall assembly so
/// its transitions can be tested on the host. It does not provide preemption:
/// callers change state only at an explicit Nagi syscall boundary.
pub struct BootstrapUserThreads {
    slots: [ThreadSlot; BOOTSTRAP_USER_THREAD_COUNT],
    current: u8,
    cursor: u8,
}

impl BootstrapUserThreads {
    pub const fn new() -> Self {
        let mut slots = [ThreadSlot::EMPTY; BOOTSTRAP_USER_THREAD_COUNT];
        slots[0].state = UserThreadState::Running;
        Self {
            slots,
            current: 0,
            cursor: 0,
        }
    }

    pub const fn current(&self) -> u8 {
        self.current
    }

    pub fn state(&self, thread: u8) -> Option<UserThreadState> {
        self.slots.get(thread as usize).map(|slot| slot.state)
    }

    pub fn allocate(&mut self) -> Option<u8> {
        let id = (1..BOOTSTRAP_USER_THREAD_COUNT)
            .find(|&id| self.slots[id].state == UserThreadState::Empty)?;
        self.slots[id] = ThreadSlot {
            state: UserThreadState::Runnable,
            ..ThreadSlot::EMPTY
        };
        Some(id as u8)
    }

    pub fn discard_unstarted(&mut self, thread: u8) -> bool {
        let Some(slot) = self.slots.get_mut(thread as usize) else {
            return false;
        };
        if slot.state != UserThreadState::Runnable || slot.joiner != NO_THREAD {
            return false;
        }
        *slot = ThreadSlot::EMPTY;
        true
    }

    pub fn yield_current(&mut self, now: u64) -> Option<u8> {
        let current = self.current as usize;
        if self.slots[current].state != UserThreadState::Running {
            return None;
        }
        self.slots[current].state = UserThreadState::Runnable;
        self.select_runnable(now)
    }

    pub fn sleep_current(&mut self, wake_at: u64, now: u64) -> Option<u8> {
        let current = self.current as usize;
        if self.slots[current].state != UserThreadState::Running {
            return None;
        }
        self.slots[current].state = UserThreadState::Sleeping { wake_at };
        self.select_runnable(now)
    }

    pub fn join_current(&mut self, target: u8, now: u64) -> JoinOutcome {
        let caller = self.current;
        if target == 0 || target == caller || target as usize >= BOOTSTRAP_USER_THREAD_COUNT {
            return JoinOutcome::Invalid;
        }

        let target_index = target as usize;
        let target_state = self.slots[target_index].state;
        let target_detached = self.slots[target_index].detached;
        let target_joiner = self.slots[target_index].joiner;
        match target_state {
            UserThreadState::Empty => return JoinOutcome::Invalid,
            UserThreadState::Zombie { exit_code } => {
                if target_detached || target_joiner != NO_THREAD {
                    return JoinOutcome::Invalid;
                }
                self.slots[target_index] = ThreadSlot::EMPTY;
                return JoinOutcome::Completed(exit_code);
            }
            _ if target_detached || target_joiner != NO_THREAD => {
                return JoinOutcome::Invalid;
            }
            _ => {}
        }

        if self.slots[caller as usize].state != UserThreadState::Running {
            return JoinOutcome::Invalid;
        }
        self.slots[target_index].joiner = caller;
        self.slots[caller as usize].state = UserThreadState::JoinBlocked { target };
        let _ = self.select_runnable(now);
        JoinOutcome::Blocked
    }

    /// Abort a join that cannot make progress because no thread is runnable
    /// and no sleeping thread has a deadline. The syscall reports EAGAIN.
    pub fn abort_blocked_join(&mut self) -> Option<u8> {
        let current = self.current;
        let UserThreadState::JoinBlocked { target } = self.slots[current as usize].state else {
            return None;
        };
        let target_slot = &mut self.slots[target as usize];
        if target_slot.joiner == current {
            target_slot.joiner = NO_THREAD;
        }
        self.slots[current as usize].state = UserThreadState::Running;
        Some(current)
    }

    pub fn detach(&mut self, target: u8) -> bool {
        if target == 0 || target as usize >= BOOTSTRAP_USER_THREAD_COUNT {
            return false;
        }
        let slot = &mut self.slots[target as usize];
        if slot.state == UserThreadState::Empty || slot.joiner != NO_THREAD {
            return false;
        }
        if matches!(slot.state, UserThreadState::Zombie { .. }) {
            *slot = ThreadSlot::EMPTY;
        } else {
            slot.detached = true;
        }
        true
    }

    pub fn exit_current(&mut self, exit_code: u64, now: u64) -> Option<ExitOutcome> {
        let current = self.current;
        if current == 0 || self.slots[current as usize].state != UserThreadState::Running {
            return None;
        }

        let current_slot = &mut self.slots[current as usize];
        let joiner = current_slot.joiner;
        let detached = current_slot.detached;
        if joiner != NO_THREAD {
            *current_slot = ThreadSlot::EMPTY;
            let joiner_slot = &mut self.slots[joiner as usize];
            if matches!(joiner_slot.state, UserThreadState::JoinBlocked { target } if target == current)
            {
                joiner_slot.state = UserThreadState::Runnable;
            }
        } else if detached {
            *current_slot = ThreadSlot::EMPTY;
        } else {
            current_slot.state = UserThreadState::Zombie { exit_code };
        }

        // Give a blocked joiner the first opportunity to reap the exited
        // child's user-space stack before making that thread ID reusable by
        // another runnable thread.
        let preferred = (joiner != NO_THREAD).then_some(joiner);
        let next_thread = self.select_runnable_prefer(now, preferred);
        Some(ExitOutcome {
            next_thread,
            woken_joiner: (joiner != NO_THREAD).then_some((joiner, exit_code)),
        })
    }

    pub fn wake_expired(&mut self, now: u64) {
        for slot in &mut self.slots {
            if matches!(slot.state, UserThreadState::Sleeping { wake_at } if wake_at <= now) {
                slot.state = UserThreadState::Runnable;
            }
        }
    }

    pub fn select_runnable(&mut self, now: u64) -> Option<u8> {
        self.select_runnable_prefer(now, None)
    }

    fn select_runnable_prefer(&mut self, now: u64, preferred: Option<u8>) -> Option<u8> {
        self.wake_expired(now);
        if let Some(thread) = preferred {
            let index = thread as usize;
            if self.slots.get(index)?.state == UserThreadState::Runnable {
                self.slots[index].state = UserThreadState::Running;
                self.current = thread;
                self.cursor = thread;
                return Some(thread);
            }
        }
        for offset in 1..=BOOTSTRAP_USER_THREAD_COUNT {
            let id = (self.cursor as usize + offset) % BOOTSTRAP_USER_THREAD_COUNT;
            if self.slots[id].state == UserThreadState::Runnable {
                self.slots[id].state = UserThreadState::Running;
                self.current = id as u8;
                self.cursor = id as u8;
                return Some(id as u8);
            }
        }
        None
    }

    pub fn ticks_until_wake(&self, now: u64) -> Option<u64> {
        self.slots
            .iter()
            .filter_map(|slot| match slot.state {
                UserThreadState::Sleeping { wake_at } => Some(wake_at.saturating_sub(now)),
                _ => None,
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        wake_transition, BootstrapUserThreads, JoinOutcome, UserThreadState, BLOCKED, DONE,
        RUNNABLE, RUNNING,
    };

    #[test]
    fn kernel_task_wake_transition_only_wakes_blocked_tasks() {
        assert_eq!(wake_transition(BLOCKED), Some(RUNNABLE));
        assert_eq!(wake_transition(RUNNABLE), None);
        assert_eq!(wake_transition(RUNNING), None);
        assert_eq!(wake_transition(DONE), None);
    }

    #[test]
    fn allocation_is_bounded_and_reuses_released_slots() {
        let mut threads = BootstrapUserThreads::new();
        let mut ids = [0_u8; 15];
        for id in &mut ids {
            *id = threads.allocate().unwrap();
        }
        assert_eq!(ids, core::array::from_fn(|index| index as u8 + 1));
        assert_eq!(threads.allocate(), None);
        assert!(threads.discard_unstarted(5));
        assert_eq!(threads.allocate(), Some(5));
    }

    #[test]
    fn yield_switches_round_robin_and_returns_to_the_initial_thread() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.allocate(), Some(2));
        assert_eq!(threads.yield_current(0), Some(1));
        assert_eq!(threads.yield_current(0), Some(2));
        assert_eq!(threads.yield_current(0), Some(0));
    }

    #[test]
    fn sleeping_thread_wakes_at_its_guest_deadline() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.yield_current(0), Some(1));
        assert_eq!(threads.sleep_current(10, 1), Some(0));
        assert_eq!(threads.ticks_until_wake(3), Some(7));
        assert_eq!(threads.yield_current(9), Some(0));
        assert_eq!(threads.yield_current(10), Some(1));
        assert_eq!(threads.state(1), Some(UserThreadState::Running));
    }

    #[test]
    fn join_blocks_then_receives_exit_value_and_releases_target() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Blocked);
        assert_eq!(threads.current(), 1);
        let outcome = threads.exit_current(0xfeed, 0).unwrap();
        assert_eq!(outcome.next_thread, Some(0));
        assert_eq!(outcome.woken_joiner, Some((0, 0xfeed)));
        assert_eq!(threads.state(1), Some(UserThreadState::Empty));
    }

    #[test]
    fn joiner_runs_before_other_runnable_threads_can_reuse_the_child_id() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.allocate(), Some(2));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Blocked);
        assert_eq!(threads.current(), 1);

        let outcome = threads.exit_current(0xfeed, 0).unwrap();

        assert_eq!(outcome.next_thread, Some(0));
        assert_eq!(threads.current(), 0);
        assert_eq!(threads.state(2), Some(UserThreadState::Runnable));
        assert_eq!(threads.state(1), Some(UserThreadState::Empty));
    }

    #[test]
    fn join_can_reap_a_completed_thread_exactly_once() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.yield_current(0), Some(1));
        let outcome = threads.exit_current(u64::MAX, 0).unwrap();
        assert_eq!(outcome.next_thread, Some(0));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Completed(u64::MAX));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Invalid);
    }

    #[test]
    fn detached_threads_release_on_exit_and_cannot_be_joined() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert!(threads.detach(1));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Invalid);
        assert_eq!(threads.yield_current(0), Some(1));
        assert_eq!(threads.exit_current(0, 0).unwrap().next_thread, Some(0));
        assert_eq!(threads.state(1), Some(UserThreadState::Empty));
    }

    #[test]
    fn a_target_cannot_be_claimed_by_a_second_joiner() {
        let mut threads = BootstrapUserThreads::new();
        assert_eq!(threads.allocate(), Some(1));
        assert_eq!(threads.allocate(), Some(2));
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Blocked);
        assert_eq!(threads.current(), 1);
        assert_eq!(threads.join_current(2, 0), JoinOutcome::Blocked);
        assert_eq!(threads.current(), 2);
        assert_eq!(threads.join_current(1, 0), JoinOutcome::Invalid);
        assert_eq!(threads.state(2), Some(UserThreadState::Running));
    }
}
