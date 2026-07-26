# Jadren standard library and module system 0.1

## Scope

The standard library is layered above the compiler and below platform
integrations. `jadren-core` contains deterministic, platform-neutral contracts;
Windows, Linux, Android, Unity and GPU packages must depend on it rather than
duplicate scalar types or error conventions.

## Initial package

The canonical package is `stdlib/core`:

```toml
[package]
name = "jadren-core"
version = "0.1.0"
edition = "2026"
```

Its first modules are:

- `jadren.core.math` – scalar helpers with explicit fixed-width types;
- `jadren.core.status` – `CoreStatus` values for recoverable API/FFI boundaries.
- `jadren.core.option` – typed `Option<Int32>` constructors and explicit
  fallback matching;
- `jadren.core.outcome` – typed `Result<Int32, CoreError>` helpers without
  exceptions;
- `jadren.core.collections` – allocation-free `read Slice<Int32>` reduction
  and an explicit-count first-item helper;
- `jadren.core.utf8` – an ownership boundary for Jadren's validated UTF-8
  `String` value.
- `jadren.core.simd` – explicit 2/3/4/8-lane slice kernels with scalar
  fallback-compatible signatures.

The math details and the Quaternion/Matrix4 runtime boundary are recorded in
[`CORE_MATH_0.1.md`](CORE_MATH_0.1.md). The source package intentionally does
not redeclare those runtime ABI types.

The 0.1 surface deliberately stays small. `Option<T>` and `Result<T, E>` are
built-in carriers with `Some`/`None` and `Ok`/`Error` patterns. `String` is an
owned UTF-8 value; callers do not receive a byte sequence that can silently
contain invalid text. Collection algorithms accept borrowed `Slice<T>` values
and use `.indices`, so they do not allocate or take ownership of caller data.

The runnable project in `examples/stdlib-core-project` is the integration
fixture. It is checked with the package-aware `jadren check <directory>`
command, while `scripts/check-stdlib.ps1` remains compatible with the older
Windows preview for source-file regression checks.

The package is intentionally free of filesystem, network, process and build
script side effects.

## Module contract

- One source file declares one logical module.
- `pub` is required for a symbol to cross a module boundary.
- Imports resolve in a deterministic multi-source compiler session.
- Value-import cycles are rejected; type-only cycles may be handled later.
- A package manifest and lockfile identify the dependency graph.
- Standard-library package versions follow SemVer and the language edition is
  independent from the compiler version.

## Next layers

1. `jadren.collections` – slices, maps, sets and owned buffers.
2. `jadren.math` – vectors, matrices, quaternions and SIMD dispatch.
3. `jadren.concurrent` – jobs, atomics and cancellation.
4. `jadren.io` – files, streams and serialization.
5. `jadren.ui`, `jadren.unity` and `jadren.gpu` – platform-facing packages.

Each layer must ship with API tests, target-specific implementations,
documentation and benchmark evidence. No package may silently execute build
scripts or access the network during compilation.
