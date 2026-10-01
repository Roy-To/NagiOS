//! Bounded user ABI for the single-process bootstrap Channel path.
//!
//! This manager deliberately identifies every endpoint as `nagi-init` (PID
//! 1). It is useful for exercising the user ABI and capability attenuation,
//! but it is not an authenticated application/service boundary.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use nagi_abi::{
    ChannelEndpoints, ChannelReceiveResult, ChannelSendRequest, MAX_CHANNEL_INLINE_PAYLOAD,
    MAX_CHANNEL_TRANSFER_HANDLES,
};

use crate::handles::{Handle, HandleError, ObjectId, ObjectKind, ObjectRegistry, Process, Rights};
use crate::ipc::{
    ChannelError, ChannelPair, MessageHeader, OutgoingMessage, WaitRegistry, MAX_INLINE_PAYLOAD,
    MAX_TRANSFER_HANDLES,
};

const BOOTSTRAP_PROCESS_ID: u32 = 1;
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
    Handle(HandleError),
    Channel(ChannelError),
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
    process: Option<Process<HANDLE_CAPACITY>>,
    channels: [ChannelSlot; CHANNEL_CAPACITY],
    waiters: WaitRegistry,
}

impl UserIpcState {
    pub const fn new() -> Self {
        Self {
            registry: ObjectRegistry::new(),
            process: None,
            channels: [const { ChannelSlot::new() }; CHANNEL_CAPACITY],
            waiters: WaitRegistry::new(),
        }
    }

    fn initialize(&mut self) -> Result<(), UserIpcError> {
        if self.process.is_some() {
            return Ok(());
        }
        let address_space = self.registry.create(ObjectKind::AddressSpace)?;
        self.process = Some(Process::new(BOOTSTRAP_PROCESS_ID, address_space));
        Ok(())
    }

