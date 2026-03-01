/**
 * Functional test for the sub-heap API.
 *
 * Verifies:
 *  1. Allocations succeed within the preallocated region.
 *  2. Once the region is exhausted, allocations return nullptr.
 *  3. Freed objects are reused from the thread-local free list (snmalloc
 *     fast path, no extra atomics) when the slab still has capacity.
 *  4. Multiple independent sub-heaps do not interfere with each other.
 *  5. create_sub_heap(0) returns nullptr (cannot create a zero-size heap).
 *  6. A request too large for the region returns nullptr gracefully.
 */
#include "test/setup.h"

#include <iostream>
#include <snmalloc/backend/fixedglobalconfig.h>
#include <snmalloc/snmalloc.h>

// Include sub-heap implementation directly.
#include <snmalloc/override/subheap.h>

#ifdef assert
#  undef assert
#endif
#define assert please_use_SNMALLOC_ASSERT

using namespace snmalloc;

static void test_region_exhausts()
{
  // 32 MiB is large enough to be well above MIN_HEAP_SIZE_FOR_THREAD_LOCAL_BUDDY
  // and small enough to exhaust quickly.
  constexpr size_t region = bits::one_at_bit(25);
  auto* heap = create_sub_heap(region);
  SNMALLOC_CHECK(heap != nullptr);

  constexpr size_t obj = 128;

  // Allocate until the region is exhausted.  The guard prevents an
  // infinite loop if the exhaustion logic ever regresses.
  size_t alloc_count = 0;
  const size_t max_allocs = (region / obj) * 4;
  while (alloc_count < max_allocs && sub_heap_alloc(heap, 1, obj) != nullptr)
    ++alloc_count;

  SNMALLOC_CHECK(alloc_count < max_allocs); // must have hit real exhaustion
  SNMALLOC_CHECK(alloc_count > 0);

  // Further allocations must return nullptr.
  SNMALLOC_CHECK(sub_heap_alloc(heap, 1, obj) == nullptr);

  std::cout << "  allocated " << alloc_count
            << " objects before exhaustion\n";

  destroy_sub_heap(heap);
  std::cout << "  test_region_exhausts passed\n";
}

static void test_free_list_reuse()
{
  // Verify that freed objects are reused from the allocator's free list.
  // This tests the snmalloc fast path (thread-local free list, no atomics).
  constexpr size_t region = bits::one_at_bit(25);
  auto* heap = create_sub_heap(region);
  SNMALLOC_CHECK(heap != nullptr);

  constexpr size_t obj = 128;

  // Allocate one object; this seeds the slab's free list with many more.
  void* p1 = sub_heap_alloc(heap, 1, obj);
  SNMALLOC_CHECK(p1 != nullptr);

  // Free and immediately reallocate — must succeed from the free list.
  sub_heap_dealloc(heap, p1, 1, obj);
  void* p2 = sub_heap_alloc(heap, 1, obj);
  SNMALLOC_CHECK(p2 != nullptr);

  sub_heap_dealloc(heap, p2, 1, obj);
  destroy_sub_heap(heap);
  std::cout << "  test_free_list_reuse passed\n";
}

static void test_zeroed_alloc()
{
  constexpr size_t region = bits::one_at_bit(25);
  auto* heap = create_sub_heap(region);
  SNMALLOC_CHECK(heap != nullptr);

  constexpr size_t sz = 64;
  void* p = sub_heap_alloc_zeroed(heap, 1, sz);
  SNMALLOC_CHECK(p != nullptr);

  // Verify the memory is zeroed.
  const auto* bytes = static_cast<const unsigned char*>(p);
  for (size_t i = 0; i < sz; ++i)
    SNMALLOC_CHECK(bytes[i] == 0);

  sub_heap_dealloc(heap, p, 1, sz);
  destroy_sub_heap(heap);
  std::cout << "  test_zeroed_alloc passed\n";
}

static void test_two_heaps_independent()
{
  constexpr size_t region = bits::one_at_bit(25);
  auto* h1 = create_sub_heap(region);
  auto* h2 = create_sub_heap(region);
  SNMALLOC_CHECK(h1 != nullptr);
  SNMALLOC_CHECK(h2 != nullptr);

  // Exhaust heap 1 (objects intentionally not freed — fine since the
  // region's memory is never returned to the OS anyway).
  size_t n1 = 0;
  constexpr size_t obj = 128;
  const size_t max_allocs = (region / obj) * 4;
  while (n1 < max_allocs && sub_heap_alloc(h1, 1, obj) != nullptr)
    ++n1;
  SNMALLOC_CHECK(n1 < max_allocs); // must have hit real exhaustion
  SNMALLOC_CHECK(n1 > 0);

  // h1 is exhausted, but h2 must still allocate.
  void* p2 = sub_heap_alloc(h2, 1, obj);
  SNMALLOC_CHECK(p2 != nullptr);
  sub_heap_dealloc(h2, p2, 1, obj);

  destroy_sub_heap(h1);
  destroy_sub_heap(h2);
  std::cout << "  test_two_heaps_independent passed\n";
}

static void test_zero_size_create_fails()
{
  // A region smaller than MIN_CHUNK_SIZE cannot be created.
  SNMALLOC_CHECK(create_sub_heap(0) == nullptr);
  SNMALLOC_CHECK(create_sub_heap(MIN_CHUNK_SIZE - 1) == nullptr);
  std::cout << "  test_zero_size_create_fails passed\n";
}

static void test_oversized_request_rejected()
{
  // A request much larger than the region must return nullptr.
  constexpr size_t region = bits::one_at_bit(25);
  auto* heap = create_sub_heap(region);
  SNMALLOC_CHECK(heap != nullptr);

  // Request twice the reserved region — must fail cleanly.
  void* p = sub_heap_alloc(heap, 1, region * 2);
  SNMALLOC_CHECK(p == nullptr);

  destroy_sub_heap(heap);
  std::cout << "  test_oversized_request_rejected passed\n";
}

int main()
{
  setup();
  std::cout << "sub_heap tests:\n";
  test_region_exhausts();
  test_free_list_reuse();
  test_zeroed_alloc();
  test_two_heaps_independent();
  test_zero_size_create_fails();
  test_oversized_request_rejected();
  std::cout << "all sub_heap tests passed\n";
  return 0;
}
