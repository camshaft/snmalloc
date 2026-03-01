#pragma once

/**
 * Sub-heap API for snmalloc.
 *
 * Provides bounded sub-heaps (arenas) that draw from the global snmalloc
 * allocator but enforce a configurable byte budget. Any allocation that would
 * exceed the budget returns nullptr instead of allocating memory.
 *
 * A sub-heap is created with a fixed size limit. Allocations are charged
 * against that limit using aligned_size(alignment, size) as the unit; the
 * same value must be passed to sub_heap_dealloc so that the budget is
 * correctly reclaimed.
 *
 * Thread safety: concurrent alloc/dealloc on the same handle is safe (the
 * in_use counter is updated with atomic compare-exchange and fetch_sub).
 *
 * Lifecycle contract: the caller must free all live allocations from a
 * sub-heap before calling destroy_sub_heap. Failure to do so will silently
 * leak the underlying snmalloc memory.
 *
 * This header requires snmalloc/snmalloc.h to have been included beforehand,
 * or includes it itself when used standalone.
 */

#include "snmalloc/snmalloc.h"

#include "snmalloc/stl/atomic.h"
#include "snmalloc/stl/new.h"

namespace snmalloc
{
  /**
   * Opaque handle for a sub-heap. Callers should treat this as an opaque
   * pointer and only interact with it through the sub_heap_* functions.
   */
  struct SubHeapHandle
  {
    /**
     * Maximum number of bytes (measured in aligned_size units) that may be
     * simultaneously live in this sub-heap.
     */
    size_t limit;

    /**
     * Current number of bytes live in this sub-heap. Updated atomically.
     */
    stl::Atomic<size_t> in_use;
  };

  /**
   * Create a new sub-heap with the given byte budget.
   *
   * Returns nullptr if the internal allocation for the handle itself fails.
   * A limit of 0 creates a valid (but empty) sub-heap: every allocation from
   * it will fail immediately.
   */
  inline SubHeapHandle* create_sub_heap(size_t size_limit)
  {
    void* p = alloc(sizeof(SubHeapHandle));
    if (p == nullptr)
      return nullptr;
    auto* heap = new (p, placement_token) SubHeapHandle;
    heap->limit = size_limit;
    heap->in_use.store(0, stl::memory_order_relaxed);
    return heap;
  }

  namespace sub_heap_internal
  {
    /**
     * Try to claim `needed` bytes from the budget. Returns true on success.
     * Uses an optimistic compare-exchange loop so that concurrent allocations
     * are all serialised through the atomic without a lock.
     */
    SNMALLOC_FAST_PATH_INLINE bool
    claim_budget(SubHeapHandle* heap, size_t needed)
    {
      size_t old = heap->in_use.load(stl::memory_order_relaxed);
      while (true)
      {
        if (old + needed > heap->limit)
          return false;
        if (heap->in_use.compare_exchange_weak(
              old,
              old + needed,
              stl::memory_order_acq_rel,
              stl::memory_order_relaxed))
          return true;
      }
    }
  } // namespace sub_heap_internal

  /**
   * Allocate from a sub-heap with the given alignment and size.
   *
   * Returns nullptr if the budget would be exceeded or if the underlying
   * snmalloc allocation fails.
   *
   * The caller must pass the same alignment and size to sub_heap_dealloc.
   */
  SNMALLOC_FAST_PATH_INLINE void*
  sub_heap_alloc(SubHeapHandle* heap, size_t alignment, size_t size)
  {
    size_t needed = aligned_size(alignment, size);
    if (!sub_heap_internal::claim_budget(heap, needed))
      return nullptr;
    void* p = alloc(needed);
    if (SNMALLOC_UNLIKELY(p == nullptr))
      heap->in_use.fetch_sub(needed, stl::memory_order_release);
    return p;
  }

  /**
   * Allocate zero-initialised memory from a sub-heap.
   *
   * Same semantics as sub_heap_alloc, but the returned memory is zeroed.
   */
  SNMALLOC_FAST_PATH_INLINE void*
  sub_heap_alloc_zeroed(SubHeapHandle* heap, size_t alignment, size_t size)
  {
    size_t needed = aligned_size(alignment, size);
    if (!sub_heap_internal::claim_budget(heap, needed))
      return nullptr;
    void* p = alloc<Zero>(needed);
    if (SNMALLOC_UNLIKELY(p == nullptr))
      heap->in_use.fetch_sub(needed, stl::memory_order_release);
    return p;
  }

  /**
   * Free a pointer previously allocated from a sub-heap, returning the bytes
   * to the budget so that subsequent allocations may succeed.
   *
   * alignment and size must match the values passed to sub_heap_alloc /
   * sub_heap_alloc_zeroed.
   */
  SNMALLOC_FAST_PATH_INLINE void sub_heap_dealloc(
    SubHeapHandle* heap, void* ptr, size_t alignment, size_t size)
  {
    size_t needed = aligned_size(alignment, size);
    dealloc(ptr, needed);
    heap->in_use.fetch_sub(needed, stl::memory_order_release);
  }

  /**
   * Destroy a sub-heap, freeing its internal handle allocation.
   *
   * All live allocations from this sub-heap must have been freed before
   * calling this function. Calling it with outstanding live allocations
   * results in a budget leak but does not otherwise corrupt state.
   */
  inline void destroy_sub_heap(SubHeapHandle* heap)
  {
    dealloc(heap, sizeof(SubHeapHandle));
  }
} // namespace snmalloc
