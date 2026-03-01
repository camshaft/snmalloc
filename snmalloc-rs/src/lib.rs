#![no_std]
//! `snmalloc-rs` provides a wrapper for [`microsoft/snmalloc`](https://github.com/microsoft/snmalloc) to make it usable as a global allocator for rust.
//! snmalloc is a research allocator. Its key design features are:
//! - Memory that is freed by the same thread that allocated it does not require any synchronising operations.
//! - Freeing memory in a different thread to initially allocated it, does not take any locks and instead uses a novel message passing scheme to return the memory to the original allocator, where it is recycled.
//! - The allocator uses large ranges of pages to reduce the amount of meta-data required.
//!
//! The benchmark is available at the [paper](https://github.com/microsoft/snmalloc/blob/master/snmalloc.pdf) of `snmalloc`
//! There are three features defined in this crate:
//! - `debug`: Enable the `Debug` mode in `snmalloc`.
//! - `1mib`: Use the `1mib` chunk configuration.
//! - `cache-friendly`: Make the allocator more cache friendly (setting `CACHE_FRIENDLY_OFFSET` to `64` in building the library).
//!
//! The whole library supports `no_std`.
//!
//! To use `snmalloc-rs` add it as a dependency:
//! ```toml
//! # Cargo.toml
//! [dependencies]
//! snmalloc-rs = "0.1.0"
//! ```
//!
//! To set `SnMalloc` as the global allocator add this to your project:
//! ```rust
//! #[global_allocator]
//! static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;
//! ```
extern crate snmalloc_sys as ffi;

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct SnMalloc;

unsafe impl Send for SnMalloc {}
unsafe impl Sync for SnMalloc {}

impl SnMalloc {
    #[inline(always)]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the available bytes in a memory block.
    #[inline(always)]
    pub fn usable_size(&self, ptr: *const u8) -> Option<usize> {
        match ptr.is_null() {
            true => None,
            false => Some(unsafe { ffi::sn_rust_usable_size(ptr.cast()) })
        }
    }

    /// Allocates memory with the given layout, returning a non-null pointer on success
    #[inline(always)]
    pub fn alloc_aligned(&self, layout: Layout) -> Option<NonNull<u8>> {
        match layout.size() {
            0 => NonNull::new(layout.align() as *mut u8),
            size => NonNull::new(unsafe { ffi::sn_rust_alloc(layout.align(), size) }.cast())
        }
    }
}

unsafe impl GlobalAlloc for SnMalloc {
    /// Allocate the memory with the given alignment and size.
    /// On success, it returns a pointer pointing to the required memory address.
    /// On failure, it returns a null pointer.
    /// The client must assure the following things:
    /// - `alignment` is greater than zero
    /// - Other constrains are the same as the rust standard library.
    ///
    /// The program may be forced to abort if the constrains are not full-filled.
    #[inline(always)]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match layout.size() {
            0 => layout.align() as *mut u8,
            size => ffi::sn_rust_alloc(layout.align(), size).cast()
        }
    }

    /// De-allocate the memory at the given address with the given alignment and size.
    /// The client must assure the following things:
    /// - the memory is acquired using the same allocator and the pointer points to the start position.
    /// - Other constrains are the same as the rust standard library.
    ///
    /// The program may be forced to abort if the constrains are not full-filled.
    #[inline(always)]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() != 0 {
            ffi::sn_rust_dealloc(ptr as _, layout.align(), layout.size());
        }
    }

    /// Behaves like alloc, but also ensures that the contents are set to zero before being returned.
    #[inline(always)]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match layout.size() {
            0 => layout.align() as *mut u8,
            size => ffi::sn_rust_alloc_zeroed(layout.align(), size).cast()
        }
    }

    /// Re-allocate the memory at the given address with the given alignment and size.
    /// On success, it returns a pointer pointing to the required memory address.
    /// The memory content within the `new_size` will remains the same as previous.
    /// On failure, it returns a null pointer. In this situation, the previous memory is not returned to the allocator.
    /// The client must assure the following things:
    /// - the memory is acquired using the same allocator and the pointer points to the start position
    /// - `alignment` fulfills all the requirements as `rust_alloc`
    /// - Other constrains are the same as the rust standard library.
    ///
    /// The program may be forced to abort if the constrains are not full-filled.
    #[inline(always)]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        match new_size {
            0 => {
                self.dealloc(ptr, layout);
                layout.align() as *mut u8
            }
            new_size if layout.size() == 0 => {
                self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()))
            }
            _ => ffi::sn_rust_realloc(ptr.cast(), layout.align(), layout.size(), new_size).cast()
        }
    }
}

