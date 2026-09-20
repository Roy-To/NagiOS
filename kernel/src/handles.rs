use core::ops::{BitOr, BitOrAssign};

pub const DEFAULT_HANDLE_CAPACITY: usize = 32;
pub const DEFAULT_OBJECT_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Rights(u32);

impl Rights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const MAP: Self = Self(1 << 2);
    pub const TRANSFER: Self = Self(1 << 3);
    pub const CONTROL: Self = Self(1 << 4);
    pub const DUPLICATE: Self = Self(1 << 5);
    pub const WAIT: Self = Self(1 << 6);
    pub const SIGNAL: Self = Self(1 << 7);
    pub const EXECUTE: Self = Self(1 << 8);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn is_subset_of(self, available: Self) -> bool {
        available.contains(self)
    }
}

impl BitOr for Rights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ObjectKind {
    Process,
    AddressSpace,
    Vmo,
    Channel,
    ChannelEndpoint,
    Event,
    Timer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId {
    slot: u32,
    generation: u32,
    kind: ObjectKind,
}

impl ObjectId {
    pub const fn kind(self) -> ObjectKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandleError {
    Invalid,
    Stale,
    Closed,
    WrongType,
    RightsMissing,
    TableFull,
    ReceiverFull,
    GenerationExhausted,
    ObjectTableFull,
    ObjectReferenceOverflow,
}

#[derive(Clone, Copy)]
struct ObjectSlot {
    generation: u32,
    kind: Option<ObjectKind>,
    references: u32,
    retired: bool,
}

impl ObjectSlot {
    const EMPTY: Self = Self {
        generation: 1,
        kind: None,
        references: 0,
        retired: false,
    };
}

pub struct ObjectRegistry<const N: usize = DEFAULT_OBJECT_CAPACITY> {
    slots: [ObjectSlot; N],
}

impl<const N: usize> ObjectRegistry<N> {
    pub const fn new() -> Self {
        Self {
            slots: [ObjectSlot::EMPTY; N],
        }
    }

    pub fn create(&mut self, kind: ObjectKind) -> Result<ObjectId, HandleError> {
        let Some((index, slot)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.kind.is_none() && !slot.retired)
        else {
            return Err(HandleError::ObjectTableFull);
        };
        slot.kind = Some(kind);
        // The creator owns one reference and must release it after publishing
        // the typed object to the object manager.
        slot.references = 1;
        Ok(ObjectId {
            slot: index as u32,
            generation: slot.generation,
            kind,
        })
    }

    pub fn retain(&mut self, object: ObjectId) -> Result<(), HandleError> {
        let slot = self.object_slot_mut(object)?;
        if slot.references == u32::MAX {
            return Err(HandleError::ObjectReferenceOverflow);
        }
        slot.references += 1;
        Ok(())
    }

    pub fn release(&mut self, object: ObjectId) -> Result<(), HandleError> {
        let slot = self.object_slot_mut(object)?;
        if slot.references == 0 {
            return Err(HandleError::Stale);
        }
        slot.references -= 1;
        if slot.references == 0 {
            slot.kind = None;
            if slot.generation == u32::MAX {
                slot.retired = true;
            } else {
                slot.generation += 1;
            }
        }
        Ok(())
    }

    pub fn validate(&self, object: ObjectId, expected: ObjectKind) -> Result<(), HandleError> {
        if object.kind != expected {
            return Err(HandleError::WrongType);
        }
        let slot = self
            .slots
            .get(object.slot as usize)
            .ok_or(HandleError::Invalid)?;
        if slot.generation != object.generation || slot.kind != Some(expected) {
            return Err(HandleError::Stale);
        }
        Ok(())
    }

    pub fn reference_count(&self, object: ObjectId) -> Result<u32, HandleError> {
        let slot = self.object_slot(object)?;
        Ok(slot.references)
    }

    fn object_slot(&self, object: ObjectId) -> Result<&ObjectSlot, HandleError> {
        if object.generation == 0 {
            return Err(HandleError::Invalid);
        }
        let slot = self
            .slots
            .get(object.slot as usize)
            .ok_or(HandleError::Invalid)?;
        if slot.generation != object.generation || slot.kind != Some(object.kind) {
            return Err(HandleError::Stale);
        }
        Ok(slot)
    }

