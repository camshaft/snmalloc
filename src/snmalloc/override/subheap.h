#pragma once

/**
 * Preallocated sub-heap API for snmalloc.
 *
 * Each sub-heap reserves a fixed virtual address region upfront and uses
 * snmalloc's normal allocation machinery within that region. Allocations
 * return nullptr only when the reserved region is exhausted — there is no
 * contended atomic on the allocation fast path.
 *
 * Implementation: `SubHeapPAL<N>` is a distinct type for each N, so
 * `FixedRangeConfig<SubHeapPAL<N>>` has completely independent static state
 * (pagemap, allocator pool, global range) from every other N and from the
 * global config. Up to SNMALLOC_MAX_SUB_HEAPS sub-heaps can be active at
 * once.
 *
 * Lifecycle: sub-heap slots are one-shot — once claimed, a slot's virtual
 * address space is never returned to the OS (the pagemap for that region
 * remains registered). This is acceptable for the intended use case of
 * long-lived bounded arenas.
 */

#include "snmalloc/backend/fixedglobalconfig.h"
#include "snmalloc/snmalloc.h"
#include "snmalloc/stl/atomic.h"
#include "snmalloc/stl/new.h"

namespace snmalloc
{
  /** Maximum number of sub-heaps that can be active simultaneously. */
  static constexpr size_t SNMALLOC_MAX_SUB_HEAPS = 8;

  namespace subheap_detail
  {
    /**
     * SubHeapPAL<N> is a unique type derived from DefaultPal for each N.
     * Using it as the PAL parameter of FixedRangeConfig creates a separate
     * template instantiation — and therefore separate static state — for each
     * slot.  All actual PAL operations are inherited from DefaultPal.
     *
     * Note: the static state of FixedRangeConfig (pagemap, allocator pool,
     * global range) is per-type — i.e. per-N — not per-instance. That is why
     * the number of slots must be fixed at compile time.
     */
    template<size_t N>
    struct SubHeapPAL : DefaultPal
    {};

    /**
     * Per-slot state and operations for slot N.
     *
     * FixedRangeConfig<SubHeapPAL<N>> has its own pagemap, allocator pool,
     * and global range, completely independent from every other slot and from
     * the standard global config.
     */
    template<size_t N>
    struct Slot
    {
      using Config = FixedRangeConfig<SubHeapPAL<N>>;
      using FixedAlloc = Allocator<Config>;

      /**
       * Guards one-time initialisation of this slot. Once set to true, the
       * slot is claimed for the lifetime of the process.
       *
       * Slots are one-shot: the static pagemap and allocator-pool state of
       * FixedRangeConfig cannot be safely reset or re-initialised, so a slot
       * that has been released cannot be reclaimed for a new sub-heap.
       */
      SNMALLOC_REQUIRE_CONSTINIT
      inline static stl::Atomic<bool> claimed{false};

      static void* do_alloc(void* a, size_t needed)
      {
        return static_cast<FixedAlloc*>(a)->template alloc<Uninit>(needed);
      }

      static void* do_alloc_zeroed(void* a, size_t needed)
      {
        return static_cast<FixedAlloc*>(a)->template alloc<Zero>(needed);
      }

      static void do_dealloc(void* a, void* ptr)
      {
        static_cast<FixedAlloc*>(a)->dealloc(ptr);
      }

      static void do_release(void* a)
      {
        auto* alloc = static_cast<FixedAlloc*>(a);
        alloc->flush();
        AllocPool<Config>::release(alloc);
      }

      /**
       * Attempt to claim this slot, reserve `size` bytes of virtual address
       * space, and return an acquired Allocator pointer (type-erased).
       *
       * Returns nullptr if the slot is already claimed or the reservation
       * fails. Once a slot is claimed, it is permanently consumed.
       */
      static void* try_create(size_t size)
      {
        bool expected = false;
        if (!claimed.compare_exchange_strong(
              expected, true, stl::memory_order_acq_rel))
          return nullptr; // slot already claimed

        void* mem = SubHeapPAL<N>::reserve(size);
        if (mem == nullptr)
        {
          // Leave claimed=true: we cannot reinitialise the pagemap even if
          // the reservation fails here (once init is called, it cannot be
          // called again). Signal failure to the caller.
          return nullptr;
        }

        Config::init(nullptr, mem, size);
        return AllocPool<Config>::acquire();
      }
    };

    // -----------------------------------------------------------------------
    // Slot dispatch: recursive if-constexpr chains select the right Slot<N>
    // static method based on a runtime slot index, without a vtable.
    // The compiler reduces these chains to a jump table for small N.
    // -----------------------------------------------------------------------

    template<size_t N = 0>
    SNMALLOC_FAST_PATH_INLINE void*
    dispatch_alloc(size_t slot, void* a, size_t needed)
    {
      if constexpr (N < SNMALLOC_MAX_SUB_HEAPS)
      {
        if (slot == N)
          return Slot<N>::do_alloc(a, needed);
        return dispatch_alloc<N + 1>(slot, a, needed);
      }
      SNMALLOC_ASSERT(false);
      return nullptr;
    }

    template<size_t N = 0>
    SNMALLOC_FAST_PATH_INLINE void*
    dispatch_alloc_zeroed(size_t slot, void* a, size_t needed)
    {
      if constexpr (N < SNMALLOC_MAX_SUB_HEAPS)
      {
        if (slot == N)
          return Slot<N>::do_alloc_zeroed(a, needed);
        return dispatch_alloc_zeroed<N + 1>(slot, a, needed);
      }
      SNMALLOC_ASSERT(false);
      return nullptr;
    }

