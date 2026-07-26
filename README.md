# Jadren

Jadren is an experimental systems and compute programming language focused on
safe native performance, predictable memory behaviour, data-parallel workloads,
and practical Unity integration.

The current implementation is the **0.1.0-preview.2 public preview**. It is
suitable for evaluation, local experiments, and integration testing. It is not
yet a stable language or a production-supported release, and the Windows
installer is unsigned.

## What Jadren is for

Jadren is designed for programs where data layout, predictable work, and an
explicit native boundary matter:

- game and simulation kernels;
- contiguous CPU and SIMD workloads;
- native libraries called from C# or Unity;
- bounded GPU compute operations;
- real-time code that must make allocation and ownership visible.

It does not promise that translating an object-heavy C# method automatically
makes it faster. The meaningful comparison is an equivalent algorithm with the
same data layout, precision, synchronization, and target hardware.

## Project goals

- safe defaults with explicit `unsafe`, FFI, and external-system boundaries;
- native CPU code with scalar and SIMD variants;
- ARM64/NEON and GPU compute paths for supported workloads;
- predictable allocation behaviour for real-time code;
- generated C ABI and C# bindings for Unity;
- one source model with capability-aware target selection and fallbacks.

## A small Jadren program

```jadren
module examples.hello

fn main() {
    print("Hello, Jadren")
}
```

The public repository is source-first. With Rust 1.97.0 and the pinned LLVM
toolchain available, the program can be checked, built, and run with:

```powershell
cargo run -p jadren-cli -- check examples/hello.jdn
cargo run -p jadren-cli -- build examples/hello.jdn
cargo run -p jadren-cli -- run examples/hello.jdn
```

## Documentation

- [Getting started](docs/GETTING_STARTED.md) — requirements and first build;
- [Language overview](docs/LANGUAGE_OVERVIEW.md) — syntax, types, and safety;
- [Compiler and platforms](docs/COMPILER_AND_PLATFORMS.md) — capability policy;
- [Unity integration](docs/UNITY_INTEGRATION.md) — native and C# boundaries;
- [Unity project guide](docs/UNITY_PROJECT_GUIDE.md) — scene setup and testing;
- [Jadren Animation System](docs/ANIMATION_SYSTEM.md) — bounded animation path;
- [VS Code extension](docs/VSCODE_EXTENSION.md) — editor support;
- [Public roadmap](docs/ROADMAP.md) — user-facing next steps;
- [Security](SECURITY.md) — responsible reporting;
- [Contributing](CONTRIBUTING.md) — changes and development expectations.

## Unity distribution

Unity packages, native plugins, samples, and asset files are distributed
separately through the Unity Asset Store. They are intentionally excluded from
this public source export. Use a package and documentation set from the same
Jadren release line; do not mix binaries from an older `Jadren_win` build with
a newer Unity package.

## Current scope

The reference compiler is written in Rust. The most mature local development
path is Windows x86-64 with LLVM and Unity native integration. Linux x86-64,
Android ARM64/NEON, and bounded GPU backends have separate capability gates.
Platform support is claimed only when the matching artifact was built and the
stated workload was executed on matching hardware.

The 0.1 preview deliberately does not claim full Unity Mecanim parity, general
GPU-kernel portability, broad mobile coverage, signed production packages, or
universal performance improvements.

## Versioning

The current language edition is `0.1-draft`. Syntax, diagnostics, ABI details,
and standard APIs may still change before the first stable edition. Examples
and release notes will describe user-visible migrations.

## License

Jadren is dual-licensed under Apache-2.0 or MIT, at your option.
