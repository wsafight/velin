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

原地格式化文件，或只检查源码是否为规范格式而不写入：

```sh
cargo run -p velin-cli -- fmt examples/adventure.velin
cargo run -p velin-cli -- fmt --check examples/adventure.velin
```

formatter 是确定性的，并保留空行、独立行注释和行尾注释。`fmt -` 从 stdin 读取并把格式化结果写到 stdout。使用 `velin check --json <file>` 可得到稳定的 `{ ok, diagnostics, error }` JSON 结果。`check`、`fmt` 和 `run` 都接受 `-` 作为源码路径；`velin --help` 与 `velin --version` 会打印命令信息并成功退出。

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

- 实时解析、降级、类型、确定赋值与模块依赖诊断。
- 语言名称及 schema 驱动宿主命令的补全、悬停说明与签名帮助。
- semantic tokens、文档格式化、格式化 code action，以及 prepare-rename/rename。
- 文档符号，以及模块、函数、变量和标签的 workspace symbols。
- 在已加载模块文档间跳转定义和查找引用。
- 对不支持的请求返回标准 JSON-RPC `MethodNotFound`。

初始化时，服务器会索引给定 workspace roots 下的 `.velin` 文件，上限为 128 个文件、合计 4 MiB 源码和 32 层目录；打开的缓冲区覆盖对应磁盘副本。服务器将 JSON-RPC 消息限制为 4 MiB，并在分配正文前检查头部预算。
启动可复用 Rust 服务器的嵌入方可以使用 `Server::with_host_schema`；独立 stdio 二进制仍保持宿主中立。

## VS Code 扩展

扩展位于 `editors/vscode-velin`，提供 `.velin` 文件注册、TextMate 高亮、缩进规则、补全、诊断和标签大纲。各平台发布的 VSIX 会内置对应的 `velin-lsp`；下面的设置仅用于覆盖内置服务器或本地开发。

```sh
cd editors/vscode-velin
npm ci
```

在 VS Code 中打开该目录，按 `F5` 启动 Extension Development Host。没有内置服务器时，扩展会从 `PATH` 启动 `velin-lsp`。使用本地二进制时设置：

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

`.github/workflows/core.yml` 运行格式检查、Clippy、workspace 测试、Wasm target 构建和整个 workspace 的 crate 打包验证。`.github/workflows/site.yml` 构建并测试完整静态站，上传产物，并从仓库默认分支部署到 GitHub Pages。`.github/workflows/release.yml` 在手动运行时构建 CLI/LSP 压缩包与各平台 VSIX；推送与 manifest 版本一致的 `v*` tag 时还会创建带 SHA-256 校验和的 GitHub Release。

## Workspace 验证

提交 pull request 前可以运行与 CI 相同的核心门禁：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build -p velin-wasm --target wasm32-unknown-unknown
cargo package --workspace
```
