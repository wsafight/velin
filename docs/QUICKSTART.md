# Quick start

[简体中文](QUICKSTART.zh-CN.md)

Velin is a small scripting language for **rules, state, and control flow**. The script never opens files, draws UI, or talks to the network. It yields a host command; your application decides what that command means.

This page is the first reading. You can try a script in the browser with no install, then run the same file from the CLI.

## Design principles

- **Deterministic**: no floating-point type; random values use an explicit seed; cloning a machine replays exactly.
- **Host-agnostic**: the core does not know about UI, networking, or business concepts. Every side effect is `perform`.
- **Resource-bounded**: source, values, bytecode, and immediate VM steps each have a budget.
- **One pipeline**: the CLI, editor, Rust host, and browser Playground share the same parser, checker, compiler, and VM.

Rust is the implementation and one way to embed. The language itself is for writing rules. The same semantics also run in the browser through WebAssembly.

## Try it in the browser

Open the [Playground](PLAYGROUND.md). The sample script is already loaded.

1. Leave `ask` replies as `[1]` and click **Run**.
2. Output should include a restored HP of `40`.
3. Change replies to `[0]` and run again to take the other branch.
4. Click **Check only** after an edit to see diagnostics without executing.

Source never leaves the tab. Details: [Playground](PLAYGROUND.md) and [WebAssembly](WASM.md).

## Run the shipped examples

From the repository root, with Rust 1.88 or newer:

```sh
cargo run -p velin-cli -- check examples/counting.velin
cargo run -p velin-cli -- run examples/counting.velin
cargo run -p velin-cli -- check examples/adventure.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

- `check` prints diagnostics. Only errors set exit status 1.
- `run` uses the line-oriented reference host: `say` prints, `ask` reads one stdin value. `run -` cannot combine stdin source with `ask`; use a source file for interactive scripts.

`say` and `ask` are host conventions, not keywords. A game can bind `open_door` or `show_menu` instead. See [Host protocol](HOST.md).

## A first script

Four-space indentation. `#` starts a comment. `default` is a compile-time constant at the top level.

```velin
default hp = 30

label start:
    perform say("You wake in a cold cell.")
    choice = perform ask("Drink a potion? (1 = yes)")
    if choice == 1:
        set hp = hp + 10
        perform say("HP is now [hp]")
    else:
        perform say("You wait.")
```

What the language owns:

| Piece | Role |
| --- | --- |
| `default` / `set` | State |
| `if` / `while` / `label` / `jump` | Control flow |
| `perform` | The only way out to the host |
| `list`, `record`, `random`, … | Built-in values, not I/O |

What the host owns: printing, prompts, UI, files, time, permissions.

Copy more patterns from [Examples](EXAMPLES.md). Keep the [syntax cheat sheet](CHEATSHEET.md) nearby.

## What happens at runtime

```text
.velin source
    -> parse
    -> check (types, definite assignment)
    -> bytecode
    -> Machine
    -> Yield::Host  <->  your app
    -> Finished
```

The VM suspends on `perform`. After the host acts, it calls `resume`. A loop with no `perform` stops after 10,000 immediate steps.

## Where to go next

1. Write more scripts: [Language reference](LANGUAGE.md), [Values](VALUES.md), [Examples](EXAMPLES.md).
2. Embed: [Rust](EMBEDDING.md) or [WebAssembly](WASM.md), then the [host protocol](HOST.md).
3. Tools: [CLI, editor, and web](TOOLING.md).
4. Boundaries: [FAQ](FAQ.md), [Architecture](ARCHITECTURE.md), [Limits](LIMITS.md).
