# Performance Design

[简体中文](PERFORMANCE.zh-CN.md)

Velin compiles source code to bytecode and executes that bytecode in a VM. Source is parsed only during compilation, and the resulting `Program` can be reused to create multiple `Machine` instances or run the same script repeatedly. A program has two instruction layers:

- `Op` handles control flow such as assignment, jumps, host calls, and termination.
- `ExprOp` is a register instruction set for constants, variable loads, operators, built-ins, random operations, interpolation, and short-circuit logic.

The complete path is:

```text
source -> line and expression parsing -> AST -> lowering -> Program -> bytecode validation
                                                               |-> static checks
                                                               +-> Machine -> VM execution
```

This separation gives parsing, static analysis, and execution clear ownership boundaries. Performance work is subordinate to semantic correctness and maintainability: the main goals are to eliminate repeated work, keep common paths linear, and make resource usage predictable.

## Design principles

### One source of semantics

Arithmetic, comparisons, and built-in behavior live in `velin-eval`. Compile-time constant folding, ordinary expression evaluation, and specialized VM paths all reuse those semantics. A specialized path may resolve operands or manage ownership differently, but it does not reimplement language rules.

This prevents the compiler and VM from disagreeing about boundaries such as overflow, division by zero, type errors, or missing keys.

### Every specialization has a general fallback

The compiler and VM specialize only forms with an exact, stable shape. Expressions that do not qualify continue through the complete expression bytecode and evaluator. Each specialization remains local instead of becoming a second execution system.

### Failed updates preserve state

In-place collection and string updates have two phases:

1. Evaluate and check every fallible operation, including types, indices, integer overflow, and resource budgets.
2. After those checks succeed, take the destination slot, update the value, and install its metrics.

A failing `push`, `put`, `remove`, or string append therefore leaves the original slot unchanged. This ordering is part of the runtime semantics and cannot be removed merely to save branches.

### Resource bounds take precedence over peak throughput

Source, expressions, bytecode, value trees, machine state, and host payloads all have explicit limits. These limits prevent pathological input from creating unbounded work and allow validators, checkers, and the VM to use compact indices and bounded preallocation safely.

## Parsing and input scanning

### Line scanning borrows source slices

Before statement parsing, Velin makes one pass over the physical source lines. Indentation accepts only ASCII spaces, so leading indentation is scanned by byte and a tab is rejected immediately. Each `Line` stores its line number, indentation level, source column, and a `&str` borrowed from the original source; line contents are not copied.

Comment stripping first checks whether the line contains `"` or `#`. A line containing neither byte returns its original slice immediately. Only a possible string or comment enters the full state scan, which also handles escapes and `#` characters inside interpolations correctly.

A source file is limited to 1 MiB, 10,000 physical lines, and 64 levels of statement nesting. These limits are checked before large intermediate structures can accumulate.

### Short expressions keep tokens inline

The expression lexer stores its first four tokens in a fixed array. A short expression does not allocate a token container on the heap. Once the array is full, its tokens move into a `Vec` once and parsing continues through the same interface.

This is a small-storage policy, not a separate parser for short expressions.

### Nested interpolations share a parse budget

One expression is limited to 64 KiB, 512 tokens, and 32 levels of parenthesis nesting. Child expressions inside string interpolation share cumulative byte and token budgets, so nested input cannot repeatedly request work near the per-expression limit.

Host-call arguments are parsed directly as a comma-separated expression list, with each argument retaining its source span. The parser does not manufacture and reparse a synthetic `list(...)` wrapper. The width limit matches the language's wide-argument boundary at 128 arguments.

## Static analysis

### Names become slots at the boundary

During lowering, `SlotTable` interns variable names as `u32` slots. Definite-assignment analysis and VM instructions carry slots directly. Type checking retains source expressions with spans; a variable read resolves through `SlotTable` and then indexes a dense state array. The reverse table recovers names for diagnostics.

Type propagation uses `Vec<Type>`, while definite assignment uses bit sets. A name-based environment supplied through the public API is converted to a dense entry state once. Basic blocks copy or merge only dense state and do not rebuild name environments.

### Straight-line programs use one pass

A program without jumps does not build a control-flow graph. When type-check sites are already ordered by program counter, type propagation consumes them during one instruction pass. Definite-assignment analysis likewise maintains one current bit set.

