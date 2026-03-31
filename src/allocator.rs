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
