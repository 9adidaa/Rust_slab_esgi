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

    pub fn slab_count(&self) -> usize {
        self.slab_count
    }

    pub fn total_used(&self) -> usize {
        self.slabs
            .iter()
            .filter_map(|s| s.as_ref())
            .map(|s| s.used_slots())
            .sum()
    }

    pub fn total_free(&self) -> usize {
        self.slabs
            .iter()
            .filter_map(|s| s.as_ref())
            .map(|s| s.free_slots())
            .sum()
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

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::page::PAGE_SIZE;
    use alloc::vec;
    use alloc::vec::Vec;

    fn alloc_pages(count: usize) -> (Vec<u8>, Vec<*mut u8>) {
        let size = PAGE_SIZE * (count + 1);
        let mut buf = vec![0u8; size];
        let base = buf.as_mut_ptr();
        let align_offset = base.align_offset(PAGE_SIZE);
        let aligned = unsafe { base.add(align_offset) };

        let pages: Vec<*mut u8> = (0..count)
            .map(|i| unsafe { aligned.add(i * PAGE_SIZE) })
            .collect();

        (buf, pages)
    }

    #[test]
    fn test_new_cache_is_empty() {
        let cache = Cache::new(64);
        assert_eq!(cache.object_size(), 64);
        assert_eq!(cache.slab_count(), 0);
        assert_eq!(cache.total_used(), 0);
        assert_eq!(cache.total_free(), 0);
    }

    #[test]
    fn test_alloc_empty_cache_returns_null() {
        let mut cache = Cache::new(64);
        assert!(cache.alloc().is_null());
    }

    #[test]
    fn test_add_slab_and_alloc() {
        let (_buf, pages) = alloc_pages(1);
        let mut cache = Cache::new(64);

        let ptr = unsafe { cache.add_slab_and_alloc(pages[0]) };
        assert!(!ptr.is_null());
        assert_eq!(cache.slab_count(), 1);
        assert_eq!(cache.total_used(), 1);
    }

    #[test]
    fn test_alloc_across_multiple_slabs() {
        let (_buf, pages) = alloc_pages(2);
        let mut cache = Cache::new(2048);

        let p1 = unsafe { cache.add_slab_and_alloc(pages[0]) };
        assert!(!p1.is_null());
        let p2 = cache.alloc();
        assert!(!p2.is_null());

        assert!(cache.alloc().is_null());

        let p3 = unsafe { cache.add_slab_and_alloc(pages[1]) };
        assert!(!p3.is_null());
        assert_eq!(cache.slab_count(), 2);
    }

    #[test]
    fn test_dealloc_frees_slot() {
        let (_buf, pages) = alloc_pages(1);
        let mut cache = Cache::new(64);

        let ptr = unsafe { cache.add_slab_and_alloc(pages[0]) };
        assert_eq!(cache.total_used(), 1);

        let found = unsafe { cache.dealloc(ptr) };
        assert!(found);
        assert_eq!(cache.total_used(), 0);
    }

    #[test]
    fn test_dealloc_unknown_pointer_returns_false() {
        let (_buf, pages) = alloc_pages(1);
        let mut cache = Cache::new(64);
        let _ = unsafe { cache.add_slab_and_alloc(pages[0]) };

        let fake_ptr = 0xDEAD_BEEF as *mut u8;
        let found = unsafe { cache.dealloc(fake_ptr) };
        assert!(!found);
    }

    #[test]
    fn test_fill_and_reuse_after_dealloc() {
        let (_buf, pages) = alloc_pages(1);
        let mut cache = Cache::new(128);

        let first = unsafe { cache.add_slab_and_alloc(pages[0]) };
        let mut ptrs = vec![first];
        for _ in 1..32 {
            let p = cache.alloc();
            assert!(!p.is_null());
            ptrs.push(p);
        }
        assert!(cache.alloc().is_null());

        for p in &ptrs[0..10] {
            unsafe { cache.dealloc(*p) };
        }
        assert_eq!(cache.total_used(), 22);
        assert_eq!(cache.total_free(), 10);

        for _ in 0..10 {
            assert!(!cache.alloc().is_null());
        }
        assert_eq!(cache.total_used(), 32);
    }
}
