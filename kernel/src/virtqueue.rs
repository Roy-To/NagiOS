//! Bounded split-ring storage for legacy PCI queues and negotiated modern queues.
//! Legacy queue size is device-owned: its descriptor and ring offsets must all
//! use the advertised count, even when the driver posts fewer buffers.
//! QEMU 10.0.13 publishes 1024 entries for every queue (virtio_add_queue).
use core::{mem::size_of, ptr};

pub const MAX_QUEUE_SIZE: usize = 1024;
const ALIGN: usize = 4096;
const STORAGE_BYTES: usize = 32 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Descriptor {
    pub address: u64,
    pub length: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UsedElement {
    pub id: u32,
    pub length: u32,
}

#[repr(C, align(4096))]
pub(crate) struct SplitQueue {
    // Keep DMA storage first; metadata is never exposed to the device.
    bytes: [u8; STORAGE_BYTES],
    size: usize,
}

impl SplitQueue {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; STORAGE_BYTES],
            size: 0,
        }
    }
    pub fn supports(size: usize) -> bool {
        size.is_power_of_two() && size <= MAX_QUEUE_SIZE
    }
    pub fn initialize(&mut self, size: usize) -> bool {
        if !Self::supports(size) {
            return false;
        }
        self.bytes.fill(0);
        self.size = size;
        true
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn available_offset(&self) -> usize {
        size_of::<Descriptor>() * self.size
    }
    pub fn used_offset(&self) -> usize {
        (self.available_offset() + 6 + 2 * self.size + ALIGN - 1) & !(ALIGN - 1)
    }
    pub fn descriptor_address(&self) -> u64 {
        self.bytes.as_ptr() as u64
    }
    pub fn available_address(&self) -> u64 {
        self.descriptor_address() + self.available_offset() as u64
    }
    pub fn used_address(&self) -> u64 {
        self.descriptor_address() + self.used_offset() as u64
    }
    pub fn set_descriptor(&mut self, index: usize, descriptor: Descriptor) {
        assert!(index < self.size);
        unsafe {
            ptr::write_volatile(
                self.bytes.as_mut_ptr().cast::<Descriptor>().add(index),
                descriptor,
            );
        }
    }
    pub fn available_index(&self) -> u16 {
        unsafe {
            ptr::read_volatile(
                self.bytes
                    .as_ptr()
                    .add(self.available_offset() + 2)
                    .cast::<u16>(),
            )
        }
    }
    pub fn set_available_index(&mut self, index: u16) {
        unsafe {
            ptr::write_volatile(
                self.bytes
                    .as_mut_ptr()
                    .add(self.available_offset() + 2)
                    .cast::<u16>(),
                index,
            );
        }
    }
    pub fn set_available(&mut self, slot: usize, id: u16) {
        assert!(slot < self.size);
        unsafe {
            ptr::write_volatile(
                self.bytes
                    .as_mut_ptr()
                    .add(self.available_offset() + 4 + slot * 2)
                    .cast::<u16>(),
                id,
            );
        }
    }
    pub fn used_index(&self) -> u16 {
        unsafe {
            ptr::read_volatile(
                self.bytes
                    .as_ptr()
                    .add(self.used_offset() + 2)
                    .cast::<u16>(),
            )
        }
    }
    pub fn used(&self, slot: usize) -> UsedElement {
        assert!(slot < self.size);
        unsafe {
            ptr::read_volatile(
                self.bytes
                    .as_ptr()
                    .add(self.used_offset() + 4)
                    .cast::<UsedElement>()
                    .add(slot),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_follow_the_advertised_size() {
        for (size, available, used) in [
            (8, 128, 4096),
            (64, 1024, 4096),
            (256, 4096, 8192),
            (1024, 16384, 20480),
        ] {
            let mut queue = SplitQueue::empty();
            assert!(queue.initialize(size));
            assert_eq!(queue.descriptor_address() % 4096, 0);
            assert_eq!(queue.available_offset(), available);
            assert_eq!(queue.used_offset(), used);
            assert!(used + 6 + size * 8 <= STORAGE_BYTES);
            queue.set_available(size - 1, 7);
            queue.set_available_index(u16::MAX);
            assert_eq!(queue.available_index(), u16::MAX);
            queue.set_available_index(queue.available_index().wrapping_add(1));
            assert_eq!(queue.available_index(), 0);
            assert_eq!(queue.used_index(), 0);
        }
    }
    #[test]
    fn rejects_unbounded_or_non_power_of_two_sizes() {
        for size in [0, 3, 255, 1025, 2048, usize::MAX] {
            assert!(!SplitQueue::supports(size));
        }
    }
    #[test]
    fn ring_wrap_uses_device_size_and_survives_index_rollover() {
        let mut queue = SplitQueue::empty();
        assert!(queue.initialize(1024));
        for step in 0..70_000_u32 {
            let index = queue.available_index();
            assert_eq!(index, step as u16);
            let slot = usize::from(index) % queue.size();
            queue.set_available(slot, 2);
            unsafe {
                let address = queue.available_address() + 4 + (slot * 2) as u64;
                assert_eq!(ptr::read_volatile(address as *const u16), 2);
            }
            queue.set_available_index(index.wrapping_add(1));
        }
        assert_eq!(queue.size(), 1024);
    }

    #[test]
    fn device_completion_uses_the_advertised_used_ring() {
        let mut queue = SplitQueue::empty();
        assert!(queue.initialize(1024));
        unsafe {
            ptr::write_volatile(
                (queue.used_address() + 4) as *mut UsedElement,
                UsedElement { id: 2, length: 513 },
            );
            ptr::write_volatile((queue.used_address() + 2) as *mut u16, 1);
        }
        assert_eq!(queue.used_index(), 1);
        assert_eq!(queue.used(0), UsedElement { id: 2, length: 513 });
    }
}

#[cfg(test)]
#[path = "virtqueue_model_tests.rs"]
mod model_tests;
