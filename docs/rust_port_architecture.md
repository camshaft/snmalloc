# snmalloc Rust Port: Architecture Reference

This document describes every component of snmalloc, their internal design, and
how they interact.  Its purpose is to guide a faithful Rust port that preserves
all behaviour and performance characteristics of the C++ original, and to
provide a basis for verifying equivalence between the two implementations.

---

## 1. Design Philosophy and Guiding Principles

snmalloc is a high-performance, security-hardened, general-purpose allocator.
Its design rests on several explicit principles that must be carried intact into
the Rust port.

**Thread-local fast paths.**  Every thread owns its allocator state.  The
common allocation and deallocation operations access only thread-local data and
require no atomic operations or locks.

**Message-passing for remote deallocation.**  When one thread frees memory that
was allocated by a different thread, it does not touch the owning thread's state
directly.  Instead it enqueues a message in the owning thread's lock-free
multi-producer single-consumer (MPSC) queue.  The owner drains that queue
lazily, during its own future allocations.  This design allows thousands of
remote deallocations to be acknowledged with a single atomic operation.

**Separated metadata.**  All slab metadata lives in a separate region of address
space, away from user allocations.  This makes metadata corruption from
out-of-bounds writes much harder than in allocators that store headers
immediately adjacent to user memory.

**Freelist integrity protection.**  Every forward pointer in a free list is
obfuscated with a random XOR key.  Each free object also carries an obfuscated
backward-edge signature that is a non-linear function of both its own address
and the address of its successor.  Any write through a dangling pointer that
corrupts the list is detected probabilistically when the corrupted entry is next
consumed.

**Randomised allocation order.**  The initial per-slab free list is a cyclic
permutation constructed by Sattolo's algorithm.  During operation, freed objects
are randomly assigned to one of two per-slab queues; the longer queue is always
preferred for allocation.  These mechanisms make the sequence of returned
addresses unpredictable to an adversary.

**Compile-time configuration.**  All tuning knobs (chunk size, size-class
granularity, batch limits, hardening mitigations) are compile-time constants.
The entire allocator is templated on a single `Config` type, so different
security levels or embedded-system profiles compile to wholly separate code
with no run-time branching.

**Platform and architecture abstraction.**  All OS interactions pass through a
Platform Abstraction Layer (PAL) and all CPU-specific operations pass through an
Architecture Abstraction Layer (AAL).  Both layers are expressed as type
constraints in C++; the Rust port should represent them as traits.

---

## 2. Configuration Constants (`ds/allocconfig.h`)

Before describing any component, the compile-time constants that govern the
entire system must be understood, because every data structure is sized or
indexed by them.

`INTERMEDIATE_BITS` (default 2) controls the number of size classes within each
power-of-two band.  Each band `[2^k, 2^(k+1))` contains exactly
`2^INTERMEDIATE_BITS` size classes; with the default of 2 that is four classes
per band (one at the boundary and three intermediate ones), giving a total of
roughly 200 small size classes.

`MIN_ALLOC_STEP_SIZE` (default `2 * sizeof(void*)`, i.e. 16 bytes on 64-bit) is
the granularity of the smallest size classes and is always a power of two.
`MIN_ALLOC_SIZE` (also 16 bytes) is the minimum allocation size that the
allocator honours; requests smaller than this are rounded up silently.

`MIN_CHUNK_BITS` (default 14, so `MIN_CHUNK_SIZE` = 16 KiB) is the base-2
logarithm of the smallest unit of address space that snmalloc ever carves from
the OS or assigns to a slab.  It is also the granularity of the pagemap.

`MAX_SMALL_SIZECLASS_BITS` (default 16) determines the largest size class
handled by the slab-based allocator.  Any allocation whose rounded-up size
exceeds `2^MAX_SMALL_SIZECLASS_BITS` bytes is a "large" allocation and is
handled by the backend directly rather than via slabs.

`MAX_CAPACITY_BITS` (default 11) is the number of bits required to represent the
count of objects in the largest slab.  This constant is used to pack both a ring
size and a pointer offset into a single pointer-sized word in remote messages.

`REMOTE_SLOT_BITS` (8) and `REMOTE_SLOTS` (256) define the hash table used by
the remote deallocation cache to route messages to their destination allocators.

`DEALLOC_BATCH_RING_SET_BITS` (3) and `DEALLOC_BATCH_RING_ASSOC` (2) together
determine the number of active batching rings in the remote deallocation cache.
The derived constant `DEALLOC_BATCH_RINGS = DEALLOC_BATCH_RING_ASSOC *
2^DEALLOC_BATCH_RING_SET_BITS` (default 16) is the total ring count; it is zero
when `DEALLOC_BATCH_RING_ASSOC` is zero, which disables same-destination batching.

`REMOTE_CACHE` (default `MIN_CHUNK_SIZE`) is the total deallocation byte budget
in the remote cache before it forces a flush to remote allocators.
`REMOTE_BATCH_LIMIT` (default 1 MiB) caps how much deallocation work one
processing pass performs.

`MIN_OBJECT_COUNT` (4, or 13 with the `random_larger_thresholds` mitigation) is
the minimum number of objects that a slab must be able to hold.

`CACHELINE_SIZE` (64 bytes) is used to pad structures that would otherwise suffer
false sharing.

---

## 3. Mitigation Flags (`ds_core/mitigations.h`)

snmalloc has a layered security model in which individual hardening measures can
be switched on or off at compile time.  Each measure is a named bit in a bitmask
of type `mitigation::type`.  The current set of measures is:

`random_pagemap`: randomise the pagemap's position within its OS allocation to
frustrate attempts to access it directly.

`random_larger_thresholds`: require more objects on each slab and a higher
fraction of free objects before a slab is considered available for reuse.
This raises the bar for use-after-reallocation attacks.

`freelist_forward_edge`: XOR-obfuscate every forward pointer in intra-slab free
lists.  A dangling-pointer write that redirects a `next` field will, after
decoding, produce a wild address that is almost certainly not in the allocator's
address range.

`freelist_backward_edge`: store obfuscated backward-edge signatures in every free
object.  Each signature is a non-linear function of the current object's address
and its successor's (already obfuscated) address.  This detects corruption of
either the forward or backward edge without requiring a traversal.  It is also
the mechanism that catches double-free.

