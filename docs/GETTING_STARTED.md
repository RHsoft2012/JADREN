# Getting started

This guide uses the compiler from a local development build. A signed public
installer is not available yet.

## Requirements

- Windows x86-64 for the current primary development path;
- Rust 1.97.0, selected by `rust-toolchain.toml`;
- LLVM 22.1.x for native LLVM code generation;
- PowerShell 7 for the repository helper commands.

## Build the compiler

From the repository root:

```powershell
cargo build -p jadren-cli
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
```

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

`build` creates a Windows x86-64 executable under
`target/jadren/debug/hello.exe` by default. Use `-o <path>`,
`--profile release`, or `--cpu avx2` to select another output, optimized build,
or explicit AVX2 code generation. The executable entry is a parameterless
`fn main()` returning either `Unit` or `Int32`; an `Int32` result becomes the
process exit code. The current Windows console runtime supports the built-in
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
- The installer and release artifacts are not yet signed for public use.
- Native `build` and `run` currently target Windows x86-64. Linux executable
  linking remains a separate validation gate.
- Target support differs by platform and workload.
- GPU and mobile targets require their own execution validation.
- Benchmark results are meaningful only for the exact published workload,
  layout, hardware, and build profile.