    pub fn create_pair(&mut self) -> Result<ChannelEndpoints, UserIpcError> {
        self.initialize()?;
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
        let process = self
            .process
            .as_mut()
            .expect("initialized bootstrap process");
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

    pub fn send(&mut self, endpoint: u64, request: ChannelSendRequest) -> Result<(), UserIpcError> {
        self.initialize()?;
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

        let (channel_index, _) = self.locate_endpoint(Handle::from_raw(endpoint))?;
        let process = self
            .process
            .as_mut()
            .expect("initialized bootstrap process");
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
        Ok(())
    }

    pub fn try_receive(
        &mut self,
        endpoint: u64,
    ) -> Result<Option<ChannelReceiveResult>, UserIpcError> {
        self.initialize()?;
        let handle = Handle::from_raw(endpoint);
        let (channel_index, _) = self.locate_endpoint(handle)?;
        let process = self
            .process
            .as_mut()
            .expect("initialized bootstrap process");
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

    pub fn close(&mut self, raw_handle: u64) -> Result<(), UserIpcError> {
        self.initialize()?;
        let handle = Handle::from_raw(raw_handle);
        self.locate_endpoint(handle)?;
        self.process
            .as_mut()
            .expect("initialized bootstrap process")
            .handles
            .close(&mut self.registry, handle)?;
        self.collect_unreachable_channels()
    }

    fn locate_endpoint(&self, handle: Handle) -> Result<(usize, ObjectId), UserIpcError> {
        let process = self.process.as_ref().ok_or(UserIpcError::InvalidEndpoint)?;
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
        if let Some(process) = self.process.as_ref() {
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

pub fn create_pair() -> Result<ChannelEndpoints, UserIpcError> {
    USER_IPC.with(UserIpcState::create_pair)
}

pub fn send(endpoint: u64, request: ChannelSendRequest) -> Result<(), UserIpcError> {
    USER_IPC.with(|state| state.send(endpoint, request))
}

pub fn try_receive(endpoint: u64) -> Result<Option<ChannelReceiveResult>, UserIpcError> {
    USER_IPC.with(|state| state.try_receive(endpoint))
}

pub fn close(handle: u64) -> Result<(), UserIpcError> {
    USER_IPC.with(|state| state.close(handle))
}

#[cfg(test)]
mod tests {
    use nagi_abi::{ChannelHandleTransfer, ChannelSendRequest};

    use crate::handles::{Handle, Rights};
    use crate::ipc::{ChannelError, MAX_QUEUE};

    use super::{UserIpcError, UserIpcState};

    fn request(payload: &[u8]) -> ChannelSendRequest {
        let mut request = ChannelSendRequest::new(7, 1, 42, 9);
        request.payload_len = payload.len() as u32;
        request.payload[..payload.len()].copy_from_slice(payload);
        request
    }

    #[test]
    fn bootstrap_channel_round_trip_stamps_process_and_ignores_payload_identity() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair().expect("pair");
        let forged_identity = [0xfe, 0xff, 0xff, 0xff, 0xfd, 0xff, 0xff, 0xff];
        ipc.send(endpoints.endpoint_a, request(&forged_identity))
            .expect("send");
        let received = ipc
            .try_receive(endpoints.endpoint_b)
            .expect("receive syscall")
            .expect("message");

        assert_eq!(received.sender_process_id, 1);
        assert_eq!(received.payload_len, forged_identity.len() as u32);
        assert_eq!(&received.payload[..forged_identity.len()], &forged_identity);
        assert_eq!(received.protocol_id, 7);
        assert_eq!(received.request_id, 42);
        assert_eq!(received.transfer_count, 0);
        ipc.close(endpoints.endpoint_a).expect("close A");
        ipc.close(endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn transferred_channel_rights_are_attenuated_and_closed_slots_are_reused() {
        let mut ipc = UserIpcState::new();
        let transport = ipc.create_pair().expect("transport pair");
        let target = ipc.create_pair().expect("target pair");

        let mut transfer = request(b"attenuated endpoint");
        transfer.transfer_count = 1;
        transfer.transfers[0] = ChannelHandleTransfer {
            handle: target.endpoint_a,
            rights: Rights::READ.bits(),
            reserved: 0,
        };
        ipc.send(transport.endpoint_a, transfer)
            .expect("send transfer");
        assert!(matches!(
            ipc.send(target.endpoint_a, request(b"moved")),
            Err(UserIpcError::Handle(_))
        ));
        let received = ipc
            .try_receive(transport.endpoint_b)
            .expect("receive syscall")
            .expect("transferred message");
        let attenuated = received.handles[0];
        assert_ne!(attenuated, 0);
        assert!(matches!(
            ipc.send(attenuated, request(b"must not write")),
            Err(UserIpcError::Channel(ChannelError::RightsMissing))
        ));

        ipc.send(target.endpoint_b, request(b"read still works"))
            .expect("peer can write");
        let target_message = ipc
            .try_receive(attenuated)
            .expect("read-only endpoint can receive")
            .expect("target message");
        assert_eq!(
            &target_message.payload[..target_message.payload_len as usize],
            b"read still works"
        );

        assert!(ipc.try_receive(transport.endpoint_a).unwrap().is_none());
        ipc.close(transport.endpoint_a).expect("close transport A");
        ipc.close(transport.endpoint_b).expect("close transport B");
        ipc.close(target.endpoint_b).expect("close target B");
        ipc.close(attenuated).expect("close moved endpoint");

        let reused = ipc.create_pair().expect("reuse Channel and handle slots");
        assert_ne!(reused.endpoint_a, transport.endpoint_a);
        assert_ne!(reused.endpoint_b, transport.endpoint_b);
        ipc.close(reused.endpoint_a).expect("close reused A");
        ipc.close(reused.endpoint_b).expect("close reused B");
    }

    #[test]
    fn queue_full_and_invalid_bounds_fail_without_consuming_messages() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair().expect("pair");
        let too_long = {
            let mut request = request(b"");
            request.payload_len = u32::MAX;
            request
        };
        assert_eq!(
            ipc.send(endpoints.endpoint_a, too_long),
            Err(UserIpcError::InvalidRequest)
        );
        let mut unknown_rights = request(b"");
        unknown_rights.transfer_count = 1;
        unknown_rights.transfers[0].rights = u32::MAX;
        assert_eq!(
            ipc.send(endpoints.endpoint_a, unknown_rights),
            Err(UserIpcError::InvalidRequest)
        );

        for index in 0..MAX_QUEUE {
            ipc.send(endpoints.endpoint_a, request(&[index as u8]))
                .expect("send within queue capacity");
        }
        assert_eq!(
            ipc.send(endpoints.endpoint_a, request(b"overflow")),
            Err(UserIpcError::Channel(ChannelError::QueueFull))
        );
        for index in 0..MAX_QUEUE {
            let received = ipc
                .try_receive(endpoints.endpoint_b)
                .expect("receive syscall")
                .expect("queued message");
            assert_eq!(received.payload[0], index as u8);
        }
        assert!(ipc.try_receive(endpoints.endpoint_b).unwrap().is_none());
        ipc.close(endpoints.endpoint_a).expect("close A");
        ipc.close(endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn forged_or_stale_endpoint_handles_are_rejected() {
        let mut ipc = UserIpcState::new();
        let endpoints = ipc.create_pair().expect("pair");
        assert!(matches!(
            ipc.send(u64::MAX, request(b"forged")),
            Err(UserIpcError::Handle(_))
        ));
        let stale = Handle::from_raw(endpoints.endpoint_a);
        ipc.close(endpoints.endpoint_a).expect("close A");
        assert!(matches!(
            ipc.try_receive(stale.raw()),
            Err(UserIpcError::Handle(_))
        ));
        ipc.close(endpoints.endpoint_b).expect("close B");
    }

    #[test]
    fn closing_last_roots_collects_cross_channel_escrow_cycles() {
        let mut ipc = UserIpcState::new();
        let first = ipc.create_pair().expect("first pair");
        let second = ipc.create_pair().expect("second pair");

        let mut first_transfer = request(b"second endpoint");
        first_transfer.transfer_count = 1;
        first_transfer.transfers[0] = ChannelHandleTransfer {
            handle: second.endpoint_a,
            rights: Rights::READ.bits() | Rights::WRITE.bits() | Rights::TRANSFER.bits(),
            reserved: 0,
        };
        ipc.send(first.endpoint_a, first_transfer)
            .expect("queue second endpoint");
        let received_second = ipc.try_receive(first.endpoint_b).expect("receive");
        let second_a = received_second.expect("transferred endpoint").handles[0];

        let mut second_transfer = request(b"first endpoint");
        second_transfer.transfer_count = 1;
        second_transfer.transfers[0] = ChannelHandleTransfer {
            handle: first.endpoint_a,
            rights: Rights::READ.bits() | Rights::WRITE.bits() | Rights::TRANSFER.bits(),
            reserved: 0,
        };
        ipc.send(second.endpoint_b, second_transfer)
            .expect("queue first endpoint");
        assert!(matches!(
            ipc.send(first.endpoint_a, request(b"moved")),
            Err(UserIpcError::Handle(_))
        ));

        // The remaining live A/B endpoints root each other's queued transfer.
        // Once those roots close, graph collection drains escrow and reuses
        // both bounded manager slots instead of leaking the cycle.
        ipc.close(first.endpoint_b).expect("close first root");
        ipc.close(second.endpoint_b).expect("close second root");
        ipc.close(second_a)
            .expect("close transferred second endpoint");
        let reused = ipc.create_pair().expect("reuse collected channel slots");
        ipc.close(reused.endpoint_a).expect("close reused A");
        ipc.close(reused.endpoint_b).expect("close reused B");
    }
}
