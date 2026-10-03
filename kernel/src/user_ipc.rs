//! Bounded user ABI for the bootstrap Channel path.
//!
//! The manager keeps one handle table per kernel Process (ADR 0043): the
//! bootstrap `nagi-init` process (PID 1) and at most one spawned isolated
//! process. Handle values are process-local, and every received message
//! carries the sender's kernel Process ID independent of its payload. Mapping
//! that ID to an application/session belongs to the trusted Supervisor.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use nagi_abi::{
    ChannelEndpoints, ChannelReceiveResult, ChannelSendRequest, MAX_CHANNEL_INLINE_PAYLOAD,
    MAX_CHANNEL_TRANSFER_HANDLES,
};

use crate::handles::{Handle, HandleError, ObjectId, ObjectKind, ObjectRegistry, Process, Rights};
use crate::ipc::{
    ChannelError, ChannelPair, MessageHeader, OutgoingMessage, Signals, WaitError, WaitRegistry,
    Waiter, WokenWaiters, MAX_INLINE_PAYLOAD, MAX_TRANSFER_HANDLES,
};

pub const BOOTSTRAP_PROCESS_ID: u32 = 1;
/// Number of kernel Processes the bootstrap IPC manager can hold: init plus
/// the concurrent isolated processes (ADR 0050).
pub const MAX_USER_PROCESSES: usize = 1 + crate::process_exit::MAX_LIVE_PROCESSES;
const CHANNEL_CAPACITY: usize = 16;
const HANDLE_CAPACITY: usize = 64;
const OBJECT_CAPACITY: usize = 128;

