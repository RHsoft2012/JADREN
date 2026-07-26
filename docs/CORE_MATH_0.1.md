# Jadren core math 0.1

`jadren.core.math` is the portable source-level math layer. It keeps the
language surface small and lets the compiler select a backend without changing
the function signature.

## Value types

The source language exposes `Float2`, `Float3`, `Float4` and `Float8` as
fixed-width values. `jadren.core.math.add2`, `add3` and `add4` use lane-wise
addition and are allocation-free. `jadren.core.simd` adds explicit checked
slice paths for 2, 3, 4 and 8 lanes:

```jadren
@noalloc
pub fn add4_in_place(values: write Slice<Float32>, index: UIntSize, delta: Float32) {
    let current: Float4 = vector_load4(values, index)
    vector_store4(values, index, current + vector_splat4(delta))
}
```

The caller owns the slice. The index and vector width remain explicit, so a
scalar tail can be handled by the caller without an implicit out-of-bounds
load.

## Quaternion and Matrix4 boundary

`Quaternion` and row-major `Matrix4` are already stable in
`jadren-runtime` and the Unity native bridge. Their C-compatible layouts,
identity helpers and shortest-arc `quaternion_slerp_unclamped` are covered by
runtime tests. They are intentionally not duplicate source-level nominal types
in Jadren 0.1; exposing a second source ABI would risk diverging from the
runtime layout. A future language edition can add constructors and methods
after the value calling convention is formally closed.

## Backend dispatch

The source contract is target-neutral. The compiler/runtime provide:

- baseline scalar fallback;
- explicit x86-64 AVX2 emission;
- explicit ARM64 NEON emission;
- one-time capability selection outside the hot loop.

Unsupported AVX2/NEON capability always falls back to baseline. The contract
does not claim that every workload is faster; performance claims require the
same input, checksum and release benchmark on the target device.

## Verification

```powershell
pwsh -File scripts/check-core-math.ps1 `
  -JadrenPath jadren -SkipCargo
```

For the package-aware source CLI add `-PackageAware`. The command prints a
short contract summary; pass `-ReportPath` when a machine-readable report is
needed.
