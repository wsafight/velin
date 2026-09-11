// The Velin VS Code extension: a thin client that highlights `.velin` files
// (via the TextMate grammar in `syntaxes/`) and connects them to the
// `velin-lsp` server over stdio for live diagnostics, completion, and the
// label outline. All language intelligence lives in the Rust server; this file
// only launches it and wires up the document selector.

const { workspace, window } = require("vscode");
const {
  LanguageClient,
  TransportKind,
} = require("vscode-languageclient/node");

/** @type {import("vscode-languageclient/node").LanguageClient | undefined} */
let client;

/**
 * Starts the language client, spawning the configured `velin-lsp` binary.
 * @param {import("vscode").ExtensionContext} _context
 */
function activate(_context) {
  const configured = workspace
    .getConfiguration("velin")
    .get("server.path", "velin-lsp");

  // Both the run and debug profiles use the same stdio transport; a build that
  // wants verbose server logging can override the command in settings.
  const serverOptions = {
    run: { command: configured, transport: TransportKind.stdio },
    debug: { command: configured, transport: TransportKind.stdio },
  };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "velin" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.velin"),
    },
  };

  client = new LanguageClient(
    "velin",
    "Velin Language Server",
    serverOptions,
    clientOptions
  );

  client.start().catch((error) => {
    window.showErrorMessage(
      `Velin: failed to start '${configured}'. Set 'velin.server.path' to the velin-lsp binary. (${error})`
    );
  });
}

/** Stops the language client on deactivation. */
function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
