//! Single-owner child wait coordinator shared by ProcessLifecycle polling and
//! PID-1's SIGCHLD wait. Linux supplies P_ALL WNOWAIT observation and exact
//! PID reaping; tracked tickets are verified through their retained pidfd.

use super::ServiceExitStatus;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use nagi_init_spawn_contract::SpawnedProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildReapState {
    Unobserved,
    Attempted,
    Reaped(ServiceExitStatus),
    Unknown,
}

#[derive(Clone)]
pub(super) struct ReapTicket {
    process_id: u32,
    pidfd: i32,
    identity: Option<SpawnedProcessIdentity>,
    state: Arc<Mutex<ChildReapState>>,
}

/// All fallible allocation/capacity growth is performed before clone3. Binding
/// the syscall result to this reservation moves only already-owned values.
pub(super) struct ReapTicketReservation {
    state: Arc<Mutex<ChildReapState>>,
}

impl ReapTicketReservation {
    pub(super) fn bind(
        self,
        identity: SpawnedProcessIdentity,
        pidfd: i32,
    ) -> ReapTicket {
        let process_id = identity.process_id();
        ReapTicket {
            process_id,
            pidfd,
            identity: Some(identity),
            state: self.state,
        }
    }

    pub(super) fn bind_unverified(self, process_id: u32) -> ReapTicket {
        let state = self.state;
        if let Ok(mut current) = state.lock() {
            *current = ChildReapState::Unknown;
        }
        ReapTicket {
            process_id,
            pidfd: -1,
            identity: None,
            state,
        }
    }
}

impl ReapTicket {
    #[cfg(test)]
    pub(super) fn new(process_id: u32, pidfd: i32) -> io::Result<Self> {
        if process_id <= 1 || pidfd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tracked child requires a valid PID and pidfd identity",
            ));
        }
        Ok(Self {
            process_id,
            pidfd,
            identity: None,
            state: Arc::new(Mutex::new(ChildReapState::Unobserved)),
        })
    }

    pub(super) fn unverified(process_id: u32) -> Self {
        Self {
            process_id,
            pidfd: -1,
            identity: None,
            state: Arc::new(Mutex::new(ChildReapState::Unknown)),
        }
    }

    pub(super) fn identity(&self) -> Option<&SpawnedProcessIdentity> {
        self.identity.as_ref()
    }

    pub(super) fn process_id(&self) -> u32 {
        self.process_id
    }

    pub(super) fn pidfd(&self) -> i32 {
        self.pidfd
    }

    pub(super) fn exit_status(&self) -> io::Result<Option<ServiceExitStatus>> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("child reap ticket lock is poisoned"))?;
        match *state {
            ChildReapState::Unobserved => Ok(None),
            ChildReapState::Reaped(status) => Ok(Some(status)),
            ChildReapState::Attempted | ChildReapState::Unknown => Err(io::Error::other(
                "child reap outcome is unknown; a second wait is forbidden",
            )),
        }
    }

    fn set_state(&self, next: ChildReapState) -> io::Result<()> {
        *self
            .state
            .lock()
            .map_err(|_| io::Error::other("child reap ticket lock is poisoned"))? = next;
        Ok(())
    }
}

/// Every mutating reap must pass through this interface while the global
/// coordinator lock is held. The first observation is always non-consuming.
pub(super) trait ReapKernel {
    fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>>;
    fn observe_tracked_exit_nonconsuming(
        &mut self,
        process_id: u32,
        pidfd: i32,
    ) -> io::Result<bool>;
    fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>>;
}

#[derive(Default)]
pub(super) struct Pid1ChildReaper {
    tracked: HashMap<u32, ReapTicket>,
    faulted: bool,
}

impl Pid1ChildReaper {
    pub(super) fn reserve_ticket(&mut self) -> io::Result<ReapTicketReservation> {
        if self.faulted {
            return Err(io::Error::other("PID-1 child reaper is fail-closed"));
        }
        self.tracked
            .try_reserve(1)
            .map_err(|_| io::Error::other("PID-1 child reaper registry capacity is unavailable"))?;
        Ok(ReapTicketReservation {
            state: Arc::new(Mutex::new(ChildReapState::Unobserved)),
        })
    }

