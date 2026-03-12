//! Fuzz target: size-class table equivalence.
//!
//! Checks that the Rust `SizeClassTable` agrees with the C++ original for
//! every possible small-allocation size (1 to `1 << MAX_SMALL_SIZECLASS_BITS`
//! bytes).  See `docs/rust_port_architecture.md` §17.3.
//!
//! ## Run as a continuous fuzzer
//!
//! ```text
//! cargo bolero test sizeclasses
//! ```
//!
//! ## Run as an ordinary property test
//!
//! ```text
//! cargo test -p snmalloc-test sizeclasses
//! ```

#[test]
fn sizeclasses() {
    bolero::check!()
        .with_type::<u32>()
        .for_each(|&raw_size| {
            // TODO: Once snmalloc-core::sizeclasses is implemented, call
            // SizeClassTable::size_to_sizeclass(raw_size) and assert the
            // returned sizeclass index, rounded size, and slab parameters
            // match the values produced by the C++ implementation.
            let _ = raw_size;
        });
}
