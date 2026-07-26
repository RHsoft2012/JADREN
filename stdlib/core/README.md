# `jadren-core` 0.1.0

The first standard-library package for Jadren. It is deliberately small and
platform-neutral: application code can use it on Windows, Linux and Android
without importing a platform runtime.

## Modules

- `jadren.core.math` – checked scalar helpers such as `clamp` and `abs`.
- `jadren.core.status` – stable `CoreStatus` values for recoverable boundaries.

## Local verification

From the repository root:

```powershell
pwsh -File scripts/check-stdlib.ps1 -JadrenPath jadren
```

The example imports two public functions from `jadren.core.math`. The command
checks all package sources in one compiler session, so import resolution is
actually exercised rather than only checking each file in isolation.
