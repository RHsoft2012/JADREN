# Getting started

This guide uses the compiler from a local development build. An unsigned
Windows development installer is available for local testing; a signed public
installer is not available yet.

The preview installer is `Jadren-Setup-0.1-dev.exe`. It installs by default to
`%LOCALAPPDATA%/Programs/Jadren` and accepts `--install-root PATH` for an
explicit destination. It does not modify PATH or claim public release support.

The isolated installer smoke is available for development validation:

```powershell
pwsh -NoProfile -File scripts/check-windows-installer-runtime.ps1 -RequirePass
```

It installs into a temporary workspace directory, runs `version`, `doctor`,
and the bundled `examples/hello.jdn` check, then removes the temporary files.
It does not validate signing or production deployment.

## Requirements

- Windows or Linux x86-64 for native CLI development;
- Rust 1.97.0, selected by `rust-toolchain.toml`;
- LLVM 22.1.x for native LLVM code generation;
- PowerShell 7 for the repository helper commands; Docker Desktop is used by
  the repeatable local Linux gate when developing on Windows.

## Build the compiler

From the repository root:

```powershell
cargo build -p jadren-cli
cargo run -p jadren-cli -- version
cargo run -p jadren-cli -- doctor
```

## Linux developer archive

The repository can produce an unsigned `jadren-0.1-dev-linux-x64.tar.gz`
developer archive. It contains the native x86-64 CLI, runtime, a statically
linked LLVM CLI backend, the minimal bundled Clang tools and resource headers,
public documentation, and examples. After extracting it on Linux:

```bash
tar -xzf jadren-0.1-dev-linux-x64.tar.gz
cd jadren-0.1-dev-linux-x64
bin/jadren version
bin/jadren doctor
bin/jadren run examples/hello.jdn
```

The current archive passed checksum validation plus `version`, `doctor`,
`check`, `format`, native Hello build/run and exit-code `42` checks in a clean
Debian 12 container. The host still needs standard glibc development files,
GCC runtime files, libstdc++, zlib, libxml2 and ICU. This is not a signed public
release or evidence for every Linux distribution and bare-metal configuration.

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
Linux. Use `-o <path>`,
`--profile release`, or `--cpu avx2` to select another output, optimized build,
or explicit AVX2 code generation. The executable entry is a parameterless
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
- The development installer and release artifacts are not yet signed for
  public use.
- Native `build` and `run` currently target Windows and Linux x86-64.
- Target support differs by platform and workload.
- GPU and mobile targets require their own execution validation.
- Benchmark results are meaningful only for the exact published workload,
  layout, hardware, and build profile.
