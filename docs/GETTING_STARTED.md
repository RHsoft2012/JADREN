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

Check it with:

```powershell
cargo run -p jadren-cli -- check hello.jdn
```

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
- Target support differs by platform and workload.
- GPU and mobile targets require their own execution validation.
- Benchmark results are meaningful only for the exact published workload,
  layout, hardware, and build profile.
