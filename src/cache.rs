use crate::slab::Slab;

const MAX_SLABS_PER_CACHE: usize = 32;

pub struct Cache {
    object_size: usize,
    slabs: [Option<Slab>; MAX_SLABS_PER_CACHE],
    slab_count: usize,
}

impl Cache {
    pub const fn new(object_size: usize) -> Self {
        const NONE_SLAB: Option<Slab> = None;
        Cache {
            object_size,
            slabs: [NONE_SLAB; MAX_SLABS_PER_CACHE],
            slab_count: 0,
        }
    }

    pub fn object_size(&self) -> usize {
        self.object_size
    }

    pub fn alloc(&mut self) -> *mut u8 {
        for slot in self.slabs.iter_mut() {
            if let Some(slab) = slot {
                if !slab.is_full() {
                    return slab.alloc();
                }
            }
        }
        core::ptr::null_mut()
    }

    pub unsafe fn add_slab_and_alloc(&mut self, page: *mut u8) -> *mut u8 {
        if self.slab_count >= MAX_SLABS_PER_CACHE {
            return core::ptr::null_mut();
        }

        let mut slab = unsafe { Slab::new(page, self.object_size) };
        let ptr = slab.alloc();

        for slot in self.slabs.iter_mut() {
            if slot.is_none() {
                *slot = Some(slab);
                self.slab_count += 1;
                return ptr;
            }
        }

        core::ptr::null_mut()
    }

    pub unsafe fn dealloc(&mut self, ptr: *mut u8) -> bool {
        for slot in self.slabs.iter_mut() {
            if let Some(slab) = slot {
                if slab.contains(ptr) {
                    unsafe { slab.dealloc(ptr) };
                    return true;
                }
            }
        }
        false
    }
}
