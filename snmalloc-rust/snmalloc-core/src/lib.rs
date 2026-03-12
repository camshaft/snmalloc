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
    use crate::mitigations;

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

    /// Minimum number of objects that a slab must hold.
    ///
    /// Raised to 13 when the `random_larger_thresholds` mitigation is active,
    /// matching `ds/allocconfig.h`.
    pub const MIN_OBJECT_COUNT: usize =
        if mitigations::MITIGATIONS.contains(mitigations::RANDOM_LARGER_THRESHOLDS) {
            13
        } else {
            4
        };

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
///
/// The active set of mitigations is stored in [`MITIGATIONS`].  All code that
/// conditionally changes behaviour based on a mitigation should check it with
/// [`MITIGATIONS`]`.contains(FLAG)`, which const-folds to a compile-time bool
/// and allows the optimizer to eliminate dead branches.
///
/// Platform-specific mitigation sets (CHERI, NetBSD, OpenEnclave) are not yet
/// implemented; they will be added when those PAL implementations land.
pub mod mitigations {
    /// A bitmask of active security mitigations.
    ///
    /// Mirrors `mitigation::type` from `ds_core/mitigations.h`.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Mitigations(usize);

    impl Mitigations {
        /// Construct a `Mitigations` value from a raw bitmask.
        pub const fn new(mask: usize) -> Self {
            Self(mask)
        }

        /// Return `true` if every bit set in `other` is also set in `self`.
        ///
        /// In practice `other` is always a single-bit mitigation constant, so
        /// this is equivalent to the C++ `operator()` on `mitigation::type`
        /// (`(mask & a.mask) != 0`).  Using a full-subset check is more
        /// correct for multi-bit queries and has the same result for single-bit
        /// checks.
        pub const fn contains(self, other: Self) -> bool {
            (self.0 & other.0) == other.0
        }

        /// Return the union of two mitigation sets (C++ `operator+`).
        pub const fn add(self, other: Self) -> Self {
            Self(self.0 | other.0)
        }

        /// Return `self` with all bits from `other` cleared (C++ `operator-`).
        pub const fn remove(self, other: Self) -> Self {
            Self(self.0 & !other.0)
        }
    }

    // ── Individual mitigation flags ───────────────────────────────────────────
    // Bit positions match `ds_core/mitigations.h` exactly.

    /// Randomise the pagemap's position in its OS allocation.
    pub const RANDOM_PAGEMAP: Mitigations = Mitigations(1 << 0);
    /// Require more objects per slab and a higher free fraction before waking.
    pub const RANDOM_LARGER_THRESHOLDS: Mitigations = Mitigations(1 << 1);
    /// XOR-obfuscate forward pointers in intra-slab free lists.
    ///
    /// This constant is defined on all platforms.  On CHERI (`__CHERI_PURE_CAPABILITY__`)
    /// it is removed from `FULL_CHECKS` because XOR-ing a pointer destroys its
    /// hardware tag, making the obfuscated value unusable.  That adjustment
    /// will be applied when the CHERI PAL/AAL implementation lands.
    pub const FREELIST_FORWARD_EDGE: Mitigations = Mitigations(1 << 2);
    /// Store obfuscated backward-edge signatures in every free object.
    pub const FREELIST_BACKWARD_EDGE: Mitigations = Mitigations(1 << 3);
    /// Walk and validate the free list when de-purposing a slab.
    pub const FREELIST_TEARDOWN_VALIDATE: Mitigations = Mitigations(1 << 4);
    /// Permute the initial per-slab free list with Sattolo's algorithm.
    pub const RANDOM_INITIAL: Mitigations = Mitigations(1 << 5);
    /// Dual-queue randomised free-list during operation (coin-flip assignment).
    pub const RANDOM_PRESERVE: Mitigations = Mitigations(1 << 6);
    /// Randomly open a new slab instead of reusing the last available one.
    pub const RANDOM_EXTRA_SLAB: Mitigations = Mitigations(1 << 7);
    /// Reuse slabs in LIFO order to increase time between address reuse.
    pub const REUSE_LIFO: Mitigations = Mitigations(1 << 8);
    /// Basic well-formedness checks on pointers passed to free.
    pub const SANITY_CHECKS: Mitigations = Mitigations(1 << 9);
    /// CHERI-specific capability well-formedness checks on freed pointers.
    /// Only meaningful on `__CHERI_PURE_CAPABILITY__` targets.
    pub const CHERI_CHECKS: Mitigations = Mitigations(1 << 10);
    /// Erase intra-slab free-list metadata before completing an allocation.
    pub const CLEAR_META: Mitigations = Mitigations(1 << 11);
    /// Guard pagemap metadata with OS-level page-protection and separate range.
    pub const METADATA_PROTECTION: Mitigations = Mitigations(1 << 12);
    /// Ask the PAL to enforce the using/not-using memory-access model.
    pub const PAL_ENFORCE_ACCESS: Mitigations = Mitigations(1 << 13);

