# Changelog

All notable changes to Velin are recorded here. The project follows semantic
versioning once an API is declared stable; the current `0.x` line may still
change source and Rust APIs between minor releases.

## 0.5.0 - 2026-09-19

### Added

- A pinned Rust 1.98.0 local toolchain, a 0.5.0 composable-DSL compatibility fixture, adversarial artifact/module/formatter/marshalling properties, and a modules-to-replay end-to-end benchmark.
- Unified `ExecutionPolicy` budgets for VM fuel, host effects, call depth, runtime data, host queues, progress callbacks, and cooperative cancellation.
- Layered source, Rust API, artifact, and C ABI compatibility policies with versioned source/artifact fixtures.
- Typed host command declarations shared by static checking, runtime validation, synchronous/asynchronous host drivers, and schema-aware LSP completion, signature help, and hover.
- Budget-aware, path-reporting Serde value marshalling plus complete List/Record JSON input and output for WebAssembly and the append-only C ABI extension.
- Host-resolved compile-time modules with explicit exports, cycle diagnostics, named pure functions, and acyclic hygienic call expansion.
- `for` / `break` / `continue`, List and Record literals, collection indexing and indexed assignment, and compound assignment sugar.
- An official source formatter exposed through `velin fmt`, `velin fmt --check`, the Rust facade, and LSP formatting/code actions.
- LSP semantic tokens, rename, formatting/code actions, bounded workspace indexing, cross-document navigation, workspace symbols, and module-dependency diagnostics.
- Source-level VM debugging with breakpoints, stepping, variable inspection, effect-boundary pauses, replayable snapshots, and source-mapped profiles.
- A persistent Wasm `PlaygroundSession` and Playground controls for stepping, resuming, inspecting state, and deterministic branch replay.
- Configurable execution-policy and cooperative-cancellation entry points for the C ABI, Wasm runtime, and Playground sessions.
- Reproducible CI performance and native/Wasm artifact-size gates backed by a checked-in historical baseline.

### Changed

- Unreleased performance recording now replaces the previous `*-last.jsonl` while retaining every versioned release snapshot.
- Machine snapshots preserve cumulative fuel, host-effect counters, policy, and cancellation state; machine restart begins a fresh budget lifetime.

## 0.4.0 - 2026-09-15

### Added

- Reusable `MachineInvoker` and `PureModuleInvoker` sessions for repeated execution without rebuilding VM state.
- Low-allocation single-argument host yields and optional execution profiling controls.

### Changed

- Register workspaces, scalar operations, read-only built-ins, and common collection-length guards now reuse validated runtime state on hot paths while preserving shared semantics and data budgets.
- VM execution modules are split so every Rust source file remains within the 500-line maintenance limit.

## 0.3.0 - 2026-09-15

### Added

- Pure value-in/value-out modules with typed input bindings, isolated invocation state, explicit `return`/`fail` control signals, and bounded execution.
- Combined static checking for entry bindings and host command schemas, including pure-module rejection of undeclared effects and random operations.

## 0.2.2 - 2026-09-15

### Fixed

- Cached artifacts retain static check sites and use the version 4 wire format; older artifacts are rejected explicitly.
- Bytecode validation binds random operations to the reserved RNG slot and rejects the legacy random builtin encoding.
- Initial frames verify slot-layout identity, while batch execution preserves effects before reporting deferred errors.
- Type checking, SSA propagation, host-event budgets, CLI stdin behavior, and LSP framing/lifecycle now handle their boundary cases consistently.
- Windows artifact replacement, Wasm benchmarks, C API smoke checks, strict rustdoc, and release-version checks are covered by CI.

## 0.2.1 - 2026-09-14

### Fixed

- Rust 1.88 Clippy accepts the bytecode metadata matcher used by the CI toolchain.
- Release-mode wasm-pack builds explicitly enable bulk-memory validation during `wasm-opt`.

## 0.2.0 - 2026-09-14

### Added

- Runtime execution images can omit slot names while preserving frame width and RNG state; optional debug sidecars and anonymous execution profiles support small embedders.
- Stable C runtime bindings, a runtime-only WebAssembly entrypoint, binary artifacts, and cached machine restarts are available for embedding.
- Bounded host-effect batching, aggregate machine-state budgets, host-payload budgets, and conservative typed SSA optimizations are included.

### Changed

- Expression bytecode now uses explicit registers for arithmetic, short-circuit control flow, built-ins, interpolation, and random operations; the VM no longer maintains an operand-stack fallback.
- Built-in evaluation accepts owned argument arrays directly; the legacy `invoke_stack_*` entry points were removed.
- Artifact version 3 stores the register bytecode format and rejects earlier artifacts at the version boundary.

### Fixed

- Batch limits no longer preallocate beyond the immediate-step budget.
- Batches return already collected valid effects before reporting a later evaluation or host-contract error.
- Manual batch Host operations are included in execution profiles, and reseeding refreshes frame metrics.

## 0.1.2 - 2026-09-13

### Added

- Aggregate VM frame and host-payload data budgets.
- Host-owned command schemas with static and runtime contract validation.
- A shared bounded `ScriptRunner` for CLI and WebAssembly hosts.
- Recovering statement parsing for multi-error LSP diagnostics and partial editor features.
- LSP hover, label definition, and label reference support.
- CLI help/version commands, stdin source support, and JSON check output.
- Web Worker execution, timeout/stop controls, and strict reply validation in the Playground.
- Cross-platform CLI/LSP archives and platform VSIX packaging in the release workflow.
- Property tests for parser robustness, deserialization validation, and evaluator/VM agreement.
- A standalone `velin-bytecode` crate for embedding validated bytecode without the compiler.

### Changed

- Runtime bytecode ownership is separated from compilation and checking dependencies.
- Release builds use ThinLTO and strip symbols to reduce embedded binary size.

### Fixed

- Short-circuit `and`/`or` bytecode now matches the reference evaluator for invalid operands.
- Invalid Playground reply items no longer shift later answers to a different `ask`.

## 0.1.0 - 2026-09-11

- Initial public release of the language core, CLI, LSP, VS Code extension, Wasm bindings, Playground, examples, and bilingual documentation.
