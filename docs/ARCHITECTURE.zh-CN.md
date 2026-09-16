# Velin 架构

[English](ARCHITECTURE.md)

Velin 是一门可嵌入的确定性脚本语言。它提供从源码到执行的完整管线，但不拥有任何具体领域行为：脚本产生宿主效果，嵌入方解释效果并恢复虚拟机。

本文只描述 Velin 自身的设计、边界与稳定性约束。

## 1. 目标

Velin 的核心目标是让不同类型的宿主共享同一套表达式、状态和控制流语义：

- 在桌面、服务端和 WebAssembly 上得到可复现的结果。
- 通过单一宿主协议接入 UI、网络、存储或业务动作。
- 在执行前提供结构化语法与静态检查诊断。
- 保持实现可测试、可序列化，并限制不受信任脚本的资源消耗。

明确不做：

- 不提供操作系统、文件、网络或时钟 API。
- 不包含任何应用领域的命令。
- 不引入浮点值、系统随机源、JIT 或原生代码生成。
- 不保证脚本源码的类型完备性；静态检查刻意保持保守。

## 2. 总体管线

```text
source (.velin)
  |
  v
velin-lang          statements + embedded expressions
  |
  +----> velin-check          diagnostics
  |
  v
velin-compile       源码降级与 ProgramBuilder
  |
  v
velin-bytecode      Program + 校验 + 执行元数据
  |
  v
velin-vm            Machine state
  |
  +----> Yield::Host { host_id, values }
  |                    |
  |                 host performs effect
  |                    |
  +<--- Machine::resume(optional value)
  |
  v
Yield::Finished
```

表达式还有一条参考路径：`velin-parse -> velin-eval`。它与字节码路径共享值和内置函数语义，并通过差分测试逐值、逐错误对拍。

## 3. Crate 边界

| crate | 所有权边界 |
| --- | --- |
| `velin-syntax` | 公共数据模型：`Value`、`Expr`、运算符、`Span`、`Diagnostic` |
| `velin-parse` | 表达式源码到 AST，不处理语句控制流 |
| `velin-eval` | 树遍历参考求值器和内置函数语义 |
| `velin-bytecode` | 寄存器/程序字节码模型、槽位表、校验、wire 格式与执行元数据 |
| `velin-compile` | 源码表达式降级、常量传播与 `ProgramBuilder` |
| `velin-vm` | 运行状态、寄存器表达式解释器、控制流循环、宿主挂起协议 |
| `velin-check` | 类型推断、条件检查与确定赋值分析 |
| `velin-lang` | 缩进敏感语句 AST、解析、降级、宿主名驻留 |
| `velin` | 重导出稳定的嵌入 API，不实现新语义 |
| `velin-cli` | 行式参考宿主和命令行体验 |
| `velin-lsp` | 编辑器协议适配，不复制解析或检查逻辑 |
| `velin-wasm` | 字符串/JSON 编组和浏览器参考宿主 |

低层 crate 不反向依赖高层工具。领域行为只能存在于宿主，不得进入 syntax、compile、eval 或 vm。

## 4. 源语言

### 4.1 表达式

表达式由 Pratt parser 解析，支持：

- `i64` 整数、布尔、字符串、列表和记录值。
- 一元 `-`、`not`。
- 算术、比较、相等以及短路 `and` / `or`。
- 内置函数调用。
- 字符串插值 `"balance: [balance]"`；`[[` 表示字面量 `[`。

表达式深度、token 数和数据尺寸均受预算限制。解析器为每个节点附加 `Expr::Spanned`，记录精确的 `Span`；错误使用 1-based 行列位置，通过 `Diagnostic` 返回。手工构造 AST 时可以省略 span，检查器会退回调用方提供的位置。

### 4.2 语句

语句前端使用 4 空格缩进，支持：

```text
default name = constant-expression
set name = expression
name = expression
perform effect(arguments)
name = perform effect(arguments)
if condition:
elif condition:
else:
while condition:
label name:
jump name
```

`default` 只能位于顶层，同名变量只能声明一次；其值在编译期求值，不能引用运行时变量。宿主命令名不是保留字：编译器将名称稳定驻留为 `u32 host_id`，并在 `CompiledScript` 中保留 ID 到名称的查找表。

## 5. 值与确定性

`Value` 只包含：

```text
Integer(i64)
Boolean
String
List
Record
```

没有浮点值，因此数值计算不受平台浮点实现或舍入模式影响。记录使用稳定顺序的数据结构，显示与序列化顺序可复现。

字符串、列表和记录使用 `Arc` 共享；集合和字符串更新采用写时复制。槽位唯一持有值时可以复用原分配，存在别名或 `Machine::clone` 检查点时仍能观察旧值。所有进入运行状态的值都经过数据预算校验，避免宿主绕过源码限制注入无限增长的数据。

