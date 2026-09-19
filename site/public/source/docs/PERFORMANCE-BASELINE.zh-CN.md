# 性能报告

[English](PERFORMANCE-BASELINE.md)

性能数据统一存放在 `benchmarks/reports/*.jsonl`。下面的报告区域由这些文件生成：先展示最新的 `<源码标识>-last.jsonl`，再与最新发布快照逐项比较，并保留全部发布快照。

<!-- BEGIN GENERATED PERFORMANCE REPORTS -->
## 最新未发布报告（2026-09-19）

本报告包含 63 个 benchmark，其中 41 个同名指标与 2026-09-16 发布的 Velin `0.4.0` 对比。负数表示更快，正数表示更慢；单次本地采样的细小差异不应单独视为性能回退结论。

### 环境与方法

| 项目 | 值 |
| --- | --- |
| Velin 版本 | `0.4.0` 加未发布变更 |
| 测量源码 | staged 源码 `5c4f3a38269a`（基于 `a9259d9`） |
| 数据文件 | `benchmarks/reports/5c4f3a38269a-last.jsonl` |
| 主机 | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| 系统 | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.1`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.1` |
| 采样 | 20 个样本，0.5 秒预热，1 秒测量, plots disabled |
| 日期 | 2026-09-19 (Asia/Shanghai) |

### Benchmark 结果

| Benchmark | 当前值 | 已发布 `0.4.0` | 变化 |
| --- | ---: | ---: | ---: |
| `artifact/binary_decode` | 392.069 us | 1.0433 ms | -62.4% |
| `artifact/cache_hit` | 411.879 us | 1.0633 ms | -61.3% |
| `artifact/cache_miss` | 1.540 us | 1.530 us | +0.6% |
| `artifact/source_compile` | 276.541 us | 255.350 us | +8.3% |
| `builtin/record_arguments/fresh` | 148.88 ns | - | 新增 |
| `builtin/record_arguments/reused` | 124.21 ns | - | 新增 |
| `check/builtin_heavy_script` | 300.387 us | - | 新增 |
| `check/expression_heavy_script` | 201.597 us | 203.650 us | -1.0% |
| `check/guard` | 137.61 ns | 133.52 ns | +3.1% |
| `check/short_circuit_heavy_script` | 637.427 us | - | 新增 |
| `check/wide_linear_script` | 38.312 us | 38.175 us | +0.4% |
| `compile/expression_heavy_script` | 662.254 us | 641.450 us | +3.2% |
| `compile/guard` | 809.54 ns | 778.81 ns | +3.9% |
| `compile/wide_linear_script` | 549.935 us | 517.160 us | +6.3% |
| `eval/tree_walk` | 96.05 ns | 96.28 ns | -0.2% |
| `host/batched_effect_roundtrip` | 30.980 us | 41.034 us | -24.5% |
| `host/c_abi_batch` | 12.163 us | 11.106 us | +9.5% |
| `host/single_effect_roundtrip` | 39.681 us | 37.435 us | +6.0% |
| `host/wasm_batch` | 13.741 us | 131.630 us | -89.6% |
| `host/wasm_load_and_batch` | 100.903 us | - | 新增 |
| `host/wasm_machine_create_reused` | 192.24 ns | - | 新增 |
| `machine/create_expression_heavy/reuse_validation` | 1.323 us | - | 新增 |
| `machine/create_expression_heavy/validate` | 186.250 us | - | 新增 |
| `machine/create_short_circuit_heavy/reuse_validation` | 1.306 us | - | 新增 |
| `machine/create_short_circuit_heavy/validate` | 400.217 us | - | 新增 |
| `machine/create_wide/reuse_validation` | 1.330 us | 1.254 us | +6.1% |
| `machine/create_wide/validate` | 4.626 us | 4.630 us | -0.1% |
| `machine/profile/disabled` | 14.489 us | 14.400 us | +0.6% |
| `machine/profile/enabled` | 14.994 us | 14.825 us | +1.1% |
| `machine/restart` | 22.570 us | 22.275 us | +1.3% |
| `memory/execution_image` | 32.578 us | 32.525 us | +0.2% |
| `parse/guard` | 1.878 us | 1.834 us | +2.4% |
| `parse/host_calls` | 309.847 us | 296.220 us | +4.6% |
| `parse/wide_linear_script` | 367.799 us | 339.320 us | +8.4% |
| `pure/invoke/map_fresh` | 564.09 ns | 544.04 ns | +3.7% |
| `pure/invoke/map_reused` | 187.61 ns | 189.90 ns | -1.2% |
| `pure/invoke/one_reused` | 88.27 ns | 82.34 ns | +7.2% |
| `queue/full_backpressure` | 129.52 ns | 130.20 ns | -0.5% |
| `queue/push_pop` | 8.237 us | 8.339 us | -1.2% |
| `snapshot/clone_and_replay` | 410.18 ns | 403.14 ns | +1.7% |
| `snapshot/machine_clone` | 667.52 ns | 674.30 ns | -1.0% |
| `vm/boolean_slot_loop` | 31.177 us | - | 新增 |
| `vm/branched_scalar_loop` | 14.465 us | - | 新增 |
| `vm/builtin_loop` | 164.796 us | - | 新增 |
| `vm/constant_folding/folded` | 34.509 us | - | 新增 |
| `vm/constant_folding/runtime_expression` | 123.939 us | - | 新增 |
| `vm/counter_loop` | 26.872 us | 27.328 us | -1.7% |
| `vm/growing_list` | 119.554 us | 115.260 us | +3.7% |
| `vm/growing_string` | 109.803 us | 109.090 us | +0.7% |
| `vm/interpolation` | 281.348 us | 276.840 us | +1.6% |
| `vm/interpolation_mixed_holes` | 113.174 us | - | 新增 |
| `vm/long_register_expression_loop` | 122.701 us | - | 新增 |
| `vm/owned_builtin_loop` | 348.184 us | - | 新增 |
| `vm/propagated_constants` | 21.035 us | - | 新增 |
| `vm/run_with_host_yield` | 3.398 us | 3.408 us | -0.3% |
| `vm/scalar_reassignment` | 45.966 us | - | 新增 |
| `vm/short_scalar_expression` | 398.18 ns | - | 新增 |
| `vm/small_register_expression_loop` | 112.743 us | - | 新增 |
| `vm/string_reads` | 1.3991 ms | 1.3896 ms | +0.7% |
| `vm/wide_linear_script` | 23.517 us | 23.013 us | +2.2% |
| `workload/dialogue` | 915.46 ns | 896.37 ns | +2.1% |
| `workload/inventory` | 66.760 us | 65.858 us | +1.4% |
| `workload/mixed` | 14.896 us | 15.049 us | -1.0% |

