# WebAssembly

[English](WASM.md)

这门语言不是“面向 Wasm”的。核心管线编译成 WebAssembly，是为了让浏览器或任何 Wasm 宿主能在没有服务端的情况下解析、检查和运行同一份脚本。`velin-wasm` 是编组层：源码字符串进去，JSON 出来。它不碰 DOM。

站点上的 Playground 是该包的一个消费者。游戏页或编辑器可以加载同样的 `check` / `run` 导出。

## check 与 run

`wasm-pack` 之后（见[工具链](TOOLING.zh-CN.md)）：

```js
import init, { check, run, run_with_policy } from "./pkg/velin_wasm.js";

await init();

const checked = JSON.parse(check(`default n = 1\nperform say(n)\n`));
// { ok: true, diagnostics: [] }

const ran = JSON.parse(run(`perform say("hi")\n`, "[]"));
// { ok: true, diagnostics: [], output: ["hi"] }

const bounded = JSON.parse(run_with_policy(
    `while true:\n    set value = 1\n`,
    "[]",
    JSON.stringify({ max_fuel: 10_000, max_immediate_fuel: 10_000 }),
));
```

`check(source)` 编译并静态检查。返回 `{ ok, diagnostics }`，每条诊断含 `severity`、`line`、`column` 和 `message`。有任何错误诊断时 `ok` 为 false。

`run(source, repliesJson)` 先检查，再用**脚本化**宿主执行：

| 命令 | Wasm 宿主 |
| --- | --- |
| `say(values...)` | 向 `output` 追加一行 |
| `ask(prompt...)` | 从 `repliesJson` 取下一个 JSON 值 |

`repliesJson` 是 Velin 值的 JSON 数组，可包含整数、布尔、字符串、List 或 Record。例如 `[1]`、`["east"]` 和 `[{"items":[1, 2]}]` 都有效；Record 会规范成稳定键顺序。null、浮点、越界、超预算或畸形值会返回失败的 `RunResult`，错误中包含失败项和嵌套路径，并且不会执行脚本。其他命令名会写进 `output`，并无返回值地恢复，与 CLI 对未知命令的行为一致。

执行失败时（溢出、缺下标、回复用尽、fuel 预算或取消）`RunResult` 还会带 `error`。

`run_with_policy(source, repliesJson, policyJson)` 接受一个 JSON 对象，省略的字段使用默认值。可配置 `max_fuel`、`max_immediate_fuel`、`max_host_effects`、`max_call_depth`、值/机器/宿主载荷/队列的数据预算，以及 `progress_interval`。`PlaygroundSession.new_with_policy` 和 runtime-only 的 `RuntimeMachine.new_with_policy` 使用相同格式；对应对象也提供 `cancel()` 与 `clear_cancellation()`。策略 JSON 限制为 64 KiB，未知字段会被拒绝。

## 持久化 Playground 调试

`PlaygroundSession` 在多次调用间保留一份已经检查的执行状态。各方法返回相同的调试 JSON 结构：`status`、从 1 开始的 `line`、`variables`、`output`、诊断，以及可选的 `error` 或快照 ID。

```js
import init, { PlaygroundSession } from "./pkg/velin_wasm.js";

await init();
const session = new PlaygroundSession(source);
console.log(JSON.parse(session.state()));
console.log(JSON.parse(session.step("[]")));
const saved = JSON.parse(session.snapshot()).snapshot;
console.log(JSON.parse(session.resume("[1]")));
console.log(JSON.parse(session.restore(saved)));
```

`resume(repliesJson)` 运行到下一个宿主效果边界或结束；`step(repliesJson)` 前进一条字节码操作。快照包含 VM、RNG、预算与输出状态，所以恢复后提供另一份回复，会从同一检查点确定性地产生另一条分支。

需要限制调试会话时，使用 `PlaygroundSession.new_with_policy(source, policyJson)`。

当宿主需要从同一份字节码创建多个机器时，可用 `RuntimeProgram` 只解析和校验一次，再创建彼此独立的轻量机器：

```js
import init, { RuntimeProgram } from "./pkg/velin_wasm.js";

await init();
const program = new RuntimeProgram(programJson);
const first = program.create_machine();
const second = program.create_machine_with_policy(policyJson);
```

原有的 `RuntimeMachine` 构造函数继续用于一次性场景。`RuntimeProgram` 本身不可变；返回的每个机器分别拥有独立的执行状态、预算、取消状态和随机数状态。

runtime-only 的 `RuntimeMachine::run_batch(limit)` 可一次返回连续的无返回值 Host 事件：

```json
{"kind":"effects","effects":[{"host_id":7,"values":[1]}]}
```

结果按源码顺序排列；遇到绑定 Host、完成或错误时停止。`{"kind":"empty"}` 表示本批没有无返回值事件，宿主随后可调用 `run()` 区分完成和绑定 Host。`limit` 是 VM 批次上限，应用仍应在 JavaScript 驱动层维护队列容量和 payload 背压。

## 这条路径上的预算

- 输出上限 1 MiB。
- 每次运行最多 1,000 次宿主效果。
- 回复 JSON 上限 1 MiB。
- 每次 `run` / `resume` 突发仍最多 10,000 条立即 VM 步。

## 什么留在 JavaScript

Wasm 负责求值脚本。JavaScript 仍然：

- 渲染界面、播放音频。
- 决定允许哪些命令名。
- 提供 `ask` 的答案（表单、存档或回复框）。
- 施加页面级的时间和体积限制。

这种划分与桌面上的[宿主协议](HOST.zh-CN.md)相同：VM 让出，嵌入方行动，再 `resume`。Playground 的脚本化宿主只是为了演示不必读 stdin。

## 构建该包

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
```

手写的 Wasm 代码是安全 Rust。`#[wasm_bindgen]` 会生成 `unsafe` 胶水，所以该 crate 不能继承 `unsafe_code = "forbid"`。核心 crate 仍保持禁止。

## 最小页面

```html
<script type="module">
  import init, { check, run } from "./pkg/velin_wasm.js";
  await init();

  const source = `default hp = 30
choice = perform ask("Drink?")
if choice == 1:
    perform say("restored")
`;

  console.log(check(source));
  console.log(run(source, "[1]"));
</script>
```

把 `console.log` 换成你的界面。把 `say` 的展示和 `ask` 的提问留在 JavaScript，不要把这些 API 加进 VM。

站点 Playground 就是带编辑器的这种写法。走读见 [Playground](PLAYGROUND.zh-CN.md)。
