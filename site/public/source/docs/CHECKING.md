# Static checking

[简体中文](CHECKING.zh-CN.md)

`velin check` runs after parsing and compilation. It does not change bytecode. The checker is conservative: it reports a problem only when it can prove one, so valid host-driven scripts are not rejected on speculation.

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

The editor LSP and the Playground `check` button use the same analyses.

## What the checker proves

Two independent passes share one `Diagnostic` type:

1. **Type inference** over `Integer`, `Boolean`, `String`, `List`, `Record`, and `Unknown`.
2. **Definite assignment**: a variable may be read only if every reachable predecessor assigns it.

Unreachable code is not checked. Diagnostics use 1-based line and column and point at the expression node.

## Types and Unknown

Defaults establish entry types. Assignments propagate along the control-flow graph. A merge keeps a concrete type only when every incoming path agrees.

These become `Unknown` without a false-positive error:

- A value returned by a bound `perform`.
- Conflicting branch types (`1` on one path, `"x"` on another).
- Loop back edges that would otherwise need a speculative type.

`Unknown` is compatible with every context. A later use that is definitely wrong (`not "x"`) is still reported. A later use that might be valid for a host return value is not.

Conditions of `if` and `while` report a known non-boolean. `Unknown` is allowed there because the host may return a boolean.

## Definite assignment

`default` names are assigned at entry. A merge treats a variable as assigned only when every reachable predecessor assigns it.

```velin
if ready:
    set score = 1
perform say(score)
```

If `ready` can be false, `score` is not assigned on every path and `check` reports the read.

## Errors versus warnings

Only diagnostics classified as errors fail `velin check` (exit status 1). Warnings alone do not. Hosts that call `check_script` should refuse to `run` when any diagnostic `is_error()`.

The checker never inserts runtime traps and never alters program counters. A script that passes can still fail at runtime: overflow, missing list indexes, and host-supplied values are outside what static analysis proves.

## What it will not catch

- Whether a host command name is implemented.
- Argument count and types of a host command (names are opaque).
- Infinite loops that `perform` (those yield; the immediate fuel budget stops loops that never yield).
- Values that become illegal only after the host resumes.

Those belong to the [host protocol](HOST.md) and [resource limits](LIMITS.md).
