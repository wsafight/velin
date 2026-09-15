# 在 Rust 中嵌入 Velin

[English](EMBEDDING.md)

`velin` 门面 crate 暴露从源码到运行时的完整管线。嵌入方负责编译并检查源码、创建机器、分派不透明的宿主效果，再用可选返回值恢复执行。

## 添加依赖

在当前 workspace 附近开发时可以使用路径依赖：

```toml
[dependencies]
velin = { path = "../velin/crates/velin" }
```

项目仍处于预稳定的 `0.x` 阶段，目前不承诺 Rust API 或序列化程序格式的向后兼容。

## 编译与检查

`compile` 返回 `CompiledScript`，其中包含共享字节码、驻留后的宿主命令名称、标签和编译期默认值。解析或降级失败会立即返回。静态分析是独立步骤，调用方可以自行决定诊断策略。

```rust
use velin::{check_script, compile};

let source = std::fs::read_to_string("rules.velin")?;
let script = compile("rules.velin", &source)?;
let diagnostics = check_script("rules.velin", &script);

for diagnostic in &diagnostics {
    eprintln!("{diagnostic}");
}
if diagnostics.iter().any(velin::Diagnostic::is_error) {
    return Err("script did not pass static checks".into());
}
```

`check_script` 运行 CFG 感知的类型传播和确定赋值分析，不会修改程序。

宿主词汇已知时，用 `HostSignature::exact` 或 `HostSignature::variadic` 构建 `HostSchema`，再调用 `check_script_with_host_schema`。这会增加命令名、参数数量、参数类型、绑定和返回值传播检查，同时不把宿主专用名称放进语言核心。

对于“输入值 -> 输出值”的扩展，使用 `PureModule`。它在编译时接收明确的输入类型映射，只允许 `return(value)` 与 `fail(message)` 两种控制信号，拒绝其他宿主命令以及 `random`/`chance`，并为每次调用创建全新的 VM 状态：

```rust
use std::collections::BTreeMap;
use velin::{PureModule, Type, Value};

let module = PureModule::compile(
    "reward.velin",
    "perform return(input * 2)\n",
    BTreeMap::from([("input".to_owned(), Type::Integer)]),
)?;
let result = module.invoke(BTreeMap::from([("input".to_owned(), Value::Integer(21))]))?;
assert_eq!(result, Value::Integer(42));
```

`PureModule::invoke` 会通过 `PureModuleError` 报告缺少或未知输入、输入类型不匹配、显式失败、缺少返回以及 VM 执行错误。它不暴露持久化机器或可配置 fuel；每次调用都使用 VM 当前的有界立即执行步数限制。

大多数语句语言宿主应优先使用 `ScriptRunner`：它会验证字节码、安装默认值、把宿主 ID 解析为名称、执行累计宿主效果预算，并可在运行时应用同一份 `HostSchema`。只有需要更底层控制时才直接使用 `Machine`。

对于连续的无返回值命令，可使用 `ScriptRunner::run_effect_batch` 批量取得事件，再交给宿主队列消费。`HostEventQueue` 位于宿主驱动层，提供容量、值数量和文本字节的背压限制；绑定命令仍会作为自然屏障交给 `run` / `resume`。

## 创建机器

`Machine::new` 会在执行前验证完整程序。新建帧后需要应用脚本的全部默认值：

```rust
use velin::Machine;

let script = velin::compile("inline.velin", "default score = 0\n")?;
let mut machine = Machine::new(script.program.clone())?;
for (name, value) in &script.defaults {
    assert!(machine.set_variable(name, value.clone()));
}
```

当脚本使用 `random` 或 `chance` 且宿主需要明确的回放种子时，使用 `Machine::with_seed(program, seed)`。`Machine::new` 使用种子 `0`。

## 驱动宿主效果

`run` 会前进到下一个宿主效果或程序结束。通过 `CompiledScript::host_name` 查询命令名称，在嵌入应用中执行行为，再调用 `resume`。

```rust
use velin::{Value, Yield};

let script = velin::compile(
    "inline.velin",
    "perform notify(\"ready\")\n",
)?;
let mut machine = velin::Machine::new(script.program.clone())?;
let mut outcome = machine.run()?;
loop {
    match outcome {
        Yield::Finished => break,
        Yield::Host { host_id, values } => {
            let name = script.host_name(host_id).ok_or("unknown host id")?;
            let reply = match name {
                "notify" => {
                    println!("{}", values[0].try_to_display()?);
                    None
                }
                "read_score" => Some(Value::Integer(42)),
                other => return Err(format!("unsupported effect: {other}").into()),
            };
            outcome = machine.resume(reply)?;
        }
    }
}
```

不带绑定的 `perform` 使用 `resume(None)`。写成 `answer = perform read_score()` 的命令要求 `Some(Value)`。效果挂起期间再次调用 `run`，或没有挂起效果时调用 `resume`，都会报错。

## 把宿主视为信任边界

Velin 自身从不执行 I/O，宿主仍应验证：

- 当前上下文是否允许该命令名称。
- 参数数量、类型、范围与应用权限。
- 恢复绑定命令前的返回值。
- 时间、输出、网络、存储等外部预算。

通过 `set_variable` 或 `resume` 进入机器的每个值都会接受 Velin 确定性数据预算检查。

## 快照与回放

`Machine` 实现了 `Clone`。克隆会捕获程序计数器、变量、挂起的宿主请求、完成状态和 RNG 进度，同时通过 `Arc<Program>` 共享不可变字节码。

```rust
let script = velin::compile(
    "inline.velin",
    "set die = random(1, 6)\n",
)?;
let mut machine = velin::Machine::with_seed(script.program.clone(), 7)?;
let checkpoint = machine.clone();
let first = machine.run();

machine = checkpoint;
let replay = machine.run();
assert_eq!(format!("{first:?}"), format!("{replay:?}"));
```

`Machine` 快照是内存值，不是稳定的持久化格式。如果应用序列化 `Program`，反序列化会再次运行结构校验，之后程序才能执行。

## 低层 API

不需要语句语言的嵌入方可以直接使用 `parse_expression`、`check_expression`、`compile_expression`、`ProgramBuilder` 和 `Machine`。`velin-eval` 保持树遍历参考语义，适合用于差分测试。
