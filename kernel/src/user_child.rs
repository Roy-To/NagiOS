//! Bounded isolated address space for one Supervisor-spawned process
//! (ADR 0043).
//!
//! The child gets its own PML4. Only the kernel's supervisor-only entries
//! are copied into it. Its user half contains just its own ELF pages and a
//! small stack. No page of the init image, TLS, mmap window, or Surface is
//! mapped, so the child cannot address init memory at all.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::memory::{PageTable, PageTableEntry, PAGE_SIZE, PAGE_TABLE_ENTRIES};
use crate::user_elf::{self, UserLoadPlan, PF_W, PF_X, USER_IMAGE_BASE};

use super::{
    map_leaf, mapped_range, page_address, table_address, user_pd_index, validate_plan, PageBytes,
    UserProcessError, USER_IMAGE_FIRST_PD_INDEX, USER_PDPT_INDEX, USER_PML4_INDEX, USER_STACK_BASE,
    USER_STACK_LIMIT,
};

/// Kernel Process ID of the bootstrap `nagi-init` process.
pub const INIT_PROCESS_ID: u32 = 1;
/// Process ID of the first isolated child. Later children receive
/// increasing IDs from `process_exit::ExitTable` (ADR 0048).
pub const CHILD_PROCESS_ID: u32 = crate::process_exit::FIRST_ISOLATED_PROCESS_ID;
/// Upper bound on the child ELF's mapped image: 1 MiB.
pub const CHILD_IMAGE_PAGES: usize = 256;
/// Child stack: 64 KiB at the top of the stack span, below an unmapped guard.
pub const CHILD_STACK_PAGES: usize = 16;
pub const CHILD_IMAGE_LIMIT: u64 = USER_IMAGE_BASE + CHILD_IMAGE_PAGES as u64 * PAGE_SIZE;
pub const CHILD_STACK_BASE: u64 = USER_STACK_LIMIT - CHILD_STACK_PAGES as u64 * PAGE_SIZE;
/// Bound on the ELF file the Supervisor may pass to `SYS_PROCESS_SPAWN`.
pub const MAX_CHILD_ELF_BYTES: usize = 2 * 1024 * 1024;

const _: () = assert!(CHILD_IMAGE_PAGES <= PAGE_TABLE_ENTRIES);
const _: () = assert!(CHILD_STACK_PAGES < PAGE_TABLE_ENTRIES);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChildContext {
    pub process_id: u32,
    pub entry: u64,
    pub user_stack_top: u64,
    pub cr3: u64,
}

#[repr(C, align(4096))]
pub(crate) struct ChildStorage {
    pml4: PageTable,
    pdpt: PageTable,
    pd: PageTable,
    image_pt: PageTable,
    stack_pt: PageTable,
    image_pages: [PageBytes; CHILD_IMAGE_PAGES],
    stack_pages: [PageBytes; CHILD_STACK_PAGES],
}

impl ChildStorage {
    pub(crate) const fn new() -> Self {
        Self {
            pml4: PageTable::empty(),
            pdpt: PageTable::empty(),
            pd: PageTable::empty(),
            image_pt: PageTable::empty(),
            stack_pt: PageTable::empty(),
            image_pages: [const { PageBytes::zeroed() }; CHILD_IMAGE_PAGES],
            stack_pages: [const { PageBytes::zeroed() }; CHILD_STACK_PAGES],
        }
    }

    fn clear(&mut self) {
        self.pml4.clear();
        self.pdpt.clear();
        self.pd.clear();
        self.image_pt.clear();
        self.stack_pt.clear();
        for page in &mut self.image_pages {
            page.0.fill(0);
        }
        for page in &mut self.stack_pages {
            page.0.fill(0);
        }
    }
}

struct ChildCell(UnsafeCell<ChildStorage>);

// The child slot is claimed through `CHILD_IN_USE`; storage is only mutated
// by the claimant while the child is not the active address space.
unsafe impl Sync for ChildCell {}

