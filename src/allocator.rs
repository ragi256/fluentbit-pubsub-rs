use std::alloc::{GlobalAlloc, Layout};
use std::ptr::{self, NonNull};

struct FluentBitAllocator;

#[global_allocator]
static ALLOCATOR: FluentBitAllocator = FluentBitAllocator;

// Fluent Bit's jemalloc can use an 8-byte quantum on both supported targets.
const MALLOC_ALIGN: usize = 8;

unsafe impl GlobalAlloc for FluentBitAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        if align <= MALLOC_ALIGN {
            return unsafe { libc::malloc(layout.size().max(align)) }.cast();
        }

        let mut ptr = ptr::null_mut();
        if unsafe { libc::posix_memalign(&mut ptr, align, layout.size()) } == 0 {
            ptr.cast()
        } else {
            ptr::null_mut()
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        if align <= MALLOC_ALIGN {
            return unsafe { libc::calloc(1, layout.size().max(align)) }.cast();
        }

        let ptr = unsafe { self.alloc(layout) };
        if let Some(ptr) = NonNull::new(ptr) {
            unsafe { ptr.as_ptr().write_bytes(0, layout.size()) };
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { libc::free(ptr.cast()) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let align = layout.align();
        if align <= MALLOC_ALIGN {
            return unsafe { libc::realloc(ptr.cast(), new_size.max(align)) }.cast();
        }

        // GlobalAlloc::realloc requires new_size to be valid for the original alignment.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, align) };
        let new_ptr = unsafe { self.alloc(new_layout) };
        if let Some(new_ptr) = NonNull::new(new_ptr) {
            unsafe {
                ptr.copy_to_nonoverlapping(new_ptr.as_ptr(), layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc};
    use std::{hint::black_box, slice};

    fn check_alignment(ptr: *mut u8, layout: Layout) {
        assert!(!ptr.is_null(), "allocation failed for {layout:?}");
        assert!(
            (black_box(ptr) as usize).is_multiple_of(layout.align()),
            "misaligned pointer {ptr:p} for {layout:?}"
        );
    }

    #[test]
    fn allocations_and_resizes_honor_layout() {
        for align in [1, 2, 4, 8, 16, 32, 64] {
            for size in 1..=128 {
                let layout = Layout::from_size_align(size, align).unwrap();
                // Cover growth and shrinkage, including sizes smaller than the alignment.
                let resized = Layout::from_size_align(129 - size, align).unwrap();
                unsafe {
                    let zeroed = alloc_zeroed(layout);
                    check_alignment(zeroed, layout);
                    assert!(
                        slice::from_raw_parts(zeroed, size)
                            .iter()
                            .all(|&byte| byte == 0),
                        "nonzero contents for {layout:?}"
                    );
                    dealloc(zeroed, layout);

                    let ptr = alloc(layout);
                    check_alignment(ptr, layout);
                    ptr.write_bytes(0xa5, size);
                    let ptr = realloc(ptr, layout, resized.size());
                    check_alignment(ptr, resized);
                    assert!(
                        slice::from_raw_parts(ptr, size.min(resized.size()))
                            .iter()
                            .all(|&byte| byte == 0xa5),
                        "contents changed resizing {layout:?} to {resized:?}"
                    );
                    dealloc(ptr, resized);
                }
            }
        }
    }
}
