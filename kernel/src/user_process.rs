use core::cell::UnsafeCell;
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nagi_abi::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
use nagi_bootinfo::{BootInfo, BootInfoError};

use crate::display::{self, SURFACE_PAGE_COUNT, USER_SURFACE_BASE};
use crate::memory::{
    current_cr3, identity_mapped, PageAllocator, PageTable, PageTableEntry, PAGE_SIZE,
    PAGE_TABLE_ENTRIES,
};
use crate::user_elf::{
    self, UserElfError, UserLoadPlan, PF_W, PF_X, USER_IMAGE_BASE, USER_IMAGE_LIMIT,
};

#[cfg(test)]
#[path = "syscall.rs"]
mod syscall;

pub const USER_STACK_BASE: u64 = USER_IMAGE_LIMIT;
// The M14 audio and M17 Servo/Softpipe bootstrap paths need deeper native
// stacks than the early init path. Keep one bounded 2 MiB page-table span for
// the bootstrap process; stack growth remains fixed and below the TLS region.
pub const USER_STACK_PAGES: usize = PAGE_TABLE_ENTRIES;
pub const USER_STACK_LIMIT: u64 = USER_STACK_BASE + USER_STACK_PAGES as u64 * PAGE_SIZE;
pub const USER_TLS_BASE: u64 = USER_IMAGE_LIMIT + 0x0040_0000;
pub const USER_TLS_THREAD_SLOT_COUNT: usize = 2;
pub const USER_TLS_PAGES_PER_THREAD: usize = 2;
pub const USER_TLS_PAGE_COUNT: usize = USER_TLS_THREAD_SLOT_COUNT * USER_TLS_PAGES_PER_THREAD;
pub const USER_TLS_CONTROL_BASE: u64 = USER_TLS_BASE + PAGE_SIZE;
pub const USER_TLS_CHILD_BASE: u64 = USER_TLS_BASE + USER_TLS_PAGES_PER_THREAD as u64 * PAGE_SIZE;
pub const USER_TLS_CHILD_CONTROL_BASE: u64 = USER_TLS_CHILD_BASE + PAGE_SIZE;
pub const USER_TLS_LIMIT: u64 = USER_TLS_BASE + USER_TLS_PAGE_COUNT as u64 * PAGE_SIZE;
pub const USER_MMAP_BASE: u64 = USER_IMAGE_LIMIT + 0x0080_0000;
const USER_MMAP_PAGE_TABLES: usize = 8;
pub const USER_MMAP_PAGES: usize = PAGE_TABLE_ENTRIES * USER_MMAP_PAGE_TABLES;
pub const USER_MMAP_LIMIT: u64 = USER_MMAP_BASE + USER_MMAP_PAGES as u64 * PAGE_SIZE;
pub const USER_SURFACE_LIMIT: u64 = USER_SURFACE_BASE + SURFACE_PAGE_COUNT as u64 * PAGE_SIZE;
const USER_PML4_INDEX: usize = 128;
const USER_PDPT_INDEX: usize = ((USER_IMAGE_BASE >> 30) & 0x1ff) as usize;
const USER_IMAGE_FIRST_PD_INDEX: usize = ((USER_IMAGE_BASE >> 21) & 0x1ff) as usize;
const USER_IMAGE_PAGE_TABLE_COUNT: usize =
    ((USER_IMAGE_LIMIT - USER_IMAGE_BASE) / (PAGE_TABLE_ENTRIES as u64 * PAGE_SIZE)) as usize;
