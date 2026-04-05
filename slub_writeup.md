# Linux SLUB Allocator Writeup

## 1. SLAB vs SLUB vs SLOB

The Linux kernel has had three slab allocator implementations:

**SLAB** was the original implementation based on Jeff Bonwick's 1994 paper. It uses per-CPU caches, per-node shared arrays, and three linked lists (full, partial, free) per cache. The metadata is stored separately from the slab pages. SLAB works well but carries significant complexity and memory overhead from maintaining all these queues.

**SLOB** is the simplest allocator, designed for embedded systems with very limited memory. It uses a single first-fit free list with no per-CPU caching. It has minimal overhead but poor performance under contention.

**SLUB** replaced SLAB as the default in Linux 2.6.23. It removes the per-CPU queues and shared arrays, storing the free list pointers directly inside free objects. This reduces metadata overhead, improves cache locality, and simplifies the code. SLUB is now the default allocator in mainline Linux.

## 2. SLUB Design Goals

SLUB was designed to address several issues with SLAB:

- Reduce memory overhead by eliminating separate metadata structures
- Improve cache locality by embedding free list pointers inside free objects
- Simplify the codebase (SLUB is roughly half the lines of code of SLAB)
- Better NUMA awareness with per-node partial lists
- Improve debugging capabilities with red zones, poisoning, and tracking
- Reduce lock contention with per-CPU slabs that require no locking on the fast path

## 3. Core Data Structures

### kmem_cache

The top-level structure representing a cache for one object size. Key fields:

- `cpu_slab`: pointer to per-CPU slab data
- `size`: actual object size including metadata
- `object_size`: requested object size
- `offset`: offset of the free pointer within the object
- `min_partial`: minimum number of partial slabs to keep per node
- `node`: array of per-node data (`kmem_cache_node`)

### kmem_cache_cpu

Per-CPU data, one per CPU per cache. This is the fast path structure:

- `freelist`: pointer to the next free object in the current slab
- `page`: pointer to the slab page currently being used on this CPU
- `tid`: transaction ID for cmpxchg-based lockless operations

The per-CPU slab is the key to SLUB performance. Allocations from the current CPU slab require no locking at all.

### kmem_cache_node

Per-NUMA-node data:

- `partial`: list of partially-used slabs for this node
- `nr_partial`: count of partial slabs
- `list_lock`: spinlock protecting the partial list

When the per-CPU slab is exhausted, SLUB picks a partial slab from the node list.

## 4. Allocation Path

### Fast path (no locking)

1. Read `cpu_slab->freelist` and `cpu_slab->tid`
2. If `freelist` is not NULL, use `cmpxchg` to atomically swap freelist to the next free object
3. Return the object

This is the common case and requires zero locks. The `tid` (transaction ID) prevents ABA problems with the cmpxchg.

### Slow path

If the per-CPU freelist is empty:

1. Check if the current page has more free objects (the page freelist)
2. If yes, move the page freelist to the CPU freelist and retry
3. If the current page is fully used, look for a partial slab on the current node
4. If a partial slab is found, make it the current CPU slab
5. If no partial slabs exist, allocate a new slab from the page allocator

Each step falls through to the next only if needed, so the fast case is extremely cheap.

## 5. Deallocation Path

### Fast path

1. If the object belongs to the current CPU slab, use `cmpxchg` to push it onto `cpu_slab->freelist`
2. No locking needed

### Slow path

If the object belongs to a different slab:

1. Find the slab page containing the object using `virt_to_head_page()`
2. Use `cmpxchg_double` to atomically add the object to that page's freelist
3. If the slab was previously full, add it to the node's partial list
4. If the slab is now completely empty and the node has enough partials, free the slab back to the page allocator

## 6. Free List Implementation

SLUB stores free list pointers directly inside free objects. When an object is free, its first bytes (at a configurable offset) contain a pointer to the next free object. When the object is allocated, the application data overwrites this pointer.

This is the same approach used in our minimal implementation. The difference is that SLUB adds optional hardening:

