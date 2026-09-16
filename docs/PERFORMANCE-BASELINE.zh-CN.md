# 当前性能基线

[English](PERFORMANCE-BASELINE.md)

这是一份可复现的本地性能快照，不是发布承诺。除非语言子集、工作负载、工具链、硬件和计时边界完全一致，否则不应将这些数字与其他引擎直接比较。

## 测量环境

| 项目 | 值 |
| --- | --- |
| Velin 版本 | `0.4.0` |
| 源码版本 | `86ab5f0`，加上本地文档和 benchmark 修改；运行时代码未变 |
| 主机 | Apple MacBook Pro，Apple M3 Pro，12 核，36 GB 内存 |
| 系统 | macOS Darwin `25.6.0`，`arm64` |
| Rust | `rustc 1.98.0`，LLVM `22.1.8` |
| Cargo | `cargo 1.98.0` |
| 构建 | Criterion `bench` profile，使用 workspace release 设置 |
| 日期 | 2026-09-16（Asia/Shanghai） |

采集期间没有修改运行时实现；工作区包含路线图中新增的 P0 benchmark harness。

## 版本规则

本文档中的指标是 Velin `0.4.0` 的首份基线。后续版本发布时，在文档中追加带版本号和日期的独立段落，并记录对应的环境与测量结果。不要覆盖 `0.4.0` 的数值；文档需要保留可比较的历史快照序列。

## 测量方法

pipeline、C ABI 和 Wasm benchmark 使用相同的缩短版 Criterion 参数，以便在一次采样中得到快照：

```text
sample-size       20
warm-up-time      0.5 s
measurement-time  1 s
plots             disabled
```

表格使用 Criterion 输出 `[low median high]` 中间的 median 值。Criterion 输出的 `change` 行没有写入表格：本机 `base` 目录没有记录硬件和工具链来源，因此这些比较不能作为可信的回归结论。

## Pipeline 指标

以下是每次 benchmark 调用的耗时，不是每条源码语句的耗时。表中保留了 fixture 大小，便于解释数字。

| 区域 | Benchmark | Fixture | Median |
| --- | --- | --- | ---: |
| 解析 | `parse/guard` | 一个复合 guard 表达式 | 1.8343 us |
| 解析 | `parse/wide_linear_script` | 512 个变量和赋值 | 339.32 us |
| 解析 | `parse/host_calls` | 512 个宿主 effect | 296.22 us |
| 检查 | `check/guard` | 一个复合 guard 表达式 | 133.52 ns |
| 检查 | `check/wide_linear_script` | 512 个变量和赋值 | 38.175 us |
| 检查 | `check/expression_heavy_script` | 512 个算术表达式 | 203.65 us |
| 编译 | `compile/guard` | 一个复合 guard 表达式 | 778.81 ns |
| 编译 | `compile/wide_linear_script` | 512 个变量和赋值 | 517.16 us |
| 编译 | `compile/expression_heavy_script` | 512 个算术表达式 | 641.45 us |
| VM | `vm/counter_loop` | 2,000 次循环 | 27.328 us |
| VM | `vm/growing_list` | 2,000 次列表追加 | 115.26 us |
| VM | `vm/growing_string` | 2,000 次字符串追加 | 109.09 us |
| VM | `vm/interpolation` | 1,500 次插值 | 276.84 us |
| VM | `vm/string_reads` | 在 16 KiB 字符串上读取 1,500 次 | 1.3896 ms |
| VM | `vm/wide_linear_script` | 512 个变量和赋值 | 23.013 us |
| Machine | `machine/create_wide/validate` | 验证 512 变量程序 | 4.630 us |
| Machine | `machine/create_wide/reuse_validation` | 复用验证证明 | 1.254 us |
| Machine | `machine/restart` | 重启 expression-heavy runner | 22.275 us |
| 求值器 | `eval/tree_walk` | 一个复合 guard 表达式 | 96.278 ns |
| 宿主 | `vm/run_with_host_yield` | 一个有返回值的宿主 yield | 3.408 us |
| 宿主 | `host/single_effect_roundtrip` | 256 个 effect，逐个 resume | 37.435 us |
| 宿主 | `host/batched_effect_roundtrip` | 256 个 effect，每批 64 个 | 41.034 us |
| Artifact | `artifact/source_compile` | 256 变量源码 | 255.35 us |
| Artifact | `artifact/binary_decode` | 164,738 字节 artifact | 1.0433 ms |
| Artifact | `artifact/cache_hit` | 已缓存的 164,738 字节 artifact | 1.0633 ms |
| Artifact | `artifact/cache_miss` | 缺失缓存项的查询 | 1.5304 us |
| 纯模块 | `pure/invoke/one_reused` | 一个整数参数，复用 invoker | 82.340 ns |
| 纯模块 | `pure/invoke/map_reused` | 一个具名整数，复用 invoker | 189.90 ns |
| 纯模块 | `pure/invoke/map_fresh` | 一个具名整数，新建 map | 544.04 ns |
| Profile | `machine/profile/enabled` | 256 次循环的 profile 工作负载 | 14.825 us |
| Profile | `machine/profile/disabled` | 相同工作负载但关闭 profile | 14.400 us |
| 内存 | `memory/execution_image` | 256 变量程序 | 32.525 us |
| 工作负载 | `workload/dialogue` | 三个宿主 effect 的对话 | 896.37 ns |
| 工作负载 | `workload/inventory` | 150 次列表更新与渲染 | 65.858 us |
| 工作负载 | `workload/mixed` | 100 次带条件 effect 的循环 | 15.049 us |

