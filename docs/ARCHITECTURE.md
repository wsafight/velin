# Velin architecture

[简体中文](ARCHITECTURE.zh-CN.md)

Velin is an embeddable deterministic scripting language. It provides a complete source-to-execution pipeline without owning domain-specific behavior: scripts emit host effects, and the embedding application interprets those effects before resuming the virtual machine.

This document describes only Velin's design, boundaries, and stability constraints.

## 1. Goals

Velin's core goal is to let different kinds of hosts share one set of expression, state, and control-flow semantics:

- Produce reproducible results on desktop, server, and WebAssembly targets.
- Connect UI, networking, storage, or business actions through one host protocol.
- Provide structured syntax and static-checking diagnostics before execution.
- Keep the implementation testable and serializable while bounding resource use by untrusted scripts.

Explicit non-goals:

- No operating-system, filesystem, network, or clock APIs.
- No commands tied to a particular application domain.
- No floating-point values, system entropy, JIT, or native code generation.
- No promise of complete source-level typing; static checking is deliberately conservative.

## 2. Overall pipeline

```text
source (.velin)
  |
  v
velin-lang          statements + embedded expressions
  |
  +----> velin-check          diagnostics
  |
  v
velin-compile       source lowering and ProgramBuilder
  |
  v
velin-bytecode      Program + validation + execution plans
  |
  v
velin-vm            Machine state
  |
  +----> Yield::Host { host_id, values }
  |                    |
  |                 host performs effect
  |                    |
  +<--- Machine::resume(optional value)
  |
  v
Yield::Finished
```

Expressions also have a reference path: `velin-parse -> velin-eval`. It shares value and built-in semantics with the bytecode path, and differential tests compare both values and errors.

## 3. Crate boundaries

| crate | Ownership boundary |
| --- | --- |
| `velin-syntax` | Shared data model: `Value`, `Expr`, operators, `Span`, and `Diagnostic` |
| `velin-parse` | Expression source to AST; does not handle statement control flow |
| `velin-eval` | Tree-walking reference evaluator and built-in semantics |
| `velin-bytecode` | Stable expression/program bytecode model, slot table, validation, wire format, and derived execution plans |
| `velin-compile` | Source expression lowering, constant propagation, and `ProgramBuilder` |
| `velin-vm` | Runtime state, expression stack machine, control-flow loop, and host suspension protocol |
| `velin-check` | Type inference, condition checking, and definite-assignment analysis |
| `velin-lang` | Indentation-sensitive statement AST, parsing, lowering, and host-name interning |
| `velin` | Re-exports the stable embedding API without implementing new semantics |
| `velin-cli` | Line-oriented reference host and command-line interface |
| `velin-lsp` | Editor protocol adapter that does not duplicate parsing or checking logic |
| `velin-wasm` | String/JSON marshaling and the browser reference host |

Lower-level crates never depend back on higher-level tools. Domain behavior belongs only in a host and must not enter syntax, compile, eval, or vm.

## 4. Source language

### 4.1 Expressions

Expressions use a Pratt parser and support:

- `i64` integers, booleans, strings, lists, and records.
- Unary `-` and `not`.
- Arithmetic, comparisons, equality, and short-circuit `and` / `or`.
- Built-in function calls.
- String interpolation such as `"balance: [balance]"`; `[[` represents a literal `[`.

Expression depth, token count, and data size are budgeted. The parser wraps every node in `Expr::Spanned` with an exact `Span`; errors use 1-based line and column positions through `Diagnostic`. Hand-built ASTs may omit spans, in which case the checker falls back to the caller-provided location.

### 4.2 Statements

The statement frontend uses four-space indentation and supports:

```text
default name = constant-expression
set name = expression
name = expression
perform effect(arguments)
name = perform effect(arguments)
if condition:
elif condition:
else:
while condition:
label name:
jump name
```

`default` may only appear at the top level, and a name may be declared only once. Its value is evaluated at compile time and cannot reference runtime variables. Host command names are not reserved words: the compiler stably interns each name to a `u32 host_id`, while `CompiledScript` retains the ID-to-name lookup.

## 5. Values and determinism

`Value` contains only:

```text
Integer(i64)
Boolean
String
List
Record
```

