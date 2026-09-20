#[cfg(test)]
mod tests {
    use crate::handles::{HandleError, ObjectId, ObjectKind, ObjectRegistry, Process, Rights};

    use super::{
        wait_many, ChannelError, ChannelPair, Event, MessageHeader, OutgoingMessage, Signals,
        Timer, WaitError, WaitRegistry, Waiter, MAX_QUEUE, MAX_WAIT_ITEMS,
    };

    struct Fixture {
        registry: ObjectRegistry<16>,
        sender: Process<8>,
        receiver: Process<8>,
        channel: ChannelPair,
        sender_endpoint: crate::handles::Handle,
        receiver_endpoint: crate::handles::Handle,
        waiters: WaitRegistry,
    }

    fn fixture() -> Fixture {
        let mut registry = ObjectRegistry::<16>::new();
        let sender_space = registry
            .create(ObjectKind::AddressSpace)
            .expect("sender address space");
        let receiver_space = registry
            .create(ObjectKind::AddressSpace)
            .expect("receiver address space");
        let channel_object = registry.create(ObjectKind::Channel).expect("channel");
        let sender_object = registry
            .create(ObjectKind::ChannelEndpoint)
            .expect("sender endpoint");
        let receiver_object = registry
            .create(ObjectKind::ChannelEndpoint)
            .expect("receiver endpoint");
        let mut sender = Process::new(1, sender_space);
        let mut receiver = Process::new(2, receiver_space);
        let channel =
            ChannelPair::new(channel_object, sender_object, receiver_object).expect("channel pair");
        let (sender_endpoint, receiver_endpoint) = channel
            .install_endpoints(
                &mut registry,
                &mut sender,
                &mut receiver,
                Rights::READ | Rights::WRITE | Rights::WAIT,
            )
            .expect("endpoints");
        Fixture {
            registry,
            sender,
            receiver,
            channel,
            sender_endpoint,
            receiver_endpoint,
            waiters: WaitRegistry::new(),
        }
    }

    fn message(payload: &[u8]) -> OutgoingMessage {
        let mut message = OutgoingMessage::new(MessageHeader {
            protocol_id: 7,
            version: 1,
            request_id: 42,
            opcode: 9,
            flags: 0,
        });
        message.set_payload(payload).expect("payload");
        message
    }

    #[test]
    fn channel_round_trip_preserves_header_and_payload() {
        let mut fixture = fixture();
        assert_eq!(fixture.channel.object_id().kind(), ObjectKind::Channel);
        fixture
            .channel
            .send(
                &mut fixture.registry,
                &mut fixture.sender,
                fixture.sender_endpoint,
                message(b"hello from A"),
                &mut fixture.waiters,
            )
            .expect("send");
        let received = fixture
            .channel
            .receive(
                &mut fixture.registry,
                &mut fixture.receiver,
                fixture.receiver_endpoint,
            )
            .expect("receive")
            .expect("message");

        assert_eq!(
            received.header(),
            MessageHeader {
                protocol_id: 7,
                version: 1,
                request_id: 42,
                opcode: 9,
                flags: 0,
            }
        );
        assert_eq!(received.payload(), b"hello from A");
        assert_eq!(received.handle_count(), 0);
    }

    #[test]
    fn channel_send_notifies_a_blocked_readable_waiter() {
        let mut fixture = fixture();
        let mut waiter = Waiter::new(8);
        {
            let mut items = [fixture
                .channel
                .wait_item_for_process(
                    &fixture.registry,
                    &fixture.receiver,
                    fixture.receiver_endpoint,
                    Signals::READABLE,
                )
                .expect("wait item")];
            assert_eq!(
                wait_many(&mut fixture.waiters, &mut waiter, &mut items),
                Err(WaitError::Blocked)
            );
        }

        fixture
            .channel
            .send(
                &mut fixture.registry,
                &mut fixture.sender,
                fixture.sender_endpoint,
                message(b"wake"),
                &mut fixture.waiters,
            )
            .expect("send");
        assert!(waiter.sync(&mut fixture.waiters));
        let wake = waiter.take_outcome().expect("channel wake");
        assert_eq!(wake.index, 0);
        assert!(wake.observed.intersects(Signals::READABLE));
    }

    #[test]
    fn transfer_is_escrowed_until_receive_and_read_only_cannot_be_strengthened() {
        let mut fixture = fixture();
        let vmo = fixture.registry.create(ObjectKind::Vmo).expect("VMO");
        let source = fixture
            .sender
            .handles
            .insert(
                &mut fixture.registry,
                vmo,
                Rights::READ | Rights::WRITE | Rights::TRANSFER,
            )
            .expect("source handle");
        let mut message = message(b"with capability");
        message
            .add_transfer(source, Rights::READ)
            .expect("transfer disposition");

        fixture
            .channel
            .send(
                &mut fixture.registry,
                &mut fixture.sender,
                fixture.sender_endpoint,
                message,
                &mut fixture.waiters,
            )
            .expect("send");
        let receiver_object = fixture
            .receiver
            .handles
            .resolve(&fixture.registry, fixture.receiver_endpoint)
            .expect("receiver endpoint");
        assert!(fixture
            .channel
            .readiness(receiver_object)
            .expect("readiness")
            .intersects(Signals::READABLE));
        assert_eq!(
            fixture.sender.handles.resolve(&fixture.registry, source),
            Err(HandleError::Stale)
        );
        let received = fixture
            .channel
            .receive(
                &mut fixture.registry,
                &mut fixture.receiver,
                fixture.receiver_endpoint,
            )
            .expect("receive")
            .expect("message");
        assert_eq!(received.handle_count(), 1);
        let received_handle = received.handle(0).expect("received handle");
        assert_eq!(
            fixture.receiver.handles.require(
                &fixture.registry,
                received_handle,
                ObjectKind::Vmo,
                Rights::READ,
            ),
            Ok(vmo)
        );
        assert_eq!(
            fixture.receiver.handles.require(
                &fixture.registry,
                received_handle,
                ObjectKind::Vmo,
                Rights::WRITE,
            ),
            Err(HandleError::RightsMissing)
        );
    }

