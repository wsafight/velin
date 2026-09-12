# VM P0-P2 开发方案

> 临时开发文档。用于 P1、P2 实施期间统一设计与验收口径；阶段完成后删除，不进入站点导航。

## 目标与范围

Velin 当前是解释执行字节码的栈 VM：源码先经过解析、lowering 和静态检查，形成 `Program`；`Machine` 再执行控制流指令 `Op` 与表达式栈指令 `ExprOp`。

这份方案中的 AOT 指“提前生成可复用的 Velin 字节码制品”，不生成本地机器码，也不引入 LLVM、Cranelift 或 JIT。`Program` 继续是唯一规范字节码，验证器、静态检查器和 VM 不维护第二套语言语义。

总体目标不是单纯追求最高吞吐，而是在以下约束内减少重复工作：

- 算术、比较和 built-in 语义继续集中在 `velin-eval`。
- 栈字节码继续作为可序列化、可检查、可回退的规范表示。
- 所有外部字节码必须经过结构与资源预算验证。
- 专用执行路径必须局部、可删除，并保留通用路径。
- 失败操作不能部分修改变量、随机数状态或宿主暂停状态。
- 只有聚焦基准改善且完整基准没有实质回退的改动才保留。

## 阶段状态

| 阶段 | 状态 | 主要交付 |
| --- | --- | --- |
| P0 | 已完成 | 基准护栏、稳定 footprint 快路径、否决有全局回退的专用指令 |
| P1 | 已完成 | 版本化字节码制品、可复用 restart；槽位比较候选未满足保留门槛 |
| P2 | 暂停，尚未实现 | 指标随值传递、运行时表达式执行计划、字符串专项、REPL 会话 |

## P0：先建立可相信的性能边界

P0 不扩展语言能力，重点是把热点、回退和保留条件测清楚。

### 基准覆盖

基准按成本边界分组：

- `check/*`：控制流图、确定赋值和类型传播。
- `compile/*`：lowering、表达式编译和字节码布局。
- `machine/*`：验证证明、执行元数据和初始帧构造。
- `vm/*`：已经构造好的机器执行字节码。

VM 侧除已有计数、built-in、集合、字符串和宽脚本场景外，增加两个针对性场景：

- `vm/scalar_reassignment`：反复用同类标量替换槽位，覆盖 footprint 不变的赋值。
- `vm/boolean_slot_loop`：循环中直接读取布尔槽位，约束后续条件跳转特化不能只改善单一场景。

Criterion 的 `change` 百分比只相对它最近保存的一次测量。多个实验连续运行时，决策使用绝对耗时并进行消融测试，不能把不同实验的相对百分比直接串联。

### 相同 footprint 的槽位替换

帧为每个槽位保存 `Value`、`DataFootprint` 和深度，同时保存整台机器的 footprint 总量。若新旧值的 footprint 完全相同，替换不会改变机器总量，也不会触及资源上限，因此只需要：

1. 写入新值；
2. 更新新值的深度缓存；
3. 保持聚合 footprint 不变。

这条路径位于统一的 `replace_slot` 边界，不改变调用方，也不复制资源预算语义。footprint 不同时仍执行原来的减旧值、加新值、检查上限流程。

### 不保留的实验

`JumpIfBooleanSlot` 能减少目标布尔场景的表达式分派，但给热 `Op` 枚举增加变体后，计数循环产生明显回退。控制流指令的枚举布局、匹配分派和生成代码是全局成本，因此不能以一个场景的收益换取已有主路径退化。

整数槽位自增的“直接修改内部 `i64`”也不保留。它表面上省去了指标更新，但实测使计数循环稳定回退；原实现的指标更新对整数是常量成本，并且与其他槽位更新保持统一。局部代码行数更少不等于生成代码更快。

P0 最终提交为 `ce29da5`。该提交只保留相同 footprint 快路径和对应基准护栏。

## P1：消除重复编译和重复初始化

P1 的收益来自复用已经完成的工作，不改变 VM 指令模型。

### 版本化字节码制品

CLI 增加以下流程：

```text
velin compile story.velin -o story.velinc
velin run story.velinc
```

`compile` 仍执行完整的源码解析、lowering 和静态检查；存在错误时不写制品。`run` 根据文件头识别 `.velinc`，载入后直接进入字节码验证与运行，不再解析源码或重复静态检查。

#### 规范数据

V1 payload 只保存重新运行脚本必需且跨进程稳定的数据：

```text
BytecodeArtifactV1
  source_name
  Program
  hosts: host_id -> name
  labels: name -> Pc
  defaults: name -> Value
```

以下内容不得序列化：

- `ValidatedProgram` 的内存证明；
- `ExecutionMetadata`；
- quickened built-in 操作数；
- 初始帧的派生指标缓存；
- CFG、类型传播状态、工作队列和其他静态分析临时数据；
- P2 可能增加的运行时表达式执行计划。

这些数据与当前进程、实现版本或可重新推导的信息绑定。载入后重建它们可以保持制品格式稳定，也避免反序列化时信任伪造缓存。

