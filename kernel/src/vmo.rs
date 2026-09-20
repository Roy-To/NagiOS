use crate::handles::{Handle, HandleError, ObjectId, ObjectKind, ObjectRegistry, Process, Rights};

pub const PAGE_SIZE: u64 = crate::memory::PAGE_SIZE;
pub const MAX_VMO_BYTES: usize = 16 * 1024;
pub const MAX_MAPPINGS: usize = 32;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmoKind {
    Anonymous,
    Shared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmoError {
    InvalidObject,
    InvalidSize,
    InvalidAddress,
    Unaligned,
    OutOfBounds,
    RightsMissing,
    Overlap,
    NotMapped,
    MappingTableFull,
}

pub struct Vmo {
    object: ObjectId,
    kind: VmoKind,
    size: u64,
    backing: [u8; MAX_VMO_BYTES],
}

impl Vmo {
    pub(crate) fn anonymous(object: ObjectId, size: u64) -> Result<Self, VmoError> {
        Self::new(object, size, VmoKind::Anonymous)
    }

    pub(crate) fn shared(object: ObjectId, size: u64) -> Result<Self, VmoError> {
        Self::new(object, size, VmoKind::Shared)
    }

    pub(crate) fn object_id(&self) -> ObjectId {
        self.object
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn kind(&self) -> VmoKind {
        self.kind
    }

    fn read(&self, offset: u64, destination: &mut [u8]) -> Result<(), VmoError> {
        let length = u64::try_from(destination.len()).map_err(|_| VmoError::OutOfBounds)?;
        let end = offset.checked_add(length).ok_or(VmoError::OutOfBounds)?;
        if end > self.size {
            return Err(VmoError::OutOfBounds);
        }
        let start = usize::try_from(offset).map_err(|_| VmoError::OutOfBounds)?;
        destination.copy_from_slice(&self.backing[start..start + destination.len()]);
        Ok(())
    }

    fn write(&mut self, offset: u64, source: &[u8]) -> Result<(), VmoError> {
        let length = u64::try_from(source.len()).map_err(|_| VmoError::OutOfBounds)?;
        let end = offset.checked_add(length).ok_or(VmoError::OutOfBounds)?;
        if end > self.size {
            return Err(VmoError::OutOfBounds);
        }
        let start = usize::try_from(offset).map_err(|_| VmoError::OutOfBounds)?;
        self.backing[start..start + source.len()].copy_from_slice(source);
        Ok(())
    }

    fn new(object: ObjectId, size: u64, kind: VmoKind) -> Result<Self, VmoError> {
        if object.kind() != ObjectKind::Vmo {
            return Err(VmoError::InvalidObject);
        }
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) || size > MAX_VMO_BYTES as u64 {
            return Err(VmoError::InvalidSize);
        }
        Ok(Self {
            object,
            kind,
            size,
            backing: [0; MAX_VMO_BYTES],
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mapping {
    pub vmo_object: ObjectId,
    pub virtual_start: u64,
    pub offset: u64,
    pub length: u64,
    pub rights: Rights,
    max_rights: Rights,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MappingRequest {
    pub(crate) virtual_start: u64,
    pub(crate) offset: u64,
    pub(crate) length: u64,
    pub(crate) rights: Rights,
}

pub struct AddressSpace {
    object: ObjectId,
    mappings: [Option<Mapping>; MAX_MAPPINGS],
}

impl AddressSpace {
    pub(crate) const fn new(object: ObjectId) -> Self {
        Self {
            object,
            mappings: [None; MAX_MAPPINGS],
        }
    }

    pub(crate) fn object_id(&self) -> ObjectId {
        self.object
    }

    fn map<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        vmo_capability: ObjectId,
        vmo: &Vmo,
        capability_rights: Rights,
        request: MappingRequest,
    ) -> Result<(), VmoError> {
        if vmo_capability != vmo.object
            || vmo_capability.kind() != ObjectKind::Vmo
            || registry.validate(vmo_capability, ObjectKind::Vmo).is_err()
        {
            return Err(VmoError::InvalidObject);
        }
        if !capability_rights.contains(Rights::MAP)
            || !request.rights.is_subset_of(capability_rights)
        {
            return Err(VmoError::RightsMissing);
        }
        validate_range(request.virtual_start, request.length)?;
        if !request.offset.is_multiple_of(PAGE_SIZE)
            || !request.length.is_multiple_of(PAGE_SIZE)
            || request
                .offset
                .checked_add(request.length)
                .ok_or(VmoError::OutOfBounds)?
                > vmo.size
        {
            return Err(VmoError::OutOfBounds);
        }
        if self.mappings.iter().flatten().any(|mapping| {
            ranges_overlap(
                request.virtual_start,
                request.length,
                mapping.virtual_start,
                mapping.length,
            )
        }) {
            return Err(VmoError::Overlap);
        }
        let Some(slot) = self.mappings.iter_mut().find(|slot| slot.is_none()) else {
            return Err(VmoError::MappingTableFull);
        };
        registry
            .retain(vmo.object)
            .map_err(|_| VmoError::InvalidObject)?;
        *slot = Some(Mapping {
            vmo_object: vmo.object,
            virtual_start: request.virtual_start,
            offset: request.offset,
            length: request.length,
            rights: request.rights,
            max_rights: request.rights,
        });
        Ok(())
    }

    fn unmap<const R: usize>(
        &mut self,
        registry: &mut ObjectRegistry<R>,
        virtual_start: u64,
    ) -> Result<Mapping, VmoError> {
        let Some(slot) = self
            .mappings
            .iter_mut()
            .find(|slot| slot.is_some_and(|mapping| mapping.virtual_start == virtual_start))
        else {
            return Err(VmoError::NotMapped);
        };
        let mapping = slot.take().ok_or(VmoError::NotMapped)?;
        if registry.release(mapping.vmo_object).is_err() {
            *slot = Some(mapping);
            return Err(VmoError::InvalidObject);
        }
        Ok(mapping)
    }

    fn protect(&mut self, virtual_start: u64, rights: Rights) -> Result<(), VmoError> {
        let Some(mapping) = self
            .mappings
            .iter_mut()
            .filter_map(Option::as_mut)
            .find(|mapping| mapping.virtual_start == virtual_start)
        else {
            return Err(VmoError::NotMapped);
        };
        if !rights.is_subset_of(mapping.max_rights) {
            return Err(VmoError::RightsMissing);
        }
        mapping.rights = rights;
        mapping.max_rights = rights;
        Ok(())
    }

    pub(crate) fn mapping(&self, virtual_start: u64) -> Option<Mapping> {
        self.mappings
            .iter()
            .filter_map(Option::as_ref)
            .find(|mapping| mapping.virtual_start == virtual_start)
            .copied()
    }
}

impl<const N: usize> Process<N> {
    fn owns_address_space<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        address_space: &AddressSpace,
    ) -> bool {
        address_space.object == self.address_space()
            && registry
                .validate(self.address_space(), ObjectKind::AddressSpace)
                .is_ok()
    }

    pub(crate) fn map_vmo<const R: usize>(
        &self,
        registry: &mut ObjectRegistry<R>,
        address_space: &mut AddressSpace,
        handle: Handle,
        vmo: &Vmo,
        request: MappingRequest,
    ) -> Result<(), VmoError> {
        if !self.owns_address_space(registry, address_space) {
            return Err(VmoError::InvalidObject);
        }
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Vmo, Rights::MAP)
            .map_err(handle_to_vmo_error)?;
        let rights = self
            .handles
            .rights(registry, handle)
            .map_err(handle_to_vmo_error)?;
        address_space.map(registry, object, vmo, rights, request)
    }

    pub(crate) fn unmap_vmo<const R: usize>(
        &self,
        registry: &mut ObjectRegistry<R>,
        address_space: &mut AddressSpace,
        handle: Handle,
        virtual_start: u64,
    ) -> Result<Mapping, VmoError> {
        if !self.owns_address_space(registry, address_space) {
            return Err(VmoError::InvalidObject);
        }
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Vmo, Rights::MAP)
            .map_err(handle_to_vmo_error)?;
        if address_space
            .mapping(virtual_start)
            .is_none_or(|mapping| mapping.vmo_object != object)
        {
            return Err(VmoError::InvalidObject);
        }
        address_space.unmap(registry, virtual_start)
    }

    pub(crate) fn protect_vmo<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        address_space: &mut AddressSpace,
        handle: Handle,
        virtual_start: u64,
        rights: Rights,
    ) -> Result<(), VmoError> {
        if !self.owns_address_space(registry, address_space) {
            return Err(VmoError::InvalidObject);
        }
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Vmo, Rights::MAP)
            .map_err(handle_to_vmo_error)?;
        if address_space
            .mapping(virtual_start)
            .is_none_or(|mapping| mapping.vmo_object != object)
        {
            return Err(VmoError::InvalidObject);
        }
        address_space.protect(virtual_start, rights)
    }

    pub(crate) fn read_vmo<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        vmo: &Vmo,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<(), VmoError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Vmo, Rights::READ)
            .map_err(handle_to_vmo_error)?;
        if object != vmo.object {
            return Err(VmoError::InvalidObject);
        }
        vmo.read(offset, destination)
    }

    pub(crate) fn write_vmo<const R: usize>(
        &self,
        registry: &ObjectRegistry<R>,
        handle: Handle,
        vmo: &mut Vmo,
        offset: u64,
        source: &[u8],
    ) -> Result<(), VmoError> {
        let object = self
            .handles
            .require(registry, handle, ObjectKind::Vmo, Rights::WRITE)
            .map_err(handle_to_vmo_error)?;
        if object != vmo.object {
            return Err(VmoError::InvalidObject);
        }
        vmo.write(offset, source)
    }
}