const _: [(); MAX_INLINE_PAYLOAD] = [(); MAX_CHANNEL_INLINE_PAYLOAD];
const _: [(); MAX_TRANSFER_HANDLES] = [(); MAX_CHANNEL_TRANSFER_HANDLES];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserIpcError {
    Capacity,
    InvalidRequest,
    InvalidEndpoint,
    InvalidProcess,
    /// No process handle or queued transfer can reach the peer endpoint, so
    /// a message could never be received.
    PeerClosed,
    Handle(HandleError),
    Channel(ChannelError),
    Wait(WaitError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelWaitOutcome {
    Readable,
    Blocked,
}

impl From<HandleError> for UserIpcError {
    fn from(error: HandleError) -> Self {
        Self::Handle(error)
    }
}

impl From<ChannelError> for UserIpcError {
    fn from(error: ChannelError) -> Self {
        Self::Channel(error)
    }
}

struct ChannelSlot {
    pair: Option<ChannelPair>,
    object: Option<ObjectId>,
    endpoint_a: Option<ObjectId>,
    endpoint_b: Option<ObjectId>,
}

impl ChannelSlot {
    const fn new() -> Self {
        Self {
            pair: None,
            object: None,
            endpoint_a: None,
            endpoint_b: None,
        }
    }

    fn contains_endpoint(&self, endpoint: ObjectId) -> bool {
        self.endpoint_a == Some(endpoint) || self.endpoint_b == Some(endpoint)
    }

    fn clear(&mut self) {
        self.pair = None;
        self.object = None;
        self.endpoint_a = None;
        self.endpoint_b = None;
    }
}

pub struct UserIpcState {
    registry: ObjectRegistry<OBJECT_CAPACITY>,
    processes: [Option<Process<HANDLE_CAPACITY>>; MAX_USER_PROCESSES],
    channels: [ChannelSlot; CHANNEL_CAPACITY],
    waiters: WaitRegistry,
}

impl UserIpcState {
    pub const fn new() -> Self {
        Self {
            registry: ObjectRegistry::new(),
            processes: [const { None }; MAX_USER_PROCESSES],
            channels: [const { ChannelSlot::new() }; CHANNEL_CAPACITY],
            waiters: WaitRegistry::new(),
        }
    }

    fn initialize(&mut self) -> Result<(), UserIpcError> {
        if self.processes[0].is_some() {
            return Ok(());
        }
        let address_space = self.registry.create(ObjectKind::AddressSpace)?;
        self.processes[0] = Some(Process::new(BOOTSTRAP_PROCESS_ID, address_space));
        Ok(())
    }

    /// Slot 0 always holds init (PID 1). Isolated processes occupy the
    /// remaining slots under kernel-assigned, never-reused IDs (ADR 0048).
    fn process_index(&self, process_id: u32) -> Result<usize, UserIpcError> {
        if process_id == BOOTSTRAP_PROCESS_ID {
            return Ok(0);
        }
        self.processes
            .iter()
            .position(|slot| {
                slot.as_ref()
                    .is_some_and(|process| process.id() == process_id)
            })
            .filter(|index| *index != 0)
            .ok_or(UserIpcError::InvalidProcess)
    }

    fn process_mut(
        &mut self,
        process_id: u32,
    ) -> Result<&mut Process<HANDLE_CAPACITY>, UserIpcError> {
        self.initialize()?;
        let index = self.process_index(process_id)?;
        self.processes[index]
            .as_mut()
            .ok_or(UserIpcError::InvalidProcess)
    }

    fn process_ref(&self, process_id: u32) -> Result<&Process<HANDLE_CAPACITY>, UserIpcError> {
        let index = self.process_index(process_id)?;
        self.processes[index]
            .as_ref()
            .ok_or(UserIpcError::InvalidProcess)
    }

    /// Register a newly spawned process and move one Channel endpoint from
    /// `parent` into it. Only init may spawn. The moved capability keeps at
    /// most `rights`, which must be a subset of the parent's rights and must
    /// retain `TRANSFER` only if the parent granted it.
    ///
    /// Returns the endpoint's raw handle value in the child's table.
    pub fn register_spawned_process(
        &mut self,
        parent: u32,
        child: u32,
        endpoint: u64,
        rights: Rights,
    ) -> Result<u64, UserIpcError> {
        self.initialize()?;
        if parent != BOOTSTRAP_PROCESS_ID || child <= BOOTSTRAP_PROCESS_ID {
            return Err(UserIpcError::InvalidProcess);
        }
        if self.process_index(child).is_ok() {
            return Err(UserIpcError::InvalidProcess);
        }
        let Some(child_index) =
            (1..MAX_USER_PROCESSES).find(|index| self.processes[*index].is_none())
        else {
            return Err(UserIpcError::Capacity);
        };
        let handle = Handle::from_raw(endpoint);
        self.locate_endpoint(parent, handle)?;
        let address_space = self.registry.create(ObjectKind::AddressSpace)?;
        let mut child_process = Process::new(child, address_space);
        let parent_index = self.process_index(parent)?;
        let token = {
            let parent_process = self.processes[parent_index]
                .as_mut()
                .ok_or(UserIpcError::InvalidProcess)?;
            match parent_process
                .handles
                .begin_move(&self.registry, handle, rights)
            {
                Ok(token) => token,
                Err(error) => {
                    let _ = self.registry.release(address_space);
                    return Err(error.into());
                }
            }
        };
        let mut token = Some(token);
        let installed = child_process
            .handles
            .install_token(&mut self.registry, &mut token);
        match installed {
            Ok(child_handle) => {
                self.processes[child_index] = Some(child_process);
                Ok(child_handle.raw())
            }
            Err(error) => {
                // The parent's slot generation already advanced; a failed
                // install leaves the capability unreachable, so release the
                // reference it held rather than restoring a stale handle.
                if let Some(token) = token.take() {
                    let _ = self.registry.release(token.capability.object);
                }
                let _ = self.registry.release(address_space);
                let _ = self.collect_unreachable_channels();
                Err(error.into())
            }
        }
    }

    /// Close every handle of an exited spawned process, release its address
    /// space object, and reclaim Channels no surviving process can reach.
    pub fn exit_process(&mut self, process_id: u32) -> Result<(), UserIpcError> {
        if process_id == BOOTSTRAP_PROCESS_ID {
            return Err(UserIpcError::InvalidProcess);
        }
        let index = self.process_index(process_id)?;
        let Some(mut process) = self.processes[index].take() else {
            return Err(UserIpcError::InvalidProcess);
        };
        process.handles.close_all(&mut self.registry);
        let _ = self.registry.release(process.address_space());
        self.collect_unreachable_channels()
    }

    pub fn create_pair(&mut self, process_id: u32) -> Result<ChannelEndpoints, UserIpcError> {
        self.process_mut(process_id)?;
        let Some(slot_index) = self.channels.iter().position(|slot| slot.pair.is_none()) else {
            return Err(UserIpcError::Capacity);
        };

        let channel = self.registry.create(ObjectKind::Channel)?;
        let endpoint_a = match self.registry.create(ObjectKind::ChannelEndpoint) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let _ = self.registry.release(channel);
                return Err(error.into());
            }
        };
        let endpoint_b = match self.registry.create(ObjectKind::ChannelEndpoint) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let _ = self.registry.release(endpoint_a);
                let _ = self.registry.release(channel);
                return Err(error.into());
            }
        };
        let pair = match ChannelPair::new(channel, endpoint_a, endpoint_b) {
            Ok(pair) => pair,
            Err(error) => {
                self.release_created_objects(channel, endpoint_a, endpoint_b);
                return Err(error.into());
            }
        };
        let rights = Rights::READ | Rights::WRITE | Rights::WAIT | Rights::TRANSFER;
        let process = self.processes[self.process_index(process_id)?]
            .as_mut()
            .expect("validated process");
        let first = match process
            .handles
            .insert(&mut self.registry, endpoint_a, rights)
        {
            Ok(handle) => handle,
            Err(error) => {
                self.release_created_objects(channel, endpoint_a, endpoint_b);
                return Err(error.into());
            }
        };
        let second = match process
            .handles
            .insert(&mut self.registry, endpoint_b, rights)
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = process.handles.close(&mut self.registry, first);
                self.release_created_objects(channel, endpoint_a, endpoint_b);
                return Err(error.into());
            }
        };

        self.channels[slot_index] = ChannelSlot {
            pair: Some(pair),
            object: Some(channel),
            endpoint_a: Some(endpoint_a),
            endpoint_b: Some(endpoint_b),
        };
        Ok(ChannelEndpoints {
            endpoint_a: first.raw(),
            endpoint_b: second.raw(),
        })
    }

    pub fn send(
        &mut self,
        process_id: u32,
        endpoint: u64,
        request: ChannelSendRequest,
    ) -> Result<WokenWaiters, UserIpcError> {
        self.process_mut(process_id)?;
        let payload_len =
            usize::try_from(request.payload_len).map_err(|_| UserIpcError::InvalidRequest)?;
        let transfer_count =
            usize::try_from(request.transfer_count).map_err(|_| UserIpcError::InvalidRequest)?;
        if payload_len > MAX_CHANNEL_INLINE_PAYLOAD || transfer_count > MAX_CHANNEL_TRANSFER_HANDLES
        {
            return Err(UserIpcError::InvalidRequest);
        }

        let mut message = OutgoingMessage::new(MessageHeader {
            protocol_id: request.protocol_id,
            version: request.version,
            request_id: request.request_id,
            opcode: request.opcode,
            flags: request.flags,
        });
        message.set_payload(&request.payload[..payload_len])?;
        for transfer in request.transfers.iter().take(transfer_count) {
            if transfer.reserved != 0 {
                return Err(UserIpcError::InvalidRequest);
            }
            let rights = Rights::from_bits(transfer.rights).ok_or(UserIpcError::InvalidRequest)?;
            message.add_transfer(Handle::from_raw(transfer.handle), rights)?;
        }

        let (channel_index, sender_endpoint) =
            self.locate_endpoint(process_id, Handle::from_raw(endpoint))?;
        let slot = &self.channels[channel_index];
        let peer = if slot.endpoint_a == Some(sender_endpoint) {
            slot.endpoint_b
        } else {
            slot.endpoint_a
        };
        if !peer.is_some_and(|peer| self.endpoint_reachable(peer)) {
            return Err(UserIpcError::PeerClosed);
        }
        let process = self.processes[self.process_index(process_id)?]
            .as_mut()
            .expect("validated process");
        let pair = self.channels[channel_index]
            .pair
            .as_mut()
            .expect("located live Channel");
        pair.send(
            &self.registry,
            process,
            Handle::from_raw(endpoint),
            message,
            &mut self.waiters,
        )?;
        Ok(self.waiters.take_woken_waiters())
    }

    pub fn wait_readable(
        &mut self,
        process_id: u32,
        endpoint: u64,
        waiter_id: u32,
        on_blocked: impl FnOnce() -> bool,
    ) -> Result<ChannelWaitOutcome, UserIpcError> {
        self.process_mut(process_id)?;
        let handle = Handle::from_raw(endpoint);
        let (channel_index, _) = self.locate_endpoint(process_id, handle)?;
        let process = self.process_ref(process_id)?;
        let item = self.channels[channel_index]
            .pair
            .as_ref()
            .expect("located live Channel")
            .wait_item_for_process(&self.registry, process, handle, Signals::READABLE)?;
        let mut waiter = Waiter::new(waiter_id);
        match crate::ipc::wait(&mut self.waiters, &mut waiter, item) {
            Ok(_) => Ok(ChannelWaitOutcome::Readable),
            Err(WaitError::Blocked) => {
                if on_blocked() {
                    Ok(ChannelWaitOutcome::Blocked)
                } else {
                    self.waiters.cancel_waiter(waiter_id);
                    Err(UserIpcError::Wait(WaitError::Invalid))
                }
            }
            Err(error) => Err(UserIpcError::Wait(error)),
        }
    }

    pub fn try_receive(
        &mut self,
        process_id: u32,
        endpoint: u64,
    ) -> Result<Option<ChannelReceiveResult>, UserIpcError> {
        self.process_mut(process_id)?;
        let handle = Handle::from_raw(endpoint);
        let (channel_index, _) = self.locate_endpoint(process_id, handle)?;
        let process = self.processes[self.process_index(process_id)?]
            .as_mut()
            .expect("validated process");
        let received = self.channels[channel_index]
            .pair
            .as_mut()
            .expect("located live Channel")
            .receive(&mut self.registry, process, handle)?;
        let Some(received) = received else {
            return Ok(None);
        };

        let header = received.header();
        let mut result = ChannelReceiveResult {
            sender_process_id: received.sender_process_id(),
            protocol_id: header.protocol_id,
            version: header.version,
            opcode: header.opcode,
            flags: header.flags,
            request_id: header.request_id,
            payload_len: received.payload().len() as u32,
            transfer_count: received.handle_count() as u32,
            ..ChannelReceiveResult::default()
        };
        result.payload[..received.payload().len()].copy_from_slice(received.payload());
        for index in 0..received.handle_count() {
            result.handles[index] = received.handle(index).expect("received handle").raw();
        }
        Ok(Some(result))
    }

    pub fn close(&mut self, process_id: u32, raw_handle: u64) -> Result<(), UserIpcError> {
        self.process_mut(process_id)?;
        let handle = Handle::from_raw(raw_handle);
        self.locate_endpoint(process_id, handle)?;
        self.processes[self.process_index(process_id)?]
            .as_mut()
            .expect("validated process")
            .handles
            .close(&mut self.registry, handle)?;
        self.collect_unreachable_channels()
    }

    fn locate_endpoint(
        &self,
        process_id: u32,
        handle: Handle,
    ) -> Result<(usize, ObjectId), UserIpcError> {
        let process = self
            .process_ref(process_id)
            .map_err(|_| UserIpcError::InvalidEndpoint)?;
        let endpoint = process.handles.resolve(&self.registry, handle)?;
        if endpoint.kind() != ObjectKind::ChannelEndpoint {
            return Err(UserIpcError::InvalidEndpoint);
        }
        let Some(index) = self
            .channels
            .iter()
            .position(|slot| slot.pair.is_some() && slot.contains_endpoint(endpoint))
        else {
            return Err(UserIpcError::InvalidEndpoint);
        };
        Ok((index, endpoint))
    }

    fn endpoint_reachable(&self, endpoint: ObjectId) -> bool {
        let mut reachable = false;
        for process in self.processes.iter().flatten() {
            process.handles.visit_objects(|object| {
                reachable |= object == endpoint;
            });
        }
        for pair in self.channels.iter().filter_map(|slot| slot.pair.as_ref()) {
            pair.visit_escrow_objects(|object| {
                reachable |= object == endpoint;
            });
        }
        reachable
    }

    fn release_created_objects(
        &mut self,
        channel: ObjectId,
        endpoint_a: ObjectId,
        endpoint_b: ObjectId,
    ) {
        let _ = self.registry.release(endpoint_a);
        let _ = self.registry.release(endpoint_b);
        let _ = self.registry.release(channel);
    }

    fn collect_unreachable_channels(&mut self) -> Result<(), UserIpcError> {
        let mut reachable = [false; CHANNEL_CAPACITY];
        for process in self.processes.iter().flatten() {
            process.handles.visit_objects(|object| {
                if object.kind() == ObjectKind::ChannelEndpoint {
                    if let Some(index) = self
                        .channels
                        .iter()
                        .position(|slot| slot.pair.is_some() && slot.contains_endpoint(object))
                    {
                        reachable[index] = true;
                    }
                }
            });
        }

        loop {
            let mut changed = false;
            for source in 0..CHANNEL_CAPACITY {
                if !reachable[source] {
                    continue;
                }
                let Some(pair) = self.channels[source].pair.as_ref() else {
                    continue;
                };
                pair.visit_escrow_objects(|object| {
                    if object.kind() == ObjectKind::ChannelEndpoint {
                        if let Some(target) = self
                            .channels
                            .iter()
                            .position(|slot| slot.pair.is_some() && slot.contains_endpoint(object))
                        {
                            if !reachable[target] {
                                reachable[target] = true;
                                changed = true;
                            }
                        }
                    }
                });
            }
            if !changed {
                break;
            }
        }

        for (index, is_reachable) in reachable.iter().copied().enumerate() {
            if is_reachable || self.channels[index].pair.is_none() {
                continue;
            }
            let slot = &mut self.channels[index];
            let endpoint_a = slot.endpoint_a.expect("live endpoint A");
            let endpoint_b = slot.endpoint_b.expect("live endpoint B");
            let object = slot.object.expect("live Channel object");
            slot.pair
                .as_mut()
                .expect("live Channel")
                .drain(&mut self.registry)?;
            self.registry.release(endpoint_a)?;
            self.registry.release(endpoint_b)?;
            self.registry.release(object)?;
            slot.clear();
        }
        Ok(())
    }
}