const MAX_INIT_IMAGE_SIZE: usize = 128 * 1024 * 1024;
#[cfg(test)]
const MAX_TEST_IMAGE_PAGES: usize = 256;
const MAX_IDENTITY_MAPPED_ADDRESS: u64 = 1 << 32;
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
    PhysicalMemoryExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareStage {
    BootInfoValidated,
    InitImageDetails {
        bytes: usize,
    },
    InitImageMappingCheckStarted,
    InitImageIdentityMapped,
    InitElfParsed,
    BootstrapSlotAcquired,
    LoadPlanValidated,
    BootstrapStorageResetStarted,
    BootstrapStorageReset,
    PageTableHierarchyBuilt,
    TlsInitialized,
    StackMapped,
    TlsMapped,
    SurfaceMapped,
    LoadSegmentMapping {
        segment_index: usize,
        page_count: usize,
        file_bytes: usize,
        memory_bytes: usize,
    },
    LoadSegmentProgress {
        segment_index: usize,
        mapped_pages: usize,
        total_pages: usize,
    },
    LoadSegmentMapped {
        segment_index: usize,
        page_count: usize,
    },
    UserContextReady,
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
    image_extra_pts: [PageTable; USER_IMAGE_PAGE_TABLE_COUNT - 1],
    pub(crate) stack_pt: PageTable,
    pub(crate) tls_pt: PageTable,
    mmap_pts: [PageTable; USER_MMAP_PAGE_TABLES],
    surface_pt: PageTable,
    #[cfg(test)]
    image_pages: [PageBytes; MAX_TEST_IMAGE_PAGES],
    stack_pages: [PageBytes; USER_STACK_PAGES],
    tls_pages: [PageBytes; USER_TLS_PAGE_COUNT],
    tls_initial_page: PageBytes,
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
            image_extra_pts: [const { PageTable::empty() }; USER_IMAGE_PAGE_TABLE_COUNT - 1],
            stack_pt: PageTable::empty(),
            tls_pt: PageTable::empty(),
            mmap_pts: [const { PageTable::empty() }; USER_MMAP_PAGE_TABLES],
            surface_pt: PageTable::empty(),
            #[cfg(test)]
            image_pages: [const { PageBytes::zeroed() }; MAX_TEST_IMAGE_PAGES],
            stack_pages: [const { PageBytes::zeroed() }; USER_STACK_PAGES],
            tls_pages: [const { PageBytes::zeroed() }; USER_TLS_PAGE_COUNT],
            tls_initial_page: PageBytes::zeroed(),
            mmap_pages: [const { PageBytes::zeroed() }; USER_MMAP_PAGES],
            mmap_regions: [None; MAX_MMAP_REGIONS],
        }
    }

    fn clear(&mut self) {
        self.pml4.clear();
        self.pdpt.clear();
        self.pd.clear();
        self.image_pt.clear();
        for table in &mut self.image_extra_pts {
            table.clear();
        }
        self.stack_pt.clear();
        self.tls_pt.clear();
        for table in &mut self.mmap_pts {
            table.clear();
        }
        self.surface_pt.clear();
        #[cfg(test)]
        for page in &mut self.image_pages {
            page.0.fill(0);
        }
        for page in &mut self.stack_pages {
            page.0.fill(0);
        }
        for page in &mut self.tls_pages {
            page.0.fill(0);
        }
        self.tls_initial_page.0.fill(0);
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

/// Validate and prepare the one-shot M17 bootstrap process image.
///
/// The loader leaves the active address space identity mapped, so the CR3
/// value and kernel static-storage addresses are physical=virtual addresses
/// in this bootstrap path. The init image must pass `identity_mapped` before
/// this function forms a slice and dereferences its bytes.
pub fn prepare(
    boot_info: &BootInfo,
    allocator: &mut PageAllocator,
) -> Result<UserContext, UserProcessError> {
    prepare_with_progress(boot_info, allocator, |_| {})
}

/// Validate and prepare the one-shot M17 bootstrap process image while
/// reporting coarse progress to the caller. The progress hook is diagnostic
/// only; it does not affect allocation, mapping, or entry decisions.
pub fn prepare_with_progress(
    boot_info: &BootInfo,
    allocator: &mut PageAllocator,
    mut progress: impl FnMut(PrepareStage),
) -> Result<UserContext, UserProcessError> {
    boot_info
        .validate_for_user_bootstrap()
        .map_err(UserProcessError::InvalidBootInfo)?;
    progress(PrepareStage::BootInfoValidated);
    let image_size =
        usize::try_from(boot_info.init_image.size).map_err(|_| UserProcessError::ImageTooLarge)?;
    if image_size > MAX_INIT_IMAGE_SIZE {
        return Err(UserProcessError::ImageTooLarge);
    }
    progress(PrepareStage::InitImageDetails { bytes: image_size });
    boot_info
        .init_image
        .address
        .checked_add(boot_info.init_image.size)
        .ok_or(UserProcessError::ImageOutOfBounds)?;

    let active_cr3 = current_cr3();
    progress(PrepareStage::InitImageMappingCheckStarted);
    let image_is_mapped = unsafe {
        identity_mapped(
            active_cr3,
            boot_info.init_image.address,
            boot_info.init_image.size,
        )
    };
    if !image_is_mapped {
        return Err(UserProcessError::ImageNotMapped);
    }
    progress(PrepareStage::InitImageIdentityMapped);
    let image =
        unsafe { slice::from_raw_parts(boot_info.init_image.address as *const u8, image_size) };
    let plan = user_elf::parse(image).map_err(UserProcessError::InvalidElf)?;
    progress(PrepareStage::InitElfParsed);

    if BOOTSTRAP_IN_USE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(UserProcessError::KernelUserSlotOccupied);
    }
    progress(PrepareStage::BootstrapSlotAcquired);
    let kernel_pml4 = unsafe { &*(active_cr3 as *const PageTable) };
    let result = unsafe {
        build_address_space_with_allocator(
            &plan,
            image,
            boot_info.init_image.address,
            kernel_pml4,
            &mut *BOOTSTRAP_STORAGE.0.get(),
            allocator,
            active_cr3,
            &mut progress,
        )
    };
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
        storage.mmap_pages[page].0.fill(0);
    }
    if protection != PROT_NONE && !remap_mmap_pages(storage, start_page, page_count, protection) {
        return None;
    }
    storage.mmap_regions[slot] = Some(MmapRegion {
        start_page,
        page_count,
        protection: protection as u8,
    });
    Some(USER_MMAP_BASE + start_page as u64 * PAGE_SIZE)
}