`freelist_teardown_validate`: when a slab is de-purposed, walk its entire free
list one last time and validate the integrity of every link.

`random_initial`: when a slab's free list is first constructed, permute it with
Sattolo's algorithm so that the allocation order is uniformly random.

`random_preserve`: during operation, randomly assign each freed object to one of
two per-slab queues, and always allocate from the longer one.  This preserves
entropy as the allocator runs.

`random_open_slab`: when a size class has only a single active slab with free
objects, randomly decide whether to use it or to open a new slab instead.  This
prevents an adversary from predicting when a slab becomes exhausted.

`metadata_protection`: guard pagemap metadata with OS-level protections and store
it in a dedicated address range.

The combination `CHECK_CLIENT` (`SNMALLOC_CHECK_CLIENT` defined) enables all of
the above simultaneously except `random_pagemap`.

---

## 4. Architecture Abstraction Layer (`aal/`)

The AAL abstracts CPU-specific operations behind a uniform interface.  In C++
this is a set of static methods and constants on a type `AAL` selected at
compile time.  The Rust port should represent this as a trait.

**Pointer bounds** (`capptr_bound`, `capptr_rebound`, `capptr_size_round`).  On
conventional architectures these are no-ops or plain casts.  On CHERI they
shrink or redirect the hardware capability that carries authority to access
memory.  Every allocation boundary must pass through these operations so that the
Rust port is correct on CHERI without modification.

**Prefetch** (`prefetch`).  Issues a non-faulting cache prefetch hint before
traversing a free-list entry.  Omitting this on conventional hardware is safe but
measurably hurts throughput.

**Cycle counter** (`tick`).  Returns a monotonically increasing CPU-cycle
estimate.  Used by the optional ticker component to amortise per-allocation work
(such as draining the remote queue) over multiple allocations.

**Address width** (`address_bits`).  The number of significant bits in a virtual
address on the current platform.  Typically 48 on x86-64, 56 on ARMv8.5-A, 64
on RISC-V with Sv64 paging.  The Rust port must not hard-code 48.

**Feature flags** (`aal_features`).  A bitmask indicating `IntegerPointers`
(ordinary integer-castable pointers) and `StrictProvenance` (CHERI
capabilities).  Several conditional code paths depend on these flags.

---

## 5. Platform Abstraction Layer (`pal/`)

The PAL abstracts OS interactions.  Each platform provides a concrete type
satisfying the PAL concept.  The Rust port should represent this as a trait.

**`error(msg)`**: emit a diagnostic message and terminate the process
unconditionally.  No return.

**`notify_not_using(ptr, size)`**: advise the OS that a range of pages will not
be accessed for a while.  On Linux this issues `MADV_FREE` or `MADV_DONTNEED`.
On Windows it decommits the pages.  The allocator calls this when returning
chunks to the backend.

**`notify_using<zeroed>(ptr, size) -> bool`**: bring a range of pages back into
use.  When `zeroed` is true the function may use OS-provided background zeroing
(e.g. `mmap` with `MAP_POPULATE`) instead of explicit `memset`.  Returns true if
the pages are already zeroed.

**`zero<page_aligned>(ptr, size)`**: zero a range of memory.  When `page_aligned`
is true the implementation may use OS page-level zeroing.

**`reserve(size) -> ptr`** and **`reserve_aligned<size_t min_size>(size) -> ptr`**:
allocate virtual address space from the OS without necessarily committing it.
`reserve_aligned` guarantees that the returned address is a multiple of
`minimum_alloc_size`.  Platforms that cannot provide aligned reservation may
omit the second method and instead use overallocation with interior alignment.

**`get_entropy64() -> u64`**: return 64 bits of high-quality randomness.  Used to
seed the per-thread entropy source.  Provided only on platforms that expose
`getrandom`, `RtlGenRandom`, or equivalent.

**`page_size`**: the OS page size, as a compile-time constant.

**`pal_features`**: a bitmask indicating `Entropy`, `AlignedAllocation`,
`LowMemoryNotification`, `NoAllocation`, and similar optional capabilities.

---

## 6. Pointer Provenance Model (`ds_core/ptrwrap.h`)

snmalloc uses a typed-capability pointer model throughout its internals.  Every
pointer is annotated with a `capptr::bounds` type that describes the address
range the pointer is authorised to access.  This annotation is purely
compile-time on conventional architectures but becomes a hardware capability
on CHERI.

The principal bounds types used internally are:

`capptr::bounds::Arena`: authority to the entire OS-reserved arena.  Only the
backend holds such pointers.

`capptr::bounds::Chunk`: authority to a single chunk (one or more
`MIN_CHUNK_SIZE` blocks).  Backend components hold these.

