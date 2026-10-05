//! Exit records and Supervisor waits for isolated processes (ADR 0048).
//!
//! The kernel assigns isolated processes monotonically increasing Process
//! IDs, so a waited-for ID always names exactly one process. When an
//! isolated process exits or is terminated by a fault, its status is kept
//! until the Supervisor consumes it with `SYS_PROCESS_WAIT`. A Supervisor
//! thread waiting on the live process is woken exactly once. This module is
//! the pure, host-testable state machine; the syscall layer drives it.

/// First Process ID handed to an isolated process; PID 1 is init.
pub const FIRST_ISOLATED_PROCESS_ID: u32 = 2;
/// Bound on exit records kept for unconsumed (unwaited) processes.
pub const MAX_EXIT_RECORDS: usize = 4;
/// Isolated processes that may be live at once (ADR 0050). Each needs its
/// own bounded address-space slot in `user_process::child`.
pub const MAX_LIVE_PROCESSES: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitKind {
    /// The process called `SYS_PROCESS_EXIT`.
    Exited,
    /// The kernel terminated the process after a CPU exception.
    Faulted { vector: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitRecord {
    pub process_id: u32,
    pub code: u64,
    pub kind: ExitKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    /// The process has exited; its record is consumed.
    Ready(ExitRecord),
    /// The process is live; `waiter` is registered to be woken at exit.
    Blocked,
    /// No live process and no unconsumed record has this ID, or another
    /// thread already waits on it.
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitTableError {
    /// Every exit-record slot holds an unconsumed record.
    RecordsFull,
    /// The Process ID space is exhausted.
    IdsExhausted,
    /// Every isolated-process slot is live.
    AlreadyLive,
}

pub struct ExitTable {
    next_process_id: u32,
    /// Live isolated processes and, per process, the thread waiting on it.
    live: [Option<(u32, Option<u8>)>; MAX_LIVE_PROCESSES],
    records: [Option<ExitRecord>; MAX_EXIT_RECORDS],
}

impl Default for ExitTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ExitTable {
    pub const fn new() -> Self {
        Self {
            next_process_id: FIRST_ISOLATED_PROCESS_ID,
            live: [None; MAX_LIVE_PROCESSES],
            records: [None; MAX_EXIT_RECORDS],
        }
    }

    fn live_count(&self) -> usize {
        self.live.iter().flatten().count()
    }

    fn free_records(&self) -> usize {
        self.records.iter().filter(|slot| slot.is_none()).count()
    }

    /// Reserve the next Process ID for a spawn. A spawn is refused unless an
    /// exit-record slot remains for every live process plus the new one, so
    /// no exit status can ever be dropped.
    pub fn reserve(&self) -> Result<u32, ExitTableError> {
        if self.live_count() == MAX_LIVE_PROCESSES {
            return Err(ExitTableError::AlreadyLive);
        }
        if self.free_records() < self.live_count() + 1 {
            return Err(ExitTableError::RecordsFull);
        }
        if self.next_process_id == u32::MAX {
            return Err(ExitTableError::IdsExhausted);
        }
        Ok(self.next_process_id)
    }

    /// Mark the reserved ID live after the spawn succeeded.
    pub fn commit_spawn(&mut self, process_id: u32) -> bool {
        if process_id != self.next_process_id || self.reserve() != Ok(process_id) {
            return false;
        }
        let Some(slot) = self.live.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some((process_id, None));
        self.next_process_id += 1;
        true
    }

    pub fn is_live(&self, process_id: u32) -> bool {
        self.live
            .iter()
            .flatten()
            .any(|(live, _)| *live == process_id)
    }

    /// Record a live process's exit. Returns the waiting thread to wake,
    /// if any.
    pub fn record_exit(&mut self, process_id: u32, code: u64, kind: ExitKind) -> Option<u8> {
        let live = self
            .live
            .iter_mut()
            .find(|slot| slot.is_some_and(|(live, _)| live == process_id))?;
        let (_, waiter) = live.take().expect("matched live process");
        // `reserve` kept a free record for every live process.
        if let Some(slot) = self.records.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(ExitRecord {
                process_id,
                code,
                kind,
            });
        }
        waiter
    }

    /// Consume `process_id`'s exit record, or register `thread` to wait for
    /// the live process.
    pub fn wait(&mut self, process_id: u32, thread: u8) -> WaitOutcome {
        if let Some(slot) = self
            .records
            .iter_mut()
            .find(|slot| slot.is_some_and(|record| record.process_id == process_id))
        {
            let record = slot.take().expect("matched record");
            return WaitOutcome::Ready(record);
        }
        if self
            .live
            .iter()
            .flatten()
            .any(|(_, waiter)| *waiter == Some(thread))
        {
            return WaitOutcome::Invalid;
        }
        match self
            .live
            .iter_mut()
            .flatten()
            .find(|(live, _)| *live == process_id)
        {
            Some((_, waiter @ None)) => {
                *waiter = Some(thread);
                WaitOutcome::Blocked
            }
            _ => WaitOutcome::Invalid,
        }
    }

    /// Withdraw a wait that cannot complete (no other thread can run).
    pub fn cancel_wait(&mut self, thread: u8) -> bool {
        for (_, waiter) in self.live.iter_mut().flatten() {
            if *waiter == Some(thread) {
                *waiter = None;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExitKind, ExitRecord, ExitTable, ExitTableError, WaitOutcome, FIRST_ISOLATED_PROCESS_ID,
        MAX_EXIT_RECORDS,
    };

    fn spawn(table: &mut ExitTable) -> u32 {
        let id = table.reserve().expect("reserve");
        assert!(table.commit_spawn(id));
        id
    }

    #[test]
    fn process_ids_are_monotonic_and_never_reused() {
        let mut table = ExitTable::new();
        let first = spawn(&mut table);
        assert_eq!(first, FIRST_ISOLATED_PROCESS_ID);
        let second = spawn(&mut table);
        assert_eq!(second, first + 1);
        assert_eq!(table.reserve(), Err(ExitTableError::AlreadyLive));
        assert!(
            !table.commit_spawn(second),
            "a committed ID cannot be reused"
        );
        table.record_exit(first, 0, ExitKind::Exited);
        assert_eq!(spawn(&mut table), second + 1);
    }

    #[test]
    fn exited_status_is_consumed_exactly_once() {
        let mut table = ExitTable::new();
        let id = spawn(&mut table);
        assert_eq!(table.record_exit(id, 7, ExitKind::Exited), None);
        assert_eq!(
            table.wait(id, 0),
            WaitOutcome::Ready(ExitRecord {
                process_id: id,
                code: 7,
                kind: ExitKind::Exited
            })
        );
        assert_eq!(table.wait(id, 0), WaitOutcome::Invalid);
        assert_eq!(table.wait(id + 1, 0), WaitOutcome::Invalid);
    }

    #[test]
    fn a_blocked_waiter_is_woken_once_with_the_fault_status() {
        let mut table = ExitTable::new();
        let id = spawn(&mut table);
        assert_eq!(table.wait(id, 3), WaitOutcome::Blocked);
        assert_eq!(table.wait(id, 4), WaitOutcome::Invalid, "one waiter");
        assert_eq!(
            table.record_exit(id, 142, ExitKind::Faulted { vector: 14 }),
            Some(3)
        );
        assert_eq!(
            table.wait(id, 3),
            WaitOutcome::Ready(ExitRecord {
                process_id: id,
                code: 142,
                kind: ExitKind::Faulted { vector: 14 }
            })
        );
    }

    #[test]
    fn cancelled_waits_and_stale_exits_are_ignored() {
        let mut table = ExitTable::new();
        let id = spawn(&mut table);
        assert_eq!(table.wait(id, 1), WaitOutcome::Blocked);
        assert!(table.cancel_wait(1));
        assert!(!table.cancel_wait(1));
        assert_eq!(table.record_exit(id + 9, 0, ExitKind::Exited), None);
        assert!(table.is_live(id));
        assert_eq!(table.record_exit(id, 0, ExitKind::Exited), None);
    }

    #[test]
    fn unconsumed_records_bound_further_spawns() {
        let mut table = ExitTable::new();
        for _ in 0..MAX_EXIT_RECORDS {
            let id = spawn(&mut table);
            table.record_exit(id, 0, ExitKind::Exited);
        }
        assert_eq!(table.reserve(), Err(ExitTableError::RecordsFull));
        // A record is held back for every live process.
        let mut table = ExitTable::new();
        for _ in 0..MAX_EXIT_RECORDS - 1 {
            let id = spawn(&mut table);
            table.record_exit(id, 0, ExitKind::Exited);
        }
        let live = spawn(&mut table);
        assert_eq!(table.reserve(), Err(ExitTableError::RecordsFull));
        table.record_exit(live, 9, ExitKind::Exited);
        assert!(matches!(table.wait(live, 0), WaitOutcome::Ready(_)));
        assert!(matches!(
            table.wait(FIRST_ISOLATED_PROCESS_ID, 0),
            WaitOutcome::Ready(_)
        ));
        assert!(table.reserve().is_ok());
    }

    #[test]
    fn concurrent_processes_have_independent_waiters() {
        let mut table = ExitTable::new();
        let first = spawn(&mut table);
        let second = spawn(&mut table);
        assert_eq!(table.wait(first, 1), WaitOutcome::Blocked);
        assert_eq!(
            table.wait(second, 1),
            WaitOutcome::Invalid,
            "one wait per thread"
        );
        assert_eq!(table.wait(second, 2), WaitOutcome::Blocked);
        assert_eq!(table.record_exit(second, 0, ExitKind::Exited), Some(2));
        assert!(table.is_live(first));
        assert_eq!(
            table.record_exit(first, 141, ExitKind::Faulted { vector: 13 }),
            Some(1)
        );
        assert!(matches!(table.wait(first, 1), WaitOutcome::Ready(record) if record.code == 141));
        assert!(matches!(table.wait(second, 2), WaitOutcome::Ready(record) if record.code == 0));
    }
}
