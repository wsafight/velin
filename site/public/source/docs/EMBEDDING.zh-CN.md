# 在 Rust 中嵌入 Velin

[English](EMBEDDING.md)

`velin` 门面 crate 暴露从源码到运行时的完整管线。嵌入方负责编译并检查源码、创建机器、分派不透明的宿主效果，再用可选返回值恢复执行。

## 添加依赖

在当前 workspace 附近开发时可以使用路径依赖：

```toml
[dependencies]
velin = { path = "../velin/crates/velin" }
```

项目仍处于首次发布前，目前不承诺 Rust API 或序列化程序格式的向后兼容。

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
