//! Integration tests and fuzz targets for the snmalloc Rust port.
//!
//! This crate provides:
//!
//! * **Bolero fuzz/property-based tests** (`tests/`) — each test file contains
//!   one or more `#[test]` functions that call `bolero::check!()`.  They run
//!   as ordinary Cargo tests by default and as continuous fuzz targets when
//!   invoked via `cargo bolero test <target>`.
//!
//! * **Equivalence tests** against the C++ implementation (long-term goal).
//!
//! ## Running the fuzz targets
//!
//! Install the bolero runner:
//!
//! ```text
//! cargo install cargo-bolero
//! ```
//!
//! Run a target under libFuzzer (requires nightly):
//!
//! ```text
//! cargo bolero test alloc_sequence --corpus corpus/alloc_sequence
//! ```
//!
//! Run all targets as ordinary property tests (stable toolchain):
//!
//! ```text
//! cargo test -p snmalloc-test
//! ```
//!
//! ## Running under Miri
//!
//! ```text
//! cargo miri test -p snmalloc-test
//! ```
//!
//! # Status
//!
//! This crate is a stub.  See `STATUS.md` at the workspace root for a
//! component-by-component progress tracker.