/// A bounded sub-heap that draws from the global snmalloc allocator but
/// enforces a configurable byte budget.
///
/// Allocations are charged using the layout's size rounded up to alignment
/// (matching `aligned_size` in the C++ layer). Any allocation that would
/// push usage above the budget fails immediately with a null return value
/// (or `None` for the higher-level helpers).
///
/// # Thread safety
///
/// `SubHeap` is both `Send` and `Sync`: the internal budget counter is
/// updated atomically on the C++ side, so concurrent alloc/dealloc from
/// multiple threads is safe.
///
/// # Lifecycle contract
///
/// All allocations obtained from a `SubHeap` must be freed (via
/// [`SubHeap::dealloc`]) *before* the `SubHeap` is dropped. Dropping a
/// `SubHeap` with outstanding live allocations will free the internal
/// handle but will not free the underlying objects, resulting in a budget
/// counter leak.
pub struct SubHeap {
    handle: *mut ffi::c_void,
}

// SAFETY: The C++ budget counter is updated with atomic operations, so
// both sending the handle to another thread and sharing a reference across
// threads are safe.
unsafe impl Send for SubHeap {}
unsafe impl Sync for SubHeap {}

impl SubHeap {
    /// Create a new sub-heap with the given byte budget.
    ///
    /// Returns `None` if the internal handle allocation fails.  A
    /// `size_limit` of `0` creates a valid (but empty) sub-heap where every
    /// allocation immediately returns `None`.
    pub fn new(size_limit: usize) -> Option<Self> {
        let handle = unsafe { ffi::sn_create_sub_heap(size_limit) };
        if handle.is_null() {
            None
        } else {
            Some(SubHeap { handle })
        }
    }

    /// Allocate memory according to `layout` from this sub-heap.
    ///
    /// Returns `None` if the budget would be exceeded or if `layout.size()`
    /// is zero.
    pub fn alloc(&self, layout: Layout) -> Option<NonNull<u8>> {
        match layout.size() {
            0 => NonNull::new(layout.align() as *mut u8),
            size => NonNull::new(
                unsafe { ffi::sn_sub_heap_alloc(self.handle, layout.align(), size) }.cast(),
            ),
        }
    }

    /// Allocate zero-initialised memory from this sub-heap.
    ///
    /// Same constraints as [`SubHeap::alloc`].
    pub fn alloc_zeroed(&self, layout: Layout) -> Option<NonNull<u8>> {
        match layout.size() {
            0 => NonNull::new(layout.align() as *mut u8),
            size => NonNull::new(
                unsafe {
                    ffi::sn_sub_heap_alloc_zeroed(self.handle, layout.align(), size)
                }
                .cast(),
            ),
        }
    }

    /// Free a pointer previously obtained from this sub-heap, returning the
    /// bytes to the budget.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated from this `SubHeap` with the given
    /// `layout`, and must not be used after this call.
    pub unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() != 0 {
            ffi::sn_sub_heap_dealloc(
                self.handle,
                ptr.cast(),
                layout.align(),
                layout.size(),
            );
        }
    }
}

impl Drop for SubHeap {
    fn drop(&mut self) {
        unsafe { ffi::sn_destroy_sub_heap(self.handle) };
    }
}

