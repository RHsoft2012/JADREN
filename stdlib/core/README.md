# `jadren-core` 0.1.0

The first standard-library package for Jadren. It is deliberately small and
platform-neutral: application code can use it on Windows, Linux and Android
without importing a platform runtime.

## Modules

- `jadren.core.math` – checked scalar helpers such as `clamp` and `abs`.
- `jadren.core.collections` – borrowed `Slice<T>` helpers and checked
  `Buffer<Int32>` sum/update/query operations (`find_buffer_i32`,
  `count_buffer_i32` and `sort_buffer_i32`) plus generic bounded index/find/count/contains/any/none/all/retain,
  numeric sort and ordered lower-bound query,
  `create`, `append`, `append_status`, `append_grow`, `append_grow_status`,
  `append_move`, `append_move_status`, `append_move_grow`,
  `append_move_grow_status`,
  `insert_grow`, `insert_grow_status`, `insert_move_from`,
  `insert_move_from_status`, `insert_move_from_grow`,
  `insert_move_from_grow_status`, `length`, `is_empty`,
  `last_index`, `copy_at`, `copy_at_status`, `capacity`,
  `reserve`, `reserve_status`, `insert`, `insert_status`, `remove_copy`,
  `remove_copy_status`, `pop_copy`, `pop_copy_status`, `resize`, `resize_status`, `clear`,
  `clear_status`, `resize_move`, `resize_move_status`, `clear_move`,
  `clear_move_status`, `pop_drop`, `pop_drop_status`, `remove_drop` and
  `remove_drop_status`, `remove_move_into`, `remove_move_into_status`,
  `pop`, `remove`, `pop_move_into` and `pop_move_into_status` helpers for dynamic application
  lists and owning nominal values. Move-aware clear/drop validates
  the concrete destructor layout after generic package materialization; empty
  status operations retain the stable `-20` boundary code.
- `jadren.core.io` – explicit caller-owned stdin, stdout and stderr byte
  streams for console programs; `read_stdin_buffer`,
  `write_stdout_buffer` and `write_stderr_buffer` provide checked dynamic
  `Buffer<UInt8>` facades without changing the native ABI.
- `jadren.core.process` – bounded UTF-8 access to native process arguments;
  index `0` is the executable path and all output storage remains
  caller-owned. `arg_read_buffer` adds an explicit-capacity
  `Buffer<UInt8>` facade without changing the native ABI.
- `jadren.core.time` – wall-clock Unix seconds, monotonic milliseconds and
  explicit UTC/fixed-offset calendar decomposition; the `_buffer` variants
  write the six calendar fields into caller-owned dynamic buffers.
- `jadren.core.status` – stable `CoreStatus` values plus named `BufferStatus`
  mapping for recoverable buffer boundaries.

Generic owning buffers also expose `buffer_append_move` and
`buffer_append_move_status`. They move a compiler-approved move-safe `T` from
a caller-owned source slot to the end of `Buffer<T>` without copying ownership
descriptors. The source slot is zeroed only after success; the status form
returns `0` or a stable negative boundary/allocation code.

The `append_grow<T>` and `append_move_grow<T>` facades reserve one additional
slot before appending, so callers do not need a separate capacity branch for
dynamic list/table rows. Their status variants preserve the existing `-13`
size-overflow and allocator status boundary; no hidden worker or allocator
policy is introduced.

The `insert_grow<T>` and `insert_move_from_grow<T>` facades apply the same
bounded reserve-before-mutation rule to list/table insertion. The insertion
index is validated before any reserve, so an invalid index returns `false` or
`-20` without changing the descriptor; representational size overflow returns
`-13` from the status variants. Move-aware insertion transfers ownership only
after the reserve succeeds.

The package fixture `examples/generic-buffer-growth-nominal-project` also
covers materialization of `create<T>` and both move-growth facades for a
generic `@repr(C) Row<OwnedString>`. Reallocation preserves the record order
and owned UTF-8 payloads before the destructor-aware clear; this is a bounded
native smoke, not a claim about arbitrary layouts or allocator policy.

The `count_buffer_i32` query scans only initialized elements from the supplied
 start index and returns a bounded match count. An empty range or a start past
 the logical length returns zero; the helper is read-only, `@noalloc`, and does
 not expose the backing pointer.

The `sort_buffer_i32` helper applies a bounded insertion sort in place. The
`descending` flag selects descending order; the helper uses one scalar
temporary, preserves logical length/capacity, and is marked `@noalloc`.

The generic `sort<T: Ordered>` facade applies a bounded insertion sort to
ordered `Buffer<T>` materializations. Numeric `Int32`/`Float32` remain covered
by `generic-buffer-sort-numeric-project`, while the ordered `Char` path is
covered by `generic-buffer-sort-ordered-project`; the helper uses one scalar
temporary and preserves logical length/capacity without a new runtime ABI or
hidden allocation.

The generic `capacity<T>` facade requires a mutable `write Buffer<T>` because a
read-only view intentionally carries only pointer/length and cannot expose the
backing allocation capacity safely. The generic `lower_bound<T: Ordered>` and
`upper_bound<T: Ordered>` facades perform bounded binary searches over an
ascending `Buffer<T>`. `lower_bound` returns the first insertion position,
while `upper_bound` returns the first position strictly after duplicate
values. Both only read the initialized view, perform no allocation or mutation,
and are covered for `Int32` and `Float64` by the standalone
`generic-buffer-lower-bound-project` and `generic-buffer-upper-bound-project`
fixtures.

