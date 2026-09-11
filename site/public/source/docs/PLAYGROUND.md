# Playground

[简体中文](PLAYGROUND.zh-CN.md)

The site Playground parses, checks, and runs `.velin` in this browser. It loads `velin-wasm`. Source and `ask` replies never go to a server.

Open it from the header, or from `/playground/` on this documentation site.

## Open the site Playground

Open `/playground/` from the header. The sample is a shortened `adventure.velin`.

1. Confirm **ask replies** is `[1]`.
2. Click **Run**. Diagnostics should stay empty; output should mention a restored HP of `40`.
3. Set replies to `[0]` and run again. The script takes the other branch.
4. Introduce a type error (for example `if 1:`) and click **Check only** to see a diagnostic without executing.

The page has three surfaces:

| Surface | Role |
| --- | --- |
| Script | A textarea of Velin source. The sample is a shortened `adventure.velin`. |
| Diagnostics | Parser, type, and definite-assignment messages, or “No problems.” |
| Output | Lines produced by `say` (and unknown commands) after **Run**. |

Buttons:

- **Run** checks, then executes. Errors in the checker skip execution.
- **Check only** compiles and analyses without running.

The `ask` replies field is a JSON array consumed in order. The sample drinks the potion when the field is `[1]`. Use `[0]` to take the other branch. Strings and booleans are also valid entries: `["east"]`, `[true]`.

Language follows the site (`?lang=zh` or the header toggle). Theme follows `velin-theme` in `localStorage`.

## Limits in the browser

A run stops after 1,000 host effects or 1 MiB of output. Reply JSON is capped at 1 MiB. Immediate VM steps stay at 10,000 between yields. See [Resource limits](LIMITS.md) and [WebAssembly](WASM.md).

## Local copy

To serve the same UI from the repository:

```sh
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

Then open `http://localhost:8080`. The documentation site copies that folder to `/playground/` during its build.

## What the Playground host is

It is the scripted reference host, not a full embedder. `say` appends output; `ask` reads the JSON array; other names are logged and resumed with no value. A product host should still allow-list commands and supply real UI. That contract is in [Host protocol](HOST.md).
