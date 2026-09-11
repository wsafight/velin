# 工具链

[English](TOOLING.md)

Velin 的全部工具与嵌入 API 共用同一套解析器、检查器、编译器和 VM。编辑器中通过的脚本，会由 CLI 与浏览器构建中的同一份代码检查。

## 环境要求

- Rust 1.88 或更高版本与 Cargo。
- 文档站需要 Node.js 22.12 或更高版本。
- 浏览器 Playground 需要 `wasm-pack` 与 `wasm32-unknown-unknown` target。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

## 命令行

从 workspace 根目录构建 CLI：

```sh
cargo build -p velin-cli
```

只检查脚本，不运行宿主效果：

```sh
cargo run -p velin-cli -- check examples/adventure.velin
```

`check` 会打印解析、降级、类型和确定赋值诊断。存在 error 时退出码为 1；只有 warning 不会导致命令失败。

通过行式参考宿主运行：

```sh
cargo run -p velin-cli -- run examples/counting.velin
echo 1 | cargo run -p velin-cli -- run examples/adventure.velin
```

参考宿主实现两个约定：

| 命令 | CLI 行为 |
| --- | --- |
| `say(values...)` | 用空格分隔值并写入 stdout |
| `ask(prompt...)` | 写出提示，读取一行，并返回整数、布尔值或字符串 |

未知命令仍然是合法的宿主效果；参考运行器会打印名称与参数，然后在不返回值的情况下恢复执行。

## 语言服务器

构建 stdio 语言服务器：

```sh
cargo build -p velin-lsp
```

`velin-lsp` 提供：

- 实时解析、降级、类型与确定赋值诊断。
- 关键字、内置函数、变量和标签补全。
- 标签文档符号。
- 对不支持的请求返回标准 JSON-RPC `MethodNotFound`。

服务器将 JSON-RPC 消息限制为 4 MiB，并在分配正文前检查头部预算。

## VS Code 扩展

扩展位于 `editors/vscode-velin`，提供 `.velin` 文件注册、TextMate 高亮、缩进规则、补全、诊断和标签大纲。

```sh
cd editors/vscode-velin
npm ci
```

在 VS Code 中打开该目录，按 `F5` 启动 Extension Development Host。扩展默认从 `PATH` 启动 `velin-lsp`。使用本地二进制时设置：

```json
{ "velin.server.path": "${workspaceFolder}/target/debug/velin-lsp" }
```

## 浏览器 Playground

站点界面的用法见 [Playground](PLAYGROUND.zh-CN.md)。在自己的页面里调用 Wasm 导出见 [WebAssembly](WASM.zh-CN.md)。本地重建该包：

```sh
wasm-pack build --target web --release \
  --out-dir ../../web/playground/pkg crates/velin-wasm
python3 -m http.server --directory web/playground 8080
```

打开 `http://localhost:8080`。`check(source)` 和 `run(source, repliesJson)` 完全在浏览器中执行。浏览器参考宿主把 `say` 参数写入输出，并从 JSON 数组依次消费 `ask` 回复。

## 文档站

`site/` 下的独立 Astro 项目从仓库 Markdown 生成中英文内容，并在 `/playground/` 发布 Playground。

```sh
cd site
npm ci
npm run check
npm run build
npm test
```

`npm run build` 会编译 `velin-wasm`、准备双语内容、生成静态 HTML，并检查全部内部链接。`npm test` 会针对构建结果运行桌面与移动端 Chromium 测试。

本地设置 `GITHUB_REPOSITORY=wsafight/velin` 可以测试 `/velin/` 下的 GitHub Pages 路径。`DOCS_SITE` 和 `DOCS_BASE` 可以覆盖生成的域名与基础路径。

## 持续集成

`.github/workflows/core.yml` 运行格式检查、Clippy、workspace 测试和 Wasm target 构建。`.github/workflows/site.yml` 构建并测试完整静态站，上传产物，并从仓库默认分支部署到 GitHub Pages。

## Workspace 验证

提交 pull request 前可以运行与 CI 相同的核心门禁：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build -p velin-wasm --target wasm32-unknown-unknown
```
