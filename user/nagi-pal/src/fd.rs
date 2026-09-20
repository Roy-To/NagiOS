#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FdKind {
    File(u64),
    Directory(u64),
    Socket(u64),
    Pipe(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fd(u32);

impl Fd {
    const INDEX_BITS: u32 = 16;
    const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;

    const fn new(index: usize, generation: u16) -> Self {
        Self(((generation as u32) << Self::INDEX_BITS) | index as u32)
    }

    const fn index(self) -> usize {
        (self.0 & Self::INDEX_MASK) as usize
    }

    const fn generation(self) -> u16 {
        (self.0 >> Self::INDEX_BITS) as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FdError {
    Full,
    Invalid,
    Stale,
}

#[derive(Clone, Copy)]
struct Slot {
    kind: Option<FdKind>,
    generation: u16,
}

impl Slot {
    const EMPTY: Self = Self {
        kind: None,
        generation: 1,
    };
}

pub struct FdTable<const SIZE: usize> {
    slots: [Slot; SIZE],
}

impl<const SIZE: usize> FdTable<SIZE> {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; SIZE],
        }
    }

    pub fn insert(&mut self, kind: FdKind) -> Result<Fd, FdError> {
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.kind.is_none())
            .ok_or(FdError::Full)?;
        slot.kind = Some(kind);
        Ok(Fd::new(index, slot.generation))
    }

    pub fn get(&self, fd: Fd) -> Result<FdKind, FdError> {
        let slot = self.slots.get(fd.index()).ok_or(FdError::Invalid)?;
        if slot.generation != fd.generation() {
            return Err(FdError::Stale);
        }
        slot.kind.ok_or(FdError::Invalid)
    }

    pub fn close(&mut self, fd: Fd) -> Result<FdKind, FdError> {
        let slot = self.slots.get_mut(fd.index()).ok_or(FdError::Invalid)?;
        if slot.generation != fd.generation() {
            return Err(FdError::Stale);
        }
        let kind = slot.kind.take().ok_or(FdError::Invalid)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        Ok(kind)
    }
}

impl<const SIZE: usize> Default for FdTable<SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{FdError, FdKind, FdTable};

    #[test]
    fn stale_handles_cannot_access_a_reused_slot() {
        let mut table = FdTable::<1>::new();
        let old = table.insert(FdKind::File(7)).unwrap();
        assert_eq!(table.close(old), Ok(FdKind::File(7)));
        let current = table.insert(FdKind::Socket(9)).unwrap();
        assert_eq!(table.get(old), Err(FdError::Stale));
        assert_eq!(table.get(current), Ok(FdKind::Socket(9)));
    }
}
