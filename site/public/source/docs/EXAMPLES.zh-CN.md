# 示例与写法

[English](EXAMPLES.md)

这些脚本都停留在 Velin 的语句语言里。I/O 仍然属于宿主：CLI 会打印 `say`，并从 stdin 读取 `ask`；Playground 消耗 JSON 数组形式的回复。其他嵌入方可以绑定同样的命令名，也可以忽略这些名字、定义自己的命令。

运行仓库里的两个文件：

```sh
cargo run -p velin-cli -- check examples/counting.velin
cargo run -p velin-cli -- run examples/counting.velin
cargo run -p velin-cli -- check examples/adventure.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

把下面任一示例粘贴进 [Playground](PLAYGROUND.zh-CN.md)，并把 `ask` 回复设成 JSON 数组，例如 `[1]`。每条写法会注明它需要的回复。

> 每层四个空格。集合是持久化的：要把 `push` / `put` 的结果赋回去。`say` 和 `ask` 是宿主名字，你的嵌入方可以换成别的。

## 运行仓库里的示例

`examples/counting.velin` 不会等待宿主。它把 `1..5` 累加后结束：

```velin
default total = 0
default n = 1

while n <= 5:
    set total = total + n
    perform say("added", n, "running total is", total)
    n = n + 1

perform say("final total:", total)
```

`examples/adventure.velin` 是典型的分支样本。`default` 设置编译期常量，`perform ask` 暂停以等待宿主整数，`jump` 选择结局：

```velin
default hp = 30
default potions = 2

label start:
    perform say("You wake in a cold cell.")
    choice = perform ask("Drink a potion? (1 = yes, 0 = no)")
    if choice == 1:
        set hp = hp + 10
        set potions = potions - 1
        perform say("You feel better.")
    else:
        perform say("You steel yourself.")

    while potions > 0:
        set potions = potions - 1
        perform say("You stash a potion for later.")

    if hp > 35:
        jump good_end
    perform say("You limp onward into the dark.")

label good_end:
    perform say("You escape, unbroken.")
```

回复 `1` 会喝下药水（`hp` 变成 `40`，走到好结局）。回复 `0` 则跳过。

## 用列表做背包

列表是持久化的。`push` 返回新列表；除非你把结果赋回去，旧值不会变。

```velin
default items = list("torch")
default gold = 5

choice = perform ask("Buy a key for 3 gold? (1 = yes)")
if choice == 1:
    if gold >= 3:
        set gold = gold - 3
        set items = push(items, "key")
        perform say("You bought a key. Gold left: [gold]")
    else:
        perform say("Not enough gold.")
else:
    perform say("You keep walking.")

perform say("Pack:", items)
```

`len(items)` 是元素个数。`contains(items, "key")` 做成员判断。`get(items, 0)` 读取第一项；缺少的下标是运行时错误，除非提供回退值：`get(items, 3, "nothing")`。

## 用记录做角色卡

记录使用交替的字符串键和值。`put` 更新一个字段并返回新记录。

```velin
default hero = record("name", "Ada", "hp", 30, "atk", 4)

perform say("[get(hero, \"name\")] stands ready.")
set hero = put(hero, "hp", get(hero, "hp") - 7)
perform say("HP is now [get(hero, \"hp\")]")

if get(hero, "hp") <= 0:
    perform say("Ada falls.")
else:
    perform say("Ada holds.")
```

`contains(hero, "hp")` 测试键是否存在。`remove(hero, "atk")` 返回去掉该字段的记录；删除不存在的键是错误。

## 带种子的随机遭遇

随机调用消耗机器局部的 RNG 状态。相同种子和回复会精确回放。

```velin
default roll = random(1, 6)

perform say("You rolled [roll].")
if roll >= 5:
    perform say("A rare find!")
elif roll >= 3:
    perform say("The path is quiet.")
else:
    perform say("You stumble.")

if chance(25):
    perform say("It starts to rain.")
```

在 Rust 里，宿主需要控制序列时用 `Machine::with_seed(program, seed)` 创建机器。CLI 和 Playground 使用种子 `0`。

## 用标签做房间循环

标签是跳转目标，适合房间、菜单和重试循环。每次 `run` / `resume` 仍会在 10,000 条立即指令后停止，所以循环应当 `perform`（让出给宿主）而不是空转。

```velin
default visits = 0

label hall:
    set visits = visits + 1
    perform say("Hall. Visit [visits].")
    action = perform ask("1 stay, 2 leave")
    if action == 1:
        jump hall
    perform say("You leave the hall.")
```

未知或重复的标签是编译错误。循环条件是局部布尔值时优先用 `while`；多个命名场景互相指向时优先用 `jump`。

## 宿主返回值写回脚本

绑定形式的 `perform` 会存下宿主返回的任何值。检查器把该值当作 `Unknown`，所以后续用法必须对宿主实际提供的值成立。

```velin
default score = 0

delta = perform read_amount("Adjustment")
if delta > 0:
    set score = score + delta
    perform say("Score is [score]")
else:
    perform say("Ignored non-positive amount.")
```

CLI 参考宿主并不实现 `read_amount`；它会打印命令并无返回值地恢复。真正的嵌入方应对这次绑定调用返回 `Some(Value::Integer(...))`。详见[宿主协议](HOST.zh-CN.md)。

## 双选项菜单

菜单就是绑定的 `ask` 加上 `if` / `elif`。回复 `[2]` 选第二项。

```velin
default room = "hall"

choice = perform ask("1 look, 2 leave")
if choice == 1:
    perform say("Dust. A door to the east.")
elif choice == 2:
    set room = "street"
    perform say("You step outside.")
else:
    perform say("That is not a choice.")

perform say("You are in the [room].")
```

把选项编号写在提示里。语言并不认识什么是菜单。

## 先检查再运行

`velin check` 报告解析错误、能够证明的类型不匹配，以及在某条入边上尚未赋值就被读取的变量。

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

涉及宿主时脚本仍可以保持动态：宿主返回值和冲突的分支类型会变成 `Unknown`，而不是误报。只有在你接受这些诊断、并认可自己的宿主约定之后再运行。
