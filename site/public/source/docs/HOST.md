# Host protocol

[简体中文](HOST.zh-CN.md)

Velin never performs I/O. The only contact with the outside world is `perform`, which becomes `Yield::Host`. This page describes that protocol: how a command suspends, what the host must return, and where the trust boundary sits.

The [embedding guide](EMBEDDING.md) shows the Rust types. This page focuses on the contract a host has to uphold.

## The yield and resume cycle

Compilation interns every distinct command name to a `u32` `host_id`. At runtime the VM:

1. Evaluates every argument expression.
2. Advances the program counter to the next instruction.
3. Remembers whether the command is bound to a variable.
4. Returns `Yield::Host { host_id, values }`.

The host looks up the name with `CompiledScript::host_name`, performs the action in application code, then calls `Machine::resume`.

```text
script:  answer = perform ask("Continue?")
                |
                v
VM:      Yield::Host { host_id, values: ["Continue?"] }
                |
                v
host:    show a prompt, read input, produce a Value
                |
                v
VM:      resume(Some(value)) -> store in `answer` -> continue
```

The program counter moves **before** the effect. Resuming cannot repeat the external action. Calling `run` while an effect is pending, or `resume` with no pending effect, is an error.

## Batching side-effect-only events

Consecutive unbound `perform` commands can be collected with
`Machine::run_effect_batch_reusable` or `ScriptRunner::run_effect_batch`. The
VM preserves source order and stops at a bound effect, an error, completion, or
the batch limit; it never crosses a host barrier that needs `resume`.
If an error occurs after one or more effects have been collected, the batch
returns that valid prefix first; the next execution call reports the pending
error. Restarting the machine clears the pending error.

The batch API only bounds VM work. The embedder owns the persistent queue and
consumer pacing; `HostEventQueue` can enforce event-count, value-count, and text
byte limits. When the queue is full or its payload budget is exhausted, pause
the VM driver until the host consumes events. This prevents dropped or
duplicated effects.

## Bound and unbound commands

```velin
perform log("started")
answer = perform ask("Continue?")
```

| Form | Resume argument | Typical use |
| --- | --- | --- |
| Unbound `perform name(...)` | `None` | Logs, UI updates, fire-and-forget actions |
| Bound `name = perform name(...)` | `Some(Value)` | Prompts, queries, host-owned data |

Passing `None` to a bound command is an error. The low-level `Machine` ignores a value supplied to an unbound command; hosts should conventionally pass `None`, and a `ScriptRunner` with a schema rejects replies for commands declared as non-returning. Return values are checked against Velin's data budget before they enter a slot.

## Names are opaque

`say`, `ask`, `log`, `unlock`, and any other identifier are not keywords. The compiler does not know their arity or meaning. Two hosts can give the same source different behavior:

| Command in source | CLI reference host | A game host |
| --- | --- | --- |
| `perform say("Hi")` | Print to stdout | Show a dialogue line |
| `choice = perform ask("Go?")` | Read one stdin line | Open a menu, return `1` or `0` |
| `perform open_door("east")` | Print the name and arguments, resume with no value | Animate a door if the player holds a key |

Unknown commands are still valid Velin. The CLI prints them and continues; a production host should reject names it did not allow-list.

## Declare host contracts

The core stays host-agnostic, but an embedder can supply a `HostSchema` at its boundary. A strict schema reports undeclared commands and checks arity, argument types, whether a bound command returns a value, and the type propagated from that return value. `HostCommand` adds the command name and optional documentation to that signature, making one declaration available to the checker, runtime drivers, and LSP.

```rust
use velin::{HostCommand, HostSchema, HostSignature, Type, check_script_with_host_schema};

let schema = HostSchema::new()
    .declare(HostCommand::new(
        "say",
        HostSignature::variadic(Vec::new(), Type::Unknown, None),
    ).description("Write values to the conversation log."))
    .declare(HostCommand::new(
        "ask",
        HostSignature::exact(vec![Type::String], Some(Type::Integer)),
    ).description("Ask the player to choose an option."));

let diagnostics = check_script_with_host_schema("rules.velin", &script, &schema);
```