There is no floating-point value, so numeric calculations do not depend on platform floating-point implementations or rounding modes. Records use an order-stable data structure, making display and serialization order reproducible.

Strings, lists, and records use `Arc` sharing. Collection and string updates are copy-on-write: a uniquely owned slot can reuse its allocation, while aliases and `Machine::clone` checkpoints continue to observe the old value. Every value entering runtime state is checked against the data budget, preventing a host from bypassing source limits by injecting unbounded data.

### 5.1 Random numbers

`random(lo, hi)` and `chance(percent)` use a pure integer PRNG. Random state lives in a reserved machine-frame slot rather than thread-local or global state.

This gives three properties:

1. The same program, inputs, and seed produce the same results.
2. `Machine::clone` preserves random progress.
3. Restoring an older machine state also rewinds the following random sequence.

Invalid random arguments fail before advancing state.

## 6. Bytecode

### 6.1 Expression bytecode

Each `ExprChunk` contains a constant pool, source line numbers, and a flat `Vec<ExprOp>`. The main operations are:

```text
Const / Load
Unary / Binary
Call
Random / Chance
JumpIfFalse / JumpIfTrue
Concat
```

Variables resolve to `u32` slots during compilation, so runtime performs no string lookup. `and` and `or` compile to conditional jumps to preserve short-circuit semantics; string interpolation evaluates each part and finishes with one `Concat`.

Pure constant subtrees are evaluated by the reference evaluator during compilation. Failed candidates, including overflow, division by zero, and type errors, remain as bytecode so runtime error behavior is unchanged. Bytecode source coordinates use compact `u32` values, and completed op and constant vectors are trimmed to their actual length.

### 6.2 Control-flow bytecode

`Program` keeps its control-flow operations small:

```text
Set { slot, value }
Update { slot, operation, line, column }
Jump(pc)
JumpIfFalse { condition, target }
Host(HostOp { host_id, args, bind, line })
Halt
```

Control-flow operations reference only expression chunks, slots, and program counters. `Update` is emitted for ownership-aware forms such as `items = push(items, value)` and `count = count + 1`; it preflights type, index, per-value, and machine budgets before taking the destination value, so failure leaves the slot unchanged. The cold, variable-sized `HostOp` payload is boxed so it does not widen every hot instruction. On 64-bit targets this keeps `Op` at 32 bytes instead of 48 and `ExprOp` at 12 bytes instead of 16 without changing the JSON representation. `Program` also owns the `SlotTable`, used for initialization, debugging, and name-based state access through the public API.

### 6.3 Validation boundary

`Program::validate` checks control-flow targets, chunk and slot indexes, forward expression jumps, stack height along every path, built-in arity, and bytecode/constant budgets. `ExprChunk::validate(slot_count)` gives direct expression-VM users the same local guarantee.

`Program` deserialization through Serde validates automatically and rejects duplicate slot names. `Machine::new` and `Machine::with_seed` still validate hand-built programs and return `Result`. Public `eval_chunk` validates standalone chunks; a constructed `Machine` owns an `Arc<Program>` and takes an internal prevalidated path instead of scanning bytecode before every expression.

## 7. Host protocol

Host effects are Velin's only boundary with the outside world.

When the VM reaches `Op::Host`, it:

1. Evaluates every argument expression.
2. Advances the program counter to the next instruction.
3. Saves the optional return-value destination slot.
4. Returns `Yield::Host { host_id, values }`.

After completing the effect, the host calls `Machine::resume(value)`:

- Effects with no return value use `None`.
- Effects with a `bind` must return `Some(Value)`.
- Return values are checked against the data budget before entering the destination slot.

Calling `run` again while the machine is waiting for the host, or calling `resume` without a suspended effect, returns an explicit error. The PC advances before the effect occurs, ensuring that resumption cannot repeat an external action.

A host must validate commands, arguments, and permissions against its own trust boundary. Velin guarantees that the language core does not access external resources itself, but it cannot manage authorization for host behavior.

## 8. Static checking

The type domain is:

```text
Integer / Boolean / String / List / Record / Unknown
```

Type inference reports a diagnostic only when an error can be proven. `Unknown` is compatible with every context so valid runtime behavior is not rejected. Defaults establish entry types, and assignments propagate along the control-flow graph. A merge keeps a concrete type only when all incoming paths agree; conflicting branches, loop back edges, and host bindings conservatively become `Unknown`. Unreachable expressions are not checked, while condition expressions separately report known non-boolean values.

