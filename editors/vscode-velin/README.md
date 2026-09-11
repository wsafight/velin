# Velin for VS Code

Syntax highlighting and language support for [Velin](../../README.md) `.velin`
scripts, backed by the `velin-lsp` language server.

## Features

- **Syntax highlighting** — keywords (`label`/`default`/`set`/`perform`/`if`/
  `elif`/`else`/`while`/`jump`), built-in functions, strings, integers,
  booleans, operators, and `#` comments (TextMate grammar, works with no server).
- **Live diagnostics** — parse, lower, and static-check errors as you type
  (definite-assignment and type inference from `velin-check`).
- **Completion** — keywords, built-ins, and the variables/labels the current
  script declares.
- **Outline** — the script's `label`s as document symbols.

## Requirements

The extension launches the `velin-lsp` binary. Build it from the workspace root:

```sh
cargo build -p velin-lsp        # produces target/debug/velin-lsp
```

Then either put `velin-lsp` on your `PATH`, or set **`velin.server.path`** in
settings to the built binary, e.g.:

```json
{ "velin.server.path": "${workspaceFolder}/target/debug/velin-lsp" }
```

## Development

```sh
cd editors/vscode-velin
npm install          # pulls vscode-languageclient
```

Open this folder in VS Code and press <kbd>F5</kbd> to launch an Extension
Development Host, then open any `.velin` file (see `../../examples`).

## Layout

| File | Purpose |
|------|---------|
| `package.json` | Language contribution, grammar wiring, `velin.server.path` setting |
| `language-configuration.json` | Comments, brackets, indentation rules |
| `syntaxes/velin.tmLanguage.json` | TextMate grammar for highlighting |
| `src/extension.js` | `vscode-languageclient` shell that spawns `velin-lsp` over stdio |
