//! Single-owner child wait coordinator shared by ProcessLifecycle polling and
//! PID-1's SIGCHLD wait. Linux supplies P_ALL WNOWAIT observation and exact
//! PID reaping; tracked tickets are verified through their retained pidfd.

use super::ServiceExitStatus;
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
use std::collections::HashMap;
use std::io;
#[cfg(test)]
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(test)]
use std::sync::Weak;
use std::sync::{Arc, Mutex};

const ECHILD_RAW_OS_ERROR: i32 = 10;

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
    pidfd_owner: Option<Arc<Mutex<SpawnedProcessPidfd>>>,
    identity: Option<SpawnedProcessIdentity>,
    cleanup_only: bool,
    state: Arc<Mutex<ChildReapState>>,
    #[cfg(test)]
    terminal_close_observer: Arc<Mutex<Option<Sender<ServiceExitStatus>>>>,
}

/// All fallible allocation/capacity growth is performed before clone3. Binding
/// the syscall result to this reservation moves only already-owned values.
pub(super) struct ReapTicketReservation {
    state: Arc<Mutex<ChildReapState>>,
}

impl ReapTicketReservation {
    pub(super) fn bind(self, identity: SpawnedProcessIdentity, pidfd: i32) -> ReapTicket {
        let process_id = identity.process_id();
        ReapTicket {
            process_id,
            pidfd,
            pidfd_owner: None,
            identity: Some(identity),
            cleanup_only: false,
            state: self.state,
            #[cfg(test)]
            terminal_close_observer: Arc::new(Mutex::new(None)),
        }
    }