static CHILD_STORAGE: ChildCell = ChildCell(UnsafeCell::new(ChildStorage::new()));
static CHILD_IN_USE: AtomicBool = AtomicBool::new(false);
static ACTIVE_PROCESS: AtomicU32 = AtomicU32::new(INIT_PROCESS_ID);

/// Kernel Process ID whose address space is loaded in CR3. Every user-pointer
/// check consults this process's mappings only.
pub fn active_process() -> u32 {
    ACTIVE_PROCESS.load(Ordering::Acquire)
}

/// Record the process whose address space the caller just loaded.
pub fn set_active_process(process_id: u32) {
    ACTIVE_PROCESS.store(process_id, Ordering::Release);
}

/// Parse, validate and load a child ELF into the isolated slot.
///
/// `kernel_pml4` must be the active (init) PML4. Only its kernel entries are
/// copied, and the user slot is left out. The ELF bytes are copied into
/// child-owned pages, so later changes to the caller's buffer cannot affect
/// the child.
pub fn prepare_child(
    image: &[u8],
    kernel_pml4: &PageTable,
    process_id: u32,
) -> Result<ChildContext, UserProcessError> {
    if image.len() > MAX_CHILD_ELF_BYTES {
        return Err(UserProcessError::ImageTooLarge);
    }
    let plan = user_elf::parse(image).map_err(UserProcessError::InvalidElf)?;
    validate_child_plan(&plan, image)?;
    if CHILD_IN_USE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(UserProcessError::KernelUserSlotOccupied);
    }
    let storage = unsafe { &mut *CHILD_STORAGE.0.get() };
    let result =
        build_child_address_space(&plan, image, kernel_pml4, storage).map(|context| ChildContext {
            process_id,
            ..context
        });
    if result.is_err() {
        storage.clear();
        CHILD_IN_USE.store(false, Ordering::Release);
    }
    result
}

/// Scrub and release the child slot after its process exits.
///
/// # Safety
///
/// The child's PML4 must not be the active CR3 on any CPU.
pub unsafe fn release_child() {
    if !CHILD_IN_USE.load(Ordering::Acquire) {
        return;
    }
    (*CHILD_STORAGE.0.get()).clear();
    CHILD_IN_USE.store(false, Ordering::Release);
}

fn validate_child_plan(plan: &UserLoadPlan, image: &[u8]) -> Result<(), UserProcessError> {
    validate_plan(plan, image)?;
    if plan.tls.is_some() {
        return Err(UserProcessError::InvalidLoadPlan);
    }
    for segment in &plan.segments[..plan.segment_count] {
        let end = segment
            .virtual_address
            .checked_add(segment.memory_size)
            .ok_or(UserProcessError::ImageTooLarge)?;
        if end > CHILD_IMAGE_LIMIT {
            return Err(UserProcessError::ImageTooLarge);
        }
    }
    Ok(())
}