### 5.1 随机数

`random(lo, hi)` 与 `chance(percent)` 使用纯整数 PRNG。随机状态保存在机器帧的保留槽位中，而不是线程局部或全局状态中。

由此得到三个性质：

1. 相同程序、输入与种子产生相同结果。
2. `Machine::clone` 同时保存随机进度。
3. 恢复旧机器状态后，后续随机序列也随之回滚。

非法随机参数在推进状态前返回错误。

## 6. 字节码

### 6.1 表达式字节码

每个 `ExprChunk` 包含常量池、源码行号、寄存器数量、结果寄存器与扁平 `Vec<ExprOp>`。每条产生值的指令都写明目标寄存器，运算直接写明源寄存器。主要操作包括：

```text
Const / Load
Unary / Binary
Call
Random / Chance
JumpIfFalse / JumpIfTrue
Concat
```

变量在编译期解析成 `u32` 槽位，运行时不做字符串查表。`and` 和 `or` 使用显式条件寄存器编译为条件跳转；built-in、随机操作与插值使用连续寄存器范围，插值最后执行一次 `Concat`。

纯常量子树在编译期交给参考求值器计算。溢出、除零和类型错误等失败候选仍保留为字节码，因此运行时错误行为不变。字节码源码位置使用紧凑的 `u32`，完成后的 op 和常量向量会收缩到实际长度。

### 6.2 控制流字节码

`Program` 的控制流操作保持精简：

```text
Set { slot, value }
Update { slot, operation, line, column }
Jump(pc)
JumpIfFalse { condition, target }
Host(HostOp { host_id, args, bind, line })
Halt
```

控制流操作只引用表达式 chunk、槽位和程序计数器。`items = push(items, value)`、`count = count + 1` 等所有权感知形式会生成 `Update`；VM 在取走目标值前预检类型、下标、单值预算和整机预算，失败时槽位保持不变。冷路径、可变长度的 `HostOp` 载荷被装箱，不再抬高每条热指令的尺寸。在 64 位目标上，`Op` 为 32 字节，`ExprOp` 为 12 字节，`ProgramChunk` 为 24 字节。`Program` 同时保存 `SlotTable`，用于初始化、调试和通过公共 API 按变量名读写状态。

### 6.3 校验边界

`Program::validate` 检查控制流目标、chunk 和槽位索引、寄存器及范围边界、表达式前向跳转、所有可达路径上的寄存器定义、结果寄存器、内置函数参数数量以及字节码和常量预算。`ExprChunk::validate(slot_count)` 为独立使用表达式 VM 的调用方提供相同的局部保证。

`Program` 的 Serde 反序列化会自动运行校验，并拒绝重复槽位名。`Machine::new` 与 `Machine::with_seed` 仍会在构造时验证手工组装的程序并返回 `Result`。公开的 `eval_chunk` 会先验证独立 chunk；`Machine` 持有 `Arc<Program>`，构造成功后走内部已校验路径，不在每次表达式执行时重复扫描。

## 7. 宿主协议

宿主效果是 Velin 与外部世界的唯一边界。

VM 遇到 `Op::Host` 时：

1. 对所有参数表达式求值。
2. 将程序计数器推进到下一条指令。
3. 保存可选返回值目标槽位。
4. 返回 `Yield::Host { host_id, values }`。

宿主处理完成后调用 `Machine::resume(value)`：

- 无返回值效果使用 `None`。
- 带 `bind` 的效果必须返回 `Some(Value)`。
- 返回值通过数据预算校验后写入目标槽位。

机器等待宿主期间再次调用 `run`，或没有挂起效果时调用 `resume`，都会得到明确错误。效果发生前先推进 PC，确保恢复时不会重复执行外部行为。

宿主应根据自己的信任边界验证命令、参数和权限。Velin 保证语言核心不主动访问外部资源，但不能替宿主管理外部行为的授权。

## 8. 静态检查

类型域为：

```text
Integer / Boolean / String / List / Record / Unknown
```

类型推断只在能证明错误时报告诊断；`Unknown` 与任何上下文兼容，避免阻止运行时合法程序。默认值建立入口类型，赋值沿控制流图传播；只有所有流入路径都具有相同具体类型时，合流点才保留该类型。冲突分支、循环回边或宿主绑定会保守地得到 `Unknown`。不可达表达式不参与检查，条件表达式另行检查已知的非布尔值。

确定赋值分析在控制流图上执行 must-analysis：只有所有可达前驱都已赋值的变量，才在合流点被视为已赋值。`default` 声明作为入口状态参与分析。

静态检查不会改变字节码或运行时行为。CLI 和宿主可以决定 warning 的展示策略，但 error 应阻止执行。

## 9. 执行与资源限制

