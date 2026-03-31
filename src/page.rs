pub const PAGE_SIZE: usize = 4096;

pub unsafe trait PageProvider {
    fn alloc_page(&self) -> *mut u8;

    unsafe fn dealloc_page(&self, ptr: *mut u8);
}

pub struct StaticPageProvider<const N: usize> {
    heap: *mut u8,
    offset: spin::Mutex<usize>,
    capacity: usize,
}

impl<const N: usize> StaticPageProvider<N> {
    pub unsafe fn new(heap_space: &mut [u8; N]) -> Self {
        let ptr = heap_space.as_mut_ptr();
        let align_offset = ptr.align_offset(PAGE_SIZE);
        Self {
            heap: unsafe { ptr.add(align_offset) },
            offset: spin::Mutex::new(0),
            capacity: N.saturating_sub(align_offset),
        }
    }
}

unsafe impl<const N: usize> PageProvider for StaticPageProvider<N> {
    fn alloc_page(&self) -> *mut u8 {
        let mut offset = self.offset.lock();
        if *offset + PAGE_SIZE > self.capacity {
            return core::ptr::null_mut();
        }

        let page = unsafe { self.heap.add(*offset) };
        *offset += PAGE_SIZE;
        page
    }

    unsafe fn dealloc_page(&self, _ptr: *mut u8) {}
}

unsafe impl<const N: usize> Send for StaticPageProvider<N> {}
unsafe impl<const N: usize> Sync for StaticPageProvider<N> {}