    fn object_slot_mut(&mut self, object: ObjectId) -> Result<&mut ObjectSlot, HandleError> {
        if object.generation == 0 {
            return Err(HandleError::Invalid);
        }
        let slot = self
            .slots
            .get_mut(object.slot as usize)
            .ok_or(HandleError::Invalid)?;
        if slot.generation != object.generation || slot.kind != Some(object.kind) {
            return Err(HandleError::Stale);
        }
        Ok(slot)
    }
}

impl<const N: usize> Default for ObjectRegistry<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Handle(u64);

impl Handle {
    const SLOT_MASK: u64 = u32::MAX as u64;

    const fn new(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn slot(self) -> u32 {
        (self.0 & Self::SLOT_MASK) as u32
    }

    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Capability {
    pub(crate) object: ObjectId,
    pub(crate) rights: Rights,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TransferToken {
    pub(crate) capability: Capability,
}

#[derive(Clone, Copy)]
struct HandleSlot {
    generation: u32,
    capability: Option<Capability>,
}

impl HandleSlot {
    const EMPTY: Self = Self {
        generation: 1,
        capability: None,
    };
}

pub struct HandleTable<const N: usize = DEFAULT_HANDLE_CAPACITY> {
    slots: [HandleSlot; N],
}

impl<const N: usize> HandleTable<N> {
    pub const fn new() -> Self {
        Self {
            slots: [HandleSlot::EMPTY; N],
        }
    }

    pub fn insert<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        object: ObjectId,
        rights: Rights,
    ) -> Result<Handle, HandleError> {
        registry.retain(object)?;
        match self.install_capability(object, rights) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                let _ = registry.release(object);
                Err(error)
            }
        }
    }

    pub fn resolve<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
    ) -> Result<ObjectId, HandleError> {
        Ok(self.entry(registry, handle)?.object)
    }

    pub fn rights<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
    ) -> Result<Rights, HandleError> {
        Ok(self.entry(registry, handle)?.rights)
    }

    pub fn require<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        expected: ObjectKind,
        required: Rights,
    ) -> Result<ObjectId, HandleError> {
        let entry = self.entry(registry, handle)?;
        if entry.object.kind != expected {
            return Err(HandleError::WrongType);
        }
        if !entry.rights.contains(required) {
            return Err(HandleError::RightsMissing);
        }
        Ok(entry.object)
    }

    pub fn close<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        handle: Handle,
    ) -> Result<(), HandleError> {
        let slot = self.slot_mut(handle)?;
        let capability = slot.capability.ok_or(HandleError::Closed)?;
        if slot.generation == u32::MAX {
            return Err(HandleError::GenerationExhausted);
        }
        registry.release(capability.object)?;
        slot.capability = None;
        slot.generation += 1;
        Ok(())
    }

    pub(crate) fn begin_move<const R: usize>(
        &mut self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        requested: Rights,
    ) -> Result<TransferToken, HandleError> {
        self.check_move(registry, handle, requested)?;
        let slot = self.slot_mut(handle)?;
        let capability = slot.capability.ok_or(HandleError::Closed)?;
        if slot.generation == u32::MAX {
            return Err(HandleError::GenerationExhausted);
        }
        slot.capability = None;
        slot.generation += 1;
        Ok(TransferToken {
            capability: Capability {
                object: capability.object,
                rights: requested,
            },
        })
    }

    pub(crate) fn check_move<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        requested: Rights,
    ) -> Result<(), HandleError> {
        let capability = self.entry(registry, handle)?;
        if !capability.rights.contains(Rights::TRANSFER)
            || !requested.is_subset_of(capability.rights)
        {
            return Err(HandleError::RightsMissing);
        }
        let slot = self.slot(handle)?;
        if slot.generation == u32::MAX {
            return Err(HandleError::GenerationExhausted);
        }
        Ok(())
    }

    pub(crate) fn install_token<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        token: &mut Option<TransferToken>,
    ) -> Result<Handle, HandleError> {
        let capability = token.as_ref().ok_or(HandleError::Stale)?.capability;
        registry.validate(capability.object, capability.object.kind())?;
        let handle = self.install_capability(capability.object, capability.rights)?;
        let _ = token.take();
        Ok(handle)
    }

    pub(crate) fn rollback_install(&mut self, handle: Handle) -> Option<TransferToken> {
        let slot = self.slot_mut(handle).ok()?;
        let capability = slot.capability.take()?;
        if slot.generation == u32::MAX {
            slot.capability = Some(capability);
            return None;
        }
        slot.generation += 1;
        Some(TransferToken { capability })
    }

    pub(crate) fn has_capacity(&self, additional: usize) -> bool {
        self.slots
            .iter()
            .filter(|slot| slot.capability.is_none() && slot.generation != u32::MAX)
            .count()
            >= additional
    }

    fn install_capability(
        &mut self,
        object: ObjectId,
        rights: Rights,
    ) -> Result<Handle, HandleError> {
        let Some((index, slot)) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.capability.is_none() && slot.generation != u32::MAX)
        else {
            return if self.slots.iter().any(|slot| slot.capability.is_none()) {
                Err(HandleError::GenerationExhausted)
            } else {
                Err(HandleError::TableFull)
            };
        };
        slot.capability = Some(Capability { object, rights });
        Ok(Handle::new(index, slot.generation))
    }

    fn entry<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
    ) -> Result<Capability, HandleError> {
        let slot = self.slot(handle)?;
        let capability = slot.capability.ok_or(HandleError::Closed)?;
        registry.validate(capability.object, capability.object.kind())?;
        Ok(capability)
    }

    fn slot(&self, handle: Handle) -> Result<&HandleSlot, HandleError> {
        let slot = self
            .slots
            .get(handle.slot() as usize)
            .ok_or(HandleError::Invalid)?;
        if handle.generation() == 0 || slot.generation != handle.generation() {
            return Err(HandleError::Stale);
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, handle: Handle) -> Result<&mut HandleSlot, HandleError> {
        let slot = self
            .slots
            .get_mut(handle.slot() as usize)
            .ok_or(HandleError::Invalid)?;
        if handle.generation() == 0 || slot.generation != handle.generation() {
            return Err(HandleError::Stale);
        }
        Ok(slot)
    }
}

