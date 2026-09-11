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

The core stays host-agnostic, but an embedder can supply a `HostSchema` at its boundary. A strict schema reports undeclared commands and checks arity, argument types, whether a bound command returns a value, and the type propagated from that return value.

```rust
use velin::{HostSchema, HostSignature, Type, check_script_with_host_schema};

let schema = HostSchema::new()
    .command(
        "say",
        HostSignature::variadic(Vec::new(), Type::Unknown, None),
    )
    .command(
        "ask",
        HostSignature::exact(vec![Type::String], Some(Type::Integer)),
    );

let diagnostics = check_script_with_host_schema("rules.velin", &script, &schema);
```

Use `.allow_unknown(true)` when a tool intentionally models only part of a host vocabulary. `ScriptRunner::configured` accepts the same schema for runtime argument and reply validation. A reply type error can be corrected and retried; an invalid call or exhausted effect budget is terminal for that runner.

## CLI and Playground conventions

The line-oriented reference host implements two names so examples can run without a custom embedder:

| Command | CLI | Playground |
| --- | --- | --- |
| `say(values...)` | Writes values to stdout, separated by spaces | Appends them to the output panel |
| `ask(prompt...)` | Writes the prompt, reads one line, parses an integer, boolean, or string | Consumes the next JSON value from the replies array |

CLI and Playground runs accept at most 1,000 host effects and 1 MiB of output. The Playground also caps reply JSON at 1 MiB. These caps are tooling limits, not language semantics.

## A minimal Rust host

```rust
use velin::{ScriptRunner, ScriptYield, Value, compile};

fn reply_for(name: &str, values: &[Value]) -> Result<Option<Value>, String> {
    match name {
        "say" => {
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                print!("{}", value.try_to_display().map_err(|e| e.to_string())?);
            }
            println!();
            Ok(None)
        }
        "ask" => Ok(Some(Value::Integer(1))),
        other => Err(format!("unsupported effect: {other}")),
    }
}

fn drive(source: &str) -> Result<(), String> {
    let script = compile("rules.velin", source).map_err(|e| e.to_string())?;
    let mut runner = ScriptRunner::new(&script).map_err(|e| e.to_string())?;
    let mut outcome = runner.run().map_err(|e| e.to_string())?;
    loop {
        match outcome {
            ScriptYield::Finished => return Ok(()),
            ScriptYield::Host { name, values } => {
                let reply = reply_for(&name, &values)?;
                outcome = runner.resume(reply).map_err(|e| e.to_string())?;
            }
        }
    }
}
```

Replace `reply_for` with UI, networking, or storage. Keep that logic out of the script.

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
