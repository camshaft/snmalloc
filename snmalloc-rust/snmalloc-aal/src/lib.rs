//! Architecture Abstraction Layer (AAL) for the snmalloc Rust port.
//!
//! Each CPU architecture provides a concrete type implementing the [`Aal`]
//! trait.  All architecture-specific operations (pointer bounds, prefetch,
//! cycle counter, address-width constants) pass through this layer.
//!
//! # Architecture
//!
//! The AAL trait mirrors the C++ AAL concept described in
//! `docs/rust_port_architecture.md` §4.  Architecture implementations live
//! as separate modules under this crate and are selected at compile time via
//! `cfg` attributes.
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

/// Feature flags that an AAL implementation may set.
pub struct AalFeatures {
    /// Pointers can be cast to/from integers without losing authority
    /// (conventional architectures).
    pub integer_pointers: bool,
    /// Pointers carry hardware-enforced bounds (CHERI).
    pub strict_provenance: bool,
}

/// The minimum interface that every CPU architecture must provide.
///
/// Implementors correspond to the C++ AAL types (e.g. `AALx86`, `AALAArch64`,
/// `AALCHERI`).  See `docs/rust_port_architecture.md` §4 for contracts.
pub trait Aal {
    /// Feature flags for this architecture.
    const FEATURES: AalFeatures;

    /// The number of significant bits in a virtual address on this platform
    /// (e.g. 48 on x86-64, 56 on ARMv8.5-A).
    ///
    /// Do not hard-code 48; use this constant wherever address-width matters.
    const ADDRESS_BITS: u32;

    /// Bound a pointer's hardware capability to `[ptr, ptr+size)`.
    ///
    /// On conventional architectures this is a no-op.  On CHERI it shrinks
    /// the capability.
    ///
    /// # Safety
    ///
    /// `ptr` must point to an allocation of at least `size` bytes.
    unsafe fn capptr_bound(ptr: *mut u8, size: usize) -> *mut u8;

    /// Issue a non-faulting cache-line prefetch for the given address.
    ///
    /// # Safety
    ///
    /// The address need not be mapped; a fault must not occur.
    unsafe fn prefetch(ptr: *const u8);

    /// Return a monotonically increasing CPU-cycle estimate.
    fn tick() -> u64;
}
