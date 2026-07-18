const vscode = require('vscode');
const {
  LanguageClient,
  TransportKind,
} = require('vscode-languageclient/node');

let client;

function activate(context) {
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
