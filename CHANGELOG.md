# Changelog

All notable changes to Velin are recorded here. The project follows semantic
versioning once an API is declared stable; the current `0.x` line may still
change source and Rust APIs between minor releases.

## Unreleased

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

### Fixed

- Short-circuit `and`/`or` bytecode now matches the reference evaluator for invalid operands.
- Invalid Playground reply items no longer shift later answers to a different `ask`.

## 0.1.0 - 2026-09-11

- Initial public release of the language core, CLI, LSP, VS Code extension, Wasm bindings, Playground, examples, and bilingual documentation.
