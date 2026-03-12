# snmalloc Rust Port — Status

This document tracks the progress of the pure-Rust port of snmalloc.
The port aims to be a faithful, functionally equivalent reimplementation of
the C++ original in safe (and, where necessary, `unsafe`) Rust, preserving
all behaviour and performance characteristics.

The authoritative design reference is
[`docs/rust_port_architecture.md`](../docs/rust_port_architecture.md).

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Complete and tested |
| 🔧 | In progress |
| 📋 | Planned / stub in place |
| ❌ | Not started |

---

## Workspace crates

| Crate | Description | Status |
|-------|-------------|--------|
| `snmalloc-pal` | Platform Abstraction Layer trait + OS implementations | 📋 |
| `snmalloc-aal` | Architecture Abstraction Layer trait + CPU implementations | 📋 |
| `snmalloc-core` | Configuration, size-class table, freelist, pagemap, MPSC, combining lock | 📋 |
| `snmalloc-backend` | Range chain, buddy allocators, `BackendAllocator` | 📋 |
| `snmalloc-frontend` | `Allocator`, `RemoteDeallocCache`, `ThreadAlloc`, `Pool` | 📋 |
| `snmalloc` | Public API, `StandardConfig`, `#[global_allocator]` | 📋 |
| `snmalloc-test` | Integration tests, bolero fuzz targets | 📋 |

---

## Component checklist

Progress is tracked against the sections of the architecture reference.

### §2 — Configuration constants (`snmalloc-core::config`)

- [x] Constants defined as `const` items matching `ds/allocconfig.h`
- [ ] Const-generic `Config` type with all tuning knobs
- [ ] `no_std` compile check

### §3 — Mitigation flags (`snmalloc-core::mitigations`)

- [ ] `Mitigations` bitmask type
- [ ] `CHECK_CLIENT` preset
- [ ] Compile-time conditional code paths for each mitigation

### §4 — Architecture Abstraction Layer (`snmalloc-aal`)

- [x] `Aal` trait skeleton (`capptr_bound`, `prefetch`, `tick`, `ADDRESS_BITS`)
- [x] `AalFeatures` struct (`integer_pointers`, `strict_provenance`)
- [ ] x86-64 implementation
- [ ] AArch64 implementation
- [ ] RISC-V implementation
- [ ] CHERI implementation

### §5 — Platform Abstraction Layer (`snmalloc-pal`)

- [x] `Pal` trait skeleton (`error`, `notify_not_using`, `notify_using`, `reserve`)
- [ ] Linux implementation (mmap, madvise)
- [ ] macOS implementation
- [ ] Windows implementation (VirtualAlloc, VirtualFree)
- [ ] OpenEnclave implementation

### §6 — Pointer provenance model (`snmalloc-core::ptrwrap`)

- [ ] `CapPtr<T, B>` wrapper with provenance bound type parameters
- [ ] `capptr_reveal` / `capptr_domesticate` operations
- [ ] Miri `-Zmiri-strict-provenance` clean

### §7 — Size-class system (`snmalloc-core::sizeclasses`)

- [ ] `SizeClassTable` computed by `const fn`
- [ ] `size_to_sizeclass` mapping
- [ ] Slab parameters (mask, capacity, waking threshold)
- [ ] Equivalence test against C++ table

### §8 — Core data structures (`snmalloc-core::ds`, `snmalloc-core::combininglock`)

- [ ] ABA-protected MPMC stack
- [ ] Intrusive red-black tree
- [ ] Combining lock
- [ ] Loom model tests for all concurrent structures

### §9 — Frontend: per-thread allocator (`snmalloc-frontend::allocator`)

- [ ] `FrontendSlabMetadata` struct
- [ ] `small_alloc` fast path
- [ ] `large_alloc` path
- [ ] `dealloc_local_object` fast path
- [ ] Message-queue drain (`handle_message_queue`)
- [ ] `alloc_classes` array indexed by size class

### §10 — Remote deallocation (`snmalloc-frontend::remote_cache`)

- [ ] `RemoteDeallocCache` with hash table of destination slots
- [ ] Batching rings (`DEALLOC_BATCH_RINGS`)
- [ ] Flush to remote MPSC queues
- [ ] Loom stress test: all deallocations on a different thread

### §11 — Backend: address-space range chain (`snmalloc-backend::range_chain`)

- [ ] `PalRange` — raw OS reserve/release
- [ ] `SubRange` — sub-divides a parent range
- [ ] `LargeBuddyRange` — coalescing buddy allocator
- [ ] `SmallBuddyRange` — sub-chunk buddy allocator
- [ ] `CommitRange` — decommit/recommit
- [ ] `StatsRange` — allocation statistics
- [ ] `GlobalRange` — process-wide singleton
- [ ] `BackendAllocator` — wires the chain

