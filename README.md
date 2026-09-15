# Velin

[简体中文](README.zh-CN.md)

Velin is an **embeddable, reproducible bytecode scripting language**. It owns expressions, state, and control flow while returning I/O and domain behavior to the embedding application as opaque host commands.

The name comes from the French *velin* (vellum): scripts write down the rules; the host gives them real-world effects.

## Design principles

- **Deterministic**: there is no floating-point type; random values use an explicit seed, and cloning machine state enables exact replay.
- **Host-agnostic**: the core knows nothing about UI, networking, audio, or business concepts. Every side effect is yielded through `Yield::Host`.
- **Safe Rust**: the core workspace forbids `unsafe` and uses neither JIT compilation nor native code generation.
- **Resource-bounded**: source, expressions, bytecode, values, immediate execution, and host output all have explicit budgets.
- **Complete tooling**: the repository includes a CLI, LSP, VS Code extension, WebAssembly bindings, and a browser Playground.

## Documentation

- [Quick start](docs/QUICKSTART.md) covers the first script, the Playground, and the CLI.
- [FAQ](docs/FAQ.md) answers when to use Velin, how checking and randomness work, and where the trust boundary sits.
- [Roadmap](docs/ROADMAP.md) defines the path to a production-grade deterministic runtime and the capabilities Velin deliberately will not pursue.
- [Playground](docs/PLAYGROUND.md) explains the in-browser editor, replies, and scripted host.
- [Language reference](docs/LANGUAGE.md) documents values, syntax, control flow, host effects, and built-ins.
- [Syntax cheat sheet](docs/CHEATSHEET.md) is a one-page lookup.
- [Values and collections](docs/VALUES.md) covers integers, strings, lists, and records.
- [Examples and recipes](docs/EXAMPLES.md) walk through the shipped samples and copy-paste patterns.
- [Embedding in Rust](docs/EMBEDDING.md) covers compilation, host dispatch, snapshots, and trust boundaries.
- [Host protocol](docs/HOST.md) describes yield/resume, bound commands, and host-side validation.
- [WebAssembly](docs/WASM.md) shows `check` / `run` from JavaScript.
- [Tooling](docs/TOOLING.md) covers the CLI, LSP, VS Code extension, Playground, and CI.
- [Architecture](docs/ARCHITECTURE.md) explains crate boundaries, bytecode validation, static analysis, and resource budgets.
- [Static checking](docs/CHECKING.md) explains types, `Unknown`, and definite assignment.
- [Resource limits](docs/LIMITS.md) lists every published budget.
- [Changelog](CHANGELOG.md) records release-level changes.
- [Security policy](SECURITY.md) explains supported versions and private reporting.

## Workspace

| crate | Responsibility |
| --- | --- |
| `velin-syntax` | Values, expressions, built-ins, source locations, and diagnostics |
| `velin-parse` | Expression tokenization and Pratt parsing |
| `velin-eval` | Reference expression evaluator and deterministic built-ins |
| `velin-bytecode` | Shared bytecode model, validation, wire format, and execution metadata |
| `velin-compile` | Compilation of expressions and control flow to bytecode |
| `velin-vm` | Bytecode execution, state frames, and host-effect yielding |
| `velin-check` | Conservative type inference and definite-assignment analysis |
| `velin-lang` | Indentation-sensitive statement language frontend |
| `velin` | Unified facade crate for embedders |
| `velin-cli` | `velin check` and `velin run` |
| `velin-lsp` | Diagnostics, completion, and document symbols |
| `velin-wasm` | WebAssembly interface for the browser Playground |

The execution pipeline is:

```text
.velin source
    -> parse statements
    -> lower expressions and control flow
    -> static checks
    -> bytecode Program
    -> Machine
    -> Yield::Host <-> host
    -> Finished
```

## Quick start

A complete script:

```text
default balance = 30

label start:
    perform log("balance: [balance]")
    amount = perform read_amount("Adjustment")
    if amount > 0:
        set balance = balance + amount
    else:
        set balance = balance - 1
    jump start
```

The syntax uses four-space indentation and `#` line comments. `label`, `default`, `set`, `perform`, `if`, `elif`, `else`, `while`, and `jump` are statement keywords. A `default` may only appear at the top level, and a name may be declared as a default only once. `log` and `read_amount` are example host commands, not built-in behavior.

Run the included examples:

