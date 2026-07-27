# Jadren versioning and compatibility

Jadren uses separate version lines for the compiler, language edition and
integrations. A matching number does not by itself mean that two artifacts
are ABI-compatible.

## Current preview line

| Artifact | Current value | Meaning |
| --- | --- | --- |
| Compiler/public installer | `0.1.0-preview.4` | unsigned evaluation preview |
| Language edition | `0.1-draft` | syntax and standard APIs may still change |
| Unity package | `com.jadren.animation` `0.1.0` | experimental Unity integration |
| VS Code extension | `jadren-vscode` `0.1.0` | syntax, LSP diagnostics and navigation |

Preview patch releases may change diagnostics, generated artifacts, package
contracts or editor behaviour. Projects should pin the compiler, Unity package
and VS Code extension to the same preview line and record the exact commit in
benchmark reports.

## Compatibility rules

- `0.x` does not promise stable source or ABI compatibility.
- A language-edition change requires a migration note and updated examples.
- Runtime ABI changes require a new ABI minor and regenerated native plugins.
- Unity package changes must update the package changelog and sample fixture.
- A release is not called stable until Windows/Linux gates, documentation,
  security review and the declared platform evidence are complete.

The current preview is suitable for local experiments, CI validation and Unity
integration testing. It is not a production-support or universal performance
claim.