impl Default for UserIpcState {
    fn default() -> Self {
        Self::new()
    }
}

struct SharedUserIpc {
    locked: AtomicBool,
    state: UnsafeCell<UserIpcState>,
}

// Access to the mutable manager is serialized by `locked` across bootstrap
// user threads and CPUs.
unsafe impl Sync for SharedUserIpc {}

impl SharedUserIpc {
    const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            state: UnsafeCell::new(UserIpcState::new()),
        }
    }

    fn with<R>(&self, operation: impl FnOnce(&mut UserIpcState) -> R) -> R {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let result = operation(unsafe { &mut *self.state.get() });
        self.locked.store(false, Ordering::Release);
        result
    }
}

static USER_IPC: SharedUserIpc = SharedUserIpc::new();

pub fn create_pair(process_id: u32) -> Result<ChannelEndpoints, UserIpcError> {
    USER_IPC.with(|state| state.create_pair(process_id))
}

pub fn send(
    process_id: u32,
    endpoint: u64,
    request: ChannelSendRequest,
) -> Result<WokenWaiters, UserIpcError> {
    USER_IPC.with(|state| state.send(process_id, endpoint, request))
}

pub fn wait_readable(
    process_id: u32,
    endpoint: u64,
    waiter_id: u32,
    on_blocked: impl FnOnce() -> bool,
) -> Result<ChannelWaitOutcome, UserIpcError> {
    USER_IPC.with(|state| state.wait_readable(process_id, endpoint, waiter_id, on_blocked))
}