    // ── Aggregate constants ───────────────────────────────────────────────────

    /// No mitigations.
    pub const NO_CHECKS: Mitigations = Mitigations(0);

    /// All mitigations except `CHERI_CHECKS`.
    ///
    /// `CHERI_CHECKS` is deliberately excluded here (as in C++) and is only
    /// added on `__CHERI_PURE_CAPABILITY__` targets.  On CHERI, `FULL_CHECKS`
    /// also removes `FREELIST_FORWARD_EDGE` and `PAL_ENFORCE_ACCESS` — those
    /// adjustments will be applied when the CHERI PAL implementation lands.
    pub const FULL_CHECKS: Mitigations = NO_CHECKS
        .add(RANDOM_PAGEMAP)
        .add(RANDOM_LARGER_THRESHOLDS)
        .add(FREELIST_FORWARD_EDGE)
        .add(FREELIST_BACKWARD_EDGE)
        .add(FREELIST_TEARDOWN_VALIDATE)
        .add(RANDOM_INITIAL)
        .add(RANDOM_PRESERVE)
        .add(RANDOM_EXTRA_SLAB)
        .add(REUSE_LIFO)
        .add(SANITY_CHECKS)
        .add(CLEAR_META)
        .add(METADATA_PROTECTION)
        .add(PAL_ENFORCE_ACCESS);

    /// The compile-time active mitigation set for this build.
    ///
    /// Equals `FULL_CHECKS` when the `check_client` cargo feature is enabled,
    /// `NO_CHECKS` otherwise.  Matches the C++ `snmalloc::mitigations` global.
    ///
    /// Platform-specific adjustments (NetBSD removes `PAL_ENFORCE_ACCESS`,
    /// OpenEnclave removes `METADATA_PROTECTION` and `RANDOM_PAGEMAP`, CHERI
    /// removes `FREELIST_FORWARD_EDGE` and `PAL_ENFORCE_ACCESS` and adds
    /// `CHERI_CHECKS`) are not yet wired up; they will be implemented alongside
    /// those PAL/AAL crates.
    #[cfg(feature = "check_client")]
    pub const MITIGATIONS: Mitigations = FULL_CHECKS;

    #[cfg(not(feature = "check_client"))]
    pub const MITIGATIONS: Mitigations = NO_CHECKS;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn contains_works() {
            assert!(FULL_CHECKS.contains(RANDOM_PAGEMAP));
            assert!(FULL_CHECKS.contains(FREELIST_FORWARD_EDGE));
            assert!(!FULL_CHECKS.contains(CHERI_CHECKS));
        }

        #[test]
        fn add_and_remove() {
            let m = NO_CHECKS.add(SANITY_CHECKS).add(CLEAR_META);
            assert!(m.contains(SANITY_CHECKS));
            assert!(m.contains(CLEAR_META));
            assert!(!m.contains(RANDOM_PAGEMAP));

            let m2 = m.remove(SANITY_CHECKS);
            assert!(!m2.contains(SANITY_CHECKS));
            assert!(m2.contains(CLEAR_META));
        }