### §12 — Pagemap (`snmalloc-core::pagemap`)

- [ ] Flat pagemap (32-bit targets)
- [ ] Two-level pagemap (64-bit targets)
- [ ] `get_metaentry` / `set_metaentry` in O(1)
- [ ] `metadata_protection` mitigation

### §13 — Thread lifecycle (`snmalloc-frontend::thread_alloc`)

- [ ] Thread-local `Allocator` initialisation
- [ ] Thread-exit destructor (flush remote cache)
- [ ] `Pool<Allocator>` for reuse across threads

### §14 — Security features

- [ ] `freelist_forward_edge` XOR obfuscation
- [ ] `freelist_backward_edge` non-linear signature
- [ ] `freelist_teardown_validate`
- [ ] `random_initial` (Sattolo's algorithm)
- [ ] `random_preserve` (two-queue coin-flip)
- [ ] `random_open_slab`
- [ ] `random_pagemap`
- [ ] `random_larger_thresholds`

---

## Testing infrastructure

### Property-based testing and fuzzing (bolero)

[bolero](https://github.com/camshaft/bolero) provides a unified interface for
both property-based testing (stable toolchain) and continuous fuzzing (nightly +
libFuzzer / AFL / honggfuzz).

**Install the cargo-bolero runner:**

```sh
cargo install cargo-bolero
```

**Run all fuzz targets as ordinary property tests (stable):**

```sh
cargo test -p snmalloc-test
```

**Run a specific target as a continuous fuzzer (nightly required):**

```sh
cargo bolero test alloc_sequence --corpus corpus/alloc_sequence
cargo bolero test sizeclasses
cargo bolero test freelist_integrity
```

| Target | Description | Status |
|--------|-------------|--------|
| `alloc_sequence` | Random alloc/dealloc/realloc sequences; checks no double-issue | 📋 stub |
| `sizeclasses` | Size-class table equivalence for every possible input size | 📋 stub |
| `freelist_integrity` | Freelist forward/backward edge integrity under injection | 📋 stub |

### Concurrency testing (loom)

[loom](https://github.com/tokio-rs/loom) performs systematic exploration of
thread interleavings by intercepting every atomic operation.  It is used for
all concurrent data structures in `snmalloc-core` and `snmalloc-backend`.

All modules that use atomics import them through `snmalloc_core::atomics`,
which resolves to `loom::sync::atomic` when compiled with `--cfg loom`.

**Run loom model tests:**

```sh
RUSTFLAGS="--cfg loom" cargo test -p snmalloc-core
RUSTFLAGS="--cfg loom" cargo test -p snmalloc-backend
RUSTFLAGS="--cfg loom" cargo test -p snmalloc-frontend
```

| Test | Description | Status |
|------|-------------|--------|
| `mpsc_loom::two_producers_one_consumer` | MPSC queue with 2 producers | 📋 stub |
| `combininglock_loom` | Combining lock under contention | 📋 stub |
| `range_chain_loom` | Concurrent backend range chain access | 📋 stub |
| `remote_dealloc_loom` | Remote deallocation stress test | 📋 stub |

### Undefined-behaviour detection (miri)

[Miri](https://github.com/rust-lang/miri) is Rust's experimental interpreter
that detects memory errors, dangling pointers, and provenance violations.

The workspace `.cargo/config.toml` documents the recommended Miri flags;
pass them explicitly via `MIRIFLAGS` when invoking Miri:

```sh
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-symbolic-alignment-check" \
  cargo miri test [-p <crate>]
```

**Or use the workspace alias:**

```sh
cargo miri-test
```

| Crate | Miri clean | Notes |
|-------|-----------|-------|
| `snmalloc-pal` | ❌ | Not yet implemented |
| `snmalloc-aal` | ❌ | Not yet implemented |
| `snmalloc-core` | ❌ | Not yet implemented |
| `snmalloc-backend` | ❌ | Not yet implemented |
| `snmalloc-frontend` | ❌ | Not yet implemented |
| `snmalloc` | ❌ | Not yet implemented |
| `snmalloc-test` | ❌ | Not yet implemented |

---

## Equivalence verification plan

See `docs/rust_port_architecture.md` §17 for the full strategy.  The planned
verification steps are:

- [ ] §17.1 Property-based testing with bolero (alloc sequences)
- [ ] §17.2 Benchmark equivalence within 5% of C++ on `mimalloc-bench`
- [ ] §17.3 Size-class table equivalence (compile-time or test-time assertion)
- [ ] §17.4 Freelist integrity under injected corruption
- [ ] §17.5 Remote deallocation stress test (all frees on different thread)
- [ ] §17.6 Backend range chain isolation tests
- [ ] §17.7 Deterministic replay for a fixed entropy seed
