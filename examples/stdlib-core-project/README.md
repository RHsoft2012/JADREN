# Jadren standard-library project

This small package demonstrates the 0.1 contracts exposed by `jadren-core`:

- `Option<T>` and `Result<T, E>` are explicit carriers, not exceptions;
- `read Slice<T>` is a borrowed, bounds-checked view for collection helpers;
- `String` is an owned valid UTF-8 value at the language boundary.
- `jadren.core.process` exposes the native argument count and bounded
  caller-owned UTF-8 argument read-back for CLI programs.
- `jadren.core.time` exposes deterministic UTC decomposition alongside the
  wall-clock and monotonic time primitives.
- `jadren.core.io` exposes caller-owned console byte streams without an FFI
  declaration.

From the repository root:

```powershell
jadren lock examples/stdlib-core-project
jadren check examples/stdlib-core-project
```

`check` must be the package-aware CLI from the current Jadren distribution.
The source-file checker is useful for older previews, but it does not load
path dependencies for a directory check.
