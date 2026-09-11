# Velin Playground

A zero-server, in-browser editor for `.velin` scripts. The page hands your
source to `velin-wasm` (compiled to WebAssembly) and renders the diagnostics and
transcript it returns — the language logic runs entirely client-side.

```
web/playground/
├── index.html      # markup: editor, controls, results
├── style.css       # single-file stylesheet
├── playground.js   # UI and bounded worker lifecycle
├── worker.js       # loads and runs Wasm away from the main thread
└── pkg/            # BUILD OUTPUT (git-ignored) — see below
```

## Build the wasm engine

The page imports `./pkg/velin_wasm.js`, which is produced by
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/) from the `velin-wasm`
crate. From the workspace root:

```sh
wasm-pack build --target web --out-dir ../../web/playground/pkg crates/velin-wasm
```

That emits `web/playground/pkg/velin_wasm.js` (a JS shim exporting `init`,
`check`, `run`) plus the `.wasm` binary next to it.

> No `wasm-pack`? Install it once with `cargo install wasm-pack`, or run the
> equivalent `cargo build -p velin-wasm --target wasm32-unknown-unknown` +
> `wasm-bindgen` CLI by hand. The crate itself is a thin, `unsafe`-free string
> wrapper over `velin`; only `wasm-bindgen`'s generated glue is `unsafe`.

## Serve it

WebAssembly modules must be fetched over HTTP (not `file://`). Any static
server works, e.g. from the workspace root:

```sh
python3 -m http.server --directory web/playground 8080
# then open http://localhost:8080
```

## How it talks to the engine

`velin-wasm` exposes exactly two functions, both taking and returning strings:

| Function                     | Returns (JSON)                                             |
| ---------------------------- | --------------------------------------------------------- |
| `check(source)`              | `{ ok, diagnostics: [{ severity, line, column, message }] }` |
| `run(source, repliesJson)`   | `{ ok, diagnostics, output: [string], error? }`           |

Both calls execute in a Web Worker. The page can stop an active request and
terminates a worker that runs for more than five seconds; the next request
starts a fresh worker.

`run` drives a deterministic reference host: `say` appends a line to `output`,
and `ask` prints its prompt then consumes the next entry from the `replies`
JSON array (integers, `true`/`false`, or strings). This mirrors the CLI's line
host without needing interactive stdin. Invalid JSON or an unsupported reply
rejects the entire reply list, so later answers cannot shift to the wrong ask.