`capptr::bounds::AllocFull`: authority to the entire slab (the allocation
object's chunk).

`capptr::bounds::Alloc`: authority bounded to the object itself.  This is what
the frontend handles after narrowing a chunk pointer.

`capptr::bounds::AllocWild`: a pointer that has passed through message-passing
and whose bounds have not yet been verified.  Must be domesticated before use.

**Domestication** is the act of verifying that an `AllocWild` pointer refers to
memory that snmalloc actually owns (via a pagemap lookup) and narrowing it to
`Alloc` bounds.  It is performed when draining the remote queue.

The Rust port should represent each distinct bounds level as a distinct type
(a newtype wrapper or a phantom-type-parameterised pointer), so that the type
checker enforces the same invariants that the C++ concept system does.

---

## 7. Size-Class System (`mem/sizeclasstable.h`)

All allocation sizes are mapped to a small integer called a size class before
any allocator state is consulted.

**Small size classes** cover sizes from `MIN_ALLOC_SIZE` up to
`MAX_SMALL_SIZECLASS_SIZE`.  The mapping uses an exponential-mantissa encoding:
within each power-of-two band `[2^k, 2^(k+1))`, there are `2^INTERMEDIATE_BITS`
equally spaced size classes.  With `INTERMEDIATE_BITS = 2`, the bands are
`[16,32)`, `[32,64)`, `[64,128)`, etc., each split into four classes.  A size is
converted to a small class by extracting its leading bit position and its next
`INTERMEDIATE_BITS` bits.  The total number of small size classes is a
compile-time constant (`NUM_SMALL_SIZECLASSES`, approximately 200 on a 64-bit
host with the defaults).

**Large size classes** cover sizes above `MAX_SMALL_SIZECLASS_SIZE`.  They are
indexed by the number of leading zeros in `(size - 1)`, so each large class
represents a single power-of-two.  There is one large class per bit of the
address space above `MAX_SMALL_SIZECLASS_BITS`.

The unified `sizeclass_t` type is a tagged union: a TAG bit distinguishes large
from small classes.  All pagemap lookups and slab metadata use this unified type.

**Per-size-class tables** are computed at initialisation (or compile time for
constexpr paths) and laid out as parallel arrays for cache-friendliness:

- `size`: the actual allocation size in bytes for this class.
- `slab_mask`: a bitmask for extracting the index within a slab from an
  allocation address.
- `div_mult`: a magic multiplier for fast reciprocal integer division, allowing
  `object_index = (offset * div_mult) >> 64` without a hardware divide.
- `mod_zero_mult`: a magic multiplier for fast divisibility testing.
- `capacity`: the number of objects that fit in a slab of this class.
- `waking`: the threshold of free objects at which a sleeping slab is re-queued.

**`size_to_sizeclass`** is the O(1) lookup that maps a raw size to a small size
class index.  An auxiliary compressed table (`SizeClassLookup`) stores the result
for all byte values up to `MAX_SMALL_SIZECLASS_SIZE` / `MIN_ALLOC_STEP_SIZE`,
allowing the mapping to be a single array access on the fast path.

**Slab sizing.**  For each small size class, the slab size is computed as the
smallest multiple of `MIN_CHUNK_SIZE` that holds at least `MIN_OBJECT_COUNT`
objects of that class.  Larger objects have proportionally larger slabs.  The
maximum span of a slab (`MAX_SLAB_SPAN_BITS`) bounds the bit-packing used in
remote messages.

**`is_start_of_object(sizeclass, address)`** verifies that an address is
correctly aligned to the start of an object in its size class, using the fast
divisibility test.  This is used on every deallocation to detect interior
pointers.

---

## 8. Core Data Structures (`ds_core/`, `ds/`, `ds_aal/`)

### 8.1 Entropy (`mem/entropy.h`)

Each thread maintains a `LocalEntropy` instance seeded from the PAL's
`get_entropy64`.  It provides:

- A 64-bit local key that is mixed into freelist XOR keys.
- A 1-bit coin-flip function used by the dual-queue randomisation.
- A stream of uniform random values used by Sattolo's permutation.

### 8.2 Free-List Object (`mem/freelist.h`)

Every unused allocation slot holds a `freelist::Object::T<BQueue>` overlay in
its first bytes.  The fields are:

`next_object` (or `atomic_next_object` for the MPSC queue variant): the encoded
forward pointer to the next free object.  When `freelist_forward_edge` is
enabled the pointer is XORed with `key.key2 ^ tweak` before storage; when
disabled it is stored as a raw integer.  On CHERI, the capability tags are
managed through special primitives rather than integer XOR.

`prev` (conditional, when `freelist_backward_edge` is enabled): an obfuscated
backward-edge signature.  If the current object is `c` and its decoded successor
is `n`, the stored value is `(c + key.key1) * (n + (key.key2 ^ tweak))`.  This
is verified every time `n` is popped from the list.

`FreeListKey` is a triple of 64-bit integers `(key0, key1, key2)`.  The
per-slab key is derived from `key_root = (0xdeadbeef, 0xbeefdead, 0xdeadbeef)`
XORed with a per-slab address-derived tweak (`as_key_tweak()` on the slab
metadata).

### 8.3 Free-List Iterator (`freelist::Iter`)

`freelist::Iter<BView, BQueue>` holds the current position in a free list
together with the key tweak needed to decode the next pointer.  Its `take`
method decodes the next pointer, verifies the backward-edge signature if
enabled, and returns the current object.  The iterator is stored in
`FastFreeLists` (see §9) and is the primary source for fast-path allocations.

### 8.4 Free-List Builder (`freelist::Builder`)

`freelist::Builder<RANDOM, TRACK_LENGTH, BView, BQueue>` constructs a free list
for a newly initialised slab.  When `RANDOM` is false it builds a single
singly-linked list.  When `RANDOM` is true (the `random_preserve` mitigation)
it maintains two independent lists and randomly assigns each added object to one
of them using the local entropy source.  The `close` method seals the builder
and returns an iterator pointing to the start of the longer list, discarding the
shorter one.

The builder is also used by the remote deallocation cache to accumulate objects
destined for the same slab before packaging them into a single `RemoteMessage`.

### 8.5 Sequential Set (`ds_aal/seqset.h`)

`SeqSet<T>` is a doubly-linked intrusive list that embeds its node (`SeqSet<T>::Node`)
directly in `T`.  All operations (push front, pop front, pop back, iterate)
are O(1).  It is used to maintain the set of slabs that have available
objects for a given size class (§9.4) and the set of full slabs.

### 8.6 Red-Black Tree (`ds_core/redblacktree.h`)

`RBTree<Rep>` is a generic self-balancing binary search tree parameterised by a
representation type `Rep` that abstracts over node storage.  `Rep` provides:
how to read and write a node, how to extract and set child pointers, how to read
and set the colour bit, and how to compare two keys.

The tree is used in two distinct ways:

- **In-place** (small buddy allocator): each free chunk stores the tree node
  within the chunk itself.  The right child pointer's low bit carries the colour.

- **Pagemap-resident** (large buddy allocator): tree nodes are stored in pagemap
  entries rather than in the allocated chunks.  This allows the backend to
  manage free chunks without touching user-visible memory.

Insertion, deletion, and search are all O(log n).  Rotations and recolouring
preserve the red-black invariants after each mutation.

### 8.7 ABA-Safe Compare-and-Swap (`ds/aba.h`)

`ABA<T>` wraps a pointer with a monotonically increasing version counter.
On x86-64 it uses a 128-bit compare-and-exchange (`CMPXCHG16B`); on other
platforms it falls back to a spin lock.  The version counter prevents the ABA
problem in lock-free stacks.

### 8.8 MPMC Stack (`ds/mpmcstack.h`)

`MPMCStack<T>` is a lock-free stack backed by `ABA<T>`.  Push and pop are both
O(1) and use CAS loops.  A bulk `pop_all` swaps the entire stack out atomically.
Objects must provide an `Atomic<T*>` field named `next` for intrusive linking.

### 8.9 Combining Lock (`ds/combininglock.h`)

`CombiningLock` is a flat-combining variant of an MCS queue lock.  When a
thread acquires the lock under contention it joins an MCS queue, submitting a
closure as the work to be performed.  The lock holder executes all queued work
items before releasing the lock.  Waiting threads spin briefly and then call
`WaitOnAddress` / `futex` to sleep.  The combining behaviour reduces context
switches by amortising lock overhead across many operations.

### 8.10 MPSC Free-List Queue (`mem/freelist_queue.h`)

`FreeListMPSCQ` is a non-blocking multi-producer, single-consumer queue built on
top of the `freelist::Object::T` mechanism and an `atomic_next_object` field.
It is used as the remote deallocation inbox (`RemoteAllocator::list`).  Producers
push entire chains atomically using a CAS on the tail; the consumer drains the
queue by reading the tail with `pop_all`-style extraction.

---

## 9. Frontend: Per-Thread Allocator (`mem/corealloc.h`, `mem/metadata.h`)

### 9.1 Overview

`Allocator<Config>` is the per-thread allocator state.  Every live thread has
exactly one instance, obtained from a pool (§12).  Its fields are:

`small_fast_free_lists[NUM_SMALL_SIZECLASSES]`: an array of `freelist::Iter`
values, one per small size class.  This is the first thing checked on every
small allocation.

`alloc_classes[NUM_SMALL_SIZECLASSES]`: per-class metadata caches, each a
`SeqSet<BackendSlabMetadata>` of slabs that have free objects.

`laden`: a `SeqSet<BackendSlabMetadata>` of slabs that are too full to be
allocated from (below the waking threshold).

`remote_dealloc_cache`: a `RemoteDeallocCache<Config>` that batches
deallocations bound for other threads.

`remote_alloc`: a `RemoteAllocator`, which is the MPSC inbox that other threads
push messages into when they free memory that this thread owns.

`backend_state`: a `LocalState` managing the thread's slice of the address-space
range chain.

`entropy`: a `LocalEntropy` instance.

`ticker`: a cycle-counter-based amortiser that triggers periodic housekeeping
(draining the remote queue) every N allocations instead of on every call.

### 9.2 Slab Metadata (`mem/metadata.h`)

`FrontendSlabMetadata<Backend, ClientMeta>` is stored in the metadata range (not
adjacent to the slab objects) and contains:

`free_queue` (`freelist::Builder`): the builder used to construct the slab's
initial free list and to accumulate freed objects during operation.  After the
initial build, `close()` produces the `freelist::Iter` that is cached in
`small_fast_free_lists`.

`needed_` (u16): a countdown.  It starts at the slab capacity.  Each allocation
decrements it; each deallocation increments it.  When it reaches zero after the
initial construction, all objects are in use and the slab moves to `laden`.
When it reaches the waking threshold after object returns, the slab wakes and is
re-added to `alloc_classes`.

`sleeping_` (bool): true when this slab has been moved to `laden` because too few
objects are free.

`large_` (bool): true for large allocations that reuse the slab metadata
structure rather than managing a true slab.

`client_meta_`: optional per-object application metadata (zero-size by default).

`node`: the intrusive `SeqSet::Node` for membership in `alloc_classes` or
`laden`.

### 9.3 Pagemap Entry (`mem/metadata.h`, `backend_helpers/defaultpagemapentry.h`)

`MetaEntryBase` is the fundamental pagemap entry, occupying exactly two
pointer-sized words per `MIN_CHUNK_SIZE` chunk of address space.

The first word (`meta`) holds:
- Bits `[N:1]`: a pointer to the `FrontendSlabMetadata` for the slab that owns
  this chunk (frontend case) or to a red-black tree node (backend case).
- Bit 0 (`META_BOUNDARY_BIT`): set if this chunk is the start of a PAL
  allocation and must not be merged with the preceding chunk.

The second word (`remote_and_sizeclass`) holds:
- Bits `[N:8]`: a pointer to the owning thread's `RemoteAllocator` (128-byte
  aligned, so bits 0–6 are free).