pub(crate) fn build_child_address_space(
    plan: &UserLoadPlan,
    image: &[u8],
    kernel_pml4: &PageTable,
    storage: &mut ChildStorage,
) -> Result<ChildContext, UserProcessError> {
    validate_child_plan(plan, image)?;
    storage.clear();
    for index in 0..PAGE_TABLE_ENTRIES {
        if index == USER_PML4_INDEX {
            continue;
        }
        if let Some(raw) = kernel_pml4.raw_entry(index) {
            if !storage
                .pml4
                .replace_empty(index, raw & !PageTableEntry::USER)
            {
                return Err(UserProcessError::InvalidLoadPlan);
            }
        }
    }

    let hierarchy = PageTableEntry::PRESENT | PageTableEntry::WRITABLE | PageTableEntry::USER;
    let pdpt = table_address(&storage.pdpt)?;
    let pd = table_address(&storage.pd)?;
    let image_pt = table_address(&storage.image_pt)?;
    let stack_pt = table_address(&storage.stack_pt)?;
    map_leaf(&mut storage.pml4, USER_PML4_INDEX, pdpt, hierarchy)?;
    map_leaf(&mut storage.pdpt, USER_PDPT_INDEX, pd, hierarchy)?;
    map_leaf(
        &mut storage.pd,
        USER_IMAGE_FIRST_PD_INDEX,
        image_pt,
        hierarchy,
    )?;
    map_leaf(
        &mut storage.pd,
        user_pd_index(USER_STACK_BASE),
        stack_pt,
        hierarchy,
    )?;

    let first_stack_entry = PAGE_TABLE_ENTRIES - CHILD_STACK_PAGES;
    for index in 0..CHILD_STACK_PAGES {
        let physical = page_address(&storage.stack_pages[index])?;
        map_leaf(
            &mut storage.stack_pt,
            first_stack_entry + index,
            physical,
            PageTableEntry::PRESENT
                | PageTableEntry::WRITABLE
                | PageTableEntry::USER
                | PageTableEntry::NO_EXECUTE,
        )?;
    }

    for segment in &plan.segments[..plan.segment_count] {
        let first_page = ((segment.virtual_address - USER_IMAGE_BASE) / PAGE_SIZE) as usize;
        let page_count = segment.memory_size.div_ceil(PAGE_SIZE) as usize;
        let mut flags = PageTableEntry::PRESENT | PageTableEntry::USER;
        if segment.flags & PF_W != 0 {
            flags |= PageTableEntry::WRITABLE;
        }
        if segment.flags & PF_X == 0 {
            flags |= PageTableEntry::NO_EXECUTE;
        }
        for relative in 0..page_count {
            let page_index = first_page + relative;
            let page = storage
                .image_pages
                .get_mut(page_index)
                .ok_or(UserProcessError::ImageTooLarge)?;
            let offset = relative as u64 * PAGE_SIZE;
            let file_bytes = segment.file_size.saturating_sub(offset).min(PAGE_SIZE) as usize;
            if file_bytes != 0 {
                let start = usize::try_from(segment.file_offset + offset)
                    .map_err(|_| UserProcessError::ImageOutOfBounds)?;
                let source = image
                    .get(start..start + file_bytes)
                    .ok_or(UserProcessError::ImageOutOfBounds)?;
                page.0[..file_bytes].copy_from_slice(source);
            }
            let physical = page_address(&storage.image_pages[page_index])?;
            map_leaf(&mut storage.image_pt, page_index, physical, flags)?;
        }
    }

    Ok(ChildContext {
        process_id: CHILD_PROCESS_ID,
        entry: plan.entry,
        // Same SysV entry convention as the init process.
        user_stack_top: USER_STACK_LIMIT - 8,
        cr3: table_address(&storage.pml4)?,
    })
}

fn child_storage() -> &'static ChildStorage {
    unsafe { &*CHILD_STORAGE.0.get() }
}

fn image_or_stack_mapped(
    storage: &ChildStorage,
    address: u64,
    length: usize,
    required: u64,
) -> bool {
    mapped_range(
        &storage.image_pt,
        USER_IMAGE_BASE,
        CHILD_IMAGE_LIMIT,
        address,
        length,
        required,
    ) || mapped_range(
        &storage.stack_pt,
        USER_STACK_BASE,
        USER_STACK_LIMIT,
        address,
        length,
        required,
    )
}

pub(crate) fn child_readable(address: u64, length: usize) -> bool {
    CHILD_IN_USE.load(Ordering::Acquire)
        && image_or_stack_mapped(
            child_storage(),
            address,
            length,
            PageTableEntry::PRESENT | PageTableEntry::USER,
        )
}

pub(crate) fn child_writable(address: u64, length: usize) -> bool {
    CHILD_IN_USE.load(Ordering::Acquire)
        && image_or_stack_mapped(
            child_storage(),
            address,
            length,
            PageTableEntry::PRESENT | PageTableEntry::USER | PageTableEntry::WRITABLE,
        )
}