Use `.allow_unknown(true)` when a tool intentionally models only part of a host vocabulary. `ScriptRunner::configured` accepts the same schema for runtime argument and reply validation. `Server::with_host_schema` exposes its signatures and documentation through LSP diagnostics, completion, signature help, and hover. A reply type error can be corrected and retried; an invalid call or exhausted effect budget is terminal for that runner.

`PureModule` uses a strict private schema with two terminating commands: `return(value)` produces the module result and `fail(message)` produces a controlled failure. No other `perform` command is accepted, and modules containing `random` or `chance` are rejected at compile time. Each `PureModule::invoke` starts from a fresh initial frame, so values and execution state never carry across calls.

## CLI and Playground conventions

The line-oriented reference host implements two names so examples can run without a custom embedder:

| Command | CLI | Playground |
| --- | --- | --- |
| `say(values...)` | Writes values to stdout, separated by spaces | Appends them to the output panel |
| `ask(prompt...)` | Writes the prompt, reads one line, parses an integer, boolean, or string | Consumes the next JSON value from the replies array |

CLI and Playground runs accept at most 1,000 host effects and 1 MiB of output. The Playground also caps reply JSON at 1 MiB. These caps are tooling limits, not language semantics.

## A schema-backed Rust host

```rust
use velin::{HostCommand, HostSignature, SyncHostDriver, Type, Value, compile};

fn drive(source: &str) -> Result<(), String> {
    let script = compile("rules.velin", source).map_err(|e| e.to_string())?;
    let mut host = SyncHostDriver::<String>::new()
        .command(
            HostCommand::new(
                "say",
                HostSignature::variadic(Vec::new(), Type::Unknown, None),
            ).description("Write values to the conversation log."),
            |values| {
                let rendered = values.iter()
                    .map(Value::try_to_display)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(str::to_owned)?;
                println!("{}", rendered.join(" "));
                Ok(None)
            },
        )
        .command(
            HostCommand::new(
                "ask",
                HostSignature::exact(vec![Type::String], Some(Type::Integer)),
            ).description("Ask the player to choose an option."),
            |_| Ok(Some(Value::Integer(1))),
        );
    host.run("rules.velin", &script)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
```

`AsyncHostDriver` provides the same registration and checks for handlers that return futures. Both drivers reject static schema errors before dispatch and return the completed `Machine` so the host can inspect final state.

## Serde value marshalling

`to_value` and `from_value` convert serializable DTOs without adding native objects to the VM. Every conversion applies explicit `MarshallingLimits` for node count, collection depth, and UTF-8 bytes. Records use stable key ordering, integers must fit `i64`, and failures identify the exact field or list index. JSON `null` and floating-point numbers are rejected because Velin has no matching value type.

## The host is the trust boundary

Velin guarantees that the core does not touch the filesystem, network, clock, or UI. It does not authorize host behavior. A host should validate:

- Whether the command name is allowed in the current context.
- Argument count, types, ranges, and application permissions.
- Any value returned through `resume` or `set_variable`.
- Time, output size, network, and storage budgets of its own.

Every value crossing into the machine is checked against the deterministic data limits (4,096 nodes, 16 collection levels, 1 MiB of text). A host that injects an oversized string still fails inside Velin; a host that writes the same string to disk is on its own.

## Snapshots and replay

`Machine::clone` captures execution progress without copying bytecode (`Arc<Program>`). Use it for undo, branching previews, or deterministic tests.

```rust
let checkpoint = machine.clone();
let _ = machine.run();
machine = checkpoint; // RNG, variables, and PC rewind together
```

This is an in-memory value, not a persistence format. Serialized `Program` values are re-validated on deserialization so malformed bytecode cannot reach `run`.

## What belongs in the script

Keep in Velin:

- Integers, booleans, strings, lists, and records.
- Conditions, loops, labels, and jumps.
- Seeded `random` / `chance`.

Keep in the host:

- Widgets, audio, files, HTTP, databases.
- Clocks, identity, permissions, and pricing.
- Floating-point units and display formatting beyond Velin's deterministic `Display`.

If a new capability needs the outside world, add a host command. Do not add it to the VM.
