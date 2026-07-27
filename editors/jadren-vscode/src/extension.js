const vscode = require('vscode');
const {
  LanguageClient,
  TransportKind,
} = require('vscode-languageclient/node');

let client;

const OFFLINE_COMPLETIONS = [
  ['Bool', vscode.CompletionItemKind.Type, 'Boolean type'],
  ['Int32', vscode.CompletionItemKind.Type, '32-bit signed integer type'],
  ['Int64', vscode.CompletionItemKind.Type, '64-bit signed integer type'],
  ['UInt32', vscode.CompletionItemKind.Type, '32-bit unsigned integer type'],
  ['Float32', vscode.CompletionItemKind.Type, '32-bit floating-point type'],
  ['Float64', vscode.CompletionItemKind.Type, '64-bit floating-point type'],
  ['String', vscode.CompletionItemKind.Type, 'UTF-8 string type'],
  ['Vec2', vscode.CompletionItemKind.Type, '2D vector type'],
  ['Vec3', vscode.CompletionItemKind.Type, '3D vector type'],
  ['Vec4', vscode.CompletionItemKind.Type, '4D vector type'],
  ['print', vscode.CompletionItemKind.Function, 'Print a value to stdout'],
  ['fn', vscode.CompletionItemKind.Keyword, 'Declare a function'],
  ['let', vscode.CompletionItemKind.Keyword, 'Declare a local binding'],
  ['if', vscode.CompletionItemKind.Keyword, 'Conditional block'],
  ['else', vscode.CompletionItemKind.Keyword, 'Alternative conditional block'],
  ['for', vscode.CompletionItemKind.Keyword, 'Iteration block'],
  ['while', vscode.CompletionItemKind.Keyword, 'While loop'],
  ['return', vscode.CompletionItemKind.Keyword, 'Return from a function'],
  ['module', vscode.CompletionItemKind.Keyword, 'Declare a module'],
  ['import', vscode.CompletionItemKind.Keyword, 'Import a module'],
  ['struct', vscode.CompletionItemKind.Keyword, 'Declare a struct'],
  ['enum', vscode.CompletionItemKind.Keyword, 'Declare an enum'],
  ['match', vscode.CompletionItemKind.Keyword, 'Pattern matching expression'],
];

const OFFLINE_HOVER_DOCS = new Map([
  ['fn', '**fn** declares a Jadren function. Example: `fn add(a: Int32, b: Int32) -> Int32 { ... }`'],
  ['let', '**let** declares a local binding with an explicit or inferred type.'],
  ['if', '**if** evaluates a condition and executes its block when the condition is true.'],
  ['else', '**else** provides the alternative branch of an `if` expression.'],
  ['for', '**for** iterates over a range or collection. Example: `for item in items { ... }`'],
  ['while', '**while** repeats a block while its condition remains true.'],
  ['return', '**return** finishes the current function and returns a value when required.'],
  ['module', '**module** declares the module name at the top of a Jadren source file.'],
  ['import', '**import** makes public declarations from another module available.'],
  ['struct', '**struct** declares a product type with named fields.'],
  ['enum', '**enum** declares a type with a fixed set of variants.'],
  ['match', '**match** selects a branch using pattern matching.'],
  ['print', '**print** writes a value to standard output.'],
  ['Bool', '**Bool** is the boolean type.'],
  ['Int32', '**Int32** is a signed 32-bit integer type.'],
  ['Int64', '**Int64** is a signed 64-bit integer type.'],
  ['UInt32', '**UInt32** is an unsigned 32-bit integer type.'],
  ['Float32', '**Float32** is a 32-bit floating-point type.'],
  ['Float64', '**Float64** is a 64-bit floating-point type.'],
  ['String', '**String** is the UTF-8 text type.'],
  ['Vec2', '**Vec2** is a two-component vector type.'],
  ['Vec3', '**Vec3** is a three-component vector type.'],
  ['Vec4', '**Vec4** is a four-component vector type.'],
]);

function registerOfflineCompletions(context) {
  const provider = vscode.languages.registerCompletionItemProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideCompletionItems(document, position) {
        const line = document.lineAt(position.line).text.slice(0, position.character);
        const match = line.match(/[A-Za-z_][A-Za-z0-9_]*$/);
        const prefix = match ? match[0].toLowerCase() : '';
        return OFFLINE_COMPLETIONS
          .filter(([label]) => !prefix || label.toLowerCase().startsWith(prefix))
          .map(([label, kind, documentation]) => {
            const item = new vscode.CompletionItem(label, kind);
            item.detail = 'Jadren offline';
            item.documentation = new vscode.MarkdownString(documentation);
            if (match) {
              item.range = new vscode.Range(
                position.line,
                position.character - match[0].length,
                position.line,
                position.character,
              );
            }
            return item;
          });
      },
    },
  );
  context.subscriptions.push(provider);
}

function registerOfflineHover(context) {
  const provider = vscode.languages.registerHoverProvider(
    { scheme: 'file', language: 'jadren' },
    {
      provideHover(document, position) {
        const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
        if (!range) {
          return undefined;
        }
        const word = document.getText(range);
        const documentation = OFFLINE_HOVER_DOCS.get(word);
        if (!documentation) {
          return undefined;
        }
        return new vscode.Hover(new vscode.MarkdownString(documentation), range);
      },
    },
  );
  context.subscriptions.push(provider);
}

function activate(context) {
  registerOfflineCompletions(context);
  registerOfflineHover(context);
  const configuration = vscode.workspace.getConfiguration('jadren');
  const command = configuration.get('lspPath', 'jadren');
  const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const serverOptions = {
    run: {
      command,
      args: ['lsp'],
      transport: TransportKind.stdio,
      options: { cwd: workspaceFolder },
    },
    debug: {
      command,
      args: ['lsp'],
      transport: TransportKind.stdio,
      options: { cwd: workspaceFolder },
    },
  };
  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'jadren' }],
    synchronize: {
      configurationSection: 'jadren',
    },
  };
  client = new LanguageClient(
    'jadrenLanguageServer',
    'Jadren Language Server',
    serverOptions,
    clientOptions,
  );
  context.subscriptions.push(client.start());
}

function deactivate() {
  return client?.stop();
}

module.exports = {
  activate,
  deactivate,
};