pub fn register_spawned_process(
    parent: u32,
    child: u32,
    endpoint: u64,
    rights: Rights,
) -> Result<u64, UserIpcError> {
    USER_IPC.with(|state| state.register_spawned_process(parent, child, endpoint, rights))
}

pub fn exit_process(process_id: u32) -> Result<(), UserIpcError> {
    USER_IPC.with(|state| state.exit_process(process_id))
}

pub fn cancel_waiter(waiter_id: u32) {
    USER_IPC.with(|state| state.waiters.cancel_waiter(waiter_id));
}

pub fn try_receive(
    process_id: u32,
    endpoint: u64,
) -> Result<Option<ChannelReceiveResult>, UserIpcError> {
    USER_IPC.with(|state| state.try_receive(process_id, endpoint))
}

pub fn close(process_id: u32, handle: u64) -> Result<(), UserIpcError> {
    USER_IPC.with(|state| state.close(process_id, handle))
}

#[cfg(test)]
mod tests {
    use nagi_abi::{ChannelHandleTransfer, ChannelSendRequest};

    use crate::handles::{Handle, Rights};
    use crate::ipc::{ChannelError, MAX_QUEUE};

    use super::{UserIpcError, UserIpcState, BOOTSTRAP_PROCESS_ID as INIT};

