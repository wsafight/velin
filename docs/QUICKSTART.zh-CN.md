# 快速开始

[English](QUICKSTART.md)

Velin 是一门写**规则、状态和控制流**的小型脚本语言。脚本不会打开文件、绘制界面或访问网络。它让出一条宿主命令，由你的应用决定这条命令的含义。

本页是第一篇阅读。可以先在浏览器里试跑、不必安装，再用 CLI 跑同一份文件。

## 设计原则

- **确定性**：没有浮点类型；随机数用显式种子；克隆机器即可精确回放。
- **宿主无关**：核心不认识界面、网络或业务概念。所有副作用都走 `perform`。
- **资源有界**：源码、值、字节码和立即 VM 步数都有预算。
- **一条管线**：CLI、编辑器、Rust 宿主和浏览器 Playground 共用同一套解析器、检查器、编译器和 VM。

Rust 是实现语言，也是一种嵌入方式。语言本身用来写规则。同一套语义也可以经 WebAssembly 在浏览器里运行。

## 在浏览器里试

打开 [Playground](PLAYGROUND.zh-CN.md)。示例脚本已经载入。

1. 把 `ask` 回复保持为 `[1]`，点 **运行**。
2. 输出里应出现恢复后的 HP `40`。
3. 把回复改成 `[0]` 再运行，走另一分支。
4. 改完后可点 **仅检查**，只看诊断、不执行。

源码不会离开当前标签页。细节见 [Playground](PLAYGROUND.zh-CN.md) 和 [WebAssembly](WASM.zh-CN.md)。

## 运行仓库示例

在仓库根目录，使用 Rust 1.88 或更新版本：

```sh
cargo run -p velin-cli -- check examples/counting.velin
cargo run -p velin-cli -- run examples/counting.velin
cargo run -p velin-cli -- check examples/adventure.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

- `check` 打印诊断。只有错误才会把退出码设为 1。
- `run` 使用面向行的参考宿主：`say` 打印，`ask` 从 stdin 读一个值。

`say` 和 `ask` 是宿主约定，不是关键字。游戏可以改绑 `open_door` 或 `show_menu`。见[宿主协议](HOST.zh-CN.md)。

## 第一段脚本

四个空格缩进。`#` 开始注释。`default` 是顶层的编译期常量。

```velin
default hp = 30

label start:
    perform say("You wake in a cold cell.")
    choice = perform ask("Drink a potion? (1 = yes)")
    if choice == 1:
        set hp = hp + 10
        perform say("HP is now [hp]")
    else:
        perform say("You wait.")
```

语言负责的：

| 部分 | 作用 |
| --- | --- |
| `default` / `set` | 状态 |
| `if` / `while` / `label` / `jump` | 控制流 |
| `perform` | 通向宿主的唯一出口 |
| `list`、`record`、`random`、… | 内置值，不是 I/O |

宿主负责的：打印、提问、界面、文件、时间、权限。

更多写法见[示例](EXAMPLES.zh-CN.md)。把[语法速查](CHEATSHEET.zh-CN.md)放在手边。

## 运行时发生了什么

```text
.velin source
    -> parse
    -> check（类型、确定赋值）
    -> bytecode
    -> Machine
    -> Yield::Host  <->  你的应用
    -> Finished
```

VM 在 `perform` 处挂起。宿主做完动作后调用 `resume`。没有 `perform` 的循环会在 10,000 条立即指令后停止。

## 接下来读什么

1. 写更多脚本：[语言参考](LANGUAGE.zh-CN.md)、[值与集合](VALUES.zh-CN.md)、[示例](EXAMPLES.zh-CN.md)。
2. 嵌入：[Rust](EMBEDDING.zh-CN.md) 或 [WebAssembly](WASM.zh-CN.md)，然后是[宿主协议](HOST.zh-CN.md)。
3. 工具：[CLI、编辑器与 Web](TOOLING.zh-CN.md)。
4. 边界：[常见问题](FAQ.zh-CN.md)、[架构](ARCHITECTURE.zh-CN.md)、[资源预算](LIMITS.zh-CN.md)。