This path covers ordinary sequential assignments and host calls while retaining precise diagnostic locations.

### Branches propagate state by basic block

A program containing jumps is divided into basic blocks with predecessor, successor, and reachability information. Dataflow state moves only across block boundaries:

- Definite assignment is a must analysis and intersects predecessor state at a join.
- Type analysis keeps a concrete type only when every reaching path agrees; otherwise it returns to `Unknown`.
- A successor enters the work queue only when its input state changes.

The checker reports only type uses that it can prove will fail. `Unknown` remains permissive. This conservative rule matters more than aggressive inference because it prevents static analysis from rejecting a script that may be valid at runtime.

### Short-circuit analysis visits only reachable loads

`and` and `or` use forward jumps over explicit condition registers. Definite-assignment analysis propagates an abstract register file containing known booleans, known non-booleans, and unknown values. It can therefore exclude variable loads hidden behind a constant short-circuit condition.

Register states merge at expression control-flow joins. A work queue also keeps the standalone analysis API conservative for malformed or noncanonical input instead of assuming every chunk came from the compiler.

Reachable `Load` sets are cached per expression chunk during one check. The state table and queue storage are also reused across chunks.

### Built-in type arguments use a small array

During built-in type inference, the first four argument types live in an inline array. Wider calls allocate a `Vec<Type>`. Both representations feed the same `infer_builtin` function, so storage policy does not create a second inference path.

## Compilation and bytecode

### Expressions live in contiguous arenas

`Program` stores all expression operations and constants in contiguous `expr_ops` and `constants` arenas. A `ProgramChunk` contains two ranges, the register count, the result register, and a source line. The VM borrows slices through those ranges and never has to assemble an instruction object for each evaluation.

Runtime hosts can call `Program::into_execution_image` to move slot names and
expression/operation columns out of the hot image. Numeric slots and contiguous
arenas remain unchanged while source locations live in an optional
`DebugTable` sidecar. This image is intended for C/Wasm runtimes that do not
need name binding or editor diagnostics.

`ProgramBuilder` owns one reusable `ExprChunk` scratch buffer. After an expression is compiled, its contents move into the arenas while the scratch capacity remains available for the next expression. At program completion, the main vectors call `shrink_to_fit` so build-time excess capacity is not retained indefinitely.

On current 64-bit targets, layout tests fix `ExprOp` at 12 bytes, `Op` at 32 bytes, and `ProgramChunk` at 24 bytes. These tests expose accidental growth when fields are added to hot enums. Variable-sized host arguments are boxed so their cold payload does not widen every `Op`.

### Constant folding is conservative

The compiler folds only pure constant subtrees that `velin-eval` can evaluate successfully. Random calls are not foldable. Integer overflow, division by zero, type mismatches, and other failing candidates remain ordinary bytecode and report their errors at runtime in the original evaluation order.

Short-circuit logic follows the same rule. If a constant left operand determines the result, an unreachable branch does not allocate an RNG state slot. Otherwise, the complete short-circuit structure remains in bytecode.

### Straight-line constant propagation

`ProgramBuilder` also tracks values installed by `SetConst` and `CopySlot` while assembling a straight-line region. A later assignment is evaluated against that small known-value environment; when the pure expression succeeds, it becomes another `SetConst` and does not need expression evaluation at runtime. The environment is cleared at jumps, host effects, unknown writes, and termination, so no value is propagated across a path merge or an external effect. Failed evaluation, random calls, and runtime-error candidates retain their original bytecode and error timing.

The known-value environment also maintains an incremental slot-to-name map. Wide straight-line scripts no longer rebuild a `BTreeMap` from every slot on each assignment; jumps, host effects, and unknown writes still clear the environment.

Validated programs also construct `TypedIr`: it splits control-flow boundaries
into basic blocks, assigns monotonic SSA value IDs to assignments, and retains
Host, random, and potentially failing operations as barriers. `TypedIr::optimize`
only propagates reachability; it never reorders side effects or replaces the
canonical bytecode.

### Direct instructions cover exact shapes

Lowering represents these common forms with direct instructions:

| Source shape | Bytecode form | Mechanism |
| --- | --- | --- |
| Constant assignment | `SetConst` | Installs the value with precomputed resource metrics |
| Variable copy | `CopySlot` | Loads by slot and reuses the source slot's metrics |
| `count = count + constant` | `Update::AddInteger` | Performs checked addition directly on the slot |
| `slot <op> integer literal` condition | `JumpIfIntegerCompare` | Reads the integer slot and decides the branch directly |
| Self-targeting `push`, `put`, `remove`, or `+` | `Update` | Retains destination ownership and updates after preflight |

Recognition is restricted to forms with equivalent semantics. Every other assignment or condition still references an expression chunk. New language behavior can therefore be implemented in the general path first, with specialization added only when it remains worthwhile and local.

### Control-flow rewriting preserves program counters

A constant boolean condition can become a direct jump, and an unconditional jump chain resolves to its final target. The process does not delete or renumber instructions, so PCs retained by static checks, source locations, and host-site mappings remain stable.

### Loop tails execute as one VM step

The VM recognizes an `Update` immediately followed by an unconditional `Jump`, the common tail emitted for a counter loop. It performs the update and chooses the jump target in one dispatch. The step budget charges both original instructions, so loop bounds and infinite-loop diagnostics remain unchanged. This is a runtime dispatch optimization only; the public bytecode shape and validation rules stay the same.

## Bytecode validation and execution preparation

### External bytecode is fully validated

`Program` is public and serializable, so the runtime cannot assume every instance came from the current compiler. `ValidatedProgram` is proof attached to a shared `Arc<Program>`. Validation covers:

- limits on instructions, chunks, slots, and argument counts;
- bounds for slot, constant, chunk, and jump-target indices;
- register and argument-range bounds, definitions on every reachable expression path, and a defined result register;
- constant value-tree and program text budgets;
- update instruction and built-in argument validity.

`Machine::new` and `Machine::with_seed` perform full validation when given an ordinary program. `ScriptRunner` can reuse the proof held by `CompiledScript`. If a caller replaces its public `program` `Arc`, pointer identity no longer matches and the runner validates again.

The safety boundary remains explicit rather than depending on a calling convention.

### Execution metadata is initialized on demand

A validated program lazily constructs `ExecutionMetadata` through `OnceLock`. Compile-only and check-only users do not pay for runtime metadata. The first `Machine` initializes it, and later machines share it.

Metadata records each expression's register count, whether it mutates RNG state, whether it inherits slot metrics directly, constant result metrics, and source lines. These properties do not change during the program's lifetime and need not be rediscovered on each run.

### Artifacts use a bounded binary payload

`.velinc` payloads now use a tagged, length-bounded binary value encoding
instead of storing the `Program` as JSON text. The format version increments
directly; decoding checks lengths, tags, UTF-8, nesting, and program budgets
before constructing the shared validation proof. Source bytes, compiler
semantics, optimization level, and Host schema inputs form the cache key;
entries are installed through a same-directory temporary file and atomic rename,
and malformed entries are treated as cache misses.

## VM execution

### Interpolation avoids repeated validation

The VM interpolation path carries `DataMetrics` for every value. It now
computes the deterministic display byte length and reserves the result once;
integers and booleans write directly into the destination buffer instead of
creating short-lived scalar `String` values. Validated values use a dedicated
append entry point that still enforces the final output limit without walking
the entire value tree again. Copy-on-write for aliases and all error ordering
remain unchanged.

### All expressions use register bytecode

Register operands are part of the serialized `ExprOp` format; execution does not derive a second plan. Constants and slot reads write named destinations, unary and binary operations name their inputs, and each chunk declares its register count and result register. `Machine` reuses parallel `Option<Value>` and `Option<DataMetrics>` register files across expressions.

Short-circuit branches, built-ins, interpolation, and random operations use the same interpreter as scalar arithmetic. Conditional instructions read a condition register, while variable-arity operations consume a validated contiguous register range. There is no operand-stack execution path or fallback.

Typed integer arithmetic and boolean negation still delegate failures and dynamic cases to the shared `velin-eval` semantics. Externally supplied values therefore retain the same overflow, type, and source-line diagnostics as the reference evaluator.

The VM also exposes a bounded `ExecutionProfile` containing only validated,
anonymous program-counter hit counts. `hot_ops` and `merge` provide an offline
feedback input; profiles never contain values, strings, or Host payloads and do
not alter default execution semantics.