    /// Bind a candidate lifecycle ticket to an owned pidfd. The registry's
    /// cloned ticket keeps this descriptor live if the lifecycle owner drops.
    pub(super) fn bind_owned(self, pidfd_owner: SpawnedProcessPidfd) -> ReapTicket {
        let identity = pidfd_owner.identity();
        let process_id = identity.process_id();
        let pidfd = pidfd_owner.as_raw_fd().unwrap_or(-1);
        ReapTicket {
            process_id,
            pidfd,
            pidfd_owner: Some(Arc::new(Mutex::new(pidfd_owner))),
            identity: Some(identity),
            cleanup_only: false,
            state: self.state,
            #[cfg(test)]
            terminal_close_observer: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn bind_unverified(self, process_id: u32) -> ReapTicket {
        let state = self.state;
        if let Ok(mut current) = state.lock() {
            *current = ChildReapState::Unobserved;
        }
        ReapTicket {
            process_id,
            pidfd: -1,
            pidfd_owner: None,
            identity: None,
            cleanup_only: true,
            state,
            #[cfg(test)]
            terminal_close_observer: Arc::new(Mutex::new(None)),
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
            pidfd_owner: None,
            identity: None,
            cleanup_only: false,
            state: Arc::new(Mutex::new(ChildReapState::Unobserved)),
            #[cfg(test)]
            terminal_close_observer: Arc::new(Mutex::new(None)),
        })
    }

    pub(super) fn unverified(process_id: u32) -> Self {
        Self {
            process_id,
            pidfd: -1,
            pidfd_owner: None,
            identity: None,
            cleanup_only: false,
            state: Arc::new(Mutex::new(ChildReapState::Unknown)),
            #[cfg(test)]
            terminal_close_observer: Arc::new(Mutex::new(None)),
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

    #[cfg(test)]
    pub(super) fn owns_live_pidfd(&self) -> bool {
        self.pidfd_owner
            .as_ref()
            .and_then(|owner| owner.lock().ok())
            .is_some_and(|owner| owner.as_raw_fd().is_ok())
    }

    /// Attach a non-owning observation channel for an exact reap followed by
    /// successful ticket-pidfd close. The returned Weak does not keep the
    /// registry's OwnedFd alive.
    #[cfg(test)]
    pub(super) fn observe_terminal_close(&mut self) -> ReapTicketCloseObserver {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut observer) = self.terminal_close_observer.lock() {
            *observer = Some(sender);
        }
        ReapTicketCloseObserver {
            pidfd_owner: self.pidfd_owner.as_ref().map(Arc::downgrade),
            terminal_after_close: receiver,
        }
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

    fn close_pidfd_after_reap_once(&self) -> io::Result<()> {
        let terminal_status = self.exit_status()?;
        if terminal_status.is_none() {
            return Err(io::Error::other(
                "cannot close tracked pidfd before exact child reap",
            ));
        }
        let Some(owner) = &self.pidfd_owner else {
            return Ok(());
        };
        let result = owner
            .lock()
            .map_err(|_| io::Error::other("tracked pidfd owner lock is poisoned"))?
            .close_now();
        #[cfg(test)]
        if result.is_ok() {
            let observer = self
                .terminal_close_observer
                .lock()
                .ok()
                .and_then(|observer| observer.clone());
            if let (Some(status), Some(observer)) = (terminal_status, observer) {
                let _ = observer.send(status);
            }
        }
        result
    }
}

/// Test observation deliberately holds no strong reference to the pidfd token.
#[cfg(test)]
pub(super) struct ReapTicketCloseObserver {
    pidfd_owner: Option<Weak<Mutex<SpawnedProcessPidfd>>>,
    terminal_after_close: Receiver<ServiceExitStatus>,
}

#[cfg(test)]
impl ReapTicketCloseObserver {
    pub(super) fn pidfd_owner_strong_count(&self) -> Option<usize> {
        self.pidfd_owner.as_ref().map(Weak::strong_count)
    }

    pub(super) fn pidfd_owner_is_live(&self) -> bool {
        self.pidfd_owner
            .as_ref()
            .and_then(Weak::upgrade)
            .and_then(|owner| owner.lock().ok().map(|owner| owner.as_raw_fd().is_ok()))
            .unwrap_or(false)
    }

    pub(super) fn try_terminal_after_close(&self) -> io::Result<Option<ServiceExitStatus>> {
        match self.terminal_after_close.try_recv() {
            Ok(status) => Ok(Some(status)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(io::Error::other(
                "reaper ticket disappeared before exact close notification",
            )),
        }
    }
}

/// Every mutating reap must pass through this interface while the global
/// coordinator lock is held. The first observation is always non-consuming.
pub(super) trait ReapKernel {
    fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>>;
    fn observe_cleanup_exit_nonconsuming(&mut self, process_id: u32) -> io::Result<bool>;
    fn observe_tracked_exit_nonconsuming(
        &mut self,
        process_id: u32,
        pidfd: i32,
    ) -> io::Result<bool>;
    fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>>;
}

/// Serialize every caller that may consume a child status through one owner.
/// PID-1's idle wait and the tracked ProcessLifecycle poll both call this
/// helper, so neither can race a second wait against the other's observation.
pub(super) fn drain_serialized(
    owner: &Mutex<Pid1ChildReaper>,
    kernel: &mut impl ReapKernel,
) -> io::Result<usize> {
    let mut owner = owner
        .lock()
        .map_err(|_| io::Error::other("PID-1 child reaper lock is poisoned"))?;
    owner.drain(kernel)
}

#[derive(Default)]
pub(super) struct Pid1ChildReaper {
    tracked: HashMap<u32, ReapTicket>,
    // Keep a failed-to-register child token alive after a registry invariant
    // failure. The coordinator remains faulted, so this is deliberate
    // quarantine rather than a retryable reap entry.
    retained_unknown: Vec<ReapTicket>,
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
        self.retained_unknown.try_reserve(1).map_err(|_| {
            io::Error::other("PID-1 child reaper quarantine capacity is unavailable")
        })?;
        Ok(ReapTicketReservation {
            state: Arc::new(Mutex::new(ChildReapState::Unobserved)),
        })
    }

    /// Preserve the ticket when registration fails after clone3 has created a
    /// child. Its wait outcome is treated as unknown and the coordinator stays
    /// faulted; dropping the token before exact termination would be unsafe.
    pub(super) fn retain_failed_ticket(&mut self, ticket: ReapTicket) {
        self.faulted = true;
        let _ = ticket.set_state(ChildReapState::Unknown);
        self.retained_unknown.push(ticket);
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
        if ticket.process_id <= 1
            || (ticket.pidfd < 0 && !ticket.cleanup_only)
            || (ticket.cleanup_only && (ticket.pidfd >= 0 || ticket.identity.is_some()))
        {
            self.faulted = true;
            return Err(io::Error::other(
                "tracked child identity or pidfd is invalid",
            ));
        }
        if ticket
            .identity
            .as_ref()
            .is_some_and(|identity| identity.process_id() != ticket.process_id)
        {
            self.faulted = true;
            return Err(io::Error::other("tracked child identity/PID mismatch"));
        }
        #[cfg(not(test))]
        if ticket.identity.is_none() && !ticket.cleanup_only {
            self.faulted = true;
            return Err(io::Error::other(
                "tracked child lacks clone3 identity proof",
            ));
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
                Ok(None) => {
                    if self.reap_one_tracked_observed_child(kernel)? {
                        reaped_count += 1;
                        continue;
                    }
                    return Ok(reaped_count);
                }
                Err(error) if error.raw_os_error() == Some(ECHILD_RAW_OS_ERROR) => {
                    // Some kernels do not surface an exited CLONE_NEWPID
                    // namespace init through P_ALL. Inspect every tracked
                    // child through its pidfd or exact child PID first.
                    if self.reap_one_tracked_observed_child(kernel)? {
                        reaped_count += 1;
                        continue;
                    }
                    return Ok(reaped_count);
                }
                Err(error) => {
                    self.faulted = true;
                    return Err(error);
                }
            };

            let ticket = self.tracked.get(&process_id).cloned();
            self.reap_observed_child(process_id, ticket, false, kernel)?;
            reaped_count += 1;
        }
    }

    fn reap_one_tracked_observed_child(
        &mut self,
        kernel: &mut impl ReapKernel,
    ) -> io::Result<bool> {
        let mut observed_ticket = None;
        for (process_id, ticket) in &self.tracked {
            if ticket.process_id() != *process_id {
                self.faulted = true;
                let _ = ticket.set_state(ChildReapState::Unknown);
                return Err(io::Error::other("tracked child identity mismatch"));
            }
            let observation = if ticket.cleanup_only {
                kernel.observe_cleanup_exit_nonconsuming(*process_id)
            } else {
                kernel.observe_tracked_exit_nonconsuming(*process_id, ticket.pidfd())
            };
            match observation {
                Ok(true) => {
                    observed_ticket = Some((*process_id, ticket.clone()));
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    self.faulted = true;
                    let _ = ticket.set_state(ChildReapState::Unknown);
                    return Err(error);
                }
            }
        }
        let Some((process_id, ticket)) = observed_ticket else {
            return Ok(false);
        };
        self.reap_observed_child(process_id, Some(ticket), true, kernel)?;
        Ok(true)
    }

    fn reap_observed_child(
        &mut self,
        process_id: u32,
        ticket: Option<ReapTicket>,
        pidfd_already_verified: bool,
        kernel: &mut impl ReapKernel,
    ) -> io::Result<()> {
        if let Some(ticket) = ticket.as_ref() {
            if ticket.process_id() != process_id {
                self.faulted = true;
                let _ = ticket.set_state(ChildReapState::Unknown);
                return Err(io::Error::other("tracked child identity mismatch"));
            }
            if !ticket.cleanup_only && !pidfd_already_verified {
                match kernel.observe_tracked_exit_nonconsuming(process_id, ticket.pidfd()) {
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
            }
            if ticket.exit_status().is_err() {
                self.faulted = true;
                let _ = ticket.set_state(ChildReapState::Unknown);
                return Err(io::Error::other("tracked wait already has an owner"));
            }
            if let Err(error) = ticket.set_state(ChildReapState::Attempted) {
                self.faulted = true;
                return Err(error);
            }
        }

        // Mark the whole reaper faulted before a consuming call. If the
        // syscall wrapper panics or its result is ambiguous, no later caller
        // may repeat wait4 for this PID or a different child.
        self.faulted = true;
        match kernel.reap_exact(process_id) {
            Ok(Some(status)) => {
                if let Some(ticket) = ticket.as_ref() {
                    ticket.set_state(ChildReapState::Reaped(status))?;
                    ticket.close_pidfd_after_reap_once()?;
                }
                self.tracked.remove(&process_id);
                self.faulted = false;
                Ok(())
            }
            Ok(None) => {
                if let Some(ticket) = ticket.as_ref() {
                    let _ = ticket.set_state(ChildReapState::Unknown);
                }
                Err(io::Error::other(
                    "child disappeared between observation and exact reap",
                ))
            }
            Err(error) => {
                if let Some(ticket) = ticket.as_ref() {
                    let _ = ticket.set_state(ChildReapState::Unknown);
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Barrier};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Event {
        ObserveAny,
        ObserveCleanup(u32),
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
            result.map_err(|error| {
                if error == "ECHILD" {
                    io::Error::from_raw_os_error(ECHILD_RAW_OS_ERROR)
                } else {
                    io::Error::other(error)
                }
            })
        }
    }

    impl ReapKernel for FakeKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            self.events.push(Event::ObserveAny);
            Self::scripted(self.observations.pop_front().unwrap_or(Ok(None)))
        }

        fn observe_cleanup_exit_nonconsuming(&mut self, process_id: u32) -> io::Result<bool> {
            self.events.push(Event::ObserveCleanup(process_id));
            Self::scripted(self.tracked_observations.pop_front().unwrap_or(Ok(false)))
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            self.events.push(Event::ObserveTracked(process_id, pidfd));
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
    fn p_all_echild_scans_tracked_pidfds_and_routes_the_exact_exit_status() {
        let mut reaper = Pid1ChildReaper::default();
        let tracked = ReapTicket::new(41, 9041).unwrap();
        reaper.register(tracked.clone()).unwrap();
        let mut kernel = FakeKernel::new(
            [Err("ECHILD"), Ok(None)],
            [Ok(true)],
            [Ok(Some(status(17)))],
        );

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 1);
        assert_eq!(tracked.exit_status().unwrap(), Some(status(17)));
        assert_eq!(
            kernel.events,
            [
                Event::ObserveAny,
                Event::ObserveTracked(41, 9041),
                Event::ReapExact(41),
                Event::ObserveAny,
            ]
        );
    }

    #[test]
    fn p_all_none_scans_tracked_pidfds_and_routes_the_exact_exit_status() {
        let mut reaper = Pid1ChildReaper::default();
        let tracked = ReapTicket::new(41, 9041).unwrap();
        reaper.register(tracked.clone()).unwrap();
        let mut kernel = FakeKernel::new([Ok(None), Ok(None)], [Ok(true)], [Ok(Some(status(23)))]);

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 1);
        assert_eq!(tracked.exit_status().unwrap(), Some(status(23)));
        assert_eq!(
            kernel.events,
            [
                Event::ObserveAny,
                Event::ObserveTracked(41, 9041),
                Event::ReapExact(41),
                Event::ObserveAny,
            ]
        );
    }

    #[test]
    fn p_all_echild_is_empty_only_after_live_tracked_pidfds_are_verified() {
        let mut reaper = Pid1ChildReaper::default();
        let tracked = ReapTicket::new(41, 9041).unwrap();
        reaper.register(tracked.clone()).unwrap();
        let mut kernel = FakeKernel::new([Err("ECHILD")], [Ok(false)], []);

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 0);
        assert_eq!(tracked.exit_status().unwrap(), None);
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ObserveTracked(41, 9041)]
        );
        assert_eq!(reaper.drain(&mut kernel).unwrap(), 0);
        assert_eq!(kernel.events.len(), 4);
    }

