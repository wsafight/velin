# Current Performance Baseline

[简体中文](PERFORMANCE-BASELINE.zh-CN.md)

This page records one reproducible local performance snapshot. The numbers are
not release guarantees and should not be compared with another engine unless
the language subset, workload, toolchain, hardware, and timing boundary are
held constant.

## Measurement environment

| Item | Value |
| --- | --- |
| Velin version | `0.4.0` |
| Source revision | `86ab5f0` plus local documentation and benchmark changes; runtime code unchanged |
| Host | Apple MacBook Pro, Apple M3 Pro, 12 cores, 36 GB RAM |
| OS | macOS Darwin `25.6.0`, `arm64` |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| Build | Criterion `bench` profile with workspace release settings |
| Date | 2026-09-16 (Asia/Shanghai) |

The runtime implementation was unchanged while collecting this snapshot. The
worktree contained the new P0 benchmark harness described in the roadmap.

## Version policy

The measurements in this document are the initial baseline for Velin `0.4.0`.
When a later version is released, append a new version- and date-labeled section
with its own environment and measurements. Do not overwrite the `0.4.0` values;
the document is intended to retain a historical sequence of comparable
snapshots.

## Method

The pipeline, C ABI, and Wasm benchmarks used the same shortened Criterion
sampling configuration so the snapshot could be collected in one pass:

```text
sample-size       20
warm-up-time      0.5 s
measurement-time  1 s
plots             disabled
```

The table reports the center value from Criterion's `[low median high]` output.
The `change` lines printed by Criterion are intentionally excluded: the local
`base` directory does not have a recorded hardware and toolchain provenance,
so those comparisons are not a trustworthy regression claim.

## Pipeline metrics

These are per benchmark invocation, not per source statement. Fixture sizes are
included to keep the measurements interpretable.

| Area | Benchmark | Fixture | Median |
| --- | --- | --- | ---: |
| Parse | `parse/guard` | one compound guard expression | 1.8343 us |
| Parse | `parse/wide_linear_script` | 512 variables and assignments | 339.32 us |
| Parse | `parse/host_calls` | 512 host effects | 296.22 us |
| Check | `check/guard` | one compound guard expression | 133.52 ns |
| Check | `check/wide_linear_script` | 512 variables and assignments | 38.175 us |
| Check | `check/expression_heavy_script` | 512 arithmetic expressions | 203.65 us |
| Compile | `compile/guard` | one compound guard expression | 778.81 ns |
| Compile | `compile/wide_linear_script` | 512 variables and assignments | 517.16 us |
| Compile | `compile/expression_heavy_script` | 512 arithmetic expressions | 641.45 us |
| VM | `vm/counter_loop` | 2,000 loop iterations | 27.328 us |
| VM | `vm/growing_list` | 2,000 list appends | 115.26 us |
| VM | `vm/growing_string` | 2,000 string appends | 109.09 us |
| VM | `vm/interpolation` | 1,500 interpolations | 276.84 us |
| VM | `vm/string_reads` | 1,500 reads over a 16 KiB string | 1.3896 ms |
| VM | `vm/wide_linear_script` | 512 variables and assignments | 23.013 us |
| Machine | `machine/create_wide/validate` | validate a 512-variable program | 4.630 us |
| Machine | `machine/create_wide/reuse_validation` | reuse the validation proof | 1.254 us |
| Machine | `machine/restart` | restart an expression-heavy runner | 22.275 us |
| Evaluation | `eval/tree_walk` | one compound guard expression | 96.278 ns |
| Host | `vm/run_with_host_yield` | one bound host yield | 3.408 us |
| Host | `host/single_effect_roundtrip` | 256 effects, one resume at a time | 37.435 us |
| Host | `host/batched_effect_roundtrip` | 256 effects, batches of 64 | 41.034 us |
| Artifact | `artifact/source_compile` | 256-variable source | 255.35 us |
| Artifact | `artifact/binary_decode` | 164,738-byte artifact | 1.0433 ms |
| Artifact | `artifact/cache_hit` | cached 164,738-byte artifact | 1.0633 ms |
| Artifact | `artifact/cache_miss` | missing cache entry lookup | 1.5304 us |
| Pure module | `pure/invoke/one_reused` | one integer argument, reused invoker | 82.340 ns |
| Pure module | `pure/invoke/map_reused` | one named integer, reused invoker | 189.90 ns |
| Pure module | `pure/invoke/map_fresh` | one named integer, fresh map | 544.04 ns |
| Profile | `machine/profile/enabled` | 256-iteration profile workload | 14.825 us |
| Profile | `machine/profile/disabled` | same workload without profile | 14.400 us |
| Memory | `memory/execution_image` | 256-variable program | 32.525 us |
| Workload | `workload/dialogue` | dialogue with three host effects | 896.37 ns |
| Workload | `workload/inventory` | 150 list updates and renders | 65.858 us |
| Workload | `workload/mixed` | 100 iterations with conditional effects | 15.049 us |

