# WebAssembly

[简体中文](WASM.zh-CN.md)

The language is not “for Wasm”. The core pipeline compiles to WebAssembly so a browser, or any Wasm host, can parse, check, and run the same scripts without a server. `velin-wasm` is a marshalling layer: source string in, JSON out. It does not touch the DOM.

The Playground on this site is one consumer of that package. A game page or editor can load the same `check` / `run` exports.

## check and run

After `wasm-pack` (see [Tooling](TOOLING.md)):

```js
import init, { check, run } from "./pkg/velin_wasm.js";

await init();

const checked = JSON.parse(check(`default n = 1\nperform say(n)\n`));
// { ok: true, diagnostics: [] }

const ran = JSON.parse(run(`perform say("hi")\n`, "[]"));
// { ok: true, diagnostics: [], output: ["hi"] }
```

`check(source)` compiles and statically checks. It returns `{ ok, diagnostics }` where each diagnostic has `severity`, `line`, `column`, and `message`. `ok` is false when any diagnostic is an error.

`run(source, repliesJson)` checks, then executes with a **scripted** host:

| Command | Wasm host |
| --- | --- |
| `say(values...)` | Appends a line to `output` |
| `ask(prompt...)` | Consumes the next JSON value from `repliesJson` |

`repliesJson` is a JSON array of integers, booleans, or strings, for example `[1]` or `["east"]`. Malformed JSON or any unsupported item returns a failed `RunResult` without executing the script. Other command names are printed into `output` and resumed without a value, matching the CLI’s unknown-command behavior.

A `RunResult` also carries `error` when execution fails (overflow, missing index, exhausted replies, fuel budget, or cancellation).

The runtime-only `RuntimeMachine::run_batch(limit)` returns consecutive
side-effect-only host events in one call:

```json
{"kind":"effects","effects":[{"host_id":7,"values":[1]}]}
```

Events preserve source order. Collection stops at a bound host, completion, or
an error. `{"kind":"empty"}` means that this batch had no side-effect-only
events; call `run()` next to distinguish completion from a bound host. `limit`
only bounds VM work; JavaScript still owns queue capacity and payload
backpressure.

## Budgets on this path

- Output is capped at 1 MiB.
- At most 1,000 host effects per run.
- Reply JSON is capped at 1 MiB.
- Immediate VM steps remain 10,000 per `run` / `resume` burst.

## What stays in JavaScript

Wasm evaluates the script. JavaScript still:

- Renders UI and plays audio.
- Decides which command names are allowed.
- Supplies `ask` answers (from a form, from saved state, or from the replies box).
- Applies page-level time and size limits.

That split is the same [host protocol](HOST.md) as on the desktop: the VM yields, the embedder acts, then `resume` continues. The Playground’s scripted host is only a convenience so a demo can run without stdin.

## Building the package

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
```

Handwritten Wasm code is safe Rust. `#[wasm_bindgen]` generates `unsafe` glue, so that crate cannot inherit `unsafe_code = "forbid"`. Core crates keep the prohibition.

## A minimal page

```html
<script type="module">
  import init, { check, run } from "./pkg/velin_wasm.js";
  await init();

  const source = `default hp = 30
choice = perform ask("Drink?")
if choice == 1:
    perform say("restored")
`;

  console.log(check(source));
  console.log(run(source, "[1]"));
</script>
```

Replace `console.log` with your UI. Keep `say` rendering and `ask` prompting in JavaScript; do not add those APIs to the VM.

The site Playground is this pattern with an editor. Walk through it in [Playground](PLAYGROUND.md).
