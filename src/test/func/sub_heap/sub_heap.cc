/**
 * Functional test for the sub-heap API.
 *
 * Verifies:
 *  1. Allocations within budget succeed.
 *  2. Allocations that would exceed the budget return nullptr.
 *  3. Deallocating into the sub-heap returns budget so that further
 *     allocations can succeed.
 *  4. Multiple independent sub-heaps do not interfere with each other.
 *  5. A zero-budget sub-heap rejects every allocation.
 */
#include "test/setup.h"

#include <iostream>
#include <snmalloc/snmalloc.h>

// Include sub-heap implementation directly.
// snmalloc.h was included above so all required types are available.
#include <snmalloc/override/subheap.h>

#ifdef assert
#  undef assert
#endif
#define assert please_use_SNMALLOC_ASSERT

using namespace snmalloc;

static void test_budget_enforced()
{
  // Create a small sub-heap (1 KiB).
  constexpr size_t limit = 1024;
  auto* heap = create_sub_heap(limit);
  SNMALLOC_CHECK(heap != nullptr);

  // Allocate as much as possible in 64-byte chunks.
  constexpr size_t chunk = 64;
  constexpr size_t alignment = 1;
  size_t alloc_count = 0;
  void* ptrs[limit / chunk + 1] = {};

  while (alloc_count <= limit / chunk)
  {
    void* p = sub_heap_alloc(heap, alignment, chunk);
    if (p == nullptr)
      break;
    ptrs[alloc_count++] = p;
  }

  // We should have been able to allocate at least some objects.
  SNMALLOC_CHECK(alloc_count > 0);
  // The next allocation must fail (budget exhausted or no more budget).
  void* overflow = sub_heap_alloc(heap, alignment, chunk);
  SNMALLOC_CHECK(overflow == nullptr);

  std::cout << "  allocated " << alloc_count << " chunks of " << chunk
            << " bytes within a " << limit << "-byte budget\n";

  // Free everything – budget should be fully returned.
  for (size_t i = 0; i < alloc_count; ++i)
    sub_heap_dealloc(heap, ptrs[i], alignment, chunk);

  // After freeing, one more allocation should succeed.
  void* after = sub_heap_alloc(heap, alignment, chunk);
  SNMALLOC_CHECK(after != nullptr);
  sub_heap_dealloc(heap, after, alignment, chunk);

  destroy_sub_heap(heap);
  std::cout << "  test_budget_enforced passed\n";
}

static void test_zeroed_alloc()
{
  constexpr size_t limit = 512;
  auto* heap = create_sub_heap(limit);
  SNMALLOC_CHECK(heap != nullptr);

  constexpr size_t sz = 64;
  void* p = sub_heap_alloc_zeroed(heap, 1, sz);
  SNMALLOC_CHECK(p != nullptr);

  // Verify memory is zeroed.
  const auto* bytes = static_cast<const unsigned char*>(p);
  for (size_t i = 0; i < sz; ++i)
    SNMALLOC_CHECK(bytes[i] == 0);

  sub_heap_dealloc(heap, p, 1, sz);
  destroy_sub_heap(heap);
  std::cout << "  test_zeroed_alloc passed\n";
}

static void test_two_heaps_independent()
{
  constexpr size_t limit = 256;
  auto* h1 = create_sub_heap(limit);
  auto* h2 = create_sub_heap(limit);
  SNMALLOC_CHECK(h1 != nullptr);
  SNMALLOC_CHECK(h2 != nullptr);

  // Exhaust heap 1.
  void* p1[limit + 1] = {};
  size_t n1 = 0;
  while (n1 <= limit)
  {
    void* p = sub_heap_alloc(h1, 1, 32);
    if (p == nullptr)
      break;
    p1[n1++] = p;
  }
  SNMALLOC_CHECK(n1 > 0);
  // h1 is now full, but h2 should still allocate.
  void* p2 = sub_heap_alloc(h2, 1, 32);
  SNMALLOC_CHECK(p2 != nullptr);

  // Cleanup.
  for (size_t i = 0; i < n1; ++i)
    sub_heap_dealloc(h1, p1[i], 1, 32);
  sub_heap_dealloc(h2, p2, 1, 32);

  destroy_sub_heap(h1);
  destroy_sub_heap(h2);
  std::cout << "  test_two_heaps_independent passed\n";
}

static void test_zero_budget()
{
  auto* heap = create_sub_heap(0);
  SNMALLOC_CHECK(heap != nullptr);

  void* p = sub_heap_alloc(heap, 1, 1);
  SNMALLOC_CHECK(p == nullptr);

  destroy_sub_heap(heap);
  std::cout << "  test_zero_budget passed\n";
}

int main()
{
  setup();
  std::cout << "sub_heap tests:\n";
  test_budget_enforced();
  test_zeroed_alloc();
  test_two_heaps_independent();
  test_zero_budget();
  std::cout << "all sub_heap tests passed\n";
  return 0;
}
