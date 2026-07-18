# Compiler and platforms

The Jadren compiler uses a staged pipeline:

1. lexer and lossless syntax tree;
2. AST and name resolution;
3. type and effect checking;
4. HIR, MIR, and Jadren IR;
5. target-specific code generation;
6. runtime, ABI, and package integration.

## CPU backends

The primary development backend uses LLVM for native x86-64 output. Scalar and
AVX2 policies are selected explicitly so unsupported processors retain a safe
fallback. ARM64/NEON work follows the same capability-based model.

## GPU backends

Jadren has bounded compute paths for SPIR-V/Vulkan, DirectX 12, and Metal source
generation. Each backend accepts only a validated resource and instruction
subset. Source generation is not reported as native execution unless the shader
was compiled, dispatched, completed, and read back on matching hardware.

## Energy-aware targets

Battery-oriented builds are expected to prefer lower parallelism, reduced update
rates, scalar or NEON code where appropriate, and explicit GPU use only when its
total transfer and dispatch cost is justified.

## Support policy

The project distinguishes four states:

- source model designed;
- compiler output generated;
- artifact compiled for the target;
- artifact executed and validated on physical hardware.

Only the last state is treated as execution support. The public compatibility
matrix will grow as repeatable device evidence becomes available.
