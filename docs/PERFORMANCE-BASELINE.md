# Performance Reports

[简体中文](PERFORMANCE-BASELINE.zh-CN.md)

Performance data is stored in `benchmarks/reports/*.jsonl`. The report section below is generated from those files: it shows the newest `<source>-last.jsonl`, compares it with the newest released snapshot, and retains every released snapshot.

<!-- BEGIN GENERATED PERFORMANCE REPORTS -->
## Latest unreleased report (2026-09-19)

This report contains 63 benchmarks. 41 matching metrics are compared with Velin `0.4.0`, released on 2026-09-16. Negative changes are faster and positive changes are slower; small differences from one local run are not regression claims.

### Environment and method

| Item | Value |
| --- | --- |
| Velin version | `0.4.0` plus unreleased changes |
| Measured source | staged source `fd8645a019b9` (base `e3e808b`) |
| Data file | `benchmarks/reports/fd8645a019b9-last.jsonl` |
| Host | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| OS | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.1`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.1` |
| Sampling | 20 samples, 0.5s warm-up, 1s measurement, plots disabled |
| Date | 2026-09-19 (Asia/Shanghai) |

### Benchmark results

| Benchmark | Current | Released `0.4.0` | Change |
| --- | ---: | ---: | ---: |
| `artifact/binary_decode` | 292.751 us | 1.0433 ms | -71.9% |
| `artifact/cache_hit` | 309.910 us | 1.0633 ms | -70.9% |
| `artifact/cache_miss` | 1.148 us | 1.530 us | -25.0% |
| `artifact/source_compile` | 199.803 us | 255.350 us | -21.8% |
| `builtin/record_arguments/fresh` | 116.63 ns | - | new |
| `builtin/record_arguments/reused` | 96.72 ns | - | new |
| `check/builtin_heavy_script` | 223.688 us | - | new |
| `check/expression_heavy_script` | 153.179 us | 203.650 us | -24.8% |
| `check/guard` | 106.84 ns | 133.52 ns | -20.0% |
| `check/short_circuit_heavy_script` | 494.078 us | - | new |
| `check/wide_linear_script` | 29.068 us | 38.175 us | -23.9% |
| `compile/expression_heavy_script` | 506.723 us | 641.450 us | -21.0% |
| `compile/guard` | 626.99 ns | 778.81 ns | -19.5% |
| `compile/wide_linear_script` | 423.041 us | 517.160 us | -18.2% |
| `eval/tree_walk` | 69.51 ns | 96.28 ns | -27.8% |
| `host/batched_effect_roundtrip` | 23.710 us | 41.034 us | -42.2% |
| `host/c_abi_batch` | 9.504 us | 11.106 us | -14.4% |
| `host/single_effect_roundtrip` | 30.365 us | 37.435 us | -18.9% |
| `host/wasm_batch` | 10.550 us | 131.630 us | -92.0% |
| `host/wasm_load_and_batch` | 78.482 us | - | new |
| `host/wasm_machine_create_reused` | 147.79 ns | - | new |
| `machine/create_expression_heavy/reuse_validation` | 972.93 ns | - | new |
| `machine/create_expression_heavy/validate` | 139.797 us | - | new |
| `machine/create_short_circuit_heavy/reuse_validation` | 980.60 ns | - | new |
| `machine/create_short_circuit_heavy/validate` | 301.367 us | - | new |
| `machine/create_wide/reuse_validation` | 956.98 ns | 1.254 us | -23.7% |
| `machine/create_wide/validate` | 3.492 us | 4.630 us | -24.6% |
| `machine/profile/disabled` | 10.919 us | 14.400 us | -24.2% |
| `machine/profile/enabled` | 11.250 us | 14.825 us | -24.1% |
| `machine/restart` | 17.139 us | 22.275 us | -23.1% |
| `memory/execution_image` | 24.216 us | 32.525 us | -25.5% |
| `parse/guard` | 1.436 us | 1.834 us | -21.7% |
| `parse/host_calls` | 224.864 us | 296.220 us | -24.1% |
| `parse/wide_linear_script` | 264.038 us | 339.320 us | -22.2% |
| `pure/invoke/map_fresh` | 414.31 ns | 544.04 ns | -23.8% |
| `pure/invoke/map_reused` | 142.02 ns | 189.90 ns | -25.2% |
| `pure/invoke/one_reused` | 66.78 ns | 82.34 ns | -18.9% |
| `queue/full_backpressure` | 93.70 ns | 130.20 ns | -28.0% |
| `queue/push_pop` | 6.316 us | 8.339 us | -24.3% |
| `snapshot/clone_and_replay` | 301.80 ns | 403.14 ns | -25.1% |
| `snapshot/machine_clone` | 494.95 ns | 674.30 ns | -26.6% |
| `vm/boolean_slot_loop` | 24.233 us | - | new |
| `vm/branched_scalar_loop` | 10.858 us | - | new |
| `vm/builtin_loop` | 123.988 us | - | new |
| `vm/constant_folding/folded` | 26.445 us | - | new |
| `vm/constant_folding/runtime_expression` | 93.172 us | - | new |
| `vm/counter_loop` | 19.970 us | 27.328 us | -26.9% |
| `vm/growing_list` | 89.105 us | 115.260 us | -22.7% |
| `vm/growing_string` | 96.452 us | 109.090 us | -11.6% |
| `vm/interpolation` | 235.137 us | 276.840 us | -15.1% |
| `vm/interpolation_mixed_holes` | 84.640 us | - | new |
| `vm/long_register_expression_loop` | 91.934 us | - | new |
| `vm/owned_builtin_loop` | 243.132 us | - | new |
| `vm/propagated_constants` | 15.736 us | - | new |
| `vm/run_with_host_yield` | 2.538 us | 3.408 us | -25.5% |
| `vm/scalar_reassignment` | 35.002 us | - | new |
| `vm/short_scalar_expression` | 301.01 ns | - | new |
| `vm/small_register_expression_loop` | 83.412 us | - | new |
| `vm/string_reads` | 1.0458 ms | 1.3896 ms | -24.7% |
| `vm/wide_linear_script` | 17.285 us | 23.013 us | -24.9% |
| `workload/dialogue` | 696.63 ns | 896.37 ns | -22.3% |
| `workload/inventory` | 49.897 us | 65.858 us | -24.2% |
| `workload/mixed` | 11.197 us | 15.049 us | -25.6% |

