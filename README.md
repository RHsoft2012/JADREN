# Jadren

Jadren is an experimental systems and compute programming language focused on
safe native performance, predictable memory behaviour, data-parallel workloads,
and practical Unity integration.

The current implementation is a **0.1 draft**. It is suitable for evaluation,
local experiments, and integration testing. It is not yet a stable language or
a production-supported public release.

## Project goals

- safe defaults with explicit `unsafe`, FFI, and external-system boundaries;
- native CPU code with scalar and SIMD variants;
- ARM64/NEON and GPU compute paths for supported workloads;
- predictable allocation behaviour for real-time code;
- generated C ABI and C# bindings for Unity;
- one source model with capability-aware target selection and fallbacks.

## Documentation

- [Getting started](docs/GETTING_STARTED.md)
- [Language overview](docs/LANGUAGE_OVERVIEW.md)
- [Compiler and platforms](docs/COMPILER_AND_PLATFORMS.md)
- [Unity integration](docs/UNITY_INTEGRATION.md)
- [Jadren Animation System](docs/ANIMATION_SYSTEM.md)
- [VS Code extension](docs/VSCODE_EXTENSION.md)
- [Public roadmap](docs/ROADMAP.md)
- [Security](SECURITY.md)
- [Contributing](CONTRIBUTING.md)

## Current scope

The reference compiler is written in Rust. The most mature local development
path is Windows x86-64 with LLVM and Unity native integration. Other CPU and GPU
targets are being validated behind explicit capability gates. Platform support
is claimed only when the matching build and execution evidence is available.

Jadren is dual-licensed under Apache-2.0 or MIT, at your option.