fn handle_to_vmo_error(error: HandleError) -> VmoError {
    match error {
        HandleError::RightsMissing => VmoError::RightsMissing,
        _ => VmoError::InvalidObject,
    }
}

fn validate_range(virtual_start: u64, length: u64) -> Result<(), VmoError> {
    if virtual_start == 0
        || !virtual_start.is_multiple_of(PAGE_SIZE)
        || length == 0
        || !length.is_multiple_of(PAGE_SIZE)
    {
        return Err(VmoError::Unaligned);
    }
    let end = virtual_start
        .checked_add(length)
        .ok_or(VmoError::InvalidAddress)?;
    if end > USER_ADDRESS_LIMIT {
        return Err(VmoError::InvalidAddress);
    }
    Ok(())
}

fn ranges_overlap(
    first_start: u64,
    first_length: u64,
    second_start: u64,
    second_length: u64,
) -> bool {
    let first_end = first_start.saturating_add(first_length);
    let second_end = second_start.saturating_add(second_length);
    first_start < second_end && second_start < first_end
}

#[cfg(test)]
mod tests {
    use crate::handles::{ObjectKind, ObjectRegistry, Process, Rights};

    use super::{AddressSpace, Vmo, VmoError, PAGE_SIZE};

    fn vmo() -> (ObjectRegistry<2>, Vmo) {
        let mut registry = ObjectRegistry::<2>::new();
        let object = registry.create(ObjectKind::Vmo).expect("object");
        let vmo = Vmo::anonymous(object, PAGE_SIZE * 2).expect("VMO");
        (registry, vmo)
    }