```sh
cargo run -p velin-cli -- check examples/adventure.velin
cargo run -p velin-cli -- run examples/counting.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

- `velin check [--json] <file|->` compiles source and prints text or structured diagnostics; only errors produce exit code 1.
- `velin run <file|->` executes a checked script with the line-oriented reference host; `say` prints text and `ask` reads one value from stdin. `run -` is only for scripts without `ask`; use a file when host replies are needed.

## Embed in Rust

The facade crate re-exports the full pipeline, so most hosts only need to depend on `velin`:

```rust
use velin::{compile, Machine, Value, Yield};

let script = compile(
    "counter.velin",
    "default count = 1\n\
     perform emit(\"count: [count]\")\n\
     set count = count + 1\n",
)
.unwrap();

let mut machine = Machine::new(script.program.clone()).unwrap();
for (name, value) in &script.defaults {
    machine.set_variable(name, value.clone());
}

match machine.run().unwrap() {
    Yield::Host { host_id, values } => {
        assert_eq!(script.host_name(host_id), Some("emit"));
        assert_eq!(values, vec![Value::String("count: 1".into())]);
        machine.resume(None).unwrap();
    }
    Yield::Finished => {}
}
```

Host commands use one protocol: the VM returns an opaque `host_id` and evaluated arguments; after performing the action, the host calls `resume`, passing `Some(Value)` when the command returns a value.

`CompiledScript` shares immutable bytecode through `Arc<Program>`, so cloning a script or machine snapshot does not copy the whole program. `Machine::new` and `Machine::with_seed` validate chunks, slots, jumps, register definitions, and bytecode budgets before returning a `Result`. Serde deserialization of `Program` runs the same validation, preventing malformed data from reaching execution.

You can also bypass the statement frontend and use `Expr`, `ProgramBuilder`, and `Machine` directly to construct a smaller language subset.

## Values and built-ins

Value types are `Integer(i64)`, `Boolean`, `String`, `List`, and `Record`. Collections use structural sharing and copy-on-write, subject to deterministic data budgets.

Built-ins are `list`, `record`, `get`, `put`, `push`, `remove`, `len`, `contains`, `random(lo, hi)`, and `chance(percent)`.

`random` and `chance` use only RNG state stored in the VM frame. Set the seed through `Machine::with_seed` or `set_rng_seed`; cloning and restoring a `Machine` also rolls back its random sequence.

## Static checking

`velin-check` provides two conservative analyses:

- Type inference covers `Integer`, `Boolean`, `String`, `List`, `Record`, and `Unknown`. Assignment types propagate through the CFG; conflicting branches, loops, and host return values conservatively merge to `Unknown` without speculative false positives.
- Definite-assignment analysis uses CFG must-analysis to report variables that may be read before assignment.

`check_script(file, &script)` runs both analyses in one pass and skips unreachable code. The parser retains precise `Span` values for expression nodes, so diagnostics point to the actual line and column of a variable, operator, or call.

## Resource limits

- A source file is limited to 1 MiB, 10,000 physical lines, and 64 nested statement blocks.
- An expression is limited to 64 KiB, 512 tokens, 32 parenthesis levels, and 32 interpolation levels; nested interpolation shares cumulative work and token budgets.
- A value is limited to 4,096 nodes, 16 collection levels, and 1 MiB of text. `Program` has additional budgets for operations, chunks, slots, constants, text, expression registers, and host arguments.
- The VM executes at most 10,000 immediate steps before yielding. CLI and Playground runs accept at most 1,000 host effects and 1 MiB of output; the Playground additionally limits reply JSON to 1 MiB and worker execution to 5 seconds.
- The LSP limits one JSON-RPC message to 4 MiB, the entire header to 64 KiB, and one header line to 8 KiB.

## Browser Playground

```sh
wasm-pack build --target web --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

Open `http://localhost:8080`. Parsing, checking, and execution happen entirely in the browser; source code is never sent to a backend.

## Documentation site

`site/` is an independent Astro static site. Its build generates English and Chinese documentation from this README, `README.zh-CN.md`, and the architecture sources, then publishes the Wasm Playground alongside them:

```sh
cd site
npm ci
npm run check
npm run build
npm test
```

GitHub Actions checks the Rust workspace and site, then deploys GitHub Pages after updates to the default branch. For project sites, the build derives its base path from `GITHUB_REPOSITORY`; `DOCS_SITE` and `DOCS_BASE` can override it.

## Verification

```sh
cargo test --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo bench -p velin
cargo build --workspace --target wasm32-unknown-unknown
```

Differential tests keep the expression VM aligned with the reference evaluator. Core crates, the statement frontend, and the WebAssembly bindings are verified in the same workspace.
