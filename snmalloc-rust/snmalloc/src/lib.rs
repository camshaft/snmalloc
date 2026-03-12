//! Pure-Rust port of the snmalloc high-performance allocator.
//!
//! This crate is the public-facing entry point.  It selects concrete PAL and
//! AAL types, wires them into a `StandardConfig`, provides the
//! `#[global_allocator]` implementation, and re-exports the public API.
//!
//! See `docs/rust_port_architecture.md` §18 for the overall crate structure.
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.

#![no_std]

use snmalloc_frontend as frontend;

// Suppress unused import warning while stubs are incomplete.
#[allow(unused_imports)]
use frontend as _;

// TODO: Select PAL and AAL based on target_os / target_arch cfg flags.
// TODO: Implement StandardConfig that wires concrete PAL + AAL into the
//       BackendAllocator and Allocator.
// TODO: Implement GlobalAlloc for the top-level allocator type.