    #[test]
    fn maps_and_unmaps_page_aligned_vmo_range() {
        let (mut registry, vmo) = vmo();
        let mut address_space = AddressSpace::new(vmo.object_id());

        address_space
            .map(
                &mut registry,
                vmo.object_id(),
                &vmo,
                Rights::READ | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            )
            .expect("map");
        assert_eq!(registry.reference_count(vmo.object_id()), Ok(2));
        assert_eq!(
            address_space.mapping(0x4000).expect("mapping").vmo_object,
            vmo.object_id()
        );
        address_space.unmap(&mut registry, 0x4000).expect("unmap");
        assert_eq!(registry.reference_count(vmo.object_id()), Ok(1));
        registry
            .release(vmo.object_id())
            .expect("release creator reference");
        assert_eq!(
            registry.reference_count(vmo.object_id()),
            Err(crate::handles::HandleError::Stale)
        );
        assert!(address_space.mapping(0x4000).is_none());
    }

    #[test]
    fn rejects_unaligned_or_out_of_bounds_mapping() {
        let (mut registry, vmo) = vmo();
        let mut address_space = AddressSpace::new(vmo.object_id());

        assert_eq!(
            address_space.map(
                &mut registry,
                vmo.object_id(),
                &vmo,
                Rights::READ | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4001,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::Unaligned)
        );
        assert_eq!(
            address_space.map(
                &mut registry,
                vmo.object_id(),
                &vmo,
                Rights::READ | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: PAGE_SIZE,
                    length: PAGE_SIZE * 2,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::OutOfBounds)
        );
    }

    #[test]
    fn mapping_protection_can_only_be_attenuated() {
        let (mut registry, vmo) = vmo();
        let mut address_space = AddressSpace::new(vmo.object_id());
        address_space
            .map(
                &mut registry,
                vmo.object_id(),
                &vmo,
                Rights::READ | Rights::WRITE | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ | Rights::WRITE,
                },
            )
            .expect("map");

