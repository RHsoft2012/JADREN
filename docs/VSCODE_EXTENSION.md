# VS Code extension

Jadren Language Support registers `.jdn` files, syntax highlighting, file icons,
and a Language Server Protocol client.

## Install a local development package

1. Open VS Code.
2. Run **Extensions: Install from VSIX...** from the Command Palette.
3. Select the Jadren `.vsix` package.
4. Reload VS Code when requested.
5. Open a `.jdn` file and confirm that the language mode is **Jadren**.

The extension setting `jadren.lspPath` selects the Jadren CLI executable. Its
default value is `jadren`, resolved from `PATH`.

```json
{
  "jadren.lspPath": "jadren"
}
```

## Current capabilities

- syntax highlighting;
- diagnostics and document symbols;
- definition, references, and rename;
- hover, completion, and inlay hints;
- semantic tokens for the supported language subset.

Debugging is not part of the current extension. Pressing F5 on a `.jdn` file may
therefore ask for a debugger extension. Use the terminal or a build task to run
the compiler during the development-preview phase.