    #[test]
    fn channel_queue_and_endpoint_rights_are_bounded() {
        let mut filled = fixture();
        for _ in 0..MAX_QUEUE {
            filled
                .channel
                .send(
                    &mut filled.registry,
                    &mut filled.sender,
                    filled.sender_endpoint,
                    message(b"x"),
                    &mut filled.waiters,
                )
                .expect("queue slot");
        }
        assert_eq!(
            filled.channel.send(
                &mut filled.registry,
                &mut filled.sender,
                filled.sender_endpoint,
                message(b"overflow"),
                &mut filled.waiters,
            ),
            Err(ChannelError::QueueFull)
        );

        let mut restricted = fixture();
        restricted
            .sender
            .handles
            .close(&mut restricted.registry, restricted.sender_endpoint)
            .expect("close endpoint");
        assert_eq!(
            restricted.channel.send(
                &mut restricted.registry,
                &mut restricted.sender,
                restricted.sender_endpoint,
                message(b"denied"),
                &mut restricted.waiters,
            ),
            Err(ChannelError::Stale)
        );
    }

    #[test]
    fn receiver_capacity_failure_keeps_transfer_in_queue_escrow() {
        let mut fixture = fixture();
        let vmo = fixture.registry.create(ObjectKind::Vmo).expect("VMO");
        let source = fixture
            .sender
            .handles
            .insert(&mut fixture.registry, vmo, Rights::READ | Rights::TRANSFER)
            .expect("source handle");
        let mut receiver_filler = [None; 7];
        for slot in &mut receiver_filler {
            let object = fixture
                .registry
                .create(ObjectKind::Event)
                .expect("receiver filler object");
            *slot = Some(
                fixture
                    .receiver
                    .handles
                    .insert(&mut fixture.registry, object, Rights::READ)
                    .expect("receiver filler handle"),
            );
        }
        let mut transfer = message(b"queued");
        transfer
            .add_transfer(source, Rights::READ)
            .expect("transfer");
        fixture
            .channel
            .send(
                &fixture.registry,
                &mut fixture.sender,
                fixture.sender_endpoint,
                transfer,
                &mut fixture.waiters,
            )
            .expect("send");
        assert!(matches!(
            fixture.channel.receive(
                &mut fixture.registry,
                &mut fixture.receiver,
                fixture.receiver_endpoint,
            ),
            Err(ChannelError::ReceiverFull)
        ));
        fixture
            .receiver
            .handles
            .close(
                &mut fixture.registry,
                receiver_filler[0].expect("filler handle"),
            )
            .expect("free receiver slot");
        let received = fixture
            .channel
            .receive(
                &mut fixture.registry,
                &mut fixture.receiver,
                fixture.receiver_endpoint,
            )
            .expect("receive after capacity recovery")
            .expect("message");
        let received_handle = received.handle(0).expect("received handle");
        assert_eq!(
            fixture.receiver.handles.require(
                &fixture.registry,
                received_handle,
                ObjectKind::Vmo,
                Rights::READ,
            ),
            Ok(vmo)
        );
    }

    #[test]
    fn draining_a_channel_releases_queued_transfer_references() {
        let mut fixture = fixture();
        let vmo = fixture.registry.create(ObjectKind::Vmo).expect("VMO");
        let source = fixture
            .sender
            .handles
            .insert(&mut fixture.registry, vmo, Rights::READ | Rights::TRANSFER)
            .expect("source handle");
        let mut transfer = message(b"drain me");
        transfer
            .add_transfer(source, Rights::READ)
            .expect("transfer");
        fixture
            .channel
            .send(
                &fixture.registry,
                &mut fixture.sender,
                fixture.sender_endpoint,
                transfer,
                &mut fixture.waiters,
            )
            .expect("send");
        assert_eq!(fixture.registry.reference_count(vmo), Ok(2));
        fixture.channel.drain(&mut fixture.registry).expect("drain");
        fixture
            .registry
            .release(vmo)
            .expect("release creator reference");
        assert_eq!(
            fixture.registry.reference_count(vmo),
            Err(HandleError::Stale)
        );
        assert!(matches!(
            fixture.channel.receive(
                &mut fixture.registry,
                &mut fixture.receiver,
                fixture.receiver_endpoint,
            ),
            Ok(None)
        ));
    }

