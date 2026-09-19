# Performance Reports

[简体中文](PERFORMANCE-BASELINE.zh-CN.md)

Performance data is stored in `benchmarks/reports/*.jsonl`. The report section below is generated from those files: it shows the newest `<source>-last.jsonl`, compares it with the newest released snapshot, and retains every released snapshot.

<!-- BEGIN GENERATED PERFORMANCE REPORTS -->
## Latest unreleased report (2026-09-19)

This report contains 64 benchmarks. 41 matching metrics are compared with Velin `0.4.0`, released on 2026-09-16. Negative changes are faster and positive changes are slower; small differences from one local run are not regression claims.

### Environment and method

| Item | Value |
| --- | --- |
| Velin version | `0.5.0` plus unreleased changes |
| Measured source | staged source `fde04f0f6528` (base `6c215f8`) |
| Data file | `benchmarks/reports/fde04f0f6528-last.jsonl` |
| Host | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| OS | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| Sampling | 20 samples, 0.5s warm-up, 1s measurement, plots disabled |
| Date | 2026-09-19 (Asia/Shanghai) |

### Benchmark results

| Benchmark | Current | Released `0.4.0` | Change |
| --- | ---: | ---: | ---: |
| `artifact/binary_decode` | 389.466 us | 1.0433 ms | -62.7% |
| `artifact/cache_hit` | 411.019 us | 1.0633 ms | -61.3% |
| `artifact/cache_miss` | 1.567 us | 1.530 us | +2.4% |
| `artifact/source_compile` | 265.195 us | 255.350 us | +3.9% |
| `builtin/record_arguments/fresh` | 155.28 ns | - | new |
| `builtin/record_arguments/reused` | 123.18 ns | - | new |
| `check/builtin_heavy_script` | 297.890 us | - | new |
| `check/expression_heavy_script` | 203.804 us | 203.650 us | +0.1% |
| `check/guard` | 138.34 ns | 133.52 ns | +3.6% |
| `check/short_circuit_heavy_script` | 640.108 us | - | new |
| `check/wide_linear_script` | 38.111 us | 38.175 us | -0.2% |
| `compile/expression_heavy_script` | 653.536 us | 641.450 us | +1.9% |
| `compile/guard` | 788.53 ns | 778.81 ns | +1.2% |
| `compile/wide_linear_script` | 533.351 us | 517.160 us | +3.1% |
| `eval/tree_walk` | 91.17 ns | 96.28 ns | -5.3% |
| `host/batched_effect_roundtrip` | 31.194 us | 41.034 us | -24.0% |
| `host/c_abi_batch` | 11.935 us | 11.106 us | +7.5% |
| `host/single_effect_roundtrip` | 39.389 us | 37.435 us | +5.2% |
| `host/wasm_batch` | 13.481 us | 131.630 us | -89.8% |
| `host/wasm_load_and_batch` | 99.856 us | - | new |
| `host/wasm_machine_create_reused` | 190.96 ns | - | new |
| `machine/create_expression_heavy/reuse_validation` | 1.314 us | - | new |
| `machine/create_expression_heavy/validate` | 181.696 us | - | new |
| `machine/create_short_circuit_heavy/reuse_validation` | 1.302 us | - | new |
| `machine/create_short_circuit_heavy/validate` | 396.382 us | - | new |
| `machine/create_wide/reuse_validation` | 1.331 us | 1.254 us | +6.1% |
| `machine/create_wide/validate` | 4.607 us | 4.630 us | -0.5% |
| `machine/profile/disabled` | 14.436 us | 14.400 us | +0.3% |
| `machine/profile/enabled` | 14.805 us | 14.825 us | -0.1% |
| `machine/restart` | 22.633 us | 22.275 us | +1.6% |
| `memory/execution_image` | 32.444 us | 32.525 us | -0.2% |
| `parse/guard` | 1.898 us | 1.834 us | +3.5% |
| `parse/host_calls` | 305.028 us | 296.220 us | +3.0% |
| `parse/wide_linear_script` | 353.855 us | 339.320 us | +4.3% |
| `pure/invoke/map_fresh` | 555.92 ns | 544.04 ns | +2.2% |
| `pure/invoke/map_reused` | 185.04 ns | 189.90 ns | -2.6% |
| `pure/invoke/one_reused` | 88.05 ns | 82.34 ns | +6.9% |
| `queue/full_backpressure` | 125.68 ns | 130.20 ns | -3.5% |
| `queue/push_pop` | 8.276 us | 8.339 us | -0.8% |
| `snapshot/clone_and_replay` | 404.64 ns | 403.14 ns | +0.4% |
| `snapshot/machine_clone` | 693.11 ns | 674.30 ns | +2.8% |
| `vm/boolean_slot_loop` | 34.878 us | - | new |
| `vm/branched_scalar_loop` | 16.122 us | - | new |
| `vm/builtin_loop` | 181.387 us | - | new |
| `vm/constant_folding/folded` | 35.454 us | - | new |
| `vm/constant_folding/runtime_expression` | 123.114 us | - | new |
| `vm/counter_loop` | 30.618 us | 27.328 us | +12.0% |
| `vm/growing_list` | 116.626 us | 115.260 us | +1.2% |
| `vm/growing_string` | 109.698 us | 109.090 us | +0.6% |
| `vm/interpolation` | 299.568 us | 276.840 us | +8.2% |
| `vm/interpolation_mixed_holes` | 119.382 us | - | new |
| `vm/long_register_expression_loop` | 122.969 us | - | new |
| `vm/owned_builtin_loop` | 328.027 us | - | new |
| `vm/propagated_constants` | 21.197 us | - | new |
| `vm/run_with_host_yield` | 3.405 us | 3.408 us | -0.1% |
| `vm/scalar_reassignment` | 50.012 us | - | new |
| `vm/short_scalar_expression` | 406.76 ns | - | new |
| `vm/small_register_expression_loop` | 111.105 us | - | new |
| `vm/string_reads` | 1.4074 ms | 1.3896 ms | +1.3% |
| `vm/wide_linear_script` | 23.663 us | 23.013 us | +2.8% |
| `workload/composable_end_to_end` | 33.217 us | - | new |
| `workload/dialogue` | 935.39 ns | 896.37 ns | +4.4% |
| `workload/inventory` | 66.828 us | 65.858 us | +1.5% |
| `workload/mixed` | 14.838 us | 15.049 us | -1.4% |

### Artifact sizes

| Artifact | Bytes | Gzip bytes | Checked-in ceiling |
| --- | ---: | ---: | ---: |
| Runtime-only example | 524,960 | 239,020 | 2,000,000 / 800,000 |
| C runtime static library | 24,728,424 | 7,853,856 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 418,260 | 142,598 | 650,000 / 250,000 |
| Source-to-run Wasm | 645,471 | 236,354 | 1,000,000 / 400,000 |
| Full CLI | 1,296,912 | 568,766 | 5,000,000 / 2,000,000 |

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
node scripts/record-performance.mjs --release 0.5.0
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

The repository toolchain is pinned by `rust-toolchain.toml` so unreleased and release measurements use the same compiler by default. Recording a new unreleased report removes the previous `*-last.jsonl`; versioned release reports are retained permanently.

## Gate

```sh
bash scripts/check-performance.sh
```