- Bit 7 (`REMOTE_BACKEND_MARKER`): set when this chunk is owned by the backend
  rather than by a frontend allocator.  The rest of the word's interpretation
  changes completely when this bit is set.
- Bits `[6:0]`: a `sizeclass_t` value encoding the size class of objects in this
  chunk (frontend case) or backend-specific data (backend case).

`FrontendMetaEntry<SlabMetadata>` inherits from `MetaEntryBase` and adds typed
accessors for extracting `SlabMetadata*` and `RemoteAllocator*` with the
appropriate ownership checks.

### 9.4 Allocation Path

**Fast path** (`small_alloc`): the allocator reads `small_fast_free_lists[sc]`
and calls `take(key, domesticate)`.  If the iterator is non-empty this is a
handful of instructions: decode the forward pointer, optionally verify the
backward edge, update the iterator, and return the pointer to the caller.  The
caller optionally zeros the memory.

**Refill path** (`small_refill`): when the fast free list is empty, the allocator
examines `alloc_classes[sc]`.  If a slab is available there it calls
`alloc_free_list()` on its `FrontendSlabMetadata`, which converts the slab's
`free_queue` builder into a new iterator and installs it into
`small_fast_free_lists[sc]`.

**Slow path** (`small_refill_slow`): when `alloc_classes[sc]` is also empty, the
allocator calls `Backend::alloc_chunk`, which traverses the range chain (§11)
to obtain a fresh `MIN_CHUNK_SIZE`-aligned chunk of address space, allocates a
`FrontendSlabMetadata` structure, writes pagemap entries for all `MIN_CHUNK_SIZE`
blocks in the slab, and returns both the chunk and the metadata pointer.  The
allocator then calls `initialise(sizeclass, chunk, key)` on the metadata, which
builds the complete initial free list via `freelist::Builder`.  If the
`random_initial` mitigation is enabled, Sattolo's algorithm is applied to the
builder at this point.

