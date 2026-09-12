# 性能

[English](PERFORMANCE.md)

本页记录近期检查器和 VM 优化带来的相对变化。数据来自本地测量，不是对所有硬件的普遍承诺。它们适合作为回归基线：比较改动时，应在同一台机器上重复同一工作负载。

## 如何看这些数字

CLI 测量包含进程启动、解析、降级和检查。它反映完整脚本检查的用户体验，但不只测算法本身。Criterion 在同一进程内运行，更接近单一路径的隔离测量。

### 结果

静态检查工作负载包含 `N` 个 default，随后是 `N` 个赋值。release CLI 测量清楚地显示了曲线变化：

| `N` | 优化前 | 优化后 |
| ---: | ---: | ---: |
| 250 | 约 13 ms | 约 6.15 ms |
| 500 | 约 35 ms | 约 4.43 ms |
| 1,000 | 约 117 ms | 约 6.31 ms |
| 2,000 | 约 452 ms | 约 9.10 ms |
| 4,000 | 约 1,765 ms | 约 15.38 ms |

最大规模约快 115 倍。小规模会被 CLI 开销主导，因此不应把它们解读为完全单调的曲线。同一优化下，固定单槽位的对照仍接近线性：4,000 次赋值约 12.93 ms，8,000 次约 20.50 ms。

更聚焦的进程内测量如下：

| 基准 | 测量值 | 相对结果 |
| --- | ---: | --- |
| `check/wide_linear_script`（512 个变量） | 约 305.29 µs | 静态检查基线 |
| `machine/create_wide/validate` | 约 146.54 µs | 每次都验证程序 |
| `machine/create_wide/reuse_validation` | 约 720.53 ns | 验证工作量减少约 203 倍 |
| `vm/counter_loop`（2,000 次迭代） | 约 120.97 µs | 相比约 189.06 µs 快约 36% |
| `vm/growing_list`（2,000 次 push） | 约 10.265 ms | 相比约 10.494 ms 快约 2% |

复用值占用量还让一个 2,000 步列表增长脚本的独立 CLI 对比从约 17.0 ms 降到约 14.2 ms，约快 17%。这是端到端对比，不是 Criterion 结果。

## 做了哪些改变

### 静态分析按基本块传播状态

旧检查器在每条控制流指令处复制完整的槽位类型向量，并重新构建 `BTreeMap` 环境。槽位增多时，宽的线性脚本因此呈二次增长。

现在检查器传播基本块所属的状态，只在块边界合并状态。线性代码的工作量因此与代码和槽位数量近似成正比，不再反复复制完整环境。保守类型规则和诊断保持不变。

### 复用值占用量

表达式求值现在同时返回 `Value` 和它的 `DataFootprint`。赋值和宿主载荷统计复用该占用量，不再对同一值进行第二次遍历。标量值走不分配内存的快速路径。

持久化列表和记录仍然使用写时复制语义。这个优化消除了重复统计，但不会把集合更新变成原地修改。

### 安全地复用已验证程序

`ValidatedProgram` 是包住共享 `Arc<Program>` 的验证证明。`Machine::new` 和 `Machine::with_seed` 仍会验证任意外部程序。`ScriptRunner` 路径复用原始编译脚本的验证结果；如果替换公开的 program `Arc`，指针身份不再匹配，就会退回验证路径。安全边界仍然明确。

### VM 复用表达式栈

热循环现在复用 VM 表达式栈，并避免执行时复制指令数据。这让紧凑的标量循环明显变快。列表增长的收益较小是预期结果：持久化集合更新仍会复制路径，因此这部分成本仍是该工作负载的主导因素。

## 有意保留的成本

持久化集合服务于快照、回放和别名安全的值。如果改成原地修改，就会改变这些语义。宿主效果、序列化、进程启动和分配行为也不在 `vm/*` 微基准范围内，因此应用层性能还取决于宿主集成，而不只是 VM。

## 重现测量

先运行完整工作区检查：

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

运行不打开图表的 Criterion 套件：

```sh
cargo bench -p velin --bench pipeline -- --noplot
```

常用的聚焦过滤器：

```sh
cargo bench -p velin --bench pipeline -- check/wide_linear_script --noplot
cargo bench -p velin --bench pipeline -- machine/create_wide --noplot
cargo bench -p velin --bench pipeline -- 'vm/(counter_loop|growing_list)' --noplot
```

做 CLI 对比时，先构建 release 二进制，再反复运行同一个生成脚本。比较结果前记录机器、Rust 工具链，以及计时是否包含进程启动。