### Built-ins consume register ranges

`Call` names a contiguous argument range. The VM collects owned arguments and their existing metrics from those registers, then invokes the shared built-in implementation. Collection constructors and edits preserve copy-on-write behavior, and all built-ins retain the reference evaluator's validation and error rules.

### Self-updates use copy-on-write

`Value::String` uses `SharedString(Arc<String>)`. Lists and records use `Arc<Vec<Value>>` and `Arc<BTreeMap<...>>`. Reads, variable aliases, and machine snapshots only increment a reference count.

After preflight, a self-update uses `Arc::make_mut`:

- A uniquely owned value reuses its allocation.
- A real variable alias or `Machine::clone` snapshot causes the backing data to be copied.
- Existing aliases continue to observe their original value after the update.

This reconciles value semantics with storage reuse. A value being in the destination slot does not prove unique ownership; the actual `Arc` reference count is the authority.

### Resource metrics travel with values

`DataMetrics` records a value tree's node count, UTF-8 text bytes, and maximum nesting depth. Expressions and built-ins return `(Value, DataMetrics)`, and assignment stores both in the frame. The frame maintains compact footprint and depth arrays per slot, plus one aggregate footprint for the whole machine.

Direct constants, variable copies, and ordinary expression results can carry existing metrics forward. A collection edit updates node and text counts by subtracting the removed item and adding the replacement. Depth uses a conservative incremental rule: the result tree is rescanned only when the edit removes a deepest child and the new child cannot preserve the previous depth.

When `Update::AddInteger` has a proven scalar input and result, the VM reuses the unchanged scalar footprint and updates only the value and depth cache. Overflow, type errors, and resource limits retain their existing checks. Conditions that are not specialized integer comparisons execute through the register expression interpreter.

Logical resource accounting is independent of `Arc` sharing. If multiple slots reference the same collection, each logical value counts against the budget, so execution does not depend on transient ownership state.

## Intentionally retained costs

The following costs support correctness, API boundaries, or maintainability and should not be removed solely for a microbenchmark:

- An arbitrary external `Program` receives full validation before entering the VM.
- `Machine::clone` copies dense slot and metric arrays so the two machines can advance independently; nested strings and collections still share their `Arc` storage.
- A real alias requires copy-on-write for collection and string updates.
- A host call collects owned arguments in a `Vec<Value>` because those values cross the VM boundary and must survive while execution is yielded.
- Host payload metrics are accumulated for every argument to enforce total value and text limits.
- Static checks retain conservative joins and precise source diagnostics instead of trading correctness for a small amount of throughput.
- An expression without a proven equivalent specialization always uses the general bytecode path.

A performance mechanism should be judged by code locality, number of semantic authorities, fallback behavior, failure atomicity, and test coverage, not by one benchmark number alone.

## Performance verification

Benchmarks are separated by stage so unrelated costs do not become one conclusion:

| Benchmark prefix | Scope |
| --- | --- |
| `check/*` | CFG construction, definite assignment, and type propagation |
| `compile/*` | Post-parse lowering, expression compilation, and program layout |
| `machine/*` | Validation-proof reuse, metadata, and frame initialization |
| `vm/*` | Execution on an already constructed machine |

CLI timing includes process startup, source parsing, lowering, and checking, so it describes the complete tool experience. Criterion repeats a target path within one process and is better suited to isolating a mechanism. Results should identify the machine, Rust toolchain, build mode, and timing boundary.

Run correctness and static-quality checks first:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Run the complete Criterion suite:

```sh
cargo bench -p velin --bench pipeline -- --noplot
```

Select focused benchmarks by mechanism:

```sh
cargo bench -p velin --bench pipeline -- check/wide_linear_script --noplot
cargo bench -p velin --bench pipeline -- machine/create_wide --noplot
cargo bench -p velin --bench pipeline -- 'wide_linear|constant_folding' --noplot
cargo bench -p velin --bench pipeline -- 'vm/(counter_loop|builtin_loop|growing_list|growing_string|string_reads)' --noplot
```

Performance results must be interpreted with semantic tests. Changes to shared semantics, ownership, or resource accounting should also run evaluator/VM differential tests, alias tests, post-failure state tests, and serialization round-trip tests.
