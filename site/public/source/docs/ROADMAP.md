# Velin Roadmap

[简体中文](ROADMAP.zh-CN.md)

Velin is not intended to catch up with Rhai or another general-purpose scripting language. Its destination is a production-grade deterministic scripting runtime: scripts describe rules, state, and control flow; the host retains I/O, permissions, and domain behavior; and execution remains bounded, suspendable, resumable, and replayable.

This roadmap states direction and acceptance criteria, not release dates. Shipped behavior is defined by the [language reference](LANGUAGE.md), [architecture](ARCHITECTURE.md), and the repository's `CHANGELOG.md`.

## Current foundation

The `0.5.x` line has the boundaries needed for further work:

- Five deterministic value types, explicitly seeded randomness, and clonable machine state.
- A complete source-to-register-bytecode pipeline with bytecode validation repeated on deserialization.
- The `Yield::Host` / `resume` protocol, batching for side-effect-only hosts, and bounded host queues.
- Conservative type inference, host-schema checks, and definite-assignment analysis.
- Reusable general and pure-module invokers, binary artifacts, runtime execution images, and execution profiles.
- Rust, C, and WebAssembly runtime entry points plus a CLI, LSP, VS Code extension, and Playground.

These capabilities constrain future work. They are not temporary implementation details to discard when the language grows.

## Evolution rules

Every new capability must preserve these rules:

1. **External effects stay explicit.** Nothing may bypass `perform` to access files, networking, clocks, UI, or system randomness.
2. **Replayable state stays complete.** A snapshot must contain every VM state component that affects later execution; equal programs, inputs, and seeds must replay equal results.
3. **Untrusted input stays bounded.** New syntax, call stacks, modules, value shapes, and tool protocols must apply budgets before allocating large objects.
4. **Tools and runtime share semantics.** The parser, checker, compiler, VM, LSP, Wasm, and C API must not invent separate, approximately compatible rules.
5. **Stable boundaries stay smaller than internals.** Source, facade APIs, artifacts, and ABIs each declare a compatibility surface; internal AST, IR, and optimization plans do not become permanent formats.

The development order is **P0 -> P1 -> P2 -> P3 -> P4**. Performance and size evidence starts at P0 and spans every priority.

## P0: runtime boundaries and compatibility contracts

Stabilize runtime boundaries before expanding the language. P0 is the prerequisite for every later capability.

**Status: complete in the current tree.** Rust, C, and Wasm high-level entry points share one execution policy; compatibility policies, versioned fixtures, migration procedures, and CI gates are in place.

### Configurable execution budgets

Introduce one execution policy shared by `Machine`, `ScriptRunner`, invokers, and pure modules. It should cover at least:

- Total fuel accumulated across multiple `run` / `resume` calls.
- Immediate fuel, host-effect count, and call depth.
- Per-value, aggregate machine-state, host-payload, and host-queue data budgets.
- Cooperative cancellation or progress callbacks controlled by the host.

Cancellation and wall-clock deadlines remain host policy and do not enter script-visible value semantics. Whether snapshots retain consumed fuel must have one documented and tested rule. Fuel exhaustion, cancellation, invalid bytecode, and host-contract violations should have distinct error types.

Completion criteria: all high-level execution entry points accept the same policy; repeated resumes between host effects cannot reset the cumulative budget; and the default policy remains suitable for directly running untrusted scripts.

### Layered compatibility policies

Define and test four separate contracts:

- **Source language**: syntax and runtime behavior that remain compatible within the current release line, plus migration guidance when they change.
- **Rust API**: the facade crate follows SemVer while experimental low-level APIs declare their status.
- **Artifacts**: the format carries a version and offers either a limited compatibility window or an explicit offline migration tool.
- **C ABI**: opaque handles, ABI version queries, structure-layout rules, and an append-only extension policy.

Golden fixtures continuously exercise old source, old artifacts, and old host integrations. The security support window must follow the actual release line.

Completion criteria: every public boundary has a written compatibility policy, CI fixtures, and a deprecation process. Incompatible changes include migration guidance and are recorded in the changelog.

## P1: typed host SDK

Once P0 boundaries are stable, make host-command integration feel close to ordinary Rust calls without placing native objects inside the VM.

**Status: complete in the current tree.** One host declaration drives checking, runtime dispatch, and editor metadata; Serde, C, and Wasm compound-value marshalling share the same budgeted value semantics.

### One schema source

Generate or build the following from one declaration:

- The `HostSchema` consumed by static checks.
- ID dispatch plus argument and return-value checks used at runtime.
- Synchronous and asynchronous host drivers.
- LSP completion, signature-help, and hover metadata.
- Optional Rust macros, C header descriptions, and TypeScript types.

Invalid arguments, binding forms, and return types should be reported statically when provable, and otherwise before effect dispatch or during resume.

### Value marshalling

Provide budget-aware conversions between Serde and `Value`, then complete List/Record input and output support for C and WebAssembly. Conversions must preserve integer ranges, stable record ordering, and precise error paths.

Arbitrary Rust types, borrowed references, and native methods will not become new `Value` variants. Host-owned domain objects use explicit IDs or serializable DTOs, and authorization remains a host responsibility.

Completion criteria: one host-command declaration drives checking, runtime dispatch, and editor assistance; ordinary hosts no longer hand-write name matching and duplicate type validation.

## P2: a composable DSL

This phase addresses script reuse and organization without turning Velin into a general-purpose dynamic language. It depends on P0 resource boundaries and reuses the P1 host schema.

