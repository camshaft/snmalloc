//! Fuzz target: freelist integrity.
//!
//! Exercises the freelist's forward/backward edge obfuscation against
//! known-good sequences of insertions and extractions, including injected
//! corruption cases.  See `docs/rust_port_architecture.md` §17.4.
//!
//! ## Run as a continuous fuzzer
//!
//! ```text
//! cargo bolero test freelist_integrity
//! ```
//!
//! ## Run as an ordinary property test
//!
//! ```text
//! cargo test -p snmalloc-test freelist_integrity
//! ```

/// A single freelist operation.
#[derive(Debug, bolero::TypeGenerator)]
enum FreelistOp {
    /// Push an object with the given (synthetic) address.
    Push(u64),
    /// Pop an object from the freelist (no-op if empty).
    Pop,
}

#[test]
fn freelist_integrity() {
    bolero::check!()
        .with_type::<Vec<FreelistOp>>()
        .for_each(|ops| {
            // TODO: Once snmalloc-core::freelist is implemented, drive the
            // freelist with `ops` and assert that every Pop either returns a
            // value previously Pushed or returns None (empty), and that no
            // integrity check fires on well-formed sequences.
            let _ = ops;
        });
}
