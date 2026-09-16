# 语法速查

[English](CHEATSHEET.md)

语句语言的一页速查。语义见[语言参考](LANGUAGE.zh-CN.md)，可复制脚本见[示例与写法](EXAMPLES.zh-CN.md)。

## 语句

每层恰好四个空格缩进，制表符会被拒绝。`#` 开始行注释。

| 形式 | 说明 |
| --- | --- |
| `default name = constant` | 仅顶层；编译期常量；每个名字只能一次 |
| `set name = expression` | 运行时赋值 |
| `name = expression` | 与 `set` 相同，关键字可省略 |
| `perform command(args...)` | 未绑定宿主效果；用 `None` 恢复 |
| `name = perform command(args...)` | 绑定宿主效果；用 `Some(value)` 恢复 |
| `if condition:` / `elif condition:` / `else:` | 条件必须是布尔 |
| `while condition:` | 每次迭代前重新检查 |
| `label name:` | 跳转目标；可以带缩进体 |
| `jump name` | 跳到标签 |

## 表达式

优先级从高到低：一元 `-` / `not`，然后 `*` `/`，然后 `+` `-`，然后比较，然后 `==` `!=`，然后 `and`，然后 `or`。

- 整数是 `i64`。溢出和除以零都是错误。
- `+` 连接两个字符串。
- `<` `<=` `>` `>=` 比较两个整数或两个字符串。
- `==` / `!=` 对任意值做确定性比较。
- `"text [expression]"` 做插值。`[[` 表示字面 `[`。

## 值与内置函数

| 类型 | 构造 |
| --- | --- |
| 整数 | `0`, `-12` |
| 布尔 | `true`, `false` |
| 字符串 | `"hello"` |
| 列表 | `list(1, 2, 3)` |
| 记录 | `record("name", "Ada", "hp", 30)` |

| 调用 | 结果 |
| --- | --- |
| `get(c, k)` / `get(c, k, fallback)` | 列表项或记录字段 |
| `put(c, k, v)` | 更新后的列表或记录 |
| `push(list, v)` | 追加 `v` 的列表 |
| `remove(c, k)` | 去掉项或字段 |
| `len(v)` | 字符、元素或字段个数 |
| `contains(c, v)` | 成员、键或子串 |
| `random(lo, hi)` | 带种子的闭区间整数 |
| `chance(percent)` | 带种子的布尔，`0`–`100` |

列表下标从零开始。集合是持久化的：要把 `push` / `put` / `remove` 的结果赋回去。

## 宿主名字

`say`、`ask`、`log` 以及 `perform` 后面的任何标识符都是宿主约定，不是关键字。CLI 和 Playground 实现 `say` 和 `ask`。

## 硬限制（短表）

- 源码：1 MiB、10,000 行、64 层语句块。
- 单条表达式：64 KiB、512 个 token、32 层括号。
- VM：默认每次执行调用最多 10,000 个立即 fuel 单位；可用 `ExecutionPolicy` 配置。

完整表格见[资源预算](LIMITS.zh-CN.md)。