**Status: complete in the current tree.** Modules resolve at compile time, pure calls are hygienically expanded over an acyclic graph, and the syntax additions lower to existing checked operations.

### Modules and pure functions

Delivered capabilities are:

- Compile-time module imports through a host-controlled resolver.
- Explicit exports and cyclic-dependency diagnostics.
- Named pure functions with parameters, local variables, and return values.
- An initially acyclic call graph. Bounded recursion is reconsidered only after call-stack budgets, diagnostics, and snapshot semantics are mature.

Pure functions cannot execute `perform` or read clocks, system randomness, or other hidden state. State-machine control flow and host effects remain in the top-level script; pure modules remain isolated value-in/value-out execution units.

### Small syntax improvements

The following forms are available without adding new value semantics:

- `for`, `break`, and `continue`.
- List and Record literals.
- Collection-read indexing and simple assignment shorthand.
- Assignment sugar such as `+=` when it lowers unambiguously to an existing operation.

Every syntax addition must participate in parser recovery, static checking, bytecode validation, debug locations, resource budgets, and Wasm/C artifact tests.

Completion criteria: scripts can be organized as multi-file modules and reuse pure computation without copy-paste, labels that imitate functions, or dynamic code evaluation.

## P3: complete domain tooling

Build tooling last, using the analyzability of a small language and host schemas to provide a more precise experience than a general dynamic language. Cross-module features depend on the P2 module semantics.

**Status: complete in the current tree.** The CLI, LSP, debugger, Wasm bindings, and Playground now share the formatter, compiler metadata, and replayable VM state supplied by the core.

Delivered capabilities are:

- An official formatter with a deterministic check mode.
- Semantic tokens, rename, signature help, code actions, and workspace symbols.
- Cross-module definition/reference navigation and module-dependency diagnostics.
- Host-schema command documentation, parameter hints, and error locations.
- Breakpoints, stepping, variable inspection, effect-boundary pauses, and source-mapped profiles built on `DebugTable`.
- Pause, resume, snapshot, and deterministic branch replay in the Playground.

Completion criteria: normal editing, refactoring, and debugging do not require reading bytecode or manually mapping program counters; the LSP and debugger consume parsing, checking, and debug metadata supplied by the core.

## P4: release hardening and adoption evidence

The first three feature stages established the intended product boundary. P4 turns that implementation into a release that downstream embedders can trust, without expanding Velin into a general-purpose language.

**Status: in progress for 0.5.0.** The workspace version and compatibility fixtures now target 0.5.0. Artifact version 4 and C ABI version 1 remain unchanged because the new APIs are additive.

### Reproducible release evidence

- Keep exactly one `<source>-last.jsonl` report and retain every versioned release snapshot.
- Pin the local measurement toolchain and reject performance comparisons that mix incompatible environments.
- Exercise a representative end-to-end workload that includes modules, static checking, host effects, snapshots, and deterministic branch replay.

### Adversarial boundary validation

- Extend property and fuzz coverage across source parsing, formatting, module graphs, artifact decoding, value marshalling, and resume/cancellation state.
- Assert resource ceilings before large allocations and preserve path-aware, typed failures at every public boundary.
- Test the published Rust facade, C header, Wasm entry points, and source and artifact fixtures as downstream users consume them.

Completion criteria: 0.5.0 ships with frozen source/API/ABI decisions, versioned compatibility fixtures, reproducible local performance evidence, and representative Rust/C/Wasm integration coverage.

Durable serialized snapshots and bounded recursion remain candidates for a later roadmap. They require concrete embedder demand and a separate design for format compatibility, call-stack budgets, diagnostics, and replay semantics; they are not implicit P4 deliverables.

## Cross-phase work: performance and size evidence

Performance work starts at P0 and spans every phase instead of becoming a final optimization sprint. Benchmarks have three groups:

**Status: continuously enforced.** The repository stores a reproducible historical baseline; CI runs representative Rust/C/Wasm benchmarks and checks native and Wasm artifact sizes, while every release appends a full snapshot.

1. **Shared language capabilities**: integers, booleans, strings, lists, records, conditions, and loops.
2. **Embedding costs**: cold compilation, artifact loading, warm execution, host round trips, effect batching, and machine restart.
3. **Velin-specific paths**: snapshots, replay, execution images, C ABI, Wasm, and queue backpressure.

Results record latency, throughput, allocations, peak memory, and native/Wasm size. Comparisons with Rhai or other engines cover only semantically equivalent subsets and pin versions, features, hardware, toolchains, and timing boundaries. CI uses Velin's own historical baselines for regression gates; cross-engine numbers explain trade-offs rather than support a universal faster-than claim.

Completion criteria: published results can be reproduced from repository commands, release notes explain significant regressions or improvements, and optimizations cannot bypass semantic tests or resource accounting.

## Explicit non-goals

The following capabilities are outside this roadmap:

- Syntax or API compatibility with Rhai, JavaScript, Lua, or another general-purpose language.
- Closures, function pointers, runtime `eval`, dynamic imports, or runtime syntax extension.
- Custom operators, operator overloading, inheritance, or a native object model.
- Exposing arbitrary Rust objects, references, or methods directly as VM values.
- Implicit I/O, system clocks, system randomness, JIT, or native code generation.
- Increasing the type count at the expense of cross-platform determinism.

If future requirements conflict with these boundaries, the architecture goals and trust model must be changed explicitly instead of bypassing the host-effect protocol for local convenience.
