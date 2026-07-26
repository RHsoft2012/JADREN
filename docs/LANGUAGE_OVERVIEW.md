# Language overview

Jadren combines a compact systems-language surface with explicit contracts for
memory, effects, native interoperability, SIMD, and compute execution.

## Modules and functions

```jadren
module demo.math

fn add(left: Int32, right: Int32) -> Int32 {
    return left + right
}
```

## Data types

The draft includes fixed-width scalar types, arrays, buffers, slices, structs,
enums, generic values, and C-compatible layouts.

```jadren
@repr(C)
pub struct Vec3 {
    x: Float32,
    y: Float32,
    z: Float32,
}
```

`@repr(C)` is an ABI promise. The compiler restricts it to layouts that can be
represented safely at a native boundary.

### Numeric widths (draft 0.2)

The canonical floating-point names are `Float16`, `Float32`, and `Float64`.
The short aliases `F16`, `F32`, and `F64` are type-checking aliases: they do
not change representation, layout, or ABI. Use an explicit literal suffix such
as `1.0f32` when the literal width matters. `F128`/`Float128` is reserved for a
future capability-gated design and is not an available built-in type yet.

For Unity transforms, animation data, SIMD, and portable GPU work, `Float32`
is the usual choice. `Float64` remains an explicit option for precision-first
CPU calculations; the width alone is not a performance guarantee.

## Safety model

Safe code is the default. Ownership, borrowing, regions, bounds checks, and
effect validation are designed to reject invalid memory access and hidden
real-time side effects before native code generation.

Operations that cross a safety boundary must be visible in source. This applies
to native FFI, unsafe operations, allocation-sensitive code, and target-specific
capabilities.

## Data-parallel execution

Jadren exposes CPU, SIMD, and GPU intent through explicit annotations and target
capabilities. A function may provide a safe CPU fallback when a faster target is
not available.

```jadren
@export (targets: [cpu, simd, gpu], fallback: cpu)
fn inspect(values: Slice<Float32>) -> Int32 {
    return 0
}
```

Target annotations do not guarantee that every function can run everywhere.
The compiler must validate the supported operation and resource subset for the
selected backend.

## Draft stability

Syntax, diagnostics, ABI details, and standard APIs may still change before the
first stable edition. Release notes will describe user-visible migrations.