Definite-assignment checking is a CFG must-analysis: a variable is considered assigned at a merge only when every reachable predecessor assigns it. `default` declarations contribute to the entry state.

Static checking never changes bytecode or runtime behavior. The CLI and other hosts may choose how to display warnings, but errors should prevent execution.

## 9. Execution and resource limits

`Machine` holds an immutable `Program` through `Arc` and owns an independent variable frame, program counter, suspended effect, completion flag, and reusable expression stack. Construction is fallible; only a validated program can enter runtime state. Frame slots retain their measured data footprints, and expression/built-in evaluation returns metrics with its value, avoiding a second recursive resource scan during assignment or host-payload accounting.

Each `run` or `resume` call executes at most `MAX_IMMEDIATE_STEPS` consecutive control-flow operations. Reaching that budget returns a possible-infinite-loop error so a script with no host yield point cannot occupy its caller forever.

Current built-in limits:

| Layer | Limit |
| --- | --- |
| Source | 1 MiB, 10,000 physical lines, 64 nested statement blocks |
| Expression | 64 KiB / 512 tokens / 32 parenthesis levels per expression; interpolation is limited to 32 levels and shares 256 KiB work and 2,048-token budgets |
| Value | 4,096 nodes, 16 collection levels, and 1 MiB of text per value tree |
| Program bytecode | 100,000 control-flow ops, 100,000 chunks, 65,536 slots, 100,000 constant-value nodes, and 16 MiB of constant/slot text |
| Expression bytecode | 4,096 ops and stack height 1,024 per chunk; 128 arguments per host instruction |
| Tooling | 1 MiB CLI/Playground output; 1,000 CLI/Playground host effects; 1 MiB Playground reply JSON and 5-second Worker request; 4 MiB LSP JSON body, 64 KiB headers, and 8 KiB per header line |

CLI, WebAssembly, and other hosts should add time, effect permission, and external resource limits appropriate to their own risk models. Those are host-layer concerns and cannot be unified by the language core.

## 10. Tooling

### CLI

`velin check` compiles and statically checks a file or stdin source, with optional JSON diagnostics. `velin run` uses a bounded line-oriented reference host intended only for examples. That host implements `say` and `ask`; other effects are printed by name and arguments.

### LSP and VS Code

`velin-lsp` speaks JSON-RPC over stdio and provides recovering multi-error diagnostics, completion, label document symbols, hover, and label definition/reference navigation. It directly calls `velin-lang` and `velin-check` instead of maintaining a second set of language semantics. Unknown requests return the standard `MethodNotFound` response.

The VS Code extension registers `.velin` files, provides TextMate highlighting, and launches the LSP. Platform release VSIX packages bundle the matching native server; explicit configuration can override it.

### WebAssembly

`velin-wasm` exposes `check(source)` and `run(source, replies_json)`. The binding layer only handles bounded strings and strictly validated JSON; the facade crate still performs parsing, checking, and execution. The Playground runs it in a terminable Worker so the UI remains responsive. Output is rendered incrementally against the remaining budget without allocating a complete intermediate string.

`#[wasm_bindgen]` generates `unsafe` glue, so the Wasm shim cannot inherit the workspace's `unsafe_code = "forbid"`. All handwritten Wasm code remains safe Rust, and core crates retain the prohibition.

## 11. Stability rules

The project is in its pre-stable `0.x` line and currently makes no backward-compatibility promise for source syntax, serialization formats, or Rust APIs. Clear boundaries and correctness take priority. Once stabilized, these rules constrain evolution:

- `velin-eval` is the reference value semantics; VM changes must pass differential tests.
- New external behavior must be modeled as a host effect, never direct VM I/O.
- New values must define deterministic equality, display, serialization, and budget cost.
- New control flow must participate in definite-assignment analysis and immediate-step accounting.
- New random capabilities must explicitly advance frame-local state and never read system entropy.
- Diagnostics must carry the real filename and source location.
- Core crates targeting WebAssembly should not depend on system APIs.

## 12. Verification gates

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo build --workspace --target wasm32-unknown-unknown
```

Benchmarks are used only to detect relative regressions, not as a pre-stable compatibility promise.
