# 宿主协议

[English](HOST.md)

Velin 从不自行做 I/O。与外部世界的唯一接触是 `perform`，它会变成 `Yield::Host`。本页说明这份协议：命令如何挂起、宿主必须返回什么，以及信任边界在哪里。

[嵌入指南](EMBEDDING.zh-CN.md)展示 Rust 类型。本页关注宿主必须遵守的约定。

## 让出与恢复

编译期会把每个不同的命令名驻留成 `u32` 的 `host_id`。运行时 VM 会：

1. 求值全部参数表达式。
2. 把程序计数器推进到下一条指令。
3. 记住该命令是否绑定到变量。
4. 返回 `Yield::Host { host_id, values }`。

宿主用 `CompiledScript::host_name` 查出名字，在应用代码里执行动作，再调用 `Machine::resume`。

```text
script:  answer = perform ask("Continue?")
                |
                v
VM:      Yield::Host { host_id, values: ["Continue?"] }
                |
                v
host:    显示提示、读取输入、得到一个 Value
                |
                v
VM:      resume(Some(value)) -> 写入 `answer` -> 继续
```

程序计数器在效果发生**之前**前进。恢复执行时不会重复外部动作。效果挂起时再调用 `run`，或没有挂起效果时调用 `resume`，都是错误。

## 绑定与未绑定命令

```velin
perform log("started")
answer = perform ask("Continue?")
```

| 形式 | `resume` 参数 | 典型用途 |
| --- | --- | --- |
| 未绑定 `perform name(...)` | `None` | 日志、界面更新、一次性动作 |
| 绑定 `name = perform name(...)` | `Some(Value)` | 提问、查询、宿主持有的数据 |

给绑定命令传 `None` 是错误。底层 `Machine` 会忽略传给未绑定命令的值；宿主按约定应传 `None`，带 schema 的 `ScriptRunner` 会拒绝给声明为无返回值的命令传回复。返回值在进入槽位前会按 Velin 的数据预算检查。

## 名字是不透明的

`say`、`ask`、`log`、`unlock` 以及任何其他标识符都不是关键字。编译器不知道它们的参数个数或含义。两套宿主可以给同一份源码完全不同的行为：

| 源码中的命令 | CLI 参考宿主 | 游戏宿主 |
| --- | --- | --- |
| `perform say("Hi")` | 打印到 stdout | 显示一句对话 |
| `choice = perform ask("Go?")` | 读一行 stdin | 打开菜单，返回 `1` 或 `0` |
| `perform open_door("east")` | 打印名字和参数，无返回值地恢复 | 玩家有钥匙时播放开门动画 |

未知命令仍然是合法的 Velin。CLI 会打印它们并继续；生产宿主应当拒绝未在允许名单中的名字。

## 声明宿主契约

核心保持宿主无关，但嵌入方可以在边界提供 `HostSchema`。严格 schema 会报告未声明命令，并检查参数数量、参数类型、绑定命令是否返回值，以及该返回值向后传播的类型。

```rust
use velin::{HostSchema, HostSignature, Type, check_script_with_host_schema};

let schema = HostSchema::new()
    .command(
        "say",
        HostSignature::variadic(Vec::new(), Type::Unknown, None),
    )
    .command(
        "ask",
        HostSignature::exact(vec![Type::String], Some(Type::Integer)),
    );

let diagnostics = check_script_with_host_schema("rules.velin", &script, &schema);
```

当工具只建模宿主词汇的一部分时，使用 `.allow_unknown(true)`。`ScriptRunner::configured` 接受同一 schema，在运行时校验参数与回复。回复类型错误可修正后重试；非法调用或效果预算耗尽对该 runner 是终止错误。

## CLI 与 Playground 约定

面向行的参考宿主实现了两个名字，这样示例不必先写自定义嵌入方也能运行：

| 命令 | CLI | Playground |
| --- | --- | --- |
| `say(values...)` | 把值以空格分隔写入 stdout | 追加到输出面板 |
| `ask(prompt...)` | 写出提示、读一行，解析成整数、布尔或字符串 | 从回复数组取出下一个 JSON 值 |

CLI 与 Playground 每次运行最多接受 1,000 次宿主效果和 1 MiB 输出。Playground 还把回复 JSON 限制为 1 MiB。这些是工具限制，不是语言语义。

## 最小 Rust 宿主

```rust
use velin::{ScriptRunner, ScriptYield, Value, compile};

fn reply_for(name: &str, values: &[Value]) -> Result<Option<Value>, String> {
    match name {
        "say" => {
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                print!("{}", value.try_to_display().map_err(|e| e.to_string())?);
            }
            println!();
            Ok(None)
        }
        "ask" => Ok(Some(Value::Integer(1))),
        other => Err(format!("unsupported effect: {other}")),
    }
}

fn drive(source: &str) -> Result<(), String> {
    let script = compile("rules.velin", source).map_err(|e| e.to_string())?;
    let mut runner = ScriptRunner::new(&script).map_err(|e| e.to_string())?;
    let mut outcome = runner.run().map_err(|e| e.to_string())?;
    loop {
        match outcome {
            ScriptYield::Finished => return Ok(()),
            ScriptYield::Host { name, values } => {
                let reply = reply_for(&name, &values)?;
                outcome = runner.resume(reply).map_err(|e| e.to_string())?;
            }
        }
    }
}
```

把 `reply_for` 换成界面、网络或存储。把这些逻辑留在脚本外面。

## 宿主就是信任边界

Velin 保证核心不接触文件系统、网络、时钟或界面。它并不授权宿主行为。宿主应当校验：

- 当前上下文是否允许该命令名。
- 参数个数、类型、范围和应用权限。
- 经 `resume` 或 `set_variable` 返回的任何值。
- 自己的时间、输出大小、网络和存储预算。

进入机器的每个值都会按确定性数据限制检查（4,096 个节点、16 层集合、1 MiB 文本）。宿主注入过大字符串仍会在 Velin 内失败；宿主把同一字符串写到磁盘则自负其责。

## 快照与回放

`Machine::clone` 捕获执行进度，且不复制字节码（`Arc<Program>`）。可用于撤销、分支预览或确定性测试。

```rust
let checkpoint = machine.clone();
let _ = machine.run();
machine = checkpoint; // RNG、变量和 PC 一起回退
```

这是内存中的值，不是持久化格式。序列化后的 `Program` 在反序列化时会重新校验，畸形字节码无法到达 `run`。

## 什么该放进脚本

留在 Velin 里：

- 整数、布尔、字符串、列表和记录。
- 条件、循环、标签和跳转。
- 带种子的 `random` / `chance`。

留在宿主里：

- 控件、音频、文件、HTTP、数据库。
- 时钟、身份、权限和计价。
- 浮点单位，以及超出 Velin 确定性 `Display` 的格式化。

新能力如果需要外部世界，就增加一条宿主命令，不要加进 VM。