    #[test]
    fn event_timer_and_wait_many_have_level_triggered_readiness() {
        let mut registry = ObjectRegistry::<4>::new();
        let event_id = registry.create(ObjectKind::Event).expect("event");
        let timer_id = registry.create(ObjectKind::Timer).expect("timer");
        let mut event = Event::new(event_id);
        let mut timer = Timer::new(timer_id);
        timer.arm(10, 5).expect("arm");
        let mut waiters = WaitRegistry::new();
        let mut waiter = Waiter::new(3);
        let mut items = [
            event.wait_item(Signals::SIGNALED),
            timer.wait_item(10, Signals::FIRED),
        ];

        assert_eq!(
            wait_many(&mut waiters, &mut waiter, &mut items),
            Err(WaitError::Blocked)
        );
        drop(items);
        event.signal(&mut waiters);
        assert!(!waiter.is_runnable());
        assert!(waiter.sync(&mut waiters));
        assert!(waiter.is_runnable());
        let mut event_ready = [
            event.wait_item(Signals::SIGNALED),
            timer.wait_item(10, Signals::FIRED),
        ];
        assert_eq!(
            wait_many(&mut waiters, &mut waiter, &mut event_ready,)
                .expect("event ready")
                .index,
            0
        );

        drop(event_ready);
        event.clear();
        timer.poll(15, &mut waiters);
        let mut timer_ready = [
            event.wait_item(Signals::SIGNALED),
            timer.wait_item(15, Signals::FIRED),
        ];
        assert_eq!(
            wait_many(&mut waiters, &mut waiter, &mut timer_ready,)
                .expect("timer ready")
                .index,
            1
        );
    }

    #[test]
    fn event_and_timer_mutation_require_process_capabilities() {
        let mut registry = ObjectRegistry::<4>::new();
        let address_space = registry
            .create(ObjectKind::AddressSpace)
            .expect("address space");
        let event_object = registry.create(ObjectKind::Event).expect("event");
        let timer_object = registry.create(ObjectKind::Timer).expect("timer");
        let mut process = Process::<4>::new(1, address_space);
        let event_handle = process
            .handles
            .insert(&mut registry, event_object, Rights::WAIT)
            .expect("event wait handle");
        let timer_handle = process
            .handles
            .insert(&mut registry, timer_object, Rights::WAIT)
            .expect("timer wait handle");
        let mut event = Event::new(event_object);
        let mut timer = Timer::new(timer_object);
        let mut waiters = WaitRegistry::new();

        assert_eq!(
            process.signal_event(&registry, event_handle, &mut event, &mut waiters),
            Err(WaitError::RightsMissing)
        );
        assert_eq!(
            process.arm_timer(&registry, timer_handle, &mut timer, 0, 1),
            Err(WaitError::RightsMissing)
        );
        assert!(process
            .handles
            .require(&registry, event_handle, ObjectKind::Event, Rights::WAIT)
            .is_ok());
    }

    #[test]
    fn event_notification_wakes_each_matching_registered_waiter_once() {
        let mut registry = ObjectRegistry::<1>::new();
        let event_object = registry.create(ObjectKind::Event).expect("event");
        let mut event = Event::new(event_object);
        let mut waiters = WaitRegistry::new();
        let mut first = Waiter::new(10);
        let mut second = Waiter::new(11);
        {
            let mut items = [event.wait_item(Signals::SIGNALED)];
            assert_eq!(
                wait_many(&mut waiters, &mut first, &mut items),
                Err(WaitError::Blocked)
            );
        }
        {
            let mut items = [event.wait_item(Signals::SIGNALED)];
            assert_eq!(
                wait_many(&mut waiters, &mut second, &mut items),
                Err(WaitError::Blocked)
            );
        }
        event.signal(&mut waiters);
        assert!(first.sync(&mut waiters));
        assert!(second.sync(&mut waiters));
        assert!(!first.sync(&mut waiters));
        assert!(!second.sync(&mut waiters));
    }

    #[test]
    fn wait_registry_capacity_does_not_drop_level_triggered_wakes() {
        let mut registry = ObjectRegistry::<{ MAX_WAIT_ITEMS * 2 }>::new();
        let event_ids: [ObjectId; MAX_WAIT_ITEMS * 2] =
            core::array::from_fn(|_| registry.create(ObjectKind::Event).expect("event object"));
        let mut events: [Event; MAX_WAIT_ITEMS * 2] =
            core::array::from_fn(|index| Event::new(event_ids[index]));
        let mut wait_registry = WaitRegistry::new();
        let mut waiters: [Waiter; MAX_WAIT_ITEMS * 2] =
            core::array::from_fn(|index| Waiter::new(index as u32));

        for index in 0..MAX_WAIT_ITEMS {
            let mut items = [events[index].wait_item(Signals::SIGNALED)];
            assert_eq!(
                wait_many(&mut wait_registry, &mut waiters[index], &mut items),
                Err(WaitError::Blocked)
            );
        }
        for event in events.iter_mut().take(MAX_WAIT_ITEMS) {
            event.signal(&mut wait_registry);
        }

        for index in MAX_WAIT_ITEMS..MAX_WAIT_ITEMS * 2 {
            let mut items = [events[index].wait_item(Signals::SIGNALED)];
            assert_eq!(
                wait_many(&mut wait_registry, &mut waiters[index], &mut items),
                Err(WaitError::TooMany)
            );
        }
        for waiter in waiters.iter_mut().take(MAX_WAIT_ITEMS) {
            assert!(waiter.sync(&mut wait_registry));
        }
        for index in MAX_WAIT_ITEMS..MAX_WAIT_ITEMS * 2 {
            let mut items = [events[index].wait_item(Signals::SIGNALED)];
            assert_eq!(
                wait_many(&mut wait_registry, &mut waiters[index], &mut items),
                Err(WaitError::Blocked)
            );
        }
        for event in events.iter_mut().skip(MAX_WAIT_ITEMS) {
            event.signal(&mut wait_registry);
        }

        for waiter in waiters.iter_mut().skip(MAX_WAIT_ITEMS) {
            assert!(waiter.sync(&mut wait_registry));
        }
    }
}
use crate::handles::{
    Handle, HandleError, ObjectId, ObjectKind, ObjectRegistry, Process, Rights, TransferToken,
};

