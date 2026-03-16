//! Criterion benchmarks comparing sub-heap and global-allocator alloc/dealloc.
//!
//! Run with:
//!   cargo bench -p snmalloc-rs --bench sub_heap
//!
//! Each benchmark group contains a `global` and a `sub_heap` variant for each
//! allocation size so they appear side-by-side in the criterion report.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use snmalloc_rs::{SnMalloc, SubHeap};
use std::alloc::{GlobalAlloc, Layout};

/// Allocation sizes to benchmark: small, typical object, medium, large.
const SIZES: &[usize] = &[8, 64, 256, 1024];

/// Virtual address region reserved for each sub-heap benchmark: 32 MiB.
/// Large enough to stay fully warm across all criterion iterations.
const REGION: usize = 32 * 1024 * 1024;

/// Benchmark a single alloc+dealloc cycle, comparing the global snmalloc
/// allocator against a pre-reserved sub-heap.
fn alloc_dealloc(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc_dealloc");

    for &size in SIZES {
        let layout = Layout::from_size_align(size, 8).unwrap();
        group.throughput(Throughput::Elements(1));

        // --- global allocator ---
        group.bench_with_input(
            BenchmarkId::new("global", size),
            &layout,
            |b, &layout| {
                let alloc = SnMalloc;
                b.iter(|| unsafe {
                    let ptr = alloc.alloc(layout);
                    black_box(ptr);
                    alloc.dealloc(ptr, layout);
                });
            },
        );

        // --- sub-heap ---
        // The sub-heap is created once outside the iter loop so that slot
        // claiming and region reservation are not included in the measured
        // time.  Because each iteration frees the allocation before the next
        // one, the thread-local slab free list stays warm and the region is
        // never exhausted.
        let heap = SubHeap::new(REGION).expect("sub-heap creation failed");
        group.bench_with_input(
            BenchmarkId::new("sub_heap", size),
            &layout,
            |b, &layout| {
                b.iter(|| unsafe {
                    let ptr = GlobalAlloc::alloc(&heap, layout);
                    black_box(ptr);
                    if !ptr.is_null() {
                        GlobalAlloc::dealloc(&heap, ptr, layout);
                    }
                });
            },
        );
        // `heap` is dropped here; all allocations have been freed in the loop.
    }

    group.finish();
}

criterion_group!(benches, alloc_dealloc);
criterion_main!(benches);