        #[test]
        fn no_checks_is_zero() {
            assert_eq!(NO_CHECKS, Mitigations(0));
        }

        #[test]
        fn cheri_checks_not_in_full_checks() {
            assert!(!FULL_CHECKS.contains(CHERI_CHECKS));
        }

        #[test]
        fn full_checks_has_all_non_cheri() {
            for (flag, name) in [
                (RANDOM_PAGEMAP, "RANDOM_PAGEMAP"),
                (RANDOM_LARGER_THRESHOLDS, "RANDOM_LARGER_THRESHOLDS"),
                (FREELIST_FORWARD_EDGE, "FREELIST_FORWARD_EDGE"),
                (FREELIST_BACKWARD_EDGE, "FREELIST_BACKWARD_EDGE"),
                (FREELIST_TEARDOWN_VALIDATE, "FREELIST_TEARDOWN_VALIDATE"),
                (RANDOM_INITIAL, "RANDOM_INITIAL"),
                (RANDOM_PRESERVE, "RANDOM_PRESERVE"),
                (RANDOM_EXTRA_SLAB, "RANDOM_EXTRA_SLAB"),
                (REUSE_LIFO, "REUSE_LIFO"),
                (SANITY_CHECKS, "SANITY_CHECKS"),
                (CLEAR_META, "CLEAR_META"),
                (METADATA_PROTECTION, "METADATA_PROTECTION"),
                (PAL_ENFORCE_ACCESS, "PAL_ENFORCE_ACCESS"),
            ] {
                assert!(
                    FULL_CHECKS.contains(flag),
                    "FULL_CHECKS is missing {name}"
                );
            }
        }
    }
}

/// Pointer provenance wrapper.
///
/// Corresponds to `ds_core/ptrwrap.h`.  See
/// `docs/rust_port_architecture.md` §6.
///
/// Every pointer in snmalloc is annotated with a [`Bound`] phantom type
/// parameter that describes the address range the pointer is authorised to
/// access.  On conventional architectures the annotation is erased at
/// compile time.  On CHERI it maps to hardware capability metadata.
///
/// # Bound types
///
/// Use the constants in [`bounds`] as the `B` type parameter:
///
/// | Type | Authority |
/// |------|-----------|
/// | [`bounds::Arena`] | Entire OS-reserved arena (backend only) |
/// | [`bounds::Chunk`] | Single chunk, full address-space control |
/// | [`bounds::ChunkUser`] | Single chunk, user address-space control |
/// | [`bounds::AllocFull`] | One allocation object, full a-s control |
/// | [`bounds::Alloc`] | One allocation object, user a-s control |
/// | [`bounds::AllocWild`] | Unverified client-supplied pointer |
///
/// # Safety model
///
/// `CapPtr<T, B>` is a **transparent** wrapper around a raw `*mut T`.  All
/// dereferences remain `unsafe`; the type parameter only provides compile-time
/// documentation of the intended authority.  Users of this type must ensure
/// that the wrapped pointer actually carries the authority its `B` parameter
/// claims.
pub mod ptrwrap {
    use core::marker::PhantomData;

    use crate::atomics::{AtomicPtr, Ordering};

    // ── Sealed-trait plumbing ─────────────────────────────────────────────────
    //
    // The `Bound` trait is sealed so that only the types defined in this module
    // can implement it, mirroring the closed set of `capptr::bound` structs in
    // the C++ `IsBound` concept.

    mod private {
        /// Prevents external crates from implementing [`super::Bound`].
        pub trait Sealed {}
    }

    // ── Dimension enumerations ────────────────────────────────────────────────

