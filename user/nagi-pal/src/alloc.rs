use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocError {
    Exhausted,
    InvalidLayout,
}

/// A bounded monotonic allocator for an early user process.
///
/// The allocator never reuses memory. This is deliberate for the bootstrap
/// process: its fixed bound makes exhaustion diagnosable and prevents an
/// unbounded dependency on a kernel heap or host allocator.
#[repr(C, align(16))]
pub struct BumpAllocator<const SIZE: usize> {
    next: AtomicUsize,
    bytes: UnsafeCell<[u8; SIZE]>,
}

unsafe impl<const SIZE: usize> Sync for BumpAllocator<SIZE> {}

impl<const SIZE: usize> BumpAllocator<SIZE> {
    pub const fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            bytes: UnsafeCell::new([0; SIZE]),
        }
    }

    pub fn allocate(&self, layout: Layout) -> Result<*mut u8, AllocError> {
        if layout.size() == 0 || !layout.align().is_power_of_two() {
            return Err(AllocError::InvalidLayout);
        }
        let base = self.bytes.get() as usize;
        let current = self.next.load(Ordering::Relaxed);
        let aligned = base
            .checked_add(current)
            .and_then(|address| address.checked_add(layout.align() - 1))
            .map(|address| address & !(layout.align() - 1))
            .ok_or(AllocError::Exhausted)?;
        let offset = aligned.checked_sub(base).ok_or(AllocError::Exhausted)?;
        let end = offset
            .checked_add(layout.size())
            .ok_or(AllocError::Exhausted)?;
        if end > SIZE {
            return Err(AllocError::Exhausted);
        }
        self.next
            .compare_exchange(current, end, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| AllocError::Exhausted)?;
        Ok(aligned as *mut u8)
    }

    pub fn used(&self) -> usize {
        self.next.load(Ordering::Acquire).min(SIZE)
    }
}

impl<const SIZE: usize> Default for BumpAllocator<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<const SIZE: usize> GlobalAlloc for BumpAllocator<SIZE> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate(layout).unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[cfg(test)]
mod tests {
    use core::alloc::Layout;

    use super::{AllocError, BumpAllocator};

    #[test]
    fn allocation_is_aligned_and_bounded() {
        let allocator = BumpAllocator::<32>::new();
        let first = allocator
            .allocate(Layout::from_size_align(3, 8).unwrap())
            .unwrap();
        let second = allocator
            .allocate(Layout::from_size_align(8, 16).unwrap())
            .unwrap();
        assert_eq!((first as usize) % 8, 0);
        assert_eq!((second as usize) % 16, 0);
        assert!(allocator
            .allocate(Layout::from_size_align(16, 16).unwrap())
            .is_err());
        assert!(matches!(
            allocator.allocate(Layout::from_size_align(0, 1).unwrap()),
            Err(AllocError::InvalidLayout)
        ));
    }
}
