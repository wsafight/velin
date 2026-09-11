# Tooling

[简体中文](TOOLING.zh-CN.md)

Velin keeps its tools on the same parser, checker, compiler, and VM as the embedding API. A script that passes in the editor is checked by the same code used by the CLI and browser build.

## Prerequisites

- Rust 1.88 or newer with Cargo.
- Node.js 22.12 or newer for the documentation site.
- `wasm-pack` and the `wasm32-unknown-unknown` target for the browser Playground.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

## Command-line interface

Build the CLI from the workspace root:

```sh
cargo build -p velin-cli
```

Check a script without running host effects:

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

`check` prints parsing, lowering, type, and definite-assignment diagnostics. It exits with status 1 when any error is present; warnings alone do not fail the command.

Use `velin check --json <file>` for a stable `{ ok, diagnostics, error }` JSON result. Both subcommands accept `-` as the source path to read UTF-8 source from stdin; `velin --help` and `velin --version` print command metadata and exit successfully.

Run through the line-oriented reference host:

```sh
cargo run -p velin-cli -- run examples/counting.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

The reference host implements two conventions:

| Command | CLI behavior |
| --- | --- |
| `say(values...)` | Writes values to stdout, separated by spaces |
| `ask(prompt...)` | Writes the prompt, reads one line, and returns an integer, boolean, or string |

Unknown commands are still valid host effects; the reference runner prints their name and arguments, then resumes without a value.

## Language server

Build the stdio language server:

```sh
cargo build -p velin-lsp
```

`velin-lsp` provides:

- Live parser, lowering, type, and definite-assignment diagnostics.
- Completion for keywords, built-ins, variables, and labels.
- Document symbols for labels.
- Hover help for language names and go-to-definition/reference search for labels.
- Standard JSON-RPC `MethodNotFound` responses for unsupported requests.

The server limits JSON-RPC messages to 4 MiB and bounds header size before allocating the body.

## VS Code extension

The extension is in `editors/vscode-velin` and contributes `.velin` registration, TextMate highlighting, indentation rules, completion, diagnostics, and label outlines. Platform release VSIX files bundle the matching `velin-lsp`; the setting below is only needed to override it or during development.

```sh
cd editors/vscode-velin
npm ci
```

Open that folder in VS Code and press `F5` to launch an Extension Development Host. Without a bundled server, the extension starts `velin-lsp` from `PATH`. To use a local binary, set:

```json
{ "velin.server.path": "${workspaceFolder}/target/debug/velin-lsp" }
```

## Browser Playground

How to use the site UI is in [Playground](PLAYGROUND.md). How to call the Wasm exports from your own page is in [WebAssembly](WASM.md). To rebuild the package locally:

```sh
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

Open `http://localhost:8080`. `check(source)` and `run(source, repliesJson)` execute entirely in the browser. The scripted browser host sends `say` values to the output and consumes `ask` answers from the JSON array.

## Documentation site

The independent Astro project under `site/` generates both languages from the repository Markdown and publishes the Playground at `/playground/`.

```sh
cd site
npm ci
npm run check
npm run build
npm test
```

`npm run build` compiles `velin-wasm`, prepares localized content, produces static HTML, and checks every internal link. `npm test` runs the built site in desktop and mobile Chromium projects.

Set `GITHUB_REPOSITORY=wsafight/velin` locally to test GitHub Pages under `/velin/`. `DOCS_SITE` and `DOCS_BASE` override the generated origin and base path.

## Continuous integration

`.github/workflows/core.yml` runs formatting, Clippy, workspace tests, a Wasm target build, and whole-workspace crate packaging verification. `.github/workflows/site.yml` builds and tests the complete static site, uploads its artifact, and deploys it to GitHub Pages from the repository's default branch. `.github/workflows/release.yml` builds CLI/LSP archives and platform-specific VSIX packages for manual runs; a matching `v*` tag also creates a GitHub Release with SHA-256 checksums.

## Workspace verification

Run the same core gates before a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build -p velin-wasm --target wasm32-unknown-unknown
cargo package --workspace
```
