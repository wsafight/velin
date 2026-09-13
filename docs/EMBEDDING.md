# Embed Velin in Rust

[简体中文](EMBEDDING.zh-CN.md)

The `velin` facade crate exposes the complete source-to-runtime pipeline. An embedder compiles and checks source, creates a machine, dispatches opaque host effects, and resumes execution with an optional value.

## Add the dependency

While working from this workspace, use a path dependency:

```toml
[dependencies]
velin = { path = "../velin/crates/velin" }
```

The project is in its pre-stable `0.x` line and does not currently promise backward compatibility for its Rust API or serialized program format.

## Compile and check

`compile` returns a `CompiledScript` containing shared bytecode, interned host names, labels, and compile-time defaults. Parsing or lowering failures are returned immediately. Static analysis is a separate step so callers can choose their diagnostic policy.

```rust
use velin::{check_script, compile};

let source = std::fs::read_to_string("rules.velin")?;
let script = compile("rules.velin", &source)?;
let diagnostics = check_script("rules.velin", &script);

for diagnostic in &diagnostics {
    eprintln!("{diagnostic}");
}
if diagnostics.iter().any(velin::Diagnostic::is_error) {
    return Err("script did not pass static checks".into());
}
```

`check_script` runs CFG-aware type propagation and definite-assignment analysis. It does not mutate the program.

For a known host vocabulary, build a `HostSchema` from `HostSignature::exact` or `HostSignature::variadic`, then call `check_script_with_host_schema`. This adds command-name, arity, argument, binding, and return-flow checks without putting host-specific names into the language.

For most surface-language hosts, prefer `ScriptRunner`: it validates bytecode, installs defaults, resolves host IDs to names, enforces a cumulative host-effect budget, and can apply the same `HostSchema` at runtime. Use `Machine` directly when an embedder needs lower-level control.

For consecutive commands without a return value, use `ScriptRunner::run_effect_batch` to collect events before handing them to the host. `HostEventQueue` lives in the host driver and applies event-count, value-count, and text-byte backpressure; a bound command remains a natural barrier handled through `run` / `resume`.

## Create a machine

`Machine::new` validates the complete program before execution. Apply every script default to the new frame:

```rust
use velin::Machine;

let script = velin::compile("inline.velin", "default score = 0\n")?;
let mut machine = Machine::new(script.program.clone())?;
for (name, value) in &script.defaults {
    assert!(machine.set_variable(name, value.clone()));
}
```

Use `Machine::with_seed(program, seed)` when scripts call `random` or `chance` and the host needs an explicit replay seed. `Machine::new` uses seed `0`.

## Drive host effects

`run` advances until the next host effect or completion. Look up the command name through `CompiledScript::host_name`, perform the behavior in the embedding application, then call `resume`.

```rust
use velin::{Value, Yield};

let script = velin::compile(
    "inline.velin",
    "perform notify(\"ready\")\n",
)?;
let mut machine = velin::Machine::new(script.program.clone())?;
let mut outcome = machine.run()?;
loop {
    match outcome {
        Yield::Finished => break,
        Yield::Host { host_id, values } => {
            let name = script.host_name(host_id).ok_or("unknown host id")?;
            let reply = match name {
                "notify" => {
                    println!("{}", values[0].try_to_display()?);
                    None
                }
                "read_score" => Some(Value::Integer(42)),
                other => return Err(format!("unsupported effect: {other}").into()),
            };
            outcome = machine.resume(reply)?;
        }
    }
}
```

Call `resume(None)` for an unbound `perform`. A command written as `answer = perform read_score()` requires `Some(Value)`. Calling `run` while an effect is pending, or calling `resume` with no pending effect, is an error.

## Treat the host as a trust boundary

Velin never performs I/O itself. The host should still validate:

- Whether a command name is allowed in the current context.
- Argument count, types, ranges, and application permissions.
- The returned value before resuming a bound command.
- Time, output, network, storage, and other external budgets.

Every value passed into `set_variable` or `resume` is checked against Velin's deterministic data limits.

## Snapshot and replay

`Machine` implements `Clone`. A clone captures the program counter, variables, pending host request, completion state, and RNG progress while sharing immutable bytecode through `Arc<Program>`.

```rust
let script = velin::compile(
    "inline.velin",
    "set die = random(1, 6)\n",
)?;
let mut machine = velin::Machine::with_seed(script.program.clone(), 7)?;
let checkpoint = machine.clone();
let first = machine.run();

machine = checkpoint;
let replay = machine.run();
assert_eq!(format!("{first:?}"), format!("{replay:?}"));
```

`Machine` snapshots are in-memory values, not a stable persistence format. If an application serializes `Program`, deserialization re-runs structural validation before the program can execute.

## Lower-level APIs

Embedders that do not need the statement language can use `parse_expression`, `check_expression`, `compile_expression`, `ProgramBuilder`, and `Machine` directly. `velin-eval` remains the tree-walking reference semantics and is useful for differential testing.
