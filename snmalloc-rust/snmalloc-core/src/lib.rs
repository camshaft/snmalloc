//! Core data structures and algorithms for the snmalloc Rust port.
//!
//! This crate contains everything described in
//! `docs/rust_port_architecture.md` §2–§14:
//!
//! * Compile-time configuration constants (`config`)
//! * Mitigation flags (`mitigations`)
//! * Pointer provenance wrapper (`ptrwrap`)
//! * Size-class table (`sizeclasses`)
//! * Free-list with integrity protection (`freelist`)
//! * Pagemap (`pagemap`)
//! * MPSC remote-deallocation queue (`mpsc`)
//! * Combining lock (`combininglock`)
//! * Red-black tree and ABA MPMC stack (`ds`)
//!
//! The crate is `no_std` and depends on the PAL and AAL only via traits.
//!
//! ## Concurrent code and loom
//!
//! Modules that use atomics import their atomic types through the `atomics`
//! re-export below.  When the crate is compiled with `--cfg loom` the re-export
//! switches to `loom::sync::atomic`, which intercepts every atomic operation
//! and performs systematic concurrency testing.  Run loom tests with:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p snmalloc-core
//! ```
//!
//! ## Unsafe code and miri
//!
//! All `unsafe` blocks must be accompanied by a `// SAFETY:` comment
//! explaining the invariants that make them sound.  The full test suite can be
//! run under the Miri interpreter with:
//!
//! ```text
//! cargo miri test -p snmalloc-core
//! ```
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

// ── Atomic type abstraction for loom ─────────────────────────────────────────
//
// All modules in this crate that use atomics should import from here rather
// than directly from `core::sync::atomic`.  When compiled under loom the
// imports are replaced with loom's intercepted equivalents, enabling systematic
// concurrency exploration.

/// Atomic primitives.  Resolves to `loom::sync::atomic` under `--cfg loom`
/// and to `core::sync::atomic` otherwise.
pub mod atomics {
    #[cfg(loom)]
    pub use loom::sync::atomic::{
        fence, AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicPtr, AtomicU32, AtomicU64,
        AtomicUsize, Ordering,
    };

    #[cfg(not(loom))]
    pub use core::sync::atomic::{
        fence, AtomicBool, AtomicI32, AtomicI64, AtomicIsize, AtomicPtr, AtomicU32, AtomicU64,
        AtomicUsize, Ordering,
    };
}

// ── Sub-modules (stubs) ───────────────────────────────────────────────────────

/// Compile-time configuration constants.
///
/// Corresponds to `ds/allocconfig.h`.  See
/// `docs/rust_port_architecture.md` §2.
pub mod config {
    /// Number of size classes per power-of-two band.
    pub const INTERMEDIATE_BITS: u32 = 2;

    /// Minimum allocation granularity in bytes (= 2 × pointer size on 64-bit).
    pub const MIN_ALLOC_STEP_SIZE: usize = 2 * core::mem::size_of::<usize>();

    /// Minimum allocation size honoured by the allocator.
    pub const MIN_ALLOC_SIZE: usize = MIN_ALLOC_STEP_SIZE;

    /// Base-2 logarithm of the minimum chunk size (= 16 KiB by default).
    pub const MIN_CHUNK_BITS: u32 = 14;

    /// Minimum chunk size in bytes.
    pub const MIN_CHUNK_SIZE: usize = 1 << MIN_CHUNK_BITS;

    /// Largest size class handled by the slab path (= 64 KiB by default).
    pub const MAX_SMALL_SIZECLASS_BITS: u32 = 16;

    /// Number of slots in the remote-deallocation cache hash table.
    pub const REMOTE_SLOT_BITS: u32 = 8;

    /// Derived: number of remote-cache slots.
    pub const REMOTE_SLOTS: usize = 1 << REMOTE_SLOT_BITS;

    /// Total deallocation byte budget before the remote cache flushes.
    pub const REMOTE_CACHE: usize = MIN_CHUNK_SIZE;

    /// Cache-line size in bytes.
    pub const CACHELINE_SIZE: usize = 64;
}

/// Compile-time mitigation flags.
///
/// Corresponds to `ds_core/mitigations.h`.  See
/// `docs/rust_port_architecture.md` §3.
pub mod mitigations {
    // TODO: Implement mitigation bitmask and `CHECK_CLIENT` preset.
}

/// Pointer provenance wrapper.
///
/// Corresponds to `ds_core/ptrwrap.h`.  See
/// `docs/rust_port_architecture.md` §6.
pub mod ptrwrap {
    // TODO: Implement `CapPtr<T, B>` with provenance bound type parameters.
}

/// Size-class table.
///
/// Corresponds to `mem/sizeclasstable.h`.  See
/// `docs/rust_port_architecture.md` §7.
pub mod sizeclasses {
    // TODO: Build the full SizeClassTable at compile time using const fn.
}

/// Free-list with integrity protection (forward/backward edge obfuscation).
///
/// Corresponds to `ds_core/seqset.h` and the freelist structures in
/// `mem/corealloc.h`.  See `docs/rust_port_architecture.md` §9.
pub mod freelist {
    // TODO: Implement Object, Builder, and Iter with XOR obfuscation.
}

/// Pagemap.
///
/// Corresponds to `backend_helpers/pagemap.h`.  See
/// `docs/rust_port_architecture.md` §12.
pub mod pagemap {
    // TODO: Implement flat/two-level pagemap keyed on chunk address.
}

/// MPSC remote-deallocation queue.
///
/// Corresponds to `mem/remoteallocator.h`.  See
/// `docs/rust_port_architecture.md` §10.
pub mod mpsc {
    // TODO: Implement the lock-free MPSC queue used for remote deallocation.
}

/// Combining lock.
///
/// Corresponds to `ds/combininglock.h`.
pub mod combininglock {
    // TODO: Implement the combining lock (flat combining pattern).
}

// ── Loom tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Loom model tests for the MPSC queue.
    ///
    /// These tests are only meaningful when run under loom:
    ///
    /// ```text
    /// RUSTFLAGS="--cfg loom" cargo test -p snmalloc-core mpsc
    /// ```
    #[cfg(loom)]
    mod mpsc_loom {
        // TODO: Add loom::model tests once the MPSC queue is implemented.
        //
        // Example skeleton:
        //
        // #[test]
        // fn two_producers_one_consumer() {
        //     loom::model(|| {
        //         let queue = Arc::new(MpscQueue::new());
        //         let q1 = queue.clone();
        //         let producer = loom::thread::spawn(move || q1.push(1));
        //         queue.push(2);
        //         producer.join().unwrap();
        //         // drain and assert both values received
        //     });
        // }
    }

    /// Loom model tests for the combining lock.
    #[cfg(loom)]
    mod combininglock_loom {
        // TODO: Add loom::model tests once the combining lock is implemented.
    }
}
