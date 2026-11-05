//! Host RAM model only: no PCI transport, DMA, memory-ordering or guest claim.
//! SplitQueue is storage, not an allocator: Driver below supplies a test-only
//! ownership protocol. The independent model uses absolute sequence numbers
//! and a byte image, rather than the queue's offset/index/accessor methods.
extern crate std;

use super::{Descriptor, SplitQueue, UsedElement, STORAGE_BYTES};
use std::{collections::VecDeque, panic::AssertUnwindSafe, vec, vec::Vec};

const CANARY: u8 = 0xa5;
const GUARD_BYTES: usize = 4096;
const START: u64 = 65_534;

#[repr(C, align(4096))]
struct Guarded {
    before: [u8; GUARD_BYTES],
    queue: SplitQueue,
    after: [u8; GUARD_BYTES],
}

// Compute wire locations from the split-ring format, independently of the SUT.
#[derive(Clone, Copy)]
struct Layout {
    size: usize,
    available: usize,
    used: usize,
}
impl Layout {
    fn new(size: usize) -> Self {
        let available = size * 16;
        let end = available + 4 + size * 2 + 2;
        Self {
            size,
            available,
            used: end.div_ceil(4096) * 4096,
        }
    }
    fn slot(self, sequence: u64) -> usize {
        (sequence % self.size as u64) as usize
    }
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn get16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn get32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn descriptor(bytes: &[u8], id: usize) -> Descriptor {
    let offset = id * 16;
    Descriptor {
        address: u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()),
        length: get32(bytes, offset + 8),
        flags: get16(bytes, offset + 12),
        next: get16(bytes, offset + 14),
    }
}
fn put_descriptor(bytes: &mut [u8], id: usize, value: Descriptor) {
    let offset = id * 16;
    bytes[offset..offset + 8].copy_from_slice(&value.address.to_le_bytes());
    put32(bytes, offset + 8, value.length);
    put16(bytes, offset + 12, value.flags);
    put16(bytes, offset + 14, value.next);
}

impl Guarded {
    fn new(size: usize) -> Self {
        let mut value = Self {
            before: [CANARY; GUARD_BYTES],
            queue: SplitQueue::empty(),
            after: [CANARY; GUARD_BYTES],
        };
        assert!(value.queue.initialize(size));
        // Poison unused entries, event fields, alignment gaps and trailing RAM.
        // Header counters alone start near rollover. Device writes use byte
        // offsets in this mock, never the queue's reported DMA addresses.
        value.queue.bytes.fill(CANARY);
        let layout = Layout::new(size);
        put16(&mut value.queue.bytes, layout.available + 2, START as u16);
        put16(&mut value.queue.bytes, layout.used + 2, START as u16);
        value
    }
    fn assert_matches(&self, model: &Model) {
        assert!(self.before.iter().all(|byte| *byte == CANARY));
        assert!(self.after.iter().all(|byte| *byte == CANARY));
        // Full image comparison also guards every internal region boundary,
        // including used_event/available_event, alignment gaps and RAM tail.
        assert_eq!(self.queue.bytes.as_slice(), model.bytes.as_slice());
        assert_eq!(self.queue.size(), model.layout.size);
        let base = self.queue.descriptor_address();
        assert_eq!(base % 4096, 0);
        assert_eq!(
            self.queue.available_address() - base,
            model.layout.available as u64
        );
        assert_eq!(self.queue.used_address() - base, model.layout.used as u64);
        assert_eq!(self.queue.available_index(), model.published as u16);
        assert_eq!(self.queue.used_index(), model.completed as u16);
    }
}

