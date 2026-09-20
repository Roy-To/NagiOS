use core::cell::UnsafeCell;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nagi_bootinfo::{BootInfo, BootInfoError};

use crate::display::{self, SURFACE_PAGE_COUNT, USER_SURFACE_BASE};
use crate::memory::{
    current_cr3, identity_mapped, PageTable, PageTableEntry, PAGE_SIZE, PAGE_TABLE_ENTRIES,
};
use crate::user_elf::{
    self, UserElfError, UserLoadPlan, PF_W, PF_X, USER_IMAGE_BASE, USER_IMAGE_LIMIT,
};

#[cfg(test)]
#[path = "syscall.rs"]
mod syscall;

pub const USER_STACK_BASE: u64 = USER_IMAGE_BASE + 0x0020_0000;
// M14's bounded audio service keeps a PCM capture buffer and mixer work area
// in the init process. Keep the native user stack large enough for that real
// service path without exposing an unbounded stack allocation mechanism.
pub const USER_STACK_PAGES: usize = 8;
pub const USER_STACK_LIMIT: u64 = USER_STACK_BASE + USER_STACK_PAGES as u64 * PAGE_SIZE;
pub const USER_TLS_BASE: u64 = USER_IMAGE_BASE + 0x0040_0000;
pub const USER_MMAP_BASE: u64 = USER_IMAGE_BASE + 0x0080_0000;
const USER_MMAP_PAGE_TABLES: usize = 8;
pub const USER_MMAP_PAGES: usize = PAGE_TABLE_ENTRIES * USER_MMAP_PAGE_TABLES;
pub const USER_MMAP_LIMIT: u64 = USER_MMAP_BASE + USER_MMAP_PAGES as u64 * PAGE_SIZE;
pub const USER_SURFACE_LIMIT: u64 = USER_SURFACE_BASE + SURFACE_PAGE_COUNT as u64 * PAGE_SIZE;
const USER_PML4_INDEX: usize = 128;
const MAX_IMAGE_PAGES: usize = 256;
const MAX_INIT_IMAGE_SIZE: usize = 4 * 1024 * 1024;
const MAX_MMAP_REGIONS: usize = 4;
const IA32_EFER: u32 = 0xC000_0080;
const IA32_FS_BASE: u32 = 0xC000_0100;
const EFER_NXE: u64 = 1 << 11;
const USER_DATA_SELECTOR: u64 = 0x18 | 3;
const USER_CODE_SELECTOR: u64 = 0x20 | 3;
// The bootstrap user process keeps asynchronous interrupts disabled. The
// low-level sleep syscall briefly enables interrupts in its kernel wait loop,
// so timer wakeups do not introduce an unprepared user interrupt path.
const USER_INITIAL_RFLAGS: u64 = 1 << 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserContext {
    pub entry: u64,
    pub user_stack_top: u64,
    pub user_tls_base: u64,
    pub cr3: u64,
    pub block_capability: u64,
    pub display_capability: u64,
    pub input_capability: u64,
    pub net_capability: u64,
    pub audio_capability: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MmapRegion {
    start_page: usize,
    page_count: usize,
    protection: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserProcessError {
    InvalidBootInfo(BootInfoError),
    ImageTooLarge,
    ImageNotMapped,
    InvalidElf(UserElfError),
    InvalidLoadPlan,
    ImageOutOfBounds,
    KernelUserSlotOccupied,
    InvalidPhysicalAddress,
}

#[repr(C, align(4096))]
struct PageBytes([u8; PAGE_SIZE as usize]);

impl PageBytes {
    const fn zeroed() -> Self {
        Self([0; PAGE_SIZE as usize])
    }
}

#[repr(C, align(4096))]
pub(crate) struct BootstrapStorage {
    pub(crate) pml4: PageTable,
    pdpt: PageTable,
    pd: PageTable,
    pub(crate) image_pt: PageTable,
    pub(crate) stack_pt: PageTable,
    pub(crate) tls_pt: PageTable,
    mmap_pts: [PageTable; USER_MMAP_PAGE_TABLES],
    surface_pt: PageTable,
    image_pages: [PageBytes; MAX_IMAGE_PAGES],
    stack_pages: [PageBytes; USER_STACK_PAGES],
    tls_page: PageBytes,
    mmap_pages: [PageBytes; USER_MMAP_PAGES],
    mmap_regions: [Option<MmapRegion>; MAX_MMAP_REGIONS],
}

impl BootstrapStorage {
    pub(crate) const fn new() -> Self {
        Self {
            pml4: PageTable::empty(),
            pdpt: PageTable::empty(),
            pd: PageTable::empty(),
            image_pt: PageTable::empty(),
            stack_pt: PageTable::empty(),
            tls_pt: PageTable::empty(),
            mmap_pts: [const { PageTable::empty() }; USER_MMAP_PAGE_TABLES],
            surface_pt: PageTable::empty(),
            image_pages: [const { PageBytes::zeroed() }; MAX_IMAGE_PAGES],
            stack_pages: [const { PageBytes::zeroed() }; USER_STACK_PAGES],
            tls_page: PageBytes::zeroed(),
            mmap_pages: [const { PageBytes::zeroed() }; USER_MMAP_PAGES],
            mmap_regions: [None; MAX_MMAP_REGIONS],
        }
    }

    fn clear(&mut self) {
        self.pml4.clear();
        self.pdpt.clear();
        self.pd.clear();
        self.image_pt.clear();
        self.stack_pt.clear();
        self.tls_pt.clear();
        for table in &mut self.mmap_pts {
            table.clear();
        }
        self.surface_pt.clear();
        for page in &mut self.image_pages {
            page.0.fill(0);
        }
        for page in &mut self.stack_pages {
            page.0.fill(0);
        }
        self.tls_page.0.fill(0);
        for page in &mut self.mmap_pages {
            page.0.fill(0);
        }
        self.mmap_regions.fill(None);
    }
}

struct BootstrapCell(UnsafeCell<BootstrapStorage>);

unsafe impl Sync for BootstrapCell {}

static BOOTSTRAP_STORAGE: BootstrapCell = BootstrapCell(UnsafeCell::new(BootstrapStorage::new()));
static BOOTSTRAP_IN_USE: AtomicBool = AtomicBool::new(false);
static CURRENT_IMAGE_PAGES: AtomicUsize = AtomicUsize::new(0);

/// Validate and prepare the one-shot M5 process image.
///
/// The loader leaves the active address space identity mapped, so the CR3
/// value and kernel static-storage addresses are physical=virtual addresses
/// in this bootstrap path. The init image must pass `identity_mapped` before
/// this function forms a slice and dereferences its bytes.
pub fn prepare(boot_info: &BootInfo) -> Result<UserContext, UserProcessError> {
    boot_info
        .validate_for_user_bootstrap()
        .map_err(UserProcessError::InvalidBootInfo)?;
    let image_size =
        usize::try_from(boot_info.init_image.size).map_err(|_| UserProcessError::ImageTooLarge)?;
    if image_size > MAX_INIT_IMAGE_SIZE {
        return Err(UserProcessError::ImageTooLarge);
    }
    boot_info
        .init_image
        .address
        .checked_add(boot_info.init_image.size)
        .ok_or(UserProcessError::ImageOutOfBounds)?;

    let active_cr3 = current_cr3();
    if !unsafe {
        identity_mapped(
            active_cr3,
            boot_info.init_image.address,
            boot_info.init_image.size,
        )
    } {
        return Err(UserProcessError::ImageNotMapped);
    }
    let image =
        unsafe { slice::from_raw_parts(boot_info.init_image.address as *const u8, image_size) };
    let plan = user_elf::parse(image).map_err(UserProcessError::InvalidElf)?;

    if BOOTSTRAP_IN_USE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(UserProcessError::KernelUserSlotOccupied);
    }
    let kernel_pml4 = unsafe { &*(active_cr3 as *const PageTable) };
    let result =
        unsafe { build_address_space(&plan, image, kernel_pml4, &mut *BOOTSTRAP_STORAGE.0.get()) };
    if result.is_ok() {
        CURRENT_IMAGE_PAGES.store(image_pages(&plan), Ordering::Release);
    }
    if result.is_err() {
        BOOTSTRAP_IN_USE.store(false, Ordering::Release);
    }
    result
}

pub fn current_image_pages() -> usize {
    CURRENT_IMAGE_PAGES.load(Ordering::Acquire)
}

/// Map a bounded anonymous VMO-backed region into the active bootstrap
/// address space.  The bootstrap process uses statically owned, page-aligned
/// backing pages until the general process VM service is introduced; the
/// page-table and protection semantics are the same capability boundary used
/// by the native VMO tests.
pub fn mmap_user(length: u64, protection: u64) -> Option<u64> {
    let page_count = validate_mmap_request(length, protection)?;
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    let slot = storage.mmap_regions.iter().position(Option::is_none)?;
    let mut used = [false; USER_MMAP_PAGES];
    for region in storage.mmap_regions.iter().flatten() {
        for page in region.start_page..region.start_page + region.page_count {
            used[page] = true;
        }
    }
    let start_page = (0..=USER_MMAP_PAGES - page_count)
        .find(|start| (0..page_count).all(|offset| !used[*start + offset]))?;
    for page in start_page..start_page + page_count {
        let physical = page_address(&storage.mmap_pages[page]).ok()?;
        let flags = mmap_page_flags(protection);
        let entry = PageTableEntry::new(physical, flags)?;
        let (table, entry_index) = mmap_page_location(page)?;
        if !storage.mmap_pts[table].replace_empty(entry_index, entry.raw()) {
            for rollback in start_page..page {
                if let Some((table, entry_index)) = mmap_page_location(rollback) {
                    storage.mmap_pts[table].unmap(entry_index);
                }
            }
            return None;
        }
    }
    storage.mmap_regions[slot] = Some(MmapRegion {
        start_page,
        page_count,
        protection: protection as u8,
    });
    Some(USER_MMAP_BASE + start_page as u64 * PAGE_SIZE)
}

pub fn munmap_user(address: u64, length: u64) -> bool {
    let Some((slot, region)) = find_mmap_region(address, length) else {
        return false;
    };
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    for page in region.start_page..region.start_page + region.page_count {
        let Some((table, entry_index)) = mmap_page_location(page) else {
            return false;
        };
        storage.mmap_pts[table].unmap(entry_index);
    }
    storage.mmap_regions[slot] = None;
    true
}

pub fn mprotect_user(address: u64, length: u64, protection: u64) -> bool {
    if validate_mmap_request(length, protection).is_none() {
        return false;
    }
    let Some((slot, region)) = find_mmap_region(address, length) else {
        return false;
    };
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    for page in region.start_page..region.start_page + region.page_count {
        let Some((table, entry_index)) = mmap_page_location(page) else {
            return false;
        };
        let Some(current) = storage.mmap_pts[table].raw_entry(entry_index) else {
            return false;
        };
        let physical = current & 0x000f_ffff_ffff_f000;
        let Some(entry) = PageTableEntry::new(physical, mmap_page_flags(protection)) else {
            return false;
        };
        if !storage.mmap_pts[table].replace(entry_index, entry.raw()) {
            return false;
        }
    }
    if let Some(record) = storage.mmap_regions[slot].as_mut() {
        record.protection = protection as u8;
    }
    true
}

fn validate_mmap_request(length: u64, protection: u64) -> Option<usize> {
    if length == 0
        || !length.is_multiple_of(PAGE_SIZE)
        || protection > 0b111
        || length > USER_MMAP_PAGES as u64 * PAGE_SIZE
    {
        return None;
    }
    usize::try_from(length / PAGE_SIZE).ok()
}

#[inline]
fn mmap_page_location(page: usize) -> Option<(usize, usize)> {
    let table = page / PAGE_TABLE_ENTRIES;
    (table < USER_MMAP_PAGE_TABLES).then_some((table, page % PAGE_TABLE_ENTRIES))
}

fn mmap_page_flags(protection: u64) -> u64 {
    let mut flags = PageTableEntry::USER;
    if protection != 0 {
        flags |= PageTableEntry::PRESENT;
    }
    if protection & 0b010 != 0 {
        flags |= PageTableEntry::WRITABLE;
    }
    if protection & 0b100 == 0 {
        flags |= PageTableEntry::NO_EXECUTE;
    }
    flags
}

fn find_mmap_region(address: u64, length: u64) -> Option<(usize, MmapRegion)> {
    if !address.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let start_page = usize::try_from(address.checked_sub(USER_MMAP_BASE)? / PAGE_SIZE).ok()?;
    let page_count = validate_mmap_request(length, 0b001)?;
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    storage
        .mmap_regions
        .iter()
        .enumerate()
        .find_map(|(slot, region)| {
            let region = (*region)?;
            (region.start_page == start_page && region.page_count == page_count)
                .then_some((slot, region))
        })
}

/// Confirm that each page in a previously range-checked console buffer is a
/// present user image page. Bootstrap storage is immutable after `prepare`
/// succeeds, and M5 permits only the BSP to enter this address space.
pub fn is_user_image_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_range(
        &storage.image_pt,
        USER_IMAGE_BASE,
        USER_IMAGE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    )
}

