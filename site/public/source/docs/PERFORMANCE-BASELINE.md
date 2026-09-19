# Performance Reports

[简体中文](PERFORMANCE-BASELINE.zh-CN.md)

Performance data is stored in `benchmarks/reports/*.jsonl`. The report section below is generated from those files: it shows the newest `<source>-last.jsonl`, compares it with the newest released snapshot, and retains every released snapshot.

<!-- BEGIN GENERATED PERFORMANCE REPORTS -->
## Latest unreleased report (2026-09-19)

This report contains 64 benchmarks. 64 matching metrics are compared with Velin `0.5.0`, released on 2026-09-19. Negative changes are faster and positive changes are slower; small differences from one local run are not regression claims.

### Environment and method

| Item | Value |
| --- | --- |
| Velin version | `0.5.0` plus unreleased changes |
| Measured source | commit `9bee1c6` |
| Data file | `benchmarks/reports/9bee1c6-last.jsonl` |
| Host | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| OS | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| Sampling | 20 samples, 0.5s warm-up, 1s measurement, plots disabled |
| Date | 2026-09-19 (Asia/Shanghai) |

### Benchmark results

| Benchmark | Current | Released `0.5.0` | Change |
| --- | ---: | ---: | ---: |
| `artifact/binary_decode` | 383.760 us | 385.747 us | -0.5% |
| `artifact/cache_hit` | 409.938 us | 439.852 us | -6.8% |
| `artifact/cache_miss` | 1.584 us | 1.656 us | -4.3% |
| `artifact/source_compile` | 268.567 us | 264.755 us | +1.4% |
| `builtin/record_arguments/fresh` | 161.28 ns | 153.79 ns | +4.9% |
| `builtin/record_arguments/reused` | 127.48 ns | 123.56 ns | +3.2% |
| `check/builtin_heavy_script` | 311.288 us | 297.376 us | +4.7% |
| `check/expression_heavy_script` | 205.548 us | 199.782 us | +2.9% |
| `check/guard` | 139.79 ns | 138.13 ns | +1.2% |
| `check/short_circuit_heavy_script` | 656.619 us | 625.248 us | +5.0% |
| `check/wide_linear_script` | 37.966 us | 38.330 us | -1.0% |
| `compile/expression_heavy_script` | 679.120 us | 647.780 us | +4.8% |
| `compile/guard` | 810.56 ns | 789.18 ns | +2.7% |
| `compile/wide_linear_script` | 584.439 us | 543.046 us | +7.6% |
| `eval/tree_walk` | 92.71 ns | 91.46 ns | +1.4% |
| `host/batched_effect_roundtrip` | 30.772 us | 30.975 us | -0.7% |
| `host/c_abi_batch` | 12.042 us | 12.290 us | -2.0% |
| `host/single_effect_roundtrip` | 39.672 us | 40.318 us | -1.6% |
| `host/wasm_batch` | 13.367 us | 13.658 us | -2.1% |
| `host/wasm_load_and_batch` | 98.779 us | 100.435 us | -1.6% |
| `host/wasm_machine_create_reused` | 192.95 ns | 202.22 ns | -4.6% |
| `machine/create_expression_heavy/reuse_validation` | 1.374 us | 1.317 us | +4.4% |
| `machine/create_expression_heavy/validate` | 186.640 us | 182.850 us | +2.1% |
| `machine/create_short_circuit_heavy/reuse_validation` | 1.302 us | 1.330 us | -2.1% |
| `machine/create_short_circuit_heavy/validate` | 406.154 us | 401.061 us | +1.3% |
| `machine/create_wide/reuse_validation` | 1.283 us | 1.340 us | -4.2% |
| `machine/create_wide/validate` | 4.603 us | 4.643 us | -0.8% |
| `machine/profile/disabled` | 14.450 us | 14.433 us | +0.1% |
| `machine/profile/enabled` | 15.189 us | 14.770 us | +2.8% |
| `machine/restart` | 22.616 us | 22.892 us | -1.2% |
| `memory/execution_image` | 32.335 us | 32.735 us | -1.2% |
| `parse/guard` | 1.872 us | 1.861 us | +0.6% |
| `parse/host_calls` | 303.723 us | 301.667 us | +0.7% |
| `parse/wide_linear_script` | 360.164 us | 352.720 us | +2.1% |
| `pure/invoke/map_fresh` | 665.54 ns | 584.60 ns | +13.8% |
| `pure/invoke/map_reused` | 183.75 ns | 202.34 ns | -9.2% |
| `pure/invoke/one_reused` | 88.34 ns | 89.70 ns | -1.5% |
| `queue/full_backpressure` | 125.06 ns | 124.35 ns | +0.6% |
| `queue/push_pop` | 8.303 us | 8.167 us | +1.7% |
| `snapshot/clone_and_replay` | 416.62 ns | 408.78 ns | +1.9% |
| `snapshot/machine_clone` | 697.05 ns | 691.61 ns | +0.8% |
| `vm/boolean_slot_loop` | 33.874 us | 33.199 us | +2.0% |
| `vm/branched_scalar_loop` | 14.944 us | 15.714 us | -4.9% |
| `vm/builtin_loop` | 163.548 us | 163.114 us | +0.3% |
| `vm/constant_folding/folded` | 36.561 us | 35.481 us | +3.0% |
| `vm/constant_folding/runtime_expression` | 126.625 us | 122.726 us | +3.2% |
| `vm/counter_loop` | 28.060 us | 27.384 us | +2.5% |
| `vm/growing_list` | 116.305 us | 118.179 us | -1.6% |
| `vm/growing_string` | 110.836 us | 112.229 us | -1.2% |
| `vm/interpolation` | 298.314 us | 289.549 us | +3.0% |
| `vm/interpolation_mixed_holes` | 111.912 us | 111.120 us | +0.7% |
| `vm/long_register_expression_loop` | 126.278 us | 123.194 us | +2.5% |
| `vm/owned_builtin_loop` | 318.879 us | 345.014 us | -7.6% |
| `vm/propagated_constants` | 21.167 us | 21.248 us | -0.4% |
| `vm/run_with_host_yield` | 3.367 us | 3.401 us | -1.0% |
| `vm/scalar_reassignment` | 47.303 us | 47.512 us | -0.4% |
| `vm/short_scalar_expression` | 412.37 ns | 396.10 ns | +4.1% |
| `vm/small_register_expression_loop` | 111.191 us | 110.563 us | +0.6% |
| `vm/string_reads` | 1.3979 ms | 1.4075 ms | -0.7% |
| `vm/wide_linear_script` | 23.502 us | 23.453 us | +0.2% |
| `workload/composable_end_to_end` | 33.302 us | 33.046 us | +0.8% |
| `workload/dialogue` | 893.61 ns | 1.043 us | -14.3% |
| `workload/inventory` | 66.423 us | 67.276 us | -1.3% |
| `workload/mixed` | 14.824 us | 14.871 us | -0.3% |