The generic `binary_search<T: Ordered>` facade returns the first matching
index in an ascending buffer or `-1` when the value is absent. It uses the
same bounded, allocation-free read-only contract and is covered for `Int32`
and `Float64` by the standalone `generic-buffer-binary-search-project`
fixture.

The generic `min_index<T: Ordered>` and `max_index<T: Ordered>` facades scan
the initialized prefix and return the first index of the smallest or largest
value, or `-1` for an empty buffer. They preserve length/capacity, perform no
allocation or mutation, and are covered for `Int32`, `Float64`, and `Char` by
the standalone `generic-buffer-extrema-project` fixture.

The generic `copy_at<T: Equatable>` and `copy_at_status<T: Equatable>` facades
copy one initialized element into the first slot of a caller-owned one-slot
`Buffer<T>`. They never resize or mutate the source; invalid source indices
return `false`/`-20`, while an empty output returns `false`/`-21`. The
`generic-buffer-copy-at-project` fixture covers `Int32`, `Bool`, status guards,
and source/output length and capacity preservation.

The generic `reverse<T: Ordered>` facade swaps the initialized prefix in place
using bounded scalar temporaries. It preserves logical length and capacity,
performs no allocation, and is covered for `Int32`, `Float64` and `Char` by the
standalone `generic-buffer-reverse-project` fixture.

The generic `find<T: Equatable>`, `find_last<T: Equatable>`,
`count<T: Equatable>`, `contains<T: Equatable>`, `any<T: Equatable>`, `none<T: Equatable>` and
`all<T: Equatable>` helpers perform bounded
queries for scalar `Buffer<T>` instantiations. They start at the
caller-selected index and never mutate or reallocate the buffer; `find`
returns `-1`, `count` returns the number of matches, `contains` returns a
boolean presence result, `any` and `none` return presence and absence results,
and `all` returns whether the entire suffix equals the needle. `find_last`
returns the final matching index in the selected suffix. An empty or
out-of-range suffix is a successful vacuous match for `none` and `all`.
`Int32` and `Bool` materializations are covered by the standalone
`generic-buffer-find-project`, `generic-buffer-find-last-project`,
`generic-buffer-all-project` and
`generic-buffer-any-project` fixtures.

The generic `retain<T: Equatable>` facade compacts matching initialized
elements in place while preserving their order. It only reduces the logical
length after the complete pass, keeps the allocation capacity unchanged, and
is covered for `Int32` and `Bool` by the standalone
`generic-buffer-retain-project` fixture.
The generic `filter<T: Equatable>` facade applies a caller-owned value predicate
while compacting matching copy-safe elements in place. It preserves order and
allocation capacity, publishes the shorter logical length after the bounded
pass, and is covered for `Int32`, `Float32`, `Bool`, and `Char` by the
standalone `generic-buffer-filter-project` fixture. The helper itself does not
allocate; effects or allocation performed by the caller's predicate remain the
predicate's explicit contract.
Concrete `filter_i32` and `filter_f32` spellings mirror the same bounded
contract when a package needs a fully concrete callback ABI for repeated or
mixed call sites.
The generic `fold<T: Numeric>` facade reduces the initialized prefix from left
to right with a caller-owned `fn(T, T) -> T` combiner. It keeps the buffer
descriptor, logical length and capacity unchanged, performs no allocation, and
is covered for `Int32` and `Float32` by the standalone
`generic-buffer-fold-project` fixture. Concrete `fold_i32` and `fold_f32`
spellings expose the same accumulator contract when a package needs a fully
concrete callback ABI.
The generic `map_in_place<T: Equatable>` facade transforms each initialized
element through a caller-owned `fn(T) -> T` callback, preserving logical
length and capacity without allocation. Concrete `map_i32` and `map_f32`
spellings provide the same in-place transform with a fully concrete callback
ABI. The standalone `generic-buffer-map-project` fixture covers repeated
`Int32` calls, `Float32`, `Bool`, `Char`, empty buffers and unchanged capacity.
For an addressable move-safe source, the shorter `buffer_append` and
`buffer_insert` spellings select the same rollback-safe move ABI automatically;
copy-safe values continue to use the ordinary byte-copy ABI.

The `pop<T>` facade returns the last move-safe element as
`Result<T, Int32>` and reports `-20` for an empty buffer. It reuses the
compiler-approved `buffer_pop` ABI after package materialization, so nested
owning buffers do not need a second runtime descriptor or allocator path.
The `remove<T>` facade uses the same result/ownership contract at a bounded
index and reports `-20` for an invalid index.

`truncate<T>` and `truncate_move<T>` are bounded shrink conveniences. They
never grow a buffer: a target at or above the current logical length is a
successful no-op. The copy-safe form delegates to `buffer_resize`, while the
move-aware form delegates to `buffer_resize_move` so removed owning elements
are released through the existing destructor path. Their status variants
return `0` for both a successful shrink and a no-op.

## Local verification

From the repository root:

```powershell
pwsh -File scripts/check-stdlib.ps1 -JadrenPath jadren
```

The example imports two public functions from `jadren.core.math`. The command
checks all package sources in one compiler session, so import resolution is
actually exercised rather than only checking each file in isolation.