pub const MAX_QUEUE: usize = 8;
pub const MAX_INLINE_PAYLOAD: usize = 128;
pub const MAX_TRANSFER_HANDLES: usize = 4;
pub const MAX_WAIT_ITEMS: usize = 8;
const MAX_WAIT_WAKEUPS: usize = MAX_WAIT_ITEMS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelError {
    InvalidEndpoint,
    Stale,
    RightsMissing,
    QueueFull,
    Empty,
    MessageTooLarge,
    TooManyTransfers,
    DuplicateTransfer,
    ReceiverFull,
    ObjectStale,
}

impl From<HandleError> for ChannelError {
    fn from(error: HandleError) -> Self {
        match error {
            HandleError::Stale | HandleError::Closed | HandleError::Invalid => Self::Stale,
            HandleError::WrongType => Self::InvalidEndpoint,
            HandleError::RightsMissing => Self::RightsMissing,
            HandleError::TableFull | HandleError::ReceiverFull => Self::ReceiverFull,
            _ => Self::ObjectStale,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageHeader {
    pub protocol_id: u16,
    pub version: u16,
    pub request_id: u64,
    pub opcode: u16,
    pub flags: u16,
}

#[derive(Clone, Copy)]
struct HandleDisposition {
    source: Handle,
    rights: Rights,
}

pub struct OutgoingMessage {
    header: MessageHeader,
    payload: [u8; MAX_INLINE_PAYLOAD],
    payload_len: usize,
    transfers: [Option<HandleDisposition>; MAX_TRANSFER_HANDLES],
    transfer_count: usize,
}

impl OutgoingMessage {
    pub fn new(header: MessageHeader) -> Self {
        Self {
            header,
            payload: [0; MAX_INLINE_PAYLOAD],
            payload_len: 0,
            transfers: [None; MAX_TRANSFER_HANDLES],
            transfer_count: 0,
        }
    }

    pub fn set_payload(&mut self, payload: &[u8]) -> Result<(), ChannelError> {
        if payload.len() > MAX_INLINE_PAYLOAD {
            return Err(ChannelError::MessageTooLarge);
        }
        self.payload[..payload.len()].copy_from_slice(payload);
        self.payload_len = payload.len();
        Ok(())
    }

    pub fn add_transfer(&mut self, source: Handle, rights: Rights) -> Result<(), ChannelError> {
        let Some(slot) = self.transfers.get_mut(self.transfer_count) else {
            return Err(ChannelError::TooManyTransfers);
        };
        *slot = Some(HandleDisposition { source, rights });
        self.transfer_count += 1;
        Ok(())
    }
}

pub struct ReceivedMessage {
    header: MessageHeader,
    payload: [u8; MAX_INLINE_PAYLOAD],
    payload_len: usize,
    handles: [Option<Handle>; MAX_TRANSFER_HANDLES],
    handle_count: usize,
}

impl ReceivedMessage {
    pub fn header(&self) -> MessageHeader {
        self.header
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.payload_len]
    }

    pub fn handle(&self, index: usize) -> Option<Handle> {
        self.handles.get(index).copied().flatten()
    }

    pub fn handle_count(&self) -> usize {
        self.handle_count
    }
}

struct QueuedMessage {
    header: MessageHeader,
    payload: [u8; MAX_INLINE_PAYLOAD],
    payload_len: usize,
    tokens: [Option<TransferToken>; MAX_TRANSFER_HANDLES],
    token_count: usize,
}

struct Queue {
    entries: [Option<QueuedMessage>; MAX_QUEUE],
    head: usize,
    tail: usize,
    length: usize,
}

impl Queue {
    const fn new() -> Self {
        Self {
            entries: [const { None }; MAX_QUEUE],
            head: 0,
            tail: 0,
            length: 0,
        }
    }

    fn push(&mut self, message: QueuedMessage) -> Result<(), ChannelError> {
        if self.length == MAX_QUEUE {
            return Err(ChannelError::QueueFull);
        }
        self.entries[self.tail] = Some(message);
        self.tail = (self.tail + 1) % MAX_QUEUE;
        self.length += 1;
        Ok(())
    }

    fn peek(&self) -> Option<&QueuedMessage> {
        self.entries[self.head].as_ref()
    }

    fn token_mut(&mut self, index: usize) -> Option<&mut Option<TransferToken>> {
        self.entries[self.head].as_mut()?.tokens.get_mut(index)
    }

    fn pop(&mut self) -> Option<QueuedMessage> {
        let message = self.entries[self.head].take()?;
        self.head = (self.head + 1) % MAX_QUEUE;
        self.length -= 1;
        Some(message)
    }

    fn is_readable(&self) -> bool {
        self.length != 0
    }
}

pub struct ChannelPair {
    object: ObjectId,
    endpoint_a: ObjectId,
    endpoint_b: ObjectId,
    queues: [Queue; 2],
}

impl ChannelPair {
    pub(crate) fn new(
        object: ObjectId,
        endpoint_a: ObjectId,
        endpoint_b: ObjectId,
    ) -> Result<Self, ChannelError> {
        if object.kind() != ObjectKind::Channel
            || endpoint_a.kind() != ObjectKind::ChannelEndpoint
            || endpoint_b.kind() != ObjectKind::ChannelEndpoint
            || endpoint_a == endpoint_b
        {
            return Err(ChannelError::InvalidEndpoint);
        }
        Ok(Self {
            object,
            endpoint_a,
            endpoint_b,
            queues: [Queue::new(), Queue::new()],
        })
    }

    pub(crate) fn install_endpoints<const R: usize, const A: usize, const B: usize>(
        &self,
        registry: &mut ObjectRegistry<R>,
        process_a: &mut Process<A>,
        process_b: &mut Process<B>,
        rights: Rights,
    ) -> Result<(Handle, Handle), ChannelError> {
        let a = process_a
            .handles
            .insert(registry, self.endpoint_a, rights)
            .map_err(ChannelError::from)?;
        let b = match process_b.handles.insert(registry, self.endpoint_b, rights) {
            Ok(handle) => handle,
            Err(error) => {
                process_a
                    .handles
                    .close(registry, a)
                    .map_err(ChannelError::from)?;
                return Err(ChannelError::from(error));
            }
        };
        Ok((a, b))
    }

    pub(crate) const fn object_id(&self) -> ObjectId {
        self.object
    }

    pub(crate) fn drain<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
    ) -> Result<(), ChannelError> {
        let mut first_error = None;
        for queue in &mut self.queues {
            while let Some(message) = queue.pop() {
                for token in message.tokens[..message.token_count].iter().flatten() {
                    if let Err(error) = registry.release(token.capability.object) {
                        if first_error.is_none() {
                            first_error = Some(ChannelError::from(error));
                        }
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn send<const R: usize, const A: usize>(
        &mut self,
        registry: &ObjectRegistry<R>,
        sender: &mut Process<A>,
        endpoint: Handle,
        message: OutgoingMessage,
        waiters: &mut WaitRegistry,
    ) -> Result<(), ChannelError> {
        let endpoint_object = sender
            .handles
            .require(
                registry,
                endpoint,
                ObjectKind::ChannelEndpoint,
                Rights::WRITE,
            )
            .map_err(ChannelError::from)?;
        let queue_index = self.queue_for_sender(endpoint_object)?;
        if self.queues[queue_index].length == MAX_QUEUE {
            return Err(ChannelError::QueueFull);
        }
        for first in 0..message.transfer_count {
            let first_disposition = message.transfers[first].ok_or(ChannelError::ObjectStale)?;
            for later in 0..first {
                let later_disposition =
                    message.transfers[later].ok_or(ChannelError::ObjectStale)?;
                if first_disposition.source == later_disposition.source {
                    return Err(ChannelError::DuplicateTransfer);
                }
            }
            sender
                .handles
                .check_move(registry, first_disposition.source, first_disposition.rights)
                .map_err(ChannelError::from)?;
        }
        let mut queued = QueuedMessage {
            header: message.header,
            payload: message.payload,
            payload_len: message.payload_len,
            tokens: [const { None }; MAX_TRANSFER_HANDLES],
            token_count: message.transfer_count,
        };
        for index in 0..message.transfer_count {
            let disposition = message.transfers[index].ok_or(ChannelError::ObjectStale)?;
            queued.tokens[index] = Some(
                sender
                    .handles
                    .begin_move(registry, disposition.source, disposition.rights)
                    .map_err(ChannelError::from)?,
            );
        }
        let result = self.queues[queue_index].push(queued);
        if result.is_ok() {
            let receiver_endpoint = if endpoint_object == self.endpoint_a {
                self.endpoint_b
            } else {
                self.endpoint_a
            };
            let _ = waiters.notify(receiver_endpoint, Signals::READABLE);
        }
        result
    }

    pub(crate) fn receive<const R: usize, const B: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        receiver: &mut Process<B>,
        endpoint: Handle,
    ) -> Result<Option<ReceivedMessage>, ChannelError> {
        let endpoint_object = receiver
            .handles
            .require(
                registry,
                endpoint,
                ObjectKind::ChannelEndpoint,
                Rights::READ,
            )
            .map_err(ChannelError::from)?;
        let queue_index = self.queue_for_receiver(endpoint_object)?;
        let Some(queued) = self.queues[queue_index].peek() else {
            return Ok(None);
        };
        let header = queued.header;
        let payload = queued.payload;
        let payload_len = queued.payload_len;
        let token_count = queued.token_count;
        // The capacity and object-validation preflight reserves every install;
        // rollback_receive handles any defensive failure without closing an
        // escrow reference out from under the still-queued message.
        if !receiver.handles.has_capacity(token_count) {
            return Err(ChannelError::ReceiverFull);
        }
        for token in queued.tokens[..token_count].iter().flatten() {
            registry
                .validate(token.capability.object, token.capability.object.kind())
                .map_err(ChannelError::from)?;
        }
        let mut handles = [None; MAX_TRANSFER_HANDLES];
        for index in 0..token_count {
            if self.queues[queue_index]
                .peek()
                .is_none_or(|queued| queued.tokens[index].is_none())
            {
                self.rollback_receive(receiver, queue_index, &mut handles, index)?;
                return Err(ChannelError::ObjectStale);
            }
            match receiver
                .handles
                .install_token(registry, self.queues[queue_index].token_mut(index).unwrap())
            {
                Ok(handle) => handles[index] = Some(handle),
                Err(error) => {
                    self.rollback_receive(receiver, queue_index, &mut handles, index)?;
                    return Err(ChannelError::from(error));
                }
            }
        }
        if self.queues[queue_index].pop().is_none() {
            self.rollback_receive(receiver, queue_index, &mut handles, token_count)?;
            return Err(ChannelError::Empty);
        }
        Ok(Some(ReceivedMessage {
            header,
            payload,
            payload_len,
            handles,
            handle_count: token_count,
        }))
    }

    fn rollback_receive<const B: usize>(
        &mut self,
        receiver: &mut Process<B>,
        queue_index: usize,
        handles: &mut [Option<Handle>; MAX_TRANSFER_HANDLES],
        count: usize,
    ) -> Result<(), ChannelError> {
        for (index, handle_slot) in handles.iter_mut().enumerate().take(count) {
            let Some(handle) = handle_slot.take() else {
                continue;
            };
            let Some(token_slot) = self.queues[queue_index].token_mut(index) else {
                return Err(ChannelError::ObjectStale);
            };
            if token_slot.is_some() {
                return Err(ChannelError::ObjectStale);
            }
            let Some(token) = receiver.handles.rollback_install(handle) else {
                return Err(ChannelError::ObjectStale);
            };
            *token_slot = Some(token);
        }
        Ok(())
    }

    fn readiness(&self, endpoint: ObjectId) -> Result<Signals, ChannelError> {
        let queue_index = self.queue_for_receiver(endpoint)?;
        let mut signals = Signals::WRITABLE;
        if self.queues[queue_index].is_readable() {
            signals |= Signals::READABLE;
        }
        Ok(signals)
    }

    fn wait_item(
        &self,
        endpoint: ObjectId,
        interests: Signals,
    ) -> Result<WaitItem<'_>, ChannelError> {
        let _ = self.readiness(endpoint)?;
        Ok(WaitItem::channel(self, endpoint, interests))
    }

    pub(crate) fn wait_item_for_process<const R: usize, const A: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        process: &Process<A>,
        endpoint: Handle,
        interests: Signals,
    ) -> Result<WaitItem<'_>, ChannelError> {
        let endpoint_object = process
            .handles
            .require(
                registry,
                endpoint,
                ObjectKind::ChannelEndpoint,
                Rights::WAIT,
            )
            .map_err(ChannelError::from)?;
        self.wait_item(endpoint_object, interests)
    }

    fn queue_for_sender(&self, endpoint: ObjectId) -> Result<usize, ChannelError> {
        if endpoint == self.endpoint_a {
            Ok(0)
        } else if endpoint == self.endpoint_b {
            Ok(1)
        } else {
            Err(ChannelError::InvalidEndpoint)
        }
    }

    fn queue_for_receiver(&self, endpoint: ObjectId) -> Result<usize, ChannelError> {
        if endpoint == self.endpoint_a {
            Ok(1)
        } else if endpoint == self.endpoint_b {
            Ok(0)
        } else {
            Err(ChannelError::InvalidEndpoint)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Signals(u32);

impl Signals {
    pub const READABLE: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const PEER_CLOSED: Self = Self(1 << 2);
    pub const SIGNALED: Self = Self(1 << 3);
    pub const FIRED: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl core::ops::BitOr for Signals {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for Signals {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

pub(crate) enum WaitSource<'a> {
    Channel(&'a ChannelPair, ObjectId),
    Event(&'a Event),
    Timer(&'a Timer, u64),
}

pub(crate) struct WaitItem<'a> {
    source: WaitSource<'a>,
    object: ObjectId,
    interests: Signals,
    observed: Signals,
}

impl<'a> WaitItem<'a> {
    fn channel(channel: &'a ChannelPair, object: ObjectId, interests: Signals) -> Self {
        Self {
            source: WaitSource::Channel(channel, object),
            object,
            interests,
            observed: Signals::empty(),
        }
    }

    fn event(event: &'a Event, interests: Signals) -> Self {
        Self {
            source: WaitSource::Event(event),
            object: event.object,
            interests,
            observed: Signals::empty(),
        }
    }

    fn timer(timer: &'a Timer, now: u64, interests: Signals) -> Self {
        Self {
            source: WaitSource::Timer(timer, now),
            object: timer.object,
            interests,
            observed: Signals::empty(),
        }
    }

    fn refresh(&mut self) {
        self.observed = match self.source {
            WaitSource::Channel(channel, object) => channel
                .readiness(object)
                .unwrap_or_else(|_| Signals::empty()),
            WaitSource::Event(event) => {
                if event.signaled {
                    Signals::SIGNALED
                } else {
                    Signals::empty()
                }
            }
            WaitSource::Timer(timer, now) => {
                if timer.fired || timer.deadline.is_some_and(|deadline| now >= deadline) {
                    Signals::FIRED
                } else {
                    Signals::empty()
                }
            }
        };
    }

    pub(crate) fn is_ready(&mut self) -> bool {
        self.refresh();
        self.observed.intersects(self.interests)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitOutcome {
    pub index: usize,
    pub observed: Signals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitError {
    Empty,
    TooMany,
    Blocked,
    Invalid,
    Stale,
    WrongType,
    RightsMissing,
}

impl From<HandleError> for WaitError {
    fn from(error: HandleError) -> Self {
        match error {
            HandleError::RightsMissing => Self::RightsMissing,
            HandleError::WrongType => Self::WrongType,
            HandleError::Invalid | HandleError::ObjectTableFull => Self::Invalid,
            _ => Self::Stale,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaiterState {
    Runnable,
    Blocked,
}

pub struct Waiter {
    id: u32,
    epoch: u64,
    state: WaiterState,
    wake: Option<WaitOutcome>,
}

impl Waiter {
    pub const fn new(id: u32) -> Self {
        Self {
            id,
            epoch: 0,
            state: WaiterState::Runnable,
            wake: None,
        }
    }

    pub const fn is_runnable(&self) -> bool {
        matches!(self.state, WaiterState::Runnable)
    }

    pub(crate) fn sync(&mut self, registry: &mut WaitRegistry) -> bool {
        let Some(index) = registry.wakeups.iter().position(|wake| {
            wake.is_some_and(|wake| wake.waiter_id == self.id && wake.epoch == self.epoch)
        }) else {
            return false;
        };
        self.wake = registry.wakeups[index].take().map(|wake| wake.outcome);
        self.state = WaiterState::Runnable;
        true
    }

    pub(crate) fn take_outcome(&mut self) -> Option<WaitOutcome> {
        self.wake.take()
    }
}

#[derive(Clone, Copy)]
struct WaitRegistration {
    waiter_id: u32,
    epoch: u64,
    object: ObjectId,
    interests: Signals,
    index: usize,
}

#[derive(Clone, Copy)]
struct WaitWake {
    waiter_id: u32,
    epoch: u64,
    outcome: WaitOutcome,
}

pub struct WaitRegistry {
    registrations: [Option<WaitRegistration>; MAX_WAIT_ITEMS],
    wakeups: [Option<WaitWake>; MAX_WAIT_WAKEUPS],
}

impl WaitRegistry {
    pub const fn new() -> Self {
        Self {
            registrations: [const { None }; MAX_WAIT_ITEMS],
            wakeups: [const { None }; MAX_WAIT_WAKEUPS],
        }
    }

    pub(crate) fn notify(&mut self, object: ObjectId, signals: Signals) -> bool {
        let mut notified = false;
        let mut registration_index = 0;
        while registration_index < self.registrations.len() {
            let Some(registration) = self.registrations[registration_index] else {
                registration_index += 1;
                continue;
            };
            if registration.object != object || !registration.interests.intersects(signals) {
                registration_index += 1;
                continue;
            }
            let already_queued = self.wakeups.iter().flatten().any(|wake| {
                wake.waiter_id == registration.waiter_id && wake.epoch == registration.epoch
            });
            if !already_queued {
                let Some(wakeup) = self.wakeups.iter_mut().find(|wake| wake.is_none()) else {
                    registration_index += 1;
                    continue;
                };
                *wakeup = Some(WaitWake {
                    waiter_id: registration.waiter_id,
                    epoch: registration.epoch,
                    outcome: WaitOutcome {
                        index: registration.index,
                        observed: signals,
                    },
                });
                notified = true;
            }
            for entry in &mut self.registrations {
                if entry.is_some_and(|entry| {
                    entry.waiter_id == registration.waiter_id && entry.epoch == registration.epoch
                }) {
                    *entry = None;
                }
            }
            registration_index = 0;
        }
        notified
    }

    fn register(&mut self, waiter: &mut Waiter, items: &[WaitItem<'_>]) -> Result<(), WaitError> {
        self.clear_for_waiter(waiter.id);
        self.clear_wakeup_for_waiter(waiter.id);
        let occupied = self
            .registrations
            .iter()
            .filter(|registration| registration.is_some())
            .count()
            + self
                .wakeups
                .iter()
                .filter(|wakeup| wakeup.is_some())
                .count();
        if occupied + items.len() > MAX_WAIT_WAKEUPS {
            return Err(WaitError::TooMany);
        }
        waiter.epoch = waiter.epoch.wrapping_add(1);
        for (index, item) in items.iter().enumerate() {
            let Some(slot) = self.registrations.iter_mut().find(|slot| slot.is_none()) else {
                self.clear_for_waiter(waiter.id);
                return Err(WaitError::TooMany);
            };
            *slot = Some(WaitRegistration {
                waiter_id: waiter.id,
                epoch: waiter.epoch,
                object: item.object,
                interests: item.interests,
                index,
            });
        }
        waiter.state = WaiterState::Blocked;
        Ok(())
    }

    fn cancel(&mut self, waiter: &Waiter) {
        for registration in &mut self.registrations {
            if registration
                .is_some_and(|entry| entry.waiter_id == waiter.id && entry.epoch == waiter.epoch)
            {
                *registration = None;
            }
        }
    }

    fn clear_wakeup_for_waiter(&mut self, waiter_id: u32) {
        for wakeup in &mut self.wakeups {
            if wakeup.is_some_and(|entry| entry.waiter_id == waiter_id) {
                *wakeup = None;
            }
        }
    }

    fn clear_for_waiter(&mut self, waiter_id: u32) {
        for registration in &mut self.registrations {
            if registration.is_some_and(|entry| entry.waiter_id == waiter_id) {
                *registration = None;
            }
        }
    }
}

impl Default for WaitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn wait_many(
    registry: &mut WaitRegistry,
    waiter: &mut Waiter,
    items: &mut [WaitItem<'_>],
) -> Result<WaitOutcome, WaitError> {
    if items.is_empty() {
        return Err(WaitError::Empty);
    }
    if items.len() > MAX_WAIT_ITEMS {
        return Err(WaitError::TooMany);
    }
    let _ = waiter.take_outcome();
    for item in items.iter_mut() {
        item.refresh();
    }
    if let Some((index, item)) = items
        .iter()
        .enumerate()
        .find(|(_, item)| item.observed.intersects(item.interests))
    {
        registry.cancel(waiter);
        waiter.state = WaiterState::Runnable;
        return Ok(WaitOutcome {
            index,
            observed: item.observed,
        });
    }
    if let Err(error) = registry.register(waiter, items) {
        waiter.state = WaiterState::Runnable;
        return Err(error);
    }
    for item in items.iter_mut() {
        item.refresh();
    }
    if let Some((index, item)) = items
        .iter()
        .enumerate()
        .find(|(_, item)| item.observed.intersects(item.interests))
    {
        registry.cancel(waiter);
        waiter.state = WaiterState::Runnable;
        return Ok(WaitOutcome {
            index,
            observed: item.observed,
        });
    }
    Err(WaitError::Blocked)
}

pub(crate) fn wait(
    registry: &mut WaitRegistry,
    waiter: &mut Waiter,
    mut item: WaitItem<'_>,
) -> Result<WaitOutcome, WaitError> {
    wait_many(registry, waiter, core::slice::from_mut(&mut item))
}

pub struct Event {
    object: ObjectId,
    signaled: bool,
}

impl Event {
    pub(crate) const fn new(object: ObjectId) -> Self {
        Self {
            object,
            signaled: false,
        }
    }

    fn signal(&mut self, waiters: &mut WaitRegistry) {
        self.signaled = true;
        let _ = waiters.notify(self.object, Signals::SIGNALED);
    }

    fn clear(&mut self) {
        self.signaled = false;
    }

    fn wait_item(&self, interests: Signals) -> WaitItem<'_> {
        WaitItem::event(self, interests)
    }

    pub(crate) fn wait_item_for_process<const R: usize, const A: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        process: &Process<A>,
        handle: Handle,
        interests: Signals,
    ) -> Result<WaitItem<'_>, WaitError> {
        let object = process
            .handles
            .require(registry, handle, ObjectKind::Event, Rights::WAIT)
            .map_err(WaitError::from)?;
        if object != self.object {
            return Err(WaitError::Stale);
        }
        Ok(self.wait_item(interests))
    }
}

pub struct Timer {
    object: ObjectId,
    deadline: Option<u64>,
    fired: bool,
}

impl Timer {
    pub(crate) const fn new(object: ObjectId) -> Self {
        Self {
            object,
            deadline: None,
            fired: false,
        }
    }

    fn arm(&mut self, now: u64, duration: u64) -> Result<(), WaitError> {
        self.deadline = Some(now.checked_add(duration).ok_or(WaitError::TooMany)?);
        self.fired = false;
        Ok(())
    }

    fn poll(&mut self, now: u64, waiters: &mut WaitRegistry) {
        if !self.fired && self.deadline.is_some_and(|deadline| now >= deadline) {
            self.fired = true;
            let _ = waiters.notify(self.object, Signals::FIRED);
        }
    }

    fn wait_item(&self, now: u64, interests: Signals) -> WaitItem<'_> {
        WaitItem::timer(self, now, interests)
    }

    pub(crate) fn wait_item_for_process<const R: usize, const A: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        process: &Process<A>,
        handle: Handle,
        now: u64,
        interests: Signals,
    ) -> Result<WaitItem<'_>, WaitError> {
        let object = process
            .handles
            .require(registry, handle, ObjectKind::Timer, Rights::WAIT)
            .map_err(WaitError::from)?;
        if object != self.object {
            return Err(WaitError::Stale);
        }
        Ok(self.wait_item(now, interests))
    }
}

impl<const N: usize> Process<N> {
    pub(crate) fn signal_event<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        event: &mut Event,
        waiters: &mut WaitRegistry,
    ) -> Result<(), WaitError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Event, Rights::SIGNAL)
            .map_err(WaitError::from)?;
        if object != event.object {
            return Err(WaitError::Stale);
        }
        event.signal(waiters);
        Ok(())
    }

    pub(crate) fn clear_event<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        event: &mut Event,
    ) -> Result<(), WaitError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Event, Rights::SIGNAL)
            .map_err(WaitError::from)?;
        if object != event.object {
            return Err(WaitError::Stale);
        }
        event.clear();
        Ok(())
    }

    pub(crate) fn arm_timer<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        timer: &mut Timer,
        now: u64,
        duration: u64,
    ) -> Result<(), WaitError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Timer, Rights::CONTROL)
            .map_err(WaitError::from)?;
        if object != timer.object {
            return Err(WaitError::Stale);
        }
        timer.arm(now, duration)
    }

    pub(crate) fn poll_timer<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        timer: &mut Timer,
        now: u64,
        waiters: &mut WaitRegistry,
    ) -> Result<(), WaitError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Timer, Rights::CONTROL)
            .map_err(WaitError::from)?;
        if object != timer.object {
            return Err(WaitError::Stale);
        }
        timer.poll(now, waiters);
        Ok(())
    }
}