    template<size_t N = 0>
    SNMALLOC_FAST_PATH_INLINE void
    dispatch_dealloc(size_t slot, void* a, void* ptr)
    {
      if constexpr (N < SNMALLOC_MAX_SUB_HEAPS)
      {
        if (slot == N)
        {
          Slot<N>::do_dealloc(a, ptr);
          return;
        }
        dispatch_dealloc<N + 1>(slot, a, ptr);
      }
      SNMALLOC_ASSERT(false);
    }

    template<size_t N = 0>
    void dispatch_release(size_t slot, void* a)
    {
      if constexpr (N < SNMALLOC_MAX_SUB_HEAPS)
      {
        if (slot == N)
        {
          Slot<N>::do_release(a);
          return;
        }
        dispatch_release<N + 1>(slot, a);
      }
      SNMALLOC_ASSERT(false);
    }

    /**
     * Walk slots 0..SNMALLOC_MAX_SUB_HEAPS-1 looking for an unclaimed one.
     * On success, sets out_slot to the claimed index and returns the acquired
     * Allocator pointer; returns nullptr if all slots are taken.
     */
    template<size_t N = 0>
    void* find_and_create(size_t size, size_t& out_slot)
    {
      if constexpr (N < SNMALLOC_MAX_SUB_HEAPS)
      {
        void* a = Slot<N>::try_create(size);
        if (a != nullptr)
        {
          out_slot = N;
          return a;
        }
        return find_and_create<N + 1>(size, out_slot);
      }
      return nullptr; // all slots exhausted
    }

  } // namespace subheap_detail

  /**
   * Opaque sub-heap handle. Interact with it only via sub_heap_* functions.
   */
  struct SubHeapHandle
  {
    /** Pointer to the Allocator<FixedRangeConfig<SubHeapPAL<slot>>>. */
    void* alloc_ptr;

    /** Index of the slot backing this handle (0..SNMALLOC_MAX_SUB_HEAPS-1). */
    size_t slot;
  };

  /**
   * Create a sub-heap that preallocates (reserves) `size` bytes of virtual
   * address space. Allocations draw from that region using snmalloc's normal
   * allocation machinery — no extra atomic on the fast path. When the region
   * is exhausted, allocations return nullptr.
   *
   * Returns nullptr if:
   *  - `size` is less than MIN_CHUNK_SIZE (region too small to hold even one
   *    slab after pagemap overhead).
   *  - All SNMALLOC_MAX_SUB_HEAPS slots have been consumed.
   *  - The virtual address space reservation fails.
   *  - The handle allocation from the global heap fails.
   */
  inline SubHeapHandle* create_sub_heap(size_t size)
  {
    // A region smaller than one slab cannot hold any allocations after the
    // pagemap overhead is subtracted; reject it early with a clear message.
    if (size < MIN_CHUNK_SIZE)
      return nullptr;

    size_t slot = SNMALLOC_MAX_SUB_HEAPS; // sentinel: "not yet assigned"
    void* a = subheap_detail::find_and_create(size, slot);
    if (a == nullptr)
      return nullptr; // all slots exhausted or reservation failed

    // Allocate the handle from the global heap (not from the sub-heap).
    void* p = snmalloc::alloc(sizeof(SubHeapHandle));
    if (p == nullptr)
    {
      subheap_detail::dispatch_release(slot, a);
      return nullptr;
    }
    return new (p, placement_token) SubHeapHandle{a, slot};
  }

  /**
   * Allocate from a sub-heap.
   *
   * Returns nullptr when the preallocated region is exhausted. The caller
   * must pass the same alignment and size to sub_heap_dealloc.
   */
  SNMALLOC_FAST_PATH_INLINE void*
  sub_heap_alloc(SubHeapHandle* heap, size_t alignment, size_t size)
  {
    return subheap_detail::dispatch_alloc(
      heap->slot, heap->alloc_ptr, aligned_size(alignment, size));
  }

  /**
   * Allocate zero-initialised memory from a sub-heap.
   *
   * Same contract as sub_heap_alloc.
   */
  SNMALLOC_FAST_PATH_INLINE void*
  sub_heap_alloc_zeroed(SubHeapHandle* heap, size_t alignment, size_t size)
  {
    return subheap_detail::dispatch_alloc_zeroed(
      heap->slot, heap->alloc_ptr, aligned_size(alignment, size));
  }

  /**
   * Free a pointer previously allocated from this sub-heap.
   *
   * The freed object is returned to the allocator's thread-local free list;
   * future sub_heap_alloc calls on any thread may reuse it.
   */
  SNMALLOC_FAST_PATH_INLINE void
  sub_heap_dealloc(SubHeapHandle* heap, void* ptr, size_t alignment, size_t size)
  {
    UNUSED(alignment);
    UNUSED(size);
    subheap_detail::dispatch_dealloc(heap->slot, heap->alloc_ptr, ptr);
  }

  /**
   * Destroy a sub-heap: flush the allocator, release it to the pool, and
   * free the handle.
   *
   * The reserved virtual address space is NOT returned to the OS because the
   * pagemap for that region remains active. The slot is permanently consumed.
   *
   * All live allocations from this sub-heap must have been freed (via
   * sub_heap_dealloc) before calling this function.
   */
  inline void destroy_sub_heap(SubHeapHandle* heap)
  {
    subheap_detail::dispatch_release(heap->slot, heap->alloc_ptr);
    snmalloc::dealloc(heap, sizeof(SubHeapHandle));
  }

} // namespace snmalloc