/// Re-map an existing Nagi-owned range at its exact guest address.
///
/// This is intentionally narrower than a general POSIX MAP_FIXED facility:
/// the address must identify a range previously reserved by `mmap_user`.
/// That is the reserve/commit contract required by MozJS JIT memory and keeps
/// arbitrary user page-table replacement outside the bootstrap ABI.
pub fn mmap_user_at(address: u64, length: u64, protection: u64) -> Option<u64> {
    let (start_page, page_count) = mmap_range(address, length, protection)?;
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    let slot = storage.mmap_regions.iter().position(|region| {
        region.is_some_and(|region| {
            region.start_page == start_page && region.page_count == page_count
        })
    })?;

    if protection == PROT_NONE {
        for page in start_page..start_page + page_count {
            let (table, entry_index) = mmap_page_location(page)?;
            storage.mmap_pts[table].unmap(entry_index);
        }
    } else if !remap_mmap_pages(storage, start_page, page_count, protection) {
        return None;
    }

    if let Some(region) = storage.mmap_regions[slot].as_mut() {
        region.protection = protection as u8;
    }
    Some(address)
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

/// Restore the one native child slot's static TLS from the initial ELF image.
/// The slot is reused after `thread_exit`, so it must not inherit the previous
/// child's modified thread-local data or control-page state.
pub fn reset_child_tls() {
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    reset_child_tls_pages(storage);
}

pub fn mprotect_user(address: u64, length: u64, protection: u64) -> bool {
    if validate_mmap_request(length, protection).is_none() {
        return false;
    }
    let Some((slot, region)) = find_mmap_region(address, length) else {
        return false;
    };
    let storage = unsafe { &mut *BOOTSTRAP_STORAGE.0.get() };
    if protection == PROT_NONE {
        for page in region.start_page..region.start_page + region.page_count {
            let Some((table, entry_index)) = mmap_page_location(page) else {
                return false;
            };
            storage.mmap_pts[table].unmap(entry_index);
        }
    } else if !remap_mmap_pages(storage, region.start_page, region.page_count, protection) {
        return false;
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

fn mmap_range(address: u64, length: u64, protection: u64) -> Option<(usize, usize)> {
    let page_count = validate_mmap_request(length, protection)?;
    if !address.is_multiple_of(PAGE_SIZE) || address < USER_MMAP_BASE {
        return None;
    }
    let end = address.checked_add(length)?;
    if end > USER_MMAP_LIMIT {
        return None;
    }
    let start_page = usize::try_from((address - USER_MMAP_BASE) / PAGE_SIZE).ok()?;
    Some((start_page, page_count))
}

fn remap_mmap_pages(
    storage: &mut BootstrapStorage,
    start_page: usize,
    page_count: usize,
    protection: u64,
) -> bool {
    for page in start_page..start_page + page_count {
        let Some(physical) = page_address(&storage.mmap_pages[page]).ok() else {
            return false;
        };
        let Some(entry) = PageTableEntry::new(physical, mmap_page_flags(protection)) else {
            return false;
        };
        let Some((table, entry_index)) = mmap_page_location(page) else {
            return false;
        };
        let mapped = if storage.mmap_pts[table].raw_entry(entry_index).is_some() {
            storage.mmap_pts[table].replace(entry_index, entry.raw())
        } else {
            storage.mmap_pts[table].replace_empty(entry_index, entry.raw())
        };
        if !mapped {
            return false;
        }
    }
    true
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
    if protection & PROT_WRITE != 0 {
        flags |= PageTableEntry::WRITABLE;
    }
    if protection & PROT_EXEC == 0 {
        flags |= PageTableEntry::NO_EXECUTE;
    }
    flags
}

fn find_mmap_region(address: u64, length: u64) -> Option<(usize, MmapRegion)> {
    if !address.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let start_page = usize::try_from(address.checked_sub(USER_MMAP_BASE)? / PAGE_SIZE).ok()?;
    let page_count = validate_mmap_request(length, PROT_READ)?;
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
/// succeeds, and the M17 bootstrap permits only the BSP to enter this address
/// space.
pub fn is_user_image_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_image_range(
        storage,
        address,
        length,
        PageTableEntry::PRESENT | PageTableEntry::USER,
    )
}

pub fn is_user_readable_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_image_range(
        storage,
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
        USER_TLS_LIMIT,
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
        image_page_entry(storage, index).is_some_and(|entry| {
            entry & (PageTableEntry::PRESENT | PageTableEntry::USER)
                == (PageTableEntry::PRESENT | PageTableEntry::USER)
                && entry & PageTableEntry::NO_EXECUTE == 0
        })
    })
}

pub fn is_user_writable_range_mapped(address: u64, length: usize) -> bool {
    let storage = unsafe { &*BOOTSTRAP_STORAGE.0.get() };
    mapped_image_range(
        storage,
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

fn mapped_image_range(
    storage: &BootstrapStorage,
    address: u64,
    length: usize,
    required_flags: u64,
) -> bool {
    if length == 0 || address < USER_IMAGE_BASE {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > USER_IMAGE_LIMIT {
        return false;
    }
    let first = ((address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    let last = ((end - 1 - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    (first..=last).all(|page| {
        image_page_entry(storage, page)
            .is_some_and(|entry| entry & required_flags == required_flags)
    })
}

fn image_page_entry(storage: &BootstrapStorage, page: usize) -> Option<u64> {
    let table = page / PAGE_TABLE_ENTRIES;
    let entry = page % PAGE_TABLE_ENTRIES;
    if table >= USER_IMAGE_PAGE_TABLE_COUNT {
        return None;
    }
    if table == 0 {
        storage.image_pt.raw_entry(entry)
    } else {
        storage.image_extra_pts[table - 1].raw_entry(entry)
    }
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

#[cfg(test)]
pub(crate) fn build_address_space(
    plan: &UserLoadPlan,
    image: &[u8],
    kernel_pml4: &PageTable,
    storage: &mut BootstrapStorage,
) -> Result<UserContext, UserProcessError> {
    validate_plan(plan, image)?;
    let mut ignore_progress = |_| {};
    prepare_address_space_storage(plan, image, kernel_pml4, storage, &mut ignore_progress)?;
    for segment in &plan.segments[..plan.segment_count] {
        map_segment_for_test(segment, image, storage)?;
    }
    build_user_context(plan, storage)
}

fn build_address_space_with_allocator(
    plan: &UserLoadPlan,
    image: &[u8],
    image_physical_base: u64,
    kernel_pml4: &PageTable,
    storage: &mut BootstrapStorage,
    allocator: &mut PageAllocator,
    active_cr3: u64,
    progress: &mut impl FnMut(PrepareStage),
) -> Result<UserContext, UserProcessError> {
    validate_plan(plan, image)?;
    progress(PrepareStage::LoadPlanValidated);
    prepare_address_space_storage(plan, image, kernel_pml4, storage, progress)?;
    for (segment_index, segment) in plan.segments[..plan.segment_count].iter().enumerate() {
        map_segment_from_image(
            segment_index,
            segment,
            image,
            image_physical_base,
            storage,
            allocator,
            active_cr3,
            progress,
        )?;
    }
    let context = build_user_context(plan, storage)?;
    progress(PrepareStage::UserContextReady);
    Ok(context)
}

fn prepare_address_space_storage(
    plan: &UserLoadPlan,
    image: &[u8],
    kernel_pml4: &PageTable,
    storage: &mut BootstrapStorage,
    progress: &mut impl FnMut(PrepareStage),
) -> Result<(), UserProcessError> {
    if kernel_pml4.raw_entry(USER_PML4_INDEX).is_some() {
        return Err(UserProcessError::KernelUserSlotOccupied);
    }
    progress(PrepareStage::BootstrapStorageResetStarted);
    storage.clear();
    progress(PrepareStage::BootstrapStorageReset);
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
    progress(PrepareStage::PageTableHierarchyBuilt);
    initialize_tls(plan, image, storage)?;
    progress(PrepareStage::TlsInitialized);
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
    progress(PrepareStage::StackMapped);
    for index in 0..USER_TLS_PAGE_COUNT {
        map_leaf(
            &mut storage.tls_pt,
            index,
            page_address(&storage.tls_pages[index])?,
            PageTableEntry::PRESENT
                | PageTableEntry::WRITABLE
                | PageTableEntry::USER
                | PageTableEntry::NO_EXECUTE,
        )?;
    }
    progress(PrepareStage::TlsMapped);
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
    progress(PrepareStage::SurfaceMapped);
    Ok(())
}

fn build_user_context(
    plan: &UserLoadPlan,
    storage: &BootstrapStorage,
) -> Result<UserContext, UserProcessError> {
    Ok(UserContext {
        entry: plan.entry,
        // The compiler-generated `_start` follows the SysV entry convention:
        // RSP is 8 mod 16 on entry, so its prologue can align local FXSAVE
        // storage before executing SIMD instructions.
        user_stack_top: USER_STACK_LIMIT - 8,
        user_tls_base: USER_TLS_CONTROL_BASE,
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
            let segment_direct_end = segment
                .file_offset
                .checked_add(segment.file_size / PAGE_SIZE * PAGE_SIZE)
                .ok_or(UserProcessError::ImageOutOfBounds)?;
            let existing_direct_end = existing
                .file_offset
                .checked_add(existing.file_size / PAGE_SIZE * PAGE_SIZE)
                .ok_or(UserProcessError::ImageOutOfBounds)?;
            let direct_file_pages_overlap = segment.file_offset < existing_direct_end
                && existing.file_offset < segment_direct_end;
            let aliased_permissions_differ = (segment.flags ^ existing.flags) & (PF_W | PF_X) != 0;
            if direct_file_pages_overlap && aliased_permissions_differ {
                return Err(UserProcessError::InvalidLoadPlan);
            }
        }
        let file_end = segment
            .file_offset
            .checked_add(segment.file_size)
            .ok_or(UserProcessError::ImageOutOfBounds)?;
        if file_end > image.len() as u64 {
            return Err(UserProcessError::ImageOutOfBounds);
        }
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
    validate_tls_plan(plan, image)?;
    Ok(())
}

fn validate_tls_plan(plan: &UserLoadPlan, image: &[u8]) -> Result<(), UserProcessError> {
    let Some(tls) = plan.tls else {
        return Ok(());
    };
    let alignment = tls.alignment.max(1);
    if tls.memory_size == 0
        || tls.file_size > tls.memory_size
        || alignment > PAGE_SIZE
        || !alignment.is_power_of_two()
        || tls.virtual_address % alignment != tls.file_offset % alignment
    {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    let aligned_size = tls
        .memory_size
        .checked_add(alignment - 1)
        .ok_or(UserProcessError::InvalidLoadPlan)?
        & !(alignment - 1);
    if aligned_size > PAGE_SIZE {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    let memory_end = tls
        .virtual_address
        .checked_add(tls.memory_size)
        .ok_or(UserProcessError::InvalidLoadPlan)?;
    if tls.virtual_address < USER_IMAGE_BASE || memory_end > USER_IMAGE_LIMIT {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    let file_end = tls
        .file_offset
        .checked_add(tls.file_size)
        .ok_or(UserProcessError::ImageOutOfBounds)?;
    if file_end > image.len() as u64 {
        return Err(UserProcessError::ImageOutOfBounds);
    }
    let segments = &plan.segments[..plan.segment_count];
    let covered = segments.iter().any(|load| {
        let Some(load_memory_end) = load.virtual_address.checked_add(load.memory_size) else {
            return false;
        };
        if tls.virtual_address < load.virtual_address || memory_end > load_memory_end {
            return false;
        }
        if tls.file_size == 0 {
            return true;
        }
        let Some(load_file_end) = load.file_offset.checked_add(load.file_size) else {
            return false;
        };
        tls.file_offset >= load.file_offset
            && file_end <= load_file_end
            && tls.virtual_address - load.virtual_address == tls.file_offset - load.file_offset
    });
    if !covered {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    Ok(())
}

fn initialize_tls(
    plan: &UserLoadPlan,
    image: &[u8],
    storage: &mut BootstrapStorage,
) -> Result<(), UserProcessError> {
    // Nagi's x86-64 static TLS code loads the thread pointer from FS:0 before
    // applying the link-time negative TPOFF. Each fixed native thread slot
    // gets its own control page and initialized ABI thread-pointer word.
    storage.tls_pages[1].0[..core::mem::size_of::<u64>()]
        .copy_from_slice(&USER_TLS_CONTROL_BASE.to_le_bytes());
    storage.tls_pages[3].0[..core::mem::size_of::<u64>()]
        .copy_from_slice(&USER_TLS_CHILD_CONTROL_BASE.to_le_bytes());
    let Some(tls) = plan.tls else {
        return Ok(());
    };
    let alignment = tls.alignment.max(1);
    let aligned_size = tls
        .memory_size
        .checked_add(alignment - 1)
        .ok_or(UserProcessError::InvalidLoadPlan)?
        & !(alignment - 1);
    let start =
        usize::try_from(PAGE_SIZE - aligned_size).map_err(|_| UserProcessError::InvalidLoadPlan)?;
    let file_start =
        usize::try_from(tls.file_offset).map_err(|_| UserProcessError::ImageOutOfBounds)?;
    let file_end = usize::try_from(
        tls.file_offset
            .checked_add(tls.file_size)
            .ok_or(UserProcessError::ImageOutOfBounds)?,
    )
    .map_err(|_| UserProcessError::ImageOutOfBounds)?;
    let data_size =
        usize::try_from(tls.file_size).map_err(|_| UserProcessError::ImageOutOfBounds)?;
    let initialized_end = start
        .checked_add(data_size)
        .ok_or(UserProcessError::InvalidLoadPlan)?;
    if initialized_end > PAGE_SIZE as usize || file_end > image.len() {
        return Err(UserProcessError::ImageOutOfBounds);
    }
    // x86-64 TLS grows backward from FS base. The fixed page ends exactly at
    // the control page, so p_memsz bytes occupy its high end and p_filesz
    // supplies the initialized prefix; clear() already zeroed the remainder.
    storage.tls_initial_page.0[start..initialized_end]
        .copy_from_slice(&image[file_start..file_end]);
    storage.tls_pages[0]
        .0
        .copy_from_slice(&storage.tls_initial_page.0);
    reset_child_tls_pages(storage);
    Ok(())
}

fn reset_child_tls_pages(storage: &mut BootstrapStorage) {
    storage.tls_pages[2]
        .0
        .copy_from_slice(&storage.tls_initial_page.0);
    storage.tls_pages[3].0.fill(0);
    storage.tls_pages[3].0[..core::mem::size_of::<u64>()]
        .copy_from_slice(&USER_TLS_CHILD_CONTROL_BASE.to_le_bytes());
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
    let stack_pt = table_address(&storage.stack_pt)?;
    let tls_pt = table_address(&storage.tls_pt)?;
    let surface_pt = table_address(&storage.surface_pt)?;
    map_leaf(&mut storage.pml4, USER_PML4_INDEX, pdpt, hierarchy_flags)?;
    map_leaf(&mut storage.pdpt, USER_PDPT_INDEX, pd, hierarchy_flags)?;
    for table_index in 0..USER_IMAGE_PAGE_TABLE_COUNT {
        let image_pt = table_address(image_page_table(storage, table_index)?)?;
        map_leaf(
            &mut storage.pd,
            USER_IMAGE_FIRST_PD_INDEX + table_index,
            image_pt,
            hierarchy_flags,
        )?;
    }
    map_leaf(
        &mut storage.pd,
        user_pd_index(USER_STACK_BASE),
        stack_pt,
        hierarchy_flags,
    )?;
    map_leaf(
        &mut storage.pd,
        user_pd_index(USER_TLS_BASE),
        tls_pt,
        hierarchy_flags,
    )?;
    map_leaf(
        &mut storage.pd,
        user_pd_index(USER_SURFACE_BASE),
        surface_pt,
        hierarchy_flags,
    )?;
    for table_index in 0..USER_MMAP_PAGE_TABLES {
        let mmap_pt = table_address(&storage.mmap_pts[table_index])?;
        map_leaf(
            &mut storage.pd,
            user_pd_index(USER_MMAP_BASE) + table_index,
            mmap_pt,
            hierarchy_flags,
        )?;
    }
    Ok(())
}

fn user_pd_index(address: u64) -> usize {
    ((address >> 21) & 0x1ff) as usize
}

fn image_page_table(
    storage: &BootstrapStorage,
    table_index: usize,
) -> Result<&PageTable, UserProcessError> {
    if table_index == 0 {
        Ok(&storage.image_pt)
    } else {
        storage
            .image_extra_pts
            .get(table_index - 1)
            .ok_or(UserProcessError::InvalidLoadPlan)
    }
}

fn image_page_table_mut(
    storage: &mut BootstrapStorage,
    table_index: usize,
) -> Result<&mut PageTable, UserProcessError> {
    if table_index == 0 {
        Ok(&mut storage.image_pt)
    } else {
        storage
            .image_extra_pts
            .get_mut(table_index - 1)
            .ok_or(UserProcessError::InvalidLoadPlan)
    }
}

fn map_image_leaf(
    storage: &mut BootstrapStorage,
    image_page: usize,
    physical_address: u64,
    flags: u64,
) -> Result<(), UserProcessError> {
    let table_index = image_page / PAGE_TABLE_ENTRIES;
    let entry_index = image_page % PAGE_TABLE_ENTRIES;
    map_leaf(
        image_page_table_mut(storage, table_index)?,
        entry_index,
        physical_address,
        flags,
    )
}

fn map_segment_from_image(
    segment_index: usize,
    segment: &crate::user_elf::UserLoadSegment,
    image: &[u8],
    image_physical_base: u64,
    storage: &mut BootstrapStorage,
    allocator: &mut PageAllocator,
    active_cr3: u64,
    progress: &mut impl FnMut(PrepareStage),
) -> Result<(), UserProcessError> {
    let first_page = ((segment.virtual_address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    let page_count = segment.memory_size.div_ceil(PAGE_SIZE) as usize;
    progress(PrepareStage::LoadSegmentMapping {
        segment_index,
        page_count,
        file_bytes: usize::try_from(segment.file_size)
            .map_err(|_| UserProcessError::ImageTooLarge)?,
        memory_bytes: usize::try_from(segment.memory_size)
            .map_err(|_| UserProcessError::ImageTooLarge)?,
    });
    let mut leaf_flags = PageTableEntry::PRESENT | PageTableEntry::USER;
    if segment.flags & PF_W != 0 {
        leaf_flags |= PageTableEntry::WRITABLE;
    }
    if segment.flags & PF_X == 0 {
        leaf_flags |= PageTableEntry::NO_EXECUTE;
    }
    for relative_page in 0..page_count {
        let segment_offset = relative_page as u64 * PAGE_SIZE;
        let memory_bytes = (segment.memory_size - segment_offset).min(PAGE_SIZE);
        let file_bytes = segment
            .file_size
            .saturating_sub(segment_offset)
            .min(PAGE_SIZE);
        let physical = if memory_bytes == PAGE_SIZE && file_bytes == PAGE_SIZE {
            image_physical_base
                .checked_add(segment.file_offset)
                .and_then(|address| address.checked_add(segment_offset))
                .ok_or(UserProcessError::ImageOutOfBounds)?
        } else {
            let page = allocate_zeroed_page(allocator, active_cr3)?;
            if file_bytes != 0 {
                let source_start = usize::try_from(
                    segment
                        .file_offset
                        .checked_add(segment_offset)
                        .ok_or(UserProcessError::ImageOutOfBounds)?,
                )
                .map_err(|_| UserProcessError::ImageOutOfBounds)?;
                let source_end = source_start
                    .checked_add(file_bytes as usize)
                    .ok_or(UserProcessError::ImageOutOfBounds)?;
                let destination =
                    unsafe { slice::from_raw_parts_mut(page as *mut u8, PAGE_SIZE as usize) };
                destination[..file_bytes as usize].copy_from_slice(
                    image
                        .get(source_start..source_end)
                        .ok_or(UserProcessError::ImageOutOfBounds)?,
                );
            }
            page
        };
        map_image_leaf(storage, first_page + relative_page, physical, leaf_flags)?;
        if (relative_page + 1).is_multiple_of(1024) {
            progress(PrepareStage::LoadSegmentProgress {
                segment_index,
                mapped_pages: relative_page + 1,
                total_pages: page_count,
            });
        }
    }
    progress(PrepareStage::LoadSegmentMapped {
        segment_index,
        page_count,
    });
    Ok(())
}

fn allocate_zeroed_page(
    allocator: &mut PageAllocator,
    active_cr3: u64,
) -> Result<u64, UserProcessError> {
    let page = allocator
        .allocate_page_below(MAX_IDENTITY_MAPPED_ADDRESS)
        .ok_or(UserProcessError::PhysicalMemoryExhausted)?;
    if !unsafe { identity_mapped(active_cr3, page, PAGE_SIZE) } {
        let _ = allocator.free_page(page);
        return Err(UserProcessError::ImageNotMapped);
    }
    unsafe { ptr::write_bytes(page as *mut u8, 0, PAGE_SIZE as usize) };
    Ok(page)
}

#[cfg(test)]
fn map_segment_for_test(
    segment: &crate::user_elf::UserLoadSegment,
    image: &[u8],
    storage: &mut BootstrapStorage,
) -> Result<(), UserProcessError> {
    let first_page = ((segment.virtual_address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
    let page_count = segment.memory_size.div_ceil(PAGE_SIZE) as usize;
    if first_page + page_count > MAX_TEST_IMAGE_PAGES {
        return Err(UserProcessError::ImageTooLarge);
    }
    let mut leaf_flags = PageTableEntry::PRESENT | PageTableEntry::USER;
    if segment.flags & PF_W != 0 {
        leaf_flags |= PageTableEntry::WRITABLE;
    }
    if segment.flags & PF_X == 0 {
        leaf_flags |= PageTableEntry::NO_EXECUTE;
    }
    for page_index in first_page..first_page + page_count {
        let physical = page_address(&storage.image_pages[page_index])?;
        map_image_leaf(storage, page_index, physical, leaf_flags)?;
    }

    let destination_offset = (segment.virtual_address - USER_IMAGE_BASE) as usize;
    let source_start =
        usize::try_from(segment.file_offset).map_err(|_| UserProcessError::ImageOutOfBounds)?;
    let source_end = source_start
        .checked_add(segment.file_size as usize)
        .ok_or(UserProcessError::ImageOutOfBounds)?;
    let destination = image_storage_bytes(storage);
    let destination_end = destination_offset
        .checked_add(segment.file_size as usize)
        .ok_or(UserProcessError::ImageOutOfBounds)?;
    destination[destination_offset..destination_end]
        .copy_from_slice(&image[source_start..source_end]);
    Ok(())
}

#[cfg(test)]
fn image_storage_bytes(storage: &mut BootstrapStorage) -> &mut [u8] {
    let pointer = storage.image_pages.as_mut_ptr().cast::<u8>();
    unsafe { slice::from_raw_parts_mut(pointer, MAX_TEST_IMAGE_PAGES * PAGE_SIZE as usize) }
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

    use nagi_abi::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};
    use std::boxed::Box;

    use crate::memory::{PageTable, PageTableEntry, PAGE_SIZE, PAGE_TABLE_ENTRIES};
    use crate::user_elf::{
        UserLoadPlan, UserLoadSegment, UserTlsSegment, MAX_LOAD_SEGMENTS, USER_IMAGE_BASE,
        USER_IMAGE_LIMIT,
    };

    use super::{
        build_address_space, efer_with_nxe, image_range_is_mapped, mapped_range, mmap_page_flags,
        reset_child_tls_pages, validate_mmap_request, BootstrapStorage, UserProcessError,
        USER_MMAP_PAGES, USER_STACK_BASE, USER_STACK_LIMIT, USER_STACK_PAGES, USER_TLS_BASE,
        USER_TLS_CHILD_CONTROL_BASE, USER_TLS_CONTROL_BASE, USER_TLS_LIMIT,
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

    #[test]
    fn bootstrap_stack_is_two_mib_and_does_not_overlap_tls() {
        assert_eq!(USER_STACK_PAGES, PAGE_TABLE_ENTRIES);
        assert_eq!(USER_STACK_LIMIT - USER_STACK_BASE, 2 * 1024 * 1024);
        assert!(USER_STACK_LIMIT <= USER_TLS_BASE);
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
            tls: None,
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
        assert_eq!(context.user_tls_base, USER_TLS_CONTROL_BASE);
        assert_eq!(
            &storage.tls_pages[1].0[..core::mem::size_of::<u64>()],
            &USER_TLS_CONTROL_BASE.to_le_bytes()
        );
        assert_eq!(
            &storage.tls_pages[3].0[..core::mem::size_of::<u64>()],
            &USER_TLS_CHILD_CONTROL_BASE.to_le_bytes()
        );
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
        assert!(storage.tls_pt.raw_entry(1).is_some());
        assert!(storage.tls_pt.raw_entry(2).is_some());
        assert!(storage.tls_pt.raw_entry(3).is_some());
        assert!(mapped_range(
            &storage.tls_pt,
            USER_TLS_BASE,
            USER_TLS_LIMIT,
            USER_TLS_BASE,
            (USER_TLS_LIMIT - USER_TLS_BASE) as usize,
            PageTableEntry::PRESENT | PageTableEntry::USER,
        ));
    }

    #[test]
    fn initializes_static_tls_at_the_end_of_its_page() {
        let kernel_pml4 = PageTable::empty();
        let mut storage = boxed_storage();
        let mut image = [0x90_u8; 16];
        image[4..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let mut plan = plan(PAGE_SIZE);
        plan.tls = Some(UserTlsSegment {
            file_offset: 4,
            virtual_address: USER_IMAGE_BASE + 4,
            file_size: 4,
            memory_size: 16,
            alignment: 16,
        });

        let context = build_address_space(&plan, &image, &kernel_pml4, &mut storage)
            .expect("static TLS address space");

        let tls_start = PAGE_SIZE as usize - 16;
        assert_eq!(context.user_tls_base, USER_TLS_CONTROL_BASE);
        assert_eq!(
            &storage.tls_pages[0].0[tls_start..tls_start + 4],
            &[0x11, 0x22, 0x33, 0x44]
        );
        assert!(storage.tls_pages[0].0[tls_start + 4..]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(
            &storage.tls_pages[2].0[tls_start..tls_start + 4],
            &[0x11, 0x22, 0x33, 0x44]
        );
        assert!(storage.tls_pages[2].0[tls_start + 4..]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(
            &storage.tls_pages[1].0[..core::mem::size_of::<u64>()],
            &USER_TLS_CONTROL_BASE.to_le_bytes()
        );
        assert_eq!(
            &storage.tls_pages[3].0[..core::mem::size_of::<u64>()],
            &USER_TLS_CHILD_CONTROL_BASE.to_le_bytes()
        );
        assert!(storage.tls_pages[1].0[core::mem::size_of::<u64>()..]
            .iter()
            .all(|byte| *byte == 0));
        assert!(storage.tls_pages[3].0[core::mem::size_of::<u64>()..]
            .iter()
            .all(|byte| *byte == 0));
        assert!(storage.tls_pt.raw_entry(0).is_some());
        assert!(storage.tls_pt.raw_entry(1).is_some());
        assert!(storage.tls_pt.raw_entry(2).is_some());
        assert!(storage.tls_pt.raw_entry(3).is_some());
    }

    #[test]
    fn resets_reused_child_tls_from_the_original_template() {
        let kernel_pml4 = PageTable::empty();
        let mut storage = boxed_storage();
        let mut image = [0x90_u8; 16];
        image[4..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let mut plan = plan(PAGE_SIZE);
        plan.tls = Some(UserTlsSegment {
            file_offset: 4,
            virtual_address: USER_IMAGE_BASE + 4,
            file_size: 4,
            memory_size: 16,
            alignment: 16,
        });
        build_address_space(&plan, &image, &kernel_pml4, &mut storage)
            .expect("static TLS address space");

        let tls_start = PAGE_SIZE as usize - 16;
        storage.tls_pages[2].0[tls_start..tls_start + 4].fill(0xff);
        storage.tls_pages[3].0.fill(0xff);
        reset_child_tls_pages(&mut storage);

        assert_eq!(
            &storage.tls_pages[2].0[tls_start..tls_start + 4],
            &[0x11, 0x22, 0x33, 0x44]
        );
        assert!(storage.tls_pages[2].0[tls_start + 4..]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(
            &storage.tls_pages[3].0[..core::mem::size_of::<u64>()],
            &USER_TLS_CHILD_CONTROL_BASE.to_le_bytes()
        );
        assert!(storage.tls_pages[3].0[core::mem::size_of::<u64>()..]
            .iter()
            .all(|byte| *byte == 0));
    }

    #[test]
    fn rejects_an_image_larger_than_the_m17_image_window() {
        let kernel_pml4 = PageTable::empty();
        let mut storage = boxed_storage();
        let image = [0_u8; 16];

        assert_eq!(
            build_address_space(
                &plan(USER_IMAGE_LIMIT - USER_IMAGE_BASE + PAGE_SIZE),
                &image,
                &kernel_pml4,
                &mut storage,
            ),
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
        assert_eq!(
            validate_mmap_request(PAGE_SIZE, PROT_READ | PROT_WRITE),
            Some(1)
        );
        assert_eq!(validate_mmap_request(2 * PAGE_SIZE, PROT_NONE), Some(2));
        assert_eq!(validate_mmap_request(0, PROT_READ), None);
        assert_eq!(validate_mmap_request(PAGE_SIZE - 1, PROT_READ), None);
        assert_eq!(
            validate_mmap_request((USER_MMAP_PAGES as u64 + 1) * PAGE_SIZE, PROT_READ),
            None
        );
        assert_eq!(validate_mmap_request(PAGE_SIZE, 0b1000), None);
    }

    #[test]
    fn mmap_page_flags_attenuate_write_and_execute_independently() {
        let read_only = mmap_page_flags(PROT_READ);
        assert_ne!(read_only & PageTableEntry::PRESENT, 0);
        assert_eq!(read_only & PageTableEntry::WRITABLE, 0);
        assert_ne!(read_only & PageTableEntry::NO_EXECUTE, 0);

        let executable = mmap_page_flags(PROT_READ | PROT_EXEC);
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
