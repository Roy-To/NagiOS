use crate::handles::{HandleError, ObjectKind, ObjectRegistry, Process, Rights};
use crate::ipc::{
    wait, wait_many, ChannelPair, Event, MessageHeader, OutgoingMessage, Signals, Timer, WaitError,
    WaitRegistry, Waiter,
};
use crate::vmo::{AddressSpace, MappingRequest, Vmo, VmoKind, PAGE_SIZE};

const REGISTRY_CAPACITY: usize = 16;
const PROCESS_HANDLE_CAPACITY: usize = 8;

pub fn run_acceptance() -> bool {
    run_acceptance_inner().is_ok()
}

fn run_acceptance_inner() -> Result<(), ()> {
    let mut registry = ObjectRegistry::<REGISTRY_CAPACITY>::new();
    let address_space_a = registry.create(ObjectKind::AddressSpace).map_err(|_| ())?;
    let address_space_b = registry.create(ObjectKind::AddressSpace).map_err(|_| ())?;
    let mut process_a = Process::<PROCESS_HANDLE_CAPACITY>::new(1, address_space_a);
    let mut process_b = Process::<PROCESS_HANDLE_CAPACITY>::new(2, address_space_b);
    if process_a.id() == process_b.id()
        || process_a.address_space() == process_b.address_space()
        || process_a.state() != crate::handles::ProcessState::Running
    {
        return Err(());
    }

    let vmo_object = registry.create(ObjectKind::Vmo).map_err(|_| ())?;
    let mut vmo = Vmo::anonymous(vmo_object, PAGE_SIZE).map_err(|_| ())?;
    if vmo.object_id() != vmo_object || vmo.size() != PAGE_SIZE || vmo.kind() != VmoKind::Anonymous
    {
        return Err(());
    }
    let shared_object = registry.create(ObjectKind::Vmo).map_err(|_| ())?;
    let shared_vmo = Vmo::shared(shared_object, PAGE_SIZE).map_err(|_| ())?;
    if shared_vmo.kind() != VmoKind::Shared {
        return Err(());
    }
    let source = process_a
        .handles
        .insert(
            &mut registry,
            vmo_object,
            Rights::READ | Rights::WRITE | Rights::MAP | Rights::TRANSFER,
        )
        .map_err(|_| ())?;
    let mut space_a = AddressSpace::new(process_a.address_space());
    if space_a.object_id() != process_a.address_space() {
        return Err(());
    }
    process_a
        .map_vmo(
            &mut registry,
            &mut space_a,
            source,
            &vmo,
            MappingRequest {
                virtual_start: 0x20_0000,
                offset: 0,
                length: PAGE_SIZE,
                rights: Rights::READ | Rights::WRITE,
            },
        )
        .map_err(|_| ())?;
    if space_a.mapping(0x20_0000).is_none() {
        return Err(());
    }
    process_a
        .protect_vmo(&registry, &mut space_a, source, 0x20_0000, Rights::READ)
        .map_err(|_| ())?;
    if space_a.mapping(0x20_0000).map(|mapping| mapping.rights) != Some(Rights::READ) {
        return Err(());
    }
    process_a
        .unmap_vmo(&mut registry, &mut space_a, source, 0x20_0000)
        .map_err(|_| ())?;
    process_a
        .write_vmo(&registry, source, &mut vmo, 0, b"M4")
        .map_err(|_| ())?;
    let mut vmo_bytes = [0; 2];
    process_a
        .read_vmo(&registry, source, &vmo, 0, &mut vmo_bytes)
        .map_err(|_| ())?;
    if &vmo_bytes != b"M4" {
        return Err(());
    }

    let channel_object = registry.create(ObjectKind::Channel).map_err(|_| ())?;
    let endpoint_a = registry
        .create(ObjectKind::ChannelEndpoint)
        .map_err(|_| ())?;
    let endpoint_b = registry
        .create(ObjectKind::ChannelEndpoint)
        .map_err(|_| ())?;
    let mut channel = ChannelPair::new(channel_object, endpoint_a, endpoint_b).map_err(|_| ())?;
    if channel.object_id() != channel_object {
        return Err(());
    }
    let (sender_endpoint, receiver_endpoint) = channel
        .install_endpoints(
            &mut registry,
            &mut process_a,
            &mut process_b,
            Rights::READ | Rights::WRITE | Rights::WAIT,
        )
        .map_err(|_| ())?;

    let mut channel_waiters = WaitRegistry::new();
    let mut channel_waiter = Waiter::new(3);
    {
        let mut channel_items = [channel
            .wait_item_for_process(&registry, &process_b, receiver_endpoint, Signals::READABLE)
            .map_err(|_| ())?];
        if wait_many(
            &mut channel_waiters,
            &mut channel_waiter,
            &mut channel_items,
        ) != Err(WaitError::Blocked)
        {
            return Err(());
        }
    }

    let mut message = OutgoingMessage::new(MessageHeader {
        protocol_id: 0x4E47,
        version: 1,
        request_id: 0x4,
        opcode: 1,
        flags: 0,
    });
    message
        .set_payload(b"Process A -> Process B")
        .map_err(|_| ())?;
    message.add_transfer(source, Rights::READ).map_err(|_| ())?;
    channel
        .send(
            &registry,
            &mut process_a,
            sender_endpoint,
            message,
            &mut channel_waiters,
        )
        .map_err(|_| ())?;
    if !channel_waiter.sync(&mut channel_waiters) {
        return Err(());
    }
    let channel_wake = channel_waiter.take_outcome().ok_or(())?;
    if channel_wake.index != 0 || !channel_wake.observed.intersects(Signals::READABLE) {
        return Err(());
    }
    if process_a.handles.resolve(&registry, source) != Err(HandleError::Stale) {
        return Err(());
    }
    if !channel
        .wait_item_for_process(&registry, &process_b, receiver_endpoint, Signals::READABLE)
        .map_err(|_| ())?
        .is_ready()
    {
        return Err(());
    }
    let received = channel
        .receive(&mut registry, &mut process_b, receiver_endpoint)
        .map_err(|_| ())?
        .ok_or(())?;
    if received.header()
        != (MessageHeader {
            protocol_id: 0x4E47,
            version: 1,
            request_id: 0x4,
            opcode: 1,
            flags: 0,
        })
        || received.payload() != b"Process A -> Process B"
        || received.handle_count() != 1
    {
        return Err(());
    }
    let received_handle = received.handle(0).ok_or(())?;
    if process_b
        .handles
        .require(&registry, received_handle, ObjectKind::Vmo, Rights::READ)
        != Ok(vmo_object)
    {
        return Err(());
    }
    if process_b
        .handles
        .require(&registry, received_handle, ObjectKind::Vmo, Rights::WRITE)
        != Err(HandleError::RightsMissing)
    {
        return Err(());
    }
    channel.drain(&mut registry).map_err(|_| ())?;

    let event_object = registry.create(ObjectKind::Event).map_err(|_| ())?;
    let timer_object = registry.create(ObjectKind::Timer).map_err(|_| ())?;
    let event_handle = process_a
        .handles
        .insert(&mut registry, event_object, Rights::WAIT | Rights::SIGNAL)
        .map_err(|_| ())?;
    let timer_handle = process_a
        .handles
        .insert(&mut registry, timer_object, Rights::WAIT | Rights::CONTROL)
        .map_err(|_| ())?;
    let mut event = Event::new(event_object);
    let mut timer = Timer::new(timer_object);
    process_a
        .arm_timer(&registry, timer_handle, &mut timer, 100, 5)
        .map_err(|_| ())?;
    let mut waiters = WaitRegistry::new();
    let mut waiter = Waiter::new(4);
    {
        let mut initial_items = [
            event
                .wait_item_for_process(&registry, &process_a, event_handle, Signals::SIGNALED)
                .map_err(|_| ())?,
            timer
                .wait_item_for_process(&registry, &process_a, timer_handle, 100, Signals::FIRED)
                .map_err(|_| ())?,
        ];
        if wait_many(&mut waiters, &mut waiter, &mut initial_items) != Err(WaitError::Blocked) {
            return Err(());
        }
    }
    process_a
        .signal_event(&registry, event_handle, &mut event, &mut waiters)
        .map_err(|_| ())?;
    if !waiter.sync(&mut waiters) {
        return Err(());
    }
    if !waiter.is_runnable() {
        return Err(());
    }
    {
        let mut event_ready = [
            event
                .wait_item_for_process(&registry, &process_a, event_handle, Signals::SIGNALED)
                .map_err(|_| ())?,
            timer
                .wait_item_for_process(&registry, &process_a, timer_handle, 100, Signals::FIRED)
                .map_err(|_| ())?,
        ];
        if wait_many(&mut waiters, &mut waiter, &mut event_ready)
            .map_err(|_| ())?
            .index
            != 0
        {
            return Err(());
        }
    }
    process_a
        .clear_event(&registry, event_handle, &mut event)
        .map_err(|_| ())?;
    if event
        .wait_item_for_process(&registry, &process_a, event_handle, Signals::SIGNALED)
        .map_err(|_| ())?
        .is_ready()
    {
        return Err(());
    }
    process_a
        .poll_timer(&registry, timer_handle, &mut timer, 105, &mut waiters)
        .map_err(|_| ())?;
    if !timer
        .wait_item_for_process(&registry, &process_a, timer_handle, 105, Signals::FIRED)
        .map_err(|_| ())?
        .is_ready()
    {
        return Err(());
    }
    {
        let mut timer_ready = [
            event
                .wait_item_for_process(&registry, &process_a, event_handle, Signals::SIGNALED)
                .map_err(|_| ())?,
            timer
                .wait_item_for_process(&registry, &process_a, timer_handle, 105, Signals::FIRED)
                .map_err(|_| ())?,
        ];
        if wait_many(&mut waiters, &mut waiter, &mut timer_ready)
            .map_err(|_| ())?
            .index
            != 1
        {
            return Err(());
        }
    }
    let timer_item = timer
        .wait_item_for_process(&registry, &process_a, timer_handle, 105, Signals::FIRED)
        .map_err(|_| ())?;
    if wait(&mut waiters, &mut waiter, timer_item)
        .map_err(|_| ())?
        .index
        != 0
    {
        return Err(());
    }

    Ok(())
}
