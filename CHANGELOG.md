# Changelog

All notable changes to Velin are recorded here. The project follows semantic
versioning once an API is declared stable; the current `0.x` line may still
change source and Rust APIs between minor releases.

## Unreleased

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