    fn request(payload: &[u8]) -> ChannelSendRequest {
        let mut request = ChannelSendRequest::new(7, 1, 42, 9);
        request.payload_len = payload.len() as u32;
        request.payload[..payload.len()].copy_from_slice(payload);
        request
    }

    #[test]
    fn bootstrap_channel_round_trip_stamps_process_and_ignores_payload_identity() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let forged_identity = [0xfe, 0xff, 0xff, 0xff, 0xfd, 0xff, 0xff, 0xff];
        ipc.send(INIT, endpoints.endpoint_a, request(&forged_identity))
            .expect("send");
        let received = ipc
            .try_receive(INIT, endpoints.endpoint_b)
            .expect("receive syscall")
            .expect("message");

        assert_eq!(received.sender_process_id, 1);
        assert_eq!(received.payload_len, forged_identity.len() as u32);
        assert_eq!(&received.payload[..forged_identity.len()], &forged_identity);
        assert_eq!(received.protocol_id, 7);
        assert_eq!(received.request_id, 42);
        assert_eq!(received.transfer_count, 0);
        ipc.close(INIT, endpoints.endpoint_a).expect("close A");
        ipc.close(INIT, endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn channel_wait_registers_before_block_and_send_returns_one_wakeup() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        assert_eq!(
            ipc.wait_readable(INIT, endpoints.endpoint_b, 7, || true),
            Ok(super::ChannelWaitOutcome::Blocked)
        );

        let woken = ipc
            .send(INIT, endpoints.endpoint_a, request(b"wake"))
            .expect("send");
        let mut woken_ids = woken.iter();
        assert_eq!(woken_ids.next(), Some(7));
        assert_eq!(woken_ids.next(), None);
        let later_send = ipc
            .send(INIT, endpoints.endpoint_a, request(b"still queued"))
            .expect("second send");
        assert_eq!(later_send.iter().count(), 0);

        let received = ipc
            .try_receive(INIT, endpoints.endpoint_b)
            .expect("receive")
            .expect("message");
        assert_eq!(&received.payload[..received.payload_len as usize], b"wake");
        assert!(ipc
            .try_receive(INIT, endpoints.endpoint_b)
            .expect("receive second")
            .is_some());
        ipc.close(INIT, endpoints.endpoint_a).expect("close A");
        ipc.close(INIT, endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn channel_wait_is_level_triggered_requires_wait_right_and_cleans_cancelled_ids() {
        let mut ipc = UserIpcState::new();
        let transport = ipc.create_pair(INIT).expect("transport pair");
        let target = ipc.create_pair(INIT).expect("target pair");

        ipc.send(INIT, transport.endpoint_a, request(b"ready"))
            .expect("send ready message");
        let mut callback_called = false;
        assert_eq!(
            ipc.wait_readable(INIT, transport.endpoint_b, 3, || {
                callback_called = true;
                true
            }),
            Ok(super::ChannelWaitOutcome::Readable)
        );
        assert!(!callback_called);
        assert!(ipc
            .try_receive(INIT, transport.endpoint_b)
            .expect("receive ready")
            .is_some());

        let mut transfer = request(b"read only");
        transfer.transfer_count = 1;
        transfer.transfers[0] = ChannelHandleTransfer {
            handle: target.endpoint_a,
            rights: Rights::READ.bits(),
            reserved: 0,
        };
        ipc.send(INIT, transport.endpoint_a, transfer)
            .expect("send attenuated endpoint");
        let moved = ipc
            .try_receive(INIT, transport.endpoint_b)
            .expect("receive attenuated endpoint")
            .expect("message")
            .handles[0];
        let mut callback_called = false;
        assert_eq!(
            ipc.wait_readable(INIT, moved, 3, || {
                callback_called = true;
                true
            }),
            Err(UserIpcError::Channel(ChannelError::RightsMissing))
        );
        assert!(!callback_called);

        assert_eq!(
            ipc.wait_readable(INIT, transport.endpoint_b, 9, || true),
            Ok(super::ChannelWaitOutcome::Blocked)
        );
        ipc.waiters.cancel_waiter(9);
        assert_eq!(
            ipc.send(INIT, transport.endpoint_a, request(b"after cancel"))
                .expect("send after cancellation")
                .iter()
                .count(),
            0
        );
        assert!(ipc
            .try_receive(INIT, transport.endpoint_b)
            .expect("receive cancelled waiter message")
            .is_some());
        assert!(matches!(
            ipc.wait_readable(INIT, transport.endpoint_b, 9, || true),
            Ok(super::ChannelWaitOutcome::Blocked)
        ));
        let woken = ipc
            .send(INIT, transport.endpoint_a, request(b"wake again"))
            .expect("send after re-register");
        let mut woken_ids = woken.iter();
        assert_eq!(woken_ids.next(), Some(9));
        assert_eq!(woken_ids.next(), None);

        for handle in [
            transport.endpoint_a,
            transport.endpoint_b,
            target.endpoint_b,
            moved,
        ] {
            ipc.close(INIT, handle).expect("close endpoint");
        }
    }

    #[test]
    fn transferred_channel_rights_are_attenuated_and_closed_slots_are_reused() {
        let mut ipc = UserIpcState::new();
        let transport = ipc.create_pair(INIT).expect("transport pair");
        let target = ipc.create_pair(INIT).expect("target pair");

        let mut transfer = request(b"attenuated endpoint");
        transfer.transfer_count = 1;
        transfer.transfers[0] = ChannelHandleTransfer {
            handle: target.endpoint_a,
            rights: Rights::READ.bits(),
            reserved: 0,
        };
        ipc.send(INIT, transport.endpoint_a, transfer)
            .expect("send transfer");
        assert!(matches!(
            ipc.send(INIT, target.endpoint_a, request(b"moved")),
            Err(UserIpcError::Handle(_))
        ));
        let received = ipc
            .try_receive(INIT, transport.endpoint_b)
            .expect("receive syscall")
            .expect("transferred message");
        let attenuated = received.handles[0];
        assert_ne!(attenuated, 0);
        assert!(matches!(
            ipc.send(INIT, attenuated, request(b"must not write")),
            Err(UserIpcError::Channel(ChannelError::RightsMissing))
        ));

        ipc.send(INIT, target.endpoint_b, request(b"read still works"))
            .expect("peer can write");
        let target_message = ipc
            .try_receive(INIT, attenuated)
            .expect("read-only endpoint can receive")
            .expect("target message");
        assert_eq!(
            &target_message.payload[..target_message.payload_len as usize],
            b"read still works"
        );

        assert!(ipc
            .try_receive(INIT, transport.endpoint_a)
            .unwrap()
            .is_none());
        ipc.close(INIT, transport.endpoint_a)
            .expect("close transport A");
        ipc.close(INIT, transport.endpoint_b)
            .expect("close transport B");
        ipc.close(INIT, target.endpoint_b).expect("close target B");
        ipc.close(INIT, attenuated).expect("close moved endpoint");

        let reused = ipc
            .create_pair(INIT)
            .expect("reuse Channel and handle slots");
        assert_ne!(reused.endpoint_a, transport.endpoint_a);
        assert_ne!(reused.endpoint_b, transport.endpoint_b);
        ipc.close(INIT, reused.endpoint_a).expect("close reused A");
        ipc.close(INIT, reused.endpoint_b).expect("close reused B");
    }

    #[test]
    fn queue_full_and_invalid_bounds_fail_without_consuming_messages() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let too_long = {
            let mut request = request(b"");
            request.payload_len = u32::MAX;
            request
        };
        assert_eq!(
            ipc.send(INIT, endpoints.endpoint_a, too_long),
            Err(UserIpcError::InvalidRequest)
        );
        let mut unknown_rights = request(b"");
        unknown_rights.transfer_count = 1;
        unknown_rights.transfers[0].rights = u32::MAX;
        assert_eq!(
            ipc.send(INIT, endpoints.endpoint_a, unknown_rights),
            Err(UserIpcError::InvalidRequest)
        );

        for index in 0..MAX_QUEUE {
            ipc.send(INIT, endpoints.endpoint_a, request(&[index as u8]))
                .expect("send within queue capacity");
        }
        assert_eq!(
            ipc.send(INIT, endpoints.endpoint_a, request(b"overflow")),
            Err(UserIpcError::Channel(ChannelError::QueueFull))
        );
        for index in 0..MAX_QUEUE {
            let received = ipc
                .try_receive(INIT, endpoints.endpoint_b)
                .expect("receive syscall")
                .expect("queued message");
            assert_eq!(received.payload[0], index as u8);
        }
        assert!(ipc
            .try_receive(INIT, endpoints.endpoint_b)
            .unwrap()
            .is_none());
        ipc.close(INIT, endpoints.endpoint_a).expect("close A");
        ipc.close(INIT, endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn forged_or_stale_endpoint_handles_are_rejected() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        assert!(matches!(
            ipc.send(INIT, u64::MAX, request(b"forged")),
            Err(UserIpcError::Handle(_))
        ));
        let stale = Handle::from_raw(endpoints.endpoint_a);
        ipc.close(INIT, endpoints.endpoint_a).expect("close A");
        assert!(matches!(
            ipc.try_receive(INIT, stale.raw()),
            Err(UserIpcError::Handle(_))
        ));
        ipc.close(INIT, endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn closing_last_roots_collects_cross_channel_escrow_cycles() {
        let mut ipc = UserIpcState::new();
        let first = ipc.create_pair(INIT).expect("first pair");
        let second = ipc.create_pair(INIT).expect("second pair");

        let mut first_transfer = request(b"second endpoint");
        first_transfer.transfer_count = 1;
        first_transfer.transfers[0] = ChannelHandleTransfer {
            handle: second.endpoint_a,
            rights: Rights::READ.bits() | Rights::WRITE.bits() | Rights::TRANSFER.bits(),
            reserved: 0,
        };
        ipc.send(INIT, first.endpoint_a, first_transfer)
            .expect("queue second endpoint");
        let received_second = ipc.try_receive(INIT, first.endpoint_b).expect("receive");
        let second_a = received_second.expect("transferred endpoint").handles[0];

        let mut second_transfer = request(b"first endpoint");
        second_transfer.transfer_count = 1;
        second_transfer.transfers[0] = ChannelHandleTransfer {
            handle: first.endpoint_a,
            rights: Rights::READ.bits() | Rights::WRITE.bits() | Rights::TRANSFER.bits(),
            reserved: 0,
        };
        ipc.send(INIT, second.endpoint_b, second_transfer)
            .expect("queue first endpoint");
        assert!(matches!(
            ipc.send(INIT, first.endpoint_a, request(b"moved")),
            Err(UserIpcError::Handle(_))
        ));

        // The remaining live A/B endpoints root each other's queued transfer.
        // Once those roots close, graph collection drains escrow and reuses
        // both bounded manager slots instead of leaking the cycle.
        ipc.close(INIT, first.endpoint_b).expect("close first root");
        ipc.close(INIT, second.endpoint_b)
            .expect("close second root");
        ipc.close(INIT, second_a)
            .expect("close transferred second endpoint");
        let reused = ipc
            .create_pair(INIT)
            .expect("reuse collected channel slots");
        ipc.close(INIT, reused.endpoint_a).expect("close reused A");
        ipc.close(INIT, reused.endpoint_b).expect("close reused B");
    }

    const CHILD: u32 = 2;

    fn spawn_child(ipc: &mut UserIpcState) -> (u64, u64) {
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let child_handle = ipc
            .register_spawned_process(
                INIT,
                CHILD,
                endpoints.endpoint_b,
                Rights::READ | Rights::WRITE | Rights::WAIT,
            )
            .expect("spawn registration");
        (endpoints.endpoint_a, child_handle)
    }

    #[test]
    fn spawned_process_messages_carry_kernel_pid_not_payload_identity() {
        let mut ipc = UserIpcState::new();
        let (init_endpoint, child_endpoint) = spawn_child(&mut ipc);
        // The child claims to be init (PID 1) inside its payload.
        ipc.send(CHILD, child_endpoint, request(&[1, 0, 0, 0]))
            .expect("child send");
        let received = ipc
            .try_receive(INIT, init_endpoint)
            .expect("receive")
            .expect("message");
        assert_eq!(received.sender_process_id, CHILD);
        assert_eq!(&received.payload[..4], &[1, 0, 0, 0]);

        ipc.send(INIT, init_endpoint, request(b"reply"))
            .expect("reply");
        let reply = ipc
            .try_receive(CHILD, child_endpoint)
            .expect("child receive")
            .expect("reply message");
        assert_eq!(reply.sender_process_id, INIT);
    }

    #[test]
    fn spawn_moves_the_endpoint_and_handle_tables_stay_process_local() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let private = ipc.create_pair(INIT).expect("init-private pair");
        let child_endpoint = ipc
            .register_spawned_process(
                INIT,
                CHILD,
                endpoints.endpoint_b,
                Rights::READ | Rights::WRITE | Rights::WAIT,
            )
            .expect("spawn registration");
        // The moved handle is no longer usable by init.
        assert!(matches!(
            ipc.send(INIT, endpoints.endpoint_b, request(b"stale")),
            Err(UserIpcError::Handle(_))
        ));
        // Init's private handle values do not resolve in the child table.
        assert!(ipc
            .send(CHILD, private.endpoint_a, request(b"guess"))
            .is_err());
        assert!(ipc.try_receive(CHILD, private.endpoint_b).is_err());
        // The child received no TRANSFER right, so it cannot re-delegate.
        let mut transfer = request(b"delegate");
        transfer.transfer_count = 1;
        transfer.transfers[0] = ChannelHandleTransfer {
            handle: child_endpoint,
            rights: Rights::READ.bits(),
            reserved: 0,
        };
        assert!(ipc.send(CHILD, child_endpoint, transfer).is_err());
    }

    #[test]
    fn only_init_spawns_and_isolated_slots_are_bounded() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let extra = ipc.create_pair(INIT).expect("second pair");
        let rights = Rights::READ | Rights::WRITE;
        assert_eq!(
            ipc.register_spawned_process(CHILD, CHILD, endpoints.endpoint_b, rights),
            Err(UserIpcError::InvalidProcess)
        );
        assert_eq!(
            ipc.register_spawned_process(INIT, INIT, endpoints.endpoint_b, rights),
            Err(UserIpcError::InvalidProcess)
        );
        assert_eq!(
            ipc.register_spawned_process(INIT, 0, endpoints.endpoint_b, rights),
            Err(UserIpcError::InvalidProcess)
        );
        ipc.register_spawned_process(INIT, CHILD, endpoints.endpoint_b, rights)
            .expect("first spawn");
        assert_eq!(
            ipc.register_spawned_process(INIT, CHILD, endpoints.endpoint_a, rights),
            Err(UserIpcError::InvalidProcess),
            "a live Process ID cannot be registered twice"
        );
        assert_eq!(
            ipc.create_pair(CHILD + 1).map(|_| ()),
            Err(UserIpcError::InvalidProcess)
        );
        ipc.register_spawned_process(INIT, CHILD + 1, extra.endpoint_b, rights)
            .expect("second concurrent isolated process");
        assert_eq!(
            ipc.register_spawned_process(INIT, CHILD + 2, endpoints.endpoint_a, rights),
            Err(UserIpcError::Capacity)
        );
    }

    #[test]
    fn spawn_cannot_strengthen_rights() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair(INIT).expect("pair");
        let (outer, _) = (endpoints.endpoint_a, endpoints.endpoint_b);
        let transport = ipc.create_pair(INIT).expect("transport");
        let mut transfer = request(b"attenuate");
        transfer.transfer_count = 1;
        transfer.transfers[0] = ChannelHandleTransfer {
            handle: endpoints.endpoint_b,
            rights: Rights::READ.bits() | Rights::TRANSFER.bits(),
            reserved: 0,
        };
        ipc.send(INIT, transport.endpoint_a, transfer)
            .expect("move");
        let received = ipc
            .try_receive(INIT, transport.endpoint_b)
            .expect("receive")
            .expect("message");
        let read_only = received.handles[0];
        assert!(matches!(
            ipc.register_spawned_process(INIT, CHILD, read_only, Rights::READ | Rights::WRITE),
            Err(UserIpcError::Handle(_))
        ));
        let _ = outer;
    }

    #[test]
    fn exited_process_releases_handles_and_peer_send_reports_closed() {
        let mut ipc = UserIpcState::new();
        let (init_endpoint, child_endpoint) = spawn_child(&mut ipc);
        ipc.send(INIT, init_endpoint, request(b"before exit"))
            .expect("send while child lives");
        ipc.exit_process(CHILD).expect("child exit");
        assert_eq!(
            ipc.send(INIT, init_endpoint, request(b"after exit")),
            Err(UserIpcError::PeerClosed)
        );
        assert!(ipc.send(CHILD, child_endpoint, request(b"ghost")).is_err());
        assert_eq!(ipc.exit_process(CHILD), Err(UserIpcError::InvalidProcess));
        assert_eq!(ipc.exit_process(INIT), Err(UserIpcError::InvalidProcess));
        ipc.close(INIT, init_endpoint).expect("close init endpoint");
        // The slot is reusable after cleanup under a new kernel-assigned ID.
        let endpoints = ipc.create_pair(INIT).expect("pair");
        ipc.register_spawned_process(INIT, CHILD + 1, endpoints.endpoint_b, Rights::READ)
            .expect("next process");
        assert_eq!(
            ipc.create_pair(CHILD).map(|_| ()),
            Err(UserIpcError::InvalidProcess)
        );
        assert!(ipc.create_pair(CHILD + 1).is_ok());
    }
}