        address_space
            .protect(0x4000, Rights::READ)
            .expect("attenuate");
        assert_eq!(
            address_space.protect(0x4000, Rights::READ | Rights::WRITE),
            Err(VmoError::RightsMissing)
        );
    }

    #[test]
    fn mapping_requires_vmo_map_right_and_rejects_overlap() {
        let mut registry = ObjectRegistry::<2>::new();
        let read_only_object = registry.create(ObjectKind::Vmo).expect("object");
        let read_only = Vmo::anonymous(read_only_object, PAGE_SIZE).expect("VMO");
        let writable_object = registry.create(ObjectKind::Vmo).expect("object");
        let writable = Vmo::anonymous(writable_object, PAGE_SIZE * 2).expect("VMO");
        let mut address_space = AddressSpace::new(writable.object_id());

        assert_eq!(
            address_space.map(
                &mut registry,
                read_only.object_id(),
                &read_only,
                Rights::READ,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::RightsMissing)
        );
        address_space
            .map(
                &mut registry,
                writable.object_id(),
                &writable,
                Rights::READ | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            )
            .expect("map");
        assert_eq!(
            address_space.map(
                &mut registry,
                writable.object_id(),
                &writable,
                Rights::READ | Rights::MAP,
                super::MappingRequest {
                    virtual_start: 0x4000,
                    offset: PAGE_SIZE,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::Overlap)
        );
    }

    #[test]
    fn unmapping_an_unknown_range_is_rejected() {
        let mut registry = ObjectRegistry::<1>::new();
        let object = registry.create(ObjectKind::AddressSpace).expect("object");
        let mut address_space = AddressSpace::new(object);
        assert_eq!(
            address_space.unmap(&mut registry, 0x4000),
            Err(VmoError::NotMapped)
        );
    }

    #[test]
    fn anonymous_vmo_backing_is_zeroed() {
        let (_registry, vmo) = vmo();
        let mut bytes = [0xAA; 16];
        vmo.read(0, &mut bytes).expect("read");
        assert_eq!(bytes, [0; 16]);
    }

    #[test]
    fn shared_vmo_writes_and_reports_bounded_metadata() {
        let mut registry = ObjectRegistry::<2>::new();
        let object = registry.create(ObjectKind::Vmo).expect("object");
        let mut vmo = Vmo::shared(object, PAGE_SIZE).expect("shared VMO");
        assert_eq!(vmo.kind(), super::VmoKind::Shared);
        assert_eq!(vmo.size(), PAGE_SIZE);
        vmo.write(0, b"nagi").expect("write");
        let mut bytes = [0; 4];
        vmo.read(0, &mut bytes).expect("read");
        assert_eq!(&bytes, b"nagi");
    }

    #[test]
    fn address_space_retains_its_object_identity() {
        let mut registry = ObjectRegistry::<1>::new();
        let object = registry.create(ObjectKind::AddressSpace).expect("object");
        let address_space = AddressSpace::new(object);
        assert_eq!(address_space.object_id(), object);
    }

    #[test]
    fn process_vmo_operations_require_handle_rights() {
        let mut registry = ObjectRegistry::<4>::new();
        let address_space_object = registry
            .create(ObjectKind::AddressSpace)
            .expect("address space");
        let vmo_object = registry.create(ObjectKind::Vmo).expect("VMO");
        let mut process = Process::<2>::new(1, address_space_object);
        let read_only = process
            .handles
            .insert(&mut registry, vmo_object, Rights::READ)
            .expect("read-only handle");
        let mut vmo = Vmo::anonymous(vmo_object, PAGE_SIZE).expect("VMO");
        let mut address_space = AddressSpace::new(address_space_object);
        let map_capability = process
            .handles
            .insert(&mut registry, vmo_object, Rights::READ | Rights::MAP)
            .expect("mapping handle");

        assert_eq!(
            process.map_vmo(
                &mut registry,
                &mut address_space,
                read_only,
                &vmo,
                super::MappingRequest {
                    virtual_start: 0x8000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::RightsMissing)
        );
        assert_eq!(
            process.write_vmo(&registry, read_only, &mut vmo, 0, b"x"),
            Err(VmoError::RightsMissing)
        );

        let other_address_space_object = registry
            .create(ObjectKind::AddressSpace)
            .expect("other address space");
        let mut other_address_space = AddressSpace::new(other_address_space_object);
        assert_eq!(
            process.map_vmo(
                &mut registry,
                &mut other_address_space,
                map_capability,
                &vmo,
                super::MappingRequest {
                    virtual_start: 0x9000,
                    offset: 0,
                    length: PAGE_SIZE,
                    rights: Rights::READ,
                },
            ),
            Err(VmoError::InvalidObject)
        );
    }
}
