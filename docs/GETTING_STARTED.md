# Getting started

This guide uses the compiler from the public source repository. The repository
is currently the 0.1.0-preview.3 public preview: it provides source, examples,
and editor support. The Windows preview installer is unsigned and installs to
`%PROGRAMFILES%\\Jadren` after a UAC confirmation. Development and Unity
Asset Store packages are distributed separately and must match the same release
line.

## Requirements

- Windows or Linux x86-64 for native CLI development;
- Rust 1.97.0, selected by `rust-toolchain.toml`;
- LLVM 22.1.x for native LLVM code generation;
- PowerShell 7 is recommended on Windows; Docker Desktop is optional for a
  repeatable Linux validation environment.

## Build the compiler

From the repository root:

```powershell
cargo build -p jadren-cli
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
```

## Linux developer builds

The public repository supports a native Linux x86-64 development build when the
matching LLVM toolchain is installed. From the repository root:

```bash
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
cargo run -p jadren-cli -- run examples/hello.jdn
```

The public CI validates the pinned Linux path. This is not evidence for every
Linux distribution or bare-metal configuration.

## Run the first program

Create `hello.jdn`:

```jadren
module examples.hello

fn main() {
    print("Hello, Jadren")
}
```

Check, build, and run it with:

```powershell
cargo run -p jadren-cli -- check hello.jdn
cargo run -p jadren-cli -- build hello.jdn
cargo run -p jadren-cli -- run hello.jdn
```

`build` creates a native x86-64 executable under
`target/jadren/debug/hello.exe` on Windows or `target/jadren/debug/hello` on
Linux. Use `-o <path>`, `--profile release`, or `--cpu avx2` to select another
output, optimized build, or explicit AVX2 code generation. The executable
entry is a parameterless
`fn main()` returning either `Unit` or `Int32`; an `Int32` result becomes the
process exit code. The Windows and Linux console runtimes support the built-in
`print(String)` used by this example.

The development CLI also exposes formatting and intermediate-representation
inspection commands:

```powershell
cargo run -p jadren-cli -- format hello.jdn --check
cargo run -p jadren-cli -- emit ast hello.jdn
cargo run -p jadren-cli -- emit hir hello.jdn
cargo run -p jadren-cli -- emit mir hello.jdn
cargo run -p jadren-cli -- emit jir hello.jdn
```

## Important limitations

- The language specification is still a draft.
- A signed public installer and release artifacts are not yet available.
- Native `build` and `run` currently target Windows and Linux x86-64.
- Target support differs by platform and workload.
- GPU and mobile targets require their own execution validation.
- Benchmark results are meaningful only for the exact published workload,
  layout, hardware, and build profile.
