# Playground

[English](PLAYGROUND.md)

站点 Playground 在当前浏览器里解析、检查并运行 `.velin`。它加载 `velin-wasm`。源码和 `ask` 回复不会发到服务端。

从顶栏打开，或打开本文档站的 `/playground/`。

## 打开站点上的 Playground

从顶栏打开 `/playground/`。示例是缩短版 `adventure.velin`。

1. 确认 **ask 回复** 是 `[1]`。
2. 点 **运行**。诊断应保持为空；输出里应出现恢复后的 HP `40`。
3. 把回复改成 `[0]` 再运行，脚本走另一分支。
4. 故意写一个类型错误（例如 `if 1:`），点 **仅检查**，只看诊断、不执行。

页面有三块：

| 区域 | 作用 |
| --- | --- |
| 脚本 | Velin 源码文本框。示例是缩短版 `adventure.velin`。 |
| 诊断 | 解析、类型和确定赋值信息，或“没有问题。” |
| 输出 | 点 **运行** 之后 `say`（以及未知命令）产生的行。 |

按钮：

- **运行** 先检查再执行。检查器报错则跳过执行。
- **仅检查** 只编译和分析，不运行。
- **停止** 会终止当前 Worker 请求；下一次操作会启动新 Worker。

`ask` 回复栏是按顺序消耗的 JSON 数组。示例在字段为 `[1]` 时喝药水。用 `[0]` 走另一分支。字符串和布尔也合法：`["east"]`、`[true]`。畸形 JSON 或任意一个不支持的项会让整组回复被拒绝，后续答案不会错配到别的 `ask`。

语言跟随站点（`?lang=zh` 或顶栏切换）。主题跟随 `localStorage` 里的 `velin-theme`。

## 浏览器里的限制

一次运行在 1,000 次宿主效果、1 MiB 输出或页面 Worker 执行 5 秒后停止。回复 JSON 上限 1 MiB。两次让出之间的立即 VM 步数仍是 10,000。见[资源预算](LIMITS.zh-CN.md)和 [WebAssembly](WASM.zh-CN.md)。

## 本地副本

从仓库提供同一套界面：

```sh
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

然后打开 `http://localhost:8080`。文档站构建时会把该目录复制到 `/playground/`。

## Playground 宿主是什么

它是脚本化的参考宿主，不是完整嵌入方。`say` 追加输出；`ask` 读 JSON 数组；其他名字会被记录并无返回值地恢复。产品宿主仍应允许名单命令并提供真实界面。约定见[宿主协议](HOST.zh-CN.md)。