- **Freelist pointer encoding**: the pointer is XORed with a random value and the object address to prevent trivial overwrites (`CONFIG_SLAB_FREELIST_HARDENED`)
- **Freelist randomization**: objects within a new slab are not linked sequentially but in random order (`CONFIG_SLAB_FREELIST_RANDOM`)

## 7. Per-CPU Caches and NUMA Awareness

Each CPU has its own `kmem_cache_cpu` structure. The current slab assigned to a CPU is used exclusively by that CPU, so the fast path allocation and deallocation require no locking.

For NUMA systems, SLUB tries to allocate from memory local to the requesting CPU's NUMA node. Each node maintains its own partial slab list (`kmem_cache_node`). When a CPU needs a new slab, it first checks partial slabs on its local node before allocating new pages.

Remote frees (freeing an object allocated on a different node) go through the slow path and the object is returned to its original slab's freelist.

## 8. Security Implications

The SLUB allocator is a frequent target for kernel exploitation because its behavior is predictable and its metadata is inline with user data.

### Heap Overflow

Since objects are stored contiguously within a slab, overflowing one object writes into the adjacent object. If the adjacent object contains function pointers or security-sensitive data, the attacker can gain control. SLUB's sequential allocation pattern makes this predictable.

### Use-After-Free (UAF)

When an object is freed, its first bytes become a freelist pointer. If the application still holds a reference and reads/writes through it:

- Reading reveals the freelist pointer (heap address leak)
- Writing corrupts the freelist, allowing arbitrary allocation (write-what-where primitive)

A common exploit technique: free object A, trigger reallocation of the same slot with attacker-controlled data, then use the dangling reference to object A to read/write the new data.

### Freelist Poisoning

An attacker who can corrupt a freelist pointer (via overflow or UAF) can make the allocator return an arbitrary address on the next allocation. This gives a write-what-where primitive. The `CONFIG_SLAB_FREELIST_HARDENED` mitigation XORs the pointer with a random cookie, making blind overwrites crash rather than succeed.

### Cross-cache attacks

Objects of different types may share the same slab cache if they have the same size (cache merging). An attacker can free one type of object and have it reallocated as a different type, bypassing type-based mitigations. `CONFIG_SLAB_VIRTUAL` and `CONFIG_RANDOM_KMALLOC_CACHES` are newer mitigations against this.

### Mitigations summary

| Mitigation | What it does |
|---|---|
| `CONFIG_SLAB_FREELIST_HARDENED` | XOR-encodes freelist pointers |
| `CONFIG_SLAB_FREELIST_RANDOM` | Randomizes allocation order within a slab |
| `CONFIG_INIT_ON_FREE_DEFAULT_ON` | Zeroes objects on free |
| `CONFIG_INIT_ON_ALLOC_DEFAULT_ON` | Zeroes objects on allocation |
| `CONFIG_RANDOM_KMALLOC_CACHES` | Multiple copies of each size class, randomly selected |

## 9. Comparison with Our Minimal Implementation

| Feature | Linux SLUB | Our implementation |
|---|---|---|
| Free list | Inline, optionally encoded | Inline, plain pointers |
| Per-CPU caching | Yes, lockless fast path | No, single spinlock |
| NUMA awareness | Per-node partial lists | None |
| Size classes | Dynamic, based on kmalloc sizes | Fixed: 8 to 2048 power-of-two |
| Slab size | One or more contiguous pages | Single page (4096 bytes) |
| Page allocator | Buddy allocator | StaticPageProvider (bump allocator) |
| Thread safety | cmpxchg + per-CPU data | Spinlock around all operations |
| Hardening | Freelist encoding, randomization, poisoning | debug_assert only |
| Cache merging | Automatic for same-size caches | No merging |

Our allocator follows the same fundamental design (size classes, slabs divided into slots, embedded free list) but strips away everything related to performance at scale: per-CPU data, NUMA, lockless operations, and security hardening. This makes it suitable for understanding the core concepts and as a starting point for a real kernel allocator.
