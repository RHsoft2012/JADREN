# Contributing

Jadren is still preparing its public contribution workflow. Until that process
is announced, use public issues for non-sensitive bug reports and feature
discussion, and use private vulnerability reporting for security concerns.

## Development principles

- preserve safe defaults and explicit capability boundaries;
- keep diagnostics actionable and deterministic;
- avoid hidden allocation in real-time paths;
- add focused tests for every semantic or backend change;
- do not claim platform support without matching execution evidence;
- compare performance only with equivalent workloads and data layouts.

## Local checks

For Rust workspace changes:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Platform-specific changes require the corresponding native, Unity, Android, or
GPU validation in addition to the workspace checks.

## Licensing

Unless explicitly stated otherwise, contributions are accepted under either the
Apache License 2.0 or the MIT License, at the user's option. A formal contributor
policy will be published before accepting broad external contributions.
