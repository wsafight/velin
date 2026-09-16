# 兼容性政策

[English](COMPATIBILITY.md)

本政策从 Velin `0.4.0` 开始生效。Velin 仍处于 1.0 之前，但各公开边界不再共用一个模糊的“预稳定”标签，而是分别遵守以下契约。

## 源语言

有效的 `0.4.x` 源码及其确定性运行行为在 `0.4.x` 补丁版本之间保持兼容。补丁版本可以增加语法或诊断，但不能静默改变已有合法程序的含义。

在 1.0 之前，次版本可以包含源码不兼容变更。此类变更必须写入 `CHANGELOG.md`，提供迁移前后示例，并保留展示旧行为的 fixture。只要旧形式能够安全识别，应优先至少经过一个次版本的弃用诊断期。

## Rust API

`velin` 门面 crate 是受支持的 Rust API，并遵循 Cargo SemVer。同一次版本线内的补丁版本保持其公开 API。1.0 之前的次版本可以包含破坏性变更，但必须提供迁移说明。

底层 crate（`velin-syntax`、`velin-parse`、`velin-eval`、`velin-bytecode`、`velin-compile`、`velin-check`、`velin-lang` 和 `velin-vm`）可供专用嵌入方使用，但未由 `velin` 重新导出的 API 均视为实验性，除非其文档另有说明。

弃用 API 在当前次版本线剩余时间内继续可调用。删除已经弃用的门面 API 需要提升次版本，并在 changelog 中提供迁移说明。

## Artifact

每个 `.velinc` artifact 都携带 `ARTIFACT_MAGIC` 和 `ARTIFACT_VERSION`。`0.4.x` 运行时读取版本 4 artifact。解码器会在执行前拒绝未知版本、旧版本、损坏、超限或语义非法的 artifact。

Artifact 采用精确版本兼容：运行时必须读取自己发布的 artifact 版本，但不承诺读取任意历史版本。受支持的离线迁移方式是保留源码，并用目标 Velin 版本重新编译。修改 artifact 版本时，必须添加 changelog 记录，并为上一个受支持版本保留固定 fixture。

Artifact 是编译程序缓存，不是长期存档。`Machine` 快照是内存值，没有稳定的序列化表示。

## C ABI

C ABI 使用不透明的 `VelinProgram` 和 `VelinMachine` handle，并同时在运行库与 header 中提供 `velin_c_api_version()` 和 `VELIN_C_API_VERSION`。当前契约版本为 ABI 1。

在 ABI 1 内：

- 已有导出函数、数字 tag、结构体字段、字段顺序、所有权规则和空指针行为保持不变。
- 可以追加新函数。已有结构体不原地扩展；需要新增带版本的结构体或函数。
- Velin 返回的缓冲区只能由配套的 Velin free 函数释放。

不兼容的布局或所有权变更必须提升 `VELIN_C_API_VERSION`，在受支持发布线内保留旧 header/runtime 配对，并提供迁移说明。

## Fixture 与 CI

兼容性门禁随普通 workspace CI 运行：

- `fixtures/compatibility/0.4.0/source.velin` 检查已经发布的源码行为。
- `fixtures/compatibility/0.4.0/artifact-v4.velinc` 独立于当前编码器检查 artifact 版本 4 解码。
- `crates/velin-capi/tests/c_api_smoke.c` 使用公开 header 编译并运行 ABI 1 宿主。

不能仅因为测试失败就重新生成历史 fixture。契约有意变化时应新增带版本 fixture；只要文档中的兼容窗口仍覆盖旧版本，就必须保留旧 fixture。

## 安全支持

安全修复覆盖当前次版本线的最新补丁，目前为 `0.4.x`。更早的次版本线和预发布版本不受支持，除非发布说明明确承诺。仓库提供私密安全报告通道时，应通过该通道报告问题。

## 发布流程

每项不兼容变更必须：

1. 标明受影响边界。
2. 在 `CHANGELOG.md` 中提供迁移说明。
3. 新增或更新带版本的兼容 fixture。
4. 在边界需要时提升 artifact 或 C ABI 版本。
5. 发布前通过源码、artifact、Rust 和 C 集成门禁。