### Artifact sizes

| Artifact | Bytes | Gzip bytes | Checked-in ceiling |
| --- | ---: | ---: | ---: |
| Runtime-only example | 524,976 | 238,922 | 2,000,000 / 800,000 |
| C runtime static library | 24,715,192 | 7,851,830 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 418,352 | 142,630 | 650,000 / 250,000 |
| Source-to-run Wasm | 645,463 | 235,018 | 1,000,000 / 400,000 |
| Full CLI | 1,296,896 | 567,523 | 5,000,000 / 2,000,000 |

## Released snapshots

### Velin `0.4.0` (2026-09-16)

These values come from the corresponding JSONL file and form the comparison baseline for later unreleased reports.

| Item | Value |
| --- | --- |
| Velin version | `0.4.0` |
| Measured source | commit `86ab5f0` |
| Data file | `benchmarks/reports/86ab5f0-0.4.0.jsonl` |
| Host | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| OS | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| Sampling | 20 samples, 0.5s warm-up, 1s measurement, plots disabled |
| Date | 2026-09-16 (Asia/Shanghai) |

#### Benchmark results

| Benchmark | Estimate |
| --- | ---: |
| `artifact/binary_decode` | 1.0433 ms |
| `artifact/cache_hit` | 1.0633 ms |
| `artifact/cache_miss` | 1.530 us |
| `artifact/source_compile` | 255.350 us |
| `check/expression_heavy_script` | 203.650 us |
| `check/guard` | 133.52 ns |
| `check/wide_linear_script` | 38.175 us |
| `compile/expression_heavy_script` | 641.450 us |
| `compile/guard` | 778.81 ns |
| `compile/wide_linear_script` | 517.160 us |
| `eval/tree_walk` | 96.28 ns |
| `host/batched_effect_roundtrip` | 41.034 us |
| `host/c_abi_batch` | 11.106 us |
| `host/single_effect_roundtrip` | 37.435 us |
| `host/wasm_batch` | 131.630 us |
| `machine/create_wide/reuse_validation` | 1.254 us |
| `machine/create_wide/validate` | 4.630 us |
| `machine/profile/disabled` | 14.400 us |
| `machine/profile/enabled` | 14.825 us |
| `machine/restart` | 22.275 us |
| `memory/execution_image` | 32.525 us |
| `parse/guard` | 1.834 us |
| `parse/host_calls` | 296.220 us |
| `parse/wide_linear_script` | 339.320 us |
| `pure/invoke/map_fresh` | 544.04 ns |
| `pure/invoke/map_reused` | 189.90 ns |
| `pure/invoke/one_reused` | 82.34 ns |
| `queue/full_backpressure` | 130.20 ns |
| `queue/push_pop` | 8.339 us |
| `snapshot/clone_and_replay` | 403.14 ns |
| `snapshot/machine_clone` | 674.30 ns |
| `vm/counter_loop` | 27.328 us |
| `vm/growing_list` | 115.260 us |
| `vm/growing_string` | 109.090 us |
| `vm/interpolation` | 276.840 us |
| `vm/run_with_host_yield` | 3.408 us |
| `vm/string_reads` | 1.3896 ms |
| `vm/wide_linear_script` | 23.013 us |
| `workload/dialogue` | 896.37 ns |
| `workload/inventory` | 65.858 us |
| `workload/mixed` | 15.049 us |

#### Artifact sizes

| Artifact | Bytes | Gzip bytes | Checked-in ceiling |
| --- | ---: | ---: | ---: |
| Runtime-only example | 524,960 | 239,161 | 2,000,000 / 800,000 |
| C runtime static library | 24,725,352 | 7,854,007 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 414,621 | 142,084 | 650,000 / 250,000 |
| Source-to-run Wasm | 642,938 | 233,886 | 1,000,000 / 400,000 |
| Full CLI | 1,296,800 | 574,136 | 5,000,000 / 2,000,000 |
<!-- END GENERATED PERFORMANCE REPORTS -->

## Recording

Run the complete benchmark suite and write `<source>-last.jsonl`:

```sh
node scripts/record-performance.mjs
```

Freeze a release snapshot as `<source>-<version>.jsonl`:

```sh
node scripts/record-performance.mjs --release 0.4.1
```

Regenerate documents without measuring:

```sh
node scripts/record-performance.mjs --generate-only
```

Enable the local pre-commit hook once per clone:

```sh
git config core.hooksPath .githooks
```

The hook runs the full measurement and stages the JSONL report and generated documents. It identifies staged content when available, so unrelated unstaged work can remain in the working tree. Configure sample count and timing with `VELIN_PERF_SAMPLE_SIZE`, `VELIN_PERF_WARMUP_TIME`, and `VELIN_PERF_MEASUREMENT_TIME`.

## Gate

```sh
bash scripts/check-performance.sh
```