impl<const N: usize> Default for HandleTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Running,
    Exited,
}

pub struct Process<const N: usize = DEFAULT_HANDLE_CAPACITY> {
    id: u32,
    address_space: ObjectId,
    pub handles: HandleTable<N>,
    state: ProcessState,
}

impl<const N: usize> Process<N> {
    pub const fn new(id: u32, address_space: ObjectId) -> Self {
        Self {
            id,
            address_space,
            handles: HandleTable::new(),
            state: ProcessState::Running,
        }
    }

    pub const fn id(&self) -> u32 {
        self.id
    }

    pub const fn address_space(&self) -> ObjectId {
        self.address_space
    }

    pub const fn state(&self) -> ProcessState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::{Handle, HandleError, HandleTable, ObjectKind, ObjectRegistry, Rights};

    #[test]
    fn handle_round_trip_resolves_the_stored_object_and_rights() {
        let mut registry = ObjectRegistry::<2>::new();
        let object = registry.create(ObjectKind::Vmo).expect("object");
        let mut table = HandleTable::<2>::new();
        let rights = Rights::READ | Rights::WRITE | Rights::TRANSFER;
        let handle = table.insert(&mut registry, object, rights).expect("handle");

        assert_eq!(handle.slot(), 0);
        assert_ne!(handle.generation(), 0);
        assert_eq!(table.resolve(&registry, handle).expect("object"), object);
        assert_eq!(table.rights(&registry, handle).expect("rights"), rights);
    }

    #[test]
    fn closing_and_reusing_a_slot_rejects_the_stale_handle() {
        let mut registry = ObjectRegistry::<2>::new();
        let first = registry.create(ObjectKind::Vmo).expect("first object");
        let second = registry.create(ObjectKind::Event).expect("second object");
        let mut table = HandleTable::<1>::new();
        let old = table
            .insert(&mut registry, first, Rights::READ)
            .expect("old handle");
        table.close(&mut registry, old).expect("close");
        let new = table
            .insert(&mut registry, second, Rights::READ)
            .expect("new handle");

        assert_ne!(old, new);
        assert_eq!(table.resolve(&registry, old), Err(HandleError::Stale));
        assert_eq!(table.resolve(&registry, new), Ok(second));
    }