## P0 boundary metrics

These benchmarks were added with the P0 runtime-boundary work:

| Benchmark | Fixture | Median |
| --- | --- | ---: |
| `snapshot/machine_clone` | finished machine with 256 populated slots | 674.30 ns |
| `snapshot/clone_and_replay` | pending host yield, clone, resume, and RNG continuation | 403.14 ns |
| `queue/push_pop` | 128 bounded events with two values each | 8.3387 us |
| `queue/full_backpressure` | full 64-event queue, reject/pop/push cycle | 130.20 ns |

The two snapshot numbers measure different shapes: the first copies a larger
finished frame, while the second clones a machine paused at a host boundary.
They are not interchangeable estimates for every snapshot size.

## C ABI and Wasm boundaries

The boundary-specific benches use the same Criterion settings on the same host:

| Boundary | Benchmark | Fixture | Median |
| --- | --- | --- | ---: |
| C ABI | `host/c_abi_batch` | 128 effects, batches of 64 | 11.106 us |
| Wasm runtime | `host/wasm_batch` | 128 effects, three runtime batches | 131.63 us |

The C benchmark measures C-handle loading already performed during setup and
measures machine creation plus batch execution inside the iteration. The Wasm
benchmark measures runtime-machine creation from serialized JSON plus three
batch calls. Neither number is a browser end-to-end latency measurement.

## Reading the snapshot

- The common VM loop fixtures complete in tens to hundreds of microseconds for
  1,500-2,000 iterations. Collection and interpolation paths are materially
  more expensive than scalar counter updates because they allocate or validate
  larger values.
- Reusing a validation proof is cheaper than validating a public program on
  every machine creation. This is the intended embedding path for repeated
  runs.
- Artifact decode and cache-hit measurements include bounded decoding and
  validation. They are not expected to beat a warm in-process runner, and the
  validation cost is part of the untrusted-input boundary.
- The queue numbers describe the bounded host queue itself. They do not include
  application dispatch, serialization, or consumer work.
- Cross-engine speed claims are deliberately absent. Future comparisons must
  pin versions, features, hardware, and equivalent semantics first.

## Reproduction

Run correctness checks before collecting a baseline:

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Collect the pipeline, C ABI, and Wasm numbers with the same settings:

```sh
cargo bench -p velin --bench pipeline -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
cargo bench -p velin-capi --bench c_api -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
cargo bench -p velin-wasm --features runtime --bench runtime -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
```

For a release-quality comparison, use the default Criterion sample size and
measurement time, save a named baseline, and repeat on the same machine and
toolchain. A baseline is useful only when its environment and source revision
are recorded next to it.

## CI gate

`benchmarks/performance-baseline.json` stores representative 0.4.0 medians and
release-artifact size ceilings. Run the same latency, C ABI, Wasm, and size
gate used by CI with:

```sh
bash scripts/check-performance.sh
```

The gate uses a short Criterion run by default (10 samples, 0.1 seconds of
warm-up, and 0.2 seconds of measurement) and allows a 5x cross-host latency
factor. It is intended to catch severe regressions, not replace a
release-quality measurement. Set `VELIN_PERF_SAMPLE_SIZE`,
`VELIN_PERF_WARMUP_TIME`, and `VELIN_PERF_MEASUREMENT_TIME` to collect a longer
run, then append a dated historical snapshot with its environment and source
revision.
