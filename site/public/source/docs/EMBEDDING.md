# Embed Velin in Rust

[简体中文](EMBEDDING.zh-CN.md)

The `velin` facade crate exposes the complete source-to-runtime pipeline. An embedder compiles and checks source, creates a machine, dispatches opaque host effects, and resumes execution with an optional value.

## Add the dependency

While working from this workspace, use a path dependency:

```toml
[dependencies]
velin = { path = "../velin/crates/velin" }
```

The project remains pre-1.0. The `velin` facade, lower-level crates, artifacts, and C ABI have distinct guarantees in the [compatibility policy](COMPATIBILITY.md).

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

Use `compile_modules` when an entry script contains imports or named pure functions. The host owns resolution, so module names never imply filesystem or network access:

```rust
use velin::{ModuleResolver, ResolvedModule, compile_modules};

struct Resolver;
impl ModuleResolver for Resolver {
    fn resolve(&self, _importer: &str, name: &str) -> Result<ResolvedModule, String> {
        match name {
            "math" => Ok(ResolvedModule::new(
                "math",
                "export fn twice(value):\n    return value * 2\n",
            )),
            _ => Err(format!("unknown module `{name}`")),
        }
    }
}

let script = compile_modules(
    "main",
    "import math\nset result = call math.twice(21)\n",
    &Resolver,
)?;
```

Resolution and expansion happen only at compile time. The resulting `CompiledScript` and artifacts contain ordinary validated bytecode and need no resolver at runtime. Import cycles, private cross-module calls, recursive function graphs, effects or randomness inside functions, and module/call budget overruns are compile diagnostics.

For a known host vocabulary, declare `HostCommand` values with `HostSignature::exact` or `HostSignature::variadic`, add them to a `HostSchema`, then call `check_script_with_host_schema`. The optional command description is also consumed by schema-aware LSP completion, signature help, and hover. This adds command-name, arity, argument, binding, and return-flow checks without putting host-specific names into the language.

For value-in/value-out extensions, use `PureModule`. It compiles with an explicit input type map, permits only the `return(value)` and `fail(message)` control signals, rejects all other host commands and `random`/`chance`, and creates a fresh VM state for every invocation:

```rust
use std::collections::BTreeMap;
use velin::{PureModule, Type, Value};

let module = PureModule::compile(
    "reward.velin",
    "perform return(input * 2)\n",
    BTreeMap::from([("input".to_owned(), Type::Integer)]),
)?;
let result = module.invoke(BTreeMap::from([("input".to_owned(), Value::Integer(21))]))?;
assert_eq!(result, Value::Integer(42));
```

`PureModule::invoke` reports missing or unknown inputs, input type mismatches, explicit failures, a missing return, fuel exhaustion, cancellation, and VM execution errors through `PureModuleError`. Use `PureModule::invoke_with_policy` or `PureModule::invoker_with_policy` when the module needs a non-default `ExecutionPolicy`; the reusable invoker restarts its cumulative budget for each invocation.

For most surface-language hosts, prefer `SyncHostDriver` or `AsyncHostDriver`. Registering a handler together with its `HostCommand` makes that declaration drive static checking, ID/name dispatch, and runtime argument/reply validation. Both drivers return the final `Machine`. Use `ScriptRunner` directly when the application needs manual suspension or batching; it validates bytecode, installs defaults, resolves host IDs to names, applies the shared `ExecutionPolicy`, and can apply the same `HostSchema` at runtime.

For consecutive commands without a return value, use `ScriptRunner::run_effect_batch` to collect events before handing them to the host. `HostEventQueue` lives in the host driver and applies event-count, value-count, and text-byte backpressure; a bound command remains a natural barrier handled through `run` / `resume`.

For serializable application DTOs, `to_value` and `from_value` provide Serde conversion with explicit `MarshallingLimits`. They preserve the `i64` integer range and stable record order, reject null and floating-point data, and report the exact failing field or list index. The C ABI retains its original display-only compound tag and adds opt-in `*_json` run/resume/batch/restart functions using `VELIN_VALUE_JSON`, so ABI version 1 hosts can adopt List/Record input and output without changing existing calls.

C hosts can call `velin_execution_policy_default()` and pass a modified
`VelinExecutionPolicy` to the append-only `velin_machine_new_with_policy`
constructor. It covers fuel, call depth, value/machine/host-payload/queue
budgets, and host-effect count; `velin_machine_cancel` and
`velin_machine_clear_cancellation` provide cooperative cancellation between C
calls. The existing `velin_machine_new` keeps default-policy behavior.

Wasm hosts use `run_with_policy`, `PlaygroundSession.new_with_policy`, or the
runtime-only `RuntimeMachine.new_with_policy` with the same JSON field names.
Default constructors remain unchanged, and JavaScript can control cancellation
with `cancel()` and `clear_cancellation()`.

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