**Large allocation** (`alloc_not_small`): sizes above `MAX_SMALL_SIZECLASS_SIZE`
bypass the slab machinery entirely.  The allocator asks the backend for a chunk
exactly as large as the rounded-up request.  The backend writes a pagemap entry
with a large size class.  A minimal `FrontendSlabMetadata` is still allocated
and initialised with `initialise_large`, so that the deallocation path can be
uniform.

### 9.5 Deallocation Path

Every deallocation starts by reading the pagemap entry for the pointer being
freed.  The pagemap lookup uses integer division by `MIN_CHUNK_SIZE` to compute
the index.

**Local deallocation** (`dealloc_local_object`): if the entry's `RemoteAllocator`
pointer matches the current thread's `remote_alloc`, the object is local.  The
allocator calls the slab metadata's `free_queue.add(object)`, which appends the
object to the appropriate list in the builder (with coin-flip randomisation if
`random_preserve` is enabled).  It then decrements `needed_`; if `needed_`
reaches zero the slab has become empty and can be returned to the backend.

**Local deallocation, slow path** (`dealloc_local_object_slow`): invoked when
`needed_` passes the waking threshold.  If the slab was sleeping, it is removed
from `laden` and re-added to `alloc_classes[sc]`.  If the slab becomes
completely empty and the class has surplus slabs, the empty slab is returned to
the backend via `Backend::dealloc_chunk`.

**Remote deallocation** (`dealloc_remote`): if the pagemap entry's
`RemoteAllocator` pointer belongs to a different thread, the object is handed to
`remote_dealloc_cache.dealloc(entry, object)`.  The cache batches objects
destined for the same slab using a `freelist::Builder`.  When the cache is
full, `post()` is called, which packages each accumulated builder into a
`RemoteMessage` and enqueues it on the destination `RemoteAllocator`.

### 9.6 Remote Queue Drain (`handle_message_queue`)

The remote queue is checked on every allocation (either directly or via the
ticker).  The check is: if the `remote_alloc.list` tail has changed since the
last check, drain it.  This is a single atomic load on the fast path.

Draining calls `remote_alloc.dequeue`, which extracts the entire MPSC queue in
O(1) and then iterates over the resulting chain.  Each `RemoteMessage` is decoded
with `open_free_ring`, which reconstructs the ring of freed objects and returns
the ring size and its head.  The objects in the ring are processed by calling
`dealloc_local_object` on each one, as if they had been freed locally.

---

## 10. Remote Deallocation (`mem/remoteallocator.h`, `mem/remotecache.h`)

### 10.1 `RemoteAllocator`

Each `Allocator` owns a `RemoteAllocator` (either embedded inline or pointed to
externally when `IsQueueInline = false`).  Its only data member is `list`, a
`FreeListMPSCQ`.

Other threads push `RemoteMessage` objects into `list` using the lock-free
multi-producer interface.  The owning thread drains `list` exclusively.

### 10.2 `BatchedRemoteMessage`

A `BatchedRemoteMessage` packages a ring of free objects together with linkage
for the message queue itself, all within a single allocation-sized object.  It
contains two `freelist::Object::T<>` fields:

`free_ring`: serves as both the tail of the ring and a container for the encoded
ring head pointer and ring size.  The `next_object` field of `free_ring` stores
`(relative_offset_to_head << MAX_CAPACITY_BITS) | ring_size` rather than a raw
pointer.  The relative offset is the signed byte distance from the `BatchedRemoteMessage`
to the first object in the ring, divided to fit within the available bits.

`message_link`: the linkage for the MPSC message queue.

When the `DEALLOC_BATCH_RINGS` constant (`DEALLOC_BATCH_RING_ASSOC *
2^DEALLOC_BATCH_RING_SET_BITS`) is zero (no batching), the simpler
`SingletonRemoteMessage` is used instead.  It wraps a single freed object and
has only a `message_link` field; the ring size is implicitly one.

### 10.3 `RemoteDeallocCache`

Each `Allocator` contains a `RemoteDeallocCache<Config>`, which accumulates
remote deallocations before flushing them.

The cache's primary structure is an array of `freelist::Builder` instances
indexed by a hash of the destination slab metadata address.  There are
`DEALLOC_BATCH_RINGS` (= `DEALLOC_BATCH_RING_ASSOC * 2^DEALLOC_BATCH_RING_SET_BITS`,
default 16) such builders.  The hash is computed as
`(meta->as_key_tweak() * 0x7EFB352D) >> 16`, keeping the low
`DEALLOC_BATCH_RING_SET_BITS` bits.  Each hash bucket has `DEALLOC_BATCH_RING_ASSOC`
ways; when all ways are occupied the least-used one is evicted and converted
to a `RemoteMessage`.

A separate `used` counter tracks total bytes accumulated.  When `used` exceeds
`REMOTE_CACHE` the entire cache is flushed by calling `post`, which closes every
open builder, constructs a `RemoteMessage` for each non-empty builder, and
enqueues each message on its destination `RemoteAllocator`.

Flushing also handles the case where the current thread is the destination
(self-deallocation to a thread-local slab that another slab holds): `post` uses
a two-round scheme with shifted bit masks to detect and process self-addressed
messages without deadlock.

---

## 11. Backend: Address-Space Range Chain

The backend is a composable chain of *range* types.  Each range manages a
contiguous span of virtual address space and forwards requests it cannot satisfy
to its parent.  Ranges are composed at compile time using the `Pipe<A, B, C, ...>`
type alias, which constructs `C::Type<B::Type<A>>`.

A range must implement:
- `alloc_range(size) -> ptr`: allocate a power-of-two-sized, power-of-two-aligned
  block of at least `size` bytes.
- `dealloc_range(ptr, size)`: return a block.
- `alloc_range_with_leftover(size) -> ptr`: allocate, returning excess to the
  range itself.

### 11.1 `PalRange`

The leaf of the chain.  It calls `PAL::reserve_aligned(size)` (or
`PAL::reserve(size)` and aligns internally) to obtain virtual address space
directly from the OS.  All higher ranges are served from what `PalRange`
provides.