## P0 边界指标

以下 benchmark 随 P0 运行时边界工作新增：

| Benchmark | Fixture | Median |
| --- | --- | ---: |
| `snapshot/machine_clone` | 256 个已填充槽位的已完成机器 | 674.30 ns |
| `snapshot/clone_and_replay` | 宿主 yield 暂停、克隆、resume 和随机状态延续 | 403.14 ns |
| `queue/push_pop` | 128 个有界事件，每个包含两个值 | 8.3387 us |
| `queue/full_backpressure` | 64 个事件的满队列，拒绝/pop/push 循环 | 130.20 ns |

两个 snapshot 数字对应不同形状：前者复制更大的已完成帧，后者复制停在宿主边界的机器。它们不能互相替代，也不能代表所有 snapshot 大小。

## C ABI 与 Wasm 边界

边界 benchmark 在同一主机上使用相同 Criterion 参数：

| 边界 | Benchmark | Fixture | Median |
| --- | --- | --- | ---: |
| C ABI | `host/c_abi_batch` | 128 个 effect，每批 64 个 | 11.106 us |
| Wasm runtime | `host/wasm_batch` | 128 个 effect，分三批运行 | 131.63 us |

C benchmark 在采样前完成 C handle 加载，采样区间包含 machine 创建和批量执行。Wasm benchmark 的采样区间包含从序列化 JSON 创建 runtime machine 和三次 batch 调用。两者都不是浏览器端到端延迟。

## 如何解读

- 常见 VM loop fixture 在 1,500-2,000 次迭代下通常耗时几十到几百微秒。集合和插值路径需要分配或校验更大的值，因此明显高于标量计数循环。
- 复用验证证明比每次 machine 创建都验证公开程序便宜，这是重复运行时的预期嵌入路径。
- Artifact decode 和 cache hit 包含有界解码与验证，不应期待它们击败进程内的 warm runner；验证成本是输入安全边界的一部分。
- 队列数字只描述有界宿主队列本身，不包含应用分派、序列化或消费者工作。
- 当前不做跨引擎速度宣传。未来比较必须先固定版本、feature、硬件和等价语义。

## 复现

采集基线前先运行正确性检查：

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

使用相同参数采集 pipeline、C ABI 和 Wasm 指标：

```sh
cargo bench -p velin --bench pipeline -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
cargo bench -p velin-capi --bench c_api -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
cargo bench -p velin-wasm --features runtime --bench runtime -- --noplot --sample-size 20 --warm-up-time 0.5 --measurement-time 1 --format terse
```

发布级比较应使用 Criterion 默认的 sample size 和 measurement time，保存具名 baseline，并在同一台机器和同一工具链上重复运行。只有同时记录环境和源码版本，baseline 才有意义。
