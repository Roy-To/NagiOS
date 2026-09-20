use core::mem::size_of;
use core::ptr;

use nagi_bootinfo::{BootInfo, MemoryMapEntry};

pub const PAGE_SIZE: u64 = 4096;
pub const PAGE_TABLE_ENTRIES: usize = 512;
const PAGE_TABLE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const HUGE_PAGE_FLAG: u64 = 1 << 7;
const CONVENTIONAL_MEMORY_TYPE: u32 = 7;
const MAX_RANGES: usize = 128;
const MAX_FREED_PAGES: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PageRange {
    start: u64,
    next: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocatorError {
    InvalidBootInfo,
    InvalidMemoryMap,
    AddressOverflow,
    TooManyRanges,
}

pub struct PageAllocator {
    ranges: [PageRange; MAX_RANGES],
    range_count: usize,
    freed: [u64; MAX_FREED_PAGES],
    freed_count: usize,
}

impl PageAllocator {
    pub const fn empty() -> Self {
        Self {
            ranges: [PageRange {
                start: 0,
                next: 0,
                end: 0,
            }; MAX_RANGES],
            range_count: 0,
            freed: [0; MAX_FREED_PAGES],
            freed_count: 0,
        }
    }

    /// # Safety
    ///
    /// `info` and its memory-map buffer must be valid, readable objects that
    /// remain alive while the allocator is constructed. The map must be the
    /// final map captured before `ExitBootServices`.
    pub unsafe fn from_boot_info(info: &BootInfo) -> Result<Self, AllocatorError> {
        if info.validate().is_err() {
            return Err(AllocatorError::InvalidBootInfo);
        }
        if info.memory_map.entry_size < size_of::<MemoryMapEntry>() as u64
            || !info.memory_map.entry_size.is_multiple_of(8)
        {
            return Err(AllocatorError::InvalidMemoryMap);
        }
        let entry_count = usize::try_from(info.memory_map.entry_count)
            .map_err(|_| AllocatorError::InvalidMemoryMap)?;
        let entry_size = usize::try_from(info.memory_map.entry_size)
            .map_err(|_| AllocatorError::InvalidMemoryMap)?;
        let base = info.memory_map.address as *const u8;
        let mut allocator = Self::empty();
        for index in 0..entry_count {
            let offset = index
                .checked_mul(entry_size)
                .ok_or(AllocatorError::AddressOverflow)?;
            let entry = ptr::read_unaligned(base.add(offset).cast::<MemoryMapEntry>());
            if entry.memory_type != CONVENTIONAL_MEMORY_TYPE || entry.page_count == 0 {
                continue;
            }
            if entry.physical_start % PAGE_SIZE != 0 {
                return Err(AllocatorError::InvalidMemoryMap);
            }
            let bytes = entry
                .page_count
                .checked_mul(PAGE_SIZE)
                .ok_or(AllocatorError::AddressOverflow)?;
            let end = entry
                .physical_start
                .checked_add(bytes)
                .ok_or(AllocatorError::AddressOverflow)?;
            if allocator.range_count == MAX_RANGES {
                return Err(AllocatorError::TooManyRanges);
            }
            allocator.ranges[allocator.range_count] = PageRange {
                start: entry.physical_start,
                next: entry.physical_start,
                end,
            };
            allocator.range_count += 1;
        }
        if allocator.range_count == 0 {
            return Err(AllocatorError::InvalidMemoryMap);
        }
        Ok(allocator)
    }

    #[allow(dead_code)]
    pub const fn from_single_range(start: u64, page_count: u64) -> Option<Self> {
        let bytes = match page_count.checked_mul(PAGE_SIZE) {
            Some(bytes) => bytes,
            None => return None,
        };
        let end = match start.checked_add(bytes) {
            Some(end) => end,
            None => return None,
        };
        if page_count == 0 || !start.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let mut allocator = Self::empty();
        allocator.ranges[0] = PageRange {
            start,
            next: start,
            end,
        };
        allocator.range_count = 1;
        Some(allocator)
    }

    pub fn allocate_page(&mut self) -> Option<u64> {
        if self.freed_count != 0 {
            self.freed_count -= 1;
            return Some(self.freed[self.freed_count]);
        }
        for range in &mut self.ranges[..self.range_count] {
            if range.next < range.end {
                let page = range.next;
                range.next += PAGE_SIZE;
                return Some(page);
            }
        }
        None
    }

    pub fn allocate_page_below(&mut self, upper_exclusive: u64) -> Option<u64> {
        if self.freed_count != 0 {
            let position = self.freed[..self.freed_count]
                .iter()
                .position(|page| *page < upper_exclusive);
            if let Some(position) = position {
                let last = self.freed_count - 1;
                let page = self.freed[position];
                self.freed[position] = self.freed[last];
                self.freed_count = last;
                return Some(page);
            }
        }
        for range in &mut self.ranges[..self.range_count] {
            if range.next < range.end && range.next < upper_exclusive {
                let page = range.next;
                range.next += PAGE_SIZE;
                return Some(page);
            }
        }
        None
    }

    pub fn free_page(&mut self, page: u64) -> bool {
        if !page.is_multiple_of(PAGE_SIZE)
            || !self.ranges[..self.range_count]
                .iter()
                .any(|range| page >= range.start && page < range.next)
        {
            return false;
        }
        if self.freed[..self.freed_count].contains(&page) || self.freed_count == MAX_FREED_PAGES {
            return false;
        }
        self.freed[self.freed_count] = page;
        self.freed_count += 1;
        true
    }

    pub const fn range_count(&self) -> usize {
        self.range_count
    }
}

/// Check that an identity-mapped virtual range is present in the active x86-64
/// page tables. This is used before an AP reuses the BSP address space.
///
/// # Safety
///
/// `cr3` must identify a readable page-table hierarchy in the current address
/// space. The hierarchy and the inspected entries must remain valid while the
/// check runs.
pub unsafe fn identity_mapped(cr3: u64, address: u64, length: u64) -> bool {
    if cr3 & (PAGE_SIZE - 1) != 0 || cr3 >= (1 << 52) || length == 0 {
        return false;
    }
    let end = match address.checked_add(length) {
        Some(end) => end,
        None => return false,
    };
    let mut page = address & !(PAGE_SIZE - 1);
    let last_page = (end - 1) & !(PAGE_SIZE - 1);
    loop {
        if !page_is_identity_mapped(cr3, page) {
            return false;
        }
        if page == last_page {
            return true;
        }
        page = match page.checked_add(PAGE_SIZE) {
            Some(page) => page,
            None => return false,
        };
    }
}

unsafe fn page_is_identity_mapped(cr3: u64, virtual_address: u64) -> bool {
    let pml4 = read_page_table_entry(cr3, (virtual_address >> 39) & 0x1ff);
    if pml4 & 1 == 0 {
        return false;
    }
    let pdpt = read_page_table_entry(
        pml4 & PAGE_TABLE_ADDRESS_MASK,
        (virtual_address >> 30) & 0x1ff,
    );
    if pdpt & 1 == 0 {
        return false;
    }
    if pdpt & HUGE_PAGE_FLAG != 0 {
        return (pdpt & 0x000f_ffff_c000_0000) + (virtual_address & 0x3fff_ffff) == virtual_address;
    }
    let pd = read_page_table_entry(
        pdpt & PAGE_TABLE_ADDRESS_MASK,
        (virtual_address >> 21) & 0x1ff,
    );
    if pd & 1 == 0 {
        return false;
    }
    if pd & HUGE_PAGE_FLAG != 0 {
        return (pd & 0x000f_ffff_ffe0_0000) + (virtual_address & 0x1f_ffff) == virtual_address;
    }
    let pt = read_page_table_entry(
        pd & PAGE_TABLE_ADDRESS_MASK,
        (virtual_address >> 12) & 0x1ff,
    );
    if pt & 1 == 0 {
        return false;
    }
    (pt & PAGE_TABLE_ADDRESS_MASK) + (virtual_address & (PAGE_SIZE - 1)) == virtual_address
}

unsafe fn read_page_table_entry(table: u64, index: u64) -> u64 {
    ptr::read_volatile((table + index * size_of::<u64>() as u64) as *const u64)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const NO_EXECUTE: u64 = 1 << 63;

    pub const fn new(physical_address: u64, flags: u64) -> Option<Self> {
        if physical_address & (PAGE_SIZE - 1) != 0 || physical_address >= (1 << 52) {
            return None;
        }
        Some(Self(physical_address | flags))
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn is_present(self) -> bool {
        self.0 & Self::PRESENT != 0
    }
}

#[repr(C, align(4096))]
pub struct PageTable {
    entries: [u64; PAGE_TABLE_ENTRIES],
}

impl PageTable {
    pub const fn empty() -> Self {
        Self {
            entries: [0; PAGE_TABLE_ENTRIES],
        }
    }

    pub fn map(&mut self, index: usize, entry: PageTableEntry) -> bool {
        self.replace_empty(index, entry.raw())
    }

    pub fn raw_entry(&self, index: usize) -> Option<u64> {
        self.entries.get(index).copied().filter(|entry| *entry != 0)
    }

    pub fn replace_empty(&mut self, index: usize, raw: u64) -> bool {
        let Some(slot) = self.entries.get_mut(index) else {
            return false;
        };
        if *slot != 0 || raw == 0 {
            return false;
        }
        *slot = raw;
        true
    }

    pub fn replace(&mut self, index: usize, raw: u64) -> bool {
        let Some(slot) = self.entries.get_mut(index) else {
            return false;
        };
        if raw == 0 || *slot == 0 {
            return false;
        }
        *slot = raw;
        true
    }

    pub fn clear(&mut self) {
        self.entries.fill(0);
    }

    pub fn entry(&self, index: usize) -> Option<PageTableEntry> {
        let raw = *self.entries.get(index)?;
        (raw != 0).then_some(PageTableEntry(raw))
    }

    pub fn unmap(&mut self, index: usize) -> Option<PageTableEntry> {
        let slot = self.entries.get_mut(index)?;
        let raw = *slot;
        if raw == 0 {
            return None;
        }
        *slot = 0;
        Some(PageTableEntry(raw))
    }
}

pub fn current_cr3() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value & PAGE_TABLE_ADDRESS_MASK
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelHeap {
    start: u64,
    next: u64,
    end: u64,
}

impl KernelHeap {
    pub const fn new(start: u64, size: u64) -> Option<Self> {
        if !start.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        let end = match start.checked_add(size) {
            Some(end) if size != 0 => end,
            _ => return None,
        };
        Some(Self {
            start,
            next: start,
            end,
        })
    }

    pub fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
        if size == 0 || !alignment.is_power_of_two() {
            return None;
        }
        let aligned = self.next.checked_add(alignment - 1)? & !(alignment - 1);
        let end = aligned.checked_add(size)?;
        if end > self.end {
            return None;
        }
        self.next = end;
        Some(aligned)
    }

    pub const fn reset(&mut self) {
        self.next = self.start;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::mem::size_of;
    use nagi_bootinfo::{BootInfo, FramebufferInfo, MemoryMapEntry, MemoryMapInfo};

    use super::{KernelHeap, PageAllocator, PageTable, PageTableEntry, PAGE_SIZE};

    #[test]
    fn builds_allocator_from_conventional_uefi_pages_and_reuses_freed_page() {
        let entries = [
            MemoryMapEntry {
                memory_type: 7,
                physical_start: 0x10_0000,
                page_count: 2,
                ..MemoryMapEntry::default()
            },
            MemoryMapEntry {
                memory_type: 5,
                physical_start: 0x20_0000,
                page_count: 10,
                ..MemoryMapEntry::default()
            },
        ];
        let info = BootInfo {
            memory_map: MemoryMapInfo {
                address: entries.as_ptr() as u64,
                entry_count: entries.len() as u64,
                entry_size: size_of::<MemoryMapEntry>() as u64,
                entry_version: 1,
                _reserved: 0,
            },
            framebuffer: FramebufferInfo {
                address: 0xE000_0000,
                byte_size: 4096,
                width: 1,
                height: 1,
                pixels_per_scanline: 1,
                pixel_format: 0,
            },
            acpi_rsdp: 0xF0000,
            init_image: nagi_bootinfo::InitImageInfo {
                address: 0x30_0000,
                size: 4096,
            },
            ..BootInfo::new()
        };
        let mut allocator = unsafe { PageAllocator::from_boot_info(&info) }.expect("allocator");
        assert_eq!(allocator.range_count(), 1);
        let page = allocator.allocate_page().expect("page");
        assert_eq!(page, 0x10_0000);
        assert!(allocator.free_page(page));
        assert_eq!(allocator.allocate_page(), Some(page));
    }

    #[test]
    fn rejects_invalid_frees_and_exhausts_a_single_range() {
        let mut allocator = PageAllocator::from_single_range(0x20_0000, 1).expect("allocator");
        assert_eq!(allocator.allocate_page(), Some(0x20_0000));
        assert_eq!(allocator.allocate_page(), None);
        assert!(!allocator.free_page(0x20_0001));
        assert!(!allocator.free_page(0x30_0000));
        assert!(allocator.free_page(0x20_0000));
        assert!(!allocator.free_page(0x20_0000));
    }

    #[test]
    fn encodes_page_table_entries_and_allocates_aligned_heap_blocks() {
        let entry = PageTableEntry::new(
            0x30_0000,
            PageTableEntry::PRESENT | PageTableEntry::WRITABLE,
        )
        .expect("entry");
        assert!(entry.is_present());
        assert_eq!(entry.raw(), 0x30_0003);
        assert!(PageTableEntry::new(0x300001, PageTableEntry::PRESENT).is_none());

        let mut heap = KernelHeap::new(0x40_0000, PAGE_SIZE * 2).expect("heap");
        assert_eq!(heap.allocate(7, 8), Some(0x40_0000));
        assert_eq!(heap.allocate(1, PAGE_SIZE), Some(0x40_1000));
        assert!(heap.allocate(1, PAGE_SIZE).is_none());
        heap.reset();
        assert_eq!(heap.allocate(1, 1), Some(0x40_0000));
    }

    #[test]
    fn maps_and_unmaps_entries_in_a_page_table() {
        let entry = PageTableEntry::new(
            0x50_0000,
            PageTableEntry::PRESENT | PageTableEntry::WRITABLE,
        )
        .expect("entry");
        let mut table = PageTable::empty();
        assert!(table.map(7, entry));
        assert_eq!(table.entry(7), Some(entry));
        assert!(!table.map(7, entry));
        assert_eq!(table.unmap(7), Some(entry));
        assert_eq!(table.entry(7), None);
        assert!(!table.map(512, entry));
    }

    #[test]
    fn exposes_raw_entries_and_clears_the_table() {
        let entry = PageTableEntry::new(0x60_0000, PageTableEntry::PRESENT | PageTableEntry::USER)
            .expect("entry");
        let mut table = PageTable::empty();

        assert!(table.replace_empty(128, entry.raw()));
        assert_eq!(table.raw_entry(128), Some(entry.raw()));
        assert!(!table.replace_empty(128, entry.raw()));
        table.clear();
        assert_eq!(table.raw_entry(128), None);
    }
}