    #[test]
    fn p_all_echild_with_lost_pidfd_observation_marks_the_ticket_unknown() {
        let mut reaper = Pid1ChildReaper::default();
        let tracked = ReapTicket::new(41, 9041).unwrap();
        reaper.register(tracked.clone()).unwrap();
        let mut kernel = FakeKernel::new([Err("ECHILD")], [Err("ECHILD")], []);

        assert!(reaper.drain(&mut kernel).is_err());
        assert!(tracked.exit_status().is_err());
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ObserveTracked(41, 9041)]
        );
        assert!(reaper.drain(&mut kernel).is_err());
        assert_eq!(kernel.events.len(), 2);
    }

    #[test]
    fn p_all_echild_checks_exact_pid_for_a_pidfd_less_cleanup_child() {
        let mut reaper = Pid1ChildReaper::default();
        let cleanup = reaper.reserve_ticket().unwrap().bind_unverified(45);
        reaper.register(cleanup.clone()).unwrap();
        let mut kernel = FakeKernel::new([Err("ECHILD")], [Ok(false)], []);

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 0);
        assert_eq!(cleanup.exit_status().unwrap(), None);
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ObserveCleanup(45)]
        );
    }

    #[test]
    fn p_all_echild_with_unwaitable_pidfd_less_cleanup_child_fails_closed() {
        let mut reaper = Pid1ChildReaper::default();
        let cleanup = reaper.reserve_ticket().unwrap().bind_unverified(45);
        reaper.register(cleanup.clone()).unwrap();
        let mut kernel = FakeKernel::new([Err("ECHILD")], [Err("ECHILD")], []);

        assert!(reaper.drain(&mut kernel).is_err());
        assert!(cleanup.exit_status().is_err());
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ObserveCleanup(45)]
        );
        assert!(reaper.drain(&mut kernel).is_err());
        assert_eq!(kernel.events.len(), 2);
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
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ObserveTracked(41, 9041)]
        );
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

    #[test]
    fn pidfd_less_direct_child_cleanup_uses_the_shared_exact_reap_owner() {
        let mut reaper = Pid1ChildReaper::default();
        let ticket = reaper.reserve_ticket().unwrap().bind_unverified(45);
        reaper.register(ticket.clone()).unwrap();
        let mut kernel = FakeKernel::new([Ok(Some(45)), Ok(None)], [], [Ok(Some(status(125)))]);

        assert_eq!(reaper.drain(&mut kernel).unwrap(), 1);
        assert_eq!(ticket.exit_status().unwrap(), Some(status(125)));
        assert_eq!(
            kernel.events,
            [Event::ObserveAny, Event::ReapExact(45), Event::ObserveAny]
        );
    }

    #[derive(Default)]
    struct SharedWaitState {
        reaped: bool,
        exact_reaps: usize,
    }

    struct SharedRaceKernel(Arc<Mutex<SharedWaitState>>);

    impl ReapKernel for SharedRaceKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            let state = self
                .0
                .lock()
                .map_err(|_| io::Error::other("shared race state lock is poisoned"))?;
            Ok((!state.reaped).then_some(41))
        }

        fn observe_cleanup_exit_nonconsuming(&mut self, _process_id: u32) -> io::Result<bool> {
            Ok(false)
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            let state = self
                .0
                .lock()
                .map_err(|_| io::Error::other("shared race state lock is poisoned"))?;
            Ok(process_id == 41 && pidfd == 9041 && !state.reaped)
        }

        fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>> {
            let mut state = self
                .0
                .lock()
                .map_err(|_| io::Error::other("shared race state lock is poisoned"))?;
            if process_id != 41 || state.reaped {
                return Err(io::Error::other("the child status was consumed twice"));
            }
            state.reaped = true;
            state.exact_reaps += 1;
            Ok(Some(status(7)))
        }
    }

    #[test]
    fn lifecycle_poll_and_pid1_idle_wait_serialize_one_child_consumption() {
        let owner = Arc::new(Mutex::new(Pid1ChildReaper::default()));
        let ticket = ReapTicket::new(41, 9041).unwrap();
        owner.lock().unwrap().register(ticket.clone()).unwrap();

        let shared_state = Arc::new(Mutex::new(SharedWaitState::default()));
        let start = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let owner = Arc::clone(&owner);
            let shared_state = Arc::clone(&shared_state);
            let start = Arc::clone(&start);
            workers.push(std::thread::spawn(move || {
                start.wait();
                drain_serialized(&owner, &mut SharedRaceKernel(shared_state))
            }));
        }
        start.wait();

        let drained = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .sum::<usize>();
        assert_eq!(drained, 1);
        assert_eq!(ticket.exit_status().unwrap(), Some(status(7)));
        let state = shared_state.lock().unwrap();
        assert!(state.reaped);
        assert_eq!(state.exact_reaps, 1);
    }
}
