//! Bounded legacy split-ring layout shared by block, network and entropy drivers.
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{fence, Ordering};

const MIN_QUEUE_SIZE: usize = 256;
const MAX_QUEUE_SIZE: usize = 1024;
const QUEUE_STORAGE_BYTES: usize = 32 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Descriptor {
    pub(crate) address: u64,
    pub(crate) length: u32,
    pub(crate) flags: u16,
    pub(crate) next: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UsedElement {
    pub(crate) id: u32,
    pub(crate) length: u32,
}

#[repr(C, align(4096))]
pub(crate) struct LegacyQueue {
    pub(crate) storage: [u8; QUEUE_STORAGE_BYTES],
}

impl LegacyQueue {
    pub(crate) const fn empty() -> Self {
        Self {
            storage: [0; QUEUE_STORAGE_BYTES],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QueueLayout {
    pub(crate) size: usize,
    pub(crate) available: usize,
    pub(crate) used: usize,
}

impl QueueLayout {
    pub(crate) fn for_size(size: usize) -> Option<Self> {
        Self::for_bounded_size(size, MIN_QUEUE_SIZE)
    }

    // Entropy submits one descriptor at a time, including on QEMU's historical
    // eight-entry ring. Block/Net retain their independent 256-entry minimum.
    pub(crate) fn for_entropy_size(size: usize) -> Option<Self> {
        Self::for_bounded_size(size, 8)
    }

    fn for_bounded_size(size: usize, minimum: usize) -> Option<Self> {
        if !(minimum..=MAX_QUEUE_SIZE).contains(&size) || !size.is_power_of_two() {
            return None;
        }
        let available = size * size_of::<Descriptor>();
        // flags/index, ring entries and used_event; used ring is page aligned.
        let available_end = available + 6 + size * size_of::<u16>();
        let used = (available_end + 4095) & !4095;
        if used + 6 + size * size_of::<UsedElement>() > QUEUE_STORAGE_BYTES {
            return None;
        }
        Some(Self {
            size,
            available,
            used,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct QueueView {
    pub(crate) base: *mut u8,
    pub(crate) layout: QueueLayout,
}

impl QueueView {
    // The caller owns a page-aligned QUEUE_STORAGE_BYTES DMA region. Use raw
    // volatile accesses: the device writes the used ring independently.
    pub(crate) unsafe fn used_index(self) -> u16 {
        ptr::read_volatile(self.base.add(self.layout.used + 2).cast::<u16>())
    }

    pub(crate) unsafe fn descriptor(self, index: usize, value: Descriptor) {
        debug_assert!(index < self.layout.size);
        ptr::write_volatile(self.base.cast::<Descriptor>().add(index), value);
    }

    pub(crate) unsafe fn publish(self, head: u16) {
        debug_assert!(usize::from(head) < self.layout.size);
        let index = self.base.add(self.layout.available + 2).cast::<u16>();
        let previous = ptr::read_volatile(index);
        let slot = usize::from(previous) % self.layout.size;
        ptr::write_volatile(
            self.base
                .add(self.layout.available + 4)
                .cast::<u16>()
                .add(slot),
            head,
        );
        fence(Ordering::SeqCst);
        ptr::write_volatile(index, previous.wrapping_add(1));
        fence(Ordering::SeqCst);
    }

    pub(crate) unsafe fn completion(self, previous: u16) -> UsedElement {
        fence(Ordering::Acquire);
        ptr::read_volatile(
            self.base
                .add(self.layout.used + 4)
                .cast::<UsedElement>()
                .add(usize::from(previous) % self.layout.size),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_the_legacy_queue_size_and_used_ring_alignment() {
        let layout = QueueLayout::for_size(256).unwrap();
        assert_eq!(layout.available, 4096);
        assert_eq!(layout.used, 8192);
        assert_eq!(core::mem::align_of::<LegacyQueue>(), 4096);
        assert_eq!(core::mem::size_of::<LegacyQueue>(), 32 * 1024);
        let middle = QueueLayout::for_size(512).unwrap();
        assert_eq!((middle.available, middle.used), (8192, 12288));
    }

    #[test]
    fn queue_1024_publishes_and_reads_at_the_device_ring_addresses() {
        let mut storage = LegacyQueue::empty();
        let queue = QueueView {
            base: storage.storage.as_mut_ptr(),
            layout: QueueLayout::for_size(1024).unwrap(),
        };
        assert_eq!((queue.layout.available, queue.layout.used), (16384, 20480));
        unsafe {
            queue.descriptor(
                0,
                Descriptor {
                    address: 0x1234,
                    length: 16,
                    flags: 0,
                    next: 0,
                },
            );
            queue.publish(0);
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(16386).cast::<u16>()),
                1
            );
            // The old fixed-256 available index remains untouched: a 1024-entry
            // device never reads it, which caused the observed M7 timeout.
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(4098).cast::<u16>()),
                0
            );
            core::ptr::write_volatile(
                queue.base.add(20484).cast::<UsedElement>(),
                UsedElement { id: 0, length: 513 },
            );
            core::ptr::write_volatile(queue.base.add(20482).cast::<u16>(), 1);
            assert_eq!(queue.used_index(), 1);
            assert_eq!(queue.completion(0), UsedElement { id: 0, length: 513 });
        }
    }

    #[test]
    fn queue_indices_wrap_using_the_device_length() {
        let mut storage = LegacyQueue::empty();
        let queue = QueueView {
            base: storage.storage.as_mut_ptr(),
            layout: QueueLayout::for_size(1024).unwrap(),
        };
        unsafe {
            core::ptr::write_volatile(queue.base.add(16386).cast::<u16>(), u16::MAX);
            core::ptr::write_volatile(queue.base.add(18434).cast::<u16>(), 9);
            queue.publish(0);
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(16386).cast::<u16>()),
                0
            );
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(18434).cast::<u16>()),
                0
            );
            core::ptr::write_volatile(
                queue.base.add(28668).cast::<UsedElement>(),
                UsedElement { id: 0, length: 1 },
            );
            assert_eq!(queue.completion(u16::MAX), UsedElement { id: 0, length: 1 });
        }
    }

    #[test]
    fn bounded_receive_pool_publishes_heads_in_a_larger_ring() {
        let mut storage = LegacyQueue::empty();
        let queue = QueueView {
            base: storage.storage.as_mut_ptr(),
            layout: QueueLayout::for_size(1024).unwrap(),
        };
        unsafe {
            for head in 0..256 {
                queue.publish(head);
            }
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(16386).cast::<u16>()),
                256
            );
            for head in 0..256 {
                assert_eq!(
                    core::ptr::read_volatile(
                        queue.base.add(16388 + usize::from(head) * 2).cast::<u16>()
                    ),
                    head
                );
            }
            // Unpublished hardware slots do not create additional RX buffers.
            assert_eq!(
                core::ptr::read_volatile(queue.base.add(16900).cast::<u16>()),
                0
            );
        }
    }

    #[test]
    fn entropy_uses_device_ring_layout_without_changing_block_net_minimum() {
        for size in [8, 16, 32, 64, 128, 256, 512, 1024] {
            let layout = QueueLayout::for_entropy_size(size).unwrap();
            assert_eq!(layout.available, size * 16);
            assert_eq!(layout.used % 4096, 0);
            let mut storage = LegacyQueue::empty();
            let queue = QueueView {
                base: storage.storage.as_mut_ptr(),
                layout,
            };
            unsafe {
                // Multiple RNG requests must also wrap at the device length.
                for index in 0..size + 1 {
                    queue.publish(0);
                    ptr::write_volatile(
                        queue
                            .base
                            .add(layout.used + 4 + (index % size) * 8)
                            .cast::<UsedElement>(),
                        UsedElement { id: 0, length: 32 },
                    );
                    assert_eq!(
                        queue.completion(index as u16),
                        UsedElement { id: 0, length: 32 }
                    );
                }
                assert_eq!(
                    ptr::read_volatile(queue.base.add(layout.available + 2).cast::<u16>()),
                    (size + 1) as u16
                );
            }
        }
        for size in [0, 4, 7, 9, 1025, 2048, usize::MAX] {
            assert_eq!(QueueLayout::for_entropy_size(size), None);
        }
        assert_eq!(QueueLayout::for_size(8), None);
        assert_eq!(QueueLayout::for_size(128), None);
    }

    #[test]
    fn unsupported_device_queue_lengths_fail_closed() {
        for size in [0, 128, 255, 257, 1023, 1025, 2048, usize::MAX] {
            assert_eq!(QueueLayout::for_size(size), None);
        }
    }
}