`Machine` 通过 `Arc` 持有不可变 `Program`，并拥有独立的变量帧、程序计数器、挂起效果、完成标记和可复用寄存器值/指标数组。构造函数是可失败的，只有验证后的程序才能进入执行状态。帧槽位保留已经测得的数据占用量，表达式和 built-in 求值同时返回值与指标，赋值及宿主载荷统计不再进行第二次递归扫描。

`ExecutionPolicy` 由 `Machine`、`ScriptRunner`、`MachineInvoker` 和 `PureModule` 共享。默认策略允许累计 10,000,000 fuel、每次 `run` / `resume` / batch 10,000 个立即 fuel 单位、1,000 次宿主效果、64 层调用深度，以及已公布的单值、整机、宿主载荷和宿主队列数据预算。进度回调可以报告 fuel 并请求协作式取消；墙钟截止时间仍属于宿主策略。

每次执行调用开始时都会重置立即 fuel；累计 fuel 和已让出的宿主效果数量会跨 `run` / `resume` 保留。`Machine::clone` 会复制策略、累计计数、挂起的宿主状态和取消标记，因此快照保留完整的预算状态。`restart` 开启新的预算生命周期，并清零 fuel、宿主效果和取消状态。

即使没有宿主效果，每次执行调用也有上限。达到立即 fuel 上限会返回可能存在无限循环的 fuel 错误，使没有宿主让出点的脚本不能永久占用调用线程。

当前内建边界如下：

| 层 | 边界 |
| --- | --- |
| 源码 | 1 MiB、10,000 个物理行、64 层语句嵌套 |
| 表达式 | 单次 64 KiB / 512 token / 32 层括号；插值最多 32 层并共享 256 KiB 工作量与 2,048 token 预算 |
| 值 | 每棵值树 4,096 个节点、16 层集合、1 MiB 文本 |
| 程序字节码 | 100,000 个控制流 op、100,000 个 chunk、65,536 个槽位、100,000 个常量值节点、16 MiB 常量及槽位文本 |
| 表达式字节码 | 每个 chunk 4,096 个 op、1,024 个寄存器；每条宿主指令最多 128 个参数 |
| 工具层 | CLI / Playground 输出 1 MiB、宿主效果 1,000 次；Playground 回复 JSON 1 MiB、Worker 请求 5 秒；LSP JSON 正文 4 MiB、头部 64 KiB、单行头部 8 KiB |

CLI、WebAssembly 或其他宿主仍应按自己的风险模型增加时间、效果权限和外部资源预算；这些限制属于宿主层，不能由语言核心统一决定。

## 10. 工具层

### CLI

`velin check` 对文件或 stdin 源码运行编译与静态检查，并可输出 JSON 诊断；`velin run` 使用有界、仅供示例的行式宿主。参考宿主对 `say` 和 `ask` 提供行为，其他效果按名称和参数回显。

### LSP 与 VS Code

`velin-lsp` 使用 stdio JSON-RPC，提供可恢复的多错误诊断、补全、标签文档符号、悬停说明，以及标签定义/引用导航。它直接调用 `velin-lang` 和 `velin-check`，不维护第二套语言语义；未知请求返回标准 `MethodNotFound`。

VS Code 扩展负责 `.velin` 文件注册、TextMate 高亮和启动 LSP。各平台发布的 VSIX 内置对应原生服务器，也允许用显式配置覆盖。

### WebAssembly

`velin-wasm` 暴露 `check(source)` 和 `run(source, replies_json)`。绑定层只处理有界字符串与严格校验的 JSON；解析、检查和执行仍由门面 crate 完成。Playground 在可终止 Worker 中运行它，保证 UI 可响应。输出按剩余预算增量渲染，不先分配一份完整的中间字符串。

`#[wasm_bindgen]` 会生成 `unsafe` 胶水，因此 wasm shim 无法继承 workspace 的 `unsafe_code = "forbid"`。所有手写 wasm 代码保持安全 Rust，核心 crate 继续执行 forbid 门禁。

## 11. 稳定性规则

Velin 仍处于 1.0 之前，但源码、Rust 门面、artifact 和 C ABI 已在[兼容性政策](COMPATIBILITY.zh-CN.md)中分别声明契约。以下架构规则约束所有边界：

- `velin-eval` 是值语义参考实现；VM 变更必须通过差分测试。
- 新的外部行为必须建模为宿主效果，不能直接加入 VM I/O。
- 新值必须定义确定性相等、显示、序列化与预算成本。
- 新控制流必须加入确定赋值分析和立即 fuel 统计。
- 新随机能力必须显式推进帧内状态，不能读取系统熵。
- 诊断必须携带真实文件名和源码位置。
- 面向 WebAssembly 的核心 crate 不应依赖系统 API。

## 12. 验证门禁

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo build --workspace --target wasm32-unknown-unknown
```

性能基准只用于发现相对回归，不作为发布前的语言兼容承诺。