    pub(super) fn register(&mut self, ticket: ReapTicket) -> io::Result<()> {
        if self.faulted {
            return Err(io::Error::other("PID-1 child reaper is fail-closed"));
        }
        if self.tracked.contains_key(&ticket.process_id) {
            self.faulted = true;
            let _ = ticket.set_state(ChildReapState::Unknown);
            if let Some(existing) = self.tracked.get(&ticket.process_id) {
                let _ = existing.set_state(ChildReapState::Unknown);
            }
            return Err(io::Error::other("duplicate tracked child PID"));
        }
        if ticket.process_id <= 1 || ticket.pidfd < 0 {
            self.faulted = true;
            return Err(io::Error::other("tracked child identity or pidfd is invalid"));
        }
        if ticket.identity.as_ref().is_some_and(|identity| {
            identity.process_id() != ticket.process_id
        }) {
            self.faulted = true;
            return Err(io::Error::other("tracked child identity/PID mismatch"));
        }
        #[cfg(not(test))]
        if ticket.identity.is_none() {
            self.faulted = true;
            return Err(io::Error::other("tracked child lacks clone3 identity proof"));
        }
        if ticket.exit_status().is_err() {
            self.faulted = true;
            return Err(io::Error::other("tracked child state is already unknown"));
        }
        self.tracked.insert(ticket.process_id, ticket);
        Ok(())
    }

    /// Drain all currently exited children. Tracked status is routed to the
    /// matching ticket; an untracked child is an orphan owned by this same
    /// reaper. Any ambiguous consuming wait permanently stops further waits.
    pub(super) fn drain(&mut self, kernel: &mut impl ReapKernel) -> io::Result<usize> {
        if self.faulted {
            return Err(io::Error::other("PID-1 child reaper is fail-closed"));
        }
        let mut reaped_count = 0;
        loop {
            let process_id = match kernel.observe_any_exit_nonconsuming() {
                Ok(Some(process_id)) if process_id > 1 => process_id,
                Ok(Some(_)) => {
                    self.faulted = true;
                    return Err(io::Error::other("P_ALL returned an invalid child PID"));
                }
                Ok(None) => return Ok(reaped_count),
                Err(error) => {
                    self.faulted = true;
                    return Err(error);
                }
            };

            let ticket = self.tracked.get(&process_id).cloned();
            if let Some(ticket) = ticket.as_ref() {
                if ticket.process_id() != process_id {
                    self.faulted = true;
                    let _ = ticket.set_state(ChildReapState::Unknown);
                    return Err(io::Error::other("tracked child identity mismatch"));
                }
                match kernel.observe_tracked_exit_nonconsuming(
                    process_id,
                    ticket.pidfd(),
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.faulted = true;
                        let _ = ticket.set_state(ChildReapState::Unknown);
                        return Err(io::Error::other(
                            "P_ALL exit did not match the tracked pidfd",
                        ));
                    }
                    Err(error) => {
                        self.faulted = true;
                        let _ = ticket.set_state(ChildReapState::Unknown);
                        return Err(error);
                    }
                }
                let state = ticket
                    .state
                    .lock()
                    .map_err(|_| io::Error::other("child reap ticket lock is poisoned"))?;
                if *state != ChildReapState::Unobserved {
                    self.faulted = true;
                    drop(state);
                    let _ = ticket.set_state(ChildReapState::Unknown);
                    return Err(io::Error::other("tracked wait already has an owner"));
                }
                drop(state);
                ticket.set_state(ChildReapState::Attempted)?;
            }

            // Mark the whole reaper faulted before a consuming call. If the
            // syscall wrapper panics or its result is ambiguous, no later
            // caller may repeat wait4 for this PID or a different child.
            self.faulted = true;
            match kernel.reap_exact(process_id) {
                Ok(Some(status)) => {
                    if let Some(ticket) = ticket.as_ref() {
                        ticket.set_state(ChildReapState::Reaped(status))?;
                    }
                    self.tracked.remove(&process_id);
                    reaped_count += 1;
                    self.faulted = false;
                }
                Ok(None) => {
                    if let Some(ticket) = ticket.as_ref() {
                        let _ = ticket.set_state(ChildReapState::Unknown);
                    }
                    return Err(io::Error::other(
                        "child disappeared between observation and exact reap",
                    ));
                }
                Err(error) => {
                    if let Some(ticket) = ticket.as_ref() {
                        let _ = ticket.set_state(ChildReapState::Unknown);
                    }
                    return Err(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Event {
        ObserveAny,
        ObserveTracked(u32, i32),
        ReapExact(u32),
    }

    struct FakeKernel {
        observations: VecDeque<Result<Option<u32>, &'static str>>,
        tracked_observations: VecDeque<Result<bool, &'static str>>,
        reaps: VecDeque<Result<Option<ServiceExitStatus>, &'static str>>,
        events: Vec<Event>,
    }

    impl FakeKernel {
        fn new(
            observations: impl IntoIterator<Item = Result<Option<u32>, &'static str>>,
            tracked_observations: impl IntoIterator<Item = Result<bool, &'static str>>,
            reaps: impl IntoIterator<Item = Result<Option<ServiceExitStatus>, &'static str>>,
        ) -> Self {
            Self {
                observations: observations.into_iter().collect(),
                tracked_observations: tracked_observations.into_iter().collect(),
                reaps: reaps.into_iter().collect(),
                events: Vec::new(),
            }
        }

        fn scripted<T>(result: Result<T, &'static str>) -> io::Result<T> {
            result.map_err(io::Error::other)
        }
    }

    impl ReapKernel for FakeKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            self.events.push(Event::ObserveAny);
            Self::scripted(self.observations.pop_front().unwrap_or(Ok(None)))
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            self.events
                .push(Event::ObserveTracked(process_id, pidfd));
            Self::scripted(self.tracked_observations.pop_front().unwrap_or(Ok(false)))
        }

        fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>> {
            self.events.push(Event::ReapExact(process_id));
            Self::scripted(self.reaps.pop_front().unwrap_or(Ok(None)))
        }
    }

    fn status(exit_code: i32) -> ServiceExitStatus {
        ServiceExitStatus {
            exit_code: Some(exit_code),
            signal: None,
        }
    }

    #[test]
    fn tracked_and_orphan_children_share_one_exact_reap_owner() {
        let mut reaper = Pid1ChildReaper::default();
        let tracked = ReapTicket::new(41, 9041).unwrap();
        reaper.register(tracked.clone()).unwrap();
        let mut kernel = FakeKernel::new(
            [Ok(Some(41)), Ok(Some(99)), Ok(None)],
            [Ok(true)],
            [Ok(Some(status(7))), Ok(Some(status(2)))],
        );

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 2);
        assert_eq!(tracked.exit_status().unwrap(), Some(status(7)));
        assert_eq!(
            kernel.events,
            [
                Event::ObserveAny,
                Event::ObserveTracked(41, 9041),
                Event::ReapExact(41),
                Event::ObserveAny,
                Event::ReapExact(99),
                Event::ObserveAny,
            ]
        );
        assert_eq!(reaper.drain(&mut kernel).unwrap(), 0);
        assert_eq!(kernel.events.len(), 7);
    }

    #[test]
    fn out_of_order_tracked_exits_route_to_their_own_pidfd_tickets() {
        let mut reaper = Pid1ChildReaper::default();
        let first = ReapTicket::new(41, 9041).unwrap();
        let second = ReapTicket::new(42, 9042).unwrap();
        reaper.register(first.clone()).unwrap();
        reaper.register(second.clone()).unwrap();
        let mut kernel = FakeKernel::new(
            [Ok(Some(42)), Ok(Some(41)), Ok(None)],
            [Ok(true), Ok(true)],
            [Ok(Some(status(12))), Ok(Some(status(11)))],
        );

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 2);
        assert_eq!(first.exit_status().unwrap(), Some(status(11)));
        assert_eq!(second.exit_status().unwrap(), Some(status(12)));
    }