    #[test]
    fn generation_exhaustion_does_not_release_a_live_capability() {
        let mut registry = ObjectRegistry::<1>::new();
        let object = registry.create(ObjectKind::Vmo).expect("object");
        let mut table = HandleTable::<1>::new();
        let handle = table
            .insert(&mut registry, object, Rights::READ)
            .expect("handle");
        registry.release(object).expect("release creator reference");
        let exhausted = Handle::from_raw((u64::from(u32::MAX) << 32) | u64::from(handle.slot()));
        table.slots[handle.slot() as usize].generation = u32::MAX;

        assert_eq!(
            table.close(&mut registry, exhausted),
            Err(HandleError::GenerationExhausted)
        );
        assert_eq!(table.resolve(&registry, exhausted), Ok(object));
        assert_eq!(registry.reference_count(object), Ok(1));
    }

    #[test]
    fn transfer_can_only_attenuate_rights() {
        let mut registry = ObjectRegistry::<2>::new();
        let object = registry.create(ObjectKind::Vmo).expect("object");
        let mut sender = HandleTable::<2>::new();
        let mut receiver = HandleTable::<2>::new();
        let source = sender
            .insert(
                &mut registry,
                object,
                Rights::READ | Rights::WRITE | Rights::TRANSFER,
            )
            .expect("source");

        let token = sender
            .begin_move(&registry, source, Rights::READ)
            .expect("read-only move");
        let mut token = Some(token);
        let read_only = receiver
            .install_token(&mut registry, &mut token)
            .expect("read-only install");
        assert!(token.is_none());
        assert_eq!(
            receiver.install_token(&mut registry, &mut token),
            Err(HandleError::Stale)
        );
        let reclaimed = receiver
            .rollback_install(read_only)
            .expect("reclaim prepared handle");
        assert_eq!(reclaimed.capability.object, object);
        let mut reclaimed = Some(reclaimed);
        let restored = receiver
            .install_token(&mut registry, &mut reclaimed)
            .expect("restore escrow token");
        assert_eq!(receiver.resolve(&registry, restored), Ok(object));
        assert_eq!(
            receiver.require(&registry, restored, ObjectKind::Vmo, Rights::READ),
            Ok(object)
        );
        assert_eq!(
            receiver.require(&registry, restored, ObjectKind::Vmo, Rights::WRITE),
            Err(HandleError::RightsMissing)
        );
    }

    #[test]
    fn a_non_transferable_handle_cannot_cross_process_boundary() {
        let mut registry = ObjectRegistry::<1>::new();
        let object = registry.create(ObjectKind::Event).expect("object");
        let mut sender = HandleTable::<1>::new();
        let receiver = HandleTable::<1>::new();
        let source = sender
            .insert(&mut registry, object, Rights::READ)
            .expect("source");

        assert_eq!(
            sender.begin_move(&registry, source, Rights::READ),
            Err(HandleError::RightsMissing)
        );
        assert!(receiver.has_capacity(1));
    }
}

#[cfg(test)]
mod object_security_tests {
    use super::{HandleError, HandleTable, ObjectKind, ObjectRegistry, Rights};

    #[test]
    fn object_generation_changes_after_last_reference_is_released() {
        let mut registry = ObjectRegistry::<1>::new();
        let first = registry.create(ObjectKind::Vmo).expect("first object");
        registry.retain(first).expect("additional reference");
        registry.release(first).expect("release");
        registry.release(first).expect("release creator reference");
        let second = registry.create(ObjectKind::Event).expect("second object");

        assert_ne!(first, second);
        assert_eq!(
            registry.validate(first, ObjectKind::Vmo),
            Err(HandleError::Stale)
        );
    }

    #[test]
    fn resolving_with_the_wrong_object_kind_fails_closed() {
        let mut registry = ObjectRegistry::<1>::new();
        let mut table = HandleTable::<1>::new();
        let object = registry.create(ObjectKind::Event).expect("object");
        let handle = table
            .insert(&mut registry, object, Rights::READ)
            .expect("handle");

        assert_eq!(
            table.require(&registry, handle, ObjectKind::Vmo, Rights::READ),
            Err(HandleError::WrongType)
        );
    }
}
