use crate::page::PAGE_SIZE;
use core::ptr;

const MIN_SLOT_SIZE: usize = core::mem::size_of::<*mut u8>();

/// one slab = one page split into equal slots
pub struct Slab {
    page_ptr: *mut u8,
    slot_size: usize,
    total_slots: usize,
    used_slots: usize,
    free_head: *mut u8,
}

impl Slab {
    /// # Safety
    ///
    /// page must point to at least PAGE_SIZE bytes of writable memory
    /// page should be aligned properly
    /// nobody else should touch this page while the slab owns it
    pub unsafe fn new(page: *mut u8, object_size: usize) -> Self {
        let slot_size = object_size.max(MIN_SLOT_SIZE);
        let total_slots = PAGE_SIZE / slot_size;

        assert!(total_slots > 0, "object_size too large for a single page");

        for i in 0..total_slots {
            // safety: stays within page bounds since i * slot_size < PAGE_SIZE
            let slot = unsafe { page.add(i * slot_size) };
            let next = if i + 1 < total_slots {
                unsafe { page.add((i + 1) * slot_size) }
            } else {
                ptr::null_mut()
            };
            // safety: each slot is at least MIN_SLOT_SIZE so it can hold a ptr
            unsafe {
                (slot as *mut *mut u8).write(next);
            }
        }

        Slab {
            page_ptr: page,
            slot_size,
            total_slots,
            used_slots: 0,
            free_head: page,
        }
    }

    /// grab a slot from the free list, null if slab is full
    pub fn alloc(&mut self) -> *mut u8 {
        if self.free_head.is_null() {
            return ptr::null_mut();
        }

        let slot = self.free_head;
        // safety: free_head is either null (handled above) or valid slot in our page
        self.free_head = unsafe { (slot as *mut *mut u8).read() };
        self.used_slots += 1;
        slot
    }

    /// # Safety
    ///
    /// ptr must come from alloc() on THIS slab
    /// no double free !!
    pub unsafe fn dealloc(&mut self, ptr: *mut u8) {
        debug_assert!(self.contains(ptr));
        debug_assert!(self.used_slots > 0);

        // safety: ptr is valid slot in our page, big enough for a pointer
        unsafe {
            (ptr as *mut *mut u8).write(self.free_head);
        }
        self.free_head = ptr;
        self.used_slots -= 1;
    }

    /// true if no more free slots
    pub fn is_full(&self) -> bool {
        self.used_slots == self.total_slots
    }

    /// true if nothing allocated
    pub fn is_empty(&self) -> bool {
        self.used_slots == 0
    }

    /// how many slots are available
    pub fn free_slots(&self) -> usize {
        self.total_slots - self.used_slots
    }

    /// total slot count in this slab
    pub fn total_slots(&self) -> usize {
        self.total_slots
    }

    /// currently used slots
    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    /// bytes per slot
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// the underlying page pointer
    pub fn page_ptr(&self) -> *mut u8 {
        self.page_ptr
    }

    /// check if ptr is somewhere inside this slab's page
    pub fn contains(&self, ptr: *mut u8) -> bool {
        let start = self.page_ptr as usize;
        let end = start + PAGE_SIZE;
        let addr = ptr as usize;
        addr >= start && addr < end
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn aligned_page() -> Vec<u8> {
        vec![0u8; PAGE_SIZE * 2]
    }

    fn page_aligned_ptr(buf: &mut Vec<u8>) -> *mut u8 {
        let ptr = buf.as_mut_ptr();
        let align_offset = ptr.align_offset(PAGE_SIZE);
        unsafe { ptr.add(align_offset) }
    }

    #[test]
    fn test_slot_count() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);

        let slab = unsafe { Slab::new(page, 64) };
        assert_eq!(slab.total_slots(), 64);
        assert_eq!(slab.used_slots(), 0);
        assert_eq!(slab.free_slots(), 64);
        assert!(slab.is_empty());
        assert!(!slab.is_full());
    }

    #[test]
    fn test_minimum_slot_size() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);

        let slab = unsafe { Slab::new(page, 1) };
        assert_eq!(slab.slot_size(), MIN_SLOT_SIZE);
        assert_eq!(slab.total_slots(), PAGE_SIZE / MIN_SLOT_SIZE);
    }

    #[test]
    fn test_alloc_returns_unique_pointers() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 128) };

        let total = slab.total_slots();
        let mut ptrs = Vec::new();

        for _ in 0..total {
            let ptr = slab.alloc();
            assert!(!ptr.is_null());
            assert!(!ptrs.contains(&ptr));
            ptrs.push(ptr);
        }

        assert!(slab.is_full());
        assert_eq!(slab.alloc(), core::ptr::null_mut());
    }

    #[test]
    fn test_alloc_pointers_are_slot_aligned() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 64) };

        for _ in 0..slab.total_slots() {
            let ptr = slab.alloc();
            assert!(!ptr.is_null());
            let offset = (ptr as usize) - (page as usize);
            assert_eq!(offset % 64, 0);
        }
    }

    #[test]
    fn test_full_returns_null() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 2048) };

        assert!(!slab.alloc().is_null());
        assert!(!slab.alloc().is_null());
        assert!(slab.alloc().is_null());
        assert!(slab.is_full());
    }

    #[test]
    fn test_dealloc_makes_slot_reusable() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 64) };

        let ptr = slab.alloc();
        assert_eq!(slab.used_slots(), 1);

        unsafe { slab.dealloc(ptr) };
        assert_eq!(slab.used_slots(), 0);
        assert!(slab.is_empty());

        let ptr2 = slab.alloc();
        assert_eq!(ptr, ptr2);
    }

    #[test]
    fn test_alloc_dealloc_cycle() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 256) };

        let mut ptrs: Vec<*mut u8> = (0..16).map(|_| slab.alloc()).collect();
        assert!(slab.is_full());

        for i in (0..16).step_by(2) {
            unsafe { slab.dealloc(ptrs[i]) };
        }
        assert_eq!(slab.used_slots(), 8);
        assert_eq!(slab.free_slots(), 8);

        for i in (0..16).step_by(2) {
            let new_ptr = slab.alloc();
            assert!(!new_ptr.is_null());
            ptrs[i] = new_ptr;
        }
        assert!(slab.is_full());

        for ptr in ptrs {
            unsafe { slab.dealloc(ptr) };
        }
        assert!(slab.is_empty());
    }

    #[test]
    fn test_write_read_memory() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let mut slab = unsafe { Slab::new(page, 64) };

        let ptr1 = slab.alloc();
        let ptr2 = slab.alloc();

        unsafe {
            core::ptr::write_bytes(ptr1, 0xAA, 64);
            core::ptr::write_bytes(ptr2, 0xBB, 64);
        }

        unsafe {
            assert_eq!(*ptr1, 0xAA);
            assert_eq!(*ptr2, 0xBB);
        }
    }

    #[test]
    fn test_contains() {
        let mut buf = aligned_page();
        let page = page_aligned_ptr(&mut buf);
        let slab = unsafe { Slab::new(page, 64) };

        assert!(slab.contains(page));
        assert!(slab.contains(unsafe { page.add(2048) }));
        assert!(!slab.contains(core::ptr::null_mut()));
        assert!(!slab.contains(unsafe { page.add(PAGE_SIZE) }));
    }
}