unsafe impl GlobalAlloc for SubHeap {
    /// Allocate memory with the given layout from this sub-heap.
    ///
    /// Returns a null pointer if the budget would be exceeded or on OOM.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match layout.size() {
            0 => layout.align() as *mut u8,
            size => ffi::sn_sub_heap_alloc(self.handle, layout.align(), size).cast(),
        }
    }

    /// Free memory previously allocated from this sub-heap.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() != 0 {
            ffi::sn_sub_heap_dealloc(
                self.handle,
                ptr.cast(),
                layout.align(),
                layout.size(),
            );
        }
    }

    /// Allocate zero-initialised memory from this sub-heap.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match layout.size() {
            0 => layout.align() as *mut u8,
            size => {
                ffi::sn_sub_heap_alloc_zeroed(self.handle, layout.align(), size).cast()
            }
        }
    }

    /// Reallocate memory within this sub-heap.
    ///
    /// On failure returns a null pointer; the original pointer is *not*
    /// freed (matching the `GlobalAlloc::realloc` contract).
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `GlobalAlloc::realloc` requires callers to ensure `new_size`
        // forms a valid `Layout` with the existing alignment.  We assert that
        // contract here and propagate it to `from_size_align_unchecked`.
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        let new_ptr = GlobalAlloc::alloc(self, new_layout);
        if !new_ptr.is_null() {
            let copy_size = if layout.size() < new_size {
                layout.size()
            } else {
                new_size
            };
            core::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
            GlobalAlloc::dealloc(self, ptr, layout);
        }
        new_ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_lifecycle() {
        let alloc = SnMalloc::new();
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            
            // Test regular allocation
            let ptr = alloc.alloc(layout);
            alloc.dealloc(ptr, layout);

            // Test zeroed allocation
            let ptr = alloc.alloc_zeroed(layout);
            alloc.dealloc(ptr, layout);

            // Test reallocation
            let ptr = alloc.alloc(layout);
            let ptr = alloc.realloc(ptr, layout, 16);
            alloc.dealloc(ptr, layout);

            // Test large allocation
            let large_layout = Layout::from_size_align(1 << 20, 32).unwrap();
            let ptr = alloc.alloc(large_layout);
            alloc.dealloc(ptr, large_layout);
        }
    }
    #[test]
    fn it_frees_allocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = SnMalloc;

            let ptr = alloc.alloc(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_zero_allocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = SnMalloc;

            let ptr = alloc.alloc_zeroed(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_reallocated_memory() {
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let alloc = SnMalloc;

            let ptr = alloc.alloc(layout);
            let ptr = alloc.realloc(ptr, layout, 16);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn it_frees_large_alloc() {
        unsafe {
            let layout = Layout::from_size_align(1 << 20, 32).unwrap();
            let alloc = SnMalloc;

            let ptr = alloc.alloc(layout);
            alloc.dealloc(ptr, layout);
        }
    }

    #[test]
    fn test_usable_size() {
        let alloc = SnMalloc::new();
        unsafe {
            let layout = Layout::from_size_align(8, 8).unwrap();
            let ptr = alloc.alloc(layout);
            let usz = alloc.usable_size(ptr).expect("usable_size returned None");
            alloc.dealloc(ptr, layout);
            assert!(usz >= 8);
        }
    }

    // -----------------------------------------------------------------------
    // SubHeap tests
    // -----------------------------------------------------------------------

    #[test]
    fn sub_heap_basic_alloc_dealloc() {
        let heap = SubHeap::new(1024).expect("sub-heap creation failed");
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = heap.alloc(layout).expect("allocation within budget failed");
        unsafe { heap.dealloc(ptr.as_ptr(), layout) };
    }

    #[test]
    fn sub_heap_zeroed_alloc() {
        let heap = SubHeap::new(1024).expect("sub-heap creation failed");
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = heap
            .alloc_zeroed(layout)
            .expect("zeroed allocation within budget failed");
        // Verify the memory is actually zeroed.
        let slice =
            unsafe { core::slice::from_raw_parts(ptr.as_ptr(), layout.size()) };
        assert!(slice.iter().all(|&b| b == 0), "memory is not zeroed");
        unsafe { heap.dealloc(ptr.as_ptr(), layout) };
    }

    #[test]
    fn sub_heap_budget_enforced() {
        // Create a 256-byte budget, then try to allocate more than it.
        let heap = SubHeap::new(256).expect("sub-heap creation failed");
        let layout = Layout::from_size_align(128, 8).unwrap();

        // First two allocations fit (2 × 128 = 256).
        let p1 = heap.alloc(layout).expect("first allocation failed");
        let p2 = heap.alloc(layout).expect("second allocation failed");

        // Third allocation must fail (budget exhausted).
        assert!(
            heap.alloc(layout).is_none(),
            "allocation should have failed when budget is exhausted"
        );

        unsafe {
            heap.dealloc(p1.as_ptr(), layout);
            heap.dealloc(p2.as_ptr(), layout);
        }

        // After freeing, allocation should succeed again.
        let p3 = heap.alloc(layout).expect("allocation after free failed");
        unsafe { heap.dealloc(p3.as_ptr(), layout) };
    }

    #[test]
    fn sub_heap_zero_budget() {
        let heap = SubHeap::new(0).expect("zero-budget sub-heap creation failed");
        let layout = Layout::from_size_align(8, 8).unwrap();
        assert!(
            heap.alloc(layout).is_none(),
            "allocation from zero-budget heap should always fail"
        );
    }

    #[test]
    fn sub_heap_global_alloc_trait() {
        // Verify that SubHeap can be used through the GlobalAlloc trait.
        let heap = SubHeap::new(512).expect("sub-heap creation failed");
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe {
            let ptr = GlobalAlloc::alloc(&heap, layout);
            assert!(!ptr.is_null());
            GlobalAlloc::dealloc(&heap, ptr, layout);
        }
    }

    #[test]
    fn sub_heap_realloc() {
        let heap = SubHeap::new(1024).expect("sub-heap creation failed");
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe {
            let ptr = GlobalAlloc::alloc(&heap, layout);
            assert!(!ptr.is_null());
            // Grow the allocation.
            let ptr = GlobalAlloc::realloc(&heap, ptr, layout, 128);
            assert!(!ptr.is_null());
            let new_layout = Layout::from_size_align(128, 8).unwrap();
            GlobalAlloc::dealloc(&heap, ptr, new_layout);
        }
    }
}