#### 文件边界

制品由固定头和带长度的 payload 组成：

```text
magic | format_version | payload_length | payload
```

- `magic` 用于在解析 payload 前区分源码与字节码。
- `format_version` 从 1 开始；未知版本直接拒绝，不尝试猜测兼容性。
- `payload_length` 使用固定字节序，并且必须与文件实际剩余长度一致。
- 读取 payload 前检查 `MAX_ARTIFACT_BYTES`，禁止由文件声明触发无界分配。
- V1 使用 Serde 数据模型；具体编码一旦发布便属于该版本协议，后续改变编码必须提升格式版本。
- 写文件采用“同目录临时文件、完整 flush、原子 rename”，编译或 I/O 失败时不留下半个目标文件。

#### 载入验证

载入顺序固定如下：

```text
检查头和文件大小
  -> 解码 V1 payload
  -> 验证 hosts / labels / defaults
  -> 验证 Program 并建立 ValidatedProgram
  -> 重建 InitialFrame
  -> 按需建立 ExecutionMetadata
```

制品元数据也属于不可信输入：

- host 数量、名称总字节数和 host id 必须有界；程序引用的每个 host id 必须存在。
- label 名称和总字节数必须有界，每个 `Pc` 必须是合法程序位置。
- default 名称必须对应程序槽位，值树及聚合机器状态必须满足现有预算。
- `source_name` 长度必须有界，只用于诊断，不参与文件路径访问。

当前 `Program::deserialize` 内部已经执行验证；随后再调用 `ValidatedProgram::new` 会重复扫描。P1 应在 `velin-compile` 内提供 `ValidatedProgram` 的受控反序列化：共享私有 wire 结构，构造 `Program` 后只调用一次验证，再生成证明。不能公开 `assume_valid` 一类绕过边界的 API。

源码运行路径继续存在。`.velin` 与标准输入仍按当前方式编译、检查和执行；`.velinc` 不从标准输入自动猜测，避免二进制数据与源码错误处理混在一起。

### Machine 与 ScriptRunner restart

重复运行同一脚本时，不必反复分配帧数组和表达式栈。增加：

```rust
Machine::restart(&InitialFrame, seed)
ScriptRunner::restart(seed)
```

restart 复用 `values`、`footprints`、`depths` 和表达式栈已有容量，但恢复出与新建 runner 等价的逻辑状态：

- 从 `InitialFrame` 恢复默认值与指标；
- 用调用方 seed 重置 RNG 槽位；
- `pc = 0`；
- 清除 pending host 与 finished；
- 清空表达式栈但保留容量；
- `ScriptRunner` 的 host effect 计数归零；
- 清除 runner 的 pending host 和 sticky failure。

restart 必须先验证帧宽度与 seed 后的聚合预算，再修改机器，保证失败时旧机器仍可观察为原状态。若公开 defaults 已被调用方修改，runner 应重新准备合法初始帧或返回初始化错误，不能继续使用过期缓存。

测试要比较“restart 后的旧 runner”和“同参数新建 runner”：

- 正常结束后的变量结果；
- 相同 seed 的随机序列与不同 seed 的分离；
- 运行时错误的位置和内容；
- host yield、reply、再次 yield 与结束；
- 在 pending host、finished 和 host budget error 状态下 restart；
- defaults 被替换后的缓存失效。

### 槽位对槽位比较仅作为候选

`slot <op> integer literal` 已有直接条件指令。`left_slot <op> right_slot` 可以候选为 `JumpIfIntegerSlots`，但不属于 P1 必须交付项。

只有同时满足以下条件才加入：

- 表达式循环基准证明分派是主要成本；
- `Op` 大小不增加；
- counter、boolean、built-in、宽脚本完整基准无实质回退；
- 比较仍调用 `velin-eval` 的共享语义或共享 helper；
- 未赋值、类型不匹配和源码位置与通用路径完全一致。

未通过门槛时只记录实验结果并删除实现，不为了阶段清单保留指令。

### P1 完成条件

- `.velin -> .velinc -> run` 与直接源码运行得到相同 host 事件和最终状态。
- 截断、超长、损坏、未知版本和伪造索引制品均在执行前被拒绝。
- 反序列化到 `ValidatedProgram` 只做一次完整字节码验证。
- restart 与新建 runner 通过语义等价测试，并由基准证明减少重复运行成本。
- 源码 API、普通 `Program` API 和通用 VM 路径继续可用。
- 工作区 test、Clippy、fmt、差分测试和完整 Criterion 套件通过。
- P1 单独提交，不混入 P2 实验。

P1 已在 `ac02113` 之后的独立提交中完成。槽位对槽位比较没有作为必要优化实现，继续使用现有的字面量比较特化和通用表达式路径。

## P2：在规范栈字节码之上准备运行时快路径

P2 的每一项都由基准驱动。没有稳定收益时允许整项不落地。

### 指标随中间值传递

普通 built-in 产生集合或深层值后，部分路径仍可能对结果调用 `data_metrics()`，重新遍历刚刚构造的值树。P2 可以在 VM 内部使用：