### 11.2 `PagemapRegisterRange`

Wraps any parent range.  On allocation, after the parent returns a pointer, it
calls `Pagemap::register_range(ptr, size)`, which ensures that pagemap entries
exist for all `MIN_CHUNK_SIZE` chunks in the range.  It also sets
`META_BOUNDARY_BIT` for the first chunk in each OS allocation to prevent merging
across OS-level boundaries.  This is needed on Windows (cannot commit across
separate `VirtualAlloc` calls) and on CHERI (capabilities cannot span
independent OS allocations).

### 11.3 `LargeBuddyRange`

`LargeBuddyRange<cache_size_bits, max_bits, Pagemap, min_bits>` is a buddy
allocator for large blocks.  All free blocks are stored in a red-black tree
(`RBTree<BuddyChunkRep>`) whose nodes are embedded in pagemap entries rather
than in the blocks themselves.  This is safe because the blocks are not yet in
use by the frontend.

When an allocation arrives the tree is searched for the smallest block of the
required size.  If none is found, `refill` is called, which requests a block
from the parent.  The refill size starts small and doubles towards `REFILL_SIZE`
(typically 2 MiB for the thread-local level, 16 MiB for the global level) as
more memory is needed, amortising PAL calls over many requests.  When a block
larger than required is obtained, the excess is added back to the tree.

When a block is freed it is inserted into the tree.  If the buddy block (the
block at the same level whose address differs only in the bit at position
`log2(size)`) is already in the tree and the two blocks do not span a PAL
allocation boundary (`META_BOUNDARY_BIT` not set), they are merged into a larger
block and the process repeats upward.

### 11.4 `SmallBuddyRange`

`SmallBuddyRange` handles sub-`MIN_CHUNK_SIZE` allocations (used for slab
metadata).  Its red-black tree nodes are stored inside the free blocks
themselves (`BuddyInplaceRep` with the colour in the low bit of the right child
pointer).  It is refilled from its parent in units of `MIN_CHUNK_SIZE`.  Blocks
smaller than `MIN_CHUNK_SIZE` are broken down to powers of two and entered into
the in-place tree.

### 11.5 `CommitRange`

A pass-through range that, after obtaining a block from its parent, calls
`PAL::notify_using` on it to ensure the pages are committed.  When blocks are
returned it calls `PAL::notify_not_using` to decommit them.

### 11.6 `GlobalRange`

A pass-through range that acquires a global lock (using `CombiningLock`) around
any request forwarded to its parent.  This is the boundary between per-thread
and globally shared state.

### 11.7 `StatsRange`

A pass-through range that accumulates allocation and deallocation counters for
`get_current_usage()` and `get_peak_usage()`.

### 11.8 Standard Range Chain Assembly

The default configuration (`StandardLocalState`) composes the chain as:

```
PalRange<Pal>
  -> PagemapRegisterRange<Pagemap>
  -> PagemapRegisterRange<Authmap>
  -> LargeBuddyRange<GlobalCacheSizeBits, bits::BITS-1, Pagemap, MinSizeBits>
       (global; holds up to GLOBAL_CACHE_SIZE; refills at 16 MiB)
  -> LogRange (optional debug logging)
  -> GlobalRange (acquires global lock for all above)
  -> CommitRange<Pal>
  -> StatsRange
  -> LargeBuddyRange<LocalCacheSizeBits, LocalCacheSizeBits, Pagemap, page_size_bits>
       (thread-local; caches up to 2 MiB; refills at 2 MiB)
  -> SmallBuddyRange
       (thread-local; serves sub-MIN_CHUNK_SIZE metadata requests)
```

The thread-local large buddy allocator is disabled for small fixed heaps (the
`OpenEnclave` scenario).

### 11.9 `BackendAllocator` (`backend/backend.h`)

`BackendAllocator<Pal, PagemapEntry, Pagemap, Authmap, LocalState>` is the
stateless bridge between the frontend allocator and the range chain.  It
provides:

`alloc_chunk(local_state, size, sizeclass)`: allocates a slab metadata structure
(via `local_state.get_meta_range()`) and the object chunk (via
`local_state.get_object_range()`).  It writes `FrontendMetaEntry` values into
the pagemap for every `MIN_CHUNK_SIZE` block in the chunk, recording the owning
`RemoteAllocator` pointer and the size class.

`dealloc_chunk(local_state, slab_metadata, chunk, size, sizeclass)`: marks all
pagemap entries in the chunk as backend-owned, then returns the metadata memory
and the chunk memory to their respective ranges.

`get_metaentry(address)`: direct O(1) pagemap read, used on every deallocation
to identify the owning allocator and size class.

---

## 12. Pagemap (`backend_helpers/pagemap.h`)

`BasicPagemap<Pal, ConcreteMap, PagemapEntry, fixed_range>` is a generic
wrapper over a concrete pagemap implementation.  The concrete map is either:

- `FlatPagemap`: a large flat array indexed by `address >> MIN_CHUNK_BITS`.  The
  array is lazily registered with the OS via `PagemapRegisterRange`.

- A hierarchical structure (for architectures with extremely sparse address
  spaces).

The pagemap is a global, singleton data structure.  Reads are unsynchronised
(they rely on the fact that pagemap entries are set before the corresponding
memory is made available to any allocator).  Writes during `register_range` use
`memset`-style bulk zeroing followed by per-entry writes, both of which are
naturally atomic on word-aligned accesses.

`set_metaentry(address, size, entry)`: set every pagemap slot in the range
`[address, address + size)` to `entry`.  The stride is `MIN_CHUNK_SIZE`.

`get_metaentry(address)`: returns the entry for the chunk containing `address`.
The `potentially_out_of_range` variant returns a zeroed entry rather than
faulting if the address lies outside the pagemap's committed region.

---

## 13. Thread Lifecycle (`global/threadalloc.h`, `mem/pool.h`)

### 13.1 Allocator Pool

`Pool<Allocator<Config>>` is a global concurrent pool of `Allocator` objects.
It maintains:

- A `PoolState` with a linked list of free allocators (protected by a `FlagWord`
  spin lock) and a separate list of all ever-allocated allocators (for iteration
  during teardown).

