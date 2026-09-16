# 语言参考

[English](LANGUAGE.md)

Velin 源文件使用 `.velin` 扩展名。语言采用缩进语法、执行结果确定，并刻意保持小型：脚本负责值与控制流，嵌入宿主负责全部外部效果。

本页是完整参考。一页查阅用[语法速查](CHEATSHEET.zh-CN.md)，可复制脚本用[示例](EXAMPLES.zh-CN.md)，列表和记录的细节在[值与集合](VALUES.zh-CN.md)。

## 文件结构

- 每层使用恰好 4 个空格缩进，制表符会被拒绝。
- `#` 开始一条行注释。
- 空行会被忽略。
- 标识符用于变量、标签和宿主命令名称。
- 单份源码最多 1 MiB、10,000 个物理行和 64 层语句块。

```velin
# 默认值是编译期常量。
default hp = 30
default ready = true

if ready:
    perform say("HP: [hp]")
```

## 值

Velin 包含五种值类型：

| 类型 | 示例与构造方式 |
| --- | --- |
| 整数 | `0`、`-12`、`9223372036854775807`（`i64`） |
| 布尔值 | `true`、`false` |
| 字符串 | `"hello"`、`"HP: [hp]"` |
| 列表 | `[1, 2, 3]` 或 `list(1, 2, 3)` |
| 记录 | `{name: "Ada", score: 10}` 或 `record("name", "Ada", "score", 10)` |

列表和记录是持久化值。`push`、`put` 和 `remove` 返回更新后的集合，不会修改输入值。没有 `null`，也没有浮点类型。

忘记把 `push` 或 `put` 的结果赋回去，原来的集合不会变。缺键和缺下标是运行时错误，除非给 `get` 提供回退值。

## 变量与默认值

`default` 声明顶层初始值。它必须是编译期常量，不能引用运行时变量，并且同一个默认值名称只能声明一次。

```velin
default score = 0
default tags = list("new")
```

`set` 为变量赋予运行时表达式。普通赋值可以省略 `set` 关键字。

```velin
set score = score + 10
score = score + 10
score += 10
items[0] = "first"
```

`+=`、`-=`、`*=` 和 `/=` 会降级为对应的赋值表达式。`value[key]` 通过 `get` 读取 List 或 Record；对单个下标赋值会降级为 `put`，并把更新后的集合写回变量。

如果变量并非在所有流入的控制流路径上都已赋值，静态检查器会报告读取错误。

## 表达式与运算符

按优先级从高到低：

| 运算符 | 含义 |
| --- | --- |
| `-value`、`not value` | 整数取负和布尔取反 |
| `*`、`/` | 带溢出检查的整数乘除法 |
| `+`、`-` | 带溢出检查的整数运算；`+` 也能拼接两个字符串 |
| `<`、`<=`、`>`、`>=` | 两个整数或两个字符串的顺序比较 |
| `==`、`!=` | 任意值的确定性相等比较 |
| `and` | 短路布尔与 |
| `or` | 短路布尔或 |

算术溢出和除以零都会报错。括号可以组合表达式。单个表达式最多 64 KiB、512 个 token 和 32 层括号。

`and` / `or` 只接受布尔。检查器能证明类型时，整数和 `and` 混用会报错。

## 字符串插值

在字符串中用方括号包围表达式。每种值都有确定性的显示形式。

```velin
default name = "Ada"
default score = 7
perform say("[name] has [score + 1] points")
```

`[[` 表示字面量左方括号。插值最多嵌套 32 层，并共享累计解析预算。

## 条件控制流

`if`、`elif` 和 `else` 分别拥有一个缩进块，条件必须求值为布尔值。

```velin
if score >= 100:
    perform unlock("gold")
elif score >= 50:
    perform unlock("silver")
else:
    perform unlock("bronze")
```

## 循环

`while` 在每次迭代前重新计算布尔条件。`for` 从下标零开始依次访问 List。`break` 离开最近的循环，`continue` 进入下一次迭代。

```velin
default n = 3
while n > 0:
    perform say(n)
    set n = n - 1

set total = 0
for item in [1, 2, 3]:
    if item == 2:
        continue
    total += item
```

