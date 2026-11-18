# Connector releases

每次修改节点生产代码、依赖、截图节点或安装包内容，都必须提高节点版本号。
`crates/store-connector/Cargo.toml` 使用独立版本；与 `Cargo.lock` 中的
`store-connector` 版本及 `crates/store-protocol/src/release.rs` 的
`CONNECTOR_VERSION` 保持一致。版本仅允许规范的 `major.minor.patch`，不得回退。

构建前由 `scripts/connector_release.py` 比较各平台上一次发布的节点输入摘要。
输入改变且版本未提高时必须停止；同版本同输入可重打。不得删除、改写历史构建报告
来绕过守卫。Linux/macOS 使用 `docs/deployment-build.json`，Windows 使用
`docs/windows-release-build.json`；旧报告从自己的 `sourceCommit` 恢复旧版本，
不得用当前版本代替历史版本。

发布新版本时重建 Linux、macOS、Windows 商店节点和 macOS arm64 截图节点，
全部安装包版本与中心最新版本一致后才部署。API-only 更新必须与已发布节点的
版本和输入完全一致；中心 UI、业务代码、文档和独立测试不触发节点升级。
构建报告必须保留节点版本、输入摘要及正式程序的 SHA256 和字节数。

所有构建与测试继续遵守上级 AGENTS.md 的 artifact-lifecycle 管理要求。
