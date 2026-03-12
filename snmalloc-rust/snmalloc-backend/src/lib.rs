//! Backend address-space management for the snmalloc Rust port.
//!
//! This crate implements the range chain described in
//! `docs/rust_port_architecture.md` §11:
//!
//! * `PalRange` — raw OS allocation/deallocation
//! * `SubRange` — sub-divides a parent range
//! * `LargeBuddyRange` — coalescing power-of-two buddy allocator
//! * `SmallBuddyRange` — small buddy allocator for sub-chunk allocations
//! * `CommitRange` — decommit/recommit wrapper
//! * `StatsRange` — allocation statistics
//! * `GlobalRange` — process-wide singleton range
//! * `BackendAllocator` — wires the chain together
//!
//! All concurrent access to the range chain uses atomics imported from
//! `snmalloc_core::atomics`, which resolves to `loom::sync::atomic` when
//! compiled under `--cfg loom`.
//!
//! ## Loom testing
//!
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test -p snmalloc-backend
//! ```
//!
//! ## Miri testing
//!
//! ```text
//! cargo miri test -p snmalloc-backend
//! ```
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

use snmalloc_core as core_alloc;

/// The range chain.
///
/// Corresponds to the range chain described in
/// `docs/rust_port_architecture.md` §11.
pub mod range_chain {
    // TODO: Implement PalRange, SubRange, LargeBuddyRange, SmallBuddyRange,
    // CommitRange, StatsRange, GlobalRange.
}

/// The backend allocator wiring.
///
/// Corresponds to `BackendAllocator` in the C++ source.
pub mod backend {
    // TODO: Wire the range chain into BackendAllocator.
    // The BackendAllocator must implement the Backend trait consumed by
    // snmalloc-frontend.
}

// Suppress unused import warning while stubs are incomplete.
#[allow(unused_imports)]
use core_alloc as _;

// ── Loom tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(loom)]
    mod range_chain_loom {
        // TODO: Add loom::model tests for concurrent range_chain access.
    }
}