struct Model {
    layout: Layout,
    bytes: Vec<u8>,
    published: u64,
    seen: u64,
    completed: u64,
    reaped: u64,
    owned: Vec<Option<Descriptor>>,
    pending: Vec<(usize, Descriptor)>,
    completions: VecDeque<UsedElement>,
}
impl Model {
    fn new(size: usize) -> Self {
        let layout = Layout::new(size);
        let mut bytes = vec![CANARY; STORAGE_BYTES];
        put16(&mut bytes, layout.available + 2, START as u16);
        put16(&mut bytes, layout.used + 2, START as u16);
        Self {
            layout,
            bytes,
            published: START,
            seen: START,
            completed: START,
            reaped: START,
            owned: vec![None; size],
            pending: Vec::new(),
            completions: VecDeque::new(),
        }
    }
    fn submit(&mut self, id: usize, value: Descriptor) -> bool {
        if self.owned[id].is_some() {
            return false;
        }
        assert!(self.published - self.reaped < self.layout.size as u64);
        self.owned[id] = Some(value);
        put_descriptor(&mut self.bytes, id, value);
        let slot = self.layout.slot(self.published);
        put16(
            &mut self.bytes,
            self.layout.available + 4 + slot * 2,
            id as u16,
        );
        self.published += 1;
        put16(
            &mut self.bytes,
            self.layout.available + 2,
            self.published as u16,
        );
        true
    }
    fn take(&mut self) -> Option<(usize, Descriptor)> {
        if self.seen == self.published {
            return None;
        }
        let slot = self.layout.slot(self.seen);
        let id = get16(&self.bytes, self.layout.available + 4 + slot * 2) as usize;
        let value = self.owned[id].unwrap();
        self.pending.push((id, value));
        self.seen += 1;
        Some((id, value))
    }
    fn complete(&mut self, index: usize, length: u32) -> UsedElement {
        let (id, value) = self.pending.remove(index);
        assert!(length <= value.length);
        let result = UsedElement {
            id: id as u32,
            length,
        };
        let slot = self.layout.slot(self.completed);
        put32(&mut self.bytes, self.layout.used + 4 + slot * 8, result.id);
        put32(&mut self.bytes, self.layout.used + 8 + slot * 8, length);
        self.completed += 1;
        put16(&mut self.bytes, self.layout.used + 2, self.completed as u16);
        self.completions.push_back(result);
        result
    }
    fn reap(&mut self) -> Option<UsedElement> {
        let result = self.completions.pop_front()?;
        assert!(self.owned[result.id as usize].take().is_some());
        self.reaped += 1;
        Some(result)
    }
}

// Test-only caller/device protocol. It deliberately uses wrapping u16 counters
// and SplitQueue's accessors, unlike Model's absolute counters and byte image.
struct Driver {
    owned: Vec<bool>,
    next_used: u16,
}
impl Driver {
    fn new(size: usize) -> Self {
        Self {
            owned: vec![false; size],
            next_used: START as u16,
        }
    }
    fn submit(&mut self, queue: &mut SplitQueue, id: usize, value: Descriptor) -> bool {
        if self.owned[id] {
            return false;
        }
        queue.set_descriptor(id, value);
        let index = queue.available_index();
        queue.set_available(usize::from(index) % queue.size(), id as u16);
        queue.set_available_index(index.wrapping_add(1));
        self.owned[id] = true;
        true
    }
    fn reap(&mut self, queue: &SplitQueue) -> Option<UsedElement> {
        if self.next_used == queue.used_index() {
            return None;
        }
        let result = queue.used(usize::from(self.next_used) % queue.size());
        assert!(self.owned[result.id as usize]);
        self.owned[result.id as usize] = false;
        self.next_used = self.next_used.wrapping_add(1);
        Some(result)
    }
}
struct Device {
    next_available: u16,
    pending: Vec<(usize, Descriptor)>,
}
impl Device {
    fn new() -> Self {
        Self {
            next_available: START as u16,
            pending: Vec::new(),
        }
    }
    fn take(&mut self, queue: &SplitQueue, layout: Layout) -> Option<(usize, Descriptor)> {
        let available = get16(&queue.bytes, layout.available + 2);
        if self.next_available == available {
            return None;
        }
        assert!(usize::from(available.wrapping_sub(self.next_available)) <= layout.size);
        let slot = usize::from(self.next_available) % layout.size;
        let id = get16(&queue.bytes, layout.available + 4 + slot * 2) as usize;
        assert!(id < layout.size);
        let value = descriptor(&queue.bytes, id);
        assert!(!self.pending.iter().any(|(pending, _)| *pending == id));
        self.pending.push((id, value));
        self.next_available = self.next_available.wrapping_add(1);
        Some((id, value))
    }
    fn complete(&mut self, queue: &mut SplitQueue, layout: Layout, index: usize, length: u32) {
        let (id, value) = self.pending.remove(index);
        assert!(length <= value.length);
        let used = get16(&queue.bytes, layout.used + 2);
        let slot = usize::from(used) % layout.size;
        put32(&mut queue.bytes, layout.used + 4 + slot * 8, id as u32);
        put32(&mut queue.bytes, layout.used + 8 + slot * 8, length);
        put16(&mut queue.bytes, layout.used + 2, used.wrapping_add(1));
    }
}

fn payload(round: usize, id: usize) -> Descriptor {
    Descriptor {
        address: 0x1234_5678_0000_0000 | ((round as u64) << 20) | ((id as u64) << 4),
        length: 100 + round as u32 + id as u32,
        flags: 2,
        next: 0,
    }
}