pub fn is_user_readable_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_range(
        &storage.image_pt,
        USER_IMAGE_BASE,
        USER_IMAGE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    ) || mapped_range(
        &storage.stack_pt,
        USER_STACK_BASE,
        USER_STACK_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    ) || mapped_range(
        &storage.tls_pt,
        USER_TLS_BASE,
        USER_TLS_BASE + PAGE_SIZE,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    ) || mapped_mmap_range(
        &storage.mmap_pts,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    ) || mapped_range(
        &storage.surface_pt,
        USER_SURFACE_BASE,
        USER_SURFACE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    )
}

/// Check that a user instruction address is backed by an executable image
/// page.  Thread entry points are accepted only from the already-loaded Nagi
/// image; writable mmap pages cannot be turned into arbitrary kernel launches.
pub fn is_user_executable_range_mapped(address: u64, length: usize) -> bool {
    if length == 0 || address < USER_IMAGE_BASE {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > USER_IMAGE_LIMIT {
        return false;
    }
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    let first = ((address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    let last = ((end - 1 - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    (first..=last).all(|index| {
        storage.image_pt.raw_entry(index).is_some_and(|entry| {
            entry & (PageTableEntry::PRESENT | PageTableEntry::USER)
                == (PageTableEntry::PRESENT | PageTableEntry::USER)
                && entry & PageTableEntry::NO_EXECUTE == 0
        })
    })
}

pub fn is_user_writable_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_range(
        &storage.image_pt,
        USER_IMAGE_BASE,
        USER_IMAGE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
    ) || mapped_range(
        &storage.stack_pt,
        USER_STACK_BASE,
        USER_STACK_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
    ) || mapped_mmap_range(
        &storage.mmap_pts,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
    ) || mapped_range(
        &storage.surface_pt,
        USER_SURFACE_BASE,
        USER_SURFACE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
    )
}

#[cfg(test)]
fn image_range_is_mapped(table: &PageTable, address: u64, length: usize) -> bool {
    mapped_range(
        table,
        USER_IMAGE_BASE,
        USER_IMAGE_LIMIT,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    )
}

fn mapped_range(
    table: &PageTable,
    base: u64,
    limit: u64,
    address: u64,
    length: usize,
    required_flags: u64,
) -> bool {
    if length == 0 || address < base {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > limit {
        return false;
    }
    let first = ((address - base) / PAGE_SIZE) as usize;
    let last = ((end - 1 - base) / PAGE_SIZE) as usize;
    (first..=last).all(|index| {
        table
            .raw_entry(index)
            .is_some_and(|entry| entry & required_flags == required_flags)
    })
}

fn mapped_mmap_range(
    tables: &[PageTable; USER_MMAP_PAGE_TABLES],
    address: u64,
    length: usize,
    required_flags: u64,
) -> bool {
    if length == 0 || address < USER_MMAP_BASE {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > USER_MMAP_LIMIT {
        return false;
    }

    let first = ((address - USER_MMAP_BASE) / PAGE_SIZE) as usize;
    let last = ((end - 1 - USER_MMAP_BASE) / PAGE_SIZE) as usize;
    let mut page = first;
    while page <= last {
        let table_index = page / PAGE_TABLE_ENTRIES;
        let table_end = ((table_index + 1) * PAGE_TABLE_ENTRIES).min(last + 1);
        let table_base =
            USER_MMAP_BASE + table_index as u64 * PAGE_TABLE_ENTRIES as u64 * PAGE_SIZE;
        let part_address = table_base + (page % PAGE_TABLE_ENTRIES) as u64 * PAGE_SIZE;
        let part_length = (table_end - page) * PAGE_SIZE as usize;
        if !mapped_range(
            &tables[table_index],
            table_base,
            table_base + PAGE_TABLE_ENTRIES as u64 * PAGE_SIZE,
            part_address,
            part_length,
            required_flags,
        ) {
            return false;
        }
        page = table_end;
    }
    true
}

pub(crate) fn build_address_space(
    plan: &UserLoadPlan,
    image: &[u8],
    kernel_pml4: &PageTable,
    storage: &mut BootstrapStorage,
) -> Result<UserContext, UserProcessError> {
    // In production, `kernel_pml4` and `storage` are physical=virtual because
    // the loader's active address space is identity mapped. The caller must
    // validate the init image before passing any bytes to this builder.
    validate_plan(plan, image)?;
    if kernel_pml4.raw_entry(USER_PML4_INDEX).is_some() {
        return Err(UserProcessError::KernelUserSlotOccupied);
    }
    storage.clear();
    display::clear_surface();
    for index in 0..512 {
        if index == USER_PML4_INDEX {
            continue;
        }
        if let Some(raw) = kernel_pml4.raw_entry(index) {
            let normalized_raw = raw & !PageTableEntry::USER;
            if !storage.pml4.replace_empty(index, normalized_raw) {
                return Err(UserProcessError::InvalidLoadPlan);
            }
        }
    }

    map_hierarchy(storage)?;
    for segment in &plan.segments[..plan.segment_count] {
        map_segment(segment, image, storage)?;
    }
    for index in 0..USER_STACK_PAGES {
        map_leaf(
            &mut storage.stack_pt,
            index,
            page_address(&storage.stack_pages[index])?,
            PageTableEntry::PRESENT
                | PageTableEntry::WRITABLE
                | PageTableEntry::USER
                | PageTableEntry::NO_EXECUTE,
        )?;
    }
    map_leaf(
        &mut storage.tls_pt,
        0,
        page_address(&storage.tls_page)?,
        PageTableEntry::PRESENT
            | PageTableEntry::WRITABLE
            | PageTableEntry::USER
            | PageTableEntry::NO_EXECUTE,
    )?;
    for index in 0..SURFACE_PAGE_COUNT {
        map_leaf(
            &mut storage.surface_pt,
            index,
            display::surface_page_address(index).ok_or(UserProcessError::InvalidPhysicalAddress)?,
            PageTableEntry::PRESENT
                | PageTableEntry::WRITABLE
                | PageTableEntry::USER
                | PageTableEntry::NO_EXECUTE,
        )?;
    }

    Ok(UserContext {
        entry: plan.entry,
        // The compiler-generated `_start` follows the SysV entry convention:
        // RSP is 8 mod 16 on entry, so its prologue can align local FXSAVE
        // storage before executing SIMD instructions.
        user_stack_top: USER_STACK_LIMIT - 8,
        user_tls_base: USER_TLS_BASE,
        cr3: table_address(&storage.pml4)?,
        block_capability: crate::virtio::user_capability(),
        display_capability: display::user_capability(),
        input_capability: crate::input::user_capability(),
        net_capability: crate::net::user_capability(),
        audio_capability: crate::audio::user_capability(),
    })
}

fn validate_plan(plan: &UserLoadPlan, image: &[u8]) -> Result<(), UserProcessError> {
    if plan.segment_count == 0 || plan.segment_count > plan.segments.len() {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    let mut mapped_pages = 0_u64;
    let segments = &plan.segments[..plan.segment_count];
    for (index, segment) in segments.iter().enumerate() {
        if segment.memory_size == 0
            || segment.file_size > segment.memory_size
            || segment.virtual_address < USER_IMAGE_BASE
            || !segment.virtual_address.is_multiple_of(PAGE_SIZE)
            || !segment.file_offset.is_multiple_of(PAGE_SIZE)
            || segment.flags & (PF_W | PF_X) == (PF_W | PF_X)
        {
            return Err(UserProcessError::InvalidLoadPlan);
        }
        let memory_end = segment
            .virtual_address
            .checked_add(segment.memory_size)
            .ok_or(UserProcessError::ImageTooLarge)?;
        if memory_end > USER_IMAGE_LIMIT {
            return Err(UserProcessError::ImageTooLarge);
        }
        for existing in &segments[..index] {
            let existing_end = existing
                .virtual_address
                .checked_add(existing.memory_size)
                .ok_or(UserProcessError::InvalidLoadPlan)?;
            if segment.virtual_address < existing_end && existing.virtual_address < memory_end {
                return Err(UserProcessError::InvalidLoadPlan);
            }
        }
        let relative_end = memory_end - USER_IMAGE_BASE;
        mapped_pages = mapped_pages.max(relative_end.div_ceil(PAGE_SIZE));
        let file_end = segment
            .file_offset
            .checked_add(segment.file_size)
            .ok_or(UserProcessError::ImageOutOfBounds)?;
        if file_end > image.len() as u64 {
            return Err(UserProcessError::ImageOutOfBounds);
        }
    }
    if mapped_pages > MAX_IMAGE_PAGES as u64 {
        return Err(UserProcessError::ImageTooLarge);
    }
    let entry_is_executable = segments.iter().any(|segment| {
        let Some(file_end) = segment.virtual_address.checked_add(segment.file_size) else {
            return false;
        };
        segment.flags & PF_X != 0 && plan.entry >= segment.virtual_address && plan.entry < file_end
    });
    if !entry_is_executable {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    Ok(())
}

fn image_pages(plan: &UserLoadPlan) -> usize {
    plan.segments[..plan.segment_count]
        .iter()
        .filter_map(|segment| {
            segment
                .virtual_address
                .checked_add(segment.memory_size)
                .map(|end| ((end - USER_IMAGE_BASE).div_ceil(PAGE_SIZE)) as usize)
        })
        .max()
        .unwrap_or(0)
}

fn map_hierarchy(storage: &mut BootstrapStorage) -> Result<(), UserProcessError> {
    let hierarchy_flags = PageTableEntry::PRESENT | PageTableEntry::WRITABLE | PageTableEntry::USER;
    let pdpt = table_address(&storage.pdpt)?;
    let pd = table_address(&storage.pd)?;
    let image_pt = table_address(&storage.image_pt)?;
    let stack_pt = table_address(&storage.stack_pt)?;
    let tls_pt = table_address(&storage.tls_pt)?;
    let surface_pt = table_address(&storage.surface_pt)?;
    map_leaf(&mut storage.pml4, USER_PML4_INDEX, pdpt, hierarchy_flags)?;
    map_leaf(&mut storage.pdpt, 0, pd, hierarchy_flags)?;
    map_leaf(&mut storage.pd, 0, image_pt, hierarchy_flags)?;
    map_leaf(&mut storage.pd, 1, stack_pt, hierarchy_flags)?;
    map_leaf(&mut storage.pd, 2, tls_pt, hierarchy_flags)?;
    map_leaf(&mut storage.pd, 3, surface_pt, hierarchy_flags)?;
    for table_index in 0..USER_MMAP_PAGE_TABLES {
        let mmap_pt = table_address(&storage.mmap_pts[table_index])?;
        map_leaf(&mut storage.pd, 4 + table_index, mmap_pt, hierarchy_flags)?;
    }
    Ok(())
}

fn map_segment(
    segment: &crate::user_elf::UserLoadSegment,
    image: &[u8],
    storage: &mut BootstrapStorage,
) -> Result<(), UserProcessError> {
    let first_page = ((segment.virtual_address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    let page_count = segment.memory_size.div_ceil(PAGE_SIZE) as usize;
    let mut leaf_flags = PageTableEntry::PRESENT | PageTableEntry::USER;
    if segment.flags & PF_W != 0 {
        leaf_flags |= PageTableEntry::WRITABLE;
    }
    if segment.flags & PF_X == 0 {
        leaf_flags |= PageTableEntry::NO_EXECUTE;
    }
    for page_index in first_page..first_page + page_count {
        let physical = page_address(&storage.image_pages[page_index])?;
        map_leaf(&mut storage.image_pt, page_index, physical, leaf_flags)?;
    }

    let destination_offset = (segment.virtual_address - USER_IMAGE_BASE) as usize;
    let source_start = segment.file_offset as usize;
    let source_end = source_start + segment.file_size as usize;
    let destination = image_storage_bytes(storage);
    let destination_end = destination_offset + segment.file_size as usize;
    destination[destination_offset..destination_end]
        .copy_from_slice(&image[source_start..source_end]);
    Ok(())
}

fn image_storage_bytes(storage: &mut BootstrapStorage) -> &mut [u8] {
    let pointer = storage.image_pages.as_mut_ptr().cast::<u8>();
    unsafe { slice::from_raw_parts_mut(pointer, MAX_IMAGE_PAGES * PAGE_SIZE as usize) }
}

fn map_leaf(
    table: &mut PageTable,
    index: usize,
    physical_address: u64,
    flags: u64,
) -> Result<(), UserProcessError> {
    let entry = PageTableEntry::new(physical_address, flags)
        .ok_or(UserProcessError::InvalidPhysicalAddress)?;
    if !table.replace_empty(index, entry.raw()) {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    Ok(())
}

fn table_address(table: &PageTable) -> Result<u64, UserProcessError> {
    aligned_address((table as *const PageTable) as u64)
}

fn page_address(page: &PageBytes) -> Result<u64, UserProcessError> {
    aligned_address((page as *const PageBytes) as u64)
}

fn aligned_address(address: u64) -> Result<u64, UserProcessError> {
    if address.is_multiple_of(PAGE_SIZE) && address < (1 << 52) {
        Ok(address)
    } else {
        Err(UserProcessError::InvalidPhysicalAddress)
    }
}

/// Install the prepared address space and the M5 TLS base immediately before
/// the ring-3 entry sequence.
///
/// # Safety
///
/// The context must come from `prepare`, and the caller must ensure no other
/// CPU is using the current bootstrap address space.
pub unsafe fn activate(context: &UserContext) {
    let efer_low: u32;
    let efer_high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") IA32_EFER,
        out("eax") efer_low,
        out("edx") efer_high,
        options(nostack, preserves_flags)
    );
    let efer = efer_with_nxe((u64::from(efer_high) << 32) | u64::from(efer_low));
    core::arch::asm!(
        "wrmsr",
        in("ecx") IA32_EFER,
        in("eax") efer as u32,
        in("edx") (efer >> 32) as u32,
        options(nostack, preserves_flags)
    );
    core::arch::asm!("mov cr3, {}", in(reg) context.cr3, options(nostack, preserves_flags));
    let low = context.user_tls_base as u32;
    let high = (context.user_tls_base >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") IA32_FS_BASE,
        in("eax") low,
        in("edx") high,
        options(nostack, preserves_flags)
    );
}

/// Enter the prepared M5 process at CPL3. Interrupts remain disabled until M6
/// provides a TSS-backed privilege-transition stack.
///
/// # Safety
///
/// `context` must be the unique context returned by `prepare`, and the M5 GDT
/// and syscall MSRs must already be installed on this BSP.
pub unsafe fn enter(context: UserContext) -> ! {
    core::arch::asm!("cli", options(nomem, nostack));
    activate(&context);
    core::arch::asm!(
        "push {user_ss}",
        "push {user_rsp}",
        "push {user_rflags}",
        "push {user_cs}",
        "push {user_rip}",
        "push {audio_capability}",
        "push {net_capability}",
        "push {input_capability}",
        "push {display_capability}",
        "push {block_capability}",
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor esi, esi",
        "xor edi, edi",
        "xor ebp, ebp",
        "xor r8d, r8d",
        "xor r9d, r9d",
        "xor r10d, r10d",
        "xor r11d, r11d",
        "xor r12d, r12d",
        "xor r13d, r13d",
        "xor r14d, r14d",
        "xor r15d, r15d",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop r8",
        "fninit",
        "fldz",
        "fldz",
        "fldz",
        "fldz",
        "fldz",
        "fldz",
        "fldz",
        "fldz",
        "fninit",
        "sub rsp, 8",
        "mov dword ptr [rsp], 0x1f80",
        "ldmxcsr [rsp]",
        "add rsp, 8",
        "pxor xmm0, xmm0",
        "pxor xmm1, xmm1",
        "pxor xmm2, xmm2",
        "pxor xmm3, xmm3",
        "pxor xmm4, xmm4",
        "pxor xmm5, xmm5",
        "pxor xmm6, xmm6",
        "pxor xmm7, xmm7",
        "pxor xmm8, xmm8",
        "pxor xmm9, xmm9",
        "pxor xmm10, xmm10",
        "pxor xmm11, xmm11",
        "pxor xmm12, xmm12",
        "pxor xmm13, xmm13",
        "pxor xmm14, xmm14",
        "pxor xmm15, xmm15",
        "iretq",
        user_ss = const USER_DATA_SELECTOR,
        user_rsp = in(reg) context.user_stack_top,
        user_rflags = const USER_INITIAL_RFLAGS,
        user_cs = const USER_CODE_SELECTOR,
        user_rip = in(reg) context.entry,
        block_capability = in(reg) context.block_capability,
        display_capability = in(reg) context.display_capability,
        input_capability = in(reg) context.input_capability,
        net_capability = in(reg) context.net_capability,
        audio_capability = in(reg) context.audio_capability,
        options(noreturn)
    );
}

const fn efer_with_nxe(efer: u64) -> u64 {
    efer | EFER_NXE
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::boxed::Box;

    use crate::memory::{PageTable, PageTableEntry, PAGE_SIZE};
    use crate::user_elf::{UserLoadPlan, UserLoadSegment, MAX_LOAD_SEGMENTS, USER_IMAGE_BASE};

    use super::{
        build_address_space, efer_with_nxe, image_range_is_mapped, mapped_range, mmap_page_flags,
        validate_mmap_request, BootstrapStorage, UserProcessError, USER_MMAP_PAGES,
        USER_STACK_BASE, USER_STACK_LIMIT, USER_TLS_BASE,
    };

    fn boxed_storage() -> Box<BootstrapStorage> {
        let mut allocation = Box::<BootstrapStorage>::new_uninit();
        unsafe {
            allocation
                .as_mut_ptr()
                .cast::<u8>()
                .write_bytes(0, core::mem::size_of::<BootstrapStorage>());
            allocation.assume_init()
        }
    }

    fn plan(memory_size: u64) -> UserLoadPlan {
        let mut segments = [UserLoadSegment::default(); MAX_LOAD_SEGMENTS];
        segments[0] = UserLoadSegment {
            file_offset: 0,
            virtual_address: USER_IMAGE_BASE,
            file_size: 16,
            memory_size,
            flags: 5,
        };
        UserLoadPlan {
            entry: USER_IMAGE_BASE,
            segments,
            segment_count: 1,
        }
    }

    #[test]
    fn builds_bounded_user_mappings_and_retains_kernel_pml4_entries() {
        let mut kernel_pml4 = PageTable::empty();
        let kernel_entry = PageTableEntry::new(
            0x20_0000,
            PageTableEntry::PRESENT | PageTableEntry::WRITABLE,
        )
        .expect("kernel entry");
        let user_marked_kernel_entry = PageTableEntry::new(
            0x21_0000,
            PageTableEntry::PRESENT | PageTableEntry::WRITABLE | PageTableEntry::USER,
        )
        .expect("user-marked kernel entry");
        assert!(kernel_pml4.replace_empty(0, kernel_entry.raw()));
        assert!(kernel_pml4.replace_empty(1, user_marked_kernel_entry.raw()));
        let mut storage = boxed_storage();
        let image = [0x90_u8; 16];

        let context = build_address_space(&plan(PAGE_SIZE), &image, &kernel_pml4, &mut storage)
            .expect("address space");

        assert_eq!(context.entry, USER_IMAGE_BASE);
        assert_eq!(context.user_stack_top, USER_STACK_LIMIT - 8);
        assert_eq!(context.user_tls_base, USER_TLS_BASE);
        assert_eq!(storage.pml4.raw_entry(0), Some(kernel_entry.raw()));
        assert_eq!(
            storage.pml4.raw_entry(1),
            Some(user_marked_kernel_entry.raw() & !PageTableEntry::USER)
        );
        assert_eq!(&storage.image_pages[0].0[..16], &image);
        assert!(storage.image_pages[0].0[16..].iter().all(|byte| *byte == 0));

        let code = storage.image_pt.raw_entry(0).expect("code mapping");
        assert_ne!(code & PageTableEntry::USER, 0);
        assert_eq!(code & PageTableEntry::WRITABLE, 0);
        assert_eq!(code & PageTableEntry::NO_EXECUTE, 0);

        let stack = storage.stack_pt.raw_entry(0).expect("stack mapping");
        assert_ne!(stack & PageTableEntry::USER, 0);
        assert_ne!(stack & PageTableEntry::WRITABLE, 0);
        assert_ne!(stack & PageTableEntry::NO_EXECUTE, 0);

        let tls = storage.tls_pt.raw_entry(0).expect("TLS mapping");
        assert_ne!(tls & PageTableEntry::USER, 0);
        assert_ne!(tls & PageTableEntry::WRITABLE, 0);
        assert_ne!(tls & PageTableEntry::NO_EXECUTE, 0);
    }

    #[test]
    fn rejects_an_image_requiring_two_hundred_fifty_seven_pages() {
        let kernel_pml4 = PageTable::empty();
        let mut storage = boxed_storage();
        let image = [0_u8; 16];

        assert_eq!(
            build_address_space(&plan(PAGE_SIZE * 257), &image, &kernel_pml4, &mut storage,),
            Err(UserProcessError::ImageTooLarge)
        );
    }

    #[test]
    fn rejects_an_unvalidated_writable_executable_plan() {
        let kernel_pml4 = PageTable::empty();
        let mut storage = boxed_storage();
        let image = [0_u8; 16];
        let mut unvalidated = plan(PAGE_SIZE);
        unvalidated.segments[0].flags = 7;

        assert_eq!(
            build_address_space(&unvalidated, &image, &kernel_pml4, &mut storage),
            Err(UserProcessError::InvalidLoadPlan)
        );
        assert_eq!(storage.pml4.raw_entry(128), None);
    }

    #[test]
    fn enables_nxe_without_dropping_existing_efer_bits() {
        assert_eq!(efer_with_nxe(1), 1 | (1 << 11));
    }

    #[test]
    fn user_read_policy_requires_every_page_to_be_present_and_user_accessible() {
        let mut table = PageTable::empty();
        let flags = PageTableEntry::PRESENT | PageTableEntry::USER;
        let first = PageTableEntry::new(0x20_0000, flags).expect("first page");
        let second = PageTableEntry::new(0x21_0000, flags).expect("second page");
        assert!(table.replace_empty(0, first.raw()));

        assert!(image_range_is_mapped(&table, USER_IMAGE_BASE, 1));
        assert!(!image_range_is_mapped(
            &table,
            USER_IMAGE_BASE + PAGE_SIZE - 1,
            2
        ));

        assert!(table.replace_empty(1, second.raw()));
        assert!(image_range_is_mapped(
            &table,
            USER_IMAGE_BASE + PAGE_SIZE - 1,
            2
        ));
    }

    #[test]
    fn writable_user_policy_rejects_read_only_and_cross_boundary_ranges() {
        let mut table = PageTable::empty();
        let read_only =
            PageTableEntry::new(0x30_0000, PageTableEntry::PRESENT | PageTableEntry::USER)
                .expect("read-only page");
        assert!(table.replace_empty(0, read_only.raw()));
        assert!(!mapped_range(
            &table,
            USER_STACK_BASE,
            USER_STACK_LIMIT,
            USER_STACK_BASE,
            512,
            PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
        ));

        let writable = PageTableEntry::new(
            0x31_0000,
            PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
        )
        .expect("writable page");
        table.clear();
        assert!(table.replace_empty(0, writable.raw()));
        assert!(mapped_range(
            &table,
            USER_STACK_BASE,
            USER_STACK_LIMIT,
            USER_STACK_BASE,
            512,
            PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
        ));
        assert!(!mapped_range(
            &table,
            USER_STACK_BASE,
            USER_STACK_LIMIT,
            USER_STACK_BASE + PAGE_SIZE - 128,
            512,
            PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
        ));
    }

    #[test]
    fn mmap_requests_are_page_aligned_and_protection_bounded() {
        assert_eq!(validate_mmap_request(PAGE_SIZE, 0b011), Some(1));
        assert_eq!(validate_mmap_request(2 * PAGE_SIZE, 0), Some(2));
        assert_eq!(validate_mmap_request(0, 0b001), None);
        assert_eq!(validate_mmap_request(PAGE_SIZE - 1, 0b001), None);
        assert_eq!(
            validate_mmap_request((USER_MMAP_PAGES as u64 + 1) * PAGE_SIZE, 0b001),
            None
        );
        assert_eq!(validate_mmap_request(PAGE_SIZE, 0b1000), None);
    }

    #[test]
    fn mmap_page_flags_attenuate_write_and_execute_independently() {
        let read_only = mmap_page_flags(0b001);
        assert_ne!(read_only & PageTableEntry::PRESENT, 0);
        assert_eq!(read_only & PageTableEntry::WRITABLE, 0);
        assert_ne!(read_only & PageTableEntry::NO_EXECUTE, 0);

        let executable = mmap_page_flags(0b101);
        assert_ne!(executable & PageTableEntry::PRESENT, 0);
        assert_eq!(executable & PageTableEntry::WRITABLE, 0);
        assert_eq!(executable & PageTableEntry::NO_EXECUTE, 0);
    }

    #[test]
    fn ring3_entry_starts_with_a_sanitized_fpu_state() {
        let source = include_str!("user_process.rs");
        let entry = source
            .split("pub unsafe fn enter(context: UserContext)")
            .nth(1)
            .unwrap()
            .split("const fn efer_with_nxe")
            .next()
            .unwrap();
        assert!(entry.contains("fninit"));
        assert!(entry.contains("ldmxcsr"));
        assert!(entry.matches("fldz").count() >= 8);
        for register in 0..16 {
            assert!(
                entry.contains(&std::format!("pxor xmm{register}, xmm{register}")),
                "XMM{register} is not initialized"
            );
        }
    }

    #[test]
    fn ring3_entry_passes_the_bootstrap_block_capability_in_rdi() {
        let source = include_str!("user_process.rs");
        let entry = source
            .split("pub unsafe fn enter(context: UserContext)")
            .nth(1)
            .unwrap()
            .split("const fn efer_with_nxe")
            .next()
            .unwrap();
        assert!(entry.contains("push {block_capability}"));
        assert!(entry.contains("push {net_capability}"));
        assert!(entry.contains("push {audio_capability}"));
        assert!(entry.contains("pop rdi"));
        assert!(entry.contains("block_capability = in(reg) context.block_capability"));
        assert!(entry.contains("net_capability = in(reg) context.net_capability"));
        assert!(entry.contains("audio_capability = in(reg) context.audio_capability"));
    }
}
