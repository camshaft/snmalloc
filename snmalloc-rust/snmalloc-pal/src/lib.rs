//! Platform Abstraction Layer (PAL) for the snmalloc Rust port.
//!
//! Each operating system provides a concrete type that implements the [`Pal`]
//! trait.  All OS interactions in snmalloc pass through this layer so that the
//! rest of the allocator is platform-independent.
//!
//! # Architecture
//!
//! The PAL trait mirrors the C++ PAL concept described in
//! `docs/rust_port_architecture.md` §5.  Platform implementations live as
//! separate modules under this crate and are selected at compile time via
//! `cfg` attributes.
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

/// The minimum interface that every platform must provide.
///
/// Implementors correspond to the C++ PAL types (e.g. `PALLinux`,
/// `PALWindows`, `PALOpenEnclave`).  See `docs/rust_port_architecture.md` §5
/// for a full description of each method's contract.
pub trait Pal {
    /// Emit a diagnostic message and terminate the process unconditionally.
    fn error(msg: &str) -> !;

    /// Advise the OS that the range `[ptr, ptr+size)` will not be accessed
    /// for a while (e.g. `MADV_FREE` on Linux, decommit on Windows).
    ///
    /// # Safety
    ///
    /// `ptr` must be page-aligned and `size` must be a non-zero multiple of
    /// the system page size.  The memory must previously have been obtained
    /// from [`Pal::reserve`].
    unsafe fn notify_not_using(ptr: *mut u8, size: usize);

    /// Bring the range `[ptr, ptr+size)` back into use.
    ///
    /// When `ZEROED` is `true` the returned memory is guaranteed to be
    /// zeroed, either by the OS or by this method.  Returns `true` if the OS
    /// provided zeroed pages natively (avoiding redundant zeroing by the
    /// caller).
    ///
    /// # Safety
    ///
    /// Same requirements as [`Pal::notify_not_using`].
    unsafe fn notify_using<const ZEROED: bool>(ptr: *mut u8, size: usize) -> bool;

    /// Reserve a contiguous range of virtual address space of at least `size`
    /// bytes, aligned to at least `align` bytes.
    ///
    /// Returns a pointer to the reserved region, or `None` on failure.
    ///
    /// # Safety
    ///
    /// `size` and `align` must both be non-zero powers of two.
    unsafe fn reserve(size: usize, align: usize) -> Option<*mut u8>;

    /// The base-2 logarithm of the system page size (e.g. 12 for 4 KiB
    /// pages).
    const PAGE_SIZE_BITS: u32;
}
