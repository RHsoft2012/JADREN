# VS Code extension

Jadren Language Support registers `.jdn` files, syntax highlighting, file icons,
and a Language Server Protocol client.

## Install a local development package

1. Open VS Code.
2. Run **Extensions: Install from VSIX...** from the Command Palette.
3. Select the Jadren `.vsix` package.
4. Reload VS Code when requested.
5. Open a `.jdn` file and confirm that the language mode is **Jadren**.

## Workspace Trust and Restricted Mode

When VS Code displays **You are in Restricted Mode**, the current workspace
folder has not been trusted. This is a VS Code safety boundary, not a Jadren
compiler error. Tasks, debugging, and some extension features stay disabled
until the folder is trusted.

Open your local checkout with **File: Open Folder...** (for example
`C:/path/to/JADREN`), then select **Trust** on the Workspace Trust page. Trust is
per-folder and per-window; opening only a `.jdn` file or another checkout can
return to Restricted Mode. **Workspaces: Manage Workspace Trust** in the
Command Palette shows the folders currently trusted. Trust only source folders
you recognize.

The extension setting `jadren.lspPath` selects the Jadren CLI executable. Its
default value is `jadren`, so install the CLI or make it available on `PATH`.
The VS Code stdio client passes `--stdio`, which Jadren accepts as the standard
LSP transport flag.

```json
{
  "jadren.lspPath": "jadren"
}
```

To verify an installed VSIX, open a `.jdn` file, confirm the **Jadren** language
mode, and check that diagnostics and completion appear without errors.

## Package trust and self-hosted updates

The preview `.vsix` is a developer package. The Windows Authenticode profile
documented for Jadren EXE/DLL artifacts does not sign the VSIX, and the local
self-signed identity `Roman Hladky, C=SK` is not a public trust chain. The
self-hosted updater uses HTTPS, the release catalog's exact byte count, and
SHA-256 before asking VS Code to install a downloaded VSIX. Public Marketplace
signing remains a separate release gate.

## Current capabilities

- syntax highlighting;
- diagnostics and document symbols;
- definition, references, and rename;
- hover, completion, and inlay hints;
- semantic tokens for the supported language subset;
- Windows native debugging through the local Microsoft C/C++ debug adapter.

## Jadren Debugger 0.2 (Windows)

Open a saved `.jdn` file and press `F5`, use the editor title's Run menu, right-click
a `.jdn` file in Explorer, or run **Jadren: Debug Current File**. The top-level
**Run > Start Debugging** command remains VS Code's configuration-based debugger
command.
Use **Jadren: Build Release EXE** from the same editor-title or Explorer menus
to create a release executable without starting a debugger.
Configure `jadren.build.releaseOutputDirectory` when a workspace needs a
different output directory.
The extension builds a debug executable and its adjacent `.pdb` file, then
launches the local `cppvsdbg` Debug Adapter Protocol implementation supplied by
the Microsoft **C/C++** extension (`ms-vscode.cpptools`).

Breakpoints, continue, step controls, call stack, function parameters, and
named source `let` locals use the generated CodeView/PDB symbols. The default
output is a debug executable in the extension's configured output directory;
change `jadren.debug.outputDirectory` when a workspace needs a different
location.
For an interactive Variables or call-stack check, set
`jadren.debug.stopAtEntry` to `true`; it defaults to `false` for normal F5 runs.

Debugger 0.2 is a Windows developer-preview feature. It maps supported Jadren
source locations and named user locals to native code. Compiler-generated
temporaries, runtime panic inspection, expression evaluation, and pretty
printers remain later work.