    #[test]
    fn pidfd_mismatch_fails_closed_without_consuming_any_status() {
        let mut reaper = Pid1ChildReaper::default();
        let ticket = ReapTicket::new(41, 9041).unwrap();
        reaper.register(ticket.clone()).unwrap();
        let mut kernel = FakeKernel::new([Ok(Some(41))], [Ok(false)], []);

        assert!(reaper.drain(&mut kernel).is_err());
        assert!(ticket.exit_status().is_err());
        assert_eq!(kernel.events, [Event::ObserveAny, Event::ObserveTracked(41, 9041)]);
        assert!(reaper.drain(&mut kernel).is_err());
        assert_eq!(kernel.events.len(), 2);
    }

    #[test]
    fn ambiguous_tracked_reap_is_attempted_once_and_never_retried() {
        let mut reaper = Pid1ChildReaper::default();
        let ticket = ReapTicket::new(41, 9041).unwrap();
        reaper.register(ticket.clone()).unwrap();
        let mut kernel = FakeKernel::new(
            [Ok(Some(41))],
            [Ok(true)],
            [Err("wait4 outcome is ambiguous")],
        );

        assert!(reaper.drain(&mut kernel).is_err());
        assert!(ticket.exit_status().is_err());
        assert!(reaper.drain(&mut kernel).is_err());
        assert_eq!(
            kernel.events,
            [
                Event::ObserveAny,
                Event::ObserveTracked(41, 9041),
                Event::ReapExact(41),
            ]
        );
    }

    #[test]
    fn duplicate_pid_registration_disables_the_single_reaper() {
        let mut reaper = Pid1ChildReaper::default();
        let first = ReapTicket::new(41, 9041).unwrap();
        let second = ReapTicket::new(41, 9042).unwrap();
        reaper.register(first.clone()).unwrap();
        assert!(reaper.register(second.clone()).is_err());
        assert!(first.exit_status().is_err());
        assert!(second.exit_status().is_err());
        let mut kernel = FakeKernel::new([Ok(Some(41))], [Ok(true)], [Ok(Some(status(7)))]);
        assert!(reaper.drain(&mut kernel).is_err());
        assert!(kernel.events.is_empty());
    }
}