`acquire()`: dequeue a free allocator, or construct a new one via `new` if the
free list is empty.

`release(alloc)`: flush the allocator's remote cache, move all remaining live
slabs back to the backend, and enqueue the allocator on the free list.  The
allocator's `RemoteAllocator` inbox is drained before the flush to ensure no
messages are lost.

### 13.2 Thread-Local Allocator Access

`ThreadAlloc::get()` returns a pointer to the current thread's allocator.  On
the fast path this is a single thread-local load.  On the slow path (first call
per thread) it calls `Pool<Allocator>::acquire()` and stores the result.

Five teardown strategies are supported:

- `CheckInitPthread`: registers a `pthread_key` destructor to flush the allocator
  when the thread exits.  Also registers an `atexit` handler for the main thread.

- `CheckInitCXX`: uses a C++11 thread-local `OnDestruct` wrapper whose destructor
  is called automatically.

- `CheckInitCXXAtExitDirect`: uses `__cxa_thread_atexit_impl` for minimal
  runtime dependency.

- `CheckInitThreadCleanup`: delegates to the platform's built-in cleanup
  mechanism.

- `ThreadAllocExternal`: the application manages thread creation and destruction
  explicitly.

When a thread exits, its allocator is returned to the pool via `release`.  Any
local slab objects that are still free at that point are returned to the backend.
Objects that are still live (in use by the application) remain in the pagemap and
will be correctly identified when freed by any thread.

---

## 14. Security Features in Detail

### 14.1 Freelist Corruption Detection

The forward pointer `next` is stored as `raw_pointer XOR (key.key2 XOR tweak)`
when `freelist_forward_edge` is enabled.  A write through a dangling pointer that
overwrites `next` will, after decoding, produce an address outside any valid
slab.  The pagemap lookup on that address will then either fault (if unmapped) or
return a zero entry, which the allocator treats as a fatal error.

The backward-edge signature `signed_prev(c, n, key, tweak) = (c + key.key1) * (n + (key.key2 XOR tweak))`
is a bivariate polynomial evaluated over the integers modulo `2^64`.  For an
adversary who does not know the key, predicting a valid signature for a forged
`(c, n)` pair requires solving a modular polynomial equation, which is
computationally infeasible.  The verification on every `take()` catches
corruption with high probability.

Double-free is detected because the second free of the same object stores a new
backward-edge signature into the just-freed object's `prev` field, overwriting
the signature written by the first free.  When the list is later consumed, the
signature check fails.

### 14.2 Randomised Free Lists

Sattolo's algorithm constructs a cyclic permutation of all objects in the slab
by maintaining a partial permutation and swapping each new element into a
uniformly random position within the already-constructed portion.  This guarantees
that the resulting list visits every object exactly once and that the access
order is uniformly distributed over all `(capacity - 1)!` possible permutations.

The dual-queue scheme (`random_preserve`) adds entropy continuously.  Each freed
object is assigned to queue 0 or queue 1 with probability 1/2, using the local
entropy source.  Allocation always draws from the longer queue.  The result is
that even after many allocation-deallocation cycles, an adversary cannot predict
which of two candidate objects will be returned next.

### 14.3 Metadata Separation

All `FrontendSlabMetadata` objects are allocated from a dedicated metadata range
that is entirely separate from the user-data range.  Guard pages can be placed
around the metadata range (the `metadata_protection` mitigation).  Because slab
metadata is never adjacent to user data, a linear overflow in user memory cannot
reach the metadata that governs that slab.

---

## 15. End-to-End Allocation Walk-Through

This section traces a single small allocation on an otherwise idle 64-bit Linux
system, starting from `malloc(100)`.

1. The C library wrapper rounds the request to the next size class (128 bytes,
   `sizeclass = 6` in the default table) and calls `Allocator::alloc(128)`.

2. `alloc` calls `small_alloc<Uninit, CheckInit>(128)`, which reads
   `small_fast_free_lists[6]`.  Since this is the first allocation, the iterator
   is empty.

3. `small_refill` examines `alloc_classes[6]`.  It is empty.

4. `small_refill_slow` calls `Backend::alloc_chunk(local_state, 16384, 6)`.

5. `alloc_chunk` calls `local_state.get_meta_range().alloc_range(sizeof(FrontendSlabMetadata))`.
   The `SmallBuddyRange` is empty so it asks its parent `LargeObjectRange`
   for `MIN_CHUNK_SIZE` bytes.

6. The thread-local `LargeBuddyRange` is empty and requests a refill.  The
   request passes up through `StatsRange -> CommitRange -> GlobalRange`.
   `GlobalRange` acquires the combining lock and forwards to `LogRange`, which
   forwards to the global `LargeBuddyRange`.

7. The global `LargeBuddyRange` is empty and asks its parent `Base`, which
   is composed as `PagemapRegisterRange<Authmap> -> PagemapRegisterRange<Pagemap>
   -> PalRange`.  `PalRange` calls `mmap(NULL, 16 MiB, ...)`;
   `PagemapRegisterRange<Pagemap>` ensures 16 MiB worth of pagemap entries are
   committed; `PagemapRegisterRange<Authmap>` does the same for the authmap.

8. The global buddy splits the 16 MiB region into 8 MiB + 4 MiB + 2 MiB and
   returns one 2 MiB block (or the smallest requested size, whichever matches
   first).  `GlobalRange` releases the lock.  `CommitRange` calls
   `madvise(MADV_WILLNEED)` or equivalent.

9. The thread-local `LargeBuddyRange` receives 2 MiB, keeps 2 MiB - 16 KiB,
   and passes 16 KiB back down the chain to `SmallBuddyRange`.

10. `SmallBuddyRange` breaks 16 KiB into sub-powers-of-two, satisfies the
    metadata allocation, and returns the metadata pointer.

11. Step 5 also calls `local_state.get_object_range().alloc_range(16384)` for
    the object slab.  The thread-local large buddy already has memory after
    step 9 and returns 16 KiB directly.

12. `alloc_chunk` writes `FrontendMetaEntry` values into the pagemap for the
    single `MIN_CHUNK_SIZE` block in this slab, recording `&thread_remote_alloc`
    and `sizeclass = 6`.