### Artifact sizes

| Artifact | Bytes | Gzip bytes | Checked-in ceiling |
| --- | ---: | ---: | ---: |
| Runtime-only example | 524,960 | 239,020 | 2,000,000 / 800,000 |
| C runtime static library | 24,728,424 | 7,853,856 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 418,260 | 142,598 | 650,000 / 250,000 |
| Source-to-run Wasm | 645,471 | 236,354 | 1,000,000 / 400,000 |
| Full CLI | 1,296,912 | 568,766 | 5,000,000 / 2,000,000 |

## Released snapshots

### Velin `0.5.0` (2026-09-19)

These values come from the corresponding JSONL file and form the comparison baseline for later unreleased reports.

| Item | Value |
| --- | --- |
| Velin version | `0.5.0` |
| Measured source | commit `9bee1c6` |
| Data file | `benchmarks/reports/9bee1c6-0.5.0.jsonl` |
| Host | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| OS | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| Sampling | 20 samples, 0.5s warm-up, 1s measurement, plots disabled |
| Date | 2026-09-19 (Asia/Shanghai) |

#### Benchmark results

| Benchmark | Estimate |
| --- | ---: |
| `artifact/binary_decode` | 385.747 us |
| `artifact/cache_hit` | 439.852 us |
| `artifact/cache_miss` | 1.656 us |
| `artifact/source_compile` | 264.755 us |
| `builtin/record_arguments/fresh` | 153.79 ns |
| `builtin/record_arguments/reused` | 123.56 ns |
| `check/builtin_heavy_script` | 297.376 us |
| `check/expression_heavy_script` | 199.782 us |
| `check/guard` | 138.13 ns |
| `check/short_circuit_heavy_script` | 625.248 us |
| `check/wide_linear_script` | 38.330 us |
| `compile/expression_heavy_script` | 647.780 us |
| `compile/guard` | 789.18 ns |
| `compile/wide_linear_script` | 543.046 us |
| `eval/tree_walk` | 91.46 ns |
| `host/batched_effect_roundtrip` | 30.975 us |
| `host/c_abi_batch` | 12.290 us |
| `host/single_effect_roundtrip` | 40.318 us |
| `host/wasm_batch` | 13.658 us |
| `host/wasm_load_and_batch` | 100.435 us |
| `host/wasm_machine_create_reused` | 202.22 ns |
| `machine/create_expression_heavy/reuse_validation` | 1.317 us |
| `machine/create_expression_heavy/validate` | 182.850 us |
| `machine/create_short_circuit_heavy/reuse_validation` | 1.330 us |
| `machine/create_short_circuit_heavy/validate` | 401.061 us |
| `machine/create_wide/reuse_validation` | 1.340 us |
| `machine/create_wide/validate` | 4.643 us |
| `machine/profile/disabled` | 14.433 us |
| `machine/profile/enabled` | 14.770 us |
| `machine/restart` | 22.892 us |
| `memory/execution_image` | 32.735 us |
| `parse/guard` | 1.861 us |
| `parse/host_calls` | 301.667 us |
| `parse/wide_linear_script` | 352.720 us |
| `pure/invoke/map_fresh` | 584.60 ns |
| `pure/invoke/map_reused` | 202.34 ns |
| `pure/invoke/one_reused` | 89.70 ns |
| `queue/full_backpressure` | 124.35 ns |
| `queue/push_pop` | 8.167 us |
| `snapshot/clone_and_replay` | 408.78 ns |
| `snapshot/machine_clone` | 691.61 ns |
| `vm/boolean_slot_loop` | 33.199 us |
| `vm/branched_scalar_loop` | 15.714 us |
| `vm/builtin_loop` | 163.114 us |
| `vm/constant_folding/folded` | 35.481 us |
| `vm/constant_folding/runtime_expression` | 122.726 us |
| `vm/counter_loop` | 27.384 us |
| `vm/growing_list` | 118.179 us |
| `vm/growing_string` | 112.229 us |
| `vm/interpolation` | 289.549 us |
| `vm/interpolation_mixed_holes` | 111.120 us |
| `vm/long_register_expression_loop` | 123.194 us |
| `vm/owned_builtin_loop` | 345.014 us |
| `vm/propagated_constants` | 21.248 us |
| `vm/run_with_host_yield` | 3.401 us |
| `vm/scalar_reassignment` | 47.512 us |
| `vm/short_scalar_expression` | 396.10 ns |
| `vm/small_register_expression_loop` | 110.563 us |
| `vm/string_reads` | 1.4075 ms |
| `vm/wide_linear_script` | 23.453 us |
| `workload/composable_end_to_end` | 33.046 us |
| `workload/dialogue` | 1.043 us |
| `workload/inventory` | 67.276 us |
| `workload/mixed` | 14.871 us |

#### Artifact sizes

| Artifact | Bytes | Gzip bytes | Checked-in ceiling |
| --- | ---: | ---: | ---: |
| Runtime-only example | 524,960 | 239,020 | 2,000,000 / 800,000 |
| C runtime static library | 24,728,424 | 7,853,856 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 418,260 | 142,598 | 650,000 / 250,000 |
| Source-to-run Wasm | 645,471 | 236,354 | 1,000,000 / 400,000 |
| Full CLI | 1,296,912 | 568,766 | 5,000,000 / 2,000,000 |

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
