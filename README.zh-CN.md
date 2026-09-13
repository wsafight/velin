# Velin

[English](README.md)

Velin 是一门**可嵌入、可复现的字节码脚本语言**。它负责表达式、状态与控制流，把 I/O 和领域行为作为不透明的宿主命令交还给嵌入方处理。

名字来自法语 *velin*（精制羊皮纸）：脚本写下规则，宿主赋予它现实效果。

## 设计原则

- **确定性**：没有浮点类型；随机数由显式种子驱动，克隆机器状态即可回放。
- **宿主无关**：核心不认识界面、网络、音频或任何业务概念，所有副作用统一经 `Yield::Host` 让出。
- **安全 Rust**：核心 workspace 禁止 `unsafe`，不使用 JIT 或原生机器码。
- **资源有界**：源码、表达式、字节码、值、立即执行步数和宿主输出均有预算限制。
- **工具完整**：提供 CLI、LSP、VS Code 扩展、WebAssembly 绑定和浏览器 Playground。

## 文档

- [快速开始](docs/QUICKSTART.zh-CN.md)介绍第一段脚本、Playground 和 CLI。
- [常见问题](docs/FAQ.zh-CN.md)说明何时使用 Velin、检查与随机数如何工作，以及信任边界在哪里。
- [Playground](docs/PLAYGROUND.zh-CN.md)说明浏览器编辑器、回复和脚本化宿主。
- [语言参考](docs/LANGUAGE.zh-CN.md)介绍值、语法、控制流、宿主效果和内置函数。
- [语法速查](docs/CHEATSHEET.zh-CN.md)是一页查阅表。
- [值与集合](docs/VALUES.zh-CN.md)介绍整数、字符串、列表和记录。
- [示例与写法](docs/EXAMPLES.zh-CN.md)走读仓库示例并提供可复制的写法。
- [嵌入 Rust](docs/EMBEDDING.zh-CN.md)介绍编译、宿主分派、快照与信任边界。
- [宿主协议](docs/HOST.zh-CN.md)描述让出/恢复、绑定命令和宿主侧校验。
- [WebAssembly](docs/WASM.zh-CN.md)说明如何从 JavaScript 调用 `check` / `run`。
- [工具链](docs/TOOLING.zh-CN.md)介绍 CLI、LSP、VS Code 扩展、Playground 与 CI。
- [架构](docs/ARCHITECTURE.zh-CN.md)解释 crate 边界、字节码校验、静态分析和资源预算。
- [静态检查](docs/CHECKING.zh-CN.md)说明类型、`Unknown` 和确定赋值。
- [资源预算](docs/LIMITS.zh-CN.md)列出全部已公布的限制。
- [更新记录](CHANGELOG.md)记录发布级别的变更。
- [安全策略](SECURITY.md)说明支持版本与私密报告方式。

## Workspace

| crate | 职责 |
| --- | --- |
| `velin-syntax` | 值、表达式、内置函数、源码位置和诊断 |
| `velin-parse` | 表达式词法分析与 Pratt 解析 |
| `velin-eval` | 参考表达式求值器与确定性内置函数 |
| `velin-bytecode` | 共享字节码模型、校验、线格式与执行计划 |
| `velin-compile` | 表达式及控制流到字节码的编译 |
| `velin-vm` | 字节码执行、状态帧和宿主效果让出 |
| `velin-check` | 保守类型推断与确定赋值分析 |
| `velin-lang` | 缩进敏感的语句语言前端 |
| `velin` | 面向嵌入方的统一门面 crate |
| `velin-cli` | `velin check` 与 `velin run` |
| `velin-lsp` | 诊断、补全和文档符号 |
| `velin-wasm` | 浏览器 Playground 的 WebAssembly 接口 |

执行管线为：

```text
.velin source
    -> parse statements
    -> lower expressions and control flow
    -> static checks
    -> bytecode Program
    -> Machine
    -> Yield::Host <-> host
    -> Finished
```

## 快速上手

一段完整脚本：

```text
default balance = 30

label start:
    perform log("balance: [balance]")
    amount = perform read_amount("Adjustment")
    if amount > 0:
        set balance = balance + amount
    else:
        set balance = balance - 1
    jump start
```

语法使用 4 空格缩进和 `#` 行注释。`label`、`default`、`set`、`perform`、`if`、`elif`、`else`、`while`、`jump` 是语句关键字。`default` 只能出现在顶层，同名默认值只能声明一次。`log` 与 `read_amount` 只是示例宿主命令，并非语言内置行为。

运行随仓示例：