13. Back in `small_refill_slow`, `FrontendSlabMetadata::initialise(6, slab, key)`
    is called.  This constructs a `freelist::Builder` over the 128 objects in
    the 16 KiB slab (16384 / 128 = 128 objects).  If `random_initial` is enabled,
    Sattolo's algorithm permutes the list.  `close()` seals the builder and
    returns an iterator.

14. The iterator is stored in `small_fast_free_lists[6]`.

15. Back in `small_alloc`, `take(key, domesticate)` is called on the fresh
    iterator.  It decodes the first pointer, optionally verifies the backward
    edge, and returns the first free object.

16. `finish_alloc<Uninit>` calls `capptr_reveal` to strip the capability bounds
    to a plain pointer and returns it to the caller.

---

## 16. End-to-End Deallocation Walk-Through

This section traces `free(p)` where `p` was allocated by the current thread and
belongs to a 128-byte small size class slab.

1. `Allocator::dealloc(p)` calls `Backend::get_metaentry(address_cast(p))`.
   This performs `pagemap[address >> MIN_CHUNK_BITS]` in O(1).

2. The entry's `is_owned()` check passes (it is a frontend entry).
   `get_remote()` returns a pointer to the current thread's `RemoteAllocator`
   because `p` was allocated here.

3. Since the remote allocator matches `&remote_alloc`, `dealloc_local_object`
   is called.

4. `dealloc_local_object` calls `entry.get_slab_metadata()` to obtain the
   `FrontendSlabMetadata*`, then calls `meta->free_queue.add(object, entropy)`,
   which appends `p` to one of the builder's lists (coin-flip selects which one
   if `random_preserve` is enabled).

5. `meta->return_object()` decrements `needed_` and checks whether it has
   crossed the waking threshold.  If the slab was sleeping and now has enough
   free objects, `dealloc_local_object_slow` re-adds it to `alloc_classes[6]`.
   If `needed_` reaches the capacity (all objects free) and the class has
   surplus slabs, the slab is returned to `Backend::dealloc_chunk`.

---

## 17. Equivalence Verification Strategy

The Rust port must be functionally equivalent to the C++ original.  The following
approaches should be used together.

### 17.1 Property-Based Testing

Use a property-based testing framework (such as `proptest` or `bolero`) to
generate sequences of `malloc`, `free`, `realloc`, and `calloc` calls.  Apply
the same sequence to both implementations on the same thread schedule.  Assert
that:

- Both implementations return non-null results for every successful allocation.
- Neither implementation returns the same address twice for live allocations
  (no double-issue).
- Every deallocation is accepted without panic or detected corruption.
- After all deallocations, both implementations report zero live bytes.

### 17.2 Allocation Pattern Benchmarks

Replicate the benchmark suite from the snmalloc repository (or the `mimalloc-bench`
suite that the snmalloc paper uses) under both implementations and compare
throughput figures.  Performance should be within a small percentage (less than
5%) of the C++ version on all benchmarks, for both single-threaded and
highly-concurrent workloads.

### 17.3 Size-Class Table Equivalence

Generate the full `SizeClassTable` independently in both C++ and Rust using the
same constants and assert that every row (`size`, `slab_mask`, `div_mult`,
`mod_zero_mult`, `capacity`, `waking`) is identical.  Run this as a
compile-time or test-time assertion.

### 17.4 Freelist Integrity Testing

Enable all freelist mitigations in both implementations simultaneously.  Run the
property-based tests and assert that no corruption is detected on any well-formed
sequence.  Separately, inject known corruptions (overwrite a `next` field after
freeing, double-free an object) and assert that both implementations detect and
report the error in the same way.

### 17.5 Remote Deallocation Stress Testing

Run a multi-threaded stress test where every deallocation is performed on a
different thread from the allocation.  Verify:

- No memory is lost (all freed bytes eventually appear as available for
  re-allocation).
- The MPSC queue drains correctly after each burst of remote frees.
- Remote message batching produces the same message boundaries on both
  implementations for a given input sequence and entropy seed.

### 17.6 Backend Range Chain Testing

Test the range chain in isolation by:

- Allocating a sequence of varied sizes and verifying that each returned block is
  non-overlapping and correctly aligned.
- Returning blocks in a different order from allocation and verifying that the
  buddy allocator merges them correctly (the free-list state matches the C++
  original for the same input sequence).
- Verifying pagemap entries before and after each `alloc_chunk` and
  `dealloc_chunk` call.

### 17.7 Deterministic Replay

For a fixed entropy seed and thread schedule, the C++ and Rust implementations
must produce identical sequences of addresses for identical sequences of
allocation/deallocation calls.  This can be verified by instrumenting both
implementations to log `(operation, size, returned_address)` tuples and
diffing the logs.  The entropy seed must be the same (pass it explicitly rather
than reading from the OS), and the thread-local state must be initialised from
that seed in the same order.

---

## 18. Crate Structure Recommendation

The Rust port should be organised as a single Cargo workspace with the following
crates:

`snmalloc-pal`: The Platform Abstraction Layer trait and implementations for
Linux (using `libc`), macOS, Windows, and OpenEnclave.  No dependency on any
other crate in this workspace.

`snmalloc-aal`: The Architecture Abstraction Layer trait and implementations for
x86-64, AArch64, RISC-V, and CHERI.  No dependency on any other crate.

`snmalloc-core`: All data structures and algorithms: freelist, red-black tree,
ABA, MPMC stack, combining lock, size-class table, slab metadata, pagemap.
Depends on `snmalloc-pal` and `snmalloc-aal` by trait, not by concrete type.

`snmalloc-backend`: The range chain components (buddy allocators, page-map
registration, commit range, global range, stats range) and `BackendAllocator`.
Depends on `snmalloc-core`.

`snmalloc-frontend`: `Allocator`, `ThreadAlloc`, `Pool`, and `RemoteDeallocCache`.
Depends on `snmalloc-backend`.

`snmalloc`: The public-facing crate.  Selects concrete PAL and AAL types, wires
them into a `StandardConfig`, provides the global allocator implementation
(`#[global_allocator]`), and re-exports the public API.  Depends on all of the
above.

`snmalloc-test`: Integration and stress tests.  Depends on `snmalloc` and any
Rust testing frameworks needed for property-based testing.
