# WebAssembly

[English](WASM.md)

这门语言不是“面向 Wasm”的。核心管线编译成 WebAssembly，是为了让浏览器或任何 Wasm 宿主能在没有服务端的情况下解析、检查和运行同一份脚本。`velin-wasm` 是编组层：源码字符串进去，JSON 出来。它不碰 DOM。

站点上的 Playground 是该包的一个消费者。游戏页或编辑器可以加载同样的 `check` / `run` 导出。

## check 与 run

`wasm-pack` 之后（见[工具链](TOOLING.zh-CN.md)）：

```js
import init, { check, run } from "./pkg/velin_wasm.js";

await init();

const checked = JSON.parse(check(`default n = 1\nperform say(n)\n`));
// { ok: true, diagnostics: [] }

const ran = JSON.parse(run(`perform say("hi")\n`, "[]"));
// { ok: true, diagnostics: [], output: ["hi"] }
```

`check(source)` 编译并静态检查。返回 `{ ok, diagnostics }`，每条诊断含 `severity`、`line`、`column` 和 `message`。有任何错误诊断时 `ok` 为 false。

`run(source, repliesJson)` 先检查，再用**脚本化**宿主执行：

| 命令 | Wasm 宿主 |
| --- | --- |
| `say(values...)` | 向 `output` 追加一行 |
| `ask(prompt...)` | 从 `repliesJson` 取下一个 JSON 值 |

`repliesJson` 是整数、布尔或字符串的 JSON 数组，例如 `[1]` 或 `["east"]`。畸形 JSON 当作 `[]`。其他命令名会写进 `output`，并无返回值地恢复，与 CLI 对未知命令的行为一致。

执行失败时（溢出、缺下标、回复用尽、步数预算）`RunResult` 还会带 `error`。

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
