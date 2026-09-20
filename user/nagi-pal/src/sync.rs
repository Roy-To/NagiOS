use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SpinMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinMutex<T> {}

impl<T> SpinMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinGuard<'_, T> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }
        SpinGuard { mutex: self }
    }
}

pub struct SpinGuard<'a, T> {
    mutex: &'a SpinMutex<T>,
}

impl<T> core::ops::Deref for SpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> core::ops::DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

pub struct TlsSlots<const SIZE: usize> {
    values: [usize; SIZE],
    used: [bool; SIZE],
}

impl<const SIZE: usize> TlsSlots<SIZE> {
    pub const fn new() -> Self {
        Self {
            values: [0; SIZE],
            used: [false; SIZE],
        }
    }

    pub fn allocate(&mut self) -> Option<usize> {
        let index = self.used.iter().position(|used| !*used)?;
        self.used[index] = true;
        Some(index)
    }

    pub fn set(&mut self, index: usize, value: usize) -> bool {
        if !self.used.get(index).copied().unwrap_or(false) {
            return false;
        }
        self.values[index] = value;
        true
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.used
            .get(index)
            .copied()
            .filter(|used| *used)
            .map(|_| self.values[index])
    }
}

impl<const SIZE: usize> Default for TlsSlots<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SpinMutex, TlsSlots};

    #[test]
    fn mutex_and_tls_are_bounded() {
        let mutex = SpinMutex::new(1_u32);
        *mutex.lock() += 1;
        assert_eq!(*mutex.lock(), 2);

        let mut tls = TlsSlots::<1>::new();
        let key = tls.allocate().unwrap();
        assert!(tls.set(key, 42));
        assert_eq!(tls.get(key), Some(42));
        assert!(tls.allocate().is_none());
    }
}