默认 VM 策略允许每次执行调用最多 10,000 个立即 fuel 单位，因此不含 `perform` 的循环不能无限占用调用线程。

## 模块与纯函数

`compile_modules` 通过宿主控制的 `ModuleResolver` 加载 `import`。被导入文件只能包含 import、私有 `fn` 和显式 `export fn`，不能执行顶层状态机语句。

```velin
# math.velin
export fn score(items):
    set total = 0
    for item in items:
        total += item
    return total * 2

# main.velin
import math
set result = call math.score([1, 2, 3])
perform publish(result)
```

函数拥有参数、局部变量和一个末尾 `return`。函数中不能使用 `perform`、`default`、标签、跳转、`random`、`chance` 或隐藏状态。同一模块内调用可省略限定名：`set doubled = call twice(value)`。调用图必须无环，跨模块调用的函数必须显式导出。

编译器会在普通静态检查和字节码 lowering 前以卫生方式展开调用。因此快照不需要保存运行时调用栈，同时仍沿用现有 fuel、数据、调试位置、artifact、Wasm 和 C 运行时规则。模块图最多 128 个模块、4 MiB 源码，展开后的调用最多 4,096 次。

## 标签与跳转

标签标记一个字节码目标，也可以拥有可选的缩进块。`jump` 将控制流转移到标签。重复标签和未知标签都是编译错误。

```velin
label retry:
    choice = perform ask("Try again?")
    if choice == true:
        jump retry

label done:
    perform say("Finished")
```

## 宿主效果

`perform` 是脚本接触外部世界的唯一方式。命令名称对 Velin 不透明，具体含义由宿主决定。

```velin
perform log("started")
answer = perform ask("Continue?")
```

第一种形式不接收返回值。绑定形式会挂起 VM，并在宿主调用 `resume(Some(value))` 后保存宿主提供的值。`say`、`ask`、`log` 和任何其他名称都是宿主约定，而不是语言关键字。

宿主即使没实现某个名字也能收到它：CLI 会打印名字和参数然后恢复。产品宿主应当改为允许名单命令。见[宿主协议](HOST.zh-CN.md)。

## 内置函数

| 函数 | 结果 |
| --- | --- |
| `list(values...)` | 最多接收 128 个参数的列表 |
| `record(key, value, ...)` | 记录；键必须是唯一字符串 |
| `get(collection, key)` | 列表项或记录值；缺失时报错 |
| `get(collection, key, fallback)` | 返回值，缺失时返回 `fallback` |
| `put(collection, key, value)` | 更新后的列表或记录 |
| `push(list, value)` | 追加 `value` 后的列表 |
| `remove(collection, key)` | 移除列表项或记录字段 |
| `len(value)` | 字符、列表项或记录字段数量 |
| `contains(collection, value)` | 成员、记录键或子串检查 |
| `random(low, high)` | 闭区间内由种子驱动的整数 |
| `chance(percent)` | 0 到 100 整数百分比对应的种子驱动布尔值 |

列表索引从 0 开始。`put` 和 `remove` 要求列表索引已经存在；删除不存在的记录键会报错。随机状态保存在机器帧内，因此同一种子与输入可以精确回放。

## 静态诊断

`velin check` 组合了解析、降级、保守类型推断和确定赋值分析。检查器只报告能够证明的类型冲突；宿主返回值与冲突分支类型会变为 `Unknown`，不会阻止合法的动态宿主行为。

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

诊断使用从 1 开始的行列位置，并指向相关表达式节点。完整策略见[静态检查](CHECKING.zh-CN.md)。

## 常见错误

- **制表符。** 解析器会拒绝。使用四个空格。
- **把 `say` 当关键字。** 要写 `perform say(...)`。单独的 `say(...)` 不是语句。
- **原地改列表。** `push(items, "key")` 若没有 `set items = ...`，看起来不会有变化。
- **`if 1:`。** 条件必须是布尔。写成 `if choice == 1:`。
- **没有 `perform` 的紧循环。** VM 会在立即 fuel 上限（默认 10,000）后停止。
- **只在一个分支里赋值却在外面读取。** 确定赋值会报。

源码、值和 VM 的预算列在[资源预算](LIMITS.zh-CN.md)。