pub(crate) fn child_executable(address: u64, length: usize) -> bool {
    if !CHILD_IN_USE.load(Ordering::Acquire) || length == 0 || address < USER_IMAGE_BASE {
        return false;
    }
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > CHILD_IMAGE_LIMIT {
        return false;
    }
    let storage = child_storage();
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

#[cfg(test)]
mod tests {
    extern crate std;

    use std::boxed::Box;

    use crate::memory::{PageTable, PageTableEntry, PAGE_SIZE, PAGE_TABLE_ENTRIES};
    use crate::user_elf::{UserLoadPlan, UserLoadSegment, MAX_LOAD_SEGMENTS, USER_IMAGE_BASE};

    use super::{
        build_child_address_space, image_or_stack_mapped, ChildStorage, CHILD_IMAGE_LIMIT,
        CHILD_PROCESS_ID, CHILD_STACK_BASE, CHILD_STACK_PAGES,
    };
    use crate::user_process::{
        UserProcessError, USER_MMAP_BASE, USER_PML4_INDEX, USER_STACK_LIMIT, USER_TLS_BASE,
    };

    fn boxed_storage() -> Box<ChildStorage> {
        let mut allocation = Box::<ChildStorage>::new_uninit();
        unsafe {
            allocation
                .as_mut_ptr()
                .cast::<u8>()
                .write_bytes(0, core::mem::size_of::<ChildStorage>());
            allocation.assume_init()
        }
    }

    fn plan(segments: &[UserLoadSegment], entry: u64) -> UserLoadPlan {
        let mut all = [UserLoadSegment {
            file_offset: 0,
            virtual_address: 0,
            file_size: 0,
            memory_size: 0,
            flags: 0,
        }; MAX_LOAD_SEGMENTS];
        all[..segments.len()].copy_from_slice(segments);
        UserLoadPlan {
            entry,
            segments: all,
            segment_count: segments.len(),
            tls: None,
        }
    }

    fn two_segment_image() -> ([u8; 3 * 4096], UserLoadPlan) {
        let mut image = [0u8; 3 * 4096];
        image[0x1000..0x1004].copy_from_slice(&[0x90, 0x90, 0x0f, 0x05]);
        image[0x2000..0x2005].copy_from_slice(b"child");
        let segments = [
            UserLoadSegment {
                file_offset: 0x1000,
                virtual_address: USER_IMAGE_BASE,
                file_size: 4,
                memory_size: 4,
                flags: super::PF_X,
            },
            UserLoadSegment {
                file_offset: 0x2000,
                virtual_address: USER_IMAGE_BASE + PAGE_SIZE,
                file_size: 5,
                memory_size: 2 * PAGE_SIZE,
                flags: super::PF_W,
            },
        ];
        (image, plan(&segments, USER_IMAGE_BASE))
    }

    fn kernel_pml4() -> Box<PageTable> {
        let mut table = Box::new(PageTable::empty());
        assert!(table.replace_empty(0, 0x1000 | PageTableEntry::PRESENT | PageTableEntry::USER));
        assert!(table.replace_empty(511, 0x2000 | PageTableEntry::PRESENT));
        table
    }

    #[test]
    fn child_address_space_maps_only_its_image_and_bounded_stack() {
        let (image, plan) = two_segment_image();
        let mut storage = boxed_storage();
        let context =
            build_child_address_space(&plan, &image, &kernel_pml4(), &mut storage).expect("load");
        assert_eq!(context.process_id, CHILD_PROCESS_ID);
        assert_eq!(context.entry, USER_IMAGE_BASE);
        assert_eq!(context.user_stack_top, USER_STACK_LIMIT - 8);

        // Kernel entries are copied supervisor-only; the user slot is ours.
        let kernel_entry = storage.pml4.raw_entry(0).expect("kernel entry");
        assert_eq!(kernel_entry & PageTableEntry::USER, 0);
        assert!(storage.pml4.raw_entry(511).is_some());
        assert!(storage.pml4.raw_entry(USER_PML4_INDEX).is_some());

        let present_user = PageTableEntry::PRESENT | PageTableEntry::USER;
        let writable = present_user | PageTableEntry::WRITABLE;
        assert!(image_or_stack_mapped(
            &storage,
            USER_IMAGE_BASE,
            4,
            present_user
        ));
        assert!(!image_or_stack_mapped(
            &storage,
            USER_IMAGE_BASE,
            4,
            writable
        ));
        assert!(image_or_stack_mapped(
            &storage,
            USER_IMAGE_BASE + PAGE_SIZE,
            2 * PAGE_SIZE as usize,
            writable
        ));
        assert!(!image_or_stack_mapped(
            &storage,
            USER_IMAGE_BASE + 3 * PAGE_SIZE,
            1,
            present_user
        ));
        assert!(image_or_stack_mapped(
            &storage,
            CHILD_STACK_BASE,
            CHILD_STACK_PAGES * PAGE_SIZE as usize,
            writable
        ));
        // Guard page below the stack, init-only TLS and mmap windows are absent.
        assert!(!image_or_stack_mapped(
            &storage,
            CHILD_STACK_BASE - 1,
            1,
            present_user
        ));
        assert!(!image_or_stack_mapped(
            &storage,
            USER_TLS_BASE,
            1,
            present_user
        ));
        assert!(!image_or_stack_mapped(
            &storage,
            USER_MMAP_BASE,
            1,
            present_user
        ));

        assert_eq!(&storage.image_pages[0].0[..4], &[0x90, 0x90, 0x0f, 0x05]);
        assert_eq!(&storage.image_pages[1].0[..5], b"child");
        assert!(storage.image_pages[2].0.iter().all(|byte| *byte == 0));
        let code = storage.image_pt.raw_entry(0).expect("code page");
        assert_eq!(code & PageTableEntry::NO_EXECUTE, 0);
        let data = storage.image_pt.raw_entry(1).expect("data page");
        assert_ne!(data & PageTableEntry::NO_EXECUTE, 0);
        assert_eq!(
            storage
                .stack_pt
                .raw_entry(PAGE_TABLE_ENTRIES - CHILD_STACK_PAGES - 1),
            None
        );
    }

    #[test]
    fn child_rejects_tls_and_images_beyond_its_bound() {
        let (image, mut plan) = two_segment_image();
        let mut storage = boxed_storage();
        plan.segments[1].memory_size = CHILD_IMAGE_LIMIT - USER_IMAGE_BASE;
        assert_eq!(
            build_child_address_space(&plan, &image, &kernel_pml4(), &mut storage),
            Err(UserProcessError::ImageTooLarge)
        );

        let (image, mut plan) = two_segment_image();
        plan.tls = Some(crate::user_elf::UserTlsSegment {
            file_offset: 0x2000,
            virtual_address: USER_IMAGE_BASE + PAGE_SIZE,
            file_size: 0,
            memory_size: 8,
            alignment: 8,
        });
        assert_eq!(
            build_child_address_space(&plan, &image, &kernel_pml4(), &mut storage),
            Err(UserProcessError::InvalidLoadPlan)
        );
    }

    #[test]
    fn rebuilding_scrubs_previous_child_pages() {
        let (image, plan) = two_segment_image();
        let mut storage = boxed_storage();
        build_child_address_space(&plan, &image, &kernel_pml4(), &mut storage).expect("load");
        storage.stack_pages[0].0[0] = 0xaa;
        let mut small = plan;
        small.segment_count = 1;
        build_child_address_space(&small, &image, &kernel_pml4(), &mut storage).expect("reload");
        assert_eq!(storage.stack_pages[0].0[0], 0);
        assert!(storage.image_pt.raw_entry(1).is_none());
        assert!(storage.image_pages[1].0.iter().all(|byte| *byte == 0));
    }
}
