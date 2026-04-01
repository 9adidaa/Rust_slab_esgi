use crate::cache::Cache;
use crate::page::PageProvider;
use core::alloc::{GlobalAlloc, Layout};
use spin::Mutex;

const NUM_SIZE_CLASSES: usize = 9;

const SIZE_CLASSES: [usize; NUM_SIZE_CLASSES] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];

fn size_class_index(size: usize) -> Option<usize> {
    SIZE_CLASSES.iter().position(|&s| s >= size)
}

pub struct SlabAllocator<P: PageProvider> {
    inner: Mutex<SlabAllocatorInner>,
    page_provider: P,
}

struct SlabAllocatorInner {
    caches: [Cache; NUM_SIZE_CLASSES],
}

impl<P: PageProvider> SlabAllocator<P> {
    pub fn new(page_provider: P) -> Self {
        let caches = [
            Cache::new(SIZE_CLASSES[0]),
            Cache::new(SIZE_CLASSES[1]),
            Cache::new(SIZE_CLASSES[2]),
            Cache::new(SIZE_CLASSES[3]),
            Cache::new(SIZE_CLASSES[4]),
            Cache::new(SIZE_CLASSES[5]),
            Cache::new(SIZE_CLASSES[6]),
            Cache::new(SIZE_CLASSES[7]),
            Cache::new(SIZE_CLASSES[8]),
        ];

        SlabAllocator {
            inner: Mutex::new(SlabAllocatorInner { caches }),
            page_provider,
        }
    }

    pub fn size_class_for(layout: Layout) -> Option<usize> {
        let size = layout.size().max(layout.align());
        SIZE_CLASSES.iter().find(|&&s| s >= size).copied()
    }
}

unsafe impl<P: PageProvider + Sync> GlobalAlloc for SlabAllocator<P> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(layout.align());

        let index = match size_class_index(size) {
            Some(i) => i,
            None => return core::ptr::null_mut(),
        };

        let mut inner = self.inner.lock();
        let cache = &mut inner.caches[index];

        let ptr = cache.alloc();
        if !ptr.is_null() {
            return ptr;
        }

        let page = self.page_provider.alloc_page();
        if page.is_null() {
            return core::ptr::null_mut();
        }

        unsafe { cache.add_slab_and_alloc(page) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(layout.align());

        let index = match size_class_index(size) {
            Some(i) => i,
            None => return,
        };

        let mut inner = self.inner.lock();
        let cache = &mut inner.caches[index];

        unsafe {
            cache.dealloc(ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::StaticPageProvider;

    unsafe fn make_allocator(
        heap: &mut [u8; 65536],
    ) -> SlabAllocator<StaticPageProvider<65536>> {
        let provider = unsafe { StaticPageProvider::new(heap) };
        SlabAllocator::new(provider)
    }

    #[test]
    fn test_size_class_roundup() {
        let l1 = Layout::from_size_align(1, 1).unwrap();
        assert_eq!(SlabAllocator::<StaticPageProvider<0>>::size_class_for(l1), Some(8));

        let l2 = Layout::from_size_align(33, 1).unwrap();
        assert_eq!(SlabAllocator::<StaticPageProvider<0>>::size_class_for(l2), Some(64));

        let l3 = Layout::from_size_align(2048, 1).unwrap();
        assert_eq!(SlabAllocator::<StaticPageProvider<0>>::size_class_for(l3), Some(2048));

        let l4 = Layout::from_size_align(4096, 1).unwrap();
        assert_eq!(SlabAllocator::<StaticPageProvider<0>>::size_class_for(l4), None);
    }

    #[test]
    fn test_single_alloc_dealloc() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null());

        unsafe { ptr.write(42) };
        assert_eq!(unsafe { ptr.read() }, 42);

        unsafe { allocator.dealloc(ptr, layout) };
    }

    #[test]
    fn test_alignment_respected() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };

        let layout = Layout::from_size_align(16, 16).unwrap();
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!((ptr as usize) % 16, 0);

        unsafe { allocator.dealloc(ptr, layout) };
    }

    #[test]
    fn test_mixed_size_allocations() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };

        let layouts = [
            Layout::from_size_align(8, 8).unwrap(),
            Layout::from_size_align(64, 8).unwrap(),
            Layout::from_size_align(256, 8).unwrap(),
            Layout::from_size_align(1024, 8).unwrap(),
        ];

        let mut ptrs = Vec::new();
        for layout in &layouts {
            let ptr = unsafe { allocator.alloc(*layout) };
            assert!(!ptr.is_null());
            ptrs.push((ptr, *layout));
        }

        for i in 0..ptrs.len() {
            for j in (i + 1)..ptrs.len() {
                assert_ne!(ptrs[i].0, ptrs[j].0);
            }
        }

        for (ptr, layout) in ptrs {
            unsafe { allocator.dealloc(ptr, layout) };
        }
    }

    #[test]
    fn test_too_large_returns_null() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };

        let layout = Layout::from_size_align(4096, 8).unwrap();
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(ptr.is_null());
    }

    #[test]
    fn test_stress_alloc_dealloc() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };
        let layout = Layout::from_size_align(32, 8).unwrap();

        for _ in 0..1000 {
            let ptr = unsafe { allocator.alloc(layout) };
            assert!(!ptr.is_null());
            unsafe { core::ptr::write_bytes(ptr, 0xCC, 32) };
            unsafe { allocator.dealloc(ptr, layout) };
        }
    }

    #[test]
    fn test_write_integrity_across_sizes() {
        let mut heap = [0u8; 65536];
        let allocator = unsafe { make_allocator(&mut heap) };

        let sizes = [8, 16, 32, 64, 128, 256, 512, 1024];
        let mut ptrs = Vec::new();

        for (i, &size) in sizes.iter().enumerate() {
            let layout = Layout::from_size_align(size, 8).unwrap();
            let ptr = unsafe { allocator.alloc(layout) };
            assert!(!ptr.is_null());
            let pattern = (i + 1) as u8;
            unsafe { core::ptr::write_bytes(ptr, pattern, size) };
            ptrs.push((ptr, layout, pattern, size));
        }

        for &(ptr, _layout, pattern, size) in &ptrs {
            for offset in 0..size {
                let byte = unsafe { *ptr.add(offset) };
                assert_eq!(byte, pattern);
            }
        }

        for (ptr, layout, _, _) in ptrs {
            unsafe { allocator.dealloc(ptr, layout) };
        }
    }
}
