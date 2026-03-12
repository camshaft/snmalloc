//! Per-thread allocator and remote deallocation cache for the snmalloc Rust port.
//!
//! This crate implements the frontend described in
//! `docs/rust_port_architecture.md` §9–§10 and §13:
//!
//! * `Allocator` — per-thread allocator with small and large allocation paths
//! * `RemoteDeallocCache` — batches remote frees before forwarding them
//! * `ThreadAlloc` — manages thread-local `Allocator` lifecycle
//! * `Pool` — global pool of reusable `Allocator` instances
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

use snmalloc_backend as backend;

/// Per-thread allocator.
///
/// Corresponds to `Allocator` in `mem/corealloc.h`.
pub mod allocator {
    // TODO: Implement small_alloc, large_alloc, dealloc_local_object,
    // handle_message_queue.
}

/// Remote deallocation cache.
///
/// Corresponds to `RemoteDeallocCache` in `mem/remotecache.h`.
pub mod remote_cache {
    // TODO: Implement the remote-deallocation cache with batching rings.
}

/// Thread-local allocator lifecycle management.
///
/// Corresponds to `global/threadalloc.h`.
pub mod thread_alloc {
    // TODO: Implement per-thread init, teardown, and the destructor that
    // flushes the remote cache on thread exit.
}

// Suppress unused import warning while stubs are incomplete.
#[allow(unused_imports)]
use backend as _;

// ── Loom tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(loom)]
    mod remote_dealloc_loom {
        // TODO: Stress-test remote deallocation across threads.
    }
}