```sh
cargo run -p velin-cli -- check examples/adventure.velin
cargo run -p velin-cli -- run examples/counting.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

- `velin check [--json] <file|->`：编译源码并输出文本或结构化诊断；仅 error 导致退出码 1。
- `velin run <file|->`：检查通过后由行式参考宿主执行；`say` 输出文本，`ask` 从 stdin 读取一个值。

## 嵌入 Rust

门面 crate 重导出完整管线，常规宿主只需依赖 `velin`：

```rust
use velin::{compile, Machine, Value, Yield};

let script = compile(
    "counter.velin",
    "default count = 1\n\
     perform emit(\"count: [count]\")\n\
     set count = count + 1\n",
)
.unwrap();

let mut machine = Machine::new(script.program.clone()).unwrap();
for (name, value) in &script.defaults {
    machine.set_variable(name, value.clone());
}

match machine.run().unwrap() {
    Yield::Host { host_id, values } => {
        assert_eq!(script.host_name(host_id), Some("emit"));
        assert_eq!(values, vec![Value::String("count: 1".into())]);
        machine.resume(None).unwrap();
    }
    Yield::Finished => {}
}
```

宿主命令只有一个协议：VM 返回不透明的 `host_id` 与已求值参数；宿主完成行为后调用 `resume`，需要返回值的命令则传入 `Some(Value)`。

`CompiledScript` 以 `Arc<Program>` 共享不可变字节码，克隆脚本或机器快照不会复制整份程序。`Machine::new` 和 `Machine::with_seed` 会先验证所有 chunk、槽位、跳转、栈路径和字节码预算，因此返回 `Result`。通过 Serde 读取 `Program` 时也会执行同一校验，坏数据不会进入执行阶段。

也可以绕过语句前端，直接使用 `Expr`、`ProgramBuilder` 和 `Machine` 构建更小的语言子集。

## 值与内置函数

值类型：`Integer(i64)`、`Boolean`、`String`、`List`、`Record`。集合使用结构共享与写时复制，且受确定性数据预算约束。

内置函数：`list`、`record`、`get`、`put`、`push`、`remove`、`len`、`contains`、`random(lo, hi)`、`chance(percent)`。

`random` 和 `chance` 只使用 VM 帧内的 RNG 状态。可通过 `Machine::with_seed` 或 `set_rng_seed` 设定种子；克隆并恢复 `Machine` 会同时回滚随机序列。

## 静态检查

`velin-check` 提供两类保守分析：

- 类型推断覆盖 `Integer`、`Boolean`、`String`、`List`、`Record` 与 `Unknown`；赋值类型沿 CFG 传播，冲突分支、循环和宿主返回值会保守合流为 `Unknown`，不产生猜测式误报。
- 确定赋值分析通过 CFG must-analysis 报告可能在赋值前读取的变量。

`check_script(file, &script)` 会一次运行两类分析，并跳过不可达代码。解析器为表达式节点保留精确 `Span`，因此诊断能指向变量、运算符或调用所在的真实行列。

## 资源边界

- 每份源码最多 1 MiB、10,000 个物理行，语句块最多嵌套 64 层。
- 单个表达式最多 64 KiB、512 个 token、32 层括号和 32 层插值；嵌套插值共享累计工作量与 token 预算。
- 单个值最多包含 4,096 个节点、16 层集合和 1 MiB 文本；`Program` 另有操作数、chunk、槽位、常量、文本、栈高和宿主参数预算。
- VM 每次让出宿主前最多立即执行 10,000 步；CLI 与 Playground 每次运行最多处理 1,000 次宿主效果和 1 MiB 输出；Playground 还会把回复 JSON 限制为 1 MiB，并把 Worker 执行限制为 5 秒。
- LSP 限制单条 JSON-RPC 消息为 4 MiB、整个头部为 64 KiB、单行头部为 8 KiB。

## 浏览器 Playground

```sh
wasm-pack build --target web --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

打开 `http://localhost:8080`。脚本在浏览器内完成解析、检查与执行，不发送到后端。

## 文档站

`site/` 是独立的 Astro 静态站，构建时从本 README 与 `docs/ARCHITECTURE.md` 生成文档页，并把 Wasm Playground 发布到同一站点：

```sh
cd site
npm ci
npm run check
npm run build
npm test
```

GitHub Actions 会检查 Rust workspace 与站点，并在默认分支更新后部署到 GitHub Pages。项目仓库使用子路径时，构建会根据 `GITHUB_REPOSITORY` 自动设置站点 base path；也可以通过 `DOCS_SITE` 和 `DOCS_BASE` 覆盖。

## 验证

```sh
cargo test --workspace
cargo clippy --all-targets --workspace -- -D warnings
cargo bench -p velin
cargo build --workspace --target wasm32-unknown-unknown
```

表达式 VM 与参考求值器由差分测试保持一致；核心 crate、语句前端和 WebAssembly 绑定均在同一 workspace 内验证。
