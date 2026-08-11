# `jadren-core` 0.1.0

The first standard-library package for Jadren. It is deliberately small and
platform-neutral: application code can use it on Windows, Linux and Android
without importing a platform runtime.

## Modules

- `jadren.core.math` – checked scalar helpers such as `clamp` and `abs`.
- `jadren.core.collections` – borrowed `Slice<T>` helpers and checked
  `Buffer<Int32>` sum/update operations for dynamic application lists.
- `jadren.core.io` – explicit caller-owned stdin, stdout and stderr byte
  streams for console programs.
- `jadren.core.process` – bounded UTF-8 access to native process arguments;
  index `0` is the executable path and all output storage remains
  caller-owned.
- `jadren.core.time` – wall-clock Unix seconds, monotonic milliseconds and
  explicit UTC/fixed-offset calendar decomposition.
- `jadren.core.status` – stable `CoreStatus` values plus named `BufferStatus`
  mapping for recoverable buffer boundaries.

Generic owning buffers also expose `buffer_append_move` and
`buffer_append_move_status`. They move a compiler-approved move-safe `T` from
a caller-owned source slot to the end of `Buffer<T>` without copying ownership
descriptors. The source slot is zeroed only after success; the status form
returns `0` or a stable negative boundary/allocation code.
For an addressable move-safe source, the shorter `buffer_append` and
`buffer_insert` spellings select the same rollback-safe move ABI automatically;
copy-safe values continue to use the ordinary byte-copy ABI.

## Local verification

From the repository root:

```powershell
pwsh -File scripts/check-stdlib.ps1 -JadrenPath jadren
```

The example imports two public functions from `jadren.core.math`. The command
checks all package sources in one compiler session, so import resolution is
actually exercised rather than only checking each file in isolation.
