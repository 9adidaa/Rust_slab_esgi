use crate::page::PAGE_SIZE;
use core::ptr;

const MIN_SLOT_SIZE: usize = core::mem::size_of::<*mut u8>();

pub struct Slab {
    page_ptr: *mut u8,
    slot_size: usize,
    total_slots: usize,
    used_slots: usize,
    free_head: *mut u8,
}

impl Slab {
    pub unsafe fn new(page: *mut u8, object_size: usize) -> Self {
        let slot_size = object_size.max(MIN_SLOT_SIZE);
        let total_slots = PAGE_SIZE / slot_size;

        assert!(total_slots > 0, "object_size too large for a single page");

        for i in 0..total_slots {
            let slot = unsafe { page.add(i * slot_size) };
            let next = if i + 1 < total_slots {
                unsafe { page.add((i + 1) * slot_size) }
            } else {
                ptr::null_mut()
            };
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

    pub fn alloc(&mut self) -> *mut u8 {
        if self.free_head.is_null() {
            return ptr::null_mut();
        }

        let slot = self.free_head;
        self.free_head = unsafe { (slot as *mut *mut u8).read() };
        self.used_slots += 1;
        slot
    }

    pub unsafe fn dealloc(&mut self, ptr: *mut u8) {
        unsafe {
            (ptr as *mut *mut u8).write(self.free_head);
        }
        self.free_head = ptr;
        self.used_slots -= 1;
    }

    pub fn is_full(&self) -> bool {
        self.used_slots == self.total_slots
    }

    pub fn is_empty(&self) -> bool {
        self.used_slots == 0
    }

    pub fn total_slots(&self) -> usize {
        self.total_slots
    }

    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    pub fn page_ptr(&self) -> *mut u8 {
        self.page_ptr
    }

    pub fn contains(&self, ptr: *mut u8) -> bool {
        let start = self.page_ptr as usize;
        let end = start + PAGE_SIZE;
        let addr = ptr as usize;
        addr >= start && addr < end
    }
}