    /// Spatial extent authorised by the pointer.
    ///
    /// Sorted so that lower values represent narrower authority (`Alloc` <
    /// `Chunk` < `Arena`), matching the C++ dimension ordering.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Spatial {
        /// Bounded to a particular allocation object.
        Alloc = 0,
        /// Bounded to one or more chunk granules.
        Chunk = 1,
        /// Unbounded (entire OS arena).
        Arena = 2,
    }

    /// Address-space control dimension.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum AddressSpaceControl {
        /// All address-space control stripped (suitable for user code).
        User,
        /// Full address-space control retained (internal backend use).
        Full,
    }

    /// Wildness dimension.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Wildness {
        /// Pointer came from client code and has not been validated.
        Wild,
        /// Pointer is from the kernel or has been domesticated.
        Tame,
    }

    // ── Bound marker trait ────────────────────────────────────────────────────

    /// Marker trait that identifies a valid combination of pointer-authority
    /// dimensions.
    ///
    /// Implemented only by the types in [`bounds`].  External crates cannot add
    /// new implementations.
    pub trait Bound: private::Sealed + Copy + 'static {
        const SPATIAL: Spatial;
        const ADDRESS_SPACE_CONTROL: AddressSpaceControl;
        const WILDNESS: Wildness;
    }

    // ── Concrete bound types ──────────────────────────────────────────────────

    /// Named bound types matching the C++ `capptr::bounds::*` aliases.
    pub mod bounds {
        use super::{AddressSpaceControl, Bound, Spatial, Wildness};
        use super::private::Sealed;

        /// Authority over the entire OS-reserved arena (`Arena × Full × Tame`).
        #[derive(Clone, Copy)] pub struct Arena;
        impl Sealed for Arena {}
        impl Bound for Arena {
            const SPATIAL: Spatial = Spatial::Arena;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::Full;
            const WILDNESS: Wildness = Wildness::Tame;
        }

        /// Authority over a single chunk, full a-s control (`Chunk × Full × Tame`).
        #[derive(Clone, Copy)] pub struct Chunk;
        impl Sealed for Chunk {}
        impl Bound for Chunk {
            const SPATIAL: Spatial = Spatial::Chunk;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::Full;
            const WILDNESS: Wildness = Wildness::Tame;
        }

        /// Authority over a chunk, user a-s control (`Chunk × User × Tame`).
        ///
        /// Used as an ephemeral intermediate when returning a large allocation.
        #[derive(Clone, Copy)] pub struct ChunkUser;
        impl Sealed for ChunkUser {}
        impl Bound for ChunkUser {
            const SPATIAL: Spatial = Spatial::Chunk;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::User;
            const WILDNESS: Wildness = Wildness::Tame;
        }

        /// Authority over one allocation, full a-s control (`Alloc × Full × Tame`).
        #[derive(Clone, Copy)] pub struct AllocFull;
        impl Sealed for AllocFull {}
        impl Bound for AllocFull {
            const SPATIAL: Spatial = Spatial::Alloc;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::Full;
            const WILDNESS: Wildness = Wildness::Tame;
        }

        /// Authority over one allocation, user a-s control (`Alloc × User × Tame`).
        ///
        /// The bound that the frontend handles after narrowing a chunk pointer.
        #[derive(Clone, Copy)] pub struct Alloc;
        impl Sealed for Alloc {}
        impl Bound for Alloc {
            const SPATIAL: Spatial = Spatial::Alloc;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::User;
            const WILDNESS: Wildness = Wildness::Tame;
        }

        /// An unverified client-supplied pointer (`Alloc × User × Wild`).
        ///
        /// Must be domesticated (validated via pagemap lookup) before use.
        /// Cannot be dereferenced through [`CapPtr`](super::CapPtr).
        #[derive(Clone, Copy)] pub struct AllocWild;
        impl Sealed for AllocWild {}
        impl Bound for AllocWild {
            const SPATIAL: Spatial = Spatial::Alloc;
            const ADDRESS_SPACE_CONTROL: AddressSpaceControl = AddressSpaceControl::User;
            const WILDNESS: Wildness = Wildness::Wild;
        }
    }

    // ── CapPtr ────────────────────────────────────────────────────────────────

    /// A raw pointer annotated with a compile-time bounds description `B`.
    ///
    /// Mirrors `CapPtr<T, bounds>` from `ds_core/ptrwrap.h`.
    ///
    /// # Safety
    ///
    /// The caller is responsible for ensuring that the wrapped pointer actually
    /// carries the authority its `B` parameter claims.  The type parameter is
    /// not enforced at run time on conventional (non-CHERI) architectures.
    ///
    /// Wild-bounded pointers (`B = `[`bounds::AllocWild`]`) cannot be
    /// dereferenced via `operator->` — no `Deref` impl is provided for that
    /// bound.
    pub struct CapPtr<T, B: Bound> {
        ptr: *mut T,
        _bounds: PhantomData<B>,
    }

    // SAFETY: CapPtr is a transparent wrapper around a raw pointer; the
    // underlying aliasing/Send/Sync requirements are the same as for *mut T.
    unsafe impl<T: Send, B: Bound> Send for CapPtr<T, B> {}
    unsafe impl<T: Sync, B: Bound> Sync for CapPtr<T, B> {}

    impl<T, B: Bound> Clone for CapPtr<T, B> {
        fn clone(&self) -> Self { *self }
    }
    impl<T, B: Bound> Copy for CapPtr<T, B> {}

    impl<T, B: Bound> core::fmt::Debug for CapPtr<T, B> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "CapPtr({:p})", self.ptr)
        }
    }

    impl<T, B: Bound> Default for CapPtr<T, B> {
        fn default() -> Self {
            Self::null()
        }
    }

    impl<T, B: Bound> PartialEq for CapPtr<T, B> {
        fn eq(&self, other: &Self) -> bool {
            self.ptr == other.ptr
        }
    }
    impl<T, B: Bound> Eq for CapPtr<T, B> {}

    impl<T, B: Bound> PartialOrd for CapPtr<T, B> {
        fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl<T, B: Bound> Ord for CapPtr<T, B> {
        fn cmp(&self, other: &Self) -> core::cmp::Ordering {
            self.ptr.cmp(&other.ptr)
        }
    }

    impl<T, B: Bound> CapPtr<T, B> {
        /// Construct a null `CapPtr` at any bounds level.
        ///
        /// Mirrors the C++ `CapPtr(nullptr)` constructor.
        pub const fn null() -> Self {
            Self {
                ptr: core::ptr::null_mut(),
                _bounds: PhantomData,
            }
        }

        /// Wrap a raw pointer.  The caller must ensure the pointer actually
        /// carries the authority that `B` describes.
        ///
        /// Mirrors `CapPtr::unsafe_from(T* p)` in `ds_core/ptrwrap.h`.
        ///
        /// # Safety
        ///
        /// The pointer must be valid for the declared bounds.
        pub const unsafe fn unsafe_from(ptr: *mut T) -> Self {
            Self { ptr, _bounds: PhantomData }
        }

        /// Unwrap to a raw pointer without any safety checks.
        ///
        /// Mirrors `unsafe_ptr()` in `ds_core/ptrwrap.h`.
        pub fn unsafe_ptr(self) -> *mut T {
            self.ptr
        }

        /// Reinterpret the pointer as a `usize` integer.
        ///
        /// Mirrors `unsafe_uintptr()` in `ds_core/ptrwrap.h`.
        pub fn unsafe_uintptr(self) -> usize {
            self.ptr as usize
        }

        /// Return `true` if the pointer is null.
        pub fn is_null(self) -> bool {
            self.ptr.is_null()
        }

        /// Cast to `*mut ()` (void pointer), preserving bounds.
        ///
        /// Mirrors `as_void()` in `ds_core/ptrwrap.h`.
        pub fn as_void(self) -> CapPtr<(), B> {
            CapPtr {
                ptr: self.ptr as *mut (),
                _bounds: PhantomData,
            }
        }

        /// Cast to a different pointee type using `as *mut U`, preserving bounds.
        ///
        /// Mirrors `as_static<U>()` (via `static_cast`) in `ds_core/ptrwrap.h`.
        /// Unlike C++ `static_cast<>`, this is a raw `as *mut U` pointer cast and
        /// does **not** validate type-hierarchy compatibility at compile time.
        /// The caller must ensure the pointed-to types are compatible.
        ///
        /// # Safety
        ///
        /// `T` and `U` must have a compatible memory layout for the intended use.
        pub unsafe fn as_static<U>(self) -> CapPtr<U, B> {
            CapPtr {
                ptr: self.ptr as *mut U,
                _bounds: PhantomData,
            }
        }

        /// Cast to a different pointee type using a raw `as *mut U` cast.
        ///
        /// Mirrors `as_reinterpret<U>()` (via `reinterpret_cast`) in
        /// `ds_core/ptrwrap.h`.
        ///
        /// # Safety
        ///
        /// Same requirements as [`as_static`](Self::as_static).
        pub unsafe fn as_reinterpret<U>(self) -> CapPtr<U, B> {
            CapPtr {
                ptr: self.ptr as *mut U,
                _bounds: PhantomData,
            }
        }

        /// Narrow the bounds annotation from a wider bound to this (narrower) one.
        ///
        /// Used to convert a chunk-level pointer to an alloc-level pointer (for
        /// example) while preserving the actual address.  No run-time check is
        /// performed on conventional architectures.
        ///
        /// Mirrors `CapPtr::unsafe_from(other.unsafe_ptr())` used in the C++
        /// source when re-annotating a pointer with a stricter bound.
        ///
        /// # Safety
        ///
        /// The caller must ensure the pointer genuinely has the authority that
        /// `B` describes.
        pub unsafe fn with_bounds<NewB: Bound>(self) -> CapPtr<T, NewB> {
            CapPtr {
                ptr: self.ptr,
                _bounds: PhantomData,
            }
        }
    }

    // ── Free functions ────────────────────────────────────────────────────────

    /// Extract a raw `*mut ()` from a domesticated allocation pointer.
    ///
    /// This is the safe operation that "reveals" the pointer value to the
    /// caller.  It is the only place where a `CapPtr` is unwrapped to a raw
    /// pointer for return to client code.  Dual to [`capptr_from_client`].
    ///
    /// Mirrors `capptr_reveal()` in `ds_core/ptrwrap.h`.
    pub fn capptr_reveal(p: CapPtr<(), bounds::Alloc>) -> *mut () {
        p.unsafe_ptr()
    }

    /// Wrap a raw `*mut ()` from the client in an [`bounds::AllocWild`] bound.
    ///
    /// This is the entry point for all client-supplied pointers.  The pointer
    /// must be domesticated (validated via pagemap lookup) before use.
    /// Dual to [`capptr_reveal`].
    ///
    /// Mirrors `capptr_from_client()` in `ds_core/ptrwrap.h`.
    pub fn capptr_from_client(p: *mut ()) -> CapPtr<(), bounds::AllocWild> {
        // SAFETY: Wrapping the pointer with Wild bounds is always safe; the
        // AllocWild annotation documents that it is unvalidated.
        unsafe { CapPtr::unsafe_from(p) }
    }

    /// Mark an [`bounds::Alloc`] pointer as wild.
    ///
    /// Since `AllocWild` is the only valid wild bound (C++ `static_assert`:
    /// wild pointers must have `Alloc × User` spatial/a-s dimensions), this
    /// function has a concrete rather than generic signature.
    ///
    /// Mirrors `capptr_rewild()` in `ds_core/ptrwrap.h`.
    pub fn capptr_rewild<T>(p: CapPtr<T, bounds::Alloc>) -> CapPtr<T, bounds::AllocWild> {
        // SAFETY: Widening the wildness annotation is always safe; we are only
        // removing the guarantee that the pointer has been domesticated.
        unsafe { CapPtr::unsafe_from(p.unsafe_ptr()) }
    }

    /// Assert that a chunk-user pointer represents a single large allocation.
    ///
    /// For large allocations the entire chunk *is* the allocation object.
    /// Converts `ChunkUser` bounds (chunk-level, user a-s control) to `Alloc`
    /// bounds (allocation-level, user a-s control) by changing only the
    /// spatial annotation.
    ///
    /// Mirrors `capptr_chunk_is_alloc()` in `ds_core/ptrwrap.h`.
    ///
    /// # Safety
    ///
    /// The pointer must really represent a large allocation whose bounds equal
    /// the entire chunk.
    pub unsafe fn capptr_chunk_is_alloc<T>(
        p: CapPtr<T, bounds::ChunkUser>,
    ) -> CapPtr<T, bounds::Alloc> {
        CapPtr::unsafe_from(p.unsafe_ptr())
    }

    // ── AtomicCapPtr ──────────────────────────────────────────────────────────

    /// An [`AtomicPtr`] annotated with a bounds type.
    ///
    /// Mirrors `AtomicCapPtr<T, bounds>` from `ds_core/ptrwrap.h`.
    ///
    /// All methods take a [`crate::atomics::Ordering`] parameter.
    pub struct AtomicCapPtr<T, B: Bound> {
        inner: AtomicPtr<T>,
        _bounds: PhantomData<B>,
    }

    impl<T, B: Bound> AtomicCapPtr<T, B> {
        /// Create a new `AtomicCapPtr` initialised to `val`.
        pub fn new(val: CapPtr<T, B>) -> Self {
            Self {
                inner: AtomicPtr::new(val.ptr),
                _bounds: PhantomData,
            }
        }

        /// Atomically load the pointer.
        ///
        /// Mirrors `AtomicCapPtr::load()` in `ds_core/ptrwrap.h`.
        pub fn load(&self, order: Ordering) -> CapPtr<T, B> {
            // SAFETY: The stored pointer was originally a valid CapPtr<T, B>;
            // loading it preserves the bounds annotation.
            unsafe { CapPtr::unsafe_from(self.inner.load(order)) }
        }

        /// Atomically store a new pointer value.
        ///
        /// Mirrors `AtomicCapPtr::store()` in `ds_core/ptrwrap.h`.
        pub fn store(&self, val: CapPtr<T, B>, order: Ordering) {
            self.inner.store(val.ptr, order);
        }

        /// Atomically swap the stored pointer, returning the old value.
        ///
        /// Mirrors `AtomicCapPtr::exchange()` in `ds_core/ptrwrap.h`.
        pub fn exchange(&self, val: CapPtr<T, B>, order: Ordering) -> CapPtr<T, B> {
            // SAFETY: Same reasoning as `load`.
            unsafe { CapPtr::unsafe_from(self.inner.swap(val.ptr, order)) }
        }
    }

    impl<T, B: Bound> Default for AtomicCapPtr<T, B> {
        /// Initialise to null, matching the C++ default constructor
        /// `AtomicCapPtr() : AtomicCapPtr(nullptr)`.
        fn default() -> Self {
            Self::new(CapPtr::null())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use bounds::*;

        #[test]
        fn null_is_null() {
            let p: CapPtr<u8, Alloc> = CapPtr::null();
            assert!(p.is_null());
            assert_eq!(p.unsafe_ptr(), core::ptr::null_mut());
        }

        #[test]
        fn default_is_null() {
            let p: CapPtr<u8, Alloc> = Default::default();
            assert!(p.is_null());
        }

        #[test]
        fn unsafe_from_roundtrip() {
            let mut x = 42u8;
            let raw = &mut x as *mut u8;
            let p: CapPtr<u8, Alloc> = unsafe { CapPtr::unsafe_from(raw) };
            assert_eq!(p.unsafe_ptr(), raw);
            assert!(!p.is_null());
        }

        #[test]
        fn as_void_preserves_address() {
            let mut x = 1u32;
            let raw = &mut x as *mut u32;
            let p: CapPtr<u32, Chunk> = unsafe { CapPtr::unsafe_from(raw) };
            let v = p.as_void();
            assert_eq!(v.unsafe_ptr() as usize, raw as usize);
        }

        #[test]
        fn equality_and_ordering() {
            let mut buf = [0u8; 4];
            let p0: CapPtr<u8, Alloc> =
                unsafe { CapPtr::unsafe_from(buf.as_mut_ptr()) };
            let p1: CapPtr<u8, Alloc> =
                // SAFETY: buf has 4 bytes, offset 1 is within bounds.
                unsafe { CapPtr::unsafe_from(buf.as_mut_ptr().add(1)) };
            assert!(p0 < p1);
            assert!(p0 != p1);
            assert_eq!(p0, p0);
        }

        #[test]
        fn capptr_reveal_roundtrip() {
            let mut x = 0u8;
            let raw = &mut x as *mut u8 as *mut ();
            let p: CapPtr<(), Alloc> = unsafe { CapPtr::unsafe_from(raw) };
            assert_eq!(capptr_reveal(p), raw);
        }

        #[test]
        fn capptr_from_client_roundtrip() {
            let mut x = 0u8;
            let raw = &mut x as *mut u8 as *mut ();
            let wild = capptr_from_client(raw);
            assert_eq!(wild.unsafe_ptr(), raw);
        }

        #[test]
        fn capptr_rewild() {
            let mut x = 0u8;
            let raw = &mut x as *mut u8;
            let tame: CapPtr<u8, Alloc> = unsafe { CapPtr::unsafe_from(raw) };
            let wild = super::capptr_rewild(tame);
            assert_eq!(wild.unsafe_ptr(), raw);
        }

        #[test]
        fn capptr_chunk_is_alloc_fn() {
            let mut x = 0u8;
            let raw = &mut x as *mut u8;
            let chunk: CapPtr<u8, ChunkUser> = unsafe { CapPtr::unsafe_from(raw) };
            let alloc = unsafe { super::capptr_chunk_is_alloc(chunk) };
            assert_eq!(alloc.unsafe_ptr(), raw);
        }

        // AtomicCapPtr must not be used outside a loom::model() closure when
        // compiled under loom.  The non-loom test exercises the API directly.
        #[cfg(not(loom))]
        #[test]
        fn atomic_cap_ptr() {
            use crate::atomics::Ordering;

            let mut x = 0u8;
            let raw = &mut x as *mut u8;
            let p: CapPtr<u8, Alloc> = unsafe { CapPtr::unsafe_from(raw) };
            let a = AtomicCapPtr::new(p);

            let loaded = a.load(Ordering::Acquire);
            assert_eq!(loaded.unsafe_ptr(), raw);

            a.store(CapPtr::null(), Ordering::Release);
            assert!(a.load(Ordering::Acquire).is_null());

            let old = a.exchange(p, Ordering::AcqRel);
            assert!(old.is_null());
            assert_eq!(a.load(Ordering::Acquire).unsafe_ptr(), raw);
        }

        #[cfg(not(loom))]
        #[test]
        fn atomic_cap_ptr_default_is_null() {
            let a: AtomicCapPtr<u8, Alloc> = Default::default();
            assert!(a.load(crate::atomics::Ordering::Relaxed).is_null());
        }

        // Loom model for AtomicCapPtr.
        // Run with: RUSTFLAGS="--cfg loom" cargo test -p snmalloc-core
        #[cfg(loom)]
        #[test]
        fn atomic_cap_ptr_loom() {
            // TODO: Add loom::model tests once there is more concurrent
            // usage of AtomicCapPtr to model.
        }
    }
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
