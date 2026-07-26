# Jadren standard-library project

This small package demonstrates the 0.1 contracts exposed by `jadren-core`:

- `Option<T>` and `Result<T, E>` are explicit carriers, not exceptions;
- `read Slice<T>` is a borrowed, bounds-checked view for collection helpers;
- `String` is an owned valid UTF-8 value at the language boundary.

From the repository root:

```powershell
jadren lock examples/stdlib-core-project
jadren check examples/stdlib-core-project
```

`check` must be the package-aware CLI from the current Jadren distribution.
The source-file checker is useful for older previews, but it does not load
path dependencies for a directory check.
