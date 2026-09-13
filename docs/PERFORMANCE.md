# Performance Design

[简体中文](PERFORMANCE.zh-CN.md)

Velin compiles source code to bytecode and executes that bytecode in a VM. Source is parsed only during compilation, and the resulting `Program` can be reused to create multiple `Machine` instances or run the same script repeatedly. A program has two instruction layers:

- `Op` handles control flow such as assignment, jumps, host calls, and termination.
- `ExprOp` is a stack-machine instruction set for constants, variable loads, operators, built-ins, and short-circuit logic.

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

`and` and `or` use forward jumps in expression bytecode. Definite-assignment analysis runs an abstract stack containing known booleans, known non-booleans, and unknown values. It can therefore exclude variable loads hidden behind a constant short-circuit condition.

The first eight abstract stack entries live in a fixed array; deeper expressions spill to a `Vec`. Branch-free expressions are scanned directly, and valid forward branches propagate state in program-counter order. If the standalone analysis API receives noncanonical branches, it falls back to a work-queue fixed point instead of relying on compiler-produced input.

Reachable `Load` sets are cached per expression chunk during one check. The state table and queue storage are also reused across chunks.

### Built-in type arguments use a small array

During built-in type inference, the first four argument types live in an inline array. Wider calls allocate a `Vec<Type>`. Both representations feed the same `infer_builtin` function, so storage policy does not create a second inference path.

## Compilation and bytecode

### Expressions live in contiguous arenas

`Program` stores all expression operations and constants in contiguous `expr_ops` and `constants` arenas. A `ProgramChunk` contains only two ranges and a source line. The VM borrows slices through those ranges and never has to assemble an instruction object for each evaluation.

`ProgramBuilder` owns one reusable `ExprChunk` scratch buffer. After an expression is compiled, its contents move into the arenas while the scratch capacity remains available for the next expression. At program completion, the main vectors call `shrink_to_fit` so build-time excess capacity is not retained indefinitely.

On current 64-bit targets, layout tests fix `ExprOp` at 12 bytes, `Op` at 32 bytes, and `ProgramChunk` at 20 bytes. These tests expose accidental growth when fields are added to hot enums. Variable-sized host arguments are boxed so their cold payload does not widen every `Op`.

### Constant folding is conservative

The compiler folds only pure constant subtrees that `velin-eval` can evaluate successfully. Random calls are not foldable. Integer overflow, division by zero, type mismatches, and other failing candidates remain ordinary bytecode and report their errors at runtime in the original evaluation order.

Short-circuit logic follows the same rule. If a constant left operand determines the result, an unreachable branch does not allocate an RNG state slot. Otherwise, the complete short-circuit structure remains in bytecode.

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

## Bytecode validation and execution preparation

### External bytecode is fully validated

`Program` is public and serializable, so the runtime cannot assume every instance came from the current compiler. `ValidatedProgram` is proof attached to a shared `Arc<Program>`. Validation covers:

- limits on instructions, chunks, slots, and argument counts;
- bounds for slot, constant, chunk, and jump-target indices;
- operand-stack height along every expression path and a single final result;
- constant value-tree and program text budgets;
- update instruction and built-in argument validity.

`Machine::new` and `Machine::with_seed` perform full validation when given an ordinary program. `ScriptRunner` can reuse the proof held by `CompiledScript`. If a caller replaces its public `program` `Arc`, pointer identity no longer matches and the runner validates again.

The safety boundary remains explicit rather than depending on a calling convention.

### Execution metadata is initialized on demand

A validated program lazily constructs `ExecutionMetadata` through `OnceLock`. Compile-only and check-only users do not pay for runtime metadata. The first `Machine` initializes it, and later machines share it.

Metadata records each expression's maximum stack depth, whether it mutates RNG state, whether it inherits slot metrics directly, constant result metrics, source lines, and directly addressable built-in call plans. These properties do not change during the program's lifetime and need not be rediscovered on each run.

## VM execution

### The operand stack is reused across expressions

Each `Machine` owns an `expression_stack`. Evaluation clears its length and reserves against the maximum stack depth computed during execution preparation. `ExprOp` instructions and constants are borrowed from the program arenas rather than copied.

After machine construction, straight-line scalar expressions normally require only stack pushes and pops, without repeatedly creating an operand container.

### Long scalar expressions can use a register plan

During execution preparation, a long expression with only constants, slot loads, unary operators, and ordinary binary operators is lowered to a non-serialized register plan. Each value is assigned a stable temporary register, so evaluation reads operands by index instead of maintaining a value stack for every intermediate. The plan is stored in execution metadata and rebuilt from the validated expression chunk; it is not part of the serialized bytecode format.

The register path uses the same `apply_unary` and `apply_binary` functions as the stack evaluator and reports errors with the expression's source line. Short expressions, short-circuit operators, built-ins, concatenation, random operations, and any shape that cannot be proven straight-line continue through the canonical `ExprOp` stack path. A reusable `Option<Value>` workspace keeps temporary allocations out of repeated evaluations, and the final value is measured before it enters the frame's normal resource accounting.

### Simple built-in calls are specialized during preparation

When an expression consists only of `Const` or `Load` operands followed by one `Call`, execution metadata resolves the arguments to `QuickenedOperand::Constant` or `QuickenedOperand::Slot`. Runtime execution does not need to interpret those load instructions again.

For read-only `len`, `get`, and `contains`, the VM borrows each `Value` directly from the constant pool or frame slot and passes a stack-allocated reference array to `velin-eval`. The source collection or string `Arc` is not cloned first.

Ownership-consuming `list`, `record`, `push`, `put`, and `remove` calls continue through the reusable operand stack. Fixed-arity built-ins pop operands directly from its end. When `list` arguments occupy the whole stack, the resulting list can take ownership of that `Vec`. Complex argument expressions always use ordinary `ExprOp` evaluation.

The borrowed path, owned path, and ordinary evaluator ultimately share one built-in implementation and one set of error rules.

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