```rust
MeasuredValue {
    value: Value,
    metrics: Option<DataMetrics>,
}
```

也可以使用与操作数栈平行的指标栈。选择标准是让 push/pop、短路和错误清理保持简单，不把指标状态散落到每条 `ExprOp`。

规则如下：

- built-in 的指标推导仍由 `velin-eval` 提供，VM 不复制集合深度和资源预算公式。
- 已有指标随值移动；无法可靠推导时使用 `None`，最终回退到 `data_metrics()`。
- 常量、槽位 load 和 copy 直接继承已有指标。
- 错误路径必须同时清理值与指标，不能出现两个栈错位。
- 重点基准是深层集合、非自更新集合构造和复合 built-in；标量路径不能因包装类型明显变慢。

### 非序列化的表达式执行计划

`ExprOp` 继续是唯一字节码。验证完成后，可以在 `ExecutionMetadata` 中为满足条件的 chunk 构造 `PreparedExpr`，把已验证的栈操作解析为更直接的运行时操作数引用。

它不是第二套公开寄存器字节码：

- 不写入 `.velinc`；
- 不参与编译器输出和静态检查；
- 不改变 `ExprOp` 格式；
- 每个 chunk 保留原始栈解释回退；
- 运算和 built-in 继续调用共享 evaluator 语义。

第一轮只处理直线、无短路跳转、操作数来源明确的表达式。复杂控制流、随机状态修改、host 边界或无法证明收益的 chunk 继续走栈 VM。可以按指令数设置阈值，避免为很短的表达式增加准备时间和元数据体积。

必须增加 prepared 与 stack fallback 的差分测试，覆盖正常值、整数溢出、除零、类型错误、未赋值、随机数和资源上限。只有表达式密集基准改善且机器创建成本、内存与普通脚本无明显回退时才保留。

### 字符串专项

字符串只按独立瓶颈逐项处理：

- `Concat` 若包含固定字面量，可在执行准备时累计已知 UTF-8 字节数，用于一次 reserve 和提前预算检查。
- 插值可以复用已验证 hole 的显示长度或指标，但最终显示格式仍由 `Value` 的共享实现决定。
- Unicode 字符数缓存只在“同一长字符串被反复 `len`”基准证明重要时考虑。

字符数缓存会影响 `SharedString::make_mut`、克隆、反序列化和所有字符串修改路径的失效规则，因此它是最后选项。若缓存引入公开布局变化或复杂失效协议，宁可保留线性字符计数。

### REPL 使用独立编译片段

REPL 不向正在执行或已经验证的 `Program` 末尾追加指令。每次输入形成独立片段：

```text
输入片段
  -> 使用 session 编译上下文解析、检查、lowering
  -> 创建临时 Machine
  -> 注入 session 值与 RNG 状态
  -> 执行并处理 host yield
  -> 成功结束后提交新的 session 状态
```

session 至少保存：

- 按名称保存的值；
- 已知静态类型；
- RNG 状态；
- 用于展示和重放的输入历史。

编译上下文把已有 session 变量视为入口已赋值，并向新片段的 `SlotTable` 预置所需名称。片段可以读取和更新旧变量，也可以创建新变量。成功结束后按名称提交值与类型；解析、检查、lowering 或运行失败时不修改 session。

host effect 已经离开 VM，无法随 session 回滚。REPL 必须明确采用“VM 状态在片段成功时提交，外部 host effect 一旦执行不可撤销”的模型，不能暗示完整事务性。

label 和跳转只在当前片段内解析，不跨历史片段保留 PC。这样每个 `Program` 仍可独立验证，也避免修改旧程序后让 PC、CFG 和诊断映射失效。

### P2 完成条件

- 每个保留的性能机制都有独立基准、通用回退和差分测试。
- `.velinc` 中不包含任何 P2 运行时缓存，跨进程载入仍从规范字节码重建。
- `ExprOp` 与 `Op` 的公开语义和序列化兼容性不因 prepared plan 改变。
- REPL 连续片段可读写已有变量，错误片段不污染 session，随机数行为可复现。
- 完整 benchmark suite 不出现无法解释的主路径回退。
- 工作区 test、Clippy、fmt 和文档检查通过。
- P2 独立提交；被基准否决的实验不进入提交。

## 实施顺序与提交边界

恢复开发后严格按以下顺序进行：

1. 完成 P1 字节码制品及其损坏输入测试。
2. 完成 P1 restart 及新建等价测试和基准。
3. 评估槽位比较候选，通过全套门槛才保留。
4. 运行完整验证，提交 P1。
5. 在 P2 中先做指标传递，再评估 prepared expression plan。
6. 独立评估字符串项，每项都可因收益不足而删除。
7. 最后实现 REPL session，因为它依赖稳定的编译入口上下文和可复用运行接口。
8. 运行完整验证，提交 P2。

每个阶段的提交只包含通过验收的代码、测试、基准和正式文档。实验代码、Criterion 历史数据、生成目录以及这份临时文档不应进入最终发布提交。