#[test]
fn model_matches_empty_full_reordered_completions_and_reuse_for_every_size() {
    for power in 0..=10 {
        let size = 1 << power;
        let mut ram = Guarded::new(size);
        let mut model = Model::new(size);
        let mut driver = Driver::new(size);
        let mut device = Device::new();
        let layout = model.layout;
        ram.assert_matches(&model);
        assert_eq!(driver.reap(&ram.queue), model.reap());
        assert_eq!(device.take(&ram.queue, layout), model.take());

        // Existing tests exercise 70,000 publications for size 1024. Here each
        // supported size crosses u16 rollover while cycling full and empty,
        // completing in different orders, and reusing every descriptor.
        for round in 0..8 {
            for step in 0..size {
                // Reverse and rotate IDs so descriptor IDs differ from slots.
                let id = (size - 1 - step + round) % size;
                let value = payload(round, id);
                assert_eq!(
                    driver.submit(&mut ram.queue, id, value),
                    model.submit(id, value)
                );
                assert!(driver.owned[id]);
                // Live descriptors cannot be reused, even before device reads.
                assert!(!driver.submit(&mut ram.queue, id, payload(round + 100, id)));
                assert!(!model.submit(id, payload(round + 100, id)));
            }
            assert!(driver.owned.iter().all(|owned| *owned));
            assert_eq!(model.published - model.reaped, size as u64);
            ram.assert_matches(&model);
            for _ in 0..size {
                assert_eq!(device.take(&ram.queue, layout), model.take());
            }
            assert_eq!(device.take(&ram.queue, layout), None);
            assert_eq!(model.take(), None);
            // Reclaim and repost one descriptor while the others remain live.
            // This keeps occupancy full while all three ring cursors advance;
            // stale descriptor contents must not survive the ownership handoff.
            for step in 0..size {
                let index = (step * 17 + round) % size;
                let id = model.pending[index].0;
                let expected = model.complete(index, (step % 97) as u32);
                device.complete(&mut ram.queue, layout, index, (step % 97) as u32);
                assert!(!driver.submit(&mut ram.queue, id, payload(round + 100, id)));
                assert!(!model.submit(id, payload(round + 100, id)));
                assert_eq!(driver.reap(&ram.queue), Some(expected));
                assert_eq!(model.reap(), Some(expected));
                assert_eq!(driver.reap(&ram.queue), None);
                assert_eq!(model.reap(), None);
                let replacement = payload(round + 10, id);
                assert!(driver.submit(&mut ram.queue, id, replacement));
                assert!(model.submit(id, replacement));
                assert_eq!(device.take(&ram.queue, layout), model.take());
                assert_eq!(model.published - model.reaped, size as u64);
            }
            ram.assert_matches(&model);
            // Both in-order and deliberately out-of-order completion. No
            // VIRTIO_F_IN_ORDER feature is assumed by this mock protocol.
            for step in 0..size {
                let remaining = size - step;
                let index = match round % 3 {
                    0 => 0,
                    1 => remaining - 1,
                    _ => (step * 17 + round) % remaining,
                };
                let id = model.pending[index].0;
                let length = (step % 97) as u32;
                let completed = model.complete(index, length);
                assert_eq!(completed.id, id as u32);
                device.complete(&mut ram.queue, layout, index, length);
                // Completion alone does not permit reuse before driver reap.
                assert!(!driver.submit(&mut ram.queue, id, payload(round + 100, id)));
                assert!(!model.submit(id, payload(round + 100, id)));
            }
            ram.assert_matches(&model);
            for _ in 0..size {
                assert_eq!(driver.reap(&ram.queue), model.reap());
            }
            assert_eq!(driver.reap(&ram.queue), None);
            assert_eq!(model.reap(), None);
            assert!(driver.owned.iter().all(|owned| !*owned));
            assert_eq!(model.published, model.reaped);
            ram.assert_matches(&model);
        }
        assert!(model.published > u16::MAX as u64);
        assert!(model.completed > u16::MAX as u64);
    }
}

#[test]
fn invalid_slots_and_reinitialization_preserve_region_sentinels() {
    for power in 0..=10 {
        let size = 1 << power;
        let mut ram = Guarded::new(size);
        let model = Model::new(size);
        for invalid in [size, size + 1, usize::MAX] {
            assert!(std::panic::catch_unwind(AssertUnwindSafe(|| {
                ram.queue.set_descriptor(invalid, payload(0, 0));
            }))
            .is_err());
            ram.assert_matches(&model);
            assert!(std::panic::catch_unwind(AssertUnwindSafe(|| {
                ram.queue.set_available(invalid, 0);
            }))
            .is_err());
            ram.assert_matches(&model);
            assert!(std::panic::catch_unwind(AssertUnwindSafe(|| {
                ram.queue.used(invalid);
            }))
            .is_err());
            ram.assert_matches(&model);
        }
        for invalid in [0, 3, 1025, 2048, usize::MAX] {
            assert!(!ram.queue.initialize(invalid));
            ram.assert_matches(&model);
        }
        // Reinitialization clears DMA RAM but does not touch the outer guards.
        assert!(ram.queue.initialize(size));
        assert!(ram.queue.bytes.iter().all(|byte| *byte == 0));
        assert_eq!(ram.queue.size(), size);
        assert!(ram.before.iter().all(|byte| *byte == CANARY));
        assert!(ram.after.iter().all(|byte| *byte == CANARY));
    }
}
