use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc};
use std::ffi::{c_int, c_void};
use std::hint::black_box;
use std::process;
use std::slice;

#[cfg(target_os = "linux")]
#[path = "../src/allocator.rs"]
mod allocator;

fn check(condition: bool, operation: &str, layout: Layout) {
    if !condition {
        eprintln!("allocator probe failed: {operation}, {layout:?}");
        process::exit(1);
    }
}

fn check_alignment(ptr: *mut u8, operation: &str, layout: Layout) {
    check(
        !ptr.is_null() && (black_box(ptr) as usize).is_multiple_of(layout.align()),
        operation,
        layout,
    );
}

#[unsafe(no_mangle)]
extern "C" fn FLBPluginRegister(_ctx: *mut c_void) -> c_int {
    for (size, align) in [(56, 16), (56, 8), (3, 64)] {
        let layout = Layout::from_size_align(size, align).unwrap();
        // Keep neighboring allocations live so reuse of one aligned address
        // cannot mask an under-aligned size class.
        let blocks: [_; 128] = std::array::from_fn(|_| unsafe {
            let ptr = alloc(layout);
            check_alignment(ptr, "alloc alignment", layout);
            let zeroed = alloc_zeroed(layout);
            check_alignment(zeroed, "alloc_zeroed alignment", layout);
            check(
                slice::from_raw_parts(zeroed, size).iter().all(|&b| b == 0),
                "alloc_zeroed contents",
                layout,
            );
            (ptr, zeroed)
        });
        for (ptr, zeroed) in blocks {
            unsafe {
                dealloc(ptr, layout);
                dealloc(zeroed, layout);
            }
        }

        let resized = Layout::from_size_align(size + 8, align).unwrap();
        let blocks: [_; 128] = std::array::from_fn(|_| unsafe {
            let ptr = alloc(resized);
            check_alignment(ptr, "alloc alignment", resized);
            ptr.write_bytes(42, resized.size());
            let ptr = realloc(ptr, resized, size);
            check_alignment(ptr, "realloc alignment", layout);
            check(
                slice::from_raw_parts(ptr, size).iter().all(|&b| b == 42),
                "realloc contents",
                layout,
            );
            ptr
        });
        for ptr in blocks {
            unsafe { dealloc(ptr, layout) };
        }
    }
    eprintln!("allocator probe passed");
    process::exit(0);
}
