/// page size we use everywhere (4k)
pub const PAGE_SIZE: usize = 4096;

/// # Safety
///
/// alloc_page has to return a PAGE_SIZE aligned pointer or null
/// dealloc_page should only be called on ptrs from alloc_page
/// dont dealloc twice
pub unsafe trait PageProvider {
    /// returns a page or null if we ran out
    fn alloc_page(&self) -> *mut u8;

    /// # Safety
    ///
    /// ptr has to come from alloc_page, and not already freed
    unsafe fn dealloc_page(&self, ptr: *mut u8);
}

/// Simple bump allocator for pages, uses a static buffer.
/// Doesnt actually free pages (bump style)
pub struct StaticPageProvider<const N: usize> {
    heap: *mut u8,
    offset: spin::Mutex<usize>,
    capacity: usize,
}

impl<const N: usize> StaticPageProvider<N> {
    /// # Safety
    ///
    /// heap_space must outlive the provider
    /// dont access heap_space from somewhere else while this is alive
    pub unsafe fn new(heap_space: &mut [u8; N]) -> Self {
        let ptr = heap_space.as_mut_ptr();
        let align_offset = ptr.align_offset(PAGE_SIZE);
        Self {
            // safety: aligning up within the buffer bounds
            heap: unsafe { ptr.add(align_offset) },
            offset: spin::Mutex::new(0),
            capacity: N.saturating_sub(align_offset),
        }
    }
}

// safety: pages returned are aligned and dont overlap
// dealloc is noop so nothing bad can happen there
unsafe impl<const N: usize> PageProvider for StaticPageProvider<N> {
    fn alloc_page(&self) -> *mut u8 {
        let mut offset = self.offset.lock();
        if *offset + PAGE_SIZE > self.capacity {
            return core::ptr::null_mut();
        }

        // safety: we checked offset + PAGE_SIZE <= capacity above
        let page = unsafe { self.heap.add(*offset) };
        *offset += PAGE_SIZE;
        page
    }

    unsafe fn dealloc_page(&self, _ptr: *mut u8) {}
}

// safety: spinlock protects the offset, heap ptr is read only after init
unsafe impl<const N: usize> Send for StaticPageProvider<N> {}
unsafe impl<const N: usize> Sync for StaticPageProvider<N> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pages_are_aligned() {
        let mut heap = [0u8; PAGE_SIZE * 4];
        let provider = unsafe { StaticPageProvider::new(&mut heap) };

        for _ in 0..3 {
            let page = provider.alloc_page();
            assert!(!page.is_null());
            assert_eq!((page as usize) % PAGE_SIZE, 0);
        }
    }

    #[test]
    fn test_pages_dont_overlap() {
        let mut heap = [0u8; PAGE_SIZE * 4];
        let provider = unsafe { StaticPageProvider::new(&mut heap) };

        let p1 = provider.alloc_page();
        let p2 = provider.alloc_page();
        assert!(!p1.is_null());
        assert!(!p2.is_null());

        let diff = (p2 as usize).abs_diff(p1 as usize);
        assert!(diff >= PAGE_SIZE);
    }

    #[test]
    fn test_exhaustion_returns_null() {
        let mut heap = [0u8; PAGE_SIZE * 2];
        let provider = unsafe { StaticPageProvider::new(&mut heap) };

        let p1 = provider.alloc_page();
        assert!(!p1.is_null());

        loop {
            let p = provider.alloc_page();
            if p.is_null() {
                break;
            }
        }
    }

    #[test]
    fn test_page_is_writable() {
        let mut heap = [0u8; PAGE_SIZE * 2];
        let provider = unsafe { StaticPageProvider::new(&mut heap) };

        let page = provider.alloc_page();
        assert!(!page.is_null());

        unsafe {
            core::ptr::write_bytes(page, 0xAB, PAGE_SIZE);
        }
        for i in 0..PAGE_SIZE {
            assert_eq!(unsafe { *page.add(i) }, 0xAB);
        }
    }
}