### 产物体积

| 产物 | 字节数 | Gzip 字节数 | 仓库上限 |
| --- | ---: | ---: | ---: |
| Runtime-only 示例 | 524,976 | 238,922 | 2,000,000 / 800,000 |
| C runtime 静态库 | 24,715,192 | 7,851,830 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 418,352 | 142,630 | 650,000 / 250,000 |
| Source-to-run Wasm | 644,708 | 234,767 | 1,000,000 / 400,000 |
| 完整 CLI | 1,280,384 | 567,130 | 5,000,000 / 2,000,000 |

## 已发布快照

### Velin `0.4.0` (2026-09-16)

以下数值来自对应 JSONL，是后续未发布报告的比较基线。

| 项目 | 值 |
| --- | --- |
| Velin 版本 | `0.4.0` |
| 测量源码 | commit `86ab5f0` |
| 数据文件 | `benchmarks/reports/86ab5f0-0.4.0.jsonl` |
| 主机 | Apple M3 Pro, 12 logical cores, 36.0 GiB RAM |
| 系统 | Darwin 25.6.0, arm64 |
| Rust | `rustc 1.98.0`, LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| 采样 | 20 个样本，0.5 秒预热，1 秒测量, plots disabled |
| 日期 | 2026-09-16 (Asia/Shanghai) |

#### Benchmark 结果

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

#### 产物体积

| 产物 | 字节数 | Gzip 字节数 | 仓库上限 |
| --- | ---: | ---: | ---: |
| Runtime-only 示例 | 524,960 | 239,161 | 2,000,000 / 800,000 |
| C runtime 静态库 | 24,725,352 | 7,854,007 | 35,000,000 / 12,000,000 |
| Runtime-only Wasm | 414,621 | 142,084 | 650,000 / 250,000 |
| Source-to-run Wasm | 642,938 | 233,886 | 1,000,000 / 400,000 |
| 完整 CLI | 1,296,800 | 574,136 | 5,000,000 / 2,000,000 |
<!-- END GENERATED PERFORMANCE REPORTS -->

## 记录

运行完整 benchmark，并写入 `<源码标识>-last.jsonl`：

```sh
node scripts/record-performance.mjs
```

发布时冻结为 `<源码标识>-<版本>.jsonl`：

```sh
node scripts/record-performance.mjs --release 0.4.1
```

只根据现有 JSONL 生成文档，不重新测量：

```sh
node scripts/record-performance.mjs --generate-only
```

每个 clone 只需执行一次以下配置启用本地 pre-commit hook：

```sh
git config core.hooksPath .githooks
```

hook 会运行完整测量，并暂存 JSONL 报告和生成后的文档；如果存在 staged 内容，源码标识会基于 staged 内容计算，因此无关的未暂存工作可以保留。可通过 `VELIN_PERF_SAMPLE_SIZE`、`VELIN_PERF_WARMUP_TIME` 和 `VELIN_PERF_MEASUREMENT_TIME` 调整采样。

## 门禁

```sh
bash scripts/check-performance.sh
```
