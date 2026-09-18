# Compatibility Policy

[简体中文](COMPATIBILITY.zh-CN.md)

This policy starts with Velin `0.4.0`. Velin remains on a pre-1.0 release line, but its public boundaries no longer share one vague stability label. Each boundary below has an explicit contract.

## Source language

Valid `0.4.x` source and its deterministic runtime behavior remain compatible across `0.4.x` patch releases. A patch release may add syntax or diagnostics, but it must not silently reinterpret an existing valid program.

A minor release may make a source-incompatible change while Velin is pre-1.0. Such a change must appear in `CHANGELOG.md`, include a before/after migration example, and have a fixture showing the old behavior. Deprecation diagnostics are preferred for at least one minor release when the old form can be recognized safely.

## Rust API

The `velin` facade crate is the supported Rust API and follows Cargo SemVer. Patch releases in one minor line preserve its public API. Before 1.0, a minor release may contain breaking changes, which must be documented with migration guidance.

The lower-level crates (`velin-syntax`, `velin-parse`, `velin-eval`, `velin-bytecode`, `velin-compile`, `velin-check`, `velin-lang`, and `velin-vm`) are available for specialized embedders, but APIs not re-exported by `velin` are experimental unless their documentation says otherwise.

Deprecations remain callable for the rest of the current minor line. Removing a deprecated facade API requires a minor version change and a changelog migration note.

## Artifacts

Every `.velinc` artifact carries `ARTIFACT_MAGIC` and `ARTIFACT_VERSION`. The `0.4.x` runtime reads version 4 artifacts. Decoders reject unknown, older, malformed, oversized, or semantically invalid artifacts before execution.

Artifact compatibility is version-exact: a runtime is required to read the artifact version it publishes, not arbitrary previous versions. The supported offline migration is to retain source and recompile it with the target Velin version. An artifact-version change must include a changelog entry and a fixed fixture for the last supported version.

Artifacts are compiled-program caches, not durable save files. `Machine` snapshots are in-memory values and have no stable serialized representation.

## C ABI

The C ABI uses opaque `VelinProgram` and `VelinMachine` handles and exposes `velin_c_api_version()` plus `VELIN_C_API_VERSION` in the header. ABI version 1 is the current contract.

Within ABI version 1:

- Existing exported functions, numeric tags, structure fields, field order, ownership rules, and null-handling behavior are preserved.
- New functions may be appended. Existing structures are not extended in place; a new versioned structure or function is used instead.
- Buffers returned by Velin are released only by the matching Velin free function.

The P1 compound-value extension follows that append-only rule: legacy calls still return `VELIN_VALUE_COMPOUND` display text, while the new `velin_machine_*_json` functions return List/Record payloads as stable JSON with `VELIN_VALUE_JSON`. The existing resume structure accepts the new tag. No existing field, tag value, ownership rule, or function behavior changed, so the ABI version remains 1.

The execution-policy extension is append-only as well: `VelinExecutionPolicy`,
`velin_execution_policy_default`, `velin_machine_new_with_policy`, and the
cancellation functions are new exports. The existing `velin_machine_new` keeps
default-policy behavior. Existing structures and functions retain their layout,
ownership, and behavior, so the ABI version remains 1.

An incompatible layout or ownership change increments `VELIN_C_API_VERSION`, keeps the old header/runtime pair available for the supported release line, and includes migration guidance.

## Fixtures And CI

Compatibility gates run with the normal workspace CI:

- `fixtures/compatibility/0.4.0/source.velin` checks the published source behavior.
- `fixtures/compatibility/0.4.0/artifact-v4.velinc` checks artifact version 4 decoding independently of the current encoder.
- `crates/velin-capi/tests/c_api_smoke.c` compiles and runs an ABI version 1 host against the public header.

Do not regenerate a historical fixture merely because a test fails. Add a new versioned fixture when the contract intentionally changes, and keep the previous fixture when the documented compatibility window requires it.

## Security Support

Security fixes are provided for the latest patch of the current minor line, currently `0.4.x`. Older minor lines and prereleases are unsupported unless a release notice explicitly says otherwise. Reports should use the repository's private security-reporting channel when available.

## Release Process

Every incompatible change must:

1. Identify the affected boundary.
2. Update `CHANGELOG.md` with migration guidance.
3. Add or update versioned compatibility fixtures.
4. Change the artifact or C ABI version when that boundary requires it.
5. Pass source, artifact, Rust, and C integration gates before release.
