# FAQ

[简体中文](FAQ.zh-CN.md)

Short answers to the questions that come up when embedding Velin, writing scripts, or comparing it with other languages.

## What Velin is for

Velin is a small scripting language for hosts that need **rules, state, and control flow** without giving the script the rest of the application.

Typical hosts are:

- Interactive fiction, dialogue, and quests.
- Game or product rule engines that must stay replayable.
- User-supplied add-ons that have to fail inside a budget.
- Browser demos that should parse, check, and run without a server.

The script never opens files, draws UI, or talks to the network. It yields a host command; the embedding application decides what that command means.

## When not to use it

Choose something else when you need:

- General-purpose programming with modules, classes, or a large standard library.
- Floating-point math, clocks, or operating-system APIs inside the script.
- A stable public language and Rust API. Velin `0.x` is still pre-stable.

Lua, Rhai, JavaScript, and WASM guests are better fits for open-ended extension. Velin is the narrower tool: deterministic values, conservative checks, and one host-effect protocol.

## Why there is no floating-point type

Integer arithmetic is checked and platform-independent. Floating-point would make equality, display, snapshots, and cross-target replay depend on rounding modes and CPU details.

If a host needs real-world units, keep them outside Velin: convert to integers (cents, millimeters, milliseconds) before the script sees them, or handle the conversion in the host command.

## Is Velin a Rust language?

No. Velin is a scripting language for rules. The runtime is written in Rust, and one embedding path is the `velin` crate, but scripts are not Rust and do not target Rust programmers.

The same pipeline also compiles to WebAssembly. A page can call `check` and `run` from JavaScript. See [WebAssembly](WASM.md).

## Are `say` and `ask` language keywords?

No. They are conventions used by the CLI reference host and the browser Playground. The language only has `perform`. Command names are interned as opaque `host_id` values.

A game host can bind `perform open_door("east")` or `choice = perform show_menu(options)` without teaching Velin what a door or a menu is.

## Why indentation uses four spaces

The statement frontend rejects tabs and requires exactly four spaces per level so that nesting is unambiguous in editors, diffs, and diagnostics. There is no optional brace form.

## How random numbers replay

`random(low, high)` and `chance(percent)` read a PRNG stored in the machine frame. `Machine::new` uses seed `0`; `Machine::with_seed` sets an explicit seed. Cloning a machine copies the remaining random sequence, so a snapshot also rewinds future rolls.

There is no access to system entropy, wall clocks, or thread-local random state.

## Can untrusted scripts take over the host?

They cannot perform I/O themselves, and every layer has a budget: source size, AST depth, values, bytecode, cumulative fuel, immediate fuel, host effects, and queue payloads. A loop with no `perform` stops after the default 10,000 immediate fuel units. Hosts can override these limits with `ExecutionPolicy`.

The host is still the trust boundary. It must allow-list command names, check arguments, and apply its own time, output, and permission limits. See [Host protocol](HOST.md).

## Can I save a running script?

`Machine` implements `Clone`. A clone is an in-memory snapshot of the program counter, variables, pending host effect, completion flag, RNG, execution policy, cumulative fuel, host-effect count, and cancellation state. Immediate fuel belongs to one call and is not stored between calls; the cumulative counters are preserved by the clone. It is not a stable on-disk format.

If you serialize a `Program`, deserialization validates the bytecode before it can run. Do not treat today's snapshot layout as a long-term save format until the project makes a compatibility promise.

## Does it run in the browser?

Yes. `velin-wasm` exposes `check` and `run` to JavaScript. The site Playground loads that package and never sends source to a server. The same parser, checker, compiler, and VM run on the desktop CLI.

## Is the language stable?

Not yet. Version `0.5.1` is the current release, but syntax, the serialized program format, and the Rust API may still change during the `0.x` line. The architectural rules in [Architecture](ARCHITECTURE.md) describe how the project intends to evolve: no VM I/O, deterministic values, conservative checking, and host effects for every external action.

## Where should I start?

1. Run the samples in [Examples and recipes](EXAMPLES.md) or the [Playground](PLAYGROUND.md).
2. Keep the [syntax cheat sheet](CHEATSHEET.md) nearby; read the [language reference](LANGUAGE.md) when a construct is unclear.
3. Embed with the [Rust guide](EMBEDDING.md), [WebAssembly](WASM.md), and the [host protocol](HOST.md).
4. Use the [tooling guide](TOOLING.md) for the CLI and LSP.
