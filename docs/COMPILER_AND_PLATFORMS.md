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

The 0.1 core math contract exposes fixed-width `Float2`, `Float3`, `Float4`
and `Float8` lane operations with checked `Slice` access. Quaternion and
row-major `Matrix4` layouts are provided by the runtime/Unity ABI, including
the shortest-arc `SlerpUnclamped` helper. Backend selection happens once
outside the hot loop and always has a scalar fallback; these contracts do not
constitute a universal FPS claim.

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

## Current bounded evidence

This table is a capability snapshot for Jadren `0.1-dev`. A row applies only to
the stated workload; it is not a blanket platform-support or performance claim.

| Area | Platform and target | Highest verified state | Bounded scope |
| --- | --- | --- | --- |
| Compiler CLI and developer ZIP | Windows x86-64 | Executed | Clean extraction and CLI command smoke; unsigned developer package only. |
| Native animation | Windows x86-64 baseline/AVX2 | Executed | Safe baseline fallback and numerical parity; no universal FPS claim. |
| Native animation | Physical Android ARM64 with NEON artifact | Executed | ABI, sample, angle and checksum smoke on three recorded device classes; no FPS, sustained thermal or broad device-support claim. |
| Unity agent update kernel | Windows Player x86-64 AVX2 | Executed | Correctness-matched managed, Burst and Jadren AoSoA8 benchmark with rendering excluded. |
| Unity GPU skinned mesh | Windows Unity Editor | Executed | Real prefab, material and texture binding plus one visible GPU frame; no production FPS claim. |
| GPU compute | Vulkan validated subset | Executed | Cross-backend artifact differential for the allowlisted workload only. |
| GPU compute | Windows DirectX 12 validated subset | Executed | Twenty-eight exact allowlisted executions; no general SPIR-V translation claim. |
| GPU compute | macOS Metal bounded source plan | Generated | Twenty-eight source identities prepared; native macOS compile, dispatch and readback remain unverified. |
| Self-hosting preview | Windows x86-64 | Executed | Bounded literal/binary-family driver with exact precedence and left associativity through sixteen operators; at most two disjoint groups may each wrap one adjacent operand pair, one group may have one redundant nested delimiter layer, and one function may use one or two typed `Int32` parameters in a binary return. Nested groups with extra operators, locals, calls and general parameter expressions remain unsupported; this is not a general expression parser or second compiler. |
| Self-hosting preview | Docker Linux x86-64 | Executed | Container-local object, loader and CLI run; not a native Linux installation claim. |
| Developer package | Linux x86-64 | Executed | Reproducible unsigned archive passed checksums plus native build/run in a clean Debian 12 container; broader distributions and bare-metal hosts remain unverified. |

The matrix intentionally keeps public-release readiness false. Signed packages,
retained physical macOS evidence, broader Linux distribution and device
coverage, and the declared external security/release gates remain separate work.
