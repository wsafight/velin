// The Velin VS Code extension: a thin client that highlights `.velin` files
// (via the TextMate grammar in `syntaxes/`) and connects them to the
// `velin-lsp` server over stdio for live diagnostics, completion, and the
// label outline. All language intelligence lives in the Rust server; this file
// only launches it and wires up the document selector.

const { workspace, window } = require("vscode");
const fs = require("node:fs");
const path = require("node:path");
const {
  LanguageClient,
  TransportKind,
} = require("vscode-languageclient/node");

/** @type {import("vscode-languageclient/node").LanguageClient | undefined} */
let client;

/**
 * Starts the language client, preferring an explicitly configured server and
 * then the platform binary bundled in release VSIX packages.
 * @param {import("vscode").ExtensionContext} context
 */
function activate(context) {
  const configuredValue = workspace
    .getConfiguration("velin")
    .get("server.path", "")
    .trim();
  const workspaceFolder = workspace.workspaceFolders?.[0]?.uri.fsPath;
  const configured = workspaceFolder
    ? configuredValue.replaceAll("${workspaceFolder}", workspaceFolder)
    : configuredValue;
  const bundled = context.asAbsolutePath(
    path.join("bin", process.platform === "win32" ? "velin-lsp.exe" : "velin-lsp")
  );
  const command = configured || (fs.existsSync(bundled) ? bundled : "velin-lsp");

  if (!configured && command === bundled && process.platform !== "win32") {
    try {
      fs.chmodSync(command, 0o755);
    } catch (error) {
      window.showErrorMessage(
        `Velin: failed to make the bundled language server executable. (${error})`
      );
      return;
    }
  }

  // Both the run and debug profiles use the same stdio transport; a build that
  // wants verbose server logging can override the command in settings.
  const serverOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
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
      `Velin: failed to start '${command}'. Set 'velin.server.path' to override the velin-lsp binary. (${error})`
    );
  });
}

/** Stops the language client on deactivation. */
function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
