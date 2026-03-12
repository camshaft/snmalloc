//! Fuzz target: allocation/deallocation sequence.
//!
//! Generates random sequences of `alloc`, `dealloc`, `realloc`, and `calloc`
//! operations and checks the invariants described in
//! `docs/rust_port_architecture.md` §17.1:
//!
//! * Every successful allocation returns a non-null, correctly aligned pointer.
//! * No two live allocations share an address (no double-issue).
//! * Every deallocation is accepted without panic or detected corruption.
//! * After all deallocations the allocator reports zero live bytes (when the
//!   underlying implementation supports that query).
//!
//! ## Run as a continuous fuzzer
//!
//! ```text
//! cargo bolero test alloc_sequence
//! ```
//!
//! ## Run as an ordinary property test
//!
//! ```text
//! cargo test -p snmalloc-test alloc_sequence
//! ```

// TODO: Replace the placeholder model below with the real allocator once
// snmalloc-frontend is implemented.

/// A single allocation operation in the test sequence.
#[derive(Debug, bolero::TypeGenerator)]
enum AllocOp {
    /// Allocate `size` bytes with `align`-byte alignment.
    Alloc {
        /// Requested size in bytes (1 to 65535).
        #[generator(1u16..)]
        size: u16,
        /// Log2 of the alignment (0 to 6 → 1 to 64 bytes).
        #[generator(0u8..=6u8)]
        align_log2: u8,
    },
    /// Deallocate the allocation at the given index (if live).
    Dealloc {
        /// Index into the live-allocation table.
        index: u8,
    },
    /// Reallocate the allocation at the given index to a new size.
    Realloc {
        /// Index into the live-allocation table.
        index: u8,
        /// New size in bytes (1 to 65535).
        #[generator(1u16..)]
        new_size: u16,
    },
}

#[test]
fn alloc_sequence() {
    bolero::check!()
        .with_type::<Vec<AllocOp>>()
        .for_each(|ops| {
            // TODO: Drive the real snmalloc allocator once implemented.
            // For now this is a placeholder that validates the test harness.
            let _ = ops;
        });
}
